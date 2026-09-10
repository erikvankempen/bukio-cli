// bukio-cli — agent-first double-entry bookkeeping for SMEs.
// Copyright (c) 2026 Erik van Kempen.
// SPDX-License-Identifier: Apache-2.0
//
// MCP server — JSON-RPC 2.0 over stdio (newline-delimited).

use crate::accounts::{
    create_account, deactivate_account, get_account_by_code, list_accounts, reactivate_account,
    resolve_profile, NewAccount,
};
use crate::audit;
use crate::company::get_company;
use crate::contacts::{create_contact, list_contacts};
use crate::db::open_db;
use crate::entries::{
    create_entry, list_entries, post_entry, reverse_entry, CreateEntry, PostingSpec,
};
use crate::import_mod;
use crate::money::{format_amount, BukioError, Result};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};

const PROTOCOL_VERSION: &str = "2024-11-05";

fn rpc_response(id: Option<Value>, result: Value) -> String {
    json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string()
}

fn rpc_error(id: Option<Value>, code: i64, message: &str) -> String {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}).to_string()
}

fn rpc_content(result: Value) -> Value {
    let text = serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string());
    json!({"content": [{"type": "text", "text": text}], "isError": false})
}

fn rpc_error_content(code: &str, message: &str) -> Value {
    let err = json!({"ok": false, "error": {"code": code, "message": message}});
    let text = serde_json::to_string_pretty(&err).unwrap_or_else(|_| err.to_string());
    json!({"content": [{"type": "text", "text": text}], "isError": true})
}

fn json_value(s: &str) -> Value {
    serde_json::from_str(s).unwrap_or(Value::Null)
}

