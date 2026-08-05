//! Posting engine — the heart of bukio-cli.
//! Lifecycle: draft -> posted -> reversed. Posted entries are never deleted.
//!
//! Faithful port of the Node `src/core/entries.js` — same validation order,
//! same error codes, same audit trail.

use crate::audit;
use crate::core::accounts::{get_account, get_account_by_code};
use crate::core::money::{format_amount, parse_amount};
use crate::error::{AppError, Result};
use chrono::{SecondsFormat, Utc};
use regex::Regex;
use rusqlite::{params, Connection, OptionalExtension};
use serde::ser::SerializeStruct;
use serde::{Serialize, Serializer};
use std::sync::OnceLock;

pub const VALID_SOURCES: [&str; 7] = ["manual", "bank", "invoice", "agent", "reversal", "recurring", "closing"];

fn date_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^\d{4}-\d{2}-\d{2}$").unwrap())
}

fn posting_spec_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^(\d{1,6}):(.+)$").unwrap())
}

fn fx_currency_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^[A-Z]{3}$").unwrap())
}

/// Same as JS `new Date().toISOString()`: UTC with millisecond precision + Z.
pub fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn validate_date(date: &str) -> Result<()> {
    if !date_re().is_match(date) {
        return Err(AppError::new("INVALID_DATE", format!("date '{date}' must be yyyy-mm-dd")));
    }
    if chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").is_err() {
        return Err(AppError::new("INVALID_DATE", format!("date '{date}' is not a valid calendar date")));
    }
    Ok(())
}

/// One parsed posting spec "CODE:AMOUNT" (optionally with VAT/FX fields set
/// by higher layers).
#[derive(Debug, Clone)]
pub struct PostingSpec {
    pub code: String,
    pub amount_cents: i64,
    pub vat_code: Option<String>,
    pub vat_amount_cents: Option<i64>,
    pub fx_currency: Option<String>,
    pub fx_amount_cents: Option<i64>,
}

impl PostingSpec {
    pub fn plain(code: String, amount_cents: i64) -> Self {
        PostingSpec { code, amount_cents, vat_code: None, vat_amount_cents: None, fx_currency: None, fx_amount_cents: None }
    }
}

/// Parse posting specs "CODE:AMOUNT" (repeatable, comma-splittable) into
/// specs. Pure — no DB access.
pub fn parse_posting_specs(raw: &[String]) -> Result<Vec<PostingSpec>> {
    let mut specs: Vec<PostingSpec> = Vec::new();
    for item in raw {
        for token in item.split(',') {
            let t = token.trim();
            if t.is_empty() {
                continue;
            }
            let caps = posting_spec_re()
                .captures(t)
                .ok_or_else(|| AppError::new("INVALID_POSTING", format!("posting '{t}' must be CODE:AMOUNT (e.g. 1100:1234.56)")))?;
            let code = caps.get(1).unwrap().as_str().to_string();
            let amount_cents = parse_amount(caps.get(2).unwrap().as_str())?;
            specs.push(PostingSpec::plain(code, amount_cents));
        }
    }
    Ok(specs)
}

/// A posting resolved against the DB: account verified, VAT code id resolved,
/// FX fields validated.
#[derive(Debug, Clone)]
pub struct ResolvedPosting {
    pub account_id: i64,
    pub code: String,
    pub amount_cents: i64,
    pub vat_code_id: Option<i64>,
    pub vat_amount_cents: Option<i64>,
    pub fx_currency: Option<String>,
    pub fx_amount_cents: Option<i64>,
}

