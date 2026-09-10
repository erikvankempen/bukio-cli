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
    header
        .iter()
        .position(|h| h == key)
        .or_else(|| header.iter().position(|h| h.contains(key)))
}

/// Result of parsing a bank CSV: the transactions plus the rows that were
/// skipped. The JS attaches `skipped` to the returned array and the CLI
/// surfaces it as a warning — never drop money silently.
#[derive(Debug)]
pub struct BankCsv {
    pub transactions: Vec<BankTx>,
    pub skipped: Vec<serde_json::Value>,
}

/// Parse a bank CSV file into transactions (mirrors the JS parseBankCsv).
pub fn parse_bank_csv(content: &str, default_iban: &str) -> Result<BankCsv, BukioError> {
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
        .or_else(|| find_column(&header, "begunstigde"))
        .or_else(|| find_column(&header, "tegenrekening"))
        .or_else(|| find_column(&header, "from"));
    let description_idx = find_column(&header, "mededelingen")
        .or_else(|| find_column(&header, "omschrijving"))
        .or_else(|| find_column(&header, "description"));
    let af_bij_idx = find_column(&header, "afbij").or_else(|| find_column(&header, "af/bij"));
    // the COUNTER party's IBAN — not the account's own (that comes from --iban)
    let iban_idx = find_column(&header, "tegenrekening")
        .or_else(|| find_column(&header, "rekening"))
        .or_else(|| find_column(&header, "iban"));

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
    let mut skipped: Vec<serde_json::Value> = Vec::new();
    for (i, line) in lines[1..].iter().enumerate() {
        let row = split_csv_line(line, delimiter);
        let raw = |idx: Option<usize>| -> String {
            idx.and_then(|i| row.get(i).map(|s| s.trim().to_string()))
                .unwrap_or_default()
        };
        let get = |idx: Option<usize>| -> Option<String> {
            let v = raw(idx);
            if v.is_empty() {
                None
            } else {
                Some(v)
            }
        };
        let date_raw = raw(Some(date_idx));
        let amount_raw = raw(Some(amount_idx));
        let (date, amount) = match (normalize_date(&date_raw), parse_bank_amount(&amount_raw)) {
            (Some(d), Some(a)) => (d, a),
            (d, _) => {
                // never drop money silently — report the row so the user can fix it
                skipped.push(serde_json::json!({
                    "line": i + 2,
                    "reason": if d.is_none() {
                        "missing/invalid date".to_string()
                    } else {
                        format!("unparseable amount '{amount_raw}'")
                    },
                }));
                continue;
            }
        };
        let mut amount_cents = amount;
        // Dutch CSV "Af Bij" column: Af = outgoing (negative), Bij = incoming
        if let Some(code) = af_bij_idx
            .and_then(|idx| row.get(idx))
            .map(|s| s.trim().to_lowercase())
        {
            if code.starts_with("af") {
                amount_cents = -amount_cents.abs();
            } else if code.starts_with("bij") {
                amount_cents = amount_cents.abs();
            }
        }
        txs.push(BankTx {
            date,
            amount_cents,
            counterparty: get(counterparty_idx),
            description: get(description_idx),
            iban_counter: get(iban_idx),
            bank_ref: None,
            iban: if default_iban.trim().is_empty() {
                None
            } else {
                Some(default_iban.to_string())
            },
        });
    }
    if txs.is_empty() {
        return Err(BukioError::new(
            "EMPTY_STATEMENT",
            "no parseable transactions found in the CSV",
        ));
    }
    Ok(BankCsv {
        transactions: txs,
        skipped,
    })
}

/// Normalize a date to ISO YYYY-MM-DD: ISO (incl. datetime forms), Dutch
/// DD-MM-YYYY (and D-M-YYYY) and compact YYYYMMDD. None for unparseable or
/// non-existent calendar dates (2026-02-31) — mirrors the JS normalizeDate.
fn normalize_date(raw: &str) -> Option<String> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    let iso = |y: &str, mo: &str, d: &str| -> Option<String> {
        let iso = format!("{:0>4}-{:0>2}-{:0>2}", y, mo, d);
        chrono::NaiveDate::parse_from_str(&iso, "%Y-%m-%d")
            .ok()
            .map(|_| iso)
    };
    let digits = |x: &str| !x.is_empty() && x.chars().all(|c| c.is_ascii_digit());
    let parts: Vec<&str> = s.split(['-', 'T', ' ']).collect();
    // ISO YYYY-MM-DD, optionally followed by a time
    if parts.len() >= 3 && parts[0].len() == 4 && parts[2].len() >= 2 {
        if digits(parts[0]) && digits(parts[1]) && digits(&parts[2][..2]) {
            return iso(parts[0], parts[1], &parts[2][..2]);
        }
    }
    // Dutch DD-MM-YYYY / D-M-YYYY
    if parts.len() >= 3 && parts[2].len() == 4 && digits(&parts[2][..4]) && digits(parts[0]) {
        return iso(&parts[2][..4], parts[1], parts[0]);
    }
    // compact YYYYMMDD
    if s.len() == 8 && digits(s) {
        return iso(&s[..4], &s[4..6], &s[6..8]);
    }
    None
}

