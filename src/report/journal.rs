//! Journal export — every posting in a date range, with account info.
//! One row per posting (entries without postings appear as a single row).
//! Port of the Node `src/report/journal.js`.

use crate::error::Result;
use rusqlite::{params, Connection};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct JournalRow {
    pub entry_id: i64,
    pub date: String,
    pub description: String,
    pub source: String,
    pub state: String,
    pub created_by: String,
    pub amount_cents: Option<i64>,
    pub account_code: Option<String>,
    pub account_name: Option<String>,
    pub account_type: Option<String>,
}

pub fn journal(conn: &Connection, from: &str, to: &str) -> Result<Vec<JournalRow>> {
    let mut stmt = conn.prepare(
        "SELECT e.id, e.date, e.description, e.source, e.state, e.created_by,\n\
                p.amount_cents, a.code, a.name, a.type\n\
         FROM journal_entries e\n\
         LEFT JOIN postings p ON p.entry_id = e.id\n\
         LEFT JOIN accounts a ON a.id = p.account_id\n\
         WHERE e.date >= ?1 AND e.date <= ?2\n\
         ORDER BY e.date, e.id, p.id",
    )?;
    let rows = stmt
        .query_map(params![from, to], |row| {
            Ok(JournalRow {
                entry_id: row.get(0)?,
                date: row.get(1)?,
                description: row.get(2)?,
                source: row.get(3)?,
                state: row.get(4)?,
                created_by: row.get(5)?,
                amount_cents: row.get(6)?,
                account_code: row.get(7)?,
                account_name: row.get(8)?,
                account_type: row.get(9)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}
