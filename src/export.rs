// bukio-cli — agent-first double-entry bookkeeping for SMEs.
// Copyright (c) 2026 Erik van Kempen.
// SPDX-License-Identifier: Apache-2.0
//
// Export — Auditfile Financieel (XAF) 4.0 + FAIA 2.01 (Luxembourg).

use crate::accounts::resolve_profile;
use crate::audit::{record, RecordArgs};
use crate::money::{format_amount, BukioError, Result};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;

fn export_error(code: &'static str, msg: impl Into<String>) -> BukioError {
    BukioError::new(code, msg.into())
}

fn sql_err(e: rusqlite::Error) -> BukioError {
    BukioError::new("DB_ERROR", e.to_string())
}

fn esc(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(c, '\x00'..='\x08' | '\x0B' | '\x0C' | '\x0E'..='\x1F'))
        .map(|c| match c {
            '&' => "&amp;".to_string(),
            '<' => "&lt;".to_string(),
            '>' => "&gt;".to_string(),
            '"' => "&quot;".to_string(),
            _ => c.to_string(),
        })
        .collect()
}

fn rekening_soort(acct_type: &str) -> &str {
    if matches!(acct_type, "asset" | "liability" | "equity") {
        "Balans"
    } else {
        "Winst en Verlies"
    }
}

fn faia_account_type(acct_type: &str) -> &str {
    match acct_type {
        "asset" => "Actif",
        "liability" | "equity" => "Passif",
        "income" => "Produit",
        _ => "Charge",
    }
}

fn balance_el(side: &str, cents: i64) -> String {
    let amt = format_amount(cents);
    if cents >= 0 {
        format!("        <{side}DebitBalance>{amt}</{side}DebitBalance>")
    } else {
        format!(
            "        <{side}CreditBalance>{}</{side}CreditBalance>",
            format_amount(-cents)
        )
    }
}

/// Decompose postings into (rekening, tegenrekening, bedrag) pairs.
/// Each debit leg matched against credit legs — lossless round-trip.
fn to_boekingen(postings: &[(i64, String)]) -> Vec<(String, String, i64)> {
    let mut debits: Vec<(String, i64)> = postings
        .iter()
        .filter(|(_, c)| c.starts_with('+') || !c.starts_with('-'))
        .filter_map(|(amt, code)| {
            if *amt > 0 {
                Some((code.clone(), *amt))
            } else {
                None
            }
        })
        .collect();
    let mut credits: Vec<(String, i64)> = postings
        .iter()
        .filter_map(|(amt, code)| {
            if *amt < 0 {
                Some((code.clone(), -amt))
            } else {
                None
            }
        })
        .collect();
    let mut result = Vec::new();
    let mut ci = 0;
    for (code, cents) in &mut debits {
        let mut remaining = *cents;
        while remaining > 0 && ci < credits.len() {
            let c = &mut credits[ci];
            if c.1 == 0 {
                ci += 1;
                continue;
            }
            let take = remaining.min(c.1);
            result.push((code.clone(), c.0.clone(), take));
            c.1 -= take;
            remaining -= take;
        }
    }
    result
}

