//! Special-meaning chart accounts (issue #5).
//!
//! A chart account can carry a ROLE — `debtors`, `creditors`, `bank`,
//! `revenue`, `vat_liability`, `equity` — and the engine resolves its default
//! postings from the FLAGGED account instead of the jurisdiction profile's
//! account CODE. A profile's code only makes sense for the profile's own
//! default chart: a book whose chart came from an import numbers its accounts
//! differently, and `invoice finalize` used to park the receivable on whatever
//! account 1200 happened to be (a fixed asset), silently and in balance.

use bukio::accounts::{
    get_account_by_code, resolve_special, seed_default_chart, set_account_role, VALID_ROLES,
};
use bukio::invoice::{create_invoice, finalize_invoice};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::process::Command;

// --- fixtures ---------------------------------------------------------------

fn company_db(vat: bool) -> Connection {
    let db = bukio::db::open_db(":memory:").unwrap();
    seed_default_chart(&db).unwrap();
    db.execute(
        "INSERT INTO company (name, registration_id, legal_form, tax_id, iban, address, postal_code, city, vat_module)
         VALUES ('Demo BV', '12345678', 'bv', 'NL123456789B01', 'NL91ABNA0417164300', 'Industrieweg 12', '2712 CD', 'Zoetermeer', ?1)",
        [vat as i64],
    )
    .unwrap();
    if vat {
        bukio::vat::enable_vat_module(&db, "agent:test").unwrap();
    }
    db
}

fn role(db: &Connection, role: &str) -> Option<String> {
    bukio::accounts::role_account(db, role)
}

/// An audit file with a legacy chart: 1200 is a fixed asset and Debiteuren
/// lives at 2200. `accounts` is passed in so both processing orders can be
/// tested — the flag has to move to 2200 either way.
fn auditfile(accounts: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<AuditFile xmlns="https://www.bukio.nl/xaf/4.0" version="4.0" exportedAt="2026-09-25T08:00:00Z">
  <Header>
    <AuditFileVersion>4.0</AuditFileVersion>
    <CompanyID>1</CompanyID>
    <CompanyName>Demo BV</CompanyName>
    <FiscalYear>2026</FiscalYear>
    <StartDate>2026-01-01</StartDate>
    <EndDate>2026-12-31</EndDate>
    <CurrencyCode>EUR</CurrencyCode>
    <SoftwareDescription>Legacy package</SoftwareDescription>
  </Header>
  <MasterFiles>
    <GeneralLedgerAccounts>{accounts}</GeneralLedgerAccounts>
  </MasterFiles>
  <GeneralLedgerEntries/>
</AuditFile>"#
    )
}

const MACHINE: &str =
    "<Account><AccountID>1200</AccountID><AccountDescription>Machines en installaties</AccountDescription><AccountType>Asset</AccountType></Account>";
const DEBTORS: &str =
    "<Account><AccountID>2200</AccountID><AccountDescription>Debiteuren</AccountDescription><AccountType>Asset</AccountType></Account>";

// --- the chart ---------------------------------------------------------------

#[test]
fn the_default_chart_flags_every_special_account() {
    let db = company_db(false);
    assert_eq!(role(&db, "debtors").as_deref(), Some("1200"));
    assert_eq!(role(&db, "creditors").as_deref(), Some("2000"));
    assert_eq!(role(&db, "bank").as_deref(), Some("1100"));
    assert_eq!(role(&db, "revenue").as_deref(), Some("8000"));
    assert_eq!(role(&db, "equity").as_deref(), Some("3000"));
    // `Kas` is cash, not the bank account; `Overige opbrengsten` is not revenue
    assert!(get_account_by_code(&db, "1000").unwrap()["role"].is_null());
    assert!(get_account_by_code(&db, "8100").unwrap()["role"].is_null());

    // the VAT accounts come into being with `vat enable`, flagged the same way
    let db = company_db(true);
    assert_eq!(role(&db, "vat_liability").as_deref(), Some("2500"));
    // "Te vorderen omzetbelasting" is the VAT receivable, not the liability
    assert!(get_account_by_code(&db, "1500").unwrap()["role"].is_null());

    // every role is held by exactly one ACTIVE account
    for r in VALID_ROLES {
        let holder = role(&db, r).unwrap_or_else(|| panic!("no account flagged {r}"));
        let acct = get_account_by_code(&db, &holder).unwrap();
        assert_eq!(acct["active"], true, "{holder} is inactive but flagged {r}");
    }
}

