// bukio-cli — agent-first double-entry bookkeeping for SMEs.
// Copyright (c) 2026 Erik van Kempen.
// SPDX-License-Identifier: Apache-2.0
//
// FX translation (mirrors src/fx/index.js + src/fx/ecb.js):
// EUR-conversion rates stored per currency per date; ECB reference-rate fetch.

use crate::accounts::resolve_profile;
use crate::actor::now_iso;
use crate::audit::{record, RecordArgs};
use crate::dates::today_iso;
use crate::money::{format_amount, BukioError, Result};
use rusqlite::Connection;
use serde_json::{json, Value};

const ECB_BASE: &str = "https://data-api.ecb.europa.eu/service/data/EXR";
const WINDOW_DAYS: i64 = 10;

fn fx_error(code: &'static str, msg: impl Into<String>) -> BukioError {
    BukioError::new(code, msg.into())
}

/// Validate + normalise a rate: "1.0875" -> 10875 (x10000, 4 decimals max).
pub fn parse_rate(rate: &str) -> Result<i64> {
    let (whole_str, frac_str) = match rate.split_once('.') {
        Some((w, f)) => (w, f),
        None => (rate, ""),
    };
    let whole: i64 = whole_str.parse().map_err(|_| {
        fx_error(
            "INVALID_RATE",
            format!("rate '{rate}' must be a positive number (e.g. 1.0875)"),
        )
    })?;
    if frac_str.len() > 4 {
        return Err(fx_error(
            "INVALID_RATE",
            "rate must have at most 4 decimal places",
        ));
    }
    let frac_padded = format!("{:0<4}", frac_str);
    let rate_x10000: i64 = whole * 10000
        + frac_padded
            .parse::<i64>()
            .map_err(|_| fx_error("INVALID_RATE", "frac parse error"))?;
    if rate_x10000 <= 0 || rate_x10000 > 100_000_000 {
        return Err(fx_error("INVALID_RATE", "rate out of range"));
    }
    Ok(rate_x10000)
}

pub fn format_rate(rate_x10000: i64) -> String {
    format!("{:.4}", rate_x10000 as f64 / 10000.0)
}

/// Convert foreign-currency cents to EUR cents.
pub fn convert_fx(fx_cents: i64, rate_x10000: i64) -> Result<i64> {
    if rate_x10000 <= 0 {
        return Err(fx_error("INVALID_RATE", "rate must be positive"));
    }
    // round-half-away-from-zero: same as JS Math.round(fx_cents * 10000 / rate)
    Ok(((fx_cents as f64 * 10000.0) / rate_x10000 as f64).round() as i64)
}

/// Set or update a stored rate.
pub fn set_fx_rate(
    db: &Connection,
    currency: &str,
    date: &str,
    rate: &str,
    source: &str,
    actor: &str,
    dry_run: bool,
) -> Result<Value> {
    if !(currency.len() == 3 && currency.bytes().all(|b| b.is_ascii_uppercase())) {
        return Err(fx_error(
            "INVALID_CURRENCY",
            format!("currency '{currency}' must be ISO 4217 (3 letters)"),
        ));
    }
    if !(date.len() == 10 && date.as_bytes()[4] == b'-' && date.as_bytes()[7] == b'-') {
        return Err(fx_error(
            "INVALID_DATE",
            format!("date '{date}' must be YYYY-MM-DD"),
        ));
    }
    // calendar validity check (chrono rejects Feb 30 etc.)
    chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").map_err(|_| {
        fx_error(
            "INVALID_DATE",
            format!("date '{date}' is not a valid calendar date"),
        )
    })?;

    let rate_x10000 = parse_rate(rate)?;
    if dry_run {
        return Ok(json!({
            "action": "fx.set", "currency": currency, "date": date,
            "rate": format_rate(rate_x10000), "rate_x10000": rate_x10000, "dryRun": true,
        }));
    }
    db.execute(
        "INSERT INTO fx_rates (currency, date, rate_x10000, source, created_by)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(currency, date) DO UPDATE SET rate_x10000 = excluded.rate_x10000, source = excluded.source",
        rusqlite::params![currency, date, rate_x10000, source, actor],
    )
    .map_err(sql_err)?;
    record(
        db,
        RecordArgs {
            actor,
            action: "fx.set",
            command: Some("fx set"),
            args: Some(
                json!({ "currency": currency, "date": date, "rate": format_rate(rate_x10000) }),
            ),
            outcome: "ok",
            entry_ids: vec![],
        },
    )?;
    Ok(json!({
        "currency": currency, "date": date,
        "rate": format_rate(rate_x10000), "rate_x10000": rate_x10000,
    }))
}

