// bukio-cli — agent-first double-entry bookkeeping for SMEs.
// Copyright (c) 2026 Erik van Kempen.
// SPDX-License-Identifier: Apache-2.0
//
// Fixed assets: schemes, register, runs, disposal.

use crate::accounts::get_account_by_code;
use crate::audit::{record, RecordArgs};
use crate::entries::{create_entry, post_entry, CreateEntry, PostingSpec};
use crate::money::{format_amount, BukioError, Result};
use rusqlite::Connection;
use serde_json::{json, Value};

fn assets_error(code: &'static str, msg: impl Into<String>) -> BukioError {
    BukioError::new(code, msg.into())
}
fn valid_date(s: &str) -> bool {
    chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok()
}

fn month_diff(a: &str, b: &str) -> i32 {
    let (y1, m1) = (
        a[..4].parse::<i32>().unwrap(),
        a[5..7].parse::<i32>().unwrap(),
    );
    let (y2, m2) = (
        b[..4].parse::<i32>().unwrap(),
        b[5..7].parse::<i32>().unwrap(),
    );
    y2 * 12 + m2 - (y1 * 12 + m1)
}

fn add_months(date_str: &str, n: i32) -> String {
    let parts: Vec<i32> = date_str.split('-').map(|s| s.parse().unwrap()).collect();
    let total = parts[0] * 12 + (parts[1] - 1) + n;
    let yy = total.div_euclid(12);
    let mm = (total.rem_euclid(12) + 1) as u32;
    let last_day = chrono::NaiveDate::from_ymd_opt(yy, mm + 1, 1)
        .map(|d| {
            (d - chrono::Duration::days(1))
                .format("%d")
                .to_string()
                .parse::<u32>()
                .unwrap_or(28)
        })
        .unwrap_or(28);
    let dd = std::cmp::min(parts[2] as u32, last_day);
    format!("{yy}-{mm:02}-{dd:02}")
}

fn next_period(period: &str) -> String {
    let parts: Vec<i32> = period.split('-').map(|s| s.parse().unwrap()).collect();
    let total = parts[0] * 12 + parts[1];
    format!(
        "{:04}-{:02}",
        total.div_euclid(12),
        (total.rem_euclid(12) + 1)
    )
}

fn first_run_period(recognition: &str, start: &str) -> String {
    let eff = if recognition > start {
        recognition
    } else {
        start
    };
    let day: u32 = eff[8..10].parse().unwrap_or(1);
    let month = &eff[..7];
    if day == 1 {
        month.to_string()
    } else {
        next_period(month)
    }
}

