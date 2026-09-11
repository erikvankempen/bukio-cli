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
use std::collections::{HashMap, HashSet};
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
    // a REVERSED opening-balances entry (the documented correction path) must
    // not block re-import: the reversal nets the old balances to zero, so a
    // fresh import books only the corrected values
    let existing_id: Option<i64> = db
        .query_row(
            "SELECT id FROM journal_entries e
             WHERE e.source = 'import' AND e.source_ref = 'opening-balances'
               AND NOT EXISTS (SELECT 1 FROM journal_entries r WHERE r.reversed_from_id = e.id)
             LIMIT 1",
            [],
            |r| r.get::<_, i64>(0),
        )
        .ok();
    if let Some(id) = existing_id {
        return Err(import_err(
            "OPENING_ALREADY_IMPORTED",
            format!("opening balances were already imported (entry {id})"),
        ));
    }

    let rows = parse_csv_rows(csv_text);
    if rows.is_empty() {
        return Err(import_err(
            "EMPTY_CSV",
            "opening-balances CSV has no data rows",
        ));
    }
    // optional header row: a data row ALWAYS starts with a 1-6 digit account code
    let data_start = if rows[0].1.first().map(|c| valid_code(c)).unwrap_or(false) {
        0
    } else {
        1
    };
    if rows.len() <= data_start {
        return Err(import_err(
            "EMPTY_CSV",
            "opening-balances CSV has no data rows after the header",
        ));
    }

    let mut errors: Vec<Value> = Vec::new();
    let mut specs: Vec<(String, i64)> = Vec::new();
    for (ln, cells) in rows.iter().skip(data_start) {
        // the layout is decided by the column count, so empties are kept
        let (code, amount) = match cells.len() {
            2 => (cells[0].trim().to_string(), cells[1].clone()),
            3 => {
                let (c, debet, credit) = (&cells[0], &cells[1], &cells[2]);
                if !debet.trim().is_empty() && !credit.trim().is_empty() {
                    errors.push(json!({
                        "line": ln,
                        "error": "INVALID_ROW: both debet and credit are filled on one line",
                    }));
                    continue;
                }
                let a = if !debet.trim().is_empty() {
                    debet.clone()
                } else if !credit.trim().is_empty() {
                    format!("-{}", credit.trim())
                } else {
                    String::new()
                };
                (c.trim().to_string(), a)
            }
            n => {
                errors.push(json!({
                    "line": ln,
                    "error": format!(
                        "INVALID_ROW: expected \"code,amount\" or \"code,debet,credit\" (got {n} columns)"
                    ),
                }));
                continue;
            }
        };
        if !valid_code(&code) {
            errors.push(json!({
                "line": ln,
                "error": format!("INVALID_CODE: '{code}' must be 1-6 digits"),
            }));
            continue;
        }
        let amount_cents = match parse_import_amount(&amount) {
            Ok(v) => v,
            Err(e) => {
                errors.push(json!({
                    "line": ln, "error": format!("{}: {}", e.code, e.message),
                }));
                continue;
            }
        };
        if amount_cents == 0 {
            errors.push(json!({
                "line": ln, "error": "INVALID_AMOUNT: amount must be non-zero",
            }));
            continue;
        }
        match get_account_by_code(db, &code) {
            None => {
                errors.push(json!({
                    "line": ln,
                    "error": format!(
                        "ACCOUNT_NOT_FOUND: account {code} does not exist (create it first or import the chart)"
                    ),
                }));
                continue;
            }
            Some(a) if account_is_inactive(&a) => {
                errors.push(json!({
                    "line": ln,
                    "error": format!("ACCOUNT_INACTIVE: account {code} is inactive"),
                }));
                continue;
            }
            _ => {}
        }
        specs.push((code, amount_cents));
    }

    let sum: i64 = specs.iter().map(|(_, a)| a).sum();
    if sum != 0 {
        errors.push(json!({
            "line": 0,
            "error": format!(
                "UNBALANCED: opening balances sum to {} — debet must equal credit",
                crate::money::format_amount(sum)
            ),
        }));
    }
    if specs.len() < 2 && errors.is_empty() {
        errors.push(json!({
            "line": 0, "error": "TOO_FEW_POSTINGS: opening balances need at least 2 accounts",
        }));
    }
    if !errors.is_empty() {
        return Err(BukioError::with_details(
            "IMPORT_VALIDATION_FAILED",
            format!(
                "opening-balances file has {} problem(s) — nothing imported",
                errors.len()
            ),
            json!(errors),
        ));
    }

    let total_debit: i64 = specs.iter().filter(|(_, a)| *a > 0).map(|(_, a)| *a).sum();
    let total_credit = -sum + total_debit;
    let plan = json!({
        "action": "import opening balances",
        "date": the_date,
        "accounts": specs.len(),
        "total_debit_cents": total_debit,
        "total_credit_cents": total_credit,
        "dryRun": true,
    });
    if dry_run {
        return Ok(plan);
    }

    let postings: Vec<PostingSpec> = specs
        .iter()
        .map(|(c, a)| PostingSpec {
            code: c.clone(),
            amount_cents: *a,
            cost_center_code: None,
            vat_code: None,
            vat_amount_cents: None,
            fx_currency: None,
            fx_amount_cents: None,
        })
        .collect();
    let entry = create_entry(
        db,
        CreateEntry {
            date: the_date,
            description: "Beginbalans",
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
            action: "import.opening-balances",
            command: Some("import opening-balances"),
            args: Some(json!({
                "date": the_date,
                "accounts": specs.len(),
                "total_debit_cents": total_debit,
            })),
            outcome: "ok",
            entry_ids: vec![entry.id],
        },
    )?;
    Ok(json!({
        "entry": {
            "id": posted.id, "date": posted.date,
            "description": posted.description, "state": posted.state,
        },
        "accounts": specs.len(),
        "total_debit_cents": total_debit,
        "total_credit_cents": total_credit,
        "dryRun": false,
    }))
}

/// Import journal CSV (boekstuk-based double-entry).
pub fn import_journal_csv(
    db: &Connection,
    csv_text: &str,
    create_missing: bool,
    actor: &str,
    dry_run: bool,
) -> Result<Value> {
    let rows = parse_csv_rows(csv_text);
    if rows.is_empty() {
        return Err(import_err("EMPTY_CSV", "journal CSV has no data rows"));
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
    let cbtw = find(&["btwcode", "vat_code", "vat"]);

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
            format!(
                "journal CSV is missing column(s): {} (got header: {})",
                missing.join(", "),
                header.join(",")
            ),
        ));
    }

    let g = |cells: &[String], col: Option<usize>| {
        col.and_then(|c| cells.get(c).cloned()).unwrap_or_default()
    };
    let mut errors: Vec<Value> = Vec::new();
    let mut parsed: Vec<Value> = Vec::new();
    let mut btw_codes: Vec<String> = Vec::new();

    for (ln, cells) in rows.iter().skip(1) {
        let date = g(cells, cd);
        let boek = g(cells, cb);
        let rek = g(cells, cr);
        let teg = g(cells, ct);
        let bed = g(cells, ca);
        if date.is_empty() && boek.is_empty() && rek.is_empty() && teg.is_empty() && bed.is_empty()
        {
            continue; // blank line
        }
        if !valid_date(&date) {
            errors.push(json!({
                "line": ln, "error": format!("INVALID_DATE: '{date}' must be yyyy-mm-dd"),
            }));
        }
        if boek.is_empty() {
            errors.push(json!({
                "line": ln, "error": "BOEKSTUK_REQUIRED: every row needs a boekstuknummer",
            }));
        }
        if !valid_code(&rek) {
            errors.push(json!({
                "line": ln,
                "error": format!("INVALID_CODE: rekening '{rek}' must be 1-6 digits"),
            }));
        }
        if !valid_code(&teg) {
            errors.push(json!({
                "line": ln,
                "error": format!("INVALID_CODE: tegenrekening '{teg}' must be 1-6 digits"),
            }));
        }
        let mut bc: Option<i64> = None;
        match parse_import_amount(&bed) {
            Ok(v) => bc = Some(v),
            Err(e) => errors.push(json!({
                "line": ln, "error": format!("{}: {}", e.code, e.message),
            })),
        }
        if bc == Some(0) {
            errors.push(json!({
                "line": ln, "error": "INVALID_AMOUNT: bedrag must be non-zero",
            }));
        }
        let bc = bc.unwrap_or(0);
        // accounts must exist (unless --create-missing) and be active
        for code in [&rek, &teg] {
            if !valid_code(code) {
                continue;
            }
            match get_account_by_code(db, code) {
                None if !create_missing => errors.push(json!({
                    "line": ln,
                    "error": format!(
                        "ACCOUNT_NOT_FOUND: account {code} does not exist (use --create-missing to create it)"
                    ),
                })),
                Some(a) if account_is_inactive(&a) => errors.push(json!({
                    "line": ln,
                    "error": format!("ACCOUNT_INACTIVE: account {code} is inactive"),
                })),
                _ => {}
            }
        }
        let btw = g(cells, cbtw);
        if !btw.is_empty() && !btw_codes.contains(&btw) {
            btw_codes.push(btw.clone());
        }
        parsed.push(json!({
            "line": ln, "date": date, "boekstuk": boek, "rekening": rek,
            "tegenrekening": teg, "bedragCents": bc, "omschrijving": g(cells, cdesc),
            "btwcode": btw,
        }));
    }
    if parsed.is_empty() {
        return Err(import_err(
            "EMPTY_CSV",
            "journal CSV has no data rows after the header",
        ));
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
    // one date per boekstuk
    for b in &groups {
        let mut ds: Vec<String> = Vec::new();
        for p in &by_b[b] {
            let d = p["date"].as_str().unwrap_or("").to_string();
            if !ds.contains(&d) {
                ds.push(d);
            }
        }
        if ds.len() > 1 {
            errors.push(json!({
                "line": 0,
                "error": format!(
                    "DATE_MISMATCH: boekstuk '{b}' has rows on different dates ({})",
                    ds.join(", ")
                ),
            }));
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
        return Err(BukioError::with_details(
            "IMPORT_VALIDATION_FAILED",
            format!(
                "journal file has {} problem(s) — nothing imported",
                errors.len()
            ),
            json!(errors),
        ));
    }

    let dupes = groups
        .iter()
        .filter(|b| existing.contains(&format!("journal:{b}")))
        .count();
    if dry_run {
        let entries: Vec<Value> = groups
            .iter()
            .map(|b| {
                json!({
                    "boekstuk": b,
                    "date": by_b[b][0]["date"],
                    "lines": by_b[b].len(),
                })
            })
            .collect();
        return Ok(json!({
            "action": "import journal",
            "boekstukken": groups.len(),
            "lines": parsed.len(),
            "entries": entries,
            "create_missing": create_missing,
            "ignored_btw_codes": btw_codes,
            "duplicates": dupes,
            "dryRun": true,
        }));
    }

    let mut imported: Vec<Value> = Vec::new();
    let mut accounts_created: Vec<Value> = Vec::new();
    // net movement per code, for inferring the type of accounts we create
    let mut net: HashMap<String, i64> = HashMap::new();
    for p in &parsed {
        *net.entry(p["rekening"].as_str().unwrap_or("").to_string())
            .or_insert(0) += p["bedragCents"].as_i64().unwrap_or(0);
        *net.entry(p["tegenrekening"].as_str().unwrap_or("").to_string())
            .or_insert(0) -= p["bedragCents"].as_i64().unwrap_or(0);
    }
    for b in &groups {
        let rf = format!("journal:{b}");
        if existing.contains(&rf) {
            continue;
        }
        let lines = &by_b[b];
        let date = lines[0]["date"].as_str().unwrap_or("");
        if create_missing {
            let mut codes: Vec<String> = Vec::new();
            for l in lines.iter() {
                for c in [
                    l["rekening"].as_str().unwrap_or(""),
                    l["tegenrekening"].as_str().unwrap_or(""),
                ] {
                    let c = c.to_string();
                    if !codes.contains(&c) && get_account_by_code(db, &c).is_none() {
                        codes.push(c.clone());
                        let (t, nb) = infer_account_type(*net.get(&c).unwrap_or(&0));
                        let name = format!("Rekening {c}");
                        let rgs = infer_rgs(&t, &name);
                        let acct = create_account(
                            db,
                            &NewAccount {
                                code: &c,
                                name: &name,
                                type_: &t,
                                normal_balance: &nb,
                                taxonomy_code: rgs,
                            },
                        )?;
                        accounts_created.push(json!({
                            "code": acct["code"], "name": acct["name"], "type": acct["type"],
                            "normal_balance": acct["normal_balance"], "taxonomy_code": acct["taxonomy_code"],
                        }));
                    }
                }
            }
        }
        let desc = lines
            .iter()
            .find_map(|l| l["omschrijving"].as_str().filter(|s| !s.is_empty()))
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("Boekstuk {b}"));
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
                fx_currency: None,
                fx_amount_cents: None,
            });
            postings.push(PostingSpec {
                code: teg.to_string(),
                amount_cents: -bed,
                cost_center_code: None,
                vat_code: None,
                vat_amount_cents: None,
                fx_currency: None,
                fx_amount_cents: None,
            });
        }
        let entry = create_entry(
            db,
            CreateEntry {
                date,
                description: &desc,
                postings,
                source: "import",
                source_ref: Some(&rf),
                actor,
            },
        )?;
        let posted = post_entry(db, entry.id, actor)?;
        imported.push(json!({
            "id": posted.id, "date": posted.date,
            "description": posted.description, "boekstuk": b,
        }));
    }
    record(
        db,
        RecordArgs {
            actor,
            action: "import.journal",
            command: Some("import journal"),
            args: Some(json!({
                "boekstukken": imported.len(),
                "duplicates": dupes,
                "create_missing": create_missing,
            })),
            outcome: "ok",
            entry_ids: imported.iter().filter_map(|e| e["id"].as_i64()).collect(),
        },
    )?;
    Ok(json!({
        "imported": imported.len(),
        "duplicates": dupes,
        "entries": imported,
        "accounts_created": accounts_created,
        "ignored_btw_codes": btw_codes,
        "dryRun": false,
    }))
}

