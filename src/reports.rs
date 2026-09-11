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
            if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 {
                29
            } else {
                28
            }
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
    LABELS
        .get_or_init(|| crate::accounts::get_profile("NL").unwrap()["reporting"]["labels"].clone())
}

fn valid_year(year: &str) -> Result<()> {
    if year.len() == 4 && year.bytes().all(|b| b.is_ascii_digit()) {
        Ok(())
    } else {
        Err(BukioError::new(
            "INVALID_YEAR",
            format!("year '{year}' must be YYYY"),
        ))
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
    let total_debit: i64 = rows
        .iter()
        .map(|r| r["debit_cents"].as_i64().unwrap_or(0))
        .sum();
    let total_credit: i64 = rows
        .iter()
        .map(|r| r["credit_cents"].as_i64().unwrap_or(0))
        .sum();
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

fn net_per_account(
    db: &Connection,
    as_of: &str,
    types: &[&str],
    exclude_closing: bool,
) -> Result<Vec<NetRow>> {
    // types are static literals from this module — inline them (safe)
    let list = types
        .iter()
        .map(|t| format!("'{t}'"))
        .collect::<Vec<_>>()
        .join(",");
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
        exclude = if exclude_closing {
            "AND e.source != 'closing'"
        } else {
            ""
        },
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
            .filter(|r| {
                types.contains(&r.type_.as_str())
                    && (r.taxonomy_code.clone().unwrap_or("overig".into()) == *code)
            })
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
        let total: i64 = accounts
            .iter()
            .map(|a| a["balance_cents"].as_i64().unwrap_or(0))
            .sum();
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
        .filter(|r| {
            types.contains(&r.type_.as_str())
                && !known.contains(&r.taxonomy_code.clone().unwrap_or("overig".into()).as_str())
        })
        .map(|r| {
            json!({
                "code": r.code, "name": r.name, "type": r.type_,
                "balance_cents": r.net_cents * sign(&r.type_),
            })
        })
        .filter(|a| a["balance_cents"].as_i64().unwrap_or(0) != 0)
        .collect();
    if !leftover.is_empty() {
        let total: i64 = leftover
            .iter()
            .map(|a| a["balance_cents"].as_i64().unwrap_or(0))
            .sum();
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

    fn one(_: &str) -> i64 {
        1
    }
    fn neg_one(_: &str) -> i64 {
        -1
    }

    let asset_sections = sectionize(&ASSET_GROUPS, &rows, &["asset"], one);
    let passiva_sections = sectionize(&PASSIVA_GROUPS, &rows, &["liability", "equity"], neg_one);
    let total_assets: i64 = asset_sections
        .iter()
        .map(|s| s["total_cents"].as_i64().unwrap_or(0))
        .sum();
    let passiva_total: i64 = passiva_sections
        .iter()
        .map(|s| s["total_cents"].as_i64().unwrap_or(0))
        .sum();
    let total_passiva: i64 = passiva_total + result_cents;

    Ok(json!({
        "as_of": as_of,
        "assets": { "total_cents": total_assets, "total": crate::money::format_amount(total_assets), "sections": asset_sections },
        "liabilities_and_equity": {
            "total_cents": total_passiva, "total": crate::money::format_amount(total_passiva),
            "sections": passiva_sections,
            "result_cents": result_cents, "result": crate::money::format_amount(result_cents),
        },
        "balanced": total_assets == total_passiva,
    }))
}

const PNL_GROUPS: [&str; 7] = [
    "WOMZ.80", "WOVB.82", "WKPR.70", "WPER.40", "WAFS.41", "WBED.42", "WFBE.84",
];

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
    let revenue: i64 = rows
        .iter()
        .filter(|r| r.type_ == "income")
        .map(|r| -r.net_cents)
        .sum();
    let costs: i64 = rows
        .iter()
        .filter(|r| r.type_ == "expense")
        .map(|r| r.net_cents)
        .sum();
    Ok(json!({
        "from": from, "to": to,
        "sections": sections,
        "revenue_cents": revenue, "revenue": crate::money::format_amount(revenue),
        "costs_cents": costs, "costs": crate::money::format_amount(costs),
        "result_cents": revenue - costs, "result": crate::money::format_amount(revenue - costs),
    }))
}

fn sectionize_pnl(groups: &[&str], rows: &[NetRow]) -> Vec<Value> {
    let sign = |t: &str| if t == "income" { -1 } else { 1 };
    let mut sections = Vec::new();
    for code in groups {
        let accounts: Vec<Value> = rows
            .iter()
            .filter(|r| r.taxonomy_code.clone().unwrap_or("overig".into()) == *code)
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
        let total: i64 = accounts
            .iter()
            .map(|a| a["amount_cents"].as_i64().unwrap_or(0))
            .sum();
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
        let total: i64 = leftover
            .iter()
            .map(|a| a["amount_cents"].as_i64().unwrap_or(0))
            .sum();
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

// ---------------------------------------------------------------------------
// Aging report (mirrors src/report/aging.js)
// ---------------------------------------------------------------------------

fn bucket_label(days: i64) -> &'static str {
    if days <= 0 {
        "current"
    } else if days <= 30 {
        "d30"
    } else if days <= 60 {
        "d60"
    } else if days <= 90 {
        "d90"
    } else {
        "d90plus"
    }
}

fn empty_aging_totals() -> Value {
    json!({
        "current": 0, "d30": 0, "d60": 0, "d90": 0, "d90plus": 0,
        "total_cents": 0,
    })
}

/// Batch-fetch invoice_lines for a set of invoice ids.
fn batch_lines(db: &Connection, ids: &[i64]) -> Result<std::collections::HashMap<i64, Vec<Value>>> {
    use std::collections::HashMap;
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    // Build IN-clause (safe: ids are i64, never user strings)
    let ph: Vec<String> = ids.iter().map(|i| i.to_string()).collect();
    let sql = format!(
        "SELECT invoice_id, description, quantity, unit_price_cents, vat_code, vat_rate_bp,
                amount_cents, vat_amount_cents, item_id, gl_account, discount_type, discount_value
         FROM invoice_lines WHERE invoice_id IN ({}) ORDER BY invoice_id, line_no",
        ph.join(",")
    );
    let mut stmt = db.prepare(&sql).map_err(sql_err)?;
    let rows = stmt
        .query_map([], |r| {
            Ok(json!({
                "invoice_id": r.get::<_, i64>(0)?,
                "description": r.get::<_, String>(1)?,
                "quantity": r.get::<_, i64>(2)?,
                "unit_price_cents": r.get::<_, i64>(3)?,
                "vat_code": r.get::<_, Option<String>>(4)?,
                "vat_rate_bp": r.get::<_, i64>(5)?,
                "amount_cents": r.get::<_, i64>(6)?,
                "vat_amount_cents": r.get::<_, i64>(7)?,
                "item_id": r.get::<_, Option<i64>>(8)?,
                "gl_account": r.get::<_, Option<String>>(9)?,
                "discount_type": r.get::<_, Option<String>>(10)?,
                "discount_value": r.get::<_, Option<i64>>(11)?,
            }))
        })
        .map_err(sql_err)?;
    let mut map: HashMap<i64, Vec<Value>> = HashMap::new();
    for row in rows.filter_map(|r| r.ok()) {
        let iid = row["invoice_id"].as_i64().unwrap_or(0);
        map.entry(iid).or_default().push(row);
    }
    Ok(map)
}

/// Batch-fetch SUM(amount_cents) per invoice from invoice_payments.
fn batch_payment_totals(
    db: &Connection,
    ids: &[i64],
) -> Result<std::collections::HashMap<i64, i64>> {
    use std::collections::HashMap;
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let ph: Vec<String> = ids.iter().map(|i| i.to_string()).collect();
    let sql = format!(
        "SELECT invoice_id, SUM(amount_cents) FROM invoice_payments WHERE invoice_id IN ({}) GROUP BY invoice_id",
        ph.join(",")
    );
    let mut stmt = db.prepare(&sql).map_err(sql_err)?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, Option<i64>>(1)?.unwrap_or(0),
            ))
        })
        .map_err(sql_err)?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

fn parse_date(s: &str) -> Result<chrono::NaiveDate> {
    chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .map_err(|_| BukioError::new("INVALID_DATE", format!("invalid date '{s}'")))
}

