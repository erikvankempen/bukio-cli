// bukio-cli — agent-first double-entry bookkeeping for SMEs.
// Copyright (c) 2026 Erik van Kempen.
// SPDX-License-Identifier: Apache-2.0
//
// Sign-and-verify gate (mirrors src/cli/util.js sign gate).
// Every command is digitally signed by its actor and verified against the
// per-company registry before dispatch. Three outcomes:
//   - 'verified': key available, actor enrolled, signature checks out.
//   - 'unsigned': no key material / not enrolled — record mode.
//   - refusal: under enforcement, anomalies abort before mutation.
// NONCE_REUSED always refuses regardless of enforce mode.

use rusqlite::Connection;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::actor::{get_actor_key, get_any_actor_key, get_enforce};
use crate::authz::check_authz;
use crate::canonical::build_digest;
use crate::money::{BukioError, Result};
use crate::sign;
use std::cell::RefCell;

// ponytail: thread_local is fine — single-threaded CLI, no contention.
thread_local! {
    static PENDING_SIGN: RefCell<Option<SignResult>> = RefCell::new(None);
}

/// Store the sign result for the current command.
pub fn set_pending_sign(result: Option<SignResult>) {
    PENDING_SIGN.with(|c| *c.borrow_mut() = result);
}

/// Take the pending sign result (consumed once).
pub fn take_pending_sign() -> Option<SignResult> {
    PENDING_SIGN.with(|c| c.borrow_mut().take())
}

/// Get a reference to the pending sign result for building audit args.
pub fn pending_sign_ref() -> Option<SignResult> {
    PENDING_SIGN.with(|c| c.borrow().clone())
}

const SIGNATURE_WINDOW_MS: i64 = 5 * 60_000; // ±5 minutes
const NONCE_RETENTION_MS: i64 = 24 * 3600_000; // nonces remembered 24h

// --- Result types ---

/// Signing key resolved from session, key file, or explicit path.
pub struct ResolvedKey {
    pub key_pem: String,
    pub keyid: String,
}

/// Result of signature bundle verification.
#[derive(Debug, Clone)]
pub struct VerifyResult {
    pub ok: bool,
    pub status: &'static str, // "verified" | "unsigned"
    pub code: Option<&'static str>,
}

/// Full sign result — audit-row fields for a signed command.
#[derive(Debug, Clone)]
pub struct SignResult {
    pub digest_hash: String,
    pub sig_keyid: String,
    pub sig_nonce: String,
    pub sig_ts: String,
    pub sig: String,
    pub sig_status: String,
    pub signed_args: Value,
    pub signed_command: String,
}

// --- Paths ---

fn config_dir() -> PathBuf {
    std::env::var("BUKIO_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".to_string())).join(".bukio")
        })
}

fn nonces_path() -> PathBuf {
    config_dir().join("nonces.json")
}

fn key_file_path(actor: &str) -> PathBuf {
    config_dir()
        .join("keys")
        .join(format!("{}.key", actor.replace(':', "-")))
}

// --- Exempt commands ---

/// Commands exempt from signing (bootstrap: key creation, session, server/mcp).
pub fn is_signing_exempt(cmd: &str) -> bool {
    matches!(
        cmd,
        "actor keygen"
            | "actor unlock"
            | "actor lock"
            | "actor verify"
            | "mcp"
            | "server start"
            | "server token"
    )
}

// --- Nonce management ---

fn read_nonces() -> HashMap<String, HashMap<String, String>> {
    let raw = fs::read_to_string(nonces_path()).unwrap_or_default();
    serde_json::from_str(&raw).unwrap_or_default()
}

