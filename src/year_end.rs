// bukio-cli — agent-first double-entry bookkeeping for SMEs.
// Copyright (c) 2026 Erik van Kempen.
// SPDX-License-Identifier: Apache-2.0
//
// Year-end close (mirrors src/year-end/index.js).

use crate::accounts::{create_account, get_account_by_code, resolve_profile, NewAccount};
use crate::audit::{record, RecordArgs};
use crate::entries::{create_entry, list_entries, post_entry, CreateEntry, PostingSpec};
use crate::money::{format_amount, BukioError, Result};
use rusqlite::Connection;
use serde_json::{json, Value};

fn year_end_error(code: &'static str, msg: impl Into<String>) -> BukioError {
    BukioError::new(code, msg.into())
}

/// Fiscal-year window: [from, to] ISO dates for a closing year.
pub fn fiscal_year_window(db: &Connection, year: &str) -> Result<(String, String)> {
    let fy: String = db
        .query_row(
            "SELECT fiscal_year_end FROM company WHERE id = 1",
            [],
            |r| r.get(0),
        )
        .unwrap_or_else(|_| "12-31".into());
    let parts: Vec<&str> = fy.split('-').collect();
    let mm: u32 = parts[parts.len() - 2].parse().unwrap_or(12);
    let dd: u32 = parts[parts.len() - 1].parse().unwrap_or(31);
    if mm == 12 && dd == 31 {
        return Ok((format!("{year}-01-01"), format!("{year}-12-31")));
    }
    let end = format!("{year}-{mm:02}-{dd:02}");
    let y: i32 = year.parse().unwrap_or(2026);
    let start = {
        let d = chrono::NaiveDate::from_ymd_opt(y - 1, mm, dd + 1)
            .unwrap_or_else(|| chrono::NaiveDate::from_ymd_opt(y - 1, mm, 1).unwrap());
        d.format("%Y-%m-%d").to_string()
    };
    Ok((start, end))
}