/// Lenient Dutch amount parser, mirroring the JS parseBankAmount: handles
/// '1.234,56', '1234.56', '1.234' (thousands), '-12,50', '(12,50)', '€ 12,50',
/// '.50'. Returns cents, or None for junk — never NaN into the ledger.
pub fn parse_bank_amount(value: &str) -> Option<i64> {
    let mut s: String = value
        .trim()
        .chars()
        .filter(|c| !matches!(c, '"' | '\'' | '€') && !c.is_whitespace())
        .collect();
    if s.is_empty() {
        return None;
    }
    let mut negative = s.starts_with('-') || s.starts_with('(');
    if s.ends_with(')') || s.ends_with('-') {
        if s.ends_with('-') {
            negative = true;
        }
        s.pop();
    }
    s = s.trim_start_matches('-').replace(['(', ')'], "");
    // '.50' → '0.50'
    if s.starts_with('.') && s.len() > 1 && s[1..].chars().all(|c| c.is_ascii_digit()) {
        s = format!("0{s}");
    }
    if !s
        .chars()
        .all(|c| c.is_ascii_digit() || c == '.' || c == ',')
    {
        return None;
    }
    let int = |x: &str| -> Option<i64> {
        if x.is_empty() {
            return None;
        }
        x.parse::<i64>().ok()
    };
    // pad a fraction to two digits and take exactly two (JS padEnd(2,'0').slice(0,2))
    let frac2 = |x: &str| -> String {
        let mut f = x.to_string();
        while f.len() < 2 {
            f.push('0');
        }
        f[..2].to_string()
    };
    let cents: Option<i64> = if s.contains('.') && s.contains(',') {
        // the LAST separator is the decimal one
        let dec = s.rfind('.').unwrap().max(s.rfind(',').unwrap());
        let int_part: String = s[..dec]
            .chars()
            .filter(|c| *c != '.' && *c != ',')
            .collect();
        let frac: String = s[dec + 1..]
            .chars()
            .filter(|c| *c != '.' && *c != ',')
            .collect();
        Some(int(&int_part)? * 100 + int(&frac2(&frac)).unwrap_or(0))
    } else if s.contains(',') {
        let mut it = s.splitn(2, ',');
        let i: String = it
            .next()
            .unwrap_or("")
            .chars()
            .filter(|c| *c != '.')
            .collect();
        let f = it.next().unwrap_or("");
        Some(int(&i)? * 100 + int(&frac2(f)).unwrap_or(0))
    } else if s.contains('.') {
        let parts: Vec<&str> = s.split('.').collect();
        if parts.len() == 2 && parts[1].len() <= 2 {
            Some(int(parts[0])? * 100 + int(&frac2(parts[1])).unwrap_or(0))
        } else {
            // thousands separator, integer amount ('1.234' = 1234.00)
            let clean: String = s.chars().filter(|c| *c != '.').collect();
            Some(int(&clean)? * 100)
        }
    } else {
        Some(int(&s)? * 100)
    };
    let cents = cents?;
    Some(if negative && cents != 0 {
        -cents
    } else {
        cents
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_bank_amount_dutch() {
        assert_eq!(parse_bank_amount("1.234,56"), Some(123456));
        assert_eq!(parse_bank_amount("-50,00"), Some(-5000));
        assert_eq!(parse_bank_amount("100"), Some(10000));
        // the JS character guard is [\d.,] only, so a leading '+' is rejected
        assert_eq!(parse_bank_amount("+1.000,50"), None);
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
        let txs = parse_bank_csv(csv, "NL91ABNA0417164300")
            .unwrap()
            .transactions;
        assert_eq!(txs.len(), 2);
        assert_eq!(txs[0].amount_cents, 123456);
        assert_eq!(txs[0].counterparty.as_deref(), Some("ACME"));
        assert_eq!(txs[1].amount_cents, -5000);
    }

    #[test]
    fn parse_dutch_csv_commas() {
        let csv = "Datum,Bedrag,Rekening,Naam,Omschrijving\n2026-01-15,1234.56,NL91ABNA0417164300,ACME,Invoice\n";
        let txs = parse_bank_csv(csv, "NL91ABNA0417164300")
            .unwrap()
            .transactions;
        assert_eq!(txs.len(), 1);
        assert_eq!(txs[0].amount_cents, 123456);
    }

    // ==== ported from test/bank.test.js ======================================

    const IBAN: &str = "NL91ABNA0417164300";

    fn rabo_csv() -> String {
        [
            "Datum;Naam / Omschrijving;Rekening;Tegenrekening;Code;Af Bij;Bedrag (EUR);MutatieSoort;Mededelingen",
            "2026-06-01;ACME B.V.;NL91ABNA0417164300;NL00RABO0123456789;GT;Bij;100,00;Overschrijving;Factuur 2026-001",
            "2026-06-02;Kantoorwinkel BV;NL91ABNA0417164300;NL00RABO9876543210;GT;Af;25,50;Overschrijving;Kantoorartikelen",
        ]
        .join("\n")
    }

    #[test]
    fn degenerate_amounts_return_none_never_nan() {
        assert_eq!(parse_bank_amount("."), None);
        assert_eq!(parse_bank_amount(","), None);
        assert_eq!(parse_bank_amount(".,"), None);
        assert_eq!(parse_bank_amount(""), None);
        assert_eq!(parse_bank_amount("12.50"), Some(1250));
        assert_eq!(parse_bank_amount("1.234,56"), Some(123456));
    }

    #[test]
    fn dutch_and_international_amount_formats() {
        assert_eq!(parse_bank_amount("100,00"), Some(10000));
        assert_eq!(parse_bank_amount("1.234,56"), Some(123456));
        assert_eq!(parse_bank_amount("1234.56"), Some(123456));
        assert_eq!(parse_bank_amount("1.234"), Some(123400)); // thousands
        assert_eq!(parse_bank_amount("-12,50"), Some(-1250));
        assert_eq!(parse_bank_amount("€ 12,50"), Some(1250));
        assert_eq!(parse_bank_amount("0,50"), Some(50));
        assert_eq!(parse_bank_amount(".50"), Some(50)); // leading dot
        assert_eq!(parse_bank_amount("-5.00"), Some(-500));
        assert_eq!(parse_bank_amount("(12,50)"), Some(-1250)); // accounting negative
        assert_eq!(parse_bank_amount("abc"), None);
    }

    #[test]
    fn rabo_style_export_with_af_bij_sign() {
        let csv = parse_bank_csv(&rabo_csv(), IBAN).unwrap();
        let txs = &csv.transactions;
        assert_eq!(txs.len(), 2);
        assert_eq!(txs[0].amount_cents, 10000); // Bij
        assert_eq!(txs[1].amount_cents, -2550); // Af
        assert_eq!(txs[0].counterparty.as_deref(), Some("ACME B.V."));
        assert_eq!(txs[0].date, "2026-06-01");
        // the account's own IBAN comes from --iban; iban_counter is the other side
        assert_eq!(txs[0].iban.as_deref(), Some(IBAN));
        assert_eq!(txs[0].iban_counter.as_deref(), Some("NL00RABO0123456789"));
    }

    #[test]
    fn missing_required_columns_are_rejected() {
        assert_eq!(
            parse_bank_csv("foo;bar\n1;2\n", IBAN).unwrap_err().code,
            "INVALID_CSV_HEADER"
        );
    }

    #[test]
    fn dutch_and_compact_dates_normalize_to_iso() {
        let csv = "Datum;Naam;Bedrag\n03-06-2026;Rabo;100,00\n20260604;ABN;-25.50\n";
        let parsed = parse_bank_csv(csv, IBAN).unwrap();
        assert_eq!(parsed.transactions.len(), 2);
        assert_eq!(parsed.transactions[0].date, "2026-06-03");
        assert_eq!(parsed.transactions[1].date, "2026-06-04");
        assert_eq!(parsed.transactions[0].amount_cents, 10000);
        assert_eq!(parsed.transactions[1].amount_cents, -2550);
        assert_eq!(parsed.skipped.len(), 0);
    }

    #[test]
    fn unparseable_date_is_skipped_and_reported_never_silently_dropped() {
        let csv = "Datum;Naam;Bedrag\n31-02-2026;Rabo;10,00\n2026-06-04;ABN;-25.50\n";
        let parsed = parse_bank_csv(csv, IBAN).unwrap();
        assert_eq!(parsed.transactions.len(), 1);
        assert_eq!(parsed.skipped.len(), 1);
        let reason = parsed.skipped[0]["reason"].as_str().unwrap_or("");
        assert!(reason.contains("date"), "got: {reason}");
        assert_eq!(parsed.skipped[0]["line"].as_i64(), Some(2));
    }
}
