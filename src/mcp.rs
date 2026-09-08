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
    json!({"content": [{"type": "text", "text": result.to_string()}], "isError": false})
}

fn rpc_error_content(code: &str, message: &str) -> Value {
    json!({"content": [{"type": "text", "text": json!({"ok": false, "error": {"code": code, "message": message}}).to_string()}], "isError": true})
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
        json!({"name": "pnl", "description": "profit & loss for a year", "inputSchema": {"type": "object", "properties": {"year": {"type": "string"}}}}),
        json!({"name": "journal", "description": "journal export for a year", "inputSchema": {"type": "object", "properties": {"year": {"type": "string"}, "limit": {"type": "integer"}}}}),
        json!({"name": "accounts", "description": "chart of accounts", "inputSchema": {"type": "object", "properties": {"include_inactive": {"type": "boolean"}}}}),
        json!({"name": "entry_add", "description": "create a journal entry", "inputSchema": {"type": "object", "properties": {"date": {"type": "string"}, "description": {"type": "string"}, "postings": {"type": "array", "items": {"type": "string"}}, "post": {"type": "boolean"}, "mode": {"type": "string"}}, "required": ["date", "description", "postings"]}}),
        json!({"name": "entry_post", "description": "post a draft entry", "inputSchema": {"type": "object", "properties": {"id": {"type": "integer"}, "mode": {"type": "string"}}, "required": ["id"]}}),
        json!({"name": "entry_reverse", "description": "reverse a posted entry", "inputSchema": {"type": "object", "properties": {"id": {"type": "integer"}, "reason": {"type": "string"}, "mode": {"type": "string"}}, "required": ["id"]}}),
        json!({"name": "invoices", "description": "list invoices", "inputSchema": {"type": "object", "properties": {"status": {"type": "string"}, "limit": {"type": "integer"}}}}),
        json!({"name": "contacts", "description": "list contacts", "inputSchema": {"type": "object", "properties": {"limit": {"type": "integer"}}}}),
        json!({"name": "contact_add", "description": "create a contact", "inputSchema": {"type": "object", "properties": {"name": {"type": "string"}, "address": {"type": "string"}, "postal_code": {"type": "string"}, "city": {"type": "string"}, "country": {"type": "string"}, "email": {"type": "string"}, "vat_id": {"type": "string"}, "kvk": {"type": "string"}, "iban": {"type": "string"}, "mode": {"type": "string"}}, "required": ["name"]}}),
        json!({"name": "audit", "description": "append-only audit log", "inputSchema": {"type": "object", "properties": {"limit": {"type": "integer"}}}}),
        json!({"name": "vat_readout", "description": "VAT return fields 1a-5d", "inputSchema": {"type": "object", "properties": {"period": {"type": "string"}}, "required": ["period"]}}),
        json!({"name": "import_file", "description": "import opening balances or journal CSV", "inputSchema": {"type": "object", "properties": {"file": {"type": "string"}, "kind": {"type": "string"}, "date": {"type": "string"}, "create_missing": {"type": "boolean"}, "mode": {"type": "string"}}, "required": ["file", "kind"]}}),
        json!({"name": "import_contacts", "description": "import contacts from UBL XML", "inputSchema": {"type": "object", "properties": {"file": {"type": "string"}, "mode": {"type": "string"}}, "required": ["file"]}}),
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
            match call_tool(db, actor, tool_name, args) {
                Ok(result) => Ok(rpc_response(id.clone(), rpc_content(result))),
                Err(e) => Ok(rpc_response(
                    id.clone(),
                    rpc_error_content(&e.code, &e.message),
                )),
            }
        }
        _ => Ok(rpc_error(
            id.clone(),
            -32601,
            &format!("method not found: {method}"),
        )),
    }
}

