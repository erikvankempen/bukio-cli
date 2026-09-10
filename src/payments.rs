// bukio-cli — agent-first double-entry bookkeeping for SMEs.
// Copyright (c) 2026 Erik van Kempen.
// SPDX-License-Identifier: Apache-2.0
//
// Payment batches — SEPA credit transfer (pain.001) + direct debit (pain.008).

use crate::audit::{record, RecordArgs};
use crate::contacts::{get_contact, list_contacts};
use crate::dates::is_valid_iban;
use crate::money::{BukioError, Result};
use rusqlite::Connection;
use serde_json::{json, Value};

fn payments_error(code: &'static str, msg: impl Into<String>) -> BukioError {
    BukioError::new(code, msg.into())
}

fn sql_err(e: rusqlite::Error) -> BukioError {
    BukioError::new("DB_ERROR", e.to_string())
}

fn valid_date(s: &str) -> bool {
    if s.len() != 10 || s.as_bytes()[4] != b'-' || s.as_bytes()[7] != b'-' {
        return false;
    }
    chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok()
}

pub fn resolve_contact(db: &Connection, ref_val: &str) -> Result<Option<Value>> {
    if let Ok(id) = ref_val.parse::<i64>() {
        if let Some(c) = get_contact(db, id)? {
            return Ok(Some(c));
        }
    }
    let name = ref_val.trim().to_lowercase();
    Ok(list_contacts(db)?
        .into_iter()
        .find(|c| c["name"].as_str().unwrap_or("").trim().to_lowercase() == name))
}

fn normalize_iban(iban: &str) -> String {
    iban.replace([' ', '-'], "")
}

fn get_company(db: &Connection) -> Result<Option<Value>> {
    let r = db.query_row("SELECT * FROM company WHERE id = 1", [], |r| {
        Ok(json!({
            "id": r.get::<_, i64>(0)?, "name": r.get::<_, Option<String>>(1)?,
            "registration_id": r.get::<_, Option<String>>(2)?,
            "legal_form": r.get::<_, Option<String>>(3)?,
            "tax_id": r.get::<_, Option<String>>(4)?,
            "iban": r.get::<_, Option<String>>(5)?,
        }))
    });
    match r {
        Ok(v) => Ok(Some(v)),
        Err(_) => Ok(None),
    }
}

// --- Mandates ---------------------------------------------------------------

