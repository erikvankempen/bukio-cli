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
    println!(
        "{}",
        serde_json::to_string_pretty(
            &json!({ "ok": false, "error": { "code": err.code, "message": err.message } })
        )
        .unwrap()
    );
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

    // --help / --version (before global flag parsing)
    let has_subcmd = argv.iter().any(|a| !a.starts_with('-'));
    if (has_flag(&argv, "--help") || has_flag(&argv, "-h")) && !has_subcmd {
        println!("bukio — agent-first bookkeeping for SMEs across thirty-one jurisdictions");
        println!("Usage: bukio [global flags] <command> [subcommand] [options]");
        println!();
        println!("Global flags:");
        println!("  --db <path>           database file (default: ~/.bukio/bukio.db)");
        println!("  --json                machine-readable JSON output");
        println!("  --actor <who>         acting entity '<role>:<name>' (required)");
        println!("  --locale <code>       output language (default: en)");
        println!("  --sign-key <path>     explicit private-key file to sign with");
        println!("  --server <url>        remote bukio server URL");
        println!("  --dry-run             show the plan without writing");
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
            if json_mode {
                ok(data);
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
        ["compliance", "status"] => cmd_compliance_status(db_path),
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
        ["payments", "batch", "export"] => cmd_batch_export(argv, db_path, dry_run),

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
        ["update"] => cmd_update(argv),

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
        ["actor", "authz"] => cmd_actor_authz(argv, db_path, dry_run),
        ["actor", "roles"] => cmd_actor_roles(db_path),
        ["actor", "grant"] => cmd_actor_grant(argv, db_path, actor),
        ["actor", "roles", "grant"] => cmd_actor_grant(argv, db_path, actor),
        ["actor", "revoke-role"] => cmd_actor_revoke_role(argv, db_path, actor),
        ["actor", "roles", "revoke"] => cmd_actor_revoke_role(argv, db_path, actor),
        ["actor", "can"] => cmd_actor_can(argv, db_path, actor),
        ["actor", "who-can"] => cmd_actor_who_can(argv, db_path, actor),
        ["actor", "verify"] => cmd_actor_verify_key(db_path, actor),

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
    Err(BukioError::new(
        "UNKNOWN_COMMAND",
        format!("unknown command: {}", positional.join(" ")),
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
            "currency": null,
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
            json!({ "action": "account.reactivate", "code": code, "from": "inactive", "to": "active", "dryRun": true }),
        );
    }
    let updated = bukio::accounts::reactivate_account(&db, &code)?;
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
    Ok(updated)
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
            action: "cost_center.add",
            command: Some("cost-center add"),
            args: Some(json!({ "code": code, "name": name })),
            outcome: "ok",
            entry_ids: vec![],
        },
    )?;
    Ok(cc)
}

fn cmd_cc_list(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let rows = bukio::accounts::list_cost_centers(&db, has_flag(argv, "--include-inactive"))?;
    Ok(json!({ "cost_centers": rows }))
}

fn cmd_cc_show(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let code = arg(argv, "--code").ok_or_else(|| missing_arg("--code"))?;
    let cc = bukio::accounts::get_cost_center_by_code(&db, &code)
        .ok_or_else(|| BukioError::new("NOT_FOUND", format!("cost center {code} not found")))?;
    Ok(cc)
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
                "cost_center.reactivate"
            } else {
                "cost_center.deactivate"
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
    Ok(updated)
}

// ── report ─────────────────────────────────────────────────────────────────