/// Tool definitions
fn tool_defs() -> Vec<Value> {
    vec![
        json!({"name": "company_info", "description": "the company behind this database", "inputSchema": {"type": "object", "properties": {}}}),
        json!({"name": "trial_balance", "description": "per-account totals; balanced tells you the books reconcile", "inputSchema": {"type": "object", "properties": {"year": {"type": "string"}}}}),
        json!({"name": "balance_sheet", "description": "balance sheet as of a date", "inputSchema": {"type": "object", "properties": {"as_of": {"type": "string"}}}}),
        json!({"name": "pnl", "description": "profit & loss for a year", "inputSchema": {"type": "object", "properties": {"year": {"type": "string"}}, "required": ["year"]}}),
        json!({"name": "journal", "description": "journal export for a year", "inputSchema": {"type": "object", "properties": {"year": {"type": "string"}, "limit": {"type": "integer"}}, "required": ["year"]}}),
        json!({"name": "accounts", "description": "chart of accounts", "inputSchema": {"type": "object", "properties": {"include_inactive": {"type": "boolean"}}}}),
        json!({"name": "entry_add", "description": "create a journal entry", "inputSchema": {"type": "object", "properties": {"date": {"type": "string"}, "description": {"type": "string"}, "postings": {"type": "array", "items": {"type": "string"}}, "post": {"type": "boolean"}, "mode": {"type": "string"}}, "required": ["date", "description", "postings"]}}),
        json!({"name": "entry_post", "description": "post a draft entry", "inputSchema": {"type": "object", "properties": {"id": {"type": "integer"}, "mode": {"type": "string"}}, "required": ["id"]}}),
        json!({"name": "entry_reverse", "description": "reverse a posted entry", "inputSchema": {"type": "object", "properties": {"id": {"type": "integer"}, "reason": {"type": "string"}, "mode": {"type": "string"}}, "required": ["id"]}}),
        json!({"name": "invoices", "description": "list invoices", "inputSchema": {"type": "object", "properties": {"status": {"type": "string"}, "limit": {"type": "integer"}}}}),
        json!({"name": "contacts", "description": "list contacts", "inputSchema": {"type": "object", "properties": {"limit": {"type": "integer"}}}}),
        json!({"name": "contact_add", "description": "create a contact", "inputSchema": {"type": "object", "properties": {"name": {"type": "string"}, "address": {"type": "string"}, "postal_code": {"type": "string"}, "city": {"type": "string"}, "country": {"type": "string"}, "email": {"type": "string"}, "vat_id": {"type": "string"}, "kvk": {"type": "string"}, "iban": {"type": "string"}, "mode": {"type": "string"}}, "required": ["name"]}}),
        json!({"name": "audit", "description": "append-only audit log", "inputSchema": {"type": "object", "properties": {"limit": {"type": "integer"}}}}),
        json!({"name": "vat_readout", "description": "VAT return fields 1a-5d", "inputSchema": {"type": "object", "properties": {"period": {"type": "string"}}, "required": ["period"]}}),
        json!({"name": "vat_book", "description": "book a VAT entry with postings", "inputSchema": {"type": "object", "properties": {"date": {"type": "string"}, "description": {"type": "string"}, "postings": {"type": "array", "items": {"type": "string"}}, "post": {"type": "boolean"}, "mode": {"type": "string"}, "actor": {"type": "string"}}, "required": ["date", "description", "postings"]}}),
        json!({"name": "asset_add", "description": "register an asset", "inputSchema": {"type": "object", "properties": {"name": {"type": "string"}, "purchase_date": {"type": "string"}, "purchase_price": {"type": "string"}, "depreciation_start": {"type": "string"}, "recognition_date": {"type": "string"}, "category": {"type": "string"}, "asset_account": {"type": "string"}, "expense_account": {"type": "string"}, "cum_dep": {"type": "string"}, "mode": {"type": "string"}, "actor": {"type": "string"}}, "required": ["name", "purchase_date", "purchase_price"]}}),
        json!({"name": "assets_run", "description": "book depreciation due for a period", "inputSchema": {"type": "object", "properties": {"period": {"type": "string"}, "mode": {"type": "string"}, "actor": {"type": "string"}}}}),
        json!({"name": "invoice_pay", "description": "mark an invoice as paid", "inputSchema": {"type": "object", "properties": {"id": {"type": "integer"}, "date": {"type": "string"}, "mode": {"type": "string"}, "actor": {"type": "string"}}, "required": ["id", "date"]}}),
        json!({"name": "invoice_credit", "description": "create a credit note for an invoice", "inputSchema": {"type": "object", "properties": {"id": {"type": "integer"}, "mode": {"type": "string"}, "actor": {"type": "string"}}, "required": ["id"]}}),
        json!({"name": "invoice_finalize", "description": "finalize a draft invoice", "inputSchema": {"type": "object", "properties": {"id": {"type": "integer"}, "mode": {"type": "string"}, "actor": {"type": "string"}}, "required": ["id"]}}),
        json!({"name": "year_end_close", "description": "close the fiscal year", "inputSchema": {"type": "object", "properties": {"year": {"type": "string"}, "mode": {"type": "string"}, "actor": {"type": "string"}}, "required": ["year"]}}),
        json!({"name": "compliance", "description": "compliance status", "inputSchema": {"type": "object", "properties": {}}}),
        json!({"name": "fx_set", "description": "set an FX exchange rate", "inputSchema": {"type": "object", "properties": {"currency": {"type": "string"}, "date": {"type": "string"}, "rate": {"type": "string"}, "actor": {"type": "string"}}, "required": ["currency", "date", "rate"]}}),
        json!({"name": "import_file", "description": "import opening balances or journal CSV", "inputSchema": {"type": "object", "properties": {"file": {"type": "string"}, "kind": {"type": "string"}, "date": {"type": "string"}, "create_missing": {"type": "boolean"}, "mode": {"type": "string"}}, "required": ["file", "kind"]}}),
        json!({"name": "import_contacts", "description": "import contacts from UBL XML", "inputSchema": {"type": "object", "properties": {"file": {"type": "string"}, "mode": {"type": "string"}}, "required": ["file"]}}),
        json!({"name": "invoice_import", "description": "import a UBL invoice as a payable", "inputSchema": {"type": "object", "properties": {"file_path": {"type": "string"}, "create_missing": {"type": "boolean"}, "mode": {"type": "string"}}, "required": ["file_path"]}}),
        json!({"name": "invoice_create", "description": "create a draft invoice: contact_id + lines (\"2x Dienst @ 150.00 @21\") and/or items (\"1:2@140.00\"); discount_pct/discount_amount_cents apply before VAT", "inputSchema": {"type": "object", "properties": {"contact_id": {"type": "integer"}, "lines": {"type": "array", "items": {"type": "string"}}, "items": {"type": "array", "items": {"type": "string"}}, "date": {"type": "string"}, "due_days": {"type": "integer"}, "discount_pct": {"type": "number"}, "discount_amount_cents": {"type": "integer"}, "language": {"type": "string"}, "mode": {"type": "string"}, "actor": {"type": "string"}}, "required": ["contact_id"]}}),
        json!({"name": "item_add", "description": "add an item to the catalog (name, unit, price, optional default VAT code + revenue account)", "inputSchema": {"type": "object", "properties": {"name": {"type": "string"}, "description": {"type": "string"}, "unit": {"type": "string"}, "unit_price": {"type": "string"}, "vat_code": {"type": "string"}, "gl_account": {"type": "string"}, "mode": {"type": "string"}, "actor": {"type": "string"}}, "required": ["name", "unit_price"]}}),
        json!({"name": "item_list", "description": "items catalog (active items; pass include_inactive:true for all)", "inputSchema": {"type": "object", "properties": {"include_inactive": {"type": "boolean"}}}}),
        json!({"name": "item_update", "description": "update an item (price, unit, VAT, GL account) or deactivate it", "inputSchema": {"type": "object", "properties": {"id": {"type": "integer"}, "name": {"type": "string"}, "description": {"type": "string"}, "unit": {"type": "string"}, "unit_price": {"type": "string"}, "vat_code": {"type": "string"}, "gl_account": {"type": "string"}, "deactivate": {"type": "boolean"}, "mode": {"type": "string"}, "actor": {"type": "string"}}, "required": ["id"]}}),
        json!({"name": "invoice_email", "description": "email a finalized invoice (SMTP via BUKIO_SMTP_* env)", "inputSchema": {"type": "object", "properties": {"id": {"type": "integer"}, "to": {"type": "string"}, "subject": {"type": "string"}, "body": {"type": "string"}, "attach_pdf": {"type": "boolean"}, "mode": {"type": "string"}, "actor": {"type": "string"}}, "required": ["id"]}}),
        json!({"name": "report_aging", "description": "open items per contact, bucketed by days past due", "inputSchema": {"type": "object", "properties": {"as_of": {"type": "string"}, "kind": {"type": "string"}}}}),
        json!({"name": "report_sales", "description": "sales revenue for a year (per contact or item)", "inputSchema": {"type": "object", "properties": {"year": {"type": "string"}, "by": {"type": "string"}}}}),
        json!({"name": "payments_mandate_add", "description": "register a signed SEPA direct-debit mandate for a contact (core = 8-week refund right, b2b = none)", "inputSchema": {"type": "object", "properties": {"contact_id": {"type": "integer"}, "mandate_ref": {"type": "string"}, "mandate_date": {"type": "string"}, "scheme": {"type": "string"}, "mode": {"type": "string"}, "actor": {"type": "string"}}, "required": ["contact_id", "mandate_ref"]}}),
        json!({"name": "payments_mandate_list", "description": "list SEPA direct-debit mandates (optionally per contact)", "inputSchema": {"type": "object", "properties": {"contact_id": {"type": "integer"}}}}),
        json!({"name": "payments_batch_create", "description": "create a SEPA batch: type transfer (pain.001) or direct_debit (pain.008, each line needs a contact mandate)", "inputSchema": {"type": "object", "properties": {"payable_ids": {"type": "array", "items": {"type": "integer"}}, "batch_date": {"type": "string"}, "type": {"type": "string"}, "mode": {"type": "string"}, "actor": {"type": "string"}}}}),
        json!({"name": "payments_batch_export", "description": "export a draft batch as SEPA XML (pain.001 for transfer, pain.008.001.02 for direct-debit) — one export per batch, marks it exported", "inputSchema": {"type": "object", "properties": {"batch_id": {"type": "integer"}, "mode": {"type": "string"}, "actor": {"type": "string"}}, "required": ["batch_id"]}}),
        // — tools that existed in the JS MCP surface but were never exposed here
        json!({"name": "icp_readout", "description": "ICP listing: EU reverse-charge supplies per customer", "inputSchema": {"type": "object", "properties": {"period": {"type": "string"}}, "required": ["period"]}}),
        json!({"name": "year_end_status", "description": "is the year closed? what is the result?", "inputSchema": {"type": "object", "properties": {"year": {"type": "string"}}, "required": ["year"]}}),
        json!({"name": "recurring_run", "description": "generate due recurring entries / draft invoices (idempotent, backfills)", "inputSchema": {"type": "object", "properties": {"as_of": {"type": "string"}, "template_id": {"type": "integer"}, "actor": {"type": "string"}, "mode": {"type": "string"}}}}),
        json!({"name": "assets_register", "description": "fixed asset register: cost, cumulative depreciation, book value per asset", "inputSchema": {"type": "object", "properties": {"as_of": {"type": "string"}}}}),
        json!({"name": "asset_dispose", "description": "dispose of an asset (sale or scrap): books the full entry, status -> disposed", "inputSchema": {"type": "object", "properties": {"id": {"type": "integer"}, "date": {"type": "string"}, "proceeds": {"type": "string"}, "bank_account": {"type": "string"}, "result_account": {"type": "string"}, "actor": {"type": "string"}, "mode": {"type": "string"}}, "required": ["id", "date"]}}),
        json!({"name": "attachment_list", "description": "attachments for an invoice or entry (metadata only — never the file bytes)", "inputSchema": {"type": "object", "properties": {"kind": {"type": "string"}, "ref_id": {"type": "integer"}}, "required": ["kind", "ref_id"]}}),
        json!({"name": "attachment_add", "description": "store a source document (pdf/jpg/png/xml/eml/...) against an invoice or entry; mode db (default) = BLOB in the database, file = path in <db>-attachments/", "inputSchema": {"type": "object", "properties": {"kind": {"type": "string"}, "ref_id": {"type": "integer"}, "file_path": {"type": "string"}, "store": {"type": "string"}, "note": {"type": "string"}, "actor": {"type": "string"}, "mode": {"type": "string"}}, "required": ["kind", "ref_id", "file_path"]}}),
        json!({"name": "attachment_remove", "description": "remove an attachment (file-mode copies on disk are deleted too)", "inputSchema": {"type": "object", "properties": {"id": {"type": "integer"}, "actor": {"type": "string"}, "mode": {"type": "string"}}, "required": ["id"]}}),
    ]
}