/// Serialise the nonce store's read-modify-write. Two concurrent bukio
/// processes (or threads) otherwise lose each other's nonces on the last
/// writer wins, which silently weakens replay protection.
fn with_nonce_lock<T>(f: impl FnOnce() -> T) -> T {
    let lock_path = nonces_path().with_extension("lock");
    if let Some(parent) = lock_path.parent() {
        fs::create_dir_all(parent).ok();
    }
    match fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
    {
        Ok(file) => {
            let _ = file.lock();
            let out = f();
            let _ = file.unlock();
            out
        }
        // no lock file: proceed unlocked rather than failing the command
        Err(_) => f(),
    }
}

/// Check if a nonce was already used for a given keyid.
pub fn is_nonce_used(keyid: &str, nonce: &str) -> bool {
    with_nonce_lock(|| {
        read_nonces()
            .get(keyid)
            .map(|by_key| by_key.contains_key(nonce))
            .unwrap_or(false)
    })
}

/// Remember a nonce with timestamp, pruning entries older than 24h.
pub fn remember_nonce(keyid: &str, nonce: &str) {
    with_nonce_lock(|| remember_nonce_locked(keyid, nonce))
}

fn remember_nonce_locked(keyid: &str, nonce: &str) {
    let now_ms = current_ts_ms();
    let cutoff_ms = now_ms - NONCE_RETENTION_MS;
    let cutoff_iso = ms_to_iso(cutoff_ms);

    let mut nonces = read_nonces();

    // Prune old entries
    nonces.retain(|_, by_key| {
        by_key.retain(|_, ts| *ts >= cutoff_iso);
        !by_key.is_empty()
    });

    // Add new nonce
    let ts_iso = ms_to_iso(now_ms);
    let entry = nonces.entry(keyid.to_string()).or_default();
    entry.insert(nonce.to_string(), ts_iso);

    // Write with restrictive permissions
    let path = nonces_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).ok();
    }
    fs::write(&path, serde_json::to_string(&nonces).unwrap_or_default()).ok();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }
}

// --- Key resolution ---

/// Resolve signing key material for an actor.
/// Priority: explicit path → session key (human unlock) → key file.
/// Returns None in record mode (no enforce, no key). Throws on enforce failures.
pub fn resolve_signing_key(
    actor: &str,
    sign_key_path: Option<&str>,
    enforce: bool,
) -> Result<Option<ResolvedKey>> {
    let file = match sign_key_path {
        Some(p) => PathBuf::from(p),
        None => key_file_path(actor),
    };

    // Try session key first (only when no explicit path)
    if sign_key_path.is_none() {
        if let Some(session_pem) = crate::actor_cli::read_session_key(actor) {
            let keyid = sign::public_key_from_private(&session_pem, None)
                .and_then(|pub_pem| sign::keyid_of(&pub_pem))
                .map_err(|e| BukioError::new("KEY_ERROR", e))?;
            return Ok(Some(ResolvedKey {
                key_pem: session_pem,
                keyid,
            }));
        }
    }

    if !file.exists() {
        if enforce {
            return Err(if sign_key_path.is_some() {
                BukioError::new(
                    "KEY_NOT_FOUND",
                    format!("signing key {} not found", file.display()),
                )
            } else {
                BukioError::new(
                    "SIGNATURE_REQUIRED",
                    format!("no key material for {actor} — run 'bukio actor keygen' + 'actor register' (and 'actor unlock' for human keys)"),
                )
            });
        }
        return Ok(None);
    }

    let pem = fs::read_to_string(&file)
        .map_err(|e| BukioError::new("IO_ERROR", format!("cannot read {}: {e}", file.display())))?;

    if sign::is_encrypted(&pem) {
        let passphrase = std::env::var("BUKIO_SIGNING_PASSPHRASE").ok();
        match passphrase.as_deref() {
            None => {
                if enforce {
                    return Err(BukioError::new(
                        "PASSPHRASE_REQUIRED",
                        format!("the key for {actor} is passphrase-encrypted — run 'bukio actor unlock' or set BUKIO_SIGNING_PASSPHRASE"),
                    ));
                }
                return Ok(None);
            }
            Some(pp) => {
                let decrypted = sign::decrypt_private_key_pem(&pem, pp).map_err(|_| {
                    if enforce {
                        BukioError::new("PASSPHRASE_INVALID", "wrong passphrase")
                    } else {
                        BukioError::new("KEY_ERROR", "wrong passphrase")
                    }
                })?;
                let keyid = sign::public_key_from_private(&pem, Some(pp))
                    .and_then(|pub_pem| sign::keyid_of(&pub_pem))
                    .map_err(|e| BukioError::new("KEY_ERROR", e))?;
                return Ok(Some(ResolvedKey {
                    key_pem: decrypted,
                    keyid,
                }));
            }
        }
    }

    let keyid = sign::public_key_from_private(&pem, None)
        .and_then(|pub_pem| sign::keyid_of(&pub_pem))
        .map_err(|e| BukioError::new("KEY_ERROR", e))?;
    Ok(Some(ResolvedKey {
        key_pem: pem,
        keyid,
    }))
}

