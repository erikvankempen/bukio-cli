// bukio-cli — agent-first double-entry bookkeeping for SMEs.
// Copyright (c) 2026 Erik van Kempen.
// SPDX-License-Identifier: Apache-2.0
//
// Reports (mirrors src/report/*.js + fiscalYearWindow from year-end).

use crate::dates::validate_labeled;
use crate::money::{BukioError, Result};
use rusqlite::Connection;
use serde_json::{json, Value};

/// fiscalYearWindow (mirrors year-end/index.js).
pub fn fiscal_year_window(db: &Connection, year: &str) -> (String, String) {
    let fy: String = db
        .query_row("SELECT fiscal_year_end FROM company WHERE id = 1", [], |r| r.get(0))
        .unwrap_or_else(|_| "12-31".into());
    let parts: Vec<&str> = fy.split('-').collect();
    let mm: u32 = parts[parts.len() - 2].parse().unwrap_or(12);
    let dd: u32 = parts[parts.len() - 1].parse().unwrap_or(31);
    if mm == 12 && dd == 31 {
        return (format!("{year}-01-01"), format!("{year}-12-31"));
    }
    let end = format!("{year}-{mm:02}-{dd:02}");
    // start = the day after (mm, dd) in the previous year
    let y: i32 = year.parse().unwrap_or(2000);
    let (sy, sm, sd) = next_day(y - 1, mm, dd);
    (format!("{sy:04}-{sm:02}-{sd:02}"), end)
}

fn next_day(y: i32, m: u32, d: u32) -> (i32, u32, u32) {
    let dim = days_in_month(y, m);
    if d + 1 <= dim {
        (y, m, d + 1)
    } else if m < 12 {
        (y, m + 1, 1)
    } else {
        (y + 1, 1, 1)
    }
}

fn days_in_month(y: i32, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 { 29 } else { 28 }
        }
        _ => 30,
    }
}

fn rgs_label(code: &str) -> String {
    let labels = resolve_profile_for_labels();
    labels
        .get(code)
        .and_then(|v| v.as_str())
        .unwrap_or(code)
        .to_string()
}

fn resolve_profile_for_labels() -> &'static Value {
    // NL labels are the canonical RGS labels (mirrors core/chart.js RGS_LABELS)
    static LABELS: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    LABELS.get_or_init(|| {
        crate::accounts::get_profile("NL").unwrap()["reporting"]["labels"].clone()
    })
}

fn valid_year(year: &str) -> Result<()> {
    if year.len() == 4 && year.bytes().all(|b| b.is_ascii_digit()) {
        Ok(())
    } else {
        Err(BukioError::new("INVALID_YEAR", format!("year '{year}' must be YYYY")))
    }
}

pub fn trial_balance(db: &Connection, year: Option<&str>) -> Result<Value> {
    if let Some(y) = year {
        valid_year(y)?;
    }
    let year_out = year.map(|y| y.to_string());
    let (from, to) = match year {
        Some(y) => {
            let (f, t) = fiscal_year_window(db, y);
            (Some(f), Some(t))
        }
        None => (None, None),
    };
    let mut stmt = db
        .prepare(
            "SELECT a.code, a.name, a.type,
              SUM(CASE WHEN p.amount_cents > 0 THEN p.amount_cents ELSE 0 END),
              SUM(CASE WHEN p.amount_cents < 0 THEN -p.amount_cents ELSE 0 END),
              SUM(p.amount_cents)
            FROM postings p
            JOIN journal_entries e ON e.id = p.entry_id
            JOIN accounts a ON a.id = p.account_id
            WHERE e.state = 'posted'
              AND (?1 IS NULL OR e.date >= ?1)
              AND (?2 IS NULL OR e.date <= ?2)
            GROUP BY a.id
            ORDER BY a.code",
        )
        .map_err(sql_err)?;
    let rows: Vec<Value> = stmt
        .query_map(rusqlite::params![from, to], |r| {
            let debit = r.get::<_, Option<i64>>(3)?.unwrap_or(0);
            let credit = r.get::<_, Option<i64>>(4)?.unwrap_or(0);
            let net = r.get::<_, Option<i64>>(5)?.unwrap_or(0);
            Ok(json!({
                "code": r.get::<_, String>(0)?,
                "name": r.get::<_, String>(1)?,
                "type": r.get::<_, String>(2)?,
                "debit_cents": debit,
                "credit_cents": credit,
                "net_cents": net,
                "debit": crate::money::format_amount(debit),
                "credit": crate::money::format_amount(credit),
                "net": crate::money::format_amount(net),
            }))
        })
        .map_err(sql_err)?
        .filter_map(|x| x.ok())
        .collect();
    let total_debit: i64 = rows.iter().map(|r| r["debit_cents"].as_i64().unwrap_or(0)).sum();
    let total_credit: i64 = rows.iter().map(|r| r["credit_cents"].as_i64().unwrap_or(0)).sum();
    Ok(json!({
        "year": year_out,
        "accounts": rows,
        "total_debit_cents": total_debit,
        "total_credit_cents": total_credit,
        "total_debit": crate::money::format_amount(total_debit),
        "total_credit": crate::money::format_amount(total_credit),
        "balanced": total_debit == total_credit,
    }))
}