/// Export XAF 4.0 (NL) — Auditfile Financieel.
fn build_xaf_40(
    db: &Connection,
    year: &str,
    out: &str,
    actor: &str,
    dry_run: bool,
) -> Result<Value> {
    if year.len() != 4 || !year.chars().all(|c| c.is_ascii_digit()) {
        return Err(export_error(
            "INVALID_YEAR",
            format!("year '{year}' must be YYYY"),
        ));
    }
    let company = db.query_row("SELECT * FROM company WHERE id = 1", [], |r| {
        Ok(json!({"name": r.get::<_, Option<String>>(1)?, "registration_id": r.get::<_, Option<String>>(2)?}))
    }).map_err(|_| export_error("NO_COMPANY", "no company initialised"))?;

    let (fy_from, fy_to) = crate::year_end::fiscal_year_window(db, year)?;

    let accounts = {
        let mut stmt = db
            .prepare("SELECT code, name, type FROM accounts ORDER BY code")
            .map_err(sql_err)?;
        let rows: Vec<(String, String, String)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map_err(sql_err)?
            .filter_map(|r| r.ok())
            .collect();
        rows
    };

    let entries = {
        let mut stmt = db.prepare("SELECT e.id, e.date, e.description, e.state FROM journal_entries e WHERE e.date >= ?1 AND e.date <= ?2 ORDER BY e.date, e.id").map_err(sql_err)?;
        let rows: Vec<(i64, String, String, String)> = stmt
            .query_map(rusqlite::params![fy_from, fy_to], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })
            .map_err(sql_err)?
            .filter_map(|r| r.ok())
            .collect();
        rows
    };

    let postings = {
        let mut stmt = db.prepare("SELECT p.entry_id, p.amount_cents, a.code AS account_code FROM postings p JOIN accounts a ON a.id = p.account_id JOIN journal_entries e ON e.id = p.entry_id WHERE e.date >= ?1 AND e.date <= ?2 ORDER BY p.id").map_err(sql_err)?;
        let rows: Vec<(i64, i64, String)> = stmt
            .query_map(rusqlite::params![fy_from, fy_to], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .map_err(sql_err)?
            .filter_map(|r| r.ok())
            .collect();
        rows
    };

    let mut by_entry: HashMap<i64, Vec<(i64, String)>> = HashMap::new();
    for (eid, amt, code) in &postings {
        by_entry.entry(*eid).or_default().push((*amt, code.clone()));
    }

    let posted: Vec<&(i64, String, String, String)> =
        entries.iter().filter(|e| e.3 == "posted").collect();
    if posted.is_empty() {
        return Err(export_error(
            "EXPORT_EMPTY_YEAR",
            format!("no posted entries in fiscal year {year}"),
        ));
    }

    let version = env!("CARGO_PKG_VERSION");

    if dry_run {
        return Ok(
            json!({"ok": true, "path": out, "year": year, "dryRun": true,
            "company": {"name": company["name"], "registration_id": company["registration_id"]},
            "rekeningen": accounts.len(), "mutaties": posted.len()}),
        );
    }

    let mut xml = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Xaf xmlns=\"http://www.auditfiles.nl/XAF/4.0\">\n  <XafHeader>\n    <Version>4.0</Version>\n");
    xml.push_str(&format!(
        "    <CompanyName>{}</CompanyName>\n",
        esc(company["name"].as_str().unwrap_or(""))
    ));
    xml.push_str(&format!(
        "    <CompanyID>{}</CompanyID>\n",
        esc(company["registration_id"].as_str().unwrap_or(""))
    ));
    xml.push_str(&format!("    <FiscalYear>{year}</FiscalYear>\n    <StartDate>{fy_from}</StartDate>\n    <EndDate>{fy_to}</EndDate>\n    <SoftwareName>bukio-cli</SoftwareName>\n    <SoftwareVersion>{version}</SoftwareVersion>\n  </XafHeader>\n"));

    xml.push_str("  <Rekeningen>\n");
    for (code, name, acct_type) in &accounts {
        xml.push_str(&format!("    <Rekening>\n      <RekeningCode>{}</RekeningCode>\n      <RekeningOmschrijving>{}</RekeningOmschrijving>\n      <RekeningSoort>{}</RekeningSoort>\n    </Rekening>\n", esc(code), esc(name), rekening_soort(acct_type)));
    }
    xml.push_str("  </Rekeningen>\n  <Mutaties>\n");

    let mut mutatie_ids = Vec::new();
    for (eid, date, desc, _) in &posted {
        let rows = by_entry.get(eid).cloned().unwrap_or_default();
        if rows.is_empty() {
            continue;
        }
        mutatie_ids.push(eid);
        let oms = if desc.is_empty() {
            format!("Boeking {eid}")
        } else {
            desc.clone()
        };
        // <Datum> is the entry's date — the loop used to ignore it and write the
        // first 10 chars of the description, so every exported boekstuk carried
        // a garbage date (and the round-trip failed INVALID_DATE)
        let datum = &date[..10.min(date.len())];
        let boekingen = to_boekingen(&rows);
        xml.push_str(&format!("    <Mutatie>\n      <Boekstuknummer>{eid}</Boekstuknummer>\n      <Datum>{}</Datum>\n      <Omschrijving>{}</Omschrijving>\n      <Boekingen>\n", datum, esc(&oms)));
        for (rek, tegen, bedrag) in &boekingen {
            xml.push_str(&format!("        <Boeking>\n          <RekeningCode>{}</RekeningCode>\n          <TegenrekeningCode>{}</TegenrekeningCode>\n          <Bedrag>{}</Bedrag>\n          <Omschrijving>{}</Omschrijving>\n        </Boeking>\n", esc(rek), esc(tegen), format_amount(*bedrag), esc(&oms)));
        }
        xml.push_str("      </Boekingen>\n    </Mutatie>\n");
    }
    xml.push_str("  </Mutaties>\n</Xaf>\n");

    fs::write(out, &xml).map_err(|e| export_error("WRITE_FAILED", e.to_string()))?;
    record(
        db,
        RecordArgs {
            actor,
            action: "export.xaf",
            command: Some("export xaf"),
            args: Some(json!({"year": year, "out": out})),
            outcome: "ok",
            entry_ids: vec![],
        },
    )?;
    Ok(
        json!({"ok": true, "path": out, "year": year, "company": {"name": company["name"], "registration_id": company["registration_id"]}, "rekeningen": accounts.len(), "mutaties": mutatie_ids.len()}),
    )
}

