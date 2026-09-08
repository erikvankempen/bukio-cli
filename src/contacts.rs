// bukio-cli — agent-first double-entry bookkeeping for SMEs.
// Copyright (c) 2026 Erik van Kempen.
// SPDX-License-Identifier: Apache-2.0
//
// Contacts CRUD (mirrors contacts functions in src/invoice/index.js).

use crate::accounts::resolve_profile;
use crate::audit::{record, RecordArgs};
use crate::dates::is_valid_iban;
use crate::money::{BukioError, Result};
use rusqlite::Connection;
use serde_json::{json, Value};

fn contact_error(code: &'static str, msg: impl Into<String>) -> BukioError {
    BukioError::new(code, msg.into())
}

fn sql_err(e: rusqlite::Error) -> BukioError {
    BukioError::new("DB_ERROR", e.to_string())
}

/// Column order after all migrations:
/// 0:id 1:name 2:address 3:postal_code 4:city 5:country 6:email
/// 7:vat_id 8:kvk 9:created_by 10:created_at 11:iban (migration 010)
fn serialize_contact(row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    Ok(json!({
        "id": row.get::<_, i64>(0)?,
        "name": row.get::<_, String>(1)?,
        "address": row.get::<_, Option<String>>(2)?,
        "postal_code": row.get::<_, Option<String>>(3)?,
        "city": row.get::<_, Option<String>>(4)?,
        "country": row.get::<_, Option<String>>(5)?,
        "email": row.get::<_, Option<String>>(6)?,
        "vat_id": row.get::<_, Option<String>>(7)?,
        "kvk": row.get::<_, Option<String>>(8)?,
        "created_by": row.get::<_, Option<String>>(9)?,
        "iban": row.get::<_, Option<String>>(11)?,
    }))
}

