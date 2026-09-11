// bukio-cli — agent-first double-entry bookkeeping for SMEs.
// Copyright (c) 2026 Erik van Kempen.
// SPDX-License-Identifier: Apache-2.0
//
// Actor identity (mirrors src/core/actor.js) + DB-backed key/role registry
// (mirrors src/core/actor-registry.js).

use crate::money::{BukioError, Result};
use serde_json::{json, Value};
fn sql_err(e: rusqlite::Error) -> BukioError {
    BukioError::new("DB_ERROR", e.to_string())
}
use rusqlite::Connection;
use std::time::Duration;

/// ^(agent|human|system):[A-Za-z0-9][A-Za-z0-9._-]{0,63}$
pub fn is_valid_actor(actor: &str) -> bool {
    let Some((role, name)) = actor.split_once(':') else {
        return false;
    };
    if !matches!(role, "agent" | "human" | "system") {
        return false;
    }
    let mut cs = name.chars();
    let Some(first) = cs.next() else { return false };
    if !first.is_ascii_alphanumeric() {
        return false;
    }
    let rest_ok = name[1..]
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    let len_ok = name.len() <= 64;
    rest_ok && len_ok
}

/// {code, message} for a missing/invalid actor, or None when valid.
pub fn actor_error(actor: Option<&str>) -> Option<BukioError> {
    match actor {
        None | Some("") => Some(BukioError::new(
            "ACTOR_REQUIRED",
            "missing actor — pass --actor '<role>:<name>' (e.g. agent:bartholomeus, human:erik) or set BUKIO_ACTOR",
        )),
        Some(a) if !is_valid_actor(a) => Some(BukioError::new(
            "INVALID_ACTOR",
            format!("invalid actor '{a}' — must be '<role>:<name>' (agent|human|system), e.g. agent:bartholomeus or human:erik"),
        )),
        _ => None,
    }
}

// --- registry (mirrors actor-registry.js) -----------------------------------


pub fn now_iso() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or(Duration::ZERO);
    chrono::DateTime::<chrono::Utc>::from_timestamp(now.as_secs() as i64, now.subsec_nanos())
        .map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
        .unwrap_or_default()
}

pub fn get_actor_key(db: &Connection, actor: &str) -> Option<ActorKeyRow> {
    db.query_row(
        "SELECT actor, keyid, public_key, enrolled_at, revoked_at, revoked_reason
         FROM actor_keys WHERE actor = ?1 AND revoked_at IS NULL
         ORDER BY enrolled_at DESC LIMIT 1",
        [actor],
        row_to_actor_key,
    )
    .ok()
}

fn row_to_actor_key(r: &rusqlite::Row<'_>) -> rusqlite::Result<ActorKeyRow> {
    Ok(ActorKeyRow {
        actor: r.get(0)?,
        keyid: r.get(1)?,
        public_key: r.get(2)?,
        enrolled_at: r.get(3)?,
        revoked_at: r.get(4)?,
        revoked_reason: r.get(5)?,
    })
}

#[derive(Debug, Clone)]
pub struct ActorKeyRow {
    #[allow(dead_code)]
    actor: String,
    pub keyid: String,
    pub public_key: String,
    pub enrolled_at: String,
    pub revoked_at: Option<String>,
    pub revoked_reason: Option<String>,
}

pub fn get_key_by_keyid(db: &Connection, keyid: &str) -> Option<ActorKeyRow> {
    db.query_row(
        "SELECT actor, keyid, public_key, enrolled_at, revoked_at, revoked_reason
         FROM actor_keys WHERE keyid = ?1 ORDER BY enrolled_at DESC LIMIT 1",
        [keyid],
        row_to_actor_key,
    )
    .ok()
}

pub fn get_any_actor_key(db: &Connection, actor: &str) -> Option<ActorKeyRow> {
    db.query_row(
        "SELECT actor, keyid, public_key, enrolled_at, revoked_at, revoked_reason
         FROM actor_keys WHERE actor = ?1 ORDER BY enrolled_at DESC LIMIT 1",
        [actor],
        row_to_actor_key,
    )
    .ok()
}

