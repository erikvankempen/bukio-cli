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
    let stored_command = s
        .signed_command
        .clone()
        .or_else(|| a.command.map(String::from));
    db.execute(
        "INSERT INTO audit_log (actor, action, command, args_json, outcome, entry_ids,
                                digest_hash, sig_keyid, sig_nonce, sig_ts, sig, sig_status)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        rusqlite::params![
            a.actor,
            a.action,
            stored_command,
            stored_args
                .as_ref()
                .map(|v| serde_json::to_string(v).unwrap()),
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
            if s.sig_status.is_empty() {
                "unsigned".to_string()
            } else {
                s.sig_status.clone()
            },
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

pub fn verify_trail(db: &Connection, since: Option<&str>, limit: Option<i64>) -> Result<Value> {
    // parity with list(): a negative limit must not silently slice from the
    // wrong end (JS slice(-(-5)) == slice(5) would DROP the oldest 5 rows)
    if let Some(n) = limit {
        if n < 0 {
            return Err(BukioError::new(
                "INVALID_LIMIT",
                format!("limit must be a non-negative integer, got '{n}'"),
            ));
        }
    }
    let sql = if since.is_some() {
        "SELECT id, ts, actor, action, command, args_json, sig_status, sig_keyid, sig_nonce, sig_ts, sig, digest_hash FROM audit_log WHERE ts >= ?1 ORDER BY id"
    } else {
        "SELECT id, ts, actor, action, command, args_json, sig_status, sig_keyid, sig_nonce, sig_ts, sig, digest_hash FROM audit_log ORDER BY id"
    };
    let mut stmt = db
        .prepare(sql)
        .map_err(|e| BukioError::new("AUDIT_READ_FAILED", e.to_string()))?;
    let map_row = |r: &rusqlite::Row| -> rusqlite::Result<AuditRow> {
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
    };
    let mut rows: Vec<AuditRow> = if let Some(s) = since {
        stmt.query_map(rusqlite::params![s], |r| map_row(r))
            .map_err(|e| BukioError::new("AUDIT_READ_FAILED", e.to_string()))?
            .filter_map(|r| r.ok())
            .collect()
    } else {
        stmt.query_map([], |r| map_row(r))
            .map_err(|e| BukioError::new("AUDIT_READ_FAILED", e.to_string()))?
            .filter_map(|r| r.ok())
            .collect()
    };
    // JS: rows.slice(-limit) — the NEWEST `limit` rows (limit 0 = no slicing)
    if let Some(n) = limit {
        if n > 0 && rows.len() as i64 > n {
            rows = rows.split_off(rows.len() - n as usize);
        }
    }
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

pub fn list(
    db: &Connection,
    since: Option<&str>,
    actor: Option<&str>,
    limit: i64,
) -> Result<Vec<Value>> {
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

    // ── ported from test/audit.test.js (the migration-018/019 cases live in
    // db.rs with the other migration tests) ──

    use crate::canonical::build_digest;
    use crate::sign::{generate_key_pair, sign};

    /// set_pending_signature is process-global (it mirrors the JS module
    /// variable), so tests that drive it must not interleave under cargo's
    /// default parallel runner.
    static SIG_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn sig_guard() -> std::sync::MutexGuard<'static, ()> {
        SIG_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn fresh_db() -> Connection {
        let db = open_db(":memory:").unwrap();
        crate::accounts::seed_default_chart(&db).unwrap();
        db
    }

    fn unique_nonce() -> String {
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        format!(
            "nonce-{}",
            N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        )
    }

    /// Enrol a key; returns (public_pem, private_pem, keyid).
    fn enrol(db: &Connection, actor: &str) -> (String, String, String) {
        let (public_pem, private_pem, keyid) = generate_key_pair();
        crate::actor::enrol_actor(db, actor, &keyid, &public_pem).unwrap();
        (public_pem, private_pem, keyid)
    }

    type Key = (String, String, String);

    /// Record a row signed exactly the way the CLI gate would.
    fn signed_row(db: &Connection, actor: &str, cmd: &str, args: Value, key: &Key) {
        let ts = crate::actor::now_iso();
        let nonce = unique_nonce();
        let digest = build_digest(actor, cmd, &args, &ts, &nonce);
        let sig = sign(digest.as_bytes(), &key.1).unwrap();
        set_pending_signature(Some(PendingSignature {
            digest_hash: Some(digest),
            sig_keyid: Some(key.2.clone()),
            sig_nonce: Some(nonce),
            sig_ts: Some(ts),
            sig: Some(sig),
            sig_status: "verified".into(),
            signed_args: Some(args),
            signed_command: Some(cmd.to_string()),
        }));
        record(
            db,
            RecordArgs {
                actor,
                action: "test.action",
                command: Some(cmd),
                args: None,
                outcome: "ok",
                entry_ids: vec![],
            },
        )
        .unwrap();
    }

    #[test]
    fn record_and_list_with_filters() {
        let db = fresh_db();
        record(
            &db,
            RecordArgs {
                actor: "human",
                action: "company.init",
                command: Some("init"),
                args: Some(json!({"name": "X"})),
                outcome: "ok",
                entry_ids: vec![],
            },
        )
        .unwrap();
        record(
            &db,
            RecordArgs {
                actor: "agent:test",
                action: "entry.create",
                command: Some("entry add"),
                args: Some(json!({"n": 1})),
                outcome: "ok",
                entry_ids: vec![7],
            },
        )
        .unwrap();

        let all = list(&db, None, None, 50).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0]["action"], "entry.create"); // newest first
        assert_eq!(all[0]["entry_ids"], json!([7]));
        assert_eq!(all[0]["args"], json!({"n": 1}));

        assert_eq!(list(&db, None, Some("agent:test"), 50).unwrap().len(), 1);
        assert_eq!(
            list(&db, Some("2999-01-01T00:00:00.000Z"), None, 50)
                .unwrap()
                .len(),
            0
        );
    }

    #[test]
    fn audit_log_is_append_only() {
        let db = fresh_db();
        record(
            &db,
            RecordArgs {
                actor: "human",
                action: "company.init",
                command: None,
                args: None,
                outcome: "ok",
                entry_ids: vec![],
            },
        )
        .unwrap();
        let updated = db.execute(
            "UPDATE audit_log SET outcome = 'hacked' WHERE action = 'company.init'",
            [],
        );
        assert!(updated.is_err());
        assert!(format!("{:?}", updated.unwrap_err()).contains("append-only"));
        let deleted = db.execute("DELETE FROM audit_log", []);
        assert!(deleted.is_err());
        assert!(format!("{:?}", deleted.unwrap_err()).contains("append-only"));
        assert_eq!(list(&db, None, None, 50).unwrap().len(), 1);
    }

    #[test]
    fn args_null_is_stored_and_read_back_as_null() {
        let db = fresh_db();
        record(
            &db,
            RecordArgs {
                actor: "human",
                action: "plain",
                command: None,
                args: None,
                outcome: "ok",
                entry_ids: vec![],
            },
        )
        .unwrap();
        assert_eq!(list(&db, None, None, 50).unwrap()[0]["args"], Value::Null);
    }

    #[test]
    fn record_stores_signature_fields_and_defaults_to_unsigned() {
        let db = fresh_db();
        record(
            &db,
            RecordArgs {
                actor: "agent:bartholomeus",
                action: "entry.add",
                command: Some("entry add"),
                args: Some(json!({"date": "2026-08-10"})),
                outcome: "ok",
                entry_ids: vec![],
            },
        )
        .unwrap();
        {
            let _g = sig_guard();
            set_pending_signature(Some(PendingSignature {
                digest_hash: Some("ab".repeat(32)),
                sig_keyid: Some("deadbeef".repeat(4)),
                sig_nonce: Some("uuid-1".into()),
                sig_ts: Some("2026-08-10T12:00:00.000Z".into()),
                sig: Some("c2lnbmF0dXJl".into()),
                sig_status: "verified".into(),
                signed_args: Some(json!({"date": "2026-08-10"})),
                signed_command: Some("entry add".into()),
            }));
            record(
                &db,
                RecordArgs {
                    actor: "agent:bartholomeus",
                    action: "entry.add",
                    command: Some("entry add"),
                    args: Some(json!({"date": "2026-08-10"})),
                    outcome: "ok",
                    entry_ids: vec![],
                },
            )
            .unwrap();
            set_pending_signature(None);
        }
        let signed = &list(&db, None, None, 50).unwrap()[0];
        assert_eq!(signed["digest_hash"], "ab".repeat(32));
        assert_eq!(signed["sig_keyid"], "deadbeef".repeat(4));
        assert_eq!(signed["sig_nonce"], "uuid-1");
        assert_eq!(signed["sig_ts"], "2026-08-10T12:00:00.000Z");
        assert_eq!(signed["sig"], "c2lnbmF0dXJl");
        assert_eq!(signed["sig_status"], "verified");

        // a plain record (no pending signature) defaults to unsigned
        record(
            &db,
            RecordArgs {
                actor: "human:erik",
                action: "company.init",
                command: None,
                args: None,
                outcome: "ok",
                entry_ids: vec![],
            },
        )
        .unwrap();
        let plain = &list(&db, None, None, 50).unwrap()[0];
        assert_eq!(plain["sig_status"], "unsigned");
        assert_eq!(plain["sig_keyid"], Value::Null);
    }

    #[test]
    fn verify_trail_reports_a_clean_signed_trail() {
        let _g = sig_guard();
        let db = fresh_db();
        let key = enrol(&db, "agent:bartholomeus");
        signed_row(
            &db,
            "agent:bartholomeus",
            "entry add",
            json!({"date": "2026-08-10", "desc": "x"}),
            &key,
        );
        signed_row(
            &db,
            "agent:bartholomeus",
            "entry post",
            json!({"id": 1}),
            &key,
        );
        set_pending_signature(None); // no gate here: the legacy row is truly unsigned
        record(
            &db,
            RecordArgs {
                actor: "human:erik",
                action: "company.init",
                command: None,
                args: None,
                outcome: "ok",
                entry_ids: vec![],
            },
        )
        .unwrap();

        let out = verify_trail(&db, None, None).unwrap();
        let s = &out["summary"];
        assert_eq!(s["total"], 3);
        assert_eq!(s["ok"], 2);
        assert_eq!(s["unsigned"], 1);
        assert_eq!(s["tampered"], 0);
        assert_eq!(s["invalid_signature"], 0);
        assert_eq!(s["unknown_key"], 0);
        assert_eq!(s["revoked"], 0);
        let mut statuses: Vec<String> = out["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["status"].as_str().unwrap().to_string())
            .collect();
        statuses.sort();
        assert_eq!(statuses, vec!["ok", "ok", "unsigned"]);
    }

    #[test]
    fn verify_trail_rejects_a_negative_limit() {
        // JS also rejects non-integers (3.7, '5'); Rust's i64 parameter makes
        // those unrepresentable, so only the sign guard is testable here.
        let db = fresh_db();
        assert_eq!(
            verify_trail(&db, None, Some(-5)).unwrap_err().code,
            "INVALID_LIMIT"
        );
        assert_eq!(
            verify_trail(&db, None, Some(-1)).unwrap_err().code,
            "INVALID_LIMIT"
        );
        // legit calls: null (all) and 0 (no slicing) both pass
        assert_eq!(
            verify_trail(&db, None, None).unwrap()["summary"]["total"],
            0
        );
        assert_eq!(
            verify_trail(&db, None, Some(0)).unwrap()["summary"]["total"],
            0
        );
    }

    #[test]
    fn verify_trail_flags_tampered_args() {
        let _g = sig_guard();
        let db = fresh_db();
        let key = enrol(&db, "agent:bartholomeus");
        let ts = crate::actor::now_iso();
        let nonce = unique_nonce();
        let args = json!({"date": "2026-08-10"});
        let real_digest = build_digest("agent:bartholomeus", "entry add", &args, &ts, &nonce);
        let tampered_digest = build_digest(
            "agent:bartholomeus",
            "entry add",
            &json!({"date": "2026-01-01"}),
            &ts,
            &nonce,
        );
        set_pending_signature(Some(PendingSignature {
            digest_hash: Some(tampered_digest),
            sig_keyid: Some(key.2.clone()),
            sig_nonce: Some(nonce),
            sig_ts: Some(ts),
            sig: Some(sign(real_digest.as_bytes(), &key.1).unwrap()),
            sig_status: "verified".into(),
            signed_args: Some(args),
            signed_command: Some("entry add".into()),
        }));
        record(
            &db,
            RecordArgs {
                actor: "agent:bartholomeus",
                action: "test.action",
                command: Some("entry add"),
                args: None,
                outcome: "ok",
                entry_ids: vec![],
            },
        )
        .unwrap();
        set_pending_signature(None);

        let out = verify_trail(&db, None, None).unwrap();
        assert_eq!(out["summary"]["tampered"], 1);
        assert_eq!(out["rows"][0]["status"], "tampered");
    }

    #[test]
    fn verify_trail_flags_a_corrupted_signature() {
        let _g = sig_guard();
        let db = fresh_db();
        let key = enrol(&db, "agent:bartholomeus");
        let ts = crate::actor::now_iso();
        let nonce = unique_nonce();
        let args = json!({"date": "2026-08-10"});
        let digest = build_digest("agent:bartholomeus", "entry add", &args, &ts, &nonce);
        set_pending_signature(Some(PendingSignature {
            digest_hash: Some(digest),
            sig_keyid: Some(key.2.clone()),
            sig_nonce: Some(nonce),
            sig_ts: Some(ts),
            sig: Some("bm90LWEtc2lnbmF0dXJl".into()), // garbage
            sig_status: "verified".into(),
            signed_args: Some(args),
            signed_command: Some("entry add".into()),
        }));
        record(
            &db,
            RecordArgs {
                actor: "agent:bartholomeus",
                action: "test.action",
                command: Some("entry add"),
                args: None,
                outcome: "ok",
                entry_ids: vec![],
            },
        )
        .unwrap();
        set_pending_signature(None);

        let out = verify_trail(&db, None, None).unwrap();
        assert_eq!(out["summary"]["invalid_signature"], 1);
        assert_eq!(out["rows"][0]["status"], "invalid-signature");
    }

    #[test]
    fn verify_trail_flags_an_unknown_keyid() {
        let _g = sig_guard();
        let db = fresh_db();
        let ts = crate::actor::now_iso();
        let nonce = unique_nonce();
        let args = json!({"date": "2026-08-10"});
        let digest = build_digest("agent:bartholomeus", "entry add", &args, &ts, &nonce);
        let (_stranger_pub, stranger_private, _) = generate_key_pair();
        set_pending_signature(Some(PendingSignature {
            digest_hash: Some(digest.clone()),
            sig_keyid: Some("ff".repeat(16)),
            sig_nonce: Some(nonce),
            sig_ts: Some(ts),
            sig: Some(sign(digest.as_bytes(), &stranger_private).unwrap()),
            sig_status: "verified".into(),
            signed_args: Some(args),
            signed_command: Some("entry add".into()),
        }));
        record(
            &db,
            RecordArgs {
                actor: "agent:bartholomeus",
                action: "test.action",
                command: Some("entry add"),
                args: None,
                outcome: "ok",
                entry_ids: vec![],
            },
        )
        .unwrap();
        set_pending_signature(None);

        let out = verify_trail(&db, None, None).unwrap();
        assert_eq!(out["summary"]["unknown_key"], 1);
        assert_eq!(out["rows"][0]["status"], "unknown-key");
    }

    #[test]
    fn verify_trail_marks_a_since_revoked_key_as_revoked() {
        let _g = sig_guard();
        let db = fresh_db();
        let key = enrol(&db, "human:erik");
        signed_row(
            &db,
            "human:erik",
            "entry add",
            json!({"date": "2026-08-10"}),
            &key,
        );
        crate::actor::revoke_actor_reason(&db, "human:erik", "lost laptop").unwrap();
        set_pending_signature(None);

        let out = verify_trail(&db, None, None).unwrap();
        assert_eq!(out["summary"]["revoked"], 1);
        assert_eq!(out["summary"]["ok"], 0);
        assert_eq!(out["rows"][0]["status"], "revoked");
    }

    #[test]
    fn verify_trail_keeps_old_rows_verifiable_after_rotation() {
        let _g = sig_guard();
        let db = fresh_db();
        let old = enrol(&db, "agent:bartholomeus");
        signed_row(
            &db,
            "agent:bartholomeus",
            "entry add",
            json!({"date": "2026-08-01"}),
            &old,
        );
        crate::actor::revoke_actor_reason(&db, "agent:bartholomeus", "rotation").unwrap();
        let fresh = enrol(&db, "agent:bartholomeus");
        signed_row(
            &db,
            "agent:bartholomeus",
            "entry add",
            json!({"date": "2026-08-10"}),
            &fresh,
        );
        set_pending_signature(None);

        let out = verify_trail(&db, None, None).unwrap();
        assert_eq!(out["summary"]["total"], 2);
        assert_eq!(out["summary"]["ok"], 1);
        assert_eq!(out["summary"]["revoked"], 1);
        assert_eq!(out["summary"]["unknown_key"], 0);
        // oldest first: the old key's row is the revoked one
        assert_eq!(out["rows"][0]["status"], "revoked");
        assert_eq!(out["rows"][1]["status"], "ok");
    }

    #[test]
    fn verify_trail_limit_checks_only_the_newest_rows() {
        let _g = sig_guard();
        let db = fresh_db();
        let key = enrol(&db, "agent:bartholomeus");
        for n in 1..=3 {
            signed_row(
                &db,
                "agent:bartholomeus",
                "entry add",
                json!({"n": n}),
                &key,
            );
        }
        set_pending_signature(None);

        let out = verify_trail(&db, None, Some(2)).unwrap();
        assert_eq!(out["summary"]["total"], 2);
        assert_eq!(out["summary"]["ok"], 2);
        assert_eq!(out["rows"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn verify_trail_works_on_a_copied_db_file() {
        let _g = sig_guard();
        let dir = std::env::temp_dir();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let source = dir.join(format!("bukio-verify-src-{nanos}.db"));
        let copy = dir.join(format!("bukio-verify-copy-{nanos}.db"));
        let source_s = source.to_string_lossy().to_string();
        let copy_s = copy.to_string_lossy().to_string();

        let key;
        {
            let file_db = crate::db::open_db(&source_s).unwrap();
            key = enrol(&file_db, "agent:bartholomeus");
            signed_row(
                &file_db,
                "agent:bartholomeus",
                "entry add",
                json!({"date": "2026-08-10"}),
                &key,
            );
            signed_row(
                &file_db,
                "agent:bartholomeus",
                "entry post",
                json!({"id": 1}),
                &key,
            );
            set_pending_signature(None);
            record(
                &file_db,
                RecordArgs {
                    actor: "human:erik",
                    action: "company.init",
                    command: None,
                    args: None,
                    outcome: "ok",
                    entry_ids: vec![],
                },
            )
            .unwrap();
        }

        std::fs::copy(&source, &copy).unwrap();
        {
            let copied = crate::db::open_db(&copy_s).unwrap();
            let out = verify_trail(&copied, None, None).unwrap();
            assert_eq!(out["summary"]["total"], 3);
            assert_eq!(out["summary"]["ok"], 2);
            assert_eq!(out["summary"]["unsigned"], 1);
            assert_eq!(out["rows"].as_array().unwrap().len(), 3);
        }
        for p in [&source, &copy] {
            for suffix in ["", "-wal", "-shm"] {
                let _ = std::fs::remove_file(format!("{}{suffix}", p.to_string_lossy()));
            }
        }
    }

    #[test]
    fn verify_trail_since_filters_by_timestamp() {
        let _g = sig_guard();
        let db = fresh_db();
        let key = enrol(&db, "agent:bartholomeus");
        signed_row(
            &db,
            "agent:bartholomeus",
            "entry add",
            json!({"n": 1}),
            &key,
        );
        set_pending_signature(None);

        let out = verify_trail(&db, Some("2999-01-01T00:00:00.000Z"), None).unwrap();
        assert_eq!(out["rows"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn record_and_verify_trail_unsigned() {
        let db = open_db(":memory:").unwrap();
        record(
            &db,
            RecordArgs {
                actor: "agent:test",
                action: "entry.create",
                command: Some("entry add"),
                args: Some(json!({"date": "2026-01-01"})),
                outcome: "ok",
                entry_ids: vec![1],
            },
        )
        .unwrap();
        let out = verify_trail(&db, None, None).unwrap();
        assert_eq!(out["summary"]["total"], 1);
        assert_eq!(out["summary"]["unsigned"], 1);
        assert_eq!(out["rows"][0]["status"], "unsigned");
    }

    #[test]
    fn empty_map_default_record() {
        let db = open_db(":memory:").unwrap();
        record(
            &db,
            RecordArgs {
                actor: "human:erik",
                action: "x",
                command: None,
                args: None,
                outcome: "ok",
                entry_ids: vec![],
            },
        )
        .unwrap();
        let listed = list(&db, None, None, 10).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0]["entry_ids"].as_array().unwrap().len(), 0);
    }
}
