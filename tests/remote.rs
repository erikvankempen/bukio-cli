// bukio-cli — agent-first double-entry bookkeeping for SMEs.
// Copyright (c) 2026 Erik van Kempen.
// SPDX-License-Identifier: Apache-2.0
//
// Ported from test/remote.test.js: the remote server, token enrolment, the
// signed-envelope RPC endpoint and the client's --server mode.
//
// Unlike the JS suite (one server shared by every test, sequential), each test
// here starts its own server + company DB, so no test can observe another's
// state and they can run in parallel. The harness (RemoteEnv) kills the daemon
// when the test ends.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use serde_json::{json, Value};

// ── helpers ─────────────────────────────────────────────────────────────────

fn temp_dir(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!(
        "bukio-remote-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::create_dir_all(&p);
    p
}

fn exe() -> &'static str {
    env!("CARGO_BIN_EXE_bukio")
}

fn key_file(cfg: &str, actor: &str) -> PathBuf {
    PathBuf::from(cfg)
        .join("keys")
        .join(format!("{}.key", actor.replace(':', "-")))
}

/// A raw HTTP request (the JS suite uses fetch; no HTTP client is otherwise
/// needed). Connection: close makes the read terminate at the body.
fn http(url: &str, method: &str, path: &str, body: Option<&str>) -> (u16, String) {
    let hostport = url.trim_start_matches("http://").to_string();
    let payload = body.unwrap_or("");
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: {hostport}\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{payload}",
        payload.len()
    );
    let mut stream = TcpStream::connect(&hostport).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    stream.write_all(req.as_bytes()).unwrap();
    let mut resp = String::new();
    stream.read_to_string(&mut resp).unwrap();
    let status: u16 = resp
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let body = resp.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
    (status, body)
}

struct RemoteEnv {
    dir: PathBuf,
    cfg: String,
    db: String,
    url: String,
    server: Option<Child>,
}

