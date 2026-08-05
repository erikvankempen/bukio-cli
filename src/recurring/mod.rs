//! Recurring entries & period automation (FR3A).
//! Templates are validated at creation; generation replays resolved postings.
//! Deterministic, dry-run friendly, fully audited — bukio never generates
//! entries on its own; the agent or a cron job triggers `run --due`.

use crate::core::accounts::get_account_by_code;
use crate::core::entries::{create_entry, parse_posting_specs, post_entry, reverse_entry};
use crate::error::{AppError, Result};
use crate::invoice::{
    create_invoice, get_contact, validate_invoice_lines, InvoiceLineSpec,
};
use crate::vat::{expand_vat_postings, parse_vat_posting_specs, ExpandedPosting};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub fn recurring_error(code: &'static str, message: impl Into<String>) -> AppError {
    AppError::new(code, message)
}

pub const FREQUENCIES: [&str; 3] = ["monthly", "quarterly", "yearly"];

/// One stored template posting (camelCase keys, mirroring the Node JSON).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplatePosting {
    pub code: String,
    pub amount_cents: i64,
    pub vat_code: Option<String>,
    pub vat_amount_cents: Option<i64>,
}

/// A recurring template (with parsed JSON columns).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Template {
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
    pub frequency: String,
    pub day_of_period: i64,
    pub start_date: String,
    pub end_date: Option<String>,
    pub runs: Option<i64>,
    pub postings: Vec<TemplatePosting>,
    pub final_postings: Option<Vec<TemplatePosting>>,
    pub reverse_previous: bool,
    pub status: String,
    pub next_run_date: String,
    pub last_run_date: Option<String>,
    pub last_entry_id: Option<i64>,
    pub runs_done: i64,
    pub vat_aware: bool,
    pub kind: String,
    pub contact_id: Option<i64>,
    pub invoice_lines: Option<Vec<InvoiceLineSpec>>,
    pub due_days: Option<i64>,
}

fn is_valid_date(s: &str) -> bool {
    if !regex::Regex::new(r"^\d{4}-\d{2}-\d{2}$").unwrap().is_match(s) {
        return false;
    }
    chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok()
}

fn today_iso() -> String {
    chrono::Utc::now().date_naive().format("%Y-%m-%d").to_string()
}

fn add_days(date_str: &str, days: i64) -> String {
    let d = chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d").unwrap();
    (d + chrono::Duration::days(days)).format("%Y-%m-%d").to_string()
}

/// Advance a YYYY-MM-DD date by one frequency period, keeping `day`.
pub fn add_period(date_str: &str, frequency: &str, day_of_period: i64) -> String {
    let (y, m) = {
        let mut it = date_str.split('-');
        (it.next().unwrap().parse::<i64>().unwrap(), it.next().unwrap().parse::<i64>().unwrap())
    };
    let months: i64 = match frequency {
        "monthly" => 1,
        "quarterly" => 3,
        _ => 12,
    };
    let total = y * 12 + (m - 1) + months;
    let ny = total.div_euclid(12);
    let nm = total.rem_euclid(12) + 1;
    format!("{ny}-{nm:02}-{day_of_period:02}")
}

/// Validate the resolved posting set: accounts exist/active, non-zero, balanced.
pub fn validate_postings(conn: &Connection, postings: &[TemplatePosting]) -> Result<()> {
    if postings.len() < 2 {
        return Err(recurring_error(
            "INVALID_POSTINGS",
            "a template needs at least two postings",
        ));
    }
    let mut sum: i64 = 0;
    for p in postings {
        if p.code.is_empty() || p.amount_cents == 0 {
            return Err(recurring_error(
                "INVALID_AMOUNT_CENTS",
                format!(
                    "posting for account {} must be a non-zero integer amount (cents)",
                    if p.code.is_empty() { "?" } else { &p.code }
                ),
            ));
        }
        let account = get_account_by_code(conn, &p.code)?.ok_or_else(|| {
            recurring_error("ACCOUNT_NOT_FOUND", format!("account {} does not exist", p.code))
        })?;
        if account.active == 0 {
            return Err(recurring_error(
                "ACCOUNT_INACTIVE",
                format!("account {} is deactivated", p.code),
            ));
        }
        sum += p.amount_cents;
    }
    if sum != 0 {
        return Err(recurring_error(
            "UNBALANCED",
            format!("postings do not sum to zero (sum = {sum} cents)"),
        ));
    }
    Ok(())
}