/// CSV file writer.
fn write_csv(path: &str, columns: &[&str], rows: &[Vec<String>]) -> Result<()> {
    let mut w = std::fs::File::create(path)
        .map_err(|e| BukioError::new("IO_ERROR", format!("cannot write {path}: {e}")))?;
    use std::io::Write;
    writeln!(w, "{}", columns.join(",")).ok();
    for row in rows {
        writeln!(w, "{}", row.join(",")).ok();
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
        print!("{}", columns.join(","));
        println!();
        for row in &rows {
            println!("{}", row.join(","));
        }
        return Ok(true);
    }
    Ok(false)
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
    let as_of = arg(argv, "--as-of").unwrap_or_else(|| {
        format!(
            "{}-12-31",
            bukio::dates::today_iso().get(0..4).unwrap_or("2026")
        )
    });
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

fn cmd_pnl(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let year = arg(argv, "--year").unwrap_or_else(|| bukio::dates::today_iso()[0..4].to_string());
    let data = bukio::reports::pnl(&db, &format!("{year}-01-01"), &format!("{year}-12-31"))?;
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
    let rows = bukio::reports::journal(
        &db,
        &format!("{year}-01-01"),
        &format!("{year}-12-31"),
        None,
    )?;
    let data = json!({ "rows": rows });
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
    Ok(json!({ "entries": rows }))
}

fn cmd_audit_verify(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    bukio::audit::verify_trail(&db)
}

// ── backup ─────────────────────────────────────────────────────────────────

fn cmd_backup(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    require_actor(actor)?;
    bukio::backup::cmd_backup(
        db_path,
        arg(argv, "--out").as_deref(),
        parse_i64(argv, "--keep").map(|v| v as usize),
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
    bukio::backup::cmd_restore(&from, &to, has_flag(argv, "--force"), actor, dry_run)
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
    let account_code = arg(argv, "--account-code").unwrap_or_else(|| "1100".into());
    bukio::bank::get_or_create_bank_account(&db, &iban, name.as_deref(), &account_code, dry_run)
}

fn cmd_bank_list(db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let accounts = bukio::bank::list_bank_accounts(&db)?;
    Ok(json!({ "accounts": accounts }))
}

fn cmd_bank_import(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    require_actor(actor)?;
    let db = open_existing(db_path)?;
    let file = arg(argv, "--file").ok_or_else(|| missing_arg("--file"))?;
    let iban = arg(argv, "--iban").ok_or_else(|| missing_arg("--iban"))?;
    let name = arg(argv, "--name");
    let account_code = arg(argv, "--account-code").unwrap_or_else(|| "1100".into());
    let content = std::fs::read_to_string(&file)
        .map_err(|e| BukioError::new("FILE_ERROR", format!("cannot read {file}: {e}")))?;
    let transactions = if content.trim_start().starts_with('<') {
        bukio::bank::parse_camt053(&content)
            .map_err(|e| BukioError::new("PARSE_ERROR", e.message.clone()))?
    } else {
        bukio::bank::parse_bank_csv(&content, &iban)
            .map_err(|e| BukioError::new("PARSE_ERROR", e.message.clone()))?
    };
    if dry_run {
        bukio::bank::preview_import(&db, &iban, &transactions)
    } else {
        bukio::bank::import_transactions(
            &db,
            &iban,
            &transactions,
            name.as_deref(),
            &account_code,
            actor,
        )
    }
}

fn cmd_bank_transactions(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let state = arg(argv, "--state");
    let iban = arg(argv, "--iban");
    let limit: i64 = parse_limit(argv, 200)?;
    let transactions =
        bukio::bank::list_transactions(&db, state.as_deref(), iban.as_deref(), limit)?;
    Ok(json!({ "transactions": transactions }))
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
    bukio::bank::auto_match(&db, window, actor, dry_run)
}

fn cmd_bank_match_suggest(db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let suggestions = bukio::bank::suggest_unmatched(&db)?;
    Ok(json!({ "suggestions": suggestions }))
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
    )
}

fn cmd_vat_settle(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    require_actor(actor)?;
    let tx: i64 = parse_i64(argv, "--tx").ok_or_else(|| missing_arg("--tx"))?;
    let db = open_existing(db_path)?;
    let row: Option<(i64, String)> = db
        .query_row(
            "SELECT t.amount_cents, a.code FROM bank_transactions t
             JOIN bank_accounts a ON a.id = t.account_id WHERE t.id = ?1",
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
    )?;
    if !dry_run {
        if let Some(entry_id) = result["entry_id"].as_i64() {
            let _ = db.execute(
                "UPDATE bank_transactions SET state = 'matched', matched_entry_id = ?1 WHERE id = ?2",
                rusqlite::params![entry_id, tx],
            );
            result["tx"] = json!({ "id": tx, "state": "matched" });
        }
    }
    Ok(result)
}

// ── recurring ──────────────────────────────────────────────────────────────

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
        let postings_raw = repeated(argv, "--postings");
        let postings_json = serde_json::to_string(
            &bukio::entries::parse_posting_specs(&postings_raw)?
                .iter()
                .map(|s| json!({ "code": s.code, "amountCents": s.amount_cents }))
                .collect::<Vec<_>>(),
        )
        .unwrap_or_default();
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
            Ok(json!({ "template": tpl, "dryRun": false }))
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
    Ok(json!({ "templates": rows }))
}