/// Resolve posting specs against the DB: verifies accounts exist & are active,
/// amounts are non-zero integers, and returns resolved postings. Optional VAT
/// fields are persisted as-is (nullable, inert when the VAT module is off);
/// if a vatCode is given it must exist in vat_codes. Pure validation — no writes.
pub fn resolve_postings(conn: &Connection, postings: &[PostingSpec]) -> Result<Vec<ResolvedPosting>> {
    let mut out = Vec::with_capacity(postings.len());
    for p in postings {
        if p.amount_cents == 0 {
            return Err(AppError::new("INVALID_AMOUNT_CENTS", "posting amounts must be non-zero integers (cents)"));
        }
        let account = get_account_by_code(conn, &p.code)?
            .ok_or_else(|| AppError::new("ACCOUNT_NOT_FOUND", format!("account {} does not exist", p.code)))?;
        if account.active == 0 {
            return Err(AppError::new("ACCOUNT_INACTIVE", format!("account {} is inactive", p.code)));
        }

        let mut vat_code_id = None;
        let mut vat_amount_cents = None;
        if let Some(vat_code) = &p.vat_code {
            let vat_row: Option<i64> = conn
                .query_row("SELECT id FROM vat_codes WHERE code = ?1", [vat_code], |r| r.get(0))
                .optional()?;
            let id = vat_row.ok_or_else(|| AppError::new("VAT_CODE_NOT_FOUND", format!("vat code '{vat_code}' does not exist")))?;
            vat_code_id = Some(id);
        }
        if let Some(v) = p.vat_amount_cents {
            vat_amount_cents = Some(v);
        }

        let mut fx_currency = None;
        let mut fx_amount_cents = None;
        if p.fx_currency.is_some() || p.fx_amount_cents.is_some() {
            let cur = p.fx_currency.clone().ok_or_else(|| AppError::new("INVALID_FX_CURRENCY", "fx currency must be ISO 4217"))?;
            if !fx_currency_re().is_match(&cur) {
                return Err(AppError::new("INVALID_FX_CURRENCY", format!("fx currency '{cur}' must be ISO 4217")));
            }
            let amt = p.fx_amount_cents.ok_or_else(|| AppError::new("INVALID_FX_AMOUNT", "fx amounts must be integers (cents)"))?;
            fx_currency = Some(cur);
            fx_amount_cents = Some(amt);
        }

        out.push(ResolvedPosting {
            account_id: account.id,
            code: account.code,
            amount_cents: p.amount_cents,
            vat_code_id,
            vat_amount_cents,
            fx_currency,
            fx_amount_cents,
        });
    }
    Ok(out)
}

/// One journal_entries row (metadata only — postings fetched separately).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct EntryMeta {
    pub id: i64,
    pub date: String,
    pub description: String,
    pub source: String,
    pub source_ref: Option<String>,
    pub state: String,
    pub reversed_from_id: Option<i64>,
    pub created_by: String,
    pub created_at: String,
    pub posted_at: Option<String>,
    pub reversed_at: Option<String>,
}

/// A posting row joined with account info (internal shape).
#[derive(Debug, Clone)]
pub struct PostingRow {
    pub id: i64,
    pub account_id: i64,
    pub amount_cents: i64,
    pub document_id: Option<i64>,
    pub vat_code_id: Option<i64>,
    pub vat_amount_cents: Option<i64>,
    pub fx_currency: Option<String>,
    pub fx_amount_cents: Option<i64>,
    pub account_code: String,
    pub account_name: String,
    pub account_type: String,
}

/// Serialize exactly like the Node CLI `serializeEntry` postings:
/// { id, account_code, account_name, account_type, amount_cents, amount }.
impl Serialize for PostingRow {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let mut st = serializer.serialize_struct("Posting", 6)?;
        st.serialize_field("id", &self.id)?;
        st.serialize_field("account_code", &self.account_code)?;
        st.serialize_field("account_name", &self.account_name)?;
        st.serialize_field("account_type", &self.account_type)?;
        st.serialize_field("amount_cents", &self.amount_cents)?;
        st.serialize_field("amount", &format_amount(self.amount_cents))?;
        st.end()
    }
}

/// A full entry with postings (what `entry add/post/reverse/show` return).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct Entry {
    #[serde(flatten)]
    pub meta: EntryMeta,
    pub postings: Vec<PostingRow>,
}

fn entry_meta_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<EntryMeta> {
    Ok(EntryMeta {
        id: row.get(0)?,
        date: row.get(1)?,
        description: row.get(2)?,
        source: row.get(3)?,
        source_ref: row.get(4)?,
        state: row.get(5)?,
        reversed_from_id: row.get(6)?,
        created_by: row.get(7)?,
        created_at: row.get(8)?,
        posted_at: row.get(9)?,
        reversed_at: row.get(10)?,
    })
}

