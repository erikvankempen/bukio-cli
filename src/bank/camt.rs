//! CAMT.053 (ISO 20022 bank statement) parser.

use crate::error::{AppError, Result};
use roxmltree::Document;

/// A parsed bank transaction (sign: CRDT positive, DBIT negative).
#[derive(Debug, Clone)]
pub struct BankTx {
    pub date: String,
    pub amount_cents: i64,
    pub counterparty: Option<String>,
    pub description: Option<String>,
    pub iban_counter: Option<String>,
    pub iban: Option<String>,
}

pub fn bank_error(code: &'static str, message: impl Into<String>) -> AppError {
    AppError::new(code, message)
}

/// Parse a decimal string to integer cents (CAMT uses '.' decimals).
fn cents(value: Option<&str>) -> Option<i64> {
    let s = value?.trim();
    if s.is_empty() {
        return None;
    }
    let n: f64 = s.parse().ok()?;
    if !n.is_finite() {
        return None;
    }
    Some((n * 100.0).round() as i64)
}

fn text(node: Option<&roxmltree::Node>) -> Option<String> {
    let n = node?;
    if n.is_text() {
        return Some(n.text()?.to_string());
    }
    n.text().map(|s| s.to_string())
}

fn iso_date(node: Option<&roxmltree::Node>) -> Option<String> {
    let s = text(node)?;
    Some(s.chars().take(10).collect()) // strip timezone offset
}

fn first_child<'input, 'doc>(
    node: &roxmltree::Node<'input, 'doc>,
    name: &str,
) -> Option<roxmltree::Node<'input, 'doc>> {
    node.children()
        .find(|c| c.is_element() && c.tag_name().name() == name)
}

