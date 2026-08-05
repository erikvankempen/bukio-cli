//! Bank module — accounts, idempotent import, matching, reconciliation.

use crate::core::accounts::get_account_by_code;
use crate::core::entries::{create_entry, get_entry, post_entry, PostingSpec};
use crate::error::{AppError, Result};
use crate::invoice;
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};

pub mod camt;
pub mod csv;

pub use camt::{parse_camt053, BankTx};

fn tx_hash(iban: &str, t: &BankTx) -> String {
    let raw = format!(
        "{}|{}|{}|{}|{}",
        iban,
        t.date,
        t.amount_cents,
        t.counterparty.clone().unwrap_or_default(),
        t.description.clone().unwrap_or_default()
    );
    let mut hasher = Sha256::new();
    hasher.update(raw.as_bytes());
    let digest = hasher.finalize();
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn normalize_iban(iban: &str) -> String {
    iban.trim().to_uppercase().chars().filter(|c| !c.is_whitespace()).collect()
}

pub fn validate_iban(iban: &str) -> Result<()> {
    let re = regex::Regex::new(r"^[A-Z]{2}\d{2}[A-Z0-9]{10,30}$").unwrap();
    if !re.is_match(iban) {
        return Err(AppError::new("INVALID_IBAN", format!("'{iban}' is not a valid IBAN")));
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct BankAccount {
    pub id: i64,
    pub iban: String,
    pub name: Option<String>,
    pub account_code: String,
}

fn bank_account_from_row(conn: &Connection, id: i64) -> Result<BankAccount> {
    let row = conn.query_row(
        "SELECT id, iban, name, account_code FROM bank_accounts WHERE id = ?1",
        [id],
        |r| {
            Ok(BankAccount {
                id: r.get(0)?,
                iban: r.get(1)?,
                name: r.get(2)?,
                account_code: r.get(3)?,
            })
        },
    )?;
    Ok(row)
}

pub fn get_or_create_bank_account(
    conn: &Connection,
    iban: &str,
    name: Option<&str>,
    account_code: &str,
) -> Result<BankAccount> {
    let iban_norm = normalize_iban(iban);
    validate_iban(&iban_norm)?;
    if get_account_by_code(conn, account_code)?.is_none() {
        return Err(AppError::new(
            "ACCOUNT_NOT_FOUND",
            format!("ledger account {account_code} does not exist"),
        ));
    }
    let existing: Option<i64> = conn
        .query_row("SELECT id FROM bank_accounts WHERE iban = ?1", [&iban_norm], |r| r.get(0))
        .optional()?;
    if let Some(id) = existing {
        return bank_account_from_row(conn, id);
    }
    conn.execute(
        "INSERT INTO bank_accounts (iban, name, account_code) VALUES (?1, ?2, ?3)",
        params![iban_norm, name, account_code],
    )?;
    let id = conn.last_insert_rowid();
    bank_account_from_row(conn, id)
}

#[derive(Debug, Clone)]
pub struct BankAccountSummary {
    pub iban: String,
    pub name: Option<String>,
    pub account_code: String,
    pub transaction_count: i64,
    pub unmatched_count: i64,
    pub balance_cents: i64,
}

pub fn list_bank_accounts(conn: &Connection) -> Result<Vec<BankAccountSummary>> {
    let mut stmt = conn.prepare(
        "SELECT ba.iban, ba.name, ba.account_code,
                COUNT(bt.id) AS transaction_count,
                COALESCE(SUM(CASE WHEN bt.state = 'unmatched' THEN 1 ELSE 0 END), 0) AS unmatched_count,
                COALESCE(SUM(bt.amount_cents), 0) AS balance_cents
         FROM bank_accounts ba
         LEFT JOIN bank_transactions bt ON bt.bank_account_id = ba.id
         GROUP BY ba.id
         ORDER BY ba.iban",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(BankAccountSummary {
                iban: r.get(0)?,
                name: r.get(1)?,
                account_code: r.get(2)?,
                transaction_count: r.get(3)?,
                unmatched_count: r.get(4)?,
                balance_cents: r.get(5)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Preview an import without writing anything (dry-run): counts new vs
/// duplicate hashes against the current database.
pub fn preview_import(conn: &Connection, iban: &str, transactions: &[BankTx]) -> Result<ImportSummary> {
    let iban_norm = normalize_iban(iban);
    validate_iban(&iban_norm)?;
    let mut imported = 0;
    let mut duplicates = 0;
    for t in transactions {
        let hash = tx_hash(&iban_norm, t);
        let exists: i64 = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM bank_transactions WHERE hash = ?1)",
            [&hash],
            |r| r.get(0),
        )?;
        if exists == 1 {
            duplicates += 1;
        } else {
            imported += 1;
        }
    }
    Ok(ImportSummary {
        iban: iban_norm,
        imported,
        duplicates,
        total: transactions.len() as i64,
    })
}

#[derive(Debug, Clone)]
pub struct ImportSummary {
    pub iban: String,
    pub imported: i64,
    pub duplicates: i64,
    pub total: i64,
}

/// Import transactions: inserts only NEW hashes (idempotent).
pub fn import_transactions(
    conn: &mut Connection,
    iban: &str,
    transactions: &[BankTx],
    name: Option<&str>,
    account_code: &str,
    actor: &str,
) -> Result<ImportSummary> {
    if transactions.is_empty() {
        return Err(AppError::new("EMPTY_STATEMENT", "no transactions to import"));
    }
    let account = get_or_create_bank_account(conn, iban, name, account_code)?;

    let mut imported = 0;
    let mut duplicates = 0;
    let tx = conn.transaction()?;
    for t in transactions {
        let hash = tx_hash(&account.iban, t);
        let exists: i64 = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM bank_transactions WHERE hash = ?1)",
            [&hash],
            |r| r.get(0),
        )?;
        if exists == 1 {
            duplicates += 1;
            continue;
        }
        tx.execute(
            "INSERT INTO bank_transactions (bank_account_id, date, amount_cents, counterparty, description, iban_counter, hash)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                account.id,
                t.date,
                t.amount_cents,
                t.counterparty,
                t.description,
                t.iban_counter,
                hash
            ],
        )?;
        imported += 1;
    }
    crate::audit::record(
        &tx,
        actor,
        "bank.import",
        Some("bank import"),
        Some(&serde_json::json!({ "iban": account.iban, "transactions": transactions.len(), "imported": imported, "duplicates": duplicates })),
        "ok",
        &[],
    )?;
    tx.commit()?;
    Ok(ImportSummary {
        iban: account.iban,
        imported,
        duplicates,
        total: transactions.len() as i64,
    })
}

