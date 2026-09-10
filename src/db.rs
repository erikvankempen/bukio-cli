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

    // ── ported from test/migration-021.test.js ──
    // That name was misleading: the file covers the whole 021-026 chain
    // upgrading a v020 database, not migration 021 on its own. The tests below
    // are named for what they actually check.

    fn apply_migrations_upto(db: &Connection, max_version: u32) {
        for (version, sql) in migrations_data::MIGRATIONS {
            if *version > max_version {
                continue;
            }
            // Rebuild migrations run outside a transaction (mirrors `migrate`)
            if sql.to_ascii_lowercase().contains("pragma foreign_keys") {
                db.pragma_update(None, "foreign_keys", "OFF").unwrap();
                db.execute_batch(sql).unwrap();
                db.pragma_update(None, "user_version", *version as i64)
                    .unwrap();
                db.pragma_update(None, "foreign_keys", "ON").unwrap();
            } else {
                db.execute_batch(sql).unwrap();
                db.pragma_update(None, "user_version", *version as i64)
                    .unwrap();
            }
        }
    }

    /// A scratch DB at user_version 20 with NL-shaped seed data.
    fn db_at_v020() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.pragma_update(None, "foreign_keys", "ON").unwrap();
        apply_migrations_upto(&db, 20);
        assert_eq!(int_of(&db, "PRAGMA user_version"), 20);
        db.execute_batch(
            "INSERT INTO company (name, kvk, legal_form, btw_id)
                 VALUES ('Test BV', '12345678', 'bv', 'NL123456789B01');
             INSERT INTO accounts (code, name, type, normal_balance, rgs_code)
                 VALUES ('8000', 'Omzet', 'income', 'credit', 'WOMZ.80');
             -- a codeless account: the taxonomy backfill must cover it too
             INSERT INTO accounts (code, name, type, normal_balance)
                 VALUES ('1990', 'Ongenummerd', 'asset', 'debit');
             INSERT INTO vat_returns (type, period, status) VALUES ('OB', '2026-Q2', 'draft');
             INSERT INTO filings (type, period, filed_at)
                 VALUES ('JAARREKENING', '2025', '2026-01-01');",
        )
        .unwrap();
        db
    }

    fn columns_of(db: &Connection, table: &str) -> Vec<String> {
        let mut stmt = db.prepare(&format!("PRAGMA table_info({table})")).unwrap();
        stmt.query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    }

    fn text_of(db: &Connection, sql: &str) -> Option<String> {
        db.query_row(sql, [], |r| r.get::<_, Option<String>>(0))
            .unwrap()
    }

    fn int_of(db: &Connection, sql: &str) -> i64 {
        db.query_row(sql, [], |r| r.get(0)).unwrap()
    }

    #[test]
    fn migration_chain_upgrades_a_v020_database() {
        let db = db_at_v020();
        migrate(&db).unwrap(); // the real runner
        assert_eq!(int_of(&db, "PRAGMA user_version"), 26); // 021-026 chain

        // company: renamed identifier columns, jurisdiction columns, CHECK gone
        let cols = columns_of(&db, "company");
        for c in [
            "registration_id",
            "tax_id",
            "country",
            "base_currency",
            "locale",
            "profile_version",
        ] {
            assert!(cols.iter().any(|x| x == c), "company is missing {c}");
        }
        assert!(!cols.iter().any(|x| x == "kvk"));
        assert!(!cols.iter().any(|x| x == "btw_id"));

        assert_eq!(
            text_of(&db, "SELECT registration_id FROM company WHERE id=1").unwrap(),
            "12345678"
        );
        assert_eq!(
            text_of(&db, "SELECT tax_id FROM company WHERE id=1").unwrap(),
            "NL123456789B01"
        );
        assert_eq!(
            text_of(&db, "SELECT legal_form FROM company WHERE id=1").unwrap(),
            "bv"
        );
        assert_eq!(
            text_of(&db, "SELECT country FROM company WHERE id=1").unwrap(),
            "NL"
        );
        assert_eq!(
            text_of(&db, "SELECT base_currency FROM company WHERE id=1").unwrap(),
            "EUR"
        );
        assert_eq!(
            text_of(&db, "SELECT locale FROM company WHERE id=1").unwrap(),
            "nl"
        );
        assert_eq!(
            int_of(&db, "SELECT profile_version FROM company WHERE id=1"),
            1
        );

        // the legal_form CHECK is gone: a non-NL value is accepted
        db.execute_batch("UPDATE company SET legal_form = 'ltd' WHERE id = 1")
            .unwrap();
        assert_eq!(
            text_of(&db, "SELECT legal_form FROM company WHERE id=1").unwrap(),
            "ltd"
        );

        // accounts: rgs_code -> taxonomy_code rename + taxonomy backfill
        let acols = columns_of(&db, "accounts");
        assert!(acols.iter().any(|x| x == "taxonomy_code"));
        assert!(acols.iter().any(|x| x == "taxonomy"));
        assert!(!acols.iter().any(|x| x == "rgs_code"));
        assert_eq!(
            text_of(&db, "SELECT taxonomy_code FROM accounts WHERE code='8000'").unwrap(),
            "WOMZ.80"
        );
        assert_eq!(
            text_of(&db, "SELECT taxonomy FROM accounts WHERE code='8000'").unwrap(),
            "rgs"
        );
        // uniform discriminator: codeless rows get 'rgs' as well
        assert_eq!(
            text_of(
                &db,
                "SELECT taxonomy FROM accounts WHERE taxonomy_code IS NULL LIMIT 1"
            )
            .unwrap(),
            "rgs"
        );

        // vat_returns: type CHECK widened
        db.execute_batch(
            "INSERT INTO vat_returns (type, period, status) VALUES ('VAT', '2026-Q2', 'draft')",
        )
        .unwrap();
        assert_eq!(
            int_of(&db, "SELECT COUNT(*) FROM vat_returns WHERE type='VAT'"),
            1
        );
        assert_eq!(
            text_of(&db, "SELECT period FROM vat_returns WHERE type='OB'").unwrap(),
            "2026-Q2"
        );

        // filings: type CHECK widened
        db.execute_batch(
            "INSERT INTO filings (type, period, filed_at) VALUES ('VAT', '2026-Q2', '2026-08-14')",
        )
        .unwrap();
        assert_eq!(
            int_of(&db, "SELECT COUNT(*) FROM filings WHERE type='VAT'"),
            1
        );

        // postings untouched (decision §9.1.2: vat_code_id stays)
        let pcols = columns_of(&db, "postings");
        assert!(pcols.iter().any(|x| x == "vat_code_id"));
        assert!(pcols.iter().any(|x| x == "vat_amount_cents"));
    }

    #[test]
    fn migration_chain_keeps_company_data_lossless() {
        let db = db_at_v020();
        migrate(&db).unwrap();

        assert_eq!(
            text_of(&db, "SELECT name FROM company WHERE id=1").unwrap(),
            "Test BV"
        );
        assert_eq!(text_of(&db, "SELECT iban FROM company WHERE id=1"), None);
        assert_eq!(int_of(&db, "SELECT vat_module FROM company WHERE id=1"), 0);
        assert_eq!(int_of(&db, "SELECT kor_flag FROM company WHERE id=1"), 0);
        assert_eq!(
            text_of(&db, "SELECT fiscal_year_end FROM company WHERE id=1").unwrap(),
            "12-31"
        );
        assert_eq!(text_of(&db, "SELECT logo FROM company WHERE id=1"), None);
        assert!(text_of(&db, "SELECT created_at FROM company WHERE id=1").is_some());
        assert!(text_of(&db, "SELECT updated_at FROM company WHERE id=1").is_some());
        // no duplicate rows (the PK id=1 CHECK is preserved)
        assert_eq!(int_of(&db, "SELECT COUNT(*) FROM company"), 1);
    }

    // ── ported from test/audit.test.js (its migration-018/019 cases) ──

    fn table_names(db: &Connection) -> Vec<String> {
        let mut stmt = db
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap();
        stmt.query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .filter_map(|x| x.ok())
            .collect()
    }

    const SIG_COLUMNS: [&str; 6] = [
        "digest_hash",
        "sig_keyid",
        "sig_nonce",
        "sig_ts",
        "sig",
        "sig_status",
    ];

    #[test]
    fn a_fresh_db_gains_the_signature_columns_actor_keys_and_settings() {
        let db = open_db(":memory:").unwrap();
        let cols = columns_of(&db, "audit_log");
        for c in SIG_COLUMNS {
            assert!(cols.iter().any(|x| x == c), "{c} missing");
        }
        let tables = table_names(&db);
        assert!(tables.contains(&"actor_keys".to_string()));
        assert!(tables.contains(&"settings".to_string()));
        assert_eq!(int_of(&db, "PRAGMA user_version"), 26); // 001-026
    }

    #[test]
    fn actor_keys_gains_a_composite_actor_keyid_primary_key() {
        let db = open_db(":memory:").unwrap();
        let mut stmt = db.prepare("PRAGMA table_info(actor_keys)").unwrap();
        let mut pk: Vec<(i64, String)> = stmt
            .query_map([], |r| Ok((r.get::<_, i64>(5)?, r.get::<_, String>(1)?)))
            .unwrap()
            .filter_map(|x| x.ok())
            .filter(|(order, _)| *order > 0)
            .collect();
        pk.sort();
        let names: Vec<String> = pk.into_iter().map(|(_, n)| n).collect();
        assert_eq!(names, vec!["actor", "keyid"]);
    }

    #[test]
    fn a_v18_db_with_a_single_actor_keys_row_upgrades_without_data_loss() {
        let db = Connection::open_in_memory().unwrap();
        db.pragma_update(None, "foreign_keys", "ON").unwrap();
        apply_migrations_upto(&db, 18);
        let keyid = "ab".repeat(16);
        db.execute(
            "INSERT INTO actor_keys (actor, keyid, public_key, enrolled_at, revoked_at, revoked_reason)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                "agent:bartholomeus",
                keyid,
                "PUBKEY-PEM",
                "2026-08-10T00:00:00.000Z",
                "2026-08-10T01:00:00.000Z",
                "test"
            ],
        )
        .unwrap();

        migrate(&db).unwrap();

        let (got_keyid, public_key, reason): (String, String, Option<String>) = db
            .query_row(
                "SELECT keyid, public_key, revoked_reason FROM actor_keys
                 WHERE actor = 'agent:bartholomeus'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(got_keyid, keyid);
        assert_eq!(public_key, "PUBKEY-PEM");
        assert_eq!(reason.as_deref(), Some("test"));
        // the composite PK lets the same keyid be enrolled for a second actor
        db.execute(
            "INSERT INTO actor_keys (actor, keyid, public_key, enrolled_at) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![
                "agent:other",
                keyid,
                "PUBKEY-PEM-2",
                "2026-08-10T02:00:00.000Z"
            ],
        )
        .unwrap();
    }

    #[test]
    fn a_v17_db_keeps_legacy_audit_rows_as_unsigned() {
        let db = Connection::open_in_memory().unwrap();
        db.pragma_update(None, "foreign_keys", "ON").unwrap();
        apply_migrations_upto(&db, 17);
        // the audit_log at 017 is the legacy shape; 018 adds the sig columns
        db.execute(
            "INSERT INTO audit_log (actor, action, outcome) VALUES (?1, ?2, ?3)",
            rusqlite::params!["human:erik", "company.init", "ok"],
        )
        .unwrap();

        migrate(&db).unwrap();

        let cols = columns_of(&db, "audit_log");
        for c in SIG_COLUMNS {
            assert!(cols.iter().any(|x| x == c), "{c} missing");
        }
        let tables = table_names(&db);
        assert!(tables.contains(&"actor_keys".to_string()));
        assert!(tables.contains(&"settings".to_string()));

        let rows = crate::audit::list(&db, None, None, 50).unwrap();
        assert_eq!(rows[0]["actor"], "human:erik");
        assert_eq!(rows[0]["sig_status"], "unsigned");
        assert!(rows[0]["digest_hash"].is_null());
    }

    #[test]
    fn rerunning_migrate_on_the_current_version_is_a_noop() {
        let db = open_db(":memory:").unwrap();
        let before = int_of(&db, "PRAGMA user_version");
        migrate(&db).unwrap();
        assert_eq!(int_of(&db, "PRAGMA user_version"), before);
    }

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