/// Net per income/expense account in the year (posted, excluding closing).
pub fn result_accounts(db: &Connection, year: &str) -> Result<Vec<Value>> {
    let (from, to) = fiscal_year_window(db, year)?;
    let sql = "SELECT a.code, a.name, a.type,
                     COALESCE(SUM(p.amount_cents), 0) AS net_cents
              FROM accounts a
              LEFT JOIN (
                SELECT p.account_id, p.amount_cents
                FROM postings p
                JOIN journal_entries e ON e.id = p.entry_id
                WHERE e.state = 'posted' AND e.source != 'closing'
                  AND e.date >= ?1 AND e.date <= ?2
              ) p ON p.account_id = a.id
              WHERE a.type IN ('income', 'expense')
              GROUP BY a.id
              HAVING net_cents != 0
              ORDER BY a.code";
    let mut stmt = db.prepare(sql).map_err(sql_err)?;
    let rows = stmt
        .query_map(rusqlite::params![from, to], |r| {
            Ok(json!({
                "code": r.get::<_, String>(0)?,
                "name": r.get::<_, String>(1)?,
                "type": r.get::<_, String>(2)?,
                "net_cents": r.get::<_, i64>(3)?,
            }))
        })
        .map_err(sql_err)?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

/// Check if a year is already closed (unreversed closing entries exist).
pub fn is_year_closed(db: &Connection, year: &str) -> Result<bool> {
    let sql = "SELECT 1 FROM journal_entries e
               WHERE e.source = 'closing' AND e.source_ref = ?1
                 AND NOT EXISTS (
                   SELECT 1 FROM journal_entries r
                   WHERE r.reversed_from_id = e.id
                 )
               LIMIT 1";
    let result = db.query_row(sql, [format!("fy:{year}")], |_| Ok(true));
    Ok(result.unwrap_or(false))
}

/// Year-end status: is it closed, result amount, accounts, closing entries.
pub fn year_end_status(db: &Connection, year: &str) -> Result<Value> {
    if year.len() != 4 || !year.bytes().all(|b| b.is_ascii_digit()) {
        return Err(year_end_error("INVALID_YEAR", format!("year '{year}' must be YYYY")));
    }
    let sql = "SELECT id, date, description, state FROM journal_entries
               WHERE source = 'closing' AND source_ref = ?1 ORDER BY id";
    let closing_entries: Vec<Value> = db
        .prepare(sql)
        .map_err(sql_err)?
        .query_map([format!("fy:{year}")], |r| {
            Ok(json!({
                "id": r.get::<_, i64>(0)?,
                "date": r.get::<_, String>(1)?,
                "description": r.get::<_, String>(2)?,
                "state": r.get::<_, String>(3)?,
            }))
        })
        .map_err(sql_err)?
        .filter_map(|r| r.ok())
        .collect();

    let accounts = result_accounts(db, year)?;
    let raw_result: i64 = accounts.iter().filter_map(|a| a["net_cents"].as_i64()).sum();
    let result_cents = if raw_result == 0 { 0 } else { -raw_result };

    Ok(json!({
        "year": year,
        "closed": !closing_entries.is_empty(),
        "result_cents": result_cents,
        "accounts": accounts,
        "closing_entries": closing_entries,
    }))
}

/// Close the fiscal year.
pub fn year_end_close(
    db: &Connection,
    year: &str,
    actor: &str,
    dry_run: bool,
) -> Result<Value> {
    if year.len() != 4 || !year.bytes().all(|b| b.is_ascii_digit()) {
        return Err(year_end_error("INVALID_YEAR", format!("year '{year}' must be YYYY")));
    }
    let company_exists: bool = db
        .query_row("SELECT 1 FROM company WHERE id = 1", [], |_| Ok(true))
        .unwrap_or(false);
    if !company_exists {
        return Err(year_end_error("NOT_INITIALISED", "company database not initialised"));
    }

    let profile = resolve_profile(db)?;
    let closing = &profile["closing"];
    let result_account = closing["resultAccount"].as_str().unwrap_or("9900");
    let equity_account = closing["equityAccount"].as_str().unwrap_or("3000");

    let (fy_from, fy_to) = fiscal_year_window(db, year)?;

    // check for draft entries
    let drafts: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM journal_entries WHERE state = 'draft' AND date >= ?1 AND date <= ?2",
            rusqlite::params![fy_from, fy_to],
            |r| r.get(0),
        )
        .map_err(sql_err)?;
    if drafts > 0 {
        return Err(year_end_error("INCOMPLETE_YEAR", format!("{drafts} draft entry/entries in {year} — post or reverse them before closing")));
    }

    if is_year_closed(db, year)? {
        return Err(year_end_error("ALREADY_CLOSED", format!("{year} is already closed (undo with entry reverse on the closing entries)")));
    }

    let accounts = result_accounts(db, year)?;
    if accounts.is_empty() {
        return Ok(json!({
            "closed": false, "year": year, "reason": "EMPTY_YEAR",
            "result_cents": 0,
            "message": format!("no income/expense activity in {year} — nothing to close"),
        }));
    }

    let raw_result: i64 = accounts.iter().filter_map(|a| a["net_cents"].as_i64()).sum();
    let result_cents = if raw_result == 0 { 0 } else { -raw_result };

    // build closing postings
    let mut closing_postings: Vec<PostingSpec> = accounts
        .iter()
        .filter_map(|a| {
            let net = a["net_cents"].as_i64()?;
            let code = a["code"].as_str()?.to_string();
            Some(PostingSpec { code, amount_cents: -net, cost_center_code: None })
        })
        .collect();
    if result_cents != 0 {
        closing_postings.push(PostingSpec {
            code: result_account.to_string(),
            amount_cents: -result_cents,
            cost_center_code: None,
        });
    }

    // appropriation postings (only when there's a result)
    let appropriation_postings: Vec<PostingSpec> = if result_cents != 0 {
        vec![
            PostingSpec { code: result_account.to_string(), amount_cents: result_cents, cost_center_code: None },
            PostingSpec { code: equity_account.to_string(), amount_cents: -result_cents, cost_center_code: None },
        ]
    } else {
        vec![]
    };

    if dry_run {
        let closing_json: Vec<Value> = closing_postings.iter().map(|p| json!({ "code": p.code, "amountCents": p.amount_cents })).collect();
        let appropriation_json: Vec<Value> = appropriation_postings.iter().map(|p| json!({ "code": p.code, "amountCents": p.amount_cents })).collect();
        let mut entries = vec![json!({ "description": format!("Afsluiting boekjaar {year}"), "postings": closing_json })];
        if !appropriation_json.is_empty() {
            entries.push(json!({ "description": format!("Resultaatbestemming {year}"), "postings": appropriation_json }));
        }
        return Ok(json!({
            "closed": false, "year": year, "dryRun": true,
            "result_cents": result_cents,
            "create_9900": result_cents != 0 && get_account_by_code(db, result_account).is_none(),
            "entries": entries,
        }));
    }

    let close_date = &fy_to;
    {
        // create 9900 if needed
        if result_cents != 0 && get_account_by_code(db, result_account).is_none() {
            create_account(db, &NewAccount {
                code: result_account,
                name: "Resultaat boekjaar",
                type_: "equity",
                normal_balance: "credit",
                taxonomy_code: Some("BEIV.05"),
            })?;
        }

        // entry 1: closing
        let e1 = create_entry(db, CreateEntry {
            date: close_date,
            description: &format!("Afsluiting boekjaar {year}"),
            postings: closing_postings,
            source: "closing",
            source_ref: Some(&format!("fy:{year}")),
            actor: "closing",
        })?;
        let p1 = post_entry(db, e1.id, "closing")?;

        let mut results = vec![p1];

        // entry 2: appropriation (only when there's a result)
        if !appropriation_postings.is_empty() {
            let e2 = create_entry(db, CreateEntry {
                date: close_date,
                description: &format!("Resultaatbestemming {year}"),
                postings: appropriation_postings,
                source: "closing",
                source_ref: Some(&format!("fy:{year}")),
                actor: "closing",
            })?;
            let p2 = post_entry(db, e2.id, "closing")?;
            results.push(p2);
        }

        record(db, RecordArgs {
            actor,
            action: "year_end.close",
            command: Some("year-end close"),
            args: Some(json!({ "year": year, "result_cents": result_cents })),
            outcome: "ok",
            entry_ids: results.iter().map(|e| e.id).collect(),
        })?;
    }

    let closing_entries = year_end_status(db, year)?;
    Ok(json!({
        "closed": true, "year": year, "result_cents": result_cents,
        "entries": closing_entries["closing_entries"],
    }))
}

