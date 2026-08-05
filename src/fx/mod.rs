//! FX translation (Phase 5) — vreemde valuta purchase invoices.
//! Rates stored per currency per date as rate_x10000 (1 EUR = N units of
//! foreign currency, scaled by 10000). Conversion is integer math,
//! round-half-up: eur_cents = round(fx_cents * 10000 / rate_x10000).
//! Missing rates fall back to the ECB reference rates (source='ECB');
//! disable the network fallback with BUKIO_FX_NO_FETCH=1.
//! Port of the Node `src/fx/index.js` + `src/fx/ecb.js`.

use crate::error::{AppError, Result};
use regex::Regex;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::io::Read;
use std::sync::OnceLock;

const ISO4217: &str = r"^[A-Z]{3}$";

fn iso4217_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(ISO4217).unwrap())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct FxRateRow {
    pub currency: String,
    pub date: String,
    pub rate: String,
    pub rate_x10000: i64,
    pub source: String,
    pub created_by: String,
}

/// Validate + normalise a rate: '1.0875' -> 10875 (x10000, 4 decimals max).
pub fn parse_rate(rate: &str) -> Result<i64> {
    let re = Regex::new(r"^(\d+)(?:\.(\d{1,4}))?$").unwrap();
    let caps = re.captures(rate).ok_or_else(|| {
        AppError::new(
            "INVALID_RATE",
            format!("rate '{rate}' must be a positive number (e.g. 1.0875)"),
        )
    })?;
    let whole: i64 = caps[1].parse().map_err(|_| {
        AppError::new("INVALID_RATE", format!("rate '{rate}' must be a positive number (e.g. 1.0875)"))
    })?;
    let frac = caps.get(2).map(|m| m.as_str()).unwrap_or("");
    let frac_padded = format!("{:<4}", format!("{frac:0<4}"));
    let frac_i: i64 = if frac.is_empty() { 0 } else { frac_padded[..4].parse().unwrap_or(0) };
    let rate_x10000 = whole * 10000 + frac_i;
    if rate_x10000 <= 0 || rate_x10000 > 100_000_000 {
        return Err(AppError::new("INVALID_RATE", format!("rate '{rate}' out of range")));
    }
    Ok(rate_x10000)
}

/// Convert a foreign-currency amount (integer cents) to EUR cents.
pub fn convert_fx(fx_cents: i64, rate_x10000: i64) -> i64 {
    (fx_cents * 10000 + rate_x10000 / 2) / rate_x10000
}

pub fn set_fx_rate(
    conn: &Connection,
    currency: &str,
    date: &str,
    rate: Option<&str>,
    source: &str,
    actor: &str,
) -> Result<FxRateRow> {
    if !iso4217_re().is_match(currency) {
        return Err(AppError::new(
            "INVALID_CURRENCY",
            format!("currency '{currency}' must be ISO 4217 (3 letters)"),
        ));
    }
    if !Regex::new(r"^\d{4}-\d{2}-\d{2}$").unwrap().is_match(date) {
        return Err(AppError::new(
            "INVALID_DATE",
            format!("date '{date}' must be YYYY-MM-DD"),
        ));
    }
    let rate_x10000 = match rate {
        Some(r) => parse_rate(r)?,
        None => {
            return Err(AppError::new("INVALID_RATE", "rate is required"));
        }
    };
    conn.execute(
        "INSERT INTO fx_rates (currency, date, rate_x10000, source, created_by)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(currency, date) DO UPDATE SET rate_x10000 = excluded.rate_x10000, source = excluded.source",
        params![currency, date, rate_x10000, source, actor],
    )?;
    crate::audit::record(
        conn,
        actor,
        "fx.set",
        Some("fx set"),
        Some(&serde_json::json!({ "currency": currency, "date": date, "rate": format!("{:.4}", rate_x10000 as f64 / 10000.0) })),
        "ok",
        &[],
    )?;
    Ok(FxRateRow {
        currency: currency.to_string(),
        date: date.to_string(),
        rate: format!("{:.4}", rate_x10000 as f64 / 10000.0),
        rate_x10000,
        source: source.to_string(),
        created_by: actor.to_string(),
    })
}