/// Parse CAMT.053 XML into transactions:
/// [{ date, amount_cents, counterparty, description, iban_counter, iban }]
/// amount sign: CRDT = positive (money in), DBIT = negative (money out).
pub fn parse_camt053(xml_text: &str) -> Result<Vec<BankTx>> {
    let doc = Document::parse(xml_text).map_err(|e| bank_error("INVALID_CAMT", format!("could not parse CAMT.053 XML: {e}")))?;

    let mut stmt_nodes: Vec<roxmltree::Node> = Vec::new();
    for n in doc.descendants().filter(|n| n.is_element() && n.tag_name().name() == "Stmt") {
        stmt_nodes.push(n);
    }
    if stmt_nodes.is_empty() {
        return Err(bank_error("INVALID_CAMT", "no BkToCstmrStmt/Stmt found in the XML"));
    }

    let mut transactions: Vec<BankTx> = Vec::new();
    for s in stmt_nodes {
        let acct = first_child(&s, "Acct");
        let iban = acct.as_ref().and_then(|a| first_child(a, "Id")).and_then(|i| first_child(&i, "IBAN")).and_then(|n| text(Some(&n)));
        let Some(iban) = iban else {
            return Err(bank_error("INVALID_CAMT", "statement is missing Acct/Id/IBAN"));
        };

        let entries: Vec<roxmltree::Node> = s.children().filter(|c| c.is_element() && c.tag_name().name() == "Ntry").collect();
        if entries.is_empty() {
            continue;
        }
        for ntry in entries {
            let amount = cents(ntry.children().find(|c| c.is_element() && c.tag_name().name() == "Amt").and_then(|n| n.text()));
            let Some(amount) = amount else { continue };
            let direction = ntry
                .children()
                .find(|c| c.is_element() && c.tag_name().name() == "CdtDbtInd")
                .and_then(|n| n.text())
                .unwrap_or("")
                .to_uppercase();

            let bookg = first_child(&ntry, "BookgDt");
            let val = first_child(&ntry, "ValDt");
            let date = iso_date(bookg.as_ref().and_then(|b| first_child(b, "Dt")).as_ref())
                .or_else(|| iso_date(val.as_ref().and_then(|v| first_child(v, "Dt")).as_ref()))
                .or_else(|| iso_date(bookg.as_ref().and_then(|b| first_child(b, "Dbt")).as_ref()));

            // TxDtls may be missing or multiple; fall back to one empty tx
            let tx_dtls: Vec<roxmltree::Node> = ntry
                .children()
                .find(|c| c.is_element() && c.tag_name().name() == "NtryDtls")
                .map(|nd| nd.children().filter(|c| c.is_element() && c.tag_name().name() == "TxDtls").collect())
                .unwrap_or_default();
            let tx_list: Vec<Option<roxmltree::Node>> = if tx_dtls.is_empty() { vec![None] } else { tx_dtls.into_iter().map(Some).collect() };

            for tx in tx_list {
                let rltd = tx.as_ref().and_then(|t| first_child(t, "RltdPties"));
                let cdtr = rltd.as_ref().and_then(|r| first_child(r, "Cdtr"));
                let dbtr = rltd.as_ref().and_then(|r| first_child(r, "Dbtr"));
                let cdtr_acct = cdtr.as_ref().and_then(|c| first_child(c, "Acct")).and_then(|a| first_child(&a, "IBAN"));
                let dbtr_acct = dbtr.as_ref().and_then(|d| first_child(d, "Acct")).and_then(|a| first_child(&a, "IBAN"));

                // counterparty: the other party — creditor when money goes out, debtor when money comes in
                let counterparty = if direction == "DBIT" {
                    cdtr.as_ref().and_then(|c| first_child(c, "Nm")).and_then(|n| text(Some(&n)))
                } else {
                    dbtr.as_ref().and_then(|d| first_child(d, "Nm")).and_then(|n| text(Some(&n)))
                }
                .or_else(|| dbtr.as_ref().and_then(|d| first_child(d, "Nm")).and_then(|n| text(Some(&n))))
                .or_else(|| cdtr.as_ref().and_then(|c| first_child(c, "Nm")).and_then(|n| text(Some(&n))));

                let iban_counter = if direction == "DBIT" {
                    cdtr_acct.as_ref().and_then(|n| text(Some(n)))
                } else {
                    dbtr_acct.as_ref().and_then(|n| text(Some(n)))
                };

                let ustrd = tx.as_ref().and_then(|t| first_child(t, "RmtInf"));
                let description = match ustrd {
                    Some(u) => {
                        let all: Vec<String> = u
                            .children()
                            .filter(|n| n.is_element() && n.tag_name().name() == "Ustrd")
                            .filter_map(|n| n.text())
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .collect();
                        if all.is_empty() {
                            None
                        } else {
                            Some(all.join(" "))
                        }
                    }
                    None => ntry
                        .children()
                        .find(|c| c.is_element() && c.tag_name().name() == "AddtlNtryInf")
                        .and_then(|n| text(Some(&n))),
                };

                let signed_amount = if direction == "DBIT" { -amount } else { amount };
                transactions.push(BankTx {
                    date: date.clone().unwrap_or_default(),
                    amount_cents: signed_amount,
                    counterparty,
                    description: description.or_else(|| Some(String::new())).filter(|s| !s.is_empty()),
                    iban_counter: iban_counter.or_else(|| Some(String::new())).filter(|s| !s.is_empty()),
                    iban: Some(iban.clone()),
                });
            }
        }
    }

    if transactions.is_empty() {
        return Err(bank_error("EMPTY_STATEMENT", "no transactions found in the CAMT.053 statement"));
    }
    Ok(transactions)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CAMT: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Document xmlns="urn:iso:std:iso:20022:tech:xsd:camt.053.001.08">
  <BkToCstmrStmt>
    <Stmt>
      <Acct><Id><IBAN>NL91ABNA0417164300</IBAN></Id></Acct>
      <Ntry>
        <Amt Ccy="EUR">1210.00</Amt>
        <CdtDbtInd>CRDT</CdtDbtInd>
        <BookgDt><Dt>2026-07-01</Dt></BookgDt>
        <NtryDtls><TxDtls>
          <RltdPties><Dbtr><Nm>Klant BV</Nm><Acct><IBAN>NL12INGB0001234567</IBAN></Acct></Dbtr></RltdPties>
          <RmtInf><Ustrd>Factuur 2026-0001</Ustrd></RmtInf>
        </TxDtls></NtryDtls>
      </Ntry>
      <Ntry>
        <Amt Ccy="EUR">363.00</Amt>
        <CdtDbtInd>DBIT</CdtDbtInd>
        <ValDt><Dt>2026-07-02</Dt></ValDt>
        <NtryDtls><TxDtls>
          <RltdPties><Cdtr><Nm>Leverancier</Nm></Cdtr></RltdPties>
          <RmtInf><Ustrd>Inkoop</Ustrd><Ustrd>juli</Ustrd></RmtInf>
        </TxDtls></NtryDtls>
      </Ntry>
    </Stmt>
  </BkToCstmrStmt>
</Document>"#;

    #[test]
    fn parses_crdt_and_dbit() {
        let txs = parse_camt053(CAMT).unwrap();
        assert_eq!(txs.len(), 2);
        assert_eq!(txs[0].amount_cents, 121000);
        assert_eq!(txs[0].iban.as_deref(), Some("NL91ABNA0417164300"));
        assert_eq!(txs[0].iban_counter.as_deref(), Some("NL12INGB0001234567"));
        assert_eq!(txs[0].counterparty.as_deref(), Some("Klant BV"));
        assert_eq!(txs[0].description.as_deref(), Some("Factuur 2026-0001"));
        assert_eq!(txs[1].amount_cents, -36300);
        assert_eq!(txs[1].counterparty.as_deref(), Some("Leverancier"));
        assert_eq!(txs[1].description.as_deref(), Some("Inkoop juli"));
    }

    #[test]
    fn rejects_non_camt() {
        let err = parse_camt053("<foo/>").unwrap_err();
        assert_eq!(err.code(), "INVALID_CAMT");
    }

    #[test]
    fn rejects_empty_statement() {
        let err = parse_camt053("<Document><BkToCstmrStmt><Stmt><Acct><Id><IBAN>NL91ABNA0417164300</IBAN></Id></Acct></Stmt></BkToCstmrStmt></Document>")
            .unwrap_err();
        assert_eq!(err.code(), "EMPTY_STATEMENT");
    }
}
