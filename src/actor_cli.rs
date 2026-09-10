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
            PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".to_string())).join(".bukio")
        })
}

fn key_file_path(actor: &str) -> PathBuf {
    config_dir()
        .join("keys")
        .join(format!("{}.key", actor.replace(':', "-")))
}

pub fn session_file_path(actor: &str) -> PathBuf {
    config_dir()
        .join("sessions")
        .join(format!("{}.key", actor.replace(":", "-")))
}

pub fn read_key_file(actor: &str) -> Result<String> {
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

pub fn write_key_file(actor: &str, pem: &str) -> Result<()> {
    let path = key_file_path(actor);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).ok();
    }
    fs::write(&path, format!("{pem}\n")).map_err(|e| {
        BukioError::new("IO_ERROR", format!("cannot write {}: {e}", path.display()))
    })?;
    // Set restrictive permissions: 0o600 for files
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

pub fn read_passphrase(actor: &str) -> Result<String> {
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

/// Keygen: generate Ed25519 keypair. Human keys are passphrase-encrypted.
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
    let (public_pem, private_pem, keyid, encrypted) = if is_human {
        let passphrase = read_passphrase(actor)?;
        let (pub_pem, priv_pem, kid) = sign::generate_key_pair_encrypted(&passphrase)
            .map_err(|e| BukioError::new("KEY_ERROR", e))?;
        (pub_pem, priv_pem, kid, true)
    } else {
        let (pub_pem, priv_pem, kid) = sign::generate_key_pair();
        (pub_pem, priv_pem, kid, false)
    };
    if dry_run {
        return Ok(json!({
            "actor": actor, "keyid": keyid, "keyFile": path.display().to_string(),
            "encrypted": encrypted, "dryRun": true, "wouldOverwrite": exists,
        }));
    }
    write_key_file(actor, &private_pem)?;
    Ok(json!({
        "actor": actor, "keyid": keyid, "keyFile": path.display().to_string(),
        "encrypted": encrypted, "publicKey": public_pem,
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

/// Revoke an actor's key.
/// Revoke an actor's key. `actor` is the TARGET; `revoked_by` is the caller
/// (JS: `actor revoke --target <who>` → {actor: target, revoked_by: caller}).
pub fn cmd_revoke(
    db_path: &str,
    actor: &str,
    revoked_by: &str,
    reason: Option<&str>,
    dry_run: bool,
) -> Result<Value> {
    if !is_valid_actor(actor) {
        return Err(BukioError::new(
            "INVALID_ACTOR",
            format!("'{actor}' is not a valid '<role>:<name>' actor"),
        ));
    }
    let reason_str = match reason {
        Some(r) if !r.is_empty() => r,
        _ => return Err(BukioError::new("INVALID_REASON", "--reason is required")),
    };
    if dry_run {
        return Ok(json!({
            "actor": actor, "revoked_by": revoked_by, "reason": reason_str, "dryRun": true,
        }));
    }
    let db = open_db(db_path).map_err(|e| BukioError::new("DB_ERROR", e.to_string()))?;
    let row = revoke_actor_reason(&db, actor, reason_str)?;
    record(
        &db,
        RecordArgs {
            actor: revoked_by,
            action: "actor.revoke",
            command: Some("actor revoke"),
            args: Some(json!({"actor": actor, "reason": reason_str})),
            outcome: "ok",
            entry_ids: vec![],
        },
    )?;
    Ok(json!({
        "actor": actor,
        "revoked_by": revoked_by,
        "keyid": row.keyid,
        "revoked_at": row.revoked_at,
        "reason": reason_str,
    }))
}

/// Enforce mode on/off.
pub fn cmd_enforce(db_path: &str, actor: &str, on: bool, dry_run: bool) -> Result<Value> {
    if dry_run {
        return Ok(json!({"enforce": if on { "on" } else { "off" }, "dryRun": true}));
    }
    let db = open_db(db_path).map_err(|e| BukioError::new("DB_ERROR", e.to_string()))?;
    set_enforce(&db, on);
    record(
        &db,
        RecordArgs {
            actor,
            action: "actor.enforce",
            command: Some("actor enforce"),
            args: Some(json!({"enforce": if on { "on" } else { "off" }})),
            outcome: "ok",
            entry_ids: vec![],
        },
    )?;
    Ok(json!({"ok": true, "enforce": if on { "on" } else { "off" }}))
}

/// Authz mode on/off.
pub fn cmd_authz(db_path: &str, actor: &str, on: bool, dry_run: bool) -> Result<Value> {
    if dry_run {
        return Ok(json!({
            "authz": if on { "on" } else { "off" },
            "enforce": if on { "on" } else { "off" },
            "dryRun": true,
            "owner_granted": if on { actor } else { "" },
        }));
    }
    let db = open_db(db_path).map_err(|e| BukioError::new("DB_ERROR", e.to_string()))?;
    // D1: authz implies enforce
    if on {
        set_authz_mode(&db, true);
        set_enforce(&db, true);
        // D3: flipper becomes owner
        grant_role(&db, actor, "owner", actor)?;
    } else {
        set_authz_mode(&db, false);
    }
    // Audit the authz flip
    record(
        &db,
        RecordArgs {
            actor,
            action: "actor.authz",
            command: Some("actor authz"),
            args: Some(json!({"authz": if on { "on" } else { "off" }})),
            outcome: "ok",
            entry_ids: vec![],
        },
    )?;
    let authz = get_authz(&db);
    let enforce = get_enforce(&db);
    Ok(json!({
        "authz": if authz { "on" } else { "off" },
        "enforce": if enforce { "on" } else { "off" },
        "owner": if on { actor } else { "" },
    }))
}

/// Unlock: decrypt key and store session.
pub fn cmd_unlock(actor: &str, ttl_hours: Option<u64>) -> Result<Value> {
    let ttl = ttl_hours.unwrap_or(DEFAULT_TTL_HOURS);
    if ttl < 1 || ttl > MAX_TTL_HOURS {
        return Err(BukioError::new(
            "INVALID_TTL",
            format!("--ttl-hours must be 1–{MAX_TTL_HOURS}"),
        ));
    }
    if !actor.starts_with("human:") {
        return Err(BukioError::new(
            "UNLOCK_NOT_APPLICABLE",
            "only human keys are unlocked per session — agent/system keys sign automatically",
        ));
    }
    let passphrase = read_passphrase(actor)?;
    let path = session_file_path(actor);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).ok();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
        }
    }
    let pem = read_key_file(actor)?;
    if !sign::is_encrypted(&pem) {
        return Err(BukioError::new(
            "KEY_NOT_ENCRYPTED",
            format!("{actor} key is not passphrase-encrypted"),
        ));
    }
    let decrypted = sign::decrypt_private_key_pem(&pem, &passphrase)
        .map_err(|e| BukioError::new("PASSPHRASE_INVALID", e))?;
    let expires = chrono::Utc::now() + chrono::Duration::hours(ttl as i64);
    let session = json!({"keyPem": decrypted, "expiresAt": expires.to_rfc3339()});
    fs::write(&path, session.to_string())
        .map_err(|e| BukioError::new("IO_ERROR", format!("cannot write session: {e}")))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }
    Ok(
        json!({"ok": true, "actor": actor, "sessionFile": path.display().to_string(), "ttl_hours": ttl, "expires": expires.to_rfc3339()}),
    )
}

