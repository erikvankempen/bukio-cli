// bukio-cli — agent-first double-entry bookkeeping for SMEs.
// Copyright (c) 2026 Erik van Kempen.
// SPDX-License-Identifier: Apache-2.0
//
// Recurring templates (mirrors src/recurring/index.js).
// Entry kind fully ported; invoice kind stubbed until invoice module lands.

use crate::accounts::get_account_by_code;
use crate::audit::{record, RecordArgs};
use crate::contacts::get_contact;
use crate::entries::{
    create_entry, list_entries, post_entry, reverse_entry, CreateEntry, PostingSpec,
};
use crate::invoice::{
    create_invoice, parse_item_spec, parse_line_spec, split_item_specs, split_line_specs,
};
use crate::items::get_item;
use crate::money::{format_amount, BukioError, Result};
use crate::vat::is_vat_enabled;
use crate::vat::{expand_vat_postings, list_vat_codes, parse_vat_posting_specs};
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

/// Add n days to a YYYY-MM-DD date (used by dry-run due_date simulation).
fn add_days(date_str: &str, days: i64) -> String {
    let d = chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d").unwrap_or_default();
    (d + chrono::Duration::days(days))
        .format("%Y-%m-%d")
        .to_string()
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

/// Create a recurring template (entry or invoice/subscription kind).
///
/// For invoice kind, `postings_json` is not a postings array but the
/// serialized CLI spec object `{"contact_id":N,"lines":[...],"items":[...],"due_days":D}`.
/// Line/item specs are validated at creation; item catalog prices are
/// snapshotted per run (recurring run), mirroring src/recurring/index.js.
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
    if kind == "invoice" && reverse_previous {
        return Err(recurring_error(
            "INVALID_REVERSE",
            "reverse-previous only applies to entry templates (accrual pattern)",
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

    // Invoice kind: the spec object arrives as postings_json from the CLI.
    let (invoice_spec, invoice_lines_raw, invoice_items_raw, due_days, mut vat_aware_flag) = if kind
        == "invoice"
    {
        if reverse_previous {
            return Err(recurring_error(
                "INVALID_REVERSE",
                "reverse-previous only applies to entry templates (accrual pattern)",
            ));
        }
        let v: Value = serde_json::from_str(postings_json).unwrap_or(Value::Null);
        let contact_id = v
            .get("contact_id")
            .and_then(|x| x.as_i64())
            .ok_or_else(|| recurring_error("INVALID_KIND", "invoice templates need --contact"))?;
        if get_contact(db, contact_id)?.is_none() {
            return Err(recurring_error(
                "CONTACT_NOT_FOUND",
                format!("contact {contact_id} does not exist"),
            ));
        }
        let lines: Vec<String> = v
            .get("lines")
            .and_then(|l| l.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let items: Vec<String> = v
            .get("items")
            .and_then(|l| l.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        if lines.is_empty() && items.is_empty() {
            return Err(recurring_error(
                "INVALID_KIND",
                "invoice templates need --lines or --items",
            ));
        }
        if !lines.is_empty() && !items.is_empty() {
            return Err(recurring_error(
                "INVALID_KIND",
                "pass either --lines or --items, not both",
            ));
        }
        let due = match v.get("due_days").and_then(|x| x.as_i64()) {
            Some(d) => {
                if d < 0 {
                    return Err(recurring_error(
                        "INVALID_DUE_DAYS",
                        "due-days must be a non-negative integer",
                    ));
                }
                Some(d)
            }
            None => Some(30),
        };
        // Validate specs up front (VAT codes must exist at creation).
        let vat_on = is_vat_enabled(db);
        let vat_codes: std::collections::HashMap<String, Value> = list_vat_codes(db)?
            .into_iter()
            .filter_map(|c| {
                let code = c.get("code")?.as_str()?.to_string();
                Some((code, c))
            })
            .collect();
        let mut vat_aware = false;
        if !items.is_empty() {
            // objects (already-parsed specs from a stored template) pass through;
            // only string specs are split. Stringifying everything made a stored
            // object arrive at parse_item_spec as its own JSON text.
            let specs: Vec<Value> =
                split_item_specs(&items.iter().cloned().map(Value::String).collect::<Vec<_>>());
            for p in specs {
                // a spec may arrive as the JSON text of an already-parsed object
                // (the CLI stores --items entries that way) — decode those instead
                // of handing the JSON text to the line-spec parser
                let p = if let Some(t) = p.as_str() {
                    if t.trim_start().starts_with('{') {
                        serde_json::from_str::<Value>(t).unwrap_or(Value::String(t.to_string()))
                    } else {
                        p.clone()
                    }
                } else {
                    p.clone()
                };
                let id = p["item_id"]
                    .as_i64()
                    .or_else(|| p["itemId"].as_i64())
                    .ok_or_else(|| {
                        recurring_error(
                            "INVALID_ITEM_SPEC",
                            format!("item spec {:?} has no item id", p),
                        )
                    })?;
                let item = get_item(db, id)?.ok_or_else(|| {
                    recurring_error("ITEM_NOT_FOUND", format!("item {id} does not exist"))
                })?;
                if item["active"].as_bool() == Some(false) {
                    return Err(recurring_error(
                        "ITEM_INACTIVE",
                        format!("item {id} is deactivated"),
                    ));
                }
                if p["vat_code"].as_str().is_some() || item["vat_code"].as_str().is_some() {
                    vat_aware = true;
                }
            }
        } else {
            let specs: Vec<Value> =
                split_line_specs(&lines.iter().cloned().map(Value::String).collect::<Vec<_>>());
            for spec in specs {
                let parsed = match spec.as_str() {
                    Some(text) => crate::invoice::parse_line_spec(text)?,
                    None => spec.clone(),
                };
                let vc = parsed["vatCode"].as_str().map(String::from);
                if let Some(ref code) = vc {
                    vat_aware = true;
                    if !vat_on {
                        return Err(recurring_error(
                            "VAT_MODULE_OFF",
                            "line has a VAT code but the VAT module is off for this company",
                        ));
                    }
                    if !vat_codes.contains_key(code) {
                        return Err(recurring_error(
                            "VAT_CODE_NOT_FOUND",
                            format!("vat code '{code}' does not exist"),
                        ));
                    }
                }
            }
        }
        (
            Some(contact_id),
            if items.is_empty() { Some(lines) } else { None },
            if items.is_empty() { None } else { Some(items) },
            due,
            vat_aware,
        )
    } else {
        (None, None, None, None, false)
    };

    // The JS resolves a MIX of strings ("CODE:AMOUNT[@VAT]", one string may carry
    // several comma-separated postings) and plain objects. The port only
    // understood objects, so the documented "4300:1000.00,1100:-1000.00" form
    // failed INVALID_POSTINGS and a VAT-tagged entry silently lost its VAT leg.
    let mut entry_postings: Option<String> = None;
    if kind != "invoice" {
        let raw: Vec<Value> = serde_json::from_str(postings_json).map_err(|_| {
            recurring_error("INVALID_POSTINGS", "postings_json must be valid JSON array")
        })?;
        let has_tag = raw
            .iter()
            .any(|p| p.as_str().map(|s| s.contains('@')).unwrap_or(false));
        let mut resolved: Vec<Value> = Vec::new();
        if has_tag {
            let strings: Vec<String> = raw
                .iter()
                .filter_map(|p| p.as_str().map(String::from))
                .collect();
            let specs = parse_vat_posting_specs(&strings)?;
            let (expanded, vats) = expand_vat_postings(db, &specs)?;
            for (i, sp) in expanded.iter().enumerate() {
                let (vc, va) = vats.get(i).cloned().unwrap_or((None, None));
                resolved.push(json!({
                    "code": sp.code, "amountCents": sp.amount_cents,
                    "vatCode": vc, "vatAmountCents": va,
                }));
            }
            // object postings in the same list are already final form — keep them
            for p in raw.iter().filter(|p| !p.is_string()) {
                resolved.push(json!({
                    "code": p["code"].clone(), "amountCents": p["amountCents"].clone(),
                    "vatCode": p["vatCode"].clone(), "vatAmountCents": p["vatAmountCents"].clone(),
                }));
            }
        } else {
            for p in &raw {
                match p.as_str() {
                    Some(spec) => {
                        for sp in crate::entries::parse_posting_specs(&[spec.to_string()])? {
                            resolved.push(json!({
                                "code": sp.code, "amountCents": sp.amount_cents,
                                "vatCode": sp.vat_code, "vatAmountCents": sp.vat_amount_cents,
                            }));
                        }
                    }
                    None => resolved.push(json!({
                        "code": p["code"].clone(), "amountCents": p["amountCents"].clone(),
                        "vatCode": p["vatCode"].clone(), "vatAmountCents": p["vatAmountCents"].clone(),
                    })),
                }
            }
        }
        let parsed: Vec<PostingSpec> = serde_json::from_value(Value::Array(resolved.clone()))
            .map_err(|e| {
                recurring_error("INVALID_POSTINGS", format!("invalid posting spec: {e}"))
            })?;
        validate_postings(db, &parsed)?;
        vat_aware_flag = has_tag;
        entry_postings = Some(serde_json::to_string(&resolved).unwrap());
    }

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
        // the JS's plan: the raw specs joined with ", " (objects stringified),
        // null when there are none; vat_aware/reverse_previous always present
        let postings_text: Value = if kind == "invoice" {
            Value::Null
        } else {
            match serde_json::from_str::<Value>(postings_json) {
                Ok(Value::Array(items)) if !items.is_empty() => {
                    let parts: Vec<String> = items
                        .iter()
                        .map(|p| match p.as_str() {
                            Some(s) => s.to_string(),
                            None => p.to_string(),
                        })
                        .collect();
                    Value::String(parts.join(", "))
                }
                _ => Value::Null,
            }
        };
        let mut plan = json!({
            "action": "recurring.template_add", "kind": kind, "name": name,
            "description": description, "frequency": frequency,
            "day_of_period": day_of_period, "start_date": start_date,
            "end_date": end_date, "runs": runs,
            "due_days": due_days, "reverse_previous": reverse_previous,
            "contact_id": invoice_spec, "vat_aware": vat_aware_flag,
            "postings": postings_text,
            "lines": Value::Null, "items": Value::Null,
            "next_run_date": next_run, "dryRun": true,
        });
        if let Some(ref l) = invoice_lines_raw {
            plan["lines"] = json!(l);
        }
        if let Some(ref i) = invoice_items_raw {
            plan["items"] = json!(i);
        }
        return Ok(plan);
    }

    // postings_json: invoice templates store '[]' (lines/items live in the
    // invoice_*_json columns, mirroring the JS engine).
    let stored_postings = if kind == "invoice" {
        "[]".to_string()
    } else {
        entry_postings.unwrap_or_else(|| postings_json.to_string())
    };
    let invoice_lines_json = invoice_lines_raw.map(|l| serde_json::to_string(&l).unwrap());
    let invoice_items_json = invoice_items_raw.map(|i| serde_json::to_string(&i).unwrap());

    db.execute(
        "INSERT INTO recurring_templates
         (name, description, frequency, day_of_period, start_date, end_date, runs,
          postings_json, reverse_previous, next_run_date, vat_aware, created_by, kind,
          contact_id, invoice_lines_json, invoice_items_json, due_days)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
        rusqlite::params![
            name,
            description,
            frequency,
            day_of_period,
            start_date,
            end_date,
            runs,
            stored_postings,
            reverse_previous as i64,
            next_run,
            vat_aware_flag as i64,
            actor,
            kind,
            invoice_spec,
            invoice_lines_json,
            invoice_items_json,
            due_days,
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
                runs_done, status, vat_aware, kind, contact_id, due_days, final_postings_json,
                invoice_lines_json, invoice_items_json
         FROM recurring_templates WHERE id = ?1",
        [id],
        |r| {
            let posts: String = r.get(8)?;
            let fp: Option<String> = r.get(19)?;
            let il: Option<String> = r.get(20)?;
            let it: Option<String> = r.get(21)?;
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
                "reverse_previous": r.get::<_, i64>(9)?,
                "next_run_date": r.get::<_, String>(10)?,
                "last_run_date": r.get::<_, Option<String>>(11)?,
                "last_entry_id": r.get::<_, Option<i64>>(12)?,
                "runs_done": r.get::<_, i64>(13)?,
                "status": r.get::<_, String>(14)?,
                "vat_aware": r.get::<_, i64>(15)?,
                "kind": r.get::<_, String>(16)?,
                "contact_id": r.get::<_, Option<i64>>(17)?,
                "due_days": r.get::<_, Option<i64>>(18)?,
                "final_postings": fp.and_then(|s| serde_json::from_str::<Value>(&s).ok()),
                "invoice_lines": il.and_then(|s| serde_json::from_str::<Value>(&s).ok()),
                "invoice_items": it.and_then(|s| serde_json::from_str::<Value>(&s).ok()),
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
    let tpl = get_template(db, id)?.ok_or_else(|| {
        recurring_error(
            "NOT_FOUND",
            format!("recurring template {id} does not exist"),
        )
    })?;
    if tpl["status"] == "completed" {
        return Err(recurring_error(
            "ALREADY_COMPLETED",
            "completed template cannot be re-activated",
        ));
    }
    if dry_run {
        return Ok(
            json!({ "action": format!("recurring.{status}"), "id": format!("{id}"), "from": tpl["status"], "to": status, "dryRun": true }),
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

/// Run one period of a template (entry or invoice kind).
/// Invoice kind generates a DRAFT sales invoice dated next_run_date —
/// finalizing stays a separate audited action (never auto-finalize).
fn run_template_once(db: &Connection, tpl: &Value, actor: &str) -> Result<Value> {
    let id = tpl["id"].as_i64().unwrap();
    let runs_done = tpl["runs_done"].as_i64().unwrap_or(0);
    let runs = tpl["runs"].as_i64();
    let entry_date = tpl["next_run_date"].as_str().unwrap_or("");

    let mut generated = Vec::new();
    let mut last_entry_id: Option<i64> = None;

    if tpl["kind"].as_str() == Some("invoice") {
        let contact_id = tpl["contact_id"]
            .as_i64()
            .ok_or_else(|| recurring_error("INVALID_KIND", "invoice template has no contact_id"))?;
        let due_days = tpl["due_days"].as_i64().unwrap_or(30);
        let name = tpl["name"].as_str().unwrap_or("recurring invoice");
        // Snapshot item catalog prices at generation time: item specs are
        // stored verbatim and resolved per run (mirrors the JS engine).
        let lines_raw: Vec<Value> = if let Some(items) =
            tpl.get("invoice_items").and_then(|i| i.as_array())
        {
            let mut resolved = Vec::new();
            let item_specs: Vec<Value> = split_item_specs(items);
            for p in item_specs {
                // parse_item_spec returns the JS camelCase shape; stored
                // templates may still carry the older snake_case keys
                let pv = |snake: &str, camel: &str| -> Value {
                    if p[snake].is_null() {
                        p[camel].clone()
                    } else {
                        p[snake].clone()
                    }
                };
                let item_id = pv("item_id", "itemId").as_i64().ok_or_else(|| {
                    recurring_error("INVALID_ITEM_SPEC", "item spec has no item id")
                })?;
                let item = get_item(db, item_id)?.ok_or_else(|| {
                    recurring_error("ITEM_NOT_FOUND", format!("item {item_id} does not exist"))
                })?;
                // build a line object create_invoice understands
                let qty = pv("qty_milli", "qtyMilli").as_i64().unwrap_or(1000);
                let price = pv("price_cents", "priceCents")
                    .as_i64()
                    .or_else(|| item["unit_price_cents"].as_i64())
                    .unwrap_or(0);
                let vat = pv("vat_code", "vatCode")
                    .as_str()
                    .map(String::from)
                    .or_else(|| item["vat_code"].as_str().map(String::from));
                resolved.push(json!({
                    "description": item["description"].as_str().or_else(|| item["name"].as_str()).unwrap_or("item"),
                    "qty_milli": qty,
                    "price_cents": price,
                    "vat_code": vat,
                    "discount_type": pv("discount_type", "discountType"),
                    "discount_value": pv("discount_value", "discountValue"),
                    "item_id": item_id,
                    "unit": item["unit"],
                    "gl_account": item["gl_account"],
                }));
            }
            resolved
        } else if let Some(lines) = tpl.get("invoice_lines").and_then(|l| l.as_array()) {
            lines.clone()
        } else {
            return Err(recurring_error(
                "INVALID_KIND",
                "invoice template has no lines or items",
            ));
        };
        let inv = create_invoice(
            db,
            contact_id,
            entry_date,
            Some(due_days),
            Some(name),
            None,
            None,
            None,
            None,
            // ponytail: templates carry no language yet — add when one does
            None,
            &lines_raw,
            "recurring",
            false,
        )?;
        generated.push(json!({
            "kind": "invoice",
            "invoice": { "id": inv["id"], "invoice_number": null, "status": "draft", "date": inv["date"] },
        }));
    } else {
        let postings_val = &tpl["postings"];
        let final_val = tpl.get("final_postings").filter(|v| !v.is_null());
        let is_final = final_val.is_some() && runs.map_or(false, |r| runs_done + 1 >= r);
        let entries_val = if is_final {
            final_val.unwrap()
        } else {
            postings_val
        };
        let postings: Vec<PostingSpec> = serde_json::from_value(entries_val.clone())
            .map_err(|_| recurring_error("INVALID_POSTINGS", "could not parse postings"))?;

        // accrual: reverse previous
        if tpl["reverse_previous"].as_i64().unwrap_or(0) == 1 {
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
                        generated.push(json!({
                            "kind": "reversal",
                            "entry": serde_json::to_value(&reversal).unwrap_or(Value::Null),
                        }));
                    }
                    Err(e) if e.code == "ALREADY_REVERSED" || e.code == "NOT_POSTED" => {}
                    Err(e) => return Err(e),
                }
            }
        }

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
        // the JS pushes the whole posted entry ({ kind:'entry', entry: posted })
        generated.push(json!({
            "kind": "entry",
            "entry": serde_json::to_value(&posted).unwrap_or(Value::Null),
        }));
    }

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
/// The JS's previewDue: a read-only plan, identical to runDue(dryRun: true).
pub fn preview_due(
    db: &Connection,
    as_of: Option<&str>,
    template_id: Option<i64>,
) -> Result<Value> {
    run_due(db, as_of, template_id, "recurring", true)
}

pub fn run_due(
    db: &Connection,
    as_of: Option<&str>,
    template_id: Option<i64>,
    actor: &str,
    dry_run: bool,
) -> Result<Value> {
    let today = today_iso();
    let date = as_of.unwrap_or(&today);
    crate::dates::validate_date(date)?;
    let templates_sql = if let Some(tid) = template_id {
        format!("SELECT id, name, description, frequency, day_of_period, start_date, end_date, runs, postings_json, reverse_previous, next_run_date, last_run_date, last_entry_id, runs_done, status, vat_aware, kind, contact_id, due_days, final_postings_json, invoice_lines_json, invoice_items_json FROM recurring_templates WHERE id = {tid} AND status = 'active' AND next_run_date <= '{date}'")
    } else {
        format!("SELECT id, name, description, frequency, day_of_period, start_date, end_date, runs, postings_json, reverse_previous, next_run_date, last_run_date, last_entry_id, runs_done, status, vat_aware, kind, contact_id, due_days, final_postings_json, invoice_lines_json, invoice_items_json FROM recurring_templates WHERE status = 'active' AND next_run_date <= '{date}' ORDER BY next_run_date, id")
    };
    let mut stmt = db.prepare(&templates_sql).map_err(sql_err)?;
    let tpl_rows: Vec<Value> = stmt.query_map([], |r| {
        let posts: String = r.get(8)?;
        let fp: Option<String> = r.get(19)?;
        let il: Option<String> = r.get(20)?;
        let it: Option<String> = r.get(21)?;
        Ok(json!({
            "id": r.get::<_, i64>(0)?, "name": r.get::<_, String>(1)?,
            "description": r.get::<_, Option<String>>(2)?,
            "frequency": r.get::<_, String>(3)?, "day_of_period": r.get::<_, i64>(4)?,
            "start_date": r.get::<_, String>(5)?, "end_date": r.get::<_, Option<String>>(6)?,
            "runs": r.get::<_, Option<i64>>(7)?, "postings": serde_json::from_str::<Value>(&posts).unwrap_or(json!([])),
            "reverse_previous": r.get::<_, i64>(9)?,
            "next_run_date": r.get::<_, String>(10)?, "last_run_date": r.get::<_, Option<String>>(11)?,
            "last_entry_id": r.get::<_, Option<i64>>(12)?, "runs_done": r.get::<_, i64>(13)?,
            "status": r.get::<_, String>(14)?, "vat_aware": r.get::<_, i64>(15)?,
            "kind": r.get::<_, String>(16)?,
            "contact_id": r.get::<_, Option<i64>>(17)?, "due_days": r.get::<_, Option<i64>>(18)?,
            "invoice_lines": il.and_then(|s| serde_json::from_str::<Value>(&s).ok()),
            "invoice_items": it.and_then(|s| serde_json::from_str::<Value>(&s).ok()),
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
            if dry_run {
                // simulate the run without writing (mirrors the JS engine's
                // dryRun branch: plan entries carry no ids, no DB writes)
                let kind = current["kind"].as_str().unwrap_or("entry").to_string();
                let run_date = current["next_run_date"].as_str().unwrap_or("").to_string();
                let run = if kind == "invoice" {
                    let contact_name = current["contact_id"]
                        .as_i64()
                        .and_then(|cid| get_contact(db, cid).ok().flatten())
                        .and_then(|c| c["name"].as_str().map(String::from))
                        .unwrap_or_else(|| "contact".into());
                    let due_days = current["due_days"].as_i64().unwrap_or(0);
                    let due_date = add_days(&run_date, due_days);
                    json!({
                        "kind": "invoice",
                        "invoice": {
                            "date": run_date,
                            "due_date": due_date,
                            "contact_name": contact_name,
                            "lines": current.get("invoice_items").and_then(|v| v.as_array()).cloned()
                                .or_else(|| current.get("invoice_lines").and_then(|v| v.as_array()).cloned())
                                .unwrap_or_default(),
                        },
                    })
                } else {
                    let name = current["name"].as_str().unwrap_or("").to_string();
                    let postings = current
                        .get("postings")
                        .cloned()
                        .unwrap_or_else(|| json!([]));
                    let mut run = json!({
                        "kind": "entry",
                        "entry": {
                            "date": run_date,
                            "postings": postings,
                            "description": format!("{name} {run_date}"),
                        },
                    });
                    if current["reverse_previous"].as_i64().unwrap_or(0) == 1
                        && current["last_entry_id"].as_i64().is_some()
                    {
                        tpl_result["runs"].as_array_mut().unwrap().push(json!({
                            "kind": "reversal",
                            "entry": {
                                "id": current["last_entry_id"],
                                "date": run_date,
                                "description": format!("recurring template \"{name}\" — previous period reversal"),
                            },
                        }));
                    }
                    run
                };
                tpl_result["runs"].as_array_mut().unwrap().push(run);
                // advance the simulated template
                let freq = current["frequency"]
                    .as_str()
                    .unwrap_or("monthly")
                    .to_string();
                let dop = current["day_of_period"].as_i64().unwrap_or(1) as u32;
                let next_run = add_period(&run_date, &freq, dop)?;
                let runs_done = current["runs_done"].as_i64().unwrap_or(0) + 1;
                let runs = current["runs"].as_i64();
                let end_date = current["end_date"].as_str().map(String::from);
                let mut status = current["status"].as_str().unwrap_or("active").to_string();
                if (runs.map_or(false, |r| runs_done >= r))
                    || (end_date
                        .as_ref()
                        .map_or(false, |ed| next_run.as_str() > ed.as_str()))
                {
                    status = "completed".into();
                }
                current["next_run_date"] = json!(next_run);
                current["runs_done"] = json!(runs_done);
                current["status"] = json!(status);
                if current["reverse_previous"].as_i64().unwrap_or(0) == 1 {
                    current["last_entry_id"] = json!(-runs_done);
                }
            } else {
                tpl_result["ok"] = json!(true);
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
        }
        // the JS sets ok = true after the try/catch, for BOTH paths
        if tpl_result.get("error").is_none() {
            tpl_result["ok"] = json!(true);
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

    #[test]
    fn invoice_template_create_and_run_generates_drafts() {
        let d = db();
        crate::contacts::create_contact(
            &d,
            "ACME BV",
            Some("Straat 1"),
            Some("1000 AA"),
            Some("Amsterdam"),
            Some("NL"),
            None,
            None,
            None,
            Some("NL91ABNA0417164300"),
            "human:erik",
            false,
        )
        .unwrap();
        let spec = json!({ "contact_id": 1, "lines": ["2x Coaching @ 100.00"], "items": [], "due_days": 14 });
        let tpl = create_template(
            &d,
            "SaaS",
            None,
            "monthly",
            1,
            "2026-01-15",
            None,
            None,
            &spec.to_string(),
            false,
            "human:erik",
            "invoice",
            false,
        )
        .unwrap();
        assert_eq!(tpl["kind"], "invoice");
        assert_eq!(tpl["contact_id"], 1);
        assert_eq!(tpl["due_days"], 14);
        assert_eq!(tpl["vat_aware"].as_i64(), Some(0));
        assert_eq!(
            tpl["invoice_lines"].as_array().unwrap()[0],
            "2x Coaching @ 100.00"
        );

        // dry-run generates plan entries without writing
        let plan = run_due(&d, Some("2026-02-01"), None, "human:erik", true).unwrap();
        assert_eq!(plan["templates"][0]["runs"].as_array().unwrap().len(), 1);
        assert_eq!(
            plan["templates"][0]["runs"][0]["invoice"]["due_date"],
            "2026-02-15"
        );
        let invoice_count: i64 = d
            .query_row("SELECT COUNT(*) FROM invoices", [], |r| r.get(0))
            .unwrap();
        assert_eq!(invoice_count, 0, "dry-run must not write invoices");

        // real run generates a draft invoice (never finalizes)
        let real = run_due(&d, Some("2026-02-01"), None, "human:erik", false).unwrap();
        assert_eq!(
            real["templates"][0]["runs"][0]["generated"][0]["kind"],
            "invoice"
        );
        assert_eq!(
            real["templates"][0]["runs"][0]["generated"][0]["invoice"]["status"],
            "draft"
        );
        let invoice_count: i64 = d
            .query_row("SELECT COUNT(*) FROM invoices", [], |r| r.get(0))
            .unwrap();
        assert_eq!(invoice_count, 1);
        assert_eq!(
            d.query_row("SELECT status FROM invoices", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "draft"
        );

        // item-based template: specs stored verbatim, prices snapshotted per run
        crate::items::create_item(
            &d,
            "SaaS",
            None,
            "unit",
            9900,
            None,
            None,
            "human:erik",
            false,
        )
        .unwrap();
        let item_spec =
            json!({ "contact_id": 1, "lines": [], "items": ["1:2@140.00"], "due_days": 14 });
        let tpl2 = create_template(
            &d,
            "SaaS items",
            None,
            "monthly",
            1,
            "2026-04-01",
            None,
            None,
            &item_spec.to_string(),
            false,
            "human:erik",
            "invoice",
            false,
        )
        .unwrap();
        assert_eq!(tpl2["invoice_items"].as_array().unwrap()[0], "1:2@140.00");
        let run2 = run_due(&d, Some("2026-04-01"), None, "human:erik", false).unwrap();
        // find the item template's run by name (the line template is also due)
        let item_run = run2["templates"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == "SaaS items")
            .unwrap();
        assert_eq!(item_run["runs"][0]["generated"][0]["kind"], "invoice");
        // the generated line carries the item price snapshot
        let inv2_id = item_run["runs"][0]["generated"][0]["invoice"]["id"]
            .as_i64()
            .unwrap();
        let line: Value = d
            .query_row(
                "SELECT description, quantity FROM invoice_lines WHERE invoice_id = ?1",
                [inv2_id],
                |r| Ok(json!({ "desc": r.get::<_, String>(0)?, "qty": r.get::<_, i64>(1)? })),
            )
            .unwrap();
        assert_eq!(line["desc"], "SaaS");
        assert_eq!(line["qty"], 2000);
    }

    // ==== ported from test/recurring.test.js ================================
    fn tdb() -> Connection {
        let d = crate::db::open_db(":memory:").unwrap();
        crate::accounts::seed_default_chart(&d).unwrap();
        d.execute(
            "INSERT INTO company (name, legal_form) VALUES ('Test BV','bv')",
            [],
        )
        .unwrap();
        d
    }

    /// The JS test's tpl(overrides) helper.
    fn tpl(db: &Connection, over: Value) -> Result<Value> {
        let mut o = json!({
            "name": "Huur kantoor", "frequency": "monthly", "dayOfPeriod": 1,
            "startDate": "2026-01-01", "postings": ["4300:1000.00,1100:-1000.00"],
        });
        if let Some(m) = over.as_object() {
            for (k, v) in m {
                o[k] = v.clone();
            }
        }
        create_template(
            db,
            o["name"].as_str().unwrap(),
            None,
            o["frequency"].as_str().unwrap(),
            o["dayOfPeriod"].as_u64().unwrap_or(1) as u32,
            o["startDate"].as_str().unwrap(),
            o["endDate"].as_str(),
            o["runs"].as_i64(),
            &serde_json::to_string(&o["postings"]).unwrap(),
            o["reversePrevious"].as_bool().unwrap_or(false),
            "agent:test",
            "entry",
            false,
        )
    }

    fn tpl_ok(db: &Connection) -> Value {
        tpl(db, json!({})).unwrap()
    }

    fn one(db: &Connection, sql: &str) -> i64 {
        db.query_row(sql, [], |r| r.get::<_, Option<i64>>(0))
            .unwrap()
            .unwrap_or(0)
    }

    fn entry_json(db: &Connection, id: i64) -> Value {
        serde_json::to_value(crate::entries::get_entry(db, id).unwrap()).unwrap()
    }

    #[test]
    fn add_period_monthly_quarterly_yearly_keeps_the_day() {
        assert_eq!(
            add_period("2026-01-15", "monthly", 1).unwrap(),
            "2026-02-01"
        );
        assert_eq!(
            add_period("2026-11-01", "monthly", 5).unwrap(),
            "2026-12-05"
        );
        assert_eq!(
            add_period("2026-12-01", "monthly", 1).unwrap(),
            "2027-01-01"
        );
        assert_eq!(
            add_period("2026-01-01", "quarterly", 1).unwrap(),
            "2026-04-01"
        );
        assert_eq!(
            add_period("2026-10-01", "quarterly", 1).unwrap(),
            "2027-01-01"
        );
        assert_eq!(add_period("2026-01-01", "yearly", 1).unwrap(), "2027-01-01");
    }

    #[test]
    fn create_template_validates_postings_balance_accounts() {
        let d = tdb();
        let e = tpl(&d, json!({"postings": ["4300:1000.00"]})).unwrap_err();
        assert_eq!(e.code, "INVALID_POSTINGS");
        let e = tpl(&d, json!({"postings": ["4300:1000.00,1100:-999.00"]})).unwrap_err();
        assert_eq!(e.code, "UNBALANCED");
        let e = tpl(&d, json!({"postings": ["9999:100.00,1100:-100.00"]})).unwrap_err();
        assert_eq!(e.code, "ACCOUNT_NOT_FOUND");
        let e = tpl(&d, json!({"frequency": "weekly"})).unwrap_err();
        assert_eq!(e.code, "INVALID_FREQUENCY");
        let e = tpl(&d, json!({"dayOfPeriod": 29})).unwrap_err();
        assert_eq!(e.code, "INVALID_DATE");
        let e = tpl(&d, json!({"startDate": "2026-13-01"})).unwrap_err();
        assert_eq!(e.code, "INVALID_DATE");
        let e = tpl(&d, json!({"endDate": "2025-12-01"})).unwrap_err();
        assert_eq!(e.code, "INVALID_RANGE");
    }

    #[test]
    fn create_template_rejects_an_inactive_account() {
        let d = tdb();
        crate::accounts::deactivate_account(&d, "4300").unwrap();
        let e = tpl(&d, json!({})).unwrap_err();
        assert_eq!(e.code, "ACCOUNT_INACTIVE");
    }

    #[test]
    fn create_template_normalizes_the_first_run_to_day_of_period() {
        let d = tdb();
        let a = tpl(&d, json!({"startDate": "2026-01-15", "dayOfPeriod": 1})).unwrap();
        assert_eq!(a["next_run_date"].as_str(), Some("2026-02-01"));
        let b = tpl(&d, json!({"startDate": "2026-01-15", "dayOfPeriod": 20})).unwrap();
        assert_eq!(b["next_run_date"].as_str(), Some("2026-01-20"));
        let c = tpl(&d, json!({"startDate": "2026-01-25", "dayOfPeriod": 1})).unwrap();
        assert_eq!(c["next_run_date"].as_str(), Some("2026-02-01"));
    }

    #[test]
    fn run_due_books_one_entry_per_period_on_schedule() {
        let d = tdb();
        tpl_ok(&d);
        let r = run_due(&d, Some("2026-03-15"), None, "agent:test", false).unwrap();
        assert_eq!(r["templates"].as_array().unwrap().len(), 1);
        assert_eq!(r["templates"][0]["runs"].as_array().unwrap().len(), 3);
        let t = get_template(&d, 1).unwrap().unwrap();
        assert_eq!(t["runs_done"].as_i64(), Some(3));
        assert_eq!(t["next_run_date"].as_str(), Some("2026-04-01"));
        assert_eq!(t["status"].as_str(), Some("active"));
        let mut stmt = d
            .prepare("SELECT date, source_ref, created_by FROM journal_entries WHERE source = 'recurring' ORDER BY date")
            .unwrap();
        let rows: Vec<(String, String, String)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        let dates: Vec<&str> = rows.iter().map(|r| r.0.as_str()).collect();
        assert_eq!(dates, vec!["2026-01-01", "2026-02-01", "2026-03-01"]);
        assert_eq!(rows[0].1, "tpl:1");
        assert_eq!(rows[0].2, "recurring");
    }

    #[test]
    fn run_due_is_idempotent() {
        let d = tdb();
        tpl_ok(&d);
        run_due(&d, Some("2026-01-31"), None, "agent:test", false).unwrap();
        let again = run_due(&d, Some("2026-01-31"), None, "agent:test", false).unwrap();
        assert_eq!(again["templates"].as_array().unwrap().len(), 0);
        assert_eq!(
            one(
                &d,
                "SELECT COUNT(*) FROM journal_entries WHERE source='recurring'"
            ),
            1
        );
    }

    #[test]
    fn run_due_completes_the_template_after_the_run_limit() {
        let d = tdb();
        tpl(&d, json!({"runs": 2})).unwrap();
        run_due(&d, Some("2026-06-30"), None, "agent:test", false).unwrap();
        let t = get_template(&d, 1).unwrap().unwrap();
        assert_eq!(t["runs_done"].as_i64(), Some(2));
        assert_eq!(t["status"].as_str(), Some("completed"));
        assert_eq!(
            one(
                &d,
                "SELECT COUNT(*) FROM journal_entries WHERE source='recurring'"
            ),
            2
        );
    }

    #[test]
    fn run_due_completes_the_template_at_end_date() {
        let d = tdb();
        tpl(&d, json!({"endDate": "2026-02-28"})).unwrap();
        run_due(&d, Some("2026-12-31"), None, "agent:test", false).unwrap();
        let t = get_template(&d, 1).unwrap().unwrap();
        assert_eq!(t["status"].as_str(), Some("completed"));
        assert_eq!(t["runs_done"].as_i64(), Some(2));
    }

    #[test]
    fn run_due_skips_paused_templates() {
        let d = tdb();
        tpl_ok(&d);
        set_template_status(&d, 1, "paused", "agent:test", false).unwrap();
        let r = run_due(&d, Some("2026-03-01"), None, "agent:test", false).unwrap();
        assert_eq!(r["templates"].as_array().unwrap().len(), 0);
        assert_eq!(
            one(
                &d,
                "SELECT COUNT(*) FROM journal_entries WHERE source='recurring'"
            ),
            0
        );
    }

    #[test]
    fn run_due_with_a_template_id_runs_only_that_template() {
        let d = tdb();
        tpl_ok(&d);
        tpl(
            &d,
            json!({"name": "Verzekering", "postings": ["4320:100.00,1700:-100.00"]}),
        )
        .unwrap();
        let r = run_due(&d, Some("2026-02-01"), Some(2), "agent:test", false).unwrap();
        assert_eq!(r["templates"].as_array().unwrap().len(), 1);
        assert_eq!(r["templates"][0]["template_id"].as_i64(), Some(2));
        assert_eq!(
            one(
                &d,
                "SELECT COUNT(*) FROM journal_entries WHERE source='recurring'"
            ),
            2
        );
    }

    #[test]
    fn run_due_dry_run_writes_nothing() {
        let d = tdb();
        tpl_ok(&d);
        let r = run_due(&d, Some("2026-03-01"), None, "agent:test", true).unwrap();
        assert_eq!(r["templates"].as_array().unwrap().len(), 1);
        assert_eq!(r["templates"][0]["runs"].as_array().unwrap().len(), 3);
        assert_eq!(
            one(
                &d,
                "SELECT COUNT(*) FROM journal_entries WHERE source='recurring'"
            ),
            0
        );
    }

    #[test]
    fn reverse_previous_reverses_the_prior_entry_each_run() {
        let d = tdb();
        tpl(
            &d,
            json!({"name": "Nog te betalen kosten", "reversePrevious": true,
                   "postings": ["4310:250.00,2400:-250.00"]}),
        )
        .unwrap();
        run_due(&d, Some("2026-01-31"), None, "agent:test", false).unwrap();
        assert_eq!(
            one(
                &d,
                "SELECT COUNT(*) FROM journal_entries WHERE source='recurring'"
            ),
            1
        );
        let net = "SELECT SUM(amount_cents) FROM postings p JOIN journal_entries e ON e.id=p.entry_id WHERE e.state='posted' AND p.account_id=(SELECT id FROM accounts WHERE code='2400')";
        assert_eq!(one(&d, net), -25000);

        run_due(&d, Some("2026-02-28"), None, "agent:test", false).unwrap();
        assert_eq!(one(&d, "SELECT COUNT(*) FROM journal_entries"), 3);
        let ids: Vec<i64> = {
            let mut stmt = d
                .prepare("SELECT id FROM journal_entries ORDER BY id")
                .unwrap();
            let v: Vec<i64> = stmt
                .query_map([], |r| r.get(0))
                .unwrap()
                .filter_map(|r| r.ok())
                .collect();
            v
        };
        let reversal = entry_json(&d, ids[1]);
        assert_eq!(reversal["source"].as_str(), Some("reversal"));
        assert!(reversal["reversed_from_id"].as_i64().is_some());
        assert_eq!(reversal["state"].as_str(), Some("posted"));
        assert_eq!(one(&d, net), -25000);
    }

    #[test]
    fn reverse_previous_completed_chain_leaves_the_last_accrual_outstanding() {
        let d = tdb();
        tpl(
            &d,
            json!({"name": "Tijdelijke post", "reversePrevious": true, "runs": 3,
                   "postings": ["4310:100.00,2400:-100.00"]}),
        )
        .unwrap();
        run_due(&d, Some("2026-03-31"), None, "agent:test", false).unwrap();
        let net = "SELECT SUM(amount_cents) FROM postings p JOIN journal_entries e ON e.id=p.entry_id WHERE e.state='posted' AND p.account_id=(SELECT id FROM accounts WHERE code='2400')";
        assert_eq!(one(&d, net), -10000);
    }

    #[test]
    fn reverse_previous_dry_run_preview_mirrors_the_execute_shape() {
        let d = tdb();
        tpl(
            &d,
            json!({"name": "Nog te betalen kosten", "reversePrevious": true,
                   "postings": ["4310:250.00,2400:-250.00"]}),
        )
        .unwrap();
        run_due(&d, Some("2026-01-31"), None, "agent:test", false).unwrap();
        let preview = run_due(&d, Some("2026-02-28"), None, "agent:test", true).unwrap();
        let t = &preview["templates"][0];
        assert_eq!(t["runs"].as_array().unwrap().len(), 2);
        assert_eq!(t["runs"][0]["kind"].as_str(), Some("reversal"));
        assert_eq!(t["runs"][0]["entry"]["id"].as_i64(), Some(1));
        assert_eq!(t["runs"][1]["kind"].as_str(), Some("entry"));
        assert_eq!(t["runs"][1]["entry"]["date"].as_str(), Some("2026-02-01"));
        assert_eq!(
            one(
                &d,
                "SELECT COUNT(*) FROM journal_entries WHERE source='recurring'"
            ),
            1
        );
    }

    #[test]
    fn build_depreciation_template_adjusts_the_final_run() {
        let d = tdb();
        let r = build_depreciation_template(
            &d,
            "Laptop Dell",
            "1800",
            "4600",
            537000,
            0,
            36,
            "2026-01-01",
            None,
            "agent:test",
            false,
        )
        .unwrap();
        assert_eq!(r["monthly_cents"].as_i64(), Some(14917));
        assert_eq!(r["final_cents"].as_i64(), Some(14905));
        assert_eq!(r["total_cents"].as_i64(), Some(537000));
        let t = get_template(&d, r["template"]["id"].as_i64().unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(t["runs"].as_i64(), Some(36));
        assert_eq!(t["frequency"].as_str(), Some("monthly"));
        assert_eq!(t["postings"][0]["code"].as_str(), Some("4600"));
        assert_eq!(t["postings"][0]["amountCents"].as_i64(), Some(14917));
        assert_eq!(t["final_postings"][0]["amountCents"].as_i64(), Some(14905));

        run_due(&d, Some("2028-12-31"), None, "agent:test", false).unwrap();
        let asset = "SELECT SUM(amount_cents) FROM postings p JOIN journal_entries e ON e.id=p.entry_id WHERE e.state='posted' AND p.account_id=(SELECT id FROM accounts WHERE code='1800')";
        assert_eq!(one(&d, asset), -537000);
    }

    #[test]
    fn build_depreciation_template_validates() {
        let d = tdb();
        let e = build_depreciation_template(
            &d,
            "x",
            "1800",
            "4600",
            -5,
            0,
            36,
            "2026-01-01",
            None,
            "a:t",
            false,
        )
        .unwrap_err();
        assert_eq!(e.code, "INVALID_COST");
        let e = build_depreciation_template(
            &d,
            "x",
            "1800",
            "4600",
            1000,
            2000,
            36,
            "2026-01-01",
            None,
            "a:t",
            false,
        )
        .unwrap_err();
        assert_eq!(e.code, "INVALID_RESIDUAL");
        let e = build_depreciation_template(
            &d,
            "x",
            "1800",
            "4600",
            1000,
            0,
            1,
            "2026-01-01",
            None,
            "a:t",
            false,
        )
        .unwrap_err();
        assert_eq!(e.code, "INVALID_LIFE");
    }

    #[test]
    fn vat_aware_template_stores_the_expansion_and_replays_it() {
        let d = tdb();
        crate::vat::enable_vat_module(&d, "agent:test").unwrap();
        let t = tpl(
            &d,
            json!({"name": "Abonnement", "postings": ["4340:100.00@21,1100:-121.00"]}),
        )
        .unwrap();
        assert_eq!(t["vat_aware"].as_i64(), Some(1));
        assert_eq!(t["postings"].as_array().unwrap().len(), 3);
        run_due(&d, Some("2026-01-31"), None, "agent:test", false).unwrap();
        let entry = entry_json(&d, 1);
        let posts = entry["postings"].as_array().unwrap();
        let net = posts
            .iter()
            .find(|p| p["account_code"] == "4340")
            .expect("4340 posting");
        assert_eq!(net["vat_amount_cents"].as_i64(), Some(2100));
        let vat_leg = posts
            .iter()
            .find(|p| p["account_code"] == "1500")
            .expect("1500 posting");
        assert_eq!(vat_leg["amount_cents"].as_i64(), Some(2100));
    }

    #[test]
    fn vat_aware_template_requires_the_vat_module() {
        let d = tdb();
        let e = tpl(&d, json!({"postings": ["4340:100.00@21,1100:-121.00"]})).unwrap_err();
        assert_eq!(e.code, "VAT_MODULE_OFF");
    }

    #[test]
    fn vat_aware_keeps_object_postings_mixed_into_a_tagged_list() {
        let d = tdb();
        crate::vat::enable_vat_module(&d, "agent:test").unwrap();
        let t = tpl(
            &d,
            json!({"name": "Gemengd", "postings": ["4340:100.00@21", {"code": "1100", "amountCents": -12100}]}),
        )
        .unwrap();
        assert_eq!(t["vat_aware"].as_i64(), Some(1));
        let codes: Vec<String> = t["postings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["code"].as_str().unwrap_or("").to_string())
            .collect();
        assert!(
            codes.contains(&"4340".to_string()),
            "string posting expanded"
        );
        assert!(codes.contains(&"1500".to_string()), "VAT leg added");
        assert!(codes.contains(&"1100".to_string()), "object posting kept");
        let sum: i64 = t["postings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["amountCents"].as_i64().unwrap_or(0))
            .sum();
        assert_eq!(sum, 0);
        run_due(&d, Some("2026-01-31"), None, "agent:test", false).unwrap();
        let entry = entry_json(&d, 1);
        assert_eq!(entry["state"].as_str(), Some("posted"));
        let esum: i64 = entry["postings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["amount_cents"].as_i64().unwrap_or(0))
            .sum();
        assert_eq!(esum, 0);
    }

    #[test]
    fn preview_due_is_read_only_and_matches_run_due() {
        let d = tdb();
        tpl_ok(&d);
        let plan = preview_due(&d, Some("2026-02-15"), None).unwrap();
        assert_eq!(plan["templates"][0]["runs"].as_array().unwrap().len(), 2);
        assert_eq!(
            one(
                &d,
                "SELECT COUNT(*) FROM journal_entries WHERE source='recurring'"
            ),
            0
        );
    }

    #[test]
    fn list_templates_filters_by_status() {
        let d = tdb();
        tpl_ok(&d);
        tpl(&d, json!({"name": "B", "runs": 1})).unwrap();
        run_due(&d, Some("2026-01-31"), None, "agent:test", false).unwrap();
        assert_eq!(list_templates(&d, "active").unwrap().len(), 1);
        assert_eq!(list_templates(&d, "completed").unwrap().len(), 1);
        assert_eq!(list_templates(&d, "all").unwrap().len(), 2);
    }

    #[test]
    fn generated_entries_are_posted_and_the_books_stay_balanced() {
        let d = tdb();
        tpl_ok(&d);
        run_due(&d, Some("2026-01-31"), None, "agent:test", false).unwrap();
        let entry = entry_json(&d, 1);
        assert_eq!(entry["state"].as_str(), Some("posted"));
        assert_eq!(
            one(&d, "SELECT COALESCE(SUM(amount_cents),0) FROM postings"),
            0
        );
    }
}

// ==== ported from test/recurring-invoice.test.js ============================
#[cfg(test)]
mod recurring_invoice_tests {
    use super::*;
    use rusqlite::Connection;
    use serde_json::{json, Value};
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    /// Peppol tests read/write process env; tests run in threads, so serialise.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn idb() -> Connection {
        let d = crate::db::open_db(":memory:").unwrap();
        crate::accounts::seed_default_chart(&d).unwrap();
        d.execute(
            "INSERT INTO company (id, name, registration_id, legal_form, tax_id, iban, address, postal_code, city, vat_module)
             VALUES (1,'Demo BV','12345678','bv','NL123456789B01','NL91ABNA0417164300','Industrieweg 12','2712 CD','Zoetermeer',1)",
            [],
        )
        .unwrap();
        crate::vat::enable_vat_module(&d, "agent:test").unwrap();
        d
    }

    fn one_i(db: &Connection, sql: &str) -> i64 {
        db.query_row(sql, [], |r| r.get::<_, Option<i64>>(0))
            .unwrap()
            .unwrap_or(0)
    }

    fn acme(db: &Connection) -> Value {
        crate::contacts::create_contact(
            db,
            "ACME B.V.",
            Some("Straat 1"),
            Some("1000 AA"),
            Some("Amsterdam"),
            None,
            None,
            Some("NL999999999B01"),
            Some("98765432"),
            None,
            "agent:test",
            false,
        )
        .unwrap()
    }

    /// The JS test's createTemplate(db, {kind:'invoice', contactId, invoiceLines,
    /// frequency, startDate, dueDays, runs, actor}) helper.
    fn itpl(db: &Connection, over: Value) -> Result<Value> {
        let mut o = json!({
            "name": "SaaS abonnement", "frequency": "monthly", "startDate": "2026-08-01",
            "invoiceLines": ["2x Premium @ 99.00 @21"],
        });
        if let Some(m) = over.as_object() {
            for (k, v) in m {
                o[k] = v.clone();
            }
        }
        let spec = json!({
            "contact_id": o["contactId"],
            "lines": o["invoiceLines"],
            "items": o["items"],
            "due_days": o["dueDays"],
        })
        .to_string();
        create_template(
            db,
            o["name"].as_str().unwrap(),
            None,
            o["frequency"].as_str().unwrap(),
            1,
            o["startDate"].as_str().unwrap(),
            o["endDate"].as_str(),
            o["runs"].as_i64(),
            &spec,
            o["reversePrevious"].as_bool().unwrap_or(false),
            "agent:test",
            "invoice",
            false,
        )
    }

    fn invoice_id(db: &Connection) -> i64 {
        db.query_row("SELECT MIN(id) FROM invoices", [], |r| r.get(0))
            .unwrap()
    }

    /// A one-shot HTTP server: answers with `status`, captures the raw request.
    fn mock_server(status: u16, ctype: &str, body: &str) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let captured: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = captured.clone();
        let body = body.to_string();
        let ctype = ctype.to_string();
        std::thread::spawn(move || {
            if let Ok((mut sock, _)) = listener.accept() {
                let _ = sock.set_read_timeout(Some(std::time::Duration::from_secs(10)));
                let mut data: Vec<u8> = Vec::new();
                let mut buf = [0u8; 8192];
                // read headers, then exactly Content-Length bytes of body
                while !data.windows(4).any(|w| w == b"\r\n\r\n") {
                    match sock.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => data.extend_from_slice(&buf[..n]),
                    }
                }
                let head_end = data
                    .windows(4)
                    .position(|w| w == b"\r\n\r\n")
                    .map(|i| i + 4)
                    .unwrap_or(data.len());
                let head = String::from_utf8_lossy(&data[..head_end]).to_lowercase();
                let clen: usize = head
                    .split("content-length:")
                    .nth(1)
                    .and_then(|s| s.split(['\r', '\n']).next())
                    .and_then(|s| s.trim().parse().ok())
                    .unwrap_or(0);
                while data.len() < head_end + clen {
                    match sock.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => data.extend_from_slice(&buf[..n]),
                    }
                }
                sink.lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&data).to_string());
                let resp = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = sock.write_all(resp.as_bytes());
                let _ = sock.flush();
            }
        });
        (format!("http://127.0.0.1:{port}"), captured)
    }

    fn set_env(k: &str, v: Option<&str>) {
        match v {
            Some(x) => std::env::set_var(k, x),
            None => std::env::remove_var(k),
        }
    }

    #[test]
    fn invoice_template_generates_drafts_on_schedule() {
        let d = idb();
        let c = acme(&d);
        let t = itpl(&d, json!({"contactId": c["id"], "dueDays": 14})).unwrap();
        assert_eq!(t["kind"].as_str(), Some("invoice"));
        assert_eq!(t["vat_aware"].as_i64(), Some(1));

        let result = run_due(&d, Some("2026-09-30"), None, "agent:test", false).unwrap();
        assert_eq!(result["templates"].as_array().unwrap().len(), 1);
        // Aug 1 + Sep 1 (backfill)
        assert_eq!(result["templates"][0]["runs"].as_array().unwrap().len(), 2);

        let dates: Vec<String> = d
            .prepare("SELECT date FROM invoices ORDER BY id")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .map(|x| x.unwrap())
            .collect();
        assert_eq!(dates, vec!["2026-08-01", "2026-09-01"]);
        // drafts only — nothing booked, no numbers
        assert_eq!(
            one_i(&d, "SELECT COUNT(*) FROM invoices WHERE status='draft'"),
            2
        );
        assert_eq!(
            one_i(
                &d,
                "SELECT COUNT(*) FROM invoices WHERE invoice_number IS NULL"
            ),
            2
        );
        assert_eq!(
            one_i(
                &d,
                "SELECT COUNT(*) FROM journal_entries WHERE source='invoice'"
            ),
            0
        );

        // lines + due date carried over
        let inv = crate::invoice::get_invoice(&d, invoice_id(&d))
            .unwrap()
            .unwrap();
        assert_eq!(inv["lines"][0]["description"].as_str(), Some("Premium"));
        assert_eq!(inv["lines"][0]["quantity"].as_i64(), Some(2000)); // milli-units
        assert_eq!(inv["lines"][0]["vat_amount_cents"].as_i64(), Some(4158)); // 198.00 @21%
        assert_eq!(inv["due_date"].as_str(), Some("2026-08-15"));
        assert_eq!(inv["contact"]["name"].as_str(), Some("ACME B.V."));
    }

    #[test]
    fn invoice_template_generated_drafts_finalize_normally() {
        let d = idb();
        let c = acme(&d);
        let today = crate::dates::today_iso();
        let y = today[..4].to_string();
        let m = today[5..7].to_string();
        // due date = start + 14 days; run on the 10th so the invoice is not overdue
        itpl(
            &d,
            json!({
                "contactId": c["id"],
                "invoiceLines": ["2x Premium @ 99.00 @21"],
                "startDate": format!("{y}-{m}-01"),
            }),
        )
        .unwrap();
        run_due(&d, Some(&format!("{y}-{m}-10")), None, "agent:test", false).unwrap();
        let draft_id = invoice_id(&d);
        let result = crate::invoice::finalize_invoice(&d, draft_id, "agent:test", false).unwrap();
        assert_eq!(
            result["invoice"]["invoice_number"].as_str(),
            Some(format!("{y}-0001").as_str())
        );
        assert_eq!(result["invoice"]["status"].as_str(), Some("sent"));
        assert_eq!(result["entry"]["state"].as_str(), Some("posted"));
    }

    #[test]
    fn invoice_template_guards() {
        let d = idb();
        acme(&d);
        // reverse-previous is entry-only
        let e = itpl(&d, json!({"contactId": 1, "reversePrevious": true})).unwrap_err();
        assert_eq!(e.code, "INVALID_REVERSE");
        // contact required + must exist
        let e = itpl(&d, json!({"contactId": Value::Null})).unwrap_err();
        assert_eq!(e.code, "INVALID_KIND");
        let e = itpl(&d, json!({"contactId": 99})).unwrap_err();
        assert_eq!(e.code, "CONTACT_NOT_FOUND");
        // unknown vat code at creation
        let e = itpl(
            &d,
            json!({"contactId": 1, "invoiceLines": ["1x A @ 10.00 @77"]}),
        )
        .unwrap_err();
        assert_eq!(e.code, "VAT_CODE_NOT_FOUND");
        // bad kind
        let e = create_template(
            &d,
            "x",
            None,
            "monthly",
            1,
            "2026-08-01",
            None,
            None,
            &json!({"lines": ["1100:1,3000:-1"]}).to_string(),
            false,
            "agent:test",
            "ledger",
            false,
        )
        .unwrap_err();
        assert_eq!(e.code, "INVALID_KIND");
    }

    #[test]
    fn invoice_template_keeps_entry_templates_working_alongside() {
        let d = idb();
        let c = acme(&d);
        itpl(
            &d,
            json!({"contactId": c["id"], "invoiceLines": ["2x Premium @ 99.00 @21"]}),
        )
        .unwrap();
        create_template(
            &d,
            "Huur kantoor",
            None,
            "monthly",
            1,
            "2026-08-01",
            None,
            None,
            &json!(["4300:1000.00,1100:-1000.00"]).to_string(),
            false,
            "agent:test",
            "entry",
            false,
        )
        .unwrap();
        let result = run_due(&d, Some("2026-08-31"), None, "agent:test", false).unwrap();
        assert_eq!(result["templates"].as_array().unwrap().len(), 2);
        let tpls = result["templates"].as_array().unwrap();
        let find = |n: &str| {
            tpls.iter()
                .find(|t| t["name"].as_str() == Some(n))
                .unwrap()
                .clone()
        };
        assert_eq!(
            find("SaaS abonnement")["runs"][0]["generated"][0]["kind"].as_str(),
            Some("invoice")
        );
        assert_eq!(
            find("Huur kantoor")["runs"][0]["generated"][0]["kind"].as_str(),
            Some("entry")
        );
        assert_eq!(one_i(&d, "SELECT COUNT(*) FROM invoices"), 1);
        assert_eq!(
            one_i(
                &d,
                "SELECT COUNT(*) FROM journal_entries WHERE source='recurring'"
            ),
            1
        );
    }

    #[test]
    fn invoice_template_dry_run_shows_the_plan_and_writes_nothing() {
        let d = idb();
        let c = acme(&d);
        itpl(&d, json!({"contactId": c["id"], "invoiceLines": ["2x Premium @ 99.00 @21"], "dueDays": 14}))
            .unwrap();
        let result = run_due(&d, Some("2026-08-31"), None, "agent:test", true).unwrap();
        let run = &result["templates"][0]["runs"][0];
        assert_eq!(run["kind"].as_str(), Some("invoice"));
        assert_eq!(run["invoice"]["date"].as_str(), Some("2026-08-01"));
        assert_eq!(run["invoice"]["due_date"].as_str(), Some("2026-08-15"));
        assert_eq!(run["invoice"]["contact_name"].as_str(), Some("ACME B.V."));
        assert_eq!(one_i(&d, "SELECT COUNT(*) FROM invoices"), 0);
    }

    #[test]
    fn invoice_template_runs_limit_completes_the_template() {
        let d = idb();
        acme(&d);
        itpl(
            &d,
            json!({
                "name": "Abonnement Q", "contactId": 1, "invoiceLines": ["2x Premium @ 99.00 @21"],
                "frequency": "quarterly", "startDate": "2026-07-01", "runs": 2,
            }),
        )
        .unwrap();
        run_due(&d, Some("2026-12-31"), None, "agent:test", false).unwrap();
        assert_eq!(one_i(&d, "SELECT COUNT(*) FROM invoices"), 2);
        let t = get_template(&d, 1).unwrap().unwrap();
        assert_eq!(t["status"].as_str(), Some("completed"));
        assert_eq!(t["runs_done"].as_i64(), Some(2));
    }

    #[test]
    fn peppol_send_posts_the_ubl_to_the_provider() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let d = idb();
        let c = acme(&d);
        let inv = crate::invoice::create_invoice(
            &d,
            c["id"].as_i64().unwrap(),
            "2026-08-01",
            None,
            None,
            Some("PO-2026-001"),
            None,
            None,
            None,
            None,
            &[json!("1x Premium @ 99.00 @21")],
            "agent:test",
            false,
        )
        .unwrap();
        crate::invoice::finalize_invoice(&d, inv["id"].as_i64().unwrap(), "agent:test", false)
            .unwrap();

        let (url, captured) = mock_server(202, "application/json", "{\"ok\":true}");
        let old_ep = std::env::var("BUKIO_PEPPOL_ENDPOINT").ok();
        let old_tok = std::env::var("BUKIO_PEPPOL_TOKEN").ok();
        set_env("BUKIO_PEPPOL_ENDPOINT", Some(&format!("{url}/invoices")));
        set_env("BUKIO_PEPPOL_TOKEN", Some("test-token-123"));
        let invoice = crate::invoice::get_invoice(&d, inv["id"].as_i64().unwrap())
            .unwrap()
            .unwrap();
        let result = crate::peppol::send_peppol_invoice(&d, &invoice, None, false);
        set_env("BUKIO_PEPPOL_ENDPOINT", old_ep.as_deref());
        set_env("BUKIO_PEPPOL_TOKEN", old_tok.as_deref());
        let result = result.unwrap();
        assert_eq!(result["status"].as_i64(), Some(202));
        assert_eq!(result["invoice_number"].as_str(), Some("2026-0001"));

        let reqs = captured.lock().unwrap();
        assert_eq!(reqs.len(), 1);
        let raw = &reqs[0];
        assert!(raw.starts_with("POST /invoices"), "request line: {raw}");
        assert!(raw.contains("Bearer test-token-123"), "auth header: {raw}");
        let lower = raw.to_lowercase();
        assert!(
            lower.contains("content-type: application/xml"),
            "ctype: {raw}"
        );
        assert!(
            raw.contains("<cbc:ID>2026-0001</cbc:ID>"),
            "ubl body: {raw}"
        );
    }

    #[test]
    fn peppol_send_not_configured_provider_error_and_dry_run() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let d = idb();
        let c = acme(&d);
        let inv = crate::invoice::create_invoice(
            &d,
            c["id"].as_i64().unwrap(),
            "2026-08-01",
            None,
            None,
            Some("REF-1"),
            None,
            None,
            None,
            None,
            &[json!("1x A @ 10.00 @21")],
            "agent:test",
            false,
        )
        .unwrap();
        crate::invoice::finalize_invoice(&d, inv["id"].as_i64().unwrap(), "agent:test", false)
            .unwrap();
        let invoice = crate::invoice::get_invoice(&d, inv["id"].as_i64().unwrap())
            .unwrap()
            .unwrap();

        let old_ep = std::env::var("BUKIO_PEPPOL_ENDPOINT").ok();
        let old_tok = std::env::var("BUKIO_PEPPOL_TOKEN").ok();
        set_env("BUKIO_PEPPOL_ENDPOINT", None);
        set_env("BUKIO_PEPPOL_TOKEN", None);
        let e = crate::peppol::send_peppol_invoice(&d, &invoice, None, false).unwrap_err();
        assert_eq!(e.code, "PEPPOL_NOT_CONFIGURED");

        // provider returns 500
        let (url, _cap) = mock_server(500, "text/plain", "provider exploded");
        set_env("BUKIO_PEPPOL_ENDPOINT", Some(&url));
        set_env("BUKIO_PEPPOL_TOKEN", Some("tok"));
        let e = crate::peppol::send_peppol_invoice(&d, &invoice, None, false).unwrap_err();
        assert_eq!(e.code, "PEPPOL_SEND_FAILED");
        assert!(e.message.contains("500"), "message: {}", e.message);
        // dry-run: no request, reports config
        let plan = crate::peppol::send_peppol_invoice(&d, &invoice, None, true).unwrap();
        assert_eq!(plan["dryRun"], json!(true));
        assert_eq!(plan["configured"], json!(true));
        assert!(plan["bytes"].as_i64().unwrap_or(0) > 0);

        set_env("BUKIO_PEPPOL_ENDPOINT", old_ep.as_deref());
        set_env("BUKIO_PEPPOL_TOKEN", old_tok.as_deref());
    }

    #[test]
    fn peppol_buyer_without_kvk_is_rejected_up_front() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let d = idb();
        let c = crate::contacts::create_contact(
            &d,
            "Geen KVK B.V.",
            Some("Straat 1"),
            Some("1000 AA"),
            Some("Amsterdam"),
            None,
            None,
            Some("NL888888888B01"),
            None, // no kvk
            None,
            "agent:test",
            false,
        )
        .unwrap();
        let inv = crate::invoice::create_invoice(
            &d,
            c["id"].as_i64().unwrap(),
            "2026-08-01",
            None,
            None,
            Some("REF-1"),
            None,
            None,
            None,
            None,
            &[json!("1x A @ 10.00 @21")],
            "agent:test",
            false,
        )
        .unwrap();
        crate::invoice::finalize_invoice(&d, inv["id"].as_i64().unwrap(), "agent:test", false)
            .unwrap();
        let invoice = crate::invoice::get_invoice(&d, inv["id"].as_i64().unwrap())
            .unwrap()
            .unwrap();
        let old_ep = std::env::var("BUKIO_PEPPOL_ENDPOINT").ok();
        let old_tok = std::env::var("BUKIO_PEPPOL_TOKEN").ok();
        set_env("BUKIO_PEPPOL_ENDPOINT", Some("http://127.0.0.1:9"));
        set_env("BUKIO_PEPPOL_TOKEN", Some("tok"));
        // fails BEFORE any network call — even dry-run validates like execute
        for dry in [false, true] {
            let e = crate::peppol::send_peppol_invoice(&d, &invoice, None, dry).unwrap_err();
            assert_eq!(e.code, "PEPPOL_BUYER_MISSING_ID");
        }
        set_env("BUKIO_PEPPOL_ENDPOINT", old_ep.as_deref());
        set_env("BUKIO_PEPPOL_TOKEN", old_tok.as_deref());
    }

    #[test]
    fn peppol_invoice_without_buyer_reference_is_rejected_up_front() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let d = idb();
        let c = acme(&d); // has kvk via fixture
        let inv = crate::invoice::create_invoice(
            &d,
            c["id"].as_i64().unwrap(),
            "2026-08-01",
            None,
            None,
            None, // no reference
            None,
            None,
            None,
            None,
            &[json!("1x A @ 10.00 @21")],
            "agent:test",
            false,
        )
        .unwrap();
        crate::invoice::finalize_invoice(&d, inv["id"].as_i64().unwrap(), "agent:test", false)
            .unwrap();
        let invoice = crate::invoice::get_invoice(&d, inv["id"].as_i64().unwrap())
            .unwrap()
            .unwrap();
        let old_ep = std::env::var("BUKIO_PEPPOL_ENDPOINT").ok();
        let old_tok = std::env::var("BUKIO_PEPPOL_TOKEN").ok();
        set_env("BUKIO_PEPPOL_ENDPOINT", Some("http://127.0.0.1:9"));
        set_env("BUKIO_PEPPOL_TOKEN", Some("tok"));
        for dry in [false, true] {
            let e = crate::peppol::send_peppol_invoice(&d, &invoice, None, dry).unwrap_err();
            assert_eq!(e.code, "PEPPOL_BUYER_REFERENCE_MISSING");
        }
        set_env("BUKIO_PEPPOL_ENDPOINT", old_ep.as_deref());
        set_env("BUKIO_PEPPOL_TOKEN", old_tok.as_deref());
    }

    #[test]
    fn ubl_buyer_reference_follows_document_currency_code() {
        let d = idb();
        let c = acme(&d);
        let inv = crate::invoice::create_invoice(
            &d,
            c["id"].as_i64().unwrap(),
            "2026-08-01",
            None,
            None,
            Some("PO-2026-007"),
            None,
            None,
            None,
            None,
            &[json!("1x A @ 10.00 @21")],
            "agent:test",
            false,
        )
        .unwrap();
        crate::invoice::finalize_invoice(&d, inv["id"].as_i64().unwrap(), "agent:test", false)
            .unwrap();
        let invoice = crate::invoice::get_invoice(&d, inv["id"].as_i64().unwrap())
            .unwrap()
            .unwrap();
        let xml = crate::ubl::invoice_to_ubl(&d, &invoice).unwrap();
        assert!(xml.contains("<cbc:BuyerReference>PO-2026-007</cbc:BuyerReference>"));
        let doc_idx = xml.find("<cbc:DocumentCurrencyCode>").unwrap();
        let ref_idx = xml.find("<cbc:BuyerReference>").unwrap();
        assert!(
            ref_idx > doc_idx,
            "BuyerReference must follow DocumentCurrencyCode"
        );
    }
}
