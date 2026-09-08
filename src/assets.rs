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

// ── create asset ─────────────────────────────────────────────────────────

pub fn create_asset(
    db: &Connection,
    name: &str,
    category: Option<&str>,
    serial: Option<&str>,
    scheme_id: Option<i64>,
    method: Option<&str>,
    life_months: Option<i64>,
    residual_bp: Option<i64>,
    residual_cents_override: Option<i64>,
    purchase_date: &str,
    purchase_price_cents: i64,
    depreciation_start_date: &str,
    recognition_date: &str,
    cum_dep_at_recognition_cents: i64,
    asset_account_code: &str,
    cum_dep_account_code: Option<&str>,
    expense_account_code: &str,
    entry_id: Option<i64>,
    note: Option<&str>,
    actor: &str,
    dry_run: bool,
) -> Result<Value> {
    use crate::accounts::get_account_by_code;

    let clean_name = name.trim();
    if clean_name.is_empty() {
        return Err(assets_error("INVALID_NAME", "asset needs a name"));
    }
    if !valid_date(purchase_date) {
        return Err(assets_error("INVALID_DATE", format!("purchase-date '{purchase_date}' must be yyyy-mm-dd")));
    }
    if !valid_date(depreciation_start_date) {
        return Err(assets_error("INVALID_DATE", format!("depreciation-start-date '{depreciation_start_date}' must be yyyy-mm-dd")));
    }
    if !valid_date(recognition_date) {
        return Err(assets_error("INVALID_DATE", format!("recognition-date '{recognition_date}' must be yyyy-mm-dd")));
    }
    if depreciation_start_date < purchase_date {
        return Err(assets_error("INVALID_DATE", "depreciation-start-date cannot be before purchase-date"));
    }
    if recognition_date < purchase_date {
        return Err(assets_error("INVALID_DATE", "recognition-date cannot be before purchase-date"));
    }
    if purchase_price_cents <= 0 {
        return Err(assets_error("INVALID_COST", "purchase-price must be a positive amount in cents"));
    }
    if cum_dep_at_recognition_cents < 0 {
        return Err(assets_error("INVALID_DEPRECIATION", "cumulative depreciation at recognition must be >= 0"));
    }

    // Resolve scheme
    let scheme = if let Some(sid) = scheme_id {
        get_scheme(db, sid)?.ok_or_else(|| assets_error("SCHEME_NOT_FOUND", format!("scheme {sid} does not exist")))?
    } else {
        ensure_default_scheme(db, actor)?
    };
    let scheme_residual_cents = ((scheme["residual_bp"].as_i64().unwrap_or(0) as f64) / 10000.0 * purchase_price_cents as f64).round() as i64;
    let residual = residual_cents_override.unwrap_or(scheme_residual_cents);
    if residual < 0 || residual >= purchase_price_cents {
        return Err(assets_error("INVALID_RESIDUAL", format!("residual must be >= 0 and < purchase-price (got {residual})")));
    }

    // Resolve accounts
    let asset_acct = get_account_by_code(db, asset_account_code)
        .ok_or_else(|| assets_error("ACCOUNT_NOT_FOUND", "asset account is required"))?;
    let expense_acct = get_account_by_code(db, expense_account_code)
        .ok_or_else(|| assets_error("ACCOUNT_NOT_FOUND", "expense account is required"))?;
    let cum_dep_acct = cum_dep_account_code.and_then(|c| get_account_by_code(db, c));

    if let Some(eid) = entry_id {
        let exists: bool = db.query_row("SELECT COUNT(*) FROM journal_entries WHERE id = ?1", [eid], |r| r.get::<_, i64>(0))
            .map_err(sql_err)? > 0;
        if !exists {
            return Err(assets_error("ENTRY_NOT_FOUND", format!("entry {eid} does not exist")));
        }
    }

    let depreciable = purchase_price_cents - residual;
    if cum_dep_at_recognition_cents > depreciable {
        return Err(assets_error("INVALID_DEPRECIATION", format!(
            "cumulative depreciation at recognition ({cum_dep_at_recognition_cents}) exceeds cost minus residual ({depreciable})"
        )));
    }

    let elapsed = std::cmp::max(0, month_diff(depreciation_start_date, recognition_date));
    let life = scheme["life_months"].as_i64().unwrap_or(60);
    let months_left = std::cmp::max(0, life - elapsed as i64);
    let first_period = first_run_period(recognition_date, depreciation_start_date);
    let fully_depreciated = months_left <= 0 || cum_dep_at_recognition_cents >= depreciable;

    if dry_run {
        return Ok(json!({
            "asset": {
                "name": clean_name, "category": category, "serial": serial,
                "purchase_date": purchase_date, "purchase_price_cents": purchase_price_cents,
                "residual_cents": residual,
                "depreciation_start_date": depreciation_start_date,
                "recognition_date": recognition_date,
                "cum_dep_at_recognition_cents": cum_dep_at_recognition_cents,
                "scheme": scheme["name"], "method": scheme["method"], "life_months": life,
                "asset_account": asset_account_code,
                "cum_dep_account": cum_dep_account_code,
                "expense_account": expense_account_code,
                "months_left": months_left, "first_run_period": first_period,
                "status": if fully_depreciated { "fully_depreciated" } else { "active" },
            },
            "dryRun": true,
        }));
    }

    let status = if fully_depreciated {
        "fully_depreciated"
    } else {
        "active"
    };
    // asset_account_id, cum_dep_account_id, expense_account_id are INTEGER FKs
    let asset_acct_id: i64 = asset_acct["code"]
        .as_str()
        .and_then(|c| {
            db.query_row(
                "SELECT id FROM accounts WHERE code = ?1",
                [c],
                |r| r.get(0),
            )
            .ok()
        })
        .or_else(|| {
            db.query_row(
                "SELECT id FROM accounts WHERE code = ?1",
                [asset_account_code],
                |r| r.get(0),
            )
            .ok()
        })
        .ok_or_else(|| assets_error("ACCOUNT_NOT_FOUND", "asset account ID not found"))?;
    let expense_acct_id: i64 = expense_acct["code"]
        .as_str()
        .and_then(|c| {
            db.query_row(
                "SELECT id FROM accounts WHERE code = ?1",
                [c],
                |r| r.get(0),
            )
            .ok()
        })
        .or_else(|| {
            db.query_row(
                "SELECT id FROM accounts WHERE code = ?1",
                [expense_account_code],
                |r| r.get(0),
            )
            .ok()
        })
        .ok_or_else(|| assets_error("ACCOUNT_NOT_FOUND", "expense account ID not found"))?;
    let cum_dep_acct_id: Option<i64> = cum_dep_acct
        .as_ref()
        .and_then(|a| a["code"].as_str())
        .or(cum_dep_account_code)
        .and_then(|c| {
            db.query_row(
                "SELECT id FROM accounts WHERE code = ?1",
                [c],
                |r| r.get(0),
            )
            .ok()
        });
    db.execute(
        "INSERT INTO assets (name, category, serial, status, scheme_id, purchase_date, \
         purchase_price_cents, residual_cents, depreciation_start_date, recognition_date, \
         cum_dep_at_recognition_cents, asset_account_id, cum_dep_account_id, \
         expense_account_id, entry_id, note, created_by) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
        rusqlite::params![
            clean_name,
            category,
            serial,
            status,
            scheme["id"].as_i64().unwrap(),
            purchase_date,
            purchase_price_cents,
            residual,
            depreciation_start_date,
            recognition_date,
            cum_dep_at_recognition_cents,
            asset_acct_id,
            cum_dep_acct_id,
            expense_acct_id,
            entry_id,
            note,
            actor,
        ],
    )
    .map_err(sql_err)?;
    let asset_id = db.last_insert_rowid();
    get_asset(db, asset_id)?.ok_or_else(|| assets_error("DB_ERROR", "asset not found after insert"))
}