impl RemoteEnv {
    /// init + enrol the operator, then start the daemon.
    fn start(tag: &str) -> RemoteEnv {
        let dir = temp_dir(tag);
        let cfg = dir.join("cfg").to_string_lossy().to_string();
        let db = dir.join("company.db").to_string_lossy().to_string();
        std::fs::create_dir_all(&cfg).unwrap();
        let mut env = RemoteEnv {
            dir,
            cfg,
            db,
            url: String::new(),
            server: None,
        };

        let (out, ok) = env.local(&[
            "init",
            "--name",
            "Remote Test BV",
            "--actor",
            "agent:op",
            "--json",
        ]);
        assert!(ok, "init: {out}");
        // the operator must be enrolled: `actor enforce` / `roles grant` are
        // signed commands against the server DB
        let (_, ok) = env.local(&["actor", "keygen", "--actor", "agent:op", "--json"]);
        assert!(ok, "keygen op");
        let (out, ok) = env.local(&["actor", "register", "--actor", "agent:op", "--json"]);
        assert!(ok, "register op: {out}");

        let mut child = Command::new(exe())
            .args([
                "server",
                "start",
                "--listen",
                "127.0.0.1:0",
                "--serve-db",
                &env.db,
                "--actor",
                "agent:op",
            ])
            .env("BUKIO_CONFIG_DIR", &env.cfg)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();

        // read stdout until the daemon announces its port (5s budget)
        let stdout = child.stdout.take().unwrap();
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        // Keep draining for the env's lifetime: returning here would drop the
        // stdout pipe, and the daemon dies on its next log write (EPIPE), which
        // looks like a refused connection to every later request.
        std::thread::spawn(move || {
            let mut announced = false;
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if !announced && line.contains("listening on") {
                    announced = true;
                    let _ = tx.send(line);
                }
            }
        });
        let line = rx
            .recv_timeout(Duration::from_secs(5))
            .unwrap_or_else(|_| panic!("server did not report a port"));
        let addr = line
            .split("listening on")
            .nth(1)
            .unwrap()
            .trim()
            .to_string();
        env.url = format!("http://{addr}");
        env.server = Some(child);
        env
    }

    /// Run a LOCAL command against the shared config dir + server DB.
    fn local(&self, args: &[&str]) -> (Value, bool) {
        let out = Command::new(exe())
            .args(args)
            .env("BUKIO_CONFIG_DIR", &self.cfg)
            .env("BUKIO_DB", &self.db)
            .env("BUKIO_ACTOR", "agent:op")
            .output()
            .unwrap();
        (
            serde_json::from_slice(&out.stdout).unwrap_or(Value::Null),
            out.status.success(),
        )
    }

    /// Run a REMOTE command through --server.
    fn remote(&self, actor: &str, args: &[&str]) -> (Value, bool) {
        let mut full = vec!["--server", self.url.as_str()];
        full.extend_from_slice(args);
        let out = Command::new(exe())
            .args(&full)
            .env("BUKIO_CONFIG_DIR", &self.cfg)
            .env("BUKIO_ACTOR", actor)
            .output()
            .unwrap();
        (
            serde_json::from_slice(&out.stdout).unwrap_or(Value::Null),
            out.status.success(),
        )
    }

    /// Raw (non-JSON) invocation, for byte-comparisons and human output.

    fn mint_token(&self, actor: &str, ttl: &str) -> String {
        let out = Command::new(exe())
            .args([
                "server",
                "token",
                actor,
                "--actor",
                actor,
                "--ttl-hours",
                ttl,
            ])
            .env("BUKIO_CONFIG_DIR", &self.cfg)
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        text.split_whitespace()
            .find(|w| {
                w.len() >= 40
                    && w.chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            })
            .unwrap_or_else(|| panic!("expected a token in output:\n{text}"))
            .to_string()
    }

    fn tokens(&self) -> Value {
        let raw = std::fs::read_to_string(PathBuf::from(&self.cfg).join("server-tokens.json"))
            .unwrap_or_else(|_| "{}".into());
        serde_json::from_str(&raw).unwrap_or_else(|_| json!({}))
    }

    fn write_tokens(&self, v: &Value) {
        std::fs::write(
            PathBuf::from(&self.cfg).join("server-tokens.json"),
            serde_json::to_string(v).unwrap(),
        )
        .unwrap();
    }

    /// A signed envelope for 'report trial-balance' with the actor's client key.
    fn envelope(
        &self,
        actor: &str,
        mutated_argv: Option<Vec<&str>>,
        drop_signature: bool,
    ) -> Value {
        let pem = std::fs::read_to_string(key_file(&self.cfg, actor)).unwrap();
        let argv: Vec<&str> =
            mutated_argv.unwrap_or(vec!["report", "trial-balance", "--actor", actor, "--json"]);
        let args = json!({ "argv": argv });
        let ts = bukio::actor::now_iso();
        let nonce = format!("{}-{}", std::process::id(), uuid_like());
        let digest =
            bukio::canonical::build_digest(actor, "report trial-balance", &args, &ts, &nonce);
        let mut env = json!({
            "v": 1, "actor": actor, "cmd": "report trial-balance", "args": args,
            "ts": ts, "nonce": nonce, "digest": digest, "sig": Value::Null, "keyid": Value::Null,
        });
        if !drop_signature {
            env["sig"] = json!(bukio::sign::sign(digest.as_bytes(), &pem).unwrap());
            let public = bukio::sign::public_key_from_private(&pem, None).unwrap();
            env["keyid"] = json!(bukio::sign::keyid_of(&public).unwrap());
        }
        env
    }

    fn post_envelope(&self, envelope: &Value) -> (u16, Value) {
        let (status, body) = http(&self.url, "POST", "/rpc", Some(&envelope.to_string()));
        (status, serde_json::from_str(&body).unwrap_or(Value::Null))
    }

    fn sha256_hex(s: &str) -> String {
        use sha2::{Digest, Sha256};
        format!("{:x}", Sha256::digest(s.as_bytes()))
    }
}