fn template_from_row(r: &rusqlite::Row) -> rusqlite::Result<Template> {
    let postings_json: String = r.get(8)?;
    let final_json: Option<String> = r.get(9)?;
    let lines_json: Option<String> = r.get(19)?;
    Ok(Template {
        id: r.get(0)?,
        name: r.get(1)?,
        description: r.get(2)?,
        frequency: r.get(3)?,
        day_of_period: r.get(4)?,
        start_date: r.get(5)?,
        end_date: r.get(6)?,
        runs: r.get(7)?,
        postings: serde_json::from_str(&postings_json).unwrap_or_default(),
        final_postings: final_json
            .as_deref()
            .map(|j| serde_json::from_str(j).unwrap_or_default()),
        reverse_previous: r.get::<_, i64>(10)? != 0,
        status: r.get(11)?,
        next_run_date: r.get(12)?,
        last_run_date: r.get(13)?,
        last_entry_id: r.get(14)?,
        runs_done: r.get(15)?,
        vat_aware: r.get::<_, i64>(16)? != 0,
        kind: r.get(17)?,
        contact_id: r.get(18)?,
        invoice_lines: lines_json
            .as_deref()
            .map(|j| serde_json::from_str(j).unwrap_or_default()),
        due_days: r.get(20)?,
    })
}

const TPL_SELECT: &str = "SELECT id, name, description, frequency, day_of_period, start_date,
    end_date, runs, postings_json, final_postings_json, reverse_previous, status,
    next_run_date, last_run_date, last_entry_id, runs_done, vat_aware, kind, contact_id,
    invoice_lines_json, due_days
    FROM recurring_templates";

pub fn get_template(conn: &Connection, id: i64) -> Result<Option<Template>> {
    let row = conn
        .query_row(&format!("{TPL_SELECT} WHERE id = ?1"), [id], template_from_row)
        .optional()?;
    Ok(row)
}