fn sql_err(e: rusqlite::Error) -> BukioError {
    BukioError::new("DB_ERROR", e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_db;

    fn db_with_data() -> Connection {
        let d = open_db(":memory:").unwrap();
        d.execute("INSERT INTO company (name) VALUES ('YECo')", []).unwrap();
        crate::accounts::seed_default_chart(&d).unwrap();
        // post some income/expense
        crate::entries::create_entry(&d, CreateEntry {
            date: "2026-03-15", description: "Verkoop",
            postings: vec![
                PostingSpec { code: "1100".into(), amount_cents: 12100, cost_center_code: None },
                PostingSpec { code: "8000".into(), amount_cents: -12100, cost_center_code: None },
            ],
            source: "manual", source_ref: None, actor: "human:erik",
        }).unwrap();
        crate::entries::post_entry(&d, 1, "human:erik").unwrap();
        crate::entries::create_entry(&d, CreateEntry {
            date: "2026-04-01", description: "Kosten",
            postings: vec![
                PostingSpec { code: "4300".into(), amount_cents: -5000, cost_center_code: None },
                PostingSpec { code: "1100".into(), amount_cents: 5000, cost_center_code: None },
            ],
            source: "manual", source_ref: None, actor: "human:erik",
        }).unwrap();
        crate::entries::post_entry(&d, 2, "human:erik").unwrap();
        d
    }

    #[test]
    fn result_accounts_picks_income_and_expense() {
        let d = db_with_data();
        let accounts = result_accounts(&d, "2026").unwrap();
        assert!(accounts.len() >= 2);
        let total: i64 = accounts.iter().filter_map(|a| a["net_cents"].as_i64()).sum();
        assert_ne!(total, 0);
    }

    #[test]
    fn year_end_close_creates_closing_entries() {
        let d = db_with_data();
        let result = year_end_close(&d, "2026", "human:erik", false).unwrap();
        assert_eq!(result["closed"], true);
        assert_eq!(result["year"], "2026");
        // check 9900 was created
        let acc = get_account_by_code(&d, "9900");
        assert!(acc.is_some());
    }

    #[test]
    fn is_year_closed_prevents_double_close() {
        let d = db_with_data();
        year_end_close(&d, "2026", "human:erik", false).unwrap();
        let err = year_end_close(&d, "2026", "human:erik", false).unwrap_err();
        assert_eq!(err.code, "ALREADY_CLOSED");
    }
}
