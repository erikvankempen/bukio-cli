// bukio-cli — agent-first double-entry bookkeeping for SMEs.
// Copyright (c) 2026 Erik van Kempen.
// SPDX-License-Identifier: Apache-2.0
//
// Ported from test/smtp.test.js. The mock SMTP server is a plain TcpListener
// thread (the JS mock is a net.createServer): it tracks connections, captures
// the DATA payload and can be told to reject auth / RCPT / greeting / STARTTLS.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bukio::smtp::{build_mime, send_mail, smtp_config, smtp_validate, Attachment, SmtpConfig};
use serde_json::{json, Value};

// ── helpers ─────────────────────────────────────────────────────────────────

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!(
        "bukio-smtp-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::create_dir_all(&p);
    p
}

/// The CLI resolver order is --db → BUKIO_DB; --actor → BUKIO_ACTOR. Passed
/// through Command::env so nothing here mutates the test process's env.
fn run_cli(args: &[&str], env: &[(&str, &str)]) -> (Value, bool, String) {
    let exe = env!("CARGO_BIN_EXE_bukio");
    let mut cmd = std::process::Command::new(exe);
    cmd.env("BUKIO_ACTOR", "agent:test");
    // a stray BUKIO_SMTP_* in the parent would leak into the unconfigured case
    for k in [
        "BUKIO_SMTP_HOST",
        "BUKIO_SMTP_PORT",
        "BUKIO_SMTP_USER",
        "BUKIO_SMTP_PASS",
        "BUKIO_SMTP_FROM",
        "BUKIO_SMTP_SECURE",
    ] {
        cmd.env_remove(k);
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.args(args).output().unwrap();
    (
        serde_json::from_slice(&out.stdout).unwrap_or(Value::Null),
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).to_string(),
    )
}

/// A blocking guard for the few tests that must set the process env (the
/// engine reads smtp_config() itself). Serialises them so they cannot race.
static ENV_LOCK: Mutex<()> = Mutex::new(());