/// Debtors aging: outstanding sales invoices per contact, bucketed by days
/// past due. Credit notes offset oldest debt first (FIFO).
fn debtors_aging(db: &Connection, as_of: &str) -> Result<Value> {
    use std::collections::HashMap;

    let mut stmt_inv = db
        .prepare(
            "SELECT i.id, i.invoice_number, i.contact_id, i.date, i.due_date,
                    i.discount_type, i.discount_value, c.name
             FROM invoices i
             LEFT JOIN contacts c ON c.id = i.contact_id
             WHERE i.invoice_type = 'sales'
               AND i.status IN ('sent', 'overdue')
               AND i.date <= ?1
             ORDER BY i.date, i.id",
        )
        .map_err(sql_err)?;
    let invoices: Vec<Value> = stmt_inv
        .query_map([as_of], |r| {
            Ok(json!({
                "id": r.get::<_, i64>(0)?,
                "invoice_number": r.get::<_, Option<String>>(1)?,
                "contact_id": r.get::<_, i64>(2)?,
                "date": r.get::<_, String>(3)?,
                "due_date": r.get::<_, Option<String>>(4)?,
                "discount_type": r.get::<_, Option<String>>(5)?,
                "discount_value": r.get::<_, Option<i64>>(6)?,
                "contact_name": r.get::<_, Option<String>>(7)?,
            }))
        })
        .map_err(sql_err)?
        .filter_map(|r| r.ok())
        .collect();

    if invoices.is_empty() {
        return Ok(json!({ "contacts": [], "totals": empty_aging_totals() }));
    }

    let inv_ids: Vec<i64> = invoices.iter().map(|i| i["id"].as_i64().unwrap()).collect();
    let lines_map = batch_lines(db, &inv_ids)?;
    let pay_map = batch_payment_totals(db, &inv_ids)?;

    let as_of_date = parse_date(as_of)?;

    // Per-contact aging buckets
    struct AgingContact {
        contact_id: i64,
        name: String,
        buckets: [i64; 5], // current, d30, d60, d90, d90plus
        total_cents: i64,
        items: Vec<Value>,
    }
    impl AgingContact {
        fn new(contact_id: i64, name: String) -> Self {
            Self {
                contact_id,
                name,
                buckets: [0; 5],
                total_cents: 0,
                items: Vec::new(),
            }
        }
        fn add(&mut self, bucket_idx: usize, cents: i64) {
            self.buckets[bucket_idx] += cents;
            self.total_cents += cents;
        }
        fn to_json(&self) -> Value {
            json!({
                "contact_id": self.contact_id,
                "name": self.name,
                "buckets": {
                    "current": self.buckets[0], "d30": self.buckets[1],
                    "d60": self.buckets[2], "d90": self.buckets[3],
                    "d90plus": self.buckets[4],
                },
                "total_cents": self.total_cents,
                "items": self.items,
            })
        }
    }

    let bucket_idx = |b: &str| match b {
        "current" => 0,
        "d30" => 1,
        "d60" => 2,
        "d90" => 3,
        _ => 4,
    };

    let mut by_contact: HashMap<i64, AgingContact> = HashMap::new();

    for inv in &invoices {
        let iid = inv["id"].as_i64().unwrap();
        let lines = lines_map.get(&iid).cloned().unwrap_or_default();
        let t = crate::invoice::compute_invoice_totals(
            &lines,
            inv["discount_type"].as_str(),
            inv["discount_value"].as_i64(),
        );
        let gross = t["gross_cents"].as_i64().unwrap_or(0);
        let paid = pay_map.get(&iid).copied().unwrap_or(0);
        let outstanding = gross - paid;
        if outstanding <= 0 {
            continue;
        }

        let due_str = inv["due_date"]
            .as_str()
            .or(inv["date"].as_str())
            .unwrap_or("");
        let due_date = chrono::NaiveDate::parse_from_str(due_str, "%Y-%m-%d").unwrap_or(as_of_date);
        let days = (as_of_date - due_date).num_days().max(0);
        let bl = bucket_label(days);

        let cid = inv["contact_id"].as_i64().unwrap();
        let contact = by_contact.entry(cid).or_insert_with(|| {
            AgingContact::new(cid, inv["contact_name"].as_str().unwrap_or("").to_string())
        });
        contact.add(bucket_idx(bl), outstanding);
        contact.items.push(json!({
            "ref": inv["invoice_number"],
            "date": inv["date"],
            "due_date": due_str,
            "days_past_due": days,
            "outstanding_cents": outstanding,
        }));
    }

    // Credit notes FIFO: offset oldest debt first
    let mut stmt_cr = db
        .prepare(
            "SELECT id, contact_id, date, discount_type, discount_value
             FROM invoices
             WHERE invoice_type = 'credit'
               AND status NOT IN ('draft', 'void')
               AND date <= ?1
             ORDER BY date, id",
        )
        .map_err(sql_err)?;
    let credits: Vec<Value> = stmt_cr
        .query_map([as_of], |r| {
            Ok(json!({
                "id": r.get::<_, i64>(0)?,
                "contact_id": r.get::<_, i64>(1)?,
                "date": r.get::<_, String>(2)?,
                "discount_type": r.get::<_, Option<String>>(3)?,
                "discount_value": r.get::<_, Option<i64>>(4)?,
            }))
        })
        .map_err(sql_err)?
        .filter_map(|r| r.ok())
        .collect();

    if !credits.is_empty() {
        let cr_ids: Vec<i64> = credits.iter().map(|c| c["id"].as_i64().unwrap()).collect();
        let cr_lines = batch_lines(db, &cr_ids)?;

        for cr in &credits {
            let cid = cr["contact_id"].as_i64().unwrap();
            let contact = match by_contact.get_mut(&cid) {
                Some(c) if c.total_cents > 0 => c,
                _ => continue,
            };
            let cr_iid = cr["id"].as_i64().unwrap();
            let lines = cr_lines.get(&cr_iid).cloned().unwrap_or_default();
            let t = crate::invoice::compute_invoice_totals(
                &lines,
                cr["discount_type"].as_str(),
                cr["discount_value"].as_i64(),
            );
            let cr_gross = t["gross_cents"].as_i64().unwrap_or(0);
            if cr_gross <= 0 {
                continue;
            }

            // FIFO offset items: the items array IS oldest-first (ORDER BY date,
            // id), so iterating it in order nets the OLDEST debt first. The
            // .rev() here did the opposite — a credit note zeroed the NEWEST
            // invoice and left the oldest outstanding, which is the row a user
            // chases for payment.
            let mut remaining = cr_gross;
            for item in contact.items.iter_mut() {
                if remaining <= 0 {
                    break;
                }
                let take =
                    std::cmp::min(item["outstanding_cents"].as_i64().unwrap_or(0), remaining);
                *item.get_mut("outstanding_cents").unwrap() =
                    json!(item["outstanding_cents"].as_i64().unwrap_or(0) - take);
                remaining -= take;
            }

            // FIFO offset buckets (oldest first: d90plus → d90 → d60 → d30 → current)
            let mut remaining = cr_gross;
            for bi in [4, 3, 2, 1, 0] {
                if remaining <= 0 {
                    break;
                }
                let take = std::cmp::min(contact.buckets[bi], remaining);
                contact.buckets[bi] -= take;
                remaining -= take;
            }
            contact.total_cents = contact.buckets.iter().sum();
        }
    }

    // Sort contacts by total descending, build JSON
    let mut contacts_vec: Vec<&AgingContact> = by_contact.values().collect();
    contacts_vec.sort_by(|a, b| b.total_cents.cmp(&a.total_cents));

    let mut totals = empty_aging_totals();
    let mut total_sum: i64 = 0;
    let mut total_buckets = [0i64; 5];
    let result_contacts: Vec<Value> = contacts_vec
        .iter()
        .map(|c| {
            for bi in 0..5 {
                total_buckets[bi] += c.buckets[bi];
            }
            total_sum += c.total_cents;
            c.to_json()
        })
        .collect();
    *totals.get_mut("current").unwrap() = json!(total_buckets[0]);
    *totals.get_mut("d30").unwrap() = json!(total_buckets[1]);
    *totals.get_mut("d60").unwrap() = json!(total_buckets[2]);
    *totals.get_mut("d90").unwrap() = json!(total_buckets[3]);
    *totals.get_mut("d90plus").unwrap() = json!(total_buckets[4]);
    *totals.get_mut("total_cents").unwrap() = json!(total_sum);

    Ok(json!({ "contacts": result_contacts, "totals": totals }))
}

/// Creditors aging: unpaid payables per contact, bucketed by days past due.
fn creditors_aging(db: &Connection, as_of: &str) -> Result<Value> {
    use std::collections::HashMap;

    #[derive(Default)]
    struct CredContact {
        contact_id: i64,
        name: String,
        buckets: [i64; 5],
        in_batch_cents: i64,
        total_cents: i64,
        items: Vec<Value>,
    }

    let mut by_contact: HashMap<i64, CredContact> = HashMap::new();
    let as_of_date = parse_date(as_of)?;

    let mut stmt_pay = db
        .prepare(
            "SELECT p.id, p.contact_id, p.invoice_ref, p.date, p.due_date,
                    p.amount_cents, p.status, c.name
             FROM payables p
             LEFT JOIN contacts c ON c.id = p.contact_id
             WHERE p.status IN ('unpaid', 'in_batch')
               AND p.date <= ?1
             ORDER BY p.due_date, p.id",
        )
        .map_err(sql_err)?;
    let rows: Vec<Value> = stmt_pay
        .query_map([as_of], |r| {
            Ok(json!({
                "id": r.get::<_, i64>(0)?,
                "contact_id": r.get::<_, i64>(1)?,
                "invoice_ref": r.get::<_, String>(2)?,
                "date": r.get::<_, String>(3)?,
                "due_date": r.get::<_, Option<String>>(4)?,
                "amount_cents": r.get::<_, i64>(5)?,
                "status": r.get::<_, String>(6)?,
                "contact_name": r.get::<_, Option<String>>(7)?,
            }))
        })
        .map_err(sql_err)?
        .filter_map(|r| r.ok())
        .collect();

    for p in &rows {
        let cid = p["contact_id"].as_i64().unwrap();
        let entry = by_contact.entry(cid).or_insert_with(|| CredContact {
            contact_id: cid,
            name: p["contact_name"].as_str().unwrap_or("").to_string(),
            ..Default::default()
        });
        let due_str = p["due_date"].as_str().or(p["date"].as_str()).unwrap_or("");
        let due_date = chrono::NaiveDate::parse_from_str(due_str, "%Y-%m-%d").unwrap_or(as_of_date);
        let days = (as_of_date - due_date).num_days().max(0);
        let bl = bucket_label(days);
        let bi = match bl {
            "current" => 0,
            "d30" => 1,
            "d60" => 2,
            "d90" => 3,
            _ => 4,
        };
        let amt = p["amount_cents"].as_i64().unwrap_or(0);
        if p["status"].as_str() == Some("in_batch") {
            entry.in_batch_cents += amt;
        } else {
            entry.buckets[bi] += amt;
        }
        entry.total_cents += amt;
        entry.items.push(json!({
            "payable_id": p["id"],
            "ref": p["invoice_ref"],
            "date": p["date"],
            "due_date": due_str,
            "days_past_due": days,
            "amount_cents": amt,
            "status": p["status"],
        }));
    }

    let mut contacts_vec: Vec<&CredContact> = by_contact.values().collect();
    contacts_vec.sort_by(|a, b| b.total_cents.cmp(&a.total_cents));

    let mut totals = empty_aging_totals();
    let mut total_sum: i64 = 0;
    let mut total_buckets = [0i64; 5];
    let result_contacts: Vec<Value> = contacts_vec
        .iter()
        .map(|c| {
            for bi in 0..5 {
                total_buckets[bi] += c.buckets[bi];
            }
            total_sum += c.total_cents;
            json!({
                "contact_id": c.contact_id,
                "name": c.name,
                "buckets": {
                    "current": c.buckets[0], "d30": c.buckets[1],
                    "d60": c.buckets[2], "d90": c.buckets[3],
                    "d90plus": c.buckets[4],
                },
                "in_batch_cents": c.in_batch_cents,
                "total_cents": c.total_cents,
                "items": c.items,
            })
        })
        .collect();
    *totals.get_mut("current").unwrap() = json!(total_buckets[0]);
    *totals.get_mut("d30").unwrap() = json!(total_buckets[1]);
    *totals.get_mut("d60").unwrap() = json!(total_buckets[2]);
    *totals.get_mut("d90").unwrap() = json!(total_buckets[3]);
    *totals.get_mut("d90plus").unwrap() = json!(total_buckets[4]);
    *totals.get_mut("total_cents").unwrap() = json!(total_sum);

    Ok(json!({ "contacts": result_contacts, "totals": totals }))
}