fn cmd_recurring_show(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let id: i64 = parse_i64(argv, "--id").ok_or_else(|| missing_arg("--id"))?;
    let tpl = bukio::recurring::get_template(&db, id)?
        .ok_or_else(|| BukioError::new("NOT_FOUND", format!("template {id} not found")))?;
    Ok(tpl)
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
        json!({ "template": bukio::recurring::set_template_status(&db, id, "paused", actor, dry_run)? }),
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
        json!({ "template": bukio::recurring::set_template_status(&db, id, "active", actor, dry_run)? }),
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
    bukio::recurring::run_due(&db, as_of.as_deref(), template_id, actor, dry_run)
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
    let cost_cents: i64 = parse_i64(argv, "--cost").unwrap_or(0);
    let life_months: i64 = parse_i64(argv, "--life-months").unwrap_or(60);
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
    Ok(updated)
}

fn cmd_contact_list(db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let rows = bukio::contacts::list_contacts(&db)?;
    Ok(json!({ "contacts": rows }))
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
    let due_days = parse_i64(argv, "--due-days");
    let description = arg(argv, "--description");
    let reference = arg(argv, "--reference");
    let notes = arg(argv, "--notes");
    let discount_pct = arg(argv, "--discount-pct");
    let discount_amount = arg(argv, "--discount-amount");
    let (discount_type, discount_value) = if let Some(pct) = discount_pct {
        (Some("percent".to_string()), pct.parse::<i64>().ok())
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
        description.as_deref(),
        reference.as_deref(),
        notes.as_deref(),
        discount_type.as_deref(),
        discount_value,
        &lines_raw,
        actor,
        dry_run,
    )?;
    if dry_run {
        Ok(invoice)
    } else {
        Ok(json!({ "invoice": invoice }))
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
    Ok(inv)
}

fn cmd_invoice_list(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let status = arg(argv, "--status");
    let invoice_type = arg(argv, "--type");
    let rows = bukio::invoice::list_invoices(&db, status.as_deref(), invoice_type.as_deref())?;
    Ok(json!({ "invoices": rows }))
}

fn cmd_invoice_show(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let id: i64 = parse_i64(argv, "--id").ok_or_else(|| missing_arg("--id"))?;
    let inv = bukio::invoice::get_invoice(&db, id)?
        .ok_or_else(|| BukioError::new("NOT_FOUND", format!("invoice {id} not found")))?;
    Ok(json!({ "invoice": inv }))
}

fn cmd_invoice_credit(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    require_actor(actor)?;
    let db = open_existing(db_path)?;
    let id: i64 = parse_i64(argv, "--id").ok_or_else(|| missing_arg("--id"))?;
    let date = arg(argv, "--date");
    let reason = arg(argv, "--reason");
    bukio::invoice::credit_invoice(&db, id, date.as_deref(), reason.as_deref(), actor, dry_run)
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
            .ok_or_else(|| BukioError::new("NOT_FOUND", format!("invoice {id} not found")))?;
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
    )?;
    if inv.get("dryRun").and_then(|v| v.as_bool()).unwrap_or(false) {
        Ok(json!({ "plan": inv }))
    } else {
        Ok(json!({ "invoice": inv }))
    }
}

// ── year-end ───────────────────────────────────────────────────────────────

fn cmd_year_end_status(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let year = arg(argv, "--year").ok_or_else(|| missing_arg("--year"))?;
    bukio::year_end::year_end_status(&db, &year)
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
    Ok(json!({ "status": "ok" }))
}

// ── compliance ─────────────────────────────────────────────────────────────