pub fn dispose_asset(
    db: &Connection,
    id: i64,
    date: &str,
    proceeds_cents: i64,
    bank_account_code: Option<&str>,
    result_account_code: Option<&str>,
    note: Option<&str>,
    actor: &str,
    dry_run: bool,
) -> Result<Value> {
    use crate::accounts::get_account_by_code;

    let asset = get_asset(db, id)?.ok_or_else(|| assets_error("ASSET_NOT_FOUND", format!("asset {id} does not exist")))?;
    if asset["status"].as_str() == Some("disposed") {
        return Err(assets_error("ALREADY_DISPOSED", format!("asset {id} is already disposed")));
    }
    if !valid_date(date) {
        return Err(assets_error("INVALID_DATE", format!("date '{date}' must be yyyy-mm-dd")));
    }
    let recog = asset["recognition_date"].as_str().unwrap_or("");
    if !recog.is_empty() && date < recog {
        return Err(assets_error("INVALID_DATE", "disposal date cannot be before the recognition date"));
    }
    if proceeds_cents < 0 {
        return Err(assets_error("INVALID_AMOUNT", "proceeds must be a non-negative amount in cents"));
    }

    // Compute book value at disposal
    let disposal_month = &date[..7];
    let prev = add_months(&format!("{disposal_month}-01"), -1);
    let prev_period = &prev[..7];
    let module_dep = module_dep_to_period(db, id, prev_period)?;
    let total_cum_dep = asset["cum_dep_at_recognition_cents"].as_i64().unwrap_or(0) + module_dep;
    let book_value = asset["purchase_price_cents"].as_i64().unwrap_or(0) - total_cum_dep;
    let result_cents = proceeds_cents - book_value;

    let bank_code = bank_account_code.unwrap_or("1100");
    let bank_acct = if proceeds_cents > 0 {
        get_account_by_code(db, bank_code)
            .ok_or_else(|| assets_error("ACCOUNT_NOT_FOUND", format!("bank account {bank_code} not found")))?
    } else {
        json!(null)
    };
    let result_code = result_account_code.unwrap_or("8100");
    let result_acct = get_account_by_code(db, result_code)
        .ok_or_else(|| assets_error("ACCOUNT_NOT_FOUND", format!("result account {result_code} not found")))?;

    let mut postings = Vec::new();
    if proceeds_cents > 0 {
        postings.push(json!({"code": bank_acct["code"], "amount_cents": proceeds_cents}));
    }
    if total_cum_dep > 0 {
        let cum_code = asset["cum_dep_account_code"].as_str()
            .or(asset["asset_account_code"].as_str()).unwrap_or("1800");
        postings.push(json!({"code": cum_code, "amount_cents": total_cum_dep}));
    }
    postings.push(json!({"code": asset["asset_account_code"], "amount_cents": -asset["purchase_price_cents"].as_i64().unwrap_or(0)}));
    if result_cents != 0 {
        postings.push(json!({"code": result_acct["code"], "amount_cents": -result_cents}));
    }

    if dry_run {
        return Ok(json!({
            "asset": {"id": asset["id"], "name": asset["name"]},
            "date": date, "proceeds_cents": proceeds_cents,
            "book_value_cents": book_value, "result_cents": result_cents,
            "total_cum_dep_cents": total_cum_dep, "postings": postings, "dryRun": true,
        }));
    }

    // Build description
    let asset_name = asset["name"].as_str().unwrap_or("");
    let description = format!("Afstoting {asset_name} ({date})");
    let source_ref = format!("dispose:{id}:{date}");

    // Create and post the entry
    let entry = crate::entries::create_entry(
        db,
        crate::entries::CreateEntry {
            date,
            description: &description,
            postings: postings.iter().map(|p| crate::entries::PostingSpec {
                code: p["code"].as_str().unwrap_or("").to_string(),
                amount_cents: p["amount_cents"].as_i64().unwrap_or(0),
                cost_center_code: None,
            }).collect(),
            source: "assets",
            source_ref: Some(&source_ref),
            actor,
        },
    )?;
    let posted = crate::entries::post_entry(db, entry.id, actor)?;

    db.execute(
        "UPDATE assets SET status = 'disposed', disposed_date = ?1, disposed_proceeds_cents = ?2, disposal_entry_id = ?3 WHERE id = ?4",
        rusqlite::params![date, proceeds_cents, posted.id, id],
    ).map_err(sql_err)?;

    record(db, RecordArgs {
        actor,
        action: "assets.dispose",
        command: Some("assets dispose"),
        args: Some(json!({"asset_id": id, "date": date, "proceeds_cents": proceeds_cents, "result_cents": result_cents})),
        outcome: "ok",
        entry_ids: vec![posted.id],
    })?;

    Ok(json!({
        "asset": {"id": id, "name": asset_name, "status": "disposed", "disposed_date": date, "disposed_proceeds_cents": proceeds_cents},
        "entry": {"id": posted.id, "date": date, "description": description, "state": posted.state},
        "book_value_cents": book_value, "result_cents": result_cents,
        "postings": postings, "dryRun": false,
    }))
}