impl Drop for RemoteEnv {
    fn drop(&mut self) {
        if let Some(child) = self.server.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn uuid_like() -> String {
    use rand::RngCore;
    let mut b = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut b);
    hex::encode(b)
}

/// Start a server, run the body, tear it down.
macro_rules! with_env {
    ($tag:expr, $env:ident, $body:block) => {{
        let $env = RemoteEnv::start($tag);
        $body
    }};
}

// ── enrolment ───────────────────────────────────────────────────────────────

#[test]
fn server_token_mints_a_single_use_actor_bound_token_hashed_at_rest() {
    with_env!("tok", env, {
        let token = env.mint_token("agent:remote", "24");
        assert!(token.len() >= 40, "{token}");
        let tokens = env.tokens();
        let entries: Vec<&Value> = tokens.as_object().unwrap().values().collect();
        assert_eq!(entries.len(), 1, "{tokens}");
        assert_eq!(entries[0]["actor"], json!("agent:remote"));
        assert!(entries[0].get("usedAt").is_none() || entries[0]["usedAt"].is_null());
        // the raw token is NOT stored — only its sha256
        assert!(
            tokens.get(RemoteEnv::sha256_hex(&token)).is_some(),
            "token file must contain the sha256 of the minted token"
        );
    });
}

#[test]
fn remote_register_enrols_a_client_only_key_and_consumes_the_token() {
    with_env!("reg", env, {
        let (_, ok) = env.local(&["actor", "keygen", "--actor", "agent:remote", "--json"]);
        assert!(ok);
        let token = env.mint_token("agent:remote", "24");
        let (r, ok) = env.local(&[
            "actor",
            "register",
            "--server",
            env.url.as_str(),
            "--token",
            &token,
            "--actor",
            "agent:remote",
            "--json",
        ]);
        assert!(ok, "{r}");
        assert_eq!(r["data"]["remote"], json!(true), "{r}");
        assert_eq!(r["data"]["actor"], json!("agent:remote"));
        // the token is consumed (single-use)
        let tokens = env.tokens();
        let entry = &tokens[RemoteEnv::sha256_hex(&token)];
        assert!(
            entry.get("usedAt").map(|v| !v.is_null()).unwrap_or(false),
            "the redeemed token must be single-use: {tokens}"
        );
    });
}

#[test]
fn remote_register_refuses_a_used_token() {
    with_env!("used", env, {
        env.local(&["actor", "keygen", "--actor", "agent:second", "--json"]);
        let token = env.mint_token("agent:second", "24");
        let (_, ok) = env.local(&[
            "actor",
            "register",
            "--server",
            env.url.as_str(),
            "--token",
            &token,
            "--actor",
            "agent:second",
            "--json",
        ]);
        assert!(ok);
        let (r, ok) = env.local(&[
            "actor",
            "register",
            "--server",
            env.url.as_str(),
            "--token",
            &token,
            "--actor",
            "agent:second",
            "--json",
        ]);
        assert!(!ok);
        assert_eq!(r["error"]["code"], json!("TOKEN_USED"), "{r}");
    });
}

#[test]
fn remote_register_refuses_an_unknown_and_a_mismatched_token() {
    with_env!("bad", env, {
        // register reads the local key before it validates the token, so the
        // actor needs a key file for the token error to surface
        env.local(&["actor", "keygen", "--actor", "agent:second", "--json"]);
        let (bad, ok) = env.local(&[
            "actor",
            "register",
            "--server",
            env.url.as_str(),
            "--token",
            "not-a-real-token-1234567890abcdef",
            "--actor",
            "agent:second",
            "--json",
        ]);
        assert!(!ok);
        assert_eq!(bad["error"]["code"], json!("TOKEN_INVALID"), "{bad}");

        // a token minted for agent:second must not enrol a DIFFERENT actor
        env.local(&["actor", "keygen", "--actor", "agent:other", "--json"]);
        let token = env.mint_token("agent:second", "24");
        let (mismatch, ok) = env.local(&[
            "actor",
            "register",
            "--server",
            env.url.as_str(),
            "--token",
            &token,
            "--actor",
            "agent:other",
            "--json",
        ]);
        assert!(!ok);
        assert_eq!(
            mismatch["error"]["code"],
            json!("TOKEN_ACTOR_MISMATCH"),
            "{mismatch}"
        );
    });
}

#[test]
fn remote_register_requires_a_token_with_server() {
    with_env!("notoken", env, {
        let (r, ok) = env.local(&[
            "actor",
            "register",
            "--server",
            env.url.as_str(),
            "--actor",
            "agent:second",
            "--json",
        ]);
        assert!(!ok);
        assert_eq!(r["error"]["code"], json!("TOKEN_REQUIRED"), "{r}");
    });
}

#[test]
fn remote_register_refuses_an_expired_token() {
    with_env!("exp", env, {
        env.local(&["actor", "keygen", "--actor", "agent:expired", "--json"]);
        let token = env.mint_token("agent:expired", "1");
        // backdate every stored entry to expire it
        let mut tokens = env.tokens();
        for (_, v) in tokens.as_object_mut().unwrap().iter_mut() {
            v["expiresAt"] = json!("2020-01-01T00:00:00.000Z");
        }
        env.write_tokens(&tokens);
        let (r, ok) = env.local(&[
            "actor",
            "register",
            "--server",
            env.url.as_str(),
            "--token",
            &token,
            "--actor",
            "agent:expired",
            "--json",
        ]);
        assert!(!ok);
        assert_eq!(r["error"]["code"], json!("TOKEN_EXPIRED"), "{r}");
    });
}

// ── remote execution ────────────────────────────────────────────────────────

#[test]
fn remote_read_matches_the_local_view() {
    with_env!("read", env, {
        env.local(&["actor", "keygen", "--actor", "agent:remote", "--json"]);
        let token = env.mint_token("agent:remote", "24");
        env.local(&[
            "actor",
            "register",
            "--server",
            env.url.as_str(),
            "--token",
            &token,
            "--actor",
            "agent:remote",
            "--json",
        ]);
        let (r, ok) = env.remote("agent:remote", &["report", "trial-balance", "--json"]);
        assert!(ok, "{r}");
        assert_eq!(r["data"]["balanced"], json!(true), "{r}");
        // same-device parity with the local view
        let (l, ok) = env.local(&[
            "report",
            "trial-balance",
            "--actor",
            "agent:remote",
            "--json",
        ]);
        assert!(ok, "{l}");
        assert_eq!(l["data"], r["data"], "remote and local must agree");
    });
}

#[test]
fn remote_mutation_posts_an_entry_with_a_real_signature_in_the_audit_row() {
    with_env!("mut", env, {
        env.local(&["actor", "keygen", "--actor", "agent:remote", "--json"]);
        let token = env.mint_token("agent:remote", "24");
        env.local(&[
            "actor",
            "register",
            "--server",
            env.url.as_str(),
            "--token",
            &token,
            "--actor",
            "agent:remote",
            "--json",
        ]);
        let (r, ok) = env.remote(
            "agent:remote",
            &[
                "entry",
                "add",
                "--date",
                "2026-08-11",
                "--desc",
                "remote booking",
                "--postings",
                "1000:25000,8000:-25000",
                "--json",
            ],
        );
        assert!(ok, "{r}");
        assert_eq!(r["data"]["id"], json!(1), "{r}");

        let (audit, ok) = env.local(&["audit", "--actor", "agent:op", "--json", "--limit", "50"]);
        assert!(ok, "{audit}");
        let row = audit["data"]["entries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["action"] == json!("entry.create"))
            .expect("audit must contain the remote entry")
            .clone();
        assert_eq!(row["sig_status"], json!("verified"), "{row}");
        assert_eq!(row["sig_keyid"].as_str().unwrap().len(), 32);
        assert!(!row["sig"].is_null());
        assert!(!row["digest_hash"].is_null());

        let (v, ok) = env.local(&["audit", "verify", "--actor", "agent:op", "--json"]);
        assert!(ok, "{v}");
        assert_eq!(v["data"]["summary"]["tampered"], json!(0), "{v}");
        assert_eq!(v["data"]["summary"]["invalid_signature"], json!(0));
    });
}

#[test]
fn remote_mutation_dry_run_has_no_side_effect() {
    with_env!("dry", env, {
        env.local(&["actor", "keygen", "--actor", "agent:remote", "--json"]);
        let token = env.mint_token("agent:remote", "24");
        env.local(&[
            "actor",
            "register",
            "--server",
            env.url.as_str(),
            "--token",
            &token,
            "--actor",
            "agent:remote",
            "--json",
        ]);
        let (before, _) = env.local(&["entry", "list", "--actor", "agent:op", "--json"]);
        let before_n = before["data"]["entries"]
            .as_array()
            .map(|a| a.len())
            .unwrap_or(0);

        let (r, ok) = env.remote(
            "agent:remote",
            &[
                "entry",
                "add",
                "--date",
                "2026-08-12",
                "--desc",
                "dry remote",
                "--postings",
                "1000:1000,8000:-1000",
                "--dry-run",
                "--json",
            ],
        );
        assert!(ok, "{r}");
        assert_eq!(r["data"]["dryRun"], json!(true), "{r}");

        let (after, _) = env.local(&["entry", "list", "--actor", "agent:op", "--json"]);
        let after_n = after["data"]["entries"]
            .as_array()
            .map(|a| a.len())
            .unwrap_or(0);
        assert_eq!(after_n, before_n, "dry-run must not mutate the books");
    });
}

#[test]
fn remote_human_output_is_byte_identical_to_local_human_output() {
    with_env!("human", env, {
        let remote_text = Command::new(exe())
            .args(["--server", env.url.as_str(), "report", "trial-balance"])
            .env("BUKIO_CONFIG_DIR", &env.cfg)
            .env("BUKIO_ACTOR", "agent:remote")
            .output()
            .unwrap();
        let local_text = Command::new(exe())
            .args(["report", "trial-balance"])
            .env("BUKIO_CONFIG_DIR", &env.cfg)
            .env("BUKIO_ACTOR", "agent:remote")
            .env("BUKIO_DB", &env.db)
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&remote_text.stdout),
            String::from_utf8_lossy(&local_text.stdout)
        );
    });
}