const BALANCE_TYPES: [&str; 3] = ["asset", "liability", "equity"];
type SignFn = fn(&str) -> i64;

const ASSET_GROUPS: [&str; 5] = ["BMVA.02", "BFVA.03", "BVRD.30", "BVOR.11", "BLIM.10"];
const PASSIVA_GROUPS: [&str; 4] = ["BEIV.05", "BVRZ.07", "BLAS.08", "BSCH.12"];

struct NetRow {
    code: String,
    name: String,
    type_: String,
    taxonomy_code: Option<String>,
    net_cents: i64,
}

fn net_per_account(db: &Connection, as_of: &str, types: &[&str], exclude_closing: bool) -> Result<Vec<NetRow>> {
    // types are static literals from this module — inline them (safe)
    let list = types.iter().map(|t| format!("'{t}'")).collect::<Vec<_>>().join(",");
    let sql = format!(
        "SELECT a.code, a.name, a.type, a.taxonomy_code, COALESCE(SUM(p.amount_cents), 0)
         FROM accounts a
         LEFT JOIN (
           SELECT p.account_id, p.amount_cents
           FROM postings p
           JOIN journal_entries e ON e.id = p.entry_id
           WHERE e.state = 'posted' {exclude} AND e.date <= ?1
         ) p ON p.account_id = a.id
         WHERE a.type IN ({list})
         GROUP BY a.id
         ORDER BY a.code",
        exclude = if exclude_closing { "AND e.source != 'closing'" } else { "" },
    );
    let mut stmt = db.prepare(&sql).map_err(sql_err)?;
    let rows = stmt
        .query_map([as_of], |r| {
            Ok(NetRow {
                code: r.get(0)?,
                name: r.get(1)?,
                type_: r.get(2)?,
                taxonomy_code: r.get(3)?,
                net_cents: r.get(4)?,
            })
        })
        .map_err(sql_err)?
        .filter_map(|x| x.ok())
        .collect();
    Ok(rows)
}