struct EnvGuard {
    saved: Vec<(&'static str, Option<String>)>,
}

impl EnvGuard {
    fn set(vars: &[(&'static str, &str)]) -> EnvGuard {
        const KEYS: [&str; 6] = [
            "BUKIO_SMTP_HOST",
            "BUKIO_SMTP_PORT",
            "BUKIO_SMTP_USER",
            "BUKIO_SMTP_PASS",
            "BUKIO_SMTP_FROM",
            "BUKIO_SMTP_SECURE",
        ];
        let saved = KEYS
            .iter()
            .map(|k| (*k, std::env::var(k).ok()))
            .collect::<Vec<_>>();
        for k in KEYS {
            std::env::remove_var(k);
        }
        for (k, v) in vars {
            std::env::set_var(k, v);
        }
        EnvGuard { saved }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (k, v) in &self.saved {
            match v {
                Some(val) => std::env::set_var(k, val),
                None => std::env::remove_var(k),
            }
        }
    }
}

// ── the mock server ─────────────────────────────────────────────────────────

#[derive(Default, Clone, Copy)]
struct MockOpts {
    fail_auth: bool,
    fail_rcpt: bool,
    fail_greeting: bool,
    advertise_starttls: bool,
    reject_starttls: bool,
}

struct Mock {
    port: u16,
    connections: Arc<AtomicUsize>,
    last_data: Arc<Mutex<Option<String>>>,
    starttls_seen: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
}

impl Mock {
    fn start(opts: MockOpts) -> Mock {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let connections = Arc::new(AtomicUsize::new(0));
        let last_data = Arc::new(Mutex::new(None));
        let starttls_seen = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));

        let (c, d, s, st) = (
            connections.clone(),
            last_data.clone(),
            starttls_seen.clone(),
            stop.clone(),
        );
        std::thread::spawn(move || loop {
            if st.load(Ordering::SeqCst) {
                return;
            }
            match listener.accept() {
                Ok((socket, _)) => {
                    c.fetch_add(1, Ordering::SeqCst);
                    let (d, s) = (d.clone(), s.clone());
                    std::thread::spawn(move || serve(socket, opts, d, s));
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(_) => return,
            }
        });

        Mock {
            port,
            connections,
            last_data,
            starttls_seen,
            stop,
        }
    }

    fn body(&self) -> String {
        self.last_data.lock().unwrap().clone().unwrap_or_default()
    }

    fn cfg(&self, user: bool) -> SmtpConfig {
        SmtpConfig {
            host: "127.0.0.1".into(),
            port: self.port,
            secure: false,
            user: if user { Some("u".into()) } else { None },
            pass: if user { Some("p".into()) } else { None },
            from: "no-reply@test.example".into(),
        }
    }
}

impl Drop for Mock {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

fn serve(
    socket: TcpStream,
    opts: MockOpts,
    last_data: Arc<Mutex<Option<String>>>,
    starttls_seen: Arc<AtomicBool>,
) {
    let _ = socket.set_read_timeout(Some(Duration::from_secs(5)));
    let mut writer = socket.try_clone().unwrap();
    let mut send = |line: &str| {
        let _ = writer.write_all(format!("{line}\r\n").as_bytes());
        let _ = writer.flush();
    };
    send(if opts.fail_greeting {
        "554 no service"
    } else {
        "220 mock ESMTP"
    });

    let mut reader = BufReader::new(socket);
    let mut line = String::new();
    let mut in_data = false;
    let mut chunks: Vec<String> = Vec::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => return,
            Ok(_) => {}
            Err(_) => return,
        }
        let cmd = line.trim_end_matches(['\r', '\n']).to_string();
        if in_data {
            if cmd == "." {
                in_data = false;
                *last_data.lock().unwrap() = Some(chunks.join("\r\n"));
                send("250 2.0.0 ok: queued");
                continue;
            }
            chunks.push(cmd);
            continue;
        }
        let up = cmd.to_uppercase();
        if up.starts_with("EHLO") {
            send("250-mock");
            if opts.advertise_starttls {
                send("250-STARTTLS");
            }
            send("250 AUTH PLAIN");
        } else if up.starts_with("AUTH PLAIN") {
            let token = cmd.get(11..).unwrap_or("").trim();
            let expected = {
                use base64::Engine;
                base64::engine::general_purpose::STANDARD.encode("\0u\0p")
            };
            if !opts.fail_auth && token == expected {
                send("235 2.7.0 ok");
            } else {
                send("535 5.7.8 authentication failed");
            }
        } else if up.starts_with("MAIL FROM") {
            send("250 2.1.0 ok");
        } else if up.starts_with("RCPT TO") {
            send(if opts.fail_rcpt {
                "550 5.1.1 no such user"
            } else {
                "250 2.1.5 ok"
            });
        } else if up.starts_with("DATA") {
            in_data = true;
            chunks.clear();
            send("354 go");
        } else if up.starts_with("STARTTLS") {
            starttls_seen.store(true, Ordering::SeqCst);
            send(if opts.reject_starttls {
                "454 4.7.0 TLS not available"
            } else {
                "220 go ahead"
            });
            if !opts.reject_starttls {
                // no real TLS in the mock: the handshake that follows fails,
                // which is what the JS mock does too
                return;
            }
        } else if up.starts_with("QUIT") {
            send("221 bye");
            return;
        } else {
            send("250 ok");
        }
    }
}

// ── sendMail ────────────────────────────────────────────────────────────────

#[test]
fn smtp_sendmail_happy_path_delivers_with_the_pdf_attachment() {
    let mock = Mock::start(MockOpts::default());
    let attachment = Attachment {
        filename: "2026-0001.pdf".into(),
        mime: "application/pdf".into(),
        data_base64: {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD.encode("%PDF-1.4 fake")
        },
    };
    let r = send_mail(
        &mock.cfg(true),
        "x@y.example",
        "Factuur 2026-0001 — Test Coaching",
        "Geachte,",
        Some(&attachment),
    )
    .unwrap();
    assert_eq!(r["accepted"], json!(true), "{r}");
    assert_eq!(r["tls"], json!(false), "the mock advertises no STARTTLS");

    let body = mock.body();
    assert!(
        body.contains("Subject: =?UTF-8?B?"),
        "a non-ASCII subject ships as an encoded-word: {body}"
    );
    assert!(body.contains("Content-Type: multipart/mixed"), "{body}");
    assert!(body.contains("application/pdf"), "{body}");
    assert!(body.contains("2026-0001.pdf"), "{body}");

    // the base64 part decodes back to the PDF bytes
    let b64: String = body
        .split("\r\n")
        .filter(|l| {
            l.len() >= 8
                && l.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "+/=".contains(c))
        })
        .collect();
    use base64::Engine;
    let decoded = String::from_utf8(
        base64::engine::general_purpose::STANDARD
            .decode(b64.trim())
            .unwrap(),
    )
    .unwrap();
    assert_eq!(decoded, "%PDF-1.4 fake");
}