// --- Signature verification ---

/// Verify a signature bundle against the company registry.
/// Returns {ok, status, code?}. Nonce reuse ALWAYS refuses.
pub fn verify_signature_bundle(
    db: &Connection,
    actor: &str,
    digest: &str,
    sig: &str,
    keyid: &str,
    ts: &str,
    nonce: &str,
    enforce: bool,
) -> VerifyResult {
    // Check enrolled key
    let row = get_actor_key(db, actor);
    if row.is_none() {
        // Distinguish never-enrolled from revoked
        if let Some(any) = get_any_actor_key(db, actor) {
            if any.revoked_at.is_some() {
                // record mode tolerates a revoked key like any other unusable
                // one (the command still runs, logged unsigned); only an
                // enforced company refuses it
                return VerifyResult {
                    ok: !enforce,
                    status: "unsigned",
                    code: if enforce {
                        Some("ACTOR_KEY_REVOKED")
                    } else {
                        None
                    },
                };
            }
        }
        return VerifyResult {
            ok: !enforce,
            status: "unsigned",
            code: if enforce {
                Some("ACTOR_KEY_UNKNOWN")
            } else {
                None
            },
        };
    }
    let row = row.unwrap();

    // Timestamp within ±5min window
    let ts_ms = parse_ts_ms(ts);
    match ts_ms {
        None => {
            return VerifyResult {
                ok: !enforce,
                status: "unsigned",
                code: if enforce {
                    Some("SIGNATURE_STALE")
                } else {
                    None
                },
            };
        }
        Some(ms) => {
            let now_ms = current_ts_ms();
            if (now_ms - ms).abs() > SIGNATURE_WINDOW_MS {
                return VerifyResult {
                    ok: !enforce,
                    status: "unsigned",
                    code: if enforce {
                        Some("SIGNATURE_STALE")
                    } else {
                        None
                    },
                };
            }
        }
    }

    // Nonce reuse always refuses
    if is_nonce_used(&row.keyid, nonce) {
        return VerifyResult {
            ok: false,
            status: "unsigned",
            code: Some("NONCE_REUSED"),
        };
    }

    // Keyid match + signature verification
    let valid = row.keyid == keyid && sign::verify(digest.as_bytes(), sig, &row.public_key);
    if !valid {
        return VerifyResult {
            ok: !enforce,
            status: "unsigned",
            code: if enforce {
                Some("SIGNATURE_INVALID")
            } else {
                None
            },
        };
    }

    // Remember nonce after successful verification
    remember_nonce(&row.keyid, nonce);

    VerifyResult {
        ok: true,
        status: "verified",
        code: None,
    }
}

// --- Signed args builder ---