fn sectionize(groups: &[&str], rows: &[NetRow], types: &[&str], sign: SignFn) -> Vec<Value> {
    // sign: asset shows net, liability/equity shows -net; pnl income shows -net
    let mut sections = Vec::new();
    for code in groups {
        let accounts: Vec<Value> = rows
            .iter()
            .filter(|r| types.contains(&r.type_.as_str()) && (r.taxonomy_code.clone().unwrap_or("overig".into()) == *code))
            .map(|r| {
                json!({
                    "code": r.code, "name": r.name, "type": r.type_,
                    "balance_cents": r.net_cents * sign(&r.type_),
                })
            })
            .filter(|a| a["balance_cents"].as_i64().unwrap_or(0) != 0)
            .collect();
        if accounts.is_empty() {
            continue;
        }
        let total: i64 = accounts.iter().map(|a| a["balance_cents"].as_i64().unwrap_or(0)).sum();
        sections.push(json!({
            "taxonomy_code": code,
            "label": rgs_label(code),
            "accounts": accounts,
            "total_cents": total,
        }));
    }
    // leftover: codes not in the known list
    let known: Vec<&str> = groups.to_vec();
    let leftover: Vec<Value> = rows
        .iter()
        .filter(|r| types.contains(&r.type_.as_str()) && !known.contains(&r.taxonomy_code.clone().unwrap_or("overig".into()).as_str()))
        .map(|r| {
            json!({
                "code": r.code, "name": r.name, "type": r.type_,
                "balance_cents": r.net_cents * sign(&r.type_),
            })
        })
        .filter(|a| a["balance_cents"].as_i64().unwrap_or(0) != 0)
        .collect();
    if !leftover.is_empty() {
        let total: i64 = leftover.iter().map(|a| a["balance_cents"].as_i64().unwrap_or(0)).sum();
        sections.push(json!({
            "taxonomy_code": null,
            "label": "Overig",
            "accounts": leftover,
            "total_cents": total,
        }));
    }
    sections
}

pub fn balans(db: &Connection, as_of: &str) -> Result<Value> {
    validate_labeled(as_of, "as-of")?;
    let rows = net_per_account(db, as_of, &BALANCE_TYPES, false)?;
    let result_net: i64 = db
        .query_row(
            "SELECT COALESCE(SUM(p.amount_cents), 0) FROM postings p
             JOIN journal_entries e ON e.id = p.entry_id
             JOIN accounts a ON a.id = p.account_id
             WHERE e.state = 'posted' AND e.date <= ?1 AND a.type IN ('income','expense')",
            [as_of],
            |r| r.get(0),
        )
        .map_err(sql_err)?;
    let result_cents = -result_net;

    fn one(_: &str) -> i64 { 1 }
fn neg_one(_: &str) -> i64 { -1 }

    let asset_sections = sectionize(&ASSET_GROUPS, &rows, &["asset"], one);
    let passiva_sections = sectionize(&PASSIVA_GROUPS, &rows, &["liability", "equity"], neg_one);
    let total_assets: i64 = asset_sections.iter().map(|s| s["total_cents"].as_i64().unwrap_or(0)).sum();
    let passiva_total: i64 = passiva_sections.iter().map(|s| s["total_cents"].as_i64().unwrap_or(0)).sum();
    let total_passiva: i64 = passiva_total + result_cents;

    Ok(json!({
        "as_of": as_of,
        "assets": { "total_cents": total_assets, "sections": asset_sections },
        "liabilities_and_equity": {
            "total_cents": total_passiva,
            "sections": passiva_sections,
            "result_cents": result_cents,
        },
        "balanced": total_assets == total_passiva,
    }))
}

const PNL_GROUPS: [&str; 7] = ["WOMZ.80", "WOVB.82", "WKPR.70", "WPER.40", "WAFS.41", "WBED.42", "WFBE.84"];

pub fn pnl(db: &Connection, from: &str, to: &str) -> Result<Value> {
    validate_labeled(from, "from")?;
    validate_labeled(to, "to")?;
    // date-windowed variant of net_per_account (source != 'closing')
    let mut stmt = db
        .prepare(
            "SELECT a.code, a.name, a.type, a.taxonomy_code, COALESCE(SUM(p.amount_cents), 0)
             FROM accounts a
             LEFT JOIN (
               SELECT p.account_id, p.amount_cents
               FROM postings p
               JOIN journal_entries e ON e.id = p.entry_id
               WHERE e.state = 'posted' AND e.source != 'closing' AND e.date >= ?1 AND e.date <= ?2
             ) p ON p.account_id = a.id
             WHERE a.type IN ('income','expense')
             GROUP BY a.id
             ORDER BY a.code",
        )
        .map_err(sql_err)?;
    let rows: Vec<NetRow> = stmt
        .query_map(rusqlite::params![from, to], |r| {
            Ok(NetRow {
                code: r.get(0)?,
                name: r.get(1)?,
                type_: r.get(2)?,
                taxonomy_code: r.get(3)?,
                net_cents: r.get(4)?,
            })
        })
        .map_err(sql_err)?
        .filter_map(|x| x.ok())
        .collect();

    // section sign: income -> -net (positive revenue), expense -> net
    let sections = sectionize_pnl(&PNL_GROUPS, &rows);
    let revenue: i64 = rows.iter().filter(|r| r.type_ == "income").map(|r| -r.net_cents).sum();
    let costs: i64 = rows.iter().filter(|r| r.type_ == "expense").map(|r| r.net_cents).sum();
    Ok(json!({
        "from": from, "to": to,
        "sections": sections,
        "revenue_cents": revenue,
        "costs_cents": costs,
        "result_cents": revenue - costs,
    }))
}

