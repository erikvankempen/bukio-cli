//! Accounts — chart of accounts CRUD, CSV import, default chart seeding.

use crate::core::chart::{AccountSeed, DEFAULT_CHART};
use crate::error::{AppError, Result};
use regex::Regex;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::sync::OnceLock;

pub const VALID_TYPES: [&str; 5] = ["asset", "liability", "equity", "income", "expense"];
pub const VALID_NORMAL: [&str; 2] = ["debit", "credit"];

fn rgs_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^[A-Z]{2,5}\.\d{2}(\.\d{1,3})*$").unwrap())
}

fn code_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^\d{1,6}$").unwrap())
}

/// One accounts row, serialized exactly like the Node DB rows (snake_case,
/// `type` for the account type, `active` as 0/1 integer).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct Account {
    pub id: i64,
    pub code: String,
    pub name: String,
    #[serde(rename = "type")]
    pub account_type: String,
    pub rgs_code: Option<String>,
    pub normal_balance: String,
    pub active: u8,
    pub created_at: String,
}

fn account_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Account> {
    Ok(Account {
        id: row.get(0)?,
        code: row.get(1)?,
        name: row.get(2)?,
        account_type: row.get(3)?,
        rgs_code: row.get(4)?,
        normal_balance: row.get(5)?,
        active: row.get(6)?,
        created_at: row.get(7)?,
    })
}

pub fn validate_account(code: &str, name: &str, account_type: &str, normal_balance: &str, rgs_code: Option<&str>) -> Result<()> {
    if !code_re().is_match(code) {
        return Err(AppError::new("INVALID_CODE", format!("account code '{code}' must be 1-6 digits")));
    }
    if name.trim().is_empty() {
        return Err(AppError::new("INVALID_NAME", "account name is required"));
    }
    if !VALID_TYPES.contains(&account_type) {
        return Err(AppError::new("INVALID_TYPE", format!("account type '{account_type}' must be one of {}", VALID_TYPES.join(", "))));
    }
    if !VALID_NORMAL.contains(&normal_balance) {
        return Err(AppError::new("INVALID_NORMAL_BALANCE", format!("normal_balance '{normal_balance}' must be debit or credit")));
    }
    if let Some(rgs) = rgs_code {
        if !rgs.is_empty() && !rgs_re().is_match(rgs) {
            return Err(AppError::new("INVALID_RGS_CODE", format!("rgs_code '{rgs}' does not look like an RGS code (e.g. BMVA.02)")));
        }
    }
    Ok(())
}

pub fn create_account(conn: &Connection, code: &str, name: &str, account_type: &str, normal_balance: &str, rgs_code: Option<&str>) -> Result<Account> {
    validate_account(code, name, account_type, normal_balance, rgs_code)?;
    let result = conn.execute(
        "INSERT INTO accounts (code, name, type, rgs_code, normal_balance) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![code, name.trim(), account_type, rgs_code, normal_balance],
    );
    let id = match result {
        Ok(_) => conn.last_insert_rowid(),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("UNIQUE constraint failed: accounts.code") {
                return Err(AppError::new("ACCOUNT_EXISTS", format!("account code {code} already exists")));
            }
            if msg.contains("CHECK constraint failed") {
                let required = if account_type == "asset" || account_type == "expense" { "debit" } else { "credit" };
                return Err(AppError::new(
                    "INVALID_COMBINATION",
                    format!("type '{account_type}' requires normal_balance '{required}'"),
                ));
            }
            return Err(e.into());
        }
    };
    get_account(conn, id)?.ok_or_else(|| AppError::new("ACCOUNT_NOT_FOUND", format!("account {id} not found after create")))
}

pub fn get_account(conn: &Connection, id: i64) -> Result<Option<Account>> {
    let row = conn
        .query_row("SELECT * FROM accounts WHERE id = ?1", [id], account_from_row)
        .optional()?;
    Ok(row)
}

pub fn get_account_by_code(conn: &Connection, code: &str) -> Result<Option<Account>> {
    let row = conn
        .query_row("SELECT * FROM accounts WHERE code = ?1", [code], account_from_row)
        .optional()?;
    Ok(row)
}

pub fn list_accounts(conn: &Connection, account_type: Option<&str>, include_inactive: bool) -> Result<Vec<Account>> {
    let mut sql = String::from("SELECT * FROM accounts");
    let mut clauses: Vec<String> = Vec::new();
    if let Some(t) = account_type {
        clauses.push(format!("type = '{}'", t));
    }
    if !include_inactive {
        clauses.push("active = 1".to_string());
    }
    if !clauses.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&clauses.join(" AND "));
    }
    sql.push_str(" ORDER BY code");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], account_from_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Seed the default chart; skips codes that already exist. Returns count of new accounts.
