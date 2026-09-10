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
    let mut transactions = Vec::new();

    // find all Ntry blocks
    let mut pos = 0;
    while let Some(ntry_start) = xml[pos..].find("<Ntry>") {
        let abs_start = pos + ntry_start;
        if let Some(ntry_end) = xml[abs_start..].find("</Ntry>") {
            let block = &xml[abs_start..abs_start + ntry_end + 7];
            if let Some(tx) = parse_ntry(block) {
                transactions.push(tx);
            }
            pos = abs_start + ntry_end + 7;
        } else {
            break;
        }
    }
    Ok(transactions)
}

fn parse_ntry(block: &str) -> Option<BankTx> {
    let date = extract_tag(block, "Dt")?;
    let amount_str = extract_tag(block, "Amt")?;
    let amount_cents = parse_amount_cents(&amount_str)?;
    let counterparty = extract_tag(block, "Nm").or_else(|| extract_tag(block, "Cdtr/Nm"));
    let description = extract_tag(block, "AddtlNtryInf")
        .or_else(|| extract_tag(block, "Ustrd"))
        .or_else(|| extract_tag(block, "Desc"));
    let iban_counter =
        extract_tag(block, "CdtrAcct/Id/IBAN").or_else(|| extract_tag(block, "DbtrAcct/Id/IBAN"));
    let bank_ref = extract_tag(block, "AcctSvcrRef");
    // DBIT = money out (negative), CRDT = money in (positive)
    let direction = extract_tag(block, "CdtDbtInd").unwrap_or_default();
    let signed_cents = if direction == "DBIT" {
        -amount_cents
    } else {
        amount_cents
    };

    Some(BankTx {
        date,
        amount_cents: signed_cents,
        counterparty,
        description,
        iban_counter,
        bank_ref,
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
}
