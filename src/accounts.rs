// bukio-cli — agent-first double-entry bookkeeping for SMEs.
// Copyright (c) 2026 Erik van Kempen.
// SPDX-License-Identifier: Apache-2.0
//
// Accounts (mirrors src/core/accounts.js) + cost centers (core/cost-centers.js)
// + the jurisdiction profiles (generated src/profiles.json).

use crate::money::{BukioError, Result};
use rusqlite::Connection;
use serde_json::{json, Value};

// --- profiles ---------------------------------------------------------------

pub static PROFILES_JSON: &str = include_str!("profiles.json");

pub fn profiles() -> &'static Value {
    static PROFILES: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    PROFILES.get_or_init(|| serde_json::from_str(PROFILES_JSON).expect("profiles.json is valid"))
}

pub fn get_profile(country: &str) -> Result<&'static Value> {
    let cc = country.trim().to_uppercase();
    if cc.len() != 2 || !cc.bytes().all(|b| b.is_ascii_alphabetic()) {
        return Err(BukioError::new(
            "INVALID_COUNTRY",
            format!("country '{country}' must be an ISO 3166-1 alpha-2 code (e.g. NL)"),
        ));
    }
    profiles().get(&cc).ok_or_else(|| {
        BukioError::new(
            "PROFILE_NOT_FOUND",
            format!("no jurisdiction profile for country {cc}"),
        )
    })
}

/// resolveProfile(db): the company's country, or NL pre-init.
pub fn resolve_profile(db: &Connection) -> Result<&'static Value> {
    let country: Option<String> = db
        .query_row("SELECT country FROM company WHERE id = 1", [], |r| r.get(0))
        .ok()
        .flatten();
    get_profile(country.as_deref().unwrap_or("NL"))
}

// --- accounts ---------------------------------------------------------------

const VALID_TYPES: [&str; 5] = ["asset", "liability", "equity", "income", "expense"];

pub struct NewAccount<'a> {
    pub code: &'a str,
    pub name: &'a str,
    pub type_: &'a str,
    pub normal_balance: &'a str,
    pub taxonomy_code: Option<&'a str>,
}

pub fn validate_account(a: &NewAccount<'_>) -> Result<()> {
    let code_ok =
        !a.code.is_empty() && a.code.len() <= 6 && a.code.bytes().all(|b| b.is_ascii_digit());
    if !code_ok {
        return Err(BukioError::new(
            "INVALID_CODE",
            format!("account code '{}' must be 1-6 digits", a.code),
        ));
    }
    if a.name.trim().is_empty() {
        return Err(BukioError::new("INVALID_NAME", "account name is required"));
    }
    if !VALID_TYPES.contains(&a.type_) {
        return Err(BukioError::new(
            "INVALID_TYPE",
            format!(
                "account type '{}' must be one of {}",
                a.type_,
                VALID_TYPES.join(", ")
            ),
        ));
    }
    if !matches!(a.normal_balance, "debit" | "credit") {
        return Err(BukioError::new(
            "INVALID_NORMAL_BALANCE",
            format!(
                "normal_balance '{}' must be debit or credit",
                a.normal_balance
            ),
        ));
    }
    if let Some(t) = a.taxonomy_code {
        if !t.is_empty() && !valid_taxonomy(t) {
            return Err(BukioError::new(
                "INVALID_RGS_CODE",
                format!("taxonomy_code '{t}' does not look like an RGS code (e.g. BMVA.02)"),
            ));
        }
    }
    Ok(())
}

fn valid_taxonomy(t: &str) -> bool {
    // ^[A-Z]{2,5}\.\d{2}(\.\d{1,3})*$
    let Some((head, tail)) = t.split_once('.') else {
        return false;
    };
    let head_ok =
        head.len() >= 2 && head.len() <= 5 && head.bytes().all(|b| b.is_ascii_uppercase());
    if !head_ok {
        return false;
    }
    let mut parts = tail.split('.');
    let first = parts.next().unwrap_or("");
    if first.len() != 2 || !first.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    for p in parts {
        if p.is_empty() || p.len() > 3 || !p.bytes().all(|b| b.is_ascii_digit()) {
            return false;
        }
    }
    true
}

