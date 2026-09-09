// bukio-cli — agent-first double-entry bookkeeping for SMEs.
// Copyright (c) 2026 Erik van Kempen.
// SPDX-License-Identifier: Apache-2.0
//
// Bank module — accounts, import, matching, reconciliation (mirrors src/bank/).

use crate::accounts::get_account_by_code;
use crate::actor::now_iso;
use crate::audit::{record, RecordArgs};
use crate::dates::validate_date;
use crate::entries::{create_entry, get_entry, post_entry, CreateEntry, PostingSpec};
use crate::money::{format_amount, BukioError, Result};
use rusqlite::Connection;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

// CAMT/CSV parsers defined in camt.rs and csv_bank.rs
mod camt;
mod csv_bank;

pub use camt::parse_camt053;
pub use csv_bank::parse_bank_csv;

fn bank_error(code: &'static str, msg: impl Into<String>) -> BukioError {
    BukioError::new(code, msg.into())
}

fn sha256_hex(input: &str) -> String {
    let hash = Sha256::digest(input.as_bytes());
    hex::encode(hash)
}

/// Normalize IBAN: strip spaces/dashes, uppercase.
pub fn normalize_iban(iban: &str) -> String {
    iban.trim()
        .to_uppercase()
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .collect()
}

/// SHA256 hash of a transaction for idempotent import.
pub fn tx_hash(iban: &str, tx: &BankTx) -> String {
    let raw = format!(
        "{}|{}|{}|{}|{}|{}",
        iban,
        tx.date,
        tx.amount_cents,
        tx.counterparty.as_deref().unwrap_or(""),
        tx.description.as_deref().unwrap_or(""),
        tx.bank_ref.as_deref().unwrap_or("")
    );
    sha256_hex(&raw)
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct BankTx {
    pub date: String,
    pub amount_cents: i64,
    pub counterparty: Option<String>,
    pub description: Option<String>,
    pub iban_counter: Option<String>,
    pub bank_ref: Option<String>,
}

#[derive(Debug, Clone)]
pub struct BankAccount {
    pub id: i64,
    pub iban: String,
    pub name: Option<String>,
    pub account_code: String,
}

impl BankAccount {
    fn from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: r.get(0)?,
            iban: r.get(1)?,
            name: r.get(2)?,
            account_code: r.get(3)?,
        })
    }
}

