// bukio-cli — agent-first double-entry bookkeeping for SMEs.
// Copyright (c) 2026 Erik van Kempen.
// SPDX-License-Identifier: Apache-2.0
//
// Posting engine (mirrors src/core/entries.js) — the heart of bukio-cli.

use crate::actor::now_iso;
use crate::audit::{record, RecordArgs};
use crate::dates::validate_date;
use crate::money::{parse_amount, BukioError, Result};
use rusqlite::Connection;
use serde_json::{json, Map, Value};

pub const VALID_SOURCES: [&str; 10] = [
    "manual",
    "bank",
    "invoice",
    "agent",
    "reversal",
    "recurring",
    "closing",
    "import",
    "xaf",
    "assets",
];

/// Parse a posting spec "CODE:AMOUNT[@CC]" list (comma-splittable) into
/// [{code, amountCents, costCenterCode}]. Pure (mirrors
/// parsePostingSpecsWithCostCenter in core/cost-centers.js).
pub fn parse_posting_specs(raw: &[String]) -> Result<Vec<PostingSpec>> {
    let mut out = Vec::new();
    for item in raw {
        for token in item.split(',') {
            let t = token.trim();
            if t.is_empty() {
                continue;
            }
            // CODE:AMOUNT[@CC]
            let bad = || {
                BukioError::new(
                    "INVALID_POSTING",
                    format!(
                        "posting '{t}' must be CODE:AMOUNT[@COSTCENTER] (e.g. 8000:-100.00@HQ)"
                    ),
                )
            };
            let Some((code, rest)) = t.split_once(':') else {
                return Err(bad());
            };
            if code.is_empty() || code.len() > 6 || !code.bytes().all(|b| b.is_ascii_digit()) {
                return Err(bad());
            }
            let (amount, cc) = match rest.split_once('@') {
                Some((a, c)) => (a, Some(c)),
                None => (rest, None),
            };
            if let Some(c) = cc {
                if !valid_cc_code(c) {
                    return Err(bad());
                }
            }
            out.push(PostingSpec {
                code: code.to_string(),
                amount_cents: parse_amount(amount)?,
                cost_center_code: cc.map(String::from),
                vat_code: None,
                vat_amount_cents: None,
            });
        }
    }
    Ok(out)
}

fn valid_cc_code(c: &str) -> bool {
    // ^[A-Z0-9][A-Z0-9 ._-]{0,31}$
    let b = c.as_bytes();
    if b.is_empty() || b.len() > 32 {
        return false;
    }
    if !b[0].is_ascii_alphanumeric() {
        return false;
    }
    b[1..]
        .iter()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, b' ' | b'.' | b'_' | b'-'))
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PostingSpec {
    pub code: String,
    pub amount_cents: i64,
    pub cost_center_code: Option<String>,
    /// Optional VAT tag (JS: `vatCode` on the posting spec) — the code must
    /// exist in vat_codes; null/inert when the VAT module is off.
    pub vat_code: Option<String>,
    /// VAT amount in cents carried alongside the base (JS: `vatAmountCents`).
    pub vat_amount_cents: Option<i64>,
}

/// Entry as returned by get_entry — the JSON shape matches serializeEntry.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Entry {
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
    pub postings: Vec<Posting>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Posting {
    pub id: i64,
    pub account_code: String,
    pub account_name: String,
    pub account_type: String,
    pub amount_cents: i64,
    pub vat_code: Option<String>,
    pub cost_center_code: Option<String>,
    pub cost_center_name: Option<String>,
}

