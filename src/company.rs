// bukio-cli — agent-first double-entry bookkeeping for SMEs.
// Copyright (c) 2026 Erik van Kempen.
// SPDX-License-Identifier: Apache-2.0
//
// Company settings (mirrors src/cli/company.js): show, update, logo.

use crate::audit::{record, RecordArgs};
use crate::dates::is_valid_iban;
use crate::money::{BukioError, Result};
use rusqlite::Connection;
use serde_json::{json, Value};

const COMPANY_FIELDS: &[(&str, &str, &str)] = &[
    ("name", "name", "company name"),
    ("registration-id", "registration_id", "registration id"),
    ("tax-id", "tax_id", "tax id"),
    ("iban", "iban", "bank account (IBAN)"),
    ("address", "address", "street address"),
    ("postal-code", "postal_code", "postal code"),
    ("city", "city", "city"),
];

/// Serialize a company row to JSON (without the logo blob).
/// Column order after migration 022: id(0) name(1) registration_id(2) legal_form(3)
/// tax_id(4) iban(5) vat_module(6) kor_flag(7) fiscal_year_end(8) created_at(9)
/// updated_at(10) address(11) postal_code(12) city(13) logo(14) logo_mime(15)
/// country(16) base_currency(17) locale(18) profile_version(19)
pub fn serialize_company(row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    let logo_bytes: Option<Vec<u8>> = row.get(14)?;
    Ok(json!({
        "id": row.get::<_, i64>(0)?,
        "name": row.get::<_, String>(1)?,
        "registration_id": row.get::<_, Option<String>>(2)?,
        "legal_form": row.get::<_, Option<String>>(3)?,
        "tax_id": row.get::<_, Option<String>>(4)?,
        "iban": row.get::<_, Option<String>>(5)?,
        "address": row.get::<_, Option<String>>(11)?,
        "postal_code": row.get::<_, Option<String>>(12)?,
        "city": row.get::<_, Option<String>>(13)?,
        "vat_module": row.get::<_, i64>(6)?,
        "kor_flag": row.get::<_, i64>(7)?,
        "fiscal_year_end": row.get::<_, Option<String>>(8)?,
        "country": row.get::<_, Option<String>>(16)?,
        "base_currency": row.get::<_, Option<String>>(17)?,
        "locale": row.get::<_, Option<String>>(18)?,
        "profile_version": row.get::<_, i64>(19)?,
        "logo_mime": row.get::<_, Option<String>>(15)?,
        "logo_bytes": logo_bytes.as_ref().map(|b| b.len() as i64),
    }))
}

pub fn get_company(db: &Connection) -> Result<Value> {
    let row = db
        .query_row("SELECT * FROM company WHERE id = 1", [], |r| serialize_company(r))
        .map_err(|_| BukioError::new("NO_COMPANY", "no company — run bukio init first"))?;
    Ok(row)
}

/// Update company fields. Returns the updated company and a map of changes.
pub fn update_company(
    db: &Connection,
    changes: &[(String, String)], // (column, value) pairs
    logo_bytes: Option<Vec<u8>>,
    logo_mime: Option<&str>,
    actor: &str,
) -> Result<(Value, Value)> {
    let row = get_company(db)?;

    // validate each change
    let mut updates: Vec<(String, String)> = Vec::new();
    for (col, val) in changes {
        if val.is_empty() && col != "tax_id" {
            let label = COMPANY_FIELDS.iter().find(|(opt, c, _)| *c == *col).map(|f| f.2).unwrap_or("field");
            return Err(BukioError::new("INVALID_VALUE", format!("{label} cannot be empty")));
        }
        if col == "iban" && !val.is_empty() && !is_valid_iban(val) {
            return Err(BukioError::new("INVALID_IBAN", format!("invalid IBAN '{val}'")));
        }
        updates.push((col.clone(), val.clone()));
    }

    if updates.is_empty() && logo_bytes.is_none() && logo_mime.is_none() {
        return Err(BukioError::new("NOTHING_TO_UPDATE", "nothing to update"));
    }

    // apply text field changes
    if !updates.is_empty() {
        let set_clauses: Vec<String> = updates.iter().map(|(c, _)| format!("{c} = ?")).collect();
        let sql = format!("UPDATE company SET {} WHERE id = 1", set_clauses.join(", "));
        let params: Vec<String> = updates.iter().map(|(_, v)| v.clone()).collect();
        let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p as &dyn rusqlite::types::ToSql).collect();
        db.execute(&sql, param_refs.as_slice()).map_err(sql_err)?;
    }

    // apply logo change
    if logo_bytes.is_some() || logo_mime.is_some() {
        db.execute(
            "UPDATE company SET logo = ?1, logo_mime = ?2 WHERE id = 1",
            rusqlite::params![logo_bytes, logo_mime],
        )
        .map_err(sql_err)?;
    }

    let updated = get_company(db)?;

    // build changes map for audit
    let changes_map = serde_json::Map::from_iter(
        updates.iter().map(|(k, v)| (k.clone(), Value::String(v.clone()))),
    );
    let changes_map = Value::Object(changes_map);

    record(
        db,
        RecordArgs {
            actor,
            action: "company.update",
            command: Some("company update"),
            args: Some(changes_map.clone()),
            outcome: "ok",
            entry_ids: vec![],
        },
    )?;

    Ok((updated, changes_map))
}

/// Extract the stored logo.
pub fn get_logo(db: &Connection) -> Result<(Vec<u8>, String)> {
    let row = db
        .query_row(
            "SELECT logo, logo_mime FROM company WHERE id = 1 AND logo IS NOT NULL",
            [],
            |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, String>(1)?)),
        )
        .map_err(|_| BukioError::new("LOGO_NOT_SET", "no logo stored"))?;
    Ok(row)
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
        d.execute("INSERT INTO company (name) VALUES ('Test BV')", []).unwrap();
        d
    }

    #[test]
    fn get_company_basic() {
        let d = db();
        let c = get_company(&d).unwrap();
        assert_eq!(c["name"], "Test BV");
    }

    #[test]
    fn update_name() {
        let d = db();
        let (updated, changes) = update_company(&d, &[("name".into(), "New BV".into())], None, None, "human:erik").unwrap();
        assert_eq!(updated["name"], "New BV");
        assert_eq!(changes["name"], "New BV");
    }

    #[test]
    fn update_empty_name_rejected() {
        let d = db();
        let err = update_company(&d, &[("name".into(), "".into())], None, None, "human:erik").unwrap_err();
        assert_eq!(err.code, "INVALID_VALUE");
    }

    #[test]
    fn nothing_to_update_rejected() {
        let d = db();
        let err = update_company(&d, &[], None, None, "human:erik").unwrap_err();
        assert_eq!(err.code, "NOTHING_TO_UPDATE");
    }
}
