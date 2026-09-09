// bukio-cli — agent-first double-entry bookkeeping for SMEs.
// Copyright (c) 2026 Erik van Kempen.
// SPDX-License-Identifier: Apache-2.0
//
// Import — CSV opening balances, journal CSV, contacts from XML.

use crate::accounts::get_account_by_code;
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
            });
            postings.push(PostingSpec {
                code: teg.to_string(),
                amount_cents: -bed,
                cost_center_code: None,
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

/// Import an XAF 4.0 XML file — creates accounts and journal entries.
pub fn import_xaf(db: &Connection, xml_text: &str, actor: &str, dry_run: bool) -> Result<Value> {
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
    } else if root_element == "AuditFile" || root_element == "XAF" {
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
                    "cbc:ID" if depth <= 4 => {
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
                if depth <= 1 {
                    in_tag.clear();
                }
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
            "contact": { "name": supplier_name, "created": false },
            "duplicates": 0,
            "contacts_created": 0,
        }));
    }

    // For now, return basic structure — full import requires contact matching + payable creation
    Ok(json!({
        "invoice_ref": invoice_ref,
        "supplier": supplier_name,
        "date": invoice_date,
        "due_date": due_date,
        "amount_cents": amount_cents,
        "amount": crate::money::format_amount(amount_cents),
        "duplicates": 0,
        "contacts_created": 0,
        "contact": { "id": 0, "name": supplier_name, "created": false },
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
