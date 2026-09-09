// bukio-cli — agent-first double-entry bookkeeping for SMEs.
// Copyright (c) 2026 Erik van Kempen.
// SPDX-License-Identifier: Apache-2.0
//
// Bank CSV parser (mirrors src/bank/csv.js).

use crate::bank::BankTx;
use crate::money::BukioError;

/// Auto-detect the delimiter (semicolon, comma, tab) from the header line.
fn detect_delimiter(header: &str) -> char {
    if header.contains(';') {
        ';'
    } else if header.contains('\t') {
        '\t'
    } else {
        ','
    }
}

/// Split a CSV line respecting quotes (naive but sufficient for bank exports).
fn split_csv_line(line: &str, delimiter: char) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '"' if !in_quotes => {
                in_quotes = true;
            }
            '"' if in_quotes => {
                // check for escaped quote ""
                if i + 1 < chars.len() && chars[i + 1] == '"' {
                    current.push('"');
                    i += 1;
                } else {
                    in_quotes = false;
                }
            }
            c if c == delimiter && !in_quotes => {
                fields.push(current.trim().to_string());
                current = String::new();
            }
            c => current.push(c),
        }
        i += 1;
    }
    fields.push(current.trim().to_string());
    fields
}

/// Normalize a header name to a canonical key.
fn normalize_header(h: &str) -> String {
    h.trim().to_lowercase().replace([' ', '_', '-'], "")
}

/// Find the column index for a canonical key (exact, then contains).
fn find_column(header: &[String], key: &str) -> Option<usize> {
    header.iter().position(|h| h == key)
        .or_else(|| header.iter().position(|h| h.contains(key)))
}

/// Parse a bank CSV file into transactions.
pub fn parse_bank_csv(content: &str, _default_iban: &str) -> Result<Vec<BankTx>, BukioError> {
    let lines: Vec<&str> = content.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.len() < 2 {
        return Err(BukioError::new(
            "EMPTY_CSV",
            "bank CSV needs a header row and at least one transaction",
        ));
    }
    let delimiter = detect_delimiter(lines[0]);
    let header: Vec<String> = split_csv_line(lines[0], delimiter)
        .iter()
        .map(|h| normalize_header(h))
        .collect();

    // map canonical keys to column indices
    let date_idx = find_column(&header, "datum")
        .or_else(|| find_column(&header, "date"))
        .or_else(|| find_column(&header, "boekingsdatum"));
    let amount_idx = find_column(&header, "bedrag")
        .or_else(|| find_column(&header, "amount"))
        .or_else(|| find_column(&header, "creditdebit"))
        .or_else(|| find_column(&header, "value"));
    let counterparty_idx = find_column(&header, "naam")
        .or_else(|| find_column(&header, "counterparty"))
        .or_else(|| find_column(&header, "tegenrekening"))
        .or_else(|| find_column(&header, "from"));
    let description_idx = find_column(&header, "mededelingen")
        .or_else(|| find_column(&header, "omschrijving"))
        .or_else(|| find_column(&header, "description"));
    let af_bij_idx = find_column(&header, "afbij")
        .or_else(|| find_column(&header, "af/bij"));
    let iban_idx = find_column(&header, "rekening")
        .or_else(|| find_column(&header, "iban"))
        .or_else(|| find_column(&header, "tegenrekeningnummer"));

    let date_idx = date_idx.ok_or_else(|| {
        BukioError::new(
            "INVALID_CSV_HEADER",
            "bank CSV needs a date column (datum/date)",
        )
    })?;
    let amount_idx = amount_idx.ok_or_else(|| {
        BukioError::new(
            "INVALID_CSV_HEADER",
            "bank CSV needs an amount column (bedrag/amount)",
        )
    })?;

    let mut txs = Vec::new();
    for (i, line) in lines[1..].iter().enumerate() {
        let row = split_csv_line(line, delimiter);
        let get = |idx: Option<usize>| -> Option<String> {
            idx.and_then(|i| row.get(i).map(|s| s.trim().to_string()))
                .filter(|s| !s.is_empty())
        };
        let date_str = get(Some(date_idx)).unwrap_or_default();
        let amount_str = get(Some(amount_idx)).unwrap_or_default();
        // normalize the date: some banks use DD-MM-YYYY
        let date = normalize_date(&date_str);
        let mut amount_cents = match parse_bank_amount(&amount_str) {
            Some(v) => v,
            None => continue, // skip unparseable rows
        };
        // Dutch CSV "Af Bij" column: "Af" = outgoing (negate), "Bij" = incoming (keep positive)
        if let Some(idx) = af_bij_idx {
            if let Some(code) = row.get(idx).map(|s| s.trim()) {
                if code.eq_ignore_ascii_case("Af") && amount_cents > 0 {
                    amount_cents = -amount_cents;
                } else if code.eq_ignore_ascii_case("Bij") && amount_cents < 0 {
                    amount_cents = amount_cents.abs();
                }
            }
        }
        if date.is_empty() {
            continue;
        }
        txs.push(BankTx {
            date,
            amount_cents,
            counterparty: get(counterparty_idx),
            description: get(description_idx),
            iban_counter: get(iban_idx),
            bank_ref: None,
        });
    }
    Ok(txs)
}