// ── negative envelope tests (raw HTTP) ──────────────────────────────────────

#[test]
fn replay_of_the_same_envelope_is_refused() {
    with_env!("replay", env, {
        env.local(&["actor", "keygen", "--actor", "agent:remote", "--json"]);
        let token = env.mint_token("agent:remote", "24");
        env.local(&[
            "actor",
            "register",
            "--server",
            env.url.as_str(),
            "--token",
            &token,
            "--actor",
            "agent:remote",
            "--json",
        ]);
        let envelope = env.envelope("agent:remote", None, false);
        let (status, _) = env.post_envelope(&envelope);
        assert_eq!(status, 200);
        let (_, body) = env.post_envelope(&envelope);
        assert_eq!(body["ok"], json!(false), "{body}");
        assert_eq!(body["error"]["code"], json!("NONCE_REUSED"), "{body}");
    });
}

#[test]
fn a_tampered_envelope_is_signature_invalid_under_enforce() {
    with_env!("tamper", env, {
        env.local(&["actor", "keygen", "--actor", "agent:remote", "--json"]);
        let token = env.mint_token("agent:remote", "24");
        env.local(&[
            "actor",
            "register",
            "--server",
            env.url.as_str(),
            "--token",
            &token,
            "--actor",
            "agent:remote",
            "--json",
        ]);
        let (_, ok) = env.local(&["actor", "enforce", "--on", "--actor", "agent:op", "--json"]);
        assert!(ok);

        // sign over a MUTATED argv, then transmit the original: the digest no
        // longer matches what was signed
        let mut envelope = env.envelope(
            "agent:remote",
            Some(vec![
                "report",
                "trial-balance",
                "--actor",
                "agent:remote",
                "--json",
                "--mutated",
            ]),
            false,
        );
        envelope["args"]["argv"] = json!([
            "report",
            "trial-balance",
            "--actor",
            "agent:remote",
            "--json"
        ]);
        let (_, body) = env.post_envelope(&envelope);
        assert_eq!(body["ok"], json!(false), "{body}");
        assert_eq!(body["error"]["code"], json!("SIGNATURE_INVALID"), "{body}");

        let (_, ok) = env.local(&["actor", "enforce", "--off", "--actor", "agent:op", "--json"]);
        assert!(ok);
    });
}