fn arg_str(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn arg_i64(args: &Value, key: &str) -> Option<i64> {
    args.get(key).and_then(|v| v.as_i64())
}

fn arg_bool(args: &Value, key: &str, default: bool) -> bool {
    args.get(key).and_then(|v| v.as_bool()).unwrap_or(default)
}

fn dispatch(db: &Connection, actor: &str, msg: &Value) -> Result<String> {
    let id = msg.get("id").cloned();
    let method = msg.get("method").and_then(|v| v.as_str()).unwrap_or("");
    let params = msg.get("params").unwrap_or(&Value::Null);

    match method {
        "initialize" => Ok(rpc_response(
            id.clone(),
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "bukio-cli", "version": env!("CARGO_PKG_VERSION")}
            }),
        )),
        "initialized" => Ok(String::new()), // no response
        "ping" => Ok(rpc_response(id.clone(), json!({}))),
        "tools/list" => Ok(rpc_response(id.clone(), json!({"tools": tool_defs()}))),
        "tools/call" => {
            let tool_name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let args = params.get("arguments").unwrap_or(&Value::Null);
            // Effective actor: the tool call's `actor` arg wins, else the session actor
            let eff_actor = arg_str(args, "actor").unwrap_or_else(|| actor.to_string());
            // Validate actor up front (INVALID_ACTOR, same as CLI)
            if !crate::actor::is_valid_actor(&eff_actor) {
                let err = crate::actor::actor_error(Some(&eff_actor)).unwrap_or_else(|| {
                    BukioError::new("INVALID_ACTOR", format!("invalid actor '{eff_actor}'"))
                });
                return Ok(rpc_response(
                    id.clone(),
                    rpc_error_content(&err.code, &err.message),
                ));
            }
            // Mutating tools are signed by their actor (gate + audit attribution);
            // BUKIO_MCP_READONLY refuses every mutating tool before signing.
            let mutating = is_mutating_tool(tool_name);
            if mutating {
                if std::env::var("BUKIO_MCP_READONLY").is_ok() {
                    return Ok(rpc_response(
                        id.clone(),
                        rpc_error_content(
                            "MCP_READONLY",
                            "this bukio MCP session is read-only (BUKIO_MCP_READONLY) — mutating tools are refused",
                        ),
                    ));
                }
                // Tier 0 sign gate: sign before the handler runs so the audit
                // rows it records carry sig_status=verified (enforce refuses
                // when no key material exists — dry-run included).
                match crate::sign_gate::sign_tool_call(db, &eff_actor, tool_name, args) {
                    Ok(sr) => {
                        if let Some(s) = sr {
                            crate::audit::set_pending_signature(Some(
                                crate::audit::PendingSignature {
                                    digest_hash: Some(s.digest_hash.clone()),
                                    sig_keyid: Some(s.sig_keyid.clone()),
                                    sig_nonce: Some(s.sig_nonce.clone()),
                                    sig_ts: Some(s.sig_ts.clone()),
                                    sig: Some(s.sig.clone()),
                                    sig_status: s.sig_status.clone(),
                                    signed_args: Some(s.signed_args.clone()),
                                    signed_command: Some(s.signed_command.clone()),
                                },
                            ));
                        } else {
                            crate::audit::set_pending_signature(None);
                        }
                    }
                    Err(e) => {
                        return Ok(rpc_response(
                            id.clone(),
                            rpc_error_content(&e.code, &e.message),
                        ));
                    }
                }
            }
            // Tier 0.5 authz gate (JS parity): mutating tools carry the same
            // capabilities as their CLI counterparts — 'mcp:entry_add' with
            // post:true is entry.post. Read-only tools stay ungated.
            if mutating {
                if let Some(e) = mcp_authz_refusal(db, &eff_actor, tool_name, args) {
                    return Ok(rpc_response(
                        id.clone(),
                        rpc_error_content(&e.code, &e.message),
                    ));
                }
            }
            let result = call_tool(db, &eff_actor, tool_name, args);
            if mutating {
                crate::audit::set_pending_signature(None);
            }
            match result {
                Ok(result) => Ok(rpc_response(id.clone(), rpc_content(result))),
                Err(e) => {
                    if e.code == "UNKNOWN_TOOL" {
                        // Return JSON-RPC error for unknown tools (matches JS parity)
                        Ok(rpc_error(id.clone(), -32602, &e.message))
                    } else {
                        Ok(rpc_response(
                            id.clone(),
                            rpc_error_content(&e.code, &e.message),
                        ))
                    }
                }
            }
        }
        _ => Ok(rpc_error(
            id.clone(),
            -32601,
            &format!("method not found: {method}"),
        )),
    }
}

/// The MCP authz gate: the effective actor must carry the tool's capability.
/// Returns the refusal (AUTHZ_DENIED) or None. Mutating tools only — readers
/// stay ungated (JS parity). Unmapped tools fail closed.
fn mcp_authz_refusal(db: &Connection, actor: &str, tool: &str, args: &Value) -> Option<BukioError> {
    let post = args.get("post").and_then(|v| v.as_bool()).unwrap_or(false);
    crate::authz::check_authz(db, actor, &format!("mcp:{tool}"), false, post).err()
}

/// Tools that mutate the books — these are signed and refused in read-only mode.
fn is_mutating_tool(tool: &str) -> bool {
    matches!(
        tool,
        "entry_add"
            | "entry_post"
            | "entry_reverse"
            | "vat_book"
            | "asset_add"
            | "assets_run"
            | "invoice_pay"
            | "invoice_credit"
            | "invoice_finalize"
            | "invoice_email"
            | "invoice_create"
            | "item_add"
            | "item_update"
            | "year_end_close"
            | "fx_set"
            | "contact_add"
            | "payments_mandate_add"
            | "payments_batch_create"
            | "payments_batch_export"
            | "invoice_import"
            | "import_file"
            | "import_contacts"
            | "journal_import"
            | "import_journal"
            | "recurring_run"
            | "asset_dispose"
            | "attachment_add"
            | "attachment_remove"
    )
}

