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
    Ok(json!({"id": id, "contact_id": contact_id, "mandate_ref": ref_trimmed, "scheme": scheme}))
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
    Ok(
        json!({"id": id, "contact_id": c_id, "invoice_ref": ref_trimmed, "amount_cents": amount_cents, "method": method}),
    )
}

pub fn list_payables(
    db: &Connection,
    status: Option<&str>,
    method: Option<&str>,
    contact_id: Option<i64>,
) -> Result<Vec<Value>> {
    let mut sql = "SELECT p.id, p.contact_id, c.name AS contact_name, p.invoice_ref, p.date, p.due_date, p.amount_cents, p.payment_method, p.status, p.entry_id, p.batch_line_id, p.created_by, p.created_at FROM payables p JOIN contacts c ON c.id = p.contact_id".to_string();
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
        Ok(json!({"id": r.get::<_, i64>(0)?, "contact_id": r.get::<_, i64>(1)?, "contact_name": r.get::<_, String>(2)?, "invoice_ref": r.get::<_, String>(3)?, "date": r.get::<_, String>(4)?, "due_date": r.get::<_, Option<String>>(5)?, "amount_cents": r.get::<_, i64>(6)?, "payment_method": r.get::<_, String>(7)?, "status": r.get::<_, String>(8)?}))
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
        Ok(json!({"id": r.get::<_, i64>(0)?, "batch_date": r.get::<_, String>(1)?, "debit_iban": r.get::<_, String>(2)?, "debit_name": r.get::<_, String>(3)?, "total_cents": r.get::<_, i64>(4)?, "status": r.get::<_, String>(5)?, "msg_id": r.get::<_, Option<String>>(6)?, "file_hash": r.get::<_, Option<String>>(7)?, "schema": r.get::<_, Option<String>>(8)?, "created_by": r.get::<_, String>(9)?, "created_at": r.get::<_, String>(10)?, "exported_at": r.get::<_, Option<String>>(11)?, "batch_kind": r.get::<_, String>(12)?}))
    }).map_err(|_| payments_error("BATCH_NOT_FOUND", format!("batch {id} does not exist")))?;

    let mut stmt = db.prepare("SELECT id, batch_id, contact_id, name, iban, amount_cents, reference, mandate_id, mandate_ref, mandate_seq, mandate_date, scheme FROM payment_batch_lines WHERE batch_id = ?1 ORDER BY id").map_err(sql_err)?;
    let lines: Vec<Value> = stmt.query_map([id], |r| {
        Ok(json!({"id": r.get::<_, i64>(0)?, "name": r.get::<_, String>(3)?, "iban": r.get::<_, String>(4)?, "amount_cents": r.get::<_, i64>(5)?, "reference": r.get::<_, Option<String>>(6)?, "mandate_id": r.get::<_, Option<i64>>(7)?, "mandate_ref": r.get::<_, Option<String>>(8)?, "mandate_seq": r.get::<_, Option<String>>(9)?, "mandate_date": r.get::<_, Option<String>>(10)?, "scheme": r.get::<_, Option<String>>(11)?}))
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
            "no valid company IBAN",
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
    let mut errors: Vec<String> = Vec::new();

    for (i, l) in lines.iter().enumerate() {
        let line_no = format!("line {}", i + 1);
        let name = l["name"].as_str();
        let iban_raw = l["iban"].as_str().map(normalize_iban);
        let amount = l["amountCents"].as_i64().unwrap_or(0);
        let reference = l["reference"].as_str();

        let (contact_id, resolved_name, resolved_iban) = if let Some(cr) = l["contact"].as_str() {
            match resolve_contact(db, cr)? {
                Some(c) => {
                    let cid = c["id"].as_i64().unwrap_or(0);
                    let n = name.unwrap_or(c["name"].as_str().unwrap_or(""));
                    let ib = iban_raw
                        .or_else(|| c["iban"].as_str().map(normalize_iban))
                        .unwrap_or_default();
                    if ib.is_empty() {
                        errors.push(format!("{line_no}: CONTACT_IBAN_MISSING"));
                        continue;
                    }
                    (Some(cid), n.to_string(), ib)
                }
                None => {
                    errors.push(format!("{line_no}: CONTACT_NOT_FOUND"));
                    continue;
                }
            }
        } else {
            (
                None,
                name.unwrap_or("").to_string(),
                iban_raw.unwrap_or_default(),
            )
        };

        if amount <= 0 {
            errors.push(format!("{line_no}: INVALID_AMOUNT"));
            continue;
        }
        if resolved_name.is_empty() {
            errors.push(format!("{line_no}: NAME_REQUIRED"));
            continue;
        }
        if !is_valid_iban(&resolved_iban) {
            errors.push(format!("{line_no}: INVALID_IBAN"));
            continue;
        }

        let mut item = json!({"contact_id": contact_id, "name": resolved_name, "iban": resolved_iban, "amount_cents": amount, "reference": reference});
        if kind == "direct_debit" {
            let mandate = contact_id.and_then(|cid| latest_mandate(db, cid));
            match mandate {
                Some(m) => {
                    let mandate_id = m["id"].as_i64();
                    item["mandate_id"] = mandate_id.clone().into();
                    item["mandate_ref"] = m["mandate_ref"].clone();
                    item["mandate_date"] = m["mandate_date"].clone();
                    item["scheme"] = m["scheme"].clone();
                }
                None => {
                    errors.push(format!("{line_no}: MANDATE_REQUIRED"));
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
        match p {
            Ok(p) => {
                if p["status"] != "unpaid" {
                    errors.push(format!("{line_no}: PAYABLE_NOT_UNPAID"));
                    continue;
                }
                let c =
                    get_contact(db, p["contact_id"].as_i64().unwrap_or(0))?.unwrap_or(Value::Null);
                let iban = normalize_iban(c["iban"].as_str().unwrap_or(""));
                if iban.is_empty() || !is_valid_iban(&iban) {
                    errors.push(format!("{line_no}: CONTACT_IBAN_MISSING"));
                    continue;
                }
                let mut item = json!({"payable_id": pid, "contact_id": p["contact_id"], "name": c["name"], "iban": iban, "amount_cents": p["amount_cents"], "reference": format!("Invoice {}", p["invoice_ref"])});
                if kind == "direct_debit" {
                    let mandate = latest_mandate(db, p["contact_id"].as_i64().unwrap_or(0));
                    match mandate {
                        Some(m) => {
                            let mid = m["id"].as_i64().unwrap_or(0);
                            let used: i64 = db.prepare("SELECT COUNT(*) FROM payment_batch_lines WHERE mandate_id = ?1")
                                .map_err(sql_err)?
                                .query_row([mid], |r| r.get(0))
                                .unwrap_or(0);
                            let seq = if used > 0 { "RCUR" } else { "FRST" };
                            item["mandate_id"] = m["id"].clone();
                            item["mandate_ref"] = m["mandate_ref"].clone();
                            item["mandate_seq"] = json!(seq);
                            item["mandate_date"] = m["mandate_date"].clone();
                            item["scheme"] = m["scheme"].clone();
                        }
                        None => {
                            errors.push(format!("{line_no}: MANDATE_REQUIRED"));
                            continue;
                        }
                    }
                }
                items.push(item);
            }
            Err(_) => {
                errors.push(format!("{line_no}: PAYABLE_NOT_FOUND"));
            }
        }
    }

    if items.is_empty() && errors.is_empty() {
        return Err(payments_error("EMPTY_BATCH", "no payments to batch"));
    }
    if !errors.is_empty() {
        return Err(payments_error("BATCH_VALIDATION_FAILED", errors.join("; ")));
    }

    let total: i64 = items
        .iter()
        .map(|l| l["amount_cents"].as_i64().unwrap_or(0))
        .sum();

    if dry_run {
        return Ok(
            json!({"action": "payments.batch.create", "batch_date": batch_date, "debit_iban": debit, "batch_kind": kind, "total_cents": total, "lines": items, "dryRun": true}),
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
}