#[test]
fn an_unsigned_envelope_is_refused_under_enforce_but_signed_ones_work() {
    with_env!("unsigned", env, {
        env.local(&["actor", "keygen", "--actor", "agent:remote", "--json"]);
        let token = env.mint_token("agent:remote", "24");
        env.local(&[
            "actor",
            "register",
            "--server",
            env.url.as_str(),
            "--token",
            &token,
            "--actor",
            "agent:remote",
            "--json",
        ]);
        let (_, ok) = env.local(&["actor", "enforce", "--on", "--actor", "agent:op", "--json"]);
        assert!(ok);

        let unsigned = env.envelope("agent:remote", None, true);
        let (_, body) = env.post_envelope(&unsigned);
        assert_eq!(body["ok"], json!(false), "{body}");
        assert_eq!(body["error"]["code"], json!("SIGNATURE_REQUIRED"), "{body}");

        let signed = env.envelope("agent:remote", None, false);
        let (status, body) = env.post_envelope(&signed);
        assert_eq!(status, 200, "{body}");

        let (_, ok) = env.local(&["actor", "enforce", "--off", "--actor", "agent:op", "--json"]);
        assert!(ok);
    });
}

#[test]
fn a_readonly_actor_is_refused_a_mutation_under_authz() {
    with_env!("authz", env, {
        env.local(&["actor", "keygen", "--actor", "agent:readonly", "--json"]);
        let token = env.mint_token("agent:readonly", "24");
        env.local(&[
            "actor",
            "register",
            "--server",
            env.url.as_str(),
            "--token",
            &token,
            "--actor",
            "agent:readonly",
            "--json",
        ]);
        let (_, ok) = env.local(&[
            "actor",
            "roles",
            "grant",
            "readonly",
            "--for",
            "agent:readonly",
            "--actor",
            "agent:op",
            "--json",
        ]);
        assert!(ok);
        let (_, ok) = env.local(&["actor", "authz", "--on", "--actor", "agent:op", "--json"]);
        assert!(ok);

        let (body, ok) = env.remote(
            "agent:readonly",
            &[
                "entry",
                "add",
                "--date",
                "2026-08-13",
                "--desc",
                "nope",
                "--postings",
                "1000:1,8000:-1",
                "--json",
            ],
        );
        assert!(!ok);
        assert_eq!(body["ok"], json!(false), "{body}");
        assert_eq!(body["error"]["code"], json!("AUTHZ_DENIED"), "{body}");

        let (_, ok) = env.local(&["actor", "authz", "--off", "--actor", "agent:op", "--json"]);
        assert!(ok);
    });
}

