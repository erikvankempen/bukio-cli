// bukio-cli — agent-first double-entry bookkeeping for SMEs.
// Copyright (c) 2026 Erik van Kempen.
// SPDX-License-Identifier: Apache-2.0
//
// SMTP — email delivery. The wire protocol, TLS and response parsing come from
// lettre's SmtpConnection (lettre's own transport hides the per-command reply
// codes the JS error taxonomy needs: SMTP_CONNECT_FAILED / SMTP_AUTH_FAILED /
// SMTP_SEND_FAILED each carry the server's reply code). The MIME builder is
// ours, mirroring src/core/smtp.js byte for byte (encoded-word subjects, the
// 76-column base64 wrap, and the CR/LF header sanitiser).
//
// Configuration from env vars: BUKIO_SMTP_HOST, BUKIO_SMTP_PORT (default 587,
// 465 when secure), BUKIO_SMTP_SECURE=1, BUKIO_SMTP_USER, BUKIO_SMTP_PASS,
// BUKIO_SMTP_FROM.

use base64::Engine;
use rusqlite::Connection;
use serde_json::{json, Value};
use std::env;
use std::time::Duration;

use lettre::transport::smtp::authentication::{Credentials, Mechanism};
use lettre::transport::smtp::client::{SmtpConnection, TlsParameters};
use lettre::transport::smtp::extension::ClientId;
use lettre::transport::smtp::Error as SmtpError;

use crate::audit::{record, RecordArgs};
use crate::money::{BukioError, Result};

const SMTP_TIMEOUT: Duration = Duration::from_secs(10);

pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub secure: bool,
    pub user: Option<String>,
    pub pass: Option<String>,
    pub from: String,
}

/// An attachment ready for MIME encoding (the JS passes dataBase64 around).
pub struct Attachment {
    pub filename: String,
    pub mime: String,
    pub data_base64: String,
}

fn smtp_error(code: &'static str, message: impl Into<String>) -> BukioError {
    BukioError::new(code, message.into())
}

/// The server's reply code, when lettre saw one: the JS messages are
/// "SMTP <code>: <text>" and tests assert on the code.
fn describe(e: &SmtpError) -> String {
    match e.status() {
        Some(code) => format!("SMTP {code}: {e}"),
        None => format!("SMTP error: {e}"),
    }
}

fn fail_connect(e: &SmtpError) -> BukioError {
    smtp_error("SMTP_CONNECT_FAILED", describe(e))
}
fn fail_send(e: &SmtpError) -> BukioError {
    smtp_error("SMTP_SEND_FAILED", describe(e))
}
fn fail_auth(e: &SmtpError) -> BukioError {
    smtp_error("SMTP_AUTH_FAILED", describe(e))
}

