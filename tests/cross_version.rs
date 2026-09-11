//! Compatibility with the Node implementation — the gate for the Rust-only
//! release.
//!
//! `tests/fixtures/node-0.17-book.db` was written end to end by bukio-cli 0.17
//! (the Node version): init, company profile, a contact, two VAT-booked sales
//! entries and one finalized invoice. This suite opens that book with the Rust
//! binary and asserts what it reads, so an upgrade path that only works in
//! theory fails here instead.
//!
//! Regenerating the fixture (only if the schema ever legitimately changes —
//! RELEASING.md says it should not, because that is what makes rollback safe):
//!
//! ```text
//! BUKIO_CONFIG_DIR=<tmp>/cfg BUKIO_DB=<tmp>/book.db BUKIO_ACTOR=agent:fixture \
//!   node <node-checkout>/bin/bukio.js init --name "Noordwind Handel BV" \
//!     --registration-id 81234567 --legal-form bv --vat on --json
//!   # … then: company update (address/tax-id/iban), contact add,
//!   #          vat book --postings "1100:1210.00,8000:-1000.00@21" --post
//!   #          vat book --postings "1100:242.00,8000:-200.00@21" --post
//!   #          invoice create --contact 1 --lines "2x Advies @ 95.00 @21"
//!   #          invoice finalize --id 1
//! ```
//!
//! The values asserted below were confirmed identical when read by the Node
//! binary itself: 8 of 9 read commands are byte-for-byte the same. The ninth,
//! `invoice ubl`, differs by one insignificant whitespace byte before
//! `<cac:PartyName>` (identical XML; a formatting detail, not a data one).

use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;

const ACTOR: &[&str] = &["--actor", "agent:fixture"];

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/node-0.17-book.db")
}

/// A private copy of the Node book plus a throwaway config dir, so the suite can
/// never touch the fixture itself or the operator's real installation.
fn node_book(tag: &str) -> (PathBuf, PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("bukio-xver-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join("book.db");
    std::fs::copy(fixture(), &db).unwrap();
    let cfg = dir.join("config");
    std::fs::create_dir_all(&cfg).unwrap();
    (dir, db, cfg)
}

fn bukio(args: &[&str], db: &PathBuf, cfg: &PathBuf) -> Value {
    let out = Command::new(env!("CARGO_BIN_EXE_bukio"))
        .args(args)
        .env("BUKIO_DB", db)
        .env("BUKIO_CONFIG_DIR", cfg)
        .env("BUKIO_LOCALE", "en")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: Value = serde_json::from_str(&stdout).unwrap_or_else(|_| {
        panic!(
            "not JSON from {args:?}: {}",
            &stdout.chars().take(300).collect::<String>()
        )
    });
    assert!(out.status.success(), "{args:?} failed: {v}");
    v
}

#[test]
fn a_book_written_by_the_node_version_reads_identically() {
    let (_dir, db, cfg) = node_book("reads");

    // the company record the Node version created
    let company = bukio(&["company", "show", "--json"], &db, &cfg);
    assert_eq!(company["data"]["company"]["name"], "Noordwind Handel BV");

    // entries: two VAT-booked sales plus the invoice booking
    let entries = bukio(&["entry", "list", "--limit", "20", "--json"], &db, &cfg);
    assert_eq!(
        entries["data"]["entries"].as_array().unwrap().len(),
        3,
        "{entries}"
    );

    // the invoice, finalized by the Node version, with its own numbers
    let invoice = bukio(&["invoice", "show", "--id", "1", "--json"], &db, &cfg);
    let inv = &invoice["data"]["invoice"];
    assert_eq!(inv["invoice_number"], "2026-0001");
    assert_eq!(inv["net_cents"], 19000);
    assert_eq!(inv["vat_cents"], 3990);
    assert_eq!(inv["gross_cents"], 22990);

    // the audit trail the Node version appended is readable and attributed
    let audit = bukio(&["audit", "--limit", "50", "--json"], &db, &cfg);
    let rows = audit["data"]["entries"]
        .as_array()
        .or_else(|| audit["data"]["audit"].as_array())
        .expect("audit rows");
    assert!(
        rows.len() >= 5,
        "expected the Node audit history, got {} rows",
        rows.len()
    );
    assert!(
        rows.iter()
            .all(|r| r["actor"].as_str().unwrap_or("") == "agent:fixture"),
        "audit attribution lost across versions: {audit}"
    );
}

#[test]
fn reports_on_a_node_book_match_the_values_the_node_version_produced() {
    let (_dir, db, cfg) = node_book("reports");

    let tb = bukio(
        &["report", "trial-balance", "--year", "2026", "--json"],
        &db,
        &cfg,
    );
    assert_eq!(tb["data"]["balanced"], true, "{tb}");
    let nets: Vec<(String, i64)> = tb["data"]["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|a| a["net_cents"].as_i64().unwrap_or(0) != 0)
        .map(|a| {
            (
                a["code"].as_str().unwrap().to_string(),
                a["net_cents"].as_i64().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        nets,
        vec![
            ("1100".to_string(), 145200),  // bank
            ("1200".to_string(), 22990),   // debtors: the finalized invoice
            ("2500".to_string(), -29190),  // VAT payable
            ("8000".to_string(), -139000), // revenue
        ],
        "trial balance differs from what the Node version reported"
    );

    // the VAT position the Node version calculated for Q1
    let q1 = bukio(
        &["vat", "readout", "--period", "2026-Q1", "--json"],
        &db,
        &cfg,
    );
    assert_eq!(q1["data"]["fields"]["1a"]["cents"], 100000, "{q1}");
    assert_eq!(q1["data"]["fields"]["5a"]["cents"], 21000);
    assert_eq!(q1["data"]["to_pay_cents"], 21000);

    // and the year's result
    let pnl = bukio(&["report", "pnl", "--year", "2026", "--json"], &db, &cfg);
    assert_eq!(pnl["data"]["revenue_cents"], 139000, "{pnl}");
    assert_eq!(pnl["data"]["result_cents"], 139000);
}