/// Import contacts from UBL XML.
pub fn import_contacts(
    db: &Connection,
    xml_text: &str,
    actor: &str,
    dry_run: bool,
) -> Result<Value> {
    let tree = xml_tree(xml_text)?;
    let root = ["Xaf", "XAF", "AuditFile"]
        .iter()
        .find_map(|k| tree.get(*k))
        .ok_or_else(|| import_err("INVALID_XAF", "root element must be <Xaf> or <AuditFile>"))?;
    let mf = root.get("MasterFiles").unwrap_or(root);
    let suppliers = xa(mf.get("Suppliers").and_then(|s| s.get("Supplier")));
    let customers = xa(mf.get("Customers").and_then(|c| c.get("Customer")));

    let mut errors: Vec<Value> = Vec::new();
    let mut rows: Vec<Value> = Vec::new();
    for (kind, list, id_tag) in [
        ("supplier", &suppliers, "SupplierID"),
        ("customer", &customers, "CustomerID"),
    ] {
        for raw in list {
            let id = xt(raw.get(id_tag));
            let name = {
                let n = xt(raw.get("CompanyName"));
                if n.is_empty() {
                    xt(raw.get("Contact"))
                } else {
                    n
                }
            };
            if name.is_empty() {
                errors.push(json!({
                    "line": 0,
                    "error": format!(
                        "CONTACT_REQUIRED: {kind} '{}' has no CompanyName/Contact",
                        if id.is_empty() { "?" } else { id.as_str() }
                    ),
                }));
                continue;
            }
            let addr = raw.get("Address");
            let street = xt(addr.and_then(|a| a.get("StreetName")));
            let extra = xt(addr.and_then(|a| a.get("AdditionalAddressDetail")));
            let address = if extra.is_empty() {
                street
            } else {
                format!("{street}, {extra}")
            };
            let country = {
                let c = xt(addr.and_then(|a| a.get("Country"))).to_uppercase();
                if c.is_empty() {
                    "NL".to_string()
                } else {
                    c
                }
            };
            rows.push(json!({
                "kind": kind,
                "name": name,
                "address": s_or_null(&address),
                "postal_code": s_or_null(&xt(addr.and_then(|a| a.get("PostalCode")))),
                "city": s_or_null(&xt(addr.and_then(|a| a.get("City")))),
                "country": country,
                "email": s_or_null(&xt(raw.get("Email"))),
                "vat_id": s_or_null(&xt(raw.get("TaxRegistrationNumber"))),
            }));
        }
    }
    if !errors.is_empty() {
        return Err(BukioError::with_details(
            "IMPORT_VALIDATION_FAILED",
            format!(
                "audit file has {} problem(s) — nothing imported",
                errors.len()
            ),
            json!(errors),
        ));
    }

    let existing: HashSet<String> = crate::contacts::list_contacts(db)?
        .iter()
        .map(|c| xt(c.get("name")).to_lowercase())
        .collect();
    let mut seen: HashSet<String> = HashSet::new();
    let fresh: Vec<&Value> = rows
        .iter()
        .filter(|r| {
            let key = r["name"].as_str().unwrap_or("").to_lowercase();
            if seen.contains(&key) || existing.contains(&key) {
                return false;
            }
            seen.insert(key);
            true
        })
        .collect();
    let duplicates = rows.len() - fresh.len();

    if dry_run {
        return Ok(json!({
            "suppliers": suppliers.len(), "customers": customers.len(),
            "contacts": rows.len(), "contacts_to_create": fresh.len(),
            "duplicates": duplicates, "dryRun": true,
        }));
    }

    let mut imported: Vec<Value> = Vec::new();
    for r in &fresh {
        let contact = create_contact(
            db,
            r["name"].as_str().unwrap_or(""),
            r["address"].as_str(),
            r["postal_code"].as_str(),
            r["city"].as_str(),
            r["country"].as_str(),
            r["email"].as_str(),
            r["vat_id"].as_str(),
            None,
            None,
            actor,
            false,
        )?;
        imported.push(json!({"id": contact["id"], "name": contact["name"], "kind": r["kind"]}));
    }
    record(
        db,
        RecordArgs {
            actor,
            action: "import.contacts",
            command: Some("import contacts"),
            args: Some(json!({
                "imported": imported.len(), "duplicates": duplicates,
                "suppliers": suppliers.len(), "customers": customers.len(),
            })),
            outcome: "ok",
            entry_ids: vec![],
        },
    )?;
    Ok(json!({
        "imported": imported.len(), "duplicates": duplicates, "contacts": imported,
        "suppliers": suppliers.len(), "customers": customers.len(), "dryRun": false,
    }))
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

    // Current mutatie
    let mut cur_boekstuk = String::new();
    let mut cur_date = String::new();
    let mut boekingen_in_mutation = 0i64;

    // Current boeking
    let mut cur_rekening = String::new();
    let mut cur_tegenrekening = String::new();
    let mut cur_bedrag = String::new();
    let mut cur_boeking_oms = String::new();

    // Current rekening (file chart)
    let mut cur_rek_code = String::new();
    let mut cur_rek_oms = String::new();
    let mut cur_rek_soort = String::new();

    // Header extras (plan + report only)
    let mut start_date = String::new();
    let mut end_date = String::new();
    let mut software_name = String::new();
    let mut software_version = String::new();

    // Collected file chart + parsed mutaties (validation stays inline)
    let mut file_chart: Vec<Value> = Vec::new();
    let mut parsed_mutaties: Vec<Value> = Vec::new();
    let mut cur_postings: Vec<Value> = Vec::new();

    // Counters
    let mut errors: Vec<String> = Vec::new();

    // Post-validation
    let mut imported: Vec<Value> = Vec::new();
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
                    }
                    "Mutatie" => {
                        in_mutatie = true;
                        cur_boekstuk.clear();
                        cur_date.clear();
                        cur_postings.clear();
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
                        "StartDate" if in_header => start_date = text.trim().to_string(),
                        "EndDate" if in_header => end_date = text.trim().to_string(),
                        "SoftwareName" if in_header => software_name = text.trim().to_string(),
                        "SoftwareVersion" if in_header => {
                            software_version = text.trim().to_string()
                        }
                        // Rekeningen section
                        "RekeningCode" if in_rekeningen && !in_mutatie => {
                            let code = text.trim().to_string();
                            cur_rek_code = code.clone();
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
                        "RekeningOmschrijving" if in_rekeningen && !in_mutatie => {
                            cur_rek_oms = text.trim().to_string()
                        }
                        "RekeningSoort" if in_rekeningen && !in_mutatie => {
                            cur_rek_soort = text.trim().to_string()
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
                        "Omschrijving" if in_boeking => {
                            cur_boeking_oms = text.trim().to_string();
                        }
                        "BtwCode" if in_boeking => {
                            let c = text.trim().to_string();
                            if !c.is_empty() && !ignored_btw_codes.contains(&c) {
                                ignored_btw_codes.push(c);
                            }
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
                        // collect the posting — validation above has already run
                        cur_postings.push(json!({
                            "rekening": cur_rekening.trim(),
                            "tegenrekening": cur_tegenrekening.trim(),
                            "bedragCents": parse_import_amount(&cur_bedrag).unwrap_or(0),
                            "omschrijving": std::mem::take(&mut cur_boeking_oms),
                        }));
                        in_boeking = false;
                    }
                    "Rekening" if !in_mutatie => {
                        file_chart.push(json!({
                            "code": cur_rek_code.trim(),
                            "name": cur_rek_oms.trim(),
                            "soort": cur_rek_soort.trim(),
                        }));
                        cur_rek_code.clear();
                        cur_rek_oms.clear();
                        cur_rek_soort.clear();
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
                        parsed_mutaties.push(json!({
                            "boekstuk": cur_boekstuk.trim(),
                            "date": cur_date.trim(),
                            "postings": std::mem::take(&mut cur_postings),
                        }));
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
    // Version — refuse immediately, like the JS importer: a file from another
    // schema generation must not be half-read
    if !valid_version(&version) {
        return Err(import_err(
            "INVALID_XAF",
            format!(
                "unsupported audit file version '{}' — expected 4.0",
                if version.is_empty() {
                    "(missing)"
                } else {
                    &version
                }
            ),
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
            return Err(import_err(
                "COMPANY_MISMATCH",
                format!(
                    "audit file is for {file_kvk} ({}), database is for {db_reg} ({db_name})",
                    if file_name.is_empty() {
                        "unknown"
                    } else {
                        file_name
                    }
                ),
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

    // --- Phase 3: Report ---
    if !errors.is_empty() {
        return Err(BukioError::with_details(
            "IMPORT_VALIDATION_FAILED",
            format!(
                "XAF file has {} problem(s) — nothing imported",
                errors.len()
            ),
            json!(errors
                .iter()
                .map(|e| json!({"line": 0, "error": e}))
                .collect::<Vec<_>>()),
        ));
    }

    // net movement per code (file chart + used codes) for type inference
    let mut net: HashMap<String, i64> = HashMap::new();
    for r in &file_chart {
        net.insert(r["code"].as_str().unwrap_or("").to_string(), 0);
    }
    for m in &parsed_mutaties {
        for p in m["postings"].as_array().unwrap_or(&vec![]) {
            let cents = p["bedragCents"].as_i64().unwrap_or(0);
            let rek = p["rekening"].as_str().unwrap_or("").to_string();
            let teg = p["tegenrekening"].as_str().unwrap_or("").to_string();
            *net.entry(rek).or_insert(0) += cents;
            *net.entry(teg).or_insert(0) -= cents;
        }
    }

    let existing_refs: HashSet<String> = {
        let mut stmt = db
            .prepare("SELECT source_ref FROM journal_entries WHERE source = 'xaf' AND source_ref IS NOT NULL")
            .map_err(sql_err)?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(sql_err)?;
        rows.filter_map(|r| r.ok()).collect()
    };

    let val_or_null = |s: &str| {
        if s.trim().is_empty() {
            Value::Null
        } else {
            Value::String(s.trim().to_string())
        }
    };
    let file_kvk = val_or_null(&company_id);
    let file_name = val_or_null(&company_name);
    let fiscal_year_v = val_or_null(&fiscal_year);
    let start_date_v = val_or_null(&start_date);
    let end_date_v = val_or_null(&end_date);
    let software = if software_name.trim().is_empty() {
        Value::Null
    } else {
        Value::String(
            format!("{} {}", software_name.trim(), software_version.trim())
                .trim()
                .to_string(),
        )
    };

    let plan = json!({
        "action": "import xaf",
        "company": {
            "name": file_name,
            "registration_id": file_kvk,
            "fiscal_year": fiscal_year_v,
            "period": format!("{}..{}", start_date.trim(), end_date.trim()),
        },
        "software": software,
        "rekeningen": file_chart.len(),
        "mutaties": parsed_mutaties.len(),
        "duplicates": parsed_mutaties
            .iter()
            .filter(|m| existing_refs.contains(m["boekstuk"].as_str().unwrap_or("")))
            .count(),
        "accounts_to_create": file_chart
            .iter()
            .filter(|r| get_account_by_code(db, r["code"].as_str().unwrap_or("")).is_none())
            .count(),
        "accounts_to_rename": file_chart
            .iter()
            .map(|r| (
                r["code"].as_str().unwrap_or("").trim().to_string(),
                r["name"].as_str().unwrap_or("").trim().to_string(),
            ))
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
        "ignored_btw_codes": ignored_btw_codes,
        "dryRun": true,
    });
    if dry_run {
        return Ok(plan);
    }

    let mut accounts_updated: Vec<Value> = Vec::new();
    let mut rgs_backfilled: Vec<Value> = Vec::new();
    let mut chart_warnings: Vec<String> = Vec::new();
    let mut existing_refs = existing_refs;

    // upsert the file's chart
    for r in &file_chart {
        let code = r["code"].as_str().unwrap_or("").trim().to_string();
        let name = {
            let n = r["name"].as_str().unwrap_or("").trim().to_string();
            if n.is_empty() {
                format!("Rekening {code}")
            } else {
                n
            }
        };
        let hint = rekening_type(
            r["soort"].as_str().unwrap_or(""),
            *net.get(&code).unwrap_or(&0),
        );
        sync_account_from_file(
            db,
            &code,
            &name,
            Some(hint),
            &mut accounts_created,
            &mut accounts_updated,
            &mut chart_warnings,
            &mut rgs_backfilled,
        )?;
    }
    for m in &parsed_mutaties {
        let boekstuk = m["boekstuk"].as_str().unwrap_or("").to_string();
        if existing_refs.contains(&boekstuk) {
            // same-file duplicate boekstuknummer -> skip (parity with AuditFile)
            duplicates += 1;
            continue;
        }
        let mut postings: Vec<PostingSpec> = Vec::new();
        for p in m["postings"].as_array().unwrap_or(&vec![]) {
            let cents = p["bedragCents"].as_i64().unwrap_or(0);
            postings.push(PostingSpec {
                code: p["rekening"].as_str().unwrap_or("").to_string(),
                amount_cents: cents,
                cost_center_code: None,
                vat_code: None,
                vat_amount_cents: None,
                fx_currency: None,
                fx_amount_cents: None,
            });
            postings.push(PostingSpec {
                code: p["tegenrekening"].as_str().unwrap_or("").to_string(),
                amount_cents: -cents,
                cost_center_code: None,
                vat_code: None,
                vat_amount_cents: None,
                fx_currency: None,
                fx_amount_cents: None,
            });
        }
        let description = m["postings"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .find_map(|p| p["omschrijving"].as_str().filter(|s| !s.is_empty()))
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("XAF {boekstuk}"));
        let entry = create_entry(
            db,
            CreateEntry {
                date: m["date"].as_str().unwrap_or(""),
                description: &description,
                postings,
                source: "xaf",
                source_ref: Some(boekstuk.as_str()),
                actor,
            },
        )?;
        let posted = post_entry(db, entry.id, actor)?;
        existing_refs.insert(boekstuk.clone());
        imported.push(json!({
            "id": posted.id, "date": posted.date,
            "description": posted.description, "boekstuk": boekstuk,
        }));
    }

    record(
        db,
        RecordArgs {
            actor,
            action: "import.xaf",
            command: Some("import xaf"),
            args: Some(json!({
                "mutaties": imported.len(), "duplicates": duplicates,
                "accounts_created": accounts_created.len(),
                "accounts_updated": accounts_updated.len(),
                "accounts_rgs_backfilled": rgs_backfilled.len(),
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
            "fiscal_year": fiscal_year_v,
            "start_date": start_date_v,
            "end_date": end_date_v,
            "software": software,
        },
        "company_mismatch": company_mismatch,
        "ignored_btw_codes": ignored_btw_codes,
        "dryRun": false,
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

/// Empty string -> JSON null (the JS importers normalise '' to null).
fn s_or_null(s: &str) -> Value {
    if s.is_empty() {
        Value::Null
    } else {
        Value::String(s.to_string())
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

/// `active` may arrive as 1/0 or true/false depending on the query path.
fn account_is_inactive(a: &Value) -> bool {
    match a.get("active") {
        Some(Value::Bool(b)) => !*b,
        Some(v) => v.as_i64() == Some(0),
        None => false,
    }
}

fn is_audit_version(v: &str) -> bool {
    v == "4" || (v.starts_with("4.") && v.len() > 2 && v[2..].chars().all(|c| c.is_ascii_digit()))
}

fn is_eight_digits(s: &str) -> bool {
    s.len() == 8 && s.chars().all(|c| c.is_ascii_digit())
}

/// XAF RekeningSoort -> account type (net movement decides asset vs liability).
fn rekening_type(soort: &str, net_cents: i64) -> (String, String) {
    let balans = soort.to_lowercase().contains("balans");
    let credit = net_cents < 0;
    if balans {
        if credit {
            ("liability".to_string(), "credit".to_string())
        } else {
            ("asset".to_string(), "debit".to_string())
        }
    } else if credit {
        ("income".to_string(), "credit".to_string())
    } else {
        ("expense".to_string(), "debit".to_string())
    }
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
                fx_currency: None,
                fx_amount_cents: None,
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

    // ── parse (path-aware: the JS picks by path, so tag names alone are not
    // enough — e.g. cbc:ID under <Invoice> vs any other id, or cbc:TaxAmount
    // under TaxTotal vs under TaxSubtotal) ───────────────────────────────────
    let mut reader = Reader::from_str(xml_text);
    reader.config_mut().trim_text(true);

    let mut root_name = String::new();
    let mut invoice_ref = String::new();
    let mut type_code = String::new();
    let mut issue_date = String::new();
    let mut due_date = String::new();
    let mut currency = String::new();
    let mut payable_raw = String::new();
    let mut sup_name = String::new();
    let mut sup_legal_name = String::new();
    let mut sup_vat = String::new();
    let mut sup_email = String::new();
    let mut sup_street = String::new();
    let mut sup_city = String::new();
    let mut sup_postal = String::new();
    let mut sup_country = String::new();
    // per-PartyTaxScheme scratch (the VAT number lives in the scheme whose
    // TaxScheme/cbc:ID is literally 'VAT' — several siblings are allowed)
    let mut scheme_id = String::new();
    let mut scheme_company_id = String::new();
    // VAT breakdown: (percent, tax amount) per cac:TaxSubtotal
    let mut subtotals: Vec<(String, String)> = Vec::new();

    let mut stack: Vec<String> = Vec::new();
    let mut buf = Vec::new();

    fn local(name: &[u8]) -> String {
        let s = String::from_utf8_lossy(name).to_string();
        match s.rsplit_once(':') {
            Some((_, l)) => l.to_string(),
            None => s,
        }
    }
    fn top_is(stack: &[String], name: &str) -> bool {
        stack.last().map(|t| t == name).unwrap_or(false)
    }
    fn has_ancestor(stack: &[String], name: &str) -> bool {
        stack.iter().any(|t| t == name)
    }
    fn parent_is(stack: &[String], name: &str) -> bool {
        stack.len() >= 2 && stack[stack.len() - 2] == name
    }

    loop {
        let ev = reader.read_event_into(&mut buf);
        match ev {
            Ok(Event::Start(e)) => {
                let name = local(e.name().as_ref());
                if stack.is_empty() {
                    root_name = name.clone();
                }
                if name == "TaxSubtotal" {
                    subtotals.push((String::new(), String::new()));
                }
                stack.push(name);
            }
            // a self-closing element has no text and no End event
            Ok(Event::Empty(_)) => {}
            Ok(Event::Text(t)) => {
                let text = t.unescape().unwrap_or_default().trim().to_string();
                if text.is_empty() {
                    continue;
                }
                let t = text.as_str();
                match stack.last().map(String::as_str).unwrap_or("") {
                    "ID" if stack.len() == 2 => invoice_ref = t.to_string(), // Invoice > cbc:ID
                    "InvoiceTypeCode" => type_code = t.to_string(),
                    "DocumentCurrencyCode" => currency = t.to_string(),
                    "IssueDate" => issue_date = t.to_string(),
                    "DueDate" => due_date = t.to_string(),
                    "PayableAmount" if parent_is(&stack, "LegalMonetaryTotal") => {
                        payable_raw = t.to_string()
                    }
                    _ if has_ancestor(&stack, "AccountingSupplierParty") => {
                        match stack.last().map(String::as_str).unwrap_or("") {
                            "Name" if parent_is(&stack, "PartyName") => sup_name = t.to_string(),
                            "RegistrationName" if parent_is(&stack, "PartyLegalEntity") => {
                                sup_legal_name = t.to_string()
                            }
                            "CompanyID" => scheme_company_id = t.to_string(),
                            "ID" if parent_is(&stack, "TaxScheme") => scheme_id = t.to_string(),
                            "ElectronicMail" => sup_email = t.to_string(),
                            "StreetName" if has_ancestor(&stack, "PostalAddress") => {
                                sup_street = t.to_string()
                            }
                            "CityName" if has_ancestor(&stack, "PostalAddress") => {
                                sup_city = t.to_string()
                            }
                            "PostalZone" if has_ancestor(&stack, "PostalAddress") => {
                                sup_postal = t.to_string()
                            }
                            "IdentificationCode" if has_ancestor(&stack, "Country") => {
                                sup_country = t.to_string()
                            }
                            _ => {}
                        }
                    }
                    "Percent" if has_ancestor(&stack, "TaxSubtotal") => {
                        if let Some(s) = subtotals.last_mut() {
                            s.0 = t.to_string();
                        }
                    }
                    "TaxAmount" if parent_is(&stack, "TaxSubtotal") => {
                        if let Some(s) = subtotals.last_mut() {
                            s.1 = t.to_string();
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::End(_)) => {
                match stack.last().map(String::as_str).unwrap_or("") {
                    "PartyTaxScheme" => {
                        // keep the first scheme whose id is literally 'VAT'
                        if scheme_id.eq_ignore_ascii_case("VAT")
                            && sup_vat.is_empty()
                            && !scheme_company_id.is_empty()
                        {
                            sup_vat = scheme_company_id.clone();
                        }
                        scheme_id.clear();
                        scheme_company_id.clear();
                    }
                    _ => {}
                }
                stack.pop();
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    // ── validation (EN 16931), BEFORE resolving contacts: a malformed document
    // must be reported as malformed whether or not a contact matches ─────────
    if root_name != "Invoice" {
        return Err(import_err(
            "INVALID_UBL_INVOICE",
            "cannot parse XML: root element must be <Invoice> in the UBL namespace \
             (urn:oasis:names:specification:ubl:schema:xsd:Invoice-2) — this is not an EN 16931 invoice",
        ));
    }
    let mut errors: Vec<Value> = Vec::new();
    let err = |e: &str| json!({ "line": 0, "error": e });

    if invoice_ref.is_empty() {
        errors.push(err(
            "INVALID_UBL_INVOICE: cbc:ID (invoice number) is missing",
        ));
    }
    if type_code.is_empty() {
        return Err(import_err(
            "INVALID_UBL_INVOICE",
            "cbc:InvoiceTypeCode is missing (EN 16931 BT-3)",
        ));
    }
    if type_code != "380" {
        return Err(import_err(
            "UNSUPPORTED_UBL_DOCUMENT",
            format!("InvoiceTypeCode '{type_code}' is not supported (380 = invoice; credit notes 381 are not imported yet)"),
        ));
    }
    if !valid_date(&issue_date) {
        errors.push(err(&format!(
            "INVALID_DATE: cbc:IssueDate '{issue_date}' must be yyyy-mm-dd"
        )));
    }
    if !due_date.is_empty() && !valid_date(&due_date) {
        errors.push(err(&format!(
            "INVALID_DATE: cbc:DueDate '{due_date}' must be yyyy-mm-dd"
        )));
    }
    let supplier_name = if !sup_name.is_empty() {
        sup_name.clone()
    } else {
        sup_legal_name.clone()
    };
    if supplier_name.is_empty() {
        errors.push(err(
            "INVALID_UBL_INVOICE: supplier name missing (PartyName/Name or PartyLegalEntity/RegistrationName)",
        ));
    }
    // EN 16931 BT-5 is mandatory: an absent element is a malformed document, not
    // an implicit EUR (a missing currency on a non-EUR invoice would silently
    // create a payable in the wrong currency)
    if currency.is_empty() {
        errors.push(err(
            "INVALID_UBL_INVOICE: cbc:DocumentCurrencyCode is missing (EN 16931 BT-5)",
        ));
    } else if currency.to_uppercase() != "EUR" {
        errors.push(err(&format!(
            "INVALID_UBL_INVOICE: cbc:DocumentCurrencyCode '{}' — only EUR invoices can be imported (payables are EUR-only)",
            currency.to_uppercase()
        )));
    }
    let mut payable_cents: Option<i64> = None;
    if payable_raw.is_empty() {
        errors.push(err("INVALID_UBL_INVOICE: cbc:PayableAmount is missing"));
    } else {
        match parse_import_amount(&payable_raw) {
            Ok(c) if c > 0 => payable_cents = Some(c),
            _ => errors.push(err(&format!(
                "INVALID_AMOUNT: cbc:PayableAmount '{payable_raw}' must be a positive amount"
            ))),
        }
    }
    // informational only — no VAT legs are booked
    let mut vat_by_rate = serde_json::Map::new();
    for (pct, tax_raw) in &subtotals {
        if tax_raw.is_empty() {
            continue;
        }
        match parse_import_amount(tax_raw) {
            Ok(c) => {
                let cur = vat_by_rate.get(pct).and_then(|v| v.as_i64()).unwrap_or(0);
                vat_by_rate.insert(pct.clone(), json!(cur + c));
            }
            Err(_) => errors.push(err(&format!(
                "INVALID_AMOUNT: cbc:TaxAmount '{tax_raw}' is not a valid amount"
            ))),
        }
    }

    if !errors.is_empty() {
        return Err(BukioError::with_details(
            "IMPORT_VALIDATION_FAILED",
            "UBL invoice failed validation — nothing was imported",
            json!(errors),
        ));
    }

    let vat_id = if sup_vat.is_empty() {
        None
    } else {
        Some(sup_vat.clone())
    };
    let payable_cents = payable_cents.unwrap_or(0);
    let vat_value = match &vat_id {
        Some(v) => json!(v),
        None => Value::Null,
    };

    // ── idempotency key ─────────────────────────────────────────────────────
    let supplier_key = match &vat_id {
        Some(v) => v.to_lowercase(),
        None => normalize_import_name(&supplier_name),
    };
    let source_ref = format!("{supplier_key}:{invoice_ref}");
    let is_dup = db
        .query_row(
            "SELECT 1 FROM payables WHERE source = 'ubl' AND source_ref = ?1",
            rusqlite::params![source_ref],
            |_| Ok(true),
        )
        .unwrap_or(false);

    // ── contact resolution ──────────────────────────────────────────────────
    use crate::contacts::{get_contact, list_contacts};
    let name_lower = normalize_import_name(&supplier_name);
    let mut resolved: Option<Value> = None;
    let mut contact_created = false;

    if let Some(cid) = contact_id {
        resolved = get_contact(db, cid)?;
        if resolved.is_none() {
            return Err(import_err(
                "CONTACT_NOT_FOUND",
                format!("contact {cid} does not exist"),
            ));
        }
    } else {
        let all = list_contacts(db)?;
        if let Some(v) = &vat_id {
            let want = v.to_lowercase();
            resolved = all
                .iter()
                .find(|c| {
                    c["vat_id"]
                        .as_str()
                        .map(|x| x.trim().to_lowercase() == want)
                        .unwrap_or(false)
                })
                .cloned();
        }
        if resolved.is_none() {
            resolved = all
                .iter()
                .find(|c| normalize_import_name(c["name"].as_str().unwrap_or("")) == name_lower)
                .cloned();
        }
        if resolved.is_none() && create_missing {
            contact_created = true;
            if !dry_run {
                let country = if sup_country.is_empty() {
                    "NL".to_string()
                } else {
                    sup_country.to_uppercase()
                };
                resolved = Some(create_contact(
                    db,
                    &supplier_name,
                    opt(&sup_street),
                    opt(&sup_postal),
                    opt(&sup_city),
                    Some(&country),
                    opt(&sup_email),
                    vat_id.as_deref(),
                    None, // kvk
                    None, // iban
                    actor,
                    false,
                )?);
            }
        }
        if resolved.is_none() && !(create_missing && dry_run) {
            return Err(import_err(
                "CONTACT_NOT_FOUND",
                format!("no contact matches supplier '{supplier_name}' — pass --contact <id> or --create-missing to create it"),
            ));
        }
    }

    let resolved_id = resolved.as_ref().and_then(|c| c["id"].as_i64());
    let resolved_name = resolved
        .as_ref()
        .and_then(|c| c["name"].as_str())
        .unwrap_or(&supplier_name)
        .to_string();
    let final_due = if due_date.is_empty() {
        add_days_iso(&issue_date, 30).unwrap_or_default()
    } else {
        due_date.clone()
    };
    let vat_by_rate = Value::Object(vat_by_rate);

    if dry_run {
        return Ok(json!({
            "dryRun": true,
            "action": "import.invoice",
            "file": Value::Null,
            "supplier": supplier_name,
            "vat_id": vat_value,
            "invoice_ref": invoice_ref,
            "date": issue_date,
            "due_date": final_due,
            "amount_cents": payable_cents,
            "amount": crate::money::format_amount(payable_cents),
            "vat_by_rate": vat_by_rate,
            "contact": {
                "id": match resolved_id { Some(i) => json!(i), None => Value::Null },
                "name": resolved_name,
                "created": contact_created,
            },
            "duplicates": if is_dup { 1 } else { 0 },
            "contacts_created": if contact_created { 1 } else { 0 },
            "imported": 0,
        }));
    }

    let mut imported = 0;
    let mut duplicates = 0;
    if is_dup {
        duplicates = 1;
    } else {
        let cid = resolved_id.unwrap_or(0);
        db.execute(
            "INSERT INTO payables (contact_id, invoice_ref, date, due_date, amount_cents, payment_method, source, source_ref, created_by) VALUES (?1, ?2, ?3, ?4, ?5, 'transfer', 'ubl', ?6, ?7)",
            rusqlite::params![cid, invoice_ref, issue_date, final_due, payable_cents, source_ref, actor],
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
                    "vat_id": vat_value,
                    "invoice_ref": invoice_ref,
                    "date": issue_date,
                    "due_date": final_due,
                    "amount_cents": payable_cents,
                    "vat_by_rate": vat_by_rate,
                    "contact_id": cid,
                })),
                outcome: "ok",
                entry_ids: vec![],
            },
        )?;
        imported = 1;
    }

    Ok(json!({
        "imported": imported,
        "duplicates": duplicates,
        "contacts_created": if contact_created { 1 } else { 0 },
        "supplier": supplier_name,
        "vat_id": vat_value,
        "invoice_ref": invoice_ref,
        "date": issue_date,
        "due_date": final_due,
        "amount_cents": payable_cents,
        "amount": crate::money::format_amount(payable_cents),
        "vat_by_rate": vat_by_rate,
        "contact": { "id": match resolved_id { Some(i) => json!(i), None => Value::Null }, "name": resolved_name },
        "dryRun": false,
    }))
}

/// The JS's normalizeName: lowercased with every non-alphanumeric dropped, so
/// 'Acme BV' and 'acmebv' are the same supplier key.
fn normalize_import_name(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect()
}

fn opt(s: &str) -> Option<&str> {
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

fn add_days_iso(date: &str, days: i64) -> Option<String> {
    chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .ok()
        .map(|d| {
            (d + chrono::Duration::days(days))
                .format("%Y-%m-%d")
                .to_string()
        })
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

    // ==== ported from test/import.test.js ====================================

    use crate::accounts::{get_account_by_code, import_chart_csv, infer_rgs, seed_default_chart};
    use crate::contacts::list_contacts;
    use crate::db::open_db;
    use crate::entries::{create_entry, get_entry, list_entries, post_entry, reverse_entry};
    use crate::money::BukioError;

    fn setup() -> Connection {
        let db = open_db(":memory:").unwrap();
        seed_default_chart(&db).unwrap();
        db.execute(
            "INSERT INTO company (name, legal_form, registration_id, tax_id, vat_module) \
             VALUES ('Demo BV','bv','12345678','NL123456789B01',0)",
            [],
        )
        .unwrap();
        db
    }

    fn spec(items: &[(&str, i64)]) -> Vec<PostingSpec> {
        items
            .iter()
            .map(|(c, a)| PostingSpec {
                code: c.to_string(),
                amount_cents: *a,
                cost_center_code: None,
                vat_code: None,
                vat_amount_cents: None,
                fx_currency: None,
                fx_amount_cents: None,
            })
            .collect()
    }

    /// The details[] of an IMPORT_VALIDATION_FAILED, as plain message strings.
    fn details(err: &BukioError) -> Vec<String> {
        err.details
            .as_ref()
            .and_then(|d| d.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|e| e["error"].as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn has_detail(err: &BukioError, needle: &str) -> bool {
        details(err).iter().any(|d| d.contains(needle))
    }

    fn has_detail_prefix(err: &BukioError, prefix: &str) -> bool {
        details(err).iter().any(|d| d.starts_with(prefix))
    }

    /// {account_code: cents} of a typed entry, keys sorted.
    fn entry_sums(e: &crate::entries::Entry) -> Vec<(String, i64)> {
        let mut m: std::collections::BTreeMap<String, i64> = std::collections::BTreeMap::new();
        for p in &e.postings {
            *m.entry(p.account_code.clone()).or_insert(0) += p.amount_cents;
        }
        m.into_iter().collect()
    }

    fn posted(db: &Connection) -> Vec<Value> {
        list_entries(db, Some("posted"), None, None, 1000).unwrap()
    }

    /// The typed entry carrying this source_ref (`list_entries` omits source_ref).
    fn entry_by_source_ref(db: &Connection, r: &str) -> crate::entries::Entry {
        for e in list_entries(db, None, None, None, 1000).unwrap() {
            if let Some(id) = e["id"].as_i64() {
                if let Some(entry) = get_entry(db, id) {
                    if entry.source_ref.as_deref() == Some(r) {
                        return entry;
                    }
                }
            }
        }
        panic!("no entry with source_ref {r}")
    }

    #[test]
    fn parse_import_amount_international_dutch_and_thousands_dot() {
        assert_eq!(parse_import_amount("1234.56").unwrap(), 123456);
        assert_eq!(parse_import_amount("1234,56").unwrap(), 123456);
        assert_eq!(parse_import_amount("1.234,56").unwrap(), 123456);
        assert_eq!(parse_import_amount("0,50").unwrap(), 50);
        assert_eq!(parse_import_amount("-12,50").unwrap(), -1250);
        assert_eq!(
            parse_import_amount("abc").unwrap_err().code,
            "INVALID_AMOUNT"
        );
        assert_eq!(
            parse_import_amount("1.234").unwrap_err().code,
            "INVALID_AMOUNT"
        );
        assert_eq!(parse_import_amount("").unwrap_err().code, "INVALID_AMOUNT");
    }

    #[test]
    fn opening_balances_imports_one_posted_beginbalans_entry() {
        let db = setup();
        let res = import_opening_balances(
            &db,
            "1100,10000.00\n3000,-10000.00\n",
            Some("2026-01-01"),
            "agent:test",
            false,
        )
        .unwrap();
        assert_eq!(res["entry"]["state"].as_str(), Some("posted"));
        assert_eq!(res["accounts"].as_i64(), Some(2));
        let e = get_entry(&db, res["entry"]["id"].as_i64().unwrap()).unwrap();
        assert_eq!(e.description, "Beginbalans");
        assert_eq!(e.source, "import");
        assert_eq!(e.source_ref.as_deref(), Some("opening-balances"));
        assert_eq!(e.created_by, "agent:test");
    }

    #[test]
    fn opening_balances_dutch_code_debet_credit_layout() {
        let db = setup();
        import_opening_balances(
            &db,
            "1100,10000.00,\n3000,,10000.00\n",
            Some("2026-01-01"),
            "human:erik",
            false,
        )
        .unwrap();
        let e = entry_by_source_ref(&db, "opening-balances");
        assert_eq!(
            entry_sums(&e),
            vec![
                ("1100".to_string(), 1000000),
                ("3000".to_string(), -1000000)
            ]
        );
    }

    #[test]
    fn opening_balances_validation_collects_all_errors_and_writes_nothing() {
        let db = setup();
        let err = import_opening_balances(
            &db,
            "1100,5000.00\n9999,3000.00\n1100,abc\n",
            Some("2026-01-01"),
            "human:erik",
            false,
        )
        .unwrap_err();
        assert_eq!(err.code, "IMPORT_VALIDATION_FAILED");
        assert_eq!(details(&err).len(), 3); // ACCOUNT_NOT_FOUND, INVALID_AMOUNT, UNBALANCED
        assert!(has_detail(&err, "9999"));
        assert!(has_detail(&err, "abc"));
        assert!(has_detail_prefix(&err, "UNBALANCED"));
        assert_eq!(list_entries(&db, None, None, None, 1000).unwrap().len(), 0);
    }

    #[test]
    fn opening_balances_reimport_is_rejected() {
        let db = setup();
        import_opening_balances(
            &db,
            "1100,1.00\n3000,-1.00\n",
            Some("2026-01-01"),
            "human:erik",
            false,
        )
        .unwrap();
        let err = import_opening_balances(
            &db,
            "1100,2.00\n3000,-2.00\n",
            Some("2026-01-02"),
            "human:erik",
            false,
        )
        .unwrap_err();
        assert_eq!(err.code, "OPENING_ALREADY_IMPORTED");
    }

    #[test]
    fn opening_balances_reimport_succeeds_after_reversing_the_opening_entry() {
        let db = setup();
        let res = import_opening_balances(
            &db,
            "1100,1.00\n3000,-1.00\n",
            Some("2026-01-01"),
            "agent:test",
            false,
        )
        .unwrap();
        let first_id = res["entry"]["id"].as_i64().unwrap();
        reverse_entry(&db, first_id, "agent:test", None).unwrap();
        // the reversal nets the old balances to zero — a fresh import is allowed
        let res2 = import_opening_balances(
            &db,
            "1100,2.00\n3000,-2.00\n",
            Some("2026-01-02"),
            "agent:test",
            false,
        )
        .unwrap();
        let second_id = res2["entry"]["id"].as_i64().unwrap();
        assert_ne!(second_id, first_id, "a NEW opening entry is created");
        let e = get_entry(&db, second_id).unwrap();
        assert_eq!(
            entry_sums(&e),
            vec![("1100".to_string(), 200), ("3000".to_string(), -200)]
        );
    }

    #[test]
    fn opening_balances_dry_run_validates_and_writes_nothing() {
        let db = setup();
        let plan = import_opening_balances(
            &db,
            "1100,1.00\n3000,-1.00\n",
            Some("2026-01-01"),
            "human:erik",
            true,
        )
        .unwrap();
        assert_eq!(plan["accounts"].as_i64(), Some(2));
        assert_eq!(plan["total_debit_cents"].as_i64(), Some(100));
        assert_eq!(plan["total_credit_cents"].as_i64(), Some(100));
        assert_eq!(list_entries(&db, None, None, None, 1000).unwrap().len(), 0);
    }

    #[test]
    fn opening_balances_zero_amount_rejected() {
        let db = setup();
        let err = import_opening_balances(
            &db,
            "1100,0.00\n3000,-0.00\n",
            Some("2026-01-01"),
            "human:erik",
            false,
        )
        .unwrap_err();
        assert_eq!(err.code, "IMPORT_VALIDATION_FAILED");
        assert!(has_detail(&err, "non-zero"));
    }

    const JOURNAL: &str = "Datum;Boekstuknummer;Rekening;Tegenrekening;Bedrag;Omschrijving\n\
2026-01-05;J1;1100;8000;1210,00;Verkoop 1\n\
2026-01-05;J1;3000;1100;210,00;correctie eigen vermogen\n\
2026-01-20;J2;1100;8000;605,00;Verkoop 2\n\
2026-01-20;J2;3000;1100;105,00;correctie eigen vermogen";

    #[test]
    fn journal_one_posted_entry_per_boekstuk_two_postings_per_line() {
        let db = setup();
        let res = import_journal_csv(&db, JOURNAL, false, "agent:test", false).unwrap();
        assert_eq!(res["imported"].as_i64(), Some(2));
        let entries = posted(&db);
        assert_eq!(entries.len(), 2);
        let j1 = entry_by_source_ref(&db, "journal:J1");
        assert_eq!(j1.source, "import");
        assert_eq!(j1.description, "Verkoop 1");
        assert_eq!(
            entry_sums(&j1),
            vec![
                ("1100".to_string(), 100000),
                ("3000".to_string(), 21000),
                ("8000".to_string(), -121000),
            ]
        );
    }

    #[test]
    fn journal_idempotent_reimport_skips_existing_boekstukken() {
        let db = setup();
        assert_eq!(
            import_journal_csv(&db, JOURNAL, false, "human:erik", false).unwrap()["imported"]
                .as_i64(),
            Some(2)
        );
        let res = import_journal_csv(&db, JOURNAL, false, "human:erik", false).unwrap();
        assert_eq!(res["imported"].as_i64(), Some(0));
        assert_eq!(res["duplicates"].as_i64(), Some(2));
        assert_eq!(list_entries(&db, None, None, None, 1000).unwrap().len(), 2);
    }

    #[test]
    fn journal_comma_delimited_with_semicolon_inside_a_quoted_field() {
        let db = setup();
        let csv = "datum,boekstuknummer,rekening,tegenrekening,bedrag,omschrijving\n\
2026-01-05,J1,1100,8000,100.00,\"Consultancy; tweede termijn\"\n\
2026-01-06,J2,1100,8000,50.00,Zonder puntkomma";
        let res = import_journal_csv(&db, csv, false, "agent:test", false).unwrap();
        assert_eq!(res["imported"].as_i64(), Some(2));
        let entries = posted(&db);
        assert_eq!(entries.len(), 2);
        let j1 = entry_by_source_ref(&db, "journal:J1");
        assert_eq!(
            j1.description, "Consultancy; tweede termijn",
            "the quoted field must survive intact"
        );
        assert_eq!(
            entry_sums(&j1),
            vec![("1100".to_string(), 10000), ("8000".to_string(), -10000)]
        );
    }

    #[test]
    fn journal_unknown_account_fails_whole_file_validation_without_create_missing() {
        let db = setup();
        let err = import_journal_csv(
            &db,
            &JOURNAL.replace("3000", "9999"),
            false,
            "human:erik",
            false,
        )
        .unwrap_err();
        assert_eq!(err.code, "IMPORT_VALIDATION_FAILED");
        assert!(has_detail(&err, "9999"));
        assert_eq!(list_entries(&db, None, None, None, 1000).unwrap().len(), 0);
    }

    #[test]
    fn journal_create_missing_infers_type_from_net_movement() {
        let db = setup();
        let csv = JOURNAL.replace("3000", "9999").replace("8000", "9998");
        let res = import_journal_csv(&db, &csv, true, "human:erik", false).unwrap();
        assert_eq!(res["imported"].as_i64(), Some(2));
        assert!(res["accounts_created"].as_array().unwrap().len() >= 2);
        // 9999 net +315 (debet) -> expense; 9998 net -1815 (credit) -> income
        assert_eq!(
            get_account_by_code(&db, "9999").unwrap()["type"].as_str(),
            Some("expense")
        );
        assert_eq!(
            get_account_by_code(&db, "9998").unwrap()["type"].as_str(),
            Some("income")
        );
    }

    #[test]
    fn journal_bad_amount_and_date_mismatch_are_both_collected() {
        let db = setup();
        let csv = "Datum;Boekstuknummer;Rekening;Tegenrekening;Bedrag\n\
2026-01-05;J1;1100;8000;abc\n\
2026-01-06;J1;1100;8000;10.00";
        let err = import_journal_csv(&db, csv, false, "human:erik", false).unwrap_err();
        assert_eq!(err.code, "IMPORT_VALIDATION_FAILED");
        assert!(has_detail_prefix(&err, "INVALID_AMOUNT"));
        assert!(has_detail_prefix(&err, "DATE_MISMATCH"));
    }

    #[test]
    fn journal_missing_required_header_column_rejected() {
        let db = setup();
        let err = import_journal_csv(
            &db,
            "Datum;Boekstuknummer;Rekening;Bedrag\n2026-01-05;J1;1100;10.00",
            false,
            "human:erik",
            false,
        )
        .unwrap_err();
        assert_eq!(err.code, "INVALID_CSV_HEADER");
    }

    // --- XAF 4.0 (Belastingdienst <Xaf> layout) -----------------------------

    const XAF: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Xaf xmlns="http://www.auditfiles.nl/XAF/4.0">
  <XafHeader>
    <Version>4.0</Version>
    <CompanyName>Demo BV</CompanyName>
    <CompanyID>12345678</CompanyID>
    <FiscalYear>2026</FiscalYear>
    <StartDate>2026-01-01</StartDate>
    <EndDate>2026-12-31</EndDate>
    <SoftwareName>OudPakket</SoftwareName>
    <SoftwareVersion>1.0</SoftwareVersion>
  </XafHeader>
  <Rekeningen>
    <Rekening><RekeningCode>1250</RekeningCode><RekeningOmschrijving>Kas klein</RekeningOmschrijving><RekeningSoort>Balans</RekeningSoort></Rekening>
    <Rekening><RekeningCode>8000</RekeningCode><RekeningOmschrijving>Omzet</RekeningOmschrijving><RekeningSoort>Winst en Verlies</RekeningSoort></Rekening>
  </Rekeningen>
  <Mutaties>
    <Mutatie>
      <Boekstuknummer>2026-0001</Boekstuknummer>
      <Datum>2026-01-15</Datum>
      <Boekingen>
        <Boeking><RekeningCode>1250</RekeningCode><TegenrekeningCode>8000</TegenrekeningCode><Bedrag>100,00</Bedrag><BtwCode>0</BtwCode><Omschrijving>Contante verkoop</Omschrijving></Boeking>
      </Boekingen>
    </Mutatie>
  </Mutaties>
</Xaf>"#;

    #[test]
    fn xaf_imports_mutaties_and_creates_file_chart_accounts() {
        let db = setup();
        let res = import_xaf(&db, XAF, "agent:test", false).unwrap();
        assert_eq!(res["imported"].as_i64(), Some(1));
        assert_eq!(res["header"]["company_name"].as_str(), Some("Demo BV"));
        assert_eq!(res["header"]["fiscal_year"].as_str(), Some("2026"));
        // created from the file (debet net -> asset)
        let kas = get_account_by_code(&db, "1250").expect("1250 Kas klein should be created");
        assert_eq!(kas["type"].as_str(), Some("asset"));
        assert!(res["accounts_created"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["code"].as_str() == Some("1250")));
        let e = entry_by_source_ref(&db, "2026-0001");
        assert_eq!(e.source, "xaf");
    }

    #[test]
    fn xaf_btw_codes_are_reported_not_booked() {
        let db = setup();
        let res = import_xaf(&db, XAF, "human:erik", false).unwrap();
        assert_eq!(res["ignored_btw_codes"], json!(["0"]));
        let e = get_entry(
            &db,
            list_entries(&db, None, None, None, 1000).unwrap()[0]["id"]
                .as_i64()
                .unwrap(),
        )
        .unwrap();
        assert!(e.postings.iter().all(|p| p.vat_code.is_none()));
    }

    #[test]
    fn xaf_idempotent_per_boekstuknummer() {
        let db = setup();
        import_xaf(&db, XAF, "human:erik", false).unwrap();
        let res = import_xaf(&db, XAF, "human:erik", false).unwrap();
        assert_eq!(res["imported"].as_i64(), Some(0));
        assert_eq!(res["duplicates"].as_i64(), Some(1));
    }

    #[test]
    fn xaf_rekening_not_in_file_chart_nor_chart_of_accounts_is_a_validation_error() {
        let db = setup();
        let bad = XAF.replace(
            "</Boeking>",
            "</Boeking><Boeking><RekeningCode>9999</RekeningCode><TegenrekeningCode>8000</TegenrekeningCode><Bedrag>50,00</Bedrag></Boeking>",
        );
        let err = import_xaf(&db, &bad, "human:erik", false).unwrap_err();
        assert_eq!(err.code, "IMPORT_VALIDATION_FAILED");
        assert!(has_detail(&err, "9999"));
        assert_eq!(list_entries(&db, None, None, None, 1000).unwrap().len(), 0);
    }

    #[test]
    fn xaf_unsupported_version_rejected() {
        let db = setup();
        let err = import_xaf(
            &db,
            &XAF.replace("<Version>4.0</Version>", "<Version>3.1</Version>"),
            "human:erik",
            false,
        )
        .unwrap_err();
        assert_eq!(err.code, "INVALID_XAF");
    }

    #[test]
    fn xaf_company_mismatch_blocks_importing_another_company() {
        let db = setup();
        let err = import_xaf(
            &db,
            &XAF.replace(
                "<CompanyID>12345678</CompanyID>",
                "<CompanyID>99999999</CompanyID>",
            ),
            "human:erik",
            false,
        )
        .unwrap_err();
        assert_eq!(err.code, "COMPANY_MISMATCH");
    }

    #[test]
    fn xaf_name_mismatch_is_only_a_warning() {
        let db = setup();
        let res = import_xaf(
            &db,
            &XAF.replace(
                "<CompanyName>Demo BV</CompanyName>",
                "<CompanyName>Demo B.V. Rotterdam</CompanyName>",
            ),
            "human:erik",
            false,
        )
        .unwrap();
        assert_eq!(res["imported"].as_i64(), Some(1));
        assert!(res["company_mismatch"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w.as_str().unwrap_or("").contains("name differs")));
    }

    #[test]
    fn xaf_dry_run_validates_and_writes_nothing() {
        let db = setup();
        let plan = import_xaf(&db, XAF, "human:erik", true).unwrap();
        assert_eq!(plan["mutaties"].as_i64(), Some(1));
        assert_eq!(plan["accounts_to_create"].as_i64(), Some(1));
        assert_eq!(list_entries(&db, None, None, None, 1000).unwrap().len(), 0);
        assert!(get_account_by_code(&db, "1250").is_none());
    }

    // --- XAF 4.0 AuditFile layout (root <AuditFile>) ------------------------

    const AUDITFILE_XAF: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<AuditFile xmlns="https://www.bukio.nl/xaf/4.0" version="4.0" exportedAt="2026-08-06T08:48:22Z">
  <Header>
    <AuditFileVersion>4.0</AuditFileVersion>
    <CompanyID>1</CompanyID>
    <CompanyName>Demo BV</CompanyName>
    <FiscalYear>2026</FiscalYear>
    <StartDate>2026-01-01</StartDate>
    <EndDate>2026-12-31</EndDate>
    <CurrencyCode>EUR</CurrencyCode>
    <SoftwareDescription>Bukio</SoftwareDescription>
  </Header>
  <MasterFiles>
    <GeneralLedgerAccounts>
      <Account><AccountID>1100</AccountID><AccountDescription>Gebouwen</AccountDescription><AccountType>Asset</AccountType></Account>
      <Account><AccountID>8000</AccountID><AccountDescription>Omzet</AccountDescription><AccountType>Revenue</AccountType></Account>
      <Account><AccountID>5100</AccountID><AccountDescription>Crediteuren</AccountDescription><AccountType>Liability</AccountType></Account>
      <Account><AccountID>7150</AccountID><AccountDescription>Platformkosten</AccountDescription><AccountType>Expense</AccountType></Account>
    </GeneralLedgerAccounts>
  </MasterFiles>
  <GeneralLedgerEntries>
    <Journal>
      <JournalID>SAL</JournalID>
      <Transaction>
        <TransactionID>2026-00001</TransactionID>
        <TransactionDate>2026-01-15</TransactionDate>
        <Description>Factuur 2026-0001</Description>
        <Line>
          <RecordID>1</RecordID>
          <AccountID>1100</AccountID>
          <Description>Factuur 2026-0001</Description>
          <DebitAmount>121.00</DebitAmount>
          <TaxInformation><TaxType>VAT</TaxType><TaxCode>NOVAT</TaxCode><TaxPercentage>0.00</TaxPercentage></TaxInformation>
        </Line>
        <Line>
          <RecordID>2</RecordID>
          <AccountID>8000</AccountID>
          <Description>Factuur 2026-0001</Description>
          <CreditAmount>121.00</CreditAmount>
        </Line>
      </Transaction>
    </Journal>
  </GeneralLedgerEntries>
</AuditFile>"#;

    #[test]
    fn auditfile_imports_transaction_creates_and_renames_chart_accounts() {
        let db = setup();
        // the dry-run plan must carry the renamed key (producer + CLI render agree)
        let plan = import_xaf(&db, AUDITFILE_XAF, "human:erik", true).unwrap();
        assert_eq!(plan["company"]["registration_id"].as_str(), Some("1")); // the file's <CompanyID>
        assert!(plan["company"].get("kvk").is_none());
        let res = import_xaf(&db, AUDITFILE_XAF, "agent:test", false).unwrap();
        assert_eq!(res["imported"].as_i64(), Some(1));
        assert_eq!(res["header"]["company_name"].as_str(), Some("Demo BV"));
        assert_eq!(res["header"]["software"].as_str(), Some("Bukio"));
        // file accounts missing from the starter chart are created with mapped types
        assert_eq!(
            get_account_by_code(&db, "5100").unwrap()["type"].as_str(),
            Some("liability")
        );
        assert_eq!(
            get_account_by_code(&db, "7150").unwrap()["type"].as_str(),
            Some("expense")
        );
        // colliding codes are renamed on the empty ledger (1100 Bank -> Gebouwen)
        assert_eq!(
            get_account_by_code(&db, "1100").unwrap()["name"].as_str(),
            Some("Gebouwen")
        );
        assert_eq!(
            res["accounts_updated"],
            json!([{
                "code": "1100", "from": "Bank", "to": "Gebouwen", "type": "asset",
                "normal_balance": "debit", "taxonomy_code": "BMVA.02",
            }])
        );
        // NOVAT reported, not booked
        assert_eq!(res["ignored_btw_codes"], json!(["NOVAT"]));
        let e = entry_by_source_ref(&db, "2026-00001");
        assert_eq!(e.source, "xaf");
        assert_eq!(e.description, "Factuur 2026-0001");
        assert_eq!(
            entry_sums(&e),
            vec![("1100".to_string(), 12100), ("8000".to_string(), -12100)]
        );
    }

    #[test]
    fn auditfile_accounts_with_postings_are_not_renamed() {
        let db = setup();
        let e = create_entry(
            &db,
            CreateEntry {
                date: "2026-01-01",
                description: "Bestaat al",
                postings: spec(&[("1100", 100), ("3000", -100)]),
                source: "manual",
                source_ref: None,
                actor: "human:erik",
            },
        )
        .unwrap();
        post_entry(&db, e.id, "human:erik").unwrap();
        let res = import_xaf(&db, AUDITFILE_XAF, "human:erik", false).unwrap();
        assert_eq!(
            get_account_by_code(&db, "1100").unwrap()["name"].as_str(),
            Some("Bank")
        );
        assert!(res["chart_warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w.as_str().unwrap_or("").contains("1100")));
    }

    #[test]
    fn auditfile_unbalanced_transaction_fails_whole_file_validation() {
        let db = setup();
        let bad = AUDITFILE_XAF.replace(
            "<CreditAmount>121.00</CreditAmount>",
            "<CreditAmount>100.00</CreditAmount>",
        );
        let err = import_xaf(&db, &bad, "human:erik", false).unwrap_err();
        assert_eq!(err.code, "IMPORT_VALIDATION_FAILED");
        assert!(has_detail_prefix(&err, "UNBALANCED"));
        assert_eq!(list_entries(&db, None, None, None, 1000).unwrap().len(), 0);
    }

    #[test]
    fn auditfile_idempotent_per_transaction_id() {
        let db = setup();
        import_xaf(&db, AUDITFILE_XAF, "human:erik", false).unwrap();
        let res = import_xaf(&db, AUDITFILE_XAF, "human:erik", false).unwrap();
        assert_eq!(res["imported"].as_i64(), Some(0));
        assert_eq!(res["duplicates"].as_i64(), Some(1));
    }

    #[test]
    fn auditfile_dry_run_lists_renames_and_writes_nothing() {
        let db = setup();
        let plan = import_xaf(&db, AUDITFILE_XAF, "human:erik", true).unwrap();
        assert_eq!(plan["mutaties"].as_i64(), Some(1));
        assert_eq!(plan["accounts_to_create"].as_i64(), Some(2));
        assert_eq!(
            plan["accounts_to_rename"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| r["code"].as_str().unwrap_or(""))
                .collect::<Vec<_>>(),
            vec!["1100"]
        );
        assert_eq!(list_entries(&db, None, None, None, 1000).unwrap().len(), 0);
        assert_eq!(
            get_account_by_code(&db, "1100").unwrap()["name"].as_str(),
            Some("Bank")
        );
    }

    #[test]
    fn auditfile_company_id_mismatch_is_an_error() {
        let db = setup();
        let err = import_xaf(
            &db,
            &AUDITFILE_XAF.replace(
                "<CompanyID>1</CompanyID>",
                "<CompanyID>99999999</CompanyID>",
            ),
            "human:erik",
            false,
        )
        .unwrap_err();
        assert_eq!(err.code, "COMPANY_MISMATCH");
    }

    #[test]
    fn auditfile_company_name_mismatch_is_only_a_warning() {
        let db = setup();
        let res = import_xaf(
            &db,
            &AUDITFILE_XAF.replace(
                "<CompanyName>Demo BV</CompanyName>",
                "<CompanyName>Demo B.V. Rotterdam</CompanyName>",
            ),
            "human:erik",
            false,
        )
        .unwrap();
        assert_eq!(res["imported"].as_i64(), Some(1));
        assert!(res["company_mismatch"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w.as_str().unwrap_or("").contains("name differs")));
    }

    // --- contacts from audit files ------------------------------------------

    const CONTACTS_XAF: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<AuditFile xmlns="https://www.bukio.nl/xaf/4.0" version="4.0">
  <Header>
    <AuditFileVersion>4.0</AuditFileVersion>
    <CompanyName>Demo BV</CompanyName>
  </Header>
  <MasterFiles>
    <Customers>
      <Customer>
        <CustomerID>1</CustomerID>
        <CompanyName>Daan van der Leen</CompanyName>
        <Email>daanleen@gmail.com</Email>
        <Address>
          <StreetName>Lamarckhof 9-1</StreetName>
          <PostalCode>1098TK</PostalCode>
          <City>Amsterdam</City>
          <Country>NL</Country>
        </Address>
      </Customer>
    </Customers>
    <Suppliers>
      <Supplier>
        <SupplierID>13</SupplierID>
        <CompanyName>Anomaly</CompanyName>
        <Contact>Matt</Contact>
        <Email>help@anoma.ly</Email>
        <Address>
          <StreetName>2443 Fillmore Street</StreetName>
          <PostalCode>94115</PostalCode>
          <City>San Francisco</City>
          <Country>us</Country>
        </Address>
      </Supplier>
      <Supplier>
        <SupplierID>14</SupplierID>
        <CompanyName>DeluxHost</CompanyName>
        <Email></Email>
      </Supplier>
    </Suppliers>
  </MasterFiles>
</AuditFile>"#;

    #[test]
    fn import_contacts_suppliers_and_customers_mapped() {
        let db = setup();
        let res = import_contacts(&db, CONTACTS_XAF, "agent:test", false).unwrap();
        assert_eq!(res["imported"].as_i64(), Some(3));
        assert_eq!(res["suppliers"].as_i64(), Some(2));
        assert_eq!(res["customers"].as_i64(), Some(1));
        let contacts = list_contacts(&db).unwrap();
        let find = |name: &str| {
            contacts
                .iter()
                .find(|c| c["name"].as_str() == Some(name))
                .unwrap_or_else(|| panic!("contact {name}"))
        };
        let daan = find("Daan van der Leen");
        assert_eq!(daan["address"].as_str(), Some("Lamarckhof 9-1"));
        assert_eq!(daan["postal_code"].as_str(), Some("1098TK"));
        assert_eq!(daan["city"].as_str(), Some("Amsterdam"));
        assert_eq!(daan["country"].as_str(), Some("NL"));
        assert_eq!(daan["email"].as_str(), Some("daanleen@gmail.com"));
        assert_eq!(find("Anomaly")["country"].as_str(), Some("US")); // normalised to uppercase
        let delux = find("DeluxHost");
        assert!(delux["address"].is_null()); // no address in the file
        let actor: String = db
            .query_row(
                "SELECT actor FROM audit_log WHERE action = 'import.contacts' ORDER BY id DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(actor, "agent:test");
    }

    #[test]
    fn import_contacts_idempotent_by_name() {
        let db = setup();
        import_contacts(&db, CONTACTS_XAF, "human:erik", false).unwrap();
        let res = import_contacts(&db, CONTACTS_XAF, "human:erik", false).unwrap();
        assert_eq!(res["imported"].as_i64(), Some(0));
        assert_eq!(res["duplicates"].as_i64(), Some(3));
    }

    #[test]
    fn import_contacts_entry_without_a_name_fails_whole_file_validation() {
        let db = setup();
        let bad = CONTACTS_XAF.replace("<CompanyName>DeluxHost</CompanyName>", "");
        let err = import_contacts(&db, &bad, "human:erik", false).unwrap_err();
        assert_eq!(err.code, "IMPORT_VALIDATION_FAILED");
        assert!(has_detail_prefix(&err, "CONTACT_REQUIRED"));
        assert_eq!(list_contacts(&db).unwrap().len(), 0);
    }

    #[test]
    fn import_contacts_dry_run_writes_nothing() {
        let db = setup();
        let plan = import_contacts(&db, CONTACTS_XAF, "human:erik", true).unwrap();
        assert_eq!(plan["contacts_to_create"].as_i64(), Some(3));
        assert_eq!(plan["duplicates"].as_i64(), Some(0));
        assert_eq!(list_contacts(&db).unwrap().len(), 0);
    }

    // --- RGS enforcement on import ------------------------------------------

    #[test]
    fn infer_rgs_keywords_within_type_then_type_based_fallbacks() {
        assert_eq!(infer_rgs("income", "Omzet diensten"), Some("WOVB.82")); // diensten before omzet
        assert_eq!(infer_rgs("income", "Omzet goederen"), Some("WOMZ.80"));
        assert_eq!(infer_rgs("income", "Overige opbrengsten"), Some("WOVB.82"));
        assert_eq!(infer_rgs("expense", "Afschrijvingskosten"), Some("WAFS.41"));
        assert_eq!(infer_rgs("expense", "Bankkosten"), Some("WFBE.84"));
        assert_eq!(infer_rgs("expense", "Rentebaten"), Some("WFBE.84"));
        assert_eq!(infer_rgs("expense", "Kosten IT"), Some("WBED.42"));
        assert_eq!(infer_rgs("expense", "Inkoopwaarde"), Some("WKPR.70"));
        assert_eq!(infer_rgs("expense", "Voorraadmutatie"), Some("WKPR.70"));
        assert_eq!(
            infer_rgs("expense", "Kosten uitbesteed werk"),
            Some("WKPR.70")
        );
        assert_eq!(infer_rgs("expense", "Pensioenlasten"), Some("WPER.40"));
        assert_eq!(infer_rgs("asset", "Bank Rabobank ZZP"), Some("BLIM.10"));
        assert_eq!(infer_rgs("asset", "Hardware"), Some("BMVA.02"));
        assert_eq!(
            infer_rgs("asset", "Te vorderen btw hoog 21%"),
            Some("BVOR.11")
        );
        assert_eq!(infer_rgs("asset", "Vraagposten"), Some("BVOR.11"));
        assert_eq!(infer_rgs("asset", "Kruisposten"), Some("BVOR.11"));
        assert_eq!(
            infer_rgs("asset", "Cumulatieve afschrijvingen"),
            Some("BMVA.02")
        ); // contra-MVA
        assert_eq!(infer_rgs("liability", "Crediteuren"), Some("BSCH.12"));
        assert_eq!(infer_rgs("equity", "Privéstortingen"), Some("BEIV.05"));
    }

    #[test]
    fn auditfile_created_accounts_carry_inferred_rgs_codes() {
        let db = setup();
        let res = import_xaf(&db, AUDITFILE_XAF, "agent:test", false).unwrap();
        // 5100 Crediteuren + 7150 Platformkosten are new; 1100/8000 exist in the seed
        assert_eq!(res["accounts_created"].as_array().unwrap().len(), 2);
        assert_eq!(
            get_account_by_code(&db, "1100").unwrap()["taxonomy_code"].as_str(),
            Some("BMVA.02")
        ); // renamed Gebouwen: BLIM.10 -> BMVA.02
        assert_eq!(
            get_account_by_code(&db, "8000").unwrap()["taxonomy_code"].as_str(),
            Some("WOMZ.80")
        ); // Omzet (revenue)
        assert_eq!(
            get_account_by_code(&db, "5100").unwrap()["taxonomy_code"].as_str(),
            Some("BSCH.12")
        ); // Crediteuren
        assert_eq!(
            get_account_by_code(&db, "7150").unwrap()["taxonomy_code"].as_str(),
            Some("WBED.42")
        ); // Platformkosten
    }

    #[test]
    fn import_xaf_reimport_backfills_rgs_codes_on_accounts_that_lack_them() {
        let db = setup();
        // pre-fix chart state: 8000 exists WITHOUT an rgs code
        db.execute(
            "UPDATE accounts SET taxonomy_code = NULL WHERE code = '8000'",
            [],
        )
        .unwrap();
        let res = import_xaf(&db, AUDITFILE_XAF, "agent:test", false).unwrap();
        assert_eq!(
            get_account_by_code(&db, "8000").unwrap()["taxonomy_code"].as_str(),
            Some("WOMZ.80")
        );
        assert!(res["accounts_rgs_backfilled"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["code"].as_str() == Some("8000")
                && a["taxonomy_code"].as_str() == Some("WOMZ.80")));
        // idempotent: second re-run backfills nothing
        let res2 = import_xaf(&db, AUDITFILE_XAF, "agent:test", false).unwrap();
        assert_eq!(res2["accounts_rgs_backfilled"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn import_journal_create_missing_accounts_also_get_rgs_codes() {
        let db = setup();
        let csv = JOURNAL.replace("3000", "9999").replace("8000", "9998");
        import_journal_csv(&db, &csv, true, "human:erik", false).unwrap();
        assert_eq!(
            get_account_by_code(&db, "9999").unwrap()["taxonomy_code"].as_str(),
            Some("WBED.42")
        ); // expense fallback
        assert_eq!(
            get_account_by_code(&db, "9998").unwrap()["taxonomy_code"].as_str(),
            Some("WOVB.82")
        ); // income fallback
    }

    #[test]
    fn import_chart_csv_without_an_rgs_column_infers_rgs_codes() {
        let db = setup();
        let csv = "code,name,type,normal_balance\n1350,Hardware,asset,debit\n8300,Omzet diensten,income,credit";
        let res = import_chart_csv(&db, csv).unwrap();
        assert_eq!(res.created, 2);
        assert_eq!(
            get_account_by_code(&db, "1350").unwrap()["taxonomy_code"].as_str(),
            Some("BMVA.02")
        );
        assert_eq!(
            get_account_by_code(&db, "8300").unwrap()["taxonomy_code"].as_str(),
            Some("WOVB.82")
        );
    }
}
