// bukio-cli — agent-first double-entry bookkeeping for SMEs.
// Copyright (c) 2026 Erik van Kempen.
// SPDX-License-Identifier: Apache-2.0
//
// Native Rust binary entry point. Same argv + JSON protocol as the JS shim.
// Manual argv parsing — no clap dependency.

use bukio::money::{BukioError, Result};
use serde_json::{json, Value};
use std::io::Write;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn ok(data: Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({ "ok": true, "data": data })).unwrap()
    );
}

fn fail_json(err: &BukioError) -> ! {
    let payload = match &err.details {
        Some(d) => json!({
            "ok": false,
            "error": { "code": err.code, "message": err.message, "details": d }
        }),
        None => json!({ "ok": false, "error": { "code": err.code, "message": err.message } }),
    };
    println!("{}", serde_json::to_string_pretty(&payload).unwrap());
    std::process::exit(1);
}

fn arg(argv: &[String], flag: &str) -> Option<String> {
    let eq = format!("{flag}=");
    argv.iter()
        .position(|a| a == flag)
        .and_then(|i| argv.get(i + 1).cloned())
        .or_else(|| {
            argv.iter()
                .find_map(|a| a.strip_prefix(&eq).map(String::from))
        })
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
    BukioError::new(
        "NO_DATABASE",
        format!("no database at {db_path} — run 'bukio init' first"),
    )
}

fn ensure_db_exists(db_path: &str) -> Result<std::path::PathBuf> {
    let p = std::path::PathBuf::from(db_path);
    if !p.exists() {
        return Err(no_database(db_path));
    }
    Ok(p)
}

fn open_existing(db_path: &str) -> Result<rusqlite::Connection> {
    ensure_db_exists(db_path)?;
    bukio::db::open_db(db_path).map_err(|e| BukioError::new("DB_ERROR", e.to_string()))
}