#[test]
fn one_account_per_role_and_explicit_flags_are_validated() {
    let db = company_db(false);

    let err = set_account_role(&db, "1600", Some("debtors"), false).unwrap_err();
    assert_eq!(err.code, "ROLE_TAKEN");
    let err = set_account_role(&db, "1600", Some("revenue"), false).unwrap_err();
    assert_eq!(err.code, "ROLE_TAKEN");
    let err = set_account_role(&db, "9999", Some("bank"), false).unwrap_err();
    assert_eq!(err.code, "ACCOUNT_NOT_FOUND");
    let err = set_account_role(&db, "1600", Some("sky"), false).unwrap_err();
    assert_eq!(err.code, "INVALID_ROLE");

    // dry-run validates the same input and writes nothing
    set_account_role(&db, "1200", None, true).unwrap();
    assert_eq!(role(&db, "debtors").as_deref(), Some("1200"));
}

#[test]
fn the_resolver_trusts_the_flag_then_a_matching_profile_code_then_the_chart() {
    let db = company_db(false);

    // 1. while the chart IS the profile's chart, its code is right
    assert_eq!(resolve_special(&db, "debtors", Some("1200")).as_deref(), Some("1200"));

    // 2. an explicit flag beats the profile code
    set_account_role(&db, "1200", None, false).unwrap();
    set_account_role(&db, "1600", Some("debtors"), false).unwrap();
    assert_eq!(resolve_special(&db, "debtors", Some("1200")).as_deref(), Some("1600"));

    // 3. a profile code that is NOT that role never wins over the chart
    set_account_role(&db, "1600", None, false).unwrap();
    db.execute(
        "UPDATE accounts SET name = 'Machines en installaties' WHERE code = '1200'",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO accounts (code, name, type, normal_balance) VALUES ('2200', 'Debiteuren', 'asset', 'debit')",
        [],
    )
    .unwrap();
    assert_eq!(resolve_special(&db, "debtors", Some("1200")).as_deref(), Some("2200"));

    // 4. nothing matches → the profile's code stays the last resort
    assert_eq!(
        resolve_special(&db, "some-role-nobody-has", Some("1800")).as_deref(),
        Some("1800")
    );
    assert_eq!(resolve_special(&db, "some-role-nobody-has", None), None);
}

// --- the issue -------------------------------------------------------------

/// The reported bug, and the wider class it belongs to: every default the
/// engine derives from the chart has to survive a foreign chart.
#[test]
fn invoice_finalize_books_the_receivable_to_the_charts_debtors_account() {
    for accounts in [
        format!("{MACHINE}{DEBTORS}"),
        format!("{DEBTORS}{MACHINE}"), // the harder order: the flag is taken
                                       // when 2200 is created, and freed later
    ] {
        let db = company_db(true);
        bukio::import_mod::import_xaf(&db, &auditfile(&accounts), "agent:test", false).unwrap();

        // 1200 changed meaning, so it may not keep the flag; 2200 owns it now
        assert_eq!(
            get_account_by_code(&db, "1200").unwrap()["name"],
            "Machines en installaties"
        );
        assert!(get_account_by_code(&db, "1200").unwrap()["role"].is_null());
        assert_eq!(role(&db, "debtors").as_deref(), Some("2200"));

        let contact = bukio::contacts::create_contact(
            &db,
            "ACME B.V.",
            Some("Straat 1"),
            Some("1000 AA"),
            Some("Amsterdam"),
            None,
            None,
            None,
            None,
            None,
            "agent:test",
            false,
        )
        .unwrap();
        let inv = create_invoice(
            &db,
            contact["id"].as_i64().unwrap(),
            "2026-07-10",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            &[json!("2x Consultancy @ 150.00 @21")],
            "agent:test",
            false,
        )
        .unwrap();
        let res = finalize_invoice(&db, inv["id"].as_i64().unwrap(), "agent:test", false).unwrap();
        let entry =
            bukio::entries::get_entry(&db, res["entry"]["id"].as_i64().unwrap()).unwrap();

        let receivable = entry
            .postings
            .iter()
            .find(|p| p.account_code == "2200")
            .expect("the receivable did not land on the chart's Debiteuren");
        assert_eq!(receivable.amount_cents, 36300); // 2 x 150.00 + 21% VAT
        // and never on the fixed asset the profile's code names
        assert!(entry.postings.iter().all(|p| p.account_code != "1200"));
        // the sales and output-VAT legs are chart-driven too
        assert!(entry.postings.iter().any(|p| p.account_code == "8000"));
        assert!(entry.postings.iter().any(|p| p.account_code == "2500"));
    }
}