// ── local-only commands ─────────────────────────────────────────────────────

#[test]
fn local_only_commands_refuse_under_server() {
    with_env!("localonly", env, {
        for args in [
            vec!["actor", "keygen", "--actor", "agent:localonly"],
            vec!["mcp"],
            vec!["server", "token", "agent:x", "--actor", "agent:x"],
            vec!["server", "start", "--actor", "agent:x"],
        ] {
            let mut full = vec!["--server", env.url.as_str()];
            full.extend_from_slice(&args);
            full.push("--json");
            let out = Command::new(exe())
                .args(&full)
                .env("BUKIO_CONFIG_DIR", &env.cfg)
                .env("BUKIO_ACTOR", "agent:remote")
                .output()
                .unwrap();
            let body: Value = serde_json::from_slice(&out.stdout).unwrap_or(Value::Null);
            assert!(
                !body.is_null(),
                "expected LOCAL_ONLY for {}",
                args.join(" ")
            );
            assert_eq!(
                body["ok"],
                json!(false),
                "expected failure for {}",
                args.join(" ")
            );
            assert_eq!(
                body["error"]["code"],
                json!("LOCAL_ONLY"),
                "expected LOCAL_ONLY for {}: {body}",
                args.join(" ")
            );
        }
    });
}

// ── health / misc ───────────────────────────────────────────────────────────