pub fn pause_asset(db: &Connection, id: i64, actor: &str, dry_run: bool) -> Result<Value> {
    let asset = get_asset(db, id)?.ok_or_else(|| assets_error("ASSET_NOT_FOUND", format!("asset {id} does not exist")))?;
    if asset["status"].as_str() != Some("active") {
        return Err(assets_error("INVALID_STATUS", format!("asset {id} is {}, only active assets can be paused", asset["status"])));
    }
    if dry_run {
        return Ok(json!({
            "action": "assets.pause", "asset_id": id, "name": asset["name"],
            "from": asset["status"], "to": "paused", "dryRun": true,
        }));
    }
    db.execute("UPDATE assets SET status = 'paused' WHERE id = ?1", [id]).map_err(sql_err)?;
    record(db, RecordArgs {
        actor, action: "assets.pause", command: Some("assets pause"),
        args: Some(json!({"asset_id": id})), outcome: "ok", entry_ids: vec![],
    })?;
    Ok(json!({"asset": {"id": id, "name": asset["name"], "status": "paused"}}))
}

pub fn resume_asset(db: &Connection, id: i64, actor: &str, dry_run: bool) -> Result<Value> {
    let asset = get_asset(db, id)?.ok_or_else(|| assets_error("ASSET_NOT_FOUND", format!("asset {id} does not exist")))?;
    if asset["status"].as_str() != Some("paused") {
        return Err(assets_error("INVALID_STATUS", format!("asset {id} is {}, only paused assets can be resumed", asset["status"])));
    }
    if dry_run {
        return Ok(json!({
            "action": "assets.resume", "asset_id": id, "name": asset["name"],
            "from": asset["status"], "to": "active", "dryRun": true,
        }));
    }
    db.execute("UPDATE assets SET status = 'active' WHERE id = ?1", [id]).map_err(sql_err)?;
    record(db, RecordArgs {
        actor, action: "assets.resume", command: Some("assets resume"),
        args: Some(json!({"asset_id": id})), outcome: "ok", entry_ids: vec![],
    })?;
    Ok(json!({"asset": {"id": id, "name": asset["name"], "status": "active"}}))
}