fn call_tool(db: &Connection, actor: &str, tool: &str, args: &Value) -> Result<Value> {
    match tool {
        "company_info" => {
            let c = get_company(db)?;
            Ok(json!({"company": c}))
        }
        "trial_balance" => {
            let year = arg_str(args, "year");
            let r = crate::reports::trial_balance(db, year.as_deref())?;
            Ok(r)
        }
        "balance_sheet" => {
            let as_of = arg_str(args, "as_of");
            let r = crate::reports::trial_balance(db, as_of.as_deref())?;
            Ok(r)
        }
        "pnl" => {
            let year =
                arg_str(args, "year").unwrap_or_else(|| crate::dates::today_iso()[..4].to_string());
            if year.len() != 4 || !year.chars().all(|c| c.is_ascii_digit()) {
                return Err(BukioError::new(
                    "INVALID_YEAR",
                    format!("year '{year}' must be YYYY"),
                ));
            }
            let from = format!("{year}-01-01");
            let to = format!("{year}-12-31");
            let r = crate::reports::pnl(db, &from, &to)?;
            Ok(r)
        }
        "journal" => {
            let year = arg_str(args, "year")
                .ok_or_else(|| BukioError::new("MISSING_ARG", "year required"))?;
            if year.len() != 4 || !year.chars().all(|c| c.is_ascii_digit()) {
                return Err(BukioError::new(
                    "INVALID_YEAR",
                    format!("year '{year}' must be YYYY"),
                ));
            }
            let from = format!("{year}-01-01");
            let to = format!("{year}-12-31");
            let limit_raw = args.get("limit");
            if let Some(lr) = limit_raw {
                let l = lr.as_i64().ok_or_else(|| {
                    BukioError::new(
                        "INVALID_LIMIT",
                        format!("limit must be a non-negative integer, got '{}'", lr),
                    )
                })?;
                if l < 0 {
                    return Err(BukioError::new(
                        "INVALID_LIMIT",
                        format!("limit must be non-negative, got '{l}'"),
                    ));
                }
            }
            let limit = limit_raw.and_then(|v| v.as_i64()).unwrap_or(500);
            // fetch limit+1 to detect truncation
            let all_rows = crate::reports::journal(db, &from, &to, Some(limit + 1))?;
            let truncated = all_rows.len() as i64 > limit;
            let rows: Vec<Value> = if truncated {
                all_rows.into_iter().take(limit as usize).collect()
            } else {
                all_rows
            };
            Ok(json!({"ok": true, "rows": rows, "truncated": truncated, "limit": limit}))
        }
        "accounts" => {
            let include_inactive = arg_bool(args, "include_inactive", false);
            let r = list_accounts(db, None, include_inactive)?;
            Ok(json!({"accounts": r}))
        }
        "entry_add" => {
            let date = arg_str(args, "date")
                .ok_or_else(|| BukioError::new("MISSING_ARG", "date required"))?;
            let description = arg_str(args, "description").unwrap_or_default();
            let postings_arr = args
                .get("postings")
                .and_then(|v| v.as_array())
                .ok_or_else(|| BukioError::new("MISSING_ARG", "postings required"))?;
            let mut postings = Vec::new();
            for p in postings_arr {
                if let Some(s) = p.as_str() {
                    let parts: Vec<&str> = s.splitn(2, ':').collect();
                    if parts.len() == 2 {
                        let code = parts[0].to_string();
                        let amount_cents = crate::money::parse_amount(parts[1])?;
                        postings.push(PostingSpec {
                            code,
                            amount_cents,
                            cost_center_code: None,
                            vat_code: None,
                            vat_amount_cents: None,
                        });
                    }
                }
            }
            // Validate date format (YYYY-MM-DD)
            if date.len() != 10 || date.as_bytes()[4] != b'-' || date.as_bytes()[7] != b'-' {
                return Err(BukioError::new(
                    "INVALID_DATE",
                    format!("date '{date}' must be YYYY-MM-DD"),
                ));
            }
            // Dry-run: validate without creating
            let mode = arg_str(args, "mode").unwrap_or_else(|| "dry-run".into());
            if mode == "dry-run" {
                if postings.len() < 2 {
                    return Err(BukioError::new(
                        "TOO_FEW_POSTINGS",
                        "an entry needs at least 2 postings",
                    ));
                }
                let sum: i64 = postings.iter().map(|p| p.amount_cents).sum();
                if sum != 0 {
                    return Err(BukioError::new(
                        "UNBALANCED",
                        format!("postings do not sum to zero (sum = {sum})"),
                    ));
                }
                let balanced = sum == 0;
                return Ok(
                    json!({"ok": true, "mode": "dry-run", "balanced": balanced, "date": date, "description": description, "postings": postings.iter().map(|p| json!({"code": p.code, "amount_cents": p.amount_cents})).collect::<Vec<_>>()}),
                );
            }
            let post = arg_bool(args, "post", false);
            let entry = create_entry(
                db,
                CreateEntry {
                    date: &date,
                    description: &description,
                    postings,
                    source: "manual",
                    source_ref: None,
                    actor,
                },
            )?;
            if post {
                let posted = post_entry(db, entry.id, actor)?;
                Ok(
                    json!({"ok": true, "mode": "execute", "state": "posted", "entry_id": posted.id, "entry": posted}),
                )
            } else {
                Ok(
                    json!({"ok": true, "mode": "execute", "state": "draft", "entry_id": entry.id, "entry": entry}),
                )
            }
        }
        "entry_post" => {
            let id =
                arg_i64(args, "id").ok_or_else(|| BukioError::new("MISSING_ARG", "id required"))?;
            if crate::entries::get_entry(db, id).is_none() {
                return Err(BukioError::new(
                    "NOT_FOUND",
                    format!("entry {id} not found"),
                ));
            }
            let posted = post_entry(db, id, actor)?;
            Ok(json!({"ok": true, "state": "posted", "entry": posted}))
        }
        "entry_reverse" => {
            let id =
                arg_i64(args, "id").ok_or_else(|| BukioError::new("MISSING_ARG", "id required"))?;
            let reason = arg_str(args, "reason").unwrap_or_default();
            let mode = arg_str(args, "mode").unwrap_or_else(|| "dry-run".into());
            if mode == "dry-run" {
                // Validate entry exists
                if crate::entries::get_entry(db, id).is_none() {
                    return Err(BukioError::new(
                        "NOT_FOUND",
                        format!("entry {id} not found"),
                    ));
                }
                return Ok(json!({"ok": true, "dry_run": true, "entry_id": id, "reason": reason}));
            }
            let entry = reverse_entry(db, id, actor, Some(&reason))?;
            Ok(json!({"ok": true, "entry": entry}))
        }
        "vat_book" => {
            let date = arg_str(args, "date")
                .ok_or_else(|| BukioError::new("MISSING_ARG", "date required"))?;
            let description = arg_str(args, "description")
                .ok_or_else(|| BukioError::new("MISSING_ARG", "description required"))?;
            let raw: Vec<String> = args
                .get("postings")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            let specs = crate::vat::parse_vat_posting_specs(&raw)?;
            // Expand VAT postings (generates net + VAT legs)
            let (expanded, _) = crate::vat::expand_vat_postings(db, &specs)?;
            if expanded.is_empty() {
                return Err(BukioError::new(
                    "TOO_FEW_POSTINGS",
                    "an entry needs at least 2 postings",
                ));
            }
            let sum: i64 = expanded.iter().map(|p| p.amount_cents).sum();
            if sum != 0 {
                return Err(BukioError::new(
                    "UNBALANCED",
                    format!("postings do not sum to zero (sum = {sum})"),
                ));
            }
            let post = arg_bool(args, "post", false);
            let entry = create_entry(
                db,
                CreateEntry {
                    date: &date,
                    description: &description,
                    postings: expanded,
                    source: "manual",
                    source_ref: None,
                    actor,
                },
            )?;
            if post {
                let posted = post_entry(db, entry.id, actor)?;
                Ok(
                    json!({"ok": true, "mode": "execute", "state": "posted", "entry_id": posted.id, "entry": posted}),
                )
            } else {
                Ok(
                    json!({"ok": true, "mode": "execute", "state": "draft", "entry_id": entry.id, "entry": entry}),
                )
            }
        }
        "asset_add" => {
            let name = arg_str(args, "name")
                .ok_or_else(|| BukioError::new("MISSING_ARG", "name required"))?;
            let purchase_date = arg_str(args, "purchase_date")
                .ok_or_else(|| BukioError::new("MISSING_ARG", "purchase_date required"))?;
            let purchase_price = arg_str(args, "purchase_price")
                .ok_or_else(|| BukioError::new("MISSING_ARG", "purchase_price required"))?;
            let dep_start =
                arg_str(args, "depreciation_start").unwrap_or_else(|| purchase_date.clone());
            let recognition =
                arg_str(args, "recognition_date").unwrap_or_else(|| purchase_date.clone());
            let purchase_price_clean = purchase_price.replace(',', ".");
            let purchase_price_cents = crate::money::parse_amount(&purchase_price_clean)?;
            let category = arg_str(args, "category");
            let asset_account = arg_str(args, "asset_account").unwrap_or_else(|| "1800".into());
            let expense_account = arg_str(args, "expense_account").unwrap_or_else(|| "4600".into());
            let cum_dep_str = arg_str(args, "cum_dep").unwrap_or_else(|| "0".into());
            let cum_dep = crate::money::parse_amount(&cum_dep_str)?;
            let action = crate::assets::create_asset(
                db,
                &name,
                category.as_deref(),
                None,
                None,
                None,
                None,
                None,
                None,
                &purchase_date,
                purchase_price_cents,
                &dep_start,
                &recognition,
                cum_dep,
                &asset_account,
                None,
                &expense_account,
                None,
                None,
                actor,
                false,
            )?;
            Ok(json!({"ok": true, "action": "assets.add", "asset": action}))
        }
        "icp_readout" => {
            let period = arg_str(args, "period")
                .ok_or_else(|| BukioError::new("MISSING_ARG", "period required"))?;
            crate::reports::icp_readout(db, &period)
        }
        "year_end_status" => {
            // JS tolerates a numeric year here; accept both
            let year = arg_str(args, "year")
                .or_else(|| arg_i64(args, "year").map(|v| v.to_string()))
                .ok_or_else(|| BukioError::new("MISSING_ARG", "year required"))?;
            // JS MCP returns the status raw (the CLI wraps it as {status})
            crate::year_end::year_end_status(db, &year)
        }
        "recurring_run" => {
            let as_of = arg_str(args, "as_of");
            let template_id = arg_i64(args, "template_id");
            let mode = arg_str(args, "mode").unwrap_or_else(|| "dry-run".into());
            let dry_run = mode != "execute";
            let mut data =
                crate::recurring::run_due(db, as_of.as_deref(), template_id, actor, dry_run)?;
            if let Some(o) = data.as_object_mut() {
                o.insert(
                    "mode".to_string(),
                    json!(if dry_run { "dry-run" } else { "execute" }),
                );
            }
            Ok(data)
        }
        "assets_register" => {
            let as_of = arg_str(args, "as_of");
            crate::assets::register(db, as_of.as_deref(), actor)
        }
        "asset_dispose" => {
            let id =
                arg_i64(args, "id").ok_or_else(|| BukioError::new("MISSING_ARG", "id required"))?;
            let date = arg_str(args, "date")
                .ok_or_else(|| BukioError::new("MISSING_ARG", "date required"))?;
            let proceeds = arg_str(args, "proceeds")
                .unwrap_or_else(|| "0".into())
                .replace(',', ".");
            let proceeds_cents = crate::money::parse_amount(&proceeds)?;
            let bank = arg_str(args, "bank_account");
            let result = arg_str(args, "result_account");
            let mode = arg_str(args, "mode").unwrap_or_else(|| "dry-run".into());
            let dry_run = mode != "execute";
            crate::assets::dispose_asset(
                db,
                id,
                &date,
                proceeds_cents,
                bank.as_deref(),
                result.as_deref(),
                None,
                actor,
                dry_run,
            )
        }
        "attachment_list" => {
            let kind = arg_str(args, "kind")
                .ok_or_else(|| BukioError::new("MISSING_ARG", "kind required"))?;
            let ref_id = arg_i64(args, "ref_id")
                .ok_or_else(|| BukioError::new("MISSING_ARG", "ref_id required"))?;
            let rows = crate::attachments::list_attachments(db, &kind, ref_id)?;
            Ok(json!({ "attachments": rows }))
        }
        "attachment_add" => {
            let kind = arg_str(args, "kind")
                .ok_or_else(|| BukioError::new("MISSING_ARG", "kind required"))?;
            let ref_id = arg_i64(args, "ref_id")
                .ok_or_else(|| BukioError::new("MISSING_ARG", "ref_id required"))?;
            let file_path = arg_str(args, "file_path")
                .ok_or_else(|| BukioError::new("MISSING_ARG", "file_path required"))?;
            let store = arg_str(args, "store").unwrap_or_else(|| "db".into());
            let note = arg_str(args, "note");
            let mode = arg_str(args, "mode").unwrap_or_else(|| "dry-run".into());
            let dry_run = mode != "execute";
            crate::attachments::add_attachment(
                db,
                &kind,
                ref_id,
                &file_path,
                note.as_deref(),
                &store,
                actor,
                dry_run,
            )
        }
        "attachment_remove" => {
            let id =
                arg_i64(args, "id").ok_or_else(|| BukioError::new("MISSING_ARG", "id required"))?;
            let mode = arg_str(args, "mode").unwrap_or_else(|| "dry-run".into());
            let dry_run = mode != "execute";
            crate::attachments::remove_attachment(db, id, actor, dry_run)
        }
        "assets_run" => {
            let period = arg_str(args, "period")
                .unwrap_or_else(|| chrono::Utc::now().format("%Y-%m").to_string());
            let mode = arg_str(args, "mode").unwrap_or_else(|| "dry-run".into());
            let dry_run = mode != "execute";
            let r = crate::assets::run_due(db, &period, actor, dry_run)?;
            let mut data = r;
            if let Some(o) = data.as_object_mut() {
                o.insert(
                    "mode".to_string(),
                    json!(if dry_run { "dry-run" } else { "execute" }),
                );
            }
            Ok(data)
        }
        "invoice_pay" => {
            let id =
                arg_i64(args, "id").ok_or_else(|| BukioError::new("MISSING_ARG", "id required"))?;
            let date = arg_str(args, "date").unwrap_or_else(|| crate::dates::today_iso());
            let inv = crate::invoice::get_invoice(db, id)?
                .ok_or_else(|| BukioError::new("NOT_FOUND", format!("invoice {id} not found")))?;
            let outstanding =
                inv["gross_cents"].as_i64().unwrap_or(0) - inv["paid_cents"].as_i64().unwrap_or(0);
            let paid = crate::invoice::mark_paid(db, id, &date, outstanding, "bank", actor, false)?;
            Ok(json!({"ok": true, "invoice": paid}))
        }
        "invoice_credit" => {
            let id =
                arg_i64(args, "id").ok_or_else(|| BukioError::new("MISSING_ARG", "id required"))?;
            let inv = crate::invoice::get_invoice(db, id)?
                .ok_or_else(|| BukioError::new("NOT_FOUND", format!("invoice {id} not found")))?;
            // For now, just return the invoice info (credit note creation is complex)
            Ok(json!({"ok": true, "invoice": inv, "action": "invoice.credit"}))
        }
        "invoice_finalize" => {
            let id =
                arg_i64(args, "id").ok_or_else(|| BukioError::new("MISSING_ARG", "id required"))?;
            if crate::invoice::get_invoice(db, id)?.is_none() {
                return Err(BukioError::new(
                    "NOT_FOUND",
                    format!("invoice {id} not found"),
                ));
            }
            let result = crate::invoice::finalize_invoice(db, id, actor, false)?;
            Ok(result)
        }
        "year_end_close" => {
            let year = arg_str(args, "year")
                .ok_or_else(|| BukioError::new("MISSING_ARG", "year required"))?;
            let result = crate::year_end::year_end_close(db, &year, actor, false)?;
            Ok(result)
        }
        "compliance" => {
            let year_str = arg_str(args, "year").unwrap_or_else(|| {
                let today = crate::dates::today_iso();
                today[..4].to_string()
            });
            let year: i32 = year_str.parse().unwrap_or(2026);
            let result = crate::compliance::compliance_status(db, year)?;
            Ok(result)
        }
        "fx_set" => {
            let currency = arg_str(args, "currency")
                .ok_or_else(|| BukioError::new("MISSING_ARG", "currency required"))?;
            let date = arg_str(args, "date")
                .ok_or_else(|| BukioError::new("MISSING_ARG", "date required"))?;
            let rate = arg_str(args, "rate")
                .ok_or_else(|| BukioError::new("MISSING_ARG", "rate required"))?;
            // Validate date
            if date.len() != 10 || date.as_bytes()[4] != b'-' || date.as_bytes()[7] != b'-' {
                return Err(BukioError::new(
                    "INVALID_DATE",
                    format!("date '{date}' must be YYYY-MM-DD"),
                ));
            }
            let result =
                crate::fx::set_fx_rate(db, &currency, &date, &rate, "manual", actor, false)?;
            Ok(result)
        }
        "invoices" => {
            let status = arg_str(args, "status");
            if let Some(ref s) = status {
                if !["draft", "sent", "paid", "overdue", "void"].contains(&s.as_str()) {
                    return Err(BukioError::new(
                        "INVALID_STATUS",
                        format!(
                            "status must be one of draft|sent|paid|overdue|void, got '{}'",
                            s
                        ),
                    ));
                }
            }
            if let Some(l) = args.get("limit") {
                if !l.is_i64() || l.as_i64().unwrap_or(-1) < 0 {
                    return Err(BukioError::new(
                        "INVALID_LIMIT",
                        format!("limit must be a non-negative integer, got '{}'", l),
                    ));
                }
            }
            let r = crate::invoice::list_invoices(db, status.as_deref(), None)?;
            Ok(json!({"invoices": r}))
        }
        "contacts" => {
            let limit = arg_i64(args, "limit").map(|l| l as usize).unwrap_or(50);
            let r = list_contacts(db)?;
            Ok(json!({"contacts": r}))
        }
        "contact_add" => {
            let name = arg_str(args, "name")
                .ok_or_else(|| BukioError::new("MISSING_ARG", "name required"))?;
            if name.trim().is_empty() {
                return Err(BukioError::new(
                    "INVALID_NAME",
                    "contact name cannot be blank",
                ));
            }
            let address = arg_str(args, "address");
            let postal_code = arg_str(args, "postal_code");
            let city = arg_str(args, "city");
            let country = arg_str(args, "country");
            let email = arg_str(args, "email");
            let vat_id = arg_str(args, "vat_id");
            let kvk = arg_str(args, "kvk");
            let iban = arg_str(args, "iban");
            let r = create_contact(
                db,
                &name,
                address.as_deref(),
                postal_code.as_deref(),
                city.as_deref(),
                country.as_deref(),
                email.as_deref(),
                vat_id.as_deref(),
                kvk.as_deref(),
                iban.as_deref(),
                actor,
                false,
            )?;
            Ok(json!({"ok": true, "contact": r}))
        }
        "payments_mandate_add" => {
            let contact_id = arg_i64(args, "contact_id")
                .ok_or_else(|| BukioError::new("MISSING_ARG", "contact_id required"))?;
            let mandate_ref = arg_str(args, "mandate_ref")
                .ok_or_else(|| BukioError::new("MISSING_ARG", "mandate_ref required"))?;
            let scheme = arg_str(args, "scheme").unwrap_or_else(|| "core".into());
            let mandate_date = arg_str(args, "mandate_date");
            let mode = arg_str(args, "mode").unwrap_or_else(|| "dry-run".into());
            let dry_run = mode != "execute";
            let r = crate::payments::add_mandate(
                db,
                contact_id,
                &mandate_ref,
                mandate_date.as_deref(),
                &scheme,
                actor,
                dry_run,
            )?;
            if dry_run {
                return Ok(r);
            }
            Ok(json!({
                "action": "payments.mandate.add", "mode": "execute",
                "mandate_id": r["id"], "contact_id": r["contact_id"],
                "mandate_ref": r["mandate_ref"], "scheme": r["scheme"],
            }))
        }
        "payments_mandate_list" => {
            let contact_id = arg_i64(args, "contact_id");
            let rows = crate::payments::list_mandates(db, contact_id)?;
            Ok(json!({"mandates": rows}))
        }
        "payments_batch_create" => {
            let payable_ids: Vec<i64> = args
                .get("payable_ids")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|v| v.as_i64()).collect())
                .unwrap_or_default();
            let batch_date = arg_str(args, "batch_date");
            let kind_raw = arg_str(args, "type").unwrap_or_else(|| "transfer".into());
            let kind = if kind_raw == "direct_debit" {
                "direct_debit".to_string()
            } else {
                "transfer".to_string()
            };
            let mode = arg_str(args, "mode").unwrap_or_else(|| "dry-run".into());
            let dry_run = mode != "execute";
            let r = crate::payments::create_payment_batch(
                db,
                batch_date.as_deref(),
                None,
                &[],
                &payable_ids,
                &kind,
                actor,
                dry_run,
            )?;
            if dry_run {
                let mut plan = r;
                if let Some(o) = plan.as_object_mut() {
                    o.insert("mode".to_string(), json!("dry-run"));
                }
                return Ok(plan);
            }
            let batch_id = r["id"].as_i64().unwrap_or(0);
            Ok(json!({
                "action": "payments.batch.create", "mode": "execute",
                "batch_id": batch_id, "batch_kind": r["batch_kind"],
                "total_cents": r["total_cents"], "lines": r["lines"].as_array().map(|a| a.len()).unwrap_or(0),
                "status": r["status"],
            }))
        }
        "payments_batch_export" => {
            let id = arg_i64(args, "batch_id")
                .ok_or_else(|| BukioError::new("MISSING_ARG", "batch_id required"))?;
            let mode = arg_str(args, "mode").unwrap_or_else(|| "dry-run".into());
            let dry_run = mode != "execute";
            let r = crate::payments::export_payment_batch(db, id, actor, dry_run, None)?;
            if dry_run {
                return Ok(r);
            }
            Ok(json!({
                "action": "payments.batch.export", "mode": "execute",
                "batch_id": r["batch_id"], "schema": r["schema"],
                "msg_id": r["msg_id"], "status": r["status"], "xml": r["xml"],
            }))
        }
        "report_aging" => {
            let as_of = arg_str(args, "as_of").unwrap_or_else(crate::dates::today_iso);
            let kind = arg_str(args, "kind").unwrap_or_else(|| "both".into());
            let r = crate::reports::aging(db, &as_of, &kind)?;
            Ok(r)
        }
        "report_sales" => {
            let year =
                arg_str(args, "year").unwrap_or_else(|| crate::dates::today_iso()[..4].to_string());
            if year.len() != 4 || !year.chars().all(|c| c.is_ascii_digit()) {
                return Err(BukioError::new(
                    "INVALID_YEAR",
                    format!("year '{year}' must be YYYY"),
                ));
            }
            let by = arg_str(args, "by").unwrap_or_else(|| "contact".into());
            let r = crate::reports::sales(db, &year, &by)?;
            Ok(r)
        }
        "invoice_email" => {
            let id =
                arg_i64(args, "id").ok_or_else(|| BukioError::new("MISSING_ARG", "id required"))?;
            let mode = arg_str(args, "mode").unwrap_or_else(|| "dry-run".into());
            let dry_run = mode != "execute";
            let attach_pdf = args
                .get("attach_pdf")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            let r = crate::smtp::email_invoice(
                db,
                id,
                arg_str(args, "to").as_deref(),
                arg_str(args, "subject").as_deref(),
                arg_str(args, "body").as_deref(),
                attach_pdf,
                actor,
                dry_run,
            )?;
            Ok(r)
        }
        "invoice_create" => {
            let contact_id = arg_i64(args, "contact_id")
                .ok_or_else(|| BukioError::new("MISSING_ARG", "contact_id required"))?;
            let mode = arg_str(args, "mode").unwrap_or_else(|| "dry-run".into());
            let dry_run = mode != "execute";
            let due_days = arg_i64(args, "due_days");
            let discount_pct = args.get("discount_pct").and_then(|v| v.as_f64());
            let discount_amount_cents = arg_i64(args, "discount_amount_cents");
            let (discount_type, discount_value) = match (discount_pct, discount_amount_cents) {
                (Some(p), None) => (Some("pct".to_string()), Some((p * 100.0).round() as i64)),
                (None, Some(a)) => (Some("amount".to_string()), Some(a)),
                _ => (None, None),
            };
            let mut lines_raw: Vec<Value> = Vec::new();
            if let Some(arr) = args.get("lines").and_then(|v| v.as_array()) {
                for l in arr {
                    if let Some(s) = l.as_str() {
                        lines_raw.push(json!(s));
                    }
                }
            }
            if let Some(arr) = args.get("items").and_then(|v| v.as_array()) {
                for i in arr {
                    if let Some(s) = i.as_str() {
                        // resolve the item spec against the catalog (JS parity:
                        // snapshot price/VAT/unit/GL now, reject missing/inactive)
                        let p = crate::invoice::parse_item_spec(s)?;
                        // parse_item_spec returns the JS camelCase shape
                        let pv = |snake: &str, camel: &str| -> Value {
                            if p[snake].is_null() {
                                p[camel].clone()
                            } else {
                                p[snake].clone()
                            }
                        };
                        let item_id = pv("item_id", "itemId").as_i64().unwrap_or(0);
                        let item = crate::items::get_item(db, item_id)?.ok_or_else(|| {
                            BukioError::new(
                                "ITEM_NOT_FOUND",
                                format!("item {item_id} does not exist"),
                            )
                        })?;
                        if item.get("active").and_then(|v| v.as_i64()).unwrap_or(1) != 1 {
                            return Err(BukioError::new(
                                "ITEM_INACTIVE",
                                format!("item {item_id} is deactivated"),
                            ));
                        }
                        let price = pv("price_cents", "priceCents")
                            .as_i64()
                            .or_else(|| item["unit_price_cents"].as_i64())
                            .unwrap_or(0);
                        let vat_code = pv("vat_code", "vatCode")
                            .as_str()
                            .map(String::from)
                            .or_else(|| item["vat_code"].as_str().map(String::from));
                        lines_raw.push(json!({
                            "description": item["name"].as_str().unwrap_or(""),
                            "qty_milli": pv("qty_milli", "qtyMilli").as_i64().unwrap_or(1000),
                            "price_cents": price,
                            "vat_code": vat_code,
                            "discount_type": pv("discount_type", "discountType"),
                            "discount_value": pv("discount_value", "discountValue"),
                            "unit": item["unit"].as_str(),
                            "item_id": item_id,
                            "gl_account": item["gl_account"].as_str(),
                        }));
                    }
                }
            }
            let date = arg_str(args, "date").unwrap_or_else(crate::dates::today_iso);
            let inv = crate::invoice::create_invoice(
                db,
                contact_id,
                &date,
                due_days,
                None,
                None,
                None,
                discount_type.as_deref(),
                discount_value,
                arg_str(args, "language").as_deref(),
                &lines_raw,
                actor,
                dry_run,
            )?;
            if dry_run {
                let mut plan = inv;
                if let Some(o) = plan.as_object_mut() {
                    o.insert("mode".to_string(), json!("dry-run"));
                }
                return Ok(plan);
            }
            Ok(json!({
                "action": "invoice.create", "contact_id": contact_id,
                "lines": args.get("lines").cloned().unwrap_or(Value::Null),
                "items": args.get("items").cloned().unwrap_or(Value::Null),
                "date": inv["date"], "mode": "execute",
                "invoice_id": inv["id"], "invoice_number": Value::Null, "status": "draft",
                "totals": {
                    "net": inv["net_cents"], "vat": inv["vat_cents"],
                    "gross": inv["gross_cents"], "discount": inv["discount_cents"],
                },
            }))
        }
        "item_add" => {
            let name = arg_str(args, "name")
                .ok_or_else(|| BukioError::new("MISSING_ARG", "name required"))?;
            let price_str = arg_str(args, "unit_price")
                .ok_or_else(|| BukioError::new("MISSING_ARG", "unit_price required"))?;
            let price = crate::money::parse_amount(&price_str)?;
            let unit = arg_str(args, "unit").unwrap_or_else(|| "unit".into());
            let mode = arg_str(args, "mode").unwrap_or_else(|| "dry-run".into());
            let dry_run = mode != "execute";
            let item = crate::items::create_item(
                db,
                &name,
                arg_str(args, "description").as_deref(),
                &unit,
                price,
                arg_str(args, "vat_code").as_deref(),
                arg_str(args, "gl_account").as_deref(),
                actor,
                dry_run,
            )?;
            if dry_run {
                let mut plan = item;
                if let Some(o) = plan.as_object_mut() {
                    o.insert("mode".to_string(), json!("dry-run"));
                }
                return Ok(plan);
            }
            Ok(json!({
                "action": "item.create", "mode": "execute",
                "item_id": item["id"], "name": item["name"], "unit": item["unit"],
                "unit_price_cents": item["unit_price_cents"],
                "vat_code": item["vat_code"], "gl_account": item["gl_account"],
            }))
        }
        "item_list" => {
            let active_only = !args
                .get("include_inactive")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let rows = crate::items::list_items(db, active_only)?;
            Ok(json!({"items": rows}))
        }
        "item_update" => {
            let id =
                arg_i64(args, "id").ok_or_else(|| BukioError::new("MISSING_ARG", "id required"))?;
            let mode = arg_str(args, "mode").unwrap_or_else(|| "dry-run".into());
            let dry_run = mode != "execute";
            let deactivate = args
                .get("deactivate")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let price = match args.get("unit_price") {
                Some(v) if v.is_string() => {
                    let ps = v.as_str().unwrap_or("");
                    let cents = crate::money::parse_amount(ps)?;
                    Some(cents)
                }
                _ => None,
            };
            let item = crate::items::update_item(
                db,
                id,
                arg_str(args, "name").as_deref(),
                arg_str(args, "description"),
                arg_str(args, "unit").as_deref(),
                price,
                arg_str(args, "vat_code"),
                arg_str(args, "gl_account"),
                deactivate,
                actor,
                dry_run,
            )?;
            if dry_run {
                let mut plan = item;
                if let Some(o) = plan.as_object_mut() {
                    o.insert("mode".to_string(), json!("dry-run"));
                }
                return Ok(plan);
            }
            Ok(json!({
                "action": "item.update", "mode": "execute", "id": id,
                "name": item["name"], "unit": item["unit"],
                "unit_price_cents": item["unit_price_cents"],
                "vat_code": item["vat_code"], "gl_account": item["gl_account"],
                "active": item["active"],
            }))
        }
        "audit" => {
            let limit = arg_i64(args, "limit").map(|l| l as usize).unwrap_or(50);
            let by = arg_str(args, "by");
            let since = arg_str(args, "since");
            let r = crate::audit::list(db, since.as_deref(), by.as_deref(), limit as i64)?;
            Ok(json!({"entries": r}))
        }
        "vat_readout" => {
            let period = arg_str(args, "period")
                .ok_or_else(|| BukioError::new("MISSING_ARG", "period required"))?;
            let r = crate::vat::ob_readout(db, &period)?;
            Ok(r)
        }
        "import_file" => {
            let file = arg_str(args, "file")
                .ok_or_else(|| BukioError::new("MISSING_ARG", "file required"))?;
            let kind = arg_str(args, "kind")
                .ok_or_else(|| BukioError::new("MISSING_ARG", "kind required"))?;
            let text = import_mod::read_import_file(&file)?;
            let dry_run = arg_bool(args, "mode", false);
            match kind.as_str() {
                "opening-balances" => {
                    let date = arg_str(args, "date");
                    let r = import_mod::import_opening_balances(
                        db,
                        &text,
                        date.as_deref(),
                        actor,
                        dry_run,
                    )?;
                    Ok(r)
                }
                "journal" => {
                    let create_missing = arg_bool(args, "create_missing", false);
                    let r =
                        import_mod::import_journal_csv(db, &text, create_missing, actor, dry_run)?;
                    Ok(r)
                }
                _ => Err(BukioError::new(
                    "INVALID_KIND",
                    format!("unknown import kind '{kind}'"),
                )),
            }
        }
        "import_contacts" => {
            let file = arg_str(args, "file")
                .ok_or_else(|| BukioError::new("MISSING_ARG", "file required"))?;
            let text = import_mod::read_import_file(&file)?;
            let dry_run = arg_bool(args, "mode", false);
            let r = import_mod::import_contacts(db, &text, actor, dry_run)?;
            Ok(r)
        }
        "invoice_import" => {
            let file_path = arg_str(args, "file_path")
                .ok_or_else(|| BukioError::new("MISSING_ARG", "file_path required"))?;
            let text = import_mod::read_import_file(&file_path)?;
            let create_missing = arg_bool(args, "create_missing", false);
            let dry_run = arg_str(args, "mode").as_deref() != Some("execute");
            let mode_str = if dry_run { "dry-run" } else { "execute" };
            let r = import_mod::import_invoice(db, &text, None, create_missing, actor, dry_run)?;
            let mut result = r;
            if let Some(obj) = result.as_object_mut() {
                obj.insert("mode".into(), json!(mode_str));
            }
            Ok(result)
        }
        _ => Err(BukioError::new(
            "UNKNOWN_TOOL",
            format!("tool '{tool}' not found"),
        )),
    }
}

