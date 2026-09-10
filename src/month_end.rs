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
        return Err(month_end_error(
            "INVALID_PERIOD",
            format!("period '{period}' must be yyyy-mm"),
        ));
    }
    let year: i32 = parts[0].parse().map_err(|_| {
        month_end_error(
            "INVALID_PERIOD",
            format!("period '{period}' must be yyyy-mm"),
        )
    })?;
    let month: u32 = parts[1].parse().map_err(|_| {
        month_end_error(
            "INVALID_PERIOD",
            format!("period '{period}' must be yyyy-mm"),
        )
    })?;
    if month < 1 || month > 12 {
        return Err(month_end_error(
            "INVALID_PERIOD",
            format!("period '{period}' must be yyyy-mm"),
        ));
    }
    let from = format!("{period}-01");
    let last_day = {
        let d = chrono::NaiveDate::from_ymd_opt(year, month + 1, 1)
            .unwrap_or_else(|| chrono::NaiveDate::from_ymd_opt(year + 1, 1, 1).unwrap());
        (d - chrono::Duration::days(1))
            .format("%d")
            .to_string()
            .parse::<u32>()
            .unwrap_or(30)
    };
    let to = format!("{period}-{last_day:02}");
    Ok((from, to))
}

pub fn month_end(db: &Connection, period: &str) -> Result<Value> {
    if !period.contains('-') || period.len() != 7 {
        return Err(month_end_error(
            "INVALID_PERIOD",
            format!("period '{period}' must be yyyy-mm"),
        ));
    }
    let (from, to) = month_bounds(period)?;

    // draft entries
    let draft_sql =
        "SELECT id FROM journal_entries WHERE state = 'draft' AND date >= ?1 AND date <= ?2";
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
        .filter(|t| {
            t["date"]
                .as_str()
                .map_or(false, |d| d >= from.as_str() && d <= to.as_str())
        })
        .cloned()
        .collect();

    // invoices — every invoice, draft/overdue counted from its status
    let invoices = crate::invoice::list_invoices(db, None, None)?;
    let draft_invoices: Vec<Value> = invoices
        .iter()
        .filter(|i| i["status"].as_str() == Some("draft"))
        .cloned()
        .collect();
    let overdue_invoices: Vec<Value> = invoices
        .iter()
        .filter(|i| i["status"].as_str() == Some("overdue"))
        .cloned()
        .collect();

    // recurring — active templates due by the end of the month
    let recurring_due: Vec<Value> = crate::recurring::list_templates(db, "active")?
        .into_iter()
        .filter(|t| {
            t["next_run_date"]
                .as_str()
                .map_or(false, |d| d <= to.as_str())
        })
        .collect();

    // assets — depreciation runs due in this period (a pre-assets DB has none)
    let assets_due = crate::assets::run_due(db, period, "agent:test", true)
        .ok()
        .and_then(|p| p["plan"].as_array().map(|a| a.len() as i64))
        .unwrap_or(0);

    // VAT readout for the quarter
    let quarter_num = (period[5..7].parse::<u32>().unwrap_or(1) - 1) / 3 + 1;
    let quarter = format!("{}-Q{quarter_num}", &period[..4]);
    let vat = if is_vat_enabled(db) {
        match ob_readout(db, &quarter) {
            Ok(r) => Some(json!({
                "quarter": r["period"],
                "from": r["from"],
                "to": r["to"],
                "fields": r["fields"],
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
        .query_map(rusqlite::params![from, to], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .map_err(sql_err)?
        .filter_map(|r| r.ok())
        .collect();
    let debit: i64 = rows
        .iter()
        .filter(|(_, _, net)| *net > 0)
        .map(|(_, _, net)| net)
        .sum();
    let credit: i64 = rows
        .iter()
        .filter(|(_, _, net)| *net < 0)
        .map(|(_, _, net)| -net)
        .sum();
    let profit_cents: i64 = rows
        .iter()
        .filter(|(_, t, _)| t == "income" || t == "expense")
        .map(|(_, _, net)| -net)
        .sum();

    // warnings
    let mut warnings = Vec::new();
    if !draft_ids.is_empty() {
        warnings.push(format!(
            "{} draft entr{} not posted",
            draft_ids.len(),
            if draft_ids.len() == 1 { "y" } else { "ies" }
        ));
    }
    if !draft_invoices.is_empty() {
        warnings.push(format!(
            "{} draft invoice{} not finalised — booked revenue may be uninvoiced",
            draft_invoices.len(),
            if draft_invoices.len() == 1 { "" } else { "s" }
        ));
    }
    if !bank_unmatched.is_empty() {
        warnings.push(format!(
            "{} unmatched bank transaction{}",
            bank_unmatched.len(),
            if bank_unmatched.len() == 1 { "" } else { "s" }
        ));
    }
    if !overdue_invoices.is_empty() {
        warnings.push(format!(
            "{} overdue invoice{}",
            overdue_invoices.len(),
            if overdue_invoices.len() == 1 { "" } else { "s" }
        ));
    }
    if !recurring_due.is_empty() {
        warnings.push(format!(
            "{} recurring template{} due by {to}",
            recurring_due.len(),
            if recurring_due.len() == 1 { "" } else { "s" }
        ));
    }
    if assets_due > 0 {
        warnings.push(format!(
            "{assets_due} depreciation run{} due — run 'assets run --period {period}'",
            if assets_due == 1 { "" } else { "s" }
        ));
    }
    if !draft_ids.is_empty()
        || !draft_invoices.is_empty()
        || !bank_unmatched.is_empty()
        || !overdue_invoices.is_empty()
        || !recurring_due.is_empty()
        || assets_due > 0
    {
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
        "invoices": {
            "draft": draft_invoices.len(),
            "draft_ids": draft_invoices.iter().filter_map(|i| i["id"].as_i64()).collect::<Vec<_>>(),
            "overdue": overdue_invoices.len(),
            "overdue_ids": overdue_invoices.iter().filter_map(|i| i["id"].as_i64()).collect::<Vec<_>>(),
            "overdue_total_cents": overdue_invoices.iter().map(|i| {
                i["outstanding_cents"].as_i64().unwrap_or_else(|| {
                    i["gross_cents"].as_i64().unwrap_or(0) - i["paid_cents"].as_i64().unwrap_or(0)
                })
            }).sum::<i64>(),
        },
        "recurring": {
            "due": recurring_due.len(),
            "due_ids": recurring_due.iter().filter_map(|t| t["id"].as_i64()).collect::<Vec<_>>(),
        },
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

// ==== ported from test/month-end.test.js ====================================
#[cfg(test)]
mod month_end_tests {
    use super::*;
    use crate::bank::BankTx;
    use crate::entries::{create_entry, post_entry, CreateEntry, PostingSpec};
    use rusqlite::Connection;
    use serde_json::{json, Value};

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

    /// The JS test's post(date, description, postings) helper.
    fn post(db: &Connection, date: &str, description: &str, postings: Vec<PostingSpec>) -> i64 {
        let e = create_entry(
            db,
            CreateEntry {
                date,
                description,
                postings,
                source: "manual",
                source_ref: None,
                actor: "agent:test",
            },
        )
        .unwrap();
        post_entry(db, e.id, "agent:test").unwrap();
        e.id
    }

    fn specs(pairs: &[(&str, i64)]) -> Vec<PostingSpec> {
        pairs
            .iter()
            .map(|(code, amount_cents)| PostingSpec {
                code: (*code).to_string(),
                amount_cents: *amount_cents,
                cost_center_code: None,
                vat_code: None,
                vat_amount_cents: None,
            })
            .collect()
    }

    fn warnings(r: &Value) -> Vec<String> {
        r["warnings"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .map(|w| w.as_str().unwrap_or("").to_string())
            .collect()
    }

    #[test]
    fn clean_month_is_all_clear_with_zero_totals() {
        let d = idb();
        let r = month_end(&d, "2026-01").unwrap();
        assert_eq!(r["period"].as_str(), Some("2026-01"));
        assert_eq!(r["from"].as_str(), Some("2026-01-01"));
        assert_eq!(r["to"].as_str(), Some("2026-01-31"));
        assert_eq!(r["entries"]["draft"].as_i64(), Some(0));
        assert_eq!(r["bank"]["unmatched"].as_i64(), Some(0));
        assert_eq!(r["invoices"]["overdue"].as_i64(), Some(0));
        assert_eq!(r["recurring"]["due"].as_i64(), Some(0));
        assert_eq!(r["totals"]["balanced"], json!(true));
        assert_eq!(r["totals"]["profit_cents"].as_i64(), Some(0));
        assert_eq!(
            warnings(&r),
            vec!["all clear — the month can be closed".to_string()]
        );
    }

    #[test]
    fn drafts_and_unmatched_bank_transactions_are_flagged() {
        let d = idb();
        create_entry(
            &d,
            CreateEntry {
                date: "2026-01-10",
                description: "Concept",
                postings: specs(&[("1100", 100), ("3000", -100)]),
                source: "manual",
                source_ref: None,
                actor: "agent:test",
            },
        )
        .unwrap();
        crate::bank::import_transactions(
            &d,
            "NL91ABNA0417164300",
            &[BankTx {
                date: "2026-01-15".to_string(),
                amount_cents: 5000,
                counterparty: Some("ACME".to_string()),
                description: Some("Betaling".to_string()),
                iban_counter: Some("NL02ABNA0123456789".to_string()),
                bank_ref: None,
                iban: None,
            }],
            None,
            "1100",
            "agent:test",
        )
        .unwrap();
        let r = month_end(&d, "2026-01").unwrap();
        assert_eq!(r["entries"]["draft"].as_i64(), Some(1));
        assert_eq!(r["bank"]["unmatched"].as_i64(), Some(1));
        let w = warnings(&r);
        assert!(w.iter().any(|x| x.contains("1 draft entry")), "{w:?}");
        assert!(
            w.iter().any(|x| x.contains("1 unmatched bank transaction")),
            "{w:?}"
        );
    }

    #[test]
    fn vat_quarter_readout_when_module_on() {
        let d = idb();
        crate::vat::book_vat_entry(
            &d,
            "2026-01-20",
            "Verkoop",
            &crate::vat::parse_vat_posting_specs(&["1100:121.00,8000:-100.00@21".to_string()])
                .unwrap(),
            "manual",
            None,
            "agent:test",
            true,
        )
        .unwrap();
        let r = month_end(&d, "2026-01").unwrap();
        assert!(!r["vat"].is_null(), "vat readout expected");
        assert_eq!(r["vat"]["quarter"].as_str(), Some("2026-Q1"));
        assert_eq!(r["vat"]["to_pay_cents"].as_i64(), Some(2100));
        assert_eq!(r["vat"]["fields"]["1a"].as_i64(), Some(10000));
    }

    #[test]
    fn profit_is_income_minus_expense_for_the_period() {
        let d = idb();
        post(
            &d,
            "2026-01-05",
            "Verkoop",
            specs(&[("1100", 100000), ("8000", -100000)]),
        );
        post(
            &d,
            "2026-01-06",
            "Inkoop",
            specs(&[("4000", 30000), ("1100", -30000)]),
        );
        let r = month_end(&d, "2026-01").unwrap();
        assert_eq!(r["totals"]["profit_cents"].as_i64(), Some(70000));
        assert_eq!(r["totals"]["profit"].as_str(), Some("700.00"));
        assert_eq!(r["totals"]["balanced"], json!(true));
        assert_eq!(r["totals"]["debit_cents"].as_i64(), Some(100000));
        assert_eq!(r["totals"]["credit_cents"].as_i64(), Some(100000));
    }

    #[test]
    fn december_totals_exclude_year_end_closing_entries() {
        let d = idb();
        post(
            &d,
            "2026-12-05",
            "Verkoop",
            specs(&[("1100", 12100), ("8000", -10000), ("2500", -2100)]),
        );
        crate::year_end::year_end_close(&d, "2026", "agent:test", false).unwrap();
        let r = month_end(&d, "2026-12").unwrap();
        assert_eq!(r["totals"]["profit_cents"].as_i64(), Some(10000));
        assert_eq!(r["totals"]["balanced"], json!(true));
        assert_eq!(r["totals"]["debit_cents"].as_i64(), Some(12100));
        assert_eq!(r["totals"]["credit_cents"].as_i64(), Some(12100));
    }

    #[test]
    fn overdue_invoice_warning_with_outstanding_total() {
        let d = idb();
        crate::contacts::create_contact(
            &d,
            "ACME B.V.",
            Some("Straat 1"),
            Some("1000 AA"),
            Some("Amsterdam"),
            None,
            None,
            None,
            None,
            None,
            "agent:test",
            false,
        )
        .unwrap();
        let inv = crate::invoice::create_invoice(
            &d,
            1,
            "2020-01-01",
            Some(0),
            None,
            None,
            None,
            None,
            None,
            &[json!("1x Oud @ 100.00 @21")],
            "agent:test",
            false,
        )
        .unwrap();
        crate::invoice::finalize_invoice(&d, inv["id"].as_i64().unwrap(), "agent:test", false)
            .unwrap();
        let r = month_end(&d, "2026-01").unwrap();
        assert_eq!(r["invoices"]["overdue"].as_i64(), Some(1));
        assert_eq!(r["invoices"]["overdue_total_cents"].as_i64(), Some(12100));
        let w = warnings(&r);
        assert!(w.iter().any(|x| x.contains("1 overdue invoice")), "{w:?}");
    }

    #[test]
    fn invalid_period_is_rejected() {
        let d = idb();
        let e = month_end(&d, "2026-13").unwrap_err();
        assert_eq!(e.code, "INVALID_PERIOD");
    }

    #[test]
    fn draft_invoices_are_warned() {
        let d = idb();
        crate::contacts::create_contact(
            &d,
            "Klant",
            Some("Straat 1"),
            None,
            Some("Amsterdam"),
            None,
            None,
            None,
            None,
            None,
            "agent:test",
            false,
        )
        .unwrap();
        let inv = crate::invoice::create_invoice(
            &d,
            1,
            "2026-01-10",
            Some(30),
            None,
            None,
            None,
            None,
            None,
            &[json!("1x Ding @ 100.00 @21")],
            "agent:test",
            false,
        )
        .unwrap();
        assert_eq!(inv["status"].as_str(), Some("draft"));
        let r = month_end(&d, "2026-01").unwrap();
        assert_eq!(r["invoices"]["draft"].as_i64(), Some(1));
        let w = warnings(&r);
        assert!(
            w.iter()
                .any(|x| x.contains("1 draft invoice not finalised")),
            "{w:?}"
        );
        assert!(!w.iter().any(|x| x.contains("all clear")), "{w:?}");
    }
}