pub fn create_account(db: &Connection, a: &NewAccount<'_>) -> Result<Value> {
    validate_account(a)?;
    let profile = resolve_profile(db)?;
    let taxonomy: Option<&str> = profile["reporting"]["taxonomy"]
        .as_str()
        .filter(|s| !s.is_empty());
    let insert = db.execute(
        "INSERT INTO accounts (code, name, type, taxonomy_code, normal_balance, taxonomy) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![a.code, a.name.trim(), a.type_, non_empty(a.taxonomy_code), a.normal_balance, taxonomy],
    );
    if let Err(e) = insert {
        let msg = e.to_string();
        if msg.contains("UNIQUE constraint failed: accounts.code") {
            return Err(BukioError::new(
                "ACCOUNT_EXISTS",
                format!("account code {} already exists", a.code),
            ));
        }
        if msg.contains("CHECK constraint failed") {
            let expected = if a.type_ == "asset" || a.type_ == "expense" {
                "debit"
            } else {
                "credit"
            };
            return Err(BukioError::new(
                "INVALID_COMBINATION",
                format!("type '{}' requires normal_balance '{}'", a.type_, expected),
            ));
        }
        return Err(BukioError::new("DB_ERROR", msg));
    }
    get_account_by_code(db, a.code).ok_or_else(|| BukioError::new("INTERNAL", "account vanished"))
}

fn non_empty(s: Option<&str>) -> Option<&str> {
    match s {
        Some("") | None => None,
        Some(v) => Some(v),
    }
}

pub fn get_account_by_code(db: &Connection, code: &str) -> Option<Value> {
    db.query_row(
        "SELECT code, name, type, taxonomy_code, normal_balance, active FROM accounts WHERE code = ?1",
        [code],
        row_to_account,
    )
    .ok()
}

/// The RAW row (JS `getAccount`: `SELECT *`) — `account reactivate` nests this
/// under `account`, unlike its siblings which emit the slim projection.
pub fn get_account_row_by_code(db: &Connection, code: &str) -> Option<Value> {
    db.query_row(
        "SELECT id, code, name, type, taxonomy_code, normal_balance, active, created_at, taxonomy \
         FROM accounts WHERE code = ?1",
        [code],
        row_to_account_raw,
    )
    .ok()
}

fn row_to_account_raw(r: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    Ok(json!({
        "id": r.get::<_, i64>(0)?,
        "code": r.get::<_, String>(1)?,
        "name": r.get::<_, String>(2)?,
        "type": r.get::<_, String>(3)?,
        "taxonomy_code": r.get::<_, Option<String>>(4)?,
        "normal_balance": r.get::<_, String>(5)?,
        "active": r.get::<_, i64>(6)?,
        "created_at": r.get::<_, Option<String>>(7)?,
        "taxonomy": r.get::<_, Option<String>>(8)?,
    }))
}

fn row_to_account(r: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    Ok(json!({
        "code": r.get::<_, String>(0)?,
        "name": r.get::<_, String>(1)?,
        "type": r.get::<_, String>(2)?,
        "taxonomy_code": r.get::<_, Option<String>>(3)?,
        "normal_balance": r.get::<_, String>(4)?,
        "active": r.get::<_, i64>(5)? == 1,
    }))
}

pub fn list_accounts(
    db: &Connection,
    type_filter: Option<&str>,
    include_inactive: bool,
) -> Result<Vec<Value>> {
    let mut sql = String::from(
        "SELECT code, name, type, taxonomy_code, normal_balance, active FROM accounts",
    );
    let mut clauses = Vec::new();
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    if let Some(t) = type_filter {
        clauses.push("type = ?");
        params.push(Box::new(t.to_string()));
    }
    if !include_inactive {
        clauses.push("active = 1");
    }
    if !clauses.is_empty() {
        sql.push_str(&format!(" WHERE {}", clauses.join(" AND ")));
    }
    sql.push_str(" ORDER BY code");
    let mut stmt = db.prepare(&sql).map_err(sql_err)?;
    let rows = stmt
        .query_map(params_refs(&params).as_slice(), row_to_account)
        .map_err(sql_err)?
        .filter_map(|x| x.ok())
        .collect();
    Ok(rows)
}