/// Rate lookup for a booking date: exact date first, else latest on/before.
pub fn get_fx_rate(db: &Connection, currency: &str, date: &str) -> Result<Option<i64>> {
    // try exact
    let exact: Option<i64> = db
        .query_row(
            "SELECT rate_x10000 FROM fx_rates WHERE currency = ?1 AND date = ?2",
            rusqlite::params![currency, date],
            |r| r.get(0),
        )
        .ok();
    if exact.is_some() {
        return Ok(exact);
    }
    // latest on or before
    let latest: Option<i64> = db
        .query_row(
            "SELECT rate_x10000 FROM fx_rates WHERE currency = ?1 AND date <= ?2 ORDER BY date DESC LIMIT 1",
            rusqlite::params![currency, date],
            |r| r.get(0),
        )
        .ok();
    Ok(latest)
}

/// List stored rates, newest first.
pub fn list_fx_rates(db: &Connection, currency: Option<&str>, limit: i64) -> Result<Vec<Value>> {
    let (sql, params): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = match currency {
        Some(c) => (
            "SELECT currency, date, rate_x10000, source, created_by FROM fx_rates WHERE currency = ?1 ORDER BY date DESC LIMIT ?2".into(),
            vec![Box::new(c.to_string()), Box::new(limit)],
        ),
        None => (
            "SELECT currency, date, rate_x10000, source, created_by FROM fx_rates ORDER BY date DESC LIMIT ?1".into(),
            vec![Box::new(limit)],
        ),
    };
    let mut stmt = db.prepare(&sql).map_err(sql_err)?;
    let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
    let rows = stmt
        .query_map(param_refs.as_slice(), |r| {
            Ok(json!({
                "currency": r.get::<_, String>(0)?,
                "date": r.get::<_, String>(1)?,
                "rate": format_rate(r.get::<_, i64>(2)?),
                "rate_x10000": r.get::<_, i64>(2)?,
                "source": r.get::<_, String>(3)?,
                "created_by": r.get::<_, String>(4)?,
            }))
        })
        .map_err(sql_err)?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

/// Fetch the ECB reference rate via HTTP (blocking). Returns (date, rate_x10000) or None.
/// Test/ops seam mirroring the JS `setEcbFetcher`: the ECB call is the only
/// network access in the port, and the JS suite injects a stub for it.
/// `Ok(None)` means "no data for this currency" (the JS's non-ok/404 branch).
pub type EcbFetcher = fn(&str) -> std::result::Result<Option<String>, String>;

static ECB_FETCHER: std::sync::Mutex<Option<EcbFetcher>> = std::sync::Mutex::new(None);

pub fn set_ecb_fetcher(f: EcbFetcher) {
    *ECB_FETCHER.lock().unwrap() = Some(f);
}

pub fn clear_ecb_fetcher() {
    *ECB_FETCHER.lock().unwrap() = None;
}

/// Parse SDMX-ML observations out of an ECB data response, oldest first.
pub fn parse_sdmx_observations(xml: &str, _currency: &str) -> Vec<(String, f64)> {
    let mut dim_dates: Vec<String> = Vec::new();
    let mut obs_values: Vec<f64> = Vec::new();
    for cap in xml.match_indices("ObsDimension") {
        let rest = &xml[cap.0..];
        if let Some(start) = rest.find("value=\"") {
            let val_start = start + 7;
            if let Some(end) = rest[val_start..].find('"') {
                dim_dates.push(rest[val_start..val_start + end].to_string());
            }
        }
    }
    for cap in xml.match_indices("ObsValue") {
        let rest = &xml[cap.0..];
        if let Some(start) = rest.find("value=\"") {
            let val_start = start + 7;
            if let Some(end) = rest[val_start..].find('"') {
                if let Ok(v) = rest[val_start..val_start + end].parse::<f64>() {
                    if v > 0.0 {
                        obs_values.push(v);
                    }
                }
            }
        }
    }
    let mut observations: Vec<(String, f64)> = dim_dates
        .iter()
        .zip(obs_values.iter())
        .map(|(d, v)| (d.clone(), *v))
        .collect();
    observations.sort_by(|a, b| a.0.cmp(&b.0));
    observations
}

pub fn fetch_ecb_rate(currency: &str, date: &str) -> Result<Option<(String, i64)>> {
    let from = {
        let d = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
            .map_err(|_| fx_error("INVALID_DATE", format!("date '{date}' is not valid")))?;
        (d - chrono::Duration::days(WINDOW_DAYS))
            .format("%Y-%m-%d")
            .to_string()
    };
    let url = format!(
        "{}/D.{currency}.EUR.SP00.A?startPeriod={from}&endPeriod={date}",
        ECB_BASE
    );
    let injected = *ECB_FETCHER.lock().unwrap();
    let xml = match injected {
        Some(fetch) => match fetch(&url) {
            Ok(Some(body)) => body,
            // no data for this currency (the JS's non-ok/404 branch)
            Ok(None) => return Ok(None),
            Err(e) => {
                return Err(fx_error(
                    "ECB_FETCH_FAILED",
                    format!("ECB unreachable for {currency}: {e}"),
                ))
            }
        },
        None => {
            let body = ureq::get(&url).call().map_err(|e| {
                fx_error(
                    "ECB_FETCH_FAILED",
                    format!("ECB unreachable for {currency}: {e}"),
                )
            })?;
            body.into_body().read_to_string().map_err(|e| {
                fx_error(
                    "ECB_FETCH_FAILED",
                    format!("failed to read ECB response: {e}"),
                )
            })?
        }
    };

    let observations = parse_sdmx_observations(&xml, currency);
    if observations.is_empty() {
        return Ok(None);
    }
    // take the latest observation on or before the target date
    let best = observations
        .iter()
        .filter(|(d, _)| d.as_str() <= date)
        .last();
    match best {
        Some((d, rate)) => Ok(Some((d.clone(), (*rate * 10000.0).round() as i64))),
        None => Ok(None),
    }
}

/// Resolve the rate for a booking: explicit --rate wins, then stored, then ECB fetch.
pub fn resolve_rate(
    db: &Connection,
    currency: &str,
    rate: Option<&str>,
    date: &str,
    actor: &str,
    no_fetch: bool,
) -> Result<i64> {
    resolve_rate_opt(db, currency, rate, date, actor, no_fetch, false)
}

/// `dry_run` mirrors the JS resolveRate({dryRun}) / resolveMcpFx: a plan-only
/// call must NOT persist the fetched rate (nor write an fx.set audit row).
pub fn resolve_rate_opt(
    db: &Connection,
    currency: &str,
    rate: Option<&str>,
    date: &str,
    actor: &str,
    no_fetch: bool,
    dry_run: bool,
) -> Result<i64> {
    if currency.is_empty() {
        return Ok(0);
    }
    if let Some(r) = rate {
        return parse_rate(r);
    }
    if let Some(stored) = get_fx_rate(db, currency, date)? {
        return Ok(stored);
    }
    if no_fetch {
        return Err(fx_error("FX_RATE_NOT_FOUND", format!("no FX rate for {currency} on/before {date} — set one with 'bukio fx set', pass --rate, or allow the ECB fetch")));
    }
    let fetched = fetch_ecb_rate(currency, date)?;
    match fetched {
        None => Err(fx_error(
            "ECB_RATE_NOT_AVAILABLE",
            format!("no ECB reference rate for {currency} on/before {date}"),
        )),
        Some((obs_date, rate_x10000)) => {
            if dry_run {
                return Ok(rate_x10000);
            }
            // store for reuse (like the JS implementation)
            set_fx_rate(
                db,
                currency,
                &obs_date,
                // the ECB observation is rate x10000; set_fx_rate takes the
                // decimal string (passing the raw integer was rejected as
                // INVALID_RATE, so auto-fetch could never store a rate)
                &format_rate(rate_x10000),
                "ECB",
                actor,
                false,
            )?;
            Ok(rate_x10000)
        }
    }
}

fn sql_err(e: rusqlite::Error) -> BukioError {
    BukioError::new("DB_ERROR", e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_db;

    fn db() -> Connection {
        let db = open_db(":memory:").unwrap();
        db.execute("INSERT INTO company (name) VALUES ('FX Co')", [])
            .unwrap();
        db
    }

    #[test]
    fn parse_rate_exact() {
        assert_eq!(parse_rate("1.0875").unwrap(), 10875);
        assert_eq!(parse_rate("1").unwrap(), 10000);
        assert_eq!(parse_rate("2.5000").unwrap(), 25000);
        assert_eq!(parse_rate("1.1").unwrap(), 11000);
        assert!(parse_rate("abc").is_err());
        assert!(parse_rate("0").is_err());
        assert!(parse_rate("-1.0875").is_err());
    }

    #[test]
    fn convert_fx_basic() {
        assert_eq!(convert_fx(10875, 10875).unwrap(), 10000); // 1:1
        assert_eq!(convert_fx(10000, 10875).unwrap(), 9195); // USD
        assert_eq!(convert_fx(-10000, 10875).unwrap(), -9195); // refund
    }

    #[test]
    fn set_and_get_rate() {
        let db = db();
        set_fx_rate(
            &db,
            "USD",
            "2026-01-15",
            "1.0875",
            "manual",
            "human:erik",
            false,
        )
        .unwrap();
        let r = get_fx_rate(&db, "USD", "2026-01-15").unwrap();
        assert_eq!(r, Some(10875));
        // fallback to earlier rate
        let r2 = get_fx_rate(&db, "USD", "2026-01-20").unwrap();
        assert_eq!(r2, Some(10875));
        // not found
        let r3 = get_fx_rate(&db, "GBP", "2026-01-15").unwrap();
        assert_eq!(r3, None);
    }

    #[test]
    fn invalid_currency() {
        let db = db();
        assert!(set_fx_rate(
            &db,
            "usd",
            "2026-01-15",
            "1.0",
            "manual",
            "human:erik",
            false
        )
        .is_err());
    }
}