#[test]
fn smtp_auth_failure_is_smtp_auth_failed() {
    let mock = Mock::start(MockOpts {
        fail_auth: true,
        ..Default::default()
    });
    let mut cfg = mock.cfg(true);
    cfg.pass = Some("wrong".into());
    let err = send_mail(&cfg, "x@y.z", "s", "t", None).unwrap_err();
    assert_eq!(err.code, "SMTP_AUTH_FAILED", "{err:?}");
    assert!(err.message.contains("535"), "{}", err.message);
}

#[test]
fn smtp_rcpt_rejection_is_smtp_send_failed_with_the_server_text() {
    let mock = Mock::start(MockOpts {
        fail_rcpt: true,
        ..Default::default()
    });
    let err = send_mail(&mock.cfg(true), "x@y.z", "s", "t", None).unwrap_err();
    assert_eq!(err.code, "SMTP_SEND_FAILED", "{err:?}");
    assert!(err.message.contains("550"), "{}", err.message);
}

#[test]
fn smtp_starttls_advertised_but_rejected_is_a_connect_failure() {
    let mock = Mock::start(MockOpts {
        advertise_starttls: true,
        reject_starttls: true,
        ..Default::default()
    });
    let err = send_mail(&mock.cfg(true), "x@y.z", "s", "t", None).unwrap_err();
    assert_eq!(err.code, "SMTP_CONNECT_FAILED", "{err:?}");
    assert!(err.message.contains("454"), "{}", err.message);
    assert!(
        mock.starttls_seen.load(Ordering::SeqCst),
        "STARTTLS was attempted"
    );
}

#[test]
fn smtp_connection_refused_and_bad_greeting_are_connect_failures() {
    // a port that is closed
    let probe = TcpListener::bind("127.0.0.1:0").unwrap();
    let dead_port = probe.local_addr().unwrap().port();
    drop(probe);

    let cfg = SmtpConfig {
        host: "127.0.0.1".into(),
        port: dead_port,
        secure: false,
        user: None,
        pass: None,
        from: "a@b.c".into(),
    };
    let err = send_mail(&cfg, "x@y.z", "s", "t", None).unwrap_err();
    assert_eq!(err.code, "SMTP_CONNECT_FAILED", "{err:?}");

    let mock = Mock::start(MockOpts {
        fail_greeting: true,
        ..Default::default()
    });
    let err = send_mail(&mock.cfg(false), "x@y.z", "s", "t", None).unwrap_err();
    assert_eq!(err.code, "SMTP_CONNECT_FAILED", "{err:?}");
}