pub fn list_templates(conn: &Connection, status: &str) -> Result<Vec<Template>> {
    let sql = if status != "all" {
        format!("{TPL_SELECT} WHERE status = ?1 ORDER BY next_run_date, id")
    } else {
        format!("{TPL_SELECT} ORDER BY next_run_date, id")
    };
    let mut stmt = conn.prepare(&sql)?;
    let rows = if status != "all" {
        stmt.query_map([status], template_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?
    } else {
        stmt.query_map([], template_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    Ok(rows)
}

/// Create a recurring template. `postings` are posting-spec strings
/// (CODE:AMOUNT[@VAT]); any with a VAT code are expanded via the VAT module.
#[allow(clippy::too_many_arguments)]
pub fn create_template(
    conn: &Connection,
    name: &str,
    description: Option<&str>,
    frequency: &str,
    day_of_period: i64,
    start_date: &str,
    end_date: Option<&str>,
    runs: Option<i64>,
    postings: &[String],
    reverse_previous: bool,
    actor: &str,
    kind: &str,
    contact_id: Option<i64>,
    invoice_lines: Option<&[String]>,
    due_days: Option<i64>,
) -> Result<Template> {
    if name.is_empty() {
        return Err(recurring_error("INVALID_NAME", "template needs a name"));
    }
    if !FREQUENCIES.contains(&frequency) {
        return Err(recurring_error(
            "INVALID_FREQUENCY",
            format!("frequency must be one of {}", FREQUENCIES.join(", ")),
        ));
    }
    if kind != "entry" && kind != "invoice" {
        return Err(recurring_error("INVALID_KIND", "kind must be one of entry, invoice"));
    }
    if kind == "invoice" && reverse_previous {
        return Err(recurring_error(
            "INVALID_REVERSE",
            "reverse-previous only applies to entry templates (accrual pattern)",
        ));
    }
    if day_of_period < 1 || day_of_period > 28 {
        return Err(recurring_error(
            "INVALID_DATE",
            "day of period must be between 1 and 28",
        ));
    }
    if !is_valid_date(start_date) {
        return Err(recurring_error(
            "INVALID_DATE",
            format!("start date '{start_date}' must be a valid YYYY-MM-DD"),
        ));
    }
    if let Some(e) = end_date {
        if !is_valid_date(e) {
            return Err(recurring_error(
                "INVALID_DATE",
                format!("end date '{e}' must be a valid YYYY-MM-DD"),
            ));
        }
        if e < start_date {
            return Err(recurring_error("INVALID_RANGE", "end date must be on or after the start date"));
        }
    }
    if let Some(r) = runs {
        if r < 1 {
            return Err(recurring_error("INVALID_RUNS", "runs must be a positive integer"));
        }
    }

    let mut postings_json = "[]".to_string();
    let mut vat_aware = false;
    let mut lines_json: Option<String> = None;
    if kind == "invoice" {
        let cid = contact_id.ok_or_else(|| {
            recurring_error("INVALID_KIND", "invoice templates need --contact")
        })?;
        if get_contact(conn, cid)?.is_none() {
            return Err(recurring_error(
                "CONTACT_NOT_FOUND",
                format!("contact {cid} does not exist"),
            ));
        }
        let specs = invoice_lines.ok_or_else(|| {
            recurring_error("INVALID_KIND", "invoice templates need --lines")
        })?;
        if specs.is_empty() {
            return Err(recurring_error("INVALID_KIND", "invoice templates need --lines"));
        }
        let parsed = validate_invoice_lines(conn, specs)?;
        vat_aware = parsed.iter().any(|l| l.vat_code.is_some());
        let stored: Vec<serde_json::Value> = parsed
            .iter()
            .map(|l| {
                serde_json::json!({
                    "description": l.description,
                    "quantity": l.qty,
                    "priceCents": l.price_cents,
                    "vatCode": l.vat_code,
                })
            })
            .collect();
        postings_json = serde_json::to_string(&stored).unwrap();
        lines_json = Some(postings_json.clone());
    } else {
        let raw: Vec<String> = postings.to_vec();
        vat_aware = raw.iter().any(|p| p.contains('@'));
        let resolved: Vec<TemplatePosting> = if vat_aware {
            let specs = parse_vat_posting_specs(&raw)?;
            expand_vat_postings(conn, &specs)?
                .iter()
                .map(expanded_to_template_posting)
                .collect()
        } else {
            let mut out = Vec::new();
            for p in &raw {
                for s in parse_posting_specs(&[p.clone()])? {
                    out.push(TemplatePosting {
                        code: s.code,
                        amount_cents: s.amount_cents,
                        vat_code: None,
                        vat_amount_cents: None,
                    });
                }
            }
            out
        };
        validate_postings(conn, &resolved)?;
        postings_json = serde_json::to_string(&resolved).unwrap();
    }

    // Normalize the first run to day_of_period (never backwards).
    let (sy, sm, sd): (i64, i64, i64) = {
        let mut it = start_date.split('-');
        (
            it.next().unwrap().parse().unwrap(),
            it.next().unwrap().parse().unwrap(),
            it.next().unwrap().parse().unwrap(),
        )
    };
    let next_run = if sd > day_of_period {
        add_period(&format!("{sy}-{sm:02}-01"), frequency, day_of_period)
    } else {
        format!("{sy}-{sm:02}-{day_of_period:02}")
    };
    if let Some(e) = end_date {
        if next_run.as_str() > e {
            return Err(recurring_error(
                "INVALID_RANGE",
                "the first run date falls after the end date",
            ));
        }
    }

    let kind_col = kind.to_string();
    let contact_col = if kind == "invoice" { contact_id } else { None };
    let lines_col = if kind == "invoice" { lines_json } else { None };
    let due_col = if kind == "invoice" { due_days } else { None };
    conn.execute(
        "INSERT INTO recurring_templates
           (name, description, frequency, day_of_period, start_date, end_date, runs,
            postings_json, reverse_previous, next_run_date, vat_aware, created_by,
            kind, contact_id, invoice_lines_json, due_days)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
        params![
            name,
            description,
            frequency,
            day_of_period,
            start_date,
            end_date,
            runs,
            postings_json,
            if reverse_previous { 1 } else { 0 },
            next_run,
            if vat_aware { 1 } else { 0 },
            actor,
            kind_col,
            contact_col,
            lines_col,
            due_col
        ],
    )?;
    let id = conn.last_insert_rowid();
    crate::audit::record(
        conn,
        actor,
        "recurring.template_add",
        Some("recurring add"),
        Some(&serde_json::json!({
            "name": name, "frequency": frequency, "dayOfPeriod": day_of_period,
            "startDate": start_date, "endDate": end_date, "runs": runs,
            "reversePrevious": reverse_previous, "kind": kind
        })),
        "ok",
        &[],
    )?;
    Ok(get_template(conn, id)?.expect("just inserted"))
}

fn expanded_to_template_posting(p: &ExpandedPosting) -> TemplatePosting {
    TemplatePosting {
        code: p.code.clone(),
        amount_cents: p.amount_cents,
        vat_code: p.vat_code.clone(),
        vat_amount_cents: p.vat_amount_cents,
    }
}

pub fn set_template_status(conn: &Connection, id: i64, status: &str, actor: &str) -> Result<Template> {
    let tpl = get_template(conn, id)?.ok_or_else(|| {
        recurring_error("NOT_FOUND", format!("recurring template {id} does not exist"))
    })?;
    if tpl.status == "completed" {
        return Err(recurring_error(
            "ALREADY_COMPLETED",
            "a completed template cannot be re-activated",
        ));
    }
    conn.execute(
        "UPDATE recurring_templates SET status = ?1 WHERE id = ?2",
        params![status, id],
    )?;
    crate::audit::record(
        conn,
        actor,
        &format!("recurring.{status}"),
        Some("recurring"),
        Some(&serde_json::json!({ "id": id })),
        "ok",
        &[],
    )?;
    Ok(get_template(conn, id)?.expect("just updated"))
}

fn is_final_run(tpl: &Template) -> bool {
    tpl.final_postings.is_some()
        && tpl.runs.is_some()
        && tpl.runs_done + 1 >= tpl.runs.unwrap()
}

/// One real (non-dry) run: generate the entry/invoice, update the schedule.
fn run_template_once(tx: &Connection, tpl: &Template) -> Result<(Vec<serde_json::Value>, Option<i64>, i64, String)> {
    let entry_actor = "recurring";
    let mut generated: Vec<serde_json::Value> = Vec::new();
    let mut last_entry_id: Option<i64> = None;

    if tpl.kind == "invoice" {
        let lines: Vec<InvoiceLineSpec> = tpl
            .invoice_lines
            .as_deref()
            .map(|ls| {
                ls.iter()
                    .map(|l| InvoiceLineSpec {
                        qty: l.qty,
                        description: l.description.clone(),
                        price_cents: l.price_cents,
                        vat_code: l.vat_code.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let inv = create_invoice(
            tx,
            tpl.contact_id.unwrap_or(0),
            &lines,
            &tpl.next_run_date,
            Some(tpl.due_days.unwrap_or(30)),
            None,
            Some(&tpl.name),
            None,
            None,
            entry_actor,
        )?;
        generated.push(serde_json::json!({
            "kind": "invoice",
            "invoice": { "id": inv.id, "invoice_number": Value::Null, "status": "draft", "date": inv.date }
        }));
    } else {
        let postings: Vec<TemplatePosting> = if is_final_run(tpl) {
            tpl.final_postings.clone().unwrap()
        } else {
            tpl.postings.clone()
        };

        // accrual pattern: reverse the previous generated entry first
        if tpl.reverse_previous && tpl.last_entry_id.is_some() {
            let reversal = reverse_entry(
                tx,
                tpl.last_entry_id.unwrap(),
                entry_actor,
                Some(&format!(
                    "recurring template \"{}\" — previous period reversal",
                    tpl.name
                )),
            );
            match reversal {
                Ok(e) => generated.push(serde_json::json!({ "kind": "reversal", "entry": entry_json(&e) })),
                Err(err) => {
                    if err.code() != "ALREADY_REVERSED" && err.code() != "NOT_POSTED" {
                        return Err(err);
                    }
                }
            }
        }

        let specs: Vec<crate::core::entries::PostingSpec> = postings
            .iter()
            .map(|p| crate::core::entries::PostingSpec {
                code: p.code.clone(),
                amount_cents: p.amount_cents,
                vat_code: p.vat_code.clone(),
                vat_amount_cents: p.vat_amount_cents,
                fx_currency: None,
                fx_amount_cents: None,
            })
            .collect();
        let entry = create_entry(
            tx,
            &tpl.next_run_date,
            &format!("{} {}", tpl.name, tpl.next_run_date),
            &specs,
            "recurring",
            Some(&format!("tpl:{}", tpl.id)),
            entry_actor,
        )?;
        let posted = post_entry(tx, entry.meta.id, entry_actor)?;
        last_entry_id = Some(posted.meta.id);
        generated.push(serde_json::json!({ "kind": "entry", "entry": entry_json(&posted) }));
    }

    let next_run = add_period(&tpl.next_run_date, &tpl.frequency, tpl.day_of_period);
    let mut status = tpl.status.clone();
    let runs_done = tpl.runs_done + 1;
    if (tpl.runs.is_some() && runs_done >= tpl.runs.unwrap())
        || (tpl.end_date.is_some() && next_run > tpl.end_date.clone().unwrap())
    {
        status = "completed".to_string();
    }
    tx.execute(
        "UPDATE recurring_templates
         SET next_run_date = ?1, last_run_date = ?2, last_entry_id = ?3, runs_done = ?4, status = ?5
         WHERE id = ?6",
        params![next_run, tpl.next_run_date, last_entry_id, runs_done, status, tpl.id],
    )?;
    Ok((generated, last_entry_id, runs_done, status))
}

fn entry_json(e: &crate::core::entries::Entry) -> serde_json::Value {
    serde_json::json!({
        "id": e.meta.id, "state": e.meta.state, "date": e.meta.date, "description": e.meta.description
    })
}

/// Generate all due runs. Backfills while next_run_date <= as_of (capped at
/// max_runs_per_template). Each template runs in its own transaction — a
/// failing template is reported and skipped. dry_run returns the plan only.
pub fn run_due(
    conn: &mut Connection,
    as_of: Option<&str>,
    template_id: Option<i64>,
    actor: &str,
    dry_run: bool,
    max_runs_per_template: i64,
) -> Result<serde_json::Value> {
    let date = as_of.map(|s| s.to_string()).unwrap_or_else(today_iso);
    let mut templates: Vec<Template> = if let Some(tid) = template_id {
        match get_template(conn, tid)? {
            Some(t) if t.status == "active" && t.next_run_date <= date => vec![t],
            _ => Vec::new(),
        }
    } else {
        list_templates(conn, "active")?
            .into_iter()
            .filter(|t| t.next_run_date <= date)
            .collect()
    };

    let mut results: Vec<serde_json::Value> = Vec::new();
    for tpl in templates {
        let mut runs: Vec<serde_json::Value> = Vec::new();
        let mut ok = true;
        let mut error: Option<serde_json::Value> = None;
        if dry_run {
            // simulate without writing
            let mut sim = tpl.clone();
            for _ in 0..max_runs_per_template {
                if sim.next_run_date > date || sim.status != "active" {
                    break;
                }
                if sim.kind.as_str() == "invoice" {
                    let contact = get_contact(conn, sim.contact_id.unwrap_or(0)).ok().flatten();
                    runs.push(serde_json::json!({
                        "kind": "invoice",
                        "invoice": {
                            "date": sim.next_run_date,
                            "due_date": sim.due_days.map(|d| add_days(&sim.next_run_date, d)),
                            "contact_name": contact.map(|c| c.name),
                            "lines": sim.invoice_lines.clone().unwrap_or_default(),
                        }
                    }));
                } else {
                    let next = if sim.runs.is_some()
                        && sim.runs_done + 1 >= sim.runs.unwrap()
                        && sim.final_postings.is_some()
                    {
                        sim.final_postings.clone().unwrap()
                    } else {
                        sim.postings.clone()
                    };
                    runs.push(serde_json::json!({
                        "kind": if sim.reverse_previous && sim.last_entry_id.is_some() {
                            Value::String("reversal".to_string())
                        } else {
                            Value::Null
                        },
                        "entry": {
                            "date": sim.next_run_date,
                            "postings": next,
                            "description": format!("{} {}", sim.name, sim.next_run_date),
                        }
                    }));
                }
                sim.next_run_date = add_period(&sim.next_run_date, &sim.frequency, sim.day_of_period);
                sim.runs_done += 1;
                if (sim.runs.is_some() && sim.runs_done >= sim.runs.unwrap())
                    || (sim.end_date.is_some() && sim.next_run_date > sim.end_date.clone().unwrap())
                {
                    sim.status = "completed".to_string();
                }
            }
        } else {
            let tx_result = (|| -> Result<Vec<serde_json::Value>> {
                // Real outer transaction; the inner helpers (create_entry etc.)
                // nest via SAVEPOINT (a BEGIN inside BEGIN is an SQLite error).
                let tx = conn.transaction()?;
                let mut current = tpl.clone();
                let mut local_runs: Vec<serde_json::Value> = Vec::new();
                for _ in 0..max_runs_per_template {
                    if current.next_run_date > date || current.status != "active" {
                        break;
                    }
                    let (generated, _, _, _) = run_template_once(&tx, &current)?;
                    // Node re-reads the template from the DB after each run
                    current = tx.query_row(
                        &format!("{TPL_SELECT} WHERE id = ?1"),
                        [tpl.id],
                        template_from_row,
                    )?;
                    let entries: Vec<serde_json::Value> = generated
                        .iter()
                        .map(|g| {
                            if g["kind"] == "invoice" {
                                serde_json::json!({
                                    "kind": "invoice",
                                    "invoice_id": g["invoice"]["id"],
                                    "date": g["invoice"]["date"],
                                    "state": "draft",
                                })
                            } else {
                                serde_json::json!({
                                    "kind": g["kind"],
                                    "entry_id": g["entry"]["id"],
                                    "date": g["entry"]["date"],
                                    "state": g["entry"]["state"],
                                })
                            }
                        })
                        .collect();
                    let run_obj = serde_json::json!({ "entries": entries });
                    local_runs.push(run_obj);
                }
                tx.commit()?;
                Ok(local_runs)
            })();
            match tx_result {
                Ok(r) => runs = r,
                Err(err) => {
                    ok = false;
                    error = Some(serde_json::json!({
                        "code": err.code().to_string(),
                        "message": err.to_string().replace(&format!("[{}] ", err.code()), "")
                    }));
                }
            }
        }
        if !runs.is_empty() || !ok {
            let mut obj = serde_json::Map::new();
            obj.insert("template_id".into(), serde_json::json!(tpl.id));
            obj.insert("name".into(), serde_json::json!(tpl.name));
            obj.insert("runs".into(), serde_json::Value::Array(runs));
            obj.insert("ok".into(), serde_json::json!(ok));
            if let Some(err) = error {
                obj.insert("error".into(), err);
            }
            results.push(serde_json::Value::Object(obj));
        }
    }

    if !dry_run && results.iter().any(|r| r["ok"].as_bool().unwrap_or(false) && !r["runs"].as_array().map(|a| a.is_empty()).unwrap_or(true)) {
        let entry_ids: Vec<i64> = results
            .iter()
            .flat_map(|r| r["runs"].as_array().cloned().unwrap_or_default())
            .flat_map(|run| run["entries"].as_array().cloned().unwrap_or_default())
            .filter_map(|e| e["entry_id"].as_i64())
            .collect();
        crate::audit::record(
            conn,
            actor,
            "recurring.run",
            Some("recurring run"),
            Some(&serde_json::json!({ "asOf": date, "templates": results.iter().filter(|r| r["ok"].as_bool().unwrap_or(false)).count() })),
            "ok",
            &entry_ids,
        )?;
    }

    Ok(serde_json::json!({ "as_of": date, "dry_run": dry_run, "templates": results }))
}

/// What is due — same computation as run_due, but always read-only.
pub fn preview_due(conn: &mut Connection, as_of: Option<&str>, template_id: Option<i64>) -> Result<serde_json::Value> {
    run_due(conn, as_of, template_id, "human", true, 120)
}

/// Linear monthly depreciation with a remainder-adjusted final run.
#[allow(clippy::too_many_arguments)]
pub fn build_depreciation_template(
    conn: &Connection,
    name: &str,
    asset_code: &str,
    expense_code: &str,
    cost_cents: i64,
    residual_cents: i64,
    life_months: i64,
    start_date: &str,
    description: Option<&str>,
    actor: &str,
) -> Result<serde_json::Value> {
    if cost_cents <= 0 {
        return Err(recurring_error(
            "INVALID_COST",
            "cost must be a positive amount in cents",
        ));
    }
    if residual_cents < 0 || residual_cents >= cost_cents {
        return Err(recurring_error(
            "INVALID_RESIDUAL",
            "residual must be >= 0 and < cost",
        ));
    }
    if life_months < 2 {
        return Err(recurring_error(
            "INVALID_LIFE",
            "life-months must be an integer >= 2",
        ));
    }
    let depreciable = cost_cents - residual_cents;
    let monthly = (depreciable as f64 / life_months as f64).round() as i64;
    if monthly == 0 {
        return Err(recurring_error(
            "INVALID_LIFE",
            "life-months too long: monthly depreciation rounds to zero",
        ));
    }
    let final_cents = depreciable - monthly * (life_months - 1);
    let normal_postings = vec![
        TemplatePosting { code: expense_code.to_string(), amount_cents: monthly, vat_code: None, vat_amount_cents: None },
        TemplatePosting { code: asset_code.to_string(), amount_cents: -monthly, vat_code: None, vat_amount_cents: None },
    ];
    let final_postings = vec![
        TemplatePosting { code: expense_code.to_string(), amount_cents: final_cents, vat_code: None, vat_amount_cents: None },
        TemplatePosting { code: asset_code.to_string(), amount_cents: -final_cents, vat_code: None, vat_amount_cents: None },
    ];
    let desc_owned = description
        .map(|d| d.to_string())
        .unwrap_or_else(|| {
            format!(
                "{life_months} months linear, cost {:.2} residual {:.2}",
                cost_cents as f64 / 100.0,
                residual_cents as f64 / 100.0
            )
        });
    // postings are objects, but create_template expects spec strings; encode
    // the resolved postings directly by inserting into the DB ourselves.
    let name_owned = if name.is_empty() {
        format!("Afschrijving {asset_code}")
    } else {
        name.to_string()
    };
    // Reuse create_template with string specs derived from the resolved postings.
    let specs: Vec<String> = normal_postings
        .iter()
        .map(|p| format!("{}:{:.2}", p.code, p.amount_cents as f64 / 100.0))
        .collect();
    let tpl = create_template(
        conn,
        &name_owned,
        Some(&desc_owned),
        "monthly",
        1,
        start_date,
        None,
        Some(life_months),
        &specs,
        false,
        actor,
        "entry",
        None,
        None,
        None,
    )?;
    conn.execute(
        "UPDATE recurring_templates SET final_postings_json = ?1 WHERE id = ?2",
        params![serde_json::to_string(&final_postings).unwrap(), tpl.id],
    )?;
    let updated = get_template(conn, tpl.id)?.expect("just updated");
    Ok(serde_json::json!({
        "template": serde_json::to_value(&updated).unwrap_or_default(),
        "monthly_cents": monthly,
        "final_cents": final_cents,
        "total_cents": monthly * (life_months - 1) + final_cents,
        "monthly": format!("{:.2}", monthly as f64 / 100.0),
        "final": format!("{:.2}", final_cents as f64 / 100.0),
    }))
}
