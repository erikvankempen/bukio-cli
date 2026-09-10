// bukio-cli — agent-first double-entry bookkeeping for SMEs.
// Copyright (c) 2026 Erik van Kempen.
// SPDX-License-Identifier: Apache-2.0
//
// CAMT.053 XML parser (mirrors src/bank/camt.js).

use crate::bank::BankTx;
use crate::money::BukioError;

/// Parse CAMT.053 XML into a list of bank transactions.
/// Uses simple string scanning (no XML crate needed for the well-defined SDMX-like structure).
pub fn parse_camt053(xml: &str) -> Result<Vec<BankTx>, BukioError> {
    // the JS validates the document shape and refuses junk/junk-only XML; the
    // old string scanner returned an empty Vec instead (money vanished silently)
    if !xml.contains("<Document") {
        return Err(BukioError::new(
            "INVALID_CAMT",
            "could not parse CAMT.053 XML",
        ));
    }
    if !xml.contains("<BkToCstmrStmt") || !xml.contains("<Stmt") {
        return Err(BukioError::new(
            "INVALID_CAMT",
            "no BkToCstmrStmt/Stmt found in the XML",
        ));
    }
    let statement_iban = extract_tag(xml, "IBAN")
        .ok_or_else(|| BukioError::new("INVALID_CAMT", "statement is missing Acct/Id/IBAN"))?;

    let mut transactions = Vec::new();
    let mut pos = 0;
    while let Some(ntry_start) = xml[pos..].find("<Ntry>") {
        let abs_start = pos + ntry_start;
        let Some(ntry_end) = xml[abs_start..].find("</Ntry>") else {
            break;
        };
        let block = &xml[abs_start..abs_start + ntry_end + 7];
        // an Ntry without a parseable Amt is corrupt: skipping it would import a
        // partial statement and make the balance diverge from the bank's
        let tx = parse_ntry(block).ok_or_else(|| {
            BukioError::new(
                "INVALID_CAMT",
                "Ntry without a valid cbc:Amt — the statement is corrupt; fix it before importing",
            )
        })?;
        transactions.push(tx);
        pos = abs_start + ntry_end + 7;
    }
    if transactions.is_empty() {
        return Err(BukioError::new(
            "EMPTY_STATEMENT",
            "no transactions found in the CAMT.053 statement",
        ));
    }
    // every row carries the statement's own IBAN (the JS stamps it per entry)
    for t in &mut transactions {
        t.iban = Some(statement_iban.clone());
    }
    Ok(transactions)
}

/// The content of the first `<tag>...</tag>` section (for nested lookups).
fn extract_section<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    Some(&xml[start..end])
}

fn parse_ntry(block: &str) -> Option<BankTx> {
    let date = extract_tag(block, "Dt")?;
    let amount_str = extract_tag(block, "Amt")?;
    let amount_cents = parse_amount_cents(&amount_str)?;
    // DBIT = money out (negative), CRDT = money in (positive)
    let direction = extract_tag(block, "CdtDbtInd").unwrap_or_default();
    let signed_cents = if direction == "DBIT" {
        -amount_cents
    } else {
        amount_cents
    };
    // counterparty: the OTHER party — creditor when money goes out, debtor when
    // it comes in; an empty <Nm></Nm> is absent, so the chain falls through
    let debtor_nm = extract_section(block, "Dbtr").and_then(|b| extract_tag(b, "Nm"));
    let creditor_nm = extract_section(block, "Cdtr").and_then(|b| extract_tag(b, "Nm"));
    let counterparty = if direction == "DBIT" {
        creditor_nm.or(debtor_nm)
    } else {
        debtor_nm.or(creditor_nm)
    };
    let creditor_iban = extract_section(block, "CdtrAcct").and_then(|s| extract_tag(s, "IBAN"));
    let debtor_iban = extract_section(block, "DbtrAcct").and_then(|s| extract_tag(s, "IBAN"));
    let iban_counter = if direction == "DBIT" {
        creditor_iban
    } else {
        debtor_iban
    };
    let bank_ref = extract_tag(block, "AcctSvcrRef");
    let description = extract_tag(block, "Ustrd")
        .or_else(|| extract_tag(block, "AddtlNtryInf"))
        .or_else(|| extract_tag(block, "Desc"));

    Some(BankTx {
        date,
        amount_cents: signed_cents,
        counterparty,
        description,
        iban_counter,
        bank_ref,
        iban: None, // stamped by parse_camt053 from the statement
    })
}

