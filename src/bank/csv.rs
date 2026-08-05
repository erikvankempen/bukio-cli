//! Dutch bank CSV parser — tolerant of Rabo / ING / ABN AMRO / generic exports.

use crate::error::Result;
use super::camt::{bank_error, BankTx};

const HEADER_ALIASES: [(&str, &[&str]); 7] = [
    (
        "date",
        &["datum", "date", "boekingsdatum", "transaction date", "transactiedatum"],
    ),
    (
        "counterparty",
        &["naam", "name", "tegenpartij", "begunstigde", "begunstigd door", "crediteur", "debiteur", "naam / omschrijving"],
    ),
    (
        "description",
        &["omschrijving", "description", "mededelingen", "memo", "opmerkingen", "naam / omschrijving"],
    ),
    (
        "amount",
        &["bedrag", "amount", "bedrag (eur)", "bedrag(eur)", "bedrag (in eur)", "bedrag in eur", "transaction amount"],
    ),
    ("afbij", &["af bij", "af/bij", "af-bij", "bij/af", "af bij (eur)", "credit debet"]),
    ("iban_counter", &["tegenrekening", "iban tegenrekening", "tegenrekening iban", "counter iban", "reknr tegenpartij"]),
    ("code", &["code", "mutatiesoort", "transactiesoort"]),
];

fn normalize_header(h: &str) -> String {
    h.trim().to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ")
}

fn detect_delimiter(line: &str) -> char {
    let semicolons = line.matches(';').count();
    let commas = line.matches(',').count();
    if semicolons >= commas {
        ';'
    } else {
        ','
    }
}

fn split_line(line: &str, delimiter: char) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    for ch in line.chars() {
        if ch == '"' {
            in_quotes = !in_quotes;
        } else if ch == delimiter && !in_quotes {
            out.push(std::mem::take(&mut cur));
        } else {
            cur.push(ch);
        }
    }
    out.push(cur);
    out
}

/// Lenient Dutch amount parser: handles '1.234,56', '1234.56', '1.234',
/// '-12,50', '(12,50)', '€ 12,50'. Returns cents or null.
pub fn parse_bank_amount(value: &str) -> Option<i64> {
    let mut s: String = value.trim().chars().filter(|c| !matches!(c, '"' | '\'' | ' ' | '€')).collect();
    if s.is_empty() {
        return None;
    }
    let mut negative = false;
    if s.starts_with('-') || s.starts_with('(') {
        negative = true;
    }
    if s.ends_with(')') || s.ends_with('-') {
        if s.ends_with('-') {
            negative = true;
        }
        s.pop();
    }
    s = s.trim_start_matches('-').replace(['(', ')'], "");
    if !s.chars().all(|c| c.is_ascii_digit() || c == '.' || c == ',') {
        return None;
    }

    let cents: i64 = if s.contains('.') && s.contains(',') {
        let dec_idx = s.rfind('.').unwrap().max(s.rfind(',').unwrap());
        let int_part = s[..dec_idx].replace(['.', ','], "");
        let frac = s[dec_idx + 1..].replace(['.', ','], "");
        let frac_padded = format!("{frac:0<2}");
        let frac2: i64 = frac_padded.chars().take(2).collect::<String>().parse().unwrap_or(0);
        int_part.parse::<i64>().unwrap_or(0) * 100 + frac2
    } else if s.contains(',') {
        let mut parts = s.split(',');
        let i = parts.next().unwrap_or("0").replace('.', "");
        let f = parts.next().unwrap_or("");
        let frac_padded = format!("{f:0<2}");
        let frac2: i64 = frac_padded.chars().take(2).collect::<String>().parse().unwrap_or(0);
        i.parse::<i64>().unwrap_or(0) * 100 + frac2
    } else if s.contains('.') {
        let parts: Vec<&str> = s.split('.').collect();
        if parts.len() == 2 && parts[1].len() <= 2 {
            let frac_padded = format!("{}0", parts[1]);
            let frac2: i64 = frac_padded.chars().take(2).collect::<String>().parse().unwrap_or(0);
            parts[0].parse::<i64>().unwrap_or(0) * 100 + frac2
        } else {
            s.replace('.', "").parse::<i64>().unwrap_or(0) * 100 // thousands separator, integer amount
        }
    } else {
        s.parse::<i64>().unwrap_or(0) * 100
    };
    if negative {
        Some(-cents)
    } else {
        Some(cents)
    }
}

fn find_column(header: &str) -> Option<&'static str> {
    let norm = normalize_header(header);
    for (key, names) in HEADER_ALIASES.iter() {
        if names.contains(&norm.as_str()) {
            return Some(key);
        }
    }
    None
}