pub fn aging(db: &Connection, as_of: &str, kind: &str) -> Result<Value> {
    validate_labeled(as_of, "as-of")?;
    if !matches!(kind, "debtors" | "creditors" | "both") {
        return Err(BukioError::new(
            "INVALID_KIND",
            format!("kind must be 'debtors', 'creditors' or 'both', got '{kind}'"),
        ));
    }
    let mut result = json!({ "as_of": as_of, "kind": kind });
    if kind == "debtors" || kind == "both" {
        result["debtors"] = debtors_aging(db, as_of)?;
    }
    if kind == "creditors" || kind == "both" {
        result["creditors"] = creditors_aging(db, as_of)?;
    }
    Ok(result)
}

// ---------------------------------------------------------------------------
// Sales report (mirrors src/report/sales.js)
// ---------------------------------------------------------------------------

/// Per-line discount in cents (matches JS lineDiscountCents).
fn line_discount_cents(line: &Value) -> i64 {
    let amt = line["amount_cents"].as_i64().unwrap_or(0);
    match line["discount_type"].as_str() {
        Some("pct") => {
            let dv = line["discount_value"].as_i64().unwrap_or(0);
            ((amt as f64 * dv as f64 / 10000.0).round()) as i64
        }
        Some("amount") => {
            let dv = line["discount_value"].as_i64().unwrap_or(0);
            std::cmp::min(dv, amt)
        }
        _ => 0,
    }
}

pub fn sales(db: &Connection, year: &str, by: &str) -> Result<Value> {
    valid_year(year)?;
    if !matches!(by, "contact" | "item") {
        return Err(BukioError::new(
            "INVALID_KIND",
            format!("by must be 'contact' or 'item', got '{by}'"),
        ));
    }

    let (from, to) = fiscal_year_window(db, year);

    // Posted sales invoices in the fiscal year
    let mut stmt = db
        .prepare(
            "SELECT i.id, i.contact_id, i.discount_type, i.discount_value, c.name
             FROM invoices i
             LEFT JOIN contacts c ON c.id = i.contact_id
             WHERE i.invoice_type = 'sales'
               AND i.status NOT IN ('draft', 'void')
               AND i.date >= ?1 AND i.date <= ?2
             ORDER BY i.id",
        )
        .map_err(sql_err)?;
    let invoices: Vec<Value> = stmt
        .query_map(rusqlite::params![from, to], |r| {
            Ok(json!({
                "id": r.get::<_, i64>(0)?,
                "contact_id": r.get::<_, i64>(1)?,
                "discount_type": r.get::<_, Option<String>>(2)?,
                "discount_value": r.get::<_, Option<i64>>(3)?,
                "contact_name": r.get::<_, Option<String>>(4)?,
            }))
        })
        .map_err(sql_err)?
        .filter_map(|r| r.ok())
        .collect();

    if invoices.is_empty() {
        return Ok(json!({
            "year": year, "by": by, "groups": [],
            "totals": if by == "contact" {
                json!({ "invoice_count": 0, "net_cents": 0, "vat_cents": 0, "gross_cents": 0 })
            } else {
                json!({ "line_count": 0, "net_cents": 0 })
            },
        }));
    }

    let inv_ids: Vec<i64> = invoices.iter().map(|i| i["id"].as_i64().unwrap()).collect();
    let lines_map = batch_lines(db, &inv_ids)?;

    if by == "contact" {
        use std::collections::HashMap;
        #[derive(Default)]
        struct ContactSales {
            contact_id: i64,
            name: String,
            invoice_count: i64,
            net_cents: i64,
            vat_cents: i64,
            gross_cents: i64,
        }
        let mut map: HashMap<i64, ContactSales> = HashMap::new();
        for inv in &invoices {
            let cid = inv["contact_id"].as_i64().unwrap();
            let entry = map.entry(cid).or_insert_with(|| ContactSales {
                contact_id: cid,
                name: inv["contact_name"].as_str().unwrap_or("").to_string(),
                ..Default::default()
            });
            entry.invoice_count += 1;
            let lines = lines_map
                .get(&inv["id"].as_i64().unwrap())
                .cloned()
                .unwrap_or_default();
            let t = crate::invoice::compute_invoice_totals(
                &lines,
                inv["discount_type"].as_str(),
                inv["discount_value"].as_i64(),
            );
            entry.net_cents += t["net_cents"].as_i64().unwrap_or(0);
            entry.vat_cents += t["vat_cents"].as_i64().unwrap_or(0);
            entry.gross_cents += t["gross_cents"].as_i64().unwrap_or(0);
        }
        let mut groups: Vec<Value> = map
            .values()
            .map(|g| {
                json!({
                    "contact_id": g.contact_id, "name": g.name,
                    "invoice_count": g.invoice_count,
                    "net_cents": g.net_cents, "vat_cents": g.vat_cents,
                    "gross_cents": g.gross_cents,
                })
            })
            .collect();
        groups.sort_by(|a, b| {
            b["gross_cents"]
                .as_i64()
                .unwrap_or(0)
                .cmp(&a["gross_cents"].as_i64().unwrap_or(0))
        });
        let totals = groups.iter().fold(
            json!({ "invoice_count": 0, "net_cents": 0, "vat_cents": 0, "gross_cents": 0 }),
            |mut t, g| {
                *t.get_mut("invoice_count").unwrap() = json!(
                    t["invoice_count"].as_i64().unwrap_or(0)
                        + g["invoice_count"].as_i64().unwrap_or(0)
                );
                *t.get_mut("net_cents").unwrap() = json!(
                    t["net_cents"].as_i64().unwrap_or(0) + g["net_cents"].as_i64().unwrap_or(0)
                );
                *t.get_mut("vat_cents").unwrap() = json!(
                    t["vat_cents"].as_i64().unwrap_or(0) + g["vat_cents"].as_i64().unwrap_or(0)
                );
                *t.get_mut("gross_cents").unwrap() = json!(
                    t["gross_cents"].as_i64().unwrap_or(0) + g["gross_cents"].as_i64().unwrap_or(0)
                );
                t
            },
        );
        return Ok(json!({ "year": year, "by": by, "groups": groups, "totals": totals }));
    }

    // by item: net after per-line discounts
    use std::collections::HashMap;
    #[derive(Default)]
    struct ItemSales {
        key: String,
        item_id: Option<i64>,
        name: String,
        line_count: i64,
        net_cents: i64,
    }
    let mut map: HashMap<String, ItemSales> = HashMap::new();

    // Cache item names
    let mut item_names: HashMap<i64, String> = HashMap::new();

    for inv in &invoices {
        let lines = lines_map
            .get(&inv["id"].as_i64().unwrap())
            .cloned()
            .unwrap_or_default();
        for l in &lines {
            let item_id = l["item_id"].as_i64();
            let key = if let Some(iid) = item_id {
                format!("item:{iid}")
            } else {
                format!("desc:{}", l["description"].as_str().unwrap_or(""))
            };
            let entry = map.entry(key.clone()).or_insert_with(|| {
                let name = if let Some(iid) = item_id {
                    item_names
                        .entry(iid)
                        .or_insert_with(|| {
                            crate::items::get_item(db, iid)
                                .ok()
                                .flatten()
                                .and_then(|v| v["name"].as_str().map(String::from))
                                .unwrap_or_else(|| {
                                    l["description"].as_str().unwrap_or("").to_string()
                                })
                        })
                        .clone()
                } else {
                    l["description"].as_str().unwrap_or("").to_string()
                };
                ItemSales {
                    key,
                    item_id,
                    name,
                    ..Default::default()
                }
            });
            entry.line_count += 1;
            entry.net_cents += l["amount_cents"].as_i64().unwrap_or(0) - line_discount_cents(l);
        }
    }
    let mut groups: Vec<Value> = map
        .values()
        .map(|g| {
            json!({
                "key": g.key, "item_id": g.item_id, "name": g.name,
                "line_count": g.line_count, "net_cents": g.net_cents,
            })
        })
        .collect();
    groups.sort_by(|a, b| {
        b["net_cents"]
            .as_i64()
            .unwrap_or(0)
            .cmp(&a["net_cents"].as_i64().unwrap_or(0))
    });
    let totals = groups
        .iter()
        .fold(json!({ "line_count": 0, "net_cents": 0 }), |mut t, g| {
            *t.get_mut("line_count").unwrap() = json!(
                t["line_count"].as_i64().unwrap_or(0) + g["line_count"].as_i64().unwrap_or(0)
            );
            *t.get_mut("net_cents").unwrap() =
                json!(t["net_cents"].as_i64().unwrap_or(0) + g["net_cents"].as_i64().unwrap_or(0));
            t
        });
    Ok(json!({ "year": year, "by": by, "groups": groups, "totals": totals }))
}

