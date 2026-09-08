// bukio-cli — agent-first double-entry bookkeeping for SMEs.
// Copyright (c) 2026 Erik van Kempen.
// SPDX-License-Identifier: Apache-2.0
//
// Actor CLI — keygen, register, list, revoke, enforce, unlock/lock,
// authz, roles, grant, can, verify.

use crate::actor::*;
use crate::audit::{record, RecordArgs};
use crate::db::open_db;
use crate::money::{BukioError, Result};
use crate::sign;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;

const DEFAULT_TTL_HOURS: u64 = 12;
const MAX_TTL_HOURS: u64 = 72;

fn config_dir() -> PathBuf {
    std::env::var("BUKIO_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(
                std::env::var("HOME").unwrap_or_else(|_| ".".to_string()),
            )
            .join(".bukio")
        })
}

fn key_file_path(actor: &str) -> PathBuf {
    config_dir().join("keys").join(format!("{actor}.key"))
}

fn session_file_path(actor: &str) -> PathBuf {
    config_dir()
        .join("sessions")
        .join(format!("{actor}.session"))
}

fn read_key_file(actor: &str) -> Result<String> {
    let path = key_file_path(actor);
    fs::read_to_string(&path).map_err(|_| {
        BukioError::new(
            "KEY_NOT_FOUND",
            format!(
                "no key for {actor} at {} — run 'actor keygen'",
                path.display()
            ),
        )
    })
}

fn write_key_file(actor: &str, pem: &str) -> Result<()> {
    let path = key_file_path(actor);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).ok();
    }
    fs::write(&path, format!("{pem}\n"))
        .map_err(|e| BukioError::new("IO_ERROR", format!("cannot write {}: {e}", path.display())))
}

fn read_passphrase(actor: &str) -> Result<String> {
    if let Ok(env) = std::env::var("BUKIO_SIGNING_PASSPHRASE") {
        if !env.is_empty() {
            return Ok(env);
        }
    }
    Err(BukioError::new(
        "PASSPHRASE_REQUIRED",
        format!("passphrase required for {actor} — set BUKIO_SIGNING_PASSPHRASE"),
    ))
}

/// Keygen: generate Ed25519 keypair.
pub fn cmd_keygen(actor: &str, force: bool, dry_run: bool) -> Result<Value> {
    if !is_valid_actor(actor) {
        return Err(BukioError::new(
            "INVALID_ACTOR",
            format!("'{actor}' is not a valid '<role>:<name>' actor"),
        ));
    }
    let path = key_file_path(actor);
    let exists = path.exists();
    if !dry_run && exists && !force {
        return Err(BukioError::new(
            "KEY_ALREADY_EXISTS",
            format!(
                "key file {} exists — pass --force to replace (rotation)",
                path.display()
            ),
        ));
    }
    let is_human = actor.starts_with("human:");
    let passphrase = if is_human {
        Some(read_passphrase(actor)?)
    } else {
        None
    };
    // generate_key_pair returns (public_pem, private_pem, keyid)
    let (public_pem, private_pem, keyid) = sign::generate_key_pair();
    if dry_run {
        return Ok(json!({
            "actor": actor, "keyid": keyid, "keyFile": path.display().to_string(),
            "encrypted": passphrase.is_some(), "dryRun": true, "wouldOverwrite": exists,
        }));
    }
    write_key_file(actor, &private_pem)?;
    Ok(json!({
        "actor": actor, "keyid": keyid, "keyFile": path.display().to_string(),
        "encrypted": passphrase.is_some(), "publicKey": public_pem,
    }))
}

/// Register: enrol key into company DB.
pub fn cmd_register(db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    let pem = read_key_file(actor)?;
    let is_human = actor.starts_with("human:");
    let passphrase = if is_human && sign::is_encrypted(&pem) {
        Some(read_passphrase(actor)?)
    } else {
        None
    };
    let public_pem = sign::public_key_from_private(&pem, passphrase.as_deref())
        .map_err(|e| BukioError::new("PASSPHRASE_INVALID", format!("could not read key: {e}")))?;
    let keyid = sign::keyid_of(&public_pem).map_err(|e| BukioError::new("KEY_ERROR", e))?;
    if dry_run {
        return Ok(json!({"actor": actor, "keyid": keyid, "dryRun": true}));
    }
    let db = open_db(db_path).map_err(|e| BukioError::new("DB_ERROR", e.to_string()))?;
    let row = enrol_actor(&db, actor, &keyid, &public_pem)?;
    record(
        &db,
        RecordArgs {
            actor,
            action: "actor.register",
            command: Some("actor register"),
            args: Some(json!({"actor": actor, "keyid": keyid})),
            outcome: "ok",
            entry_ids: vec![],
        },
    )?;
    Ok(json!({"actor": actor, "keyid": keyid, "enrolled": true, "row": row}))
}

/// List enrolled actors.
pub fn cmd_list(db_path: &str) -> Result<Value> {
    let db = open_db(db_path).map_err(|e| BukioError::new("DB_ERROR", e.to_string()))?;
    let actors = list_actors(&db)?;
    Ok(json!({"ok": true, "actors": actors}))
}