fn sectionize_pnl(groups: &[&str], rows: &[NetRow]) -> Vec<Value> {
    let sign = |t: &str| if t == "income" { -1 } else { 1 };
    let mut sections = Vec::new();
    for code in groups {
        let accounts: Vec<Value> = rows
            .iter()
            .filter(|r| (r.taxonomy_code.clone().unwrap_or("overig".into()) == *code))
            .map(|r| {
                json!({
                    "code": r.code, "name": r.name, "type": r.type_,
                    "amount_cents": r.net_cents * sign(&r.type_),
                })
            })
            .filter(|a| a["amount_cents"].as_i64().unwrap_or(0) != 0)
            .collect();
        if accounts.is_empty() {
            continue;
        }
        let total: i64 = accounts.iter().map(|a| a["amount_cents"].as_i64().unwrap_or(0)).sum();
        sections.push(json!({
            "taxonomy_code": code,
            "label": rgs_label(code),
            "accounts": accounts,
            "total_cents": total,
        }));
    }
    let known: Vec<&str> = groups.to_vec();
    let leftover: Vec<Value> = rows
        .iter()
        .filter(|r| !known.contains(&r.taxonomy_code.clone().unwrap_or("overig".into()).as_str()))
        .map(|r| {
            json!({
                "code": r.code, "name": r.name, "type": r.type_,
                "amount_cents": r.net_cents * sign(&r.type_),
            })
        })
        .filter(|a| a["amount_cents"].as_i64().unwrap_or(0) != 0)
        .collect();
    if !leftover.is_empty() {
        let total: i64 = leftover.iter().map(|a| a["amount_cents"].as_i64().unwrap_or(0)).sum();
        sections.push(json!({
            "taxonomy_code": null,
            "label": "Overig",
            "accounts": leftover,
            "total_cents": total,
        }));
    }
    sections
}

pub fn journal(db: &Connection, from: &str, to: &str, limit: Option<i64>) -> Result<Vec<Value>> {
    validate_labeled(from, "from")?;
    validate_labeled(to, "to")?;
    let mut stmt = db
        .prepare(
            "SELECT e.id, e.date, e.description, e.source, e.state, e.created_by,
              p.amount_cents, a.code, a.name, a.type
            FROM journal_entries e
            LEFT JOIN postings p ON p.entry_id = e.id
            LEFT JOIN accounts a ON a.id = p.account_id
            WHERE e.date >= ?1 AND e.date <= ?2
            ORDER BY e.date, e.id, p.id
            LIMIT ?3",
        )
        .map_err(sql_err)?;
    let rows = stmt
        .query_map(rusqlite::params![from, to, limit.unwrap_or(-1)], |r| {
            Ok(json!({
                "entry_id": r.get::<_, i64>(0)?,
                "date": r.get::<_, String>(1)?,
                "description": r.get::<_, String>(2)?,
                "source": r.get::<_, String>(3)?,
                "state": r.get::<_, String>(4)?,
                "created_by": r.get::<_, String>(5)?,
                "amount_cents": r.get::<_, Option<i64>>(6)?,
                "account_code": r.get::<_, Option<String>>(7)?,
                "account_name": r.get::<_, Option<String>>(8)?,
                "account_type": r.get::<_, Option<String>>(9)?,
            }))
        })
        .map_err(sql_err)?
        .filter_map(|x| x.ok())
        .collect();
    Ok(rows)
}

