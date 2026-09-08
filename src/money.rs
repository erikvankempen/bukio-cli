// bukio-cli — agent-first double-entry bookkeeping for SMEs.
// Copyright (c) 2026 Erik van Kempen.
// SPDX-License-Identifier: Apache-2.0
//
// Money — integer cents everywhere (mirrors src/core/money.js).

/// Typed domain error: every module throws codes, the CLI maps them to
/// { ok:false, error:{code,message} } exactly like util.js fail().
#[derive(Debug, Clone)]
pub struct BukioError {
    pub code: &'static str,
    pub message: String,
}

impl BukioError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }
}

impl std::fmt::Display for BukioError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for BukioError {}

pub type Result<T> = std::result::Result<T, BukioError>;

/// Parse a decimal amount string to integer cents — strict international
/// format, no thousands separators (mirrors parseAmount).
pub fn parse_amount(input: &str) -> Result<i64> {
    let s = input.trim();
    let invalid = || BukioError::new(
        "INVALID_AMOUNT",
        format!("invalid amount '{input}' — use e.g. 1234.56 (max 2 decimals, no thousands separators)"),
    );
    let body = if let Some(rest) = s.strip_prefix('-') { rest } else { s };
    if body.is_empty() {
        return Err(invalid());
    }
    if !body.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
        return Err(invalid());
    }
    if body.matches('.').count() > 1 {
        return Err(invalid());
    }
    let (whole, frac) = match body.split_once('.') {
        Some((w, f)) => (w, f),
        None => (body, ""),
    };
    if whole.is_empty() || !whole.bytes().all(|b| b.is_ascii_digit()) {
        return Err(invalid());
    }
    if !frac.is_empty() && !frac.bytes().all(|b| b.is_ascii_digit()) {
        return Err(invalid());
    }
    if frac.len() > 2 {
        return Err(invalid());
    }
    if body.contains('.') && frac.is_empty() {
        return Err(invalid()); // "1." — JS regex requires digits after the dot
    }
    let whole_val: i128 = whole.parse().map_err(|_| invalid())?;
    // frac pads to 2 digits exactly like JS padEnd(2,'0'): "1" -> 10, "05" -> 5
    let frac_val: i128 = if frac.is_empty() {
        0
    } else {
        let f: i128 = frac.parse().map_err(|_| invalid())?;
        if frac.len() == 1 {
            f * 10
        } else {
            f
        }
    };
    let cents = whole_val * 100 + frac_val;
    if cents > i64::MAX as i128 {
        return Err(invalid());
    }
    let cents = cents as i64;
    let negative = s.starts_with('-');
    Ok(if negative && cents != 0 { -cents } else { cents })
}

/// Format integer cents as "1234.56" (mirrors formatAmount).
pub fn format_amount(cents: i64) -> String {
    let sign = if cents < 0 { "-" } else { "" };
    let abs = cents.unsigned_abs();
    format!("{sign}{}.{:02}", abs / 100, abs % 100)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_js_suite_cases() {
        assert_eq!(parse_amount("1234.56").unwrap(), 123456);
        assert_eq!(parse_amount("0.1").unwrap(), 10);
        assert_eq!(parse_amount("-0.10").unwrap(), -10);
        assert_eq!(parse_amount("-0").unwrap(), 0);
        assert_eq!(parse_amount("100").unwrap(), 10000);
        assert_eq!(parse_amount(" 42.5 ").unwrap(), 4250);
    }

    #[test]
    fn rejects_bad_formats() {
        for bad in ["", "1,5", "1.234", "12.345", "1.2.3", "abc", "1 000", "--1", "1.", ".5"] {
            assert!(parse_amount(bad).is_err(), "should reject {bad:?}");
        }
    }

    #[test]
    fn formats_like_js() {
        assert_eq!(format_amount(123456), "1234.56");
        assert_eq!(format_amount(-10), "-0.10");
        assert_eq!(format_amount(0), "0.00");
        assert_eq!(format_amount(-123456), "-1234.56");
    }
}