#[test]
fn health_endpoint_reports_ok_and_unknown_routes_are_404() {
    with_env!("health", env, {
        let mut ok_res = None;
        for _ in 0..3 {
            if let Ok(r) = std::panic::catch_unwind(|| http(&env.url, "GET", "/health", None)) {
                ok_res = Some(r);
                break;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        let (status, body) = ok_res.expect("health must answer");
        assert_eq!(status, 200);
        let parsed: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["ok"], json!(true), "{parsed}");
        assert_eq!(parsed["data"]["status"], json!("ok"));
    });
}

#[test]
fn unknown_routes_are_404() {
    with_env!("nope", env, {
        let (status, _) = http(&env.url, "GET", "/nope", None);
        assert_eq!(status, 404);
    });
}

#[test]
fn an_unreachable_server_is_a_clean_remote_unreachable() {
    with_env!("unreach", env, {
        let out = Command::new(exe())
            .args([
                "--server",
                "http://127.0.0.1:1",
                "report",
                "trial-balance",
                "--json",
            ])
            .env("BUKIO_CONFIG_DIR", &env.cfg)
            .env("BUKIO_ACTOR", "agent:remote")
            .output()
            .unwrap();
        let body: Value = serde_json::from_slice(&out.stdout).unwrap_or(Value::Null);
        assert!(!body.is_null(), "expected a JSON error");
        assert_eq!(body["ok"], json!(false), "{body}");
        assert_eq!(body["error"]["code"], json!("REMOTE_UNREACHABLE"), "{body}");
    });
}

#[test]
fn server_token_rejects_a_bad_ttl() {
    with_env!("ttl", env, {
        let out = Command::new(exe())
            .args([
                "server",
                "token",
                "agent:x",
                "--actor",
                "agent:x",
                "--ttl-hours",
                "abc",
                "--json",
            ])
            .env("BUKIO_CONFIG_DIR", &env.cfg)
            .output()
            .unwrap();
        let body: Value = serde_json::from_slice(&out.stdout).unwrap_or(Value::Null);
        assert!(!body.is_null());
        assert_eq!(body["ok"], json!(false), "{body}");
        assert_eq!(body["error"]["code"], json!("INVALID_TTL"), "{body}");
    });
}

#[test]
fn an_envelope_may_carry_a_client_db_but_the_server_db_is_authoritative() {
    with_env!("dbstrip", env, {
        env.local(&["actor", "keygen", "--actor", "agent:remote", "--json"]);
        let token = env.mint_token("agent:remote", "24");
        env.local(&[
            "actor",
            "register",
            "--server",
            env.url.as_str(),
            "--token",
            &token,
            "--actor",
            "agent:remote",
            "--json",
        ]);
        // a signed envelope naming a BOGUS --db must still run against the
        // server's company DB (sanitizeArgv strips it)
        let pem = std::fs::read_to_string(key_file(&env.cfg, "agent:remote")).unwrap();
        let args = json!({ "argv": ["report", "trial-balance", "--actor", "agent:remote", "--db", "/tmp/bogus.db", "--json"] });
        let ts = bukio::actor::now_iso();
        let nonce = uuid_like();
        let digest = bukio::canonical::build_digest(
            "agent:remote",
            "report trial-balance",
            &args,
            &ts,
            &nonce,
        );
        let envelope = json!({
            "v": 1, "actor": "agent:remote", "cmd": "report trial-balance", "args": args,
            "ts": ts, "nonce": nonce, "digest": digest,
            "sig": bukio::sign::sign(digest.as_bytes(), &pem).unwrap(),
            "keyid": bukio::sign::keyid_of(&bukio::sign::public_key_from_private(&pem, None).unwrap()).unwrap(),
        });
        let (status, body) = env.post_envelope(&envelope);
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["ok"], json!(true), "{body}");
        assert_eq!(body["exitCode"], json!(0), "{body}");
    });
}