pub fn get_entry(db: &Connection, id: i64) -> Option<Entry> {
    let row = db
        .query_row(
            "SELECT id, date, description, source, source_ref, state, reversed_from_id, created_by, created_at, posted_at
             FROM journal_entries WHERE id = ?1",
            [id],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, Option<i64>>(6)?,
                    r.get::<_, String>(7)?,
                    r.get::<_, String>(8)?,
                    r.get::<_, Option<String>>(9)?,
                ))
            },
        )
        .ok()?;
    let mut stmt = db
        .prepare(
            "SELECT p.id, a.code, a.name, a.type, p.amount_cents, vc.code, cc.code, cc.name
             FROM postings p
             JOIN accounts a ON a.id = p.account_id
             LEFT JOIN vat_codes vc ON vc.id = p.vat_code_id
             LEFT JOIN cost_centers cc ON cc.id = p.cost_center_id
             WHERE p.entry_id = ?1 ORDER BY p.id",
        )
        .ok()?;
    let postings = stmt
        .query_map([id], |r| {
            Ok(Posting {
                id: r.get(0)?,
                account_code: r.get(1)?,
                account_name: r.get(2)?,
                account_type: r.get(3)?,
                amount_cents: r.get(4)?,
                vat_code: r.get(5)?,
                cost_center_code: r.get(6)?,
                cost_center_name: r.get(7)?,
            })
        })
        .ok()?
        .filter_map(|p| p.ok())
        .collect();
    Some(Entry {
        id: row.0,
        date: row.1,
        description: row.2,
        source: row.3,
        source_ref: row.4,
        state: row.5,
        reversed_from_id: row.6,
        created_by: row.7,
        created_at: row.8,
        posted_at: row.9,
        postings,
    })
}

pub struct CreateEntry<'a> {
    pub date: &'a str,
    pub description: &'a str,
    pub postings: Vec<PostingSpec>,
    pub source: &'a str,
    pub source_ref: Option<&'a str>,
    pub actor: &'a str,
}

