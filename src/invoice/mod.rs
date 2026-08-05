//! Invoice core — contacts, line parsing/validation, create (draft) and the
//! payment functions the bank auto-match engine depends on. The full invoice
//! module (finalize/credit/UBL/PDF/CLI) is ported in the invoice milestone.

use crate::core::entries::{create_entry, post_entry};
use crate::error::{AppError, Result};
use crate::vat::{is_vat_enabled, list_vat_codes};
use regex::Regex;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub fn invoice_error(code: &'static str, message: impl Into<String>) -> AppError {
    AppError::new(code, message)
}

/// A contact (customer) row.
#[derive(Debug, Clone)]
pub struct Contact {
    pub id: i64,
    pub name: String,
    pub address: Option<String>,
    pub postal_code: Option<String>,
    pub city: Option<String>,
    pub country: String,
    pub email: Option<String>,
    pub vat_id: Option<String>,
    pub kvk: Option<String>,
    pub created_by: String,
    pub created_at: String,
}

pub fn get_contact(conn: &Connection, id: i64) -> Result<Option<Contact>> {
    let row: Option<(i64, String, Option<String>, Option<String>, Option<String>, String, Option<String>, Option<String>, Option<String>, String, String)> = conn
        .query_row(
            "SELECT id, name, address, postal_code, city, country, email, vat_id, kvk, created_by, created_at FROM contacts WHERE id = ?1",
            [id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                    r.get(8)?,
                    r.get(9)?,
                    r.get(10)?,
                ))
            },
        )
        .optional()?;
    Ok(row.map(
        |(id, name, address, postal_code, city, country, email, vat_id, kvk, created_by, created_at)| {
            Contact {
                id,
                name,
                address,
                postal_code,
                city,
                country,
                email,
                vat_id,
                kvk,
                created_by,
                created_at,
            }
        },
    ))
}

/// A line of an invoice.
#[derive(Debug, Clone)]
pub struct InvoiceLine {
    pub id: i64,
    pub invoice_id: i64,
    pub line_no: i64,
    pub description: String,
    pub quantity: i64,
    pub unit_price_cents: i64,
    pub vat_code: Option<String>,
    pub vat_rate_bp: i64,
    pub amount_cents: i64,
    pub vat_amount_cents: i64,
}

/// A recorded payment (tracking only — the posting comes from the bank flow).
#[derive(Debug, Clone, Serialize)]
pub struct InvoicePayment {
    pub id: i64,
    pub invoice_id: i64,
    pub date: String,
    pub amount_cents: i64,
    pub method: String,
    pub bank_tx_id: Option<i64>,
    pub created_by: String,
    pub created_at: String,
}

/// A full invoice with derived amounts (net/vat/gross/paid) and contact.
#[derive(Debug, Clone)]
pub struct Invoice {
    pub id: i64,
    pub invoice_number: Option<String>,
    pub invoice_type: String,
    pub contact_id: i64,
    pub date: String,
    pub due_date: Option<String>,
    pub delivery_date: Option<String>,
    pub description: Option<String>,
    pub reference: Option<String>,
    pub status: String,
    pub credit_for_invoice_id: Option<i64>,
    pub entry_id: Option<i64>,
    pub currency: String,
    pub notes: Option<String>,
    pub created_by: String,
    pub net_cents: i64,
    pub vat_cents: i64,
    pub gross_cents: i64,
    pub paid_cents: i64,
    pub contact: Option<Contact>,
    pub lines: Vec<InvoiceLine>,
    pub payments: Vec<InvoicePayment>,
    pub created_at: String,
}

