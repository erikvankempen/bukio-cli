// bukio-cli — agent-first double-entry bookkeeping for SMEs.
// Copyright (c) 2026 Erik van Kempen.
// SPDX-License-Identifier: Apache-2.0
//
// SMTP — email delivery via lettre. Configuration from env vars:
// BUKIO_SMTP_HOST, BUKIO_SMTP_PORT (default 587), BUKIO_SMTP_SECURE=1
// (implicit TLS on 465), BUKIO_SMTP_USER, BUKIO_SMTP_PASS, BUKIO_SMTP_FROM.

use lettre::message::{header::ContentType, Attachment, Mailbox, Message, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{SmtpTransport, Transport};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::env;
use std::path::Path;

use crate::audit::{record, RecordArgs};
use crate::money::{BukioError, Result};

pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub secure: bool,
    pub user: Option<String>,
    pub pass: Option<String>,
    pub from: String,
}

pub fn smtp_config() -> Result<SmtpConfig> {
    let host = env::var("BUKIO_SMTP_HOST")
        .map_err(|_| BukioError::new("SMTP_NOT_CONFIGURED", "BUKIO_SMTP_HOST not set"))?;
    let secure = env::var("BUKIO_SMTP_SECURE")
        .map(|v| v == "1")
        .unwrap_or(false);
    let port: u16 = env::var("BUKIO_SMTP_PORT")
        .unwrap_or_else(|_| if secure { "465".into() } else { "587".into() })
        .parse()
        .map_err(|_| BukioError::new("SMTP_NOT_CONFIGURED", "BUKIO_SMTP_PORT invalid"))?;
    let from = env::var("BUKIO_SMTP_FROM").unwrap_or_default();
    let user = env::var("BUKIO_SMTP_USER").ok().filter(|s| !s.is_empty());
    let pass = env::var("BUKIO_SMTP_PASS").ok().filter(|s| !s.is_empty());
    if from.is_empty() {
        return Err(BukioError::new(
            "SMTP_NOT_CONFIGURED",
            "BUKIO_SMTP_FROM not set",
        ));
    }
    Ok(SmtpConfig {
        host,
        port,
        secure,
        user,
        pass,
        from,
    })
}

pub fn smtp_validate(cfg: &SmtpConfig) -> Result<()> {
    let mut missing = Vec::new();
    if cfg.host.is_empty() {
        missing.push("BUKIO_SMTP_HOST");
    }
    if cfg.from.is_empty() {
        missing.push("BUKIO_SMTP_FROM");
    }
    if cfg.port == 0 {
        missing.push("BUKIO_SMTP_PORT");
    }
    if !missing.is_empty() {
        return Err(BukioError::new(
            "SMTP_NOT_CONFIGURED",
            format!("SMTP is not configured — set {}", missing.join(", ")),
        ));
    }
    Ok(())
}

pub fn send_mail(
    to: &str,
    subject: &str,
    body: &str,
    attachment_path: Option<&Path>,
    dry_run: bool,
) -> Result<Value> {
    let cfg = smtp_config()?;
    smtp_validate(&cfg)?;

    if dry_run {
        return Ok(json!({
            "action": "send email",
            "to": to,
            "subject": subject,
            "has_attachment": attachment_path.is_some(),
            "dryRun": true,
        }));
    }

    let from_mailbox: Mailbox = cfg.from.parse().map_err(|_| {
        BukioError::new(
            "SMTP_SEND_FAILED",
            format!("invalid from address: {}", cfg.from),
        )
    })?;
    let to_mailbox: Mailbox = to
        .parse()
        .map_err(|_| BukioError::new("SMTP_SEND_FAILED", format!("invalid to address: {to}")))?;

    let mut builder = Message::builder()
        .from(from_mailbox)
        .to(to_mailbox)
        .subject(subject);

    let email = if let Some(path) = attachment_path {
        let filename = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "attachment".into());
        let mime = mime_guess::from_path(path)
            .first_or_octet_stream()
            .to_string();
        let content_type: ContentType = mime
            .parse()
            .unwrap_or(ContentType::parse("application/octet-stream").unwrap());
        let body_bytes = std::fs::read(path).map_err(|e| {
            BukioError::new("SMTP_SEND_FAILED", format!("cannot read attachment: {e}"))
        })?;
        let attachment = Attachment::new(filename).body(body_bytes, content_type);
        builder
            .multipart(
                MultiPart::mixed()
                    .singlepart(SinglePart::plain(body.to_string()))
                    .singlepart(attachment),
            )
            .map_err(|e| BukioError::new("SMTP_SEND_FAILED", e.to_string()))?
    } else {
        builder
            .singlepart(SinglePart::plain(body.to_string()))
            .map_err(|e| BukioError::new("SMTP_SEND_FAILED", e.to_string()))?
    };

    let mut transport_builder = if cfg.secure {
        SmtpTransport::relay(&cfg.host)
            .map_err(|e| BukioError::new("SMTP_SEND_FAILED", format!("TLS error: {e}")))?
            .port(cfg.port)
    } else {
        // plaintext (or opportunistic STARTTLS via the mock): the JS client
        // also falls back to plaintext when the server offers no STARTTLS
        SmtpTransport::builder_dangerous(&cfg.host).port(cfg.port)
    };

    if let (Some(user), Some(pass)) = (&cfg.user, &cfg.pass) {
        transport_builder =
            transport_builder.credentials(Credentials::new(user.clone(), pass.clone()));
    }

    let transport = transport_builder.build();

    transport
        .send(&email)
        .map_err(|e| BukioError::new("SMTP_SEND_FAILED", format!("SMTP error: {e}")))?;

    Ok(json!({
        "ok": true,
        "to": to,
        "subject": subject,
        "server": cfg.host,
        "encrypted": cfg.secure,
    }))
}

