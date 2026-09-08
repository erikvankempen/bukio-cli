// bukio-cli — agent-first double-entry bookkeeping for SMEs.
// Copyright (c) 2026 Erik van Kempen.
// SPDX-License-Identifier: Apache-2.0
//
// CLI entry: same argv contract + JSON protocol as bin/bukio.js. During the
// port the JS shim execs this binary; the sign gate stays JS until the final
// phase. Human (non-JSON) output is deferred — the parity harness compares
// --json output (the machine contract everything consumes).

mod actor;
mod audit;
mod canonical;
mod db;
mod money;
mod sign;
mod dates;
mod entries;
mod accounts;
mod reports;
mod vat;
mod fx;

use money::{BukioError, Result};
use serde_json::{json, Value};
use std::io::Write;

fn ok(data: Value) {
    println!("{}", serde_json::to_string_pretty(&json!({ "ok": true, "data": data })).unwrap());
}

fn fail_json(err: &BukioError) -> ! {
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({ "ok": false, "error": { "code": err.code, "message": err.message } })).unwrap()
    );
    std::process::exit(1);
}

fn arg(argv: &[String], flag: &str) -> Option<String> {
    let eq = format!("{flag}=");
    argv.iter().position(|a| a == flag).map(|i| argv.get(i + 1).cloned())
        .flatten()
        .or_else(|| argv.iter().find_map(|a| a.strip_prefix(&eq).map(String::from)))
}

fn has_flag(argv: &[String], flag: &str) -> bool {
    argv.iter().any(|a| a == flag)
}

fn repeated(argv: &[String], flag: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < argv.len() {
        if argv[i] == flag {
            if let Some(v) = argv.get(i + 1) {
                out.push(v.clone());
            }
            i += 2;
        } else if let Some(v) = argv[i].strip_prefix(&format!("{flag}=")) {
            out.push(v.to_string());
            i += 1;
        } else {
            i += 1;
        }
    }
    out
}

fn no_database(db_path: &str) -> BukioError {
    BukioError::new("NO_DATABASE", format!("no database at {db_path} — run 'bukio init' first"))
}

fn ensure_db_exists(db_path: &str) -> Result<std::path::PathBuf> {
    let p = std::path::PathBuf::from(db_path);
    if !p.exists() {
        return Err(no_database(db_path));
    }
    Ok(p)
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    // pull global flags anywhere in argv (JS commander allows both positions)
    let db_path = arg(&argv, "--db")
        .or_else(|| std::env::var("BUKIO_DB").ok())
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
            format!("{home}/.bukio/bukio.db")
        });
    let actor = arg(&argv, "--actor")
        .or_else(|| std::env::var("BUKIO_ACTOR").ok())
        .unwrap_or_default();
    let json_mode = has_flag(&argv, "--json");
    let dry_run = has_flag(&argv, "--dry-run");

    let result = dispatch(&argv, &db_path, &actor, dry_run);
    match result {
        Ok(data) => {
            if json_mode {
                ok(data);
            } else {
                // human output: minimal — print data JSON for now
                // ponytail: human renderers per command ship with the JS-layer removal
                println!("{}", serde_json::to_string_pretty(&data).unwrap());
            }
        }
        Err(e) => {
            if json_mode {
                fail_json(&e);
            } else {
                eprintln!("error [{}]: {}", e.code, e.message);
                std::process::exit(1);
            }
        }
    }
}