// --- the CLI ----------------------------------------------------------------

fn run(args: &[&str], db: &std::path::Path, cfg: &std::path::Path) -> Value {
    let out = Command::new(env!("CARGO_BIN_EXE_bukio"))
        .args(args)
        .env("BUKIO_DB", db)
        .env("BUKIO_CONFIG_DIR", cfg)
        .env("BUKIO_ACTOR", "agent:test")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    serde_json::from_str(&stdout).unwrap_or_else(|_| {
        panic!("not JSON from {args:?}: {}", &stdout.chars().take(300).collect::<String>())
    })
}

#[test]
fn cli_flags_an_account_and_refuses_a_second_claim_before_writing() {
    let dir = std::env::temp_dir().join(format!("bukio-account-role-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join("book.db");
    let cfg = dir.join("config");

    run(&["init", "--name", "Test Coaching", "--country", "NL", "--json"], &db, &cfg);

    // the seeded 1200 holds 'debtors': a second claim is refused BEFORE the
    // account is created (no half-written chart)
    let refused = run(
        &[
            "account", "add", "--code", "2200", "--name", "Debiteuren", "--type", "asset",
            "--normal-balance", "debit", "--role", "debtors", "--json",
        ],
        &db,
        &cfg,
    );
    assert_eq!(refused["error"]["code"], "ROLE_TAKEN");
    let show = run(&["account", "show", "--code", "2200", "--json"], &db, &cfg);
    assert_eq!(show["error"]["code"], "ACCOUNT_NOT_FOUND");

    // dry-run rejects a bad role without writing either
    let bad = run(
        &[
            "account", "add", "--code", "2300", "--name", "Overig", "--type", "asset",
            "--normal-balance", "debit", "--role", "sky", "--dry-run", "--json",
        ],
        &db,
        &cfg,
    );
    assert_eq!(bad["error"]["code"], "INVALID_ROLE");

    // clear the seeded flag, then claim it
    let cleared = run(
        &["account", "set-role", "--code", "1200", "--clear", "--json"],
        &db,
        &cfg,
    );
    assert_eq!(cleared["data"]["account"]["role"], Value::Null);
    let added = run(
        &[
            "account", "add", "--code", "2200", "--name", "Debiteuren", "--type", "asset",
            "--normal-balance", "debit", "--role", "debtors", "--json",
        ],
        &db,
        &cfg,
    );
    assert_eq!(added["data"]["role"], "debtors");
    // the flag is on the account, and only there
    let listed = run(&["account", "list", "--json"], &db, &cfg);
    let flagged: Vec<String> = listed["data"]["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|a| a["role"] == json!("debtors"))
        .filter_map(|a| a["code"].as_str().map(String::from))
        .collect();
    assert_eq!(flagged, vec!["2200"]);

    let _ = std::fs::remove_dir_all(&dir);
}
