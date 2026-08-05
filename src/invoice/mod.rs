//! Invoice core — contacts, line parsing/validation, create (draft) and the
//! payment functions the bank auto-match engine depends on. The full invoice
//! module (finalize/credit/UBL/PDF/CLI) is ported in the invoice milestone.

use crate::core::entries::{create_entry, post_entry};
use crate::error::{AppError, Result};
use crate::vat::{is_vat_enabled, list_vat_codes};
use regex::Regex;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

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
}

pub fn get_contact(conn: &Connection, id: i64) -> Result<Option<Contact>> {
    let row: Option<Contact> = conn
        .query_row(
            "SELECT id, name, address, postal_code, city, country, email, vat_id, kvk FROM contacts WHERE id = ?1",
            [id],
            |r| {
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
                })
            },
        )
        .optional()?;
    Ok(row)
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
}

pub fn get_invoice(conn: &Connection, id: i64) -> Result<Option<Invoice>> {
    let row: Option<(i64, Option<String>, String, i64, String, Option<String>, Option<String>, Option<String>, Option<String>, String, Option<i64>, Option<i64>, String, Option<String>, String)> = conn
        .query_row(
            "SELECT id, invoice_number, invoice_type, contact_id, date, due_date, delivery_date,
                    description, reference, status, credit_for_invoice_id, entry_id, currency, notes, created_by
             FROM invoices WHERE id = ?1",
            [id],
            |r| {
                Ok((
                    r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?,
                    r.get(7)?, r.get(8)?, r.get(9)?, r.get(10)?, r.get(11)?, r.get(12)?,
                    r.get(13)?, r.get(14)?,
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
        net_cents,
        vat_cents,
        gross_cents: net_cents + vat_cents,
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
    let mut stmt = conn.prepare("SELECT id, name, address, postal_code, city, country, email, vat_id, kvk FROM contacts ORDER BY name")?;
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