/// Seed the default chart from the company's jurisdiction profile.
pub fn seed_default_chart(db: &Connection) -> Result<usize> {
    let chart = resolve_profile(db)?["reporting"]["defaultChart"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut created = 0;
    for a in chart {
        let code = a["code"].as_str().unwrap_or_default();
        if get_account_by_code(db, code).is_some() {
            continue;
        }
        create_account(
            db,
            &NewAccount {
                code,
                name: a["name"].as_str().unwrap_or_default(),
                type_: a["type"].as_str().unwrap_or_default(),
                normal_balance: a["normalBalance"].as_str().unwrap_or("credit"),
                taxonomy_code: a["taxonomyCode"].as_str(),
            },
        )?;
        created += 1;
    }
    Ok(created)
}

pub fn deactivate_account(db: &Connection, code: &str) -> Result<Value> {
    let a = get_account_by_code(db, code).ok_or_else(|| {
        BukioError::new(
            "ACCOUNT_NOT_FOUND",
            format!("account {code} does not exist"),
        )
    })?;
    if !a["active"].as_bool().unwrap_or(false) {
        return Err(BukioError::new(
            "ALREADY_INACTIVE",
            format!("account {code} is already inactive"),
        ));
    }
    db.execute("UPDATE accounts SET active = 0 WHERE code = ?1", [code])
        .map_err(sql_err)?;
    get_account_by_code(db, code).ok_or_else(|| BukioError::new("INTERNAL", "account vanished"))
}

pub fn reactivate_account(db: &Connection, code: &str) -> Result<Value> {
    let a = get_account_by_code(db, code).ok_or_else(|| {
        BukioError::new(
            "ACCOUNT_NOT_FOUND",
            format!("account {code} does not exist"),
        )
    })?;
    if a["active"].as_bool().unwrap_or(false) {
        return Err(BukioError::new(
            "ALREADY_ACTIVE",
            format!("account {code} is already active"),
        ));
    }
    db.execute("UPDATE accounts SET active = 1 WHERE code = ?1", [code])
        .map_err(sql_err)?;
    get_account_by_code(db, code).ok_or_else(|| BukioError::new("INTERNAL", "account vanished"))
}

// --- cost centers -----------------------------------------------------------

pub fn create_cost_center(db: &Connection, code: &str, name: &str) -> Result<Value> {
    // JS CODE_RE is ^[A-Z0-9][A-Z0-9 ._-]{0,31}$ — UPPERCASE only: a lowercase
    // code is rejected by the JS and was accepted here
    let code_ok = {
        let b = code.as_bytes();
        let alnum = |c: u8| c.is_ascii_uppercase() || c.is_ascii_digit();
        let body = |c: u8| alnum(c) || matches!(c, b' ' | b'.' | b'_' | b'-');
        !b.is_empty() && b.len() <= 32 && alnum(b[0]) && b[1..].iter().all(|c| body(*c))
    };
    if !code_ok {
        return Err(BukioError::new(
            "INVALID_CODE",
            format!("cost center code '{code}' must be 1-32 chars: letters, digits, space, . _ - (no leading punctuation)"),
        ));
    }
    if name.trim().is_empty() {
        return Err(BukioError::new(
            "INVALID_NAME",
            "cost center name is required",
        ));
    }
    if let Err(e) = db.execute(
        "INSERT INTO cost_centers (code, name, active) VALUES (?1, ?2, 1)",
        rusqlite::params![code, name.trim()],
    ) {
        if e.to_string().contains("UNIQUE constraint failed") {
            return Err(BukioError::new(
                "COST_CENTER_EXISTS",
                format!("cost center '{code}' already exists"),
            ));
        }
        return Err(BukioError::new("DB_ERROR", e.to_string()));
    }
    get_cost_center_by_code(db, code)
        .ok_or_else(|| BukioError::new("INTERNAL", "cost center vanished"))
}

/// The RAW row (JS `getCostCenter`: `SELECT *`) — deactivate/reactivate nest it.
fn row_to_cost_center(r: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    Ok(json!({
        "id": r.get::<_, i64>(0)?,
        "code": r.get::<_, String>(1)?,
        "name": r.get::<_, String>(2)?,
        "active": r.get::<_, i64>(3)?,
        "created_at": r.get::<_, Option<String>>(4)?,
    }))
}

/// JS CLI `serialize(cc)`: slim shape with a boolean `active`.
pub fn serialize_cost_center(v: &Value) -> Value {
    json!({
        "id": v["id"],
        "code": v["code"],
        "name": v["name"],
        "active": v["active"].as_i64().unwrap_or(0) == 1,
    })
}

pub fn get_cost_center_by_code(db: &Connection, code: &str) -> Option<Value> {
    db.query_row(
        "SELECT id, code, name, active, created_at FROM cost_centers WHERE code = ?1",
        [code],
        row_to_cost_center,
    )
    .ok()
}

pub fn list_cost_centers(db: &Connection, include_inactive: bool) -> Result<Vec<Value>> {
    let sql = if include_inactive {
        "SELECT id, code, name, active, created_at FROM cost_centers ORDER BY code"
    } else {
        "SELECT id, code, name, active, created_at FROM cost_centers WHERE active = 1 ORDER BY code"
    };
    let mut stmt = db.prepare(sql).map_err(sql_err)?;
    let rows = stmt
        .query_map([], row_to_cost_center)
        .map_err(sql_err)?
        .filter_map(|x| x.ok())
        .collect();
    Ok(rows)
}

pub fn set_cost_center_active(
    db: &Connection,
    code: &str,
    active: bool,
    want: bool,
) -> Result<Value> {
    let cc = get_cost_center_by_code(db, code).ok_or_else(|| {
        BukioError::new(
            "COST_CENTER_NOT_FOUND",
            format!("cost center '{code}' does not exist"),
        )
    })?;
    if (cc["active"].as_i64().unwrap_or(0) == 1) == want {
        let err_code = if want {
            "ALREADY_ACTIVE"
        } else {
            "ALREADY_INACTIVE"
        };
        return Err(BukioError::new(
            err_code,
            format!(
                "cost center '{code}' is already {}",
                if want { "active" } else { "inactive" }
            ),
        ));
    }
    db.execute(
        "UPDATE cost_centers SET active = ?1 WHERE code = ?2",
        rusqlite::params![if active { 1 } else { 0 }, code],
    )
    .map_err(sql_err)?;
    get_cost_center_by_code(db, code)
        .ok_or_else(|| BukioError::new("INTERNAL", "cost center vanished"))
}

// --- chart CSV import (mirrors importChartCsv) ------------------------------

pub fn split_csv_line(line: &str, delim: char) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    for ch in line.chars() {
        match ch {
            '"' => in_quotes = !in_quotes,
            c if c == delim && !in_quotes => {
                out.push(std::mem::take(&mut cur));
            }
            c => cur.push(c),
        }
    }
    out.push(cur);
    out
}