fn cmd_compliance_status(db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let year: i32 = bukio::dates::today_iso()[0..4].parse().unwrap_or(2026);
    bukio::compliance::compliance_status(&db, year)
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
    // Country is immutable after init
    if changes.iter().any(|(col, _)| col == "country") {
        let cur = db
            .query_row("SELECT country FROM company WHERE id=1", [], |r| {
                r.get::<_, Option<String>>(0)
            })
            .ok()
            .flatten()
            .unwrap_or_default();
        return Err(BukioError::new("COUNTRY_IMMUTABLE", format!("country is immutable after init — company stays {cur} (re-init a new DB for another country)")));
    }
    if changes.is_empty() {
        return Err(BukioError::new("NOTHING_TO_UPDATE", "nothing to update — pass at least one of --name/--registration-id/--tax-id/--iban/--address/--postal-code/--city"));
    }
    if dry_run {
        return Ok(json!({
            "company": bukio::company::get_company(&db)?,
            "changes": serde_json::Map::from_iter(changes.iter().map(|(k,v)| (k.clone(), Value::String(v.clone())))),
            "dryRun": true,
        }));
    }
    let (updated, changes_map) = bukio::company::update_company(&db, &changes, None, None, actor)?;
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
    let life_months: i64 = parse_i64(argv, "--life-months").unwrap_or(60);
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
    Ok(json!({ "assets": rows }))
}

fn cmd_asset_show(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let id: i64 = parse_i64(argv, "--id").ok_or_else(|| missing_arg("--id"))?;
    let asset = bukio::assets::get_asset(&db, id)?
        .ok_or_else(|| BukioError::new("NOT_FOUND", format!("asset {id} not found")))?;
    Ok(asset)
}