fn extract_tag(xml: &str, tag: &str) -> Option<String> {
    // find <tag (with or without attributes) followed by >
    let tag_prefix = format!("<{tag}");
    let start = xml.find(&tag_prefix)?;
    let after_tag = &xml[start + tag_prefix.len()..];
    let gt_pos = after_tag.find('>')?;
    let content_start = start + tag_prefix.len() + gt_pos + 1;
    let end_tag = format!("</{tag}>");
    let end = xml[content_start..].find(&end_tag)?;
    let value = &xml[content_start..content_start + end];
    let trimmed = value.trim().to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn parse_amount_cents(s: &str) -> Option<i64> {
    // CAMT amounts: "1234.56" or "-1234.56"
    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() == 2 {
        let whole: i64 = parts[0].parse().ok()?;
        let sign = if whole < 0 { -1 } else { 1 };
        let frac_str = parts[1];
        let frac: i64 = if frac_str.len() == 1 {
            frac_str.parse::<i64>().ok()? * 10
        } else if frac_str.len() >= 2 {
            frac_str[..2].parse().ok()?
        } else {
            0
        };
        Some(sign * (whole.abs() * 100 + frac))
    } else {
        let v: i64 = s.parse().ok()?;
        Some(v * 100)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_camt() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<Document xmlns="urn:iso:std:iso:20022:tech:xsd:camt.053.001.08">
  <BkToCstmrStmt>
    <Stmt>
      <Ntry>
        <Amt Ccy="EUR">123.45</Amt>
        <CdtDbtInd>CRDT</CdtDbtInd>
        <BookgDt><Dt>2026-01-15</Dt></BookgDt>
        <NtryDtls><TxDtls>
          <Refs><AcctSvcrRef>TX001</AcctSvcrRef></Refs>
          <Cdtr><Nm>ACME Corp</Nm></Cdtr>
          <CdtrAcct><Id><IBAN>NL91ABNA0417164300</IBAN></Id></CdtrAcct>
          <AddtlNtryInf>Invoice 42</AddtlNtryInf>
        </TxDtls></NtryDtls>
      </Ntry>
      <Ntry>
        <Amt Ccy="EUR">50.00</Amt>
        <CdtDbtInd>DBIT</CdtDbtInd>
        <BookgDt><Dt>2026-01-16</Dt></BookgDt>
        <NtryDtls><TxDtls>
          <Refs><AcctSvcrRef>TX002</AcctSvcrRef></Refs>
          <Dbtr><Nm>Shop</Nm></Dbtr>
        </TxDtls></NtryDtls>
      </Ntry>
    </Stmt>
  </BkToCstmrStmt>
</Document>"#;
        let txs = parse_camt053(xml).unwrap();
        assert_eq!(txs.len(), 2);
        assert_eq!(txs[0].amount_cents, 12345);
        assert_eq!(txs[0].counterparty.as_deref(), Some("ACME Corp"));
        assert_eq!(txs[0].bank_ref.as_deref(), Some("TX001"));
        assert_eq!(txs[1].amount_cents, -5000);
    }

    // ==== ported from test/bank.test.js ======================================

    const IBAN: &str = "NL91ABNA0417164300";
    const CAMT: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Document xmlns="urn:iso:std:iso:20022:tech:xsd:camt.053.001.02">
  <BkToCstmrStmt>
    <Stmt>
      <Acct><Id><IBAN>NL91ABNA0417164300</IBAN></Id></Acct>
      <Ntry>
        <Amt>100.00</Amt>
        <CdtDbtInd>CRDT</CdtDbtInd>
        <BookgDt><Dt>2026-06-01</Dt></BookgDt>
        <NtryDtls><TxDtls>
          <RltdPties><Dbtr><Nm>ACME B.V.</Nm></Dbtr></RltdPties>
          <RmtInf><Ustrd>Factuur 2026-001</Ustrd></RmtInf>
        </TxDtls></NtryDtls>
      </Ntry>
      <Ntry>
        <Amt>25.50</Amt>
        <CdtDbtInd>DBIT</CdtDbtInd>
        <BookgDt><Dt>2026-06-02</Dt></BookgDt>
        <NtryDtls><TxDtls>
          <RltdPties><Cdtr><Nm>Kantoorwinkel BV</Nm></Cdtr></RltdPties>
          <RmtInf><Ustrd>Kantoorartikelen</Ustrd></RmtInf>
        </TxDtls></NtryDtls>
      </Ntry>
    </Stmt>
  </BkToCstmrStmt>
</Document>"#;

    #[test]
    fn crdt_is_positive_dbit_negative_with_counterparty_and_description() {
        let txs = parse_camt053(CAMT).unwrap();
        assert_eq!(txs.len(), 2);
        assert_eq!(txs[0].amount_cents, 10000);
        assert_eq!(txs[0].date, "2026-06-01");
        assert_eq!(txs[0].counterparty.as_deref(), Some("ACME B.V."));
        assert_eq!(txs[0].description.as_deref(), Some("Factuur 2026-001"));
        assert_eq!(txs[0].iban.as_deref(), Some(IBAN));
        assert_eq!(txs[1].amount_cents, -2550);
        assert_eq!(txs[1].counterparty.as_deref(), Some("Kantoorwinkel BV"));
    }

    #[test]
    fn rejects_non_camt_input() {
        assert_eq!(
            parse_camt053("<foo><bar/></foo>").unwrap_err().code,
            "INVALID_CAMT"
        );
        assert_eq!(
            parse_camt053("not xml at all").unwrap_err().code,
            "INVALID_CAMT"
        );
    }

    #[test]
    fn empty_party_name_element_falls_through_to_the_other_party() {
        // <Nm></Nm> (empty element) must count as absent so the counterparty
        // falls through to the other party instead of storing ''
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<Document xmlns="urn:iso:std:iso:20022:tech:xsd:camt.053.001.02">
  <BkToCstmrStmt><Stmt>
    <Acct><Id><IBAN>NL91ABNA0417164300</IBAN></Id></Acct>
    <Ntry>
      <Amt>100.00</Amt><CdtDbtInd>DBIT</CdtDbtInd>
      <BookgDt><Dt>2026-06-01</Dt></BookgDt>
      <NtryDtls><TxDtls>
        <RltdPties><Cdtr><Nm></Nm></Cdtr><Dbtr><Nm>Acme BV</Nm></Dbtr></RltdPties>
      </TxDtls></NtryDtls>
    </Ntry>
  </Stmt></BkToCstmrStmt>
</Document>"#;
        let txs = parse_camt053(xml).unwrap();
        assert_eq!(txs.len(), 1);
        assert_eq!(txs[0].counterparty.as_deref(), Some("Acme BV"));
    }
}
