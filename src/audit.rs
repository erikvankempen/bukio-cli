// bukio-cli — agent-first double-entry bookkeeping for SMEs.
// Copyright (c) 2026 Erik van Kempen.
// SPDX-License-Identifier: Apache-2.0
//
// Audit log (mirrors src/audit/index.js): append-only record of every
// mutation, signature plumbing, trail verification.

use crate::actor::{get_key_by_keyid, now_iso};
use crate::canonical::build_digest;
use crate::money::{BukioError, Result};
use crate::sign::verify;
use rusqlite::Connection;
use serde_json::{json, Value};

/// The pending signature bundle set by the sign gate before dispatch.
#[derive(Debug, Clone, Default)]
pub struct PendingSignature {
    pub digest_hash: Option<String>,
    pub sig_keyid: Option<String>,
    pub sig_nonce: Option<String>,
    pub sig_ts: Option<String>,
    pub sig: Option<String>,
    pub sig_status: String,
    pub signed_args: Option<Value>,
    pub signed_command: Option<String>,
}

/// Rust port runs one command per process: the pending signature is global
/// state for this process's audit rows (same lifetime as the JS module
/// variable). ponytail: global for process lifetime, fine for a CLI.
static PENDING: std::sync::Mutex<Option<PendingSignature>> = std::sync::Mutex::new(None);

pub fn set_pending_signature(sig: Option<PendingSignature>) {
    *PENDING.lock().unwrap() = sig;
}

pub fn pending_signature() -> Option<PendingSignature> {
    PENDING.lock().unwrap().clone()
}

pub struct RecordArgs<'a> {
    pub actor: &'a str,
    pub action: &'a str,
    pub command: Option<&'a str>,
    pub args: Option<Value>,
    pub outcome: &'a str,
    pub entry_ids: Vec<i64>,
}

pub fn record(db: &Connection, a: RecordArgs<'_>) -> Result<()> {
    let s = pending_signature().unwrap_or_default();
    let stored_args = s.signed_args.clone().or(a.args);
    let stored_command = s.signed_command.clone().or_else(|| a.command.map(String::from));
    db.execute(
        "INSERT INTO audit_log (actor, action, command, args_json, outcome, entry_ids,
                                digest_hash, sig_keyid, sig_nonce, sig_ts, sig, sig_status)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        rusqlite::params![
            a.actor,
            a.action,
            stored_command,
            stored_args.as_ref().map(|v| serde_json::to_string(v).unwrap()),
            a.outcome,
            if a.entry_ids.is_empty() {
                None
            } else {
                Some(serde_json::to_string(&a.entry_ids).unwrap())
            },
            s.digest_hash,
            s.sig_keyid,
            s.sig_nonce,
            s.sig_ts,
            s.sig,
            if s.sig_status.is_empty() { "unsigned".to_string() } else { s.sig_status.clone() },
        ],
    )
    .map_err(|e| BukioError::new("AUDIT_WRITE_FAILED", e.to_string()))?;
    Ok(())
}

pub fn classify_row(db: &Connection, row: &AuditRow) -> &'static str {
    if row.sig_status == "unsigned" || (row.digest_hash.is_none() && row.sig.is_none()) {
        return "unsigned";
    }
    let args: Value = match &row.args_json {
        Some(s) => serde_json::from_str(s).unwrap_or(Value::Null),
        None => Value::Null,
    };
    let recomputed = build_digest(
        &row.actor,
        row.command.as_deref().unwrap_or(""),
        &args,
        row.sig_ts.as_deref().unwrap_or(""),
        row.sig_nonce.as_deref().unwrap_or(""),
    );
    if row.digest_hash.as_deref() != Some(recomputed.as_str()) {
        return "tampered";
    }
    let Some(key) = get_key_by_keyid(db, row.sig_keyid.as_deref().unwrap_or("")) else {
        return "unknown-key";
    };
    if !verify(
        recomputed.as_bytes(),
        row.sig.as_deref().unwrap_or(""),
        &key.public_key,
    ) {
        return "invalid-signature";
    }
    if key.revoked_at.is_some() {
        return "revoked";
    }
    "ok"
}

#[derive(Debug, Clone, Default)]
pub struct AuditRow {
    pub id: i64,
    pub ts: String,
    pub actor: String,
    pub action: String,
    pub command: Option<String>,
    pub args_json: Option<String>,
    pub sig_status: String,
    pub sig_keyid: Option<String>,
    pub sig_nonce: Option<String>,
    pub sig_ts: Option<String>,
    pub sig: Option<String>,
    pub digest_hash: Option<String>,
}