/// Parse a bank CSV export into transactions.
/// The IBAN may come from a --iban argument (caller injects it).
pub fn parse_bank_csv(csv_text: &str, default_iban: Option<&str>) -> Result<Vec<BankTx>> {
    let lines: Vec<&str> = csv_text
        .split('\n')
        .map(|l| l.trim_end_matches('\r').trim_end())
        .filter(|l| !l.is_empty())
        .collect();
    if lines.len() < 2 {
        return Err(bank_error("EMPTY_CSV", "bank CSV needs a header row and at least one transaction"));
    }

    let delimiter = detect_delimiter(lines[0]);
    let header: Vec<String> = split_line(lines[0], delimiter).iter().map(|h| normalize_header(h)).collect();
    let columns: Vec<Option<&str>> = header.iter().map(|h| find_column(h)).collect();

    let mut idx: Vec<(String, isize)> = Vec::new();
    for (key, _) in HEADER_ALIASES.iter() {
        let pos = columns.iter().position(|c| *c == Some(*key)).map(|p| p as isize).unwrap_or(-1);
        idx.push((key.to_string(), pos));
    }
    let get_idx = |key: &str| idx.iter().find(|(k, _)| k == key).map(|(_, p)| *p).unwrap_or(-1);

    if get_idx("date") == -1 || get_idx("amount") == -1 {
        return Err(bank_error(
            "INVALID_CSV_HEADER",
            format!("bank CSV needs at least 'datum' and 'bedrag' columns (got: {})", header.join(", ")),
        ));
    }

    let mut transactions: Vec<BankTx> = Vec::new();
    for line in &lines[1..] {
        let row = split_line(line, delimiter);
        if row.len() == 1 && row[0].trim().is_empty() {
            continue;
        }
        let get = |key: &str| -> String {
            let i = get_idx(key);
            if i >= 0 {
                row.get(i as usize).map(|v| v.trim().to_string()).unwrap_or_default()
            } else {
                String::new()
            }
        };

        let date: String = get("date").chars().take(10).collect();
        let amount = parse_bank_amount(&get("amount"));
        let Some(amount) = amount else { continue };
        if date.is_empty() {
            continue;
        }

        let mut signed_amount = amount;
        let afbij = get("afbij").to_lowercase();
        if afbij.starts_with("af") {
            signed_amount = -amount.abs();
        } else if afbij.starts_with("bij") {
            signed_amount = amount.abs();
        }

        let iban_counter = get("iban_counter");
        transactions.push(BankTx {
            date,
            amount_cents: signed_amount,
            counterparty: {
                let c = get("counterparty");
                if c.is_empty() { None } else { Some(c) }
            },
            description: {
                let d = get("description");
                if d.is_empty() { None } else { Some(d) }
            },
            iban_counter: if iban_counter.is_empty() { None } else { Some(iban_counter) },
            iban: default_iban.map(|s| s.to_string()),
        });
    }

    if transactions.is_empty() {
        return Err(bank_error("EMPTY_STATEMENT", "no parseable transactions found in the CSV"));
    }
    Ok(transactions)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_dutch_amounts() {
        assert_eq!(parse_bank_amount("1.234,56"), Some(123456));
        assert_eq!(parse_bank_amount("1234.56"), Some(123456));
        assert_eq!(parse_bank_amount("1.234"), Some(123400));
        assert_eq!(parse_bank_amount("-12,50"), Some(-1250));
        assert_eq!(parse_bank_amount("(12,50)"), Some(-1250));
        assert_eq!(parse_bank_amount("€ 12,50"), Some(1250));
        assert_eq!(parse_bank_amount("100"), Some(10000));
        assert_eq!(parse_bank_amount(""), None);
        assert_eq!(parse_bank_amount("abc"), None);
    }

    #[test]
    fn parses_rabo_style_csv() {
        let csv = "Datum;Naam / Omschrijving;Rekening;Tegenrekening;Code;Af Bij;Bedrag (EUR);Mutatiesoort\n\
                   2026-07-01;Klant BV;NL91ABNA0417164300;NL12INGB0001234567;GT;Bij;1.234,56;Betaalautomaat\n\
                   2026-07-02;Albert Heijn;NL91ABNA0417164300;;BA;Af;23,40;Betaalautomaat";
        let txs = parse_bank_csv(csv, Some("NL91ABNA0417164300")).unwrap();
        assert_eq!(txs.len(), 2);
        assert_eq!(txs[0].amount_cents, 123456);
        assert_eq!(txs[0].counterparty.as_deref(), Some("Klant BV"));
        assert_eq!(txs[1].amount_cents, -2340);
        assert_eq!(txs[1].counterparty.as_deref(), Some("Albert Heijn"));
    }

    #[test]
    fn rejects_missing_columns() {
        let err = parse_bank_csv("foo;bar\n1;2", None).unwrap_err();
        assert_eq!(err.code(), "INVALID_CSV_HEADER");
    }
}