pub fn list_schemes(db: &Connection) -> Result<Vec<Value>> {
    let mut stmt = db.prepare("SELECT id, name, method, life_months, residual_bp FROM depreciation_schemes ORDER BY id").map_err(sql_err)?;
    let rows = stmt.query_map([], |r| {
        Ok(json!({"id": r.get::<_, i64>(0)?, "name": r.get::<_, String>(1)?, "method": r.get::<_, String>(2)?, "life_months": r.get::<_, i64>(3)?, "residual_bp": r.get::<_, i64>(4)?}))
    }).map_err(sql_err)?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

pub fn get_scheme(db: &Connection, id: i64) -> Result<Option<Value>> {
    let r = db.query_row("SELECT id, name, method, life_months, residual_bp FROM depreciation_schemes WHERE id = ?1", [id], |r| {
        Ok(json!({"id": r.get::<_, i64>(0)?, "name": r.get::<_, String>(1)?, "method": r.get::<_, String>(2)?, "life_months": r.get::<_, i64>(3)?, "residual_bp": r.get::<_, i64>(4)?}))
    });
    match r {
        Ok(v) => Ok(Some(v)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(assets_error("DB_ERROR", e.to_string())),
    }
}

pub fn ensure_default_scheme(db: &Connection, actor: &str) -> Result<Value> {
    let existing = db.query_row(
        "SELECT id FROM depreciation_schemes WHERE name = 'Standaard 5 jaar lineair'",
        [],
        |r| r.get::<_, i64>(0),
    );
    if let Ok(id) = existing {
        return get_scheme(db, id)?.ok_or_else(|| assets_error("DB_ERROR", "scheme not found"));
    }
    create_scheme(
        db,
        "Standaard 5 jaar lineair",
        "lineair",
        60,
        0,
        actor,
        false,
    )
}

pub fn create_scheme(
    db: &Connection,
    name: &str,
    method: &str,
    life_months: i64,
    residual_bp: i64,
    actor: &str,
    dry_run: bool,
) -> Result<Value> {
    if name.trim().is_empty() {
        return Err(assets_error("INVALID_NAME", "scheme needs a name"));
    }
    if !["lineair", "degressief"].contains(&method) {
        return Err(assets_error(
            "INVALID_METHOD",
            format!("method must be lineair or degressief, got '{method}'"),
        ));
    }
    if life_months < 1 || life_months > 600 {
        return Err(assets_error("INVALID_LIFE", "life-months must be 1-600"));
    }
    if residual_bp < 0 || residual_bp > 10000 {
        return Err(assets_error(
            "INVALID_RESIDUAL",
            "residual-bp must be 0-10000",
        ));
    }
    if dry_run {
        return Ok(
            json!({"action": "assets.scheme.add", "name": name, "method": method, "life_months": life_months, "residual_bp": residual_bp, "dryRun": true}),
        );
    }
    db.execute("INSERT INTO depreciation_schemes (name, method, life_months, residual_bp, created_by) VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![name, method, life_months, residual_bp, actor]).map_err(sql_err)?;
    let id = db.last_insert_rowid();
    get_scheme(db, id)?.ok_or_else(|| assets_error("DB_ERROR", "scheme not found after insert"))
}

pub fn schedule_depreciation(
    cost_cents: i64,
    residual_cents: i64,
    life_months: i64,
    method: &str,
    first_period: &str,
    as_of: &str,
) -> Vec<(String, i64)> {
    let depreciable = cost_cents - residual_cents;
    if depreciable <= 0 || first_period.is_empty() || as_of.is_empty() || as_of < first_period {
        return vec![];
    }
    let rate = if method == "degressief" {
        2.0 / life_months as f64
    } else {
        0.0
    };
    let mut out = Vec::new();
    let mut remaining = depreciable;
    let mut months_left = life_months;
    let mut p = first_period.to_string();
    let mut guard = 0;
    while p.as_str() <= as_of && remaining > 0 && guard < 2000 {
        guard += 1;
        let linear = ((remaining as f64) / (months_left as f64)).round() as i64;
        let deg = if method == "degressief" {
            ((remaining as f64) * rate).round() as i64
        } else {
            0
        };
        let mut amt = if method == "degressief" {
            std::cmp::max(linear, deg)
        } else {
            linear
        };
        if amt <= 0 {
            amt = remaining;
        }
        amt = std::cmp::min(amt, remaining);
        out.push((p.clone(), amt));
        remaining -= amt;
        months_left -= 1;
        if months_left <= 0 {
            months_left = 1;
        }
        p = next_period(&p);
    }
    out
}

pub fn module_dep_to_period(db: &Connection, asset_id: i64, period: &str) -> Result<i64> {
    db.query_row("SELECT COALESCE(SUM(amount_cents),0) FROM asset_depreciation_runs WHERE asset_id = ?1 AND period <= ?2",
        rusqlite::params![asset_id, period], |r| r.get(0)).map_err(sql_err)
}

fn serialize_asset(db: &Connection, row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    let id: i64 = row.get(0)?;
    let name: String = row.get(1)?;
    let status: String = row.get(4)?;
    let scheme_id: i64 = row.get(5)?;
    let purchase_date: String = row.get(6)?;
    let purchase_price_cents: i64 = row.get(7)?;
    let residual_cents: i64 = row.get(8)?;
    let dep_start: String = row.get(9)?;
    let recog: String = row.get(10)?;
    let cum_dep_at_rec: i64 = row.get(11)?;
    let asset_acct_id: i64 = row.get(12)?;
    let cum_dep_acct_id: Option<i64> = row.get(13)?;
    let expense_acct_id: i64 = row.get(14)?;
    let asset_acct = db
        .query_row(
            "SELECT code FROM accounts WHERE id = ?1",
            [asset_acct_id],
            |r| r.get::<_, String>(0),
        )
        .ok();
    let cum_dep_acct = cum_dep_acct_id.and_then(|id| {
        db.query_row("SELECT code FROM accounts WHERE id = ?1", [id], |r| {
            r.get::<_, String>(0)
        })
        .ok()
    });
    let expense_acct = db
        .query_row(
            "SELECT code FROM accounts WHERE id = ?1",
            [expense_acct_id],
            |r| r.get::<_, String>(0),
        )
        .ok();
    let scheme = get_scheme(db, scheme_id).ok().flatten();
    Ok(
        json!({"id": id, "name": name, "scheme": scheme, "purchase_date": purchase_date, "purchase_price_cents": purchase_price_cents, "residual_cents": residual_cents, "depreciation_start_date": dep_start, "recognition_date": recog, "cum_dep_at_recognition_cents": cum_dep_at_rec, "asset_account_code": asset_acct.as_deref(), "cum_dep_account_code": cum_dep_acct.as_deref(), "expense_account_code": expense_acct.as_deref(), "status": status}),
    )
}

pub fn get_asset(db: &Connection, id: i64) -> Result<Option<Value>> {
    let r = db.query_row("SELECT * FROM assets WHERE id = ?1", [id], |r| {
        serialize_asset(db, r)
    });
    match r {
        Ok(v) => Ok(Some(v)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(assets_error("DB_ERROR", e.to_string())),
    }
}

pub fn list_assets(db: &Connection, status: Option<&str>) -> Result<Vec<Value>> {
    if let Some(s) = status {
        let mut stmt = db
            .prepare("SELECT * FROM assets WHERE status = ?1 ORDER BY id")
            .map_err(sql_err)?;
        let rows = stmt
            .query_map([s], |r| serialize_asset(db, r))
            .map_err(sql_err)?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    } else {
        let mut stmt = db
            .prepare("SELECT * FROM assets ORDER BY id")
            .map_err(sql_err)?;
        let rows = stmt
            .query_map([], |r| serialize_asset(db, r))
            .map_err(sql_err)?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }
}

pub fn run_due(db: &Connection, period: &str, actor: &str, dry_run: bool) -> Result<Value> {
    let assets = list_assets(db, Some("active"))?;
    let target = if period.is_empty() {
        chrono::Utc::now().format("%Y-%m").to_string()
    } else {
        period.to_string()
    };
    let mut plan = Vec::new();
    let mut to_book = Vec::new();
    for a in &assets {
        let aid = a["id"].as_i64().unwrap();
        let scheme = a["scheme"].as_object().unwrap();
        let method = scheme["method"].as_str().unwrap_or("lineair");
        let life_months = scheme["life_months"].as_i64().unwrap_or(60);
        let pp = a["purchase_price_cents"].as_i64().unwrap_or(0);
        let res = a["residual_cents"].as_i64().unwrap_or(0);
        let cdr = a["cum_dep_at_recognition_cents"].as_i64().unwrap_or(0);
        let ds = a["depreciation_start_date"].as_str().unwrap_or("");
        let rec = a["recognition_date"].as_str().unwrap_or("");
        let exp = a["expense_account_code"].as_str().unwrap_or("");
        let cum = a["cum_dep_account_code"]
            .as_str()
            .or(a["asset_account_code"].as_str())
            .unwrap_or("");
        let elapsed = std::cmp::max(0, month_diff(ds, rec));
        let ml = std::cmp::max(1, life_months - elapsed as i64);
        let rem = pp - res - cdr;
        if rem <= 0 {
            continue;
        }
        let first = first_run_period(rec, ds);
        let sched = schedule_depreciation(rem, 0, ml, method, &first, &target);
        let booked: std::collections::HashSet<String> = db
            .prepare("SELECT period FROM asset_depreciation_runs WHERE asset_id = ?1")
            .map_err(sql_err)?
            .query_map([aid], |r| r.get::<_, String>(0))
            .map_err(sql_err)?
            .filter_map(|r| r.ok())
            .collect();
        let due: Vec<_> = sched
            .into_iter()
            .filter(|(p, _)| !booked.contains(p))
            .collect();
        if !due.is_empty() {
            let total: i64 = due.iter().map(|(_, a)| a).sum();
            plan.push(json!({"asset_id": aid, "name": a["name"], "total_cents": total}));
            to_book.push((a.clone(), due, exp.to_string(), cum.to_string()));
        }
    }
    if to_book.is_empty() {
        return Ok(json!({"booked": [], "plan": plan, "dryRun": dry_run}));
    }
    if dry_run {
        return Ok(json!({"booked": [], "plan": plan, "dryRun": true}));
    }
    let mut booked = Vec::new();
    for (a, due, exp, cum) in &to_book {
        let aid = a["id"].as_i64().unwrap();
        for (period, amt) in due {
            let desc = format!("Afschrijving {} {period}", a["name"].as_str().unwrap_or(""));
            let sr = format!("asset:{aid}:{period}");
            let entry = create_entry(
                db,
                CreateEntry {
                    date: &format!("{period}-01"),
                    description: &desc,
                    postings: vec![
                        PostingSpec {
                            code: exp.clone(),
                            amount_cents: *amt,
                            cost_center_code: None,
                        },
                        PostingSpec {
                            code: cum.clone(),
                            amount_cents: -amt,
                            cost_center_code: None,
                        },
                    ],
                    source: "assets",
                    source_ref: Some(&sr),
                    actor,
                },
            )?;
            let posted = post_entry(db, entry.id, actor)?;
            db.execute("INSERT INTO asset_depreciation_runs (asset_id, period, entry_id, amount_cents, created_by) VALUES (?1, ?2, ?3, ?4, ?5)", rusqlite::params![aid, period, posted.id, amt, actor]).map_err(sql_err)?;
            booked.push(json!({"asset_id": aid, "period": period, "entry_id": posted.id, "amount_cents": amt}));
        }
    }
    record(
        db,
        RecordArgs {
            actor,
            action: "assets.run",
            command: Some("assets run"),
            args: Some(json!({"period": target})),
            outcome: "ok",
            entry_ids: booked
                .iter()
                .filter_map(|b| b["entry_id"].as_i64())
                .collect(),
        },
    )?;
    Ok(json!({"booked": booked, "plan": [], "dryRun": false}))
}

fn sql_err(e: rusqlite::Error) -> BukioError {
    BukioError::new("DB_ERROR", e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_db;

    fn test_db() -> Connection {
        let d = open_db(":memory:").unwrap();
        d.execute("INSERT INTO company (name) VALUES ('AssetCo')", [])
            .unwrap();
        crate::accounts::seed_default_chart(&d).unwrap();
        d
    }

    #[test]
    fn add_months_clamps_day() {
        assert_eq!(add_months("2026-01-31", 1), "2026-02-28");
        assert_eq!(add_months("2026-03-15", 1), "2026-04-15");
    }
    #[test]
    fn next_period_wraps_year() {
        assert_eq!(next_period("2026-12"), "2027-01");
    }
    #[test]
    fn first_run_period_logic() {
        assert_eq!(first_run_period("2026-01-01", "2026-01-01"), "2026-01");
        assert_eq!(first_run_period("2026-01-15", "2026-01-01"), "2026-02");
    }
    #[test]
    fn schedule_linear_basic() {
        let s = schedule_depreciation(120000, 0, 24, "lineair", "2026-01", "2026-06");
        assert_eq!(s.len(), 6);
        let total: i64 = s.iter().map(|(_, a)| a).sum();
        assert_eq!(total, 30000);
    }
    #[test]
    fn create_and_list_schemes() {
        let d = test_db();
        let s = create_scheme(&d, "Test 3yr", "lineair", 36, 0, "human:erik", false).unwrap();
        assert_eq!(s["name"], "Test 3yr");
    }
}