pub fn get_invoice(conn: &Connection, id: i64) -> Result<Option<Invoice>> {
    let row: Option<(i64, Option<String>, String, i64, String, Option<String>, Option<String>, Option<String>, Option<String>, String, Option<i64>, Option<i64>, String, Option<String>, String, String)> = conn
        .query_row(
            "SELECT id, invoice_number, invoice_type, contact_id, date, due_date, delivery_date,
                    description, reference, status, credit_for_invoice_id, entry_id, currency, notes, created_by, created_at
             FROM invoices WHERE id = ?1",
            [id],
            |r| {
                Ok((
                    r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?,
                    r.get(7)?, r.get(8)?, r.get(9)?, r.get(10)?, r.get(11)?, r.get(12)?,
                    r.get(13)?, r.get(14)?, r.get(15)?,
                ))
            },
        )
        .optional()?;
    let Some(row) = row else { return Ok(None) };

    let net_cents: i64 =
        conn.query_row("SELECT COALESCE(SUM(amount_cents),0) FROM invoice_lines WHERE invoice_id = ?1", [id], |r| r.get(0))?;
    let vat_cents: i64 =
        conn.query_row("SELECT COALESCE(SUM(vat_amount_cents),0) FROM invoice_lines WHERE invoice_id = ?1", [id], |r| r.get(0))?;
    let paid_cents: i64 =
        conn.query_row("SELECT COALESCE(SUM(amount_cents),0) FROM invoice_payments WHERE invoice_id = ?1", [id], |r| r.get(0))?;

    // derived status: sent + past due_date -> overdue
    let today = crate::core::entries::now_iso();
    let mut status = row.9.clone();
    if status == "sent" {
        if let Some(due) = &row.5 {
            if due.as_str() < today.as_str() {
                status = "overdue".to_string();
            }
        }
    }

    let contact = get_contact(conn, row.3)?;

    let lines: Vec<InvoiceLine> = {
        let mut stmt = conn.prepare(
            "SELECT id, invoice_id, line_no, description, quantity, unit_price_cents, vat_code, vat_rate_bp, amount_cents, vat_amount_cents
             FROM invoice_lines WHERE invoice_id = ?1 ORDER BY line_no",
        )?;
        let rows = stmt.query_map([id], |r| {
            Ok(InvoiceLine {
                id: r.get(0)?,
                invoice_id: r.get(1)?,
                line_no: r.get(2)?,
                description: r.get(3)?,
                quantity: r.get(4)?,
                unit_price_cents: r.get(5)?,
                vat_code: r.get(6)?,
                vat_rate_bp: r.get(7)?,
                amount_cents: r.get(8)?,
                vat_amount_cents: r.get(9)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        out
    };

    let payments: Vec<InvoicePayment> = {
        let mut stmt = conn.prepare(
            "SELECT id, invoice_id, date, amount_cents, method, bank_tx_id, created_by, created_at FROM invoice_payments WHERE invoice_id = ?1 ORDER BY date, id",
        )?;
        let rows = stmt.query_map([id], |r| {
            Ok(InvoicePayment {
                id: r.get(0)?,
                invoice_id: r.get(1)?,
                date: r.get(2)?,
                amount_cents: r.get(3)?,
                method: r.get(4)?,
                bank_tx_id: r.get(5)?,
                created_by: r.get(6)?,
                created_at: r.get(7)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        out
    };

    Ok(Some(Invoice {
        id: row.0,
        invoice_number: row.1,
        invoice_type: row.2,
        contact_id: row.3,
        date: row.4,
        due_date: row.5,
        delivery_date: row.6,
        description: row.7,
        reference: row.8,
        status,
        credit_for_invoice_id: row.10,
        entry_id: row.11,
        currency: row.12,
        notes: row.13,
        created_by: row.14,
        created_at: row.15,
        net_cents,
        vat_cents,
        gross_cents: net_cents + vat_cents,
        lines,
        payments,
        paid_cents,
        contact,
    }))
}

/// Record a payment (tracking only — the posting comes from the bank flow).
/// When fully paid, the invoice status becomes 'paid'.
pub fn mark_paid(
    conn: &Connection,
    id: i64,
    date: &str,
    amount_cents: i64,
    method: &str,
    bank_tx_id: Option<i64>,
    actor: &str,
) -> Result<Invoice> {
    let invoice = get_invoice(conn, id)?.ok_or_else(|| invoice_error("NOT_FOUND", format!("invoice {id} does not exist")))?;
    if invoice.invoice_type == "credit" {
        return Err(invoice_error("CREDIT_NOT_PAYABLE", "credit notes are not payable"));
    }
    if !["sent", "overdue"].contains(&invoice.status.as_str()) {
        return Err(invoice_error("NOT_PAYABLE", format!("invoice {id} is {}", invoice.status)));
    }
    if amount_cents <= 0 {
        return Err(invoice_error("INVALID_AMOUNT", "payment amount must be positive cents"));
    }
    let remaining = invoice.gross_cents - invoice.paid_cents;
    if amount_cents > remaining {
        return Err(invoice_error("OVERPAYMENT", format!("payment {amount_cents} exceeds the outstanding {remaining}")));
    }
    conn.execute(
        "INSERT INTO invoice_payments (invoice_id, date, amount_cents, method, bank_tx_id, created_by) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![id, date, amount_cents, method, bank_tx_id, actor],
    )?;
    let paid_now = invoice.paid_cents + amount_cents;
    if paid_now >= invoice.gross_cents {
        conn.execute("UPDATE invoices SET status = 'paid' WHERE id = ?1", [id])?;
    }
    crate::audit::record(
        conn,
        actor,
        "invoice.pay",
        Some("invoice pay"),
        Some(&serde_json::json!({ "id": id, "amountCents": amount_cents })),
        "ok",
        &[],
    )?;
    get_invoice(conn, id)?.ok_or_else(|| invoice_error("NOT_FOUND", format!("invoice {id} does not exist")))
}

/// Apply a bank payment to an invoice: record the payment, post the bank
/// entry (Bank / Debiteuren) and reconcile the transaction. Used by the bank
/// auto-match engine and by `invoice pay --from-bank`.
pub fn payment_from_bank(conn: &Connection, invoice_id: i64, bank_tx_id: i64, actor: &str) -> Result<Invoice> {
    let invoice = get_invoice(conn, invoice_id)?
        .ok_or_else(|| invoice_error("NOT_FOUND", format!("invoice {invoice_id} does not exist")))?;
    let tx_row: Option<(i64, i64, String, i64, String)> = conn
        .query_row(
            "SELECT id, bank_account_id, date, amount_cents, state FROM bank_transactions WHERE id = ?1",
            [bank_tx_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()?;
    let Some((_, bank_account_id, tx_date, tx_amount, tx_state)) = tx_row else {
        return Err(invoice_error("NOT_FOUND", format!("bank transaction {bank_tx_id} does not exist")));
    };
    if tx_state != "unmatched" {
        return Err(invoice_error("ALREADY_MATCHED", format!("bank transaction {bank_tx_id} is already {tx_state}")));
    }
    let bank_account_code: String = conn.query_row(
        "SELECT account_code FROM bank_accounts WHERE id = ?1",
        [bank_account_id],
        |r| r.get(0),
    )?;

    let paid = mark_paid(conn, invoice_id, &tx_date, tx_amount, "bank", Some(bank_tx_id), actor)?;
    let contact_suffix = invoice
        .contact
        .as_ref()
        .map(|c| format!(" - {}", c.name))
        .unwrap_or_default();
    let description = format!(
        "Betaling {}{}",
        invoice.invoice_number.as_deref().unwrap_or(&invoice_id.to_string()),
        contact_suffix
    );
    let postings = vec![
        crate::core::entries::PostingSpec {
            code: bank_account_code.clone(),
            amount_cents: tx_amount,
            vat_code: None,
            vat_amount_cents: None,
            fx_currency: None,
            fx_amount_cents: None,
        },
        crate::core::entries::PostingSpec {
            code: "1200".to_string(),
            amount_cents: -tx_amount,
            vat_code: None,
            vat_amount_cents: None,
            fx_currency: None,
            fx_amount_cents: None,
        },
    ];
    let entry = create_entry(conn, &tx_date, &description, &postings, "bank", Some(&format!("tx:{bank_tx_id}")), actor)?;
    let posted = post_entry(conn, entry.meta.id, actor)?;
    conn.execute(
        "INSERT INTO reconciliations (bank_tx_id, target_type, target_id, method, confidence, created_by) VALUES (?1, 'invoice', ?2, ?3, 1.0, ?4)",
        params![bank_tx_id, invoice_id, if actor.starts_with("agent") { "agent" } else { "manual" }, actor],
    )?;
    conn.execute("UPDATE bank_transactions SET state = 'matched' WHERE id = ?1", [bank_tx_id])?;
    crate::audit::record(
        conn,
        actor,
        "invoice.payment_bank",
        Some("bank match"),
        Some(&serde_json::json!({ "invoiceId": invoice_id, "bankTxId": bank_tx_id })),
        "ok",
        &[posted.meta.id],
    )?;
    Ok(paid)
}

// --- line spec parsing ----------------------------------------------------

/// A parsed invoice line ("[QTYx] DESCRIPTION @ PRICE [@ VATCODE]").
#[derive(Debug, Clone)]
pub struct ParsedLine {
    pub qty: i64,
    pub description: String,
    pub price_cents: i64,
    pub vat_code: Option<String>,
    pub vat_rate_bp: i64,
}

fn is_vat_like(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
}

/// Parse a line spec, splitting from the right so descriptions may contain '@'.
pub fn parse_line_spec(spec: &str) -> Result<ParsedLine> {
    let s = spec.trim();
    let qty_re = Regex::new(r"^(\d+)\s*x\s+(.+)$").unwrap();
    let (qty, rest) = match qty_re.captures(s) {
        Some(c) => (c[1].parse::<i64>().unwrap_or(1), c[2].to_string()),
        None => (1, s.to_string()),
    };
    let parts: Vec<&str> = rest.split('@').map(|p| p.trim()).collect();
    let mut vat_code = None;
    let mut price_part = parts.last().copied().unwrap_or("");
    if parts.len() >= 2 && is_vat_like(price_part) {
        vat_code = Some(price_part.to_string());
        price_part = parts[parts.len() - 2];
    }
    let keep = parts.len() - if vat_code.is_some() { 2 } else { 1 };
    let description = parts[..keep].join("@").trim().to_string();
    let price_cents = crate::bank::csv::parse_bank_amount(price_part);
    match price_cents {
        Some(p) if !description.is_empty() && p > 0 => Ok(ParsedLine {
            qty,
            description,
            price_cents: p,
            vat_code,
            vat_rate_bp: 0,
        }),
        _ => Err(invoice_error(
            "INVALID_LINE",
            format!("line '{spec}' must be \"[QTYx] DESCRIPTION @ PRICE [@ VATCODE]\""),
        )),
    }
}

/// Split a (possibly comma-separated) line-spec list into individual specs.
pub fn split_line_specs(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .flat_map(|spec| spec.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()))
        .collect()
}

/// Validate a line-spec list without inserting anything. Returns the parsed
/// lines with vat_rate_bp resolved (used by recurring invoice templates).
pub fn validate_invoice_lines(conn: &Connection, lines: &[String]) -> Result<Vec<ParsedLine>> {
    let vat_on = is_vat_enabled(conn)?;
    let vat_codes = list_vat_codes(conn)?;
    let mut out = Vec::new();
    for spec in split_line_specs(lines) {
        let mut p = parse_line_spec(&spec)?;
        if p.description.is_empty() || p.price_cents <= 0 {
            return Err(invoice_error(
                "INVALID_LINE",
                format!("line '\"{spec}\"' is not parseable"),
            ));
        }
        if p.qty < 1 {
            return Err(invoice_error(
                "INVALID_LINE",
                format!("line '\"{spec}\"': quantity must be a positive integer"),
            ));
        }
        if let Some(vc) = &p.vat_code {
            if !vat_on {
                return Err(invoice_error(
                    "VAT_MODULE_OFF",
                    "line has a VAT code but the VAT module is off for this company",
                ));
            }
            if !vat_codes.iter().any(|c| &c.code == vc) {
                return Err(invoice_error(
                    "VAT_CODE_NOT_FOUND",
                    format!("vat code '{vc}' does not exist"),
                ));
            }
            p.vat_rate_bp = vat_codes.iter().find(|c| &c.code == vc).map(|c| c.rate_bp).unwrap_or(0);
        }
        out.push(p);
    }
    Ok(out)
}

// --- contacts -------------------------------------------------------------

pub fn create_contact(
    conn: &Connection,
    name: &str,
    address: Option<&str>,
    postal_code: Option<&str>,
    city: Option<&str>,
    country: &str,
    email: Option<&str>,
    vat_id: Option<&str>,
    kvk: Option<&str>,
    actor: &str,
) -> Result<Contact> {
    if name.is_empty() {
        return Err(invoice_error("INVALID_NAME", "contact needs a name"));
    }
    conn.execute(
        "INSERT INTO contacts (name, address, postal_code, city, country, email, vat_id, kvk, created_by)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![name, address, postal_code, city, country, email, vat_id, kvk, actor],
    )?;
    let id = conn.last_insert_rowid();
    crate::audit::record(
        conn,
        actor,
        "contact.create",
        Some("contact add"),
        Some(&serde_json::json!({ "name": name })),
        "ok",
        &[],
    )?;
    Ok(get_contact(conn, id)?.expect("just inserted"))
}

pub fn list_contacts(conn: &Connection) -> Result<Vec<Contact>> {
    let mut stmt = conn.prepare("SELECT id, name, address, postal_code, city, country, email, vat_id, kvk, created_by, created_at FROM contacts ORDER BY name")?;
    let rows = stmt
        .query_map([], |r| {
            Ok(Contact {
                id: r.get(0)?,
                name: r.get(1)?,
                address: r.get(2)?,
                postal_code: r.get(3)?,
                city: r.get(4)?,
                country: r.get(5)?,
                email: r.get(6)?,
                vat_id: r.get(7)?,
                kvk: r.get(8)?,
                created_by: r.get(9)?,
                created_at: r.get(10)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

// --- create (draft) -------------------------------------------------------

/// A line spec for create_invoice (pre-parsed by the caller).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvoiceLineSpec {
    pub qty: i64,
    pub description: String,
    pub price_cents: i64,
    pub vat_code: Option<String>,
}

/// Create a draft invoice. Lines carry a VAT rate snapshot (vat_rate_bp).
pub fn create_invoice(
    conn: &Connection,
    contact_id: i64,
    lines: &[InvoiceLineSpec],
    date: &str,
    due_days: Option<i64>,
    delivery_date: Option<&str>,
    description: Option<&str>,
    reference: Option<&str>,
    notes: Option<&str>,
    actor: &str,
) -> Result<Invoice> {
    if get_contact(conn, contact_id)?.is_none() {
        return Err(invoice_error(
            "CONTACT_NOT_FOUND",
            format!("contact {contact_id} does not exist"),
        ));
    }
    let iso = Regex::new(r"^\d{4}-\d{2}-\d{2}$").unwrap();
    if !iso.is_match(date) {
        return Err(invoice_error(
            "INVALID_DATE",
            format!("date '{date}' must be YYYY-MM-DD"),
        ));
    }
    if lines.is_empty() {
        return Err(invoice_error("NO_LINES", "an invoice needs at least one line"));
    }

    let vat_on = is_vat_enabled(conn)?;
    let vat_codes = list_vat_codes(conn)?;
    let mut parsed: Vec<(i64, String, i64, Option<String>, i64, i64, i64)> = Vec::new();
    for l in lines {
        if l.description.is_empty() || l.price_cents <= 0 {
            return Err(invoice_error("INVALID_LINE", "line is not parseable"));
        }
        if l.qty < 1 {
            return Err(invoice_error(
                "INVALID_LINE",
                "quantity must be a positive integer",
            ));
        }
        if let Some(vc) = &l.vat_code {
            if !vat_on {
                return Err(invoice_error(
                    "VAT_MODULE_OFF",
                    "line has a VAT code but the VAT module is off for this company",
                ));
            }
            if !vat_codes.iter().any(|c| &c.code == vc) {
                return Err(invoice_error(
                    "VAT_CODE_NOT_FOUND",
                    format!("vat code '{vc}' does not exist"),
                ));
            }
        }
        let rate_bp = l
            .vat_code
            .as_ref()
            .and_then(|vc| vat_codes.iter().find(|c| &c.code == vc))
            .map(|c| c.rate_bp)
            .unwrap_or(0);
        let amount = l.qty * l.price_cents;
        let vat = ((amount.abs() * rate_bp) as f64 / 10000.0).round() as i64;
        parsed.push((l.qty, l.description.clone(), l.price_cents, l.vat_code.clone(), rate_bp, amount, vat));
    }

    let due_date = due_days.map(|d| {
        let dt = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap();
        (dt + chrono::Duration::days(d)).format("%Y-%m-%d").to_string()
    });

    let tx = crate::core::db::SavepointGuard::begin(conn)?;
    tx.execute(
        "INSERT INTO invoices (contact_id, date, due_date, delivery_date, description, reference, notes, created_by)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![contact_id, date, due_date, delivery_date, description, reference, notes, actor],
    )?;
    let invoice_id = tx.last_insert_rowid();
    {
        let mut stmt = tx.prepare(
            "INSERT INTO invoice_lines
               (invoice_id, line_no, description, quantity, unit_price_cents, vat_code, vat_rate_bp, amount_cents, vat_amount_cents)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        )?;
        for (i, (qty, desc, price, vc, rate, amount, vat)) in parsed.iter().enumerate() {
            stmt.execute(params![
                invoice_id,
                (i + 1) as i64,
                desc,
                qty,
                price,
                vc,
                rate,
                amount,
                vat
            ])?;
        }
    }
    let net: i64 = parsed.iter().map(|p| p.5).sum();
    crate::audit::record(
        &tx,
        actor,
        "invoice.create",
        Some("invoice create"),
        Some(&serde_json::json!({ "contactId": contact_id, "date": date, "lines": parsed.len(), "net": net })),
        "ok",
        &[],
    )?;
    tx.commit()?;
    Ok(get_invoice(conn, invoice_id)?.expect("just inserted"))
}

// --- numbering, compliance, finalize, credit, list ------------------------

/// Sequential per-year invoice number: `YYYY-NNNN`.
pub fn next_invoice_number(conn: &Connection, year: &str) -> Result<String> {
    let m: Option<i64> = conn.query_row(
        "SELECT MAX(CAST(SUBSTR(invoice_number, 6) AS INTEGER)) AS m FROM invoices WHERE invoice_number LIKE ?1",
        [format!("{year}-%")],
        |r| r.get::<_, Option<i64>>(0),
    )?;
    Ok(format!("{year}-{:04}", m.unwrap_or(0) + 1))
}

/// Validate the 12 verplichte factuurvereisten (art. 35c/35d Wet OB + KVK).
pub fn validate_compliance(conn: &Connection, invoice: &Invoice) -> Result<()> {
    let company: (Option<String>, Option<String>, Option<String>, Option<String>, Option<String>, Option<String>) =
        conn.query_row(
            "SELECT name, btw_id, kvk, address, postal_code, city FROM company WHERE id = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
        )?;
    let mut missing: Vec<&str> = Vec::new();
    if company.0.as_deref().unwrap_or("").is_empty() { missing.push("bedrijfsnaam"); }
    if company.1.as_deref().unwrap_or("").is_empty() { missing.push("btw-id"); }
    if company.2.as_deref().unwrap_or("").is_empty() { missing.push("kvk-nummer"); }
    if company.3.as_deref().unwrap_or("").is_empty() { missing.push("adres"); }
    if company.4.as_deref().unwrap_or("").is_empty() { missing.push("postcode"); }
    if company.5.as_deref().unwrap_or("").is_empty() { missing.push("plaats"); }
    if !missing.is_empty() {
        return Err(invoice_error(
            "SUPPLIER_INCOMPLETE",
            format!("supplier gegevens ontbreken (vereiste 1-3): {} — set them with init/company update", missing.join(", ")),
        ));
    }
    let contact = invoice.contact.as_ref();
    let name_ok = contact.map(|c| !c.name.is_empty()).unwrap_or(false);
    let address_ok = contact.map(|c| c.address.as_deref().unwrap_or("").len() > 0).unwrap_or(false);
    let city_ok = contact.map(|c| c.city.as_deref().unwrap_or("").len() > 0).unwrap_or(false);
    if !(name_ok && address_ok && city_ok) {
        return Err(invoice_error(
            "CUSTOMER_INCOMPLETE",
            "klantgegevens ontbreken (vereiste 6): naam, adres en plaats zijn verplicht",
        ));
    }
    let has_reverse = invoice.lines.iter().any(|l| {
        matches!(l.vat_code.as_deref(), Some("R") | Some("RE"))
    });
    if has_reverse && contact.map(|c| c.vat_id.is_none()).unwrap_or(true) {
        return Err(invoice_error(
            "CUSTOMER_VAT_REQUIRED",
            "btw verlegd op de factuur: het btw-id van de klant is verplicht (vereiste 7)",
        ));
    }
    Ok(())
}

/// Build the booking postings for an invoice: Debiteuren vs Omzet + Te betalen
/// btw (VAT module on) or Debiteuren vs Omzet (module off). Credit notes use
/// the reversed signs. Line-exact VAT rounding is preserved per rate group.
pub fn build_invoice_postings(
    conn: &Connection,
    invoice: &Invoice,
) -> Result<Vec<crate::core::entries::PostingSpec>> {
    let vat_on = is_vat_enabled(conn)?;
    let is_credit = invoice.invoice_type == "credit";
    let sign: i64 = if is_credit { 1 } else { -1 };
    let gross = invoice.gross_cents;
    let mut postings: Vec<crate::core::entries::PostingSpec> = Vec::new();

    if vat_on {
        // group nets by vat code (vat totals are the exact per-line sums)
        let mut groups: Vec<(Option<String>, i64, i64)> = Vec::new();
        for l in &invoice.lines {
            let key = l.vat_code.clone();
            if let Some(g) = groups.iter_mut().find(|g| g.0 == key) {
                g.1 += l.amount_cents;
                g.2 += l.vat_amount_cents;
            } else {
                groups.push((key, l.amount_cents, l.vat_amount_cents));
            }
        }
        for (code, net, vat) in groups {
            if net == 0 { continue; }
            match &code {
                Some(c) if vat > 0 => postings.push(crate::core::entries::PostingSpec {
                    code: "8000".to_string(), amount_cents: sign * net,
                    vat_code: Some(c.clone()), vat_amount_cents: Some(sign * vat),
                    fx_currency: None, fx_amount_cents: None,
                }),
                Some(c) => postings.push(crate::core::entries::PostingSpec {
                    code: "8000".to_string(), amount_cents: sign * net,
                    vat_code: Some(c.clone()), vat_amount_cents: Some(0),
                    fx_currency: None, fx_amount_cents: None,
                }),
                None => postings.push(crate::core::entries::PostingSpec {
                    code: "8000".to_string(), amount_cents: sign * net,
                    vat_code: None, vat_amount_cents: None,
                    fx_currency: None, fx_amount_cents: None,
                }),
            }
        }
        if invoice.vat_cents > 0 {
            postings.push(crate::core::entries::PostingSpec {
                code: "2500".to_string(), amount_cents: sign * invoice.vat_cents,
                vat_code: None, vat_amount_cents: None,
                fx_currency: None, fx_amount_cents: None,
            });
        }
    } else {
        postings.push(crate::core::entries::PostingSpec {
            code: "8000".to_string(), amount_cents: sign * invoice.net_cents,
            vat_code: None, vat_amount_cents: None,
            fx_currency: None, fx_amount_cents: None,
        });
    }
    postings.push(crate::core::entries::PostingSpec {
        code: "1200".to_string(), amount_cents: if is_credit { -gross } else { gross },
        vat_code: None, vat_amount_cents: None,
        fx_currency: None, fx_amount_cents: None,
    });
    Ok(postings)
}

/// Result of `finalize_invoice` (dry-run plan or real outcome).
#[derive(Debug, Clone)]
pub struct FinalizeResult {
    pub dry_run: bool,
    pub invoice_number: String,
    pub postings: Option<Vec<crate::core::entries::PostingSpec>>,
    pub net_cents: i64,
    pub vat_cents: i64,
    pub gross_cents: i64,
    pub invoice: Option<Invoice>,
    pub entry_id: Option<i64>,
    pub entry_state: Option<String>,
}

/// Finalize a draft invoice: assign the sequential number and book the entry.
pub fn finalize_invoice(
    conn: &Connection,
    id: i64,
    actor: &str,
    dry_run: bool,
) -> Result<FinalizeResult> {
    let invoice = get_invoice(conn, id)?.ok_or_else(|| {
        invoice_error("NOT_FOUND", format!("invoice {id} does not exist"))
    })?;
    if invoice.status != "draft" {
        return Err(invoice_error(
            "ALREADY_FINALIZED",
            format!("invoice {id} is already {}", invoice.status),
        ));
    }
    validate_compliance(conn, &invoice)?;
    let year = invoice.date[..4].to_string();
    let number = next_invoice_number(conn, &year)?;
    let postings = build_invoice_postings(conn, &invoice)?;

    if dry_run {
        return Ok(FinalizeResult {
            dry_run: true,
            invoice_number: number,
            postings: Some(postings),
            net_cents: invoice.net_cents,
            vat_cents: invoice.vat_cents,
            gross_cents: invoice.gross_cents,
            invoice: None,
            entry_id: None,
            entry_state: None,
        });
    }

    let sp = crate::core::db::SavepointGuard::begin(conn)?;
    let description = format!(
        "Factuur {}{}",
        number,
        invoice.contact.as_ref().map(|c| format!(" - {}", c.name)).unwrap_or_default()
    );
    let entry = create_entry(
        conn,
        &invoice.date,
        &description,
        &postings,
        "invoice",
        Some(&format!("inv:{id}")),
        actor,
    )?;
    let posted = post_entry(conn, entry.meta.id, actor)?;
    conn.execute(
        "UPDATE invoices SET invoice_number = ?1, status = 'sent', entry_id = ?2 WHERE id = ?3",
        params![number, posted.meta.id, id],
    )?;
    crate::audit::record(
        conn,
        actor,
        "invoice.finalize",
        Some("invoice finalize"),
        Some(&serde_json::json!({ "id": id, "invoice_number": number, "gross": invoice.gross_cents })),
        "ok",
        &[posted.meta.id],
    )?;
    sp.commit()?;

    Ok(FinalizeResult {
        dry_run: false,
        invoice_number: number,
        postings: None,
        net_cents: invoice.net_cents,
        vat_cents: invoice.vat_cents,
        gross_cents: invoice.gross_cents,
        invoice: get_invoice(conn, id)?,
        entry_id: Some(posted.meta.id),
        entry_state: Some(posted.meta.state),
    })
}

/// Create a credit note (draft) from a finalized sales invoice.
pub fn credit_invoice(
    conn: &Connection,
    id: i64,
    date: Option<&str>,
    reason: Option<&str>,
    actor: &str,
) -> Result<Invoice> {
    let original = get_invoice(conn, id)?.ok_or_else(|| {
        invoice_error("NOT_FOUND", format!("invoice {id} does not exist"))
    })?;
    if original.invoice_type != "sales" {
        return Err(invoice_error("NOT_SALES_INVOICE", "only sales invoices can be credited"));
    }
    if !["sent", "paid", "overdue"].contains(&original.status.as_str()) {
        return Err(invoice_error(
            "NOT_FINALIZED",
            "the invoice must be finalized before crediting",
        ));
    }
    let lines: Vec<InvoiceLineSpec> = original
        .lines
        .iter()
        .map(|l| InvoiceLineSpec {
            qty: l.quantity,
            description: l.description.clone(),
            price_cents: l.unit_price_cents,
            vat_code: l.vat_code.clone(),
        })
        .collect();
    let date_owned = date.map(|d| d.to_string()).unwrap_or_else(today_iso);
    let description = reason
        .map(|r| r.to_string())
        .unwrap_or_else(|| format!("Creditfactuur voor {}", original.invoice_number.clone().unwrap_or_default()));
    let credit_id = create_invoice(
        conn,
        original.contact_id,
        &lines,
        &date_owned,
        None,
        None,
        Some(&description),
        original.invoice_number.as_deref(),
        None,
        actor,
    )?
    .id;
    conn.execute(
        "UPDATE invoices SET invoice_type = 'credit', credit_for_invoice_id = ?1 WHERE id = ?2",
        params![id, credit_id],
    )?;
    crate::audit::record(
        conn,
        actor,
        "invoice.credit",
        Some("invoice credit"),
        Some(&serde_json::json!({ "id": id, "creditId": credit_id })),
        "ok",
        &[],
    )?;
    get_invoice(conn, credit_id)?.ok_or_else(|| {
        invoice_error("NOT_FOUND", format!("invoice {credit_id} does not exist"))
    })
}

/// List invoices with optional status/type filters (oldest first).
pub fn list_invoices(
    conn: &Connection,
    status: Option<&str>,
    inv_type: Option<&str>,
) -> Result<Vec<Invoice>> {
    let mut sql = String::from(
        "SELECT id FROM invoices",
    );
    let mut clauses: Vec<String> = Vec::new();
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    if let Some(s) = status {
        clauses.push("status = ?".to_string());
        args.push(Box::new(s.to_string()));
    }
    if let Some(t) = inv_type {
        clauses.push("invoice_type = ?".to_string());
        args.push(Box::new(t.to_string()));
    }
    if !clauses.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&clauses.join(" AND "));
    }
    sql.push_str(" ORDER BY id DESC");
    let mut stmt = conn.prepare(&sql)?;
    let ids: Vec<i64> = stmt
        .query_map(rusqlite::params_from_iter(args.iter()), |r| r.get(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut out = Vec::new();
    for id in ids {
        if let Some(inv) = get_invoice(conn, id)? {
            out.push(inv);
        }
    }
    Ok(out)
}

// --- UBL / Peppol / PDF ---------------------------------------------------

fn xml_esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn money_amount(cents: i64) -> String {
    format!("{:.2}", cents as f64 / 100.0)
}

struct CompanyRow {
    name: String,
    kvk: Option<String>,
    btw_id: Option<String>,
    iban: Option<String>,
    address: Option<String>,
    postal_code: Option<String>,
    city: Option<String>,
}

fn get_company(conn: &Connection) -> Result<CompanyRow> {
    let (name, kvk, btw_id, iban, address, postal_code, city) = conn.query_row(
        "SELECT name, kvk, btw_id, iban, address, postal_code, city FROM company WHERE id = 1",
        [],
        |r| {
            Ok((
                r.get::<_, String>(0)?, r.get(1)?, r.get(2)?, r.get(3)?,
                r.get(4)?, r.get(5)?, r.get(6)?,
            ))
        },
    )?;
    Ok(CompanyRow { name, kvk, btw_id, iban, address, postal_code, city })
}

/// Build a Peppol BIS 3.0 UBL Invoice (380) or CreditNote (381) XML document.
pub fn invoice_to_ubl(conn: &Connection, invoice: &Invoice) -> Result<String> {
    let company = get_company(conn)?;
    let contact = invoice.contact.as_ref().ok_or_else(|| {
        invoice_error("CUSTOMER_INCOMPLETE", "invoice has no contact")
    })?;
    let is_credit = invoice.invoice_type == "credit";
    let type_code = if is_credit { "381" } else { "380" };

    let mut tax_subtotals = String::new();
    let mut by_rate: Vec<(Option<String>, i64, i64, i64)> = Vec::new();
    for l in &invoice.lines {
        let key = (l.vat_code.clone(), l.vat_rate_bp);
        if let Some(g) = by_rate.iter_mut().find(|g| g.0 == key.0 && g.1 == key.1) {
            g.2 += l.amount_cents;
            g.3 += l.vat_amount_cents;
        } else {
            by_rate.push((key.0, key.1, l.amount_cents, l.vat_amount_cents));
        }
    }
    for (code, rate_bp, taxable, tax) in by_rate {
        if tax == 0 { continue; }
        let is_reverse = matches!(code.as_deref(), Some("R") | Some("RE"));
        let category = if is_reverse { "AE" } else { "S" };
        let percent = if is_reverse {
            "21.00".to_string()
        } else {
            format!("{:.2}", rate_bp as f64 / 100.0)
        };
        tax_subtotals.push_str(&format!(
            "\n      <cac:TaxSubtotal>\n        <cbc:TaxableAmount currencyID=\"{cur}\">{taxable}</cbc:TaxableAmount>\n        <cbc:TaxAmount currencyID=\"{cur}\">{tax}</cbc:TaxAmount>\n        <cac:TaxCategory>\n          <cbc:ID>{category}</cbc:ID>\n          <cbc:Percent>{percent}</cbc:Percent>\n          <cac:TaxScheme><cbc:ID>VAT</cbc:ID></cac:TaxScheme>\n        </cac:TaxCategory>\n      </cac:TaxSubtotal>",
            cur = invoice.currency,
            taxable = money_amount(taxable),
            tax = money_amount(tax),
        ));
    }

    let mut lines_xml = String::new();
    for (i, l) in invoice.lines.iter().enumerate() {
        let is_reverse = matches!(l.vat_code.as_deref(), Some("R") | Some("RE"));
        let category = if is_reverse { "AE" } else if l.vat_code.is_some() { "S" } else { "E" };
        let percent = if is_reverse {
            "21.00".to_string()
        } else {
            format!("{:.2}", l.vat_rate_bp as f64 / 100.0)
        };
        lines_xml.push_str(&format!(
            "\n    <cac:InvoiceLine>\n      <cbc:ID>{}</cbc:ID>\n      <cbc:InvoicedQuantity unitCode=\"C62\">{}</cbc:InvoicedQuantity>\n      <cbc:LineExtensionAmount currencyID=\"{cur}\">{amount}</cbc:LineExtensionAmount>\n      <cac:Item>\n        <cbc:Name>{desc}</cbc:Name>\n        <cac:ClassifiedTaxCategory>\n          <cbc:ID>{category}</cbc:ID>\n          <cbc:Percent>{percent}</cbc:Percent>\n          <cac:TaxScheme><cbc:ID>VAT</cbc:ID></cac:TaxScheme>\n        </cac:ClassifiedTaxCategory>\n      </cac:Item>\n      <cac:Price>\n        <cbc:PriceAmount currencyID=\"{cur}\">{price}</cbc:PriceAmount>\n      </cac:Price>\n    </cac:InvoiceLine>",
            i + 1,
            l.quantity,
            cur = invoice.currency,
            amount = money_amount(l.amount_cents),
            desc = xml_esc(&l.description),
            price = money_amount(l.unit_price_cents),
        ));
    }

    let buyer_tax = contact
        .vat_id
        .as_ref()
        .map(|v| {
            format!(
                "\n        <cac:PartyTaxScheme><cbc:CompanyID schemeID=\"VAT\">{}</cbc:CompanyID><cac:TaxScheme><cbc:ID>VAT</cbc:ID></cac:TaxScheme></cac:PartyTaxScheme>",
                xml_esc(v)
            )
        })
        .unwrap_or_default();

    let supplier_block = format!(
        // NB: mirrors Node's addressBlock(company.name, company) destructure
        // {address, postalCode, city, country} — the camelCase key misses the
        // snake_case postal_code column, so the supplier PostalZone is ALWAYS
        // empty in Node. Faithful port: keep it empty.
        "\n        <cac:Party>\n          <cac:PartyName><cbc:Name>{}</cbc:Name></cac:PartyName>\n          <cac:PostalAddress>\n            <cbc:StreetName>{}</cbc:StreetName>\n            <cbc:CityName>{}</cbc:CityName>\n            <cbc:PostalZone></cbc:PostalZone>\n            <cac:Country><cbc:IdentificationCode>NL</cbc:IdentificationCode></cac:Country>\n          </cac:PostalAddress>\n        </cac:Party>",
        xml_esc(&company.name),
        xml_esc(company.address.as_deref().unwrap_or("")),
        xml_esc(company.city.as_deref().unwrap_or("")),
    );

    let customer_block = format!(
        "\n    <cac:Party>\n      <cac:PartyName><cbc:Name>{}</cbc:Name></cac:PartyName>\n      <cac:PostalAddress>\n        <cbc:StreetName>{}</cbc:StreetName>\n        <cbc:CityName>{}</cbc:CityName>\n        <cbc:PostalZone>{}</cbc:PostalZone>\n        <cac:Country><cbc:IdentificationCode>{}</cbc:IdentificationCode></cac:Country>\n      </cac:PostalAddress>{buyer_tax}\n    </cac:Party>",
        xml_esc(&contact.name),
        xml_esc(contact.address.as_deref().unwrap_or("")),
        xml_esc(contact.city.as_deref().unwrap_or("")),
        xml_esc(contact.postal_code.as_deref().unwrap_or("")),
        xml_esc(&contact.country),
    );

    let due_date_xml = invoice
        .due_date
        .as_ref()
        .map(|d| format!("\n  <cbc:DueDate>{d}</cbc:DueDate>"))
        .unwrap_or_default();
    let notes_xml = invoice
        .notes
        .as_ref()
        .map(|n| format!("\n  <cbc:Note>{}</cbc:Note>", xml_esc(n)))
        // Node template: "  ${notes ? `<cbc:Note>..</cbc:Note>` : ''}\n  <cac:...>"
        // — an absent Note still leaves the "  " line (2 spaces + newline).
        .unwrap_or_else(|| "\n  ".to_string());
    let payment_terms = invoice
        .due_date
        .as_ref()
        .map(|d| format!("\n  <cac:PaymentTerms><cbc:PaymentDueDate>{d}</cbc:PaymentDueDate></cac:PaymentTerms>"))
        .unwrap_or_default();

    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Invoice xmlns=\"urn:oasis:names:specification:ubl:schema:xsd:Invoice-2\"\n         xmlns:cac=\"urn:oasis:names:specification:ubl:schema:xsd:CommonAggregateComponents-2\"\n         xmlns:cbc=\"urn:oasis:names:specification:ubl:schema:xsd:CommonBasicComponents-2\">\n  <cbc:CustomizationID>urn:cen.eu:en16931:2017#compliant#urn:fdc:peppol.eu:2017:poacc:billing:3.0</cbc:CustomizationID>\n  <cbc:ProfileID>urn:fdc:peppol.eu:2017:poacc:billing:01:1.0</cbc:ProfileID>\n  <cbc:ID>{}</cbc:ID>\n  <cbc:IssueDate>{}</cbc:IssueDate>{due_date_xml}\n  <cbc:InvoiceTypeCode>{}</cbc:InvoiceTypeCode>{notes_xml}\n  <cac:AccountingSupplierParty>{supplier_block}</cac:AccountingSupplierParty>\n  <cac:AccountingCustomerParty>{customer_block}\n  </cac:AccountingCustomerParty>\n  <cac:PaymentMeans>\n    <cbc:PaymentMeansCode>30</cbc:PaymentMeansCode>\n    <cac:PayeeFinancialAccount><cbc:ID>{}</cbc:ID></cac:PayeeFinancialAccount>\n  </cac:PaymentMeans>{payment_terms}\n  <cac:TaxTotal>\n    <cbc:TaxAmount currencyID=\"{cur}\">{vat}</cbc:TaxAmount>{tax_subtotals}\n  </cac:TaxTotal>\n  <cac:LegalMonetaryTotal>\n    <cbc:LineExtensionAmount currencyID=\"{cur}\">{net}</cbc:LineExtensionAmount>\n    <cbc:TaxExclusiveAmount currencyID=\"{cur}\">{net}</cbc:TaxExclusiveAmount>\n    <cbc:TaxInclusiveAmount currencyID=\"{cur}\">{gross}</cbc:TaxInclusiveAmount>\n    <cbc:PayableAmount currencyID=\"{cur}\">{gross}</cbc:PayableAmount>\n  </cac:LegalMonetaryTotal>{lines_xml}\n</Invoice>",
        xml_esc(invoice.invoice_number.as_deref().unwrap_or("")),
        invoice.date,
        type_code,
        xml_esc(company.iban.as_deref().unwrap_or("")),
        cur = invoice.currency,
        vat = money_amount(invoice.vat_cents),
        net = money_amount(invoice.net_cents),
        gross = money_amount(invoice.gross_cents),
    );
    Ok(xml)
}

/// Peppol access-point config from the environment (never hard-coded).
pub fn peppol_config() -> (Option<String>, Option<String>) {
    (
        std::env::var("BUKIO_PEPPOL_ENDPOINT").ok().filter(|s| !s.is_empty()),
        std::env::var("BUKIO_PEPPOL_TOKEN").ok().filter(|s| !s.is_empty()),
    )
}

/// Send a finalized invoice to the Peppol access point. dry_run validates
/// the configuration + payload without making the request.
pub fn send_peppol_invoice(
    conn: &Connection,
    invoice: &Invoice,
    endpoint: Option<&str>,
    dry_run: bool,
) -> Result<serde_json::Value> {
    let (cfg_ep, cfg_token) = peppol_config();
    let ep = endpoint.map(|e| e.to_string()).or(cfg_ep);
    let Some(ep) = ep else {
        return Err(invoice_error(
            "PEPPOL_NOT_CONFIGURED",
            "no Peppol endpoint — set BUKIO_PEPPOL_ENDPOINT (or pass --endpoint)",
        ));
    };
    if dry_run {
        let xml = invoice_to_ubl(conn, invoice)?;
        return Ok(serde_json::json!({
            "dryRun": true,
            "invoice_number": invoice.invoice_number,
            "endpoint": ep,
            "configured": cfg_token.is_some(),
            "bytes": xml.len(),
        }));
    }
    let xml = invoice_to_ubl(conn, invoice)?;
    let mut req = ureq::post(&ep)
        .header("Content-Type", "application/xml; charset=utf-8");
    if let Some(token) = &cfg_token {
        req = req.header("Authorization", &format!("Bearer {token}"));
    }
    match req.send(xml.clone()) {
        Ok(mut resp) => {
            let status = resp.status();
            let body = resp.body_mut().read_to_string().unwrap_or_default();
            Ok(serde_json::json!({
                "dryRun": false,
                "invoice_number": invoice.invoice_number,
                "endpoint": ep,
                "status": status.as_u16(),
                "response": if body.is_empty() { Value::Null } else { Value::String(body.chars().take(500).collect()) },
            }))
        }
        Err(ureq::Error::StatusCode(code)) => {
            let status_u16: u16 = code.to_string().parse().unwrap_or(0);
            Err(invoice_error(
                "PEPPOL_SEND_FAILED",
                format!("provider returned {status_u16}"),
            ))
        }
        Err(e) => Err(invoice_error(
            "PEPPOL_SEND_FAILED",
            format!("provider unreachable: {e}"),
        )),
    }
}

fn today_iso() -> String {
    chrono::Utc::now().date_naive().format("%Y-%m-%d").to_string()
}

fn find_chromium() -> Option<std::path::PathBuf> {
    let cache = dirs_home().unwrap_or_else(|| "/root".to_string()) + "/.cache/ms-playwright";
    let entries = std::fs::read_dir(&cache).ok()?;
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with("chromium-") || name.starts_with("chromium_headless_shell-") {
            let bin = e.path().join("chrome-linux64/chrome");
            if bin.exists() {
                return Some(bin);
            }
            let bin2 = e.path().join("chrome-linux/chrome");
            if bin2.exists() {
                return Some(bin2);
            }
        }
    }
    None
}

fn dirs_home() -> Option<String> {
    std::env::var("HOME").ok()
}

/// Invoice PDF via headless Chromium (Playwright's locally installed
/// browsers) from an HTML template — mirrors the Node pdf.js approach.
pub fn invoice_to_pdf(
    conn: &Connection,
    invoice: &Invoice,
    out_path: &str,
) -> Result<(usize, String)> {
    let html = invoice_html(conn, invoice)?;
    let chrome = find_chromium().ok_or_else(|| {
        invoice_error("PDF_UNAVAILABLE", "could not find Playwright Chromium — run the playwright install step")
    })?;
    let tmp_html = format!("/tmp/bukio-invoice-{}.html", invoice.id);
    std::fs::write(&tmp_html, html)
        .map_err(|e| invoice_error("PDF_UNAVAILABLE", format!("cannot write html: {e}")))?;
    let status = std::process::Command::new(&chrome)
        .args([
            "--headless",
            "--disable-gpu",
            "--no-sandbox",
            "--print-to-pdf-no-header",
            &format!("--print-to-pdf={out_path}"),
            &tmp_html,
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    let _ = std::fs::remove_file(&tmp_html);
    match status {
        Ok(s) if s.success() => {
            let bytes = std::fs::metadata(out_path).map(|m| m.len() as usize).unwrap_or(0);
            Ok((bytes, out_path.to_string()))
        }
        _ => Err(invoice_error(
            "PDF_UNAVAILABLE",
            "could not render the invoice PDF (headless Chromium failed)",
        )),
    }
}

fn invoice_html(conn: &Connection, invoice: &Invoice) -> Result<String> {
    let company = get_company(conn)?;
    let contact = invoice.contact.as_ref().ok_or_else(|| {
        invoice_error("CUSTOMER_INCOMPLETE", "invoice has no contact")
    })?;
    let is_credit = invoice.invoice_type == "credit";
    let vat_on = invoice.vat_cents > 0;

    let mut rows = String::new();
    for (i, l) in invoice.lines.iter().enumerate() {
        let vat_col = if l.vat_rate_bp > 0 {
            format!("{:.1}%", l.vat_rate_bp as f64 / 100.0)
        } else if matches!(l.vat_code.as_deref(), Some("R") | Some("RE")) {
            "verlegd".to_string()
        } else {
            "-".to_string()
        };
        rows.push_str(&format!(
            "\n      <tr>\n        <td>{}</td>\n        <td>{}</td>\n        <td class=\"num\">{}</td>\n        <td class=\"num\">{}</td>\n        <td class=\"num\">{}</td>\n        <td class=\"num\">{}</td>\n      </tr>",
            i + 1,
            xml_esc(&l.description),
            l.quantity,
            money_amount(l.unit_price_cents),
            vat_col,
            money_amount(l.amount_cents),
        ));
    }

    let vat_row = if vat_on {
        format!("<tr><td>Btw</td><td class=\"num\">{}</td></tr>", money_amount(invoice.vat_cents))
    } else {
        String::new()
    };
    let due_date_xml = invoice
        .due_date
        .as_ref()
        .map(|d| format!("<p>Vervaldatum: {d}</p>"))
        .unwrap_or_default();
    let reference_xml = invoice
        .reference
        .as_ref()
        .map(|r| format!("<p>Referentie: {}</p>", xml_esc(r)))
        .unwrap_or_default();
    let contact_vat = contact
        .vat_id
        .as_ref()
        .map(|v| format!("<p>BTW {}</p>", xml_esc(v)))
        .unwrap_or_default();
    let notes_xml = invoice
        .notes
        .as_ref()
        .map(|n| format!("<p>{}</p>", xml_esc(n)))
        .unwrap_or_default();
    let footer_term = invoice
        .due_date
        .as_ref()
        .map(|d| d.clone())
        .unwrap_or_else(|| "de gestelde termijn".to_string());

    let html = format!(
        "<!DOCTYPE html>\n<html lang=\"nl\">\n<head>\n<meta charset=\"utf-8\">\n<style>\n  body {{ font-family: 'DejaVu Sans', sans-serif; font-size: 11px; color: #1a1a1a; margin: 0; }}\n  .header {{ display: flex; justify-content: space-between; margin-bottom: 28px; }}\n  .supplier h1 {{ font-size: 18px; margin: 0 0 4px 0; }}\n  .supplier p {{ margin: 1px 0; color: #444; }}\n  .title {{ text-align: right; }}\n  .title h2 {{ font-size: 22px; margin: 0 0 8px 0; }}\n  .title p {{ margin: 2px 0; }}\n  .parties {{ display: flex; justify-content: space-between; margin-bottom: 24px; }}\n  .parties h3 {{ font-size: 11px; text-transform: uppercase; color: #666; margin: 0 0 6px 0; }}\n  .parties p {{ margin: 1px 0; }}\n  table {{ width: 100%; border-collapse: collapse; margin-bottom: 20px; }}\n  th {{ text-align: left; border-bottom: 2px solid #333; padding: 4px 6px; font-size: 10px; text-transform: uppercase; color: #555; }}\n  td {{ border-bottom: 1px solid #ddd; padding: 6px; }}\n  .num {{ text-align: right; }}\n  .totals {{ width: 260px; margin-left: auto; }}\n  .totals td {{ border-bottom: none; padding: 3px 6px; }}\n  .totals .grand td {{ border-top: 2px solid #333; font-weight: bold; font-size: 13px; }}\n  .footer {{ margin-top: 40px; font-size: 10px; color: #666; }}\n  .footer p {{ margin: 2px 0; }}\n</style>\n</head>\n<body>\n  <div class=\"header\">\n    <div class=\"supplier\">\n      <h1>{}</h1>\n      <p>{}</p>\n      <p>{} {}</p>\n      <p>KvK {} · BTW {}</p>\n    </div>\n    <div class=\"title\">\n      <h2>{}</h2>\n      <p><strong>{}</strong></p>\n      <p>Datum: {}</p>\n      {}{}\n    </div>\n  </div>\n  <div class=\"parties\">\n    <div>\n      <h3>Factuur aan</h3>\n      <p><strong>{}</strong></p>\n      <p>{}</p>\n      <p>{} {}</p>\n      {}\n    </div>\n  </div>\n  <table>\n    <thead><tr><th>#</th><th>Omschrijving</th><th class=\"num\">Aantal</th><th class=\"num\">Prijs</th><th class=\"num\">Btw</th><th class=\"num\">Bedrag</th></tr></thead>\n    <tbody>{}</tbody>\n  </table>\n  <table class=\"totals\">\n    <tr><td>Subtotaal excl. btw</td><td class=\"num\">{}</td></tr>\n    {}\n    <tr class=\"grand\"><td>Totaal</td><td class=\"num\">{}</td></tr>\n  </table>\n  <div class=\"footer\">\n    <p>Gelieve het bedrag binnen {} over te maken op IBAN {} t.n.v. {}.</p>\n    {}\n  </div>\n</body>\n</html>",
        xml_esc(&company.name),
        xml_esc(company.address.as_deref().unwrap_or("")),
        xml_esc(company.postal_code.as_deref().unwrap_or("")),
        xml_esc(company.city.as_deref().unwrap_or("")),
        xml_esc(company.kvk.as_deref().unwrap_or("")),
        xml_esc(company.btw_id.as_deref().unwrap_or("")),
        if is_credit { "CREDITFACTUUR" } else { "FACTUUR" },
        xml_esc(invoice.invoice_number.as_deref().unwrap_or("concept")),
        invoice.date,
        due_date_xml,
        reference_xml,
        xml_esc(&contact.name),
        xml_esc(contact.address.as_deref().unwrap_or("")),
        xml_esc(contact.postal_code.as_deref().unwrap_or("")),
        xml_esc(contact.city.as_deref().unwrap_or("")),
        contact_vat,
        rows,
        money_amount(invoice.net_cents),
        vat_row,
        money_amount(invoice.gross_cents),
        footer_term,
        xml_esc(company.iban.as_deref().unwrap_or("")),
        xml_esc(&company.name),
        notes_xml,
    );
    Ok(html)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::accounts::seed_default_chart;
    use crate::core::db::open_in_memory;

    fn seeded() -> Connection {
        let conn = open_in_memory().unwrap();
        seed_default_chart(&conn).unwrap();
        conn.execute(
            "INSERT INTO contacts (name, country, created_by) VALUES ('Klant BV', 'NL', 'human')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO invoices (invoice_type, contact_id, date, status, created_by) VALUES ('sales', 1, '2026-07-01', 'sent', 'human')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO invoice_lines (invoice_id, line_no, description, quantity, unit_price_cents, vat_rate_bp, amount_cents, vat_amount_cents)
             VALUES (1, 1, 'Dienst', 1, 10000, 2100, 10000, 2100)",
            [],
        )
        .unwrap();
        conn
    }

    #[test]
    fn get_invoice_computes_amounts() {
        let conn = seeded();
        let inv = get_invoice(&conn, 1).unwrap().unwrap();
        assert_eq!(inv.net_cents, 10000);
        assert_eq!(inv.vat_cents, 2100);
        assert_eq!(inv.gross_cents, 12100);
        assert_eq!(inv.paid_cents, 0);
        assert_eq!(inv.status, "sent");
        assert_eq!(inv.contact.as_ref().unwrap().name, "Klant BV");
    }

    #[test]
    fn mark_paid_rejects_overpayment_and_updates_status() {
        let conn = seeded();
        assert!(mark_paid(&conn, 1, "2026-07-10", 12200, "bank", None, "human").is_err()); // overpayment
        let inv = mark_paid(&conn, 1, "2026-07-10", 12100, "bank", None, "human").unwrap();
        assert_eq!(inv.status, "paid");
        assert_eq!(inv.paid_cents, 12100);
        // second payment on a paid invoice -> NOT_PAYABLE
        let err = mark_paid(&conn, 1, "2026-07-11", 100, "bank", None, "human").unwrap_err();
        assert_eq!(err.code(), "NOT_PAYABLE");
    }

    #[test]
    fn payment_from_bank_posts_and_reconciles() {
        let conn = seeded();
        conn.execute(
            "INSERT INTO bank_accounts (iban, name, account_code) VALUES ('NL91ABNA0417164300', 'Bank', '1100')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO bank_transactions (bank_account_id, date, amount_cents, counterparty, description, hash)
             VALUES (1, '2026-07-10', 12100, 'Klant BV', 'Factuur 1', 'abc123')",
            [],
        )
        .unwrap();
        let inv = payment_from_bank(&conn, 1, 1, "human").unwrap();
        assert_eq!(inv.status, "paid");
        let state: String = conn
            .query_row("SELECT state FROM bank_transactions WHERE id = 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(state, "matched");
        let recon_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM reconciliations WHERE bank_tx_id = 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(recon_count, 1);
        let entry_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM journal_entries WHERE source = 'bank'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(entry_count, 1);
    }
}