/// Create a journal entry (state: draft) with its postings — validates date,
/// description, >=2 postings, non-zero, active accounts, sum==0. One tx.
pub fn create_entry(db: &Connection, input: CreateEntry<'_>) -> Result<Entry> {
    validate_date(input.date)?;
    if input.description.trim().is_empty() {
        return Err(BukioError::new(
            "INVALID_DESCRIPTION",
            "description is required",
        ));
    }
    if input.postings.len() < 2 {
        return Err(BukioError::new(
            "TOO_FEW_POSTINGS",
            "an entry needs at least 2 postings",
        ));
    }
    if !VALID_SOURCES.contains(&input.source) {
        return Err(BukioError::new(
            "INVALID_SOURCE",
            format!("source '{}' is not allowed", input.source),
        ));
    }
    if input.actor.trim().is_empty() {
        return Err(BukioError::new(
            "INVALID_ACTOR",
            "actor is required (human or agent:<name>)",
        ));
    }

    // resolve postings: accounts exist + active; cost centers exist + active
    let mut resolved: Vec<(i64, i64, Option<i64>, Option<i64>, Option<i64>)> = Vec::new(); // (account_id, amount, cc_id, vat_code_id, vat_amount_cents)
    for p in &input.postings {
        if p.amount_cents == 0 {
            return Err(BukioError::new(
                "INVALID_AMOUNT_CENTS",
                "posting amounts must be non-zero integers (cents)",
            ));
        }
        let account = db
            .query_row(
                "SELECT id, active FROM accounts WHERE code = ?1",
                [&p.code],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
            )
            .map_err(|_| {
                BukioError::new(
                    "ACCOUNT_NOT_FOUND",
                    format!("account {} does not exist", p.code),
                )
            })?;
        if account.1 == 0 {
            return Err(BukioError::new(
                "ACCOUNT_INACTIVE",
                format!("account {} is inactive", p.code),
            ));
        }
        let cc_id = match &p.cost_center_code {
            None => None,
            Some(code) => {
                let cc = db
                    .query_row(
                        "SELECT id, active FROM cost_centers WHERE code = ?1",
                        [code],
                        |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
                    )
                    .map_err(|_| {
                        BukioError::new(
                            "COST_CENTER_NOT_FOUND",
                            format!("cost center '{code}' does not exist"),
                        )
                    })?;
                if cc.1 == 0 {
                    return Err(BukioError::new(
                        "COST_CENTER_INACTIVE",
                        format!("cost center '{code}' is inactive"),
                    ));
                }
                Some(cc.0)
            }
        };
        // VAT tag (optional): the code must exist, the amount is inert when
        // the VAT module is off — same contract as JS resolvePostings
        let vat_code_id = match &p.vat_code {
            None => None,
            Some(code) => Some(
                db.query_row("SELECT id FROM vat_codes WHERE code = ?1", [code.as_str()], |r| {
                    r.get::<_, i64>(0)
                })
                .map_err(|_| {
                    BukioError::new(
                        "VAT_CODE_NOT_FOUND",
                        format!("vat code '{code}' does not exist"),
                    )
                })?,
            ),
        };
        resolved.push((
            account.0,
            p.amount_cents,
            cc_id,
            vat_code_id,
            p.vat_amount_cents,
        ));
    }

    let sum: i64 = resolved.iter().map(|(_, a, ..)| a).sum();
    if sum != 0 {
        return Err(BukioError::new(
            "UNBALANCED",
            format!("postings do not sum to zero (sum = {sum} cents)"),
        ));
    }

    let tx = db.unchecked_transaction().map_err(sql_err)?;
    let entry_id = {
        tx.execute(
            "INSERT INTO journal_entries (date, description, source, source_ref, state, created_by)
             VALUES (?1, ?2, ?3, ?4, 'draft', ?5)",
            rusqlite::params![
                input.date,
                input.description.trim(),
                input.source,
                input.source_ref,
                input.actor
            ],
        )
        .map_err(sql_err)?;
        let id = tx.last_insert_rowid();
        for (account_id, amount, cc_id, vat_code_id, vat_amount_cents) in &resolved {
            tx.execute(
                "INSERT INTO postings (entry_id, account_id, amount_cents, vat_code_id, vat_amount_cents, fx_currency, fx_amount_cents, cost_center_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, NULL, NULL, ?6)",
                rusqlite::params![id, account_id, amount, vat_code_id, vat_amount_cents, cc_id],
            )
            .map_err(sql_err)?;
        }
        record(
            &tx,
            RecordArgs {
                actor: input.actor,
                action: "entry.create",
                command: Some("entry add"),
                args: Some(json!({
                    "date": input.date,
                    "description": input.description.trim(),
                    "postings": input.postings.iter().map(|p| format!("{}:{}", p.code, p.amount_cents)).collect::<Vec<_>>(),
                    "source": input.source,
                    "sourceRef": input.source_ref,
                })),
                outcome: "ok",
                entry_ids: vec![id],
            },
        )?;
        id
    };
    tx.commit().map_err(sql_err)?;
    get_entry(db, entry_id)
        .ok_or_else(|| BukioError::new("INTERNAL", "entry vanished after insert"))
}

fn sql_err(e: rusqlite::Error) -> BukioError {
    BukioError::new("DB_ERROR", e.to_string())
}

/// Transition draft -> posted (DB trigger backstops balance).
pub fn post_entry(db: &Connection, id: i64, actor: &str) -> Result<Entry> {
    let entry = get_entry(db, id)
        .ok_or_else(|| BukioError::new("NOT_FOUND", format!("entry {id} does not exist")))?;
    match entry.state.as_str() {
        "posted" => {
            return Err(BukioError::new(
                "ALREADY_POSTED",
                format!("entry {id} is already posted"),
            ))
        }
        "reversed" => {
            return Err(BukioError::new(
                "ALREADY_REVERSED",
                format!("entry {id} is reversed"),
            ))
        }
        _ => {}
    }
    let tx = db.unchecked_transaction().map_err(sql_err)?;
    {
        let inactive: Vec<String> = {
            let mut stmt = tx
                .prepare(
                    "SELECT a.code FROM postings p JOIN accounts a ON a.id = p.account_id
                     WHERE p.entry_id = ?1 AND a.active = 0",
                )
                .map_err(sql_err)?;
            let rows: Vec<String> = stmt
                .query_map([id], |r| r.get::<_, String>(0))
                .map_err(sql_err)?
                .filter_map(|x| x.ok())
                .collect();
            rows
        };
        if !inactive.is_empty() {
            return Err(BukioError::new(
                "ACCOUNT_INACTIVE",
                format!(
                    "entry {id} postings reference deactivated account(s): {}",
                    inactive.join(", ")
                ),
            ));
        }
        tx.execute(
            "UPDATE journal_entries SET state = 'posted', posted_at = ?1 WHERE id = ?2",
            rusqlite::params![now_iso(), id],
        )
        .map_err(sql_err)?;
        record(
            &tx,
            RecordArgs {
                actor,
                action: "entry.post",
                command: Some("entry post"),
                args: Some(json!({"id": id})),
                outcome: "ok",
                entry_ids: vec![id],
            },
        )?;
    }
    tx.commit().map_err(sql_err)?;
    get_entry(db, id).ok_or_else(|| BukioError::new("INTERNAL", "entry vanished"))
}