/// inferRgs (mirrors core/chart.js) — keyword match within type, type fallback.
pub fn infer_rgs(type_: &str, name: &str) -> Option<&'static str> {
    let n = name.to_lowercase();
    let has = |kws: &[&str]| kws.iter().any(|k| n.contains(k));
    match type_ {
        "income" => {
            if has(&["diensten", "service"]) {
                Some("WOVB.82")
            } else if has(&["omzet", "verkopen", "verkoop"]) {
                Some("WOMZ.80")
            } else {
                Some("WOVB.82")
            }
        }
        "expense" => {
            if has(&["inkoop", "voorraad", "uitbesteed"]) {
                Some("WKPR.70")
            } else if has(&["personeel", "loon", "salaris", "sociale", "pensioen"]) {
                Some("WPER.40")
            } else if has(&["afschrijving", "afschr"]) {
                Some("WAFS.41")
            } else if has(&["rente", "financie", "interest", "bankkosten"]) {
                Some("WFBE.84")
            } else {
                Some("WBED.42")
            }
        }
        "asset" => {
            if has(&[
                "bank", "kas", "geld", "tegoed", "spaar", "sumup", "onefor", "business",
            ]) {
                Some("BLIM.10")
            } else if has(&[
                "debiteur",
                "vorderen",
                "vraagpost",
                "kruispost",
                "vooruitbetaald",
                "nog te ontvangen",
            ]) {
                Some("BVOR.11")
            } else if has(&["voorraad"]) {
                Some("BVRD.30")
            } else {
                Some("BMVA.02")
            }
        }
        "liability" => Some("BSCH.12"),
        "equity" => Some("BEIV.05"),
        _ => None,
    }
}

#[derive(Debug)]
pub struct ChartImportResult {
    pub created: usize,
    pub skipped: usize,
    pub total: usize,
    pub errors: Vec<(usize, String)>,
}