/// Build signed args from raw argv and command, stripping identity/output flags.
/// The `--actor` flag is kept when it names a target (not the acting actor).
pub fn build_signed_args(argv: &[String], _cmd: &str, actor: &str) -> Value {
    let mut args = Map::new();
    let mut i = 0;
    let mut positional = Vec::new();

    while i < argv.len() {
        let tok = &argv[i];

        // Bare -- ends flags, everything after is positional
        if !positional.is_empty() || tok == "--" {
            positional.push(tok.clone());
            i += 1;
            continue;
        }

        // Long flags: --key, --key=value
        if let Some(stripped) = tok.strip_prefix("--") {
            if let Some(eq_pos) = stripped.find('=') {
                let key = &stripped[..eq_pos];
                let val = &stripped[eq_pos + 1..];
                if !matches!(key, "sign-key" | "signKey" | "json" | "server" | "db") {
                    args.insert(key.to_string(), Value::String(val.to_string()));
                }
            } else {
                let key = stripped;
                if matches!(key, "sign-key" | "signKey" | "json" | "server" | "db") {
                    // Flag takes a value — skip next token if it's not a flag
                    if i + 1 < argv.len() && !argv[i + 1].starts_with('-') {
                        i += 1;
                    }
                } else if key == "actor" {
                    // --actor: skip if it matches the acting actor, keep otherwise
                    if i + 1 < argv.len() && !argv[i + 1].starts_with('-') {
                        i += 1;
                        if argv[i] != actor {
                            args.insert("actor".to_string(), Value::String(argv[i].clone()));
                        }
                    }
                } else if key == "dry-run" {
                    if i + 1 < argv.len() && !argv[i + 1].starts_with('-') {
                        i += 1;
                        args.insert("dryRun".to_string(), Value::String(argv[i].clone()));
                    }
                } else {
                    // Flag takes a value
                    if i + 1 < argv.len() && !argv[i + 1].starts_with('-') {
                        i += 1;
                        args.insert(key.to_string(), Value::String(argv[i].clone()));
                    } else {
                        // Boolean flag (no value) — set true
                        args.insert(key.to_string(), Value::Bool(true));
                    }
                }
            }
        } else if tok.starts_with('-') {
            // Short flags like -n value
            i += 1;
            continue;
        } else {
            positional.push(tok.clone());
        }
        i += 1;
    }

    if !positional.is_empty() {
        args.insert(
            "positionals".to_string(),
            Value::Array(positional.into_iter().map(Value::String).collect()),
        );
    }

    Value::Object(args)
}

// --- Main entry point ---