/// Reverse a posted entry: posts a linked contra-entry (negated postings).
pub fn reverse_entry(db: &Connection, id: i64, actor: &str, reason: Option<&str>) -> Result<Entry> {
    let entry = get_entry(db, id)
        .ok_or_else(|| BukioError::new("NOT_FOUND", format!("entry {id} does not exist")))?;
    match entry.state.as_str() {
        "draft" => {
            return Err(BukioError::new(
                "NOT_POSTED",
                format!("entry {id} must be posted before it can be reversed"),
            ))
        }
        "reversed" => {
            return Err(BukioError::new(
                "ALREADY_REVERSED",
                format!("entry {id} is already reversed"),
            ))
        }
        _ => {}
    }
    let count_reversals = |db: &Connection| -> Result<i64> {
        db.query_row(
            "SELECT COUNT(*) FROM journal_entries WHERE reversed_from_id = ?1 AND state = 'posted'",
            [id],
            |r| r.get(0),
        )
        .map_err(sql_err)
    };
    if count_reversals(db)? > 0 {
        return Err(BukioError::new(
            "ALREADY_REVERSED",
            format!("entry {id} already has a posted reversal"),
        ));
    }

    let description = match reason {
        Some(r) if !r.is_empty() => format!("Reversal of entry {id} — {r}"),
        _ => format!("Reversal of entry {id}"),
    };
    let tx = db.unchecked_transaction().map_err(sql_err)?;
    let reversal_id = {
        // re-check inside the transaction (two processes, WAL) — same guard as JS
        if count_reversals(&tx)? > 0 {
            return Err(BukioError::new(
                "ALREADY_REVERSED",
                format!("entry {id} already has a posted reversal"),
            ));
        }
        tx.execute(
            "INSERT INTO journal_entries (date, description, source, source_ref, state, reversed_from_id, created_by)
             VALUES (?1, ?2, 'reversal', NULL, 'draft', ?3, ?4)",
            rusqlite::params![entry.date, description, id, actor],
        )
        .map_err(sql_err)?;
        let rid = tx.last_insert_rowid();
        // negated postings (VAT/FX negated, cost center carried over) — single INSERT…SELECT, no loop
        tx.execute(
                "INSERT INTO postings (entry_id, account_id, amount_cents, vat_code_id, vat_amount_cents, fx_currency, fx_amount_cents, cost_center_id)
                 SELECT ?1, account_id, -amount_cents, vat_code_id,
                        CASE WHEN vat_amount_cents IS NULL THEN NULL ELSE -vat_amount_cents END,
                        fx_currency,
                        CASE WHEN fx_amount_cents IS NULL THEN NULL ELSE -fx_amount_cents END,
                        cost_center_id
                 FROM postings WHERE entry_id = ?2 ORDER BY id",
                rusqlite::params![rid, id],
            )
            .map_err(sql_err)?;
        tx.execute(
            "UPDATE journal_entries SET state = 'posted', posted_at = ?1 WHERE id = ?2",
            rusqlite::params![now_iso(), rid],
        )
        .map_err(sql_err)?;
        record(
            &tx,
            RecordArgs {
                actor,
                action: "entry.reverse",
                command: Some("entry reverse"),
                args: Some(json!({"id": id, "reason": reason})),
                outcome: "ok",
                entry_ids: vec![rid, id],
            },
        )?;
        rid
    };
    tx.commit().map_err(sql_err)?;
    get_entry(db, reversal_id).ok_or_else(|| BukioError::new("INTERNAL", "reversal vanished"))
}