/// Export FAIA 2.01 reduced B (Luxembourg).
fn build_faia_b(
    db: &Connection,
    year: &str,
    out: &str,
    actor: &str,
    dry_run: bool,
) -> Result<Value> {
    if year.len() != 4 || !year.chars().all(|c| c.is_ascii_digit()) {
        return Err(export_error(
            "INVALID_YEAR",
            format!("year '{year}' must be YYYY"),
        ));
    }
    let company = db
        .query_row("SELECT * FROM company WHERE id = 1", [], |r| {
            Ok(json!({
                "name": r.get::<_, Option<String>>(1)?,
                "registration_id": r.get::<_, Option<String>>(2)?,
                "tax_id": r.get::<_, Option<String>>(4)?,
                "address": r.get::<_, Option<String>>(11)?,
                "city": r.get::<_, Option<String>>(13)?,
                "postal_code": r.get::<_, Option<String>>(12)?,
                "country": r.get::<_, Option<String>>(16)?,
                "base_currency": r.get::<_, Option<String>>(17)?,
            }))
        })
        .map_err(|_| export_error("NO_COMPANY", "no company initialised"))?;

    let sel_from = format!("{year}-01-01");
    let sel_to = format!("{year}-12-31");

    let accounts: Vec<(String, String, String)> = db
        .prepare("SELECT code, name, type FROM accounts ORDER BY code")
        .map_err(sql_err)?
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .map_err(sql_err)?
        .filter_map(|r| r.ok())
        .collect();

    let entries: Vec<(i64, String, String, String, Option<String>)> = db.prepare("SELECT e.id, e.date, e.description, e.state, e.posted_at FROM journal_entries e WHERE e.date >= ?1 AND e.date <= ?2 ORDER BY e.date, e.id").map_err(sql_err)?
        .query_map(rusqlite::params![sel_from, sel_to], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, Option<String>>(4)?)))
        .map_err(sql_err)?.filter_map(|r| r.ok()).collect();

    let postings: Vec<(i64, i64, i64, String)> = db.prepare("SELECT p.id, p.entry_id, p.amount_cents, a.code AS account_code FROM postings p JOIN accounts a ON a.id = p.account_id JOIN journal_entries e ON e.id = p.entry_id WHERE e.date >= ?1 AND e.date <= ?2 ORDER BY p.id").map_err(sql_err)?
        .query_map(rusqlite::params![sel_from, sel_to], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?, r.get::<_, String>(3)?)))
        .map_err(sql_err)?.filter_map(|r| r.ok()).collect();

    let mut by_entry: HashMap<i64, Vec<(i64, i64, String)>> = HashMap::new();
    for (pid, eid, amt, code) in &postings {
        by_entry
            .entry(*eid)
            .or_default()
            .push((*pid, *amt, code.clone()));
    }

    let posted: Vec<_> = entries.iter().filter(|e| e.3 == "posted").collect();
    if posted.is_empty() {
        return Err(export_error(
            "EXPORT_EMPTY_YEAR",
            format!("no posted entries in {year}"),
        ));
    }

    // Balances: opening = before 01-01, closing = through 12-31 (JS parity)
    let balances: Vec<(String, i64, i64)> = db
        .prepare("SELECT a.code, COALESCE(SUM(CASE WHEN e.date < ?1 THEN p.amount_cents END), 0), COALESCE(SUM(CASE WHEN e.date <= ?2 THEN p.amount_cents END), 0) FROM accounts a LEFT JOIN postings p ON p.account_id = a.id LEFT JOIN journal_entries e ON e.id = p.entry_id AND e.state = 'posted' GROUP BY a.code")
        .map_err(sql_err)?
        .query_map(rusqlite::params![sel_from, sel_to], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?)))
        .map_err(sql_err)?
        .filter_map(|r| r.ok())
        .collect();
    let bal_map: HashMap<String, (i64, i64)> = balances
        .into_iter()
        .map(|(k, v1, v2)| (k, (v1, v2)))
        .collect();

    let version = env!("CARGO_PKG_VERSION");

    if dry_run {
        return Ok(
            json!({"ok": true, "path": out, "year": year, "dryRun": true,
            "company": {"name": company["name"], "registration_id": company["registration_id"]},
            "rekeningen": accounts.len(), "mutaties": posted.len()}),
        );
    }

    let mut xml = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<AuditFile>\n  <Header>\n    <AuditFileVersion>2.01</AuditFileVersion>\n    <AuditFileCountry>LU</AuditFileCountry>\n");
    xml.push_str(&format!("    <AuditFileDateCreated>{}</AuditFileDateCreated>\n    <SoftwareCompanyName>bukio-cli</SoftwareCompanyName>\n    <SoftwareID>bukio-cli</SoftwareID>\n    <SoftwareVersion>{version}</SoftwareVersion>\n", crate::dates::today_iso()));
    xml.push_str("    <Company>\n");
    xml.push_str(&format!(
        "      <RegistrationNumber>{}</RegistrationNumber>\n",
        esc(company["registration_id"].as_str().unwrap_or(""))
    ));
    xml.push_str(&format!(
        "      <Name>{}</Name>\n",
        esc(company["name"].as_str().unwrap_or(""))
    ));
    xml.push_str(&format!("      <Address>\n        <StreetName>{}</StreetName>\n        <City>{}</City>\n        <PostalCode>{}</PostalCode>\n        <Country>LU</Country>\n      </Address>\n", esc(company["address"].as_str().unwrap_or("")), esc(company["city"].as_str().unwrap_or("")), esc(company["postal_code"].as_str().unwrap_or(""))));
    xml.push_str(&format!("      <Contact>\n        <ContactPerson><FirstName>{}</FirstName><LastName></LastName></ContactPerson>\n        <Telephone></Telephone>\n      </Contact>\n", esc(company["name"].as_str().unwrap_or(""))));
    if let Some(tid) = company["tax_id"].as_str() {
        xml.push_str(&format!("      <TaxRegistration>\n        <TaxRegistrationNumber>{}</TaxRegistrationNumber>\n        <TaxType>TVA</TaxType>\n        <TaxNumber>{}</TaxNumber>\n      </TaxRegistration>\n", esc(tid), esc(tid)));
    }
    xml.push_str("    </Company>\n");
    xml.push_str(&format!(
        "    <DefaultCurrencyCode>{}</DefaultCurrencyCode>\n",
        esc(company["base_currency"].as_str().unwrap_or("EUR"))
    ));
    xml.push_str(&format!("    <SelectionCriteria>\n      <SelectionStartDate>{sel_from}</SelectionStartDate>\n      <SelectionEndDate>{sel_to}</SelectionEndDate>\n    </SelectionCriteria>\n    <TaxAccountingBasis>Invoice Accounting</TaxAccountingBasis>\n  </Header>\n"));

    xml.push_str("  <MasterFiles>\n    <GeneralLedgerAccounts>\n");
    for (code, name, acct_type) in &accounts {
        let (opening, closing) = bal_map.get(code).copied().unwrap_or((0, 0));
        xml.push_str(&format!("      <Account>\n        <AccountID>{}</AccountID>\n        <AccountDescription>{}</AccountDescription>\n        <AccountType>{}</AccountType>\n{}\n{}\n      </Account>\n", esc(code), esc(name), faia_account_type(acct_type), balance_el("Opening", opening), balance_el("Closing", closing)));
    }
    xml.push_str("    </GeneralLedgerAccounts>\n");

    // TaxTable
    if company["tax_id"].is_string() {
        let profile = resolve_profile(db)?;
        if let Some(codes) = profile["tax"]["codes"].as_array() {
            xml.push_str("    <TaxTable>\n      <TaxTableEntry>\n        <TaxType>TVA</TaxType>\n        <Description>Taxe sur la valeur ajoutée</Description>\n");
            for c in codes {
                xml.push_str(&format!("        <TaxCodeDetails>\n          <TaxCode>{}</TaxCode>\n          <Description>{}</Description>\n", esc(c["code"].as_str().unwrap_or("")), esc(c["description"].as_str().unwrap_or(""))));
                if c["type"] == "standard" {
                    let rate = c["rateBp"].as_f64().unwrap_or(0.0) / 100.0;
                    xml.push_str(&format!(
                        "          <TaxPercentage>{:.2}</TaxPercentage>\n",
                        rate
                    ));
                }
                xml.push_str("          <Country>LU</Country>\n        </TaxCodeDetails>\n");
            }
            xml.push_str("      </TaxTableEntry>\n    </TaxTable>\n");
        }
    }
    xml.push_str("  </MasterFiles>\n");

    let mut total_debit: i64 = 0;
    let mut total_credit: i64 = 0;
    let mut all_lines: Vec<(i64, i64, String, String)> = Vec::new();
    for (eid, _date, desc, _state, _posted) in &posted {
        let rows = by_entry.get(eid).cloned().unwrap_or_default();
        if rows.is_empty() {
            continue;
        }
        for (pid, amt, code) in &rows {
            all_lines.push((*pid, *amt, code.clone(), desc.clone()));
        }
    }
    for (_, amt, _, _) in &all_lines {
        if *amt > 0 {
            total_debit += amt;
        } else {
            total_credit += -amt;
        }
    }

    xml.push_str("  <GeneralLedgerEntries>\n");
    xml.push_str(&format!(
        "    <NumberOfEntries>{}</NumberOfEntries>\n",
        posted.len()
    ));
    xml.push_str(&format!(
        "    <TotalDebit>{}</TotalDebit>\n    <TotalCredit>{}</TotalCredit>\n",
        format_amount(total_debit),
        format_amount(total_credit)
    ));
    xml.push_str("    <Journal>\n      <JournalID>1</JournalID>\n      <Description>Journal général</Description>\n      <Type>GR</Type>\n");

    for (eid, date, desc, _state, posted_at) in &posted {
        let rows = by_entry.get(eid).cloned().unwrap_or_default();
        if rows.is_empty() {
            continue;
        }
        let d = &date[..10.min(date.len())];
        let mm: i64 = if d.len() >= 7 {
            d[5..7].parse().unwrap_or(1)
        } else {
            1
        };
        let posted_date = posted_at
            .as_deref()
            .map(|p| p[..10.min(p.len())].to_string())
            .unwrap_or_else(|| d.to_string());
        let desc_str = if desc.is_empty() {
            format!("Écriture {eid}")
        } else {
            desc.clone()
        };
        xml.push_str(&format!("      <Transaction>\n        <TransactionID>{eid}</TransactionID>\n        <Period>{mm}</Period>\n        <PeriodYear>{}</PeriodYear>\n        <TransactionDate>{d}</TransactionDate>\n        <Description>{}</Description>\n        <SystemEntryDate>{posted_date}</SystemEntryDate>\n        <GLPostingDate>{}</GLPostingDate>\n", &d[..4.min(d.len())], esc(&desc_str), posted_date));
        for (pid, amt, code) in &rows {
            xml.push_str("        <Line>\n");
            xml.push_str(&format!("          <RecordID>{pid}</RecordID>\n          <AccountID>{}</AccountID>\n          <Description>{}</Description>\n", esc(code), esc(&desc_str)));
            if *amt > 0 {
                xml.push_str(&format!(
                    "          <DebitAmount><Amount>{}</Amount></DebitAmount>\n",
                    format_amount(*amt)
                ));
            } else if *amt < 0 {
                xml.push_str(&format!(
                    "          <CreditAmount><Amount>{}</Amount></CreditAmount>\n",
                    format_amount(-amt)
                ));
            } else {
                xml.push_str("          <DebitAmount><Amount>0.00</Amount></DebitAmount>\n");
            }
            xml.push_str("        </Line>\n");
        }
        xml.push_str("      </Transaction>\n");
    }
    xml.push_str("    </Journal>\n  </GeneralLedgerEntries>\n</AuditFile>\n");

    fs::write(out, &xml).map_err(|e| export_error("WRITE_FAILED", e.to_string()))?;
    record(
        db,
        RecordArgs {
            actor,
            action: "export.xaf",
            command: Some("export xaf"),
            args: Some(json!({"year": year, "out": out})),
            outcome: "ok",
            entry_ids: vec![],
        },
    )?;
    Ok(
        json!({"ok": true, "path": out, "year": year, "company": {"name": company["name"], "registration_id": company["registration_id"]}, "rekeningen": accounts.len(), "mutaties": if all_lines.is_empty() { 0 } else { posted.len() }}),
    )
}