/// Create a journal entry (state: draft) with its postings.
/// Validates: date, description, >= 2 postings, non-zero amounts, existing &
/// active accounts, sum == 0. All inside one transaction.
#[allow(clippy::too_many_arguments)]
pub fn create_entry(
    conn: &Connection,
    date: &str,
    description: &str,
    postings: &[PostingSpec],
    source: &str,
    source_ref: Option<&str>,
    actor: &str,
) -> Result<Entry> {
    validate_date(date)?;
    if description.trim().is_empty() {
        return Err(AppError::new("INVALID_DESCRIPTION", "description is required"));
    }
    if postings.len() < 2 {
        return Err(AppError::new("TOO_FEW_POSTINGS", "an entry needs at least 2 postings"));
    }
    if !VALID_SOURCES.contains(&source) {
        return Err(AppError::new("INVALID_SOURCE", format!("source '{source}' is not allowed")));
    }
    if actor.trim().is_empty() {
        return Err(AppError::new("INVALID_ACTOR", "actor is required (human or agent:<name>)"));
    }

    let resolved = resolve_postings(conn, postings)?;

    let sum: i64 = resolved.iter().map(|p| p.amount_cents).sum();
    if sum != 0 {
        return Err(AppError::new("UNBALANCED", format!("postings do not sum to zero (sum = {sum} cents)")));
    }

    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO journal_entries (date, description, source, source_ref, state, created_by) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![date, description.trim(), source, source_ref, "draft", actor],
    )?;
    let entry_id = tx.last_insert_rowid();
    for p in &resolved {
        tx.execute(
            "INSERT INTO postings (entry_id, account_id, amount_cents, vat_code_id, vat_amount_cents, fx_currency, fx_amount_cents) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![entry_id, p.account_id, p.amount_cents, p.vat_code_id, p.vat_amount_cents, p.fx_currency, p.fx_amount_cents],
        )?;
    }
    let args = serde_json::json!({
        "date": date,
        "description": description,
        "postings": resolved.iter().map(|p| format!("{}:{}", p.code, p.amount_cents)).collect::<Vec<_>>(),
        "source": source,
        "sourceRef": source_ref,
    });
    audit::record(&tx, actor, "entry.create", Some("entry add"), Some(&args), "ok", &[entry_id])?;
    tx.commit()?;

    get_entry(conn, entry_id)?.ok_or_else(|| AppError::new("NOT_FOUND", format!("entry {entry_id} does not exist")))
}

/// Transition draft -> posted. DB trigger backstops >= 2 postings and balance.
pub fn post_entry(conn: &Connection, id: i64, actor: &str) -> Result<Entry> {
    let entry = get_entry(conn, id)?
        .ok_or_else(|| AppError::new("NOT_FOUND", format!("entry {id} does not exist")))?;
    if entry.meta.state == "posted" {
        return Err(AppError::new("ALREADY_POSTED", format!("entry {id} is already posted")));
    }
    if entry.meta.state == "reversed" {
        return Err(AppError::new("ALREADY_REVERSED", format!("entry {id} is reversed")));
    }

    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "UPDATE journal_entries SET state = 'posted', posted_at = ?1 WHERE id = ?2",
        params![now_iso(), id],
    )?;
    audit::record(&tx, actor, "entry.post", Some("entry post"), Some(&serde_json::json!({"id": id})), "ok", &[id])?;
    tx.commit()?;

    get_entry(conn, id)?.ok_or_else(|| AppError::new("NOT_FOUND", format!("entry {id} does not exist")))
}