fn sql_err(e: rusqlite::Error) -> BukioError {
    BukioError::new("DB_ERROR", e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::{seed_default_chart, NewAccount, create_account};
    use crate::db::open_db;
    use crate::entries::{create_entry, post_entry, CreateEntry, PostingSpec};

    fn books() -> Connection {
        let db = open_db(":memory:").unwrap();
        db.execute("INSERT INTO company (name) VALUES ('T')", []).unwrap();
        seed_default_chart(&db).unwrap();
        db
    }

    fn add(db: &Connection, date: &str, desc: &str, postings: &[(&str, i64)], post: bool) -> i64 {
        let specs: Vec<PostingSpec> = postings
            .iter()
            .map(|(c, a)| PostingSpec { code: c.to_string(), amount_cents: *a, cost_center_code: None })
            .collect();
        let e = create_entry(db, CreateEntry {
            date, description: desc, postings: specs,
            source: "manual", source_ref: None, actor: "human:erik",
        })
        .unwrap();
        if post {
            post_entry(db, e.id, "human:erik").unwrap();
        }
        e.id
    }

    #[test]
    fn trial_balance_balances() {
        let db = books();
        create_account(&db, &NewAccount { code: "1500", name: "BTW", type_: "liability", normal_balance: "credit", taxonomy_code: None }).unwrap();
        add(&db, "2026-01-05", "start", &[("1100", 500000), ("3000", -500000)], true);
        add(&db, "2026-02-05", "verkoop", &[("1100", 12100), ("8000", -10000), ("1500", -2100)], true);
        let tb = trial_balance(&db, Some("2026")).unwrap();
        assert_eq!(tb["balanced"], true);
        assert_eq!(tb["total_debit_cents"], 512100);
        assert!(trial_balance(&db, Some("20x6")).is_err());
        // account 1500 (btw) may not exist in the base chart — use own VAT leg
        let tb2 = trial_balance(&db, None).unwrap();
        assert_eq!(tb2["balanced"], true);
    }

    #[test]
    fn balans_and_pnl_shape() {
        let db = books();
        create_account(&db, &NewAccount { code: "1510", name: "BTW te betalen", type_: "liability", normal_balance: "credit", taxonomy_code: None }).unwrap();
        add(&db, "2026-01-05", "start", &[("1100", 1000000), ("3000", -1000000)], true);
        add(&db, "2026-03-05", "omzet", &[("1100", 6050), ("8000", -5000), ("1510", -1050)], true);
        let b = balans(&db, "2026-12-31").unwrap();
        assert_eq!(b["balanced"], true);
        assert_eq!(b["as_of"], "2026-12-31");
        let p = pnl(&db, "2026-01-01", "2026-12-31").unwrap();
        assert_eq!(p["revenue_cents"], 5000);
        assert_eq!(p["result_cents"], 5000);
        // garbage dates rejected
        assert!(pnl(&db, "garbage", "2026-12-31").is_err());
        assert!(balans(&db, "garbage").is_err());
    }

    #[test]
    fn fiscal_year_windows() {
        let db = books(); // fiscal_year_end default 12-31
        assert_eq!(fiscal_year_window(&db, "2026"), ("2026-01-01".into(), "2026-12-31".into()));
        db.execute("UPDATE company SET fiscal_year_end = '06-30'", []).unwrap();
        assert_eq!(fiscal_year_window(&db, "2026"), ("2025-07-01".into(), "2026-06-30".into()));
    }

    #[test]
    fn journal_lists_postings() {
        let db = books();
        add(&db, "2026-01-05", "start", &[("1100", 100000), ("3000", -100000)], true);
        let rows = journal(&db, "2026-01-01", "2026-12-31", None).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["account_code"], "1100");
    }
}
