// bukio-cli — agent-first double-entry bookkeeping for SMEs.
// Copyright (c) 2026 Erik van Kempen.
// SPDX-License-Identifier: Apache-2.0
//
// Server — HTTP JSON-RPC remote execution. Tokens, signature gate, child process dispatch.

use crate::actor::{get_authz, get_enforce};
use crate::db::open_db;
use crate::money::{BukioError, Result};
use crate::sign_gate;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const TOKENS_FILE: &str = "server-tokens.json";
const MAX_BODY_BYTES: usize = 1024 * 1024;

// ponytail: exact match like JS REMOTE_LOCAL_ONLY.has(cmd)
fn is_local_only(cmd: &str) -> bool {
    matches!(
        cmd,
        "server start"
            | "server token"
            | "mcp"
            | "init"
            | "update"
            | "actor keygen"
            | "actor unlock"
            | "actor lock"
    )
}

fn config_dir() -> PathBuf {
    std::env::var("BUKIO_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".to_string())).join(".bukio")
        })
}

fn tokens_path() -> PathBuf {
    config_dir().join(TOKENS_FILE)
}

// --- Token store -----------------------------------------------------------

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
struct TokenEntry {
    actor: String,
    #[serde(rename = "createdAt")]
    created_at: String,
    #[serde(rename = "expiresAt")]
    expires_at: String,
    #[serde(rename = "usedAt")]
    used_at: Option<String>,
}

fn read_tokens() -> HashMap<String, TokenEntry> {
    let path = tokens_path();
    fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn write_tokens(tokens: &HashMap<String, TokenEntry>) {
    let path = tokens_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).ok();
    }
    let json = serde_json::to_string_pretty(tokens).unwrap_or_default();
    fs::write(&path, json).ok();
}

/// Mint a one-time enrolment token for an actor.
pub fn mint_enrol_token(actor: &str, ttl_hours: u64) -> Result<String> {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    let token = {
        use base64::Engine as _;
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&bytes)
    };
    let hash = hex::encode(Sha256::digest(token.as_bytes()));
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let created = format_timestamp(now);
    let expires = format_timestamp(now + ttl_hours * 3600);
    let mut tokens = read_tokens();
    tokens.insert(
        hash,
        TokenEntry {
            actor: actor.to_string(),
            created_at: created,
            expires_at: expires,
            used_at: None,
        },
    );
    write_tokens(&tokens);
    Ok(token)
}