/// Sign a command and return audit-row fields, or None (record mode / exempt).
///
/// `argv` is the raw command-line arguments (excluding program name).
/// `cmd` is the canonical command path ('entry add', 'actor keygen').
/// `db_path` is the database file path.
/// `sign_key` is the optional explicit --sign-key path.
/// `enforce` overrides the DB setting (false = always record mode).
pub fn sign_command(
    actor: &str,
    cmd: &str,
    argv: &[String],
    db_path: &str,
    sign_key: Option<&str>,
    enforce_override: Option<bool>,
) -> Result<Option<SignResult>> {
    // Remote-exec: the SERVER already verified the envelope (nonce, window,
    // key registry, signature). The child only replays the verified bundle
    // so its audit rows carry the REAL signature.
    if std::env::var("BUKIO_REMOTE_EXEC").as_deref() == Ok("1") {
        let bundle: Value = std::env::var("BUKIO_REMOTE_SIG")
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or(Value::Null);
        let sig = bundle["sig"].as_str().unwrap_or("").to_string();
        return Ok(Some(SignResult {
            digest_hash: bundle["digest"].as_str().unwrap_or("").to_string(),
            sig_keyid: bundle["keyid"].as_str().unwrap_or("").to_string(),
            sig_nonce: bundle["nonce"].as_str().unwrap_or("").to_string(),
            sig_ts: bundle["ts"].as_str().unwrap_or("").to_string(),
            sig,
            sig_status: if bundle["sig"].is_string() {
                "verified".to_string()
            } else {
                "unsigned".to_string()
            },
            signed_args: bundle["args"].clone(),
            signed_command: bundle["cmd"].as_str().unwrap_or(cmd).to_string(),
        }));
    }
    if is_signing_exempt(cmd) {
        return Ok(None);
    }

    // Open DB — tolerate missing (command itself reports NO_DATABASE later)
    let db = fs::metadata(db_path)
        .ok()
        .and_then(|_| crate::db::open_db(db_path).ok());

    // Special case: `actor register` for a revoked actor is exempt
    if cmd == "actor register" {
        if let Some(ref db) = db {
            if let Some(any) = get_any_actor_key(db, actor) {
                if any.revoked_at.is_some() {
                    return Ok(None);
                }
            }
        }
    }

    let ts = now_iso();
    let nonce = uuid_v4();
    let args = build_signed_args(argv, cmd, actor);
    let digest = build_digest(actor, cmd, &args, &ts, &nonce);
    let enforce =
        enforce_override.unwrap_or_else(|| db.as_ref().map(|d| get_enforce(d)).unwrap_or(false));

    // Resolve signing key — when authz is on and key is missing,
    // let the authz gate handle denial instead of signing gate
    let key = resolve_signing_key(actor, sign_key, enforce);
    let key = match key {
        Ok(Some(k)) => k,
        Ok(None) => return Ok(None), // record mode, no key
        Err(e) if e.code == "SIGNATURE_REQUIRED" => {
            // Authz gate may handle this — check if authz mode is on
            if let Some(ref db) = db {
                if crate::actor::get_authz(db) {
                    // Authz is on — let authz gate deny, not signing gate
                    check_authz(
                        db,
                        actor,
                        cmd,
                        argv.iter().any(|a| a == "--target" || a == "--for"),
                        false,
                    )?;
                    // If we get here, authz passed (shouldn't happen for missing key)
                    return Ok(None);
                }
            }
            return Err(e); // enforce without authz → refuse
        }
        Err(e) => return Err(e),
    };

    // Sign the digest
    let sig = sign::sign(digest.as_bytes(), &key.key_pem)
        .map_err(|e| BukioError::new("SIGN_ERROR", e))?;

    // Verify the bundle against the registry
    let result = db.as_ref().map(|d| {
        verify_signature_bundle(d, actor, &digest, &sig, &key.keyid, &ts, &nonce, enforce)
    });

    let verify = match result {
        Some(v) => v,
        None => {
            // No DB — record mode, accept signature as-is
            VerifyResult {
                ok: true,
                status: "verified",
                code: None,
            }
        }
    };

    if !verify.ok {
        let code = verify.code.unwrap_or("SIGNATURE_FAILED");
        return Err(BukioError::new(code, message_for(code, actor)));
    }

    // Tier 0.5 authz gate
    if let Some(ref db) = db {
        let has_target = argv.iter().any(|a| a == "--target" || a == "--for");
        check_authz(db, actor, cmd, has_target, false)?;
    }

    Ok(Some(SignResult {
        digest_hash: digest,
        sig_keyid: key.keyid,
        sig_nonce: nonce,
        sig_ts: ts,
        sig,
        sig_status: verify.status.to_string(),
        signed_args: args,
        signed_command: cmd.to_string(),
    }))
}