pub fn export_xaf(
    db: &Connection,
    year: &str,
    out: &str,
    actor: &str,
    dry_run: bool,
) -> Result<Value> {
    let profile = resolve_profile(db)?;
    let audit_format = profile["documents"]["auditFile"]
        .as_str()
        .unwrap_or("xaf-auditfile-4.0");
    match audit_format {
        "xaf-auditfile-4.0" => build_xaf_40(db, year, out, actor, dry_run),
        "faia-2.01-reduced-b" => build_faia_b(db, year, out, actor, dry_run),
        _ => Err(export_error(
            "FORMAT_NOT_SUPPORTED",
            format!("audit file format '{audit_format}' has no builder"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Connection {
        let d = crate::db::open_db(":memory:").unwrap();
        d.execute(
            "INSERT INTO company (name, registration_id) VALUES ('ExportCo', '12345')",
            [],
        )
        .unwrap();
        crate::accounts::seed_default_chart(&d).unwrap();
        d
    }

    #[test]
    fn export_xaf_dry_run() {
        let d = test_db();
        // Create and post an entry so there's data
        let e = crate::entries::create_entry(
            &d,
            crate::entries::CreateEntry {
                date: "2026-01-15",
                description: "Test",
                postings: vec![
                    crate::entries::PostingSpec {
                        code: "8000".into(),
                        amount_cents: 10000,
                        cost_center_code: None,
                        vat_code: None,
                        vat_amount_cents: None,
                    },
                    crate::entries::PostingSpec {
                        code: "1000".into(),
                        amount_cents: -10000,
                        cost_center_code: None,
                        vat_code: None,
                        vat_amount_cents: None,
                    },
                ],
                source: "manual",
                source_ref: None,
                actor: "human:erik",
            },
        )
        .unwrap();
        crate::entries::post_entry(&d, e.id, "human:erik").unwrap();

        let result = export_xaf(&d, "2026", "/tmp/test.xaf", "human:erik", true).unwrap();
        assert!(result["dryRun"].as_bool().unwrap_or(false));
    }

    #[test]
    fn to_boekingen_pairs() {
        let postings = vec![(10000, "8000".into()), (-10000, "1000".into())];
        let pairs = to_boekingen(&postings);
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].2, 10000);
    }
}