/// Lock: remove session.
pub fn cmd_lock(actor: &str) -> Result<Value> {
    let path = session_file_path(actor);
    let removed = path.exists();
    if removed {
        fs::remove_file(&path).ok();
    }
    Ok(json!({"ok": true, "actor": actor, "locked": true, "removed": removed}))
}

/// Read session key PEM (if valid, not expired).
pub fn read_session_key(actor: &str) -> Option<String> {
    let path = session_file_path(actor);
    let raw = fs::read_to_string(&path).ok()?;
    let v: Value = serde_json::from_str(&raw).ok()?;
    let expires = v["expiresAt"].as_str()?;
    if let Ok(ts) = chrono::DateTime::parse_from_rfc3339(expires) {
        if ts > chrono::Utc::now() {
            return v["keyPem"].as_str().map(String::from);
        }
    }
    None
}

/// Grant role. JS CLI validates, writes the audit row and returns the
/// actor's RESULTING roles + SoD warnings (not a single warning string).
pub fn cmd_grant(db_path: &str, actor: &str, role: &str, granted_by: &str) -> Result<Value> {
    if !crate::authz::ROLES.contains(&role) {
        return Err(BukioError::new(
            "INVALID_ROLE",
            format!(
                "'{role}' is not a role — use one of {}",
                crate::authz::ROLES.join("|")
            ),
        ));
    }
    if !is_valid_actor(actor) {
        return Err(BukioError::new(
            "INVALID_ACTOR",
            format!("'{actor}' is not a valid '<role>:<name>' actor"),
        ));
    }
    let db = open_db(db_path).map_err(|e| BukioError::new("DB_ERROR", e.to_string()))?;
    grant_role(&db, actor, role, granted_by)?;
    record(
        &db,
        RecordArgs {
            actor: granted_by,
            action: "actor.roles.grant",
            command: Some("actor roles grant"),
            args: Some(json!({ "role": role, "actor": actor })),
            outcome: "ok",
            entry_ids: vec![],
        },
    )?;
    let mut roles = get_roles(&db, actor);
    roles.sort();
    Ok(json!({
        "actor": actor,
        "role": role,
        "roles": roles,
        "warnings": crate::authz::sod_warnings(&roles),
    }))
}