fn dispatch(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    // walk argv: a token starting with '-' consumes the next token as its
    // value unless that token is itself a flag; non-flag tokens outside a
    // flag-value position are positionals (command path)
    let mut positional: Vec<&str> = Vec::new();
    let mut i = 0;
    while i < argv.len() {
        let a = &argv[i];
        if a.starts_with('-') && a != "-" {
            let takes_value = !matches!(
                a.as_str(),
                "--json" | "--dry-run" | "--post" | "--kor" | "--include-inactive"
            );
            if takes_value && i + 1 < argv.len() && !argv[i + 1].starts_with('-') {
                i += 2;
            } else {
                i += 1;
            }
        } else {
            positional.push(a);
            i += 1;
        }
    }
    match positional.as_slice() {
        ["init"] => cmd_init(argv, db_path, actor, dry_run),
        ["entry", "add"] => cmd_entry_add(argv, db_path, actor, dry_run),
        ["entry", "post"] => cmd_entry_post(argv, db_path, actor, dry_run),
        ["entry", "reverse"] => cmd_entry_reverse(argv, db_path, actor, dry_run),
        ["entry", "list"] => cmd_entry_list(argv, db_path),
        ["entry", "show"] => cmd_entry_show(argv, db_path),
        ["account", "add"] => cmd_account_add(argv, db_path, actor, dry_run),
        ["account", "list"] => cmd_account_list(argv, db_path),
        ["account", "show"] => cmd_account_show(argv, db_path),
        ["account", "deactivate"] => cmd_account_deactivate(argv, db_path, actor, dry_run),
        ["account", "reactivate"] => cmd_account_reactivate(argv, db_path, actor, dry_run),
        ["account", "import"] => cmd_account_import(argv, db_path, actor, dry_run),
        ["cost-center", "add"] => cmd_cc_add(argv, db_path, actor),
        ["cost-center", "list"] => cmd_cc_list(argv, db_path),
        ["cost-center", "deactivate"] | ["cost-center", "reactivate"] => cmd_cc_toggle(argv, db_path, actor),
        ["report", "trial-balance"] => cmd_tb(argv, db_path),
        ["report", "balance-sheet"] | ["report", "balans"] => cmd_balans(argv, db_path),
        ["report", "pnl"] => cmd_pnl(argv, db_path),
        ["report", "journal"] => cmd_journal(argv, db_path),
        ["audit"] | ["audit", "list"] => cmd_audit_list(argv, db_path),
        ["vat", "enable"] => cmd_vat_enable(db_path, actor),
        ["vat", "codes"] => cmd_vat_codes(db_path),
        ["vat", "book"] => cmd_vat_book(argv, db_path, actor, dry_run),
        ["vat", "readout"] => cmd_vat_readout(argv, db_path, actor),
        ["fx", "set"] => cmd_fx_set(argv, db_path, actor, dry_run),
        ["fx", "show"] => cmd_fx_show(argv, db_path),
        ["fx", "list"] => cmd_fx_list(argv, db_path),
        ["vat", "file"] => cmd_vat_file(argv, db_path, actor, dry_run),
        ["vat", "settle"] => cmd_vat_settle(argv, db_path, actor, dry_run),
        ["audit", "verify"] => cmd_audit_verify(db_path),
        _ => Err(BukioError::new(
            "UNKNOWN_COMMAND",
            format!("command '{}' is not ported to Rust yet", positional.join(" ")),
        )),
    }
}

