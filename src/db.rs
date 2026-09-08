// bukio-cli — agent-first double-entry bookkeeping for SMEs.
// Copyright (c) 2026 Erik van Kempen.
// SPDX-License-Identifier: Apache-2.0
//
// DB layer — open, pragmas, migrations (mirrors src/core/db.js).

use rusqlite::Connection;
use std::path::Path;

pub mod migrations_data {
    include!("migrations_data.rs");
}

pub fn open_db(db_path: &str) -> rusqlite::Result<Connection> {
    let db = if db_path == ":memory:" {
        Connection::open_in_memory()?
    } else {
        Connection::open(Path::new(db_path))?
    };
    db.pragma_update(None, "journal_mode", "WAL")?;
    db.pragma_update(None, "foreign_keys", "ON")?;
    db.pragma_update(None, "busy_timeout", 5000)?;
    migrate(&db)?;
    Ok(db)
}

pub fn migrate(db: &Connection) -> rusqlite::Result<()> {
    let current: i64 = db.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    for (version, sql) in migrations_data::MIGRATIONS {
        let v = *version as i64;
        if v <= current {
            continue;
        }
        if sql.to_ascii_lowercase().contains("pragma foreign_keys") {
            // Rebuild migrations run OUTSIDE a transaction — the pragma is a
            // no-op inside one (mirrors the JS runner).
            let was_on: bool =
                db.query_row("PRAGMA foreign_keys", [], |r| r.get::<_, i64>(0))? == 1;
            db.pragma_update(None, "foreign_keys", "OFF")?;
            let result = (|| -> rusqlite::Result<()> {
                db.execute_batch(sql)?;
                db.pragma_update(None, "user_version", v)?;
                Ok(())
            })();
            let restore = db.pragma_update(None, "foreign_keys", if was_on { "ON" } else { "OFF" });
            result?;
            restore?;
        } else {
            db.execute_batch("BEGIN")?;
            let inner = (|| -> rusqlite::Result<()> {
                db.execute_batch(sql)?;
                db.pragma_update(None, "user_version", v)?;
                Ok(())
            })();
            match inner {
                Ok(()) => {
                    db.execute_batch("COMMIT")?;
                }
                Err(e) => {
                    let _ = db.execute_batch("ROLLBACK");
                    return Err(e);
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_memory_db_reaches_latest_migration() {
        let db = open_db(":memory:").unwrap();
        let v: i64 = db
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        let last = migrations_data::MIGRATIONS.last().unwrap().0 as i64;
        assert_eq!(v, last, "fresh DB must apply every migration");
        // the core tables exist
        let n: i64 = db
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='journal_entries'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn reopen_existing_db_does_not_reapply() {
        let db = open_db(":memory:").unwrap();
        migrate(&db).unwrap(); // idempotent second call
        let v: i64 = db
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, migrations_data::MIGRATIONS.last().unwrap().0 as i64);
    }
}
