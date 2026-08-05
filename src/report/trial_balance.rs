//! Trial balance — per-account debit/credit/net from posted entries.
//! Port of the Node `src/report/trial-balance.js`.

use crate::error::Result;
use rusqlite::{params, Connection};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct TbAccount {
    pub code: String,
    pub name: String,
    #[serde(rename = "type")]
    pub account_type: String,
    pub debit_cents: i64,
    pub credit_cents: i64,
    pub net_cents: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct TrialBalance {
    pub accounts: Vec<TbAccount>,
    pub total_debit_cents: i64,
    pub total_credit_cents: i64,
    pub balanced: bool,
}

pub fn trial_balance(conn: &Connection, year: Option<&str>) -> Result<TrialBalance> {
    let mut stmt = conn.prepare(
        "SELECT a.code, a.name, a.type,\n\
                SUM(CASE WHEN p.amount_cents > 0 THEN p.amount_cents ELSE 0 END) AS debit_cents,\n\
                SUM(CASE WHEN p.amount_cents < 0 THEN -p.amount_cents ELSE 0 END) AS credit_cents,\n\
                SUM(p.amount_cents) AS net_cents\n\
         FROM postings p\n\
         JOIN journal_entries e ON e.id = p.entry_id\n\
         JOIN accounts a ON a.id = p.account_id\n\
         WHERE e.state = 'posted'\n\
           AND (?1 IS NULL OR substr(e.date, 1, 4) = ?1)\n\
         GROUP BY a.id\n\
         ORDER BY a.code",
    )?;
    let rows = stmt
        .query_map(params![year], |row| {
            Ok(TbAccount {
                code: row.get(0)?,
                name: row.get(1)?,
                account_type: row.get(2)?,
                debit_cents: row.get(3)?,
                credit_cents: row.get(4)?,
                net_cents: row.get(5)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let total_debit: i64 = rows.iter().map(|r| r.debit_cents).sum();
    let total_credit: i64 = rows.iter().map(|r| r.credit_cents).sum();

    Ok(TrialBalance {
        accounts: rows,
        total_debit_cents: total_debit,
        total_credit_cents: total_credit,
        balanced: total_debit == total_credit,
    })
}