pub fn get_contact(db: &Connection, id: i64) -> Result<Option<Value>> {
    let r = db.query_row("SELECT * FROM contacts WHERE id = ?1", [id], |r| {
        serialize_contact(r)
    });
    match r {
        Ok(v) => Ok(Some(v)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(contact_error("DB_ERROR", e.to_string())),
    }
}

pub fn list_contacts(db: &Connection) -> Result<Vec<Value>> {
    let mut stmt = db
        .prepare("SELECT * FROM contacts ORDER BY name")
        .map_err(sql_err)?;
    let rows = stmt
        .query_map([], |r| serialize_contact(r))
        .map_err(sql_err)?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

pub fn create_contact(
    db: &Connection,
    name: &str,
    address: Option<&str>,
    postal_code: Option<&str>,
    city: Option<&str>,
    country: Option<&str>,
    email: Option<&str>,
    vat_id: Option<&str>,
    kvk: Option<&str>,
    iban: Option<&str>,
    actor: &str,
    dry_run: bool,
) -> Result<Value> {
    if name.trim().is_empty() {
        return Err(contact_error("INVALID_NAME", "contact needs a name"));
    }
    if let Some(ib) = iban {
        if !is_valid_iban(ib) {
            return Err(contact_error(
                "INVALID_IBAN",
                format!("'{ib}' is not a valid IBAN"),
            ));
        }
    }
    let cc_owned = country
        .map(String::from)
        .or_else(|| {
            resolve_profile(db)
                .ok()
                .and_then(|p| p["meta"]["country"].as_str().map(String::from))
        })
        .unwrap_or_else(|| "NL".to_string());
    let clean_iban = iban.map(|i| i.replace([' ', '-'], ""));
    if dry_run {
        return Ok(json!({
            "action": "contact.create", "name": name, "address": address,
            "postal_code": postal_code, "city": city, "country": cc_owned,
            "email": email, "vat_id": vat_id, "kvk": kvk, "iban": clean_iban, "dryRun": true
        }));
    }
    db.execute(
        "INSERT INTO contacts (name, address, postal_code, city, country, email, vat_id, kvk, iban, created_by) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        rusqlite::params![name, address, postal_code, city, cc_owned.as_str(), email, vat_id, kvk, clean_iban.as_deref(), actor],
    ).map_err(sql_err)?;
    let id = db.last_insert_rowid();
    record(
        db,
        RecordArgs {
            actor,
            action: "contact.create",
            command: Some("contact add"),
            args: Some(json!({"name": name})),
            outcome: "ok",
            entry_ids: vec![],
        },
    )?;
    get_contact(db, id)?.ok_or_else(|| contact_error("DB_ERROR", "contact not found after insert"))
}

pub fn update_contact(
    db: &Connection,
    id: i64,
    name: Option<&str>,
    address: Option<&str>,
    postal_code: Option<&str>,
    city: Option<&str>,
    country: Option<&str>,
    email: Option<&str>,
    vat_id: Option<&str>,
    kvk: Option<&str>,
    iban: Option<&str>,
    actor: &str,
    dry_run: bool,
) -> Result<Value> {
    let existing = get_contact(db, id)?.ok_or_else(|| {
        contact_error("CONTACT_NOT_FOUND", format!("contact {id} does not exist"))
    })?;
    let new_name = name.unwrap_or(existing["name"].as_str().unwrap_or(""));
    let new_address = address
        .map(String::from)
        .or_else(|| existing["address"].as_str().map(String::from));
    let new_postal = postal_code
        .map(String::from)
        .or_else(|| existing["postal_code"].as_str().map(String::from));
    let new_city = city
        .map(String::from)
        .or_else(|| existing["city"].as_str().map(String::from));
    let new_country = country
        .map(String::from)
        .or_else(|| existing["country"].as_str().map(String::from));
    let new_email = email
        .map(String::from)
        .or_else(|| existing["email"].as_str().map(String::from));
    let new_vat = vat_id
        .map(String::from)
        .or_else(|| existing["vat_id"].as_str().map(String::from));
    let new_kvk = kvk
        .map(String::from)
        .or_else(|| existing["kvk"].as_str().map(String::from));
    let new_iban = if iban.is_some() {
        iban.map(|i| i.replace([' ', '-'], ""))
    } else {
        existing["iban"].as_str().map(String::from)
    };
    if let Some(ref ib) = new_iban {
        if !is_valid_iban(ib) {
            return Err(contact_error(
                "INVALID_IBAN",
                format!("'{ib}' is not a valid IBAN"),
            ));
        }
    }
    if dry_run {
        return Ok(json!({"action": "contact.update", "id": id, "dryRun": true}));
    }
    db.execute(
        "UPDATE contacts SET name = ?1, address = ?2, postal_code = ?3, city = ?4, country = ?5, email = ?6, vat_id = ?7, kvk = ?8, iban = ?9 WHERE id = ?10",
        rusqlite::params![new_name, new_address, new_postal, new_city, new_country, new_email, new_vat, new_kvk, new_iban, id],
    ).map_err(sql_err)?;
    record(
        db,
        RecordArgs {
            actor,
            action: "contact.update",
            command: Some("contact update"),
            args: Some(json!({"contact_id": id})),
            outcome: "ok",
            entry_ids: vec![],
        },
    )?;
    get_contact(db, id)?.ok_or_else(|| contact_error("DB_ERROR", "contact not found after update"))
}

// ── contact statement (opgave) ────────────────────────────────────────────

pub fn contact_statement(
    db: &Connection,
    contact_id: i64,
    as_of: Option<&str>,
) -> Result<Value> {
    let contact = get_contact(db, contact_id)?
        .ok_or_else(|| contact_error("CONTACT_NOT_FOUND", format!("contact {contact_id} does not exist")))?;
    let as_of_date = match as_of {
        Some(d) => d.to_string(),
        None => chrono::Utc::now().format("%Y-%m-%d").to_string(),
    };
    if !as_of_date.chars().all(|c| c.is_ascii_digit() || c == '-') || as_of_date.len() != 10 {
        return Err(contact_error("INVALID_DATE", format!("as-of '{as_of_date}' must be YYYY-MM-DD")));
    }

    let mut rows: Vec<Value> = Vec::new();

    // Invoices for this contact (sales + credit, non-draft, non-void, date <= as_of)
    {
        let mut stmt = db
            .prepare(
                "SELECT id, invoice_number, invoice_type, date, description, status \
                 FROM invoices WHERE contact_id = ?1 \
                 AND invoice_type IN ('sales','credit') \
                 AND status NOT IN ('draft','void') AND date <= ?2 \
                 ORDER BY date, id",
            )
            .map_err(|e| contact_error("DB_ERROR", e.to_string()))?;
        let inv_rows = stmt
            .query_map(rusqlite::params![contact_id, as_of_date], |r| {
                Ok(json!({
                    "id": r.get::<_, i64>(0)?,
                    "invoice_number": r.get::<_, Option<String>>(1)?,
                    "invoice_type": r.get::<_, String>(2)?,
                    "date": r.get::<_, String>(3)?,
                    "description": r.get::<_, Option<String>>(4)?,
                    "status": r.get::<_, String>(5)?,
                }))
            })
            .map_err(|e| contact_error("DB_ERROR", e.to_string()))?;
        for inv in inv_rows.filter_map(|r| r.ok()) {
            let iid = inv["id"].as_i64().unwrap();
            let is_credit = inv["invoice_type"].as_str() == Some("credit");
            // Get payments for this invoice
            let payments = get_invoice_payments(db, iid)?;
            // Compute gross from line items
            let gross = compute_gross(db, iid)?;
            let inv_num = inv["invoice_number"].as_str().unwrap_or("");
            let desc = if is_credit {
                format!("Credit note {inv_num}").trim().to_string()
            } else {
                inv["description"]
                    .as_str()
                    .unwrap_or(&format!("Invoice {inv_num}"))
                    .to_string()
            };
            rows.push(json!({
                "date": inv["date"],
                "kind": if is_credit { "credit" } else { "invoice" },
                "ref": inv_num,
                "description": desc,
                "debit_cents": if is_credit { 0 } else { gross },
                "credit_cents": if is_credit { gross } else { 0 },
                "balance_cents": 0,
            }));
            for p in payments {
                let pdate = p["date"].as_str().unwrap_or("");
                if pdate <= as_of_date.as_str() {
                    let pamt = p["amount_cents"].as_i64().unwrap_or(0);
                    let pdesc = if is_credit {
                        format!("Refund {inv_num}")
                    } else {
                        format!("Payment {inv_num}")
                    };
                    rows.push(json!({
                        "date": pdate, "kind": "payment", "ref": inv_num,
                        "description": pdesc,
                        "debit_cents": if is_credit { pamt } else { 0 },
                        "credit_cents": if is_credit { 0 } else { pamt },
                        "balance_cents": 0,
                    }));
                }
            }
        }
    }

    // Payables for this contact
    {
        let mut stmt = db
            .prepare(
                "SELECT id, invoice_ref, date, amount_cents FROM payables \
                 WHERE contact_id = ?1 AND date <= ?2 ORDER BY date, id",
            )
            .map_err(|e| contact_error("DB_ERROR", e.to_string()))?;
        let pay_rows = stmt
            .query_map(rusqlite::params![contact_id, as_of_date], |r| {
                Ok(json!({
                    "id": r.get::<_, i64>(0)?,
                    "invoice_ref": r.get::<_, String>(1)?,
                    "date": r.get::<_, String>(2)?,
                    "amount_cents": r.get::<_, i64>(3)?,
                }))
            })
            .map_err(|e| contact_error("DB_ERROR", e.to_string()))?;
        for p in pay_rows.filter_map(|r| r.ok()) {
            let pref = p["invoice_ref"].as_str().unwrap_or("");
            rows.push(json!({
                "date": p["date"], "kind": "payable", "ref": pref,
                "description": format!("Purchase invoice {pref}"),
                "debit_cents": 0, "credit_cents": p["amount_cents"],
                "balance_cents": 0,
            }));
        }
    }

    rows.sort_by(|a, b| {
        a["date"]
            .as_str()
            .unwrap_or("")
            .cmp(b["date"].as_str().unwrap_or(""))
            .then(a["kind"].as_str().unwrap_or("").cmp(b["kind"].as_str().unwrap_or("")))
    });
    let mut balance: i64 = 0;
    for r in rows.iter_mut() {
        balance += r["debit_cents"].as_i64().unwrap_or(0) - r["credit_cents"].as_i64().unwrap_or(0);
        r["balance_cents"] = json!(balance);
    }

    let cname = contact["name"].as_str().unwrap_or("");
    let cid = contact["id"].as_i64().unwrap_or(contact_id);
    Ok(json!({
        "contact": {"id": cid, "name": cname},
        "as_of": as_of_date, "rows": rows, "balance_cents": balance,
    }))
}

fn get_invoice_payments(db: &Connection, invoice_id: i64) -> Result<Vec<Value>> {
    let mut stmt = db
        .prepare(
            "SELECT date, amount_cents FROM invoice_payments \
             WHERE invoice_id = ?1 ORDER BY date, id",
        )
        .map_err(|e| contact_error("DB_ERROR", e.to_string()))?;
    let rows = stmt
        .query_map([invoice_id], |r| {
            Ok(json!({
                "date": r.get::<_, String>(0)?,
                "amount_cents": r.get::<_, i64>(1)?,
            }))
        })
        .map_err(|e| contact_error("DB_ERROR", e.to_string()))?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

fn compute_gross(db: &Connection, invoice_id: i64) -> Result<i64> {
    // Reuse invoice totals computation — get the invoice and compute
    let inv = crate::invoice::get_invoice(db, invoice_id)?
        .ok_or_else(|| contact_error("DB_ERROR", format!("invoice {invoice_id} not found")))?;
    Ok(inv["gross_cents"].as_i64().unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_db;

    fn test_db() -> Connection {
        let d = open_db(":memory:").unwrap();
        d.execute("INSERT INTO company (name) VALUES ('ContactCo')", [])
            .unwrap();
        crate::accounts::seed_default_chart(&d).unwrap();
        d
    }

    #[test]
    fn create_and_get_contact() {
        let d = test_db();
        let c = create_contact(
            &d,
            "ACME BV",
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
        assert_eq!(c["name"], "ACME BV");
        assert_eq!(c["country"], "NL");
    }

    #[test]
    fn empty_name_rejected() {
        let d = test_db();
        let err = create_contact(
            &d,
            " ",
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
        .unwrap_err();
        assert_eq!(err.code, "INVALID_NAME");
    }

    #[test]
    #[test]
    fn update_contact_name() {
        let d = test_db();
        let c = create_contact(
            &d,
            "Old",
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
        let id = c["id"].as_i64().unwrap();
        let updated = update_contact(
            &d,
            id,
            Some("New"),
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
        assert_eq!(updated["name"], "New");
    }
}