/// Revoke role. `revoked_by` is the CALLER (the audit row names the caller,
/// the args name the target) — JS parity.
pub fn cmd_revoke_role(db_path: &str, actor: &str, role: &str, revoked_by: &str) -> Result<Value> {
    if !crate::authz::ROLES.contains(&role) {
        return Err(BukioError::new(
            "INVALID_ROLE",
            format!(
                "'{role}' is not a role — use one of {}",
                crate::authz::ROLES.join("|")
            ),
        ));
    }
    if !is_valid_actor(actor) {
        return Err(BukioError::new(
            "INVALID_ACTOR",
            format!("'{actor}' is not a valid '<role>:<name>' actor"),
        ));
    }
    let db = open_db(db_path).map_err(|e| BukioError::new("DB_ERROR", e.to_string()))?;
    revoke_role(&db, actor, role)?;
    record(
        &db,
        RecordArgs {
            actor: revoked_by,
            action: "actor.roles.revoke",
            command: Some("actor roles revoke"),
            args: Some(json!({ "role": role, "actor": actor })),
            outcome: "ok",
            entry_ids: vec![],
        },
    )?;
    let mut roles = get_roles(&db, actor);
    roles.sort();
    Ok(json!({ "actor": actor, "role": role, "roles": roles }))
}

/// List role grants — your own, or another actor's via `--for` (the authz
/// gate refuses a non-owner `--for` before we get here).
pub fn cmd_roles(db_path: &str, actor: &str, for_who: Option<&str>) -> Result<Value> {
    let who = for_who.unwrap_or(actor);
    if !is_valid_actor(who) {
        return Err(BukioError::new(
            "INVALID_ACTOR",
            format!("'{who}' is not a valid '<role>:<name>' actor"),
        ));
    }
    let db = open_db(db_path).map_err(|e| BukioError::new("DB_ERROR", e.to_string()))?;
    Ok(json!({ "actor": who, "roles": get_roles(&db, who) }))
}

/// Can: capability check against the ACTUAL mutation (`--post` flips
/// entry.draft -> entry.post; `mcp:<tool>` maps through the MCP table).
pub fn cmd_can(db_path: &str, actor: &str, who: &str, action: &str) -> Result<Value> {
    let tokens: Vec<&str> = action.split_whitespace().collect();
    let path = tokens
        .iter()
        .filter(|t| !t.starts_with('-'))
        .copied()
        .collect::<Vec<&str>>()
        .join(" ");
    let post = tokens.iter().any(|t| *t == "--post");
    let capability = crate::authz::capability_of(&path, post);
    let db = open_db(db_path).map_err(|e| BukioError::new("DB_ERROR", e.to_string()))?;
    let allowed = capability.map_or(false, |c| crate::authz::can_act(&db, who, c));
    let roles = get_roles(&db, who);
    let mut out = json!({
        "actor": who,
        "command": action,
        "capability": capability,
        "allowed": allowed,
        "roles": roles,
    });
    if !allowed {
        out["denied_reason"] = json!(match capability {
            Some(c) => format!("no capability '{c}'"),
            None => "no capability mapping (fail closed)".to_string(),
        });
    }
    Ok(out)
}

/// Verify: check actor's key state against the company registry.
pub fn cmd_verify_actor(db_path: &str, actor: &str) -> Result<Value> {
    let db = open_db(db_path).map_err(|e| BukioError::new("DB_ERROR", e.to_string()))?;
    let any = get_any_actor_key(&db, actor);
    let active = get_actor_key(&db, actor).is_some();
    let registered = any.is_some();
    let revoked = any.as_ref().and_then(|a| a.revoked_at.as_ref()).is_some();
    let key_file = key_file_path(actor);
    let key_file_exists = key_file.exists();
    Ok(json!({
        "actor": actor,
        "registered": registered,
        "active": active,
        "revoked": revoked,
        "keyFileExists": key_file_exists,
    }))
}

/// Verify: check signature (low-level).
pub fn cmd_verify_signature(data: &str, signature: &str, public_pem: &str) -> Result<Value> {
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
