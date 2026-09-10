// bukio-cli — agent-first double-entry bookkeeping for SMEs.
// Copyright (c) 2026 Erik van Kempen.
// SPDX-License-Identifier: Apache-2.0
//
// Items catalog (mirrors src/items/index.js).

use crate::accounts::get_account_by_code;
use crate::audit::{record, RecordArgs};
use crate::money::{BukioError, Result};
use crate::vat::{is_vat_enabled, list_vat_codes};
use rusqlite::Connection;
use serde_json::{json, Value};

const UNIT_CODES: &[&str] = &[
    "h", "day", "month", "unit", "session", "km", "kg", "project",
];

fn item_error(code: &'static str, msg: impl Into<String>) -> BukioError {
    BukioError::new(code, msg.into())
}

pub fn get_item(db: &Connection, id: i64) -> Result<Option<Value>> {
    let result = db.query_row(
        "SELECT id, name, description, unit, unit_price_cents, vat_code, gl_account, active, created_by, created_at, updated_at
         FROM items WHERE id = ?1",
        [id],
        |r| {
            Ok(json!({
                "id": r.get::<_, i64>(0)?,
                "name": r.get::<_, String>(1)?,
                "description": r.get::<_, Option<String>>(2)?,
                "unit": r.get::<_, String>(3)?,
                "unit_price_cents": r.get::<_, i64>(4)?,
                "vat_code": r.get::<_, Option<String>>(5)?,
                "gl_account": r.get::<_, Option<String>>(6)?,
                "active": r.get::<_, i64>(7)? == 1,
                "created_by": r.get::<_, Option<String>>(8)?,
                "created_at": r.get::<_, Option<String>>(9)?,
                "updated_at": r.get::<_, Option<String>>(10)?,
            }))
        },
    );
    match result {
        Ok(v) => Ok(Some(v)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(BukioError::new("DB_ERROR", e.to_string())),
    }
}

pub fn list_items(db: &Connection, active_only: bool) -> Result<Vec<Value>> {
    let sql = if active_only {
        "SELECT id, name, description, unit, unit_price_cents, vat_code, gl_account, active
         FROM items WHERE active = 1 ORDER BY name"
    } else {
        "SELECT id, name, description, unit, unit_price_cents, vat_code, gl_account, active
         FROM items ORDER BY name"
    };
    let mut stmt = db.prepare(sql).map_err(sql_err)?;
    let rows = stmt
        .query_map([], |r| {
            Ok(json!({
                "id": r.get::<_, i64>(0)?,
                "name": r.get::<_, String>(1)?,
                "description": r.get::<_, Option<String>>(2)?,
                "unit": r.get::<_, String>(3)?,
                "unit_price_cents": r.get::<_, i64>(4)?,
                "vat_code": r.get::<_, Option<String>>(5)?,
                "gl_account": r.get::<_, Option<String>>(6)?,
                "active": r.get::<_, i64>(7)? == 1,
            }))
        })
        .map_err(sql_err)?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

fn validate_item(
    name: &str,
    unit: &str,
    unit_price_cents: i64,
    vat_code: Option<&str>,
) -> Result<()> {
    if name.trim().is_empty() {
        return Err(item_error("INVALID_NAME", "item needs a name"));
    }
    if !UNIT_CODES.contains(&unit) {
        return Err(item_error(
            "INVALID_UNIT",
            format!("unit '{unit}' must be one of: {}", UNIT_CODES.join(", ")),
        ));
    }
    if unit_price_cents <= 0 {
        return Err(item_error(
            "INVALID_PRICE",
            "unit price must be positive cents",
        ));
    }
    if let Some(vc) = vat_code {
        // a dotted RATE is legal (FR 5.5 / 2.1) but only one dot, with digits on
        // both sides — '5..5' used to pass because any dot was allowed
        let ok = if !vc.is_empty() && vc.chars().all(|c| c.is_ascii_digit() || c == '.') {
            let parts: Vec<&str> = vc.split('.').collect();
            parts.len() <= 2 && parts.iter().all(|x| !x.is_empty())
        } else {
            !vc.is_empty()
                && vc
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        };
        if !ok {
            return Err(item_error(
                "INVALID_VAT_CODE",
                format!("vat code '{vc}' is malformed"),
            ));
        }
    }
    Ok(())
}

pub fn create_item(
    db: &Connection,
    name: &str,
    description: Option<&str>,
    unit: &str,
    unit_price_cents: i64,
    vat_code: Option<&str>,
    gl_account: Option<&str>,
    actor: &str,
    dry_run: bool,
) -> Result<Value> {
    validate_item(name, unit, unit_price_cents, vat_code)?;
    if let Some(vc) = vat_code {
        if !is_vat_enabled(db) {
            return Err(item_error(
                "VAT_MODULE_OFF",
                "item has a VAT code but the VAT module is off",
            ));
        }
        let known = list_vat_codes(db)?.iter().any(|c| c["code"] == vc);
        if !known {
            return Err(item_error(
                "VAT_CODE_NOT_FOUND",
                format!("vat code '{vc}' does not exist"),
            ));
        }
    }
    if let Some(acct) = gl_account {
        if get_account_by_code(db, acct).is_none() {
            return Err(item_error(
                "ACCOUNT_NOT_FOUND",
                format!("account '{acct}' does not exist"),
            ));
        }
    }
    if dry_run {
        return Ok(json!({
            "action": "item.create", "name": name, "description": description,
            "unit": unit, "unit_price_cents": unit_price_cents,
            "vat_code": vat_code, "gl_account": gl_account, "dryRun": true,
        }));
    }
    db.execute(
        "INSERT INTO items (name, description, unit, unit_price_cents, vat_code, gl_account, created_by)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![name.trim(), description, unit, unit_price_cents, vat_code, gl_account, actor],
    )
    .map_err(sql_err)?;
    let id = db.last_insert_rowid();
    record(
        db,
        RecordArgs {
            actor,
            action: "item.create",
            command: Some("item add"),
            args: Some(
                json!({ "name": name.trim(), "unit": unit, "unit_price_cents": unit_price_cents, "vat_code": vat_code, "gl_account": gl_account }),
            ),
            outcome: "ok",
            entry_ids: vec![],
        },
    )?;
    get_item(db, id)?.ok_or_else(|| item_error("DB_ERROR", "insert succeeded but item not found"))
}

pub fn update_item(
    db: &Connection,
    id: i64,
    name: Option<&str>,
    description: Option<String>,
    unit: Option<&str>,
    unit_price_cents: Option<i64>,
    vat_code: Option<String>,
    gl_account: Option<String>,
    deactivate: bool,
    actor: &str,
    dry_run: bool,
) -> Result<Value> {
    let existing = get_item(db, id)?
        .ok_or_else(|| item_error("ITEM_NOT_FOUND", format!("item {id} does not exist")))?;

    let next_name = name.unwrap_or(existing["name"].as_str().unwrap_or(""));
    let next_desc = description.or(existing["description"].as_str().map(String::from));
    let next_unit = unit.unwrap_or(existing["unit"].as_str().unwrap_or("unit"));
    let next_price = unit_price_cents
        .or(existing["unit_price_cents"].as_i64())
        .unwrap_or(0);
    let next_vat = if vat_code == Some("".into()) {
        None
    } else {
        vat_code.or_else(|| existing["vat_code"].as_str().map(String::from))
    };
    let next_gl = if gl_account == Some("".into()) {
        None
    } else {
        gl_account.or_else(|| existing["gl_account"].as_str().map(String::from))
    };
    let next_active = if deactivate {
        0
    } else {
        if existing["active"].as_bool().unwrap_or(true) {
            1
        } else {
            0
        }
    };

    validate_item(next_name, next_unit, next_price, next_vat.as_deref())?;
    if let Some(vc) = &next_vat {
        if !is_vat_enabled(db) {
            return Err(item_error(
                "VAT_MODULE_OFF",
                "item has a VAT code but the VAT module is off",
            ));
        }
        let known = list_vat_codes(db)?.iter().any(|c| c["code"] == vc.as_str());
        if !known {
            return Err(item_error(
                "VAT_CODE_NOT_FOUND",
                format!("vat code '{vc}' does not exist"),
            ));
        }
    }
    if let Some(acct) = &next_gl {
        if get_account_by_code(db, acct).is_none() {
            return Err(item_error(
                "ACCOUNT_NOT_FOUND",
                format!("account '{acct}' does not exist"),
            ));
        }
    }

    if dry_run {
        return Ok(json!({
            "action": "item.update", "id": id, "dryRun": true,
            "changes": { "name": next_name, "unit": next_unit, "unit_price_cents": next_price },
        }));
    }

    db.execute(
        "UPDATE items SET name = ?1, description = ?2, unit = ?3, unit_price_cents = ?4,
         vat_code = ?5, gl_account = ?6, active = ?7, updated_by = ?8,
         updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
         WHERE id = ?9",
        rusqlite::params![
            next_name,
            next_desc,
            next_unit,
            next_price,
            next_vat,
            next_gl,
            next_active,
            actor,
            id
        ],
    )
    .map_err(sql_err)?;
    record(
        db,
        RecordArgs {
            actor,
            action: "item.update",
            command: Some("item update"),
            args: Some(json!({ "id": id })),
            outcome: "ok",
            entry_ids: vec![],
        },
    )?;
    get_item(db, id)?.ok_or_else(|| item_error("DB_ERROR", "update succeeded but item not found"))
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
        d.execute("INSERT INTO company (name) VALUES ('ItemCo')", [])
            .unwrap();
        crate::accounts::seed_default_chart(&d).unwrap();
        d
    }

    #[test]
    fn create_item_basic() {
        let d = db();
        let item = create_item(
            &d,
            "Consulting",
            Some("IT consulting"),
            "h",
            15000,
            None,
            None,
            "human:erik",
            false,
        )
        .unwrap();
        assert_eq!(item["name"], "Consulting");
        assert_eq!(item["unit"], "h");
        assert_eq!(item["unit_price_cents"], 15000);
    }

    #[test]
    fn invalid_unit_rejected() {
        let d = db();
        let err = create_item(
            &d,
            "X",
            None,
            "invalid",
            100,
            None,
            None,
            "human:erik",
            false,
        )
        .unwrap_err();
        assert_eq!(err.code, "INVALID_UNIT");
    }

    #[test]
    fn update_item_name() {
        let d = db();
        let item = create_item(
            &d,
            "Old Name",
            None,
            "unit",
            100,
            None,
            None,
            "human:erik",
            false,
        )
        .unwrap();
        let updated = update_item(
            &d,
            item["id"].as_i64().unwrap(),
            Some("New Name"),
            None,
            None,
            None,
            None,
            None,
            false,
            "human:erik",
            false,
        )
        .unwrap();
        assert_eq!(updated["name"], "New Name");
    }
}
