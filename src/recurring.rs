// bukio-cli — agent-first double-entry bookkeeping for SMEs.
// Copyright (c) 2026 Erik van Kempen.
// SPDX-License-Identifier: Apache-2.0
//
// Recurring templates (mirrors src/recurring/index.js).
// Entry kind fully ported; invoice kind stubbed until invoice module lands.

use crate::accounts::get_account_by_code;
use crate::audit::{record, RecordArgs};
use crate::entries::{
    create_entry, list_entries, post_entry, reverse_entry, CreateEntry, PostingSpec,
};
use crate::money::{format_amount, BukioError, Result};
use crate::vat::{expand_vat_postings, parse_vat_posting_specs};
use rusqlite::Connection;
use serde_json::{json, Value};

pub const FREQUENCIES: &[&str] = &["monthly", "quarterly", "yearly"];
const KINDS: &[&str] = &["entry", "invoice"];

fn recurring_error(code: &'static str, msg: impl Into<String>) -> BukioError {
    BukioError::new(code, msg.into())
}

fn today_iso() -> String {
    chrono::Utc::now().format("%Y-%m-%d").to_string()
}

fn is_valid_date(s: &str) -> bool {
    chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok()
}

/// Advance a date by one frequency period.
pub fn add_period(date_str: &str, frequency: &str, day_of_period: u32) -> Result<String> {
    if day_of_period < 1 || day_of_period > 28 {
        return Err(recurring_error(
            "INVALID_DAY",
            format!("day-of-period must be 1-28, got '{day_of_period}'"),
        ));
    }
    let d = chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d")
        .map_err(|_| recurring_error("INVALID_DATE", format!("invalid date '{date_str}'")))?;
    let months = match frequency {
        "monthly" => 1i32,
        "quarterly" => 3,
        "yearly" => 12,
        _ => {
            return Err(recurring_error(
                "INVALID_FREQUENCY",
                format!("frequency must be monthly/quarterly/yearly, got '{frequency}'"),
            ))
        }
    };
    let y = d.format("%Y").to_string().parse::<i32>().unwrap();
    let m = d.format("%m").to_string().parse::<i32>().unwrap();
    let total_months = y * 12 + (m - 1) + months;
    let ny = total_months.div_euclid(12);
    let nm = (total_months.rem_euclid(12) + 1) as u32;
    Ok(format!("{ny}-{nm:02}-{day_of_period:02}"))
}