fn cmd_asset_run(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    require_actor(actor)?;
    let db = open_existing(db_path)?;
    let period = arg(argv, "--period")
        .or_else(|| arg(argv, "--as-of"))
        .unwrap_or_else(bukio::dates::today_iso);
    bukio::assets::run_due(&db, &period, actor, dry_run)
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
    bukio::assets::register(&db, as_of.as_deref(), actor)
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
    let invoice_ref = arg(argv, "--invoice-ref").unwrap_or_default();
    let due = arg(argv, "--due").unwrap_or_else(bukio::dates::today_iso);
    let amount_str = arg(argv, "--amount").ok_or_else(|| missing_arg("--amount"))?;
    let amount_cents = bukio::money::parse_amount(&amount_str)?;
    let method = arg(argv, "--method").unwrap_or_else(|| "transfer".into());
    let entry_id = parse_i64(argv, "--entry-id");
    bukio::payments::add_payable(
        &db,
        &contact_ref,
        &invoice_ref,
        &due,
        None,
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
    let rows = bukio::payments::list_payables(&db, status.as_deref(), method.as_deref(), None)?;
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
    let kind = arg(argv, "--type").unwrap_or_else(|| "transfer".into());
    let payable_ids: Vec<i64> = arg(argv, "--payable")
        .map(|s| s.split(',').filter_map(|v| v.trim().parse().ok()).collect())
        .unwrap_or_default();
    let lines: Vec<Value> = arg(argv, "--lines")
        .map(|s| {
            s.split(';')
                .filter(|l| !l.trim().is_empty())
                .map(|l| json!({ "raw": l.trim() }))
                .collect()
        })
        .unwrap_or_default();
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

fn cmd_batch_export(argv: &[String], db_path: &str, dry_run: bool) -> Result<Value> {
    let db = open_existing(db_path)?;
    let id: i64 = parse_i64(argv, "--id").ok_or_else(|| missing_arg("--id"))?;
    let batch = bukio::payments::get_payment_batch(&db, id)?;
    let schema = arg(argv, "--schema").unwrap_or_else(|| "001.03".into());
    let out = arg(argv, "--out");
    // Build pain.001 XML
    let lines = batch["lines"].as_array().cloned().unwrap_or_default();
    let xml = bukio::payments::build_pain001(
        &batch["id"].to_string(),
        &bukio::dates::today_iso(),
        &batch
            .get("debit_name")
            .and_then(|v| v.as_str())
            .unwrap_or(""),
        &batch
            .get("debit_iban")
            .and_then(|v| v.as_str())
            .unwrap_or(""),
        &batch
            .get("date")
            .and_then(|v| v.as_str())
            .unwrap_or(&bukio::dates::today_iso()),
        &lines,
        &schema,
    );
    if let Some(path) = &out {
        std::fs::write(path, &xml)
            .map_err(|e| BukioError::new("IO_ERROR", format!("cannot write {path}: {e}")))?;
    }
    if dry_run {
        Ok(json!({ "batch": batch, "schema": schema, "xml_length": xml.len(), "dryRun": true }))
    } else {
        Ok(json!({ "batch": batch, "schema": schema, "xml_length": xml.len() }))
    }
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

fn cmd_update(argv: &[String]) -> Result<Value> {
    let repo = arg(argv, "--repo").unwrap_or_else(|| ".".into());
    let yes = has_flag(argv, "--yes");
    let dry_run = has_flag(argv, "--dry-run");
    let trust_remote = has_flag(argv, "--trust-remote");
    // Validate remote URL
    let output = std::process::Command::new("git")
        .args(["-C", &repo, "remote", "get-url", "origin"])
        .output()
        .map_err(|e| BukioError::new("GIT_ERROR", format!("git failed: {e}")))?;
    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !url.contains("github.com/erikvankempen/bukio-cli") && !trust_remote {
        return Err(BukioError::new(
            "UNTRUSTED_REMOTE",
            format!("remote URL {url} does not match expected repository"),
        ));
    }
    if dry_run || !yes {
        return Ok(
            json!({ "repo": repo, "url": url, "dryRun": true, "message": "pass --yes to apply update" }),
        );
    }
    let fetch = std::process::Command::new("git")
        .args(["-C", &repo, "fetch", "origin"])
        .output()
        .map_err(|e| BukioError::new("GIT_ERROR", format!("git fetch failed: {e}")))?;
    if !fetch.status.success() {
        return Err(BukioError::new(
            "GIT_ERROR",
            format!(
                "git fetch failed: {}",
                String::from_utf8_lossy(&fetch.stderr)
            ),
        ));
    }
    let reset = std::process::Command::new("git")
        .args(["-C", &repo, "reset", "--hard", "origin/main"])
        .output()
        .map_err(|e| BukioError::new("GIT_ERROR", format!("git reset failed: {e}")))?;
    if !reset.status.success() {
        return Err(BukioError::new(
            "GIT_ERROR",
            format!(
                "git reset failed: {}",
                String::from_utf8_lossy(&reset.stderr)
            ),
        ));
    }
    Ok(json!({ "repo": repo, "url": url, "updated": true }))
}

// ── attach ─────────────────────────────────────────────────────────────────

fn cmd_attach_add(argv: &[String], db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    require_actor(actor)?;
    let db = open_existing(db_path)?;
    let file = arg(argv, "--file").ok_or_else(|| missing_arg("--file"))?;
    let store = arg(argv, "--store").unwrap_or_else(|| "db".into());
    let note = arg(argv, "--note");
    let (kind, ref_id) = if let Some(id) = parse_i64(argv, "--invoice") {
        ("invoice", id)
    } else if let Some(id) = parse_i64(argv, "--entry") {
        ("entry", id)
    } else {
        return Err(missing_arg("--invoice or --entry"));
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
    bukio::actor_cli::cmd_revoke(db_path, actor, reason.as_deref(), dry_run)
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

fn cmd_actor_authz(argv: &[String], db_path: &str, dry_run: bool) -> Result<Value> {
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
    bukio::actor_cli::cmd_authz(
        db_path,
        &std::env::var("BUKIO_ACTOR").unwrap_or_default(),
        on,
        dry_run,
    )
}

fn cmd_actor_roles(db_path: &str) -> Result<Value> {
    bukio::actor_cli::cmd_roles(db_path, &std::env::var("BUKIO_ACTOR").unwrap_or_default())
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
    bukio::actor_cli::cmd_revoke_role(db_path, &for_who, &role)
}

fn cmd_actor_can(argv: &[String], db_path: &str, actor: &str) -> Result<Value> {
    require_actor(actor)?;
    let action = positional_after(argv, "can").ok_or_else(|| missing_arg("command"))?;
    bukio::actor_cli::cmd_can(db_path, actor, &action)
}

fn cmd_actor_verify_key(db_path: &str, actor: &str) -> Result<Value> {
    require_actor(actor)?;
    bukio::actor_cli::cmd_verify_actor(db_path, actor)
}

fn cmd_actor_verify(db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    bukio::audit::verify_trail(&db)
}

fn cmd_actor_who_can(argv: &[String], db_path: &str, actor: &str) -> Result<Value> {
    require_actor(actor)?;
    let db = open_existing(db_path)?;
    let mut results = Vec::new();
    for &(cmd, cap) in bukio::authz::CLI_CAPABILITIES {
        let allowed = bukio::authz::can_act(&db, actor, cap);
        results.push(json!({ "command": cmd, "capability": cap, "allowed": allowed }));
    }
    Ok(json!({ "actor": actor, "commands": results }))
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
    let ttl: u64 = parse_i64(argv, "--ttl-hours").unwrap_or(24) as u64;
    let target_actor = positional_after(argv, "token").ok_or_else(|| missing_arg("actor"))?;
    let token = bukio::server::mint_enrol_token(&target_actor, ttl)?;
    Ok(json!({ "token": token, "actor": target_actor, "ttl_hours": ttl }))
}

// ── report aging ───────────────────────────────────────────────────
fn cmd_report_aging(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
    let as_of = arg(argv, "--as-of").unwrap_or_else(bukio::dates::today_iso);
    let kind = arg(argv, "--kind").unwrap_or_else(|| "both".into());
    bukio::reports::aging(&db, &as_of, &kind)
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
    let year = arg(argv, "--year");
    let from = arg(argv, "--from");
    let to = arg(argv, "--to");
    let cc = arg(argv, "--cost-center");
    bukio::reports::cost_center_report(
        &db,
        year.as_deref(),
        from.as_deref(),
        to.as_deref(),
        cc.as_deref(),
    )
}

// ── financial-statements report ───────────────────────────────────
fn cmd_financial_statements_report(argv: &[String], db_path: &str) -> Result<Value> {
    let db = open_existing(db_path)?;
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
    let _db = open_existing(db_path)?;
    let id = parse_i64(argv, "--id").ok_or_else(|| missing_arg("--id"))?;
    Err(BukioError::new(
        "NOT_SUPPORTED",
        format!("invoice pdf for id {id} requires Chromium — not ported to Rust"),
    ))
}

// ── invoice ubl ──────────────────────────────────────────────────
fn cmd_invoice_ubl(argv: &[String], db_path: &str) -> Result<Value> {
    let _db = open_existing(db_path)?;
    let id = parse_i64(argv, "--id").ok_or_else(|| missing_arg("--id"))?;
    Err(BukioError::new(
        "NOT_SUPPORTED",
        format!("invoice UBL for id {id} is not ported to Rust"),
    ))
}

// ── invoice email ────────────────────────────────────────────────
fn cmd_invoice_email(
    argv: &[String],
    db_path: &str,
    _actor: &str,
    _dry_run: bool,
) -> Result<Value> {
    let _db = open_existing(db_path)?;
    let id = parse_i64(argv, "--id").ok_or_else(|| missing_arg("--id"))?;
    Err(BukioError::new(
        "SMTP_NOT_CONFIGURED",
        format!("invoice email for id {id} requires SMTP configuration"),
    ))
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
    _dry_run: bool,
) -> Result<Value> {
    let _db = open_existing(db_path)?;
    let id = parse_i64(argv, "--id").ok_or_else(|| missing_arg("--id"))?;
    Err(BukioError::new(
        "PEPPOL_NOT_AVAILABLE",
        format!("Peppol send for invoice {id} requires network access"),
    ))
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
    let content = std::fs::read_to_string(&file)
        .map_err(|e| BukioError::new("FILE_READ_ERROR", format!("cannot read {file}: {e}")))?;
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