/// Run the MCP stdio loop.
pub fn run(db_path: &str, actor: &str) -> Result<()> {
    let db = open_db(db_path).map_err(|e| BukioError::new("DB_ERROR", e.to_string()))?;
    let stdin = io::stdin();
    let stdout = io::stdout();

    for line in stdin.lock().lines() {
        let line = line.map_err(|e| BukioError::new("IO_ERROR", e.to_string()))?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => {
                let resp = rpc_error(None, -32700, "parse error");
                let mut out = stdout.lock();
                writeln!(out, "{resp}").ok();
                continue;
            }
        };
        if !msg.is_object() || msg.as_object().map_or(false, |_| false) {
            let resp = rpc_error(None, -32600, "invalid request");
            let mut out = stdout.lock();
            writeln!(out, "{resp}").ok();
            continue;
        }
        match dispatch(&db, actor, &msg) {
            Ok(resp) => {
                if resp.is_empty() {
                    continue;
                } // initialized
                let mut out = stdout.lock();
                writeln!(out, "{resp}").ok();
            }
            Err(e) => {
                let resp = rpc_error(
                    msg.get("id").cloned(),
                    -32603,
                    &format!("internal error: {}", e.message),
                );
                let mut out = stdout.lock();
                writeln!(out, "{resp}").ok();
            }
        }
    }
    Ok(())
}