#[derive(Debug, Clone)]
pub struct BankTxRow {
    pub id: i64,
    pub bank_account_id: i64,
    pub date: String,
    pub amount_cents: i64,
    pub counterparty: Option<String>,
    pub description: Option<String>,
    pub iban_counter: Option<String>,
    pub hash: String,
    pub state: String,
    pub iban: String,
    pub account_code: String,
}

fn tx_row_from_row(r: &rusqlite::Row) -> rusqlite::Result<BankTxRow> {
    Ok(BankTxRow {
        id: r.get(0)?,
        bank_account_id: r.get(1)?,
        date: r.get(2)?,
        amount_cents: r.get(3)?,
        counterparty: r.get(4)?,
        description: r.get(5)?,
        iban_counter: r.get(6)?,
        hash: r.get(7)?,
        state: r.get(8)?,
        iban: r.get(9)?,
        account_code: r.get(10)?,
    })
}

const TX_SELECT: &str = "SELECT bt.id, bt.bank_account_id, bt.date, bt.amount_cents, bt.counterparty,
    bt.description, bt.iban_counter, bt.hash, bt.state, ba.iban AS iban, ba.account_code
    FROM bank_transactions bt JOIN bank_accounts ba ON ba.id = bt.bank_account_id";

pub fn list_transactions(conn: &Connection, state: Option<&str>, iban: Option<&str>, limit: i64) -> Result<Vec<BankTxRow>> {
    let mut clauses: Vec<String> = Vec::new();
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    if let Some(s) = state {
        clauses.push("bt.state = ?".to_string());
        args.push(Box::new(s.to_string()));
    }
    if let Some(i) = iban {
        clauses.push("ba.iban = ?".to_string());
        args.push(Box::new(normalize_iban(i)));
    }
    let where_clause = if clauses.is_empty() { String::new() } else { format!("WHERE {}", clauses.join(" AND ")) };
    let sql = format!("{TX_SELECT} {where_clause} ORDER BY bt.date DESC, bt.id DESC LIMIT ?");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(args.iter().map(|a| a.as_ref()).chain(std::iter::once(&limit as &dyn rusqlite::ToSql))), tx_row_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn get_transaction(conn: &Connection, id: i64) -> Result<Option<BankTxRow>> {
    let sql = format!("{TX_SELECT} WHERE bt.id = ?1");
    let row = conn.query_row(&sql, [id], tx_row_from_row).optional()?;
    Ok(row)
}

pub fn set_transaction_state(conn: &Connection, id: i64, state: &str, actor: &str) -> Result<BankTxRow> {
    get_transaction(conn, id)?.ok_or_else(|| AppError::new("NOT_FOUND", format!("bank transaction {id} does not exist")))?;
    conn.execute("UPDATE bank_transactions SET state = ?1 WHERE id = ?2", params![state, id])?;
    crate::audit::record(
        conn,
        actor,
        &format!("bank.{state}"),
        Some("bank match"),
        Some(&serde_json::json!({ "id": id })),
        "ok",
        &[],
    )?;
    get_transaction(conn, id)?.ok_or_else(|| AppError::new("NOT_FOUND", format!("bank transaction {id} does not exist")))
}

/// Link a transaction to an existing posted entry.
pub fn link_transaction(
    conn: &Connection,
    tx_id: i64,
    entry_id: i64,
    method: &str,
    confidence: Option<f64>,
    actor: &str,
) -> Result<BankTxRow> {
    let tx_row = get_transaction(conn, tx_id)?.ok_or_else(|| AppError::new("NOT_FOUND", format!("bank transaction {tx_id} does not exist")))?;
    if tx_row.state != "unmatched" {
        return Err(AppError::new("ALREADY_MATCHED", format!("bank transaction {tx_id} is already {}", tx_row.state)));
    }
    let entry = get_entry(conn, entry_id)?.ok_or_else(|| AppError::new("NOT_FOUND", format!("entry {entry_id} does not exist")))?;
    if entry.meta.state != "posted" {
        return Err(AppError::new("NOT_POSTED", format!("entry {entry_id} must be posted before linking")));
    }
    conn.execute(
        "INSERT INTO reconciliations (bank_tx_id, target_type, target_id, method, confidence, created_by)
         VALUES (?1, 'entry', ?2, ?3, ?4, ?5)",
        params![tx_id, entry_id, method, confidence, actor],
    )?;
    conn.execute("UPDATE bank_transactions SET state = 'matched' WHERE id = ?1", [tx_id])?;
    crate::audit::record(
        conn,
        actor,
        "bank.link",
        Some("bank match --link"),
        Some(&serde_json::json!({ "txId": tx_id, "entryId": entry_id, "method": method, "confidence": confidence })),
        "ok",
        &[entry_id],
    )?;
    get_transaction(conn, tx_id)?.ok_or_else(|| AppError::new("NOT_FOUND", format!("bank transaction {tx_id} does not exist")))
}

/// Post a new entry from an unmatched transaction (bank leg + counter leg).
pub fn post_from_transaction(
    conn: &Connection,
    tx_id: i64,
    account_code: &str,
    actor: &str,
    post: bool,
) -> Result<(BankTxRow, crate::core::entries::Entry)> {
    let tx_row = get_transaction(conn, tx_id)?.ok_or_else(|| AppError::new("NOT_FOUND", format!("bank transaction {tx_id} does not exist")))?;
    if tx_row.state != "unmatched" {
        return Err(AppError::new("ALREADY_MATCHED", format!("bank transaction {tx_id} is already {}", tx_row.state)));
    }
    if get_account_by_code(conn, account_code)?.is_none() {
        return Err(AppError::new("ACCOUNT_NOT_FOUND", format!("account {account_code} does not exist")));
    }

    let description = tx_row
        .description
        .clone()
        .or_else(|| tx_row.counterparty.clone())
        .unwrap_or_else(|| format!("Banktransactie {tx_id}"));
    let postings = vec![
        PostingSpec {
            code: tx_row.account_code.clone(),
            amount_cents: tx_row.amount_cents,
            vat_code: None,
            vat_amount_cents: None,
            fx_currency: None,
            fx_amount_cents: None,
        },
        PostingSpec {
            code: account_code.to_string(),
            amount_cents: -tx_row.amount_cents,
            vat_code: None,
            vat_amount_cents: None,
            fx_currency: None,
            fx_amount_cents: None,
        },
    ];
    let entry = create_entry(conn, &tx_row.date, &description, &postings, "bank", Some(&format!("tx:{tx_id}")), actor)?;
    let posted = if post { post_entry(conn, entry.meta.id, actor)? } else { entry };
    let method = if actor.starts_with("agent") { "agent" } else { "manual" };
    conn.execute(
        "INSERT INTO reconciliations (bank_tx_id, target_type, target_id, method, confidence, created_by)
         VALUES (?1, 'entry', ?2, ?3, 1.0, ?4)",
        params![tx_id, posted.meta.id, method, actor],
    )?;
    conn.execute("UPDATE bank_transactions SET state = 'matched' WHERE id = ?1", [tx_id])?;
    crate::audit::record(
        conn,
        actor,
        "bank.post",
        Some("bank match --post"),
        Some(&serde_json::json!({ "txId": tx_id, "accountCode": account_code })),
        "ok",
        &[posted.meta.id],
    )?;
    Ok((get_transaction(conn, tx_id)?.unwrap(), posted))
}

/// A suggested or auto-matched candidate.
#[derive(Debug, Clone)]
pub struct MatchResult {
    pub kind: String,
    pub tx_id: i64,
    pub tx_date: String,
    pub amount_cents: i64,
    pub description: Option<String>,
    pub counterparty: Option<String>,
    pub entry_id: Option<i64>,
    pub entry_date: Option<String>,
    pub day_diff: Option<f64>,
    pub method: String,
    pub confidence: f64,
    pub invoice_id: Option<i64>,
    pub invoice_number: Option<String>,
    pub contact_name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AutoMatchResult {
    pub matched: Vec<MatchResult>,
    pub unmatched_remaining: i64,
}

/// Auto-match unmatched transactions to posted entries on the linked ledger
/// account. Best candidate = smallest |date difference| within windowDays.
/// method: 'exact' (<= 2 days) or 'fuzzy' (<= windowDays).
/// dry_run returns the would-be matches without writing.
pub fn auto_match(conn: &mut Connection, window_days: i64, actor: &str, dry_run: bool) -> Result<AutoMatchResult> {
    let unmatched: Vec<BankTxRow> = {
        let mut stmt = conn.prepare(
            "SELECT bt.id, bt.bank_account_id, bt.date, bt.amount_cents, bt.counterparty,
                    bt.description, bt.iban_counter, bt.hash, bt.state, ba.iban AS iban, ba.account_code
             FROM bank_transactions bt JOIN bank_accounts ba ON ba.id = bt.bank_account_id
             WHERE bt.state = 'unmatched'
             ORDER BY bt.date, bt.id",
        )?;
        let x = stmt
            .query_map([], tx_row_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        x
    };

    let mut matches: Vec<MatchResult> = Vec::new();
    for tx_row in &unmatched {
        // 1) exact/fuzzy match against already-booked entries on the bank account
        let best: Option<(i64, String, f64)> = conn
            .query_row(
                "SELECT e.id, e.date, ABS(julianday(e.date) - julianday(?1)) AS day_diff
                 FROM postings p
                 JOIN journal_entries e ON e.id = p.entry_id AND e.state = 'posted'
                 JOIN accounts a ON a.id = p.account_id
                 WHERE a.code = ?2 AND p.amount_cents = ?3
                   AND e.id NOT IN (SELECT target_id FROM reconciliations WHERE target_type = 'entry')
                 ORDER BY day_diff
                 LIMIT 1",
                params![tx_row.date, tx_row.account_code, tx_row.amount_cents],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;

        if let Some((entry_id, entry_date, day_diff)) = best {
            if day_diff <= window_days as f64 {
                matches.push(MatchResult {
                    kind: "entry".into(),
                    tx_id: tx_row.id,
                    tx_date: tx_row.date.clone(),
                    amount_cents: tx_row.amount_cents,
                    description: tx_row.description.clone(),
                    counterparty: tx_row.counterparty.clone(),
                    entry_id: Some(entry_id),
                    entry_date: Some(entry_date),
                    day_diff: Some(day_diff),
                    method: if day_diff <= 2.0 { "exact".into() } else { "fuzzy".into() },
                    confidence: if day_diff <= 2.0 { 0.99 } else { 0.8 },
                    invoice_id: None,
                    invoice_number: None,
                    contact_name: None,
                });
                continue;
            }
        }

        // 2) incoming money -> unpaid sales invoice with a matching outstanding amount
        if tx_row.amount_cents > 0 {
            let inv: Option<(i64, String, Option<String>)> = conn
                .query_row(
                    "SELECT i.id, i.invoice_number, c.name AS contact_name
                     FROM invoices i
                     LEFT JOIN contacts c ON c.id = i.contact_id
                     WHERE i.invoice_type = 'sales' AND i.status IN ('sent','overdue')
                       AND (SELECT COALESCE(SUM(l.amount_cents + l.vat_amount_cents), 0) FROM invoice_lines l WHERE l.invoice_id = i.id)
                         - (SELECT COALESCE(SUM(p.amount_cents), 0) FROM invoice_payments p WHERE p.invoice_id = i.id)
                         = ?1
                     ORDER BY i.due_date IS NULL, i.due_date, i.id
                     LIMIT 1",
                    [tx_row.amount_cents],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()?;
            if let Some((inv_id, inv_number, contact_name)) = inv {
                matches.push(MatchResult {
                    kind: "invoice".into(),
                    tx_id: tx_row.id,
                    tx_date: tx_row.date.clone(),
                    amount_cents: tx_row.amount_cents,
                    description: tx_row.description.clone(),
                    counterparty: tx_row.counterparty.clone(),
                    entry_id: None,
                    entry_date: None,
                    day_diff: None,
                    method: "invoice".into(),
                    confidence: 0.95,
                    invoice_id: Some(inv_id),
                    invoice_number: Some(inv_number),
                    contact_name,
                });
            }
        }
    }

    if !dry_run {
        let tx = conn.transaction()?;
        for m in &matches {
            if m.kind == "invoice" {
                invoice::payment_from_bank(&tx, m.invoice_id.unwrap(), m.tx_id, actor)?;
            } else {
                tx.execute(
                    "INSERT INTO reconciliations (bank_tx_id, target_type, target_id, method, confidence, created_by)
                     VALUES (?1, 'entry', ?2, ?3, ?4, ?5)",
                    params![m.tx_id, m.entry_id.unwrap(), m.method, m.confidence, actor],
                )?;
                tx.execute("UPDATE bank_transactions SET state = 'matched' WHERE id = ?1", [m.tx_id])?;
            }
        }
        crate::audit::record(
            &tx,
            actor,
            "bank.auto_match",
            Some("bank match --auto"),
            Some(&serde_json::json!({ "windowDays": window_days, "matched": matches.len() })),
            "ok",
            &matches.iter().filter_map(|m| m.entry_id).collect::<Vec<_>>(),
        )?;
        tx.commit()?;
    }

    let remaining = unmatched.len() as i64 - matches.len() as i64;
    Ok(AutoMatchResult {
        matched: matches,
        unmatched_remaining: remaining,
    })
}

/// Suggest a posting for each unmatched transaction (expense -> 4300, income -> 8000).
pub fn suggest_unmatched(conn: &Connection) -> Result<Vec<(BankTxRow, String)>> {
    let mut stmt = conn.prepare(&format!(
        "{TX_SELECT} WHERE bt.state = 'unmatched' ORDER BY bt.date, bt.id"
    ))?;
    let rows: Vec<BankTxRow> = stmt
        .query_map([], tx_row_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows
        .into_iter()
        .map(|t| {
            let suggested = if t.amount_cents > 0 { "8000" } else { "4300" };
            (t, suggested.to_string())
        })
        .collect())
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
            "INSERT INTO company (name, kvk, legal_form, vat_module, kor_flag) VALUES ('Test BV','00000000','bv',0,0)",
            [],
        )
        .unwrap();
        conn
    }

    fn tx(date: &str, amount: i64, counterparty: &str, desc: &str) -> BankTx {
        BankTx {
            date: date.to_string(),
            amount_cents: amount,
            counterparty: Some(counterparty.to_string()),
            description: Some(desc.to_string()),
            iban_counter: None,
            iban: Some("NL91ABNA0417164300".to_string()),
        }
    }

    #[test]
    fn import_is_idempotent() {
        let mut conn = seeded();
        let txs = vec![tx("2026-07-01", 10000, "Klant", "Betaald")];
        let r1 = import_transactions(&mut conn, "NL91ABNA0417164300", &txs, None, "1100", "human").unwrap();
        assert_eq!(r1.imported, 1);
        let r2 = import_transactions(&mut conn, "NL91ABNA0417164300", &txs, None, "1100", "human").unwrap();
        assert_eq!(r2.imported, 0);
        assert_eq!(r2.duplicates, 1);
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM bank_transactions", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn preview_counts_new_and_dup() {
        let mut conn = seeded();
        let txs = vec![tx("2026-07-01", 10000, "Klant", "Betaald"), tx("2026-07-02", -5000, "Winkel", "Inkoop")];
        import_transactions(&mut conn, "NL91ABNA0417164300", &txs, None, "1100", "human").unwrap();
        let p = preview_import(&conn, "NL91ABNA0417164300", &txs).unwrap();
        assert_eq!(p.imported, 0);
        assert_eq!(p.duplicates, 2);
        let p2 = preview_import(&conn, "NL91ABNA0417164300", &[tx("2026-07-03", 1, "X", "Y")]).unwrap();
        assert_eq!(p2.imported, 1);
    }

    #[test]
    fn post_from_transaction_creates_and_reconciles() {
        let conn = seeded();
        conn.execute(
            "INSERT INTO bank_accounts (iban, name, account_code) VALUES ('NL91ABNA0417164300', 'Bank', '1100')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO bank_transactions (bank_account_id, date, amount_cents, counterparty, description, hash)
             VALUES (1, '2026-07-05', -4500, 'Albert Heijn', 'Boodschappen', 'hash1')",
            [],
        )
        .unwrap();
        let (_, entry) = post_from_transaction(&conn, 1, "4300", "human", true).unwrap();
        assert_eq!(entry.meta.state, "posted");
        assert_eq!(entry.postings.len(), 2);
        let state: String = conn.query_row("SELECT state FROM bank_transactions WHERE id = 1", [], |r| r.get(0)).unwrap();
        assert_eq!(state, "matched");
    }

    #[test]
    fn link_requires_posted_entry() {
        let conn = seeded();
        conn.execute(
            "INSERT INTO bank_accounts (iban, name, account_code) VALUES ('NL91ABNA0417164300', 'Bank', '1100')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO bank_transactions (bank_account_id, date, amount_cents, hash)
             VALUES (1, '2026-07-05', 1000, 'hash2')",
            [],
        )
        .unwrap();
        // draft entry -> link must fail with NOT_POSTED
        let specs = vec![
            PostingSpec { code: "1100".into(), amount_cents: 1000, vat_code: None, vat_amount_cents: None, fx_currency: None, fx_amount_cents: None },
            PostingSpec { code: "8000".into(), amount_cents: -1000, vat_code: None, vat_amount_cents: None, fx_currency: None, fx_amount_cents: None },
        ];
        let entry = create_entry(&conn, "2026-07-05", "test", &specs, "manual", None, "human").unwrap();
        let err = link_transaction(&conn, 1, entry.meta.id, "manual", None, "human").unwrap_err();
        assert_eq!(err.code(), "NOT_POSTED");
        // after posting -> works
        post_entry(&conn, entry.meta.id, "human").unwrap();
        let tx_row = link_transaction(&conn, 1, entry.meta.id, "manual", None, "human").unwrap();
        assert_eq!(tx_row.state, "matched");
    }

    #[test]
    fn auto_match_links_same_amount_entry() {
        let mut conn = seeded();
        conn.execute(
            "INSERT INTO bank_accounts (iban, name, account_code) VALUES ('NL91ABNA0417164300', 'Bank', '1100')",
            [],
        )
        .unwrap();
        // posted entry: 1100 +50000 / 8000 -50000
        let specs = vec![
            PostingSpec { code: "1100".into(), amount_cents: 50000, vat_code: None, vat_amount_cents: None, fx_currency: None, fx_amount_cents: None },
            PostingSpec { code: "8000".into(), amount_cents: -50000, vat_code: None, vat_amount_cents: None, fx_currency: None, fx_amount_cents: None },
        ];
        let entry = create_entry(&conn, "2026-07-10", "Verkoop", &specs, "manual", None, "human").unwrap();
        post_entry(&conn, entry.meta.id, "human").unwrap();
        // bank tx with same amount, 1 day later
        conn.execute(
            "INSERT INTO bank_transactions (bank_account_id, date, amount_cents, hash)
             VALUES (1, '2026-07-11', 50000, 'hash3')",
            [],
        )
        .unwrap();
        let result = auto_match(&mut conn, 5, "human", true).unwrap();
        assert_eq!(result.matched.len(), 1);
        assert_eq!(result.matched[0].kind, "entry");
        assert_eq!(result.matched[0].method, "exact");
        assert_eq!(result.matched[0].entry_id, Some(entry.meta.id));
    }
}