pub fn list_entries(
    db: &Connection,
    state: Option<&str>,
    date_from: Option<&str>,
    date_to: Option<&str>,
    limit: i64,
) -> Result<Vec<Value>> {
    if limit < 0 {
        return Err(BukioError::new(
            "INVALID_LIMIT",
            format!("limit must be a non-negative integer, got '{limit}'"),
        ));
    }
    if let Some(d) = date_from {
        validate_date(d)?;
    }
    if let Some(d) = date_to {
        validate_date(d)?;
    }
    let mut sql = String::from(
        "SELECT id, date, description, state, source, created_by FROM journal_entries",
    );
    let mut clauses = Vec::new();
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    if let Some(s) = state {
        clauses.push("state = ?");
        params.push(Box::new(s.to_string()));
    }
    if let Some(d) = date_from {
        clauses.push("date >= ?");
        params.push(Box::new(d.to_string()));
    }
    if let Some(d) = date_to {
        clauses.push("date <= ?");
        params.push(Box::new(d.to_string()));
    }
    if !clauses.is_empty() {
        sql.push_str(&format!(" WHERE {}", clauses.join(" AND ")));
    }
    sql.push_str(" ORDER BY date DESC, id DESC LIMIT ?");
    params.push(Box::new(limit));
    let mut stmt = db.prepare(&sql).map_err(sql_err)?;
    let rows = stmt
        .query_map(params_refs(&params).as_slice(), |r| {
            Ok(json!({
                "id": r.get::<_, i64>(0)?,
                "date": r.get::<_, String>(1)?,
                "description": r.get::<_, String>(2)?,
                "state": r.get::<_, String>(3)?,
                "source": r.get::<_, String>(4)?,
                "created_by": r.get::<_, String>(5)?,
            }))
        })
        .map_err(sql_err)?
        .filter_map(|x| x.ok())
        .collect();
    Ok(rows)
}

/// serializeEntry — the exact JSON shape of the JS CLI (camelCase keys
/// converted to snake_case, amount strings added).
pub fn entry_to_json(e: &Entry) -> Value {
    let mut root = Map::new();
    root.insert("id".into(), json!(e.id));
    root.insert("date".into(), json!(e.date));
    root.insert("description".into(), json!(e.description));
    root.insert("source".into(), json!(e.source));
    root.insert("source_ref".into(), json!(e.source_ref));
    root.insert("state".into(), json!(e.state));
    root.insert("reversed_from_id".into(), json!(e.reversed_from_id));
    root.insert("created_by".into(), json!(e.created_by));
    root.insert("created_at".into(), json!(e.created_at));
    root.insert("posted_at".into(), json!(e.posted_at));
    let postings: Vec<Value> = e
        .postings
        .iter()
        .map(|p| {
            json!({
                "id": p.id,
                "account_code": p.account_code,
                "account_name": p.account_name,
                "account_type": p.account_type,
                "amount_cents": p.amount_cents,
                "amount": crate::money::format_amount(p.amount_cents),
                "cost_center_code": p.cost_center_code,
                "cost_center_name": p.cost_center_name,
            })
        })
        .collect();
    root.insert("postings".into(), Value::Array(postings));
    Value::Object(root)
}