pub fn get_enforce(db: &Connection) -> bool {
    db.query_row(
        "SELECT value FROM settings WHERE key = 'signing_enforce'",
        [],
        |r| r.get::<_, String>(0),
    )
    .map(|v| v == "on")
    .unwrap_or(false)
}

pub fn get_authz(db: &Connection) -> bool {
    db.query_row(
        "SELECT value FROM settings WHERE key = 'authz_mode'",
        [],
        |r| r.get::<_, String>(0),
    )
    .map(|v| v == "on")
    .unwrap_or(false)
}

pub fn get_roles(db: &Connection, actor: &str) -> Vec<String> {
    let mut stmt = db
        .prepare("SELECT role FROM actor_roles WHERE actor = ?1 ORDER BY role")
        .unwrap();
    let rows = stmt
        .query_map([actor], |r| r.get::<_, String>(0))
        .unwrap()
        .filter_map(|r| r.ok())
        .collect();
    rows
}

pub fn can_act_enrolled(db: &Connection, actor: &str) -> bool {
    get_actor_key(db, actor).is_some()
}

/// The full registry row as JSON (JS returns the raw `SELECT *` row).
fn actor_key_json(row: &ActorKeyRow) -> Value {
    json!({
        "actor": row.actor,
        "keyid": row.keyid,
        "public_key": row.public_key,
        "enrolled_at": row.enrolled_at,
        "revoked_at": row.revoked_at,
        "revoked_reason": row.revoked_reason,
    })
}

/// Enrol an actor key into the company registry.
pub fn enrol_actor(db: &Connection, actor: &str, keyid: &str, public_key: &str) -> Result<Value> {
    if !is_valid_actor(actor) {
        return Err(BukioError::new(
            "INVALID_ACTOR",
            format!("'{actor}' is not a valid '<role>:<name>' actor"),
        ));
    }
    if keyid.is_empty() || public_key.is_empty() {
        return Err(BukioError::new(
            "INVALID_KEY",
            "keyid and public key are required",
        ));
    }
    if let Some(active) = get_actor_key(db, actor) {
        return Err(BukioError::new(
            "ALREADY_ENROLLED",
            format!(
                "actor {actor} already has an active key ({}) — revoke it first to rotate",
                active.keyid
            ),
        ));
    }
    let now = now_iso();
    // Plain INSERT, never INSERT OR REPLACE: replacing the row would destroy the
    // revoked history that audit verification depends on.
    db.execute(
        "INSERT INTO actor_keys (actor, keyid, public_key, enrolled_at) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![actor, keyid, public_key, now],
    )
    .map_err(sql_err)?;
    let row = get_actor_key(db, actor)
        .ok_or_else(|| BukioError::new("DB_ERROR", "enrolment row vanished".to_string()))?;
    Ok(actor_key_json(&row))
}

/// Revoke an actor key.
pub fn revoke_actor(db: &Connection, actor: &str) -> Result<()> {
    let now = now_iso();
    db.execute(
        "UPDATE actor_keys SET revoked_at = ?1 WHERE actor = ?2 AND revoked_at IS NULL",
        rusqlite::params![now, actor],
    )
    .map_err(|e| BukioError::new("DB_ERROR", e.to_string()))?;
    Ok(())
}