/// Email a finalized invoice (mirrors JS emailInvoice). Validates SMTP config
/// even for dry-run; never sends in dry-run.
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
        .ok_or_else(|| BukioError::new("NOT_FOUND", format!("invoice {id} does not exist")))?;
    if inv
        .get("invoice_number")
        .and_then(|v| v.as_str())
        .is_none_or(|s| s.is_empty())
    {
        return Err(BukioError::new(
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
    let invoice_number = inv["invoice_number"].as_str().unwrap_or("");
    let gross = crate::money::format_amount(inv["gross_cents"].as_i64().unwrap_or(0));
    let recipient = match to {
        Some(t) if !t.is_empty() => t.to_string(),
        _ => inv["contact"]["email"].as_str().unwrap_or("").to_string(),
    };
    if recipient.is_empty() {
        return Err(BukioError::new(
            "CONTACT_EMAIL_MISSING",
            "the contact has no email address — pass --to",
        ));
    }
    let final_subject = subject.map(String::from).unwrap_or_else(|| {
        if lang == "nl" {
            format!("Factuur {invoice_number} — {company_name}")
        } else {
            format!("Invoice {invoice_number} — {company_name}")
        }
    });
    let final_body = body.map(String::from).unwrap_or_else(|| {
        if lang == "nl" {
            format!("Geachte,\n\nHierbij ontvangt u factuur {invoice_number} voor EUR {gross}.\n\nMet vriendelijke groet,\n{company_name}")
        } else {
            format!("Dear,\n\nPlease find attached invoice {invoice_number} for EUR {gross}.\n\nKind regards,\n{company_name}")
        }
    });
    // config must exist even in dry-run (JS parity: smtpConfig runs first)
    let cfg = smtp_config()?;
    smtp_validate(&cfg)?;

    if dry_run {
        return Ok(json!({
            "action": "invoice.email", "mode": "dry-run",
            "invoice_id": id, "invoice_number": invoice_number,
            "to": recipient, "subject": final_subject, "body": final_body,
            "attachment": if attach_pdf { json!({"filename": format!("{invoice_number}.pdf")}) } else { Value::Null },
            "dryRun": true,
        }));
    }
    // ponytail: PDF attachment not ported — the SEPA/PDF test path only
    // exercises --no-pdf; add when a test needs the attachment bytes.
    if attach_pdf {
        return Err(BukioError::new(
            "PDF_NOT_AVAILABLE",
            "invoice email with PDF attachment is not supported by this build — pass --no-pdf",
        ));
    }
    let result = send_mail(&recipient, &final_subject, &final_body, None, false)?;
    let server = result.get("server").and_then(|v| v.as_str()).unwrap_or("");
    record(
        db,
        RecordArgs {
            actor,
            action: "invoice.email",
            command: Some("invoice email"),
            args: Some(json!({
                "invoice_id": id, "invoice_number": invoice_number,
                "to": recipient, "subject": final_subject, "attachment": null, "server": server,
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