/// Read the configuration from the environment. Never throws on a missing
/// host/from — `smtp_validate` owns that (the JS split is the same).
pub fn smtp_config() -> SmtpConfig {
    let secure = env::var("BUKIO_SMTP_SECURE")
        .map(|v| v == "1")
        .unwrap_or(false);
    let port = env::var("BUKIO_SMTP_PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(if secure { 465 } else { 587 });
    SmtpConfig {
        host: env::var("BUKIO_SMTP_HOST").unwrap_or_default(),
        port,
        secure,
        user: env::var("BUKIO_SMTP_USER").ok().filter(|s| !s.is_empty()),
        pass: env::var("BUKIO_SMTP_PASS").ok().filter(|s| !s.is_empty()),
        from: env::var("BUKIO_SMTP_FROM").unwrap_or_default(),
    }
}

/// Validate the config (throws SMTP_NOT_CONFIGURED); also used by dry-run.
pub fn smtp_validate(cfg: &SmtpConfig) -> Result<()> {
    let mut missing = Vec::new();
    if cfg.host.is_empty() {
        missing.push("BUKIO_SMTP_HOST".to_string());
    }
    if cfg.from.is_empty() {
        missing.push("BUKIO_SMTP_FROM".to_string());
    }
    if cfg.port == 0 {
        missing.push(format!("BUKIO_SMTP_PORT ('{}')", cfg.port));
    }
    if !missing.is_empty() {
        return Err(smtp_error(
            "SMTP_NOT_CONFIGURED",
            format!("SMTP is not configured — set {}", missing.join(", ")),
        ));
    }
    Ok(())
}

/// UTF-8 base64 encoded-word for non-ASCII headers (Dutch accents).
fn encode_word(value: &str) -> String {
    let ascii = value.chars().all(|c| ('\x20'..='\x7e').contains(&c));
    if ascii && value.len() <= 60 {
        value.to_string()
    } else {
        format!(
            "=?UTF-8?B?{}?=",
            base64::engine::general_purpose::STANDARD.encode(value)
        )
    }
}

/// Base64 wrapped at 76 chars for the MIME body.
fn wrap_base64(b64: &str) -> String {
    b64.as_bytes()
        .chunks(76)
        .map(|c| String::from_utf8_lossy(c).to_string())
        .collect::<Vec<_>>()
        .join("\r\n")
}

/// CR/LF sanitisation — a header or SMTP envelope must never carry a line break
/// (header/command injection). The JS strips the break and keeps the text, so
/// "a@b.c\r\nBcc: victim" becomes the single value "a@b.cBcc: victim".
fn clean_header(value: &str) -> String {
    value.replace(['\r', '\n'], "").trim().to_string()
}

/// Build a multipart/mixed MIME message: text part + optional attachment.
pub fn build_mime(
    from: &str,
    to: &str,
    subject: &str,
    text: &str,
    attachment: Option<&Attachment>,
) -> String {
    let boundary = {
        use rand::RngCore;
        let mut b = [0u8; 12];
        rand::thread_rng().fill_bytes(&mut b);
        format!("BUKIO-{}", hex::encode(b))
    };
    let mut lines: Vec<String> = vec![
        format!("From: {}", encode_word(&clean_header(from))),
        format!("To: {}", encode_word(&clean_header(to))),
        format!("Subject: {}", encode_word(&clean_header(subject))),
        "MIME-Version: 1.0".into(),
        format!("Content-Type: multipart/mixed; boundary=\"{boundary}\""),
        String::new(),
        format!("--{boundary}"),
        "Content-Type: text/plain; charset=utf-8".into(),
        "Content-Transfer-Encoding: 8bit".into(),
        String::new(),
        text.to_string(),
        String::new(),
    ];
    if let Some(a) = attachment {
        let name = encode_word(&a.filename);
        lines.push(format!("--{boundary}"));
        lines.push(format!("Content-Type: {}; name=\"{name}\"", a.mime));
        lines.push(format!(
            "Content-Disposition: attachment; filename=\"{name}\""
        ));
        lines.push("Content-Transfer-Encoding: base64".into());
        lines.push(String::new());
        lines.push(wrap_base64(&a.data_base64));
        lines.push(String::new());
    }
    lines.push(format!("--{boundary}--"));
    lines.push(String::new());
    // the text part may carry bare LF, but DATA is a CRLF protocol and lettre's
    // codec only dot-stuffs CRLF lines — normalise so a body line starting with
    // '.' is doubled on the wire either way (the JS stuffs bare-LF lines via
    // /^[.]/gm; same result)
    lines
        .join("\r\n")
        .replace("\r\n", "\n")
        .replace('\n', "\r\n")
}

/// Send one message. Errors: SMTP_NOT_CONFIGURED / SMTP_CONNECT_FAILED /
/// SMTP_AUTH_FAILED / SMTP_SEND_FAILED (each carrying the server's reply code).
pub fn send_mail(
    cfg: &SmtpConfig,
    to: &str,
    subject: &str,
    text: &str,
    attachment: Option<&Attachment>,
) -> Result<Value> {
    smtp_validate(cfg)?;
    if to.is_empty() {
        return Err(smtp_error("SMTP_SEND_FAILED", "no recipient"));
    }
    // envelope + headers must be single-line (CR/LF stripped) — header/command
    // injection guard; the MIME builder sanitises again for defense in depth
    let env_from = clean_header(&cfg.from);
    let env_to = clean_header(to);

    let tls_params = TlsParameters::new(cfg.host.clone()).map_err(|e| {
        smtp_error(
            "SMTP_CONNECT_FAILED",
            format!("cannot connect to {}:{} — {e}", cfg.host, cfg.port),
        )
    })?;

    // implicit TLS on 465 (secure), plaintext otherwise — STARTTLS is opted into
    // below when the server advertises it
    let mut conn = SmtpConnection::connect(
        (cfg.host.as_str(), cfg.port),
        Some(SMTP_TIMEOUT),
        &ClientId::Domain("bukio".to_string()),
        if cfg.secure { Some(&tls_params) } else { None },
        None,
    )
    .map_err(|e| fail_connect(&e))?;

    if !cfg.secure && conn.can_starttls() {
        // a server that advertises STARTTLS and then refuses it is a connect
        // failure, not a send failure (the JS fails here too)
        conn.starttls(&tls_params, &ClientId::Domain("bukio".to_string()))
            .map_err(|e| fail_connect(&e))?;
    }

    if let Some(user) = &cfg.user {
        let creds = Credentials::new(user.clone(), cfg.pass.clone().unwrap_or_default());
        conn.auth(&[Mechanism::Plain], &creds)
            .map_err(|e| fail_auth(&e))?;
    }

    conn.command(format!("MAIL FROM:<{env_from}>\r\n").as_str())
        .map_err(|e| fail_send(&e))?;
    conn.command(format!("RCPT TO:<{env_to}>\r\n").as_str())
        .map_err(|e| fail_send(&e))?;

    conn.command("DATA\r\n").map_err(|e| fail_send(&e))?; // 354 is a positive intermediate
                                                          // message() dot-stuffs the payload (a body line starting with . is
                                                          // doubled, RFC 5321 4.5.2), appends the terminator and reads the reply
    let payload = build_mime(&env_from, &env_to, subject, text, attachment);
    conn.message(payload.as_bytes())
        .map_err(|e| fail_send(&e))?;

    let tls_used = conn.is_encrypted();
    let _ = conn.quit();

    Ok(json!({
        "accepted": true,
        "server": format!("{}:{}", cfg.host, cfg.port),
        "tls": tls_used,
    }))
}

/// Email a finalized invoice. Validates SMTP config even for dry-run; never
/// sends in dry-run.
pub fn email_invoice(
    db: &Connection,
    id: i64,
    to: Option<&str>,
    subject: Option<&str>,
    body: Option<&str>,
    attach_pdf: bool,
    actor: &str,
    dry_run: bool,
) -> Result<Value> {
    let inv = crate::invoice::get_invoice(db, id)?
        .ok_or_else(|| smtp_error("NOT_FOUND", format!("invoice {id} does not exist")))?;
    if inv
        .get("invoice_number")
        .and_then(|v| v.as_str())
        .is_none_or(|s| s.is_empty())
    {
        return Err(smtp_error(
            "NOT_FINALIZED",
            "finalize the invoice before emailing it",
        ));
    }
    let company_name = db
        .query_row("SELECT name FROM company WHERE id = 1", [], |r| {
            r.get::<_, String>(0)
        })
        .unwrap_or_else(|_| "Bukio".into());
    let lang = inv.get("language").and_then(|v| v.as_str()).unwrap_or("en");
    let invoice_number = inv["invoice_number"].as_str().unwrap_or("").to_string();
    let gross = crate::money::format_amount(inv["gross_cents"].as_i64().unwrap_or(0));
    let recipient = match to {
        Some(t) if !t.is_empty() => t.to_string(),
        _ => inv["contact"]["email"].as_str().unwrap_or("").to_string(),
    };
    if recipient.is_empty() {
        return Err(smtp_error(
            "CONTACT_EMAIL_MISSING",
            "the contact has no email address — pass --to",
        ));
    }
    let final_subject = match subject {
        Some(s) => s.to_string(),
        None => crate::pdf::default_subject(lang, &invoice_number, &company_name),
    };
    let final_body = body.map(String::from).unwrap_or_else(|| {
        if lang == "nl" {
            format!("Geachte,\n\nHierbij ontvangt u factuur {invoice_number} voor EUR {gross}.\n\nMet vriendelijke groet,\n{company_name}")
        } else {
            format!("Dear,\n\nPlease find attached invoice {invoice_number} for EUR {gross}.\n\nKind regards,\n{company_name}")
        }
    });
    // config must exist even in dry-run (JS parity: smtpConfig runs first)
    let cfg = smtp_config();
    smtp_validate(&cfg)?;

    let pdf_name = format!("{invoice_number}.pdf");
    if dry_run {
        return Ok(json!({
            "action": "invoice.email", "mode": "dry-run",
            "invoice_id": id, "invoice_number": invoice_number,
            "to": recipient, "subject": final_subject, "body": final_body,
            "attachment": if attach_pdf { json!({"filename": pdf_name}) } else { Value::Null },
            "dryRun": true,
        }));
    }

    // the attachment is rendered from the invoice itself (native PDF writer)
    let attachment = if attach_pdf {
        let rendered = crate::pdf::invoice_to_pdf(db, &inv, None)?;
        Some(Attachment {
            filename: pdf_name.clone(),
            mime: "application/pdf".into(),
            data_base64: rendered["data"].as_str().unwrap_or_default().to_string(),
        })
    } else {
        None
    };

    let result = send_mail(
        &cfg,
        &recipient,
        &final_subject,
        &final_body,
        attachment.as_ref(),
    )?;
    let server = result["server"].as_str().unwrap_or("").to_string();
    record(
        db,
        RecordArgs {
            actor,
            action: "invoice.email",
            command: Some("invoice email"),
            args: Some(json!({
                "invoice_id": id, "invoice_number": invoice_number,
                "to": recipient, "subject": final_subject,
                "attachment": if attach_pdf { json!(pdf_name) } else { Value::Null },
                "server": server,
            })),
            outcome: "ok",
            entry_ids: vec![],
        },
    )?;
    Ok(json!({
        "action": "invoice.email", "mode": "execute",
        "id": id, "invoice_number": invoice_number, "to": recipient,
        "subject": final_subject, "delivered": true, "server": server,
    }))
}