pub fn import_chart_csv(db: &Connection, csv_text: &str) -> Result<ChartImportResult> {
    let lines: Vec<&str> = csv_text
        .split(['\r', '\n'])
        .filter(|l| !l.trim().is_empty())
        .collect();
    if lines.len() < 2 {
        return Err(BukioError::new(
            "EMPTY_CSV",
            "chart CSV must have a header row and at least one account",
        ));
    }
    let header: Vec<String> = split_csv_line(lines[0], ',')
        .iter()
        .map(|h| h.trim().to_string())
        .collect();
    for col in ["code", "name", "type", "normal_balance"] {
        if !header.iter().any(|h| h == col) {
            return Err(BukioError::new(
                "INVALID_CSV_HEADER",
                format!(
                    "chart CSV is missing column '{col}' (got: {})",
                    header.join(",")
                ),
            ));
        }
    }
    let idx = |name: &str| header.iter().position(|h| h == name);
    let tax_idx = idx("taxonomy_code").or_else(|| idx("rgs_code"));

    let mut result = ChartImportResult {
        created: 0,
        skipped: 0,
        total: lines.len() - 1,
        errors: Vec::new(),
    };
    for (i, line) in lines.iter().enumerate().skip(1) {
        let row: Vec<String> = split_csv_line(line, ',')
            .iter()
            .map(|c| c.trim().to_string())
            .collect();
        if row.len() == 1 && row[0].is_empty() {
            continue;
        }
        let cell = |n: &str| idx(n).and_then(|i| row.get(i)).cloned().unwrap_or_default();
        let taxonomy = tax_idx
            .and_then(|i| row.get(i))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let account = NewAccount {
            code: &cell("code"),
            name: &cell("name"),
            type_: &cell("type"),
            normal_balance: &cell("normal_balance"),
            taxonomy_code: taxonomy
                .as_deref()
                .or_else(|| infer_rgs(&cell("type"), &cell("name"))),
        };
        let res = if get_account_by_code(db, account.code).is_some() {
            Err(BukioError::new(
                "ACCOUNT_EXISTS",
                format!("account {} already exists (skipped)", account.code),
            ))
        } else {
            create_account(db, &account)
        };
        match res {
            Ok(_) => result.created += 1,
            Err(e) => {
                result.skipped += 1;
                result
                    .errors
                    .push((i + 1, format!("{}: {}", e.code, e.message)));
            }
        }
    }
    Ok(result)
}

fn sql_err(e: rusqlite::Error) -> BukioError {
    BukioError::new("DB_ERROR", e.to_string())
}