/// Revoke an actor key with a reason. Returns the key info.
pub fn revoke_actor_reason(db: &Connection, actor: &str, reason: &str) -> Result<ActorKeyRow> {
    // the reason is checked FIRST (JS order): an empty reason is INVALID_REASON
    // even for an actor that has no active key.
    let reason = reason.trim();
    if reason.is_empty() {
        return Err(BukioError::new(
            "INVALID_REASON",
            "a revocation reason is required",
        ));
    }
    let active = get_actor_key(db, actor).ok_or_else(|| {
        BukioError::new(
            "NOT_ENROLLED",
            format!("actor {actor} has no active key in this company DB"),
        )
    })?;
    let now = now_iso();
    db.execute(
        "UPDATE actor_keys SET revoked_at = ?1, revoked_reason = ?2 WHERE actor = ?3 AND keyid = ?4",
        rusqlite::params![now, reason, actor, active.keyid],
    )
    .map_err(sql_err)?;
    get_key_by_keyid(db, &active.keyid)
        .ok_or_else(|| BukioError::new("ACTOR_NOT_FOUND", format!("actor {actor} not found")))
}

pub fn list_actors(db: &Connection) -> Result<Vec<Value>> {
    let mut stmt = db.prepare("SELECT actor, keyid, enrolled_at, revoked_at, revoked_reason FROM actor_keys ORDER BY enrolled_at, actor").map_err(sql_err)?;
    let rows = stmt.query_map([], |r| {
        let revoked_at: Option<String> = r.get(3)?;
Ok(json!({"actor": r.get::<_, String>(0)?, "keyid": r.get::<_, String>(1)?, "enrolled_at": r.get::<_, String>(2)?, "revoked_at": revoked_at.as_deref(), "revoked_reason": r.get::<_, Option<String>>(4)?, "active": revoked_at.is_none()}))
    }).map_err(sql_err)?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

pub fn set_enforce(db: &Connection, on: bool) {
    db.execute("INSERT INTO settings (key, value) VALUES ('signing_enforce', ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [if on { "on" } else { "off" }]).ok();
}

pub fn set_authz_mode(db: &Connection, on: bool) {
    db.execute("INSERT INTO settings (key, value) VALUES ('authz_mode', ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [if on { "on" } else { "off" }]).ok();
}

pub fn grant_role(db: &Connection, actor: &str, role: &str, granted_by: &str) -> Result<()> {
    if !is_valid_actor(actor) {
        return Err(BukioError::new(
            "INVALID_ACTOR",
            format!("'{actor}' is not a valid '<role>:<name>' actor"),
        ));
    }
    if !crate::authz::ROLES.contains(&role) {
        return Err(BukioError::new(
            "INVALID_ROLE",
            format!("{role} is not a valid role"),
        ));
    }
    if granted_by.is_empty() {
        return Err(BukioError::new("INVALID_ACTOR", "grantedBy is required"));
    }
    db.execute("INSERT INTO actor_roles (actor, role, granted_by, granted_at) VALUES (?1, ?2, ?3, ?4) ON CONFLICT(actor, role) DO NOTHING",
        rusqlite::params![actor, role, granted_by, now_iso()]).map_err(sql_err)?;
    Ok(())
}

pub fn revoke_role(db: &Connection, actor: &str, role: &str) -> Result<()> {
    // JS order: validate the role, then that it is actually held, then the
    // last-owner guarantee.
    if !crate::authz::ROLES.contains(&role) {
        return Err(BukioError::new(
            "INVALID_ROLE",
            format!("{role} is not a valid role"),
        ));
    }
    let held: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM actor_roles WHERE actor = ?1 AND role = ?2",
            rusqlite::params![actor, role],
            |r| r.get(0),
        )
        .map_err(sql_err)?;
    if held == 0 {
        return Err(BukioError::new(
            "ROLE_NOT_GRANTED",
            format!("actor {actor} does not hold the role '{role}' — nothing to revoke"),
        ));
    }
    if role == "owner" {
        let count: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM actor_roles WHERE role = 'owner'",
                [],
                |r| r.get(0),
            )
            .map_err(sql_err)?;
        if count <= 1 {
            return Err(BukioError::new(
                "LAST_OWNER",
                format!("{actor} is the last owner — grant owner to another actor first"),
            ));
        }
    }
    db.execute(
        "DELETE FROM actor_roles WHERE actor = ?1 AND role = ?2",
        rusqlite::params![actor, role],
    )
    .map_err(sql_err)?;
    Ok(())
}

pub fn list_role_grants(db: &Connection) -> Result<Vec<Value>> {
    let mut stmt = db
        .prepare("SELECT actor, role, granted_by, granted_at FROM actor_roles ORDER BY actor, role")
        .map_err(sql_err)?;
    let rows = stmt.query_map([], |r| {
        Ok(json!({"actor": r.get::<_, String>(0)?, "role": r.get::<_, String>(1)?, "granted_by": r.get::<_, String>(2)?, "granted_at": r.get::<_, String>(3)?}))
    }).map_err(sql_err)?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── ported from test/actor-registry.test.js ──

    fn registry_db() -> Connection {
        crate::db::open_db(":memory:").unwrap()
    }

    /// A fresh (public_pem, keyid) pair.
    fn keypair() -> (String, String) {
        let (public_pem, _private, keyid) = crate::sign::generate_key_pair();
        (public_pem, keyid)
    }

    /// Enrol a fresh key for `actor`; returns (public_pem, keyid).
    fn enrol(db: &Connection, actor: &str) -> (String, String) {
        let (public_pem, keyid) = keypair();
        enrol_actor(db, actor, &keyid, &public_pem).unwrap();
        (public_pem, keyid)
    }

    fn temp_db_path(tag: &str) -> String {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir()
            .join(format!("bukio-{tag}-{nanos}.db"))
            .to_string_lossy()
            .to_string()
    }

    fn cleanup_db(path: &str) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{path}{suffix}"));
        }
    }

    fn has_role(db: &Connection, actor: &str, role: &str) -> bool {
        get_roles(db, actor).iter().any(|r| r == role)
    }

    #[test]
    fn enrol_writes_a_registry_row_with_keyid_public_key_and_timestamp() {
        let db = registry_db();
        let (public_pem, keyid) = keypair();
        let row = enrol_actor(&db, "agent:bartholomeus", &keyid, &public_pem).unwrap();
        assert_eq!(row["actor"], "agent:bartholomeus");
        assert_eq!(row["keyid"], keyid.as_str());
        assert_eq!(row["public_key"], public_pem.as_str());
        assert!(!row["enrolled_at"].as_str().unwrap().is_empty());
        assert_eq!(row["revoked_at"], Value::Null);
        assert_eq!(
            get_actor_key(&db, "agent:bartholomeus").unwrap().keyid,
            keyid
        );
    }

    #[test]
    fn enrol_rejects_a_duplicate_while_an_active_key_exists() {
        let db = registry_db();
        let (_a_pub, a_keyid) = enrol(&db, "human:erik");
        let (b_pub, b_keyid) = keypair();
        assert_eq!(
            enrol_actor(&db, "human:erik", &b_keyid, &b_pub)
                .unwrap_err()
                .code,
            "ALREADY_ENROLLED"
        );
        // the original key is untouched
        assert_eq!(get_actor_key(&db, "human:erik").unwrap().keyid, a_keyid);
    }

    #[test]
    fn enrol_rejects_an_invalid_actor_or_missing_key_material() {
        let db = registry_db();
        let (public_pem, keyid) = keypair();
        assert_eq!(
            enrol_actor(&db, "human", &keyid, &public_pem)
                .unwrap_err()
                .code,
            "INVALID_ACTOR"
        );
        assert_eq!(
            enrol_actor(&db, "human:erik", "", &public_pem)
                .unwrap_err()
                .code,
            "INVALID_KEY"
        );
        // JS passes publicKey: null; Rust's &str equivalent is empty
        assert_eq!(
            enrol_actor(&db, "human:erik", &keyid, "").unwrap_err().code,
            "INVALID_KEY"
        );
    }

    #[test]
    fn revoke_marks_the_row_with_a_reason_and_keeps_it_as_history() {
        let db = registry_db();
        let (_public_pem, keyid) = enrol(&db, "agent:bartholomeus");
        let revoked = revoke_actor_reason(&db, "agent:bartholomeus", "key rotation").unwrap();
        assert!(revoked.revoked_at.is_some());
        assert_eq!(revoked.revoked_reason.as_deref(), Some("key rotation"));
        assert_eq!(revoked.keyid, keyid); // row kept, not deleted
                                          // no active key remains, but the revoked row is still findable by keyid
        assert!(get_actor_key(&db, "agent:bartholomeus").is_none());
        assert_eq!(
            get_key_by_keyid(&db, &keyid).unwrap().revoked_at,
            revoked.revoked_at
        );
    }

    #[test]
    fn revoke_requires_a_reason_and_rejects_unknown_or_already_revoked_actors() {
        let db = registry_db();
        assert_eq!(
            revoke_actor_reason(&db, "human:erik", "").unwrap_err().code,
            "INVALID_REASON"
        );
        assert_eq!(
            revoke_actor_reason(&db, "human:erik", "   ")
                .unwrap_err()
                .code,
            "INVALID_REASON"
        );
        assert_eq!(
            revoke_actor_reason(&db, "human:erik", "nope")
                .unwrap_err()
                .code,
            "NOT_ENROLLED"
        );
        enrol(&db, "human:erik");
        revoke_actor_reason(&db, "human:erik", "lost laptop").unwrap();
        assert_eq!(
            revoke_actor_reason(&db, "human:erik", "again")
                .unwrap_err()
                .code,
            "NOT_ENROLLED"
        );
    }

    #[test]
    fn can_act_is_true_only_for_enrolled_actors() {
        let db = registry_db();
        assert!(!can_act_enrolled(&db, "agent:bartholomeus")); // not enrolled
        enrol(&db, "agent:bartholomeus");
        assert!(can_act_enrolled(&db, "agent:bartholomeus"));
        revoke_actor_reason(&db, "agent:bartholomeus", "compromised").unwrap();
        assert!(!can_act_enrolled(&db, "agent:bartholomeus"));
    }

    #[test]
    fn rotation_reenrols_a_fresh_key_and_retains_the_old_row() {
        let db = registry_db();
        let (old_pub, old_keyid) = keypair();
        enrol_actor(&db, "human:erik", &old_keyid, &old_pub).unwrap();
        revoke_actor_reason(&db, "human:erik", "rotate").unwrap();

        let (fresh_pub, fresh_keyid) = keypair();
        let row = enrol_actor(&db, "human:erik", &fresh_keyid, &fresh_pub).unwrap();
        assert_eq!(row["keyid"], fresh_keyid.as_str());
        assert_eq!(row["revoked_at"], Value::Null);
        assert!(can_act_enrolled(&db, "human:erik"));
        // the active row is the fresh key; the OLD row stays as history so
        // audit verify can still validate signatures made with it
        assert_eq!(get_actor_key(&db, "human:erik").unwrap().keyid, fresh_keyid);
        let old = get_key_by_keyid(&db, &old_keyid).unwrap();
        assert!(old.revoked_at.is_some());
        assert_eq!(old.public_key, old_pub);
    }

    #[test]
    fn enforce_flag_defaults_off_and_toggles_per_db() {
        let db = registry_db();
        assert!(!get_enforce(&db));
        set_enforce(&db, true);
        assert!(get_enforce(&db));
        set_enforce(&db, false);
        assert!(!get_enforce(&db));

        let other = registry_db();
        assert!(!get_enforce(&other));
        set_enforce(&other, true);
        assert!(!get_enforce(&db)); // untouched
        assert!(get_enforce(&other));
    }

    #[test]
    fn registry_is_per_company_db() {
        let db = registry_db();
        let (_a_pub, a_keyid) = enrol(&db, "agent:bartholomeus");

        let other = registry_db();
        assert!(get_actor_key(&other, "agent:bartholomeus").is_none());
        let (b_pub, b_keyid) = keypair();
        enrol_actor(&other, "agent:bartholomeus", &b_keyid, &b_pub).unwrap();
        assert_eq!(
            get_actor_key(&other, "agent:bartholomeus").unwrap().keyid,
            b_keyid
        );
        assert_eq!(
            get_actor_key(&db, "agent:bartholomeus").unwrap().keyid,
            a_keyid
        ); // DB A untouched
        assert!(can_act_enrolled(&other, "agent:bartholomeus"));

        revoke_actor_reason(&db, "agent:bartholomeus", "a-only").unwrap();
        assert!(!can_act_enrolled(&db, "agent:bartholomeus"));
        assert!(can_act_enrolled(&other, "agent:bartholomeus")); // B independent
    }

    #[test]
    fn registry_persists_to_disk_and_survives_reopen() {
        let path = temp_db_path("registry");
        let (public_pem, keyid) = keypair();
        {
            let first = crate::db::open_db(&path).unwrap();
            enrol_actor(&first, "system:month-end", &keyid, &public_pem).unwrap();
            set_enforce(&first, true);
        }
        {
            let reopened = crate::db::open_db(&path).unwrap();
            assert_eq!(
                get_actor_key(&reopened, "system:month-end").unwrap().keyid,
                keyid
            );
            assert!(get_enforce(&reopened));
        }
        cleanup_db(&path);
    }

    #[test]
    fn authz_flag_defaults_off_and_toggles_per_db() {
        let db = registry_db();
        assert!(!get_authz(&db));
        set_authz_mode(&db, true);
        assert!(get_authz(&db));
        set_authz_mode(&db, false);
        assert!(!get_authz(&db));

        let other = registry_db();
        assert!(!get_authz(&other));
        set_authz_mode(&other, true);
        assert!(!get_authz(&db)); // untouched
        assert!(get_authz(&other));
    }

    #[test]
    fn grant_role_writes_a_row_and_is_idempotent() {
        let db = registry_db();
        grant_role(&db, "agent:invoicing", "bookkeeper", "human:erik").unwrap();
        let rows = list_role_grants(&db).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["actor"], "agent:invoicing");
        assert_eq!(rows[0]["role"], "bookkeeper");
        assert_eq!(rows[0]["granted_by"], "human:erik");
        assert!(rows[0]["granted_at"].as_str().unwrap().len() > 0);
        assert_eq!(
            get_roles(&db, "agent:invoicing"),
            vec!["bookkeeper".to_string()]
        );
        // a repeat grant is a no-op, not an error
        grant_role(&db, "agent:invoicing", "bookkeeper", "human:erik").unwrap();
        assert_eq!(
            get_roles(&db, "agent:invoicing"),
            vec!["bookkeeper".to_string()]
        );
        assert_eq!(list_role_grants(&db).unwrap().len(), 1);
    }

    #[test]
    fn grant_role_rejects_invalid_actors_and_roles() {
        let db = registry_db();
        assert_eq!(
            grant_role(&db, "agent", "bookkeeper", "human:erik")
                .unwrap_err()
                .code,
            "INVALID_ACTOR"
        );
        assert_eq!(
            grant_role(&db, "agent:invoicing", "superuser", "human:erik")
                .unwrap_err()
                .code,
            "INVALID_ROLE"
        );
        // JS passes grantedBy: null; Rust's &str equivalent is empty
        assert_eq!(
            grant_role(&db, "agent:invoicing", "bookkeeper", "")
                .unwrap_err()
                .code,
            "INVALID_ACTOR"
        );
    }

    #[test]
    fn revoke_role_removes_the_row_and_rejects_roles_not_held() {
        let db = registry_db();
        grant_role(&db, "agent:tax", "tax", "human:erik").unwrap();
        revoke_role(&db, "agent:tax", "tax").unwrap();
        assert!(get_roles(&db, "agent:tax").is_empty());
        assert_eq!(
            revoke_role(&db, "agent:tax", "tax").unwrap_err().code,
            "ROLE_NOT_GRANTED"
        );
        assert_eq!(
            revoke_role(&db, "agent:tax", "bogus").unwrap_err().code,
            "INVALID_ROLE"
        );
    }

    #[test]
    fn the_last_owner_can_never_be_revoked() {
        let db = registry_db();
        grant_role(&db, "human:erik", "owner", "human:erik").unwrap();
        assert_eq!(
            revoke_role(&db, "human:erik", "owner").unwrap_err().code,
            "LAST_OWNER"
        );
        // once a second owner exists, the first may step down
        grant_role(&db, "agent:backup", "owner", "human:erik").unwrap();
        revoke_role(&db, "human:erik", "owner").unwrap();
        assert!(get_roles(&db, "human:erik").is_empty());
        assert!(has_role(&db, "agent:backup", "owner"));
    }

    #[test]
    fn role_grants_are_per_company_db() {
        let db = registry_db();
        grant_role(&db, "agent:invoicing", "bookkeeper", "human:erik").unwrap();

        let other = registry_db();
        assert!(get_roles(&other, "agent:invoicing").is_empty());
        assert!(!has_role(&other, "agent:invoicing", "bookkeeper"));
        grant_role(&other, "agent:invoicing", "readonly", "human:erik").unwrap();
        assert_eq!(
            get_roles(&db, "agent:invoicing"),
            vec!["bookkeeper".to_string()]
        ); // A untouched
    }

    #[test]
    fn list_role_grants_lists_every_grant_with_grantor_and_timestamp() {
        let db = registry_db();
        grant_role(&db, "agent:invoicing", "bookkeeper", "human:erik").unwrap();
        grant_role(&db, "agent:tax", "tax", "human:erik").unwrap();
        let rows = list_role_grants(&db).unwrap();
        assert_eq!(rows.len(), 2);
        let find = |actor: &str| {
            rows.iter()
                .find(|r| r["actor"] == actor)
                .unwrap_or_else(|| panic!("no grant for {actor}"))
        };
        assert_eq!(find("agent:invoicing")["role"], "bookkeeper");
        assert_eq!(find("agent:tax")["granted_by"], "human:erik");
    }

    #[test]
    fn authz_flag_and_role_grants_persist_to_disk() {
        let path = temp_db_path("role-registry");
        {
            let first = crate::db::open_db(&path).unwrap();
            set_authz_mode(&first, true);
            grant_role(&first, "human:erik", "owner", "human:erik").unwrap();
        }
        {
            let reopened = crate::db::open_db(&path).unwrap();
            assert!(get_authz(&reopened));
            assert!(has_role(&reopened, "human:erik", "owner"));
        }
        cleanup_db(&path);
    }

    #[test]
    fn actor_regex_matches_js() {
        let long_name = format!("agent:{}", "x".repeat(64));
        for good in [
            "human:erik",
            "agent:bartholomeus",
            "system:cron",
            "agent:a",
            long_name.as_str(),
            "agent:with.dots_and-dashes",
        ] {
            assert!(is_valid_actor(&good), "should accept {good}");
        }
        for bad in [
            "",
            "human",
            "agent",
            "agent:",
            ":erik",
            "robot:erik",
            "agent:-lead",
            "agent:x y",
            "human:erik:extra",
        ] {
            assert!(!is_valid_actor(&bad), "should reject {bad:?}");
        }
    }

    #[test]
    fn registry_roundtrip_on_memory_db() {
        let conn = crate::db::open_db(":memory:").unwrap();
        assert!(get_actor_key(&conn, "human:erik").is_none());
        assert!(!get_enforce(&conn));
        conn.execute(
            "INSERT INTO actor_keys (actor, keyid, public_key, enrolled_at) VALUES ('human:erik', 'abc', 'KEY', '2026-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        let row = get_actor_key(&conn, "human:erik").unwrap();
        assert_eq!(row.keyid, "abc");
        assert!(get_any_actor_key(&conn, "human:erik").is_some());
    }
}
