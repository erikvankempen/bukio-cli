//! DB layer — open, pragmas, migrations.
//!
//! The migrations are embedded at compile time (include_str!), so the binary
//! is self-contained. The SQL is byte-identical to the Node version — the
//! schema, triggers and user_version bookkeeping are shared, which keeps
//! existing database files 100% compatible.

use crate::error::Result;
use rusqlite::Connection;

pub const MIGRATIONS: &[(u32, &str)] = &[
    (1, include_str!("../../migrations/001_initial.sql")),
    (2, include_str!("../../migrations/002_bank_vat.sql")),
    (3, include_str!("../../migrations/003_recurring.sql")),
    (4, include_str!("../../migrations/004_invoices.sql")),
    (5, include_str!("../../migrations/005_recurring_invoices.sql")),
    (6, include_str!("../../migrations/006_fx.sql")),
    (7, include_str!("../../migrations/007_compliance.sql")),
];

pub fn open_db(path: &str) -> Result<Connection> {
    let conn = Connection::open(path)?;
    configure(&conn)?;
    migrate(&conn)?;
    Ok(conn)
}

pub fn open_in_memory() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    configure(&conn)?;
    migrate(&conn)?;
    Ok(conn)
}

fn configure(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;\n\
         PRAGMA foreign_keys = ON;\n\
         PRAGMA busy_timeout = 5000;",
    )?;
    Ok(())
}

/// Apply pending migrations; each runs in its own transaction and bumps
/// `user_version` (same bookkeeping as the Node version).
pub fn migrate(conn: &Connection) -> Result<()> {
    let current: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    for (version, sql) in MIGRATIONS {
        if *version as i64 <= current {
            continue;
        }
        let tx = conn.unchecked_transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", *version as i64)?;
        tx.commit()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrate_applies_all_versions() {
        let conn = open_in_memory().unwrap();
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(version, 7);
        // core tables exist
        for table in ["company", "accounts", "journal_entries", "postings", "audit_log",
                      "bank_accounts", "bank_transactions", "reconciliations", "vat_codes",
                      "vat_returns", "recurring_templates", "contacts", "invoices",
                      "invoice_lines", "invoice_payments", "fx_rates", "filings"] {
            let count: i64 = conn
                .query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1", [table], |r| r.get(0))
                .unwrap();
            assert_eq!(count, 1, "missing table {table}");
        }
    }

    #[test]
    fn migrate_is_idempotent() {
        let conn = open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(version, 7);
    }
}
