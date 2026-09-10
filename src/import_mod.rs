// bukio-cli — agent-first double-entry bookkeeping for SMEs.
// Copyright (c) 2026 Erik van Kempen.
// SPDX-License-Identifier: Apache-2.0
//
// Import — CSV opening balances, journal CSV, contacts from XML.

use crate::accounts::get_account_by_code;
use crate::accounts::{create_account, infer_rgs, NewAccount};
use crate::audit::{record, RecordArgs};
use crate::contacts::create_contact;
use crate::entries::{create_entry, post_entry, CreateEntry, PostingSpec};
use crate::money::{BukioError, Result};
use quick_xml::events::Event;
use quick_xml::Reader;
use rusqlite::Connection;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;

fn import_err(code: &'static str, msg: impl Into<String>) -> BukioError {
    BukioError::new(code, msg.into())
}

fn sql_err(e: rusqlite::Error) -> BukioError {
    BukioError::new("DB_ERROR", e.to_string())
}

fn valid_date(s: &str) -> bool {
    chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok()
}

fn valid_code(s: &str) -> bool {
    !s.is_empty() && s.len() <= 6 && s.bytes().all(|b| b.is_ascii_digit())
}

/// Quote-aware CSV — comma or semicolon, auto-detected from header.
pub fn parse_csv_rows(text: &str) -> Vec<(usize, Vec<String>)> {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return vec![];
    }
    // Detect delimiter from first line
    let fl_end = chars.iter().position(|&c| c == '\n').unwrap_or(chars.len());
    let fl: String = chars[..fl_end].iter().collect();
    let delim = if fl.matches(';').count() > fl.matches(',').count() {
        ';'
    } else {
        ','
    };

    let mut rows = Vec::new();
    let mut i = 0;
    let mut ln = 0;

    while i < chars.len() {
        ln += 1;
        let mut cells = Vec::new();
        let mut cur = String::new();
        let mut in_q = false;

        loop {
            if i >= chars.len() {
                // End of input — flush last cell
                if !cur.is_empty() || !cells.is_empty() {
                    cells.push(cur.trim().to_string());
                }
                break;
            }
            let c = chars[i];
            if in_q {
                if c == '"' {
                    if i + 1 < chars.len() && chars[i + 1] == '"' {
                        cur.push('"');
                        i += 2;
                    } else {
                        in_q = false;
                        i += 1;
                    }
                } else {
                    cur.push(c);
                    i += 1;
                }
            } else if c == '"' {
                in_q = true;
                i += 1;
            } else if c == '\n' || c == '\r' {
                cells.push(cur.trim().to_string());
                i += 1;
                if c == '\r' && i < chars.len() && chars[i] == '\n' {
                    i += 1;
                }
                break; // End of row
            } else if c == delim {
                cells.push(cur.trim().to_string());
                cur = String::new();
                i += 1;
                // Continue to next cell in same row
            } else {
                cur.push(c);
                i += 1;
            }
        }

        if cells.iter().all(String::is_empty) {
            continue;
        }
        rows.push((ln, cells));
    }
    rows
}

/// Parse Dutch/English amount to cents.
pub fn parse_import_amount(input: &str) -> Result<i64> {
    let s = input.trim();
    if s.is_empty() {
        return Err(import_err(
            "INVALID_AMOUNT",
            format!("invalid amount '{input}'"),
        ));
    }
    let normalized = if s.contains(',') && s.contains('.') {
        s.replace('.', "").replace(',', ".")
    } else if s.contains(',') {
        s.replace(',', ".")
    } else {
        s.to_string()
    };
    // Validate: optional minus, digits, optional .digits(1-2)
    let v = normalized.as_str();
    let v = v.strip_prefix('-').unwrap_or(v);
    let valid = match v.find('.') {
        Some(p) => {
            let int_part = &v[..p];
            let dec_part = &v[p + 1..];
            !int_part.is_empty()
                && int_part.bytes().all(|b| b.is_ascii_digit())
                && !dec_part.is_empty()
                && dec_part.len() <= 2
                && dec_part.bytes().all(|b| b.is_ascii_digit())
        }
        None => !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()),
    };
    if !valid {
        return Err(import_err(
            "INVALID_AMOUNT",
            format!("invalid amount '{input}' — use e.g. 1234.56, 1234,56 or 1.234,56"),
        ));
    }
    let val: f64 = normalized
        .parse()
        .map_err(|_| import_err("INVALID_AMOUNT", format!("bad amount '{input}'")))?;
    Ok((val * 100.0).round() as i64)
}

/// Import opening balances from CSV (code,amount or code,debet,credit).
pub fn import_opening_balances(
    db: &Connection,
    csv_text: &str,
    date: Option<&str>,
    actor: &str,
    dry_run: bool,
) -> Result<Value> {
    let today = crate::dates::today_iso();
    let the_date = date.unwrap_or(&today);
    if !valid_date(the_date) {
        return Err(import_err(
            "INVALID_DATE",
            format!("date '{the_date}' must be yyyy-mm-dd"),
        ));
    }
    let already: bool = db
        .query_row(
            "SELECT 1 FROM journal_entries WHERE source='import' AND source_ref='opening-balances' LIMIT 1",
            [],
            |r| r.get::<_, i32>(0),
        )
        .map(|_| true)
        .unwrap_or(false);
    if already {
        return Err(import_err(
            "OPENING_ALREADY_IMPORTED",
            "opening balances already imported",
        ));
    }

    let rows = parse_csv_rows(csv_text);
    if rows.is_empty() {
        return Err(import_err("EMPTY_CSV", "CSV has no data rows"));
    }

    let header = &rows[0].1;
    let is_three = header.len() >= 3
        && (header[1].to_lowercase().contains("debet")
            || header[1].to_lowercase().contains("credit"));
    let data_start = if header.iter().any(|h| {
        matches!(
            h.to_lowercase().as_str(),
            "code" | "account" | "rekening" | "rekeningcode"
        )
    }) {
        1
    } else {
        0
    };

    let mut errors = Vec::new();
    let mut parsed: Vec<(String, i64)> = Vec::new();
    for (ln, cells) in rows.iter().skip(data_start) {
        if cells.len() < 2 || cells[0].is_empty() {
            continue;
        }
        let code = cells[0].trim().to_string();
        if !valid_code(&code) {
            errors.push(format!("line {ln}: INVALID_CODE '{code}'"));
            continue;
        }
        let amt = if is_three && cells.len() >= 3 {
            let d = if cells[1].is_empty() {
                0
            } else {
                parse_import_amount(&cells[1]).unwrap_or(0)
            };
            let c = if cells[2].is_empty() {
                0
            } else {
                parse_import_amount(&cells[2]).unwrap_or(0)
            };
            d - c
        } else {
            parse_import_amount(&cells[1]).unwrap_or(0)
        };
        if amt == 0 {
            errors.push(format!("line {ln}: zero amount for {code}"));
            continue;
        }
        parsed.push((code, amt));
    }
    if parsed.is_empty() {
        return Err(import_err("EMPTY_CSV", "no valid rows"));
    }
    let total: i64 = parsed.iter().map(|(_, a)| a).sum();
    if total != 0 {
        errors.push(format!("sum is {total} — must be zero"));
    }
    for (code, _) in &parsed {
        if get_account_by_code(db, code).is_none() {
            errors.push(format!("ACCOUNT_NOT_FOUND: {code}"));
        }
    }
    if !errors.is_empty() {
        return Err(import_err(
            "IMPORT_VALIDATION_FAILED",
            format!("{} problem(s)", errors.len()),
        ));
    }
    if dry_run {
        return Ok(
            json!({"action": "import opening balances", "accounts": parsed.len(), "date": the_date, "dryRun": true}),
        );
    }

    let postings: Vec<PostingSpec> = parsed
        .iter()
        .map(|(c, a)| PostingSpec {
            code: c.clone(),
            amount_cents: *a,
            cost_center_code: None,
            vat_code: None,
            vat_amount_cents: None,
        })
        .collect();
    let entry = create_entry(
        db,
        CreateEntry {
            date: the_date,
            description: "Opening balances",
            postings,
            source: "import",
            source_ref: Some("opening-balances"),
            actor,
        },
    )?;
    let posted = post_entry(db, entry.id, actor)?;
    record(
        db,
        RecordArgs {
            actor,
            action: "import.opening_balances",
            command: Some("import opening-balances"),
            args: Some(json!({"accounts": parsed.len()})),
            outcome: "ok",
            entry_ids: vec![posted.id],
        },
    )?;
    Ok(
        json!({"ok": true, "imported": true, "entry_id": posted.id, "date": the_date, "accounts": parsed.len()}),
    )
}