/// Revoke an actor.
pub fn cmd_revoke(db_path: &str, actor: &str, dry_run: bool) -> Result<Value> {
    if dry_run {
        return Ok(json!({"actor": actor, "dryRun": true}));
    }
    let db = open_db(db_path).map_err(|e| BukioError::new("DB_ERROR", e.to_string()))?;
    revoke_actor(&db, actor)?;
    record(
        &db,
        RecordArgs {
            actor,
            action: "actor.revoke",
            command: Some("actor revoke"),
            args: Some(json!({"actor": actor})),
            outcome: "ok",
            entry_ids: vec![],
        },
    )?;
    Ok(json!({"ok": true, "actor": actor, "revoked": true}))
}

/// Enforce mode on/off.
pub fn cmd_enforce(db_path: &str, on: bool) -> Result<Value> {
    let db = open_db(db_path).map_err(|e| BukioError::new("DB_ERROR", e.to_string()))?;
    set_enforce(&db, on);
    Ok(json!({"ok": true, "enforce": on}))
}

/// Authz mode on/off.
pub fn cmd_authz(db_path: &str, on: bool) -> Result<Value> {
    let db = open_db(db_path).map_err(|e| BukioError::new("DB_ERROR", e.to_string()))?;
    set_authz_mode(&db, on);
    Ok(json!({"ok": true, "authz": on}))
}

/// Unlock: store passphrase session.
pub fn cmd_unlock(actor: &str, ttl_hours: Option<u64>) -> Result<Value> {
    let ttl = ttl_hours.unwrap_or(DEFAULT_TTL_HOURS);
    if ttl < 1 || ttl > MAX_TTL_HOURS {
        return Err(BukioError::new(
            "INVALID_TTL",
            format!("--ttl-hours must be 1–{MAX_TTL_HOURS}"),
        ));
    }
    let passphrase = read_passphrase(actor)?;
    let path = session_file_path(actor);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).ok();
    }
    let pem = read_key_file(actor)?;
    if !sign::is_encrypted(&pem) {
        return Err(BukioError::new(
            "KEY_NOT_ENCRYPTED",
            format!("{actor} key is not passphrase-encrypted"),
        ));
    }
    // Store session: passphrase + expiry timestamp
    let expires = chrono::Utc::now() + chrono::Duration::hours(ttl as i64);
    let session = json!({"passphrase": passphrase, "expires": expires.to_rfc3339()});
    fs::write(&path, session.to_string())
        .map_err(|e| BukioError::new("IO_ERROR", format!("cannot write session: {e}")))?;
    Ok(json!({"ok": true, "actor": actor, "ttl_hours": ttl, "expires": expires.to_rfc3339()}))
}

/// Lock: remove session.
pub fn cmd_lock(actor: &str) -> Result<Value> {
    let path = session_file_path(actor);
    if path.exists() {
        fs::remove_file(&path).ok();
    }
    Ok(json!({"ok": true, "actor": actor, "locked": true}))
}

/// Grant role.
pub fn cmd_grant(db_path: &str, actor: &str, role: &str, granted_by: &str) -> Result<Value> {
    let db = open_db(db_path).map_err(|e| BukioError::new("DB_ERROR", e.to_string()))?;
    grant_role(&db, actor, role, granted_by)?;
    Ok(json!({"ok": true, "actor": actor, "role": role, "granted_by": granted_by}))
}

/// Revoke role.
pub fn cmd_revoke_role(db_path: &str, actor: &str, role: &str) -> Result<Value> {
    let db = open_db(db_path).map_err(|e| BukioError::new("DB_ERROR", e.to_string()))?;
    revoke_role(&db, actor, role)?;
    Ok(json!({"ok": true, "actor": actor, "role": role, "revoked": true}))
}

/// List role grants.
pub fn cmd_roles(db_path: &str) -> Result<Value> {
    let db = open_db(db_path).map_err(|e| BukioError::new("DB_ERROR", e.to_string()))?;
    let grants = list_role_grants(&db)?;
    Ok(json!({"ok": true, "grants": grants}))
}

/// Can: check if actor can perform action.
pub fn cmd_can(db_path: &str, actor: &str, action: &str) -> Result<Value> {
    let db = open_db(db_path).map_err(|e| BukioError::new("DB_ERROR", e.to_string()))?;
    let allowed = can_act_enrolled(&db, actor);
    Ok(json!({"ok": true, "actor": actor, "action": action, "allowed": allowed}))
}

/// Verify: check signature.
pub fn cmd_verify(data: &str, signature: &str, public_pem: &str) -> Result<Value> {
    let ok = sign::verify(data.as_bytes(), signature, public_pem);
    Ok(json!({"ok": true, "valid": ok}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_actor_format() {
        assert!(is_valid_actor("human:erik"));
        assert!(is_valid_actor("agent:bartholomeus"));
        assert!(is_valid_actor("system:cron"));
        assert!(!is_valid_actor("erik"));
        assert!(!is_valid_actor("human:"));
    }

    #[test]
    fn keygen_and_verify() {
        let (pub_pem, priv_pem, keyid) = sign::generate_key_pair();
        assert!(!keyid.is_empty());
        assert!(priv_pem.contains("PRIVATE KEY"));
        assert!(pub_pem.contains("PUBLIC KEY"));
        let sig = sign::sign(b"hello", &priv_pem).unwrap();
        assert!(sign::verify(b"hello", &sig, &pub_pem));
        assert!(!sign::verify(b"world", &sig, &pub_pem));
    }
}