/// Reverse a posted entry: posts a linked contra-entry (negated postings).
/// The original entry REMAINS posted — the contra-entry cancels it. Posted
/// entries are never deleted — they are reversed.
pub fn reverse_entry(conn: &Connection, id: i64, actor: &str, reason: Option<&str>) -> Result<Entry> {
    let entry = get_entry(conn, id)?
        .ok_or_else(|| AppError::new("NOT_FOUND", format!("entry {id} does not exist")))?;
    if entry.meta.state != "posted" {
        let code = if entry.meta.state == "draft" { "NOT_POSTED" } else { "ALREADY_REVERSED" };
        let msg = if entry.meta.state == "draft" {
            format!("entry {id} must be posted before it can be reversed")
        } else {
            format!("entry {id} is already reversed")
        };
        return Err(AppError::new(code, msg));
    }
    let existing: i64 = conn.query_row(
        "SELECT COUNT(*) FROM journal_entries WHERE reversed_from_id = ?1 AND state = 'posted'",
        [id],
        |r| r.get(0),
    )?;
    if existing > 0 {
        return Err(AppError::new("ALREADY_REVERSED", format!("entry {id} already has a posted reversal")));
    }

    let description = match reason {
        Some(r) => format!("Reversal of entry {id} — {r}"),
        None => format!("Reversal of entry {id}"),
    };

    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO journal_entries (date, description, source, source_ref, state, reversed_from_id, created_by) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![entry.meta.date, description, "reversal", Option::<String>::None, "draft", id, actor],
    )?;
    let reversal_id = tx.last_insert_rowid();
    for p in &entry.postings {
        let fx_amount = match &p.fx_amount_cents {
            Some(v) => Some(-v),
            None => None,
        };
        tx.execute(
            "INSERT INTO postings (entry_id, account_id, amount_cents, fx_currency, fx_amount_cents) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![reversal_id, p.account_id, -p.amount_cents, p.fx_currency, fx_amount],
        )?;
    }
    tx.execute(
        "UPDATE journal_entries SET state = 'posted', posted_at = ?1 WHERE id = ?2",
        params![now_iso(), reversal_id],
    )?;
    audit::record(
        &tx,
        actor,
        "entry.reverse",
        Some("entry reverse"),
        Some(&serde_json::json!({"id": id, "reason": reason})),
        "ok",
        &[reversal_id, id],
    )?;
    tx.commit()?;

    get_entry(conn, reversal_id)?.ok_or_else(|| AppError::new("NOT_FOUND", format!("entry {reversal_id} does not exist")))
}

pub fn list_entries(conn: &Connection, state: Option<&str>, date_from: Option<&str>, date_to: Option<&str>, limit: i64) -> Result<Vec<EntryMeta>> {
    let mut clauses: Vec<String> = Vec::new();
    let mut params_vec: Vec<String> = Vec::new();
    if let Some(s) = state {
        clauses.push("state = ?".to_string());
        params_vec.push(s.to_string());
    }
    if let Some(d) = date_from {
        clauses.push("date >= ?".to_string());
        params_vec.push(d.to_string());
    }
    if let Some(d) = date_to {
        clauses.push("date <= ?".to_string());
        params_vec.push(d.to_string());
    }
    let mut sql = String::from("SELECT * FROM journal_entries");
    if !clauses.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&clauses.join(" AND "));
    }
    sql.push_str(" ORDER BY date DESC, id DESC LIMIT ?");

    let mut stmt = conn.prepare(&sql)?;
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = params_vec.into_iter().map(|p| Box::new(p) as Box<dyn rusqlite::ToSql>).collect();
    args.push(Box::new(limit));
    let param_refs: Vec<&dyn rusqlite::ToSql> = args.iter().map(|b| b.as_ref()).collect();
    let mut query = stmt.query(rusqlite::params_from_iter(param_refs))?;
    let mut rows = Vec::new();
    while let Some(row) = query.next()? {
        rows.push(entry_meta_from_row(row)?);
    }
    Ok(rows)
}