/// Import journal CSV (boekstuk-based double-entry).
pub fn import_journal_csv(
    db: &Connection,
    csv_text: &str,
    _create_missing: bool,
    actor: &str,
    dry_run: bool,
) -> Result<Value> {
    let rows = parse_csv_rows(csv_text);
    if rows.is_empty() {
        return Err(import_err("EMPTY_CSV", "no data rows"));
    }
    let header = &rows[0].1;
    let find = |aliases: &[&str]| {
        header.iter().position(|h| {
            let lc = h.to_lowercase();
            aliases.iter().any(|a| lc == *a)
        })
    };
    let cd = find(&["date", "datum"]);
    let cb = find(&["boekstuk", "boekstuknummer", "reference", "ref"]);
    let cr = find(&["rekening", "account", "code"]);
    let ct = find(&["tegenrekening", "contra_account", "contra"]);
    let ca = find(&["bedrag", "amount", "amount_cents"]);
    let cdesc = find(&["omschrijving", "description", "desc"]);
    let _cbtw = find(&["btwcode", "vat_code", "vat"]);

    let req = [
        ("date", cd),
        ("boekstuk", cb),
        ("rekening", cr),
        ("tegenrekening", ct),
        ("bedrag", ca),
    ];
    let missing: Vec<&str> = req
        .iter()
        .filter(|(_, c)| c.is_none())
        .map(|(n, _)| *n)
        .collect();
    if !missing.is_empty() {
        return Err(import_err(
            "INVALID_CSV_HEADER",
            format!("missing: {}", missing.join(", ")),
        ));
    }

    let g = |cells: &[String], col: Option<usize>| {
        col.and_then(|c| cells.get(c).cloned()).unwrap_or_default()
    };
    let mut errors = Vec::new();
    let mut parsed: Vec<Value> = Vec::new();

    for (ln, cells) in rows.iter().skip(1) {
        let date = g(cells, cd);
        let boek = g(cells, cb);
        let rek = g(cells, cr);
        let teg = g(cells, ct);
        let bed = g(cells, ca);
        if date.is_empty() && boek.is_empty() && rek.is_empty() {
            continue;
        }
        if !valid_date(&date) {
            errors.push(format!("line {ln}: INVALID_DATE '{date}'"));
        }
        if boek.is_empty() {
            errors.push(format!("line {ln}: BOEKSTUK_REQUIRED"));
        }
        if !valid_code(&rek) {
            errors.push(format!("line {ln}: INVALID_CODE '{rek}'"));
        }
        if !valid_code(&teg) {
            errors.push(format!("line {ln}: INVALID_CODE '{teg}'"));
        }
        let bc = match parse_import_amount(&bed) {
            Ok(v) => v,
            Err(e) => {
                errors.push(format!("line {ln}: {e}"));
                0
            }
        };
        if bc == 0 {
            errors.push(format!("line {ln}: amount must be non-zero"));
        }
        parsed.push(json!({"line": ln, "date": date, "boekstuk": boek, "rekening": rek, "tegenrekening": teg, "bedragCents": bc, "omschrijving": g(cells, cdesc)}));
    }
    if parsed.is_empty() {
        return Err(import_err("EMPTY_CSV", "no data after header"));
    }

    let mut groups: Vec<String> = Vec::new();
    let mut by_b: HashMap<String, Vec<&Value>> = HashMap::new();
    for p in &parsed {
        let b = p["boekstuk"].as_str().unwrap_or("");
        if !by_b.contains_key(b) {
            groups.push(b.to_string());
        }
        by_b.entry(b.to_string()).or_default().push(p);
    }
    for b in &groups {
        let ds: std::collections::HashSet<&str> =
            by_b[b].iter().filter_map(|p| p["date"].as_str()).collect();
        if ds.len() > 1 {
            errors.push(format!("DATE_MISMATCH: boekstuk '{b}'"));
        }
    }
    let existing: std::collections::HashSet<String> = db
        .prepare(
            "SELECT source_ref FROM journal_entries WHERE source='import' AND source_ref IS NOT NULL",
        )
        .map_err(sql_err)?
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(sql_err)?
        .filter_map(|r| r.ok())
        .collect();
    if !errors.is_empty() {
        return Err(import_err(
            "IMPORT_VALIDATION_FAILED",
            format!("{} problem(s)", errors.len()),
        ));
    }

    let dupes = groups
        .iter()
        .filter(|b| existing.contains(&format!("journal:{b}")))
        .count();
    if dry_run {
        return Ok(
            json!({"action": "import journal", "boekstukken": groups.len(), "lines": parsed.len(), "duplicates": dupes, "dryRun": true}),
        );
    }

    let mut imported = Vec::new();
    for b in &groups {
        let rf = format!("journal:{b}");
        if existing.contains(&rf) {
            continue;
        }
        let lines = &by_b[b];
        let date = lines[0]["date"].as_str().unwrap_or("");
        let desc = lines
            .iter()
            .find_map(|l| l["omschrijving"].as_str().filter(|s| !s.is_empty()))
            .unwrap_or("B");
        let mut postings = Vec::new();
        for l in lines {
            let rek = l["rekening"].as_str().unwrap_or("");
            let teg = l["tegenrekening"].as_str().unwrap_or("");
            let bed = l["bedragCents"].as_i64().unwrap_or(0);
            postings.push(PostingSpec {
                code: rek.to_string(),
                amount_cents: bed,
                cost_center_code: None,
                vat_code: None,
                vat_amount_cents: None,
            });
            postings.push(PostingSpec {
                code: teg.to_string(),
                amount_cents: -bed,
                cost_center_code: None,
                vat_code: None,
                vat_amount_cents: None,
            });
        }
        let entry = create_entry(
            db,
            CreateEntry {
                date,
                description: desc,
                postings,
                source: "import",
                source_ref: Some(&rf),
                actor,
            },
        )?;
        let posted = post_entry(db, entry.id, actor)?;
        imported.push(json!({"id": posted.id, "date": date, "boekstuk": b}));
    }
    record(
        db,
        RecordArgs {
            actor,
            action: "import.journal",
            command: Some("import journal"),
            args: Some(json!({"boekstukken": imported.len(), "duplicates": dupes})),
            outcome: "ok",
            entry_ids: imported.iter().filter_map(|e| e["id"].as_i64()).collect(),
        },
    )?;
    Ok(
        json!({"ok": true, "imported": imported.len(), "duplicates": dupes, "entries": imported, "dryRun": false}),
    )
}

