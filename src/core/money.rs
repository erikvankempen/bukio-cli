//! Money handling — integer cents everywhere, no floats in financial code paths.

use crate::error::{AppError, Result};
use regex::Regex;
use std::sync::OnceLock;

fn amount_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^-?\d+(\.\d{1,2})?$").unwrap())
}

/// Parse a decimal amount string to integer cents.
/// Strict international format: optional '-', digits, optional single '.'
/// with max 2 decimals. Thousands separators are rejected on purpose
/// (agents output international format; ambiguity is the enemy).
pub fn parse_amount(input: &str) -> Result<i64> {
    let s = input.trim();
    if !amount_re().is_match(s) {
        return Err(AppError::new(
            "INVALID_AMOUNT",
            format!("invalid amount '{input}' — use e.g. 1234.56 (max 2 decimals, no thousands separators)"),
        ));
    }
    let negative = s.starts_with('-');
    let body = s.trim_start_matches('-');
    let (whole, frac) = match body.split_once('.') {
        Some((w, f)) => (w, f),
        None => (body, ""),
    };
    let whole_i: i64 = whole.parse().map_err(|_| AppError::new("INVALID_AMOUNT", format!("invalid amount '{input}'")))?;
    let frac_i: i64 = if frac.is_empty() {
        0
    } else {
        frac.parse().map_err(|_| AppError::new("INVALID_AMOUNT", format!("invalid amount '{input}'")))?
    };
    let frac_cents = if frac.len() == 1 { frac_i * 10 } else { frac_i };
    let cents = whole_i * 100 + frac_cents;
    Ok(if negative { -cents } else { cents })
}

/// Format integer cents as "1234.56" (always 2 decimals, '-' prefix for negative).
pub fn format_amount(cents: i64) -> String {
    let abs = cents.abs();
    let sign = if cents < 0 { "-" } else { "" };
    format!("{}{}.{:02}", sign, abs / 100, abs % 100)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_amount_valid() {
        assert_eq!(parse_amount("0").unwrap(), 0);
        assert_eq!(parse_amount("0.5").unwrap(), 50);
        assert_eq!(parse_amount("1234").unwrap(), 123400);
        assert_eq!(parse_amount("1234.56").unwrap(), 123456);
        assert_eq!(parse_amount("-12.34").unwrap(), -1234);
        assert_eq!(parse_amount(" 42.10 ").unwrap(), 4210);
        assert_eq!(parse_amount("1000000.01").unwrap(), 100000001);
    }

    #[test]
    fn parse_amount_rejects_invalid() {
        for bad in ["", "abc", "1.234,56", "1.234", "12.345", "1.2.3", ".5", "5.", "1,5", "--5", "1e3", "NaN", "Infinity"] {
            let err = parse_amount(bad).unwrap_err();
            assert_eq!(err.code(), "INVALID_AMOUNT", "should reject {bad:?}");
        }
    }

    #[test]
    fn parse_amount_rejects_more_than_2_decimals() {
        assert_eq!(parse_amount("1.234").unwrap_err().code(), "INVALID_AMOUNT");
        assert_eq!(parse_amount("0.001").unwrap_err().code(), "INVALID_AMOUNT");
    }

    #[test]
    fn format_amount_round_trips() {
        for cents in [0i64, 1, -1, 50, -50, 123456, -123456, 100000001] {
            assert_eq!(parse_amount(&format_amount(cents)).unwrap(), cents);
        }
    }

    #[test]
    fn format_amount_formatting() {
        assert_eq!(format_amount(0), "0.00");
        assert_eq!(format_amount(123456), "1234.56");
        assert_eq!(format_amount(-1234), "-12.34");
        assert_eq!(format_amount(5), "0.05");
    }
}