#[test]
fn smtp_config_is_env_driven_and_port_defaults_follow_secure() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _g = EnvGuard::set(&[]);
    let err = smtp_validate(&smtp_config()).unwrap_err();
    assert_eq!(err.code, "SMTP_NOT_CONFIGURED", "{err:?}");

    let _g = EnvGuard::set(&[
        ("BUKIO_SMTP_HOST", "smtp.example"),
        ("BUKIO_SMTP_FROM", "a@b.c"),
        ("BUKIO_SMTP_PORT", "587"),
    ]);
    let cfg = smtp_config();
    assert_eq!(cfg.port, 587);
    assert_eq!(cfg.secure, false);

    let _g = EnvGuard::set(&[
        ("BUKIO_SMTP_HOST", "smtp.example"),
        ("BUKIO_SMTP_FROM", "a@b.c"),
        ("BUKIO_SMTP_PORT", "587"),
        ("BUKIO_SMTP_SECURE", "1"),
    ]);
    assert!(smtp_config().secure);
    assert_eq!(
        smtp_config().port,
        587,
        "an explicit port wins over the secure default"
    );

    let _g = EnvGuard::set(&[
        ("BUKIO_SMTP_HOST", "smtp.example"),
        ("BUKIO_SMTP_FROM", "a@b.c"),
        ("BUKIO_SMTP_SECURE", "1"),
    ]);
    assert_eq!(
        smtp_config().port,
        465,
        "secure without a port falls back to 465"
    );
}

#[test]
fn smtp_build_mime_encodes_non_ascii_subjects_and_keeps_ascii_readable() {
    let mime = build_mime(
        "a@b.c",
        "x@y.z",
        "Factuur met ééndag",
        "hallo",
        Some(&Attachment {
            filename: "a.pdf".into(),
            mime: "application/pdf".into(),
            data_base64: "YWJj".into(),
        }),
    );
    assert!(mime.contains("Subject: =?UTF-8?B?"), "{mime}");
    assert!(mime.contains("Content-Disposition: attachment"));
    assert!(mime.contains("--BUKIO-"), "{mime}");

    let plain = build_mime("a@b.c", "x@y.z", "Factuur 2026-0001", "hallo", None);
    assert!(plain.contains("Subject: Factuur 2026-0001"), "{plain}");
}

#[test]
fn smtp_build_mime_strips_crlf_so_nothing_can_inject_a_header() {
    let mime = build_mime(
        "a@b.c\r\nBcc: victim@evil.example",
        "x@y.z\r\nX-Evil: 1",
        "Factuur\r\nBcc: victim@evil.example",
        "hallo",
        Some(&Attachment {
            filename: "a.pdf\r\nX-Evil: 1".into(),
            mime: "application/pdf".into(),
            data_base64: "YQ==".into(),
        }),
    );
    // the injected text stays inside the header value, stripped and merged
    assert!(mime.contains("a@b.cBcc: victim@evil.example"), "{mime}");
    let headers: Vec<&str> = mime
        .split("\r\n")
        .filter(|l| !l.starts_with("--") && l.contains(':'))
        .collect();
    assert!(
        !headers.iter().any(|l| l.starts_with("Bcc:")),
        "no standalone Bcc: header may appear: {headers:?}"
    );
    assert!(
        !headers.iter().any(|l| l.starts_with("X-Evil:")),
        "no standalone X-Evil: header may appear: {headers:?}"
    );
}

#[test]
fn smtp_dot_stuffed_payload_survives() {
    let mock = Mock::start(MockOpts::default());
    send_mail(
        &mock.cfg(true),
        "x@y.z",
        "T",
        "Hallo\n.een aparte regel\nEinde",
        None,
    )
    .unwrap();
    assert!(
        mock.body().contains("..een aparte regel"),
        "a body line starting with '.' must be doubled in the DATA payload: {}",
        mock.body()
    );
}

// ── emailInvoice ────────────────────────────────────────────────────────────