/// Import contacts from UBL XML.
pub fn import_contacts(
    db: &Connection,
    xml_text: &str,
    actor: &str,
    dry_run: bool,
) -> Result<Value> {
    let mut reader = Reader::from_str(xml_text);
    let mut buf = Vec::new();
    let mut contacts: Vec<HashMap<String, String>> = Vec::new();
    let mut cur: HashMap<String, String> = HashMap::new();
    let mut field = String::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                let t = String::from_utf8_lossy(e.name().as_ref()).to_string();
                match t.as_str() {
                    "Supplier" | "Customer" | "Party" => {
                        cur.clear();
                    }
                    "Name" | "StreetName" | "CityName" | "PostalZone" | "Country" | "CompanyID"
                    | "EndpointID" | "Telephone" => {
                        field = t;
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(e)) => {
                let txt = e.unescape().map(|u| u.to_string()).unwrap_or_default();
                if !field.is_empty() && !txt.trim().is_empty() {
                    cur.insert(field.clone(), txt.trim().to_string());
                }
            }
            Ok(Event::End(e)) => {
                let t = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if matches!(t.as_str(), "Supplier" | "Customer" | "Party") {
                    if let Some(name) = cur.get("Name") {
                        if !name.is_empty() {
                            contacts.push(cur.clone());
                        }
                    }
                    cur.clear();
                }
                field.clear();
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    if contacts.is_empty() {
        return Err(import_err("EMPTY_CONTACTS", "no contacts in XML"));
    }

    let existing: std::collections::HashSet<String> = db
        .prepare("SELECT name FROM contacts")
        .map_err(sql_err)?
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(sql_err)?
        .filter_map(|r| r.ok())
        .map(|n| n.to_lowercase())
        .collect();
    let dupes = contacts
        .iter()
        .filter(|c| existing.contains(&c.get("Name").unwrap_or(&String::new()).to_lowercase()))
        .count();

    if dry_run {
        return Ok(
            json!({"action": "import contacts", "contacts": contacts.len(), "duplicates": dupes, "dryRun": true}),
        );
    }

    let mut count = 0;
    for c in &contacts {
        let default_name = String::new();
        let name = c.get("Name").unwrap_or(&default_name);
        if existing.contains(&name.to_lowercase()) {
            continue;
        }
        let postal = c.get("PostalZone").cloned().unwrap_or_default();
        let street = c.get("StreetName").cloned();
        let city = c.get("CityName").cloned();
        let country = c.get("Country").cloned();
        let kvk = c.get("CompanyID").or_else(|| c.get("EndpointID")).cloned();
        create_contact(
            db,
            name,
            street.as_deref(),
            Some(&postal),
            city.as_deref(),
            country.as_deref(),
            None,
            None,
            kvk.as_deref(),
            None,
            actor,
            false,
        )?;
        count += 1;
    }
    record(
        db,
        RecordArgs {
            actor,
            action: "import.contacts",
            command: Some("import contacts"),
            args: Some(json!({"contacts": count})),
            outcome: "ok",
            entry_ids: vec![],
        },
    )?;
    Ok(json!({"ok": true, "imported": count, "duplicates": dupes, "dryRun": false}))
}

pub fn read_import_file(path: &str) -> Result<String> {
    fs::read_to_string(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            import_err("FILE_NOT_FOUND", format!("'{path}' not found"))
        } else {
            import_err(
                "IMPORT_VALIDATION_FAILED",
                format!("cannot read '{path}': {e}"),
            )
        }
    })
}

// ---------------------------------------------------------------------------
// XAF import (XML Auditfile Financieel 4.0)
// ---------------------------------------------------------------------------

fn valid_version(s: &str) -> bool {
    if s == "4" {
        return true;
    }
    if let Some(rest) = s.strip_prefix("4.") {
        return !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit());
    }
    false
}

/// First element name in the document (the root tag), if any.
fn first_element(xml: &str) -> Option<String> {
    let mut reader = Reader::from_str(xml);
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                return Some(String::from_utf8_lossy(e.name().as_ref()).to_string())
            }
            Ok(Event::Eof) | Err(_) => return None,
            _ => {}
        }
    }
}