fn call_tool(db: &Connection, actor: &str, tool: &str, args: &Value) -> Result<Value> {
    match tool {
        "company_info" => {
            let c = get_company(db)?;
            Ok(json!({"ok": true, "data": c}))
        }
        "trial_balance" => {
            let year = arg_str(args, "year");
            let r = crate::reports::trial_balance(db, year.as_deref())?;
            Ok(json!({"ok": true, "data": r}))
        }
        "balance_sheet" => {
            let as_of = arg_str(args, "as_of");
            let r = crate::reports::trial_balance(db, as_of.as_deref())?;
            Ok(json!({"ok": true, "data": r}))
        }
        "pnl" => {
            let year =
                arg_str(args, "year").unwrap_or_else(|| crate::dates::today_iso()[..4].to_string());
            let from = format!("{year}-01-01");
            let to = format!("{year}-12-31");
            let r = crate::reports::pnl(db, &from, &to)?;
            Ok(json!({"ok": true, "data": r}))
        }
        "journal" => {
            let year =
                arg_str(args, "year").unwrap_or_else(|| crate::dates::today_iso()[..4].to_string());
            let from = format!("{year}-01-01");
            let to = format!("{year}-12-31");
            let limit = arg_i64(args, "limit");
            let r = crate::reports::journal(db, &from, &to, limit)?;
            Ok(json!({"ok": true, "data": r}))
        }
        "accounts" => {
            let include_inactive = arg_bool(args, "include_inactive", false);
            let r = list_accounts(db, None, include_inactive)?;
            Ok(json!({"ok": true, "data": r}))
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
                        let amount: i64 = parts[1].parse().map_err(|_| {
                            BukioError::new("INVALID_AMOUNT", format!("bad amount in '{s}'"))
                        })?;
                        postings.push(PostingSpec {
                            code,
                            amount_cents: amount,
                            cost_center_code: None,
                        });
                    }
                }
            }
            let post = arg_bool(args, "post", false);
            let entry = create_entry(
                db,
                CreateEntry {
                    date: &date,
                    description: &description,
                    postings,
                    source: "mcp",
                    source_ref: None,
                    actor,
                },
            )?;
            if post {
                let posted = post_entry(db, entry.id, actor)?;
                Ok(json!({"ok": true, "entry": posted}))
            } else {
                Ok(json!({"ok": true, "entry": entry}))
            }
        }
        "entry_post" => {
            let id =
                arg_i64(args, "id").ok_or_else(|| BukioError::new("MISSING_ARG", "id required"))?;
            let posted = post_entry(db, id, actor)?;
            Ok(json!({"ok": true, "entry": posted}))
        }
        "entry_reverse" => {
            let id =
                arg_i64(args, "id").ok_or_else(|| BukioError::new("MISSING_ARG", "id required"))?;
            let reason = arg_str(args, "reason").unwrap_or_default();
            let entry = reverse_entry(db, id, actor, Some(&reason))?;
            Ok(json!({"ok": true, "entry": entry}))
        }
        "invoices" => {
            let status = arg_str(args, "status");
            let limit = arg_i64(args, "limit").map(|l| l as usize).unwrap_or(50);
            let r = crate::invoice::list_invoices(db, status.as_deref(), None)?;
            Ok(json!({"ok": true, "data": r}))
        }
        "contacts" => {
            let limit = arg_i64(args, "limit").map(|l| l as usize).unwrap_or(50);
            let r = list_contacts(db)?;
            Ok(json!({"ok": true, "data": r}))
        }
        "contact_add" => {
            let name = arg_str(args, "name")
                .ok_or_else(|| BukioError::new("MISSING_ARG", "name required"))?;
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
        "audit" => {
            let limit = arg_i64(args, "limit").map(|l| l as usize).unwrap_or(50);
            let r = crate::audit::list(db, None, None, limit as i64)?;
            Ok(json!({"ok": true, "data": r}))
        }
        "vat_readout" => {
            let period = arg_str(args, "period")
                .ok_or_else(|| BukioError::new("MISSING_ARG", "period required"))?;
            let r = crate::vat::ob_readout(db, &period)?;
            Ok(json!({"ok": true, "data": r}))
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
                    Ok(json!({"ok": true, "data": r}))
                }
                "journal" => {
                    let create_missing = arg_bool(args, "create_missing", false);
                    let r =
                        import_mod::import_journal_csv(db, &text, create_missing, actor, dry_run)?;
                    Ok(json!({"ok": true, "data": r}))
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
            Ok(json!({"ok": true, "data": r}))
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
