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
pub fn valid_date(s: &str) -> bool {
    chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok()
}

/// `YYYY-MM` with month 01-12 (mirrors the JS /^\d{4}-(0[1-9]|1[0-2])$/).
fn valid_period(s: &str) -> bool {
    s.len() == 7 && chrono::NaiveDate::parse_from_str(&format!("{s}-01"), "%Y-%m-%d").is_ok()
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
    // the JS listSchemes is SELECT * — every column, not a projection
    let mut stmt = db
        .prepare("SELECT * FROM depreciation_schemes ORDER BY id")
        .map_err(sql_err)?;
    let rows = stmt.query_map([], row_json).map_err(sql_err)?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

pub fn get_scheme_by_name(db: &Connection, name: &str) -> Result<Option<Value>> {
    let r = db.query_row(
        "SELECT id, name, method, life_months, residual_bp FROM depreciation_schemes WHERE name = ?1",
        [name],
        |r| {
            Ok(json!({
                "id": r.get::<_, i64>(0)?, "name": r.get::<_, String>(1)?,
                "method": r.get::<_, String>(2)?, "life_months": r.get::<_, i64>(3)?,
                "residual_bp": r.get::<_, i64>(4)?,
            }))
        },
    );
    match r {
        Ok(v) => Ok(Some(v)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(sql_err(e)),
    }
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
    let clean = name.trim();
    if clean.is_empty() {
        return Err(assets_error("INVALID_NAME", "scheme needs a name"));
    }
    if get_scheme_by_name(db, clean)?.is_some() {
        return Err(assets_error(
            "SCHEME_NAME_TAKEN",
            format!("scheme '{clean}' already exists"),
        ));
    }
    if !["lineair", "degressief"].contains(&method) {
        return Err(assets_error(
            "INVALID_METHOD",
            format!("method must be 'lineair' or 'degressief', got '{method}'"),
        ));
    }
    if life_months < 1 || life_months > 600 {
        return Err(assets_error(
            "INVALID_LIFE",
            "life-months must be an integer between 1 and 600",
        ));
    }
    if residual_bp < 0 || residual_bp > 10000 {
        return Err(assets_error(
            "INVALID_RESIDUAL",
            "residual-bp must be between 0 and 10000",
        ));
    }
    if dry_run {
        return Ok(
            json!({"action": "assets.scheme.add", "name": clean, "method": method, "life_months": life_months, "residual_bp": residual_bp, "dryRun": true}),
        );
    }
    db.execute("INSERT INTO depreciation_schemes (name, method, life_months, residual_bp, created_by) VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![clean, method, life_months, residual_bp, actor]).map_err(sql_err)?;
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

/// All of a row's columns as JSON — the JS spread `...row`.
fn row_json(row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    let mut obj = serde_json::Map::new();
    for (i, col) in row.as_ref().column_names().iter().enumerate() {
        let v = match row.get_ref(i)? {
            rusqlite::types::ValueRef::Null => Value::Null,
            rusqlite::types::ValueRef::Integer(n) => json!(n),
            rusqlite::types::ValueRef::Real(f) => json!(f),
            rusqlite::types::ValueRef::Text(t) => json!(String::from_utf8_lossy(t).to_string()),
            // ponytail: no blob columns on the tables this reads; length as placeholder
            rusqlite::types::ValueRef::Blob(b) => json!(b.len()),
        };
        obj.insert((*col).to_string(), v);
    }
    Ok(Value::Object(obj))
}

/// Mirrors the JS serializeAsset: the whole row spread, then the resolved
/// account codes/names and the scheme as an object. The spread matters —
/// register() reads category/serial/disposed_* straight off this value.
fn serialize_asset(db: &Connection, row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    let Value::Object(mut obj) = row_json(row)? else {
        unreachable!()
    };
    let account = |id: Option<i64>| -> Option<(String, String)> {
        let id = id?;
        db.query_row("SELECT code, name FROM accounts WHERE id = ?1", [id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .ok()
    };
    let asset_acc = account(obj.get("asset_account_id").and_then(|v| v.as_i64()));
    let cum_acc = account(obj.get("cum_dep_account_id").and_then(|v| v.as_i64()));
    let exp_acc = account(obj.get("expense_account_id").and_then(|v| v.as_i64()));
    for (key, acc) in [
        ("asset", &asset_acc),
        ("cum_dep", &cum_acc),
        ("expense", &exp_acc),
    ] {
        obj.insert(
            format!("{key}_account_code"),
            str_or_null(acc.as_ref().map(|a| a.0.as_str())),
        );
        obj.insert(
            format!("{key}_account_name"),
            str_or_null(acc.as_ref().map(|a| a.1.as_str())),
        );
    }
    let scheme = obj
        .get("scheme_id")
        .and_then(|v| v.as_i64())
        .and_then(|id| get_scheme(db, id).ok().flatten())
        .map(|s| {
            json!({
                "id": s["id"], "name": s["name"], "method": s["method"],
                "life_months": s["life_months"], "residual_bp": s["residual_bp"],
            })
        })
        .unwrap_or(Value::Null);
    obj.insert("scheme".to_string(), scheme);
    Ok(Value::Object(obj))
}

fn str_or_null(v: Option<&str>) -> Value {
    match v {
        Some(s) => Value::String(s.to_string()),
        None => Value::Null,
    }
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
    if !period.is_empty() && !valid_period(period) {
        return Err(assets_error(
            "INVALID_PERIOD",
            format!("period '{period}' must be YYYY-MM (01-12)"),
        ));
    }
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
            let periods: Vec<Value> = due
                .iter()
                .map(|(p, amt)| json!({"period": p, "amountCents": amt}))
                .collect();
            plan.push(json!({
                "asset_id": aid, "name": a["name"], "periods": periods, "total_cents": total,
            }));
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
    let tx = crate::entries::begin(db)?;
    for (a, due, exp, cum) in &to_book {
        let aid = a["id"].as_i64().unwrap();
        let price = a["purchase_price_cents"].as_i64().unwrap_or(0);
        let residual = a["residual_cents"].as_i64().unwrap_or(0);
        let recognised = a["cum_dep_at_recognition_cents"].as_i64().unwrap_or(0);
        for (period, amt) in due {
            let desc = format!("Afschrijving {} {period}", a["name"].as_str().unwrap_or(""));
            let sr = format!("asset:{aid}:{period}");
            let entry = create_entry(
                &tx,
                CreateEntry {
                    date: &format!("{period}-01"),
                    description: &desc,
                    postings: vec![
                        PostingSpec {
                            code: exp.clone(),
                            amount_cents: *amt,
                            cost_center_code: None,
                            vat_code: None,
                            vat_amount_cents: None,
                            fx_currency: None,
                            fx_amount_cents: None,
                        },
                        PostingSpec {
                            code: cum.clone(),
                            amount_cents: -amt,
                            cost_center_code: None,
                            vat_code: None,
                            vat_amount_cents: None,
                            fx_currency: None,
                            fx_amount_cents: None,
                        },
                    ],
                    source: "assets",
                    source_ref: Some(&sr),
                    actor,
                },
            )?;
            let posted = post_entry(&tx, entry.id, actor)?;
            tx.execute("INSERT INTO asset_depreciation_runs (asset_id, period, entry_id, amount_cents, created_by) VALUES (?1, ?2, ?3, ?4, ?5)", rusqlite::params![aid, period, posted.id, amt, actor]).map_err(sql_err)?;
            booked.push(json!({"asset_id": aid, "period": period, "entry_id": posted.id, "amount_cents": amt}));
            // auto-complete: the asset reached its residual
            let total_dep = recognised + module_dep_to_period(&tx, aid, period)?;
            if total_dep >= price - residual {
                tx.execute(
                    "UPDATE assets SET status = 'fully_depreciated' WHERE id = ?1 AND status = 'active'",
                    [aid],
                )
                .map_err(sql_err)?;
            }
        }
    }
    tx.commit()?;
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
        return Err(assets_error(
            "INVALID_DATE",
            format!("purchase-date '{purchase_date}' must be yyyy-mm-dd"),
        ));
    }
    if !valid_date(depreciation_start_date) {
        return Err(assets_error(
            "INVALID_DATE",
            format!("depreciation-start-date '{depreciation_start_date}' must be yyyy-mm-dd"),
        ));
    }
    if !valid_date(recognition_date) {
        return Err(assets_error(
            "INVALID_DATE",
            format!("recognition-date '{recognition_date}' must be yyyy-mm-dd"),
        ));
    }
    if depreciation_start_date < purchase_date {
        return Err(assets_error(
            "INVALID_DATE",
            "depreciation-start-date cannot be before purchase-date",
        ));
    }
    if recognition_date < purchase_date {
        return Err(assets_error(
            "INVALID_DATE",
            "recognition-date cannot be before purchase-date",
        ));
    }
    if purchase_price_cents <= 0 {
        return Err(assets_error(
            "INVALID_COST",
            "purchase-price must be a positive amount in cents",
        ));
    }
    if cum_dep_at_recognition_cents < 0 {
        return Err(assets_error(
            "INVALID_DEPRECIATION",
            "cumulative depreciation at recognition must be >= 0",
        ));
    }

    // Resolve scheme: explicit id, or built from inline method/life/residual, or default
    let scheme = if let Some(sid) = scheme_id {
        get_scheme(db, sid)?.ok_or_else(|| {
            assets_error("SCHEME_NOT_FOUND", format!("scheme {sid} does not exist"))
        })?
    } else if method.is_some() || life_months.is_some() {
        let m = method.unwrap_or("lineair");
        let life = life_months.unwrap_or(60);
        let bp = residual_bp.unwrap_or(0);
        let scheme_name = format!("{life} maanden {m} {bp}basis");
        if dry_run {
            // a dry-run must not write: reuse an existing scheme with the same
            // parameters, else plan with an in-memory one (the JS does the same,
            // so a second dry-run cannot fail SCHEME_NAME_TAKEN)
            match get_scheme_by_name(db, &scheme_name)? {
                Some(s) => s,
                None => json!({
                    "id": Value::Null, "name": scheme_name, "method": m,
                    "life_months": life, "residual_bp": bp,
                }),
            }
        } else {
            create_scheme(db, &scheme_name, m, life, bp, actor, false)?
        }
    } else if dry_run {
        match get_scheme_by_name(db, "Standaard 5 jaar lineair")? {
            Some(s) => s,
            None => json!({
                "id": Value::Null, "name": "Standaard 5 jaar lineair",
                "method": "lineair", "life_months": 60, "residual_bp": 0,
            }),
        }
    } else {
        ensure_default_scheme(db, actor)?
    };
    let scheme_residual_cents = ((scheme["residual_bp"].as_i64().unwrap_or(0) as f64) / 10000.0
        * purchase_price_cents as f64)
        .round() as i64;
    let residual = residual_cents_override.unwrap_or(scheme_residual_cents);
    if residual < 0 || residual >= purchase_price_cents {
        return Err(assets_error(
            "INVALID_RESIDUAL",
            format!("residual must be >= 0 and < purchase-price (got {residual})"),
        ));
    }

    // Resolve accounts: each must exist and carry the expected type
    let resolve = |code: &str, expected: &str, label: &str| -> Result<Value> {
        if code.trim().is_empty() {
            return Err(assets_error(
                "ACCOUNT_NOT_FOUND",
                format!("{label} account is required"),
            ));
        }
        let acc = get_account_by_code(db, code).ok_or_else(|| {
            assets_error(
                "ACCOUNT_NOT_FOUND",
                format!("{label} account {code} does not exist"),
            )
        })?;
        let got = acc["type"].as_str().unwrap_or("");
        if got != expected {
            return Err(assets_error(
                "ACCOUNT_TYPE",
                format!("{label} account {code} is '{got}', expected '{expected}'"),
            ));
        }
        Ok(acc)
    };
    let asset_acct = resolve(asset_account_code, "asset", "asset")?;
    let expense_acct = resolve(expense_account_code, "expense", "expense")?;
    let cum_dep_acct = match cum_dep_account_code {
        Some(c) => Some(resolve(c, "asset", "cumulative-depreciation")?),
        None => None,
    };

    if let Some(eid) = entry_id {
        let exists: bool = db
            .query_row(
                "SELECT COUNT(*) FROM journal_entries WHERE id = ?1",
                [eid],
                |r| r.get::<_, i64>(0),
            )
            .map_err(sql_err)?
            > 0;
        if !exists {
            return Err(assets_error(
                "ENTRY_NOT_FOUND",
                format!("entry {eid} does not exist"),
            ));
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
            db.query_row("SELECT id FROM accounts WHERE code = ?1", [c], |r| r.get(0))
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
            db.query_row("SELECT id FROM accounts WHERE code = ?1", [c], |r| r.get(0))
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
            db.query_row("SELECT id FROM accounts WHERE code = ?1", [c], |r| r.get(0))
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
    let asset = get_asset(db, asset_id)?
        .ok_or_else(|| assets_error("DB_ERROR", "asset not found after insert"))?;

    // GL reconciliation warnings (never blockers — migration must stay frictionless)
    let mut warnings: Vec<String> = Vec::new();
    let posted_balance = |code: &str| -> i64 {
        db.query_row(
            "SELECT COALESCE(SUM(p.amount_cents),0) FROM postings p \
             JOIN journal_entries e ON e.id = p.entry_id AND e.state = 'posted' \
             WHERE p.account_id = (SELECT id FROM accounts WHERE code = ?1)",
            [code],
            |r| r.get::<_, i64>(0),
        )
        .unwrap_or(0)
    };
    let asset_balance = posted_balance(asset_account_code);
    if asset_balance < purchase_price_cents {
        warnings.push(format!(
            "asset account {asset_account_code} carries {asset_balance} on the ledger, less than the purchase price {purchase_price_cents} — verify the purchase was booked there"
        ));
    }
    if let Some(cum) = cum_dep_account_code {
        let cum_balance = posted_balance(cum);
        if cum_balance.abs() < cum_dep_at_recognition_cents {
            warnings.push(format!(
                "cumulative-depreciation account {cum} carries {cum_balance} on the ledger, less than the recognised {cum_dep_at_recognition_cents}"
            ));
        }
    }
    if let Some(eid) = entry_id {
        let entry_date: Option<String> = db
            .query_row(
                "SELECT date FROM journal_entries WHERE id = ?1",
                [eid],
                |r| r.get(0),
            )
            .ok();
        if entry_date.as_deref() != Some(purchase_date) {
            warnings.push(format!(
                "linked entry {eid} has a different date than purchase-date"
            ));
        }
    }

    record(
        db,
        RecordArgs {
            actor,
            action: "assets.add",
            command: Some("assets add"),
            args: Some(json!({
                "name": clean_name, "purchase_price_cents": purchase_price_cents,
                "scheme": scheme["name"], "recognition_date": recognition_date,
                "cum_dep_at_recognition_cents": cum_dep_at_recognition_cents,
            })),
            outcome: "ok",
            entry_ids: vec![],
        },
    )?;
    Ok(json!({"asset": asset, "warnings": warnings, "dryRun": false}))
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

    let asset = get_asset(db, id)?
        .ok_or_else(|| assets_error("ASSET_NOT_FOUND", format!("asset {id} does not exist")))?;
    if asset["status"].as_str() == Some("disposed") {
        return Err(assets_error(
            "ALREADY_DISPOSED",
            format!("asset {id} is already disposed"),
        ));
    }
    if !valid_date(date) {
        return Err(assets_error(
            "INVALID_DATE",
            format!("date '{date}' must be yyyy-mm-dd"),
        ));
    }
    let recog = asset["recognition_date"].as_str().unwrap_or("");
    if !recog.is_empty() && date < recog {
        return Err(assets_error(
            "INVALID_DATE",
            "disposal date cannot be before the recognition date",
        ));
    }
    if proceeds_cents < 0 {
        return Err(assets_error(
            "INVALID_AMOUNT",
            "proceeds must be a non-negative amount in cents",
        ));
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
        get_account_by_code(db, bank_code).ok_or_else(|| {
            assets_error(
                "ACCOUNT_NOT_FOUND",
                format!("bank account {bank_code} not found"),
            )
        })?
    } else {
        json!(null)
    };
    let result_code = result_account_code.unwrap_or("8100");
    let result_acct = get_account_by_code(db, result_code).ok_or_else(|| {
        assets_error(
            "ACCOUNT_NOT_FOUND",
            format!("result account {result_code} not found"),
        )
    })?;

    let mut postings = Vec::new();
    if proceeds_cents > 0 {
        postings.push(json!({"code": bank_acct["code"], "amountCents": proceeds_cents}));
    }
    if total_cum_dep > 0 {
        let cum_code = asset["cum_dep_account_code"]
            .as_str()
            .or(asset["asset_account_code"].as_str())
            .unwrap_or("1800");
        postings.push(json!({"code": cum_code, "amountCents": total_cum_dep}));
    }
    postings.push(json!({"code": asset["asset_account_code"], "amountCents": -asset["purchase_price_cents"].as_i64().unwrap_or(0)}));
    if result_cents != 0 {
        postings.push(json!({"code": result_acct["code"], "amountCents": -result_cents}));
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

    // Create and post the entry, then close the asset — one transaction, so a
    // failure on either side leaves neither (the JS wraps both the same way)
    let tx = crate::entries::begin(db)?;
    let entry = crate::entries::create_entry(
        &tx,
        crate::entries::CreateEntry {
            date,
            description: &description,
            postings: postings
                .iter()
                .map(|p| crate::entries::PostingSpec {
                    code: p["code"].as_str().unwrap_or("").to_string(),
                    amount_cents: p["amountCents"].as_i64().unwrap_or(0),
                    cost_center_code: None,
                    vat_code: None,
                    vat_amount_cents: None,
                    fx_currency: None,
                    fx_amount_cents: None,
                })
                .collect(),
            source: "assets",
            source_ref: Some(&source_ref),
            actor,
        },
    )?;
    let posted = crate::entries::post_entry(&tx, entry.id, actor)?;

    tx.execute(
        "UPDATE assets SET status = 'disposed', disposed_date = ?1, disposed_proceeds_cents = ?2, disposal_entry_id = ?3 WHERE id = ?4",
        rusqlite::params![date, proceeds_cents, posted.id, id],
    ).map_err(sql_err)?;
    tx.commit()?;

    record(
        db,
        RecordArgs {
            actor,
            action: "assets.dispose",
            command: Some("assets dispose"),
            args: Some(
                json!({"asset_id": id, "date": date, "proceeds_cents": proceeds_cents, "result_cents": result_cents}),
            ),
            outcome: "ok",
            entry_ids: vec![posted.id],
        },
    )?;

    Ok(json!({
        "asset": {"id": id, "name": asset_name, "status": "disposed", "disposed_date": date, "disposed_proceeds_cents": proceeds_cents},
        "entry": {"id": posted.id, "date": date, "description": description, "state": posted.state},
        "book_value_cents": book_value, "result_cents": result_cents,
        "postings": postings, "dryRun": false,
    }))
}

pub fn pause_asset(db: &Connection, id: i64, actor: &str, dry_run: bool) -> Result<Value> {
    let asset = get_asset(db, id)?
        .ok_or_else(|| assets_error("ASSET_NOT_FOUND", format!("asset {id} does not exist")))?;
    if asset["status"].as_str() != Some("active") {
        return Err(assets_error(
            "INVALID_STATUS",
            format!(
                "asset {id} is {}, only active assets can be paused",
                asset["status"]
            ),
        ));
    }
    if dry_run {
        return Ok(json!({
            "action": "assets.pause", "asset_id": id, "name": asset["name"],
            "from": asset["status"], "to": "paused", "dryRun": true,
        }));
    }
    db.execute("UPDATE assets SET status = 'paused' WHERE id = ?1", [id])
        .map_err(sql_err)?;
    record(
        db,
        RecordArgs {
            actor,
            action: "assets.pause",
            command: Some("assets pause"),
            args: Some(json!({"asset_id": id})),
            outcome: "ok",
            entry_ids: vec![],
        },
    )?;
    Ok(json!({"asset": {"id": id, "name": asset["name"], "status": "paused"}}))
}

pub fn resume_asset(db: &Connection, id: i64, actor: &str, dry_run: bool) -> Result<Value> {
    let asset = get_asset(db, id)?
        .ok_or_else(|| assets_error("ASSET_NOT_FOUND", format!("asset {id} does not exist")))?;
    if asset["status"].as_str() != Some("paused") {
        return Err(assets_error(
            "INVALID_STATUS",
            format!(
                "asset {id} is {}, only paused assets can be resumed",
                asset["status"]
            ),
        ));
    }
    if dry_run {
        return Ok(json!({
            "action": "assets.resume", "asset_id": id, "name": asset["name"],
            "from": asset["status"], "to": "active", "dryRun": true,
        }));
    }
    db.execute("UPDATE assets SET status = 'active' WHERE id = ?1", [id])
        .map_err(sql_err)?;
    record(
        db,
        RecordArgs {
            actor,
            action: "assets.resume",
            command: Some("assets resume"),
            args: Some(json!({"asset_id": id})),
            outcome: "ok",
            entry_ids: vec![],
        },
    )?;
    Ok(json!({"asset": {"id": id, "name": asset["name"], "status": "active"}}))
}

pub fn register(db: &Connection, as_of: Option<&str>, actor: &str) -> Result<Value> {
    let target_period = match as_of {
        Some(d) if valid_date(d) => d[..7].to_string(),
        None => chrono::Utc::now().format("%Y-%m").to_string(),
        _ => {
            return Err(assets_error(
                "INVALID_DATE",
                format!("as-of '{}' must be yyyy-mm-dd", as_of.unwrap_or("")),
            ))
        }
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
        let method = scheme
            .and_then(|s| s["method"].as_str())
            .unwrap_or("lineair");
        let first_period = first_run_period(recog, dep_start);

        // Find next unbooked period
        let booked_set: std::collections::HashSet<String> = db
            .prepare("SELECT period FROM asset_depreciation_runs WHERE asset_id = ?1")
            .map_err(sql_err)?
            .query_map([aid], |r| r.get::<_, String>(0))
            .map_err(sql_err)?
            .filter_map(|r| r.ok())
            .collect();
        let remaining_cost = std::cmp::max(
            0,
            purchase_price - residual - a["cum_dep_at_recognition_cents"].as_i64().unwrap_or(0),
        );
        let ml = std::cmp::max(1, life_months - elapsed as i64);
        let sched = schedule_depreciation(
            remaining_cost,
            0,
            ml,
            method,
            &first_period,
            &next_period(&target_period),
        );
        let next_run = sched
            .iter()
            .find(|(p, _)| !booked_set.contains(p))
            .map(|(p, _)| p.clone());

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

    // ==== ported from test/assets.test.js ====================================

    use crate::accounts::{create_account, NewAccount};

    fn chart_db() -> Connection {
        let d = test_db();
        create_account(
            &d,
            &NewAccount {
                code: "1500",
                name: "Cumulatieve afschrijvingen",
                type_: "asset",
                normal_balance: "debit",
                taxonomy_code: None,
            },
        )
        .unwrap();
        d
    }

    fn spec(code: &str, cents: i64) -> crate::entries::PostingSpec {
        crate::entries::PostingSpec {
            code: code.to_string(),
            amount_cents: cents,
            cost_center_code: None,
            vat_code: None,
            vat_amount_cents: None,
            fx_currency: None,
            fx_amount_cents: None,
        }
    }

    fn posted(db: &Connection) -> Vec<Value> {
        crate::entries::list_entries(db, Some("posted"), None, None, 1000).unwrap()
    }

    /// create_asset with this suite's defaults (agent:test, no category/serial/note).
    #[allow(clippy::too_many_arguments)]
    fn add_asset(
        db: &Connection,
        name: &str,
        purchase_date: &str,
        price: i64,
        dep_start: &str,
        recognition: &str,
        cum_dep_at_recognition: i64,
        asset_account: &str,
        cum_dep_account: Option<&str>,
        expense_account: &str,
        entry_id: Option<i64>,
        dry_run: bool,
    ) -> Value {
        create_asset(
            db,
            name,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            purchase_date,
            price,
            dep_start,
            recognition,
            cum_dep_at_recognition,
            asset_account,
            cum_dep_account,
            expense_account,
            entry_id,
            None,
            "agent:test",
            dry_run,
        )
        .unwrap()
    }

    fn asset_id(v: &Value) -> i64 {
        v["id"].as_i64().unwrap()
    }

    /// Postings summed per account code, keys sorted.
    fn sums_by_code(postings: &Value) -> Vec<(String, i64)> {
        let mut m: std::collections::BTreeMap<String, i64> = std::collections::BTreeMap::new();
        for p in postings.as_array().cloned().unwrap_or_default() {
            *m.entry(p["code"].as_str().unwrap_or("").to_string())
                .or_insert(0) += p["amountCents"].as_i64().unwrap_or(0);
        }
        m.into_iter().collect()
    }

    /// The account codes of an entry's legs, sorted.
    fn leg_codes(db: &Connection, entry_id: i64) -> Vec<String> {
        let mut stmt = db
            .prepare(
                "SELECT a.code FROM postings p JOIN accounts a ON a.id = p.account_id \
                 WHERE p.entry_id = ?1 ORDER BY a.code",
            )
            .unwrap();
        let rows = stmt
            .query_map([entry_id], |r| r.get::<_, String>(0))
            .unwrap();
        rows.filter_map(|r| r.ok()).collect()
    }

    #[test]
    fn ensure_default_scheme_creates_the_standard_5y_linear_scheme_lazily() {
        let db = chart_db();
        let s = ensure_default_scheme(&db, "human:erik").unwrap();
        assert_eq!(s["name"].as_str(), Some("Standaard 5 jaar lineair"));
        assert_eq!(s["method"].as_str(), Some("lineair"));
        assert_eq!(s["life_months"].as_i64(), Some(60));
        assert_eq!(s["residual_bp"].as_i64(), Some(0));
        // idempotent
        assert_eq!(
            ensure_default_scheme(&db, "human:erik").unwrap()["id"],
            s["id"]
        );
    }

    #[test]
    fn create_scheme_rejects_duplicate_names_and_bad_methods() {
        let db = chart_db();
        create_scheme(&db, "A", "lineair", 60, 0, "human:erik", false).unwrap();
        assert_eq!(
            create_scheme(&db, "A", "lineair", 60, 0, "human:erik", false)
                .unwrap_err()
                .code,
            "SCHEME_NAME_TAKEN"
        );
        assert_eq!(
            create_scheme(&db, "B", "weird", 60, 0, "human:erik", false)
                .unwrap_err()
                .code,
            "INVALID_METHOD"
        );
        assert_eq!(
            create_scheme(&db, "C", "lineair", 0, 0, "human:erik", false)
                .unwrap_err()
                .code,
            "INVALID_LIFE"
        );
    }

    #[test]
    fn schedule_linear_60m_is_cents_exact_and_remainder_adjusted() {
        let s = schedule_depreciation(100000, 0, 60, "lineair", "2026-01", "2030-12");
        assert_eq!(s.len(), 60);
        assert_eq!(s[0].1, 1667);
        let head: i64 = s[..59].iter().map(|(_, a)| *a).sum();
        assert_eq!(s[59].1, 100000 - head);
        let total: i64 = s.iter().map(|(_, a)| *a).sum();
        assert_eq!(total, 100000); // cents-exact
        assert!(s.iter().all(|(_, a)| (*a - 1667).abs() <= 2)); // no drift
    }

    #[test]
    fn schedule_degressief_is_double_declining_with_a_switch_to_linear() {
        let s = schedule_depreciation(60000, 0, 12, "degressief", "2025-02", "2026-02");
        assert_eq!(s.len(), 12);
        assert_eq!(s[0].1, 10000); // 600 * 2/12
        assert_eq!(s[6].1, 3349); // switched to the linear view
        let total: i64 = s.iter().map(|(_, a)| *a).sum();
        assert_eq!(total, 60000); // exactly to residual
    }

    #[test]
    fn schedule_stops_at_the_residual_and_never_overshoots() {
        let s = schedule_depreciation(10000, 1000, 36, "lineair", "2026-01", "2035-12");
        let total: i64 = s.iter().map(|(_, a)| *a).sum();
        assert_eq!(total, 9000);
        assert!(s.len() <= 36);
    }

    #[test]
    fn add_asset_standard_5y_linear_warns_when_the_purchase_is_not_booked() {
        let db = chart_db();
        let r = add_asset(
            &db,
            "Laptop",
            "2024-01-15",
            200000,
            "2024-02-01",
            "2024-02-01",
            0,
            "1800",
            None,
            "4600",
            None,
            false,
        );
        assert_eq!(r["asset"]["status"].as_str(), Some("active"));
        assert_eq!(
            r["asset"]["scheme"]["name"].as_str(),
            Some("Standaard 5 jaar lineair")
        );
        assert!(r["asset"]["cum_dep_account_code"].is_null()); // booked on the asset account
        assert!(r["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w.as_str().unwrap_or("").contains("asset account 1800")));
    }

    #[test]
    fn add_asset_mid_life_adoption_keeps_only_the_remaining_depreciation() {
        let db = chart_db();
        // bought 2023, recognised 2024-06 with 250.00 already depreciated:
        // remaining 950.00 over 48 months = 19.79
        let r = add_asset(
            &db,
            "Fotocamera",
            "2023-05-10",
            120000,
            "2023-06-01",
            "2024-06-01",
            25000,
            "1800",
            Some("1500"),
            "4600",
            None,
            false,
        );
        let aid = asset_id(&r["asset"]);
        assert_eq!(r["asset"]["status"].as_str(), Some("active"));
        assert_eq!(r["asset"]["scheme"]["life_months"].as_i64(), Some(60));
        let run = run_due(&db, "2024-06", "agent:test", false).unwrap();
        assert_eq!(run["booked"].as_array().unwrap().len(), 1);
        assert_eq!(run["booked"][0]["amount_cents"].as_i64(), Some(1979)); // round(950/48)
        let reg = register(&db, Some("2024-06-30"), "human:erik").unwrap();
        let a = reg["assets"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["id"].as_i64() == Some(aid))
            .unwrap();
        assert_eq!(a["total_cum_dep_cents"].as_i64(), Some(25000 + 1979));
        assert_eq!(a["book_value_cents"].as_i64(), Some(120000 - 25000 - 1979));
    }

    #[test]
    fn add_asset_rejects_cum_dep_at_recognition_above_cost_minus_residual() {
        let db = chart_db();
        let err = create_asset(
            &db,
            "X",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            "2024-01-01",
            100000,
            "2024-01-01",
            "2024-06-01",
            100001,
            "1800",
            None,
            "4600",
            None,
            None,
            "agent:test",
            false,
        )
        .unwrap_err();
        assert_eq!(err.code, "INVALID_DEPRECIATION");
    }

    #[test]
    fn add_asset_recognises_an_already_fully_depreciated_asset() {
        let db = chart_db();
        let r = add_asset(
            &db,
            "Oud",
            "2019-01-01",
            100000,
            "2019-02-01",
            "2025-02-01",
            100000,
            "1800",
            None,
            "4600",
            None,
            false,
        );
        assert_eq!(r["asset"]["status"].as_str(), Some("fully_depreciated"));
        let run = run_due(&db, "2026-01", "human:erik", false).unwrap();
        assert_eq!(run["booked"].as_array().unwrap().len(), 0); // nothing left
    }

    #[test]
    fn add_asset_validates_account_types() {
        let db = chart_db();
        let err = create_asset(
            &db,
            "X",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            "2024-01-01",
            100000,
            "2024-01-01",
            "2024-01-01",
            0,
            "8000",
            None,
            "4600",
            None,
            None,
            "agent:test",
            false, // 8000 is income
        )
        .unwrap_err();
        assert_eq!(err.code, "ACCOUNT_TYPE");
    }

    #[test]
    fn add_asset_missing_entry_link_fails_entry_not_found() {
        let db = chart_db();
        let err = create_asset(
            &db,
            "X",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            "2024-01-01",
            100000,
            "2024-01-01",
            "2024-01-01",
            0,
            "1800",
            None,
            "4600",
            Some(999),
            None,
            "agent:test",
            false,
        )
        .unwrap_err();
        assert_eq!(err.code, "ENTRY_NOT_FOUND");
    }

    #[test]
    fn add_asset_dry_run_writes_nothing() {
        let db = chart_db();
        let r = add_asset(
            &db,
            "Laptop",
            "2024-01-15",
            200000,
            "2024-02-01",
            "2024-02-01",
            0,
            "1800",
            None,
            "4600",
            None,
            true,
        );
        assert_eq!(r["dryRun"].as_bool(), Some(true));
        assert_eq!(r["asset"]["months_left"].as_i64(), Some(60));
        assert_eq!(list_assets(&db, None).unwrap().len(), 0);
    }

    #[test]
    fn run_due_books_monthly_depreciation_idempotently_per_asset_month() {
        let db = chart_db();
        add_asset(
            &db,
            "Laptop",
            "2024-01-15",
            200000,
            "2024-02-01",
            "2024-02-01",
            0,
            "1800",
            None,
            "4600",
            None,
            false,
        );
        let r1 = run_due(&db, "2024-03", "agent:test", false).unwrap();
        assert_eq!(r1["booked"].as_array().unwrap().len(), 2); // 2024-02 + 2024-03
        let r2 = run_due(&db, "2024-03", "agent:test", false).unwrap();
        assert_eq!(r2["booked"].as_array().unwrap().len(), 0); // idempotent
        let entries = posted(&db);
        assert_eq!(entries.len(), 2);
        for e in &entries {
            let full = crate::entries::get_entry(&db, e["id"].as_i64().unwrap()).unwrap();
            assert_eq!(full.source, "assets");
            assert!(full
                .source_ref
                .as_deref()
                .unwrap_or("")
                .starts_with("asset:"));
        }
    }

    #[test]
    fn run_due_skips_paused_assets_and_resuming_restarts_them() {
        let db = chart_db();
        let r = add_asset(
            &db,
            "Laptop",
            "2024-01-15",
            200000,
            "2024-02-01",
            "2024-02-01",
            0,
            "1800",
            None,
            "4600",
            None,
            false,
        );
        let aid = asset_id(&r["asset"]);
        db.execute("UPDATE assets SET status = 'paused' WHERE id = ?1", [aid])
            .unwrap();
        assert_eq!(
            run_due(&db, "2024-03", "human:erik", false).unwrap()["booked"]
                .as_array()
                .unwrap()
                .len(),
            0
        );
        db.execute("UPDATE assets SET status = 'active' WHERE id = ?1", [aid])
            .unwrap();
        assert_eq!(
            run_due(&db, "2024-03", "human:erik", false).unwrap()["booked"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn run_due_dry_run_plans_but_books_nothing() {
        let db = chart_db();
        add_asset(
            &db,
            "Laptop",
            "2024-01-15",
            200000,
            "2024-02-01",
            "2024-02-01",
            0,
            "1800",
            None,
            "4600",
            None,
            false,
        );
        let plan = run_due(&db, "2024-03", "human:erik", true).unwrap();
        assert_eq!(plan["plan"].as_array().unwrap().len(), 1);
        assert_eq!(plan["plan"][0]["periods"].as_array().unwrap().len(), 2);
        assert_eq!(posted(&db).len(), 0);
    }

    #[test]
    fn run_due_auto_completes_to_fully_depreciated_at_the_residual() {
        let db = chart_db();
        create_asset(
            &db,
            "Klein",
            None,
            None,
            None,
            None,
            Some(12),
            None,
            None,
            "2026-01-01",
            12000,
            "2026-02-01",
            "2026-02-01",
            0,
            "1800",
            None,
            "4600",
            None,
            None,
            "agent:test",
            false,
        )
        .unwrap();
        let r = run_due(&db, "2027-02", "human:erik", false).unwrap();
        assert_eq!(r["booked"].as_array().unwrap().len(), 12);
        assert_eq!(
            list_assets(&db, None).unwrap()[0]["status"].as_str(),
            Some("fully_depreciated")
        );
        let reg = register(&db, Some("2027-02-01"), "human:erik").unwrap();
        assert_eq!(
            reg["assets"][0]["total_cum_dep_cents"].as_i64(),
            Some(12000)
        );
    }

    #[test]
    fn run_due_books_on_the_cum_dep_account_when_given_else_on_the_asset_account() {
        let db = chart_db();
        add_asset(
            &db,
            "A",
            "2024-01-01",
            120000,
            "2024-02-01",
            "2024-02-01",
            0,
            "1800",
            Some("1500"),
            "4600",
            None,
            false,
        );
        add_asset(
            &db,
            "B",
            "2024-01-01",
            120000,
            "2024-02-01",
            "2024-02-01",
            0,
            "1850",
            None,
            "4600",
            None,
            false,
        );
        run_due(&db, "2024-02", "human:erik", false).unwrap();
        let mut entries = posted(&db);
        entries.sort_by_key(|e| e["id"].as_i64().unwrap());
        // asset A: cum-dep account leg; asset B: asset account leg
        assert_eq!(
            leg_codes(&db, entries[0]["id"].as_i64().unwrap()),
            vec!["1500".to_string(), "4600".to_string()]
        );
        assert_eq!(
            leg_codes(&db, entries[1]["id"].as_i64().unwrap()),
            vec!["1850".to_string(), "4600".to_string()]
        );
    }

    #[test]
    fn dispose_asset_sale_with_winst_books_the_full_entry_and_closes_the_asset() {
        let db = chart_db();
        let r = add_asset(
            &db,
            "Fotocamera",
            "2023-05-10",
            120000,
            "2023-06-01",
            "2024-06-01",
            25000,
            "1800",
            Some("1500"),
            "4600",
            None,
            false,
        );
        let aid = asset_id(&r["asset"]);
        run_due(&db, "2024-07", "human:erik", false).unwrap(); // 2 runs of 19.79
        let d = dispose_asset(
            &db,
            aid,
            "2024-08-15",
            95000,
            None,
            None,
            None,
            "agent:test",
            false,
        )
        .unwrap();
        // book value = 1200 - (250 + 39.58) = 910.42 -> winst 39.58
        assert_eq!(d["book_value_cents"].as_i64(), Some(91042));
        assert_eq!(d["result_cents"].as_i64(), Some(3958));
        assert_eq!(
            sums_by_code(&d["postings"]),
            vec![
                ("1100".to_string(), 95000),
                ("1500".to_string(), 28958),
                ("1800".to_string(), -120000),
                ("8100".to_string(), -3958),
            ]
        );
        let a = &list_assets(&db, None).unwrap()[0];
        assert_eq!(a["status"].as_str(), Some("disposed"));
        assert_eq!(a["disposed_proceeds_cents"].as_i64(), Some(95000));
        assert_eq!(d["entry"]["state"].as_str(), Some("posted"));
        assert_eq!(posted(&db).len(), 3); // 2 runs + disposal
    }

    #[test]
    fn dispose_asset_scrap_books_a_verlies() {
        let db = chart_db();
        let r = add_asset(
            &db,
            "Printer",
            "2024-01-01",
            50000,
            "2024-02-01",
            "2024-02-01",
            0,
            "1800",
            None,
            "4600",
            None,
            false,
        );
        let aid = asset_id(&r["asset"]);
        run_due(&db, "2024-02", "human:erik", false).unwrap();
        let d = dispose_asset(
            &db,
            aid,
            "2024-03-01",
            0,
            None,
            None,
            None,
            "agent:test",
            false,
        )
        .unwrap();
        // book value = 500 - 8.33 = 491.67 -> verlies 491.67 (debit 8100);
        // no cum-dep account -> the cum-dep leg lands on the asset account itself
        assert_eq!(d["result_cents"].as_i64(), Some(-49167));
        assert_eq!(
            sums_by_code(&d["postings"]),
            vec![("1800".to_string(), -49167), ("8100".to_string(), 49167)]
        );
    }

    #[test]
    fn dispose_asset_rejects_double_disposal() {
        let db = chart_db();
        let r = add_asset(
            &db,
            "X",
            "2024-01-01",
            50000,
            "2024-02-01",
            "2024-02-01",
            0,
            "1800",
            None,
            "4600",
            None,
            false,
        );
        let aid = asset_id(&r["asset"]);
        dispose_asset(
            &db,
            aid,
            "2024-06-01",
            0,
            None,
            None,
            None,
            "human:erik",
            false,
        )
        .unwrap();
        let err = dispose_asset(
            &db,
            aid,
            "2024-07-01",
            0,
            None,
            None,
            None,
            "human:erik",
            false,
        )
        .unwrap_err();
        assert_eq!(err.code, "ALREADY_DISPOSED");
    }

    #[test]
    fn dispose_asset_dry_run_books_nothing() {
        let db = chart_db();
        let r = add_asset(
            &db,
            "X",
            "2024-01-01",
            50000,
            "2024-02-01",
            "2024-02-01",
            0,
            "1800",
            None,
            "4600",
            None,
            false,
        );
        let aid = asset_id(&r["asset"]);
        let d = dispose_asset(
            &db,
            aid,
            "2024-06-01",
            0,
            None,
            None,
            None,
            "human:erik",
            true,
        )
        .unwrap();
        assert_eq!(d["dryRun"].as_bool(), Some(true));
        assert_eq!(
            list_assets(&db, None).unwrap()[0]["status"].as_str(),
            Some("active")
        );
        assert_eq!(posted(&db).len(), 0);
    }

    #[test]
    fn register_reports_book_values_and_totals_as_of_a_date() {
        let db = chart_db();
        add_asset(
            &db,
            "Laptop",
            "2024-01-15",
            200000,
            "2024-02-01",
            "2024-02-01",
            0,
            "1800",
            None,
            "4600",
            None,
            false,
        );
        run_due(&db, "2024-04", "human:erik", false).unwrap(); // 3 runs of 33.33
        let reg = register(&db, Some("2024-04-30"), "human:erik").unwrap();
        assert_eq!(reg["assets"].as_array().unwrap().len(), 1);
        assert_eq!(reg["assets"][0]["total_cum_dep_cents"].as_i64(), Some(9999));
        assert_eq!(
            reg["assets"][0]["book_value_cents"].as_i64(),
            Some(200000 - 9999)
        );
        assert_eq!(
            reg["assets"][0]["next_run_period"].as_str(),
            Some("2024-05")
        );
        assert_eq!(reg["totals"]["purchase_price_cents"].as_i64(), Some(200000));
    }

    #[test]
    fn register_surfaces_disposal_dates_and_proceeds() {
        let db = chart_db();
        let r = add_asset(
            &db,
            "X",
            "2024-01-01",
            50000,
            "2024-02-01",
            "2024-02-01",
            0,
            "1800",
            None,
            "4600",
            None,
            false,
        );
        let aid = asset_id(&r["asset"]);
        dispose_asset(
            &db,
            aid,
            "2024-06-01",
            10000,
            None,
            None,
            None,
            "human:erik",
            false,
        )
        .unwrap();
        let reg = register(&db, Some("2024-06-30"), "human:erik").unwrap();
        let a = &reg["assets"][0];
        assert_eq!(a["status"].as_str(), Some("disposed"));
        assert_eq!(a["disposed_date"].as_str(), Some("2024-06-01"));
        assert_eq!(a["disposed_proceeds_cents"].as_i64(), Some(10000));
    }

    #[test]
    fn trial_balance_stays_balanced_through_the_whole_lifecycle() {
        let db = chart_db();
        let e = crate::entries::create_entry(
            &db,
            crate::entries::CreateEntry {
                date: "2024-01-15",
                description: "Aankoop",
                postings: vec![spec("1800", 200000), spec("1100", -200000)],
                source: "manual",
                source_ref: None,
                actor: "agent:test",
            },
        )
        .unwrap();
        crate::entries::post_entry(&db, e.id, "agent:test").unwrap();
        let r = add_asset(
            &db,
            "Laptop",
            "2024-01-15",
            200000,
            "2024-02-01",
            "2024-02-01",
            0,
            "1800",
            None,
            "4600",
            None,
            false,
        );
        let aid = asset_id(&r["asset"]);
        run_due(&db, "2024-12", "human:erik", false).unwrap();
        dispose_asset(
            &db,
            aid,
            "2025-01-02",
            90000,
            None,
            None,
            None,
            "human:erik",
            false,
        )
        .unwrap();
        let (d, c): (i64, i64) = db
            .query_row(
                "SELECT COALESCE(SUM(CASE WHEN p.amount_cents > 0 THEN p.amount_cents ELSE 0 END),0),
                        COALESCE(SUM(CASE WHEN p.amount_cents < 0 THEN -p.amount_cents ELSE 0 END),0)
                 FROM postings p JOIN journal_entries e ON e.id = p.entry_id AND e.state = 'posted'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(d, c);
    }

    #[test]
    fn dispose_asset_entry_and_status_are_atomic_on_rollback() {
        let db = chart_db();
        let r = add_asset(
            &db,
            "Atomic",
            "2024-01-01",
            50000,
            "2024-02-01",
            "2024-02-01",
            0,
            "1800",
            None,
            "4600",
            None,
            false,
        );
        let aid = asset_id(&r["asset"]);
        let before = posted(&db).len();
        // force the UPDATE assets step (inside the disposal transaction) to fail
        db.execute(
            "CREATE TRIGGER IF NOT EXISTS trg_fail_disposal BEFORE UPDATE OF status ON assets \
             WHEN NEW.status = 'disposed' AND OLD.name = 'Atomic' \
             BEGIN SELECT RAISE(ABORT, 'boom'); END",
            [],
        )
        .unwrap();
        let err = dispose_asset(
            &db,
            aid,
            "2024-06-01",
            0,
            None,
            None,
            None,
            "human:erik",
            false,
        )
        .unwrap_err();
        assert!(err.message.contains("boom"), "got: {}", err.message);
        db.execute("DROP TRIGGER trg_fail_disposal", []).unwrap();
        // nothing persisted: no new posted entry, asset still active
        assert_eq!(posted(&db).len(), before);
        assert_eq!(
            list_assets(&db, None).unwrap()[0]["status"].as_str(),
            Some("active")
        );
    }
}