// ---------------------------------------------------------------------------
// Cost-center report (mirrors src/report/cost-center.js)
// ---------------------------------------------------------------------------

pub fn cost_center_report(
    db: &Connection,
    year: Option<&str>,
    from: Option<&str>,
    to: Option<&str>,
    cost_center: Option<&str>,
) -> Result<Value> {
    if let Some(y) = year {
        valid_year(y)?;
    }
    let (fy_from, fy_to) = match year {
        Some(y) => {
            let (f, t) = fiscal_year_window(db, y);
            (Some(f), Some(t))
        }
        None => (None, None),
    };
    let eff_from = from.or(fy_from.as_deref());
    let eff_to = to.or(fy_to.as_deref());

    // One query: posted income/expense postings with cost center
    let mut stmt = db
        .prepare(
            "SELECT p.account_id, p.amount_cents,
                    a.code AS account_code, a.name AS account_name, a.type AS account_type,
                    cc.code AS cost_center_code, cc.name AS cost_center_name
             FROM postings p
             JOIN journal_entries e ON e.id = p.entry_id
             JOIN accounts a ON a.id = p.account_id
             LEFT JOIN cost_centers cc ON cc.id = p.cost_center_id
             WHERE e.state = 'posted'
               AND e.source != 'closing'
               AND a.type IN ('income', 'expense')
               AND (?1 IS NULL OR e.date >= ?1)
               AND (?2 IS NULL OR e.date <= ?2)
               AND (?3 IS NULL OR cc.code = ?3)",
        )
        .map_err(sql_err)?;

    use std::collections::HashMap;
    struct CcBucket {
        code: Option<String>,
        name: Option<String>,
        accounts: HashMap<i64, Value>, // account_id -> {code, name, type, net_cents}
    }

    let mut by_cc: HashMap<String, CcBucket> = HashMap::new();
    let rows = stmt
        .query_map(rusqlite::params![eff_from, eff_to, cost_center], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, Option<String>>(5)?,
                r.get::<_, Option<String>>(6)?,
            ))
        })
        .map_err(sql_err)?;

    for row in rows.filter_map(|r| r.ok()) {
        let (acc_id, amt, acc_code, acc_name, acc_type, cc_code, cc_name) = row;
        let key = cc_code
            .clone()
            .unwrap_or_else(|| "__unassigned__".to_string());
        let bucket = by_cc.entry(key).or_insert_with(|| CcBucket {
            code: cc_code.clone(),
            name: cc_name.clone(),
            accounts: HashMap::new(),
        });
        let acc = bucket.accounts.entry(acc_id).or_insert_with(|| {
            json!({
                "code": acc_code, "name": acc_name, "type": acc_type, "net_cents": 0i64,
            })
        });
        *acc.get_mut("net_cents").unwrap() = json!(acc["net_cents"].as_i64().unwrap_or(0) + amt);
    }

    // Build result: per cost center with accounts, revenue, costs, result
    let mut centers: Vec<Value> = by_cc
        .values()
        .map(|bucket| {
            let mut accs: Vec<Value> = bucket
                .accounts
                .values()
                .map(|a| {
                    let net = a["net_cents"].as_i64().unwrap_or(0);
                    let amount = if a["type"].as_str() == Some("income") {
                        -net
                    } else {
                        net
                    };
                    json!({
                        "code": a["code"], "name": a["name"], "type": a["type"],
                        "net_cents": net,
                        "net": crate::money::format_amount(net),
                        "amount_cents": amount,
                    })
                })
                .filter(|a| a["amount_cents"].as_i64().unwrap_or(0) != 0)
                .collect();
            accs.sort_by(|a, b| a["code"].as_str().cmp(&b["code"].as_str()));

            let revenue: i64 = accs
                .iter()
                .filter(|a| a["type"].as_str() == Some("income"))
                .map(|a| a["amount_cents"].as_i64().unwrap_or(0))
                .sum();
            let costs: i64 = accs
                .iter()
                .filter(|a| a["type"].as_str() == Some("expense"))
                .map(|a| a["amount_cents"].as_i64().unwrap_or(0))
                .sum();

            json!({
                "cost_center_code": bucket.code,
                "cost_center_name": bucket.name
                    .clone()
                    .or_else(|| if bucket.code.is_none() { Some("Unassigned".into()) } else { None }),
                "accounts": accs,
                "revenue_cents": revenue,
                "costs_cents": costs,
                "result_cents": revenue - costs,
            })
        })
        .collect();

    // Sort: named centers first, unassigned last
    centers.sort_by(|a, b| {
        if a["cost_center_code"].is_null() {
            return std::cmp::Ordering::Greater;
        }
        if b["cost_center_code"].is_null() {
            return std::cmp::Ordering::Less;
        }
        a["cost_center_code"]
            .as_str()
            .cmp(&b["cost_center_code"].as_str())
    });

    Ok(json!({
        "year": year,
        "from": eff_from,
        "to": eff_to,
        "centers": centers,
    }))
}

fn sql_err(e: rusqlite::Error) -> BukioError {
    BukioError::new("DB_ERROR", e.to_string())
}

// ---------------------------------------------------------------------------
// Financial statements (jaarrekening) — statutory annual accounts
// ---------------------------------------------------------------------------

/// Group balans/P&L sections by statutory lines from the profile.
/// For "auto" format: match `taxonomy_code` to line's `rgs`.
/// Returns Vec<(label, taxonomy_code, total_cents, accounts_json)>.
fn group_by_statutory_lines(sections: &[Value], lines: &[Value]) -> Vec<Value> {
    let mut out = Vec::new();
    let mut known_codes: Vec<String> = Vec::new();
    for line in lines {
        let rgs = line["rgs"].as_str().unwrap_or("");
        let label = line["label"].as_str().unwrap_or(rgs).to_string();
        known_codes.push(rgs.to_string());
        let hits: Vec<&Value> = sections
            .iter()
            .filter(|s| s["taxonomy_code"].as_str().unwrap_or("") == rgs)
            .collect();
        if hits.is_empty() {
            continue;
        }
        let total_cents: i64 = hits
            .iter()
            .map(|s| s["total_cents"].as_i64().unwrap_or(0))
            .sum();
        let accounts: Vec<Value> = hits
            .iter()
            .flat_map(|s| {
                s["accounts"]
                    .as_array()
                    .map(|a| a.iter())
                    .unwrap_or_default()
            })
            .cloned()
            .collect();
        out.push(json!({
            "label": label,
            "taxonomy_code": rgs,
            "accounts": accounts,
            "total_cents": total_cents,
        }));
    }
    // leftover
    let leftover: Vec<&Value> = sections
        .iter()
        .filter(|s| !known_codes.contains(&s["taxonomy_code"].as_str().unwrap_or("").to_string()))
        .collect();
    if !leftover.is_empty() {
        let total_cents: i64 = leftover
            .iter()
            .map(|s| s["total_cents"].as_i64().unwrap_or(0))
            .sum();
        let accounts: Vec<Value> = leftover
            .iter()
            .flat_map(|s| {
                s["accounts"]
                    .as_array()
                    .map(|a| a.iter())
                    .unwrap_or_default()
            })
            .cloned()
            .collect();
        out.push(json!({
            "label": "Overig",
            "taxonomy_code": null,
            "accounts": accounts,
            "total_cents": total_cents,
        }));
    }
    out
}