pub fn add_mandate(
    db: &Connection,
    contact_id: i64,
    mandate_ref: &str,
    mandate_date: Option<&str>,
    scheme: &str,
    actor: &str,
    dry_run: bool,
) -> Result<Value> {
    if contact_id <= 0 {
        return Err(payments_error(
            "CONTACT_NOT_FOUND",
            "a contact id is required",
        ));
    }
    let contact = get_contact(db, contact_id)?.ok_or_else(|| {
        payments_error(
            "CONTACT_NOT_FOUND",
            format!("contact {contact_id} does not exist"),
        )
    })?;
    let ref_trimmed = mandate_ref.trim();
    if ref_trimmed.is_empty() {
        return Err(payments_error(
            "INVALID_MANDATE_REF",
            "a mandate reference is required",
        ));
    }
    if ref_trimmed.len() > 35 {
        return Err(payments_error(
            "INVALID_MANDATE_REF",
            "mandate reference max 35 characters",
        ));
    }
    let today_default = crate::dates::today_iso();
    let date = mandate_date.unwrap_or(&today_default);
    if !valid_date(date) {
        return Err(payments_error(
            "INVALID_DATE",
            format!("mandate date '{date}' must be YYYY-MM-DD"),
        ));
    }
    if !["core", "b2b"].contains(&scheme) {
        return Err(payments_error(
            "INVALID_SCHEME",
            "mandate scheme must be 'core' or 'b2b'",
        ));
    }

    let dup = db
        .query_row(
            "SELECT id FROM sepa_mandates WHERE contact_id = ?1 AND mandate_ref = ?2",
            rusqlite::params![contact_id, ref_trimmed],
            |r| r.get::<_, i64>(0),
        )
        .ok();
    if let Some(id) = dup {
        return Err(payments_error(
            "MANDATE_DUPLICATE",
            format!(
                "contact {} already has mandate '{}' (id {id})",
                contact["name"], ref_trimmed
            ),
        ));
    }

    if dry_run {
        return Ok(
            json!({"action": "payments.mandate.add", "contact_id": contact_id, "mandate_ref": ref_trimmed, "dryRun": true}),
        );
    }

    db.execute("INSERT INTO sepa_mandates (contact_id, mandate_ref, mandate_date, scheme, created_by) VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![contact_id, ref_trimmed, date, scheme, actor]).map_err(sql_err)?;
    let id = db.last_insert_rowid();
    record(
        db,
        RecordArgs {
            actor,
            action: "payments.mandate.add",
            command: Some("payments mandate add"),
            args: Some(json!({"mandate_id": id})),
            outcome: "ok",
            entry_ids: vec![],
        },
    )?;
    // the JS returns the stored mandate row (contact_name + mandate_date
    // included), not a bare insert id — reuse the list query
    Ok(list_mandates(db, Some(contact_id))?
        .into_iter()
        .find(|m| m["id"] == json!(id))
        .unwrap_or_else(|| {
            json!({"id": id, "contact_id": contact_id, "mandate_ref": ref_trimmed, "scheme": scheme})
        }))
}

pub fn list_mandates(db: &Connection, contact_id: Option<i64>) -> Result<Vec<Value>> {
    let base = "SELECT m.id, m.contact_id, c.name AS contact_name, m.mandate_ref, m.mandate_date, m.scheme FROM sepa_mandates m JOIN contacts c ON c.id = m.contact_id";
    let (sql, has_filter) = if contact_id.is_some() {
        (
            format!("{base} WHERE m.contact_id = ?1 ORDER BY m.id"),
            true,
        )
    } else {
        (format!("{base} ORDER BY m.id"), false)
    };
    let mut stmt = db.prepare(&sql).map_err(sql_err)?;
    let rows = if has_filter {
        let cid = contact_id.unwrap();
        stmt.query_map([cid], |r| {
            Ok(json!({"id": r.get::<_, i64>(0)?, "contact_id": r.get::<_, i64>(1)?, "contact_name": r.get::<_, String>(2)?, "mandate_ref": r.get::<_, String>(3)?, "mandate_date": r.get::<_, String>(4)?, "scheme": r.get::<_, String>(5)?}))
        }).map_err(sql_err)?.filter_map(|r| r.ok()).collect()
    } else {
        stmt.query_map([], |r| {
            Ok(json!({"id": r.get::<_, i64>(0)?, "contact_id": r.get::<_, i64>(1)?, "contact_name": r.get::<_, String>(2)?, "mandate_ref": r.get::<_, String>(3)?, "mandate_date": r.get::<_, String>(4)?, "scheme": r.get::<_, String>(5)?}))
        }).map_err(sql_err)?.filter_map(|r| r.ok()).collect()
    };
    Ok(rows)
}

pub fn remove_mandate(db: &Connection, id: i64, actor: &str, dry_run: bool) -> Result<Value> {
    let mandate = db
        .query_row("SELECT * FROM sepa_mandates WHERE id = ?1", [id], |r| {
            r.get::<_, i64>(0)
        })
        .map_err(sql_err);
    if mandate.is_err() {
        return Err(payments_error(
            "MANDATE_NOT_FOUND",
            format!("mandate {id} does not exist"),
        ));
    }
    if dry_run {
        return Ok(json!({"action": "payments.mandate.remove", "mandate_id": id, "dryRun": true}));
    }
    db.execute("DELETE FROM sepa_mandates WHERE id = ?1", [id])
        .map_err(sql_err)?;
    record(
        db,
        RecordArgs {
            actor,
            action: "payments.mandate.remove",
            command: Some("payments mandate remove"),
            args: Some(json!({"mandate_id": id})),
            outcome: "ok",
            entry_ids: vec![],
        },
    )?;
    Ok(json!({"mandate_id": id, "status": "deleted"}))
}

fn latest_mandate(db: &Connection, contact_id: i64) -> Option<Value> {
    db.query_row("SELECT id, contact_id, mandate_ref, mandate_date, scheme FROM sepa_mandates WHERE contact_id = ?1 ORDER BY id DESC LIMIT 1", [contact_id], |r| {
        Ok(json!({"id": r.get::<_, i64>(0)?, "contact_id": r.get::<_, i64>(1)?, "mandate_ref": r.get::<_, String>(2)?, "mandate_date": r.get::<_, String>(3)?, "scheme": r.get::<_, String>(4)?}))
    }).ok()
}

// --- Payables ---------------------------------------------------------------

pub fn add_payable(
    db: &Connection,
    contact_ref: &str,
    invoice_ref: &str,
    date: &str,
    due_date: Option<&str>,
    amount_cents: i64,
    method: &str,
    actor: &str,
    dry_run: bool,
) -> Result<Value> {
    let c = resolve_contact(db, contact_ref)?.ok_or_else(|| {
        payments_error(
            "CONTACT_NOT_FOUND",
            format!("contact '{contact_ref}' does not exist"),
        )
    })?;
    let c_id = c["id"].as_i64().unwrap_or(0);
    if amount_cents <= 0 {
        return Err(payments_error("INVALID_AMOUNT", "amount must be positive"));
    }
    let ref_trimmed = invoice_ref.trim();
    if ref_trimmed.is_empty() {
        return Err(payments_error(
            "INVOICE_REF_REQUIRED",
            "invoice reference is required",
        ));
    }
    if !["transfer", "direct_debit"].contains(&method) {
        return Err(payments_error(
            "INVALID_METHOD",
            "payment method must be 'transfer' or 'direct_debit'",
        ));
    }
    if !valid_date(date) {
        return Err(payments_error(
            "INVALID_DATE",
            format!("date '{date}' must be yyyy-mm-dd"),
        ));
    }
    if let Some(dd) = due_date {
        if !valid_date(dd) {
            return Err(payments_error(
                "INVALID_DATE",
                format!("due date '{dd}' must be yyyy-mm-dd"),
            ));
        }
    }

    let dup = db.query_row("SELECT id FROM payables WHERE contact_id = ?1 AND invoice_ref = ?2 AND status = 'unpaid'", rusqlite::params![c_id, ref_trimmed], |r| r.get::<_, i64>(0)).ok();
    if let Some(id) = dup {
        return Err(payments_error(
            "PAYABLE_DUPLICATE",
            format!(
                "payable {id} already exists for {} / '{ref_trimmed}' and is still unpaid",
                c["name"]
            ),
        ));
    }

    if dry_run {
        return Ok(
            json!({"action": "payables.add", "contact_id": c_id, "invoice_ref": ref_trimmed, "dryRun": true}),
        );
    }

    db.execute("INSERT INTO payables (contact_id, invoice_ref, date, due_date, amount_cents, payment_method, created_by) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![c_id, ref_trimmed, date, due_date, amount_cents, method, actor]).map_err(sql_err)?;
    let id = db.last_insert_rowid();
    record(
        db,
        RecordArgs {
            actor,
            action: "payables.add",
            command: Some("payments payables add"),
            args: Some(json!({"payable_id": id})),
            outcome: "ok",
            entry_ids: vec![],
        },
    )?;
    // the JS hands back the stored payables ROW (payment_method, status, ...)
    Ok(db
        .query_row(
            "SELECT p.id, p.contact_id, c.name AS contact_name, p.invoice_ref, p.date, p.due_date, p.amount_cents, p.payment_method, p.status, p.entry_id, p.batch_line_id, p.created_by, p.created_at FROM payables p JOIN contacts c ON c.id = p.contact_id WHERE p.id = ?1",
            [id],
            |r| {
                Ok(json!({
                    "id": r.get::<_, i64>(0)?, "contact_id": r.get::<_, i64>(1)?,
                    "contact_name": r.get::<_, String>(2)?, "invoice_ref": r.get::<_, String>(3)?,
                    "date": r.get::<_, String>(4)?, "due_date": r.get::<_, Option<String>>(5)?,
                    "amount_cents": r.get::<_, i64>(6)?, "payment_method": r.get::<_, String>(7)?,
                    "status": r.get::<_, String>(8)?, "entry_id": r.get::<_, Option<i64>>(9)?,
                    "batch_line_id": r.get::<_, Option<i64>>(10)?, "created_by": r.get::<_, String>(11)?,
                    "created_at": r.get::<_, String>(12)?,
                }))
            },
        )
        .map_err(sql_err)?)
}

pub fn list_payables(
    db: &Connection,
    status: Option<&str>,
    method: Option<&str>,
    contact_id: Option<i64>,
) -> Result<Vec<Value>> {
    let mut sql =
        "SELECT p.*, c.name AS contact_name FROM payables p JOIN contacts c ON c.id = p.contact_id"
            .to_string();
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let mut clauses = Vec::new();
    if let Some(s) = status {
        clauses.push("p.status = ?".to_string());
        params.push(Box::new(s.to_string()));
    }
    if let Some(m) = method {
        clauses.push("p.payment_method = ?".to_string());
        params.push(Box::new(m.to_string()));
    }
    if let Some(cid) = contact_id {
        clauses.push("p.contact_id = ?".to_string());
        params.push(Box::new(cid));
    }
    if !clauses.is_empty() {
        sql.push_str(&format!(" WHERE {}", clauses.join(" AND ")));
    }
    sql.push_str(" ORDER BY p.due_date IS NULL, p.due_date, p.id");
    let mut stmt = db.prepare(&sql).map_err(sql_err)?;
    let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
    let rows = stmt.query_map(param_refs.as_slice(), |r| {
        // `SELECT p.*, c.name AS contact_name` — the JS hands the whole row back
        Ok(json!({
            "id": r.get::<_, i64>(0)?, "contact_id": r.get::<_, i64>(1)?,
            "invoice_ref": r.get::<_, String>(2)?, "date": r.get::<_, String>(3)?,
            "due_date": r.get::<_, Option<String>>(4)?, "amount_cents": r.get::<_, i64>(5)?,
            "payment_method": r.get::<_, String>(6)?, "status": r.get::<_, String>(7)?,
            "entry_id": r.get::<_, Option<i64>>(8)?, "batch_line_id": r.get::<_, Option<i64>>(9)?,
            "created_by": r.get::<_, String>(10)?, "created_at": r.get::<_, String>(11)?,
            "source": r.get::<_, Option<String>>(12)?, "source_ref": r.get::<_, Option<String>>(13)?,
            "contact_name": r.get::<_, String>(14)?,
        }))
    }).map_err(sql_err)?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

pub fn mark_payable_paid(db: &Connection, id: i64, actor: &str, dry_run: bool) -> Result<Value> {
    let p = db
        .query_row("SELECT status FROM payables WHERE id = ?1", [id], |r| {
            r.get::<_, String>(0)
        })
        .map_err(|_| payments_error("PAYABLE_NOT_FOUND", format!("payable {id} does not exist")))?;
    if p == "paid" {
        return Err(payments_error(
            "ALREADY_PAID",
            format!("payable {id} is already paid"),
        ));
    }
    if dry_run {
        return Ok(json!({"action": "payables.pay", "payable_id": id, "dryRun": true}));
    }
    db.execute("UPDATE payables SET status = 'paid' WHERE id = ?1", [id])
        .map_err(sql_err)?;
    record(
        db,
        RecordArgs {
            actor,
            action: "payables.pay",
            command: Some("payments payables pay"),
            args: Some(json!({"payable_id": id})),
            outcome: "ok",
            entry_ids: vec![],
        },
    )?;
    Ok(json!({"payable_id": id, "status": "paid"}))
}

// --- Batch creation ---------------------------------------------------------

fn serialize_batch(db: &Connection, id: i64) -> Result<Value> {
    let batch = db.query_row("SELECT id, batch_date, debit_iban, debit_name, total_cents, status, msg_id, file_hash, schema, created_by, created_at, exported_at, batch_kind FROM payment_batches WHERE id = ?1", [id], |r| {
        {
            let total = r.get::<_, i64>(4)?;
            Ok(json!({"id": r.get::<_, i64>(0)?, "batch_date": r.get::<_, String>(1)?, "debit_iban": r.get::<_, String>(2)?, "debit_name": r.get::<_, String>(3)?, "total_cents": total, "total": crate::money::format_amount(total), "status": r.get::<_, String>(5)?, "msg_id": r.get::<_, Option<String>>(6)?, "file_hash": r.get::<_, Option<String>>(7)?, "schema": r.get::<_, Option<String>>(8)?, "created_by": r.get::<_, String>(9)?, "created_at": r.get::<_, String>(10)?, "exported_at": r.get::<_, Option<String>>(11)?, "batch_kind": r.get::<_, String>(12)?}))
        }
    }).map_err(|_| payments_error("BATCH_NOT_FOUND", format!("batch {id} does not exist")))?;

    let mut stmt = db.prepare("SELECT id, batch_id, contact_id, name, iban, amount_cents, reference, mandate_id, mandate_ref, mandate_seq, mandate_date, scheme FROM payment_batch_lines WHERE batch_id = ?1 ORDER BY id").map_err(sql_err)?;
    let lines: Vec<Value> = stmt.query_map([id], |r| {
        {
            let amt = r.get::<_, i64>(5)?;
            Ok(json!({"id": r.get::<_, i64>(0)?, "batch_id": r.get::<_, i64>(1)?, "contact_id": r.get::<_, Option<i64>>(2)?, "name": r.get::<_, String>(3)?, "iban": r.get::<_, String>(4)?, "amount_cents": amt, "amount": crate::money::format_amount(amt), "reference": r.get::<_, Option<String>>(6)?, "mandate_id": r.get::<_, Option<i64>>(7)?, "mandate_ref": r.get::<_, Option<String>>(8)?, "mandate_seq": r.get::<_, Option<String>>(9)?, "mandate_date": r.get::<_, Option<String>>(10)?, "scheme": r.get::<_, Option<String>>(11)?}))
        }
    }).map_err(sql_err)?.filter_map(|r| r.ok()).collect();

    let mut b = batch;
    b["lines"] = json!(lines);
    Ok(b)
}

pub fn get_payment_batch(db: &Connection, id: i64) -> Result<Value> {
    serialize_batch(db, id)
}

pub fn list_payment_batches(db: &Connection, status: Option<&str>) -> Result<Vec<Value>> {
    let (sql, param): (String, Option<String>) = if let Some(s) = status {
        (
            "SELECT id FROM payment_batches WHERE status = ?1 ORDER BY id DESC".into(),
            Some(s.to_string()),
        )
    } else {
        (
            "SELECT id FROM payment_batches ORDER BY id DESC".into(),
            None,
        )
    };
    let mut stmt = db.prepare(&sql).map_err(sql_err)?;
    let rows: Vec<i64> = if let Some(ref p) = param {
        stmt.query_map([p.as_str()], |r| r.get::<_, i64>(0))
            .map_err(sql_err)?
            .filter_map(|r| r.ok())
            .collect()
    } else {
        stmt.query_map([], |r| r.get::<_, i64>(0))
            .map_err(sql_err)?
            .filter_map(|r| r.ok())
            .collect()
    };
    let mut result = Vec::new();
    for id in rows {
        result.push(serialize_batch(db, id)?);
    }
    Ok(result)
}

/// The JS's mandateSeqFor: FRST for the first use of a mandate, RCUR after.
fn mandate_seq_for(db: &Connection, mandate_id: i64) -> Result<String> {
    let used: i64 = db
        .prepare("SELECT COUNT(*) FROM payment_batch_lines WHERE mandate_id = ?1")
        .map_err(sql_err)?
        .query_row([mandate_id], |r| r.get(0))
        .unwrap_or(0);
    Ok(if used > 0 { "RCUR" } else { "FRST" }.to_string())
}

pub fn create_payment_batch(
    db: &Connection,
    date: Option<&str>,
    debit_iban: Option<&str>,
    lines: &[Value],
    payable_ids: &[i64],
    kind: &str,
    actor: &str,
    dry_run: bool,
) -> Result<Value> {
    if !["transfer", "direct_debit"].contains(&kind) {
        return Err(payments_error(
            "INVALID_KIND",
            "batch kind must be 'transfer' or 'direct_debit'",
        ));
    }
    let company = get_company(db)?
        .ok_or_else(|| payments_error("COMPANY_REQUIRED", "company is not initialised"))?;
    let debit = if let Some(ib) = debit_iban {
        normalize_iban(ib)
    } else {
        normalize_iban(company["iban"].as_str().unwrap_or(""))
    };
    if !is_valid_iban(&debit) {
        return Err(payments_error(
            "COMPANY_INCOMPLETE",
            "no valid company IBAN — set one with: bukio company update --iban <IBAN>",
        ));
    }
    let today = crate::dates::today_iso();
    let batch_date = date.unwrap_or(&today);
    if !valid_date(batch_date) {
        return Err(payments_error(
            "INVALID_DATE",
            format!("batch date '{batch_date}' must be YYYY-MM-DD"),
        ));
    }

    let mut items: Vec<Value> = Vec::new();
    // the JS collects every problem as {line, error: "CODE: message"} and
    // reports them all at once — first-error abort loses the rest of the file
    let mut errors: Vec<Value> = Vec::new();

    for (i, l) in lines.iter().enumerate() {
        let line_no = format!("line {}", i + 1);
        let mut name = l["name"].as_str().unwrap_or("").to_string();
        let mut iban = l["iban"].as_str().map(normalize_iban);
        let amount = l["amountCents"].as_i64().unwrap_or(0);
        let reference = l["reference"].as_str();
        let mut contact_id: Option<i64> = None;

        // a contact ref is a NAME or an ID (the JS resolveContact takes both)
        let contact_ref = l["contact"]
            .as_str()
            .map(|s| s.to_string())
            .or_else(|| l["contact"].as_i64().map(|n| n.to_string()));
        if let Some(cr) = contact_ref {
            match resolve_contact(db, &cr)? {
                Some(c) => {
                    let cid = c["id"].as_i64().unwrap_or(0);
                    if name.is_empty() {
                        name = c["name"].as_str().unwrap_or("").to_string();
                    }
                    if iban.is_none() {
                        iban = Some(normalize_iban(c["iban"].as_str().unwrap_or("")));
                    }
                    if iban.as_deref().unwrap_or("").is_empty() {
                        errors.push(json!({"line": line_no, "error": format!("CONTACT_IBAN_MISSING: contact {name} has no IBAN — set one with: bukio contact update --id {cid} --iban <IBAN>")}));
                        continue;
                    }
                    contact_id = Some(cid);
                }
                None => {
                    errors.push(json!({"line": line_no, "error": format!("CONTACT_NOT_FOUND: contact '{cr}' does not exist")}));
                    continue;
                }
            }
        }
        let iban = iban.unwrap_or_default();

        if amount <= 0 {
            errors.push(json!({"line": line_no, "error": "INVALID_AMOUNT: amount must be a positive amount in cents"}));
            continue;
        }
        if name.is_empty() {
            errors.push(
                json!({"line": line_no, "error": "NAME_REQUIRED: beneficiary name is required"}),
            );
            continue;
        }
        if name.chars().count() > 70 {
            errors.push(json!({"line": line_no, "error": "SEPA_NAME_TOO_LONG: beneficiary name max 70 characters (SEPA Max70Text)"}));
            continue;
        }
        if !is_valid_iban(&iban) {
            errors.push(json!({"line": line_no, "error": format!("INVALID_IBAN: '{iban}' is not a valid IBAN")}));
            continue;
        }
        if reference.map(|r| r.chars().count() > 140).unwrap_or(false) {
            errors.push(json!({"line": line_no, "error": "REFERENCE_TOO_LONG: reference max 140 characters"}));
            continue;
        }

        let mut item = json!({"contact_id": contact_id, "name": name, "iban": iban, "amount_cents": amount, "reference": reference});
        if kind == "direct_debit" {
            let mandate = contact_id.and_then(|cid| latest_mandate(db, cid));
            match mandate {
                Some(m) => {
                    let mid = m["id"].as_i64().unwrap_or(0);
                    item["mandate_id"] = m["id"].clone();
                    item["mandate_ref"] = m["mandate_ref"].clone();
                    item["mandate_seq"] = json!(mandate_seq_for(db, mid)?);
                    item["mandate_date"] = m["mandate_date"].clone();
                    item["scheme"] = m["scheme"].clone();
                }
                None => {
                    errors.push(json!({"line": line_no, "error": format!("MANDATE_REQUIRED: {} has no SEPA mandate — add one with: bukio payments mandate add --contact {} --ref <REF> [--type b2b], or use a transfer batch", contact_id.map(|c| format!("contact {c}")).unwrap_or_else(|| format!("line '{name}'")), contact_id.map(|c| c.to_string()).unwrap_or_else(|| "<id>".into()))}));
                    continue;
                }
            }
        }
        items.push(item);
    }

    // Payables
    for &pid in payable_ids {
        let line_no = format!("payable {pid}");
        let p = db.query_row("SELECT id, contact_id, invoice_ref, amount_cents, payment_method, status FROM payables WHERE id = ?1", [pid], |r| {
            Ok(json!({"id": r.get::<_, i64>(0)?, "contact_id": r.get::<_, i64>(1)?, "invoice_ref": r.get::<_, String>(2)?, "amount_cents": r.get::<_, i64>(3)?, "payment_method": r.get::<_, String>(4)?, "status": r.get::<_, String>(5)?}))
        });
        let p = match p {
            Ok(p) => p,
            Err(_) => {
                errors.push(json!({"line": line_no, "error": format!("PAYABLE_NOT_FOUND: payable {pid} does not exist")}));
                continue;
            }
        };
        if p["status"] != "unpaid" {
            errors.push(json!({"line": line_no, "error": format!("PAYABLE_NOT_UNPAID: payable {pid} is {}", p["status"].as_str().unwrap_or(""))}));
            continue;
        }
        if kind == "transfer" {
            if p["payment_method"] != "transfer" {
                errors.push(json!({"line": line_no, "error": format!("PAYABLE_DIRECT_DEBIT: payable {pid} is paid by direct debit — excluded from transfer batches")}));
                continue;
            }
        } else if p["payment_method"] != "direct_debit" {
            errors.push(json!({"line": line_no, "error": format!("PAYABLE_NOT_DIRECT_DEBIT: payable {pid} is a transfer — not a direct debit; use a transfer batch")}));
            continue;
        }
        let c = get_contact(db, p["contact_id"].as_i64().unwrap_or(0))?.unwrap_or(Value::Null);
        let cname = c["name"].as_str().unwrap_or("").to_string();
        let iban = normalize_iban(c["iban"].as_str().unwrap_or(""));
        if iban.is_empty() {
            errors.push(json!({"line": line_no, "error": format!("CONTACT_IBAN_MISSING: contact {cname} has no IBAN — set one with: bukio contact update --id {} --iban <IBAN>", p["contact_id"].as_i64().unwrap_or(0))}));
            continue;
        }
        if !is_valid_iban(&iban) {
            errors.push(json!({"line": line_no, "error": format!("INVALID_IBAN: '{iban}' is not a valid IBAN")}));
            continue;
        }
        if cname.chars().count() > 70 {
            let head: String = cname.chars().take(40).collect();
            errors.push(json!({"line": line_no, "error": format!("SEPA_NAME_TOO_LONG: contact {head}… name max 70 characters (SEPA Max70Text)")}));
            continue;
        }
        let mut item = json!({"payable_id": pid, "contact_id": p["contact_id"], "name": cname, "iban": iban, "amount_cents": p["amount_cents"], "reference": format!("Invoice {}", p["invoice_ref"].as_str().unwrap_or(""))});
        if kind == "direct_debit" {
            let mandate = latest_mandate(db, p["contact_id"].as_i64().unwrap_or(0));
            match mandate {
                Some(m) => {
                    let mid = m["id"].as_i64().unwrap_or(0);
                    item["mandate_id"] = m["id"].clone();
                    item["mandate_ref"] = m["mandate_ref"].clone();
                    item["mandate_seq"] = json!(mandate_seq_for(db, mid)?);
                    item["mandate_date"] = m["mandate_date"].clone();
                    item["scheme"] = m["scheme"].clone();
                }
                None => {
                    errors.push(json!({"line": line_no, "error": format!("MANDATE_REQUIRED: contact {cname} has no SEPA mandate — add one with: bukio payments mandate add --contact {} --ref <REF> [--type b2b]", p["contact_id"].as_i64().unwrap_or(0))}));
                    continue;
                }
            }
        }
        items.push(item);
    }

    if items.is_empty() && errors.is_empty() {
        return Err(payments_error(
            "EMPTY_BATCH",
            "no payments to batch — pass --lines, --csv or --from-invoices",
        ));
    }
    if !errors.is_empty() {
        let mut err = payments_error(
            "BATCH_VALIDATION_FAILED",
            format!(
                "{} payment line{} failed validation",
                errors.len(),
                if errors.len() == 1 { "" } else { "s" }
            ),
        );
        err.details = Some(Value::Array(errors));
        return Err(err);
    }

    // SEPA Max70Text applies to the debtor name too
    let company_name = company["name"].as_str().unwrap_or("");
    if company_name.chars().count() > 70 {
        return Err(payments_error(
            "SEPA_NAME_TOO_LONG",
            format!(
                "company name max 70 characters for SEPA (Max70Text) — got {}",
                company_name.chars().count()
            ),
        ));
    }

    let total: i64 = items
        .iter()
        .map(|l| l["amount_cents"].as_i64().unwrap_or(0))
        .sum();

    if dry_run {
        return Ok(
            json!({"action": "payments.batch.create", "batch_date": batch_date, "debit_iban": debit, "debit_name": company["name"], "batch_kind": kind, "total_cents": total, "lines": items, "dryRun": true}),
        );
    }

    db.execute("INSERT INTO payment_batches (batch_date, debit_iban, debit_name, total_cents, batch_kind, created_by) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![batch_date, debit, company["name"].as_str().unwrap_or(""), total, kind, actor]).map_err(sql_err)?;
    let batch_id = db.last_insert_rowid();

    for l in &items {
        db.execute("INSERT INTO payment_batch_lines (batch_id, contact_id, name, iban, amount_cents, reference, mandate_id, mandate_ref, mandate_seq, mandate_date, scheme) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            rusqlite::params![batch_id, l["contact_id"].as_i64(), l["name"].as_str().unwrap_or(""), l["iban"].as_str().unwrap_or(""), l["amount_cents"].as_i64().unwrap_or(0), l["reference"].as_str(), l["mandate_id"].as_i64(), l["mandate_ref"].as_str(), l["mandate_seq"].as_str(), l["mandate_date"].as_str(), l["scheme"].as_str()]).map_err(sql_err)?;
        if let Some(pid) = l["payable_id"].as_i64() {
            let line_id = db.last_insert_rowid();
            db.execute(
                "UPDATE payables SET status = 'in_batch', batch_line_id = ?1 WHERE id = ?2",
                [line_id, pid],
            )
            .map_err(sql_err)?;
        }
    }

    record(
        db,
        RecordArgs {
            actor,
            action: "payments.batch.create",
            command: Some("payments batch create"),
            args: Some(json!({"batch_id": batch_id, "kind": kind, "total_cents": total})),
            outcome: "ok",
            entry_ids: vec![],
        },
    )?;
    serialize_batch(db, batch_id)
}

// --- SEPA XML builders ------------------------------------------------------

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

const CSV_HEADER_ALIASES: [(&str, &[&str]); 3] = [
    ("contact", &["contact", "naam", "leverancier", "name"]),
    ("amount", &["amount", "bedrag"]),
    (
        "reference",
        &["reference", "omschrijving", "ref", "description"],
    ),
];

/// Parse a payment-batch CSV (JS: parseBatchCsv). The delimiter is detected
/// ONCE from the first line — the old per-row heuristic flipped to ';' mode for
/// any row containing a semicolon and misparsed comma-delimited CSVs.
pub fn parse_batch_csv(csv_text: &str) -> Result<Value> {
    let lines: Vec<&str> = csv_text
        .split('\n')
        .map(|l| l.trim_end_matches('\r'))
        .filter(|l| !l.trim().is_empty())
        .collect();
    if lines.is_empty() {
        return Err(payments_error("EMPTY_CSV", "batch CSV is empty"));
    }
    let delimiter = if lines[0].matches(';').count() > lines[0].matches(',').count() {
        ';'
    } else {
        ','
    };

    // quote-aware split: a quoted field may contain the delimiter
    let parse_row = |line: &str| -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut cur = String::new();
        let mut quoted = false;
        for ch in line.chars() {
            if ch == '"' {
                quoted = !quoted;
                continue;
            }
            if ch == delimiter && !quoted {
                out.push(cur.trim().to_string());
                cur.clear();
                continue;
            }
            cur.push(ch);
        }
        out.push(cur.trim().to_string());
        out
    };

    let header: Vec<String> = parse_row(lines[0])
        .iter()
        .map(|h| h.to_lowercase())
        .collect();
    let has_header = header
        .iter()
        .any(|h| ["contact", "naam", "leverancier", "amount", "bedrag"].contains(&h.as_str()));
    let idx: Option<std::collections::HashMap<String, usize>> = if has_header {
        Some(
            header
                .iter()
                .enumerate()
                .map(|(i, h)| (h.clone(), i))
                .collect(),
        )
    } else {
        None
    };
    let col = |row: &[String], key: &str| -> Option<String> {
        match &idx {
            Some(ix) => {
                let aliases = CSV_HEADER_ALIASES
                    .iter()
                    .find(|(k, _)| *k == key)
                    .map(|(_, a)| *a)
                    .unwrap_or(&[]);
                for alias in aliases {
                    if let Some(i) = ix.get(*alias) {
                        return row.get(*i).cloned();
                    }
                }
                None
            }
            None => {
                let pos = match key {
                    "contact" => 0,
                    "amount" => 1,
                    _ => 2,
                };
                row.get(pos).cloned()
            }
        }
    };

    let mut out: Vec<Value> = Vec::new();
    let mut errors: Vec<Value> = Vec::new();
    let start = if has_header { 1 } else { 0 };
    for i in start..lines.len() {
        let row = parse_row(lines[i]);
        if row.len() == 1 && row[0].is_empty() {
            continue;
        }
        let contact = col(&row, "contact").unwrap_or_default();
        let amount_str = col(&row, "amount");
        let reference = col(&row, "reference").filter(|r| !r.is_empty());
        if contact.trim().is_empty() {
            errors.push(json!({
                "line": i + 1,
                "error": "CONTACT_REQUIRED: every batch line needs a contact",
            }));
            continue;
        }
        let amount_cents = amount_str
            .as_deref()
            .and_then(|a| crate::import_mod::parse_import_amount(a).ok());
        match amount_cents {
            Some(a) if a > 0 => out.push(json!({
                "contact": contact, "amountCents": a, "reference": reference,
            })),
            _ => errors.push(json!({
                "line": i + 1,
                "error": format!(
                    "INVALID_AMOUNT: '{}' is not a positive amount",
                    amount_str.unwrap_or_default()
                ),
            })),
        }
    }
    if out.is_empty() && errors.is_empty() {
        return Err(payments_error(
            "EMPTY_CSV",
            "batch CSV contains no payment lines",
        ));
    }
    Ok(json!({ "lines": out, "errors": errors, "hasHeader": has_header }))
}

pub fn create_payment_batch_from_csv(
    db: &Connection,
    csv_text: &str,
    date: Option<&str>,
    debit_iban: Option<&str>,
    actor: &str,
    dry_run: bool,
) -> Result<Value> {
    let parsed = parse_batch_csv(csv_text)?;
    let errors = parsed["errors"].as_array().cloned().unwrap_or_default();
    if !errors.is_empty() {
        let mut err = payments_error(
            "IMPORT_VALIDATION_FAILED",
            format!(
                "{} CSV line{} failed validation",
                errors.len(),
                if errors.len() == 1 { "" } else { "s" }
            ),
        );
        err.details = Some(Value::Array(errors));
        return Err(err);
    }
    let lines = parsed["lines"].as_array().cloned().unwrap_or_default();
    create_payment_batch(
        db,
        date,
        debit_iban,
        &lines,
        &[],
        "transfer",
        actor,
        dry_run,
    )
}

pub fn build_pain001(
    msg_id: &str,
    created_iso: &str,
    debit_name: &str,
    debit_iban: &str,
    batch_date: &str,
    lines: &[Value],
    schema: &str,
) -> String {
    let ns = if schema == "001.09" {
        "pain.001.001.09"
    } else {
        "pain.001.001.03"
    };
    let total: i64 = lines
        .iter()
        .map(|l| l["amount_cents"].as_i64().unwrap_or(0))
        .sum();
    let ctrl = format!("{:.2}", total as f64 / 100.0);
    let txs: Vec<String> = lines.iter().enumerate().map(|(i, l)| {
        let e2e = l["reference"].as_str().map(|r| r.chars().take(35).collect::<String>()).unwrap_or_else(|| format!("BUKIO{}", i + 1));
        let amt = format!("{:.2}", l["amount_cents"].as_i64().unwrap_or(0) as f64 / 100.0);
        let rmt = l["reference"].as_str().map(|r| format!("<RmtInf><Ustrd>{}</Ustrd></RmtInf>", esc(r))).unwrap_or_default();
        format!("      <CdtTrfTxInf>\n        <PmtId><EndToEndId>{}</EndToEndId></PmtId>\n        <Amt><InstdAmt Ccy=\"EUR\">{}</InstdAmt></Amt>\n        <Cdtr><Nm>{}</Nm></Cdtr>\n        <CdtrAcct><Id><IBAN>{}</IBAN></Id></CdtrAcct>\n        {}\n      </CdtTrfTxInf>", esc(&e2e), amt, esc(l["name"].as_str().unwrap_or("")), esc(l["iban"].as_str().unwrap_or("")), rmt)
    }).collect();

    format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Document xmlns=\"urn:iso:std:iso:20022:tech:xsd:{ns}\">\n  <CstmrCdtTrfInitn>\n    <GrpHdr>\n      <MsgId>{}</MsgId>\n      <CreDtTm>{}</CreDtTm>\n      <NbOfTxs>{}</NbOfTxs>\n      <CtrlSum>{}</CtrlSum>\n      <InitgPty><Nm>{}</Nm></InitgPty>\n    </GrpHdr>\n    <PmtInf>\n      <PmtInfId>{}</PmtInfId>\n      <PmtMtd>TRF</PmtMtd>\n      <BtchBookg>true</BtchBookg>\n      <NbOfTxs>{}</NbOfTxs>\n      <CtrlSum>{}</CtrlSum>\n      <PmtTpInf><SvcLvl><Cd>SEPA</Cd></SvcLvl></PmtTpInf>\n      <ReqdExctnDt>{}</ReqdExctnDt>\n      <Dbtr><Nm>{}</Nm></Dbtr>\n      <DbtrAcct><Id><IBAN>{}</IBAN></Id></DbtrAcct>\n      <ChrgBr>SLEV</ChrgBr>\n{}\n    </PmtInf>\n  </CstmrCdtTrfInitn>\n</Document>\n",
        esc(msg_id), esc(created_iso), lines.len(), ctrl, esc(debit_name), esc(msg_id), lines.len(), ctrl, esc(batch_date), esc(debit_name), esc(debit_iban), txs.join("\n"))
}

/// pain.008.001.02 direct-debit initiation. One PmtInf per mandate scheme.
/// Lines carry the debtor's mandate snapshot (mirrors JS buildPain008).
pub fn build_pain008(
    msg_id: &str,
    created_iso: &str,
    debit_name: &str,
    debit_iban: &str,
    batch_date: &str,
    lines: &[Value],
) -> String {
    let total: i64 = lines
        .iter()
        .map(|l| l["amount_cents"].as_i64().unwrap_or(0))
        .sum();
    let ctrl = format!("{:.2}", total as f64 / 100.0);
    let by_scheme = |scheme: &str| -> Vec<&Value> {
        lines
            .iter()
            .filter(|l| l["scheme"].as_str().unwrap_or("core") == scheme)
            .collect()
    };
    let tx_inf = |l: &Value, i: usize| -> String {
        let e2e = l["reference"]
            .as_str()
            .map(|r| r.chars().take(35).collect::<String>())
            .unwrap_or_else(|| format!("BUKIO{}", i + 1));
        let amt = format!(
            "{:.2}",
            l["amount_cents"].as_i64().unwrap_or(0) as f64 / 100.0
        );
        let mandate_ref = l["mandate_ref"].as_str().unwrap_or("");
        let mandate_date = l["mandate_date"].as_str().unwrap_or("");
        let rmt = l["reference"]
            .as_str()
            .map(|r| format!("        <RmtInf><Ustrd>{}</Ustrd></RmtInf>\n", esc(r)))
            .unwrap_or_default();
        format!(
            "      <DrctDbtTxInf>\n        <PmtId><EndToEndId>{}</EndToEndId></PmtId>\n        <InstdAmt Ccy=\"EUR\">{}</InstdAmt>\n        <DrctDbtTx><MndtRltdInf>\n          <MndtId>{}</MndtId>\n          <DtOfSgntr>{}</DtOfSgntr>\n        </MndtRltdInf></DrctDbtTx>\n        <DbtrAgt><FinInstnId><Othr><Id>NOTPROVIDED</Id></Othr></FinInstnId></DbtrAgt>\n        <Dbtr><Nm>{}</Nm></Dbtr>\n        <DbtrAcct><Id><IBAN>{}</IBAN></Id></DbtrAcct>\n{}\n      </DrctDbtTxInf>",
            esc(&e2e),
            amt,
            esc(mandate_ref),
            esc(mandate_date),
            esc(l["name"].as_str().unwrap_or("")),
            esc(l["iban"].as_str().unwrap_or("")),
            rmt,
        )
    };
    let pmt_inf = |scheme: &str, scheme_lines: &[&Value], idx: usize| -> String {
        let sub_total: i64 = scheme_lines
            .iter()
            .map(|l| l["amount_cents"].as_i64().unwrap_or(0))
            .sum();
        let sub_ctrl = format!("{:.2}", sub_total as f64 / 100.0);
        let txs: Vec<String> = scheme_lines
            .iter()
            .enumerate()
            .map(|(i, l)| tx_inf(l, i))
            .collect();
        let pmt_inf_id = format!("{}{}", &msg_id.chars().take(34).collect::<String>(), idx);
        let instr = if scheme == "b2b" { "B2B" } else { "CORE" };
        format!(
            "    <PmtInf>\n      <PmtInfId>{}</PmtInfId>\n      <PmtMtd>DD</PmtMtd>\n      <BtchBookg>true</BtchBookg>\n      <NbOfTxs>{}</NbOfTxs>\n      <CtrlSum>{}</CtrlSum>\n      <PmtTpInf><SvcLvl><Cd>SEPA</Cd></SvcLvl><LclInstrm><Cd>{}</Cd></LclInstrm></PmtTpInf>\n      <ReqdColltnDt>{}</ReqdColltnDt>\n      <Cdtr><Nm>{}</Nm></Cdtr>\n      <CdtrAcct><Id><IBAN>{}</IBAN></Id></CdtrAcct>\n      <CdtrAgt><FinInstnId><Othr><Id>NOTPROVIDED</Id></Othr></FinInstnId></CdtrAgt>\n      <ChrgBr>SLEV</ChrgBr>\n{}\n    </PmtInf>",
            esc(&pmt_inf_id),
            scheme_lines.len(),
            sub_ctrl,
            instr,
            esc(batch_date),
            esc(debit_name),
            esc(debit_iban),
            txs.join("\n"),
        )
    };
    let schemes: Vec<&str> = ["core", "b2b"]
        .iter()
        .copied()
        .filter(|s| !by_scheme(s).is_empty())
        .collect();
    let pmt_infs: Vec<String> = schemes
        .iter()
        .enumerate()
        .map(|(i, s)| pmt_inf(s, &by_scheme(s), i + 1))
        .collect();
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Document xmlns=\"urn:iso:std:iso:20022:tech:xsd:pain.008.001.02\">\n  <CstmrDrctDbtInitn>\n    <GrpHdr>\n      <MsgId>{}</MsgId>\n      <CreDtTm>{}</CreDtTm>\n      <NbOfTxs>{}</NbOfTxs>\n      <CtrlSum>{}</CtrlSum>\n      <InitgPty><Nm>{}</Nm></InitgPty>\n    </GrpHdr>\n{}\n  </CstmrDrctDbtInitn>\n</Document>\n",
        esc(msg_id),
        esc(created_iso),
        lines.len(),
        ctrl,
        esc(debit_name),
        pmt_infs.join("\n"),
    )
}

/// Export a draft batch as SEPA XML: pain.001 for transfer, pain.008.001.02
/// for direct-debit. Marks the batch exported (mirrors JS exportPaymentBatch).
pub fn export_payment_batch(
    db: &Connection,
    id: i64,
    actor: &str,
    dry_run: bool,
    schema_override: Option<&str>,
) -> Result<Value> {
    let batch = serialize_batch(db, id)?;
    if batch.get("status").and_then(|v| v.as_str()) != Some("draft") {
        return Err(payments_error(
            "BATCH_ALREADY_EXPORTED",
            format!("batch {id} is already exported — exporting again could double-pay; create a new batch instead"),
        ));
    }
    let is_dd = batch.get("batch_kind").and_then(|v| v.as_str()) == Some("direct_debit");
    let lines = batch["lines"].as_array().cloned().unwrap_or_default();
    let msg_id = format!(
        "BUKIO{}{}",
        chrono::Utc::now()
            .format("%Y%m%d%H%M%S")
            .to_string()
            .chars()
            .take(14)
            .collect::<String>(),
        id.to_string()
            .chars()
            .rev()
            .take(16)
            .collect::<String>()
            .chars()
            .rev()
            .collect::<String>()
    );
    let created_iso = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let debit_name = batch
        .get("debit_name")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let debit_iban = batch
        .get("debit_iban")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let batch_date = batch
        .get("batch_date")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let (schema, xml) = if is_dd {
        // A direct-debit batch is always pain.008: an override asking for
        // anything else is an INVALID_SCHEMA. The port ignored the override on
        // this branch entirely, so `--schema 001.03` on a DD batch "succeeded"
        // and produced pain.008 anyway — the caller's mistake went unreported.
        if let Some(o) = schema_override {
            if !o.contains("008") {
                return Err(payments_error(
                    "INVALID_SCHEMA",
                    format!("batch {id} is a direct-debit batch — schema must be pain.008.001.02, got '{o}'"),
                ));
            }
        }
        (
            "pain.008.001.02".to_string(),
            build_pain008(
                &msg_id,
                &created_iso,
                debit_name,
                debit_iban,
                batch_date,
                &lines,
            ),
        )
    } else {
        // the JS lets the caller pin the pain.001 schema version
        let version = schema_override.unwrap_or("001.03");
        (
            if version == "001.09" {
                "pain.001.001.09".to_string()
            } else {
                "pain.001.001.03".to_string()
            },
            build_pain001(
                &msg_id,
                &created_iso,
                debit_name,
                debit_iban,
                batch_date,
                &lines,
                version,
            ),
        )
    };
    let file_hash = format!("{:x}", {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        use std::io::Write;
        h.write_all(xml.as_bytes()).ok();
        h.finalize()
    });
    if dry_run {
        return Ok(json!({
            "action": "payments.batch.export", "batch_id": id, "batch_kind": batch.get("batch_kind"),
            "schema": schema, "msg_id": msg_id, "lines": lines.len(), "total_cents": batch.get("total_cents"),
            "file_hash": file_hash, "xml": xml, "dryRun": true,
        }));
    }
    db.execute(
        "UPDATE payment_batches SET status = 'exported', msg_id = ?1, file_hash = ?2, schema = ?3, exported_at = ?4 WHERE id = ?5",
        rusqlite::params![msg_id, file_hash, schema, created_iso, id],
    )
    .map_err(sql_err)?;
    record(
        db,
        RecordArgs {
            actor,
            action: "payments.batch.export",
            command: Some("payments batch export"),
            args: Some(
                json!({"batch_id": id, "kind": batch.get("batch_kind"), "msg_id": msg_id, "lines": lines.len(), "total_cents": batch.get("total_cents"), "file_hash": &file_hash[..12.min(file_hash.len())], "schema": schema}),
            ),
            outcome: "ok",
            entry_ids: vec![],
        },
    )?;
    Ok(json!({
        "action": "payments.batch.export", "batch_id": id, "status": "exported",
        "msg_id": msg_id, "file_hash": file_hash, "schema": schema,
        "xml": xml, "lines": lines.len(), "total_cents": batch.get("total_cents"),
    }))
}

pub fn delete_payment_batch(db: &Connection, id: i64, actor: &str, dry_run: bool) -> Result<Value> {
    let batch = db
        .query_row(
            "SELECT status FROM payment_batches WHERE id = ?1",
            [id],
            |r| r.get::<_, String>(0),
        )
        .map_err(|_| payments_error("BATCH_NOT_FOUND", format!("batch {id} does not exist")))?;
    if batch != "draft" {
        return Err(payments_error(
            "BATCH_ALREADY_EXPORTED",
            format!("batch {id} is already {batch}"),
        ));
    }
    if dry_run {
        return Ok(json!({"action": "payments.batch.delete", "batch_id": id, "dryRun": true}));
    }
    db.execute("UPDATE payables SET status = 'unpaid', batch_line_id = NULL WHERE batch_line_id IN (SELECT id FROM payment_batch_lines WHERE batch_id = ?1)", [id]).map_err(sql_err)?;
    db.execute("DELETE FROM payment_batch_lines WHERE batch_id = ?1", [id])
        .map_err(sql_err)?;
    db.execute("DELETE FROM payment_batches WHERE id = ?1", [id])
        .map_err(sql_err)?;
    record(
        db,
        RecordArgs {
            actor,
            action: "payments.batch.delete",
            command: Some("payments batch delete"),
            args: Some(json!({"batch_id": id})),
            outcome: "ok",
            entry_ids: vec![],
        },
    )?;
    Ok(json!({"batch_id": id, "status": "deleted"}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Connection {
        let d = crate::db::open_db(":memory:").unwrap();
        d.execute(
            "INSERT INTO company (name, iban) VALUES ('PayCo', 'NL91ABNA0417164300')",
            [],
        )
        .unwrap();
        crate::accounts::seed_default_chart(&d).unwrap();
        d
    }

    #[test]
    fn add_list_mandate() {
        let d = test_db();
        let c = crate::contacts::create_contact(
            &d,
            "Vendor",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            "human:erik",
            false,
        )
        .unwrap();
        let m = add_mandate(
            &d,
            c["id"].as_i64().unwrap(),
            "MANDATE-001",
            None,
            "core",
            "human:erik",
            false,
        )
        .unwrap();
        assert_eq!(m["mandate_ref"], "MANDATE-001");
        let list = list_mandates(&d, None).unwrap();
        assert_eq!(list.len(), 1);
    }

    #[test]
    fn add_payable_and_list() {
        let d = test_db();
        let c = crate::contacts::create_contact(
            &d,
            "Vendor",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            "human:erik",
            false,
        )
        .unwrap();
        let p = add_payable(
            &d,
            &c["id"].as_i64().unwrap().to_string(),
            "INV-001",
            "2026-01-15",
            None,
            5000,
            "transfer",
            "human:erik",
            false,
        )
        .unwrap();
        assert_eq!(p["amount_cents"], 5000);
        let list = list_payables(&d, None, None, None).unwrap();
        assert_eq!(list.len(), 1);
    }

    #[test]
    fn build_pain001_xml() {
        let lines = vec![
            json!({"name": "ACME", "iban": "NL91ABNA0417164300", "amount_cents": 10000, "reference": "INV-001"}),
        ];
        let xml = build_pain001(
            "MSG-001",
            "2026-01-15T10:00:00Z",
            "MyCo",
            "NL91ABNA0417164300",
            "2026-01-15",
            &lines,
            "001.03",
        );
        assert!(xml.contains("pain.001.001.03"));
        assert!(xml.contains("100.00"));
        assert!(xml.contains("INV-001"));
    }

    // ==== ported from test/payments.test.js ================================
    const IBAN: &str = "NL91ABNA0417164300";

    fn cdb() -> Connection {
        let d = crate::db::open_db(":memory:").unwrap();
        crate::accounts::seed_default_chart(&d).unwrap();
        d.execute(
            "INSERT INTO company (name, registration_id, legal_form, iban, vat_module) VALUES ('Demo BV','12345678','bv',?1,0)",
            [IBAN],
        )
        .unwrap();
        d
    }

    fn vendor(d: &Connection) -> i64 {
        crate::contacts::create_contact(
            d,
            "Vimexx",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some("NL02ABNA0123456789"),
            "agent:test",
            false,
        )
        .unwrap()["id"]
            .as_i64()
            .unwrap()
    }

    fn code_of(err: &BukioError) -> &str {
        err.code
    }

    fn batch_count(d: &Connection) -> i64 {
        d.query_row("SELECT COUNT(*) FROM payment_batches", [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn iban_mod97_check_and_normalization() {
        assert!(crate::dates::is_valid_iban("NL91ABNA0417164300"));
        assert!(crate::dates::is_valid_iban("nl91 abna 0417 1643 00"));
        assert!(!crate::dates::is_valid_iban("NL91ABNA0417164301"));
        assert!(!crate::dates::is_valid_iban("NL91ABNA04171643"));
        assert!(!crate::dates::is_valid_iban(""));
    }

    #[test]
    fn payables_add_transfer_and_direct_debit_with_audit_and_filters() {
        let d = cdb();
        let v = vendor(&d);
        let p1 = add_payable(
            &d,
            "Vimexx",
            "2026-118",
            "2026-07-01",
            Some("2026-08-01"),
            12100,
            "transfer",
            "agent:test",
            false,
        )
        .unwrap();
        assert_eq!(p1["contact_id"].as_i64(), Some(v));
        let p2 = add_payable(
            &d,
            &v.to_string(),
            "DD-1",
            "2026-07-02",
            None,
            9999,
            "direct_debit",
            "agent:test",
            false,
        )
        .unwrap();
        assert_eq!(p2["payment_method"].as_str(), Some("direct_debit"));
        assert_eq!(
            list_payables(&d, Some("unpaid"), None, None).unwrap().len(),
            2
        );
        assert_eq!(
            list_payables(&d, None, Some("transfer"), None)
                .unwrap()
                .len(),
            1
        );
        let dd = list_payables(&d, None, Some("direct_debit"), None).unwrap();
        assert_eq!(dd[0]["invoice_ref"].as_str(), Some("DD-1"));
        let audit: i64 = d
            .query_row(
                "SELECT COUNT(*) FROM audit_log WHERE action = 'payables.add'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(audit, 2);
    }

    #[test]
    fn payables_reject_unknown_contact_bad_amount_missing_ref_and_bad_method() {
        let d = cdb();
        vendor(&d);
        let e = add_payable(
            &d,
            "Nobody",
            "X",
            "2026-07-01",
            None,
            100,
            "transfer",
            "a:t",
            false,
        )
        .unwrap_err();
        assert_eq!(code_of(&e), "CONTACT_NOT_FOUND");
        let e = add_payable(
            &d,
            "Vimexx",
            "X",
            "2026-07-01",
            None,
            0,
            "transfer",
            "a:t",
            false,
        )
        .unwrap_err();
        assert_eq!(code_of(&e), "INVALID_AMOUNT");
        let e = add_payable(
            &d,
            "Vimexx",
            " ",
            "2026-07-01",
            None,
            100,
            "transfer",
            "a:t",
            false,
        )
        .unwrap_err();
        assert_eq!(code_of(&e), "INVOICE_REF_REQUIRED");
        let e = add_payable(
            &d,
            "Vimexx",
            "X",
            "2026-07-01",
            None,
            100,
            "card",
            "a:t",
            false,
        )
        .unwrap_err();
        assert_eq!(code_of(&e), "INVALID_METHOD");
    }

    #[test]
    fn payable_mark_paid_dry_run_writes_nothing_and_real_is_audited() {
        let d = cdb();
        vendor(&d);
        let p = add_payable(
            &d,
            "Vimexx",
            "R1",
            "2026-07-01",
            None,
            5000,
            "transfer",
            "a:t",
            false,
        )
        .unwrap();
        let id = p["id"].as_i64().unwrap();
        let dry = mark_payable_paid(&d, id, "agent:test", true).unwrap();
        assert_eq!(dry["dryRun"].as_bool(), Some(true));
        assert_eq!(
            list_payables(&d, Some("unpaid"), None, None).unwrap().len(),
            1
        );
        let r = mark_payable_paid(&d, id, "agent:test", false).unwrap();
        assert_eq!(r["status"].as_str(), Some("paid"));
        let e = mark_payable_paid(&d, id, "agent:test", false).unwrap_err();
        assert_eq!(code_of(&e), "ALREADY_PAID");
        let n: i64 = d
            .query_row(
                "SELECT COUNT(*) FROM audit_log WHERE action = 'payables.pay'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(n > 0);
    }

    #[test]
    fn contact_iban_is_validated_on_create_and_normalized_on_update() {
        let d = cdb();
        let e = crate::contacts::create_contact(
            &d,
            "Bad",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some("NL00INVALID"),
            "a:t",
            false,
        )
        .unwrap_err();
        assert_eq!(e.code, "INVALID_IBAN");
        let v = vendor(&d);
        let c = crate::contacts::update_contact(
            &d,
            v,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(" NL86 INGB 0002 4455 88 "),
            "agent:test",
            false,
        )
        .unwrap();
        assert_eq!(c["iban"].as_str(), Some("NL86INGB0002445588"));
        let n: i64 = d
            .query_row(
                "SELECT COUNT(*) FROM audit_log WHERE action = 'contact.update'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(n > 0);
    }

    #[test]
    fn batch_lines_resolve_contacts_by_name_or_id() {
        let d = cdb();
        vendor(&d);
        let lines = vec![
            json!({"contact": "Vimexx", "amountCents": 12100, "reference": "Factuur 2026-118"}),
            json!({"contact": 1, "amountCents": 4550}),
        ];
        let b = create_payment_batch(&d, None, None, &lines, &[], "transfer", "agent:test", false)
            .unwrap();
        assert_eq!(b["lines"].as_array().unwrap().len(), 2);
        assert_eq!(b["total_cents"].as_i64(), Some(16650));
        assert_eq!(b["debit_iban"].as_str(), Some(IBAN));
        assert_eq!(b["lines"][0]["iban"].as_str(), Some("NL02ABNA0123456789"));
        assert_eq!(
            crate::entries::list_entries(&d, None, None, None, 500)
                .unwrap()
                .len(),
            0
        );
    }

    #[test]
    fn batch_without_company_iban_fails_with_a_hint() {
        let d = cdb();
        d.execute("UPDATE company SET iban = NULL", []).unwrap();
        vendor(&d);
        let lines = vec![json!({"contact": "Vimexx", "amountCents": 100})];
        let e = create_payment_batch(&d, None, None, &lines, &[], "transfer", "a:t", false)
            .unwrap_err();
        assert_eq!(e.code, "COMPANY_INCOMPLETE");
        assert!(e.message.contains("company update --iban"), "{}", e.message);
    }

    #[test]
    fn batch_contact_without_iban_reports_details() {
        let d = cdb();
        crate::contacts::create_contact(
            &d, "NoIbanCo", None, None, None, None, None, None, None, None, "a:t", false,
        )
        .unwrap();
        let lines = vec![json!({"contact": "NoIbanCo", "amountCents": 100})];
        let e = create_payment_batch(&d, None, None, &lines, &[], "transfer", "a:t", false)
            .unwrap_err();
        assert_eq!(e.code, "BATCH_VALIDATION_FAILED");
        let details = e.details.unwrap();
        let has = details.as_array().unwrap().iter().any(|x| {
            x["error"]
                .as_str()
                .unwrap_or("")
                .contains("CONTACT_IBAN_MISSING")
        });
        assert!(has, "{details}");
    }

    #[test]
    fn batch_collects_every_bad_line() {
        let d = cdb();
        crate::contacts::create_contact(
            &d, "BadCo", None, None, None, None, None, None, None, None, "a:t", false,
        )
        .unwrap();
        let lines = vec![
            json!({"contact": "BadCo", "iban": "NL00BAD0000000000", "amountCents": 100}),
            json!({"name": "X", "iban": IBAN, "amountCents": 0}),
            json!({"name": "", "iban": IBAN, "amountCents": 100}),
            json!({"name": "Y", "iban": IBAN, "amountCents": 100, "reference": "r".repeat(141)}),
        ];
        let e = create_payment_batch(&d, None, None, &lines, &[], "transfer", "a:t", false)
            .unwrap_err();
        assert_eq!(e.code, "BATCH_VALIDATION_FAILED");
        assert_eq!(e.details.unwrap().as_array().unwrap().len(), 4);
    }

    #[test]
    fn batch_rejects_sepa_names_over_70_chars() {
        let d = cdb();
        vendor(&d);
        let long =
            "Stichting Voor Het Behoud Van Historische Monumenten In De Provincie Noord-Holland";
        let lines = vec![json!({"name": long, "iban": IBAN, "amountCents": 100})];
        let e = create_payment_batch(&d, None, None, &lines, &[], "transfer", "a:t", false)
            .unwrap_err();
        assert_eq!(e.code, "BATCH_VALIDATION_FAILED");
        let has = e.details.unwrap().as_array().unwrap().iter().any(|x| {
            x["error"]
                .as_str()
                .unwrap_or("")
                .contains("SEPA_NAME_TOO_LONG")
        });
        assert!(has);
        let ok_lines = vec![json!({"name": &long[..70], "iban": IBAN, "amountCents": 100})];
        let ok =
            create_payment_batch(&d, None, None, &ok_lines, &[], "transfer", "a:t", false).unwrap();
        assert_eq!(ok["lines"][0]["name"].as_str().unwrap().len(), 70);
    }

    #[test]
    fn batch_payables_path_rejects_overlong_contact_names() {
        let d = cdb();
        let long =
            "Stichting Voor Het Behoud Van Historische Monumenten In De Provincie Noord-Holland";
        let c = crate::contacts::create_contact(
            &d,
            long,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(IBAN),
            "a:t",
            false,
        )
        .unwrap()["id"]
            .as_i64()
            .unwrap();
        add_payable(
            &d,
            &c.to_string(),
            "LONG",
            "2026-07-01",
            None,
            12100,
            "transfer",
            "a:t",
            false,
        )
        .unwrap();
        let e = create_payment_batch(&d, None, None, &[], &[1], "transfer", "agent:test", false)
            .unwrap_err();
        assert_eq!(e.code, "BATCH_VALIDATION_FAILED");
        let has = e.details.unwrap().as_array().unwrap().iter().any(|x| {
            x["error"]
                .as_str()
                .unwrap_or("")
                .contains("SEPA_NAME_TOO_LONG")
        });
        assert!(has);
    }

    #[test]
    fn batch_rejects_overlong_company_name() {
        let d = cdb();
        vendor(&d);
        d.execute(
            "UPDATE company SET name = ?1",
            ["Naamloze Vennootschap Voor Het Beheer Van Onroerende Zaken En Effecten In Amsterdam Zuidoost"],
        )
        .unwrap();
        let lines = vec![json!({"name": "Vimexx", "iban": IBAN, "amountCents": 100})];
        let e = create_payment_batch(&d, None, None, &lines, &[], "transfer", "a:t", false)
            .unwrap_err();
        assert_eq!(e.code, "SEPA_NAME_TOO_LONG");
        assert!(e.message.contains("company name"), "{}", e.message);
    }

    #[test]
    fn batch_from_payables_excludes_direct_debit_and_marks_in_batch() {
        let d = cdb();
        vendor(&d);
        add_payable(
            &d,
            "Vimexx",
            "A1",
            "2026-07-01",
            None,
            12100,
            "transfer",
            "a:t",
            false,
        )
        .unwrap();
        add_payable(
            &d,
            "Vimexx",
            "A2",
            "2026-07-01",
            None,
            4550,
            "transfer",
            "a:t",
            false,
        )
        .unwrap();
        add_payable(
            &d,
            "Vimexx",
            "DD",
            "2026-07-01",
            None,
            9999,
            "direct_debit",
            "a:t",
            false,
        )
        .unwrap();
        let ids: Vec<i64> = list_payables(&d, Some("unpaid"), Some("transfer"), None)
            .unwrap()
            .iter()
            .map(|p| p["id"].as_i64().unwrap())
            .collect();
        let b = create_payment_batch(&d, None, None, &[], &ids, "transfer", "agent:test", false)
            .unwrap();
        assert_eq!(b["lines"].as_array().unwrap().len(), 2);
        assert_eq!(b["total_cents"].as_i64(), Some(16650));
        let in_batch: Vec<String> = list_payables(&d, Some("in_batch"), None, None)
            .unwrap()
            .iter()
            .map(|p| p["invoice_ref"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(in_batch, vec!["A1".to_string(), "A2".to_string()]);
        let unpaid: Vec<String> = list_payables(&d, Some("unpaid"), None, None)
            .unwrap()
            .iter()
            .map(|p| p["invoice_ref"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(unpaid, vec!["DD".to_string()]);
        assert_eq!(
            crate::entries::list_entries(&d, None, None, None, 500)
                .unwrap()
                .len(),
            0
        );
    }

    #[test]
    fn batch_dry_run_writes_nothing_and_empty_batch_is_rejected() {
        let d = cdb();
        vendor(&d);
        let lines = vec![json!({"contact": "Vimexx", "amountCents": 100})];
        let plan =
            create_payment_batch(&d, None, None, &lines, &[], "transfer", "a:t", true).unwrap();
        assert_eq!(plan["dryRun"].as_bool(), Some(true));
        assert_eq!(plan["lines"].as_array().unwrap().len(), 1);
        assert_eq!(batch_count(&d), 0);
        let e =
            create_payment_batch(&d, None, None, &[], &[], "transfer", "a:t", false).unwrap_err();
        assert_eq!(code_of(&e), "EMPTY_BATCH");
    }

    #[test]
    fn batch_csv_supports_comma_and_semicolon_with_dutch_amounts() {
        let d = cdb();
        vendor(&d);
        let csv1 = "contact,amount,reference\nVimexx,121.00,Factuur A\nVimexx,45,Factuur B";
        let b1 = create_payment_batch_from_csv(&d, csv1, None, None, "agent:test", false).unwrap();
        assert_eq!(b1["lines"].as_array().unwrap().len(), 2);
        assert_eq!(b1["total_cents"].as_i64(), Some(16600));
        let csv2 = "contact;amount;reference\nVimexx;121,00;Factuur A\nVimexx;45,50;Factuur B";
        let b2 = create_payment_batch_from_csv(&d, csv2, None, None, "agent:test", false).unwrap();
        assert_eq!(b2["lines"].as_array().unwrap().len(), 2);
        assert_eq!(b2["total_cents"].as_i64(), Some(16650));
    }

    #[test]
    fn batch_csv_reports_every_bad_line_and_keeps_the_batch_unwritten() {
        let d = cdb();
        vendor(&d);
        let csv = "contact;amount;reference\nVimexx;abc;x\nNobody;10.00;y\nVimexx;-5;z\n";
        let e = create_payment_batch_from_csv(&d, csv, None, None, "a:t", false).unwrap_err();
        assert_eq!(e.code, "IMPORT_VALIDATION_FAILED");
        assert_eq!(e.details.unwrap().as_array().unwrap().len(), 2);
        assert_eq!(batch_count(&d), 0);
        let e = create_payment_batch_from_csv(
            &d,
            "contact;amount\nNobody;10.00",
            None,
            None,
            "a:t",
            false,
        )
        .unwrap_err();
        assert_eq!(e.code, "BATCH_VALIDATION_FAILED");
        let has = e.details.unwrap().as_array().unwrap().iter().any(|x| {
            x["error"]
                .as_str()
                .unwrap_or("")
                .contains("CONTACT_NOT_FOUND")
        });
        assert!(has);
    }

    #[test]
    fn batch_csv_detects_the_delimiter_once_from_the_first_line() {
        let csv = "contact,amount,reference\nA&B; Trading,100.00,ref-1\n";
        let r = parse_batch_csv(csv).unwrap();
        assert_eq!(r["errors"].as_array().unwrap().len(), 0, "{r}");
        assert_eq!(r["lines"].as_array().unwrap().len(), 1);
        assert_eq!(r["lines"][0]["contact"].as_str(), Some("A&B; Trading"));
        assert_eq!(r["lines"][0]["amountCents"].as_i64(), Some(10000));
    }

    #[test]
    fn export_writes_pain001_with_totals_sepa_level_and_escaping() {
        let d = cdb();
        vendor(&d);
        let lines = vec![
            json!({"contact": "Vimexx", "amountCents": 12100, "reference": "Factuur 2026-118"}),
        ];
        let b = create_payment_batch(&d, None, None, &lines, &[], "transfer", "agent:test", false)
            .unwrap();
        let id = b["id"].as_i64().unwrap();
        let r = export_payment_batch(&d, id, "agent:test", false, None).unwrap();
        assert_eq!(r["status"].as_str(), Some("exported"));
        let msg_id = r["msg_id"].as_str().unwrap();
        assert_eq!(msg_id.len(), 20); // BUKIO + 14 digits + the batch id
        assert!(msg_id.starts_with("BUKIO"));
        assert!(msg_id[5..].chars().all(|c| c.is_ascii_digit()));
        assert_eq!(r["file_hash"].as_str().unwrap().len(), 64);
        let xml = r["xml"].as_str().unwrap();
        assert!(xml.contains("urn:iso:std:iso:20022:tech:xsd:pain.001.001.03"));
        assert!(xml.contains("<MsgId>"));
        assert!(xml.contains("<NbOfTxs>1</NbOfTxs>"));
        assert!(xml.contains("<CtrlSum>121.00</CtrlSum>"));
        assert!(xml.contains("<SvcLvl><Cd>SEPA</Cd></SvcLvl>"));
        assert!(xml.contains("<ChrgBr>SLEV</ChrgBr>"));
        assert!(xml.contains(&format!("<IBAN>{IBAN}</IBAN>")));
        assert!(xml.contains("Factuur 2026-118"));
        let e = export_payment_batch(&d, id, "agent:test", false, None).unwrap_err();
        assert_eq!(code_of(&e), "BATCH_ALREADY_EXPORTED");
        let n: i64 = d
            .query_row(
                "SELECT COUNT(*) FROM audit_log WHERE action = 'payments.batch.export'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(n > 0);
        assert_eq!(
            crate::entries::list_entries(&d, None, None, None, 500)
                .unwrap()
                .len(),
            0
        );
    }

    #[test]
    fn export_escapes_xml_and_supports_the_09_schema() {
        let d = cdb();
        crate::contacts::create_contact(
            &d,
            "Amp & Sons",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some("NL86INGB0002445588"),
            "a:t",
            false,
        )
        .unwrap();
        let lines = vec![
            json!({"contact": "Amp & Sons", "amountCents": 1234, "reference": "a < b & c > d"}),
        ];
        let b =
            create_payment_batch(&d, None, None, &lines, &[], "transfer", "a:t", false).unwrap();
        let r = export_payment_batch(&d, b["id"].as_i64().unwrap(), "a:t", false, Some("001.09"))
            .unwrap();
        let xml = r["xml"].as_str().unwrap();
        assert!(xml.contains("pain.001.001.09"));
        assert!(xml.contains("Amp &amp; Sons"));
        assert!(xml.contains("a &lt; b &amp; c &gt; d"));
    }

    #[test]
    fn build_pain001_uses_the_batch_date_for_reqdexctndt() {
        let lines = vec![json!({"name": "X", "iban": IBAN, "amount_cents": 100})];
        let xml = build_pain001(
            "M1",
            "2026-08-06T10:00:00Z",
            "Demo BV",
            IBAN,
            "2026-08-10",
            &lines,
            "001.03",
        );
        assert!(xml.contains("<ReqdExctnDt>2026-08-10</ReqdExctnDt>"));
    }

    #[test]
    fn delete_only_removes_drafts_and_releases_payables() {
        let d = cdb();
        vendor(&d);
        add_payable(
            &d,
            "Vimexx",
            "K1",
            "2026-07-01",
            None,
            1000,
            "transfer",
            "a:t",
            false,
        )
        .unwrap();
        let b = create_payment_batch(&d, None, None, &[], &[1], "transfer", "agent:test", false)
            .unwrap();
        let in_batch: Vec<String> = list_payables(&d, Some("in_batch"), None, None)
            .unwrap()
            .iter()
            .map(|p| p["invoice_ref"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(in_batch, vec!["K1".to_string()]);
        let r = delete_payment_batch(&d, b["id"].as_i64().unwrap(), "agent:test", false).unwrap();
        assert_eq!(r["status"].as_str(), Some("deleted"));
        assert_eq!(batch_count(&d), 0);
        let unpaid: Vec<String> = list_payables(&d, Some("unpaid"), None, None)
            .unwrap()
            .iter()
            .map(|p| p["invoice_ref"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(unpaid, vec!["K1".to_string()]);
        let lines = vec![json!({"contact": "Vimexx", "amountCents": 100})];
        let b2 =
            create_payment_batch(&d, None, None, &lines, &[], "transfer", "a:t", false).unwrap();
        export_payment_batch(&d, b2["id"].as_i64().unwrap(), "a:t", false, None).unwrap();
        let e = delete_payment_batch(&d, b2["id"].as_i64().unwrap(), "a:t", false).unwrap_err();
        assert_eq!(code_of(&e), "BATCH_ALREADY_EXPORTED");
    }

    #[test]
    fn get_payment_batch_serializes_total_and_lines() {
        let d = cdb();
        vendor(&d);
        let lines = vec![json!({"contact": "Vimexx", "amountCents": 12100})];
        let b =
            create_payment_batch(&d, None, None, &lines, &[], "transfer", "a:t", false).unwrap();
        let fetched = get_payment_batch(&d, b["id"].as_i64().unwrap()).unwrap();
        assert_eq!(fetched["total"].as_str(), Some("121.00"));
        assert_eq!(fetched["lines"].as_array().unwrap().len(), 1);
        assert_eq!(fetched["lines"][0]["amount"].as_str(), Some("121.00"));
        assert_eq!(crate::contacts::list_contacts(&d).unwrap().len(), 1);
    }

    #[test]
    fn add_payable_rejects_the_same_contact_and_ref_while_unpaid() {
        let d = cdb();
        let v = vendor(&d);
        let cid = v.to_string();
        add_payable(
            &d,
            &cid,
            "F-2026-01",
            "2026-06-01",
            None,
            10000,
            "transfer",
            "agent:test",
            false,
        )
        .unwrap();
        let e = add_payable(
            &d,
            &cid,
            "F-2026-01",
            "2026-06-01",
            None,
            10000,
            "transfer",
            "agent:test",
            false,
        )
        .unwrap_err();
        assert_eq!(code_of(&e), "PAYABLE_DUPLICATE");
        let v2 = crate::contacts::create_contact(
            &d,
            "Ander BV",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some("NL02ABNA0123456789"),
            "agent:test",
            false,
        )
        .unwrap()["id"]
            .as_i64()
            .unwrap();
        add_payable(
            &d,
            &v2.to_string(),
            "F-2026-01",
            "2026-06-01",
            None,
            10000,
            "transfer",
            "agent:test",
            false,
        )
        .unwrap();
    }

    #[test]
    fn direct_debit_batches_require_a_mandate_that_reaches_the_line() {
        let d = cdb();
        let v = vendor(&d);
        let lines = vec![json!({"contact": v, "amountCents": 10000})];
        let e = create_payment_batch(
            &d,
            Some("2026-06-01"),
            Some(IBAN),
            &lines,
            &[],
            "direct_debit",
            "agent:test",
            false,
        )
        .unwrap_err();
        assert_eq!(code_of(&e), "BATCH_VALIDATION_FAILED");
        add_mandate(
            &d,
            v,
            "M-001",
            Some("2026-01-01"),
            "core",
            "agent:test",
            false,
        )
        .unwrap();
        let b = create_payment_batch(
            &d,
            Some("2026-06-01"),
            Some(IBAN),
            &lines,
            &[],
            "direct_debit",
            "agent:test",
            false,
        )
        .unwrap();
        assert_eq!(b["lines"][0]["mandate_ref"].as_str(), Some("M-001"));
    }
}
