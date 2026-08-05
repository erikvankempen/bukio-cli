//! Foundation integration tests — the Rust port of the Phase 0/1 core,
//! mirroring the Node test suites (money, accounts, entries, audit).

use bukio_cli::core::accounts;
use bukio_cli::core::db;
use bukio_cli::core::entries::{self, parse_posting_specs};
use bukio_cli::core::money::{format_amount, parse_amount};
use bukio_cli::vat;

fn tmp_db_path(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("bukio-rs-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

/// Simulate `bukio init` (the core of it — company row + chart + audit).
fn init_company(path: &str, vat_on: bool) -> rusqlite::Connection {
    let conn = db::open_db(path).unwrap();
    conn.execute(
        "INSERT INTO company (name, legal_form, vat_module, kor_flag) VALUES ('Demo BV', 'bv', ?1, 0)",
        [if vat_on { 1 } else { 0 }],
    )
    .unwrap();
    accounts::seed_default_chart(&conn).unwrap();
    if vat_on {
        vat::enable_vat_module(&conn, "agent:test").unwrap();
    }
    bukio_cli::audit::record(&conn, "agent:test", "company.init", Some("init"), None, "ok", &[]).unwrap();
    conn
}

#[test]
fn money_parity_with_node_suite() {
    // mirrors test/money.test.js
    assert_eq!(parse_amount("0").unwrap(), 0);
    assert_eq!(parse_amount("0.5").unwrap(), 50);
    assert_eq!(parse_amount("1234").unwrap(), 123400);
    assert_eq!(parse_amount("1234.56").unwrap(), 123456);
    assert_eq!(parse_amount("-12.34").unwrap(), -1234);
    assert_eq!(parse_amount(" 42.10 ").unwrap(), 4210);
    for bad in ["", "abc", "1.234,56", "1.234", "12.345", "1.2.3", ".5", "5.", "1,5", "--5", "1e3", "NaN", "Infinity"] {
        assert_eq!(parse_amount(bad).unwrap_err().code(), "INVALID_AMOUNT");
    }
    assert_eq!(format_amount(0), "0.00");
    assert_eq!(format_amount(123456), "1234.56");
    assert_eq!(format_amount(-1234), "-12.34");
    assert_eq!(format_amount(5), "0.05");
}

#[test]
fn init_sets_up_company_and_chart() {
    let path = tmp_db_path("init.db");
    let _ = std::fs::remove_file(&path);
    let conn = init_company(path.to_str().unwrap(), true);
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM accounts", [], |r| r.get(0)).unwrap();
    assert_eq!(count, 30); // 28 + 1500 + 2500
    let vat_module: i64 = conn.query_row("SELECT vat_module FROM company", [], |r| r.get(0)).unwrap();
    assert_eq!(vat_module, 1);
    let codes: i64 = conn.query_row("SELECT COUNT(*) FROM vat_codes", [], |r| r.get(0)).unwrap();
    assert_eq!(codes, 8);
}

#[test]
fn db_file_is_compatible_with_node_schema() {
    // The schema (user_version=7, all tables + triggers) is the contract.
    let path = tmp_db_path("schema.db");
    let _ = std::fs::remove_file(&path);
    let conn = db::open_db(path.to_str().unwrap()).unwrap();
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
    assert_eq!(version, 7);
    let triggers: i64 = conn
        .query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='trigger'", [], |r| r.get(0))
        .unwrap();
    assert!(triggers >= 8, "expected the immutability/balance triggers, got {triggers}");
}

#[test]
fn full_entry_lifecycle_on_disk_db() {
    let path = tmp_db_path("lifecycle.db");
    let _ = std::fs::remove_file(&path);
    let conn = init_company(path.to_str().unwrap(), false);

    // draft -> post -> reverse, with audit trail
    let specs = parse_posting_specs(&["1100:10000.00,3000:-10000.00".to_string()]).unwrap();
    let e = entries::create_entry(&conn, "2026-08-05", "Startkapitaal", &specs, "manual", None, "agent:test").unwrap();
    assert_eq!(e.meta.state, "draft");
    assert_eq!(e.meta.created_by, "agent:test");

    let posted = entries::post_entry(&conn, e.meta.id, "agent:test").unwrap();
    assert_eq!(posted.meta.state, "posted");
    assert!(posted.meta.posted_at.is_some());

    let reversed = entries::reverse_entry(&conn, e.meta.id, "agent:test", Some("foutje")).unwrap();
    assert_eq!(reversed.meta.state, "posted");
    assert_eq!(reversed.meta.source, "reversal");
    assert_eq!(reversed.postings[0].amount_cents, -1000000);

    // books net to zero
    let sum: i64 = conn
        .query_row("SELECT SUM(amount_cents) FROM postings WHERE entry_id IN (SELECT id FROM journal_entries WHERE state='posted')", [], |r| r.get(0))
        .unwrap();
    assert_eq!(sum, 0);

    // audit log has 3 entries: create, post, reverse
    let rows = bukio_cli::audit::list(&conn, None, None, 10).unwrap();
    assert_eq!(rows.len(), 4); // + company.init
    assert_eq!(rows[0].action, "entry.reverse");
    assert_eq!(rows[0].entry_ids.len(), 2);
}

#[test]
fn unbalanced_entry_rejected() {
    let conn = db::open_in_memory().unwrap();
    accounts::seed_default_chart(&conn).unwrap();
    let specs = parse_posting_specs(&["1100:100.00,3000:-99.00".to_string()]).unwrap();
    let err = entries::create_entry(&conn, "2026-08-05", "x", &specs, "manual", None, "human").unwrap_err();
    assert_eq!(err.code(), "UNBALANCED");
}

#[test]
fn posted_entry_immutable_even_via_raw_sql() {
    let conn = db::open_in_memory().unwrap();
    accounts::seed_default_chart(&conn).unwrap();
    let specs = parse_posting_specs(&["1100:100.00,3000:-100.00".to_string()]).unwrap();
    let e = entries::create_entry(&conn, "2026-08-05", "x", &specs, "manual", None, "human").unwrap();
    entries::post_entry(&conn, e.meta.id, "human").unwrap();
    let err = conn
        .execute("UPDATE journal_entries SET description='hacked' WHERE id=?1", [e.meta.id])
        .unwrap_err();
    assert!(err.to_string().contains("cannot modify a posted entry"));
}