/// Sign an MCP tool call (args already JSON, no argv parsing).
/// Mirrors `sign_command` for the MCP surface: cmd = `mcp:<tool>`,
/// args = tool args minus the identity flag `actor`.
pub fn sign_tool_call(
    db: &Connection,
    actor: &str,
    tool: &str,
    tool_args: &Value,
) -> Result<Option<SignResult>> {
    let cmd = format!("mcp:{tool}");
    if is_signing_exempt(&cmd) {
        return Ok(None);
    }

    let ts = now_iso();
    let nonce = uuid_v4();
    // Tool args minus the identity flag — the exact signed payload
    let mut args = Map::new();
    if let Some(obj) = tool_args.as_object() {
        for (k, v) in obj {
            if k != "actor" {
                args.insert(k.clone(), v.clone());
            }
        }
    }
    let args = Value::Object(args);
    let digest = build_digest(actor, &cmd, &args, &ts, &nonce);
    let enforce = get_enforce(db);

    let key = resolve_signing_key(actor, None, enforce)?;
    let key = match key {
        Some(k) => k,
        None => return Ok(None), // record mode, no key
    };

    // Sign the digest
    let sig = sign::sign(digest.as_bytes(), &key.key_pem)
        .map_err(|e| BukioError::new("SIGN_ERROR", e))?;

    // Verify the bundle against the registry
    let verify =
        verify_signature_bundle(db, actor, &digest, &sig, &key.keyid, &ts, &nonce, enforce);

    if !verify.ok {
        let code = verify.code.unwrap_or("SIGNATURE_FAILED");
        return Err(BukioError::new(code, message_for(code, actor)));
    }

    Ok(Some(SignResult {
        digest_hash: digest,
        sig_keyid: key.keyid,
        sig_nonce: nonce,
        sig_ts: ts,
        sig,
        sig_status: verify.status.to_string(),
        signed_args: args,
        signed_command: cmd,
    }))
}

// --- Helpers ---

fn message_for(code: &str, actor: &str) -> String {
    match code {
        "ACTOR_KEY_UNKNOWN" => format!("actor {actor} has no enrolled key in this company's DB — run 'bukio actor register' (a FIRST enrolment requires enforcement to be off: 'actor enforce --off', register, 'actor enforce --on')"),
        "ACTOR_KEY_REVOKED" => format!("the key for {actor} is revoked in this company's DB — rotate with 'bukio actor keygen --force' + 'actor register'"),
        "SIGNATURE_STALE" => "signature timestamp is outside the ±5 minute window".to_string(),
        "NONCE_REUSED" => "signature nonce was already used — a replayed command is refused".to_string(),
        "SIGNATURE_INVALID" => format!("signature does not verify against the enrolled key for {actor}"),
        _ => "signature verification failed".to_string(),
    }
}

fn current_ts_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn ms_to_iso(ms: i64) -> String {
    let secs = (ms / 1000) as i64;
    let nanos = ((ms % 1000) * 1_000_000) as u32;
    chrono::DateTime::from_timestamp(secs, nanos)
        .map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
        .unwrap_or_default()
}

fn parse_ts_ms(ts: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(ts)
        .ok()
        .map(|dt| dt.timestamp_millis())
}

/// Simple UUID v4 (random). Not cryptographically significant — just a nonce.
fn uuid_v4() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes[6] = (bytes[6] & 0x0f) | 0x40; // version 4
    bytes[8] = (bytes[8] & 0x3f) | 0x80; // variant 1
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3],
        bytes[4], bytes[5],
        bytes[6], bytes[7],
        bytes[8], bytes[9],
        bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15],
    )
}