pub fn get_entry(conn: &Connection, id: i64) -> Result<Option<Entry>> {
    let meta: Option<EntryMeta> = conn
        .query_row("SELECT * FROM journal_entries WHERE id = ?1", [id], entry_meta_from_row)
        .optional()?;
    let Some(meta) = meta else { return Ok(None) };

    let mut stmt = conn.prepare(
        "SELECT p.id, p.account_id, p.amount_cents, p.document_id, p.vat_code_id, p.vat_amount_cents,\n\
                p.fx_currency, p.fx_amount_cents,\n\
                a.code AS account_code, a.name AS account_name, a.type AS account_type\n\
         FROM postings p JOIN accounts a ON a.id = p.account_id\n\
         WHERE p.entry_id = ?1 ORDER BY p.id",
    )?;
    let postings = stmt
        .query_map([id], |row| {
            Ok(PostingRow {
                id: row.get(0)?,
                account_id: row.get(1)?,
                amount_cents: row.get(2)?,
                document_id: row.get(3)?,
                vat_code_id: row.get(4)?,
                vat_amount_cents: row.get(5)?,
                fx_currency: row.get(6)?,
                fx_amount_cents: row.get(7)?,
                account_code: row.get(8)?,
                account_name: row.get(9)?,
                account_type: row.get(10)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    Ok(Some(Entry { meta, postings }))
}

pub fn get_account_by_id(conn: &Connection, id: i64) -> Result<Option<crate::core::accounts::Account>> {
    get_account(conn, id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::accounts::seed_default_chart;
    use crate::core::db::open_in_memory;

    fn seeded() -> Connection {
        let conn = open_in_memory().unwrap();
        seed_default_chart(&conn).unwrap();
        conn
    }

    fn specs(s: &str) -> Vec<PostingSpec> {
        parse_posting_specs(&[s.to_string()]).unwrap()
    }

    #[test]
    fn parse_specs_splits_commas_and_repeats() {
        let parsed = parse_posting_specs(&["1100:100.00,3000:-100.00".to_string(), "1200:5.00,2000:-5.00".to_string()]).unwrap();
        assert_eq!(parsed.len(), 4);
        assert_eq!(parsed[0].code, "1100");
        assert_eq!(parsed[0].amount_cents, 10000);
        assert_eq!(parsed[3].code, "2000");
        let err = parse_posting_specs(&["nope".to_string()]).unwrap_err();
        assert_eq!(err.code(), "INVALID_POSTING");
    }

    #[test]
    fn create_draft_entry() {
        let conn = seeded();
        let e = create_entry(&conn, "2026-08-05", "Test", &specs("1100:100.00,3000:-100.00"), "manual", None, "agent:test").unwrap();
        assert_eq!(e.meta.state, "draft");
        assert_eq!(e.meta.source, "manual");
        assert_eq!(e.meta.created_by, "agent:test");
        assert_eq!(e.postings.len(), 2);
        assert_eq!(e.postings[0].amount_cents, 10000);
        assert_eq!(e.postings[0].account_code, "1100");
        assert_eq!(e.postings[0].account_type, "asset");
        // audit row
        let audit_rows = audit::list(&conn, None, None, 10).unwrap();
        assert_eq!(audit_rows.len(), 1);
        assert_eq!(audit_rows[0].action, "entry.create");
        assert_eq!(audit_rows[0].entry_ids, vec![1]);
    }

    #[test]
    fn create_rejects_bad_input() {
        let conn = seeded();
        let err = create_entry(&conn, "2026-13-99", "x", &specs("1100:1.00,3000:-1.00"), "manual", None, "human").unwrap_err();
        assert_eq!(err.code(), "INVALID_DATE");
        let err = create_entry(&conn, "2026-08-05", "  ", &specs("1100:1.00,3000:-1.00"), "manual", None, "human").unwrap_err();
        assert_eq!(err.code(), "INVALID_DESCRIPTION");
        let err = create_entry(&conn, "2026-08-05", "x", &specs("1100:1.00"), "manual", None, "human").unwrap_err();
        assert_eq!(err.code(), "TOO_FEW_POSTINGS");
        let err = create_entry(&conn, "2026-08-05", "x", &specs("1100:1.00,3000:-1.00"), "bogus", None, "human").unwrap_err();
        assert_eq!(err.code(), "INVALID_SOURCE");
        let err = create_entry(&conn, "2026-08-05", "x", &specs("1100:1.00,3000:-1.00"), "manual", None, "").unwrap_err();
        assert_eq!(err.code(), "INVALID_ACTOR");
    }

    #[test]
    fn create_rejects_unbalanced() {
        let conn = seeded();
        let err = create_entry(&conn, "2026-08-05", "x", &specs("1100:100.00,3000:-99.00"), "manual", None, "human").unwrap_err();
        assert_eq!(err.code(), "UNBALANCED");
        assert!(err.message.contains("sum = 100 cents"));
    }

    #[test]
    fn create_rejects_unknown_or_inactive_account() {
        let conn = seeded();
        let err = create_entry(&conn, "2026-08-05", "x", &specs("9999:1.00,3000:-1.00"), "manual", None, "human").unwrap_err();
        assert_eq!(err.code(), "ACCOUNT_NOT_FOUND");
        crate::core::accounts::deactivate_account(&conn, "1100").unwrap();
        let err = create_entry(&conn, "2026-08-05", "x", &specs("1100:1.00,3000:-1.00"), "manual", None, "human").unwrap_err();
        assert_eq!(err.code(), "ACCOUNT_INACTIVE");
    }

    #[test]
    fn post_then_reverse() {
        let conn = seeded();
        let e = create_entry(&conn, "2026-08-05", "Test", &specs("1100:100.00,3000:-100.00"), "manual", None, "agent:test").unwrap();
        let posted = post_entry(&conn, e.meta.id, "agent:test").unwrap();
        assert_eq!(posted.meta.state, "posted");
        assert!(posted.meta.posted_at.is_some());
        // double post rejected
        let err = post_entry(&conn, e.meta.id, "human").unwrap_err();
        assert_eq!(err.code(), "ALREADY_POSTED");

        let reversed = reverse_entry(&conn, e.meta.id, "agent:test", Some("was wrong")).unwrap();
        assert_eq!(reversed.meta.state, "posted");
        assert_eq!(reversed.meta.source, "reversal");
        assert_eq!(reversed.meta.reversed_from_id, Some(e.meta.id));
        assert_eq!(reversed.postings[0].amount_cents, -10000);
        assert!(reversed.meta.description.contains("was wrong"));
        // double reversal rejected
        let err = reverse_entry(&conn, e.meta.id, "human", None).unwrap_err();
        assert_eq!(err.code(), "ALREADY_REVERSED");
    }

    #[test]
    fn reverse_requires_posted() {
        let conn = seeded();
        let e = create_entry(&conn, "2026-08-05", "Test", &specs("1100:100.00,3000:-100.00"), "manual", None, "human").unwrap();
        let err = reverse_entry(&conn, e.meta.id, "human", None).unwrap_err();
        assert_eq!(err.code(), "NOT_POSTED");
        let err = post_entry(&conn, 999, "human").unwrap_err();
        assert_eq!(err.code(), "NOT_FOUND");
    }

    #[test]
    fn posted_entries_are_immutable_at_db_level() {
        let conn = seeded();
        let e = create_entry(&conn, "2026-08-05", "Test", &specs("1100:100.00,3000:-100.00"), "manual", None, "human").unwrap();
        post_entry(&conn, e.meta.id, "human").unwrap();
        let err = conn
            .execute("UPDATE journal_entries SET description = 'hacked' WHERE id = ?1", [e.meta.id])
            .unwrap_err();
        assert!(err.to_string().contains("cannot modify a posted entry"));
        // postings too
        let err = conn
            .execute("UPDATE postings SET amount_cents = 1 WHERE entry_id = ?1", [e.meta.id])
            .unwrap_err();
        assert!(err.to_string().contains("cannot modify postings of a non-draft entry"));
    }

    #[test]
    fn list_filters_and_orders() {
        let conn = seeded();
        create_entry(&conn, "2026-08-01", "A", &specs("1100:10.00,3000:-10.00"), "manual", None, "human").unwrap();
        create_entry(&conn, "2026-08-05", "B", &specs("1100:20.00,3000:-20.00"), "manual", None, "human").unwrap();
        let all = list_entries(&conn, None, None, None, 100).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].description, "B"); // newest first
        let drafts = list_entries(&conn, Some("draft"), None, None, 100).unwrap();
        assert_eq!(drafts.len(), 2);
        let range = list_entries(&conn, None, Some("2026-08-02"), None, 100).unwrap();
        assert_eq!(range.len(), 1);
        assert_eq!(range[0].description, "B");
        let one = list_entries(&conn, None, None, None, 1).unwrap();
        assert_eq!(one.len(), 1);
    }
}