/// Rate lookup for a booking date: exact date first, else the latest rate
/// on or before that date. Returns rate_x10000 or null.
pub fn get_fx_rate(conn: &Connection, currency: &str, date: &str) -> Result<Option<i64>> {
    let exact: Option<i64> = conn
        .query_row(
            "SELECT rate_x10000 FROM fx_rates WHERE currency = ?1 AND date = ?2",
            params![currency, date],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(r) = exact {
        return Ok(Some(r));
    }
    let latest: Option<i64> = conn
        .query_row(
            "SELECT rate_x10000 FROM fx_rates WHERE currency = ?1 AND date <= ?2 ORDER BY date DESC LIMIT 1",
            params![currency, date],
            |r| r.get(0),
        )
        .optional()?;
    Ok(latest)
}

pub fn list_fx_rates(conn: &Connection, currency: Option<&str>, limit: i64) -> Result<Vec<FxRateRow>> {
    let rows = match currency {
        Some(c) => {
            let mut stmt = conn.prepare("SELECT * FROM fx_rates WHERE currency = ?1 ORDER BY date DESC LIMIT ?2")?;
            let x = stmt.query_map(params![c, limit], fx_row_from_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
            x
        }
        None => {
            let mut stmt = conn.prepare("SELECT * FROM fx_rates ORDER BY date DESC LIMIT ?1")?;
            let x = stmt.query_map(params![limit], fx_row_from_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
            x
        }
    };
    Ok(rows)
}

fn fx_row_from_row(row: &rusqlite::Row) -> rusqlite::Result<FxRateRow> {
    let rate_x10000: i64 = row.get(2)?;
    Ok(FxRateRow {
        currency: row.get(0)?,
        date: row.get(1)?,
        rate: format!("{:.4}", rate_x10000 as f64 / 10000.0),
        rate_x10000,
        source: row.get(3)?,
        created_by: row.get(4)?,
    })
}

fn no_fetch_env() -> bool {
    std::env::var("BUKIO_FX_NO_FETCH").as_deref() == Ok("1")
}

/// ECB reference-rate fetch: last observation on or before `date` in a 10-day
/// window. Returns (observation_date, rate_x10000) or None when the currency
/// is not in the ECB reference set. Throws ECB_FETCH_FAILED on network errors.
fn fetch_ecb_rate(currency: &str, date: &str) -> Result<Option<(String, i64)>> {
    let d = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .map_err(|_| AppError::new("INVALID_DATE", format!("date '{date}' must be YYYY-MM-DD")))?;
    let from = (d - chrono::Duration::days(10)).format("%Y-%m-%d").to_string();
    let url = format!(
        "https://data-api.ecb.europa.eu/service/data/EXR/D.{currency}.EUR.SP00.A?startPeriod={from}&endPeriod={date}"
    );
    let body = match ureq::get(&url).call() {
        Ok(mut resp) => resp.body_mut().read_to_string().map_err(|e| {
            AppError::new("ECB_FETCH_FAILED", format!("ECB unreachable for {currency}: {e}"))
        })?,
        Err(ureq::Error::StatusCode(404)) => return Ok(None),
        Err(e) => {
            return Err(AppError::new(
                "ECB_FETCH_FAILED",
                format!("ECB unreachable for {currency}: {e}"),
            ));
        }
    };
    let obs = parse_sdmx_observations(&body);
    if obs.is_empty() {
        return Ok(None);
    }
    let best = obs
        .iter()
        .filter(|(od, _)| od.as_str() <= date)
        .last()
        .or_else(|| obs.last());
    match best {
        Some((od, rate)) => Ok(Some((od.clone(), (rate * 10000.0).round() as i64))),
        None => Ok(None),
    }
}

/// Parse the SDMX-ML GenericData response into [(date, rate)] ascending.
fn parse_sdmx_observations(xml: &str) -> Vec<(String, f64)> {
    let date_re = Regex::new(r#"ObsDimension\s+value="([^"]+)""#).unwrap();
    let value_re = Regex::new(r#"ObsValue\s+value="([^"]+)""#).unwrap();
    let dates: Vec<String> = date_re.captures_iter(xml).map(|c| c[1].to_string()).collect();
    let values: Vec<f64> = value_re
        .captures_iter(xml)
        .filter_map(|c| c[1].parse::<f64>().ok())
        .filter(|v| v.is_finite() && *v > 0.0)
        .collect();
    let mut out: Vec<(String, f64)> = dates.into_iter().zip(values.into_iter()).collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Resolve the rate for a booking: explicit --rate wins, then the stored rate
/// (exact date, else latest on/before), then — when allowed — a live ECB fetch
/// (stored as source='ECB' for reuse). Returns None when currency is None.
/// Throws FX_RATE_NOT_FOUND / ECB_RATE_NOT_AVAILABLE when nothing is available.
pub fn resolve_rate(
    conn: &Connection,
    currency: Option<&str>,
    rate: Option<&str>,
    date: &str,
    actor: &str,
) -> Result<Option<i64>> {
    let Some(currency) = currency else { return Ok(None) };
    if let Some(r) = rate {
        return Ok(Some(parse_rate(r)?));
    }
    if let Some(stored) = get_fx_rate(conn, currency, date)? {
        return Ok(Some(stored));
    }
    if no_fetch_env() {
        return Err(AppError::new(
            "FX_RATE_NOT_FOUND",
            format!("no FX rate for {currency} on/before {date} — set one with 'bukio fx set', pass --rate, or allow the ECB fetch"),
        ));
    }
    match fetch_ecb_rate(currency, date)? {
        Some((obs_date, rate_x10000)) => {
            set_fx_rate(conn, currency, &obs_date, Some(&format!("{rate_x10000}")), "ECB", actor)?;
            Ok(Some(rate_x10000))
        }
        None => Err(AppError::new(
            "ECB_RATE_NOT_AVAILABLE",
            format!("no ECB reference rate for {currency} on/before {date} (not in the ECB set, or before 1999)"),
        )),
    }
}

/// A posting spec before FX conversion: foreign-currency amounts.
#[derive(Debug, Clone)]
pub struct RawSpec {
    pub code: String,
    pub amount_cents: i64,
    pub vat_code: Option<String>,
    pub fx_currency: Option<String>,
    pub fx_amount_cents: Option<i64>,
}

/// Convert a posting spec list from a foreign currency into EUR, attaching the
/// original amounts for the audit trail. The vat leg computed later is EUR.
pub fn to_eur_postings(specs: &[RawSpec], currency: &str, rate_x10000: i64) -> Vec<RawSpec> {
    specs
        .iter()
        .map(|p| RawSpec {
            code: p.code.clone(),
            amount_cents: convert_fx(p.amount_cents, rate_x10000),
            vat_code: p.vat_code.clone(),
            fx_currency: Some(currency.to_string()),
            fx_amount_cents: Some(p.amount_cents),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::db::open_in_memory;

    #[test]
    fn parse_rate_valid_and_invalid() {
        assert_eq!(parse_rate("1.0875").unwrap(), 10875);
        assert_eq!(parse_rate("1").unwrap(), 10000);
        assert_eq!(parse_rate("1.2").unwrap(), 12000);
        assert_eq!(parse_rate("0.0001").unwrap(), 1);
        assert_eq!(parse_rate("9999.0").unwrap(), 99_990_000);
        assert!(parse_rate("10875.0").is_err()); // > 100000000 x10000 limit
        for bad in ["", "abc", "1,0875", "-1", "1.12345", "0", "0.0", "10001.0"] {
            assert!(parse_rate(bad).is_err(), "should reject {bad:?}");
        }
    }

    #[test]
    fn convert_rounds_half_up() {
        assert_eq!(convert_fx(10875, 10875), 10000);
        assert_eq!(convert_fx(10000, 10875), 9195); // 10000*10000/10875 = 9195.4 -> 9195
        assert_eq!(convert_fx(544, 10875), 500); // 544*10000/10875 = 500.2 -> 500
    }

    #[test]
    fn set_and_get_rate() {
        let conn = open_in_memory().unwrap();
        set_fx_rate(&conn, "USD", "2026-07-01", Some("1.0875"), "manual", "human").unwrap();
        assert_eq!(get_fx_rate(&conn, "USD", "2026-07-01").unwrap(), Some(10875));
        // latest on/before fallback
        assert_eq!(get_fx_rate(&conn, "USD", "2026-07-15").unwrap(), Some(10875));
        assert_eq!(get_fx_rate(&conn, "USD", "2026-06-30").unwrap(), None);
        // upsert
        set_fx_rate(&conn, "USD", "2026-07-01", Some("1.10"), "manual", "human").unwrap();
        assert_eq!(get_fx_rate(&conn, "USD", "2026-07-01").unwrap(), Some(11000));
    }

    #[test]
    fn resolve_prefers_explicit_rate() {
        let conn = open_in_memory().unwrap();
        assert_eq!(resolve_rate(&conn, Some("USD"), Some("1.1"), "2026-07-01", "human").unwrap(), Some(11000));
        assert_eq!(resolve_rate(&conn, None, None, "2026-07-01", "human").unwrap(), None);
    }

    #[test]
    fn to_eur_attaches_fx_fields() {
        let specs = vec![RawSpec { code: "4300".into(), amount_cents: 100_00, vat_code: Some("21".into()), fx_currency: None, fx_amount_cents: None }];
        let converted = to_eur_postings(&specs, "USD", 10875);
        assert_eq!(converted[0].amount_cents, 9195);
        assert_eq!(converted[0].fx_currency.as_deref(), Some("USD"));
        assert_eq!(converted[0].fx_amount_cents, Some(100_00));
    }

    #[test]
    fn parses_sdmx_xml() {
        let xml = r#"<GenericData xmlns="http://www.sdmx.org/resources/sdmxml/schemas/v2_1/data/generic">
          <DataSet><Series><Obs><ObsDimension value="2026-07-03"/><ObsValue value="1.0822"/></Obs>
          <Obs><ObsDimension value="2026-07-06"/><ObsValue value="1.0851"/></Obs></Series></DataSet></GenericData>"#;
        let obs = parse_sdmx_observations(xml);
        assert_eq!(obs.len(), 2);
        assert_eq!(obs[0].0, "2026-07-03");
        assert_eq!(obs[1].1, 1.0851);
    }
}