/// Group balans/P&L sections by PCN prefix lines (LU format).
fn group_by_prefix_lines(sections: &[Value], lines: &[Value]) -> Vec<Value> {
    // flatten all accounts from sections
    let all_accounts: Vec<Value> = sections
        .iter()
        .flat_map(|s| {
            s["accounts"]
                .as_array()
                .map(|a| a.iter())
                .unwrap_or_default()
        })
        .cloned()
        .collect();
    let mut out = Vec::new();
    let mut known_prefixes: Vec<String> = Vec::new();
    // balans() sections carry balance_cents; pnl() sections carry amount_cents
    let amount_of = |a: &Value| -> i64 {
        a["balance_cents"]
            .as_i64()
            .or_else(|| a["amount_cents"].as_i64())
            .unwrap_or(0)
    };
    for line in lines {
        let prefixes: Vec<String> = line["prefixes"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let label = line["label"].as_str().unwrap_or("").to_string();
        for p in &prefixes {
            known_prefixes.push(p.clone());
        }
        let hits: Vec<&Value> = all_accounts
            .iter()
            .filter(|a| {
                let code = a["code"].as_str().unwrap_or("");
                prefixes.iter().any(|p| code.starts_with(p.as_str()))
            })
            .collect();
        if hits.is_empty() {
            continue;
        }
        let total_cents: i64 = hits.iter().map(|a| amount_of(a)).sum();
        let accounts: Vec<Value> = hits
            .iter()
            .map(|a| {
                json!({
                    "code": a["code"],
                    "name": a["name"],
                    "amount_cents": amount_of(a),
                })
            })
            .collect();
        out.push(json!({
            "label": label,
            "prefixes": prefixes,
            "accounts": accounts,
            "total_cents": total_cents,
        }));
    }
    // leftover
    let leftover: Vec<&Value> = all_accounts
        .iter()
        .filter(|a| {
            let code = a["code"].as_str().unwrap_or("");
            !known_prefixes.iter().any(|p| code.starts_with(p.as_str()))
        })
        .collect();
    if !leftover.is_empty() {
        let total_cents: i64 = leftover.iter().map(|a| amount_of(a)).sum();
        let accounts: Vec<Value> = leftover
            .iter()
            .map(|a| {
                json!({
                    "code": a["code"],
                    "name": a["name"],
                    "amount_cents": amount_of(a),
                })
            })
            .collect();
        out.push(json!({
            "label": "Autres",
            "prefixes": [],
            "accounts": accounts,
            "total_cents": total_cents,
        }));
    }
    out
}

/// Statutory annual accounts (jaarrekening) — mirrors src/report/jaarrekening.js.
/// JS parity: the statutory balans account rows expose `amount_cents` (the raw
/// balans report uses `balance_cents`, the pnl report `amount_cents`; the
/// jaarrekening's own rows are the ones consumers read). The port passed the
/// raw balans rows through, so its activa rows had no amount_cents at all.
fn with_amount_cents(rows: &[Value]) -> Vec<Value> {
    rows.iter()
        .cloned()
        .map(|mut g| {
            // both levels expose amount_cents: the group rollup and each
            // section's account rows (the renderer and the JS read the latter)
            let normalize = |accs: &mut Vec<Value>| {
                for a in accs.iter_mut() {
                    let amt = a["balance_cents"]
                        .as_i64()
                        .or_else(|| a["amount_cents"].as_i64())
                        .unwrap_or(0);
                    a["amount_cents"] = json!(amt);
                }
            };
            if let Some(accs) = g["accounts"].as_array_mut() {
                normalize(accs);
            }
            if let Some(secs) = g["sections"].as_array_mut() {
                for sec in secs.iter_mut() {
                    if let Some(accs) = sec["accounts"].as_array_mut() {
                        normalize(accs);
                    }
                }
            }
            // The statutory balans nests its account rows under `sections` (the
            // shape the renderer and the JS both read). Some builders emit the
            // accounts directly on the group, which left the printed asset side
            // EMPTY — wrap those into a single section.
            let has_sections = g["sections"]
                .as_array()
                .map(|a| !a.is_empty())
                .unwrap_or(false);
            let loose = g["accounts"].as_array().cloned().unwrap_or_default();
            if !has_sections && !loose.is_empty() {
                let total: i64 = loose
                    .iter()
                    .map(|a| a["amount_cents"].as_i64().unwrap_or(0))
                    .sum();
                g["sections"] = json!([{
                    "taxonomy_code": g["taxonomy_code"],
                    "label": g["label"],
                    "accounts": loose,
                    "total_cents": total,
                }]);
            }
            g
        })
        .collect()
}

pub fn jaarrekening(db: &Connection, year: &str, model: Option<&str>) -> Result<Value> {
    let profile = crate::accounts::resolve_profile(db)?;
    let reporting = &profile["reporting"];
    let format = reporting["format"].as_str().unwrap_or("");
    let sa = &reporting["statutoryAccounts"];
    let models: Vec<String> = sa["models"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let default_model = models.last().cloned().unwrap_or_default();
    let model = model.unwrap_or(&default_model);

    if models.is_empty() {
        // A market with no statutory-accounts builder: the JS reports the
        // missing builder, never an empty model list as INVALID_MODEL.
        return Err(BukioError::new(
            "FORMAT_NOT_SUPPORTED",
            format!(
                "financial statements format '{format}' has no builder (registered: auto, lu-lsc)"
            ),
        ));
    }
    if !models.contains(&model.to_string()) {
        return Err(BukioError::new(
            "INVALID_MODEL",
            format!("model must be one of {}", models.join(", ")),
        ));
    }
    if !year.bytes().all(|b| b.is_ascii_digit()) || year.len() != 4 {
        return Err(BukioError::new(
            "INVALID_YEAR",
            format!("year '{year}' must be YYYY"),
        ));
    }

    let company: Value = db
        .query_row("SELECT * FROM company WHERE id = 1", [], |r| {
            // Column order after migration 022: id(0) name(1) registration_id(2)
            // legal_form(3) tax_id(4) iban(5) vat_module(6) kor_flag(7)
            // fiscal_year_end(8) ... address(11) postal_code(12) city(13)
            Ok(json!({
                "name": r.get::<_, Option<String>>(1)?,
                "registration_id": r.get::<_, Option<String>>(2)?,
                "tax_id": r.get::<_, Option<String>>(4)?,
                "legal_form": r.get::<_, Option<String>>(3)?,
                "address": r.get::<_, Option<String>>(11)?,
                "postal_code": r.get::<_, Option<String>>(12)?,
                "city": r.get::<_, Option<String>>(13)?,
                "fiscal_year_end": r.get::<_, Option<String>>(8)?,
            }))
        })
        .map_err(|e| BukioError::new("NOT_INITIALISED", e.to_string()))?;

    let fye = company["fiscal_year_end"].as_str().unwrap_or("12-31");
    let fye_parts: Vec<&str> = fye.split('-').collect();
    let fye_month = fye_parts[fye_parts.len().saturating_sub(2)]
        .parse::<u32>()
        .unwrap_or(12);
    let fye_day = fye_parts
        .last()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(31);
    let as_of = format!("{year}-{fye_month:02}-{fye_day:02}");

    let b = balans(db, &as_of)?;
    let lines_activa = sa["lines"]["activa"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let lines_passiva = sa["lines"]["passiva"]
        .as_array()
        .cloned()
        .unwrap_or_default();

    let (activa, passiva) = match format {
        "auto" => {
            let activa_sections = b["assets"]["sections"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            let passiva_sections = b["liabilities_and_equity"]["sections"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            (
                group_by_statutory_lines(&activa_sections, &lines_activa),
                group_by_statutory_lines(&passiva_sections, &lines_passiva),
            )
        }
        "lu-lsc" => {
            let activa_sections = b["assets"]["sections"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            let passiva_sections = b["liabilities_and_equity"]["sections"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            (
                group_by_prefix_lines(&activa_sections, &lines_activa),
                group_by_prefix_lines(&passiva_sections, &lines_passiva),
            )
        }
        _ => {
            return Err(BukioError::new(
                "FORMAT_NOT_SUPPORTED",
                format!("financial statements format '{format}' has no builder"),
            ));
        }
    };

    // onverdeeld resultaat folds into equity
    let result_cents = b["liabilities_and_equity"]["result_cents"]
        .as_i64()
        .unwrap_or(0);
    let mut passiva = passiva;
    if result_cents != 0 {
        // try to find the equity line
        let ev_idx = passiva.iter().position(|s| {
            s["taxonomy_code"].as_str() == Some("BEIV.05")
                || s["label"].as_str() == Some("Capitaux propres")
        });
        if let Some(idx) = ev_idx {
            passiva[idx]["total_cents"] =
                json!(passiva[idx]["total_cents"].as_i64().unwrap_or(0) + result_cents);
            if let Some(arr) = passiva[idx]
                .get_mut("sections")
                .and_then(|v| v.as_array_mut())
            {
                arr.push(json!({
                    "taxonomy_code": null,
                    "label": "Onverdeeld resultaat",
                    "accounts": [{ "code": "—", "name": "Resultaat boekjaar", "amount_cents": result_cents }],
                    "total_cents": result_cents,
                }));
            }
        } else {
            passiva.push(json!({
                "label": if format == "lu-lsc" { "Capitaux propres" } else { "Eigen vermogen" },
                "taxonomy_code": "BEIV.05",
                "sections": [{
                    "taxonomy_code": null,
                    "label": "Onverdeeld resultaat",
                    "accounts": [{ "code": "—", "name": "Resultaat boekjaar", "amount_cents": result_cents }],
                    "total_cents": result_cents,
                }],
                "total_cents": result_cents,
            }));
        }
    }

    let total_activa: i64 = activa
        .iter()
        .map(|g| g["total_cents"].as_i64().unwrap_or(0))
        .sum();
    let total_passiva: i64 = passiva
        .iter()
        .map(|g| g["total_cents"].as_i64().unwrap_or(0))
        .sum();

    let mut report = json!({
        "year": year,
        "model": model,
        "company": {
            "name": company["name"],
            "kvk": company["registration_id"],
            "btw_id": company["tax_id"],
            "legal_form": company["legal_form"],
            "address": company["address"],
            "postal_code": company["postal_code"],
            "city": company["city"],
        },
        "as_of": as_of,
        "balans": {
            "activa": with_amount_cents(&activa),
            "passiva": with_amount_cents(&passiva),
            "total_activa_cents": total_activa,
            "total_passiva_cents": total_passiva,
            "balanced": total_activa == total_passiva,
        },
    });

    // JS parity: the W&V is emitted for klein (and always for the LU format);
    // a MICRO jaarrekening has NO P&L at all (r.pnl is undefined in the JS)
    let lines_pnl = sa["lines"]["pnl"].as_array().cloned().unwrap_or_default();
    if !lines_pnl.is_empty() && (model != "micro" || format == "lu-lsc") {
        let (pnl_from, pnl_to) = fiscal_year_window(db, year);
        let p = pnl(db, &pnl_from, &pnl_to)?;
        let pnl_sections = p["sections"].as_array().cloned().unwrap_or_default();

        let pnl_lines = match format {
            "lu-lsc" => {
                // LU P&L: group by prefix with sign
                let sign_map: std::collections::HashMap<String, i64> = lines_pnl
                    .iter()
                    .filter_map(|l| {
                        let label = l["label"].as_str()?.to_string();
                        let sign = l["sign"].as_i64().unwrap_or(1);
                        Some((label, sign))
                    })
                    .collect();
                let mut grouped = group_by_prefix_lines(&pnl_sections, &lines_pnl);
                // leftover 'Autres' carries net_cents (JS parity): income adds
                // amount_cents, expense subtracts — so MIXED leftovers
                // (expense + income) reconcile with the balans result
                if let Some(autres) = grouped
                    .iter_mut()
                    .find(|l| l["label"].as_str() == Some("Autres"))
                {
                    let mut known: Vec<String> = lines_pnl
                        .iter()
                        .flat_map(|l| {
                            l["prefixes"]
                                .as_array()
                                .map(|a| {
                                    a.iter()
                                        .filter_map(|v| v.as_str().map(String::from))
                                        .collect::<Vec<_>>()
                                })
                                .unwrap_or_default()
                        })
                        .collect();
                    // leftover accounts = pnl accounts not covered by any prefix
                    let mut net: i64 = 0;
                    for section in &pnl_sections {
                        if let Some(accounts) = section["accounts"].as_array() {
                            for a in accounts {
                                let code = a["code"].as_str().unwrap_or("");
                                let covered = known.iter().any(|p| code.starts_with(p.as_str()));
                                if covered {
                                    continue;
                                }
                                let amount = a["amount_cents"].as_i64().unwrap_or(0);
                                if a["type"].as_str() == Some("income") {
                                    net += amount;
                                } else {
                                    net -= amount;
                                }
                            }
                        }
                    }
                    autres["net_cents"] = json!(net);
                    let _ = &mut known;
                }
                let mut result = Vec::new();
                for mut line in grouped {
                    let label = line["label"].as_str().unwrap_or("").to_string();
                    let sign = sign_map.get(&label).copied().unwrap_or(1);
                    line["sign"] = json!(sign);
                    result.push(line);
                }
                result
            }
            _ => {
                // NL P&L: group by taxonomy_code
                group_by_statutory_lines(&pnl_sections, &lines_pnl)
            }
        };

        // compute resultaat
        let resultaat_cents: i64 = pnl_lines
            .iter()
            .map(|l| {
                let tc = l["taxonomy_code"].as_str();
                let total = l["total_cents"].as_i64().unwrap_or(0);
                if format == "lu-lsc" {
                    // a line carrying net_cents (the 'Autres' catch-all)
                    // contributes that directly; others: sign × total
                    if let Some(net) = l.get("net_cents").and_then(|v| v.as_i64()) {
                        net
                    } else {
                        let sign = l["sign"].as_i64().unwrap_or(1);
                        sign * total
                    }
                } else {
                    // NL: income (WOMZ.80, WOVB.82) positive, costs negative
                    match tc {
                        Some("WOMZ.80") | Some("WOVB.82") => total,
                        Some("WKPR.70") => -total,
                        _ => -total, // costs
                    }
                }
            })
            .sum();

        let mut pnl_out = serde_json::Map::new();
        pnl_out.insert("lines".to_string(), Value::Array(pnl_lines.clone()));
        // JS parity: LU emits resultat_cents, NL emits resultaat_cents
        let result_key = if format == "lu-lsc" {
            "resultat_cents"
        } else {
            "resultaat_cents"
        };
        if format != "lu-lsc" {
            // the statutory klein W&V also reports the aggregates the JS derives
            // from the same lines: omzet / overige opbrengsten / inkoop (counted
            // ONCE — not again inside kosten) / bruto marge / operating kosten
            let line_total = |code: &str| -> i64 {
                pnl_lines
                    .iter()
                    .find(|l| l["taxonomy_code"] == json!(code))
                    .and_then(|l| l["total_cents"].as_i64())
                    .unwrap_or(0)
            };
            let omzet = line_total("WOMZ.80");
            let overige = line_total("WOVB.82");
            let inkoop = line_total("WKPR.70");
            let kosten: i64 = pnl_lines
                .iter()
                .filter(|l| {
                    !["WOMZ.80", "WKPR.70", "WOVB.82"]
                        .contains(&l["taxonomy_code"].as_str().unwrap_or(""))
                })
                .map(|l| l["total_cents"].as_i64().unwrap_or(0))
                .sum();
            pnl_out.insert("omzet_cents".into(), json!(omzet));
            pnl_out.insert("overige_opbrengsten_cents".into(), json!(overige));
            pnl_out.insert("inkoop_cents".into(), json!(inkoop));
            pnl_out.insert("bruto_marge_cents".into(), json!(omzet - inkoop));
            pnl_out.insert("kosten_cents".into(), json!(kosten));
        }
        // The JS builds the pnl object aggregates-first, result-last; key order
        // is part of the byte-parity contract the CLI comparison checks.
        // the LU model is French throughout: src/report/jaarrekening.js emits
        // resultat_cents + resultat there, resultaat_cents + resultaat for NL
        let result_label = if format == "lu-lsc" {
            "resultat"
        } else {
            "resultaat"
        };
        pnl_out.insert(result_key.to_string(), json!(resultaat_cents));
        pnl_out.insert(
            result_label.to_string(),
            json!(crate::money::format_amount(resultaat_cents)),
        );
        report["pnl"] = Value::Object(pnl_out);
    }

    Ok(report)
}

// ---------------------------------------------------------------------------
// ICP readout — Intra-community supply listing
// ---------------------------------------------------------------------------

/// ICP readout: EU reverse-charge supplies per customer for a period.
pub fn icp_readout(db: &Connection, period: &str) -> Result<Value> {
    let (from, to) = crate::vat::parse_period(period)?;
    let label = period.to_string();

    let mut stmt = db
        .prepare(
            "SELECT DISTINCT i.id, i.invoice_type, i.invoice_number, i.date, i.contact_id,
                   c.name, c.vat_id, c.country
            FROM invoices i
            JOIN invoice_lines l ON l.invoice_id = i.id
            JOIN contacts c ON c.id = i.contact_id
            WHERE i.invoice_number IS NOT NULL
              AND i.status IN ('sent', 'paid', 'overdue')
              AND i.date >= ?1 AND i.date <= ?2
              AND l.vat_code = 'RE'
            ORDER BY c.name, i.id",
        )
        .map_err(sql_err)?;

    struct IcpRow {
        id: i64,
        invoice_type: String,
        invoice_number: String,
        contact_id: i64,
        name: String,
        vat_id: Option<String>,
        country: Option<String>,
    }

    let rows: Vec<IcpRow> = stmt
        .query_map(rusqlite::params![from, to], |r| {
            Ok(IcpRow {
                id: r.get(0)?,
                invoice_type: r.get(1)?,
                invoice_number: r.get(2)?,
                contact_id: r.get(4)?,
                name: r.get(5)?,
                vat_id: r.get(6)?,
                country: r.get(7)?,
            })
        })
        .map_err(sql_err)?
        .filter_map(|r| r.ok())
        .collect();

    use std::collections::HashMap;
    struct ContactAgg {
        name: String,
        vat_id: Option<String>,
        country: Option<String>,
        amount_cents: i64,
        invoice_numbers: Vec<String>,
    }
    let mut per_contact: HashMap<i64, ContactAgg> = HashMap::new();

    for row in &rows {
        if let Some(inv) = crate::invoice::get_invoice(db, row.id)? {
            if let Some(lines) = inv["lines"].as_array() {
                let totals = crate::invoice::compute_invoice_totals(
                    lines,
                    inv.get("discount_type").and_then(|v| v.as_str()),
                    inv.get("discount_value").and_then(|v| v.as_i64()),
                );
                if let Some(groups) = totals.get("groups").and_then(|v| v.as_array()) {
                    let re_net: i64 = groups
                        .iter()
                        .find(|g| g["code"].as_str() == Some("RE"))
                        .and_then(|g| g["discountedNet"].as_i64())
                        .unwrap_or(0);
                    if re_net == 0 {
                        continue;
                    }
                    let signed = if row.invoice_type == "credit" {
                        -re_net
                    } else {
                        re_net
                    };
                    let entry = per_contact
                        .entry(row.contact_id)
                        .or_insert_with(|| ContactAgg {
                            name: row.name.clone(),
                            vat_id: row.vat_id.clone(),
                            country: row.country.clone(),
                            amount_cents: 0,
                            invoice_numbers: Vec::new(),
                        });
                    entry.amount_cents += signed;
                    if !entry.invoice_numbers.contains(&row.invoice_number) {
                        entry.invoice_numbers.push(row.invoice_number.clone());
                    }
                }
            }
        }
    }

    let mut customers: Vec<Value> = per_contact
        .values()
        .map(|c| {
            json!({
                "contact_id": serde_json::Value::Null,
                "name": c.name,
                "vat_id": c.vat_id,
                "country": c.country,
                "amount_cents": c.amount_cents,
                "amount": crate::money::format_amount(c.amount_cents),
                "invoice_numbers": c.invoice_numbers,
            })
        })
        .collect();
    customers.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));

    let total_cents: i64 = customers
        .iter()
        .map(|c| c["amount_cents"].as_i64().unwrap_or(0))
        .sum();

    // check for missing VAT IDs
    let missing: Vec<&str> = customers
        .iter()
        .filter(|c| c["vat_id"].as_str().is_none() || c["vat_id"].as_str() == Some(""))
        .filter_map(|c| c["name"].as_str())
        .collect();
    if !missing.is_empty() {
        return Err(BukioError::new(
            "ICP_VAT_ID_MISSING",
            format!(
                "EU customers without a btw-id (required for the ICP listing): {} — add it with contact add / an update",
                missing.join(", ")
            ),
        ));
    }

    Ok(json!({
        "period": label,
        "from": from,
        "to": to,
        "customers": customers,
        "total_cents": total_cents,
        "total": crate::money::format_amount(total_cents),
        "note": "Manual filing aid only — bukio never submits the ICP listing.",
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::{create_account, seed_default_chart, NewAccount};
    use crate::db::open_db;
    use crate::entries::{create_entry, post_entry, reverse_entry, CreateEntry, PostingSpec};

    fn books() -> Connection {
        let db = open_db(":memory:").unwrap();
        db.execute("INSERT INTO company (name) VALUES ('T')", [])
            .unwrap();
        seed_default_chart(&db).unwrap();
        db
    }

    fn add(db: &Connection, date: &str, desc: &str, postings: &[(&str, i64)], post: bool) -> i64 {
        let specs: Vec<PostingSpec> = postings
            .iter()
            .map(|(c, a)| PostingSpec {
                code: c.to_string(),
                amount_cents: *a,
                cost_center_code: None,
                vat_code: None,
                vat_amount_cents: None,
                fx_currency: None,
                fx_amount_cents: None,
            })
            .collect();
        let e = create_entry(
            db,
            CreateEntry {
                date,
                description: desc,
                postings: specs,
                source: "manual",
                source_ref: None,
                actor: "human:erik",
            },
        )
        .unwrap();
        if post {
            post_entry(db, e.id, "human:erik").unwrap();
        }
        e.id
    }

    // ── ported from test/trial-balance.test.js ──

    #[test]
    fn tb_per_account_totals() {
        let db = books();
        add(
            &db,
            "2026-01-10",
            "Startkapitaal",
            &[("1100", 1000000), ("3000", -1000000)],
            true,
        );
        add(
            &db,
            "2026-01-15",
            "Kantoorartikelen",
            &[("4300", 50000), ("1100", -50000)],
            true,
        );

        let tb = trial_balance(&db, None).unwrap();
        assert_eq!(tb["balanced"], true);
        let net = |code: &str| -> i64 {
            tb["accounts"]
                .as_array()
                .unwrap()
                .iter()
                .find(|a| a["code"] == code)
                .unwrap_or_else(|| panic!("account {code} missing"))["net_cents"]
                .as_i64()
                .unwrap_or(0)
        };
        assert_eq!(net("1100"), 950000);
        assert_eq!(net("3000"), -1000000);
        assert_eq!(net("4300"), 50000);
        assert_eq!(tb["total_debit_cents"], 1050000);
        assert_eq!(tb["total_credit_cents"], 1050000);
    }

    #[test]
    fn tb_year_filter() {
        let db = books();
        add(
            &db,
            "2026-01-10",
            "y2026",
            &[("1100", 100), ("3000", -100)],
            true,
        );
        add(
            &db,
            "2025-12-31",
            "y2025",
            &[("1100", 500), ("3000", -500)],
            true,
        );

        assert_eq!(
            trial_balance(&db, Some("2026")).unwrap()["total_debit_cents"],
            100
        );
        assert_eq!(
            trial_balance(&db, Some("2025")).unwrap()["total_debit_cents"],
            500
        );
        assert_eq!(trial_balance(&db, None).unwrap()["total_debit_cents"], 600);
    }

    #[test]
    fn tb_excludes_drafts_and_nets_out_reversals() {
        let db = books();
        // never posted — must not appear
        add(
            &db,
            "2026-02-01",
            "draft only",
            &[("1100", 999), ("3000", -999)],
            false,
        );
        let posted = add(
            &db,
            "2026-02-02",
            "Omzet",
            &[("1100", 12100), ("8000", -12100)],
            true,
        );
        reverse_entry(&db, posted, "human:erik", Some("credit note")).unwrap();

        let tb = trial_balance(&db, None).unwrap();
        assert_eq!(tb["balanced"], true);
        assert_eq!(tb["total_debit_cents"], tb["total_credit_cents"]);
        assert!(tb["accounts"]
            .as_array()
            .unwrap()
            .iter()
            .all(|a| a["net_cents"] == 0));
    }

    #[test]
    fn trial_balance_balances() {
        let db = books();
        create_account(
            &db,
            &NewAccount {
                code: "1500",
                name: "BTW",
                type_: "liability",
                normal_balance: "credit",
                taxonomy_code: None,
            },
        )
        .unwrap();
        add(
            &db,
            "2026-01-05",
            "start",
            &[("1100", 500000), ("3000", -500000)],
            true,
        );
        add(
            &db,
            "2026-02-05",
            "verkoop",
            &[("1100", 12100), ("8000", -10000), ("1500", -2100)],
            true,
        );
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
        create_account(
            &db,
            &NewAccount {
                code: "1510",
                name: "BTW te betalen",
                type_: "liability",
                normal_balance: "credit",
                taxonomy_code: None,
            },
        )
        .unwrap();
        add(
            &db,
            "2026-01-05",
            "start",
            &[("1100", 1000000), ("3000", -1000000)],
            true,
        );
        add(
            &db,
            "2026-03-05",
            "omzet",
            &[("1100", 6050), ("8000", -5000), ("1510", -1050)],
            true,
        );
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
        assert_eq!(
            fiscal_year_window(&db, "2026"),
            ("2026-01-01".into(), "2026-12-31".into())
        );
        db.execute("UPDATE company SET fiscal_year_end = '06-30'", [])
            .unwrap();
        assert_eq!(
            fiscal_year_window(&db, "2026"),
            ("2025-07-01".into(), "2026-06-30".into())
        );
    }

    #[test]
    fn journal_lists_postings() {
        let db = books();
        add(
            &db,
            "2026-01-05",
            "start",
            &[("1100", 100000), ("3000", -100000)],
            true,
        );
        let rows = journal(&db, "2026-01-01", "2026-12-31", None).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["account_code"], "1100");
    }

    // ==== ported from test/cost-centers.test.js (report half) ================

    fn cc_spec(code: &str, cents: i64, cc: Option<&str>) -> PostingSpec {
        PostingSpec {
            code: code.to_string(),
            amount_cents: cents,
            cost_center_code: cc.map(String::from),
            vat_code: None,
            vat_amount_cents: None,
            fx_currency: None,
            fx_amount_cents: None,
        }
    }

    fn cc_entry(db: &Connection, date: &str, desc: &str, postings: Vec<PostingSpec>) {
        create_entry(
            db,
            CreateEntry {
                date,
                description: desc,
                postings,
                source: "manual",
                source_ref: None,
                actor: "human:erik",
            },
        )
        .unwrap();
    }

    fn post_everything(db: &Connection) {
        let entries = crate::entries::list_entries(db, None, None, None, 500).unwrap();
        for e in entries {
            post_entry(db, e["id"].as_i64().unwrap(), "human:erik").unwrap();
        }
    }

    #[test]
    fn cost_center_report_groups_postings_by_center() {
        let db = books();
        crate::accounts::create_cost_center(&db, "ADM", "Admin").unwrap();
        crate::accounts::create_cost_center(&db, "SALES", "Sales").unwrap();
        cc_entry(
            &db,
            "2026-08-04",
            "adm expense",
            vec![
                cc_spec("4700", 20000, Some("ADM")),
                cc_spec("1100", -20000, None),
            ],
        );
        cc_entry(
            &db,
            "2026-08-05",
            "sales revenue",
            vec![
                cc_spec("1100", 30000, None),
                cc_spec("8000", -30000, Some("SALES")),
            ],
        );
        cc_entry(
            &db,
            "2026-08-06",
            "unassigned expense",
            vec![cc_spec("4700", 5000, None), cc_spec("1100", -5000, None)],
        );
        post_everything(&db);

        let r = cost_center_report(&db, Some("2026"), None, None, None).unwrap();
        let centers = r["centers"].as_array().unwrap();
        assert_eq!(centers.len(), 3); // ADM, SALES, unassigned
        let adm = centers
            .iter()
            .find(|c| c["cost_center_code"].as_str() == Some("ADM"))
            .unwrap();
        assert!(adm["result_cents"].as_i64().unwrap() < 0); // expense only
        let sales = centers
            .iter()
            .find(|c| c["cost_center_code"].as_str() == Some("SALES"))
            .unwrap();
        assert!(sales["result_cents"].as_i64().unwrap() > 0); // revenue only
        assert!(centers.iter().any(|c| c["cost_center_code"].is_null()));
    }

    #[test]
    fn trial_balance_stays_balanced_after_cc_tagged_entries() {
        let db = books();
        crate::accounts::create_cost_center(&db, "ADM", "Admin").unwrap();
        cc_entry(
            &db,
            "2026-08-04",
            "CC entry",
            vec![
                cc_spec("8000", -50000, Some("ADM")),
                cc_spec("3000", 50000, None),
            ],
        );
        post_everything(&db);
        let tb = trial_balance(&db, Some("2026")).unwrap();
        assert_eq!(tb["balanced"].as_bool(), Some(true));
    }

    #[test]
    fn cost_center_report_filters_by_period() {
        let db = books();
        crate::accounts::create_cost_center(&db, "ADM", "Admin").unwrap();
        cc_entry(
            &db,
            "2026-01-15",
            "jan",
            vec![
                cc_spec("4700", 10000, Some("ADM")),
                cc_spec("1100", -10000, None),
            ],
        );
        cc_entry(
            &db,
            "2026-08-04",
            "aug",
            vec![
                cc_spec("4700", 20000, Some("ADM")),
                cc_spec("1100", -20000, None),
            ],
        );
        post_everything(&db);
        // filter to August only: the January expense must not appear
        let r =
            cost_center_report(&db, None, Some("2026-08-01"), Some("2026-08-31"), None).unwrap();
        let adm = r["centers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["cost_center_code"].as_str() == Some("ADM"))
            .unwrap();
        let total: i64 = adm["accounts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["amount_cents"].as_i64().unwrap_or(0))
            .sum();
        assert_eq!(total, 20000);
    }

    #[test]
    fn cost_center_report_filter_returns_only_that_center() {
        let db = books();
        crate::accounts::create_cost_center(&db, "ADM", "Admin").unwrap();
        crate::accounts::create_cost_center(&db, "SALES", "Sales").unwrap();
        cc_entry(
            &db,
            "2026-08-04",
            "adm",
            vec![
                cc_spec("4700", 10000, Some("ADM")),
                cc_spec("1100", -10000, None),
            ],
        );
        cc_entry(
            &db,
            "2026-08-05",
            "sales",
            vec![
                cc_spec("4700", 20000, Some("SALES")),
                cc_spec("1100", -20000, None),
            ],
        );
        post_everything(&db);
        let r = cost_center_report(&db, Some("2026"), None, None, Some("ADM")).unwrap();
        let centers = r["centers"].as_array().unwrap();
        assert_eq!(centers.len(), 1);
        assert_eq!(centers[0]["cost_center_code"].as_str(), Some("ADM"));
    }
}

// ==== ported from test/reports.test.js ======================================
#[cfg(test)]
mod reports_tests {
    use super::*;
    use crate::entries::{create_entry, post_entry, reverse_entry, CreateEntry, PostingSpec};
    use rusqlite::Connection;
    use serde_json::{json, Value};

    fn idb() -> Connection {
        let d = crate::db::open_db(":memory:").unwrap();
        crate::accounts::seed_default_chart(&d).unwrap();
        d
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
                fx_currency: None,
                fx_amount_cents: None,
            })
            .collect()
    }

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

    fn seed_scenario(db: &Connection) {
        post(
            db,
            "2026-01-05",
            "Startkapitaal",
            specs(&[("1100", 1000000), ("3000", -1000000)]),
        );
        post(
            db,
            "2026-02-10",
            "Omzet",
            specs(&[("1100", 121000), ("8000", -121000)]),
        );
        post(
            db,
            "2026-03-01",
            "Kantoorartikelen",
            specs(&[("4300", 25000), ("1100", -25000)]),
        );
    }

    /// find a section by taxonomy_code in a {sections: [...]} object
    fn section(container: &Value, val: &str) -> Value {
        container["sections"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["taxonomy_code"] == val)
            .cloned()
            .unwrap_or_else(|| panic!("no section {val}: {container:?}"))
    }

    #[test]
    fn balans_assets_equal_liabilities_plus_equity_plus_result() {
        let d = idb();
        seed_scenario(&d);
        let b = balans(&d, "2026-03-31").unwrap();
        assert_eq!(b["balanced"], json!(true));
        assert_eq!(b["assets"]["total_cents"].as_i64(), Some(1096000));
        assert_eq!(
            b["liabilities_and_equity"]["total_cents"].as_i64(),
            Some(1096000)
        );
        assert_eq!(
            b["liabilities_and_equity"]["result_cents"].as_i64(),
            Some(96000)
        );
        let blim = section(&b["assets"], "BLIM.10");
        assert_eq!(blim["label"].as_str(), Some("Liquide middelen"));
        assert_eq!(blim["total_cents"].as_i64(), Some(1096000));
        let ev = section(&b["liabilities_and_equity"], "BEIV.05");
        assert_eq!(ev["total_cents"].as_i64(), Some(1000000));
    }

    #[test]
    fn balans_result_is_zero_before_any_income_or_expense() {
        let d = idb();
        post(
            &d,
            "2026-01-05",
            "Startkapitaal",
            specs(&[("1100", 1000000), ("3000", -1000000)]),
        );
        let b = balans(&d, "2026-01-31").unwrap();
        assert_eq!(b["balanced"], json!(true));
        assert_eq!(b["assets"]["total_cents"].as_i64(), Some(1000000));
        assert_eq!(
            b["liabilities_and_equity"]["result_cents"].as_i64(),
            Some(0)
        );
    }

    #[test]
    fn balans_empty_books_balance_at_zero() {
        let d = idb();
        let b = balans(&d, "2026-01-31").unwrap();
        assert_eq!(b["balanced"], json!(true));
        assert_eq!(b["assets"]["total_cents"].as_i64(), Some(0));
        assert_eq!(b["liabilities_and_equity"]["total_cents"].as_i64(), Some(0));
    }

    #[test]
    fn balans_excludes_drafts_and_nets_out_reversals() {
        let d = idb();
        // never posted — must not appear
        create_entry(
            &d,
            CreateEntry {
                date: "2026-02-01",
                description: "draft",
                postings: specs(&[("1100", 999), ("3000", -999)]),
                source: "manual",
                source_ref: None,
                actor: "agent:test",
            },
        )
        .unwrap();
        let posted = post(
            &d,
            "2026-02-02",
            "Omzet",
            specs(&[("1100", 50000), ("8000", -50000)]),
        );
        reverse_entry(&d, posted, "agent:test", Some("credit note")).unwrap();

        let b = balans(&d, "2026-12-31").unwrap();
        assert_eq!(b["balanced"], json!(true));
        assert_eq!(b["assets"]["total_cents"].as_i64(), Some(0));
        assert_eq!(
            b["liabilities_and_equity"]["result_cents"].as_i64(),
            Some(0)
        );
    }

    #[test]
    fn pnl_revenue_costs_and_result() {
        let d = idb();
        seed_scenario(&d);
        let p = pnl(&d, "2026-01-01", "2026-12-31").unwrap();
        assert_eq!(p["revenue_cents"].as_i64(), Some(121000));
        assert_eq!(p["costs_cents"].as_i64(), Some(25000));
        assert_eq!(p["result_cents"].as_i64(), Some(96000));
        assert_eq!(section(&p, "WOMZ.80")["total_cents"].as_i64(), Some(121000));
        assert_eq!(section(&p, "WBED.42")["total_cents"].as_i64(), Some(25000));
    }

    #[test]
    fn pnl_empty_period_gives_zero_result_and_no_sections() {
        let d = idb();
        let p = pnl(&d, "2025-01-01", "2025-12-31").unwrap();
        assert_eq!(p["result_cents"].as_i64(), Some(0));
        assert_eq!(p["sections"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn pnl_legacy_chart_without_rgs_codes_splits_by_account_type() {
        let d = idb();
        for (code, name, type_, nb) in [
            ("8200", "Omzet diensten", "income", "credit"),
            ("6531", "Kosten IT", "expense", "debit"),
            ("6710", "Afschrijvingskosten", "expense", "debit"),
            ("9100", "Rentebaten", "expense", "debit"), // contra-expense
        ] {
            crate::accounts::create_account(
                &d,
                &crate::accounts::NewAccount {
                    code,
                    name,
                    type_,
                    normal_balance: nb,
                    taxonomy_code: None,
                },
            )
            .unwrap();
        }
        post(
            &d,
            "2026-03-01",
            "Factuur",
            specs(&[("1100", 29420), ("8200", -29420)]),
        );
        post(
            &d,
            "2026-03-02",
            "Hosting",
            specs(&[("6531", 24036), ("1100", -24036)]),
        );
        post(
            &d,
            "2026-03-03",
            "Afschrijving",
            specs(&[("6710", 29976), ("1100", -29976)]),
        );
        post(
            &d,
            "2026-03-04",
            "Rente",
            specs(&[("1100", 11716), ("9100", -11716)]),
        );

        let p = pnl(&d, "2026-01-01", "2026-12-31").unwrap();
        assert_eq!(p["revenue_cents"].as_i64(), Some(29420));
        assert_eq!(p["costs_cents"].as_i64(), Some(24036 + 29976 - 11716));
        assert_eq!(
            p["result_cents"].as_i64(),
            Some(29420 - (24036 + 29976 - 11716))
        );
    }

    #[test]
    fn pnl_catch_all_section_for_unknown_taxonomy() {
        let d = idb();
        crate::accounts::create_account(
            &d,
            &crate::accounts::NewAccount {
                code: "5000",
                name: "Testkosten",
                type_: "expense",
                normal_balance: "debit",
                taxonomy_code: None,
            },
        )
        .unwrap();
        post(
            &d,
            "2026-05-01",
            "Testkosten",
            specs(&[("5000", 1000), ("1100", -1000)]),
        );
        let p = pnl(&d, "2026-01-01", "2026-12-31").unwrap();
        let overig = p["sections"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["label"] == "Overig")
            .cloned()
            .unwrap_or_else(|| panic!("no Overig section: {p:?}"));
        assert_eq!(overig["total_cents"].as_i64(), Some(1000));
        assert_eq!(p["costs_cents"].as_i64(), Some(1000));
    }

    #[test]
    fn journal_one_row_per_posting_ordered_by_date() {
        let d = idb();
        seed_scenario(&d);
        let rows = journal(&d, "2026-01-01", "2026-12-31", None).unwrap();
        assert_eq!(rows.len(), 6);
        assert_eq!(rows[0]["entry_id"].as_i64(), Some(1));
        assert_eq!(rows[0]["account_code"].as_str(), Some("1100"));
        assert_eq!(rows[0]["amount_cents"].as_i64(), Some(1000000));
        assert_eq!(rows[0]["state"].as_str(), Some("posted"));
    }
}
