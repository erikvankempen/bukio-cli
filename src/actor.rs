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

pub const VALID_ROLES: [&str; 6] = [
    "owner",
    "bookkeeper",
    "payments",
    "tax",
    "assets",
    "readonly",
];

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

/// Enrol an actor key into the company registry.
pub fn enrol_actor(db: &Connection, actor: &str, keyid: &str, public_pem: &str) -> Result<Value> {
    let now = now_iso();
    db.execute(
        "INSERT OR REPLACE INTO actor_keys (actor, keyid, public_pem, enrolled_at) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![actor, keyid, public_pem, now],
    ).map_err(|e| BukioError::new("DB_ERROR", e.to_string()))?;
    Ok(json!({"actor": actor, "keyid": keyid, "enrolled_at": now}))
}

/// Revoke an actor key.
pub fn revoke_actor(db: &Connection, actor: &str) -> Result<()> {
    let now = now_iso();
    db.execute(
        "UPDATE actor_keys SET revoked_at = ?1 WHERE actor = ?2 AND revoked_at IS NULL",
        rusqlite::params![now, actor],
    ).map_err(|e| BukioError::new("DB_ERROR", e.to_string()))?;
    Ok(())
}

pub fn list_actors(db: &Connection) -> Result<Vec<Value>> {
    let mut stmt = db.prepare("SELECT actor, keyid, enrolled_at, revoked_at, revoked_reason FROM actor_keys ORDER BY enrolled_at, actor").map_err(sql_err)?;
    let rows = stmt.query_map([], |r| {
        Ok(json!({"actor": r.get::<_, String>(0)?, "keyid": r.get::<_, String>(1)?, "enrolled_at": r.get::<_, String>(2)?, "revoked_at": r.get::<_, Option<String>>(3)?, "revoked_reason": r.get::<_, Option<String>>(4)?}))
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
    if !crate::authz::ROLES.contains(&role) {
        return Err(BukioError::new(
            "INVALID_ROLE",
            format!("{role} is not a valid role"),
        ));
    }
    db.execute("INSERT INTO actor_roles (actor, role, granted_by, granted_at) VALUES (?1, ?2, ?3, ?4) ON CONFLICT(actor, role) DO NOTHING",
        rusqlite::params![actor, role, granted_by, now_iso()]).map_err(sql_err)?;
    Ok(())
}

pub fn revoke_role(db: &Connection, actor: &str, role: &str) -> Result<()> {
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
    let affected = db
        .execute(
            "DELETE FROM actor_roles WHERE actor = ?1 AND role = ?2",
            rusqlite::params![actor, role],
        )
        .map_err(sql_err)?;
    if affected == 0 {
        return Err(BukioError::new(
            "ROLE_NOT_GRANTED",
            format!("actor {actor} does not hold role '{role}'"),
        ));
    }
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