/// Import an XAF 4.0 XML file — creates accounts and journal entries.
pub fn import_xaf(db: &Connection, xml_text: &str, actor: &str, dry_run: bool) -> Result<Value> {
    // The AuditFile layout is a different document shape end to end — parse it
    // as a tree and hand off; the streaming reader below only understands the
    // Belastingdienst <Xaf> layout.
    if first_element(xml_text).as_deref() == Some("AuditFile") {
        return import_audit_file_layout(db, &xml_tree(xml_text)?, actor, dry_run);
    }
    use quick_xml::events::Event;
    use quick_xml::Reader;
    use std::collections::HashSet;

    let mut reader = Reader::from_str(xml_text);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();

    // --- Streaming state ---
    let mut root_element = String::new();
    let mut version = String::new();
    let mut company_id = String::new();
    let mut company_name = String::new();
    let mut fiscal_year = String::new();
    let mut tag_stack: Vec<String> = Vec::new();
    let mut in_header = false;
    let mut in_rekeningen = false;
    let mut in_mutatie = false;
    let mut in_boeking = false;

    // Rekeningen
    let mut file_codes: HashSet<String> = HashSet::new();
    let mut seen_rekening_codes: HashSet<String> = HashSet::new();
    let mut has_accounts = false;

    // Current mutatie
    let mut cur_boekstuk = String::new();
    let mut cur_date = String::new();
    let mut boekingen_in_mutation = 0i64;

    // Current boeking
    let mut cur_rekening = String::new();
    let mut cur_tegenrekening = String::new();
    let mut cur_bedrag = String::new();

    // Counters
    let mut mutations_count = 0i64;
    let mut errors: Vec<String> = Vec::new();

    // Post-validation
    let mut imported = 0i64;
    let mut accounts_created: Vec<Value> = Vec::new();
    let mut duplicates = 0i64;
    let mut ignored_btw_codes: Vec<String> = Vec::new();
    let mut company_mismatch: Vec<String> = Vec::new();

    // --- Phase 1: Stream-parse + in-stream validation ---
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                let tag = String::from_utf8_lossy(e.name().as_ref()).to_string();
                tag_stack.push(tag.clone());
                match tag.as_str() {
                    "Xaf" | "XAF" | "AuditFile" => {
                        if root_element.is_empty() {
                            root_element = tag;
                        }
                    }
                    "XafHeader" | "Header" => in_header = true,
                    "Rekeningen" => {
                        in_rekeningen = true;
                        has_accounts = true;
                    }
                    "Mutatie" => {
                        in_mutatie = true;
                        mutations_count += 1;
                        cur_boekstuk.clear();
                        cur_date.clear();
                        boekingen_in_mutation = 0;
                    }
                    "Boeking" => {
                        in_boeking = true;
                        boekingen_in_mutation += 1;
                        cur_rekening.clear();
                        cur_tegenrekening.clear();
                        cur_bedrag.clear();
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(t)) => {
                let text = t.unescape().unwrap_or_default().to_string();
                if let Some(tag) = tag_stack.last() {
                    match tag.as_str() {
                        // Header fields
                        "Version" if in_header => version = text,
                        "CompanyID" if in_header => company_id = text,
                        "CompanyName" if in_header => company_name = text,
                        "FiscalYear" if in_header => fiscal_year = text,
                        // Rekeningen section
                        "RekeningCode" if in_rekeningen && !in_mutatie => {
                            let code = text.trim().to_string();
                            if !valid_code(&code) {
                                errors.push(format!(
                                    "INVALID_CODE: rekening code '{code}' must be 1-6 digits"
                                ));
                            } else if seen_rekening_codes.contains(&code) {
                                errors.push(format!(
                                    "DUPLICATE_CODE: rekening {code} appears twice in the audit file"
                                ));
                            } else {
                                seen_rekening_codes.insert(code.clone());
                                file_codes.insert(code);
                            }
                        }
                        // Mutatie fields
                        "Boekstuknummer" if in_mutatie && !in_boeking => {
                            cur_boekstuk = text.trim().to_string();
                        }
                        "Datum" | "Factuurdatum" if in_mutatie && !in_boeking => {
                            cur_date = text.trim().to_string();
                        }
                        // Boeking fields
                        "RekeningCode" if in_boeking => {
                            cur_rekening = text.trim().to_string();
                        }
                        "TegenrekeningCode" if in_boeking => {
                            cur_tegenrekening = text.trim().to_string();
                        }
                        "Bedrag" if in_boeking => {
                            cur_bedrag = text.trim().to_string();
                        }
                        _ => {}
                    }
                }
            }
            Ok(Event::End(e)) => {
                let tag = String::from_utf8_lossy(e.name().as_ref()).to_string();
                tag_stack.pop();
                match tag.as_str() {
                    "XafHeader" | "Header" => in_header = false,
                    "Rekeningen" => in_rekeningen = false,
                    "Boeking" => {
                        let boekstuk_label = if cur_boekstuk.is_empty() {
                            "?"
                        } else {
                            &cur_boekstuk
                        };
                        // Validate RekeningCode
                        for code in [&cur_rekening, &cur_tegenrekening] {
                            let c = code.trim();
                            if c.is_empty() {
                                // TegenrekeningCode can be empty in some XAF files
                                if code == &cur_rekening {
                                    errors.push(format!(
                                        "INVALID_CODE: '' must be 1-6 digits (mutatie '{boekstuk_label}')"
                                    ));
                                }
                                continue;
                            }
                            if !valid_code(c) {
                                errors.push(format!(
                                    "INVALID_CODE: '{c}' must be 1-6 digits (mutatie '{boekstuk_label}')"
                                ));
                            } else if !file_codes.contains(c)
                                && get_account_by_code(db, c).is_none()
                            {
                                errors.push(format!(
                                    "RECORDING_NOT_FOUND: rekening {c} (mutatie '{boekstuk_label}') is not in <Rekeningen> nor in the chart"
                                ));
                            }
                        }
                        // Validate Bedrag
                        match parse_import_amount(&cur_bedrag) {
                            Ok(cents) if cents == 0 => {
                                errors.push(format!(
                                    "INVALID_AMOUNT: mutatie '{boekstuk_label}' bedrag must be non-zero"
                                ));
                            }
                            Err(e) => {
                                errors.push(format!(
                                    "{}: mutatie '{boekstuk_label}' bedrag '{}'",
                                    e.code, &cur_bedrag
                                ));
                            }
                            _ => {}
                        }
                        in_boeking = false;
                    }
                    "Mutatie" => {
                        let boekstuk_label = if cur_boekstuk.is_empty() {
                            "?"
                        } else {
                            &cur_boekstuk
                        };
                        if cur_boekstuk.is_empty() {
                            errors.push(
                                "BOEKSTUK_REQUIRED: every <Mutatie> needs a <Boekstuknummer>"
                                    .into(),
                            );
                        }
                        if cur_date.is_empty() || !valid_date(&cur_date) {
                            errors.push(format!(
                                "INVALID_DATE: mutatie '{boekstuk_label}' date '{}' must be yyyy-mm-dd",
                                if cur_date.is_empty() { "(missing)" } else { &cur_date }
                            ));
                        }
                        if boekingen_in_mutation == 0 {
                            errors.push(format!(
                                "NO_BOEKINGEN: mutatie '{boekstuk_label}' has no <Boeking> rows"
                            ));
                        }
                        in_mutatie = false;
                        boekingen_in_mutation = 0;
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }

    // --- Phase 2: Post-stream validation ---
    // Root element
    if root_element.is_empty() {
        errors.push(
            "root element must be <Xaf> (Belastingdienst) or <AuditFile> (general-ledger export)"
                .into(),
        );
    } else if root_element == "XAF" {
        // <AuditFile> never reaches here — import_audit_file_layout handles it
        errors.push(format!(
            "alternate root <{}> not yet supported — use <Xaf> format",
            root_element
        ));
    }
    // Version
    if !valid_version(&version) {
        errors.push(format!(
            "unsupported audit file version '{}' — expected 4.x",
            if version.is_empty() {
                "(missing)"
            } else {
                &version
            }
        ));
    }
    // Company KVK cross-check
    let company_row: Option<(String, String)> = db
        .prepare("SELECT name, registration_id FROM company WHERE id = 1")
        .ok()
        .and_then(|mut stmt| {
            stmt.query_row([], |row| {
                Ok((
                    row.get::<_, String>(0).unwrap_or_default(),
                    row.get::<_, String>(1).unwrap_or_default(),
                ))
            })
            .ok()
        });
    if let Some((db_name, db_reg)) = company_row {
        let file_kvk = company_id.trim();
        let file_name = company_name.trim();
        if !file_kvk.is_empty() && !db_reg.is_empty() && file_kvk != db_reg {
            errors.push(format!(
                "COMPANY_MISMATCH: audit file is for {file_kvk} ({file_name}), database is for {db_reg} ({db_name})"
            ));
        }
        if !file_kvk.is_empty() && db_reg.is_empty() {
            company_mismatch.push(format!("database has no KVK; file is for {file_kvk}"));
        }
        if !file_name.is_empty() && !db_name.is_empty() && file_name != db_name {
            company_mismatch.push(format!(
                "company name differs: file '{file_name}' vs database '{db_name}'"
            ));
        }
    }
    // Section presence
    if !has_accounts && mutations_count > 0 {
        errors.push("XAF has mutations but no <Rekeningen> section — nothing imported".into());
    }

    // --- Phase 3: Report ---
    if !errors.is_empty() {
        return Err(import_err(
            "IMPORT_VALIDATION_FAILED",
            format!(
                "XAF validation: {} problem(s) — nothing imported",
                errors.len()
            ),
        ));
    }

    if dry_run {
        return Ok(json!({
            "dryRun": true,
            "company": { "name": company_name, "registration_id": company_id },
            "fiscal_year": fiscal_year,
            "rekeningen": file_codes.len() as i64,
            "mutaties": mutations_count,
            "accounts_to_create": 0,
            "accounts_to_rename": [],
            "duplicates": duplicates,
            "ignored_btw_codes": ignored_btw_codes,
            "company_mismatch": company_mismatch,
            "accounts_created": [],
            "accounts_updated": [],
            "accounts_rgs_backfilled": [],
            "chart_warnings": [],
        }));
    }

    // Full import (unchanged stub for now)
    Ok(json!({
        "imported": imported,
        "duplicates": duplicates,
        "accounts_created": accounts_created,
        "accounts_updated": [],
        "accounts_rgs_backfilled": [],
        "ignored_btw_codes": ignored_btw_codes,
        "chart_warnings": [],
    }))
}

// ---------------------------------------------------------------------------
// XML tree + account reconciliation (shared by both XAF layouts)
// ---------------------------------------------------------------------------

/// Minimal XML -> JSON tree: elements become objects, leaves become strings,
/// repeated sibling tags become arrays. Attributes are ignored — no producer we
/// read carries data in them. ponytail: no namespace/attribute/mixed-content
/// handling; add if a real producer needs it.
pub fn xml_tree(xml: &str) -> Result<Value> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut stack: Vec<(String, serde_json::Map<String, Value>)> = Vec::new();
    let mut text = String::new();
    let mut root: Option<Value> = None;
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                stack.push((
                    String::from_utf8_lossy(e.name().as_ref()).to_string(),
                    serde_json::Map::new(),
                ));
                text.clear();
            }
            Ok(Event::Text(t)) => text.push_str(&t.unescape().unwrap_or_default()),
            Ok(Event::End(_)) => {
                let (name, map) = match stack.pop() {
                    Some(x) => x,
                    None => return Err(import_err("INVALID_XAF", "unbalanced XML")),
                };
                let node = if map.is_empty() {
                    Value::String(text.trim().to_string())
                } else {
                    Value::Object(map)
                };
                text.clear();
                if stack.is_empty() {
                    let mut m = serde_json::Map::new();
                    m.insert(name, node);
                    root = Some(Value::Object(m));
                } else {
                    let parent = &mut stack.last_mut().unwrap().1;
                    match parent.get_mut(&name) {
                        Some(existing) => {
                            let prev = std::mem::replace(existing, Value::Null);
                            *existing = match prev {
                                Value::Array(mut a) => {
                                    a.push(node);
                                    Value::Array(a)
                                }
                                other => Value::Array(vec![other, node]),
                            };
                        }
                        None => {
                            parent.insert(name, node);
                        }
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(import_err("INVALID_XAF", format!("cannot parse XML: {e}"))),
            _ => {}
        }
    }
    root.ok_or_else(|| import_err("INVALID_XAF", "empty XML document"))
}

/// Trimmed text of a node ("" for missing/null).
fn xt(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.trim().to_string(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

/// JS `asArray`: null/undefined -> [], array -> as-is, scalar -> [scalar].
fn xa(v: Option<&Value>) -> Vec<Value> {
    match v {
        None | Some(Value::Null) => vec![],
        Some(Value::Array(a)) => a.clone(),
        Some(other) => vec![other.clone()],
    }
}

fn is_audit_version(v: &str) -> bool {
    v == "4" || (v.starts_with("4.") && v.len() > 2 && v[2..].chars().all(|c| c.is_ascii_digit()))
}

fn is_eight_digits(s: &str) -> bool {
    s.len() == 8 && s.chars().all(|c| c.is_ascii_digit())
}

/// Fallback type when the file gives none: net credit movement = income.
fn infer_account_type(net_cents: i64) -> (String, String) {
    if net_cents < 0 {
        ("income".to_string(), "credit".to_string())
    } else {
        ("expense".to_string(), "debit".to_string())
    }
}

/// Reconcile one chart account against the file's version of it: create when
/// missing, backfill an RGS code when the stored one is stale, and rename only
/// when the account carries no postings (otherwise warn). Mirrors JS
/// `syncAccountFromFile` — shared by both XAF layouts.
#[allow(clippy::too_many_arguments)]
fn sync_account_from_file(
    db: &Connection,
    code: &str,
    name: &str,
    type_hint: Option<(String, String)>,
    created: &mut Vec<Value>,
    updated: &mut Vec<Value>,
    warnings: &mut Vec<String>,
    backfilled: &mut Vec<Value>,
) -> Result<()> {
    let clean = if name.trim().is_empty() {
        format!("Rekening {code}")
    } else {
        name.trim().to_string()
    };
    let Some(existing) = get_account_by_code(db, code) else {
        let (t, nb) = type_hint.unwrap_or_else(|| infer_account_type(0));
        let rgs = infer_rgs(&t, &clean);
        let acct = create_account(
            db,
            &NewAccount {
                code,
                name: &clean,
                type_: &t,
                normal_balance: &nb,
                taxonomy_code: rgs,
            },
        )?;
        created.push(json!({
            "code": acct["code"], "name": acct["name"], "type": acct["type"],
            "normal_balance": acct["normal_balance"], "taxonomy_code": acct["taxonomy_code"],
        }));
        return Ok(());
    };

    let ex_name = xt(existing.get("name"));
    let ex_type = xt(existing.get("type"));
    let ex_nb = xt(existing.get("normal_balance"));
    let ex_rgs = existing["taxonomy_code"].as_str().map(|s| s.to_string());
    let want_rgs = infer_rgs(&ex_type, &ex_name);
    if ex_rgs.as_deref() != want_rgs {
        if let Some(rgs) = want_rgs {
            db.execute(
                "UPDATE accounts SET taxonomy_code = ?1 WHERE code = ?2",
                rusqlite::params![rgs, code],
            )
            .map_err(sql_err)?;
            backfilled.push(json!({"code": code, "name": ex_name, "taxonomy_code": rgs}));
        }
    }

    if ex_name.trim().to_lowercase() == clean.to_lowercase() {
        return Ok(());
    }
    let used: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM postings p JOIN accounts a ON a.id = p.account_id WHERE a.code = ?1",
            [code],
            |r| r.get(0),
        )
        .map_err(sql_err)?;
    if used > 0 {
        warnings.push(format!(
            "account {code} stays '{ex_name}' — it already has postings (file calls it '{clean}')"
        ));
        return Ok(());
    }
    let (t, nb) = type_hint.unwrap_or_else(|| (ex_type.clone(), ex_nb.clone()));
    // a chart-authoritative rename also realigns the RGS code (name and type
    // usually change together: 1100 Bank -> Gebouwen, BLIM.10 -> BMVA.02)
    let rgs = infer_rgs(&t, &clean);
    let stored = rgs.map(|s| s.to_string()).or(ex_rgs);
    db.execute(
        "UPDATE accounts SET name = ?1, type = ?2, normal_balance = ?3, taxonomy_code = ?4 WHERE code = ?5",
        rusqlite::params![clean, t, nb, stored, code],
    )
    .map_err(sql_err)?;
    updated.push(json!({
        "code": code, "from": ex_name, "to": clean,
        "type": t, "normal_balance": nb, "taxonomy_code": rgs,
    }));
    Ok(())
}

// ---------------------------------------------------------------------------
// XAF 4.0 AuditFile layout (root <AuditFile>)
// ---------------------------------------------------------------------------
//
// The generic audit-file layout used by general-ledger exports (and by bukio's
// own `export xaf`):
//   Header            (AuditFileVersion, CompanyName, CompanyID, FiscalYear,
//                      StartDate, EndDate, CurrencyCode, SoftwareDescription)
//   MasterFiles/GeneralLedgerAccounts/Account   (AccountID, AccountDescription,
//                      AccountType: Asset|Liability|Equity|Revenue|Expense)
//   GeneralLedgerEntries/Journal[]/Transaction[]  (TransactionID,
//                      TransactionDate, Description; Line[] with AccountID +
//                      DebitAmount | CreditAmount, TaxInformation)
// Every Line is a complete posting on one account (no tegenrekening); the entry
// is the sum of its lines. Tax codes (e.g. NOVAT) are reported, not booked.

const AUDITFILE_ACCOUNT_TYPES: [(&str, &str, &str); 5] = [
    ("asset", "asset", "debit"),
    ("liability", "liability", "credit"),
    ("equity", "equity", "credit"),
    ("revenue", "income", "credit"),
    ("expense", "expense", "debit"),
];

fn auditfile_account_type(raw: &str) -> Option<(String, String)> {
    let key = raw.to_lowercase().trim().to_string();
    AUDITFILE_ACCOUNT_TYPES
        .iter()
        .find(|(k, _, _)| *k == key)
        .map(|(_, t, nb)| (t.to_string(), nb.to_string()))
}

pub fn import_audit_file_layout(
    db: &Connection,
    doc: &Value,
    actor: &str,
    dry_run: bool,
) -> Result<Value> {
    let audit = doc.get("AuditFile").unwrap_or(&Value::Null);
    let header = audit.get("Header").unwrap_or(&Value::Null);
    let version = xt(header.get("AuditFileVersion"));
    if !is_audit_version(&version) {
        let shown = if version.is_empty() {
            "(missing)".to_string()
        } else {
            version.clone()
        };
        return Err(import_err(
            "INVALID_XAF",
            format!("unsupported audit file version '{shown}' — expected 4.0"),
        ));
    }

    // company cross-check: only an 8-digit CompanyID is a plausible KVK; other
    // ids (e.g. the exporting database's row id) fall back to the name check
    let db_company: Option<Value> = db
        .query_row(
            "SELECT name, registration_id FROM company WHERE id = 1",
            [],
            |r| {
                Ok(json!({
                    "name": r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                    "registration_id": r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                }))
            },
        )
        .ok();
    let file_kvk = {
        let v = xt(header.get("CompanyID"));
        if v.is_empty() {
            None
        } else {
            Some(v)
        }
    };
    let file_name = {
        let n = xt(header.get("CompanyName"));
        let n = if n.is_empty() {
            xt(header.get("BusinessName"))
        } else {
            n
        };
        if n.is_empty() {
            None
        } else {
            Some(n)
        }
    };
    let currency_raw = xt(header.get("CurrencyCode"));
    let currency = if currency_raw.is_empty() {
        "EUR".to_string()
    } else {
        currency_raw
    };
    let mut company_mismatch: Vec<String> = Vec::new();
    if let Some(c) = &db_company {
        let db_reg = c["registration_id"].as_str().unwrap_or("").to_string();
        let db_name = c["name"].as_str().unwrap_or("").to_string();
        if let Some(k) = &file_kvk {
            if is_eight_digits(k) && !db_reg.is_empty() && *k != db_reg {
                return Err(import_err(
                    "COMPANY_MISMATCH",
                    format!(
                        "audit file is for {k} ({}), database is for {db_reg} ({db_name})",
                        file_name.clone().unwrap_or_else(|| "unknown".to_string())
                    ),
                ));
            }
        }
        if let Some(n) = &file_name {
            if !db_name.is_empty() && n.to_lowercase() != db_name.to_lowercase() {
                company_mismatch.push(format!(
                    "company name differs: file '{n}' vs database '{db_name}'"
                ));
            }
        }
    }
    if currency != "EUR" {
        company_mismatch.push(format!(
            "currency {currency} — imported amounts are booked as EUR"
        ));
    }

    let accounts = xa(audit
        .get("MasterFiles")
        .and_then(|m| m.get("GeneralLedgerAccounts"))
        .and_then(|g| g.get("Account")));
    let journals = xa(audit
        .get("GeneralLedgerEntries")
        .and_then(|g| g.get("Journal")));
    let mut transactions: Vec<Value> = Vec::new();
    for j in &journals {
        transactions.extend(xa(j.get("Transaction")));
    }

    // --- validate the whole file first ---
    let mut errors: Vec<Value> = Vec::new();
    let mut err = |msg: String| errors.push(json!({"line": 0, "error": msg}));
    let mut seen_codes: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut account_type_by_code: HashMap<String, (String, String)> = HashMap::new();
    for a in &accounts {
        let code = xt(a.get("AccountID"));
        if !valid_code(&code) {
            err(format!("INVALID_CODE: account '{code}' must be 1-6 digits"));
        } else if seen_codes.contains(&code) {
            err(format!(
                "DUPLICATE_CODE: account {code} appears twice in the audit file"
            ));
        } else {
            seen_codes.insert(code.clone());
        }
        if let Some(t) = auditfile_account_type(&xt(a.get("AccountType"))) {
            account_type_by_code.insert(code, t);
        }
    }

    let mut parsed: Vec<Value> = Vec::new();
    let mut tax_codes: Vec<String> = Vec::new();
    for t in &transactions {
        let id = xt(t.get("TransactionID"));
        let date = xt(t.get("TransactionDate"));
        let lines = xa(t.get("Line"));
        if id.is_empty() {
            err("TRANSACTION_REQUIRED: every <Transaction> needs a <TransactionID>".to_string());
        }
        if !valid_date(&date) {
            err(format!(
                "INVALID_DATE: transaction '{}' date '{date}' must be yyyy-mm-dd",
                if id.is_empty() { "?" } else { &id }
            ));
        }
        if lines.is_empty() {
            err(format!(
                "NO_LINES: transaction '{}' has no <Line> rows",
                if id.is_empty() { "?" } else { &id }
            ));
        }
        let label = if id.is_empty() {
            "?".to_string()
        } else {
            id.clone()
        };
        let mut postings: Vec<Value> = Vec::new();
        let mut sum: i64 = 0;
        for ln in &lines {
            let code = xt(ln.get("AccountID"));
            let debit = xt(ln.get("DebitAmount"));
            let credit = xt(ln.get("CreditAmount"));
            if !valid_code(&code) {
                err(format!(
                    "INVALID_CODE: '{code}' must be 1-6 digits (transaction '{label}')"
                ));
            } else if !seen_codes.contains(&code) && get_account_by_code(db, &code).is_none() {
                err(format!(
                    "RECORDING_NOT_FOUND: account {code} (transaction '{label}') is not in the file's accounts nor in the chart"
                ));
            }
            let mut amount_cents: Option<i64> = None;
            if !debit.is_empty() && !credit.is_empty() {
                err(format!(
                    "INVALID_AMOUNT: transaction '{label}' line {code} has both DebitAmount and CreditAmount"
                ));
            } else if debit.is_empty() && credit.is_empty() {
                err(format!(
                    "INVALID_AMOUNT: transaction '{label}' line {code} has no DebitAmount/CreditAmount"
                ));
            } else {
                let raw = if !debit.is_empty() { &debit } else { &credit };
                match parse_import_amount(raw) {
                    Ok(c) => amount_cents = Some(if !debit.is_empty() { c } else { -c }),
                    Err(e) => err(format!(
                        "{}: transaction '{label}' line {code} amount '{raw}'",
                        e.code
                    )),
                }
            }
            let cents = amount_cents.unwrap_or(0);
            if cents == 0 {
                err(format!(
                    "INVALID_AMOUNT: transaction '{label}' line {code} must be non-zero"
                ));
            }
            let tax_info = {
                let ti = xa(ln.get("TaxInformation"));
                ti.into_iter().next().unwrap_or(Value::Null)
            };
            let tax_code = xt(tax_info.get("TaxCode"));
            if !tax_code.is_empty() && !tax_codes.contains(&tax_code) {
                tax_codes.push(tax_code);
            }
            sum += cents;
            postings.push(json!({"code": code, "amount_cents": cents}));
        }
        if sum != 0 {
            err(format!(
                "UNBALANCED: transaction '{label}' lines sum to {} — debet must equal credit",
                crate::money::format_amount(sum)
            ));
        }
        let desc = {
            let d = xt(t.get("Description"));
            let d = if d.is_empty() {
                xt(lines.first().and_then(|l| l.get("Description")))
            } else {
                d
            };
            if d.is_empty() {
                format!("Boekstuk {label}")
            } else {
                d
            }
        };
        parsed.push(json!({"id": id, "date": date, "description": desc, "postings": postings}));
    }

    if !errors.is_empty() {
        return Err(BukioError::with_details(
            "IMPORT_VALIDATION_FAILED",
            format!(
                "XAF file has {} problem(s) — nothing imported",
                errors.len()
            ),
            json!(errors),
        ));
    }

    let mut existing_refs: std::collections::HashSet<String> = {
        let mut stmt = db
            .prepare("SELECT source_ref FROM journal_entries WHERE source = 'xaf' AND source_ref IS NOT NULL")
            .map_err(sql_err)?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(sql_err)?;
        rows.filter_map(|r| r.ok()).collect()
    };

    let duplicates_in_file = parsed
        .iter()
        .filter(|p| existing_refs.contains(p["id"].as_str().unwrap_or("")))
        .count() as i64;
    let fiscal_year = header.get("FiscalYear").cloned().unwrap_or(Value::Null);
    let start_date = {
        let s = xt(header.get("StartDate"));
        if s.is_empty() {
            Value::Null
        } else {
            Value::String(s)
        }
    };
    let end_date = {
        let s = xt(header.get("EndDate"));
        if s.is_empty() {
            Value::Null
        } else {
            Value::String(s)
        }
    };
    let software = {
        let s = xt(header.get("SoftwareDescription"));
        if s.is_empty() {
            Value::Null
        } else {
            Value::String(s)
        }
    };
    let plan = json!({
        "action": "import xaf",
        "company": {
            "name": file_name,
            "registration_id": file_kvk,
            "fiscal_year": fiscal_year,
            "period": format!("{}..{}", xt(header.get("StartDate")), xt(header.get("EndDate"))),
        },
        "software": software,
        "rekeningen": accounts.len(),
        "mutaties": parsed.len(),
        "duplicates": duplicates_in_file,
        "accounts_to_create": accounts
            .iter()
            .filter(|a| get_account_by_code(db, &xt(a.get("AccountID"))).is_none())
            .count(),
        "accounts_to_rename": accounts
            .iter()
            .map(|a| (xt(a.get("AccountID")), xt(a.get("AccountDescription"))))
            .filter(|(code, name)| {
                if name.is_empty() {
                    return false;
                }
                match get_account_by_code(db, code) {
                    Some(e) => xt(e.get("name")).to_lowercase() != name.to_lowercase(),
                    None => false,
                }
            })
            .map(|(code, name)| json!({"code": code, "name": name}))
            .collect::<Vec<_>>(),
        "company_mismatch": company_mismatch,
        "ignored_btw_codes": tax_codes,
        "dryRun": true,
    });
    if dry_run {
        return Ok(plan);
    }

    // net movement per code (file chart + used codes) for type inference
    let mut net: HashMap<String, i64> = HashMap::new();
    for a in &accounts {
        net.insert(xt(a.get("AccountID")), 0);
    }
    for p in &parsed {
        for po in p["postings"].as_array().unwrap_or(&vec![]) {
            let code = po["code"].as_str().unwrap_or("").to_string();
            *net.entry(code).or_insert(0) += po["amount_cents"].as_i64().unwrap_or(0);
        }
    }

    let mut accounts_created: Vec<Value> = Vec::new();
    let mut accounts_updated: Vec<Value> = Vec::new();
    let mut rgs_backfilled: Vec<Value> = Vec::new();
    let mut chart_warnings: Vec<String> = Vec::new();
    let mut imported: Vec<Value> = Vec::new();
    let mut duplicates = 0i64;

    for a in &accounts {
        let code = xt(a.get("AccountID"));
        let name = {
            let n = xt(a.get("AccountDescription"));
            if n.is_empty() {
                format!("Rekening {code}")
            } else {
                n
            }
        };
        let hint = account_type_by_code.get(&code).cloned().or_else(|| {
            // no <AccountType> in the file: infer from the net movement
            Some(infer_account_type(*net.get(&code).unwrap_or(&0)))
        });
        sync_account_from_file(
            db,
            &code,
            &name,
            hint,
            &mut accounts_created,
            &mut accounts_updated,
            &mut chart_warnings,
            &mut rgs_backfilled,
        )?;
    }
    for p in &parsed {
        let id = p["id"].as_str().unwrap_or("").to_string();
        if existing_refs.contains(&id) {
            duplicates += 1;
            continue;
        }
        let postings: Vec<PostingSpec> = p["postings"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .map(|po| PostingSpec {
                code: po["code"].as_str().unwrap_or("").to_string(),
                amount_cents: po["amount_cents"].as_i64().unwrap_or(0),
                cost_center_code: None,
                vat_code: None,
                vat_amount_cents: None,
            })
            .collect();
        let entry = create_entry(
            db,
            CreateEntry {
                date: p["date"].as_str().unwrap_or(""),
                description: p["description"].as_str().unwrap_or(""),
                postings,
                source: "xaf",
                source_ref: Some(id.as_str()),
                actor,
            },
        )?;
        let posted = post_entry(db, entry.id, actor)?;
        existing_refs.insert(id.clone());
        imported.push(json!({
            "id": posted.id, "date": posted.date, "description": posted.description,
            "state": posted.state, "source_ref": id,
        }));
    }

    record(
        db,
        RecordArgs {
            actor,
            action: "import.xaf",
            command: Some("import xaf"),
            args: Some(json!({
                "transactions": parsed.len(), "imported": imported.len(), "duplicates": duplicates,
                "accounts_created": accounts_created.len(), "accounts_updated": accounts_updated.len(),
                "accounts_rgs_backfilled": rgs_backfilled.len(), "layout": "auditfile",
            })),
            outcome: "ok",
            entry_ids: imported.iter().filter_map(|e| e["id"].as_i64()).collect(),
        },
    )?;

    Ok(json!({
        "imported": imported.len(),
        "duplicates": duplicates,
        "entries": imported,
        "accounts_created": accounts_created,
        "accounts_updated": accounts_updated,
        "accounts_rgs_backfilled": rgs_backfilled,
        "chart_warnings": chart_warnings,
        "header": {
            "company_name": file_name,
            "company_registration_id": file_kvk,
            "fiscal_year": fiscal_year,
            "start_date": start_date,
            "end_date": end_date,
            "software": software,
        },
        "company_mismatch": company_mismatch,
        "ignored_btw_codes": tax_codes,
        "dryRun": false,
    }))
}

// ---------------------------------------------------------------------------
// UBL invoice import (EN 16931 / Peppol BIS 3.0)
// ---------------------------------------------------------------------------

/// Import an inbound UBL e-invoice into the payables register.
pub fn import_invoice(
    db: &Connection,
    xml_text: &str,
    contact_id: Option<i64>,
    create_missing: bool,
    actor: &str,
    dry_run: bool,
) -> Result<Value> {
    use quick_xml::events::Event;
    use quick_xml::Reader;

    let mut reader = Reader::from_str(xml_text);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();

    let mut invoice_ref = String::new();
    let mut invoice_date = String::new();
    let mut due_date = String::new();
    let mut supplier_name = String::new();
    let mut supplier_vat_id = String::new();
    let mut total_amount = String::new();
    let mut pay_amount = String::new();

    let mut in_tag = String::new();
    let mut depth = 0i32;
    let mut in_supplier = false;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                let tag = String::from_utf8_lossy(e.name().as_ref()).to_string();
                depth += 1;
                match tag.as_str() {
                    "Invoice" | "CreditNote" => {
                        in_tag = "root".to_string();
                    }
                    "cbc:ID" if depth == 2 => {
                        in_tag = "id".to_string();
                    }
                    "cbc:IssueDate" => {
                        in_tag = "date".to_string();
                    }
                    "cbc:DueDate" => {
                        in_tag = "due".to_string();
                    }
                    "cac:AccountingSupplierParty" => {
                        in_supplier = true;
                    }
                    "cac:AccountingCustomerParty" => {
                        in_supplier = false;
                    }
                    "cbc:Name" if in_supplier => {
                        in_tag = "supplier".to_string();
                    }
                    "cbc:CompanyID" if in_supplier => {
                        in_tag = "supplier_vat".to_string();
                    }
                    "cbc:TaxExclusiveAmount" | "cbc:LineExtensionAmount" => {
                        in_tag = "total".to_string();
                    }
                    "cbc:PayableAmount" => {
                        in_tag = "payable".to_string();
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(t)) => {
                let text = t.unescape().unwrap_or_default().to_string();
                match in_tag.as_str() {
                    "id" => invoice_ref = text,
                    "date" => invoice_date = text,
                    "due" => due_date = text,
                    "supplier" => supplier_name = text,
                    "supplier_vat" => supplier_vat_id = text,
                    "total" => {
                        if total_amount.is_empty() {
                            total_amount = text;
                        }
                    }
                    "payable" => pay_amount = text,
                    _ => {}
                }
            }
            Ok(Event::End(_)) => {
                depth -= 1;
                in_tag.clear();
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }

    let amount_str = if !pay_amount.is_empty() {
        &pay_amount
    } else {
        &total_amount
    };
    let amount_cents = parse_import_amount(amount_str).unwrap_or(0);

    // Default due_date to issue_date + 30 days (EN 16931 BT-9)
    if due_date.is_empty() {
        if let Ok(d) = chrono::NaiveDate::parse_from_str(&invoice_date, "%Y-%m-%d") {
            due_date = (d + chrono::Duration::days(30))
                .format("%Y-%m-%d")
                .to_string();
        }
    }

    // --- contact resolution -------------------------------------------------
    use crate::contacts::{get_contact, list_contacts};

    let supplier_lower = supplier_name.trim().to_lowercase();
    let vat_lower = if supplier_vat_id.is_empty() {
        String::new()
    } else {
        supplier_vat_id.trim().to_lowercase()
    };

    let mut resolved_contact: Option<Value> = None;
    let mut contact_created = false;

    if let Some(cid) = contact_id {
        resolved_contact = get_contact(db, cid)?;
        if resolved_contact.is_none() {
            return Err(import_err(
                "CONTACT_NOT_FOUND",
                format!("contact {cid} does not exist"),
            ));
        }
    } else {
        // match by VAT ID first
        if !vat_lower.is_empty() {
            for c in list_contacts(db)? {
                if c["vat_id"].as_str().unwrap_or("").trim().to_lowercase() == vat_lower {
                    resolved_contact = Some(c);
                    break;
                }
            }
        }
        // fallback: match by normalized name
        if resolved_contact.is_none() {
            for c in list_contacts(db)? {
                if c["name"].as_str().unwrap_or("").trim().to_lowercase() == supplier_lower {
                    resolved_contact = Some(c);
                    break;
                }
            }
        }
        if resolved_contact.is_none() && create_missing {
            contact_created = true;
            if !dry_run {
                let r = create_contact(
                    db,
                    &supplier_name,
                    None, // address
                    None, // postal_code
                    None, // city
                    None, // country
                    None, // email
                    if supplier_vat_id.is_empty() {
                        None
                    } else {
                        Some(&supplier_vat_id)
                    },
                    None, // kvk
                    None, // iban
                    actor,
                    false,
                )?;
                resolved_contact = Some(r);
            }
        }
        if resolved_contact.is_none() && !(create_missing && dry_run) {
            return Err(import_err(
                "CONTACT_NOT_FOUND",
                format!(
                    "no contact matches supplier '{supplier_name}' — pass --contact <id> or --create-missing to create it"
                ),
            ));
        }
    }

    let contact_id_resolved = resolved_contact
        .as_ref()
        .and_then(|c| c["id"].as_i64())
        .unwrap_or(0);
    let contact_name = resolved_contact
        .as_ref()
        .and_then(|c| c["name"].as_str())
        .unwrap_or(&supplier_name)
        .to_string();
    let contacts_created_count: i64 = if contact_created { 1 } else { 0 };

    // --- idempotency: source_ref dedup -------------------------------------
    let supplier_key = if !vat_lower.is_empty() {
        vat_lower.clone()
    } else {
        supplier_lower.clone()
    };
    let source_ref = format!("{}:{}", supplier_key, invoice_ref);

    let is_dup = db
        .query_row(
            "SELECT 1 FROM payables WHERE source = 'ubl' AND source_ref = ?1 AND status = 'unpaid'",
            rusqlite::params![source_ref],
            |_| Ok(true),
        )
        .unwrap_or(false);

    if dry_run {
        return Ok(json!({
            "dryRun": true,
            "invoice_ref": invoice_ref,
            "supplier": supplier_name,
            "date": invoice_date,
            "due_date": due_date,
            "amount_cents": amount_cents,
            "amount": crate::money::format_amount(amount_cents),
            "vat_by_rate": {},
            "contact": { "id": contact_id_resolved, "name": contact_name, "created": contact_created },
            "duplicates": if is_dup { 1 } else { 0 },
            "contacts_created": contacts_created_count,
            "imported": 0,
        }));
    }

    // --- execute: create payable -------------------------------------------
    let mut imported: i64 = 0;
    let mut duplicates: i64 = 0;

    if is_dup {
        duplicates = 1;
    } else {
        db.execute(
            "INSERT INTO payables (contact_id, invoice_ref, date, due_date, amount_cents, payment_method, source, source_ref, created_by) VALUES (?1, ?2, ?3, ?4, ?5, 'transfer', 'ubl', ?6, ?7)",
            rusqlite::params![contact_id_resolved, invoice_ref, invoice_date, due_date, amount_cents, source_ref, actor],
        )
        .map_err(sql_err)?;
        let pid = db.last_insert_rowid();
        record(
            db,
            RecordArgs {
                actor,
                action: "import.invoice",
                command: Some("import invoice"),
                args: Some(json!({
                    "payable_id": pid,
                    "supplier": supplier_name,
                    "invoice_ref": invoice_ref,
                    "date": invoice_date,
                    "due_date": due_date,
                    "amount_cents": amount_cents,
                    "contact_id": contact_id_resolved,
                })),
                outcome: "ok",
                entry_ids: vec![],
            },
        )?;
        imported = 1;
    }

    Ok(json!({
        "invoice_ref": invoice_ref,
        "supplier": supplier_name,
        "date": invoice_date,
        "due_date": due_date,
        "amount_cents": amount_cents,
        "amount": crate::money::format_amount(amount_cents),
        "imported": imported,
        "duplicates": duplicates,
        "contacts_created": contacts_created_count,
        "contact": { "id": contact_id_resolved, "name": contact_name, "created": contact_created },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_basic() {
        let r = parse_csv_rows("code,amount\n1000,100.00\n1100,-100.00");
        assert_eq!(r.len(), 3);
        assert_eq!(r[1].1, vec!["1000", "100.00"]);
    }

    #[test]
    fn csv_semicolon() {
        let r = parse_csv_rows("code;bedrag\n1000;100,00");
        assert_eq!(r.len(), 2);
        assert_eq!(r[1].1, vec!["1000", "100,00"]);
    }

    #[test]
    fn csv_quoted() {
        let r = parse_csv_rows("code,desc\n1000,\"hello, world\"\n1100,\"test\"\"q\"");
        assert_eq!(r[1].1[1], "hello, world");
        assert_eq!(r[2].1[1], "test\"q");
    }

    #[test]
    fn amount_dutch() {
        assert_eq!(parse_import_amount("1.234,56").unwrap(), 123456);
        assert_eq!(parse_import_amount("100,00").unwrap(), 10000);
        assert_eq!(parse_import_amount("1234.56").unwrap(), 123456);
        assert_eq!(parse_import_amount("-50,00").unwrap(), -5000);
        assert_eq!(parse_import_amount("0,50").unwrap(), 50);
    }

    #[test]
    fn amount_errors() {
        assert!(parse_import_amount("").is_err());
        assert!(parse_import_amount("abc").is_err());
        assert!(parse_import_amount("12.345,678").is_err());
    }
}