/// A company + a contact with an email + a finalized invoice.
fn seed_invoice(file: &str, email: Option<&str>) -> i64 {
    let (_, ok, out) = run_cli(
        &[
            "--json",
            "init",
            "--name",
            "Test Coaching",
            "--registration-id",
            "12345678",
            "--legal-form",
            "eenmanszaak",
            "--vat",
            "off",
            "--db",
            file,
        ],
        &[],
    );
    assert!(ok, "init failed: {out}");
    run_cli(
        &[
            "--json",
            "company",
            "update",
            "--address",
            "Teststraat 1",
            "--postal-code",
            "1000 AA",
            "--city",
            "Amsterdam",
            "--db",
            file,
        ],
        &[],
    );
    let db = bukio::db::open_db(file).unwrap();
    let contact = bukio::contacts::create_contact(
        &db,
        "Acme BV",
        Some("Klantstraat 1"),
        None,
        Some("Amsterdam"),
        None,
        email,
        None,
        None,
        None,
        "agent:test",
        false,
    )
    .unwrap();
    let inv = bukio::invoice::create_invoice(
        &db,
        contact["id"].as_i64().unwrap(),
        "2026-08-10",
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        &[json!("Ding @ 100.00")],
        "agent:test",
        false,
    )
    .unwrap();
    let id = inv["id"].as_i64().unwrap();
    bukio::invoice::finalize_invoice(&db, id, "agent:test", false).unwrap();
    id
}