/// Normalize a date from various formats to YYYY-MM-DD.
fn normalize_date(s: &str) -> String {
    if s.len() == 10 && s.as_bytes()[4] == b'-' && s.as_bytes()[7] == b'-' {
        return s.to_string(); // already YYYY-MM-DD
    }
    if s.len() == 10 && s.as_bytes()[2] == b'-' && s.as_bytes()[5] == b'-' {
        // DD-MM-YYYY -> YYYY-MM-DD
        let parts: Vec<&str> = s.split('-').collect();
        if parts.len() == 3 {
            return format!("{}-{}-{}", parts[2], parts[1], parts[0]);
        }
    }
    s.to_string()
}

/// Parse a bank amount string (handles Dutch "1.234,56" and English "1,234.56").
fn parse_bank_amount(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let negative = s.starts_with('-');
    let s = s.trim_start_matches(['-', '+']);

    // detect format: if we see both . and , the one that appears LAST is the decimal separator
    let has_dot = s.contains('.');
    let has_comma = s.contains(',');

    let normalized = match (has_dot, has_comma) {
        (true, true) => {
            // "1.234,56" (Dutch) or "1,234.56" (English) — last occurrence wins
            let last_dot = s.rfind('.');
            let last_comma = s.rfind(',');
            if last_comma > last_dot {
                // Dutch: dots are thousand separators, comma is decimal
                s.replace('.', "").replace(',', ".")
            } else {
                // English: commas are thousand separators, dot is decimal
                s.replace(',', "")
            }
        }
        (true, false) => s.replace(',', ""), // English with dots only
        (false, true) => s.replace(',', "."), // Dutch with comma only
        (false, false) => s.to_string(),     // integer
    };

    // parse as decimal
    let parts: Vec<&str> = normalized.split('.').collect();
    let whole: i64 = parts[0].parse().ok()?;
    let frac = if parts.len() >= 2 {
        let frac_str = parts[1];
        if frac_str.len() >= 2 {
            frac_str[..2].parse::<i64>().unwrap_or(0)
        } else if frac_str.len() == 1 {
            frac_str.parse::<i64>().unwrap_or(0) * 10
        } else {
            0
        }
    } else {
        0
    };
    let cents = whole.abs() * 100 + frac;
    Some(if negative { -cents } else { cents })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_bank_amount_dutch() {
        assert_eq!(parse_bank_amount("1.234,56"), Some(123456));
        assert_eq!(parse_bank_amount("-50,00"), Some(-5000));
        assert_eq!(parse_bank_amount("100"), Some(10000));
        assert_eq!(parse_bank_amount("+1.000,50"), Some(100050));
        assert_eq!(parse_bank_amount("abc"), None);
        assert_eq!(parse_bank_amount(""), None);
    }

    #[test]
    fn parse_bank_amount_english() {
        assert_eq!(parse_bank_amount("1,234.56"), Some(123456));
        assert_eq!(parse_bank_amount("-50.00"), Some(-5000));
    }

    #[test]
    fn parse_dutch_csv_semicolons() {
        let csv = "Datum;Bedrag;Rekening;Naam;Omschrijving\n2026-01-15;1234,56;NL91ABNA0417164300;ACME;Invoice 42\n2026-01-16;-50,00;;Shop;Payment\n";
        let txs = parse_bank_csv(csv, "NL91ABNA0417164300").unwrap();
        assert_eq!(txs.len(), 2);
        assert_eq!(txs[0].amount_cents, 123456);
        assert_eq!(txs[0].counterparty.as_deref(), Some("ACME"));
        assert_eq!(txs[1].amount_cents, -5000);
    }

    #[test]
    fn parse_dutch_csv_commas() {
        let csv = "Datum,Bedrag,Rekening,Naam,Omschrijving\n2026-01-15,1234.56,NL91ABNA0417164300,ACME,Invoice\n";
        let txs = parse_bank_csv(csv, "NL91ABNA0417164300").unwrap();
        assert_eq!(txs.len(), 1);
        assert_eq!(txs[0].amount_cents, 123456);
    }
}