/// Validate posting set: accounts exist, non-zero, balanced.
pub fn validate_postings(db: &Connection, postings: &[PostingSpec]) -> Result<()> {
    if postings.len() < 2 {
        return Err(recurring_error(
            "INVALID_POSTINGS",
            "a template needs at least two postings",
        ));
    }
    let mut sum = 0i64;
    for p in postings {
        if p.amount_cents == 0 {
            return Err(recurring_error(
                "INVALID_AMOUNT_CENTS",
                format!("posting for account {} must be non-zero", p.code),
            ));
        }
        let account = get_account_by_code(db, &p.code).ok_or_else(|| {
            recurring_error(
                "ACCOUNT_NOT_FOUND",
                format!("account {} does not exist", p.code),
            )
        })?;
        // active check
        let active: i64 = db
            .query_row(
                "SELECT active FROM accounts WHERE code = ?1",
                [&p.code],
                |r| r.get(0),
            )
            .unwrap_or(0);
        if active == 0 {
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

/// Create a recurring template (entry kind; invoice kind stubbed).
pub fn create_template(
    db: &Connection,
    name: &str,
    description: Option<&str>,
    frequency: &str,
    day_of_period: u32,
    start_date: &str,
    end_date: Option<&str>,
    runs: Option<i64>,
    postings_json: &str,
    reverse_previous: bool,
    actor: &str,
    kind: &str,
    dry_run: bool,
) -> Result<Value> {
    if name.is_empty() {
        return Err(recurring_error("INVALID_NAME", "template needs a name"));
    }
    if !FREQUENCIES.contains(&frequency) {
        return Err(recurring_error(
            "INVALID_FREQUENCY",
            format!("frequency must be one of {}", FREQUENCIES.join(", ")),
        ));
    }
    if !KINDS.contains(&kind) {
        return Err(recurring_error(
            "INVALID_KIND",
            format!("kind must be one of {}", KINDS.join(", ")),
        ));
    }
    if kind == "invoice" {
        return Err(recurring_error(
            "INVALID_KIND",
            "invoice templates not yet ported",
        ));
    }
    if day_of_period < 1 || day_of_period > 28 {
        return Err(recurring_error(
            "INVALID_DATE",
            "day of period must be 1-28",
        ));
    }
    if !is_valid_date(start_date) {
        return Err(recurring_error(
            "INVALID_DATE",
            format!("start date '{start_date}' must be valid"),
        ));
    }
    if let Some(ed) = end_date {
        if !is_valid_date(ed) {
            return Err(recurring_error(
                "INVALID_DATE",
                format!("end date '{ed}' must be valid"),
            ));
        }
        if ed < start_date {
            return Err(recurring_error(
                "INVALID_RANGE",
                "end date must be on or after start date",
            ));
        }
    }
    if let Some(r) = runs {
        if r < 1 {
            return Err(recurring_error("INVALID_RUNS", "runs must be positive"));
        }
    }

    // validate postings
    let parsed: Vec<PostingSpec> = serde_json::from_str(postings_json).map_err(|_| {
        recurring_error("INVALID_POSTINGS", "postings_json must be valid JSON array")
    })?;
    validate_postings(db, &parsed)?;

    // compute next_run_date
    let start_parts: Vec<&str> = start_date.split('-').collect();
    let sd: u32 = start_parts[2].parse().unwrap_or(1);
    let next_run = if sd > day_of_period {
        add_period(
            &format!("{}-{}-{day_of_period:02}", start_parts[0], start_parts[1]),
            frequency,
            day_of_period,
        )?
    } else {
        format!("{}-{}-{day_of_period:02}", start_parts[0], start_parts[1])
    };
    if let Some(ed) = end_date {
        if next_run.as_str() > ed {
            return Err(recurring_error(
                "INVALID_RANGE",
                "first run falls after end date",
            ));
        }
    }

    if dry_run {
        return Ok(json!({
            "action": "recurring.template_add", "kind": kind, "name": name,
            "description": description, "frequency": frequency,
            "day_of_period": day_of_period, "start_date": start_date,
            "end_date": end_date, "runs": runs,
            "postings": postings_json, "next_run_date": next_run, "dryRun": true,
        }));
    }

    db.execute(
        "INSERT INTO recurring_templates
         (name, description, frequency, day_of_period, start_date, end_date, runs,
          postings_json, reverse_previous, next_run_date, vat_aware, created_by, kind)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 0, ?11, ?12)",
        rusqlite::params![
            name,
            description,
            frequency,
            day_of_period,
            start_date,
            end_date,
            runs,
            postings_json,
            reverse_previous as i64,
            next_run,
            actor,
            kind
        ],
    )
    .map_err(sql_err)?;
    let id = db.last_insert_rowid();
    record(
        db,
        RecordArgs {
            actor,
            action: "recurring.template_add",
            command: Some("recurring add"),
            args: Some(json!({ "name": name, "frequency": frequency })),
            outcome: "ok",
            entry_ids: vec![],
        },
    )?;
    get_template(db, id)?
        .ok_or_else(|| recurring_error("DB_ERROR", "template not found after insert"))
}

pub fn get_template(db: &Connection, id: i64) -> Result<Option<Value>> {
    let result = db.query_row(
        "SELECT id, name, description, frequency, day_of_period, start_date, end_date, runs,
                postings_json, reverse_previous, next_run_date, last_run_date, last_entry_id,
                runs_done, status, vat_aware, kind, contact_id, due_days, final_postings_json
         FROM recurring_templates WHERE id = ?1",
        [id],
        |r| {
            let posts: String = r.get(8)?;
            let fp: Option<String> = r.get(19)?;
            Ok(json!({
                "id": r.get::<_, i64>(0)?,
                "name": r.get::<_, String>(1)?,
                "description": r.get::<_, Option<String>>(2)?,
                "frequency": r.get::<_, String>(3)?,
                "day_of_period": r.get::<_, i64>(4)?,
                "start_date": r.get::<_, String>(5)?,
                "end_date": r.get::<_, Option<String>>(6)?,
                "runs": r.get::<_, Option<i64>>(7)?,
                "postings": serde_json::from_str::<Value>(&posts).unwrap_or(json!([])),
                "reverse_previous": r.get::<_, i64>(9)? == 1,
                "next_run_date": r.get::<_, String>(10)?,
                "last_run_date": r.get::<_, Option<String>>(11)?,
                "last_entry_id": r.get::<_, Option<i64>>(12)?,
                "runs_done": r.get::<_, i64>(13)?,
                "status": r.get::<_, String>(14)?,
                "vat_aware": r.get::<_, i64>(15)? == 1,
                "kind": r.get::<_, String>(16)?,
                "contact_id": r.get::<_, Option<i64>>(17)?,
                "due_days": r.get::<_, Option<i64>>(18)?,
                "final_postings": fp.and_then(|s| serde_json::from_str::<Value>(&s).ok()),
            }))
        },
    );
    match result {
        Ok(v) => Ok(Some(v)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(BukioError::new("DB_ERROR", e.to_string())),
    }
}

pub fn list_templates(db: &Connection, status: &str) -> Result<Vec<Value>> {
    let (sql, params): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = if status == "all" {
        ("SELECT id, name, kind, frequency, next_run_date, runs_done, status FROM recurring_templates ORDER BY next_run_date, id".into(), vec![])
    } else {
        ("SELECT id, name, kind, frequency, next_run_date, runs_done, status FROM recurring_templates WHERE status = ?1 ORDER BY next_run_date, id".into(), vec![Box::new(status.to_string())])
    };
    let mut stmt = db.prepare(&sql).map_err(sql_err)?;
    let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
    let rows = stmt.query_map(param_refs.as_slice(), |r| {
        Ok(json!({
            "id": r.get::<_, i64>(0)?, "name": r.get::<_, String>(1)?, "kind": r.get::<_, String>(2)?,
            "frequency": r.get::<_, String>(3)?, "next_run_date": r.get::<_, String>(4)?,
            "runs_done": r.get::<_, i64>(5)?, "status": r.get::<_, String>(6)?,
        }))
    }).map_err(sql_err)?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

pub fn set_template_status(
    db: &Connection,
    id: i64,
    status: &str,
    actor: &str,
    dry_run: bool,
) -> Result<Value> {
    let tpl = get_template(db, id)?
        .ok_or_else(|| recurring_error("NOT_FOUND", format!("template {id} does not exist")))?;
    if tpl["status"] == "completed" {
        return Err(recurring_error(
            "ALREADY_COMPLETED",
            "completed template cannot be re-activated",
        ));
    }
    if dry_run {
        return Ok(
            json!({ "action": format!("recurring.{status}"), "id": id, "from": tpl["status"], "to": status, "dryRun": true }),
        );
    }
    db.execute(
        "UPDATE recurring_templates SET status = ?1 WHERE id = ?2",
        rusqlite::params![status, id],
    )
    .map_err(sql_err)?;
    record(
        db,
        RecordArgs {
            actor,
            action: &format!("recurring.{status}"),
            command: Some("recurring"),
            args: Some(json!({ "id": id })),
            outcome: "ok",
            entry_ids: vec![],
        },
    )?;
    get_template(db, id)?.ok_or_else(|| recurring_error("DB_ERROR", "template not found"))
}

/// Run one period of a template (entry kind).
fn run_template_once(db: &Connection, tpl: &Value, actor: &str) -> Result<Value> {
    let id = tpl["id"].as_i64().unwrap();
    let postings_val = &tpl["postings"];
    let final_val = tpl.get("final_postings");
    let runs_done = tpl["runs_done"].as_i64().unwrap_or(0);
    let runs = tpl["runs"].as_i64();
    let is_final = final_val.is_some() && runs.map_or(false, |r| runs_done + 1 >= r);

    let entries_val = if is_final {
        final_val.unwrap()
    } else {
        postings_val
    };
    let postings: Vec<PostingSpec> = serde_json::from_value(entries_val.clone())
        .map_err(|_| recurring_error("INVALID_POSTINGS", "could not parse postings"))?;

    let mut generated = Vec::new();
    let mut last_entry_id: Option<i64> = None;

    // accrual: reverse previous
    if tpl["reverse_previous"].as_bool().unwrap_or(false) {
        if let Some(prev_id) = tpl["last_entry_id"].as_i64() {
            match crate::entries::reverse_entry(
                db,
                prev_id,
                actor,
                Some(&format!(
                    "recurring template \"{}\" — previous period reversal",
                    tpl["name"].as_str().unwrap_or("")
                )),
            ) {
                Ok(reversal) => {
                    generated.push(json!({ "kind": "reversal", "entry_id": reversal.id }));
                }
                Err(e) if e.code == "ALREADY_REVERSED" || e.code == "NOT_POSTED" => {}
                Err(e) => return Err(e),
            }
        }
    }

    let entry_date = tpl["next_run_date"].as_str().unwrap_or("");
    let desc = format!("{} {entry_date}", tpl["name"].as_str().unwrap_or(""));
    let source_ref = format!("tpl:{id}");
    let entry = create_entry(
        db,
        CreateEntry {
            date: entry_date,
            description: &desc,
            postings,
            source: "recurring",
            source_ref: Some(&source_ref),
            actor: "recurring",
        },
    )?;
    let posted = post_entry(db, entry.id, "recurring")?;
    last_entry_id = Some(posted.id);
    generated.push(json!({ "kind": "entry", "entry_id": last_entry_id }));

    // advance
    let freq = tpl["frequency"].as_str().unwrap_or("monthly");
    let dop = tpl["day_of_period"].as_i64().unwrap_or(1) as u32;
    let next_run = add_period(entry_date, freq, dop)?;
    let new_runs_done = runs_done + 1;
    let mut new_status = tpl["status"].as_str().unwrap_or("active").to_string();
    let end_date = tpl["end_date"].as_str();
    if (runs.map_or(false, |r| new_runs_done >= r))
        || (end_date.map_or(false, |ed| next_run.as_str() > ed))
    {
        new_status = "completed".into();
    }
    db.execute(
        "UPDATE recurring_templates SET next_run_date = ?1, last_run_date = ?2, last_entry_id = ?3, runs_done = ?4, status = ?5 WHERE id = ?6",
        rusqlite::params![next_run, entry_date, last_entry_id, new_runs_done, new_status, id],
    ).map_err(sql_err)?;

    Ok(json!({ "generated": generated, "status": new_status, "runs_done": new_runs_done }))
}

/// Generate all due runs.
pub fn run_due(
    db: &Connection,
    as_of: Option<&str>,
    template_id: Option<i64>,
    actor: &str,
    dry_run: bool,
) -> Result<Value> {
    let today = today_iso();
    let date = as_of.unwrap_or(&today);
    let templates_sql = if let Some(tid) = template_id {
        format!("SELECT id, name, description, frequency, day_of_period, start_date, end_date, runs, postings_json, reverse_previous, next_run_date, last_run_date, last_entry_id, runs_done, status, vat_aware, kind, contact_id, due_days, final_postings_json FROM recurring_templates WHERE id = {tid} AND status = 'active' AND next_run_date <= '{date}'")
    } else {
        format!("SELECT id, name, description, frequency, day_of_period, start_date, end_date, runs, postings_json, reverse_previous, next_run_date, last_run_date, last_entry_id, runs_done, status, vat_aware, kind, contact_id, due_days, final_postings_json FROM recurring_templates WHERE status = 'active' AND next_run_date <= '{date}' ORDER BY next_run_date, id")
    };
    let mut stmt = db.prepare(&templates_sql).map_err(sql_err)?;
    let tpl_rows: Vec<Value> = stmt.query_map([], |r| {
        let posts: String = r.get(8)?;
        let fp: Option<String> = r.get(19)?;
        Ok(json!({
            "id": r.get::<_, i64>(0)?, "name": r.get::<_, String>(1)?,
            "description": r.get::<_, Option<String>>(2)?,
            "frequency": r.get::<_, String>(3)?, "day_of_period": r.get::<_, i64>(4)?,
            "start_date": r.get::<_, String>(5)?, "end_date": r.get::<_, Option<String>>(6)?,
            "runs": r.get::<_, Option<i64>>(7)?, "postings": serde_json::from_str::<Value>(&posts).unwrap_or(json!([])),
            "reverse_previous": r.get::<_, i64>(9)? == 1,
            "next_run_date": r.get::<_, String>(10)?, "last_run_date": r.get::<_, Option<String>>(11)?,
            "last_entry_id": r.get::<_, Option<i64>>(12)?, "runs_done": r.get::<_, i64>(13)?,
            "status": r.get::<_, String>(14)?, "vat_aware": r.get::<_, i64>(15)? == 1,
            "kind": r.get::<_, String>(16)?,
        }))
    }).map_err(sql_err)?.filter_map(|r| r.ok()).collect();

    let mut results = Vec::new();
    for tpl in &tpl_rows {
        let mut tpl_result = json!({ "template_id": tpl["id"], "name": tpl["name"], "runs": [] });
        let mut current = tpl.clone();
        for _ in 0..120 {
            if current["status"].as_str() != Some("active")
                || current["next_run_date"].as_str().unwrap_or("") > date
            {
                break;
            }
            if current["kind"].as_str() == Some("invoice") {
                // stub: invoice templates not yet ported
                break;
            }
            match run_template_once(db, &current, actor) {
                Ok(r) => {
                    tpl_result["runs"].as_array_mut().unwrap().push(r.clone());
                    // re-read template state
                    if let Some(updated) = get_template(db, tpl["id"].as_i64().unwrap())? {
                        current = updated;
                    } else {
                        break;
                    }
                }
                Err(e) => {
                    tpl_result["ok"] = json!(false);
                    tpl_result["error"] = json!({ "code": e.code, "message": e.message });
                    break;
                }
            }
        }
        if tpl_result["runs"]
            .as_array()
            .map_or(false, |a| !a.is_empty())
            || tpl_result.get("error").is_some()
        {
            results.push(tpl_result);
        }
    }

    if !dry_run
        && results
            .iter()
            .any(|r| r["runs"].as_array().map_or(false, |a| !a.is_empty()))
    {
        record(
            db,
            RecordArgs {
                actor,
                action: "recurring.run",
                command: Some("recurring run"),
                args: Some(json!({ "asOf": date })),
                outcome: "ok",
                entry_ids: vec![],
            },
        )?;
    }
    Ok(json!({ "as_of": date, "dry_run": dry_run, "templates": results }))
}

/// Build a depreciation template (linear, remainder-adjusted final run).
pub fn build_depreciation_template(
    db: &Connection,
    name: &str,
    asset_code: &str,
    expense_code: &str,
    cost_cents: i64,
    residual_cents: i64,
    life_months: i64,
    start_date: &str,
    description: Option<&str>,
    actor: &str,
    dry_run: bool,
) -> Result<Value> {
    if cost_cents <= 0 {
        return Err(recurring_error("INVALID_COST", "cost must be positive"));
    }
    if residual_cents < 0 || residual_cents >= cost_cents {
        return Err(recurring_error(
            "INVALID_RESIDUAL",
            "residual must be >= 0 and < cost",
        ));
    }
    if life_months < 2 {
        return Err(recurring_error("INVALID_LIFE", "life-months must be >= 2"));
    }
    let depreciable = cost_cents - residual_cents;
    let monthly = ((depreciable as f64) / (life_months as f64)).round() as i64;
    if monthly == 0 {
        return Err(recurring_error(
            "INVALID_LIFE",
            "life-months too long: monthly depreciation rounds to zero",
        ));
    }
    let final_amt = depreciable - monthly * (life_months - 1);
    if final_amt <= 0 {
        return Err(recurring_error(
            "INVALID_LIFE",
            format!("final run would be {final_amt} cents"),
        ));
    }

    if dry_run {
        return Ok(json!({
            "template": null, "monthly_cents": monthly, "final_cents": final_amt,
            "total_cents": monthly * (life_months - 1) + final_amt,
            "monthly": format_amount(monthly), "final": format_amount(final_amt), "dryRun": true,
        }));
    }

    let normal = format!("[{{\"code\":\"{expense_code}\",\"amountCents\":{monthly}}},{{\"code\":\"{asset_code}\",\"amountCents\":{}}}]",
        -monthly);
    let final_postings_json = format!("[{{\"code\":\"{expense_code}\",\"amountCents\":{final_amt}}},{{\"code\":\"{asset_code}\",\"amountCents\":{}}}]",
        -final_amt);
    let tpl = create_template(
        db,
        name,
        Some(description.unwrap_or("")),
        "monthly",
        1,
        start_date,
        None,
        Some(life_months),
        &normal,
        false,
        actor,
        "entry",
        false,
    )?;
    let tpl_id = tpl["id"].as_i64().unwrap();
    db.execute(
        "UPDATE recurring_templates SET final_postings_json = ?1 WHERE id = ?2",
        rusqlite::params![final_postings_json, tpl_id],
    )
    .map_err(sql_err)?;
    let updated = get_template(db, tpl_id)?.unwrap();
    Ok(json!({
        "template": updated, "monthly_cents": monthly, "final_cents": final_amt,
        "total_cents": monthly * (life_months - 1) + final_amt,
        "monthly": format_amount(monthly), "final": format_amount(final_amt),
    }))
}

fn sql_err(e: rusqlite::Error) -> BukioError {
    BukioError::new("DB_ERROR", e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_db;

    fn db() -> Connection {
        let d = open_db(":memory:").unwrap();
        d.execute("INSERT INTO company (name) VALUES ('RecCo')", [])
            .unwrap();
        crate::accounts::seed_default_chart(&d).unwrap();
        d
    }

    #[test]
    fn add_period_monthly() {
        assert_eq!(
            add_period("2026-01-15", "monthly", 1).unwrap(),
            "2026-02-01"
        );
        assert_eq!(
            add_period("2026-01-15", "quarterly", 1).unwrap(),
            "2026-04-01"
        );
        assert_eq!(add_period("2026-01-15", "yearly", 1).unwrap(), "2027-01-01");
    }

    #[test]
    fn create_and_list_entry_template() {
        let d = db();
        let posts = r#"[{"code":"1100","amountCents":5000},{"code":"4300","amountCents":-5000}]"#;
        let tpl = create_template(
            &d,
            "Rent",
            None,
            "monthly",
            1,
            "2026-01-01",
            None,
            None,
            posts,
            false,
            "human:erik",
            "entry",
            false,
        )
        .unwrap();
        assert_eq!(tpl["name"], "Rent");
        assert_eq!(tpl["status"], "active");
        assert_eq!(tpl["next_run_date"], "2026-01-01");
        let list = list_templates(&d, "active").unwrap();
        assert_eq!(list.len(), 1);
    }

    #[test]
    fn build_depreciation_basic() {
        let d = db();
        let result = build_depreciation_template(
            &d,
            "Laptop",
            "1800",
            "4600",
            120000,
            0,
            24,
            "2026-01-01",
            None,
            "human:erik",
            false,
        )
        .unwrap();
        assert_eq!(result["monthly_cents"], 5000);
        assert_eq!(result["final_cents"], 5000); // remainder-adjusted: 120000/24=5000, final=5000
        assert_eq!(result["total_cents"], 120000);
    }
}