pub fn verify_trail(db: &Connection) -> Result<Value> {
    let mut stmt = db
        .prepare("SELECT id, ts, actor, action, command, args_json, sig_status, sig_keyid, sig_nonce, sig_ts, sig, digest_hash FROM audit_log ORDER BY id")
        .map_err(|e| BukioError::new("AUDIT_READ_FAILED", e.to_string()))?;
    let rows: Vec<AuditRow> = stmt
        .query_map([], |r| {
            Ok(AuditRow {
                id: r.get(0)?,
                ts: r.get(1)?,
                actor: r.get(2)?,
                action: r.get(3)?,
                command: r.get(4)?,
                args_json: r.get(5)?,
                sig_status: r.get(6)?,
                sig_keyid: r.get(7)?,
                sig_nonce: r.get(8)?,
                sig_ts: r.get(9)?,
                sig: r.get(10)?,
                digest_hash: r.get(11)?,
            })
        })
        .map_err(|e| BukioError::new("AUDIT_READ_FAILED", e.to_string()))?
        .filter_map(|r| r.ok())
        .collect();
    let mut summary = json!({
        "total": rows.len(), "ok": 0, "unsigned": 0, "revoked": 0,
        "tampered": 0, "invalid_signature": 0, "unknown_key": 0,
    });
    let checked: Vec<Value> = rows
        .iter()
        .map(|row| {
            let status = classify_row(db, row);
            let key = status.replace('-', "_");
            summary[&key] = Value::from(summary[&key].as_i64().unwrap_or(0) + 1);
            json!({
                "id": row.id, "ts": row.ts, "actor": row.actor, "action": row.action,
                "command": row.command, "sig_status": row.sig_status, "status": status,
            })
        })
        .collect();
    Ok(json!({ "summary": summary, "rows": checked }))
}

pub fn list(db: &Connection, since: Option<&str>, actor: Option<&str>, limit: i64) -> Result<Vec<Value>> {
    if limit < 0 {
        return Err(BukioError::new(
            "INVALID_LIMIT",
            format!("limit must be a non-negative integer, got '{limit}'"),
        ));
    }
    let mut clauses = Vec::new();
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    if let Some(s) = since {
        clauses.push("ts >= ?");
        params.push(Box::new(s.to_string()));
    }
    if let Some(a) = actor {
        clauses.push("actor = ?");
        params.push(Box::new(a.to_string()));
    }
    let where_sql = if clauses.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", clauses.join(" AND "))
    };
    let sql = format!(
        "SELECT id, ts, actor, action, command, args_json, outcome, entry_ids, digest_hash, sig_keyid, sig_nonce, sig_ts, sig, sig_status FROM audit_log {where_sql} ORDER BY id DESC LIMIT ?"
    );
    params.push(Box::new(limit));
    let mut stmt = db
        .prepare(&sql)
        .map_err(|e| BukioError::new("AUDIT_READ_FAILED", e.to_string()))?;
    let rows = stmt
        .query_map(params_refs(&params).as_slice(), |r| {
            let args_json: Option<String> = r.get(5)?;
            let entry_ids: Option<String> = r.get(7)?;
            Ok(json!({
                "id": r.get::<_, i64>(0)?,
                "ts": r.get::<_, String>(1)?,
                "actor": r.get::<_, String>(2)?,
                "action": r.get::<_, String>(3)?,
                "command": r.get::<_, Option<String>>(4)?,
                "args_json": args_json,
                "outcome": r.get::<_, String>(6)?,
                "entry_ids": entry_ids.and_then(|s| serde_json::from_str::<Vec<i64>>(&s).ok()).unwrap_or_default(),
                "digest_hash": r.get::<_, Option<String>>(8)?,
                "sig_keyid": r.get::<_, Option<String>>(9)?,
                "sig_nonce": r.get::<_, Option<String>>(10)?,
                "sig_ts": r.get::<_, Option<String>>(11)?,
                "sig": r.get::<_, Option<String>>(12)?,
                "sig_status": r.get::<_, String>(13)?,
                "args": args_json.and_then(|s| serde_json::from_str::<Value>(&s).ok()),
            }))
        })
        .map_err(|e| BukioError::new("AUDIT_READ_FAILED", e.to_string()))?
        .filter_map(|r| r.ok())
        .collect();
    Ok(rows)
}


/// ponytail: dyn-param helper — rusqlite 0.32 has no params_from_vec; refs
/// satisfy Params for &[&dyn ToSql].
fn params_refs(params: &[Box<dyn rusqlite::types::ToSql>]) -> Vec<&dyn rusqlite::types::ToSql> {
    params.iter().map(|p| p.as_ref()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_db;

    #[test]
    fn record_and_verify_trail_unsigned() {
        let db = open_db(":memory:").unwrap();
        record(&db, RecordArgs {
            actor: "agent:test", action: "entry.create", command: Some("entry add"),
            args: Some(json!({"date": "2026-01-01"})), outcome: "ok", entry_ids: vec![1],
        })
        .unwrap();
        let out = verify_trail(&db).unwrap();
        assert_eq!(out["summary"]["total"], 1);
        assert_eq!(out["summary"]["unsigned"], 1);
        assert_eq!(out["rows"][0]["status"], "unsigned");
    }

    #[test]
    fn empty_map_default_record() {
        let db = open_db(":memory:").unwrap();
        record(&db, RecordArgs {
            actor: "human:erik", action: "x", command: None, args: None, outcome: "ok",
            entry_ids: vec![],
        })
        .unwrap();
        let listed = list(&db, None, None, 10).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0]["entry_ids"].as_array().unwrap().len(), 0);
    }
}