pub fn seed_default_chart(conn: &Connection) -> Result<i64> {
    let mut created = 0;
    for a in DEFAULT_CHART {
        if get_account_by_code(conn, a.code)?.is_some() {
            continue;
        }
        create_account(conn, a.code, a.name, a.account_type, a.normal_balance, Some(a.rgs_code))?;
        created += 1;
    }
    Ok(created)
}

/// Create a single account from an AccountSeed (used by the VAT module).
pub fn create_account_from_seed(conn: &Connection, seed: &AccountSeed) -> Result<Account> {
    create_account(conn, seed.code, seed.name, seed.account_type, seed.normal_balance, Some(seed.rgs_code))
}

pub fn deactivate_account(conn: &Connection, code: &str) -> Result<Account> {
    let account = get_account_by_code(conn, code)?
        .ok_or_else(|| AppError::new("ACCOUNT_NOT_FOUND", format!("account {code} does not exist")))?;
    if account.active == 0 {
        return Err(AppError::new("ALREADY_INACTIVE", format!("account {code} is already inactive")));
    }
    conn.execute("UPDATE accounts SET active = 0 WHERE id = ?1", [account.id])?;
    get_account(conn, account.id)?.ok_or_else(|| AppError::new("ACCOUNT_NOT_FOUND", format!("account {code} does not exist")))
}

pub fn reactivate_account(conn: &Connection, code: &str) -> Result<Account> {
    let account = get_account_by_code(conn, code)?
        .ok_or_else(|| AppError::new("ACCOUNT_NOT_FOUND", format!("account {code} does not exist")))?;
    if account.active == 1 {
        return Err(AppError::new("ALREADY_ACTIVE", format!("account {code} is already active")));
    }
    conn.execute("UPDATE accounts SET active = 1 WHERE id = ?1", [account.id])?;
    get_account(conn, account.id)?.ok_or_else(|| AppError::new("ACCOUNT_NOT_FOUND", format!("account {code} does not exist")))
}

#[derive(Debug, Serialize)]
pub struct CsvImportError {
    pub line: usize,
    pub error: String,
}

#[derive(Debug, Serialize)]
pub struct CsvImportResult {
    pub created: usize,
    pub skipped: usize,
    pub total: usize,
    pub errors: Vec<CsvImportError>,
}

/// Import a chart from CSV. Columns (header row required):
///   code,name,type,normal_balance[,rgs_code]
/// Valid rows are created; invalid rows are skipped with a reported error.
pub fn import_chart_csv(conn: &Connection, csv_text: &str) -> Result<CsvImportResult> {
    let lines: Vec<&str> = csv_text
        .split('\n')
        .map(|l| l.trim_end_matches('\r'))
        .filter(|l| !l.trim().is_empty())
        .collect();
    if lines.len() < 2 {
        return Err(AppError::new("EMPTY_CSV", "chart CSV must have a header row and at least one account"));
    }
    let header = parse_csv_line(lines[0]);
    let header: Vec<String> = header.iter().map(|h| h.trim().to_string()).collect();
    let expected = ["code", "name", "type", "normal_balance"];
    for col in expected {
        if !header.iter().any(|h| h == col) {
            return Err(AppError::new(
                "INVALID_CSV_HEADER",
                format!("chart CSV is missing column '{col}' (got: {})", header.join(",")),
            ));
        }
    }
    let idx_of = |name: &str| -> Option<usize> { header.iter().position(|h| h == name) };

    let mut created = 0usize;
    let mut errors: Vec<CsvImportError> = Vec::new();
    for (i, line) in lines.iter().enumerate().skip(1) {
        let row: Vec<String> = parse_csv_line(line).iter().map(|c| c.trim().to_string()).collect();
        if row.len() == 1 && row[0].is_empty() {
            continue;
        }
        let code = idx_of("code").and_then(|ix| row.get(ix)).cloned().unwrap_or_default();
        let name = idx_of("name").and_then(|ix| row.get(ix)).cloned().unwrap_or_default();
        let account_type = idx_of("type").and_then(|ix| row.get(ix)).cloned().unwrap_or_default();
        let normal_balance = idx_of("normal_balance").and_then(|ix| row.get(ix)).cloned().unwrap_or_default();
        let rgs_code = idx_of("rgs_code")
            .and_then(|ix| row.get(ix))
            .cloned()
            .filter(|c| !c.is_empty());

        let result = (|| -> Result<()> {
            validate_account(&code, &name, &account_type, &normal_balance, rgs_code.as_deref())?;
            if get_account_by_code(conn, &code)?.is_some() {
                return Err(AppError::new("ACCOUNT_EXISTS", format!("account {code} already exists (skipped)")));
            }
            create_account(conn, &code, &name, &account_type, &normal_balance, rgs_code.as_deref())?;
            Ok(())
        })();
        match result {
            Ok(()) => created += 1,
            Err(e) => errors.push(CsvImportError { line: i + 1, error: format!("{}: {}", e.code(), e.message) }),
        }
    }
    Ok(CsvImportResult { created, skipped: errors.len(), total: lines.len() - 1, errors })
}