/// ponytail: dyn-param helper — rusqlite 0.32 has no params_from_vec; refs
/// satisfy Params for &[&dyn ToSql].
fn params_refs(params: &[Box<dyn rusqlite::types::ToSql>]) -> Vec<&dyn rusqlite::types::ToSql> {
    params.iter().map(|p| p.as_ref()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_db;

    fn company_db() -> Connection {
        let db = open_db(":memory:").unwrap();
        db.execute("INSERT INTO company (name) VALUES ('T')", [])
            .unwrap();
        db
    }

    #[test]
    fn profiles_load_all_31() {
        assert_eq!(profiles().as_object().unwrap().len(), 31);
        assert_eq!(get_profile("nl").unwrap()["meta"]["country"], "NL");
        assert_eq!(get_profile("XX").unwrap_err().code, "PROFILE_NOT_FOUND");
        assert_eq!(get_profile("N").unwrap_err().code, "INVALID_COUNTRY");
    }

    #[test]
    fn seeds_nl_default_chart_of_29() {
        let db = company_db();
        let created = seed_default_chart(&db).unwrap();
        assert_eq!(created, 29);
        let accounts = list_accounts(&db, None, false).unwrap();
        assert_eq!(accounts.len(), 29);
        // all RGS-mapped
        assert!(accounts.iter().all(|a| a["taxonomy_code"].is_string()));
        // spot-checks carried over from the entry suite's fixture test
        assert!(accounts
            .iter()
            .any(|a| a["code"] == "1100" && a["type"] == "asset"));
        // 4840 Koersverschillen (FX differences on invoice payments)
        assert!(accounts
            .iter()
            .any(|a| a["code"] == "4840" && a["taxonomy_code"] == "WFBE.84"));
        assert!(accounts
            .iter()
            .any(|a| a["code"] == "8000" && a["type"] == "income"));
        // VAT-agnostic: no btw accounts in the core chart
        assert!(!accounts.iter().any(|a| {
            let name = a["name"].as_str().unwrap_or("").to_lowercase();
            name.contains("btw") || name.contains("omzetbelasting")
        }));
        // second seed is a no-op
        assert_eq!(seed_default_chart(&db).unwrap(), 0);
    }

    // ── ported from test/accounts.test.js ──

    fn acct<'a>(code: &'a str, name: &'a str, type_: &'a str, nb: &'a str) -> NewAccount<'a> {
        NewAccount {
            code,
            name,
            type_,
            normal_balance: nb,
            taxonomy_code: None,
        }
    }

    #[test]
    fn create_account_valid_and_validation_codes() {
        let db = company_db();
        seed_default_chart(&db).unwrap();
        assert_eq!(
            create_account(&db, &acct("abc", "x", "asset", "debit"))
                .unwrap_err()
                .code,
            "INVALID_CODE"
        );
        assert_eq!(
            create_account(
                &db,
                &NewAccount {
                    code: "5001",
                    name: "x",
                    type_: "expense",
                    normal_balance: "debit",
                    taxonomy_code: Some("BMVA"),
                }
            )
            .unwrap_err()
            .code,
            "INVALID_RGS_CODE"
        );
        let a = create_account(&db, &acct("5000", "Testkosten", "expense", "debit")).unwrap();
        assert_eq!(a["code"], "5000");
        assert_eq!(
            get_account_by_code(&db, "5000").unwrap()["name"],
            "Testkosten"
        );
    }

    #[test]
    fn deactivate_reactivate_lifecycle_blocks_new_postings() {
        use crate::entries::{create_entry, post_entry, CreateEntry, PostingSpec};
        let db = company_db();
        seed_default_chart(&db).unwrap();
        create_account(&db, &acct("5000", "Testkosten", "expense", "debit")).unwrap();

        deactivate_account(&db, "5000").unwrap();
        assert_eq!(get_account_by_code(&db, "5000").unwrap()["active"], false);
        assert_eq!(
            deactivate_account(&db, "5000").unwrap_err().code,
            "ALREADY_INACTIVE"
        );
        assert_eq!(
            deactivate_account(&db, "9999").unwrap_err().code,
            "ACCOUNT_NOT_FOUND"
        );

        let postings = || {
            vec![
                PostingSpec {
                    code: "5000".into(),
                    amount_cents: 100,
                    cost_center_code: None,
                    vat_code: None,
                    vat_amount_cents: None,
                    fx_currency: None,
                    fx_amount_cents: None,
                },
                PostingSpec {
                    code: "1100".into(),
                    amount_cents: -100,
                    cost_center_code: None,
                    vat_code: None,
                    vat_amount_cents: None,
                    fx_currency: None,
                    fx_amount_cents: None,
                },
            ]
        };
        let mk = || CreateEntry {
            date: "2026-08-04",
            description: "x",
            postings: postings(),
            source: "manual",
            source_ref: None,
            actor: "human:erik",
        };
        assert_eq!(
            create_entry(&db, mk()).unwrap_err().code,
            "ACCOUNT_INACTIVE"
        );

        reactivate_account(&db, "5000").unwrap();
        assert_eq!(get_account_by_code(&db, "5000").unwrap()["active"], true);
        assert_eq!(
            reactivate_account(&db, "5000").unwrap_err().code,
            "ALREADY_ACTIVE"
        );
        let e = create_entry(&db, mk()).unwrap();
        post_entry(&db, e.id, "human:erik").unwrap();
    }

    #[test]
    fn chart_csv_import_details_and_quoting() {
        let db = company_db();
        seed_default_chart(&db).unwrap();
        let csv = "code,name,type,normal_balance,taxonomy_code\n\
                   5000,Testkosten,expense,debit,WBED.42\n\
                   5100,Andere kosten,expense,debit,WBED.42\n\
                   5200,Verkeerd type,weird,debit,\n\
                   1100,Bank duplicaat,asset,debit,BLIM.10\n";
        let r = import_chart_csv(&db, csv).unwrap();
        assert_eq!(r.created, 2);
        assert_eq!(r.skipped, 2);
        assert_eq!(r.total, 4);
        assert_eq!(r.errors.len(), 2);
        assert!(r.errors.iter().any(|(_, m)| m.contains("INVALID_TYPE")));
        assert!(r.errors.iter().any(|(_, m)| m.contains("ACCOUNT_EXISTS")));
        assert_eq!(
            get_account_by_code(&db, "5100").unwrap()["name"],
            "Andere kosten"
        );

        // header validation
        assert_eq!(
            import_chart_csv(&db, "foo,bar\n1,2\n").unwrap_err().code,
            "INVALID_CSV_HEADER"
        );
        assert_eq!(
            import_chart_csv(&db, "code,name,type,normal_balance\n")
                .unwrap_err()
                .code,
            "EMPTY_CSV"
        );

        // quoted values containing commas (fresh DB — 5000 exists in the one above)
        let db2 = company_db();
        seed_default_chart(&db2).unwrap();
        let one = import_chart_csv(
            &db2,
            "code,name,type,normal_balance,taxonomy_code\n\
             \"5000\",\"Kosten, algemeen\",expense,debit,WBED.42\n",
        )
        .unwrap();
        assert_eq!(one.created, 1);
        assert_eq!(
            get_account_by_code(&db2, "5000").unwrap()["name"],
            "Kosten, algemeen"
        );
    }

    #[test]
    fn list_accounts_type_filter_and_include_inactive() {
        let db = company_db();
        seed_default_chart(&db).unwrap();
        assert_eq!(list_accounts(&db, Some("income"), false).unwrap().len(), 2);
        // 13 expense accounts in the NL chart, incl. 4840 Koersverschillen
        assert_eq!(
            list_accounts(&db, Some("expense"), false).unwrap().len(),
            13
        );
        create_account(&db, &acct("5000", "x", "expense", "debit")).unwrap();
        deactivate_account(&db, "5000").unwrap();
        assert_eq!(
            list_accounts(&db, Some("expense"), false).unwrap().len(),
            13
        );
        assert_eq!(list_accounts(&db, Some("expense"), true).unwrap().len(), 14);
    }

    #[test]
    fn account_validation_matches_js() {
        let db = company_db();
        seed_default_chart(&db).unwrap();
        assert_eq!(
            create_account(
                &db,
                &NewAccount {
                    code: "1100",
                    name: "dup",
                    type_: "asset",
                    normal_balance: "debit",
                    taxonomy_code: None
                }
            )
            .unwrap_err()
            .code,
            "ACCOUNT_EXISTS"
        );
        assert_eq!(
            create_account(
                &db,
                &NewAccount {
                    code: "9",
                    name: "bad type",
                    type_: "cash",
                    normal_balance: "debit",
                    taxonomy_code: None
                }
            )
            .unwrap_err()
            .code,
            "INVALID_TYPE"
        );
        assert_eq!(
            create_account(
                &db,
                &NewAccount {
                    code: "10",
                    name: "bad combo",
                    type_: "income",
                    normal_balance: "debit",
                    taxonomy_code: None
                }
            )
            .unwrap_err()
            .code,
            "INVALID_COMBINATION"
        );
        let ok = create_account(
            &db,
            &NewAccount {
                code: "1230",
                name: "Spaarrekening 2",
                type_: "asset",
                normal_balance: "debit",
                taxonomy_code: None,
            },
        )
        .unwrap();
        assert_eq!(ok["taxonomy_code"], Value::Null); // JS createAccount does not infer (only CSV import does)
        assert_eq!(ok["active"], true);
    }

    #[test]
    fn chart_csv_import() {
        let db = company_db();
        seed_default_chart(&db).unwrap();
        // 1100 exists -> skipped as ACCOUNT_EXISTS; 2000 imports; 9999 has a
        // bad taxonomy code -> validation error
        let csv = "code,name,type,normal_balance,taxonomy_code\n1100,Bank,asset,debit,BLIM.10\n2500,Schuld,liability,credit,\n9999,Vraagpost,asset,debit,NOPE\n";
        let r = import_chart_csv(&db, csv).unwrap();
        assert_eq!(r.created, 1);
        assert_eq!(r.skipped, 2);
        assert_eq!(r.errors.len(), 2);
    }

    #[test]
    fn cost_center_lifecycle() {
        let db = company_db();
        let cc = create_cost_center(&db, "HQ", "Hoofdkantoor").unwrap();
        // the accessor returns the RAW row (JS getCostCenter: SELECT *), with the
        // integer flag; the CLI serializes it to `active: bool` for add/list/show.
        assert_eq!(cc["active"], 1);
        assert!(cc["id"].as_i64().unwrap_or(0) > 0);
        assert!(cc["created_at"].is_string());
        assert_eq!(serialize_cost_center(&cc)["active"], true);
        assert_eq!(
            create_cost_center(&db, "HQ", "again").unwrap_err().code,
            "COST_CENTER_EXISTS"
        );
        let off = set_cost_center_active(&db, "HQ", false, false).unwrap();
        assert_eq!(off["active"], 0);
        assert_eq!(serialize_cost_center(&off)["active"], false);
        assert_eq!(
            set_cost_center_active(&db, "HQ", false, false)
                .unwrap_err()
                .code,
            "ALREADY_INACTIVE"
        );
    }

    // ==== ported from test/cost-centers.test.js ==============================

    fn cc_db() -> Connection {
        let db = company_db();
        seed_default_chart(&db).unwrap();
        db
    }

    fn cc_spec(code: &str, cents: i64, cc: Option<&str>) -> crate::entries::PostingSpec {
        crate::entries::PostingSpec {
            code: code.to_string(),
            amount_cents: cents,
            cost_center_code: cc.map(String::from),
            vat_code: None,
            vat_amount_cents: None,
            fx_currency: None,
            fx_amount_cents: None,
        }
    }

    fn add_cc_entry(
        db: &Connection,
        date: &str,
        description: &str,
        postings: Vec<crate::entries::PostingSpec>,
    ) -> crate::entries::Entry {
        crate::entries::create_entry(
            db,
            crate::entries::CreateEntry {
                date,
                description,
                postings,
                source: "manual",
                source_ref: None,
                actor: "human:erik",
            },
        )
        .unwrap()
    }

    #[test]
    fn cost_center_basic_crud() {
        let db = cc_db();
        let cc = create_cost_center(&db, "ADM", "Administration").unwrap();
        assert_eq!(cc["code"].as_str(), Some("ADM"));
        assert_eq!(cc["name"].as_str(), Some("Administration"));
        assert_eq!(cc["active"].as_i64(), Some(1));
        assert!(get_cost_center_by_code(&db, "ADM").is_some());
        assert_eq!(list_cost_centers(&db, false).unwrap().len(), 1);
    }

    #[test]
    fn cost_center_rejects_a_duplicate_code() {
        let db = cc_db();
        create_cost_center(&db, "ADM", "Administration").unwrap();
        assert_eq!(
            create_cost_center(&db, "ADM", "Admin").unwrap_err().code,
            "COST_CENTER_EXISTS"
        );
    }

    #[test]
    fn cost_center_rejects_an_invalid_code() {
        let db = cc_db();
        assert_eq!(
            create_cost_center(&db, "x", "X").unwrap_err().code,
            "INVALID_CODE"
        );
    }

    #[test]
    fn deactivating_a_cost_center_blocks_new_bookings_but_keeps_history() {
        let db = cc_db();
        create_cost_center(&db, "ADM", "Administration").unwrap();
        let updated = set_cost_center_active(&db, "ADM", false, false).unwrap();
        assert_eq!(updated["active"].as_i64(), Some(0));
        // booking on an inactive center is refused (JS resolveCostCenterIds)
        let spec = || {
            vec![
                cc_spec("8000", -10000, Some("ADM")),
                cc_spec("3000", 10000, None),
            ]
        };
        let err = crate::entries::create_entry(
            &db,
            crate::entries::CreateEntry {
                date: "2026-08-04",
                description: "CC test",
                postings: spec(),
                source: "manual",
                source_ref: None,
                actor: "human:erik",
            },
        )
        .unwrap_err();
        assert_eq!(err.code, "COST_CENTER_INACTIVE");
        // after reactivation it is resolvable again
        set_cost_center_active(&db, "ADM", true, true).unwrap();
        let e = add_cc_entry(&db, "2026-08-04", "CC test", spec());
        assert_eq!(e.postings.len(), 2);
    }

    #[test]
    fn posting_specs_without_a_cost_center() {
        let specs =
            crate::entries::parse_posting_specs(&["8000:-100.00,3000:100.00".to_string()]).unwrap();
        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].code, "8000");
        assert_eq!(specs[0].amount_cents, -10000);
        assert_eq!(specs[0].cost_center_code, None);
    }

    #[test]
    fn posting_specs_with_an_at_cc_suffix() {
        let specs =
            crate::entries::parse_posting_specs(&["8000:-100.00@ADM,3000:100.00".to_string()])
                .unwrap();
        assert_eq!(specs[0].cost_center_code.as_deref(), Some("ADM"));
        assert_eq!(specs[1].cost_center_code, None);
    }

    #[test]
    fn an_entry_carries_the_cost_center_and_surfaces_it() {
        let db = cc_db();
        create_cost_center(&db, "ADM", "Admin").unwrap();
        let e = add_cc_entry(
            &db,
            "2026-08-04",
            "CC test",
            vec![
                cc_spec("8000", -10000, Some("ADM")),
                cc_spec("3000", 10000, None),
            ],
        );
        assert_eq!(e.postings[0].cost_center_code.as_deref(), Some("ADM"));
        assert_eq!(e.postings[1].cost_center_code, None);
    }

    #[test]
    fn a_reversal_carries_the_cost_center_to_the_contra_entry() {
        let db = cc_db();
        create_cost_center(&db, "ADM", "Admin").unwrap();
        let e = add_cc_entry(
            &db,
            "2026-08-04",
            "CC test",
            vec![
                cc_spec("8000", -10000, Some("ADM")),
                cc_spec("3000", 10000, None),
            ],
        );
        crate::entries::post_entry(&db, e.id, "human:erik").unwrap();
        let rev = crate::entries::reverse_entry(&db, e.id, "human:erik", None).unwrap();
        assert_eq!(rev.postings[0].cost_center_code.as_deref(), Some("ADM"));
        // the reversal balances
        let sum: i64 = rev.postings.iter().map(|p| p.amount_cents).sum();
        assert_eq!(sum, 0);
    }
}