fn require_actor(actor: &str) -> Result<()> {
    match bukio::actor::actor_error(if actor.is_empty() { None } else { Some(actor) }) {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

fn not_ported(cmd: &str) -> BukioError {
    BukioError::new("NOT_YET_PORTED", format!("{cmd} is not ported to Rust yet"))
}

fn missing_arg(flag: &str) -> BukioError {
    BukioError::new("MISSING_ARG", format!("{flag} is required"))
}

fn parse_i64(argv: &[String], flag: &str) -> Option<i64> {
    arg(argv, flag).and_then(|v| v.parse().ok())
}

/// Parse --life-months: missing → 60, garbage → INVALID_LIFE.
fn parse_life_months(argv: &[String]) -> Result<i64> {
    match arg(argv, "--life-months") {
        None => Ok(60),
        Some(v) => v.parse::<i64>().map_err(|_| {
            BukioError::new(
                "INVALID_LIFE",
                format!("invalid --life-months '{v}' — must be a positive integer"),
            )
        }),
    }
}

/// Parse --limit with validation: "abc" → INVALID_LIMIT, "0" → 0, missing → default.
fn parse_limit(argv: &[String], default: i64) -> Result<i64> {
    match arg(argv, "--limit") {
        None => Ok(default),
        Some(v) => v.parse::<i64>().map_err(|_| {
            BukioError::new(
                "INVALID_LIMIT",
                format!("invalid --limit '{v}' — must be a non-negative integer"),
            )
        }),
    }
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();

    // --help: the captured JS help for the deepest known command path, so
    // `bukio bank --help` and `bukio payments payables add --help` work like
    // commander's output instead of UNKNOWN_COMMAND
    if has_flag(&argv, "--help") || has_flag(&argv, "-h") {
        print!("{}", bukio::help::resolve(&argv));
        std::process::exit(0);
    }
    // `bukio help <path>` — the JS exposes this as a command
    if argv.first().map(|a| a.as_str()) == Some("help") {
        print!("{}", bukio::help::resolve(&argv[1..]));
        std::process::exit(0);
    }
    if has_flag(&argv, "--version") || has_flag(&argv, "-V") {
        println!("0.17.0");
        std::process::exit(0);
    }

    // Global flags
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
    let _locale = arg(&argv, "--locale")
        .or_else(|| std::env::var("BUKIO_LOCALE").ok())
        .unwrap_or_else(|| "en".into());
    let _sign_key = arg(&argv, "--sign-key");
    let _server = arg(&argv, "--server").or_else(|| std::env::var("BUKIO_SERVER").ok());

    let result = dispatch(&argv, &db_path, &actor, dry_run);
    match result {
        Ok(data) => {
            // `server token` prints the raw token for operator capture (JS parity)
            if !json_mode
                && argv.first().map(|s| s.as_str()) == Some("server")
                && argv.get(1).map(|s| s.as_str()) == Some("token")
            {
                println!("{}", data["token"].as_str().unwrap_or(""));
                return;
            }
            if json_mode {
                ok(data);
            } else if let Some(p) = data.get("path").and_then(|v| v.as_str()) {
                // file-writing commands print human text like the JS CLI
                println!("wrote {p}");
                if let Some(year) = data.get("year").and_then(|v| v.as_str()) {
                    let name = data["company"]["name"].as_str().unwrap_or("");
                    let reg = data["company"]["registration_id"].as_str().unwrap_or("-");
                    let rek = data.get("rekeningen").and_then(|v| v.as_i64()).unwrap_or(0);
                    let mut_count = data.get("mutaties").and_then(|v| v.as_i64()).unwrap_or(0);
                    println!("  {name} (KVK {reg}) — fiscal year {year}");
                    println!("  {rek} accounts, {mut_count} mutations");
                }
            } else {
                println!("{}", serde_json::to_string_pretty(&data).unwrap());
            }
        }
        Err(e) => {
            if json_mode {
                fail_json(&e);
            } else {
                println!("error [{}]: {}", e.code, e.message);
                std::process::exit(1);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

fn try_match_cmd(
    pos: &[&str],
    argv: &[String],
    db_path: &str,
    actor: &str,
    dry_run: bool,
) -> Option<Result<Value>> {
    let r = match pos {
        // ── init ──────────────────────────────────────────────────────
        ["init"] => cmd_init(argv, db_path, actor, dry_run),

        // ── entry ─────────────────────────────────────────────────────
        ["entry", "add"] => cmd_entry_add(argv, db_path, actor, dry_run),
        ["entry", "post"] => cmd_entry_post(argv, db_path, actor, dry_run),
        ["entry", "reverse"] => cmd_entry_reverse(argv, db_path, actor, dry_run),
        ["entry", "list"] => cmd_entry_list(argv, db_path),
        ["entry", "show"] => cmd_entry_show(argv, db_path),

        // ── account ───────────────────────────────────────────────────
        ["account", "add"] => cmd_account_add(argv, db_path, actor, dry_run),
        ["account", "list"] => cmd_account_list(argv, db_path),
        ["account", "show"] => cmd_account_show(argv, db_path),
        ["account", "deactivate"] => cmd_account_deactivate(argv, db_path, actor, dry_run),
        ["account", "reactivate"] => cmd_account_reactivate(argv, db_path, actor, dry_run),
        ["account", "import"] => cmd_account_import(argv, db_path, actor, dry_run),

        // ── cost-center ───────────────────────────────────────────────
        ["cost-center", "add"] => cmd_cc_add(argv, db_path, actor),
        ["cost-center", "list"] => cmd_cc_list(argv, db_path),
        ["cost-center", "show"] => cmd_cc_show(argv, db_path),
        ["cost-center", "deactivate"] | ["cost-center", "reactivate"] => {
            cmd_cc_toggle(argv, db_path, actor)
        }

        // ── report ────────────────────────────────────────────────────
        ["report", "trial-balance"] => cmd_tb(argv, db_path),
        ["report", "balance-sheet"] | ["report", "balans"] => cmd_balans(argv, db_path),
        ["report", "pnl"] => cmd_pnl(argv, db_path),
        ["report", "journal"] => cmd_journal(argv, db_path),
        ["report", "aging"] => cmd_report_aging(argv, db_path),
        ["report", "sales"] => cmd_report_sales(argv, db_path),
        ["report", "cost-center"] => cmd_report_cost_center(argv, db_path),

        // ── audit ─────────────────────────────────────────────────────
        ["audit"] | ["audit", "list"] => cmd_audit_list(argv, db_path),
        ["audit", "verify"] => cmd_audit_verify(argv, db_path),

        // ── backup ────────────────────────────────────────────────────
        ["backup"] => cmd_backup(argv, db_path, actor, dry_run),
        ["restore"] => cmd_backup_restore(argv, db_path, actor, dry_run),
        ["backup", "restore"] => cmd_backup_restore(argv, db_path, actor, dry_run),

        // ── bank ──────────────────────────────────────────────────────
        ["bank", "add"] => cmd_bank_add(argv, db_path, actor, dry_run),
        ["bank", "list"] => cmd_bank_list(db_path),
        ["bank", "import"] => cmd_bank_import(argv, db_path, actor, dry_run),
        ["bank", "transactions"] => cmd_bank_transactions(argv, db_path),
        ["bank", "match", "auto"] => cmd_bank_match_auto(argv, db_path, actor, dry_run),
        ["bank", "match", "suggest"] => cmd_bank_match_suggest(db_path),
        ["bank", "match", "link"] => cmd_bank_match_link(argv, db_path, actor, dry_run),
        ["bank", "match", "post"] => cmd_bank_match_post(argv, db_path, actor, dry_run),
        ["bank", "ignore"] => cmd_bank_ignore(argv, db_path, actor, dry_run),
        ["bank", "unignore"] => cmd_bank_unignore(argv, db_path, actor, dry_run),

        // ── vat ───────────────────────────────────────────────────────
        ["vat", "enable"] => cmd_vat_enable(db_path, actor),
        ["vat", "codes"] => cmd_vat_codes(db_path),
        ["vat", "book"] => cmd_vat_book(argv, db_path, actor, dry_run),
        ["vat", "readout"] => cmd_vat_readout(argv, db_path, actor),
        ["vat", "file"] => cmd_vat_file(argv, db_path, actor, dry_run),
        ["vat", "settle"] => cmd_vat_settle(argv, db_path, actor, dry_run),

        // ── recurring ─────────────────────────────────────────────────
        ["recurring", "add"] => cmd_recurring_add(argv, db_path, actor, dry_run),
        ["recurring", "list"] => cmd_recurring_list(argv, db_path),
        ["recurring", "show"] => cmd_recurring_show(argv, db_path),
        ["recurring", "pause"] => cmd_recurring_pause(argv, db_path, actor, dry_run),
        ["recurring", "resume"] => cmd_recurring_resume(argv, db_path, actor, dry_run),
        ["recurring", "preview"] => cmd_recurring_preview(argv, db_path),
        ["recurring", "run"] => cmd_recurring_run(argv, db_path, actor, dry_run),

        // ── depreciation ──────────────────────────────────────────────
        ["depreciation", "add"] => cmd_depreciation_add(argv, db_path, actor, dry_run),

        // ── contact ───────────────────────────────────────────────────
        ["contact", "add"] => cmd_contact_add(argv, db_path, actor, dry_run),
        ["contact", "update"] => cmd_contact_update(argv, db_path, actor, dry_run),
        ["contact", "list"] => cmd_contact_list(db_path),
        ["contact", "statement"] => cmd_contact_statement(argv, db_path),

        // ── invoice ───────────────────────────────────────────────────
        ["invoice", "create"] => cmd_invoice_create(argv, db_path, actor, dry_run),
        ["invoice", "finalize"] => cmd_invoice_finalize(argv, db_path, actor, dry_run),
        ["invoice", "list"] => cmd_invoice_list(argv, db_path),
        ["invoice", "show"] => cmd_invoice_show(argv, db_path),
        ["invoice", "pdf"] => cmd_invoice_pdf(argv, db_path),
        ["invoice", "ubl"] => cmd_invoice_ubl(argv, db_path),
        ["invoice", "credit"] => cmd_invoice_credit(argv, db_path, actor, dry_run),
        ["invoice", "peppol-send"] => cmd_invoice_peppol_send(argv, db_path, actor, dry_run),
        ["invoice", "pay"] => cmd_invoice_pay(argv, db_path, actor, dry_run),
        ["invoice", "email"] => cmd_invoice_email(argv, db_path, actor, dry_run),
        ["invoice", "reminders"] => cmd_invoice_reminders(argv, db_path, actor, dry_run),

        // ── year-end ──────────────────────────────────────────────────
        ["year-end", "status"] => cmd_year_end_status(argv, db_path),
        ["year-end", "close"] => cmd_year_end_close(argv, db_path, actor, dry_run),

        // ── financial-statements ──────────────────────────────────────
        ["financial-statements", "report"] => cmd_financial_statements_report(argv, db_path),

        // ── icp ───────────────────────────────────────────────────────
        ["icp", "readout"] => cmd_icp_readout(argv, db_path),

        // ── fx ────────────────────────────────────────────────────────
        ["fx", "set"] => cmd_fx_set(argv, db_path, actor, dry_run),
        ["fx", "show"] => cmd_fx_show(argv, db_path),
        ["fx", "list"] => cmd_fx_list(argv, db_path),
        ["fx", "fetch"] => cmd_fx_fetch(argv),

        // ── mcp ───────────────────────────────────────────────────────
        ["mcp"] => cmd_mcp(db_path, actor),

        // ── compliance ────────────────────────────────────────────────
        ["compliance", "status"] => cmd_compliance_status(argv, db_path),
        ["compliance", "mark"] => cmd_compliance_mark(argv, db_path, actor, dry_run),

        // ── import ────────────────────────────────────────────────────
        ["import", "opening-balances"] => {
            cmd_import_opening_balances(argv, db_path, actor, dry_run)
        }
        ["import", "journal"] => cmd_import_journal(argv, db_path, actor, dry_run),
        ["import", "contacts"] => cmd_import_contacts(argv, db_path, actor, dry_run),
        ["import", "xaf"] => cmd_import_xaf(argv, db_path, actor, dry_run),
        ["import", "invoice"] => cmd_import_invoice(argv, db_path, actor, dry_run),

        // ── export ────────────────────────────────────────────────────
        ["export", "xaf"] => cmd_export_xaf(argv, db_path, actor, dry_run),

        // ── month-end ─────────────────────────────────────────────────
        ["month-end"] => cmd_month_end(argv, db_path),

        // ── company ───────────────────────────────────────────────────
        ["company", "show"] => cmd_company_show(db_path),
        ["company", "update"] => cmd_company_update(argv, db_path, actor, dry_run),
        ["company", "logo"] => cmd_company_logo(argv, db_path),

        // ── assets ────────────────────────────────────────────────────
        ["assets", "scheme", "add"] => cmd_asset_scheme_add(argv, db_path, actor, dry_run),
        ["assets", "scheme", "list"] => cmd_asset_scheme_list(db_path),
        ["assets", "add"] => cmd_asset_add(argv, db_path, actor, dry_run),
        ["assets", "list"] => cmd_asset_list(argv, db_path),
        ["assets", "show"] => cmd_asset_show(argv, db_path),
        ["assets", "run"] => cmd_asset_run(argv, db_path, actor, dry_run),
        ["assets", "register"] => cmd_asset_register(argv, db_path, actor),
        ["assets", "dispose"] => cmd_asset_dispose(argv, db_path, actor, dry_run),
        ["assets", "pause"] => cmd_asset_pause(argv, db_path, actor, dry_run),
        ["assets", "resume"] => cmd_asset_resume(argv, db_path, actor, dry_run),

        // ── payments ──────────────────────────────────────────────────
        ["payments", "payables", "add"] => cmd_payable_add(argv, db_path, actor, dry_run),
        ["payments", "payables", "list"] => cmd_payable_list(argv, db_path),
        ["payments", "payables", "pay"] => cmd_payable_pay(argv, db_path, actor, dry_run),
        ["payments", "mandate", "add"] => cmd_mandate_add(argv, db_path, actor, dry_run),
        ["payments", "mandate", "list"] => cmd_mandate_list(argv, db_path),
        ["payments", "mandate", "remove"] => cmd_mandate_remove(argv, db_path, actor, dry_run),
        ["payments", "batch", "create"] => cmd_batch_create(argv, db_path, actor, dry_run),
        ["payments", "batch", "list"] => cmd_batch_list(argv, db_path),
        ["payments", "batch", "show"] => cmd_batch_show(argv, db_path),
        ["payments", "batch", "delete"] => cmd_batch_delete(argv, db_path, actor, dry_run),
        ["payments", "batch", "export"] => cmd_batch_export(argv, db_path, actor, dry_run),

        // ── item ──────────────────────────────────────────────────────
        ["item", "add"] => cmd_item_add(argv, db_path, actor, dry_run),
        ["item", "list"] => cmd_item_list(argv, db_path),
        ["item", "show"] => cmd_item_show(argv, db_path),
        ["item", "update"] => cmd_item_update(argv, db_path, actor, dry_run),

        // ── attach ────────────────────────────────────────────────────
        ["attach", "add"] => cmd_attach_add(argv, db_path, actor, dry_run),
        ["attach", "list"] => cmd_attach_list(argv, db_path),
        ["attach", "show"] => cmd_attach_show(argv, db_path),
        ["attach", "remove"] => cmd_attach_remove(argv, db_path, actor, dry_run),

        // ── update ────────────────────────────────────────────────────
        ["update"] => cmd_update(argv, db_path, actor),

        // ── actor ─────────────────────────────────────────────────────
        ["actor"] | ["actor", "--help"] | ["actor", "-h"] => {
            println!("actor: keygen, register, list, revoke, enforce, unlock, lock, verify, authz, roles, grant, revoke-role, can");
            std::process::exit(0);
        }
        ["actor", "keygen"] => cmd_actor_keygen(argv, actor, dry_run),
        ["actor", "register"] => cmd_actor_register(argv, db_path, actor, dry_run),
        ["actor", "list"] => cmd_actor_list(db_path),
        ["actor", "revoke"] => cmd_actor_revoke(argv, db_path, actor, dry_run),
        ["actor", "enforce"] => cmd_actor_enforce(argv, db_path, actor, dry_run),
        ["actor", "unlock"] => cmd_actor_unlock(argv, actor),
        ["actor", "lock"] => cmd_actor_lock(argv, actor),
        ["actor", "authz"] => cmd_actor_authz(argv, db_path, actor, dry_run),
        ["actor", "roles"] => cmd_actor_roles(argv, db_path, actor),
        ["actor", "grant"] => cmd_actor_grant(argv, db_path, actor),
        ["actor", "roles", "grant"] => cmd_actor_grant(argv, db_path, actor),
        ["actor", "revoke-role"] => cmd_actor_revoke_role(argv, db_path, actor),
        ["actor", "roles", "revoke"] => cmd_actor_revoke_role(argv, db_path, actor),
        ["actor", "can"] => cmd_actor_can(argv, db_path, actor),
        ["actor", "who-can"] => cmd_actor_who_can(argv, db_path, actor),
        ["actor", "verify"] => cmd_actor_verify_key(db_path, actor),
        ["actor", sub] => {
            let valid = "keygen, register, list, revoke, enforce, unlock, lock, verify, authz, roles, grant, revoke-role, can, who-can";
            Err(BukioError::new(
                "UNKNOWN_SUBCOMMAND",
                format!("unknown actor subcommand '{sub}' — valid: {valid}"),
            ))
        }

        // ── server ────────────────────────────────────────────────────
        ["server", "start"] => cmd_server_start(argv, db_path),
        ["server", "token"] => cmd_server_token(argv, actor),

        _ => return None,
    };
    Some(r)
}

fn dispatch(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    // Extract positional args (skip flags and their values)
    let mut positional: Vec<&str> = Vec::new();
    let mut i = 0;
    while i < argv.len() {
        let a = &argv[i];
        if a.starts_with('-') && a != "-" {
            let takes_value = !matches!(
                a.as_str(),
                "--json"
                    | "--dry-run"
                    | "--post"
                    | "--kor"
                    | "--include-inactive"
                    | "--force"
                    | "--encrypt"
                    | "--yes"
                    | "--trust-remote"
                    | "--create-missing"
                    | "--reverse-previous"
                    | "--no-pdf"
                    | "--draft-emails"
                    | "--all"
                    | "--from-invoices"
                    | "--on"
                    | "--off"
                    | "--mark-filed"
                    | "--peppol"
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

    // Remote client mode: --server <url> signs the command and POSTs it
    if let Some(server) = arg(argv, "--server").or_else(|| std::env::var("BUKIO_SERVER").ok()) {
        if !server.is_empty() {
            remote_client_mode(&server, &positional, argv, actor);
            return Ok(json!({"ok": true})); // unreachable — remote fn exits
        }
    }

    // Signing gate: verify signature before any command (unless exempt)
    let sign_key = std::env::var("BUKIO_SIGNING_KEY")
        .ok()
        .or_else(|| arg(argv, "--sign-key"));
    let cmd_str = positional.join(" ");
    let sign_result =
        bukio::sign_gate::sign_command(actor, &cmd_str, argv, db_path, sign_key.as_deref(), None)?;
    // Bridge: store sign result for audit rows
    if let Some(ref sr) = sign_result {
        bukio::audit::set_pending_signature(Some(bukio::audit::PendingSignature {
            digest_hash: Some(sr.digest_hash.clone()),
            sig_keyid: Some(sr.sig_keyid.clone()),
            sig_nonce: Some(sr.sig_nonce.clone()),
            sig_ts: Some(sr.sig_ts.clone()),
            sig: Some(sr.sig.clone()),
            sig_status: sr.sig_status.clone(),
            signed_args: Some(sr.signed_args.clone()),
            signed_command: Some(sr.signed_command.clone()),
        }));
    } else {
        bukio::audit::set_pending_signature(None);
    }

    // Try full match first, then try dropping trailing positional tokens
    // (extra tokens from multi-word flag values like --postal-code '1000 AA')
    if let Some(result) = try_match_cmd(&positional, argv.clone(), db_path, actor, dry_run) {
        return result;
    }
    for trim in 1..positional.len() {
        let shorter = &positional[..positional.len() - trim];
        if let Some(result) = try_match_cmd(shorter, argv.clone(), db_path, actor, dry_run) {
            return result;
        }
    }
    // A group typed on its own (`bukio bank`) is not an error in commander: it
    // prints the group's help and exits 1. Do the same from the captured text.
    let path = positional.join(" ");
    if bukio::help::has_children(&path) {
        eprint!("{}", bukio::help::text_for(&path).unwrap_or(""));
        std::process::exit(1);
    }
    Err(BukioError::new(
        "UNKNOWN_COMMAND",
        format!("unknown command: {path}"),
    ))
}

// ═══════════════════════════════════════════════════════════════════════════
// Command implementations
// ═══════════════════════════════════════════════════════════════════════════

// ── init ───────────────────────────────────────────────────────────────────

fn cmd_init(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    require_actor(actor)?;
    let name = arg(argv, "--name").ok_or_else(|| missing_arg("--name"))?;
    let country = arg(argv, "--country").unwrap_or_else(|| "NL".into());
    let profile = bukio::accounts::get_profile(&country)?;
    let legal_form = arg(argv, "--legal-form").unwrap_or_else(|| "eenmanszaak".into());
    let legal_forms: Vec<&str> = profile["meta"]["legalForms"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    if !legal_forms.contains(&legal_form.as_str()) {
        return Err(BukioError::new(
            "INVALID_LEGAL_FORM",
            format!(
                "legal form '{}' must be one of {}",
                legal_form,
                legal_forms.join(", ")
            ),
        ));
    }
    let vat = arg(argv, "--vat").unwrap_or_else(|| "off".into());
    if vat != "on" && vat != "off" {
        return Err(BukioError::new(
            "INVALID_VAT_CHOICE",
            format!("--vat must be 'on' or 'off', got '{vat}'"),
        ));
    }
    let kor = has_flag(argv, "--kor");
    if kor && profile["tax"]["smallBusinessScheme"].as_str() != Some("kor") {
        return Err(BukioError::new(
            "INVALID_VAT_CHOICE",
            format!("--kor is not available for country {country}"),
        ));
    }
    let iban = arg(argv, "--iban");
    if let Some(i) = &iban {
        if !bukio::dates::is_valid_iban(i) {
            return Err(BukioError::new(
                "INVALID_IBAN",
                format!("'{i}' is not a valid IBAN"),
            ));
        }
    }
    // Validate --fiscal-year-end (MM-DD format)
    if let Some(fye) = arg(argv, "--fiscal-year-end") {
        let parts: Option<(&str, &str)> = fye.split_once('-');
        let valid = match parts {
            Some((m, d)) => {
                let month: u32 = m.parse().unwrap_or(0);
                let day: u32 = d.parse().unwrap_or(0);
                (1..=12).contains(&month) && {
                    let max_day = match month {
                        2 => 29,
                        4 | 6 | 9 | 11 => 30,
                        _ => 31,
                    };
                    (1..=max_day).contains(&day)
                }
            }
            None => false,
        };
        if !valid {
            return Err(BukioError::new(
                "INVALID_FISCAL_YEAR_END",
                format!("'{fye}' is not a valid fiscal year end — use MM-DD format"),
            ));
        }
    }
    let chart_len = profile["reporting"]["defaultChart"]
        .as_array()
        .map(|a| a.len())
        .unwrap_or(0);
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
            "dryRun": true,
        }));
    }
    if std::path::Path::new(db_path).exists() {
        let existing =
            bukio::db::open_db(db_path).map_err(|e| BukioError::new("DB_ERROR", e.to_string()))?;
        let has: i64 = existing
            .query_row("SELECT count(*) FROM company", [], |r| r.get(0))
            .unwrap_or(0);
        if has > 0 {
            return Err(BukioError::new(
                "ALREADY_INITIALISED",
                format!("database {db_path} already has a company"),
            ));
        }
    }
    if let Some(parent) = std::path::Path::new(db_path).parent() {
        std::fs::create_dir_all(parent).map_err(|e| BukioError::new("IO_ERROR", e.to_string()))?;
    }
    let db = bukio::db::open_db(db_path).map_err(|e| BukioError::new("DB_ERROR", e.to_string()))?;
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
    let created = bukio::accounts::seed_default_chart(&db)?;
    let mut vat_created = 0;
    if company["vat_module"] == 1 {
        let vat_result = bukio::vat::enable_vat_module(&db, actor)?;
        vat_created = vat_result["accounts"]
            .as_array()
            .map(|a| a.len())
            .unwrap_or(0);
    }
    bukio::audit::record(
        &db,
        bukio::audit::RecordArgs {
            actor,
            action: "company.init",
            command: Some("init"),
            args: Some(company.clone()),
            outcome: "ok",
            entry_ids: vec![],
        },
    )?;
    let total = bukio::accounts::list_accounts(&db, None, true)?.len();
    Ok(
        json!({ "company": company, "db": db_path, "chart": { "accounts": total, "created": created + vat_created }, "dryRun": false }),
    )
}

// ── entry ──────────────────────────────────────────────────────────────────

fn cmd_entry_add(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    require_actor(actor)?;
    let date = arg(argv, "--date").unwrap_or_else(bukio::dates::today_iso);
    let desc = arg(argv, "--desc").ok_or_else(|| missing_arg("--desc"))?;
    let postings_raw = repeated(argv, "--postings");
    let specs = bukio::entries::parse_posting_specs(&postings_raw)?;
    // --currency: convert to EUR up front so BOTH the plan and the booking
    // show the amounts that will be booked (the JS applyFx runs before the
    // dry-run branch too)
    let currency = arg(argv, "--currency");
    let specs = if currency.is_some() {
        let fx_db = open_existing(db_path)?;
        bukio::fx::resolve_fx(
            &fx_db,
            specs,
            currency.as_deref(),
            arg(argv, "--rate").as_deref(),
            &date,
            actor,
            dry_run,
        )?
    } else {
        specs
    };
    if dry_run {
        bukio::dates::validate_date(&date)?;
        if desc.trim().is_empty() {
            return Err(BukioError::new(
                "INVALID_DESCRIPTION",
                "description is required",
            ));
        }
        if specs.len() < 2 {
            return Err(BukioError::new(
                "TOO_FEW_POSTINGS",
                "an entry needs at least 2 postings",
            ));
        }
        let sum: i64 = specs.iter().map(|s| s.amount_cents).sum();
        if sum != 0 {
            return Err(BukioError::new(
                "UNBALANCED",
                format!("postings do not sum to zero (sum = {sum} cents)"),
            ));
        }
        let plan: Vec<Value> = specs
            .iter()
            .map(|s| json!({ "code": s.code, "amount_cents": s.amount_cents, "amount": bukio::money::format_amount(s.amount_cents), "cost_center": s.cost_center_code }))
            .collect();
        let validation = if std::path::Path::new(db_path).exists() {
            match open_existing(db_path) {
                Ok(db) => {
                    let mut bad = false;
                    for s in &specs {
                        match db.query_row(
                            "SELECT active FROM accounts WHERE code = ?1",
                            [&s.code],
                            |r| r.get::<_, i64>(0),
                        ) {
                            Ok(active) if active == 1 => {}
                            _ => {
                                bad = true;
                                break;
                            }
                        }
                    }
                    if bad {
                        return Err(BukioError::new(
                            "ACCOUNT_NOT_FOUND",
                            "account validation failed",
                        ));
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
            "currency": currency.clone(),
            "postings": plan, "sum_cents": sum, "sum": bukio::money::format_amount(sum), "state": "draft",
            "post": has_flag(argv, "--post"), "account_validation": validation, "dryRun": true,
        }));
    }
    let db = open_existing(db_path)?;
    let e = bukio::entries::create_entry(
        &db,
        bukio::entries::CreateEntry {
            date: &date,
            description: &desc,
            postings: specs,
            source: "manual",
            source_ref: arg(argv, "--source-ref").as_deref(),
            actor,
        },
    )?;
    let e = if has_flag(argv, "--post") {
        bukio::entries::post_entry(&db, e.id, actor)?
    } else {
        e
    };
    Ok(bukio::entries::entry_to_json(&e))
}

fn cmd_entry_post(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    require_actor(actor)?;
    let id: i64 = parse_i64(argv, "--id").ok_or_else(|| missing_arg("--id"))?;
    let db = open_existing(db_path)?;
    let entry = bukio::entries::get_entry(&db, id)
        .ok_or_else(|| BukioError::new("NOT_FOUND", format!("entry {id} does not exist")))?;
    if entry.state != "draft" {
        let code = if entry.state == "reversed" {
            "ALREADY_REVERSED"
        } else {
            "ALREADY_POSTED"
        };
        return Err(BukioError::new(
            code,
            format!("entry {id} is {}", entry.state),
        ));
    }
    if dry_run {
        return Ok(
            json!({ "action": "post entry", "id": id, "current_state": "draft", "target_state": "posted", "dryRun": true }),
        );
    }
    let posted = bukio::entries::post_entry(&db, id, actor)?;
    Ok(bukio::entries::entry_to_json(&posted))
}

fn cmd_entry_reverse(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    require_actor(actor)?;
    let id: i64 = parse_i64(argv, "--id").ok_or_else(|| missing_arg("--id"))?;
    let db = open_existing(db_path)?;
    let entry = bukio::entries::get_entry(&db, id)
        .ok_or_else(|| BukioError::new("NOT_FOUND", format!("entry {id} does not exist")))?;
    if entry.state != "posted" {
        let code = if entry.state == "draft" {
            "NOT_POSTED"
        } else {
            "ALREADY_REVERSED"
        };
        let msg = if entry.state == "draft" {
            format!("entry {id} must be posted before it can be reversed")
        } else {
            format!("entry {id} is already reversed")
        };
        return Err(BukioError::new(code, msg));
    }
    let reversals: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM journal_entries WHERE reversed_from_id = ?1 AND state = 'posted'",
            [id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if reversals > 0 {
        return Err(BukioError::new(
            "ALREADY_REVERSED",
            format!("entry {id} is already reversed"),
        ));
    }
    if dry_run {
        let reversed: Vec<Value> = entry.postings.iter().map(|p| json!({
            "account_code": p.account_code, "amount_cents": -p.amount_cents, "amount": bukio::money::format_amount(-p.amount_cents),
        })).collect();
        return Ok(
            json!({ "action": "reverse entry (create linked contra-entry)", "id": id, "current_state": "posted", "reversed_postings": reversed, "dryRun": true }),
        );
    }
    let reason = arg(argv, "--reason");
    let reversed = bukio::entries::reverse_entry(&db, id, actor, reason.as_deref())?;
    Ok(bukio::entries::entry_to_json(&reversed))
}

fn cmd_entry_list(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let limit: i64 = parse_limit(argv, 100)?;
    let rows = bukio::entries::list_entries(
        &db,
        arg(argv, "--state").as_deref().filter(|s| !s.is_empty()),
        arg(argv, "--date-from")
            .as_deref()
            .filter(|s| !s.is_empty()),
        arg(argv, "--date-to").as_deref().filter(|s| !s.is_empty()),
        limit,
    )?;
    Ok(json!({ "entries": rows }))
}

fn cmd_entry_show(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let id: i64 = parse_i64(argv, "--id").ok_or_else(|| missing_arg("--id"))?;
    let e = bukio::entries::get_entry(&db, id)
        .ok_or_else(|| BukioError::new("NOT_FOUND", format!("entry {id} does not exist")))?;
    Ok(bukio::entries::entry_to_json(&e))
}

// ── account ────────────────────────────────────────────────────────────────

fn cmd_account_add(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    require_actor(actor)?;
    let code = arg(argv, "--code").ok_or_else(|| missing_arg("--code"))?;
    let name = arg(argv, "--name").ok_or_else(|| missing_arg("--name"))?;
    let type_ = arg(argv, "--type").ok_or_else(|| missing_arg("--type"))?;
    let nb = arg(argv, "--normal-balance").ok_or_else(|| missing_arg("--normal-balance"))?;
    let tax = arg(argv, "--taxonomy-code");
    let new = bukio::accounts::NewAccount {
        code: &code,
        name: &name,
        type_: &type_,
        normal_balance: &nb,
        taxonomy_code: tax.as_deref(),
    };
    if dry_run {
        bukio::accounts::validate_account(&new)?;
        let db = open_existing(db_path)?;
        let exists = bukio::accounts::get_account_by_code(&db, &code).is_some();
        return Ok(json!({
            "action": "add account",
            "account": { "code": code, "name": name, "type": type_, "normal_balance": nb, "taxonomy_code": tax },
            "exists": exists, "dryRun": true,
        }));
    }
    let db = open_existing(db_path)?;
    let account = bukio::accounts::create_account(&db, &new)?;
    bukio::audit::record(
        &db,
        bukio::audit::RecordArgs {
            actor,
            action: "account.add",
            command: Some("account add"),
            args: Some(
                json!({ "code": code, "name": name, "type": type_, "normal_balance": nb, "taxonomy_code": tax }),
            ),
            outcome: "ok",
            entry_ids: vec![],
        },
    )?;
    Ok(account)
}

fn cmd_account_list(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let rows = bukio::accounts::list_accounts(
        &db,
        arg(argv, "--type").as_deref().filter(|s| !s.is_empty()),
        has_flag(argv, "--include-inactive"),
    )?;
    Ok(json!({ "accounts": rows }))
}

fn cmd_account_show(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let code = arg(argv, "--code").ok_or_else(|| missing_arg("--code"))?;
    bukio::accounts::get_account_by_code(&db, &code).ok_or_else(|| {
        BukioError::new(
            "ACCOUNT_NOT_FOUND",
            format!("account {code} does not exist"),
        )
    })
}

fn cmd_account_deactivate(
    argv: &[String],
    db_path: &str,
    actor: &str,
    dry_run: bool,
) -> Result<Value> {
    require_actor(actor)?;
    let code = arg(argv, "--code").ok_or_else(|| missing_arg("--code"))?;
    let db = open_existing(db_path)?;
    let a = bukio::accounts::get_account_by_code(&db, &code).ok_or_else(|| {
        BukioError::new(
            "ACCOUNT_NOT_FOUND",
            format!("account {code} does not exist"),
        )
    })?;
    if dry_run {
        let current = if a["active"].as_bool().unwrap_or(false) {
            "active"
        } else {
            "inactive"
        };
        return Ok(
            json!({ "action": "deactivate account", "code": code, "current": current, "dryRun": true }),
        );
    }
    let updated = bukio::accounts::deactivate_account(&db, &code)?;
    bukio::audit::record(
        &db,
        bukio::audit::RecordArgs {
            actor,
            action: "account.deactivate",
            command: Some("account deactivate"),
            args: Some(json!({ "code": code })),
            outcome: "ok",
            entry_ids: vec![],
        },
    )?;
    Ok(updated)
}

fn cmd_account_reactivate(
    argv: &[String],
    db_path: &str,
    actor: &str,
    dry_run: bool,
) -> Result<Value> {
    require_actor(actor)?;
    let code = arg(argv, "--code").ok_or_else(|| missing_arg("--code"))?;
    let db = open_existing(db_path)?;
    if dry_run {
        let a = bukio::accounts::get_account_by_code(&db, &code).ok_or_else(|| {
            BukioError::new(
                "ACCOUNT_NOT_FOUND",
                format!("account {code} does not exist"),
            )
        })?;
        if a["active"].as_bool().unwrap_or(false) {
            return Err(BukioError::new(
                "ALREADY_ACTIVE",
                format!("account {code} is already active"),
            ));
        }
        return Ok(
            json!({ "account": { "action": "account.reactivate", "code": code, "from": "inactive", "to": "active", "dryRun": true } }),
        );
    }
    bukio::accounts::reactivate_account(&db, &code)?;
    bukio::audit::record(
        &db,
        bukio::audit::RecordArgs {
            actor,
            action: "account.reactivate",
            command: Some("account reactivate"),
            args: Some(json!({ "code": code })),
            outcome: "ok",
            entry_ids: vec![],
        },
    )?;
    // JS nests the RAW row here ({account: getAccount(db, id)}) while its
    // sibling `deactivate` emits the slim projection — match JS.
    let row = bukio::accounts::get_account_row_by_code(&db, &code)
        .ok_or_else(|| BukioError::new("INTERNAL", "account vanished"))?;
    Ok(json!({ "account": row }))
}

fn cmd_account_import(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    require_actor(actor)?;
    let file = arg(argv, "--file").ok_or_else(|| missing_arg("--file"))?;
    let text = std::fs::read_to_string(&file)
        .map_err(|e| BukioError::new("IO_ERROR", format!("cannot read {file}: {e}")))?;
    let db = if dry_run {
        bukio::db::open_db(":memory:").map_err(|e| BukioError::new("DB_ERROR", e.to_string()))?
    } else {
        open_existing(db_path)?
    };
    let r = bukio::accounts::import_chart_csv(&db, &text)?;
    if !dry_run {
        bukio::audit::record(
            &db,
            bukio::audit::RecordArgs {
                actor,
                action: "account.import",
                command: Some("account import"),
                args: Some(json!({ "file": file, "created": r.created, "skipped": r.skipped })),
                outcome: "ok",
                entry_ids: vec![],
            },
        )?;
    }
    Ok(json!({
        "file": file, "created": r.created, "skipped": r.skipped, "total": r.total,
        "errors": r.errors.iter().map(|(line, e)| json!({ "line": line, "error": e })).collect::<Vec<_>>(),
        "dryRun": dry_run,
    }))
}

// ── cost-center ────────────────────────────────────────────────────────────

fn cmd_cc_add(argv: &[String], db_path: &str, actor: &str) -> Result<Value> {
    require_actor(actor)?;
    let code = arg(argv, "--code").ok_or_else(|| missing_arg("--code"))?;
    let name = arg(argv, "--name").ok_or_else(|| missing_arg("--name"))?;
    let db = open_existing(db_path)?;
    let cc = bukio::accounts::create_cost_center(&db, &code, &name)?;
    bukio::audit::record(
        &db,
        bukio::audit::RecordArgs {
            actor,
            action: "cost-center.add",
            command: Some("cost-center add"),
            args: Some(json!({ "code": code, "name": name })),
            outcome: "ok",
            entry_ids: vec![],
        },
    )?;
    Ok(bukio::accounts::serialize_cost_center(&cc))
}

fn cmd_cc_list(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let rows = bukio::accounts::list_cost_centers(&db, has_flag(argv, "--include-inactive"))?;
    let rows: Vec<Value> = rows
        .iter()
        .map(bukio::accounts::serialize_cost_center)
        .collect();
    Ok(json!({ "cost_centers": rows }))
}

fn cmd_cc_show(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let code = arg(argv, "--code").ok_or_else(|| missing_arg("--code"))?;
    let cc = bukio::accounts::get_cost_center_by_code(&db, &code).ok_or_else(|| {
        BukioError::new(
            "COST_CENTER_NOT_FOUND",
            format!("cost center '{code}' does not exist"),
        )
    })?;
    Ok(bukio::accounts::serialize_cost_center(&cc))
}

fn cmd_cc_toggle(argv: &[String], db_path: &str, actor: &str) -> Result<Value> {
    require_actor(actor)?;
    let reactivate = argv.iter().any(|a| a == "reactivate");
    let code = arg(argv, "--code").ok_or_else(|| missing_arg("--code"))?;
    let db = open_existing(db_path)?;
    let updated = bukio::accounts::set_cost_center_active(&db, &code, reactivate, reactivate)?;
    bukio::audit::record(
        &db,
        bukio::audit::RecordArgs {
            actor,
            action: if reactivate {
                "cost-center.reactivate"
            } else {
                "cost-center.deactivate"
            },
            command: Some(if reactivate {
                "cost-center reactivate"
            } else {
                "cost-center deactivate"
            }),
            args: Some(json!({ "code": code })),
            outcome: "ok",
            entry_ids: vec![],
        },
    )?;
    // JS nests the RAW row here ({cost_center: updated}) — same asymmetry as
    // account reactivate vs deactivate.
    Ok(json!({ "cost_center": updated }))
}

// ── report ─────────────────────────────────────────────────────────────────

/// One CSV cell: neuter spreadsheet formula injection, then quote per RFC 4180.
/// A value starting with = @ + — or - (but not a plain number, so a negative
/// amount stays an amount) is prefixed with a single quote, as the JS toCsv does.
fn csv_cell(v: &str) -> String {
    let numeric = !v.is_empty() && v.parse::<f64>().is_ok();
    let guarded = match v.chars().next() {
        Some(c) if "=@".contains(c) || ("+-".contains(c) && !numeric) => format!("'{v}"),
        _ => v.to_string(),
    };
    if guarded.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", guarded.replace('"', "\"\""))
    } else {
        guarded
    }
}

fn csv_row(cells: &[String]) -> String {
    cells
        .iter()
        .map(|c| csv_cell(c))
        .collect::<Vec<_>>()
        .join(",")
}

/// CSV file writer.
fn write_csv(path: &str, columns: &[&str], rows: &[Vec<String>]) -> Result<()> {
    let mut w = std::fs::File::create(path)
        .map_err(|e| BukioError::new("IO_ERROR", format!("cannot write {path}: {e}")))?;
    use std::io::Write;
    let cols: Vec<String> = columns.iter().map(|c| c.to_string()).collect();
    writeln!(w, "{}", csv_row(&cols)).ok();
    for row in rows {
        writeln!(w, "{}", csv_row(row)).ok();
    }
    Ok(())
}
/// Generic CSV export for sectioned reports (P&L, balance-sheet, journal).
fn emit_csv(
    argv: &[String],
    data: &Value,
    columns: &[&str],
    flat_fn: impl Fn(&Value) -> Vec<Vec<String>>,
) -> Result<bool> {
    if let Some(fmt) = arg(argv, "--format").filter(|f| f == "csv") {
        if let Some(path) = arg(argv, "--out") {
            let rows = flat_fn(data);
            write_csv(&path, columns, &rows)?;
            return Ok(true);
        }
        // CSV to stdout
        let rows = flat_fn(data);
        let cols: Vec<String> = columns.iter().map(|c| c.to_string()).collect();
        print!("{}", csv_row(&cols));
        println!();
        for row in &rows {
            println!("{}", csv_row(row));
        }
        return Ok(true);
    }
    // XLSX format
    if let Some(fmt) = arg(argv, "--format").filter(|f| f == "xlsx") {
        let path = arg(argv, "--out").ok_or_else(|| {
            BukioError::new("OUT_REQUIRED", "--out <path> is required for xlsx output")
        })?;
        let rows = flat_fn(data);
        write_xlsx(&path, columns, &rows)?;
        return Ok(true);
    }
    Ok(false)
}

fn write_xlsx(path: &str, columns: &[&str], rows: &[Vec<String>]) -> Result<()> {
    use rust_xlsxwriter::Workbook;
    let mut workbook = Workbook::new();
    let sheet = workbook.add_worksheet();
    // Header row
    for (i, col) in columns.iter().enumerate() {
        sheet
            .write_string(0, i as u16, col.to_string())
            .map_err(|e| BukioError::new("FILE_ERROR", e.to_string()))?;
    }
    // Data rows
    for (r_idx, row) in rows.iter().enumerate() {
        for (c_idx, val) in row.iter().enumerate() {
            sheet
                .write_string((r_idx + 1) as u32, c_idx as u16, val.clone())
                .map_err(|e| BukioError::new("FILE_ERROR", e.to_string()))?;
        }
    }
    workbook
        .save(path)
        .map_err(|e| BukioError::new("FILE_ERROR", format!("cannot write xlsx: {e}")))?;
    Ok(())
}

fn cmd_tb(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let data = bukio::reports::trial_balance(
        &db,
        arg(argv, "--year").as_deref().filter(|s| !s.is_empty()),
    )?;
    if let Some(fmt) = arg(argv, "--format").filter(|f| f == "csv") {
        if let Some(path) = arg(argv, "--out") {
            let empty_accounts: Vec<serde_json::Value> = vec![];
            let accounts = data["accounts"].as_array().unwrap_or(&empty_accounts);
            let columns = vec!["code", "account", "type", "debit", "credit", "net"];
            let mut rows: Vec<Vec<String>> = accounts
                .iter()
                .map(|a| {
                    vec![
                        a["code"].as_str().unwrap_or("").to_string(),
                        a["name"].as_str().unwrap_or("").to_string(),
                        a["type"].as_str().unwrap_or("").to_string(),
                        a["debit"].as_str().unwrap_or("").to_string(),
                        a["credit"].as_str().unwrap_or("").to_string(),
                        a["net"].as_str().unwrap_or("").to_string(),
                    ]
                })
                .collect();
            rows.push(vec![
                "".into(),
                "TOTAAL".into(),
                "".into(),
                data["total_debit"].as_str().unwrap_or("0.00").to_string(),
                data["total_credit"].as_str().unwrap_or("0.00").to_string(),
                format!(
                    "{:.2}",
                    (data["total_debit_cents"].as_i64().unwrap_or(0)
                        - data["total_credit_cents"].as_i64().unwrap_or(0))
                        as f64
                        / 100.0
                ),
            ]);
            write_csv(&path, &columns, &rows)?;
            return Ok(json!({"ok": true, "path": path, "format": "csv"}));
        }
    }
    Ok(data)
}

fn cmd_balans(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    // JS defaults --as-of to TODAY, not the fiscal year end
    let as_of = arg(argv, "--as-of").unwrap_or_else(bukio::dates::today_iso);
    let data = bukio::reports::balans(&db, &as_of)?;
    let emitted = emit_csv(
        argv,
        &data,
        &["rgs", "group", "code", "name", "amount"],
        |d| {
            let mut rows = Vec::new();
            for side_key in &["assets", "liabilities_and_equity"] {
                if let Some(sections) = d[side_key]["sections"].as_array() {
                    for sec in sections {
                        let label = sec["label"].as_str().unwrap_or("");
                        let rgs = sec["taxonomy_code"].as_str().unwrap_or("");
                        if let Some(accounts) = sec["accounts"].as_array() {
                            for a in accounts {
                                rows.push(vec![
                                    rgs.to_string(),
                                    label.to_string(),
                                    a["code"].as_str().unwrap_or("").to_string(),
                                    a["name"].as_str().unwrap_or("").to_string(),
                                    a["amount"].as_str().unwrap_or("0.00").to_string(),
                                ]);
                            }
                        }
                    }
                }
            }
            rows
        },
    )?;
    if emitted {
        return Ok(json!({"ok": true}));
    }
    Ok(data)
}

/// Compute fiscal year window: reads company.fiscal_year_end, returns (from, to) for a given year.
/// Default 12-31 → calendar year. Otherwise end=${year}-MM-DD, start=day after ${year-1}-MM-DD.
fn fiscal_year_window(db: &rusqlite::Connection, year: &str) -> (String, String) {
    let fy: String = db
        .prepare("SELECT fiscal_year_end FROM company WHERE id = 1")
        .ok()
        .and_then(|mut s| s.query_row([], |r| r.get(0)).ok())
        .unwrap_or_else(|| "12-31".to_string());
    let parts: Vec<&str> = fy.split('-').collect();
    let mm: u32 = parts[parts.len().checked_sub(2).unwrap_or(0)]
        .parse()
        .unwrap_or(12);
    let dd: u32 = parts.last().and_then(|s| s.parse().ok()).unwrap_or(31);
    if mm == 12 && dd == 31 {
        return (format!("{year}-01-01"), format!("{year}-12-31"));
    }
    let end = format!("{}-{:02}-{:02}", year, mm, dd);
    let y: i32 = year.parse().unwrap_or(2026);
    // start = day after (y-1)-MM-DD
    let start = chrono::NaiveDate::from_ymd_opt(y - 1, mm, dd)
        .map(|d| {
            (d + chrono::Duration::days(1))
                .format("%Y-%m-%d")
                .to_string()
        })
        .unwrap_or_else(|| format!("{}-01-01", y));
    (start, end)
}

fn cmd_pnl(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let year = arg(argv, "--year").unwrap_or_else(|| bukio::dates::today_iso()[0..4].to_string());
    let (from, to) = fiscal_year_window(&db, &year);
    let data = bukio::reports::pnl(&db, &from, &to)?;
    let emitted = emit_csv(
        argv,
        &data,
        &["rgs", "group", "code", "name", "amount"],
        |d| {
            let mut rows = Vec::new();
            if let Some(sections) = d["sections"].as_array() {
                for sec in sections {
                    let label = sec["label"].as_str().unwrap_or("");
                    let rgs = sec["taxonomy_code"].as_str().unwrap_or("");
                    if let Some(accounts) = sec["accounts"].as_array() {
                        for a in accounts {
                            rows.push(vec![
                                rgs.to_string(),
                                label.to_string(),
                                a["code"].as_str().unwrap_or("").to_string(),
                                a["name"].as_str().unwrap_or("").to_string(),
                                a["amount"].as_str().unwrap_or("0.00").to_string(),
                            ]);
                        }
                    }
                }
            }
            rows
        },
    )?;
    if emitted {
        return Ok(json!({"ok": true}));
    }
    Ok(data)
}

fn cmd_journal(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let year = arg(argv, "--year").unwrap_or_else(|| bukio::dates::today_iso()[0..4].to_string());
    let (from, to) = fiscal_year_window(&db, &year);
    let rows = bukio::reports::journal(&db, &from, &to, None)?;
    // JS CLI adds a human `amount` string ('' when the posting has no amount)
    let rows: Vec<Value> = rows
        .iter()
        .map(|r| {
            let mut o = r.clone();
            let amount = match o.get("amount_cents").and_then(|v| v.as_i64()) {
                Some(c) => Value::String(bukio::money::format_amount(c)),
                None => Value::String(String::new()),
            };
            if let Some(m) = o.as_object_mut() {
                m.insert("amount".to_string(), amount);
            }
            o
        })
        .collect();
    let data = json!({ "from": from, "to": to, "rows": rows });
    let emitted = emit_csv(
        argv,
        &data,
        &[
            "date",
            "entry",
            "description",
            "account",
            "debit",
            "credit",
            "actor",
        ],
        |d| {
            let mut out = Vec::new();
            if let Some(arr) = d["rows"].as_array() {
                for r in arr {
                    out.push(vec![
                        r["date"].as_str().unwrap_or("").to_string(),
                        r["entry_id"].as_i64().unwrap_or(0).to_string(),
                        r["description"].as_str().unwrap_or("").to_string(),
                        r["account_code"].as_str().unwrap_or("").to_string(),
                        r["debit"].as_str().unwrap_or("0.00").to_string(),
                        r["credit"].as_str().unwrap_or("0.00").to_string(),
                        r["actor"].as_str().unwrap_or("").to_string(),
                    ]);
                }
            }
            out
        },
    )?;
    if emitted {
        return Ok(json!({"ok": true}));
    }
    Ok(data)
}

// ── audit ──────────────────────────────────────────────────────────────────

fn cmd_audit_list(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let limit: i64 = parse_limit(argv, 50)?;
    let rows = bukio::audit::list(
        &db,
        arg(argv, "--since").as_deref().filter(|s| !s.is_empty()),
        arg(argv, "--by").as_deref().filter(|s| !s.is_empty()),
        limit,
    )?;
    // csv/xlsx export (JS parity: audit --format csv|xlsx --out <path>)
    let columns = [
        "id",
        "timestamp",
        "actor",
        "action",
        "command",
        "args",
        "outcome",
        "entry_ids",
    ];
    let flat = |rows: &[Value]| -> Vec<Vec<String>> {
        rows.iter()
            .map(|r| {
                vec![
                    r["id"].as_i64().map(|v| v.to_string()).unwrap_or_default(),
                    r["timestamp"].as_str().unwrap_or("").to_string(),
                    r["actor"].as_str().unwrap_or("").to_string(),
                    r["action"].as_str().unwrap_or("").to_string(),
                    r["command"].as_str().unwrap_or("").to_string(),
                    r["args"].to_string(),
                    r["outcome"].as_str().unwrap_or("").to_string(),
                    r["entry_ids"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(|v| v.as_i64())
                                .map(|v| v.to_string())
                                .collect::<Vec<_>>()
                                .join(";")
                        })
                        .unwrap_or_default(),
                ]
            })
            .collect()
    };
    let fmt = arg(argv, "--format").unwrap_or_else(|| "csv".into());
    if fmt == "xlsx" && arg(argv, "--out").is_none() {
        return Err(BukioError::new(
            "OUT_REQUIRED",
            "--out <path> is required for xlsx output",
        ));
    }
    if let Some(path) = arg(argv, "--out") {
        if fmt == "csv" {
            write_csv(&path, &columns, &flat(&rows))?;
        } else if fmt == "xlsx" {
            write_xlsx(&path, &columns, &flat(&rows))?;
        } else if fmt == "json" {
            return Ok(json!({ "ok": true, "data": { "entries": rows } }));
        } else {
            return Err(BukioError::new(
                "INVALID_FORMAT",
                format!("unknown --format '{fmt}' — use csv, xlsx or json"),
            ));
        }
        Ok(json!({ "ok": true, "path": path }))
    } else if let Some(fmt) = arg(argv, "--format").filter(|f| f == "json") {
        // --format json: wrap in {ok, data} even without --json flag
        Ok(json!({ "ok": true, "data": { "entries": rows } }))
    } else {
        Ok(json!({ "entries": rows }))
    }
}

fn cmd_audit_verify(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let since = arg(argv, "--since").filter(|s| !s.is_empty());
    let limit: Option<i64> = match arg(argv, "--limit") {
        Some(raw) => Some(raw.parse::<i64>().map_err(|_| {
            BukioError::new(
                "INVALID_LIMIT",
                format!("limit must be a non-negative integer, got '{raw}'"),
            )
        })?),
        None => None,
    };
    let result = bukio::audit::verify_trail(&db, since.as_deref(), limit)?;
    let tampered = result["summary"]["tampered"].as_i64().unwrap_or(0);
    let invalid_sig = result["summary"]["invalid_signature"].as_i64().unwrap_or(0);
    let unknown_key = result["summary"]["unknown_key"].as_i64().unwrap_or(0);
    if tampered + invalid_sig + unknown_key > 0 {
        use std::io::Write;
        let wrapped = json!({ "ok": true, "data": result });
        let json = serde_json::to_string_pretty(&wrapped).unwrap();
        std::io::stdout().write_all(json.as_bytes()).ok();
        std::io::stdout().flush().ok();
        std::process::exit(1);
    }
    Ok(result)
}

// ── backup ─────────────────────────────────────────────────────────────────

fn cmd_backup(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    require_actor(actor)?;
    // --keep must be a positive integer
    let keep = match arg(argv, "--keep") {
        None => None,
        Some(v) => match v.parse::<usize>() {
            Ok(n) if n >= 1 => Some(n),
            _ => {
                return Err(BukioError::new(
                    "INVALID_KEEP",
                    format!("--keep must be a positive integer, got '{v}'"),
                ));
            }
        },
    };
    bukio::backup::cmd_backup(
        db_path,
        arg(argv, "--out").as_deref(),
        keep,
        has_flag(argv, "--encrypt"),
        arg(argv, "--passphrase").as_deref(),
        actor,
        dry_run,
    )
}

fn cmd_backup_restore(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    require_actor(actor)?;
    let from = arg(argv, "--from")
        .or_else(|| arg(argv, "--file"))
        .or_else(|| positional_first(argv))
        .ok_or_else(|| missing_arg("--from"))?;
    let to = arg(argv, "--to").unwrap_or_else(|| db_path.to_string());
    bukio::backup::cmd_restore(
        &from,
        &to,
        has_flag(argv, "--force"),
        arg(argv, "--passphrase").as_deref(),
        actor,
        dry_run,
    )
}

/// Get the first non-flag positional argument after known subcommand tokens.
fn positional_first(argv: &[String]) -> Option<String> {
    let mut seen_cmd = false;
    for a in argv {
        if !a.starts_with('-') {
            if !seen_cmd {
                seen_cmd = true;
                continue; // skip command tokens
            }
            return Some(a.clone());
        }
    }
    None
}

// ── bank ───────────────────────────────────────────────────────────────────

fn cmd_bank_add(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    require_actor(actor)?;
    let db = open_existing(db_path)?;
    let iban = arg(argv, "--iban").ok_or_else(|| missing_arg("--iban"))?;
    let name = arg(argv, "--name");
    let account_code = arg(argv, "--account-code").unwrap_or_else(|| {
        let profile = bukio::accounts::resolve_profile(&db).ok();
        profile
            .and_then(|p| {
                p["reporting"]["bankAccountDefault"]
                    .as_str()
                    .map(String::from)
            })
            .unwrap_or_else(|| "1100".into())
    });
    let result = bukio::bank::get_or_create_bank_account(
        &db,
        &iban,
        name.as_deref(),
        &account_code,
        dry_run,
    )?;
    if dry_run {
        Ok(json!({ "plan": result }))
    } else {
        Ok(
            json!({ "bank_account": { "iban": result["iban"], "name": result["name"], "account_code": result["account_code"], "id": result["id"] } }),
        )
    }
}

fn cmd_bank_list(db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let accounts = bukio::bank::list_bank_accounts(&db)?;
    // the JS CLI projects exactly these seven keys (no id, no created_*)
    let data: Vec<Value> = accounts
        .iter()
        .map(|a| {
            json!({
                "iban": a["iban"], "name": a["name"], "account_code": a["account_code"],
                "transaction_count": a["transaction_count"],
                "unmatched_count": a["unmatched_count"],
                "balance_cents": a["balance_cents"], "balance": a["balance"],
            })
        })
        .collect();
    Ok(json!({ "accounts": data }))
}

fn cmd_bank_import(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    require_actor(actor)?;
    let db = open_existing(db_path)?;
    let _profile = bukio::accounts::resolve_profile(&db)?; // fail early for unknown country
    let file = arg(argv, "--file").ok_or_else(|| missing_arg("--file"))?;
    let iban = arg(argv, "--iban").ok_or_else(|| missing_arg("--iban"))?;
    let name = arg(argv, "--name");
    let account_code = arg(argv, "--account-code").unwrap_or_else(|| "1100".into());
    let content = std::fs::read_to_string(&file)
        .map_err(|e| BukioError::new("FILE_ERROR", format!("cannot read {file}: {e}")))?;
    let (transactions, skipped, from_csv) = if content.trim_start().starts_with('<') {
        (
            bukio::bank::parse_camt053(&content)
                .map_err(|e| BukioError::new("PARSE_ERROR", e.message.clone()))?,
            Vec::new(),
            false,
        )
    } else {
        let parsed = bukio::bank::parse_bank_csv(&content, &iban)
            .map_err(|e| BukioError::new("PARSE_ERROR", e.message.clone()))?;
        (parsed.transactions, parsed.skipped, true)
    };
    if dry_run {
        // the JS plan carries the first ten parsed rows (with a human amount)
        // plus the preview counts
        let preview = bukio::bank::preview_import(&db, &iban, &transactions)?;
        let first: Vec<Value> = transactions
            .iter()
            .take(10)
            .map(|t| {
                let mut v = serde_json::to_value(t).unwrap_or(Value::Null);
                // the JS CSV parser sets no bank_ref at all; the camt one does
                if from_csv {
                    if let Some(o) = v.as_object_mut() {
                        o.remove("bank_ref");
                    }
                }
                v["amount"] = json!(bukio::money::format_amount(t.amount_cents));
                v
            })
            .collect();
        let mut out = json!({
            "action": "import bank transactions", "file": file,
            "transactions": first, "dryRun": true,
        });
        if let (Some(obj), Some(p)) = (out.as_object_mut(), preview.as_object()) {
            for (k, v) in p {
                obj.insert(k.clone(), v.clone());
            }
        }
        Ok(out)
    } else {
        let mut result = bukio::bank::import_transactions(
            &db,
            &iban,
            &transactions,
            name.as_deref(),
            &account_code,
            actor,
        )?;
        // rows the parser had to skip (never dropped silently)
        if !skipped.is_empty() {
            result["skipped"] = json!(skipped);
        }
        result["dryRun"] = json!(false);
        Ok(result)
    }
}

fn cmd_bank_transactions(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let state = arg(argv, "--state");
    let iban = arg(argv, "--iban");
    let limit: i64 = parse_limit(argv, 200)?;
    let transactions =
        bukio::bank::list_transactions(&db, state.as_deref(), iban.as_deref(), limit)?;
    // the JS CLI's fmtTx projection (drops bank_account_id, adds the human amount)
    let data: Vec<Value> = transactions
        .iter()
        .map(|t| {
            json!({
                "id": t["id"], "date": t["date"], "amount_cents": t["amount_cents"],
                "amount": t["amount"], "counterparty": t["counterparty"],
                "description": t["description"], "iban_counter": t["iban_counter"],
                "iban": t["iban"], "account_code": t["account_code"],
                "state": t["state"], "hash": t["hash"],
            })
        })
        .collect();
    Ok(json!({ "transactions": data }))
}

fn cmd_bank_match_auto(
    argv: &[String],
    db_path: &str,
    actor: &str,
    dry_run: bool,
) -> Result<Value> {
    require_actor(actor)?;
    let db = open_existing(db_path)?;
    let window: i64 = match arg(argv, "--window-days") {
        None => 5,
        Some(v) => v.parse::<i64>().map_err(|_| {
            BukioError::new(
                "INVALID_WINDOW",
                format!("invalid --window-days '{v}' — must be a non-negative integer"),
            )
        })?,
    };
    if window < 0 {
        return Err(BukioError::new(
            "INVALID_WINDOW",
            "--window-days must not be negative",
        ));
    }
    let mut result = bukio::bank::auto_match(&db, window, actor, dry_run)?;
    // the JS CLI adds the human amount per match and the dryRun flag
    if let Some(arr) = result["matched"].as_array_mut() {
        for m in arr.iter_mut() {
            let amt = m["amount_cents"].as_i64().unwrap_or(0);
            m["amount"] = json!(bukio::money::format_amount(amt));
        }
    }
    result["dryRun"] = json!(dry_run);
    Ok(result)
}

fn cmd_bank_match_suggest(db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let suggestions = bukio::bank::suggest_unmatched(&db)?;
    // the JS CLI drops the internal bank_account_id from the row
    let data: Vec<Value> = suggestions
        .iter()
        .map(|s| {
            let mut v = s.clone();
            if let Some(o) = v.as_object_mut() {
                o.remove("bank_account_id");
            }
            v
        })
        .collect();
    Ok(json!({ "suggestions": data }))
}

fn cmd_bank_match_link(
    argv: &[String],
    db_path: &str,
    actor: &str,
    dry_run: bool,
) -> Result<Value> {
    require_actor(actor)?;
    let db = open_existing(db_path)?;
    let tx_id: i64 = parse_i64(argv, "--tx").ok_or_else(|| missing_arg("--tx"))?;
    let entry_id: i64 = parse_i64(argv, "--entry").ok_or_else(|| missing_arg("--entry"))?;
    let method = arg(argv, "--method").unwrap_or_else(|| "manual".into());
    let result =
        bukio::bank::link_transaction(&db, tx_id, entry_id, &method, None, actor, dry_run)?;
    if dry_run {
        Ok(json!({ "plan": result }))
    } else {
        Ok(json!({ "transaction": result, "entry_id": entry_id }))
    }
}

fn cmd_bank_match_post(
    argv: &[String],
    db_path: &str,
    actor: &str,
    dry_run: bool,
) -> Result<Value> {
    require_actor(actor)?;
    let db = open_existing(db_path)?;
    let tx_id: i64 = parse_i64(argv, "--tx").ok_or_else(|| missing_arg("--tx"))?;
    let account = arg(argv, "--account").ok_or_else(|| missing_arg("--account"))?;
    if dry_run {
        let tx = bukio::bank::get_transaction(&db, tx_id)?.ok_or_else(|| {
            BukioError::new(
                "NOT_FOUND",
                format!("bank transaction {tx_id} does not exist"),
            )
        })?;
        // Validate: must be unmatched
        if tx["state"].as_str() == Some("matched") {
            return Err(BukioError::new(
                "ALREADY_MATCHED",
                format!("bank transaction {tx_id} is already matched"),
            ));
        }
        // Validate: account must exist
        let account_exists = db
            .prepare("SELECT 1 FROM accounts WHERE code = ?1 AND active = 1")
            .map_err(|e| BukioError::new("DB_ERROR", e.to_string()))?
            .exists(rusqlite::params![account])
            .map_err(|e| BukioError::new("DB_ERROR", e.to_string()))?;
        if !account_exists {
            return Err(BukioError::new(
                "ACCOUNT_NOT_FOUND",
                format!("account '{account}' does not exist"),
            ));
        }
        let amount = tx["amount_cents"].as_i64().unwrap_or(0);
        return Ok(json!({
            "action": "post entry from bank transaction",
            "tx": tx,
            "postings": [
                { "code": tx["account_code"], "amount_cents": amount, "amount": bukio::money::format_amount(amount) },
                { "code": account, "amount_cents": -amount, "amount": bukio::money::format_amount(-amount) },
            ],
            "dryRun": true,
        }));
    }
    let (_tx, entry) = bukio::bank::post_from_transaction(&db, tx_id, &account, actor, true)?;
    Ok(json!({ "entry_id": entry["id"], "state": entry["state"] }))
}

fn cmd_bank_ignore(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    require_actor(actor)?;
    let db = open_existing(db_path)?;
    let tx_id: i64 = parse_i64(argv, "--tx").ok_or_else(|| missing_arg("--tx"))?;
    bukio::bank::set_transaction_state(&db, tx_id, "ignored", actor, dry_run)
}

fn cmd_bank_unignore(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    require_actor(actor)?;
    let db = open_existing(db_path)?;
    let tx_id: i64 = parse_i64(argv, "--tx").ok_or_else(|| missing_arg("--tx"))?;
    bukio::bank::set_transaction_state(&db, tx_id, "unmatched", actor, dry_run)
}

// ── vat ────────────────────────────────────────────────────────────────────

fn cmd_vat_enable(db_path: &str, actor: &str) -> Result<Value> {
    require_actor(actor)?;
    let db = open_existing(db_path)?;
    bukio::vat::enable_vat_module(&db, actor)
}

fn cmd_vat_codes(db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let codes = bukio::vat::list_vat_codes(&db)?;
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
    let date = arg(argv, "--date").unwrap_or_else(bukio::dates::today_iso);
    let desc = arg(argv, "--desc").ok_or_else(|| missing_arg("--desc"))?;
    let postings_raw = repeated(argv, "--postings");
    let specs = bukio::vat::parse_vat_posting_specs(&postings_raw)?;
    if dry_run {
        bukio::dates::validate_date(&date)?;
        // Validate balanced (parity with entry add)
        let sum: i64 = specs.iter().map(|s| s.amount_cents).sum();
        if sum != 0 {
            return Err(BukioError::new(
                "UNBALANCED",
                format!("postings do not sum to zero (sum = {sum})"),
            ));
        }
        // Validate at least 2 postings
        if specs.len() < 2 {
            return Err(BukioError::new(
                "TOO_FEW_POSTINGS",
                "an entry needs at least 2 postings",
            ));
        }
        return Ok(json!({
            "action": "create vat-aware journal entry", "date": date, "description": desc,
            "postings": specs.iter().map(|s| json!({ "code": s.code, "amount_cents": s.amount_cents, "amount": bukio::money::format_amount(s.amount_cents), "vat_code": s.vat_code })).collect::<Vec<_>>(),
            "post": has_flag(argv, "--post"), "dryRun": true,
        }));
    }
    let db = open_existing(db_path)?;
    let entry = bukio::vat::book_vat_entry(
        &db,
        &date,
        &desc,
        &specs,
        "manual",
        arg(argv, "--source-ref").as_deref(),
        actor,
        has_flag(argv, "--post"),
    )?;
    let db2 = open_existing(db_path)?;
    let (all_legs, _) = bukio::vat::expand_vat_postings(&db2, &specs)?;
    let expanded: Vec<Value> = {
        let mut out: Vec<Value> = Vec::new();
        for sp in specs.iter() {
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
    // module first, then the format dispatch — the same order as the JS, so a
    // company with VAT off hears VAT_MODULE_OFF rather than a format complaint
    bukio::vat::require_vat(&db)?;
    let profile = bukio::accounts::resolve_profile(&db)?;
    if profile["tax"]["returnLayout"].as_str().is_none() {
        return Err(BukioError::new(
            "FORMAT_NOT_SUPPORTED",
            format!(
                "VAT return is not supported for country {}",
                profile["meta"]["country"].as_str().unwrap_or("?")
            ),
        ));
    }
    let period = arg(argv, "--period").ok_or_else(|| missing_arg("--period"))?;
    if has_flag(argv, "--mark-filed") {
        require_actor(actor)?;
        let result = bukio::vat::mark_filed(&db, &period, actor)?;
        let mut out = result.clone();
        out["dryRun"] = json!(false);
        return Ok(out);
    }
    let readout = bukio::vat::ob_readout(&db, &period)?;
    let mut fields = serde_json::Map::new();
    if let Some(obj) = readout["fields"].as_object() {
        for (k, v) in obj {
            let cents = v.as_i64().unwrap_or(0);
            fields.insert(
                k.clone(),
                json!({ "cents": cents, "amount": bukio::money::format_amount(cents) }),
            );
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
    bukio::vat::vat_file(
        &db,
        arg(argv, "--account").as_deref(),
        arg(argv, "--period").as_deref(),
        arg(argv, "--desc").as_deref(),
        actor,
        dry_run,
        &bukio::i18n::resolve_locale(arg(argv, "--locale").as_deref()),
    )
}

fn cmd_vat_settle(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    require_actor(actor)?;
    let tx: i64 = parse_i64(argv, "--tx").ok_or_else(|| missing_arg("--tx"))?;
    let db = open_existing(db_path)?;
    let row: Option<(i64, String)> = db
        .query_row(
            "SELECT t.amount_cents, a.account_code FROM bank_transactions t
             JOIN bank_accounts a ON a.id = t.bank_account_id WHERE t.id = ?1",
            [tx],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .ok();
    let Some((tx_amount, bank_code)) = row else {
        return Err(BukioError::new(
            "NOT_FOUND",
            format!("bank transaction {tx} does not exist"),
        ));
    };
    // Check if already matched before calling vat_settle
    if !dry_run {
        let state: String = db
            .query_row(
                "SELECT state FROM bank_transactions WHERE id = ?1",
                [tx],
                |r| r.get(0),
            )
            .map_err(|_| {
                BukioError::new("NOT_FOUND", format!("bank transaction {tx} does not exist"))
            })?;
        if state != "unmatched" {
            return Err(BukioError::new(
                "ALREADY_MATCHED",
                format!("bank transaction {tx} is already {state} — use a different payment transaction"),
            ));
        }
    }
    let mut result = bukio::vat::vat_settle(
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
        &bukio::i18n::resolve_locale(arg(argv, "--locale").as_deref()),
    )?;
    if !dry_run {
        if let Some(entry_id) = result["entry_id"].as_i64() {
            let _ = db.execute(
                "UPDATE bank_transactions SET state = 'matched' WHERE id = ?1",
                rusqlite::params![tx],
            );
            result["tx"] = json!({ "id": tx, "state": "matched" });
        }
    }
    Ok(result)
}

// ── recurring ──────────────────────────────────────────────────────────────

/// The JS CLI's fmtPostings (src/cli/recurring.js): snake_case + a formatted
/// amount string alongside the cents.
fn fmt_postings(postings: &Value) -> Value {
    let arr = postings.as_array().cloned().unwrap_or_default();
    Value::Array(
        arr.iter()
            .map(|p| {
                let cents = p["amountCents"]
                    .as_i64()
                    .or_else(|| p["amount_cents"].as_i64())
                    .unwrap_or(0);
                json!({
                    "code": p["code"].clone(),
                    "amount_cents": cents,
                    "amount": bukio::money::format_amount(cents),
                    "vat_code": p["vatCode"].clone(),
                })
            })
            .collect(),
    )
}

/// The JS CLI's fmtTemplate — a fixed key set, bools for the 0/1 flags.
fn fmt_template(t: &Value) -> Value {
    let postings = if t["postings"].is_null() {
        Value::Null
    } else {
        fmt_postings(&t["postings"])
    };
    let final_postings = if t["final_postings"].is_null() {
        Value::Null
    } else {
        fmt_postings(&t["final_postings"])
    };
    let mut out = json!({
        "id": t["id"].clone(), "name": t["name"].clone(),
        "description": t["description"].clone(),
        "kind": t["kind"].clone(), "contact_id": t["contact_id"].clone(),
        "invoice_lines": t["invoice_lines"].clone(),
        "frequency": t["frequency"].clone(), "day_of_period": t["day_of_period"].clone(),
        "start_date": t["start_date"].clone(), "end_date": t["end_date"].clone(),
        "runs": t["runs"].clone(), "status": t["status"].clone(),
        "next_run_date": t["next_run_date"].clone(), "last_run_date": t["last_run_date"].clone(),
        "runs_done": t["runs_done"].clone(),
        "reverse_previous": json!(t["reverse_previous"].as_i64().unwrap_or(0) == 1),
        "vat_aware": json!(t["vat_aware"].as_i64().unwrap_or(0) == 1),
        "postings": postings, "final_postings": final_postings,
    });
    if !t["action"].is_null() {
        out["action"] = t["action"].clone();
    }
    if t["dryRun"].is_boolean() {
        out["dryRun"] = t["dryRun"].clone();
    }
    out
}

/// The JS CLI's recurring-run projection: {ok, error, runs:[{kind?, entries[]}]}.
fn fmt_run(data: &Value) -> Value {
    let templates = data["templates"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|t| {
            let runs = t["runs"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .iter()
                .map(|r| {
                    let entries: Vec<Value> = match r["generated"].as_array() {
                        Some(gens) => gens
                            .iter()
                            .map(|g| {
                                if g["kind"] == "invoice" {
                                    json!({
                                        "kind": "invoice",
                                        "invoice_id": g["invoice"]["id"].clone(),
                                        "date": g["invoice"]["date"].clone(),
                                        "state": "draft",
                                    })
                                } else {
                                    json!({
                                        "kind": g["kind"].clone(),
                                        "entry_id": g["entry"]["id"].clone(),
                                        "date": g["entry"]["date"].clone(),
                                        "state": g["entry"]["state"].clone(),
                                    })
                                }
                            })
                            .collect(),
                        None => vec![json!({
                            "kind": if r["kind"].is_null() { json!("entry") } else { r["kind"].clone() },
                            "entry_id": r["entry"]["id"].clone(),
                            "date": r["entry"]["date"].clone(),
                            "state": "plan",
                        })],
                    };
                    let mut run = json!({ "entries": entries });
                    if r["generated"].is_null() {
                        run["kind"] = if r["kind"].is_null() { json!("entry") } else { r["kind"].clone() };
                    }
                    run
                })
                .collect::<Vec<Value>>();
            json!({
                "template_id": t["template_id"].clone(),
                "name": t["name"].clone(),
                "ok": t["ok"].clone(),
                "error": if t["error"].is_null() { Value::Null } else { t["error"].clone() },
                "runs": runs,
            })
        })
        .collect::<Vec<Value>>();
    json!({ "as_of": data["as_of"].clone(), "dry_run": data["dry_run"].clone(), "templates": templates })
}

fn cmd_recurring_add(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    require_actor(actor)?;
    let db = open_existing(db_path)?;
    let kind = arg(argv, "--kind").unwrap_or_else(|| "entry".into());
    let desc = arg(argv, "--desc");
    let day: u32 = match arg(argv, "--day") {
        None => 1,
        Some(v) => v.parse::<u32>().map_err(|_| {
            BukioError::new(
                "INVALID_DATE",
                format!("invalid --day '{v}' — must be 1-28"),
            )
        })?,
    };
    if day == 0 || day > 28 {
        return Err(BukioError::new(
            "INVALID_DATE",
            "--day must be between 1 and 28",
        ));
    }
    let end = arg(argv, "--end");
    let runs = parse_i64(argv, "--runs").map(|v| v as i64);
    let reverse_previous = has_flag(argv, "--reverse-previous");
    let frequency = arg(argv, "--frequency").ok_or_else(|| missing_arg("--frequency"))?;
    let start = arg(argv, "--start").ok_or_else(|| missing_arg("--start"))?;

    if kind == "entry" {
        // pass the raw specs through (like the JS CLI): flattening them here
        // dropped the "@VAT" tag, so a VAT-tagged template failed UNBALANCED
        let postings_raw = repeated(argv, "--postings");
        let postings_json = serde_json::to_string(&postings_raw).unwrap_or_default();
        let name = arg(argv, "--name")
            .unwrap_or_else(|| desc.clone().unwrap_or_else(|| "recurring entry".into()));
        let tpl = bukio::recurring::create_template(
            &db,
            &name,
            desc.as_deref(),
            &frequency,
            day,
            &start,
            end.as_deref(),
            runs,
            &postings_json,
            reverse_previous,
            actor,
            &kind,
            dry_run,
        )?;
        // dry-run: the plan is returned bare (JS parity); real runs wrap.
        if dry_run {
            Ok(tpl)
        } else {
            Ok(json!({ "template": fmt_template(&tpl), "dryRun": false }))
        }
    } else {
        // invoice kind — need contact + lines
        let contact_id: i64 =
            parse_i64(argv, "--contact").ok_or_else(|| missing_arg("--contact"))?;
        let due_days = match arg(argv, "--due-days") {
            None => None,
            Some(v) => {
                let val = v.parse::<i64>().map_err(|_| {
                    BukioError::new(
                        "INVALID_DUE_DAYS",
                        format!("invalid --due-days '{v}' — must be a non-negative integer"),
                    )
                })?;
                if val < 0 {
                    return Err(BukioError::new(
                        "INVALID_DUE_DAYS",
                        "--due-days must not be negative",
                    ));
                }
                Some(val)
            }
        };
        let lines_raw = repeated(argv, "--lines");
        let items_raw = repeated(argv, "--items");
        let name = arg(argv, "--name")
            .unwrap_or_else(|| desc.clone().unwrap_or_else(|| "recurring invoice".into()));

        // Combine lines + items into a JSON array for the template (kept for
        // parity with the JS CLI; the engine reads lines/items directly).
        let postings_json = json!({
            "contact_id": contact_id,
            "lines": lines_raw,
            "items": items_raw,
            "due_days": due_days,
        })
        .to_string();

        let tpl = bukio::recurring::create_template(
            &db,
            &name,
            desc.as_deref(),
            &frequency,
            day,
            &start,
            end.as_deref(),
            runs,
            &postings_json,
            reverse_previous,
            actor,
            &kind,
            dry_run,
        )?;
        if dry_run {
            Ok(tpl)
        } else {
            Ok(json!({ "template": tpl, "dryRun": false }))
        }
    }
}

fn cmd_recurring_list(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let status = arg(argv, "--status").unwrap_or_else(|| "active".into());
    let rows = bukio::recurring::list_templates(&db, &status)?;
    let mapped: Vec<Value> = rows.iter().map(fmt_template).collect();
    Ok(json!({ "templates": mapped }))
}

fn cmd_recurring_show(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let id: i64 = parse_i64(argv, "--id").ok_or_else(|| missing_arg("--id"))?;
    let tpl = bukio::recurring::get_template(&db, id)?.ok_or_else(|| {
        BukioError::new(
            "NOT_FOUND",
            format!("recurring template {id} does not exist"),
        )
    })?;
    Ok(json!({ "template": fmt_template(&tpl) }))
}

fn cmd_recurring_pause(
    argv: &[String],
    db_path: &str,
    actor: &str,
    dry_run: bool,
) -> Result<Value> {
    require_actor(actor)?;
    let db = open_existing(db_path)?;
    let id: i64 = parse_i64(argv, "--id").ok_or_else(|| missing_arg("--id"))?;
    Ok(
        json!({ "template": fmt_template(&bukio::recurring::set_template_status(&db, id, "paused", actor, dry_run)?) }),
    )
}

fn cmd_recurring_resume(
    argv: &[String],
    db_path: &str,
    actor: &str,
    dry_run: bool,
) -> Result<Value> {
    require_actor(actor)?;
    let db = open_existing(db_path)?;
    let id: i64 = parse_i64(argv, "--id").ok_or_else(|| missing_arg("--id"))?;
    Ok(
        json!({ "template": fmt_template(&bukio::recurring::set_template_status(&db, id, "active", actor, dry_run)?) }),
    )
}

fn cmd_recurring_preview(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let as_of = arg(argv, "--as-of");
    let template_id = parse_i64(argv, "--template");
    bukio::recurring::run_due(&db, as_of.as_deref(), template_id, "preview", true)
}

fn cmd_recurring_run(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    require_actor(actor)?;
    let db = open_existing(db_path)?;
    let as_of = arg(argv, "--as-of");
    let template_id = parse_i64(argv, "--template");
    let result = bukio::recurring::run_due(&db, as_of.as_deref(), template_id, actor, dry_run)?;
    // Text rendering for non-json mode (matches JS CLI output)
    let json_mode = has_flag(argv, "--json");
    if !json_mode {
        let total: usize = result["templates"].as_array().map_or(0, |t| {
            t.iter()
                .map(|t| t["runs"].as_array().map_or(0, |r| r.len()))
                .sum()
        });
        println!(
            "recurring run: {} period(s) across {} template(s){}",
            total,
            result["templates"].as_array().map_or(0, |t| t.len()),
            if dry_run { " (dry run)" } else { "" }
        );
        for t in result["templates"].as_array().unwrap_or(&vec![]) {
            for run in t["runs"].as_array().unwrap_or(&vec![]) {
                if let Some(invoice) = run.get("invoice") {
                    // invoice plan
                    let date = invoice["date"].as_str().unwrap_or("?");
                    let contact = invoice["contact_name"].as_str().unwrap_or("contact");
                    println!("  {}  → draft invoice ({}) (plan)", date, contact);
                } else if let Some(gen) = run.get("generated") {
                    for g in gen.as_array().unwrap_or(&vec![]) {
                        let kind = g["kind"].as_str().unwrap_or("entry");
                        if kind == "invoice" {
                            let date = g["invoice"]["date"].as_str().unwrap_or("?");
                            println!(
                                "  {}  → draft invoice #{} (finalize to book & number)",
                                date, g["invoice"]["id"]
                            );
                        } else {
                            let date = g["entry"]["date"].as_str().unwrap_or("?");
                            let id = g["entry"]["id"].as_i64().unwrap_or(0);
                            let state = g["entry"]["state"].as_str().unwrap_or("draft");
                            println!("  {}  → booked entry #{} ({})", date, id, state);
                        }
                    }
                } else {
                    // dry-run plan entry
                    let kind = run["kind"].as_str().unwrap_or("entry");
                    let date = run
                        .get("entry")
                        .and_then(|e| e["date"].as_str())
                        .unwrap_or("?");
                    if kind == "reversal" {
                        println!("  {}  → reversal of previous entry (plan)", date);
                    } else {
                        println!("  {}  → entry (plan)", date);
                    }
                }
            }
        }
        std::process::exit(0);
    }
    Ok(fmt_run(&result))
}

// ── depreciation ───────────────────────────────────────────────────────────

fn cmd_depreciation_add(
    argv: &[String],
    db_path: &str,
    actor: &str,
    dry_run: bool,
) -> Result<Value> {
    require_actor(actor)?;
    let db = open_existing(db_path)?;
    let name = arg(argv, "--desc").unwrap_or_else(|| "depreciation".into());
    let asset_code = arg(argv, "--asset").unwrap_or_else(|| "1800".into());
    let expense_code = arg(argv, "--expense").unwrap_or_else(|| "4600".into());
    let residual = arg(argv, "--residual").unwrap_or_else(|| "0".into());
    let residual_cents = bukio::money::parse_amount(&residual)?;
    // cost_cents comes from the asset account balance or a param — use 0 as default
    let cost_str = arg(argv, "--cost").unwrap_or_else(|| "0".into());
    let cost_cents: i64 = bukio::money::parse_amount(&cost_str)?;
    let life_months: i64 = parse_life_months(argv)?;
    bukio::recurring::build_depreciation_template(
        &db,
        &name,
        &asset_code,
        &expense_code,
        cost_cents,
        residual_cents,
        life_months,
        &bukio::dates::today_iso(),
        arg(argv, "--desc").as_deref(),
        actor,
        dry_run,
    )
}

// ── contact ────────────────────────────────────────────────────────────────

fn cmd_contact_add(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    require_actor(actor)?;
    let name = arg(argv, "--name").ok_or_else(|| missing_arg("--name"))?;
    let db = open_existing(db_path)?;
    let contact = bukio::contacts::create_contact(
        &db,
        &name,
        arg(argv, "--address").as_deref(),
        arg(argv, "--postal-code").as_deref(),
        arg(argv, "--city").as_deref(),
        arg(argv, "--country").as_deref(),
        arg(argv, "--email").as_deref(),
        arg(argv, "--vat-id").as_deref(),
        arg(argv, "--kvk").as_deref(),
        arg(argv, "--iban").as_deref(),
        actor,
        dry_run,
    )?;
    bukio::audit::record(
        &db,
        bukio::audit::RecordArgs {
            actor,
            action: "contact.add",
            command: Some("contact add"),
            args: Some(json!({ "name": name })),
            outcome: "ok",
            entry_ids: vec![],
        },
    )?;
    Ok(json!({ "contact": contact }))
}

fn cmd_contact_update(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    require_actor(actor)?;
    let id: i64 = parse_i64(argv, "--id").ok_or_else(|| missing_arg("--id"))?;
    let db = open_existing(db_path)?;
    let updated = bukio::contacts::update_contact(
        &db,
        id,
        arg(argv, "--name").as_deref(),
        arg(argv, "--address").as_deref(),
        arg(argv, "--postal-code").as_deref(),
        arg(argv, "--city").as_deref(),
        arg(argv, "--country").as_deref(),
        arg(argv, "--email").as_deref(),
        arg(argv, "--vat-id").as_deref(),
        arg(argv, "--kvk").as_deref(),
        arg(argv, "--iban").as_deref(),
        actor,
        dry_run,
    )?;
    // the JS wraps the updated contact: { contact: c }
    Ok(json!({ "contact": updated }))
}

fn cmd_contact_list(db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let rows = bukio::contacts::list_contacts(&db)?;
    // the JS CLI projects five keys (src/cli/invoice.js)
    let contacts: Vec<Value> = rows
        .iter()
        .map(|c| {
            json!({
                "id": c["id"], "name": c["name"], "city": c["city"],
                "vat_id": c["vat_id"], "email": c["email"],
            })
        })
        .collect();
    Ok(json!({ "contacts": contacts }))
}

fn cmd_contact_statement(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let id: i64 = parse_i64(argv, "--id").ok_or_else(|| missing_arg("--id"))?;
    let as_of = arg(argv, "--as-of");
    bukio::contacts::contact_statement(&db, id, as_of.as_deref())
}

// ── invoice ────────────────────────────────────────────────────────────────

fn cmd_invoice_create(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    require_actor(actor)?;
    let db = open_existing(db_path)?;
    let contact_id: i64 = parse_i64(argv, "--contact").ok_or_else(|| missing_arg("--contact"))?;
    // parse_i64 turns garbage into None, which silently became the 30-day
    // default; the JS throws INVALID_DUE_DAYS, and a wrong payment term is a
    // wrong due date on a real invoice
    let due_days = match arg(argv, "--due-days") {
        None => None,
        Some(v) => Some(v.parse::<i64>().map_err(|_| {
            BukioError::new(
                "INVALID_DUE_DAYS",
                format!("invalid --due-days '{v}' — must be a non-negative integer"),
            )
        })?),
    };
    let delivery_date = arg(argv, "--delivery-date");
    let description = arg(argv, "--description");
    let reference = arg(argv, "--reference");
    let notes = arg(argv, "--notes");
    let discount_pct = arg(argv, "--discount-pct");
    let discount_amount = arg(argv, "--discount-amount");
    if discount_pct.is_some() && discount_amount.is_some() {
        return Err(BukioError::new(
            "INVALID_DISCOUNT",
            "use --discount-pct OR --discount-amount, not both",
        ));
    }
    let (discount_type, discount_value) = if let Some(pct) = discount_pct {
        (
            Some("pct".to_string()),
            pct.parse::<f64>().ok().map(|v| (v * 100.0).round() as i64),
        )
    } else if let Some(amt) = discount_amount {
        (
            Some("amount".to_string()),
            bukio::money::parse_amount(&amt).ok(),
        )
    } else {
        (None, None)
    };

    // Build lines from --lines and --items (raw strings for parse_line_spec)
    let lines_raw: Vec<Value> = repeated(argv, "--lines")
        .into_iter()
        .map(|l| json!(l))
        .chain(
            repeated(argv, "--items")
                .into_iter()
                .map(|i| json!({ "type": "item", "spec": i })),
        )
        .collect();

    let date = arg(argv, "--date").unwrap_or_else(|| bukio::dates::today_iso());
    let invoice = bukio::invoice::create_invoice(
        &db,
        contact_id,
        &date,
        due_days,
        delivery_date.as_deref(),
        description.as_deref(),
        reference.as_deref(),
        notes.as_deref(),
        discount_type.as_deref(),
        discount_value,
        arg(argv, "--language").as_deref(),
        &lines_raw,
        actor,
        dry_run,
    )?;
    if dry_run {
        Ok(invoice)
    } else {
        Ok(json!({ "invoice": bukio::invoice::fmt_invoice(&invoice), "dryRun": false }))
    }
}

fn cmd_invoice_finalize(
    argv: &[String],
    db_path: &str,
    actor: &str,
    dry_run: bool,
) -> Result<Value> {
    require_actor(actor)?;
    let db = open_existing(db_path)?;
    let id: i64 = parse_i64(argv, "--id").ok_or_else(|| missing_arg("--id"))?;
    let inv = bukio::invoice::finalize_invoice(&db, id, actor, dry_run)?;
    if dry_run {
        return Ok(inv); // the JS echoes the plan raw
    }
    Ok(json!({
        "invoice": bukio::invoice::fmt_invoice(&inv["invoice"]),
        "entry": { "id": inv["entry"]["id"], "state": inv["entry"]["state"] },
        "dryRun": false,
    }))
}

fn cmd_invoice_list(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let status = arg(argv, "--status");
    let invoice_type = arg(argv, "--type");
    let rows = bukio::invoice::list_invoices(&db, status.as_deref(), invoice_type.as_deref())?;
    Ok(json!({ "invoices": rows.iter().map(bukio::invoice::fmt_invoice).collect::<Vec<_>>() }))
}

fn cmd_invoice_show(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let id: i64 = parse_i64(argv, "--id").ok_or_else(|| missing_arg("--id"))?;
    let inv = bukio::invoice::get_invoice(&db, id)?
        .ok_or_else(|| BukioError::new("NOT_FOUND", format!("invoice {id} does not exist")))?;
    Ok(json!({ "invoice": bukio::invoice::fmt_invoice(&inv) }))
}

fn cmd_invoice_credit(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    require_actor(actor)?;
    let db = open_existing(db_path)?;
    let id: i64 = parse_i64(argv, "--id").ok_or_else(|| missing_arg("--id"))?;
    let date = arg(argv, "--date");
    let reason = arg(argv, "--reason");
    let inv = bukio::invoice::credit_invoice(
        &db,
        id,
        date.as_deref(),
        reason.as_deref(),
        actor,
        dry_run,
    )?;
    if dry_run {
        return Ok(inv); // the JS echoes the plan raw
    }
    Ok(json!({ "invoice": bukio::invoice::fmt_invoice(&inv), "dryRun": false }))
}

fn cmd_invoice_pay(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    require_actor(actor)?;
    let db = open_existing(db_path)?;
    let id: i64 = parse_i64(argv, "--id").ok_or_else(|| missing_arg("--id"))?;
    let method = arg(argv, "--method").unwrap_or_else(|| "bank".into());
    let amount_str = arg(argv, "--amount");
    let amount_cents = if let Some(a) = &amount_str {
        bukio::money::parse_amount(a)?
    } else {
        // Full outstanding — get from invoice
        let inv = bukio::invoice::get_invoice(&db, id)?
            .ok_or_else(|| BukioError::new("NOT_FOUND", format!("invoice {id} does not exist")))?;
        inv["outstanding_cents"].as_i64().unwrap_or(0)
    };
    let inv = bukio::invoice::mark_paid(
        &db,
        id,
        &bukio::dates::today_iso(),
        amount_cents,
        &method,
        actor,
        dry_run,
        None,
    )?;
    if inv.get("dryRun").and_then(|v| v.as_bool()).unwrap_or(false) {
        Ok(json!({ "plan": inv }))
    } else {
        Ok(json!({ "invoice": bukio::invoice::fmt_invoice(&inv) }))
    }
}

// ── year-end ───────────────────────────────────────────────────────────────

fn cmd_year_end_status(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let year = arg(argv, "--year").ok_or_else(|| missing_arg("--year"))?;
    // JS CLI wraps: { status: yearEndStatus(...) }
    Ok(json!({ "status": bukio::year_end::year_end_status(&db, &year)? }))
}

fn cmd_year_end_close(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    require_actor(actor)?;
    let db = open_existing(db_path)?;
    let year = arg(argv, "--year").ok_or_else(|| missing_arg("--year"))?;
    bukio::year_end::year_end_close(&db, &year, actor, dry_run)
}

// ── fx ─────────────────────────────────────────────────────────────────────

fn cmd_fx_set(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    require_actor(actor)?;
    let db = open_existing(db_path)?;
    let currency = arg(argv, "--currency").ok_or_else(|| missing_arg("--currency"))?;
    let date = arg(argv, "--date").unwrap_or_else(bukio::dates::today_iso);
    let rate = arg(argv, "--rate").ok_or_else(|| missing_arg("--rate"))?;
    let source = arg(argv, "--source").unwrap_or_else(|| "manual".into());
    let r = bukio::fx::set_fx_rate(&db, &currency, &date, &rate, &source, actor, dry_run)?;
    if dry_run {
        Ok(json!({ "rate": r, "dryRun": true }))
    } else {
        Ok(json!({ "rate": r }))
    }
}

fn cmd_fx_show(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let currency = arg(argv, "--currency").ok_or_else(|| missing_arg("--currency"))?;
    let limit: i64 = parse_limit(argv, 50)?;
    let rates = bukio::fx::list_fx_rates(&db, Some(&currency), limit)?;
    Ok(json!({ "currency": currency, "rates": rates }))
}

fn cmd_fx_list(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let limit: i64 = parse_limit(argv, 50)?;
    let rates = bukio::fx::list_fx_rates(&db, None, limit)?;
    Ok(json!({ "rates": rates }))
}

fn cmd_fx_fetch(argv: &[String]) -> Result<Value> {
    let currency = arg(argv, "--currency").ok_or_else(|| missing_arg("--currency"))?;
    let date = arg(argv, "--date").ok_or_else(|| missing_arg("--date"))?;
    let rate = bukio::fx::fetch_ecb_rate(&currency, &date)?;
    Ok(json!({ "currency": currency, "date": date, "rate": rate }))
}

// ── mcp ────────────────────────────────────────────────────────────────────

fn cmd_mcp(db_path: &str, actor: &str) -> Result<Value> {
    ensure_db_exists(db_path)?;
    bukio::mcp::run(db_path, actor)?;
    // MCP server handles all I/O — no extra output after run() returns
    std::process::exit(0);
}

// ── compliance ─────────────────────────────────────────────────────────────

fn cmd_compliance_status(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    // --year was accepted and silently ignored: every call reported the current
    // year, so asking for 2025 returned 2026's calendar (the JS CLI reads it)
    let year: i32 = match arg(argv, "--year") {
        None => bukio::dates::today_iso()[0..4].parse().unwrap_or(2026),
        Some(raw) => raw.parse().map_err(|_| {
            BukioError::new(
                "INVALID_YEAR",
                format!("--year must be a year, got '{raw}'"),
            )
        })?,
    };
    // the JS CLI wraps the engine value under `compliance`
    Ok(json!({
        "compliance": bukio::compliance::compliance_status(&db, year)?
    }))
}

fn cmd_compliance_mark(
    argv: &[String],
    db_path: &str,
    actor: &str,
    dry_run: bool,
) -> Result<Value> {
    require_actor(actor)?;
    let db = open_existing(db_path)?;
    let filing_type = arg(argv, "--type").ok_or_else(|| missing_arg("--type"))?;
    let period = arg(argv, "--period").ok_or_else(|| missing_arg("--period"))?;
    let date = arg(argv, "--date");
    bukio::compliance::mark_filed(&db, &filing_type, &period, date.as_deref(), actor, dry_run)
}

// ── import ─────────────────────────────────────────────────────────────────

fn cmd_import_opening_balances(
    argv: &[String],
    db_path: &str,
    actor: &str,
    dry_run: bool,
) -> Result<Value> {
    require_actor(actor)?;
    let db = open_existing(db_path)?;
    let file = arg(argv, "--file").ok_or_else(|| missing_arg("--file"))?;
    let text = std::fs::read_to_string(&file)
        .map_err(|e| BukioError::new("IO_ERROR", format!("cannot read {file}: {e}")))?;
    let date = arg(argv, "--date");
    bukio::import_mod::import_opening_balances(&db, &text, date.as_deref(), actor, dry_run)
}

fn cmd_import_journal(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    require_actor(actor)?;
    let db = open_existing(db_path)?;
    let file = arg(argv, "--file").ok_or_else(|| missing_arg("--file"))?;
    let text = std::fs::read_to_string(&file)
        .map_err(|e| BukioError::new("IO_ERROR", format!("cannot read {file}: {e}")))?;
    let create_missing = has_flag(argv, "--create-missing");
    bukio::import_mod::import_journal_csv(&db, &text, create_missing, actor, dry_run)
}

fn cmd_import_contacts(
    argv: &[String],
    db_path: &str,
    actor: &str,
    dry_run: bool,
) -> Result<Value> {
    require_actor(actor)?;
    let db = open_existing(db_path)?;
    let file = arg(argv, "--file").ok_or_else(|| missing_arg("--file"))?;
    let text = std::fs::read_to_string(&file)
        .map_err(|e| BukioError::new("IO_ERROR", format!("cannot read {file}: {e}")))?;
    bukio::import_mod::import_contacts(&db, &text, actor, dry_run)
}

// ── export ─────────────────────────────────────────────────────────────────

fn cmd_export_xaf(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    require_actor(actor)?;
    let db = open_existing(db_path)?;
    let profile = bukio::accounts::resolve_profile(&db)?;
    if profile["documents"]["auditFile"].as_str().is_none() {
        return Err(BukioError::new(
            "FORMAT_NOT_SUPPORTED",
            format!(
                "audit file export is not supported for country {}",
                profile["meta"]["country"].as_str().unwrap_or("?")
            ),
        ));
    }
    let year = arg(argv, "--year").unwrap_or_else(|| bukio::dates::today_iso()[0..4].to_string());
    let out = arg(argv, "--out").unwrap_or_else(|| format!("bukio-{year}.xaf"));
    bukio::export::export_xaf(&db, &year, &out, actor, dry_run)
}

// ── month-end ──────────────────────────────────────────────────────────────

fn cmd_month_end(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let period = arg(argv, "--period").ok_or_else(|| missing_arg("--period"))?;
    bukio::month_end::month_end(&db, &period)
}

// ── company ────────────────────────────────────────────────────────────────

fn cmd_company_show(db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let c = bukio::company::get_company(&db)?;
    Ok(json!({ "company": c }))
}

fn cmd_company_update(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    require_actor(actor)?;
    let db = open_existing(db_path)?;
    let fields = [
        ("--name", "name"),
        ("--country", "country"),
        ("--registration-id", "registration_id"),
        ("--tax-id", "tax_id"),
        ("--iban", "iban"),
        ("--address", "address"),
        ("--postal-code", "postal_code"),
        ("--city", "city"),
    ];
    let mut changes = Vec::new();
    for (opt, col) in fields {
        if let Some(v) = arg(argv, opt) {
            changes.push((col.to_string(), v));
        }
    }
    // Country is immutable after init — allow same value (case-insensitive)
    if let Some(idx) = changes.iter().position(|(col, _)| col == "country") {
        let cur = db
            .query_row("SELECT country FROM company WHERE id=1", [], |r| {
                r.get::<_, Option<String>>(0)
            })
            .ok()
            .flatten()
            .unwrap_or_default();
        let new_val = &changes[idx].1;
        if !new_val.eq_ignore_ascii_case(&cur) {
            return Err(BukioError::new("COUNTRY_IMMUTABLE", format!("country is immutable after init — company stays {cur} (re-init a new DB for another country)")));
        }
        // same value — drop from changes
        changes.remove(idx);
    }
    // Logo: --logo <file> sets it; --remove-logo clears it
    let logo_file = arg(argv, "--logo");
    let remove_logo = has_flag(argv, "--remove-logo");
    if logo_file.is_some() && remove_logo {
        return Err(BukioError::new(
            "INVALID_ARGS",
            "pass either --logo <file> or --remove-logo, not both",
        ));
    }
    let mut logo_bytes: Option<Vec<u8>> = None;
    let mut logo_mime: Option<String> = None;
    if let Some(path) = &logo_file {
        let (bytes, mime) = bukio::company::read_logo_file(path)?;
        logo_bytes = Some(bytes);
        logo_mime = Some(mime);
    } else if remove_logo {
        logo_bytes = Some(Vec::new()); // empty = clear
    }
    if changes.is_empty() && logo_file.is_none() && !remove_logo {
        return Err(BukioError::new("NOTHING_TO_UPDATE", "nothing to update — pass at least one of --name/--registration-id/--tax-id/--iban/--address/--postal-code/--city, or --logo/--remove-logo"));
    }
    if dry_run {
        return Ok(json!({
            "company": bukio::company::get_company(&db)?,
            "changes": serde_json::Map::from_iter(changes.iter().map(|(k,v)| (k.clone(), Value::String(v.clone())))),
            "dryRun": true,
        }));
    }
    let (updated, changes_map) =
        bukio::company::update_company(&db, &changes, logo_bytes, logo_mime.as_deref(), actor)?;
    Ok(json!({ "company": updated, "changes": changes_map }))
}

fn cmd_company_logo(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let (bytes, mime) = bukio::company::get_logo(&db)?;
    if let Some(path) = arg(argv, "--out") {
        std::fs::write(&path, &bytes)
            .map_err(|e| BukioError::new("IO_ERROR", format!("cannot write {path}: {e}")))?;
        Ok(json!({ "path": path, "mime": mime, "length": bytes.len() }))
    } else {
        std::io::stdout()
            .write_all(&bytes)
            .map_err(|e| BukioError::new("IO_ERROR", e.to_string()))?;
        Ok(json!({ "mime": mime, "length": bytes.len() }))
    }
}

// ── assets ─────────────────────────────────────────────────────────────────

fn cmd_asset_scheme_add(
    argv: &[String],
    db_path: &str,
    actor: &str,
    dry_run: bool,
) -> Result<Value> {
    require_actor(actor)?;
    let db = open_existing(db_path)?;
    let name = arg(argv, "--name").unwrap_or_else(|| "standard".into());
    let method = arg(argv, "--method").unwrap_or_else(|| "lineair".into());
    let life_months: i64 = parse_life_months(argv)?;
    let residual_bp: i64 = parse_i64(argv, "--residual-bp").unwrap_or(0);
    bukio::assets::create_scheme(
        &db,
        &name,
        &method,
        life_months,
        residual_bp,
        actor,
        dry_run,
    )
}

fn cmd_asset_scheme_list(db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let rows = bukio::assets::list_schemes(&db)?;
    Ok(json!({ "schemes": rows }))
}

fn cmd_asset_list(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let status = arg(argv, "--status");
    let rows = bukio::assets::list_assets(&db, status.as_deref())?;
    // the JS CLI projects these seven keys and flattens the scheme to its name
    let data: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r["id"], "name": r["name"], "category": r["category"],
                "status": r["status"], "purchase_date": r["purchase_date"],
                "purchase_price_cents": r["purchase_price_cents"],
                "scheme": r["scheme"]["name"],
            })
        })
        .collect();
    Ok(json!({ "assets": data }))
}

fn cmd_asset_show(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let id: i64 = parse_i64(argv, "--id").ok_or_else(|| missing_arg("--id"))?;
    let asset = bukio::assets::get_asset(&db, id)?
        .ok_or_else(|| BukioError::new("ASSET_NOT_FOUND", format!("asset {id} does not exist")))?;
    let mut stmt = db
        .prepare("SELECT period, amount_cents, entry_id FROM asset_depreciation_runs WHERE asset_id = ?1 ORDER BY period")
        .map_err(|e| BukioError::new("DB_ERROR", e.to_string()))?;
    let runs: Vec<Value> = stmt
        .query_map([id], |r| {
            Ok(json!({
                "period": r.get::<_, String>(0)?,
                "amount_cents": r.get::<_, i64>(1)?,
                "entry_id": r.get::<_, i64>(2)?,
            }))
        })
        .map_err(|e| BukioError::new("DB_ERROR", e.to_string()))?
        .filter_map(|r| r.ok())
        .collect();
    Ok(json!({ "asset": asset, "runs": runs }))
}

fn cmd_asset_run(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    require_actor(actor)?;
    let db = open_existing(db_path)?;
    // the engine takes a period; --as-of is a full date, so only its month
    // applies (JS: `period ?? asOf.slice(0,7) ?? today`)
    let period = match arg(argv, "--period") {
        Some(p) => p,
        None => match arg(argv, "--as-of") {
            Some(d) => {
                if !bukio::assets::valid_date(&d) {
                    return Err(BukioError::new(
                        "INVALID_DATE",
                        format!("as-of '{d}' must be yyyy-mm-dd"),
                    ));
                }
                d[..7].to_string()
            }
            None => bukio::dates::today_iso()[..7].to_string(),
        },
    };
    let result = bukio::assets::run_due(&db, &period, actor, dry_run)?;
    if dry_run {
        // the JS dry-run output carries the plan only (no empty booked list)
        return Ok(json!({"plan": result["plan"], "dryRun": true}));
    }
    Ok(result)
}

fn cmd_asset_add(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    require_actor(actor)?;
    let db = open_existing(db_path)?;
    let name = arg(argv, "--name").ok_or_else(|| missing_arg("--name"))?;
    let purchase_date =
        arg(argv, "--purchase-date").ok_or_else(|| missing_arg("--purchase-date"))?;
    let purchase_price_str =
        arg(argv, "--purchase-price").ok_or_else(|| missing_arg("--purchase-price"))?;
    let purchase_price_cents = bukio::money::parse_amount(&purchase_price_str)?;
    let dep_start =
        arg(argv, "--depreciation-start").ok_or_else(|| missing_arg("--depreciation-start"))?;
    let recognition =
        arg(argv, "--recognition-date").ok_or_else(|| missing_arg("--recognition-date"))?;
    let asset_account = arg(argv, "--asset-account").unwrap_or_else(|| "1800".into());
    let expense_account = arg(argv, "--expense-account").unwrap_or_else(|| "4600".into());
    let cum_dep_str = arg(argv, "--cum-dep").unwrap_or_else(|| "0".into());
    let cum_dep_cents = bukio::money::parse_amount(&cum_dep_str)?;
    let residual_str = arg(argv, "--residual");
    let residual_cents = residual_str
        .as_ref()
        .map(|s| bukio::money::parse_amount(s))
        .transpose()?;
    let scheme_id = parse_i64(argv, "--scheme");
    let method = arg(argv, "--method");
    let life_months = parse_i64(argv, "--life-months");
    let residual_bp = parse_i64(argv, "--residual-bp");
    let entry_id = parse_i64(argv, "--entry-id");
    bukio::assets::create_asset(
        &db,
        &name,
        arg(argv, "--category").as_deref(),
        arg(argv, "--serial").as_deref(),
        scheme_id,
        method.as_deref(),
        life_months,
        residual_bp,
        residual_cents,
        &purchase_date,
        purchase_price_cents,
        &dep_start,
        &recognition,
        cum_dep_cents,
        &asset_account,
        arg(argv, "--cum-dep-account").as_deref(),
        &expense_account,
        entry_id,
        arg(argv, "--note").as_deref(),
        actor,
        dry_run,
    )
}

fn cmd_asset_register(argv: &[String], db_path: &str, actor: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let as_of = arg(argv, "--as-of");
    let format = arg(argv, "--format").unwrap_or_else(|| "json".into());
    let data = bukio::assets::register(&db, as_of.as_deref(), actor)?;
    match format.as_str() {
        "csv" => {
            let assets = data["assets"].as_array().cloned().unwrap_or_default();
            let totals = data["totals"].clone();
            let header = "id,name,category,status,purchase,purchase price,cum. deprec.,book value";
            let mut lines = vec![header.to_string()];
            for a in &assets {
                lines.push(format!(
                    "{},{},{},{},{},{},{},{}",
                    a["id"],
                    a["name"],
                    a.get("category").unwrap_or(&serde_json::Value::Null),
                    a["status"],
                    a["purchase_date"],
                    bukio::money::format_amount(a["purchase_price_cents"].as_i64().unwrap_or(0)),
                    bukio::money::format_amount(a["total_cum_dep_cents"].as_i64().unwrap_or(0)),
                    bukio::money::format_amount(a["book_value_cents"].as_i64().unwrap_or(0)),
                ));
            }
            lines.push(format!(
                "TOTAL,,,,,,{},{}",
                bukio::money::format_amount(totals["total_cum_dep_cents"].as_i64().unwrap_or(0)),
                bukio::money::format_amount(totals["book_value_cents"].as_i64().unwrap_or(0)),
            ));
            // CSV output is raw text, not wrapped in {ok,data}
            println!("{}", lines.join("\n"));
            std::process::exit(0);
        }
        "json" => {
            // --format json is the declared default and must emit JSON even
            // without the global --json flag (the JS CLI prints and returns here)
            ok(data);
            std::process::exit(0);
        }
        _ => Ok(data),
    }
}

fn cmd_asset_dispose(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    require_actor(actor)?;
    let db = open_existing(db_path)?;
    let id: i64 = parse_i64(argv, "--id").ok_or_else(|| missing_arg("--id"))?;
    let date = arg(argv, "--date").ok_or_else(|| missing_arg("--date"))?;
    let proceeds_str = arg(argv, "--proceeds").unwrap_or_else(|| "0".into());
    let proceeds_cents = bukio::money::parse_amount(&proceeds_str)?;
    bukio::assets::dispose_asset(
        &db,
        id,
        &date,
        proceeds_cents,
        arg(argv, "--bank-account").as_deref(),
        arg(argv, "--result-account").as_deref(),
        arg(argv, "--note").as_deref(),
        actor,
        dry_run,
    )
}

fn cmd_asset_pause(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    require_actor(actor)?;
    let db = open_existing(db_path)?;
    let id: i64 = parse_i64(argv, "--id").ok_or_else(|| missing_arg("--id"))?;
    bukio::assets::pause_asset(&db, id, actor, dry_run)
}

fn cmd_asset_resume(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    require_actor(actor)?;
    let db = open_existing(db_path)?;
    let id: i64 = parse_i64(argv, "--id").ok_or_else(|| missing_arg("--id"))?;
    bukio::assets::resume_asset(&db, id, actor, dry_run)
}

// ── payments ───────────────────────────────────────────────────────────────

fn cmd_payable_add(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    require_actor(actor)?;
    let db = open_existing(db_path)?;
    let contact_ref = arg(argv, "--contact").ok_or_else(|| missing_arg("--contact"))?;
    let invoice_ref = arg(argv, "--ref")
        .or_else(|| arg(argv, "--invoice-ref"))
        .unwrap_or_default();
    let date = arg(argv, "--date").unwrap_or_else(bukio::dates::today_iso);
    let due = arg(argv, "--due"); // absent => NULL, like the JS (no default)
    let amount_str = arg(argv, "--amount").ok_or_else(|| missing_arg("--amount"))?;
    let amount_cents = bukio::money::parse_amount(&amount_str)?;
    let method_raw = arg(argv, "--method").unwrap_or_else(|| "transfer".into());
    let method = if method_raw == "direct-debit" {
        "direct_debit".to_string()
    } else {
        method_raw
    };
    let entry_id = parse_i64(argv, "--entry-id");
    bukio::payments::add_payable(
        &db,
        &contact_ref,
        &invoice_ref,
        &date,
        due.as_deref(),
        amount_cents,
        &method,
        actor,
        dry_run,
    )
}

fn cmd_payable_list(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let status = arg(argv, "--status");
    let method = arg(argv, "--method");
    // the JS CLI maps ONLY 'direct-debit'/'transfer' and passes null otherwise,
    // so any other value silently disables the filter — match that
    let method_filter = match method.as_deref() {
        Some("direct-debit") => Some("direct_debit"),
        Some("transfer") => Some("transfer"),
        _ => None,
    };
    let rows = bukio::payments::list_payables(&db, status.as_deref(), method_filter, None)?;
    Ok(json!({ "payables": rows }))
}

fn cmd_payable_pay(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    require_actor(actor)?;
    let db = open_existing(db_path)?;
    let id: i64 = parse_i64(argv, "--id").ok_or_else(|| missing_arg("--id"))?;
    bukio::payments::mark_payable_paid(&db, id, actor, dry_run)
}

fn cmd_mandate_add(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    require_actor(actor)?;
    let db = open_existing(db_path)?;
    let contact_id: i64 = parse_i64(argv, "--contact").ok_or_else(|| missing_arg("--contact"))?;
    let mandate_ref = arg(argv, "--ref").unwrap_or_else(|| format!("mandate-{contact_id}"));
    let date = arg(argv, "--date");
    let scheme = arg(argv, "--type").unwrap_or_else(|| "core".into());
    bukio::payments::add_mandate(
        &db,
        contact_id,
        &mandate_ref,
        date.as_deref(),
        &scheme,
        actor,
        dry_run,
    )
}

fn cmd_mandate_list(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let contact_id = parse_i64(argv, "--contact");
    let rows = bukio::payments::list_mandates(&db, contact_id)?;
    Ok(json!({ "mandates": rows }))
}

fn cmd_mandate_remove(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    require_actor(actor)?;
    let db = open_existing(db_path)?;
    let id: i64 = parse_i64(argv, "--id").ok_or_else(|| missing_arg("--id"))?;
    bukio::payments::remove_mandate(&db, id, actor, dry_run)
}

fn cmd_batch_create(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    require_actor(actor)?;
    let db = open_existing(db_path)?;
    let date = arg(argv, "--date");
    let from_iban = arg(argv, "--from-iban");
    let kind_raw = arg(argv, "--type").unwrap_or_else(|| "transfer".into());
    let kind = if kind_raw == "direct-debit" {
        "direct_debit".to_string()
    } else {
        kind_raw
    };
    let from_invoices = has_flag(argv, "--from-invoices");
    let mut payable_ids: Vec<i64> = arg(argv, "--payable")
        .map(|s| s.split(',').filter_map(|v| v.trim().parse().ok()).collect())
        .unwrap_or_default();
    // --from-invoices: gather all unpaid payables matching the batch kind
    let eligible: Vec<i64> = {
        let method_filter = &kind;
        db.prepare("SELECT id FROM payables WHERE status = 'unpaid' AND payment_method = ?1")
            .map_err(|e| BukioError::new("DB_ERROR", e.to_string()))?
            .query_map([method_filter], |r| r.get(0))
            .map_err(|e| BukioError::new("DB_ERROR", e.to_string()))?
            .filter_map(|r| r.ok())
            .collect()
    };
    if from_invoices && payable_ids.is_empty() {
        payable_ids = eligible.clone();
    }
    // Validate explicit --payable IDs against eligible list
    if !payable_ids.is_empty() {
        for &pid in &payable_ids {
            if !eligible.contains(&pid) {
                return Err(BukioError::new(
                    "PAYABLE_NOT_ELIGIBLE",
                    format!("payable {pid} is not unpaid+{kind} (already batched or wrong payment term)"),
                ));
            }
        }
    }
    // "CONTACT:AMOUNT[:REF];..." (the JS CLI's parseLinesSpec) — the old code
    // passed the raw text through and the engine dropped every line
    let lines: Vec<Value> = arg(argv, "--lines")
        .map(|s| {
            s.split(';')
                .filter(|l| !l.trim().is_empty())
                .map(|l| {
                    let parts: Vec<&str> = l.trim().split(':').collect();
                    let contact = parts.first().copied().unwrap_or("");
                    let amount =
                        bukio::import_mod::parse_import_amount(parts.get(1).copied().unwrap_or(""))
                            .unwrap_or(0);
                    let reference = if parts.len() > 2 {
                        Some(parts[2..].join(":"))
                    } else {
                        None
                    };
                    json!({"contact": contact, "amountCents": amount, "reference": reference})
                })
                .collect()
        })
        .unwrap_or_default();
    if let Some(csv_file) = arg(argv, "--csv") {
        let csv_text = std::fs::read_to_string(&csv_file)
            .map_err(|e| BukioError::new("FILE_ERROR", format!("cannot read {csv_file}: {e}")))?;
        return bukio::payments::create_payment_batch_from_csv(
            &db,
            &csv_text,
            date.as_deref(),
            from_iban.as_deref(),
            actor,
            dry_run,
        );
    }
    bukio::payments::create_payment_batch(
        &db,
        date.as_deref(),
        from_iban.as_deref(),
        &lines,
        &payable_ids,
        &kind,
        actor,
        dry_run,
    )
}

fn cmd_batch_list(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let status = arg(argv, "--status");
    let rows = bukio::payments::list_payment_batches(&db, status.as_deref())?;
    Ok(json!({ "batches": rows }))
}

fn cmd_batch_show(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let id: i64 = parse_i64(argv, "--id").ok_or_else(|| missing_arg("--id"))?;
    let batch = bukio::payments::get_payment_batch(&db, id)?;
    Ok(batch)
}

fn cmd_batch_delete(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    require_actor(actor)?;
    let db = open_existing(db_path)?;
    let id: i64 = parse_i64(argv, "--id").ok_or_else(|| missing_arg("--id"))?;
    bukio::payments::delete_payment_batch(&db, id, actor, dry_run)
}

fn cmd_batch_export(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    let db = open_existing(db_path)?;
    let id: i64 = parse_i64(argv, "--id").ok_or_else(|| missing_arg("--id"))?;
    // One code path owns the SEPA build + status/audit transition (JS parity:
    // the CLI calls exportPaymentBatch) — a second hand-rolled builder here
    // silently skipped the status update and the msg_id/file_hash bookkeeping.
    let schema = arg(argv, "--schema");
    let mut result =
        bukio::payments::export_payment_batch(&db, id, actor, dry_run, schema.as_deref())?;
    if let Some(path) = arg(argv, "--out") {
        let xml = result["xml"].as_str().unwrap_or("");
        std::fs::write(&path, xml)
            .map_err(|e| BukioError::new("IO_ERROR", format!("cannot write {path}: {e}")))?;
        // main() prints "wrote <path>" when data.path is present
        if let Value::Object(m) = &mut result {
            m.insert("path".to_string(), json!(path));
        }
    }
    Ok(result)
}

// ── item ───────────────────────────────────────────────────────────────────

fn cmd_item_add(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    require_actor(actor)?;
    let db = open_existing(db_path)?;
    let name = arg(argv, "--name").ok_or_else(|| missing_arg("--name"))?;
    let description = arg(argv, "--description");
    let unit = arg(argv, "--unit").unwrap_or_else(|| "unit".into());
    let price_str = arg(argv, "--price").ok_or_else(|| missing_arg("--price"))?;
    let unit_price_cents = bukio::money::parse_amount(&price_str)?;
    let vat_code = arg(argv, "--vat");
    let gl_account = arg(argv, "--gl");
    let item = bukio::items::create_item(
        &db,
        &name,
        description.as_deref(),
        &unit,
        unit_price_cents,
        vat_code.as_deref(),
        gl_account.as_deref(),
        actor,
        dry_run,
    )?;
    Ok(json!({ "item": item }))
}

fn cmd_item_list(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let active_only = !has_flag(argv, "--all");
    let rows = bukio::items::list_items(&db, active_only)?;
    Ok(json!({ "items": rows }))
}

fn cmd_item_show(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let id: i64 = parse_i64(argv, "--id").ok_or_else(|| missing_arg("--id"))?;
    let item = bukio::items::get_item(&db, id)?
        .ok_or_else(|| BukioError::new("NOT_FOUND", format!("item {id} not found")))?;
    Ok(item)
}

fn cmd_item_update(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    require_actor(actor)?;
    let db = open_existing(db_path)?;
    let id: i64 = parse_i64(argv, "--id").ok_or_else(|| missing_arg("--id"))?;
    let name = arg(argv, "--name");
    let description = arg(argv, "--description");
    let unit = arg(argv, "--unit");
    let price_str = arg(argv, "--price");
    let unit_price_cents = price_str
        .as_ref()
        .map(|p| bukio::money::parse_amount(p))
        .transpose()?;
    let vat_code = arg(argv, "--vat");
    let deactivate = has_flag(argv, "--deactivate");
    bukio::items::update_item(
        &db,
        id,
        name.as_deref(),
        description,
        unit.as_deref(),
        unit_price_cents,
        vat_code,
        None,
        deactivate,
        actor,
        dry_run,
    )
}

fn cmd_update(argv: &[String], db_path: &str, actor: &str) -> Result<Value> {
    let repo = arg(argv, "--repo").unwrap_or_else(|| ".".into());
    let yes = has_flag(argv, "--yes");
    let dry_run = has_flag(argv, "--dry-run");
    let trust_remote = has_flag(argv, "--trust-remote");
    let git = |args: &[&str]| -> Result<String> {
        let out = std::process::Command::new("git")
            .args(["-C", &repo])
            .args(args)
            .output()
            .map_err(|e| BukioError::new("GIT_ERROR", format!("git failed: {e}")))?;
        if !out.status.success() {
            return Err(BukioError::new(
                "GIT_ERROR",
                format!(
                    "git {} failed: {}",
                    args.join(" "),
                    String::from_utf8_lossy(&out.stderr).trim()
                ),
            ));
        }
        // trailing-only trim: porcelain output KEEPS its leading space column
        // (" M file"); a full trim would eat it and slice(3) would then drop the
        // filename's first character on the first line
        Ok(String::from_utf8_lossy(&out.stdout).trim_end().to_string())
    };
    // A bare git dir is not a clone; the fixture's origin.git is bare
    let git_dir_ok = std::process::Command::new("git")
        .args(["-C", &repo, "rev-parse", "--git-dir"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !git_dir_ok {
        return Err(BukioError::new(
            "UPDATE_NOT_A_CLONE",
            format!("'{repo}' is not a git clone — `bukio update` works on a cloned installation; an npm -g install must be updated with `npm update -g bukio-cli`"),
        ));
    }
    let url = git(&["remote", "get-url", "origin"])?;
    // ANCHORED like the JS's regex — a substring test would accept
    // https://evil.com/github.com:erikvankempen/bukio-cli.git
    let official = regex::Regex::new(
        r"(?i)^(?:[a-z][a-z0-9+.\-]*://)?(?:[^@\s/]+@)?github\.com[:/]erikvankempen/bukio-cli(?:\.git)?$",
    )
    .expect("static regex");
    if !official.is_match(&url) && !trust_remote {
        return Err(BukioError::new(
            "UPDATE_WRONG_REMOTE",
            format!(
                "remote URL {url} is not the official bukio-cli repository — refusing to update from it (pass --trust-remote to override)"
            ),
        ));
    }
    git(&["fetch", "origin", "main"])?;
    let current_sha = git(&["rev-parse", "HEAD"])?;
    let target_sha = git(&["rev-parse", "origin/main"]).map_err(|_| {
        BukioError::new(
            "UPDATE_NO_REMOTE_BRANCH",
            "no origin/main ref after fetching origin",
        )
    })?;
    let incoming: Vec<String> =
        git(&["log", "--oneline", &format!("{current_sha}..{target_sha}")])?
            .split('\n')
            .filter(|l| !l.is_empty())
            .map(String::from)
            .collect();
    let local_commits: Vec<String> =
        git(&["log", "--oneline", &format!("{target_sha}..{current_sha}")])?
            .split('\n')
            .filter(|l| !l.is_empty())
            .map(String::from)
            .collect();
    let status = git(&["status", "--porcelain"])?;
    let modified_files: Vec<String> = status
        .split('\n')
        .filter(|l| !l.is_empty() && !l.starts_with("??"))
        .map(|l| l.chars().skip(3).collect())
        .collect();
    let untracked_count = status.split('\n').filter(|l| l.starts_with("??")).count();
    let package_json_changed = !git(&[
        "diff",
        "--name-only",
        &format!("{current_sha}..{target_sha}"),
        "--",
        "package.json",
    ])?
    .is_empty();
    let up_to_date = current_sha == target_sha;
    let warning = if !modified_files.is_empty() || !local_commits.is_empty() {
        Some(format!(
            "OVERWRITES LOCAL CUSTOMIZATIONS: {} modified file(s) and {} local commit(s) will be lost by resetting to origin/main. Untracked files are kept.",
            modified_files.len(),
            local_commits.len()
        ))
    } else {
        None
    };
    let current_version = std::fs::read_to_string(format!("{repo}/package.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| v.get("version").and_then(|x| x.as_str()).map(String::from));

    if dry_run {
        return Ok(json!({
            "action": "update", "dryRun": true,
            "repo_path": repo, "remote": "origin", "branch": "main", "remote_url": url,
            "current_sha": current_sha, "target_sha": target_sha,
            "current_version": current_version,
            "incoming_count": incoming.len(), "incoming": incoming,
            "local_commits": local_commits, "modified_files": modified_files,
            "untracked_count": untracked_count,
            "package_json_changed": package_json_changed,
            "up_to_date": up_to_date, "warning": warning,
        }));
    }
    if up_to_date {
        return Ok(json!({
            "action": "update", "updated": false,
            "repo_path": repo, "remote": "origin", "branch": "main", "remote_url": url,
            "current_sha": current_sha, "target_sha": target_sha,
            "current_version": current_version,
            "incoming_count": incoming.len(), "incoming": incoming,
            "local_commits": local_commits, "modified_files": modified_files,
            "untracked_count": untracked_count,
            "package_json_changed": package_json_changed,
            "up_to_date": true, "warning": warning,
        }));
    }
    if !yes {
        return Err(BukioError::new(
            "UPDATE_CONFIRM_REQUIRED",
            "refusing to reset to origin/main without confirmation — pass --yes (this would overwrite local customizations; run --dry-run to see the plan)",
        ));
    }
    let from_sha = current_sha;
    git(&["reset", "--hard", "origin/main"])?;
    let mut deps_installed = false;
    let mut deps_error: Option<String> = None;
    if package_json_changed {
        let dep = std::process::Command::new("npm")
            .args(["install"])
            .current_dir(&repo)
            .output();
        match dep {
            Ok(o) if o.status.success() => deps_installed = true,
            Ok(o) => deps_error = Some(String::from_utf8_lossy(&o.stderr).trim().to_string()),
            Err(e) => deps_error = Some(e.to_string()),
        }
    }
    let to_sha = git(&["rev-parse", "HEAD"])?;
    // mirror the JS: record an audit row when a company db is present
    if let Ok(db) = open_existing(db_path) {
        let _ = bukio::audit::record(
            &db,
            bukio::audit::RecordArgs {
                actor,
                action: "update",
                command: Some("update"),
                args: Some(json!({ "commits": incoming.len() })),
                outcome: "ok",
                entry_ids: vec![],
            },
        );
    }
    let version_after = std::fs::read_to_string(format!("{repo}/package.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| v.get("version").and_then(|x| x.as_str()).map(String::from));
    Ok(json!({
        "action": "update", "updated": true,
        "from_sha": from_sha, "to_sha": to_sha,
        "commits_applied": incoming.len(), "version_after": version_after,
        "deps_installed": deps_installed, "deps_error": deps_error,
        "repo_path": repo, "branch": "main",
    }))
}

// ── attach ─────────────────────────────────────────────────────────────────

fn cmd_attach_add(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    require_actor(actor)?;
    let db = open_existing(db_path)?;
    let file = arg(argv, "--file").ok_or_else(|| missing_arg("--file"))?;
    let store = arg(argv, "--store").unwrap_or_else(|| "db".into());
    let note = arg(argv, "--note");
    let inv_id = parse_i64(argv, "--invoice");
    let entry_id = parse_i64(argv, "--entry");
    let (kind, ref_id) = match (inv_id, entry_id) {
        (Some(_), Some(_)) => {
            return Err(BukioError::new(
                "REF_REQUIRED",
                "pass exactly one of --invoice <id> or --entry <id>",
            ));
        }
        (Some(i), None) => ("invoice", i),
        (None, Some(e)) => ("entry", e),
        (None, None) => {
            return Err(BukioError::new(
                "REF_REQUIRED",
                "pass exactly one of --invoice <id> or --entry <id>",
            ));
        }
    };
    bukio::attachments::add_attachment(
        &db,
        kind,
        ref_id,
        &file,
        note.as_deref(),
        &store,
        actor,
        dry_run,
    )
}

fn cmd_attach_list(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let (kind, ref_id) = if let Some(id) = parse_i64(argv, "--invoice") {
        ("invoice", id)
    } else if let Some(id) = parse_i64(argv, "--entry") {
        ("entry", id)
    } else {
        return Err(missing_arg("--invoice or --entry"));
    };
    let rows = bukio::attachments::list_attachments(&db, kind, ref_id)?;
    Ok(json!({ "attachments": rows }))
}

fn cmd_attach_show(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let id: i64 = parse_i64(argv, "--id").ok_or_else(|| missing_arg("--id"))?;
    if let Some(out) = arg(argv, "--out") {
        let force = has_flag(argv, "--force");
        return bukio::attachments::extract_attachment(&db, id, &out, force);
    }
    bukio::attachments::get_attachment(&db, id)
}

fn cmd_attach_remove(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    require_actor(actor)?;
    let db = open_existing(db_path)?;
    let id: i64 = parse_i64(argv, "--id").ok_or_else(|| missing_arg("--id"))?;
    bukio::attachments::remove_attachment(&db, id, actor, dry_run)
}

// ── actor ──────────────────────────────────────────────────────────────────

fn cmd_actor_keygen(argv: &[String], actor: &str, dry_run: bool) -> Result<Value> {
    require_actor(actor)?;
    let force = has_flag(argv, "--force");
    bukio::actor_cli::cmd_keygen(actor, force, dry_run)
}

fn cmd_actor_register(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    require_actor(actor)?;
    bukio::actor_cli::cmd_register(db_path, actor, dry_run)
}

fn cmd_actor_list(db_path: &str) -> Result<Value> {
    bukio::actor_cli::cmd_list(db_path)
}

fn cmd_actor_revoke(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    require_actor(actor)?;
    let reason = arg(argv, "--reason");
    // --target <who> kills another actor; without it the caller revokes itself
    let target = arg(argv, "--target").unwrap_or_else(|| actor.to_string());
    bukio::actor_cli::cmd_revoke(db_path, &target, actor, reason.as_deref(), dry_run)
}

fn cmd_actor_enforce(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    require_actor(actor)?;
    let on = has_flag(argv, "--on");
    let off = has_flag(argv, "--off");
    if !on && !off {
        return Err(BukioError::new("INVALID_ENFORCE", "pass --on or --off"));
    }
    bukio::actor_cli::cmd_enforce(db_path, actor, on, dry_run)
}

fn cmd_actor_unlock(argv: &[String], actor: &str) -> Result<Value> {
    require_actor(actor)?;
    let ttl = parse_i64(argv, "--ttl-hours").map(|v| v as u64);
    bukio::actor_cli::cmd_unlock(actor, ttl)
}

fn cmd_actor_lock(argv: &[String], actor: &str) -> Result<Value> {
    require_actor(actor)?;
    bukio::actor_cli::cmd_lock(actor)
}

fn cmd_actor_authz(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    let on = has_flag(argv, "--on");
    let off = has_flag(argv, "--off");
    if on && off {
        return Err(BukioError::new(
            "INVALID_AUTHZ",
            "pass exactly one of --on or --off",
        ));
    }
    if !on && !off {
        return Err(BukioError::new(
            "INVALID_AUTHZ",
            "pass exactly one of --on or --off",
        ));
    }
    // the resolved --actor is the flipper (D3 grants THEM owner) — never the
    // env var, and never an empty string
    require_actor(actor)?;
    bukio::actor_cli::cmd_authz(db_path, actor, on, dry_run)
}

fn cmd_actor_roles(argv: &[String], db_path: &str, actor: &str) -> Result<Value> {
    require_actor(actor)?;
    bukio::actor_cli::cmd_roles(db_path, actor, arg(argv, "--for").as_deref())
}

fn cmd_actor_grant(argv: &[String], db_path: &str, actor: &str) -> Result<Value> {
    require_actor(actor)?;
    let role = positional_after(argv, "grant").ok_or_else(|| missing_arg("role"))?;
    let for_who = arg(argv, "--for").ok_or_else(|| missing_arg("--for"))?;
    bukio::actor_cli::cmd_grant(db_path, &for_who, &role, actor)
}

fn cmd_actor_revoke_role(argv: &[String], db_path: &str, actor: &str) -> Result<Value> {
    require_actor(actor)?;
    let role = positional_after(argv, "revoke-role")
        .or_else(|| positional_after(argv, "revoke"))
        .ok_or_else(|| missing_arg("role"))?;
    let for_who = arg(argv, "--for").ok_or_else(|| missing_arg("--for"))?;
    bukio::actor_cli::cmd_revoke_role(db_path, &for_who, &role, actor)
}

fn cmd_actor_can(argv: &[String], db_path: &str, actor: &str) -> Result<Value> {
    require_actor(actor)?;
    let action = positional_after(argv, "can").ok_or_else(|| missing_arg("command"))?;
    let who = arg(argv, "--for").unwrap_or_else(|| actor.to_string());
    bukio::actor_cli::cmd_can(db_path, actor, &who, &action)
}

fn cmd_actor_verify_key(db_path: &str, actor: &str) -> Result<Value> {
    require_actor(actor)?;
    bukio::actor_cli::cmd_verify_actor(db_path, actor)
}

fn cmd_actor_verify(db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    bukio::audit::verify_trail(&db, None, None)
}

fn cmd_actor_who_can(argv: &[String], db_path: &str, actor: &str) -> Result<Value> {
    require_actor(actor)?;
    let db = open_existing(db_path)?;
    let action = positional_after(argv, "who-can").ok_or_else(|| missing_arg("command"))?;
    let tokens: Vec<&str> = action.split_whitespace().collect();
    let path = tokens
        .iter()
        .filter(|t| !t.starts_with('-'))
        .copied()
        .collect::<Vec<&str>>()
        .join(" ");
    let post = tokens.iter().any(|t| *t == "--post");
    let capability = bukio::authz::capability_of(&path, post);
    // every actor that holds a role, plus every enrolled actor (JS parity)
    let mut names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for sql in [
        "SELECT DISTINCT actor FROM actor_roles",
        "SELECT DISTINCT actor FROM actor_keys",
    ] {
        if let Ok(mut stmt) = db.prepare(sql) {
            if let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(0)) {
                for name in rows.flatten() {
                    names.insert(name);
                }
            }
        }
    }
    let actors: Vec<Value> = names
        .iter()
        .map(|a| {
            let roles = bukio::actor::get_roles(&db, a);
            let allowed = capability.map_or(false, |c| bukio::authz::can_act(&db, a, c));
            json!({ "actor": a, "roles": roles, "allowed": allowed })
        })
        .collect();
    Ok(json!({ "command": action, "capability": capability, "actors": actors }))
}

/// Get the positional argument that comes right after a given command token.
fn positional_after(argv: &[String], after: &str) -> Option<String> {
    let mut found = false;
    for a in argv {
        if found && !a.starts_with('-') {
            return Some(a.clone());
        }
        if a == after {
            found = true;
        }
    }
    None
}

// ── server ─────────────────────────────────────────────────────────────────

fn cmd_server_start(argv: &[String], db_path: &str) -> Result<Value> {
    let listen = arg(argv, "--listen").unwrap_or_else(|| "127.0.0.1:8787".into());
    let serve_db = arg(argv, "--serve-db").unwrap_or_else(|| db_path.to_string());
    let parts: Vec<&str> = listen.splitn(2, ':').collect();
    let host = parts.first().unwrap_or(&"127.0.0.1");
    let port: u16 = parts.get(1).and_then(|p| p.parse().ok()).unwrap_or(8787);
    bukio::server::cmd_server_start(&serve_db, port, host)?;
    Ok(json!({ "status": "listening", "address": listen }))
}

fn cmd_server_token(argv: &[String], actor: &str) -> Result<Value> {
    require_actor(actor)?;
    let ttl_raw = arg(argv, "--ttl-hours").unwrap_or_else(|| "24".into());
    let ttl: u64 = ttl_raw.parse().map_err(|_| {
        BukioError::new(
            "INVALID_TTL",
            format!("--ttl-hours must be a positive number of hours, got '{ttl_raw}'"),
        )
    })?;
    if ttl == 0 {
        return Err(BukioError::new(
            "INVALID_TTL",
            "--ttl-hours must be a positive number of hours",
        ));
    }
    let target_actor = positional_after(argv, "token").ok_or_else(|| missing_arg("actor"))?;
    let token = bukio::server::mint_enrol_token(&target_actor, ttl)?;
    Ok(json!({ "token": token, "actor": target_actor, "ttl_hours": ttl }))
}

// ── remote client (--server) ──────────────────────────────────────
const REMOTE_LOCAL_ONLY_CMDS: &[&str] = &[
    "server start",
    "server token",
    "mcp",
    "init",
    "update",
    "actor keygen",
    "actor unlock",
    "actor lock",
];

fn remote_key_file(actor: &str) -> std::path::PathBuf {
    let cfg = std::env::var("BUKIO_CONFIG_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            std::path::PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()))
                .join(".bukio")
        });
    cfg.join("keys")
        .join(format!("{}.key", actor.replace(':', "-")))
}

/// Output a JSON document and exit with the right code (used by the remote
/// client to replay server answers verbatim).
fn remote_exit_json(body: &serde_json::Value, code: i32) -> ! {
    println!("{}", serde_json::to_string_pretty(body).unwrap_or_default());
    std::process::exit(code);
}

fn remote_client_mode(server: &str, positional: &[&str], argv: &[String], actor: &str) {
    let json_mode = has_flag(argv, "--json");
    let cmd = positional.join(" ");
    // LOCAL_ONLY matches the command PATH (JS: commandPathOf), not the full
    // positional line — `server token agent:x` is still `server token`.
    let is_local_only = REMOTE_LOCAL_ONLY_CMDS
        .iter()
        .any(|c| cmd == *c || cmd.starts_with(&format!("{c} ")));
    if is_local_only {
        let e = json!({
            "ok": false,
            "error": {"code": "LOCAL_ONLY",
                      "message": format!("'{cmd}' cannot run with --server — it is a local/operator command")}
        });
        remote_exit_json(&e, 1);
    }
    let base = server.trim_end_matches('/');
    if actor.is_empty() {
        let e = json!({"ok": false, "error": {"code": "ACTOR_REQUIRED",
            "message": "a named actor is required (--actor <role>:<name> or BUKIO_ACTOR)"}});
        remote_exit_json(&e, 1);
    }

    // `actor register --server` → /register (key never leaves the client)
    if cmd == "actor register" {
        let token = match arg(argv, "--token") {
            Some(t) if !t.is_empty() => t,
            _ => {
                let e = json!({"ok": false, "error": {"code": "TOKEN_REQUIRED",
                    "message": "remote registration needs --token <t> — mint one with 'bukio server token <actor>' on the server machine"}});
                remote_exit_json(&e, 1);
            }
        };
        let key_file = remote_key_file(actor);
        if !key_file.exists() {
            let e = json!({"ok": false, "error": {"code": "KEY_NOT_FOUND",
                "message": format!("no key file for {actor} at {} — run 'bukio actor keygen' first", key_file.display())}});
            remote_exit_json(&e, 1);
        }
        let pem = std::fs::read_to_string(&key_file).unwrap_or_default();
        let public_pem = match bukio::sign::public_key_from_private(&pem, None) {
            Ok(p) => p,
            Err(_) => {
                let e = json!({"ok": false, "error": {"code": "PASSPHRASE_INVALID",
                    "message": format!("could not read the key for {actor} — wrong passphrase or corrupt key file")}});
                remote_exit_json(&e, 1);
            }
        };
        let keyid = bukio::sign::keyid_of(&public_pem).unwrap_or_default();
        let dry_run = has_flag(argv, "--dry-run");
        if dry_run {
            let d = json!({"actor": actor, "keyid": keyid, "server": base, "dryRun": true});
            if json_mode {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({"ok": true, "data": d})).unwrap()
                );
            } else {
                println!("plan: register {actor} (keyid {keyid}) at {base}");
                println!("(dry run — nothing sent)");
            }
            std::process::exit(0);
        }
        let payload =
            json!({"actor": actor, "keyid": keyid, "publicKey": public_pem, "token": token});
        let body = match remote_post(&format!("{base}/register"), &payload) {
            Ok(b) => b,
            Err((code, message)) => {
                let e = json!({"ok": false, "error": {"code": code, "message": message}});
                remote_exit_json(&e, 1);
            }
        };
        let mut data = body.get("data").cloned().unwrap_or(Value::Null);
        if data.is_null() {
            // the server refused: {ok:false,error}
            remote_exit_json(&body, 1);
        }
        if let Value::Object(m) = &mut data {
            m.insert("server".to_string(), Value::String(base.to_string()));
        }
        if json_mode {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({"ok": true, "data": data})).unwrap()
            );
        } else {
            println!("enrolled {actor} (keyid {keyid}) at {base}");
        }
        std::process::exit(0);
    }

    // Envelope: sanitize argv (transport flags out), keep semantics
    let mut env_argv: Vec<String> = Vec::new();
    {
        let transport = ["--server", "--db", "--sign-key"];
        let mut i = 0;
        while i < argv.len() {
            let tok = &argv[i];
            let flag = tok.split('=').next().unwrap_or("");
            if transport.contains(&flag) {
                if !tok.contains('=') && i + 1 < argv.len() && !argv[i + 1].starts_with('-') {
                    i += 2;
                } else {
                    i += 1;
                }
                continue;
            }
            env_argv.push(tok.clone());
            i += 1;
        }
    }
    if !env_argv.iter().any(|a| a == "--actor") && !actor.is_empty() {
        env_argv.push("--actor".to_string());
        env_argv.push(actor.to_string());
    }
    let ts = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let nonce = uuid4();
    let args = json!({"argv": env_argv});
    let digest = bukio::canonical::build_digest(actor, &cmd, &args, &ts, &nonce);
    let mut envelope = json!({
        "v": 1, "actor": actor, "cmd": cmd, "args": args, "ts": ts, "nonce": nonce,
        "digest": digest, "sig": Value::Null, "keyid": Value::Null,
    });
    let key_file = remote_key_file(actor);
    if key_file.exists() {
        if let Ok(pem) = std::fs::read_to_string(&key_file) {
            if let (Ok(public_pem), Ok(sig)) = (
                bukio::sign::public_key_from_private(&pem, None),
                bukio::sign::sign(digest.as_bytes(), &pem),
            ) {
                if let Ok(keyid) = bukio::sign::keyid_of(&public_pem) {
                    envelope["sig"] = json!(sig);
                    envelope["keyid"] = json!(keyid);
                }
            }
        }
    }
    let body = match remote_post(&format!("{base}/rpc"), &envelope) {
        Ok(b) => b,
        Err((code, message)) => {
            let e = json!({"ok": false, "error": {"code": code, "message": message}});
            remote_exit_json(&e, 1);
        }
    };
    // server reply: {ok, stdout, stderr, exitCode} or {ok:false,error}
    if body.get("error").is_some() {
        remote_exit_json(&body, 1);
    }
    let stdout = body["stdout"].as_str().unwrap_or("");
    let code = body["exitCode"].as_i64().unwrap_or(1);
    print!("{stdout}");
    let _ = std::io::Write::flush(&mut std::io::stdout());
    if code != 0 {
        let stderr = body["stderr"].as_str().unwrap_or("");
        eprint!("{stderr}");
        std::process::exit(1);
    }
    std::process::exit(0);
}

/// POST JSON over plain HTTP/1.1 (tiny stdlib client — ureq 3.x drops error
/// bodies, which we need for gate refusals). Returns the parsed JSON reply.
fn remote_post(url: &str, payload: &Value) -> std::result::Result<Value, (String, String)> {
    use std::io::{Read, Write};
    let rest = url.strip_prefix("http://").unwrap_or(url);
    let (hostport, path) = match rest.split_once('/') {
        Some((h, p)) => (h, format!("/{p}")),
        None => (rest, "/".to_string()),
    };
    let (host, port) = match hostport.rsplit_once(':') {
        Some((h, p)) => (h.to_string(), p.parse().unwrap_or(80)),
        None => (hostport.to_string(), 80),
    };
    let body = serde_json::to_string(payload).unwrap_or_default();
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: {hostport}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let mut stream = std::net::TcpStream::connect((host.as_str(), port)).map_err(|e| {
        (
            "REMOTE_UNREACHABLE".into(),
            format!("cannot reach {url}: {e}"),
        )
    })?;
    stream.write_all(request.as_bytes()).ok();
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).ok();
    let text = String::from_utf8_lossy(&raw).to_string();
    let json_part = text.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or(&text);
    serde_json::from_str(json_part).map_err(|_| {
        (
            "REMOTE_ERROR".into(),
            format!(
                "non-JSON reply from {url}: {}",
                &text[..text.len().min(200)]
            ),
        )
    })
}

fn uuid4() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15]
    )
}

// ── report aging ───────────────────────────────────────────────────
fn cmd_report_aging(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let as_of = arg(argv, "--as-of").unwrap_or_else(bukio::dates::today_iso);
    let kind = arg(argv, "--kind").unwrap_or_else(|| "both".into());
    let data = bukio::reports::aging(&db, &as_of, &kind)?;
    // CSV export flattens contacts into rows
    emit_csv(
        argv,
        &data,
        &[
            "kind",
            "contact_id",
            "name",
            "current",
            "d30",
            "d60",
            "d90",
            "d90plus",
            "total",
        ],
        |d| {
            let mut rows = Vec::new();
            for kind_key in &["debtors", "creditors"] {
                if let Some(section) = d.get(*kind_key) {
                    if let Some(contacts) = section["contacts"].as_array() {
                        for c in contacts {
                            rows.push(vec![
                                kind_key.to_string(),
                                c["id"].as_i64().map(|v| v.to_string()).unwrap_or_default(),
                                c["name"].as_str().unwrap_or("").to_string(),
                                c["current"]
                                    .as_i64()
                                    .map(|v| v.to_string())
                                    .unwrap_or_default(),
                                c["d30"].as_i64().map(|v| v.to_string()).unwrap_or_default(),
                                c["d60"].as_i64().map(|v| v.to_string()).unwrap_or_default(),
                                c["d90"].as_i64().map(|v| v.to_string()).unwrap_or_default(),
                                c["d90plus"]
                                    .as_i64()
                                    .map(|v| v.to_string())
                                    .unwrap_or_default(),
                                c["total_cents"]
                                    .as_i64()
                                    .map(|v| v.to_string())
                                    .unwrap_or_default(),
                            ]);
                        }
                    }
                }
            }
            rows
        },
    )?;
    // If --format was specified, emit_csv handled it; otherwise return data
    if has_flag(argv, "--format") {
        Ok(json!({ "ok": true }))
    } else {
        Ok(data)
    }
}

// ── report sales ───────────────────────────────────────────────────
fn cmd_report_sales(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let year = arg(argv, "--year").unwrap_or_else(|| bukio::dates::today_iso()[0..4].to_string());
    let by = arg(argv, "--by").unwrap_or_else(|| "contact".into());
    bukio::reports::sales(&db, &year, &by)
}

// ── report cost-center ─────────────────────────────────────────────
fn cmd_report_cost_center(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let mut year = arg(argv, "--year");
    let mut from = arg(argv, "--from");
    let mut to = arg(argv, "--to");
    let cc = arg(argv, "--cost-center");
    if let Some(period) = arg(argv, "--period") {
        let (f, t) = bukio::vat::parse_period(&period)?;
        from = Some(f);
        to = Some(t);
    }
    // the JS CLI falls back to the current year when no period is given at all
    if year.is_none() && from.is_none() && to.is_none() {
        year = Some(bukio::dates::today_iso()[..4].to_string());
    }
    let r = bukio::reports::cost_center_report(
        &db,
        year.as_deref(),
        from.as_deref(),
        to.as_deref(),
        cc.as_deref(),
    )?;
    // the JS CLI projects a summary: formatted amounts, no per-account rows
    let centers: Vec<Value> = r["centers"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|c| {
            json!({
                "cost_center_code": c["cost_center_code"],
                "cost_center_name": c["cost_center_name"],
                "revenue": bukio::money::format_amount(c["revenue_cents"].as_i64().unwrap_or(0)),
                "costs": bukio::money::format_amount(c["costs_cents"].as_i64().unwrap_or(0)),
                "result": bukio::money::format_amount(c["result_cents"].as_i64().unwrap_or(0)),
            })
        })
        .collect();
    Ok(json!({
        "year": r["year"], "from": r["from"], "to": r["to"], "centers": centers,
    }))
}

// ── financial-statements report ───────────────────────────────────
fn cmd_financial_statements_report(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let profile = bukio::accounts::resolve_profile(&db)?;
    if profile["reporting"]["format"].as_str().is_none() {
        return Err(BukioError::new(
            "FORMAT_NOT_SUPPORTED",
            format!(
                "financial statements are not supported for country {}",
                profile["meta"]["country"].as_str().unwrap_or("?")
            ),
        ));
    }
    let year = arg(argv, "--year").ok_or_else(|| missing_arg("--year"))?;
    let format = arg(argv, "--format").unwrap_or_else(|| "json".into());

    // csv/xlsx not yet ported — return clear error
    if format == "csv" || format == "xlsx" {
        return Err(BukioError::new(
            "FORMAT_NOT_SUPPORTED",
            format!("financial-statements format '{format}' is not ported to Rust yet"),
        ));
    }

    let model = arg(argv, "--model");
    let report = bukio::reports::jaarrekening(&db, &year, model.as_deref())?;

    if format == "html" {
        return Ok(json!({ "html": bukio::pdf::jaarrekening_html(&report) }));
    }
    if format == "pdf" {
        // the JS default out path: financial-statements-<year>-<model>.pdf
        let default_out = format!(
            "financial-statements-{year}-{}.pdf",
            report["model"].as_str().unwrap_or("klein")
        );
        let out = arg(argv, "--out").unwrap_or(default_out);
        let result = bukio::pdf::jaarrekening_to_pdf(&report, Some(&out))?;
        return Ok(json!({ "path": result["path"], "bytes": result["bytes"] }));
    }
    Ok(json!({ "financial_statements": report }))
}

// ── icp readout ──────────────────────────────────────────────────
fn cmd_icp_readout(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let period = arg(argv, "--period").ok_or_else(|| missing_arg("--period"))?;
    bukio::reports::icp_readout(&db, &period)
}

// ── invoice pdf ──────────────────────────────────────────────────
fn cmd_invoice_pdf(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let id = parse_i64(argv, "--id").ok_or_else(|| missing_arg("--id"))?;
    let inv = bukio::invoice::get_invoice(&db, id)?
        .ok_or_else(|| BukioError::new("NOT_FOUND", format!("invoice {id} does not exist")))?;
    let number = inv["invoice_number"].as_str().ok_or_else(|| {
        BukioError::new(
            "NOT_FINALIZED",
            "finalize the invoice first — a draft has no number yet",
        )
    })?;
    let default_out = format!("{number}.pdf");
    let out = arg(argv, "--out").unwrap_or(default_out);
    bukio::pdf::invoice_to_pdf(&db, &inv, Some(&out))
}

// ── invoice ubl ──────────────────────────────────────────────────
fn cmd_invoice_ubl(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let id = parse_i64(argv, "--id").ok_or_else(|| missing_arg("--id"))?;
    let inv = bukio::invoice::get_invoice(&db, id)?
        .ok_or_else(|| BukioError::new("NOT_FOUND", format!("invoice {id} does not exist")))?;
    if inv["invoice_number"].is_null() {
        return Err(BukioError::new(
            "NOT_FINALIZED",
            "finalize the invoice first".to_string(),
        ));
    }
    let xml = bukio::ubl::invoice_to_ubl(&db, &inv)?;
    let out_path = arg(argv, "--out").unwrap_or_else(|| {
        format!(
            "{}.xml",
            inv["invoice_number"].as_str().unwrap_or("invoice")
        )
    });
    std::fs::write(&out_path, &xml).map_err(|e| {
        BukioError::new("FILE_WRITE_ERROR", format!("cannot write {out_path}: {e}"))
    })?;
    let bytes = xml.chars().count();
    if !has_flag(argv, "--json") {
        println!("wrote {out_path} ({bytes} bytes)");
    }
    Ok(json!({ "path": out_path, "bytes": bytes }))
}

// ── invoice email ────────────────────────────────────────────────
fn cmd_invoice_email(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    let db = open_existing(db_path)?;
    let id = parse_i64(argv, "--id").ok_or_else(|| missing_arg("--id"))?;
    let to = arg(argv, "--to");
    let subject = arg(argv, "--subject");
    let body = arg(argv, "--body");
    let attach_pdf = !has_flag(argv, "--no-pdf");
    bukio::smtp::email_invoice(
        &db,
        id,
        to.as_deref(),
        subject.as_deref(),
        body.as_deref(),
        attach_pdf,
        actor,
        dry_run,
    )
}

// ── invoice reminders ────────────────────────────────────────────
fn cmd_invoice_reminders(
    argv: &[String],
    db_path: &str,
    _actor: &str,
    _dry_run: bool,
) -> Result<Value> {
    let db = open_existing(db_path)?;
    let within = match arg(argv, "--within-days") {
        None => 30,
        Some(v) => v.parse::<i64>().map_err(|_| {
            BukioError::new(
                "INVALID_WINDOW",
                format!("invalid --within-days '{v}' — must be a non-negative integer"),
            )
        })?,
    };
    if within < 0 {
        return Err(BukioError::new(
            "INVALID_WINDOW",
            "--within-days must not be negative",
        ));
    }
    let rows = bukio::invoice::list_invoices(&db, None, None)?;
    let overdue: Vec<Value> = rows
        .into_iter()
        .filter(|inv| {
            if let Some(status) = inv.get("status").and_then(|v| v.as_str()) {
                (status == "sent" || status == "overdue") && within >= 0
            } else {
                false
            }
        })
        .collect();
    Ok(json!({ "reminders": overdue, "within_days": within }))
}

// ── invoice peppol-send ──────────────────────────────────────────
fn cmd_invoice_peppol_send(
    argv: &[String],
    db_path: &str,
    _actor: &str,
    dry_run: bool,
) -> Result<Value> {
    let db = open_existing(db_path)?;
    let id = parse_i64(argv, "--id").ok_or_else(|| missing_arg("--id"))?;
    let inv = bukio::invoice::get_invoice(&db, id)?
        .ok_or_else(|| BukioError::new("NOT_FOUND", format!("invoice {id} does not exist")))?;
    if inv["invoice_number"].is_null() {
        return Err(BukioError::new(
            "NOT_FINALIZED",
            "finalize the invoice first".to_string(),
        ));
    }
    let endpoint = arg(argv, "--endpoint");
    let result = bukio::peppol::send_peppol_invoice(&db, &inv, endpoint.as_deref(), dry_run)?;
    if !has_flag(argv, "--json") {
        if result["dryRun"] == json!(true) {
            println!(
                "plan: POST UBL for {} ({} bytes) to {}{}",
                result["invoice_number"].as_str().unwrap_or(""),
                result["bytes"].as_i64().unwrap_or(0),
                result["endpoint"].as_str().unwrap_or(""),
                if result["configured"] == json!(true) {
                    " (token set)"
                } else {
                    " (NO TOKEN — add BUKIO_PEPPOL_TOKEN)"
                }
            );
            println!("(dry run — nothing sent)");
        } else {
            println!(
                "sent {} to {} — HTTP {}{}",
                result["invoice_number"].as_str().unwrap_or(""),
                result["endpoint"].as_str().unwrap_or(""),
                result["status"].as_i64().unwrap_or(0),
                match result["response"].as_str() {
                    Some(r) => format!(": {r}"),
                    None => String::new(),
                }
            );
        }
    }
    Ok(result)
}

// ── import xaf ──────────────────────────────────────────────────
fn cmd_import_xaf(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    let db = open_existing(db_path)?;
    let file = arg(argv, "--file").ok_or_else(|| missing_arg("--file"))?;
    let content = std::fs::read_to_string(&file)
        .map_err(|e| BukioError::new("FILE_READ_ERROR", format!("cannot read {file}: {e}")))?;
    let result = bukio::import_mod::import_xaf(&db, &content, actor, dry_run)?;
    Ok(result)
}

// ── import invoice ──────────────────────────────────────────────
fn cmd_import_invoice(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    let db = open_existing(db_path)?;
    let file = arg(argv, "--file").ok_or_else(|| missing_arg("--file"))?;
    let content = match std::fs::read_to_string(&file) {
        Ok(c) => c,
        Err(_) => {
            return Err(BukioError::new(
                "FILE_NOT_FOUND",
                format!("'{file}' not found"),
            ));
        }
    };
    let contact_id = parse_i64(argv, "--contact-id");
    let create_missing = has_flag(argv, "--create-missing");
    let result = bukio::import_mod::import_invoice(
        &db,
        &content,
        contact_id,
        create_missing,
        actor,
        dry_run,
    )?;
    Ok(result)
}