/// Port of the Node `parseCsvLine`: split on commas, honour double quotes.
pub fn parse_csv_line(line: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    for ch in line.chars() {
        if ch == '"' {
            in_quotes = !in_quotes;
        } else if ch == ',' && !in_quotes {
            out.push(std::mem::take(&mut cur));
        } else {
            cur.push(ch);
        }
    }
    out.push(cur);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::db::open_in_memory;

    #[test]
    fn seed_default_chart_creates_28() {
        let conn = open_in_memory().unwrap();
        let created = seed_default_chart(&conn).unwrap();
        assert_eq!(created, 28);
        // idempotent
        assert_eq!(seed_default_chart(&conn).unwrap(), 0);
        assert_eq!(list_accounts(&conn, None, false).unwrap().len(), 28);
    }

    #[test]
    fn create_account_validates() {
        let conn = open_in_memory().unwrap();
        let err = create_account(&conn, "abc", "Bad", "asset", "debit", None).unwrap_err();
        assert_eq!(err.code(), "INVALID_CODE");
        let err = create_account(&conn, "5000", "Bad", "asset", "credit", None).unwrap_err();
        assert_eq!(err.code(), "INVALID_COMBINATION");
        let err = create_account(&conn, "5000", "Bad", "nope", "debit", None).unwrap_err();
        assert_eq!(err.code(), "INVALID_TYPE");
    }

    #[test]
    fn create_account_duplicate_code() {
        let conn = open_in_memory().unwrap();
        create_account(&conn, "5000", "One", "expense", "debit", None).unwrap();
        let err = create_account(&conn, "5000", "Two", "expense", "debit", None).unwrap_err();
        assert_eq!(err.code(), "ACCOUNT_EXISTS");
    }

    #[test]
    fn deactivate_reactivate() {
        let conn = open_in_memory().unwrap();
        seed_default_chart(&conn).unwrap();
        let a = deactivate_account(&conn, "1100").unwrap();
        assert_eq!(a.active, 0);
        let err = deactivate_account(&conn, "1100").unwrap_err();
        assert_eq!(err.code(), "ALREADY_INACTIVE");
        let a = reactivate_account(&conn, "1100").unwrap();
        assert_eq!(a.active, 1);
        let err = reactivate_account(&conn, "1100").unwrap_err();
        assert_eq!(err.code(), "ALREADY_ACTIVE");
        // inactive accounts are hidden unless requested
        deactivate_account(&conn, "1100").unwrap();
        assert!(get_account_by_code(&conn, "1100").unwrap().unwrap().active == 0);
        assert!(list_accounts(&conn, None, false).unwrap().iter().all(|a| a.active == 1));
    }

    #[test]
    fn csv_import() {
        let conn = open_in_memory().unwrap();
        let csv = "code,name,type,normal_balance,rgs_code\n\
                   5000,Test kosten,expense,debit,WKPR.70\n\
                   5100,Slecht,expense,credit,\n\
                   bad,Bad code,expense,debit,\n\
                   5000,Dupe,expense,debit,\n";
        let res = import_chart_csv(&conn, csv).unwrap();
        assert_eq!(res.created, 1);
        assert_eq!(res.skipped, 3);
        assert_eq!(res.total, 4);
        assert_eq!(res.errors[0].line, 3);
        assert!(res.errors[0].error.starts_with("INVALID_COMBINATION"));
        assert!(res.errors[1].error.starts_with("INVALID_CODE"));
        assert!(res.errors[2].error.starts_with("ACCOUNT_EXISTS"));
    }

    #[test]
    fn csv_import_missing_header() {
        let conn = open_in_memory().unwrap();
        let err = import_chart_csv(&conn, "foo,bar\n1,2\n").unwrap_err();
        assert_eq!(err.code(), "INVALID_CSV_HEADER");
        let err = import_chart_csv(&conn, "code,name,type,normal_balance\n").unwrap_err();
        assert_eq!(err.code(), "EMPTY_CSV");
    }

    #[test]
    fn csv_line_quotes() {
        assert_eq!(parse_csv_line("a,\"b,c\",d"), vec!["a", "b,c", "d"]);
        assert_eq!(parse_csv_line("plain"), vec!["plain"]);
    }
}
