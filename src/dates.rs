// bukio-cli — agent-first double-entry bookkeeping for SMEs.
// Copyright (c) 2026 Erik van Kempen.
// SPDX-License-Identifier: Apache-2.0
//
// Date helpers (mirrors src/core/dates.js) + IBAN (mirrors src/core/iban.js).

use crate::money::{BukioError, Result};

/// True when s is a real calendar date in yyyy-mm-dd form (no overflow).
pub fn is_valid_date(s: &str) -> bool {
    let Some((y, m, d)) = parse_parts(s) else {
        return false;
    };
    calendar_ok(y, m, d)
}

/// Throw INVALID_DATE unless date is a valid yyyy-mm-dd calendar date.
pub fn validate_date(date: &str) -> Result<()> {
    validate_labeled(date, "date")
}

pub fn validate_labeled(date: &str, label: &str) -> Result<()> {
    let bad_shape = || {
        BukioError::new(
            "INVALID_DATE",
            format!("{label} '{date}' must be yyyy-mm-dd"),
        )
    };
    let Some((y, m, d)) = parse_parts(date) else {
        return Err(bad_shape());
    };
    if !calendar_ok(y, m, d) {
        return Err(BukioError::new(
            "INVALID_DATE",
            format!("{label} '{date}' is not a valid calendar date"),
        ));
    }
    Ok(())
}

fn parse_parts(s: &str) -> Option<(i32, u32, u32)> {
    let mut it = s.split('-');
    let y: i32 = it.next()?.parse().ok()?;
    let m: u32 = it.next()?.parse().ok()?;
    let d: u32 = it.next()?.parse().ok()?;
    if it.next().is_some() || s.len() != 10 {
        return None;
    }
    // strict shape: no leading '+', month/day 2-digit (parse above accepts "1")
    if !s[5..7].bytes().all(|b| b.is_ascii_digit()) || !s[8..10].bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    Some((y, m, d))
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
        _ => 0,
    }
}

fn calendar_ok(y: i32, m: u32, d: u32) -> bool {
    (1..=12).contains(&m) && d >= 1 && d <= days_in_month(y, m)
}

/// Today in UTC as yyyy-mm-dd.
pub fn today_iso() -> String {
    use chrono::Utc;
    Utc::now().format("%Y-%m-%d").to_string()
}

/// Add n months; day clamped to the target month's last day.
pub fn add_months(date_str: &str, n: i32) -> String {
    let Some((y, m, d)) = parse_parts(date_str) else {
        return date_str.to_string();
    };
    let total = y * 12 + (m as i32 - 1) + n;
    let yy = total.div_euclid(12);
    let mm = (total.rem_euclid(12) + 1) as u32;
    let last = days_in_month(yy, mm);
    let dd = d.min(last);
    format!("{yy:04}-{mm:02}-{dd:02}")
}

// --- IBAN (mirrors src/core/iban.js) ---------------------------------------

pub fn normalize_iban(iban: &str) -> String {
    iban.chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .collect::<String>()
        .to_uppercase()
}

pub fn is_valid_iban(iban: &str) -> bool {
    let s = normalize_iban(iban);
    let b = s.as_bytes();
    if b.len() < 15 || b.len() > 34 {
        return false;
    }
    if !b[0..2].iter().all(|c| c.is_ascii_uppercase())
        || !b[2..4].iter().all(|c| c.is_ascii_digit())
    {
        return false;
    }
    if !b[4..].iter().all(|c| c.is_ascii_alphanumeric()) {
        return false;
    }
    // mod-97 via running remainder (no bignum needed)
    let rearranged: Vec<u8> = [&b[4..], &b[0..4]].concat();
    let mut rem: u32 = 0;
    for &ch in &rearranged {
        let digits: Vec<u8> = if ch.is_ascii_digit() {
            vec![ch]
        } else {
            // A=10..Z=35 -> two decimal digits (as ASCII)
            let v = ch - b'A' + 10;
            vec![v / 10 + b'0', v % 10 + b'0']
        };
        for &dgt in &digits {
            rem = (rem * 10 + (dgt - b'0') as u32) % 97;
        }
    }
    rem == 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_validation_matches_js() {
        assert!(is_valid_date("2026-02-28"));
        assert!(is_valid_date("2024-02-29"));
        assert!(!is_valid_date("2026-02-30")); // overflow
        assert!(!is_valid_date("2026-13-01"));
        assert!(!is_valid_date("2026-1-1"));
        assert!(!is_valid_date("garbage"));
        assert!(validate_date("2026-09-08").is_ok());
        assert_eq!(
            validate_date("2026-02-30").unwrap_err().code,
            "INVALID_DATE"
        );
    }

    #[test]
    fn month_arithmetic_clamps() {
        assert_eq!(add_months("2026-01-31", 1), "2026-02-28");
        assert_eq!(add_months("2024-01-31", 1), "2024-02-29"); // leap
        assert_eq!(add_months("2026-12-15", -1), "2026-11-15");
        assert_eq!(add_months("2026-06-30", 6), "2026-12-30");
    }

    #[test]
    fn iban_checks() {
        assert!(is_valid_iban("NL91ABNA0417164300"));
        assert!(is_valid_iban(" GB33 BUKB 2020 1555 5555 55 ".trim()));
        assert!(!is_valid_iban("NL91ABNA0417164299"));
        assert!(!is_valid_iban("XX91ABNA0417164300"));
        assert!(!is_valid_iban("NL91ABNA"));
    }
}