// --- Tests ---

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_db;

    #[test]
    fn exempt_commands() {
        assert!(is_signing_exempt("actor keygen"));
        assert!(is_signing_exempt("actor unlock"));
        assert!(is_signing_exempt("actor lock"));
        assert!(is_signing_exempt("mcp"));
        assert!(is_signing_exempt("server start"));
        assert!(is_signing_exempt("server token"));
        assert!(!is_signing_exempt("entry add"));
        assert!(!is_signing_exempt("actor register"));
        assert!(!is_signing_exempt("actor enforce"));
    }

    #[test]
    fn nonce_management() {
        let keyid = "testkey123";
        let nonce = uuid_v4();
        assert!(!is_nonce_used(keyid, &nonce));
        remember_nonce(keyid, &nonce);
        assert!(is_nonce_used(keyid, &nonce));
        // Same nonce cannot be used again
        assert!(is_nonce_used(keyid, &nonce));
    }

    #[test]
    fn uuid_v4_format() {
        let id = uuid_v4();
        assert_eq!(id.len(), 36);
        assert_eq!(id.chars().filter(|c| *c == '-').count(), 4);
    }

    #[test]
    fn sign_command_exempt_returns_none() {
        let result = sign_command(
            "human:erik",
            "actor keygen",
            &["--actor".into(), "human:erik".into()],
            ":memory:",
            None,
            Some(true),
        )
        .unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn sign_command_no_key_record_mode() {
        let tmp = std::env::temp_dir().join(format!("bukio_sg_test_{}", uuid_v4()));
        std::fs::create_dir_all(&tmp).ok();
        std::env::set_var("BUKIO_CONFIG_DIR", &tmp);
        let result = sign_command(
            "human:erik",
            "entry add",
            &["--actor".into(), "human:erik".into()],
            ":memory:",
            None,
            Some(false), // enforce off → record mode
        )
        .unwrap();
        assert!(result.is_none());
        std::env::remove_var("BUKIO_CONFIG_DIR");
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn sign_command_no_key_enforce_throws() {
        let tmp = std::env::temp_dir().join(format!("bukio_sg_test_{}", uuid_v4()));
        std::fs::create_dir_all(&tmp).ok();
        std::env::set_var("BUKIO_CONFIG_DIR", &tmp);
        let result = sign_command(
            "human:erik",
            "entry add",
            &["--actor".into(), "human:erik".into()],
            ":memory:",
            None,
            Some(true), // enforce on → should fail
        );
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, "SIGNATURE_REQUIRED");
        std::env::remove_var("BUKIO_CONFIG_DIR");
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn sign_with_explicit_key_roundtrip() {
        let (pub_pem, priv_pem, keyid) = sign::generate_key_pair();
        let dir = std::env::temp_dir().join(format!("bukio_sign_test_{}", uuid_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let key_path = dir.join("test.key");
        fs::write(&key_path, &priv_pem).unwrap();

        let result = sign_command(
            "agent:test",
            "entry add",
            &["--actor".into(), "agent:test".into()],
            ":memory:",
            Some(key_path.to_str().unwrap()),
            Some(true),
        )
        .unwrap();

        let sig = result.unwrap();
        assert_eq!(sig.sig_status, "verified");
        assert_eq!(sig.sig_keyid, keyid);
        assert_eq!(sig.signed_command, "entry add");

        // Verify the signature independently
        assert!(sign::verify(sig.digest_hash.as_bytes(), &sig.sig, &pub_pem));
    }

    #[test]
    fn verify_bundle_rejects_revoked_key() {
        let db = open_db(":memory:").unwrap();
        // Enrol then revoke
        crate::actor::enrol_actor(&db, "human:erik", "kid123", "PUBKEY").unwrap();
        crate::actor::revoke_actor(&db, "human:erik").unwrap();

        let result = verify_signature_bundle(
            &db,
            "human:erik",
            "digest",
            "sig",
            "kid123",
            &now_iso(),
            &uuid_v4(),
            false, // enforce off
        );
        // record mode tolerates it: the command runs, logged unsigned
        assert!(result.ok);
        assert_eq!(result.status, "unsigned");
        assert_eq!(result.code, None);

        // an enforced company refuses the revoked key
        let enforced = verify_signature_bundle(
            &db,
            "human:erik",
            "digest",
            "sig",
            "kid123",
            &now_iso(),
            &uuid_v4(),
            true,
        );
        assert!(!enforced.ok);
        assert_eq!(enforced.code, Some("ACTOR_KEY_REVOKED"));
    }

    #[test]
    fn verify_bundle_rejects_unknown_actor() {
        let db = open_db(":memory:").unwrap();

        let result = verify_signature_bundle(
            &db,
            "human:unknown",
            "digest",
            "sig",
            "kid123",
            &now_iso(),
            &uuid_v4(),
            true, // enforce on
        );
        assert!(!result.ok);
        assert_eq!(result.code, Some("ACTOR_KEY_UNKNOWN"));
    }
}