/// ponytail: dyn-param helper — rusqlite 0.32 has no params_from_vec; refs
/// satisfy Params for &[&dyn ToSql].
fn params_refs(params: &[Box<dyn rusqlite::types::ToSql>]) -> Vec<&dyn rusqlite::types::ToSql> {
    params.iter().map(|p| p.as_ref()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_db;

    fn setup() -> Connection {
        let db = open_db(":memory:").unwrap();
        db.execute("INSERT INTO company (name) VALUES ('Test BV')", [])
            .unwrap();
        for (code, name, ty) in [
            ("1100", "Bank", "asset"),
            ("3000", "Eigen vermogen", "equity"),
            ("8000", "Omzet", "income"),
        ] {
            db.execute(
                "INSERT INTO accounts (code, name, type, normal_balance, taxonomy) VALUES (?1, ?2, ?3, ?4, 'RGS')",
                rusqlite::params![code, name, ty, if ty == "asset" { "debit" } else { "credit" }],
            )
            .unwrap();
        }
        db
    }

    fn spec(items: &[(&str, i64)]) -> Vec<PostingSpec> {
        items
            .iter()
            .map(|(c, a)| PostingSpec {
                code: c.to_string(),
                amount_cents: *a,
                cost_center_code: None,
                vat_code: None,
                vat_amount_cents: None,
            })
            .collect()
    }

    #[test]
    fn create_post_reverse_lifecycle() {
        let db = setup();
        let e = create_entry(
            &db,
            CreateEntry {
                date: "2026-01-10",
                description: "Startkapitaal",
                postings: spec(&[("1100", 1000000), ("3000", -1000000)]),
                source: "manual",
                source_ref: None,
                actor: "human:erik",
            },
        )
        .unwrap();
        assert_eq!(e.state, "draft");
        assert_eq!(e.postings.len(), 2);
        assert_eq!(e.created_by, "human:erik");

        let p = post_entry(&db, e.id, "human:erik").unwrap();
        assert_eq!(p.state, "posted");

        let r = reverse_entry(&db, p.id, "human:erik", Some("fout")).unwrap();
        assert_eq!(r.state, "posted");
        assert_eq!(r.reversed_from_id, Some(p.id));
        // double reversal refused
        assert_eq!(
            reverse_entry(&db, p.id, "human:erik", None)
                .unwrap_err()
                .code,
            "ALREADY_REVERSED"
        );
    }

    #[test]
    fn rejects_like_js() {
        let db = setup();
        let unbalanced = create_entry(
            &db,
            CreateEntry {
                date: "2026-01-10",
                description: "x",
                postings: spec(&[("1100", 100), ("3000", -99)]),
                source: "manual",
                source_ref: None,
                actor: "human:erik",
            },
        );
        assert_eq!(unbalanced.unwrap_err().code, "UNBALANCED");
        let one = create_entry(
            &db,
            CreateEntry {
                date: "2026-01-10",
                description: "x",
                postings: spec(&[("1100", 100)]),
                source: "manual",
                source_ref: None,
                actor: "human:erik",
            },
        );
        assert_eq!(one.unwrap_err().code, "TOO_FEW_POSTINGS");
        let unknown = create_entry(
            &db,
            CreateEntry {
                date: "2026-01-10",
                description: "x",
                postings: spec(&[("9999", 100), ("3000", -100)]),
                source: "manual",
                source_ref: None,
                actor: "human:erik",
            },
        );
        assert_eq!(unknown.unwrap_err().code, "ACCOUNT_NOT_FOUND");
        let bad_date = create_entry(
            &db,
            CreateEntry {
                date: "2026-02-30",
                description: "x",
                postings: spec(&[("1100", 100), ("3000", -100)]),
                source: "manual",
                source_ref: None,
                actor: "human:erik",
            },
        );
        assert_eq!(bad_date.unwrap_err().code, "INVALID_DATE");
    }

    #[test]
    fn posting_spec_parsing() {
        let specs = parse_posting_specs(&["1100:100.00,3000:-50.00@HQ ".to_string()]).unwrap();
        assert_eq!(specs.len(), 2);
        assert_eq!(specs[1].cost_center_code.as_deref(), Some("HQ"));
        assert_eq!(specs[0].amount_cents, 10000);
        let bad = parse_posting_specs(&["nope".to_string()]);
        assert_eq!(bad.unwrap_err().code, "INVALID_POSTING");
    }
}