pub fn register(db: &Connection, as_of: Option<&str>, actor: &str) -> Result<Value> {
    let target_period = match as_of {
        Some(d) if valid_date(d) => d[..7].to_string(),
        None => chrono::Utc::now().format("%Y-%m").to_string(),
        _ => return Err(assets_error("INVALID_DATE", format!("as-of '{}' must be yyyy-mm-dd", as_of.unwrap_or("")))),
    };
    let assets = list_assets(db, None)?;
    let mut rows = Vec::new();
    for a in &assets {
        let aid = a["id"].as_i64().unwrap();
        let booked = module_dep_to_period(db, aid, &target_period)?;
        let total_dep = a["cum_dep_at_recognition_cents"].as_i64().unwrap_or(0) + booked;
        let purchase_price = a["purchase_price_cents"].as_i64().unwrap_or(0);
        let residual = a["residual_cents"].as_i64().unwrap_or(0);
        let book_value = std::cmp::max(0, purchase_price - total_dep);
        let remaining = std::cmp::max(0, purchase_price - residual - total_dep);
        let recog = a["recognition_date"].as_str().unwrap_or("");
        let dep_start = a["depreciation_start_date"].as_str().unwrap_or("");
        let scheme = a["scheme"].as_object();
        let life_months = scheme.and_then(|s| s["life_months"].as_i64()).unwrap_or(60);
        let elapsed = std::cmp::max(0, month_diff(dep_start, recog));
        let method = scheme.and_then(|s| s["method"].as_str()).unwrap_or("lineair");
        let first_period = first_run_period(recog, dep_start);

        // Find next unbooked period
        let booked_set: std::collections::HashSet<String> = db
            .prepare("SELECT period FROM asset_depreciation_runs WHERE asset_id = ?1")
            .map_err(sql_err)?
            .query_map([aid], |r| r.get::<_, String>(0))
            .map_err(sql_err)?
            .filter_map(|r| r.ok())
            .collect();
        let remaining_cost = std::cmp::max(0, purchase_price - residual - a["cum_dep_at_recognition_cents"].as_i64().unwrap_or(0));
        let ml = std::cmp::max(1, life_months - elapsed as i64);
        let sched = schedule_depreciation(remaining_cost, 0, ml, method, &first_period, &next_period(&target_period));
        let next_run = sched.iter().find(|(p, _)| !booked_set.contains(p)).map(|(p, _)| p.clone());

        rows.push(json!({
            "id": aid, "name": a["name"], "category": a["category"], "serial": a["serial"],
            "status": a["status"],
            "scheme": scheme.and_then(|s| s["name"].as_str()).unwrap_or(""),
            "method": method, "life_months": life_months,
            "purchase_date": a["purchase_date"],
            "purchase_price_cents": purchase_price, "residual_cents": residual,
            "cum_dep_at_recognition_cents": a["cum_dep_at_recognition_cents"],
            "module_dep_to_date_cents": booked,
            "total_cum_dep_cents": total_dep, "book_value_cents": book_value,
            "remaining_to_depreciate_cents": remaining,
            "next_run_period": next_run,
            "asset_account": a["asset_account_code"],
            "cum_dep_account": a["cum_dep_account_code"],
            "expense_account": a["expense_account_code"],
            "disposed_date": a["disposed_date"],
            "disposed_proceeds_cents": a["disposed_proceeds_cents"],
        }));
    }
    let totals = json!({
        "purchase_price_cents": rows.iter().map(|r| r["purchase_price_cents"].as_i64().unwrap_or(0)).sum::<i64>(),
        "total_cum_dep_cents": rows.iter().map(|r| r["total_cum_dep_cents"].as_i64().unwrap_or(0)).sum::<i64>(),
        "book_value_cents": rows.iter().map(|r| r["book_value_cents"].as_i64().unwrap_or(0)).sum::<i64>(),
    });
    Ok(json!({"as_of": target_period, "assets": rows, "totals": totals, "actor": actor}))
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
