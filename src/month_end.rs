// bukio-cli — agent-first double-entry bookkeeping for SMEs.
// Copyright (c) 2026 Erik van Kempen.
// SPDX-License-Identifier: Apache-2.0
//
// Month-end close check (mirrors src/month-end/index.js).
// Read-only: drafts, unmatched bank txs, invoices, recurring, VAT.

use crate::bank::list_transactions;
use crate::entries::list_entries;
use crate::money::{format_amount, BukioError, Result};
use crate::vat::{is_vat_enabled, ob_readout};
use rusqlite::Connection;
use serde_json::{json, Value};

fn month_end_error(code: &'static str, msg: impl Into<String>) -> BukioError {
    BukioError::new(code, msg.into())
}

fn month_bounds(period: &str) -> Result<(String, String)> {
    let parts: Vec<&str> = period.split('-').collect();
    if parts.len() != 2 {
        return Err(month_end_error("INVALID_PERIOD", format!("period '{period}' must be yyyy-mm")));
    }
    let year: i32 = parts[0].parse().map_err(|_| month_end_error("INVALID_PERIOD", format!("period '{period}' must be yyyy-mm")))?;
    let month: u32 = parts[1].parse().map_err(|_| month_end_error("INVALID_PERIOD", format!("period '{period}' must be yyyy-mm")))?;
    if month < 1 || month > 12 {
        return Err(month_end_error("INVALID_PERIOD", format!("period '{period}' must be yyyy-mm")));
    }
    let from = format!("{period}-01");
    let last_day = {
        let d = chrono::NaiveDate::from_ymd_opt(year, month + 1, 1)
            .unwrap_or_else(|| chrono::NaiveDate::from_ymd_opt(year + 1, 1, 1).unwrap());
        (d - chrono::Duration::days(1)).format("%d").to_string().parse::<u32>().unwrap_or(30)
    };
    let to = format!("{period}-{last_day:02}");
    Ok((from, to))
}

pub fn month_end(db: &Connection, period: &str) -> Result<Value> {
    if !period.contains('-') || period.len() != 7 {
        return Err(month_end_error("INVALID_PERIOD", format!("period '{period}' must be yyyy-mm")));
    }
    let (from, to) = month_bounds(period)?;

    // draft entries
    let draft_sql = "SELECT id FROM journal_entries WHERE state = 'draft' AND date >= ?1 AND date <= ?2";
    let draft_ids: Vec<i64> = db
        .prepare(draft_sql)
        .map_err(sql_err)?
        .query_map(rusqlite::params![from, to], |r| r.get(0))
        .map_err(sql_err)?
        .filter_map(|r| r.ok())
        .collect();

    // unmatched bank transactions in period
    let bank_txs = list_transactions(db, Some("unmatched"), None, 100_000)?;
    let bank_unmatched: Vec<Value> = bank_txs
        .iter()
        .filter(|t| t["date"].as_str().map_or(false, |d| d >= from.as_str() && d <= to.as_str()))
        .cloned()
        .collect();

    // invoices — stub until invoice module is ported
    let draft_invoices: Vec<Value> = Vec::new();
    let overdue_invoices: Vec<Value> = Vec::new();

    // recurring — stub until recurring module is ported
    let recurring_due: Vec<Value> = Vec::new();

    // assets depreciation — stub until assets module is ported
    let assets_due = 0i64;

    // VAT readout for the quarter
    let quarter_num = (period[5..7].parse::<u32>().unwrap_or(1) - 1) / 3 + 1;
    let quarter = format!("{}-Q{quarter_num}", &period[..4]);
    let vat = if is_vat_enabled(db) {
        match ob_readout(db, &quarter) {
            Ok(r) => Some(json!({
                "quarter": r["period"],
                "from": r["from"],
                "to": r["to"],
                "to_pay_cents": r["to_pay_cents"],
                "to_pay": r["to_pay"],
            })),
            Err(_) => None,
        }
    } else {
        None
    };

    // period totals
    let totals_sql = "SELECT a.code, a.type, SUM(p.amount_cents) AS net
                      FROM postings p
                      JOIN journal_entries e ON e.id = p.entry_id AND e.state = 'posted'
                      JOIN accounts a ON a.id = p.account_id
                      WHERE e.date >= ?1 AND e.date <= ?2 AND e.source != 'closing'
                      GROUP BY a.id";
    let rows: Vec<(String, String, i64)> = db
        .prepare(totals_sql)
        .map_err(sql_err)?
        .query_map(rusqlite::params![from, to], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .map_err(sql_err)?
        .filter_map(|r| r.ok())
        .collect();
    let debit: i64 = rows.iter().filter(|(_, _, net)| *net > 0).map(|(_, _, net)| net).sum();
    let credit: i64 = rows.iter().filter(|(_, _, net)| *net < 0).map(|(_, _, net)| -net).sum();
    let profit_cents: i64 = rows
        .iter()
        .filter(|(_, t, _)| t == "income" || t == "expense")
        .map(|(_, _, net)| -net)
        .sum();

    // warnings
    let mut warnings = Vec::new();
    if !draft_ids.is_empty() {
        warnings.push(format!("{} draft entr{} not posted", draft_ids.len(), if draft_ids.len() == 1 { "y" } else { "ies" }));
    }
    if !bank_unmatched.is_empty() {
        warnings.push(format!("{} unmatched bank transaction{}", bank_unmatched.len(), if bank_unmatched.len() == 1 { "" } else { "s" }));
    }
    if !overdue_invoices.is_empty() {
        warnings.push(format!("{} overdue invoice{}", overdue_invoices.len(), if overdue_invoices.len() == 1 { "" } else { "s" }));
    }
    if !recurring_due.is_empty() {
        warnings.push(format!("{} recurring template{} due by {to}", recurring_due.len(), if recurring_due.len() == 1 { "" } else { "s" }));
    }
    if !draft_ids.is_empty() || !bank_unmatched.is_empty() || !overdue_invoices.is_empty() || !recurring_due.is_empty() || assets_due > 0 {
        // warnings exist
    } else {
        warnings.push("all clear — the month can be closed".into());
    }

    Ok(json!({
        "period": period,
        "from": from,
        "to": to,
        "entries": { "draft": draft_ids.len(), "draft_ids": draft_ids },
        "bank": { "unmatched": bank_unmatched.len(), "unmatched_ids": bank_unmatched.iter().filter_map(|t| t["id"].as_i64()).collect::<Vec<_>>() },
        "vat": vat,
        "invoices": { "draft": 0, "draft_ids": [], "overdue": 0, "overdue_ids": [], "overdue_total_cents": 0 },
        "recurring": { "due": 0, "due_ids": [] },
        "totals": {
            "debit_cents": debit,
            "credit_cents": credit,
            "balanced": debit == credit,
            "profit_cents": profit_cents,
            "profit": format_amount(profit_cents),
        },
        "warnings": warnings,
    }))
}

fn sql_err(e: rusqlite::Error) -> BukioError {
    BukioError::new("DB_ERROR", e.to_string())
}