/// Get or create a bank account (idempotent).
pub fn get_or_create_bank_account(
    db: &Connection,
    iban: &str,
    name: Option<&str>,
    account_code: &str,
    dry_run: bool,
) -> Result<Value> {
    let iban_norm = normalize_iban(iban);
    if !crate::dates::is_valid_iban(&iban_norm) {
        return Err(bank_error(
            "INVALID_IBAN",
            format!("'{iban_norm}' is not a valid IBAN"),
        ));
    }
    if get_account_by_code(db, account_code).is_none() {
        return Err(bank_error(
            "ACCOUNT_NOT_FOUND",
            format!("ledger account {account_code} does not exist"),
        ));
    }
    // check existing
    let existing: Option<(i64, String, Option<String>, String)> = db
        .query_row(
            "SELECT id, iban, name, account_code FROM bank_accounts WHERE iban = ?1",
            [&iban_norm],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .ok();
    if let Some((id, ib, nm, ac)) = existing {
        return Ok(json!({ "id": id, "iban": ib, "name": nm, "account_code": ac }));
    }
    if dry_run {
        return Ok(json!({
            "action": "bank.add", "iban": iban_norm, "name": name,
            "account_code": account_code, "would_create": true, "dryRun": true,
        }));
    }
    db.execute(
        "INSERT INTO bank_accounts (iban, name, account_code) VALUES (?1, ?2, ?3)",
        rusqlite::params![iban_norm, name, account_code],
    )
    .map_err(sql_err)?;
    let id = db.last_insert_rowid();
    let row = db
        .query_row(
            "SELECT id, iban, name, account_code FROM bank_accounts WHERE id = ?1",
            [id],
            BankAccount::from_row,
        )
        .map_err(sql_err)?;
    Ok(
        json!({ "id": row.id, "iban": row.iban, "name": row.name, "account_code": row.account_code }),
    )
}

/// List all bank accounts with transaction stats.
pub fn list_bank_accounts(db: &Connection) -> Result<Vec<Value>> {
    let mut stmt = db
        .prepare(
            "SELECT ba.id, ba.iban, ba.name, ba.account_code,
                    COUNT(bt.id) AS transaction_count,
                    COALESCE(SUM(CASE WHEN bt.state = 'unmatched' THEN 1 ELSE 0 END), 0) AS unmatched_count,
                    COALESCE(SUM(bt.amount_cents), 0) AS balance_cents
             FROM bank_accounts ba
             LEFT JOIN bank_transactions bt ON bt.bank_account_id = ba.id
             GROUP BY ba.id
             ORDER BY ba.iban",
        )
        .map_err(sql_err)?;
    let rows = stmt
        .query_map([], |r| {
            let balance = r.get::<_, i64>(6)?;
            Ok(json!({
                "iban": r.get::<_, String>(1)?,
                "name": r.get::<_, Option<String>>(2)?,
                "account_code": r.get::<_, String>(3)?,
                "transaction_count": r.get::<_, i64>(4)?,
                "unmatched_count": r.get::<_, i64>(5)?,
                "balance_cents": balance,
                "balance": format_amount(balance),
            }))
        })
        .map_err(sql_err)?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

/// Preview import: count new vs duplicate hashes.
pub fn preview_import(db: &Connection, iban: &str, transactions: &[BankTx]) -> Result<Value> {
    let iban_norm = normalize_iban(iban);
    if !crate::dates::is_valid_iban(&iban_norm) {
        return Err(bank_error(
            "INVALID_IBAN",
            format!("'{iban_norm}' is not a valid IBAN"),
        ));
    }
    let mut imported = 0i64;
    let mut duplicates = 0i64;
    for tx in transactions {
        let hash = tx_hash(&iban_norm, tx);
        let exists: bool = db
            .query_row(
                "SELECT 1 FROM bank_transactions WHERE hash = ?1",
                [&hash],
                |_| Ok(true),
            )
            .unwrap_or(false);
        if exists {
            duplicates += 1;
        } else {
            imported += 1;
        }
    }
    Ok(json!({
        "iban": iban_norm, "imported": imported, "duplicates": duplicates,
        "total": transactions.len() as i64,
    }))
}

/// Import transactions (idempotent — only new hashes are inserted).
pub fn import_transactions(
    db: &Connection,
    iban: &str,
    transactions: &[BankTx],
    name: Option<&str>,
    account_code: &str,
    actor: &str,
) -> Result<Value> {
    if transactions.is_empty() {
        return Err(bank_error("EMPTY_STATEMENT", "no transactions to import"));
    }
    // validate all dates and amounts
    for tx in transactions {
        validate_date(&tx.date)?;
        // amount must be integer (it is in our struct, but check for sanity)
    }
    let acct = get_or_create_bank_account(db, iban, name, account_code, false)?;
    let bank_account_id = acct["id"].as_i64().unwrap();
    let iban_norm = acct["iban"].as_str().unwrap_or(iban);

    let mut imported = 0i64;
    let mut duplicates = 0i64;
    let tx_ref = db.unchecked_transaction().map_err(sql_err)?;
    {
        let mut stmt = tx_ref
            .prepare(
                "INSERT INTO bank_transactions (bank_account_id, date, amount_cents, counterparty, description, iban_counter, hash)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )
            .map_err(sql_err)?;
        let mut check = tx_ref
            .prepare("SELECT 1 FROM bank_transactions WHERE hash = ?1")
            .map_err(sql_err)?;
        for tx in transactions {
            let hash = tx_hash(iban_norm, tx);
            let exists: bool = check.query_row([&hash], |_| Ok(true)).unwrap_or(false);
            if exists {
                duplicates += 1;
                continue;
            }
            stmt.execute(rusqlite::params![
                bank_account_id,
                tx.date,
                tx.amount_cents,
                tx.counterparty,
                tx.description,
                tx.iban_counter,
                hash,
            ])
            .map_err(sql_err)?;
            imported += 1;
        }
        record(
            &tx_ref,
            RecordArgs {
                actor,
                action: "bank.import",
                command: Some("bank import"),
                args: Some(
                    json!({ "iban": iban_norm, "transactions": transactions.len(), "imported": imported, "duplicates": duplicates }),
                ),
                outcome: "ok",
                entry_ids: vec![],
            },
        )?;
    }
    tx_ref.commit().map_err(sql_err)?;
    Ok(json!({
        "iban": iban_norm, "imported": imported, "duplicates": duplicates,
        "total": transactions.len() as i64,
    }))
}

/// List bank transactions with optional filters.
pub fn list_transactions(
    db: &Connection,
    state: Option<&str>,
    iban: Option<&str>,
    limit: i64,
) -> Result<Vec<Value>> {
    let mut where_clauses = Vec::new();
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    if let Some(s) = state {
        where_clauses.push("bt.state = ?".to_string());
        params.push(Box::new(s.to_string()));
    }
    if let Some(i) = iban {
        where_clauses.push("ba.iban = ?".to_string());
        params.push(Box::new(normalize_iban(i)));
    }
    let where_sql = if where_clauses.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", where_clauses.join(" AND "))
    };
    let sql = format!(
        "SELECT bt.id, bt.date, bt.amount_cents, bt.counterparty, bt.description,
                bt.iban_counter, bt.hash, bt.state, bt.bank_account_id,
                ba.iban AS iban, ba.account_code
         FROM bank_transactions bt
         JOIN bank_accounts ba ON ba.id = bt.bank_account_id
         {where_sql}
         ORDER BY bt.date DESC, bt.id DESC
         LIMIT ?"
    );
    params.push(Box::new(limit));
    let mut stmt = db.prepare(&sql).map_err(sql_err)?;
    let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
    let rows = stmt
        .query_map(param_refs.as_slice(), |r| Ok(tx_to_json(r)?))
        .map_err(sql_err)?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

/// Get a single transaction.
pub fn get_transaction(db: &Connection, id: i64) -> Result<Option<Value>> {
    let sql = "SELECT bt.id, bt.date, bt.amount_cents, bt.counterparty, bt.description,
                      bt.iban_counter, bt.hash, bt.state, bt.bank_account_id,
                      ba.iban AS iban, ba.account_code
               FROM bank_transactions bt
               JOIN bank_accounts ba ON ba.id = bt.bank_account_id
               WHERE bt.id = ?1";
    let result = db.query_row(sql, [id], |r| tx_to_json(r));
    match result {
        Ok(v) => Ok(Some(v)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(sql_err(e)),
    }
}

fn tx_to_json(r: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    let ac = r.get::<_, i64>(2)?;
    Ok(json!({
        "id": r.get::<_, i64>(0)?,
        "date": r.get::<_, String>(1)?,
        "amount_cents": ac,
        "amount": format_amount(ac),
        "counterparty": r.get::<_, Option<String>>(3)?,
        "description": r.get::<_, Option<String>>(4)?,
        "iban_counter": r.get::<_, Option<String>>(5)?,
        "hash": r.get::<_, String>(6)?,
        "state": r.get::<_, String>(7)?,
        "bank_account_id": r.get::<_, i64>(8)?,
        "iban": r.get::<_, String>(9)?,
        "account_code": r.get::<_, String>(10)?,
    }))
}

const VALID_TX_STATES: &[&str] = &["unmatched", "matched", "ignored"];

/// Change a transaction's state.
pub fn set_transaction_state(
    db: &Connection,
    id: i64,
    state: &str,
    actor: &str,
    dry_run: bool,
) -> Result<Value> {
    if !VALID_TX_STATES.contains(&state) {
        return Err(bank_error(
            "INVALID_STATE",
            format!(
                "transaction state '{state}' must be one of {}",
                VALID_TX_STATES.join(", ")
            ),
        ));
    }
    let tx_row = get_transaction(db, id)?
        .ok_or_else(|| bank_error("NOT_FOUND", format!("bank transaction {id} does not exist")))?;
    if dry_run {
        return Ok(json!({
            "action": format!("bank.{state}"), "id": id,
            "from": tx_row["state"], "to": state, "dryRun": true,
        }));
    }
    db.execute(
        "UPDATE bank_transactions SET state = ?1 WHERE id = ?2",
        rusqlite::params![state, id],
    )
    .map_err(sql_err)?;
    record(
        db,
        RecordArgs {
            actor,
            action: &format!("bank.{state}"),
            command: Some("bank match"),
            args: Some(json!({ "id": id })),
            outcome: "ok",
            entry_ids: vec![],
        },
    )?;
    get_transaction(db, id)?.ok_or_else(|| bank_error("NOT_FOUND", "transaction disappeared"))
}

/// Link a transaction to an existing posted entry.
pub fn link_transaction(
    db: &Connection,
    tx_id: i64,
    entry_id: i64,
    method: &str,
    confidence: Option<f64>,
    actor: &str,
    dry_run: bool,
) -> Result<Value> {
    let tx_row = get_transaction(db, tx_id)?.ok_or_else(|| {
        bank_error(
            "NOT_FOUND",
            format!("bank transaction {tx_id} does not exist"),
        )
    })?;
    if tx_row["state"] != "unmatched" {
        return Err(bank_error(
            "ALREADY_MATCHED",
            format!("bank transaction {tx_id} is already {}", tx_row["state"]),
        ));
    }
    let entry_opt = get_entry(db, entry_id);
    let entry = entry_opt
        .ok_or_else(|| bank_error("NOT_FOUND", format!("entry {entry_id} does not exist")))?;
    if entry.state != "posted" {
        return Err(bank_error(
            "NOT_POSTED",
            format!("entry {entry_id} must be posted before linking"),
        ));
    }
    if dry_run {
        return Ok(json!({
            "action": "bank.link", "tx_id": tx_id, "entry_id": entry_id,
            "method": method, "confidence": confidence,
            "entry_date": entry.date, "amount_cents": tx_row["amount_cents"], "dryRun": true,
        }));
    }
    let tx_ref = db.unchecked_transaction().map_err(sql_err)?;
    {
        tx_ref.execute(
            "INSERT INTO reconciliations (bank_tx_id, target_type, target_id, method, confidence, created_by)
             VALUES (?1, 'entry', ?2, ?3, ?4, ?5)",
            rusqlite::params![tx_id, entry_id, method, confidence, actor],
        )
        .map_err(sql_err)?;
        tx_ref
            .execute(
                "UPDATE bank_transactions SET state = 'matched' WHERE id = ?1",
                [tx_id],
            )
            .map_err(sql_err)?;
        record(
            &tx_ref,
            RecordArgs {
                actor,
                action: "bank.link",
                command: Some("bank match --link"),
                args: Some(
                    json!({ "txId": tx_id, "entryId": entry_id, "method": method, "confidence": confidence }),
                ),
                outcome: "ok",
                entry_ids: vec![entry_id],
            },
        )?;
    }
    tx_ref.commit().map_err(sql_err)?;
    get_transaction(db, tx_id)?.ok_or_else(|| bank_error("NOT_FOUND", "transaction disappeared"))
}

/// Post a new entry from an unmatched transaction.
pub fn post_from_transaction(
    db: &Connection,
    tx_id: i64,
    account_code: &str,
    actor: &str,
    do_post: bool,
) -> Result<(Value, Value)> {
    let tx_row = get_transaction(db, tx_id)?.ok_or_else(|| {
        bank_error(
            "NOT_FOUND",
            format!("bank transaction {tx_id} does not exist"),
        )
    })?;
    if tx_row["state"] != "unmatched" {
        return Err(bank_error(
            "ALREADY_MATCHED",
            format!("bank transaction {tx_id} is already {}", tx_row["state"]),
        ));
    }
    if get_account_by_code(db, account_code).is_none() {
        return Err(bank_error(
            "ACCOUNT_NOT_FOUND",
            format!("account {account_code} does not exist"),
        ));
    }
    let description_raw = tx_row["description"]
        .as_str()
        .or_else(|| tx_row["counterparty"].as_str());
    let fallback = format!("Banktransactie {tx_id}");
    let description = description_raw.unwrap_or(&fallback);
    let amount = tx_row["amount_cents"].as_i64().unwrap();
    let ledger_code = tx_row["account_code"].as_str().unwrap();
    let date = tx_row["date"].as_str().unwrap();
    let entry = create_entry(
        db,
        CreateEntry {
            date,
            description,
            postings: vec![
                PostingSpec {
                    code: ledger_code.to_string(),
                    amount_cents: amount,
                    cost_center_code: None,
                },
                PostingSpec {
                    code: account_code.to_string(),
                    amount_cents: -amount,
                    cost_center_code: None,
                },
            ],
            source: "bank",
            source_ref: Some(&format!("tx:{tx_id}")),
            actor,
        },
    )?;
    let posted = if do_post {
        post_entry(db, entry.id, actor)?
    } else {
        entry.clone()
    };
    let method = if actor.starts_with("agent") {
        "agent"
    } else {
        "manual"
    };
    let tx_ref = db.unchecked_transaction().map_err(sql_err)?;
    {
        tx_ref.execute(
            "INSERT INTO reconciliations (bank_tx_id, target_type, target_id, method, confidence, created_by)
             VALUES (?1, 'entry', ?2, ?3, 1.0, ?4)",
            rusqlite::params![tx_id, posted.id, method, actor],
        )
        .map_err(sql_err)?;
        tx_ref
            .execute(
                "UPDATE bank_transactions SET state = 'matched' WHERE id = ?1",
                [tx_id],
            )
            .map_err(sql_err)?;
        record(
            &tx_ref,
            RecordArgs {
                actor,
                action: "bank.post",
                command: Some("bank match --post"),
                args: Some(json!({ "txId": tx_id, "accountCode": account_code })),
                outcome: "ok",
                entry_ids: vec![posted.id],
            },
        )?;
    }
    tx_ref.commit().map_err(sql_err)?;
    let tx_json = get_transaction(db, tx_id)?
        .ok_or_else(|| bank_error("NOT_FOUND", "transaction disappeared"))?;
    let posted_json = crate::entries::entry_to_json(&posted);
    Ok((tx_json, posted_json))
}

/// Suggest accounts for unmatched transactions (simple heuristic).
pub fn suggest_unmatched(db: &Connection) -> Result<Vec<Value>> {
    let sql = "SELECT bt.id, bt.date, bt.amount_cents, bt.counterparty, bt.description,
                      bt.iban_counter, bt.hash, bt.state, bt.bank_account_id,
                      ba.iban AS iban, ba.account_code
               FROM bank_transactions bt
               JOIN bank_accounts ba ON ba.id = bt.bank_account_id
               WHERE bt.state = 'unmatched'
               ORDER BY bt.date, bt.id";
    let mut stmt = db.prepare(sql).map_err(sql_err)?;
    let rows = stmt
        .query_map([], |r| {
            let mut v = tx_to_json(r)?;
            let amount = v["amount_cents"].as_i64().unwrap_or(0);
            v["suggested_account"] = Value::String(if amount > 0 {
                "8000".into()
            } else {
                "4300".into()
            });
            Ok(v)
        })
        .map_err(sql_err)?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

/// Auto-match: match unmatched transactions to posted entries or invoices.
/// Stubs the invoice-matching path until the invoice module is ported.
pub fn auto_match(db: &Connection, window_days: i64, actor: &str, dry_run: bool) -> Result<Value> {
    if window_days < 0 {
        return Err(bank_error(
            "INVALID_WINDOW",
            format!("window-days must be non-negative, got {window_days}"),
        ));
    }
    let unmatched_sql = "SELECT bt.id, bt.date, bt.amount_cents, bt.counterparty, bt.description,
                        bt.iban_counter, bt.hash, bt.state, bt.bank_account_id,
                        ba.iban AS iban, ba.account_code
                 FROM bank_transactions bt
                 JOIN bank_accounts ba ON ba.id = bt.bank_account_id
                 WHERE bt.state = 'unmatched'
                 ORDER BY bt.date, bt.id";
    let mut stmt = db.prepare(unmatched_sql).map_err(sql_err)?;
    let unmatched_rows: Vec<Value> = stmt
        .query_map([], |r| tx_to_json(r))
        .map_err(sql_err)?
        .filter_map(|r| r.ok())
        .collect();
    let unmatched_count = unmatched_rows.len() as i64;
    let mut matches: Vec<Value> = Vec::new();
    let mut used_entry_ids: Vec<i64> = Vec::new();

    for tx_row in &unmatched_rows {
        let tx_id = tx_row["id"].as_i64().unwrap();
        let tx_date = tx_row["date"].as_str().unwrap();
        let amount = tx_row["amount_cents"].as_i64().unwrap();
        let account_code = tx_row["account_code"].as_str().unwrap();
        let bank_account_id = tx_row["bank_account_id"].as_i64().unwrap();

        // build exclusion clause for already-claimed entries
        let exclusion = if used_entry_ids.is_empty() {
            String::new()
        } else {
            let placeholders: Vec<String> =
                used_entry_ids.iter().map(|_| "?".to_string()).collect();
            format!("AND e.id NOT IN ({})", placeholders.join(","))
        };

        let candidate_sql = format!(
            "SELECT e.id, e.date,
                    ABS(julianday(e.date) - julianday(?1)) AS day_diff
             FROM postings p
             JOIN journal_entries e ON e.id = p.entry_id AND e.state = 'posted'
             JOIN accounts a ON a.id = p.account_id
             WHERE a.code = ?2 AND p.amount_cents = ?3
               AND e.id NOT IN (SELECT target_id FROM reconciliations WHERE target_type = 'entry')
               {exclusion}
               AND (
                 e.source != 'bank'
                 OR EXISTS (
                   SELECT 1 FROM bank_transactions bt2
                   WHERE bt2.id = CAST(SUBSTR(e.source_ref, 4) AS INTEGER)
                     AND bt2.bank_account_id = ?4
                 )
               )
             ORDER BY day_diff, e.id
             LIMIT 1"
        );

        // build params: tx_date, account_code, amount, [excluded_ids...], bank_account_id
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![
            Box::new(tx_date.to_string()),
            Box::new(account_code.to_string()),
            Box::new(amount),
        ];
        for eid in &used_entry_ids {
            params.push(Box::new(*eid));
        }
        params.push(Box::new(bank_account_id));

        let mut cand_stmt = db.prepare(&candidate_sql).map_err(sql_err)?;
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();
        let candidate: Option<(i64, String, f64)> = cand_stmt
            .query_row(param_refs.as_slice(), |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .ok();

        if let Some((entry_id, entry_date, day_diff)) = candidate {
            if day_diff <= window_days as f64 {
                let method = if day_diff <= 2.0 { "exact" } else { "fuzzy" };
                let confidence = if day_diff <= 2.0 { 0.99 } else { 0.8 };
                matches.push(json!({
                    "kind": "entry", "tx_id": tx_id, "tx_date": tx_date,
                    "amount_cents": amount, "description": tx_row["description"],
                    "counterparty": tx_row["counterparty"],
                    "entry_id": entry_id, "entry_date": entry_date,
                    "day_diff": day_diff, "method": method, "confidence": confidence,
                }));
                used_entry_ids.push(entry_id);
                continue;
            }
        }
        // ponytail: invoice matching stubbed — needs invoice module (getInvoice, paymentFromBank)
    }

    if !dry_run {
        let tx_ref = db.unchecked_transaction().map_err(sql_err)?;
        {
            for m in &matches {
                let entry_id = m["entry_id"].as_i64().unwrap();
                let tx_id = m["tx_id"].as_i64().unwrap();
                let method = m["method"].as_str().unwrap();
                let confidence = m["confidence"].as_f64();
                tx_ref.execute(
                    "INSERT INTO reconciliations (bank_tx_id, target_type, target_id, method, confidence, created_by)
                     VALUES (?1, 'entry', ?2, ?3, ?4, ?5)",
                    rusqlite::params![tx_id, entry_id, method, confidence, actor],
                )
                .map_err(sql_err)?;
                tx_ref
                    .execute(
                        "UPDATE bank_transactions SET state = 'matched' WHERE id = ?1",
                        [tx_id],
                    )
                    .map_err(sql_err)?;
            }
            record(
                &tx_ref,
                RecordArgs {
                    actor,
                    action: "bank.auto_match",
                    command: Some("bank match --auto"),
                    args: Some(json!({ "windowDays": window_days, "matched": matches.len() })),
                    outcome: "ok",
                    entry_ids: matches
                        .iter()
                        .filter_map(|m| m["entry_id"].as_i64())
                        .collect(),
                },
            )?;
        }
        tx_ref.commit().map_err(sql_err)?;
    }

    Ok(json!({
        "matched": matches,
        "unmatched_remaining": unmatched_count - matches.len() as i64,
    }))
}

fn sql_err(e: rusqlite::Error) -> BukioError {
    BukioError::new("DB_ERROR", e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_db;

    fn db() -> Connection {
        let db = open_db(":memory:").unwrap();
        db.execute("INSERT INTO company (name) VALUES ('Bank Co')", [])
            .unwrap();
        crate::accounts::seed_default_chart(&db).unwrap();
        db
    }

    #[test]
    fn normalize_iban_strips_and_uppercases() {
        assert_eq!(
            normalize_iban("nl91 abna 0417 1643 00"),
            "NL91ABNA0417164300"
        );
        assert_eq!(
            normalize_iban("NL91-ABNA-0417-1643-00"),
            "NL91ABNA0417164300"
        );
    }

    #[test]
    fn tx_hash_deterministic() {
        let tx = BankTx {
            date: "2026-01-15".into(),
            amount_cents: 10000,
            counterparty: Some("ACME".into()),
            description: Some("Invoice".into()),
            iban_counter: None,
            bank_ref: None,
        };
        let h1 = tx_hash("NL91ABNA0417164300", &tx);
        let h2 = tx_hash("NL91ABNA0417164300", &tx);
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64); // sha256 hex
    }

    #[test]
    fn get_or_create_bank_account_idempotent() {
        let d = db();
        let a1 = get_or_create_bank_account(&d, "NL91ABNA0417164300", Some("Main"), "1100", false)
            .unwrap();
        let a2 = get_or_create_bank_account(&d, "NL91ABNA0417164300", Some("Main"), "1100", false)
            .unwrap();
        assert_eq!(a1["id"], a2["id"]); // same record
    }

    #[test]
    fn import_and_list_transactions() {
        let d = db();
        let txs = vec![
            BankTx {
                date: "2026-01-15".into(),
                amount_cents: 5000,
                counterparty: Some("ACME".into()),
                description: None,
                iban_counter: None,
                bank_ref: None,
            },
            BankTx {
                date: "2026-01-16".into(),
                amount_cents: -2000,
                counterparty: None,
                description: Some("Payment".into()),
                iban_counter: None,
                bank_ref: None,
            },
        ];
        let r = import_transactions(&d, "NL91ABNA0417164300", &txs, None, "1100", "human:erik")
            .unwrap();
        assert_eq!(r["imported"], 2);
        // idempotent: second import = 0 new
        let r2 = import_transactions(&d, "NL91ABNA0417164300", &txs, None, "1100", "human:erik")
            .unwrap();
        assert_eq!(r2["imported"], 0);
        assert_eq!(r2["duplicates"], 2);
        // list
        let list = list_transactions(&d, None, None, 10).unwrap();
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn state_change_and_link() {
        let d = db();
        let txs = vec![BankTx {
            date: "2026-01-15".into(),
            amount_cents: 10000,
            counterparty: None,
            description: None,
            iban_counter: None,
            bank_ref: None,
        }];
        import_transactions(&d, "NL91ABNA0417164300", &txs, None, "1100", "human:erik").unwrap();
        let tx = get_transaction(&d, 1).unwrap().unwrap();
        assert_eq!(tx["state"], "unmatched");
        // ignore
        let ignored = set_transaction_state(&d, 1, "ignored", "human:erik", false).unwrap();
        assert_eq!(ignored["state"], "ignored");
        // unignore
        let reopened = set_transaction_state(&d, 1, "unmatched", "human:erik", false).unwrap();
        assert_eq!(reopened["state"], "unmatched");
    }

    #[test]
    fn post_from_transaction_creates_entry() {
        let d = db();
        let txs = vec![BankTx {
            date: "2026-01-15".into(),
            amount_cents: 5000,
            counterparty: Some("Shop".into()),
            description: Some("Koffie".into()),
            iban_counter: None,
            bank_ref: None,
        }];
        import_transactions(&d, "NL91ABNA0417164300", &txs, None, "1100", "human:erik").unwrap();
        let (tx, entry) = post_from_transaction(&d, 1, "4300", "human:erik", true).unwrap();
        assert_eq!(tx["state"], "matched");
        assert_eq!(entry["state"], "posted");
    }
}