fn require_actor(actor: &str) -> Result<()> {
    match actor::actor_error(if actor.is_empty() { None } else { Some(actor) }) {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

fn cmd_init(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    require_actor(actor)?;
    let name = arg(argv, "--name").ok_or_else(|| BukioError::new("MISSING_ARG", "--name is required"))?;
    let country = arg(argv, "--country").unwrap_or_else(|| "NL".into());
    let profile = accounts::get_profile(&country)?;
    let legal_form = arg(argv, "--legal-form").unwrap_or_else(|| "eenmanszaak".into());
    let legal_forms: Vec<&str> = profile["meta"]["legalForms"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    if !legal_forms.contains(&legal_form.as_str()) {
        return Err(BukioError::new(
            "INVALID_LEGAL_FORM",
            format!("legal form '{}' must be one of {}", legal_form, legal_forms.join(", ")),
        ));
    }
    let vat = arg(argv, "--vat").unwrap_or_else(|| "off".into());
    if vat != "on" && vat != "off" {
        return Err(BukioError::new("INVALID_VAT_CHOICE", format!("--vat must be 'on' or 'off', got '{vat}'")));
    }
    let kor = has_flag(argv, "--kor");
    if kor && profile["tax"]["smallBusinessScheme"].as_str() != Some("kor") {
        return Err(BukioError::new("INVALID_VAT_CHOICE", format!("--kor is not available for country {}", country)));
    }
    let iban = arg(argv, "--iban");
    if let Some(i) = &iban {
        if !dates::is_valid_iban(i) {
            return Err(BukioError::new("INVALID_IBAN", format!("'{i}' is not a valid IBAN")));
        }
    }
    let chart_len = profile["reporting"]["defaultChart"].as_array().map(|a| a.len()).unwrap_or(0);
    let company = json!({
        "name": name,
        "registration_id": arg(argv, "--registration-id"),
        "legal_form": legal_form,
        "tax_id": arg(argv, "--tax-id"),
        "iban": iban,
        "address": arg(argv, "--address"),
        "postal_code": arg(argv, "--postal-code"),
        "city": arg(argv, "--city"),
        "vat_module": if kor { 0 } else if vat == "on" { 1 } else { 0 },
        "kor_flag": if kor { 1 } else { 0 },
        "fiscal_year_end": arg(argv, "--fiscal-year-end")
            .unwrap_or_else(|| profile["meta"]["defaultFiscalYearEnd"].as_str().unwrap_or("12-31").to_string()),
        "country": profile["meta"]["country"].as_str().unwrap_or("NL"),
        "base_currency": profile["meta"]["baseCurrency"].as_str().unwrap_or("EUR"),
        "locale": profile["meta"]["locale"].as_str().unwrap_or("en"),
        "profile_version": 1,
    });
    if dry_run {
        return Ok(json!({
            "action": "create company + seed default chart",
            "company": company,
            "db": db_path,
            "db_exists": std::path::Path::new(db_path).exists(),
            "chart": { "accounts": chart_len + if company["vat_module"] == 1 { 2 } else { 0 } },
            "dry_run": true,
        }));
    }
    if std::path::Path::new(db_path).exists() {
        let existing = db::open_db(db_path).map_err(|e| BukioError::new("DB_ERROR", e.to_string()))?;
        let has: i64 = existing.query_row("SELECT count(*) FROM company", [], |r| r.get(0)).unwrap_or(0);
        if has > 0 {
            return Err(BukioError::new("ALREADY_INITIALISED", format!("database {db_path} already has a company")));
        }
    }
    if let Some(parent) = std::path::Path::new(db_path).parent() {
        std::fs::create_dir_all(parent).map_err(|e| BukioError::new("IO_ERROR", e.to_string()))?;
    }
    let db = db::open_db(db_path).map_err(|e| BukioError::new("DB_ERROR", e.to_string()))?;
    db.execute(
        "INSERT INTO company (name, registration_id, legal_form, tax_id, iban, address, postal_code, city, vat_module, kor_flag, fiscal_year_end, country, base_currency, locale, profile_version)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, 1)",
        rusqlite::params![
            company["name"].as_str().unwrap(), company["registration_id"].as_str(),
            company["legal_form"].as_str().unwrap(), company["tax_id"].as_str(),
            company["iban"].as_str(), company["address"].as_str(), company["postal_code"].as_str(),
            company["city"].as_str(), company["vat_module"].as_i64().unwrap(), company["kor_flag"].as_i64().unwrap(),
            company["fiscal_year_end"].as_str().unwrap(), company["country"].as_str().unwrap(),
            company["base_currency"].as_str().unwrap(), company["locale"].as_str().unwrap(),
        ],
    )
    .map_err(|e| BukioError::new("DB_ERROR", e.to_string()))?;
    let created = accounts::seed_default_chart(&db)?;
    let mut vat_created = 0;
    if company["vat_module"] == 1 {
        let vat_result = vat::enable_vat_module(&db, actor)?;
        vat_created = vat_result["accounts"].as_array().map(|a| a.len()).unwrap_or(0);
    }
    audit::record(&db, audit::RecordArgs {
        actor,
        action: "company.init",
        command: Some("init"),
        args: Some(company.clone()),
        outcome: "ok",
        entry_ids: vec![],
    })?;
    let total = accounts::list_accounts(&db, None, true)?.len();
    Ok(json!({ "company": company, "db": db_path, "chart": { "accounts": total, "created": created + vat_created }, "dryRun": false }))
}

fn open_existing(db_path: &str) -> Result<rusqlite::Connection> {
    ensure_db_exists(db_path)?;
    db::open_db(db_path).map_err(|e| BukioError::new("DB_ERROR", e.to_string()))
}

fn cmd_entry_add(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    require_actor(actor)?;
    let date = arg(argv, "--date").unwrap_or_else(|| dates::today_iso());
    let desc = arg(argv, "--desc").ok_or_else(|| BukioError::new("MISSING_ARG", "--desc is required"))?;
    let postings_raw = repeated(argv, "--postings");
    let specs = entries::parse_posting_specs(&postings_raw)?;
    if dry_run {
        dates::validate_date(&date)?;
        if desc.trim().is_empty() {
            return Err(BukioError::new("INVALID_DESCRIPTION", "description is required"));
        }
        if specs.len() < 2 {
            return Err(BukioError::new("TOO_FEW_POSTINGS", "an entry needs at least 2 postings"));
        }
        let sum: i64 = specs.iter().map(|s| s.amount_cents).sum();
        if sum != 0 {
            return Err(BukioError::new("UNBALANCED", format!("postings do not sum to zero (sum = {sum} cents)")));
        }
        let plan: Vec<Value> = specs
            .iter()
            .map(|s| json!({ "code": s.code, "amount_cents": s.amount_cents, "amount": money::format_amount(s.amount_cents), "cost_center": s.cost_center_code }))
            .collect();
        // same account-validation semantics as JS: open the DB only if the
        // file exists (never create on dry-run)
        let validation = if std::path::Path::new(db_path).exists() {
            match open_existing(db_path) {
                Ok(db) => {
                    let mut bad = false;
                    for s in &specs {
                        match db.query_row(
                            "SELECT active FROM accounts WHERE code = ?1",
                            [&s.code], |r| r.get::<_, i64>(0),
                        ) {
                            Ok(active) if active == 1 => {}
                            _ => { bad = true; break; }
                        }
                    }
                    if bad {
                        return Err(BukioError::new("ACCOUNT_NOT_FOUND", "account validation failed"));
                    }
                    "ok"
                }
                Err(_) => "ok",
            }
        } else {
            "skipped (no database yet)"
        };
        return Ok(json!({
            "action": "create journal entry", "date": date, "description": desc,
            "currency": null,
            "postings": plan, "sum_cents": 0, "sum": "0.00", "state": "draft",
            "post": has_flag(argv, "--post"), "account_validation": validation,
        }));
    }
    let db = open_existing(db_path)?;
    let e = entries::create_entry(&db, entries::CreateEntry {
        date: &date,
        description: &desc,
        postings: specs,
        source: "manual",
        source_ref: arg(argv, "--source-ref").as_deref(),
        actor,
    })?;
    let e = if has_flag(argv, "--post") {
        entries::post_entry(&db, e.id, actor)?
    } else {
        e
    };
    Ok(entries::entry_to_json(&e))
}

fn cmd_entry_post(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    require_actor(actor)?;
    let id: i64 = arg(argv, "--id").and_then(|v| v.parse().ok())
        .ok_or_else(|| BukioError::new("MISSING_ARG", "--id is required"))?;
    let db = open_existing(db_path)?;
    let entry = entries::get_entry(&db, id)
        .ok_or_else(|| BukioError::new("NOT_FOUND", format!("entry {id} does not exist")))?;
    if entry.state != "draft" {
        let code = if entry.state == "reversed" { "ALREADY_REVERSED" } else { "ALREADY_POSTED" };
        return Err(BukioError::new(code, format!("entry {id} is {}", entry.state)));
    }
    if dry_run {
        return Ok(json!({ "action": "post entry", "id": id, "current_state": "draft", "target_state": "posted", "dry_run": true }));
    }
    let posted = entries::post_entry(&db, id, actor)?;
    Ok(entries::entry_to_json(&posted))
}

fn cmd_entry_reverse(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    require_actor(actor)?;
    let id: i64 = arg(argv, "--id").and_then(|v| v.parse().ok())
        .ok_or_else(|| BukioError::new("MISSING_ARG", "--id is required"))?;
    let db = open_existing(db_path)?;
    let entry = entries::get_entry(&db, id)
        .ok_or_else(|| BukioError::new("NOT_FOUND", format!("entry {id} does not exist")))?;
    if entry.state != "posted" {
        let code = if entry.state == "draft" { "NOT_POSTED" } else { "ALREADY_REVERSED" };
        let msg = if entry.state == "draft" {
            format!("entry {id} must be posted before it can be reversed")
        } else {
            format!("entry {id} is already reversed")
        };
        return Err(BukioError::new(code, msg));
    }
    let reversals: i64 = db.query_row(
        "SELECT COUNT(*) FROM journal_entries WHERE reversed_from_id = ?1 AND state = 'posted'",
        [id], |r| r.get(0),
    ).unwrap_or(0);
    if reversals > 0 {
        return Err(BukioError::new("ALREADY_REVERSED", format!("entry {id} is already reversed")));
    }
    if dry_run {
        let reversed: Vec<Value> = entry.postings.iter().map(|p| json!({
            "account_code": p.account_code, "amount_cents": -p.amount_cents, "amount": money::format_amount(-p.amount_cents),
        })).collect();
        return Ok(json!({ "action": "reverse entry (create linked contra-entry)", "id": id, "current_state": "posted", "reversed_postings": reversed, "dry_run": true }));
    }
    let reason = arg(argv, "--reason");
    let reversed = entries::reverse_entry(&db, id, actor, reason.as_deref())?;
    Ok(entries::entry_to_json(&reversed))
}

fn cmd_entry_list(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let limit: i64 = arg(argv, "--limit").and_then(|v| v.parse().ok()).unwrap_or(100);
    let rows = entries::list_entries(
        &db,
        arg(argv, "--state").as_deref().filter(|s| !s.is_empty()),
        arg(argv, "--date-from").as_deref().filter(|s| !s.is_empty()),
        arg(argv, "--date-to").as_deref().filter(|s| !s.is_empty()),
        limit,
    )?;
    Ok(json!({ "entries": rows }))
}

fn cmd_entry_show(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let id: i64 = arg(argv, "--id").and_then(|v| v.parse().ok())
        .ok_or_else(|| BukioError::new("MISSING_ARG", "--id is required"))?;
    let e = entries::get_entry(&db, id)
        .ok_or_else(|| BukioError::new("NOT_FOUND", format!("entry {id} does not exist")))?;
    Ok(entries::entry_to_json(&e))
}

fn cmd_account_add(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    require_actor(actor)?;
    let code = arg(argv, "--code").ok_or_else(|| BukioError::new("MISSING_ARG", "--code is required"))?;
    let name = arg(argv, "--name").ok_or_else(|| BukioError::new("MISSING_ARG", "--name is required"))?;
    let type_ = arg(argv, "--type").ok_or_else(|| BukioError::new("MISSING_ARG", "--type is required"))?;
    let nb = arg(argv, "--normal-balance").ok_or_else(|| BukioError::new("MISSING_ARG", "--normal-balance is required"))?;
    let tax = arg(argv, "--taxonomy-code");
    let new = accounts::NewAccount { code: &code, name: &name, type_: &type_, normal_balance: &nb, taxonomy_code: tax.as_deref() };
    if dry_run {
        accounts::validate_account(&new)?;
        let db = open_existing(db_path)?;
        let exists = accounts::get_account_by_code(&db, &code).is_some();
        return Ok(json!({
            "action": "add account",
            "account": { "code": code, "name": name, "type": type_, "normal_balance": nb, "taxonomy_code": tax },
            "exists": exists, "dry_run": true,
        }));
    }
    let db = open_existing(db_path)?;
    let account = accounts::create_account(&db, &new)?;
    audit::record(&db, audit::RecordArgs {
        actor, action: "account.add", command: Some("account add"),
        args: Some(json!({ "code": code, "name": name, "type": type_, "normal_balance": nb, "taxonomy_code": tax })),
        outcome: "ok", entry_ids: vec![],
    })?;
    Ok(account)
}

fn cmd_account_list(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let rows = accounts::list_accounts(
        &db,
        arg(argv, "--type").as_deref().filter(|s| !s.is_empty()),
        has_flag(argv, "--include-inactive"),
    )?;
    Ok(json!({ "accounts": rows }))
}

fn cmd_account_show(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let code = arg(argv, "--code").ok_or_else(|| BukioError::new("MISSING_ARG", "--code is required"))?;
    accounts::get_account_by_code(&db, &code)
        .ok_or_else(|| BukioError::new("ACCOUNT_NOT_FOUND", format!("account {code} does not exist")))
}

fn cmd_account_deactivate(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    require_actor(actor)?;
    let code = arg(argv, "--code").ok_or_else(|| BukioError::new("MISSING_ARG", "--code is required"))?;
    let db = open_existing(db_path)?;
    let a = accounts::get_account_by_code(&db, &code)
        .ok_or_else(|| BukioError::new("ACCOUNT_NOT_FOUND", format!("account {code} does not exist")))?;
    if dry_run {
        let current = if a["active"].as_bool().unwrap_or(false) { "active" } else { "inactive" };
        return Ok(json!({ "action": "deactivate account", "code": code, "current": current, "dry_run": true }));
    }
    let updated = accounts::deactivate_account(&db, &code)?;
    audit::record(&db, audit::RecordArgs {
        actor, action: "account.deactivate", command: Some("account deactivate"),
        args: Some(json!({ "code": code })), outcome: "ok", entry_ids: vec![],
    })?;
    Ok(updated)
}

fn cmd_account_reactivate(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    require_actor(actor)?;
    let code = arg(argv, "--code").ok_or_else(|| BukioError::new("MISSING_ARG", "--code is required"))?;
    let db = open_existing(db_path)?;
    if dry_run {
        let a = accounts::get_account_by_code(&db, &code)
            .ok_or_else(|| BukioError::new("ACCOUNT_NOT_FOUND", format!("account {code} does not exist")))?;
        if a["active"].as_bool().unwrap_or(false) {
            return Err(BukioError::new("ALREADY_ACTIVE", format!("account {code} is already active")));
        }
        return Ok(json!({ "action": "account.reactivate", "code": code, "from": "inactive", "to": "active", "dry_run": true }));
    }
    let updated = accounts::reactivate_account(&db, &code)?;
    audit::record(&db, audit::RecordArgs {
        actor, action: "account.reactivate", command: Some("account reactivate"),
        args: Some(json!({ "code": code })), outcome: "ok", entry_ids: vec![],
    })?;
    Ok(updated)
}

fn cmd_account_import(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    require_actor(actor)?;
    let file = arg(argv, "--file").ok_or_else(|| BukioError::new("MISSING_ARG", "--file is required"))?;
    let text = std::fs::read_to_string(&file).map_err(|e| BukioError::new("IO_ERROR", format!("cannot read {file}: {e}")))?;
    let db = if dry_run {
        db::open_db(":memory:").map_err(|e| BukioError::new("DB_ERROR", e.to_string()))?
    } else {
        open_existing(db_path)?
    };
    let r = accounts::import_chart_csv(&db, &text)?;
    if !dry_run {
        audit::record(&db, audit::RecordArgs {
            actor, action: "account.import", command: Some("account import"),
            args: Some(json!({ "file": file, "created": r.created, "skipped": r.skipped })),
            outcome: "ok", entry_ids: vec![],
        })?;
    }
    Ok(json!({
        "file": file, "created": r.created, "skipped": r.skipped, "total": r.total,
        "errors": r.errors.iter().map(|(line, e)| json!({ "line": line, "error": e })).collect::<Vec<_>>(),
        "dry_run": dry_run,
    }))
}

fn cmd_cc_add(argv: &[String], db_path: &str, actor: &str) -> Result<Value> {
    require_actor(actor)?;
    let code = arg(argv, "--code").ok_or_else(|| BukioError::new("MISSING_ARG", "--code is required"))?;
    let name = arg(argv, "--name").ok_or_else(|| BukioError::new("MISSING_ARG", "--name is required"))?;
    let db = open_existing(db_path)?;
    let cc = accounts::create_cost_center(&db, &code, &name)?;
    audit::record(&db, audit::RecordArgs {
        actor, action: "cost_center.add", command: Some("cost-center add"),
        args: Some(json!({ "code": code, "name": name })), outcome: "ok", entry_ids: vec![],
    })?;
    Ok(cc)
}

fn cmd_cc_list(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let rows = accounts::list_cost_centers(&db, has_flag(argv, "--include-inactive"))?;
    Ok(json!({ "cost_centers": rows }))
}

fn cmd_cc_toggle(argv: &[String], db_path: &str, actor: &str) -> Result<Value> {
    require_actor(actor)?;
    let reactivate = argv.iter().any(|a| a == "reactivate");
    let code = arg(argv, "--code").ok_or_else(|| BukioError::new("MISSING_ARG", "--code is required"))?;
    let db = open_existing(db_path)?;
    let updated = accounts::set_cost_center_active(&db, &code, reactivate, reactivate)?;
    audit::record(&db, audit::RecordArgs {
        actor,
        action: if reactivate { "cost_center.reactivate" } else { "cost_center.deactivate" },
        command: Some(if reactivate { "cost-center reactivate" } else { "cost-center deactivate" }),
        args: Some(json!({ "code": code })), outcome: "ok", entry_ids: vec![],
    })?;
    Ok(updated)
}

fn cmd_tb(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    reports::trial_balance(&db, arg(argv, "--year").as_deref().filter(|s| !s.is_empty()))
}

fn cmd_balans(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let as_of = arg(argv, "--as-of").unwrap_or_else(|| format!("{}-12-31", dates::today_iso().get(0..4).unwrap_or("2026")));
    reports::balans(&db, &as_of)
}

fn cmd_pnl(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let year = arg(argv, "--year").unwrap_or_else(|| dates::today_iso()[0..4].to_string());
    reports::pnl(&db, &format!("{year}-01-01"), &format!("{year}-12-31"))
}

fn cmd_journal(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let year = arg(argv, "--year").unwrap_or_else(|| dates::today_iso()[0..4].to_string());
    let rows = reports::journal(&db, &format!("{year}-01-01"), &format!("{year}-12-31"), None)?;
    Ok(json!({ "rows": rows }))
}

fn cmd_audit_list(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let limit: i64 = arg(argv, "--limit").and_then(|v| v.parse().ok()).unwrap_or(50);
    let rows = audit::list(
        &db,
        arg(argv, "--since").as_deref().filter(|s| !s.is_empty()),
        arg(argv, "--by").as_deref().filter(|s| !s.is_empty()),
        limit,
    )?;
    Ok(json!({ "entries": rows }))
}

fn cmd_audit_verify(db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    audit::verify_trail(&db)
}

// silence unused warnings for helpers wired in later phases
#[allow(dead_code)]
fn _touch() {
    let _ = canonical::canonical_json(&serde_json::json!({}));
    let _ = sign::is_encrypted("");
    let _ = std::io::stdout().flush();
}

fn cmd_vat_enable(db_path: &str, actor: &str) -> Result<Value> {
    require_actor(actor)?;
    let db = open_existing(db_path)?;
    vat::enable_vat_module(&db, actor)
}

fn cmd_vat_codes(db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let codes = vat::list_vat_codes(&db)?;
    let mapped: Vec<Value> = codes
        .iter()
        .map(|c| {
            json!({
                "code": c["code"], "rate_bp": c["rate_bp"],
                "rate": format!("{:.1}%", c["rate_bp"].as_i64().unwrap_or(0) as f64 / 100.0),
                "type": c["type"], "eu_reverse": c["eu_reverse"], "description": c["description"],
            })
        })
        .collect();
    Ok(json!({ "codes": mapped }))
}

fn cmd_vat_book(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    require_actor(actor)?;
    let date = arg(argv, "--date").unwrap_or_else(|| dates::today_iso());
    let desc = arg(argv, "--desc").ok_or_else(|| BukioError::new("MISSING_ARG", "--desc is required"))?;
    let postings_raw = repeated(argv, "--postings");
    let specs = vat::parse_vat_posting_specs(&postings_raw)?;
    if dry_run {
        dates::validate_date(&date)?;
        return Ok(json!({
            "action": "create vat-aware journal entry", "date": date, "description": desc,
            "postings": specs.iter().map(|s| json!({ "code": s.code, "amount_cents": s.amount_cents, "amount": money::format_amount(s.amount_cents), "vat_code": s.vat_code })).collect::<Vec<_>>(),
            "post": has_flag(argv, "--post"), "dry_run": true,
        }));
    }
    let db = open_existing(db_path)?;
    let entry = vat::book_vat_entry(
        &db, &date, &desc, &specs, "manual", arg(argv, "--source-ref").as_deref(), actor,
        has_flag(argv, "--post"),
    )?;
    // expanded mirrors the JS CLI: every expanded leg incl. auto VAT legs
    let db = open_existing(db_path)?;
    let (all_legs, _) = vat::expand_vat_postings(&db, &specs)?;
    let expanded: Vec<Value> = {
        let mut out: Vec<Value> = Vec::new();
        for (i, sp) in specs.iter().enumerate() {
            out.push(json!({ "code": sp.code, "vat_code": sp.vat_code }));
        }
        for leg in all_legs.iter().skip(specs.len()) {
            out.push(json!({ "code": leg.code }));
        }
        out
    };
    Ok(json!({ "entry": entry, "expanded": expanded }))
}

fn cmd_vat_readout(argv: &[String], db_path: &str, actor: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let period = arg(argv, "--period").ok_or_else(|| BukioError::new("MISSING_ARG", "--period is required"))?;
    if has_flag(argv, "--mark-filed") {
        require_actor(actor)?;
        let result = vat::mark_filed(&db, &period, actor)?;
        let mut out = result.clone();
        out["dryRun"] = json!(false);
        return Ok(out);
    }
    let readout = vat::ob_readout(&db, &period)?;
    let mut fields = serde_json::Map::new();
    if let Some(obj) = readout["fields"].as_object() {
        for (k, v) in obj {
            let cents = v.as_i64().unwrap_or(0);
            fields.insert(k.clone(), json!({ "cents": cents, "amount": money::format_amount(cents) }));
        }
    }
    Ok(json!({
        "period": readout["period"], "from": readout["from"], "to": readout["to"],
        "fields": Value::Object(fields),
        "to_pay_cents": readout["to_pay_cents"], "to_pay": readout["to_pay"],
        "note": readout["note"],
    }))
}

fn cmd_vat_file(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    require_actor(actor)?;
    let db = open_existing(db_path)?;
    vat::vat_file(
        &db,
        arg(argv, "--account").as_deref(),
        arg(argv, "--period").as_deref(),
        arg(argv, "--desc").as_deref(),
        actor,
        dry_run,
    )
}

fn cmd_vat_settle(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    require_actor(actor)?;
    let tx: i64 = arg(argv, "--tx").and_then(|v| v.parse().ok())
        .ok_or_else(|| BukioError::new("MISSING_ARG", "--tx is required (bank transaction id)"))?;
    let db = open_existing(db_path)?;
    // fetch the bank transaction (amount + account) like the JS CLI
    let row: Option<(i64, String)> = db
        .query_row(
            "SELECT t.amount_cents, a.code FROM bank_transactions t
             JOIN bank_accounts a ON a.id = t.account_id WHERE t.id = ?1",
            [tx],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .ok();
    let Some((tx_amount, bank_code)) = row else {
        return Err(BukioError::new("NOT_FOUND", format!("bank transaction {tx} does not exist")));
    };
    let result = vat::vat_settle(
        &db,
        tx_amount,
        arg(argv, "--date").as_deref(),
        &bank_code,
        arg(argv, "--account").as_deref(),
        arg(argv, "--difference-account").as_deref(),
        arg(argv, "--period").as_deref(),
        arg(argv, "--desc").as_deref(),
        actor,
        dry_run,
    )?;
    if !dry_run {
        // link the transaction to the booked entry (like the JS CLI)
        if let Some(entry_id) = result["entry_id"].as_i64() {
            let _ = db.execute(
                "UPDATE bank_transactions SET state = 'matched', matched_entry_id = ?1 WHERE id = ?2",
                rusqlite::params![entry_id, tx],
            );
        }
    }
    Ok(result)
}

fn cmd_fx_set(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    require_actor(actor)?;
    let db = open_existing(db_path)?;
    let currency = arg(argv, "--currency").ok_or_else(|| BukioError::new("MISSING_ARG", "--currency is required"))?;
    let date = arg(argv, "--date").unwrap_or_else(dates::today_iso);
    let rate = arg(argv, "--rate").ok_or_else(|| BukioError::new("MISSING_ARG", "--rate is required"))?;
    let source = arg(argv, "--source").unwrap_or_else(|| "manual".into());
    let r = fx::set_fx_rate(&db, &currency, &date, &rate, &source, actor, dry_run)?;
    if dry_run {
        Ok(json!({
            "rate": r,
            "dryRun": true,
        }))
    } else {
        Ok(json!({ "rate": r }))
    }
}

fn cmd_fx_show(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let currency = arg(argv, "--currency").ok_or_else(|| BukioError::new("MISSING_ARG", "--currency is required"))?;
    let limit: i64 = arg(argv, "--limit").and_then(|v| v.parse().ok()).unwrap_or(50);
    let rates = fx::list_fx_rates(&db, Some(&currency), limit)?;
    Ok(json!({ "currency": currency, "rates": rates }))
}

fn cmd_fx_list(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let limit: i64 = arg(argv, "--limit").and_then(|v| v.parse().ok()).unwrap_or(50);
    let rates = fx::list_fx_rates(&db, None, limit)?;
    Ok(json!({ "rates": rates }))
}