/// Consume (redeem) a one-time enrolment token.
pub fn consume_enrol_token(token: &str, actor: &str) -> Result<()> {
    if token.is_empty() {
        return Err(BukioError::new(
            "TOKEN_INVALID",
            "an enrolment token is required",
        ));
    }
    let hash = hex::encode(Sha256::digest(token.as_bytes()));
    let mut tokens = read_tokens();
    let entry = tokens
        .get_mut(&hash)
        .ok_or_else(|| BukioError::new("TOKEN_INVALID", "unknown enrolment token"))?;
    if entry.actor != actor {
        return Err(BukioError::new(
            "TOKEN_ACTOR_MISMATCH",
            format!("token was minted for {}, not {}", entry.actor, actor),
        ));
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let expires = parse_timestamp(&entry.expires_at).unwrap_or(0);
    if expires < now {
        return Err(BukioError::new(
            "TOKEN_EXPIRED",
            "enrolment token has expired — ask the operator for a fresh one",
        ));
    }
    if entry.used_at.is_some() {
        return Err(BukioError::new(
            "TOKEN_USED",
            "enrolment token was already used (single-use)",
        ));
    }
    entry.used_at = Some(format_timestamp(now));
    write_tokens(&tokens);
    Ok(())
}

// --- Envelope verification -------------------------------------------------

fn verify_envelope(db: &rusqlite::Connection, envelope: &Value) -> Result<Value> {
    let actor = envelope["actor"].as_str().unwrap_or("");
    let cmd = envelope["cmd"].as_str().unwrap_or("");
    let sig = envelope["sig"].as_str();
    let keyid = envelope["keyid"].as_str();
    let ts = envelope["ts"].as_str().unwrap_or("");
    let nonce = envelope["nonce"].as_str().unwrap_or("");
    let digest = envelope["digest"].as_str().unwrap_or("");

    // LOCAL_ONLY blacklist (defense in depth)
    if is_local_only(cmd) {
        return Err(BukioError::new(
            "LOCAL_ONLY",
            format!("'{cmd}' cannot run remotely — it is a local/operator command"),
        ));
    }

    let enforce = get_enforce(db);

    // No signature → unsigned record mode (only if not enforced)
    if sig.is_none() || keyid.is_none() {
        if enforce {
            return Err(BukioError::new(
                "SIGNATURE_REQUIRED",
                format!(
                    "no signature in envelope for {actor} — the company enforces signed commands"
                ),
            ));
        }
        return Ok(json!({"ok": true, "sigStatus": "unsigned"}));
    }

    let sig = sig.unwrap();
    let keyid = keyid.unwrap();

    // Recompute the digest over the TRANSMITTED args — a digest that does
    // not match what was actually sent means the signed payload differs
    // from the executed argv (tamper refusal, JS parity)
    let recomputed = crate::canonical::build_digest(actor, cmd, &envelope["args"], ts, nonce);
    if digest.is_empty() || recomputed != digest {
        return Err(BukioError::new(
            "SIGNATURE_INVALID",
            "signature does not cover the transmitted args — the envelope was tampered with",
        ));
    }

    // Full Tier 0 gate: nonce replay, timestamp window, registry check, sig verify
    let gate =
        sign_gate::verify_signature_bundle(db, actor, digest, sig, keyid, ts, nonce, enforce);

    if !gate.ok {
        let code = gate.code.unwrap_or("SIGNATURE_FAILED");
        let messages = [
            (
                "ACTOR_KEY_UNKNOWN",
                format!("actor {actor} has no enrolled key"),
            ),
            (
                "ACTOR_KEY_REVOKED",
                format!("the key for {actor} is revoked"),
            ),
            (
                "SIGNATURE_STALE",
                "signature timestamp is outside the ±5 minute window".into(),
            ),
            (
                "NONCE_REUSED",
                "signature nonce was already used — a replayed command is refused".into(),
            ),
            (
                "SIGNATURE_INVALID",
                format!("signature does not verify against the enrolled key for {actor}"),
            ),
        ];
        let message = messages
            .iter()
            .find(|(c, _)| *c == code)
            .map(|(_, m)| m.clone())
            .unwrap_or_else(|| format!("signature verification failed for {actor}"));
        return Err(BukioError::new(code, message));
    }

    // Authz gate (Tier 0.5)
    if get_authz(db) {
        crate::authz::check_authz(db, actor, cmd, false, false)?;
    }

    Ok(json!({"ok": true, "sigStatus": "verified"}))
}

// --- Child process dispatch ------------------------------------------------

/// Remove transport flags (+ their values) from argv.
fn sanitize_argv(argv: &[String]) -> Vec<String> {
    let transport_flags = ["--server", "--db", "--sign-key"];
    let mut out = Vec::new();
    let mut i = 0;
    while i < argv.len() {
        let tok = &argv[i];
        if tok == "--" {
            out.push(tok.clone());
            // everything after -- is kept
            for t in &argv[i + 1..] {
                out.push(t.clone());
            }
            break;
        }
        let flag = if let Some(eq) = tok.strip_prefix("--") {
            eq.split_once('=')
                .map(|(k, _)| format!("--{k}"))
                .unwrap_or_else(|| tok.clone())
        } else {
            tok.clone()
        };
        if transport_flags.contains(&flag.as_str()) {
            // skip the flag and its value (if not --flag=value form and next token isn't a flag)
            if !tok.contains('=') && i + 1 < argv.len() && !argv[i + 1].starts_with('-') {
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }
        out.push(tok.clone());
        i += 1;
    }
    out
}

fn run_child(db_path: &str, argv: &[String], env_extra: Option<&str>) -> Result<Value> {
    let safe_argv = sanitize_argv(argv);
    let mut cmd = Command::new(std::env::current_exe().unwrap_or_else(|_| PathBuf::from("bukio")));
    cmd.arg("--db").arg(db_path);
    for arg in &safe_argv {
        cmd.arg(arg);
    }
    cmd.env("BUKIO_REMOTE_EXEC", "1");
    // Pass BUKIO_CONFIG_DIR to child so keys/nonces resolve
    if let Ok(cfg) = std::env::var("BUKIO_CONFIG_DIR") {
        cmd.env("BUKIO_CONFIG_DIR", cfg);
    }
    if let Some(env) = env_extra {
        cmd.env("BUKIO_REMOTE_SIG", env);
    }

    let output = cmd
        .output()
        .map_err(|e| BukioError::new("SERVER_EXEC", format!("failed to spawn CLI: {e}")))?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let exit_code = output.status.code().unwrap_or(1);

    Ok(json!({
        "ok": exit_code == 0,
        "stdout": stdout,
        "stderr": stderr,
        "exitCode": exit_code,
    }))
}

// --- HTTP server -----------------------------------------------------------

fn read_json_body(reader: &mut dyn Read, content_length: usize) -> Result<Value> {
    if content_length > MAX_BODY_BYTES {
        return Err(BukioError::new(
            "BODY_TOO_LARGE",
            format!("request body exceeds {MAX_BODY_BYTES} bytes"),
        ));
    }
    let mut body = vec![0u8; content_length];
    reader
        .read_exact(&mut body)
        .map_err(|e| BukioError::new("BAD_JSON", format!("read error: {e}")))?;
    serde_json::from_slice(&body)
        .map_err(|e| BukioError::new("BAD_JSON", format!("request body is not valid JSON: {e}")))
}

fn send_json_response(writer: &mut dyn Write, status: &str, payload: &Value) {
    let body = serde_json::to_vec(payload).unwrap_or_default();
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    writer.write_all(response.as_bytes()).ok();
    writer.write_all(&body).ok();
}

fn handle_request(db_path: &str, method: &str, path: &str, body: Value) -> (String, Value) {
    match (method, path) {
        ("GET", "/health") => (
            "200 OK".into(),
            json!({"ok": true, "data": {"status": "ok"}}),
        ),

        ("POST", "/register") => {
            let db = match open_db(db_path) {
                Ok(db) => db,
                Err(e) => {
                    return (
                        "500 Internal Server Error".into(),
                        json!({"ok": false, "error": {"code": "ERROR", "message": e.to_string()}}),
                    )
                }
            };
            let actor = body["actor"].as_str().unwrap_or("");
            let keyid = body["keyid"].as_str().unwrap_or("");
            let public_key = body["publicKey"].as_str().unwrap_or("");
            let token = body["token"].as_str().unwrap_or("");
            match consume_enrol_token(token, actor) {
                Ok(()) => match crate::actor::enrol_actor(&db, actor, keyid, public_key) {
                    Ok(row) => {
                        let _ = crate::audit::record(
                            &db,
                            crate::audit::RecordArgs {
                                actor,
                                action: "actor.register",
                                command: Some("actor register"),
                                args: Some(json!({"actor": actor, "keyid": keyid, "remote": true})),
                                outcome: "ok",
                                entry_ids: vec![],
                            },
                        );
                        (
                            "200 OK".into(),
                            json!({"ok": true, "data": {"actor": row["actor"], "keyid": row["keyid"], "enrolled_at": row["enrolled_at"], "remote": true}}),
                        )
                    }
                    Err(e) => (
                        "401 Unauthorized".into(),
                        json!({"ok": false, "error": {"code": e.code, "message": e.message}}),
                    ),
                },
                Err(e) => (
                    "401 Unauthorized".into(),
                    json!({"ok": false, "error": {"code": e.code, "message": e.message}}),
                ),
            }
        }

        ("POST", "/rpc") => {
            let db = match open_db(db_path) {
                Ok(db) => db,
                Err(e) => {
                    return (
                        "500 Internal Server Error".into(),
                        json!({"ok": false, "error": {"code": "ERROR", "message": e.to_string()}}),
                    )
                }
            };
            match verify_envelope(&db, &body) {
                Ok(_gate) => {
                    let argv: Vec<String> = body["args"]["argv"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(|v| v.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default();
                    if argv.is_empty() {
                        return (
                            "400 Bad Request".into(),
                            json!({"ok": false, "error": {"code": "INVALID_ENVELOPE", "message": "args.argv missing"}}),
                        );
                    }
                    let sig_bundle = serde_json::to_string(&body).unwrap_or_default();
                    match run_child(db_path, &argv, Some(&sig_bundle)) {
                        Ok(r) => ("200 OK".into(), r),
                        Err(e) => (
                            "500 Internal Server Error".into(),
                            json!({"ok": false, "error": {"code": "ERROR", "message": e.to_string()}}),
                        ),
                    }
                }
                Err(e) => (
                    "401 Unauthorized".into(),
                    json!({"ok": false, "error": {"code": e.code, "message": e.message}}),
                ),
            }
        }

        _ => (
            "404 Not Found".into(),
            json!({"ok": false, "error": {"code": "NOT_FOUND", "message": "unknown endpoint"}}),
        ),
    }
}

/// Start the HTTP server.
pub fn cmd_server_start(db_path: &str, port: u16, host: &str) -> Result<()> {
    let addr = format!("{host}:{port}");
    let listener = TcpListener::bind(&addr)
        .map_err(|e| BukioError::new("SERVER_START", format!("cannot bind {addr}: {e}")))?;
    // Print the ACTUAL bound address (port 0 → ephemeral port), like JS srv.address()
    let shown = listener
        .local_addr()
        .map(|a| format!("{}:{}", a.ip(), a.port()))
        .unwrap_or_else(|_| addr.clone());
    // Print to stdout (not stderr) — test reads stdout
    println!("listening on {shown}");
    println!("serving company DB: {db_path}");
    std::io::stdout().flush().ok();
    for stream in listener.incoming() {
        let stream = match stream {
            Ok(s) => s,
            Err(_) => continue,
        };
        let db_path = db_path.to_string();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(&stream);
            let mut writer = &stream;

            // Parse request line
            let mut request_line = String::new();
            if reader.read_line(&mut request_line).is_err() {
                return;
            }
            let parts: Vec<&str> = request_line.trim().split_whitespace().collect();
            if parts.len() < 2 {
                send_json_response(&mut writer, "400 Bad Request", &json!({"ok": false}));
                return;
            }
            let method = parts[0];
            let path = parts[1];

            // Read headers
            let mut content_length = 0;
            loop {
                let mut header = String::new();
                if reader.read_line(&mut header).is_err() || header.trim().is_empty() {
                    break;
                }
                // HTTP header names are case-insensitive; Node's fetch sends lowercase.
                if let Some((name, val)) = header.split_once(':') {
                    if name.trim().eq_ignore_ascii_case("content-length") {
                        content_length = val.trim().parse().unwrap_or(0);
                    }
                }
            }

            // Read body
            let body = if content_length > 0 {
                read_json_body(&mut reader, content_length).unwrap_or_else(|e| {
                    send_json_response(
                        &mut writer,
                        "400 Bad Request",
                        &json!({"ok": false, "error": {"code": "ERROR", "message": e.to_string()}}),
                    );
                    Value::Null
                })
            } else {
                json!({})
            };

            if body.is_null() {
                return;
            }

            let (status, payload) = handle_request(&db_path, method, path, body);
            send_json_response(&mut writer, &status, &payload);
        });
    }
    Ok(())
}

// --- Helpers ---------------------------------------------------------------

fn format_timestamp(secs: u64) -> String {
    let dt: chrono::DateTime<chrono::Utc> =
        chrono::DateTime::from_timestamp(secs as i64, 0).unwrap_or_default();
    dt.to_rfc3339()
}

fn parse_timestamp(s: &str) -> Option<u64> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.timestamp() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mint_and_consume_token() {
        let token = mint_enrol_token("agent:bartholomeus", 24).unwrap();
        assert!(!token.is_empty());
        assert!(token.len() >= 40);
    }

    #[test]
    fn format_parse_timestamp_roundtrip() {
        let now = 1700000000u64;
        let s = format_timestamp(now);
        let p = parse_timestamp(&s).unwrap();
        assert_eq!(now, p);
    }
}