#[test]
fn smtp_email_invoice_delivers_and_audits() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = temp_dir("email");
    let file = dir.join("test.db");
    let f = file.to_str().unwrap().to_string();
    let id = seed_invoice(&f, Some("klant@acme.example"));

    let mock = Mock::start(MockOpts::default());
    let _g = EnvGuard::set(&[
        ("BUKIO_SMTP_HOST", "127.0.0.1"),
        ("BUKIO_SMTP_PORT", &mock.port.to_string()),
        ("BUKIO_SMTP_USER", "u"),
        ("BUKIO_SMTP_PASS", "p"),
        ("BUKIO_SMTP_FROM", "no-reply@test.example"),
    ]);
    let db = bukio::db::open_db(&f).unwrap();
    let r =
        bukio::smtp::email_invoice(&db, id, None, None, None, false, "agent:test", false).unwrap();
    assert_eq!(r["delivered"], json!(true), "{r}");
    assert_eq!(r["to"], json!("klant@acme.example"));
    assert_eq!(r["invoice_number"], json!("2026-0001"));

    let (n, actor, args): (i64, String, String) = db
        .query_row(
            "SELECT COUNT(*), MAX(actor), MAX(args_json) FROM audit_log WHERE action = 'invoice.email'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(n, 1);
    assert_eq!(actor, "agent:test");
    let args: Value = serde_json::from_str(&args).unwrap();
    assert_eq!(args["to"], json!("klant@acme.example"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn smtp_email_invoice_guards_draft_missing_email_and_config() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = temp_dir("guards");
    let file = dir.join("test.db");
    let f = file.to_str().unwrap().to_string();
    let _g = EnvGuard::set(&[]); // SMTP deliberately unconfigured

    let (_, ok, out) = run_cli(
        &[
            "--json",
            "init",
            "--name",
            "Test Coaching",
            "--registration-id",
            "12345678",
            "--legal-form",
            "eenmanszaak",
            "--vat",
            "off",
            "--db",
            &f,
        ],
        &[],
    );
    assert!(ok, "{out}");
    run_cli(
        &[
            "--json",
            "company",
            "update",
            "--address",
            "Teststraat 1",
            "--postal-code",
            "1000 AA",
            "--city",
            "Amsterdam",
            "--db",
            &f,
        ],
        &[],
    );
    let db = bukio::db::open_db(&f).unwrap();
    let contact = bukio::contacts::create_contact(
        &db,
        "Acme BV",
        Some("Klantstraat 1"),
        None,
        Some("Amsterdam"),
        None,
        None,
        None,
        None,
        None,
        "agent:test",
        false,
    )
    .unwrap();
    let draft = bukio::invoice::create_invoice(
        &db,
        contact["id"].as_i64().unwrap(),
        "2026-08-10",
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        &[json!("Ding @ 100.00")],
        "agent:test",
        false,
    )
    .unwrap();
    let draft_id = draft["id"].as_i64().unwrap();
    let err =
        bukio::smtp::email_invoice(&db, draft_id, None, None, None, false, "agent:test", false)
            .unwrap_err();
    assert_eq!(err.code, "NOT_FINALIZED", "{err:?}");

    bukio::invoice::finalize_invoice(&db, draft_id, "agent:test", false).unwrap();
    // no email on the contact and no --to
    let err =
        bukio::smtp::email_invoice(&db, draft_id, None, None, None, false, "agent:test", false)
            .unwrap_err();
    assert_eq!(err.code, "CONTACT_EMAIL_MISSING", "{err:?}");
    // --to override works, but SMTP is unconfigured
    let err = bukio::smtp::email_invoice(
        &db,
        draft_id,
        Some("x@y.z"),
        None,
        None,
        false,
        "agent:test",
        false,
    )
    .unwrap_err();
    assert_eq!(err.code, "SMTP_NOT_CONFIGURED", "{err:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn smtp_email_invoice_dry_run_makes_no_connection_and_audits_nothing() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = temp_dir("dry");
    let file = dir.join("test.db");
    let f = file.to_str().unwrap().to_string();
    let id = seed_invoice(&f, Some("klant@acme.example"));

    let mock = Mock::start(MockOpts::default());
    let _g = EnvGuard::set(&[
        ("BUKIO_SMTP_HOST", "127.0.0.1"),
        ("BUKIO_SMTP_PORT", &mock.port.to_string()),
        ("BUKIO_SMTP_FROM", "no-reply@test.example"),
    ]);
    let db = bukio::db::open_db(&f).unwrap();
    let plan =
        bukio::smtp::email_invoice(&db, id, None, None, None, false, "agent:test", true).unwrap();
    assert_eq!(plan["dryRun"], json!(true), "{plan}");
    assert_eq!(plan["to"], json!("klant@acme.example"));
    assert_eq!(
        mock.connections.load(Ordering::SeqCst),
        0,
        "dry-run must not open a connection"
    );
    let n: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM audit_log WHERE action = 'invoice.email'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn smtp_email_invoice_attaches_a_pdf_that_decodes_to_percent_pdf() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = temp_dir("pdf");
    let file = dir.join("test.db");
    let f = file.to_str().unwrap().to_string();
    let id = seed_invoice(&f, Some("klant@acme.example"));

    let mock = Mock::start(MockOpts::default());
    let _g = EnvGuard::set(&[
        ("BUKIO_SMTP_HOST", "127.0.0.1"),
        ("BUKIO_SMTP_PORT", &mock.port.to_string()),
        ("BUKIO_SMTP_USER", "u"),
        ("BUKIO_SMTP_PASS", "p"),
        ("BUKIO_SMTP_FROM", "no-reply@test.example"),
    ]);
    let db = bukio::db::open_db(&f).unwrap();
    let r =
        bukio::smtp::email_invoice(&db, id, None, None, None, true, "agent:test", false).unwrap();
    assert_eq!(r["delivered"], json!(true), "{r}");

    let body = mock.body();
    assert!(body.contains("application/pdf"), "{body}");
    assert!(
        body.contains("2026-0001.pdf"),
        "no invoice pdf name in {body}"
    );
    let b64: String = body
        .split("\r\n")
        .filter(|l| {
            l.len() >= 8
                && l.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "+/=".contains(c))
        })
        .collect();
    use base64::Engine;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(b64.trim())
        .unwrap();
    assert!(decoded.starts_with(b"%PDF"), "attachment must be a PDF");
    let _ = std::fs::remove_dir_all(&dir);
}

// ── CLI + MCP ───────────────────────────────────────────────────────────────

#[test]
fn smtp_cli_invoice_email_e2e_with_audit_row() {
    let dir = temp_dir("cli");
    let file = dir.join("test.db");
    let f = file.to_str().unwrap().to_string();
    let id = seed_invoice(&f, Some("klant@acme.example"));

    let mock = Mock::start(MockOpts::default());
    let port = mock.port.to_string();
    let env = [
        ("BUKIO_SMTP_HOST", "127.0.0.1"),
        ("BUKIO_SMTP_PORT", port.as_str()),
        ("BUKIO_SMTP_USER", "u"),
        ("BUKIO_SMTP_PASS", "p"),
        ("BUKIO_SMTP_FROM", "no-reply@test.example"),
    ];

    let (dry, ok, out) = run_cli(
        &[
            "--json",
            "invoice",
            "email",
            "--id",
            &id.to_string(),
            "--no-pdf",
            "--dry-run",
            "--db",
            &f,
        ],
        &env,
    );
    assert!(ok, "{out}");
    assert_eq!(dry["data"]["dryRun"], json!(true), "{dry}");

    let (sent, ok, out) = run_cli(
        &[
            "--json",
            "invoice",
            "email",
            "--id",
            &id.to_string(),
            "--no-pdf",
            "--db",
            &f,
        ],
        &env,
    );
    assert!(ok, "{out}");
    assert_eq!(sent["data"]["delivered"], json!(true), "{sent}");

    let db = bukio::db::open_db(&f).unwrap();
    let actor: String = db
        .query_row(
            "SELECT actor FROM audit_log WHERE action = 'invoice.email' ORDER BY id DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(actor, "agent:test");

    let (unconfigured, ok, _) = run_cli(
        &[
            "--json",
            "invoice",
            "email",
            "--id",
            &id.to_string(),
            "--no-pdf",
            "--db",
            &f,
        ],
        &[],
    );
    assert!(!ok);
    assert_eq!(
        unconfigured["error"]["code"],
        json!("SMTP_NOT_CONFIGURED"),
        "{unconfigured}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn smtp_mcp_invoice_email_dry_run_then_execute() {
    let dir = temp_dir("mcp");
    let file = dir.join("test.db");
    let f = file.to_str().unwrap().to_string();
    let id = seed_invoice(&f, Some("klant@acme.example"));

    let mock = Mock::start(MockOpts::default());
    let port = mock.port.to_string();
    let exe = env!("CARGO_BIN_EXE_bukio");
    let mut child = std::process::Command::new(exe)
        .args(["mcp", "--db", &f])
        .env("BUKIO_ACTOR", "agent:test")
        .env("BUKIO_SMTP_HOST", "127.0.0.1")
        .env("BUKIO_SMTP_PORT", &port)
        .env("BUKIO_SMTP_USER", "u")
        .env("BUKIO_SMTP_PASS", "p")
        .env("BUKIO_SMTP_FROM", "no-reply@test.example")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();

    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut reader = BufReader::new(stdout);
    let call = |stdin: &mut std::process::ChildStdin,
                    reader: &mut BufReader<std::process::ChildStdout>,
                    msg: Value|
     -> Value {
        let _ = writeln!(stdin, "{msg}");
        let _ = stdin.flush();
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                panic!("mcp closed");
            }
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(v) = serde_json::from_str::<Value>(&line) {
                return v;
            }
        }
    };

    call(
        &mut stdin,
        &mut reader,
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}),
    );

    let plan = call(
        &mut stdin,
        &mut reader,
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"invoice_email","arguments":{"id":id,"attach_pdf":false}}}),
    );
    let plan_data: Value =
        serde_json::from_str(plan["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(plan_data["mode"], json!("dry-run"), "{plan_data}");
    assert_eq!(
        mock.connections.load(Ordering::SeqCst),
        0,
        "the MCP dry-run must not connect"
    );

    let exec = call(
        &mut stdin,
        &mut reader,
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"invoice_email","arguments":{"id":id,"attach_pdf":false,"mode":"execute"}}}),
    );
    let exec_data: Value =
        serde_json::from_str(exec["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(exec_data["mode"], json!("execute"), "{exec_data}");
    assert_eq!(exec_data["delivered"], json!(true), "{exec_data}");
    assert_eq!(mock.connections.load(Ordering::SeqCst), 1);

    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&dir);
}
