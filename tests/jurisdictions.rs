//! Jurisdiction profile tests, ported from test/jurisdictions.test.js.
//!
//! The JS modules are the single source of truth for the profile *data*: the
//! port embeds the generated src/profiles.json, and test/profiles-drift.test.js
//! asserts that file still matches the JS (all 31 profiles, every field). What
//! these tests cover is that the port *uses* the data correctly — chart
//! seeding, taxonomy, strict dispatch, calendars, UBL schemes, audit formats.

use rusqlite::Connection;
use serde_json::{json, Value};
use std::path::PathBuf;

const CLI_IBAN: &str = "NL91ABNA0417164300";

fn bin() -> String {
    std::env::var("CARGO_BIN_EXE_bukio").unwrap_or_else(|_| "target/release/bukio".to_string())
}

fn tmp(tag: &str) -> (PathBuf, String) {
    let dir = std::env::temp_dir().join(format!("bukio-jur-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join("test.db").to_string_lossy().to_string();
    (dir, db)
}

/// Run the CLI; the JS helper appends --json and parses stdout.
fn cli(db: &str, args: &[&str]) -> Value {
    let out = std::process::Command::new(bin())
        .args(args)
        .args(["--db", db, "--json"])
        .env("BUKIO_ACTOR", "agent:test")
        .output()
        .expect("cli runs");
    serde_json::from_slice(&out.stdout).unwrap_or_else(|_| {
        json!({
            "ok": false,
            "error": { "code": "NO_JSON" },
            "stdout": String::from_utf8_lossy(&out.stdout),
            "stderr": String::from_utf8_lossy(&out.stderr),
        })
    })
}

fn code(v: &Value) -> String {
    v["error"]["code"].as_str().unwrap_or("").to_string()
}

fn open(db: &str) -> Connection {
    bukio::db::open_db(db).unwrap()
}

/// Create an invoice for contact 1 with the given line specs.
fn inv(db: &Connection, date: &str, lines: &[&str]) -> i64 {
    let raw: Vec<Value> = lines.iter().map(|l| json!(l)).collect();
    let v = bukio::invoice::create_invoice(
        db,
        1,
        date,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        &raw,
        "agent:test",
        false,
    )
    .unwrap();
    v["id"]
        .as_i64()
        .or_else(|| v["invoice"]["id"].as_i64())
        .unwrap_or_else(|| panic!("no invoice id in {v}"))
}

fn net(accounts: &Value, code: &str) -> Option<i64> {
    accounts
        .as_array()?
        .iter()
        .find(|a| a["code"] == json!(code))
        .and_then(|a| a["net_cents"].as_i64())
}

// --- profile registry -------------------------------------------------------

#[test]
fn jur_get_profile_returns_nl_for_any_case() {
    for cc in ["NL", "nl", " Nl "] {
        let p = bukio::accounts::get_profile(cc).unwrap();
        assert_eq!(p["meta"]["country"], json!("NL"), "{cc}");
        assert_eq!(p["meta"]["baseCurrency"], json!("EUR"));
        assert_eq!(p["meta"]["locale"], json!("nl"));
    }
}

#[test]
fn jur_get_profile_rejects_malformed_country() {
    for bad in ["NETHERLANDS", "N", "NLD", "NL!", "", " "] {
        assert_eq!(
            bukio::accounts::get_profile(bad).unwrap_err().code,
            "INVALID_COUNTRY",
            "'{bad}'"
        );
    }
}

#[test]
fn jur_get_profile_throws_profile_not_found_for_unknown_valid_codes() {
    assert_eq!(
        bukio::accounts::get_profile("ZZ").unwrap_err().code,
        "PROFILE_NOT_FOUND"
    );
    assert_eq!(
        bukio::accounts::get_profile("IS").unwrap_err().code,
        "PROFILE_NOT_FOUND"
    );
}

#[test]
fn jur_nl_profile_tax_section_matches_the_legacy_vat_module() {
    let p = bukio::accounts::get_profile("NL").unwrap();
    assert_eq!(p["tax"]["system"], json!("vat"));
    assert_eq!(p["tax"]["standardRateBp"], json!(2100));
    assert_eq!(p["tax"]["reverseChargeEffectiveRateBp"], json!(2100));
    assert_eq!(p["tax"]["smallBusinessScheme"], json!("kor"));
    let codes = p["tax"]["codes"].as_array().unwrap();
    assert_eq!(codes.len(), 8);
    for c in codes {
        assert!(
            ["standard", "exempt", "reverse", "margin", "private"]
                .contains(&c["type"].as_str().unwrap_or("")),
            "{c}"
        );
        assert!(c["rateBp"].is_number(), "{c}");
        assert!(c["description"].is_string(), "{c}");
    }
    let seq: Vec<&str> = codes.iter().map(|c| c["code"].as_str().unwrap()).collect();
    assert_eq!(seq, vec!["21", "9", "0", "V", "R", "RE", "M", "P"]);
    let ledger: Vec<&str> = p["tax"]["accounts"]["ledger"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["code"].as_str().unwrap())
        .collect();
    assert_eq!(ledger, vec!["1500", "2500"]);
    assert_eq!(p["tax"]["accounts"]["fileDefault"], json!("2510"));
    assert_eq!(p["tax"]["accounts"]["differenceDefault"], json!("4700"));
    assert_eq!(
        p["tax"]["accounts"]["settlementAccountName"],
        json!("Af te dragen omzetbelasting")
    );
}

#[test]
fn jur_nl_profile_reporting_section_matches_the_legacy_chart() {
    let p = bukio::accounts::get_profile("NL").unwrap();
    assert_eq!(p["reporting"]["taxonomy"], json!("rgs"));
    let chart = p["reporting"]["defaultChart"].as_array().unwrap();
    assert_eq!(chart.len(), 29);
    for a in chart {
        assert!(
            ["asset", "liability", "equity", "income", "expense"]
                .contains(&a["type"].as_str().unwrap_or("")),
            "{a}"
        );
        assert!(
            ["debit", "credit"].contains(&a["normalBalance"].as_str().unwrap_or("")),
            "{a}"
        );
        assert!(
            a["taxonomyCode"]
                .as_str()
                .map(|s| !s.is_empty())
                .unwrap_or(false),
            "{a}"
        );
    }
    assert_eq!(p["reporting"]["labels"].as_object().unwrap().len(), 16);
    assert_eq!(p["reporting"]["labels"]["WOMZ.80"], json!("Omzet"));
    assert_eq!(
        p["reporting"]["statutoryAccounts"]["models"],
        json!(["micro", "klein"])
    );
    let lines = &p["reporting"]["statutoryAccounts"]["lines"];
    assert_eq!(lines["activa"].as_array().unwrap().len(), 6);
    assert_eq!(lines["passiva"].as_array().unwrap().len(), 4);
    assert_eq!(lines["pnl"].as_array().unwrap().len(), 7);
}

#[test]
fn jur_nl_profile_identifiers_compliance_documents_closing() {
    let p = bukio::accounts::get_profile("NL").unwrap();
    assert_eq!(p["identifiers"]["companyIdLabel"], json!("registration_id"));
    assert_eq!(p["identifiers"]["vatIdLabel"], json!("tax_id"));
    assert_eq!(p["identifiers"]["peppolSchemeId"], json!("9944"));
    assert_eq!(p["identifiers"]["accountNumber"]["kind"], json!("iban"));
    assert_eq!(
        p["meta"]["legalForms"],
        json!(["eenmanszaak", "vof", "bv", "nv", "stichting", "vereniging"])
    );
    let types: Vec<&str> = p["compliance"]["filingTypes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["type"].as_str().unwrap())
        .collect();
    assert_eq!(types, vec!["OB", "ICP", "JAARREKENING"]);
    assert_eq!(
        p["compliance"]["filingTypes"][0]["deadlineRule"],
        json!("nl-quarterly")
    );
    assert_eq!(
        p["compliance"]["filingTypes"][2]["deadlineRule"],
        json!("nl-13-months")
    );
    assert_eq!(
        p["documents"]["invoiceCompliance"],
        json!("nl-12-vereisten")
    );
    assert_eq!(p["documents"]["eInvoicing"], json!("peppol-bis-3.0"));
    assert_eq!(p["documents"]["auditFile"], json!("xaf-auditfile-4.0"));
    assert_eq!(p["exchange"]["fxSource"], json!("ecb"));
    assert_eq!(p["exchange"]["baseCurrency"], json!("EUR"));
    assert_eq!(p["closing"]["resultAccount"], json!("9900"));
    assert_eq!(p["closing"]["equityAccount"], json!("3000"));
    // the JS deep-freeze assertions have no Rust equivalent: profiles() hands out
    // &'static Value, so no consumer can mutate the registry at all.
}

// --- resolveProfile ---------------------------------------------------------

#[test]
fn jur_resolve_profile_returns_nl_for_a_dutch_company() {
    let (_d, db) = tmp("rp-nl");
    cli(&db, &["init", "--name", "Test BV", "--country", "NL"]);
    let p = bukio::accounts::resolve_profile(&open(&db)).unwrap();
    assert_eq!(p["meta"]["country"], json!("NL"));
}

#[test]
fn jur_resolve_profile_defaults_to_nl_without_a_country() {
    let (_d, db) = tmp("rp-nocountry");
    cli(&db, &["init", "--name", "Test BV"]);
    // a fresh company that never set a country (the port's schema is
    // NOT NULL DEFAULT 'NL', so there is no NULL state to reproduce)
    let c = open(&db);
    let stored: Option<String> = c
        .query_row("SELECT country FROM company WHERE id = 1", [], |r| r.get(0))
        .unwrap();
    // the port's schema defaults the column to NL at init instead of leaving it
    // NULL, so the JS pre-021 fallback is unobservable; what matters is that an
    // unset country behaves as NL
    assert!(
        stored
            .as_deref()
            .map(|s| s.is_empty() || s == "NL")
            .unwrap_or(true),
        "{stored:?}"
    );
    assert_eq!(
        bukio::accounts::resolve_profile(&c).unwrap()["meta"]["country"],
        json!("NL")
    );
}

#[test]
fn jur_resolve_profile_defaults_to_nl_when_no_company_row_exists() {
    let (_d, db) = tmp("rp-nocompany");
    cli(&db, &["init", "--name", "Test BV"]);
    let c = open(&db);
    c.execute("DELETE FROM company", []).unwrap();
    assert_eq!(
        bukio::accounts::resolve_profile(&c).unwrap()["meta"]["country"],
        json!("NL")
    );
}

#[test]
fn jur_resolve_profile_rejects_an_unsupported_company_country() {
    for cc in ["IS", "ZZ"] {
        let (_d, db) = tmp(&format!("rp-bad-{cc}"));
        cli(&db, &["init", "--name", "Test BV"]);
        let c = open(&db);
        c.execute("UPDATE company SET country = ?1 WHERE id = 1", [cc])
            .unwrap();
        assert_eq!(
            bukio::accounts::resolve_profile(&c).unwrap_err().code,
            "PROFILE_NOT_FOUND",
            "{cc}"
        );
    }
}

// --- M3: init --country + generic identifier flags (CLI level) --------------

#[test]
fn jur_init_rejects_a_country_without_a_profile() {
    for cc in ["IS", "ZZ"] {
        let (_d, db) = tmp("init-bad");
        let r = cli(&db, &["init", "--name", "Test BV", "--country", cc]);
        assert_eq!(code(&r), "PROFILE_NOT_FOUND", "{cc}: {r}");
    }
}

#[test]
fn jur_init_normalizes_a_lowercase_country_and_stores_the_profile_fields() {
    let (_d, db) = tmp("init-nl");
    let r = cli(
        &db,
        &[
            "init",
            "--name",
            "Test BV",
            "--country",
            "nl",
            "--vat",
            "on",
        ],
    );
    let c = &r["data"]["company"];
    assert_eq!(c["country"], json!("NL"), "{r}");
    assert_eq!(c["base_currency"], json!("EUR"));
    assert_eq!(c["locale"], json!("nl"));
    assert_eq!(c["profile_version"], json!(1));
    let show = cli(&db, &["company", "show"]);
    let c = &show["data"]["company"];
    assert_eq!(c["country"], json!("NL"));
    assert_eq!(c["base_currency"], json!("EUR"));
    assert_eq!(c["locale"], json!("nl"));
    assert_eq!(c["profile_version"], json!(1));
}

#[test]
fn jur_init_stores_generic_identifiers_without_warnings() {
    let (_d, db) = tmp("init-ids");
    let r = cli(
        &db,
        &[
            "init",
            "--name",
            "Test BV",
            "--registration-id",
            "12345678",
            "--tax-id",
            "NL123456789B01",
        ],
    );
    assert_eq!(
        r["data"]["company"]["registration_id"],
        json!("12345678"),
        "{r}"
    );
    assert_eq!(r["data"]["company"]["tax_id"], json!("NL123456789B01"));
    assert_eq!(r["data"]["warnings"], Value::Null, "no deprecation warning");
}

#[test]
fn jur_company_update_country_is_immutable() {
    let (_d, db) = tmp("immutable");
    cli(&db, &["init", "--name", "Test BV"]);
    let r = cli(&db, &["company", "update", "--country", "US"]);
    assert_eq!(code(&r), "COUNTRY_IMMUTABLE", "{r}");
}

#[test]
fn jur_company_update_country_same_value_passes() {
    let (_d, db) = tmp("same-country");
    cli(&db, &["init", "--name", "Test BV"]);
    let r = cli(
        &db,
        &[
            "company",
            "update",
            "--country",
            "nl",
            "--city",
            "Amsterdam",
        ],
    );
    assert_eq!(r["data"]["company"]["country"], json!("NL"), "{r}");
    assert_eq!(r["data"]["company"]["city"], json!("Amsterdam"));
}

#[test]
fn jur_company_update_registration_id() {
    let (_d, db) = tmp("upd-reg");
    cli(&db, &["init", "--name", "Test BV"]);
    let r = cli(&db, &["company", "update", "--registration-id", "87654321"]);
    assert_eq!(
        r["data"]["company"]["registration_id"],
        json!("87654321"),
        "{r}"
    );
    let r2 = cli(
        &db,
        &[
            "company",
            "update",
            "--registration-id",
            "11112222",
            "--tax-id",
            "NL999999999B01",
        ],
    );
    assert_eq!(r2["data"]["company"]["registration_id"], json!("11112222"));
    assert_eq!(r2["data"]["company"]["tax_id"], json!("NL999999999B01"));
    assert_eq!(r2["data"]["warnings"], Value::Null);
}

// --- M4..M9: the profile indirection is live in every command ---------------

#[test]
fn jur_profile_indirection_is_live_in_every_command() {
    let cases: Vec<(&str, Vec<&str>)> = vec![
        (
            "M4 vat readout",
            vec!["vat", "readout", "--period", "2026-Q2"],
        ),
        (
            "M5 jaarrekening",
            vec!["financial-statements", "report", "--year", "2025"],
        ),
        (
            "M6 compliance",
            vec!["compliance", "status", "--year", "2026"],
        ),
        ("M8 year-end", vec!["year-end", "close", "--year", "2026"]),
        (
            "M9 export xaf",
            vec![
                "export",
                "xaf",
                "--year",
                "2026",
                "--out",
                "/tmp/never-jur.xaf",
            ],
        ),
    ];
    for (name, args) in cases {
        let (_d, db) = tmp("indirect");
        cli(&db, &["init", "--name", "Test BV", "--vat", "on"]);
        let c = open(&db);
        c.execute("UPDATE company SET country = 'ZZ' WHERE id = 1", [])
            .unwrap();
        drop(c);
        let r = cli(&db, &args);
        assert_eq!(code(&r), "PROFILE_NOT_FOUND", "{name}: {r}");
    }
}

#[test]
fn jur_bank_import_resolves_the_profile() {
    let (dir, db) = tmp("bank-profile");
    cli(&db, &["init", "--name", "Test BV", "--vat", "on"]);
    let c = open(&db);
    c.execute("UPDATE company SET country = 'ZZ' WHERE id = 1", [])
        .unwrap();
    drop(c);
    let csv = dir.join("tx.csv");
    std::fs::write(
        &csv,
        "date,description,amount,iban\n2026-01-05,test,100.00,IBAN123\n",
    )
    .unwrap();
    let r = cli(
        &db,
        &[
            "bank",
            "import",
            "--file",
            &csv.to_string_lossy(),
            "--iban",
            "NL00BANK0123456789",
            "--dry-run",
        ],
    );
    assert_eq!(code(&r), "PROFILE_NOT_FOUND", "{r}");
}

#[test]
fn jur_ubl_resolves_the_profile() {
    let (_d, db) = tmp("ubl-profile");
    cli(
        &db,
        &[
            "init",
            "--name",
            "Test BV",
            "--registration-id",
            "12345678",
            "--tax-id",
            "NL123456789B01",
            "--vat",
            "on",
        ],
    );
    let c = open(&db);
    c.execute("UPDATE company SET country = 'ZZ' WHERE id = 1", [])
        .unwrap();
    let err = bukio::ubl::invoice_to_ubl(
        &c,
        &json!({ "invoice_type": "invoice", "lines": [], "contact": {} }),
    )
    .unwrap_err();
    assert_eq!(err.code, "PROFILE_NOT_FOUND");
}

#[test]
fn jur_account_add_accepts_a_taxonomy_code() {
    let (_d, db) = tmp("taxonomy-flag");
    cli(&db, &["init", "--name", "Test BV"]);
    let r = cli(
        &db,
        &[
            "account",
            "add",
            "--code",
            "1300",
            "--name",
            "Testrekening",
            "--type",
            "asset",
            "--normal-balance",
            "debit",
            "--taxonomy-code",
            "BMVA.02",
            "--dry-run",
        ],
    );
    assert_eq!(
        r["data"]["account"]["taxonomy_code"],
        json!("BMVA.02"),
        "{r}"
    );
}

// --- LU: profile, chart, strict dispatch, UBL -------------------------------

#[test]
fn jur_lu_profile_is_the_pcn_luxembourg_one() {
    let p = bukio::accounts::get_profile("LU").unwrap();
    assert_eq!(p["meta"]["country"], json!("LU"));
    assert_eq!(p["meta"]["baseCurrency"], json!("EUR"));
    assert_eq!(p["meta"]["locale"], json!("fr-lu"));
    let forms = p["meta"]["legalForms"].as_array().unwrap();
    assert!(forms.contains(&json!("sarl")));
    assert!(
        !forms.contains(&json!("bv")),
        "NL legal form must not be accepted for LU"
    );
    assert_eq!(p["identifiers"]["peppolSchemeId"], json!("0195"));
    assert_eq!(p["tax"]["standardRateBp"], json!(1700));
    assert_eq!(p["tax"]["smallBusinessScheme"], json!("franchise"));
    let codes: Vec<&str> = p["tax"]["codes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["code"].as_str().unwrap())
        .collect();
    assert_eq!(
        codes,
        vec!["17", "14", "8", "3", "0", "V", "R", "RE", "M", "P"]
    );
    let ledger: Vec<&str> = p["tax"]["accounts"]["ledger"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["code"].as_str().unwrap())
        .collect();
    assert_eq!(ledger, vec!["421611", "461411"]);
    assert_eq!(p["tax"]["accounts"]["fileDefault"], json!("461412"));
    assert_eq!(p["closing"]["resultAccount"], json!("142"));
    assert_eq!(p["closing"]["equityAccount"], json!("1411"));
    assert_eq!(
        p["tax"]["returnLayout"],
        Value::Null,
        "B1 scope: no eCDF layout"
    );
    assert_eq!(p["documents"]["auditFile"], json!("faia-2.01-reduced-b"));
    assert_eq!(
        p["documents"]["invoiceCompliance"],
        json!("lu-invoice-vereisten")
    );
    assert_eq!(p["documents"]["eInvoicing"], json!("peppol-bis-3.0"));
    assert_eq!(
        p["compliance"]["filingTypes"],
        json!([
            { "type": "TVA", "periodShape": "YYYY-Qn", "deadlineRule": "lu-quarterly" },
            { "type": "COMPTES_ANNUELS", "periodShape": "YYYY", "deadlineRule": "lu-7-months" },
        ])
    );
    assert_eq!(p["reporting"]["format"], json!("lu-lsc"));
    assert_eq!(
        p["reporting"]["statutoryAccounts"]["models"],
        json!(["abrege"])
    );
    assert!(
        p["reporting"]["statutoryAccounts"]["lines"]["activa"]
            .as_array()
            .unwrap()
            .len()
            >= 5
    );
    assert_eq!(
        p["exchange"]["paymentFormats"],
        json!(["sepa-pain.001", "sepa-pain.008"])
    );
    let chart = p["reporting"]["defaultChart"].as_array().unwrap();
    assert!(chart
        .iter()
        .any(|a| a["code"] == json!("516") && a["name"] == json!("Caisse")));
    assert!(chart
        .iter()
        .any(|a| a["code"] == json!("421611") && a["name"] == json!("TVA en amont")));
    assert!(chart
        .iter()
        .any(|a| a["code"] == json!("7021") && a["name"] == json!("Ventes de produits finis")));
}

#[test]
fn jur_lu_init_creates_a_french_company_with_the_pcn_chart() {
    let (_d, db) = tmp("lu-init");
    let r = cli(
        &db,
        &[
            "init",
            "--name",
            "Sàrl Test",
            "--country",
            "LU",
            "--legal-form",
            "sarl",
            "--vat",
            "on",
        ],
    );
    assert_eq!(r["data"]["company"]["country"], json!("LU"), "{r}");
    assert_eq!(r["data"]["company"]["base_currency"], json!("EUR"));
    assert_eq!(r["data"]["company"]["locale"], json!("fr-lu"));

    let c = open(&db);
    let (country, cur, locale, version): (String, String, String, i64) = c
        .query_row(
            "SELECT country, base_currency, locale, profile_version FROM company WHERE id = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        (country.as_str(), cur.as_str(), locale.as_str(), version),
        ("LU", "EUR", "fr-lu", 1)
    );
    let mut stmt = c
        .prepare("SELECT code, name, taxonomy FROM accounts WHERE active = 1")
        .unwrap();
    let rows: Vec<(String, String, String)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    drop(stmt);
    assert!(
        rows.iter().any(|(c, n, _)| c == "516" && n == "Caisse"),
        "{rows:?}"
    );
    assert!(rows
        .iter()
        .any(|(c, n, _)| c == "421611" && n == "TVA en amont"));
    assert!(rows.iter().any(|(c, _, _)| c == "7021"));
    assert!(rows.len() >= 40, "{} accounts", rows.len());
    for (code, _, taxonomy) in &rows {
        assert_eq!(
            taxonomy, "pcn",
            "account {code} carries the pcn discriminator"
        );
    }
    drop(c);

    let bad = cli(
        &tmp("lu-bad-form").1,
        &[
            "init",
            "--name",
            "X",
            "--country",
            "LU",
            "--legal-form",
            "bv",
        ],
    );
    assert_eq!(code(&bad), "INVALID_LEGAL_FORM", "{bad}");
    let kor = cli(
        &tmp("lu-kor").1,
        &[
            "init",
            "--name",
            "X",
            "--country",
            "LU",
            "--legal-form",
            "sarl",
            "--kor",
        ],
    );
    assert_eq!(code(&kor), "INVALID_VAT_CHOICE", "KOR is NL-only: {kor}");

    let (_dn, nldb) = tmp("nl-taxonomy");
    cli(&nldb, &["init", "--name", "Test BV", "--vat", "on"]);
    let nc = open(&nldb);
    let tax: String = nc
        .query_row(
            "SELECT taxonomy FROM accounts WHERE code = '1000'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(tax, "rgs", "NL accounts are unchanged");
}

#[test]
fn jur_lu_unregistered_formats_fail_loudly() {
    let (_d, db) = tmp("lu-dispatch");
    cli(
        &db,
        &[
            "init",
            "--name",
            "Sàrl Test",
            "--country",
            "LU",
            "--legal-form",
            "sarl",
            "--vat",
            "on",
        ],
    );
    let r = cli(&db, &["vat", "readout", "--period", "2026-Q1"]);
    assert_eq!(code(&r), "FORMAT_NOT_SUPPORTED", "no NL fallback: {r}");
}

#[test]
fn jur_lu_ubl_uses_the_rcs_scheme_and_country() {
    let (_d, db) = tmp("lu-ubl");
    cli(
        &db,
        &[
            "init",
            "--name",
            "Sàrl Test",
            "--country",
            "LU",
            "--legal-form",
            "sarl",
            "--registration-id",
            "B123456",
            "--tax-id",
            "LU12345678",
            "--vat",
            "on",
        ],
    );
    cli(
        &db,
        &[
            "company",
            "update",
            "--address",
            "1 rue du Test",
            "--postal-code",
            "L-1234",
            "--city",
            "Luxembourg",
        ],
    );
    cli(
        &db,
        &[
            "contact",
            "add",
            "--name",
            "Client SARL",
            "--address",
            "1 rue du Test",
            "--city",
            "Luxembourg",
            "--vat-id",
            "LU99999999",
            "--kvk",
            "B654321",
        ],
    );
    let c = open(&db);
    let id = inv(&c, "2026-08-15", &["1x Prestation @ 100.00 @17"]);
    bukio::invoice::finalize_invoice(&c, id, "agent:test", false).unwrap();
    let invoice = bukio::invoice::get_invoice(&c, id).unwrap().unwrap();
    let xml = bukio::ubl::invoice_to_ubl(&c, &invoice).unwrap();
    assert!(
        xml.contains("schemeID=\"0195\""),
        "endpoints use the RCS scheme: {xml}"
    );
    assert!(
        !xml.contains("schemeID=\"9944\""),
        "never the Dutch KVK scheme"
    );
    assert!(
        xml.contains("<cbc:IdentificationCode>LU</cbc:IdentificationCode>"),
        "country LU: {xml}"
    );
    assert!(xml.contains("LU99999999"), "buyer carries the LU TVA id");
}

// --- B6: LU invoice compliance ---------------------------------------------

#[test]
fn jur_lu_invoice_finalizes_end_to_end() {
    let (_d, db) = tmp("lu-finalize");
    cli(
        &db,
        &[
            "init",
            "--name",
            "Sàrl Test",
            "--country",
            "LU",
            "--legal-form",
            "sarl",
            "--registration-id",
            "B123456",
            "--tax-id",
            "LU12345678",
            "--vat",
            "on",
        ],
    );
    cli(
        &db,
        &[
            "company",
            "update",
            "--address",
            "1 rue du Test",
            "--postal-code",
            "L-1234",
            "--city",
            "Luxembourg",
        ],
    );
    cli(
        &db,
        &[
            "contact",
            "add",
            "--name",
            "Client SARL",
            "--address",
            "1 rue du Test",
            "--city",
            "Luxembourg",
            "--vat-id",
            "LU99999999",
        ],
    );
    let c = open(&db);
    let id = inv(&c, "2026-08-15", &["1x Prestation @ 100.00 @17"]);
    let fin = bukio::invoice::finalize_invoice(&c, id, "agent:test", false).unwrap();
    let invoice = fin.get("invoice").cloned().unwrap_or(fin.clone());
    assert_eq!(invoice["invoice_number"], json!("2026-0001"), "{fin}");
    assert_eq!(invoice["status"], json!("sent"));
    let entry_id = invoice["entry_id"].as_i64().unwrap();
    let entry = bukio::entries::get_entry(&c, entry_id).unwrap();
    assert_eq!(entry.state, "posted");
    let mut legs: Vec<String> = entry
        .postings
        .iter()
        .map(|p| format!("{}:{}", p.account_code, p.amount_cents))
        .collect();
    legs.sort();
    let joined = legs.join(" ");
    for want in ["4011:11700", "7021:-10000", "461411:-1700"] {
        assert!(joined.contains(want), "missing {want} in {joined}");
    }
}

#[test]
fn jur_lu_supplier_requirements_fail_with_french_messages() {
    let (_d, db) = tmp("lu-supplier");
    cli(
        &db,
        &[
            "init",
            "--name",
            "Sàrl Test",
            "--country",
            "LU",
            "--legal-form",
            "sarl",
            "--vat",
            "on",
        ],
    );
    cli(
        &db,
        &[
            "company",
            "update",
            "--address",
            "1 rue du Test",
            "--postal-code",
            "L-1234",
            "--city",
            "Luxembourg",
        ],
    );
    cli(
        &db,
        &[
            "contact",
            "add",
            "--name",
            "Client SARL",
            "--address",
            "1 rue du Test",
            "--city",
            "Luxembourg",
            "--vat-id",
            "LU99999999",
        ],
    );
    let c = open(&db);
    let id = inv(&c, "2026-08-15", &["1x Prestation @ 100.00 @17"]);
    let e1 = bukio::invoice::finalize_invoice(&c, id, "agent:test", false).unwrap_err();
    assert_eq!(e1.code, "SUPPLIER_INCOMPLETE", "{e1:?}");
    assert!(e1.message.contains("numéro RCS"), "{}", e1.message);
    c.execute(
        "UPDATE company SET registration_id = 'B123456' WHERE id = 1",
        [],
    )
    .unwrap();
    let e2 = bukio::invoice::finalize_invoice(&c, id, "agent:test", false).unwrap_err();
    assert_eq!(e2.code, "SUPPLIER_INCOMPLETE");
    assert!(e2.message.contains("numéro de TVA"), "{}", e2.message);
}

#[test]
fn jur_lu_reverse_charge_requires_the_customer_vat_number() {
    let (_d, db) = tmp("lu-reverse");
    cli(
        &db,
        &[
            "init",
            "--name",
            "Sàrl Test",
            "--country",
            "LU",
            "--legal-form",
            "sarl",
            "--registration-id",
            "B123456",
            "--tax-id",
            "LU12345678",
            "--vat",
            "on",
        ],
    );
    cli(
        &db,
        &[
            "company",
            "update",
            "--address",
            "1 rue du Test",
            "--postal-code",
            "L-1234",
            "--city",
            "Luxembourg",
        ],
    );
    cli(
        &db,
        &[
            "contact",
            "add",
            "--name",
            "Client SARL",
            "--address",
            "1 rue du Test",
            "--city",
            "Luxembourg",
        ],
    );
    let c = open(&db);
    let id = inv(&c, "2026-08-15", &["1x Prestation @ 100.00 @RE"]);
    let err = bukio::invoice::finalize_invoice(&c, id, "agent:test", false).unwrap_err();
    assert_eq!(err.code, "CUSTOMER_VAT_REQUIRED", "{err:?}");
    assert!(err.message.contains("auto-liquidation"), "{}", err.message);
    c.execute(
        "UPDATE contacts SET vat_id = 'FR12345678901' WHERE id = 1",
        [],
    )
    .unwrap();
    let fin = bukio::invoice::finalize_invoice(&c, id, "agent:test", false).unwrap();
    let invoice = fin.get("invoice").cloned().unwrap_or(fin.clone());
    assert_eq!(invoice["status"], json!("sent"));
}

#[test]
fn jur_nl_invoice_compliance_is_unchanged() {
    let (_d, db) = tmp("nl-compliance");
    cli(
        &db,
        &[
            "init",
            "--name",
            "Test BV",
            "--registration-id",
            "12345678",
            "--tax-id",
            "NL123456789B01",
            "--vat",
            "on",
        ],
    );
    cli(
        &db,
        &[
            "company",
            "update",
            "--address",
            "Industrieweg 12",
            "--postal-code",
            "2712 CD",
            "--city",
            "Zoetermeer",
        ],
    );
    cli(
        &db,
        &[
            "contact",
            "add",
            "--name",
            "ACME B.V.",
            "--address",
            "Straat 1",
            "--city",
            "Amsterdam",
            "--vat-id",
            "NL999999999B01",
        ],
    );
    let c = open(&db);
    let id = inv(&c, "2026-08-15", &["2x Consultancy @ 150.00 @21"]);
    bukio::invoice::finalize_invoice(&c, id, "agent:test", false).unwrap();
    let invoice = bukio::invoice::get_invoice(&c, id).unwrap().unwrap();
    bukio::invoice::validate_compliance(&c, &invoice).unwrap();
    assert_eq!(
        bukio::accounts::resolve_profile(&c).unwrap()["documents"]["invoiceCompliance"],
        json!("nl-12-vereisten")
    );
    assert_eq!(invoice["status"], json!("sent"));
}

// --- B2: LU statutory accounts ---------------------------------------------

#[test]
fn jur_lu_financial_statements_report_the_lsc_abridged_layout() {
    let (_d, db) = tmp("lu-lsc");
    cli(
        &db,
        &[
            "init",
            "--name",
            "Sàrl Test",
            "--country",
            "LU",
            "--legal-form",
            "sarl",
            "--registration-id",
            "B123456",
            "--tax-id",
            "LU12345678",
            "--vat",
            "on",
        ],
    );
    cli(
        &db,
        &[
            "company",
            "update",
            "--address",
            "1 rue du Test",
            "--postal-code",
            "L-1234",
            "--city",
            "Luxembourg",
        ],
    );
    cli(
        &db,
        &[
            "contact",
            "add",
            "--name",
            "Client SARL",
            "--address",
            "1 rue du Test",
            "--city",
            "Luxembourg",
            "--vat-id",
            "LU99999999",
        ],
    );
    let c = open(&db);
    let id = inv(&c, "2026-08-15", &["1x Prestation @ 100.00 @17"]);
    bukio::invoice::finalize_invoice(&c, id, "agent:test", false).unwrap();
    drop(c);

    let r = cli(
        &db,
        &[
            "financial-statements",
            "report",
            "--year",
            "2026",
            "--format",
            "json",
        ],
    );
    let fs = &r["data"]["financial_statements"];
    assert_eq!(fs["model"], json!("abrege"), "profile-driven default: {r}");
    assert_eq!(fs["as_of"], json!("2026-12-31"));
    assert_eq!(fs["balans"]["balanced"], json!(true));
    let has = |arr: &Value, label: &str, total: i64| -> bool {
        arr.as_array()
            .map(|a| {
                a.iter()
                    .any(|l| l["label"] == json!(label) && l["total_cents"] == json!(total))
            })
            .unwrap_or(false)
    };
    assert!(
        has(&fs["balans"]["activa"], "Actif circulant", 11700),
        "{fs}"
    );
    assert!(has(&fs["balans"]["passiva"], "Dettes", 1700), "{fs}");
    assert!(
        has(&fs["balans"]["passiva"], "Capitaux propres", 10000),
        "{fs}"
    );
    assert!(
        has(&fs["pnl"]["lines"], "Chiffre d'affaires net", 10000),
        "{fs}"
    );
    assert_eq!(fs["pnl"]["resultat_cents"], json!(10000));
}

#[test]
fn jur_lu_pnl_reconciles_a_mixed_leftover() {
    let (_d, db) = tmp("lu-mixed");
    cli(
        &db,
        &[
            "init",
            "--name",
            "Sàrl Test",
            "--country",
            "LU",
            "--legal-form",
            "sarl",
            "--vat",
            "on",
        ],
    );
    cli(
        &db,
        &[
            "account",
            "add",
            "--code",
            "6600",
            "--name",
            "Autres charges",
            "--type",
            "expense",
            "--normal-balance",
            "debit",
        ],
    );
    cli(
        &db,
        &[
            "account",
            "add",
            "--code",
            "7700",
            "--name",
            "Autres produits",
            "--type",
            "income",
            "--normal-balance",
            "credit",
        ],
    );
    cli(
        &db,
        &[
            "entry", "add", "--date", "2026-06-30", "--desc", "exercice", "--postings",
            "101:-1000,5131:1000,4011:11700,7021:-10000,461411:-1700,6600:500,5131:-500,7700:-200,5131:200", "--post",
        ],
    );
    let r = cli(
        &db,
        &[
            "financial-statements",
            "report",
            "--year",
            "2026",
            "--format",
            "json",
        ],
    );
    let fs = &r["data"]["financial_statements"];
    assert_eq!(fs["balans"]["balanced"], json!(true), "{fs}");
    assert!(has_line(&fs["pnl"]["lines"], "Autres", 70000), "{fs}");
    assert_eq!(
        fs["pnl"]["resultat_cents"],
        json!(970000),
        "resultat reconciles for mixed leftovers"
    );
}

#[test]
fn jur_lu_pnl_puts_73x_subventions_on_line_four() {
    let (_d, db) = tmp("lu-subventions");
    cli(
        &db,
        &[
            "init",
            "--name",
            "Sàrl Test",
            "--country",
            "LU",
            "--legal-form",
            "sarl",
            "--vat",
            "on",
        ],
    );
    cli(
        &db,
        &[
            "account",
            "add",
            "--code",
            "7300",
            "--name",
            "Subventions d'exploitation",
            "--type",
            "income",
            "--normal-balance",
            "credit",
        ],
    );
    cli(
        &db,
        &[
            "account",
            "add",
            "--code",
            "6600",
            "--name",
            "Autres charges",
            "--type",
            "expense",
            "--normal-balance",
            "debit",
        ],
    );
    cli(
        &db,
        &[
            "entry", "add", "--date", "2026-06-30", "--desc", "exercice", "--postings",
            "101:-1000,5131:1000,4011:11700,7021:-10000,461411:-1700,6600:500,5131:-500,7300:-200,5131:200", "--post",
        ],
    );
    let r = cli(
        &db,
        &[
            "financial-statements",
            "report",
            "--year",
            "2026",
            "--format",
            "json",
        ],
    );
    let fs = &r["data"]["financial_statements"];
    assert_eq!(fs["balans"]["balanced"], json!(true), "{fs}");
    assert!(
        has_line(&fs["pnl"]["lines"], "Autres produits d'exploitation", 20000),
        "{fs}"
    );
    assert!(has_line(&fs["pnl"]["lines"], "Autres", 50000), "{fs}");
    assert_eq!(fs["pnl"]["resultat_cents"], json!(970000));
}

fn has_line(lines: &Value, label: &str, total: i64) -> bool {
    lines
        .as_array()
        .map(|a| {
            a.iter()
                .any(|l| l["label"] == json!(label) && l["total_cents"] == json!(total))
        })
        .unwrap_or(false)
}

#[test]
fn jur_cross_border_buyer_endpoint_uses_the_buyer_country_scheme() {
    let (_d, db) = tmp("lu-crossborder");
    cli(
        &db,
        &[
            "init",
            "--name",
            "Sàrl Test",
            "--country",
            "LU",
            "--legal-form",
            "sarl",
            "--registration-id",
            "B123456",
            "--tax-id",
            "LU12345678",
            "--vat",
            "on",
        ],
    );
    cli(
        &db,
        &[
            "company",
            "update",
            "--address",
            "1 rue du Test",
            "--postal-code",
            "L-1234",
            "--city",
            "Luxembourg",
        ],
    );
    // buyer in NL: the KVK number was issued by the Dutch registry -> 9944
    cli(
        &db,
        &[
            "contact",
            "add",
            "--name",
            "ACME B.V.",
            "--address",
            "Straat 1",
            "--city",
            "Amsterdam",
            "--country",
            "NL",
            "--vat-id",
            "NL999999999B01",
            "--kvk",
            "98765432",
        ],
    );
    // same-market buyer: no country -> LU, seller scheme 0195
    cli(
        &db,
        &[
            "contact",
            "add",
            "--name",
            "Sàrl LU",
            "--address",
            "1 rue du Test",
            "--city",
            "Luxembourg",
            "--vat-id",
            "LU99999999",
            "--kvk",
            "B123456",
        ],
    );
    // unregistered market (IS): valid code, no profile -> seller scheme 0195
    cli(
        &db,
        &[
            "contact",
            "add",
            "--name",
            "Island ehf.",
            "--address",
            "Gata 1",
            "--city",
            "Reykjavik",
            "--country",
            "IS",
            "--vat-id",
            "IS123456",
            "--kvk",
            "0000123456",
        ],
    );
    let c = open(&db);
    let mut xmls = vec![];
    for contact in [1i64, 2, 3] {
        let v = bukio::invoice::create_invoice(
            &c,
            contact,
            "2026-07-10",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            &[json!("1x Prestation @ 100.00 @17")],
            "agent:test",
            false,
        )
        .unwrap();
        let id = v["id"]
            .as_i64()
            .or_else(|| v["invoice"]["id"].as_i64())
            .unwrap();
        bukio::invoice::finalize_invoice(&c, id, "agent:test", false).unwrap();
        let invoice = bukio::invoice::get_invoice(&c, id).unwrap().unwrap();
        xmls.push(bukio::ubl::invoice_to_ubl(&c, &invoice).unwrap());
    }
    assert!(
        xmls[0].contains("<cbc:EndpointID schemeID=\"9944\">98765432</cbc:EndpointID>"),
        "NL buyer keeps the Dutch scheme: {}",
        xmls[0]
    );
    assert!(
        xmls[1].contains("<cbc:EndpointID schemeID=\"0195\">B123456</cbc:EndpointID>"),
        "same-market buyer falls back to the seller scheme: {}",
        xmls[1]
    );
    assert!(
        xmls[2].contains("<cbc:EndpointID schemeID=\"0195\">0000123456</cbc:EndpointID>"),
        "unregistered market falls back to the seller scheme: {}",
        xmls[2]
    );
}

#[test]
fn jur_lu_rejects_the_nl_statutory_model() {
    let (_d, db) = tmp("lu-model");
    cli(
        &db,
        &[
            "init",
            "--name",
            "Sàrl Test",
            "--country",
            "LU",
            "--legal-form",
            "sarl",
            "--vat",
            "on",
        ],
    );
    let r = cli(
        &db,
        &[
            "financial-statements",
            "report",
            "--year",
            "2026",
            "--model",
            "klein",
        ],
    );
    assert_eq!(code(&r), "INVALID_MODEL", "{r}");
    let ok = cli(&db, &["financial-statements", "report", "--year", "2026"]);
    assert_eq!(ok["data"]["financial_statements"]["model"], json!("abrege"));
}

#[test]
fn jur_nl_financial_statements_keep_the_klein_default() {
    let (_d, db) = tmp("nl-model");
    cli(&db, &["init", "--name", "Test BV", "--vat", "on"]);
    let r = cli(&db, &["financial-statements", "report", "--year", "2026"]);
    assert_eq!(
        r["data"]["financial_statements"]["model"],
        json!("klein"),
        "{r}"
    );
    let m = cli(
        &db,
        &[
            "financial-statements",
            "report",
            "--year",
            "2026",
            "--model",
            "micro",
        ],
    );
    assert_eq!(m["data"]["financial_statements"]["model"], json!("micro"));
}

// --- B5: LU compliance calendar --------------------------------------------

#[test]
fn jur_lu_compliance_calendar() {
    let (_d, db) = tmp("lu-calendar");
    cli(
        &db,
        &[
            "init",
            "--name",
            "Sàrl Test",
            "--country",
            "LU",
            "--legal-form",
            "sarl",
            "--vat",
            "on",
        ],
    );
    let r = cli(&db, &["compliance", "status", "--year", "2026"]);
    let obs = r["data"]["compliance"]["obligations"]
        .as_array()
        .cloned()
        .unwrap_or_else(|| {
            panic!("no obligations in {r}");
        });
    let deadline = |kind: &str, period: &str| -> String {
        obs.iter()
            .find(|o| o["type"] == json!(kind) && o["period"] == json!(period))
            .unwrap_or_else(|| panic!("no {kind} {period} in {r}"))["deadline"]
            .as_str()
            .unwrap()
            .to_string()
    };
    assert_eq!(deadline("TVA", "2026-Q1"), "2026-04-15");
    assert_eq!(deadline("TVA", "2026-Q2"), "2026-07-15");
    assert_eq!(deadline("TVA", "2026-Q3"), "2026-10-15");
    assert_eq!(deadline("TVA", "2026-Q4"), "2027-01-15");
    assert_eq!(deadline("TVA", "2025-Q4"), "2026-01-15");
    assert_eq!(deadline("COMPTES_ANNUELS", "2026"), "2027-07-31");
    assert!(
        !obs.iter()
            .any(|o| ["OB", "ICP", "JAARREKENING"].contains(&o["type"].as_str().unwrap_or(""))),
        "no NL filing types in the LU calendar: {r}"
    );
    let today = bukio::dates::today_iso();
    let q3 = obs
        .iter()
        .find(|o| o["type"] == json!("TVA") && o["period"] == json!("2026-Q3"))
        .unwrap();
    let want = if "2026-10-15" < today.as_str() {
        "overdue"
    } else {
        "open"
    };
    assert_eq!(q3["status"], json!(want), "{q3}");
}

// --- review fixes: profile-driven behaviour per market ----------------------

#[test]
fn jur_de_ubl_reverse_charge_percent_is_profile_driven() {
    let (_d, db) = tmp("de-ubl");
    cli(
        &db,
        &[
            "init",
            "--name",
            "Test GmbH",
            "--country",
            "DE",
            "--legal-form",
            "gmbh",
            "--vat",
            "on",
        ],
    );
    cli(
        &db,
        &[
            "contact",
            "add",
            "--name",
            "Kunde",
            "--address",
            "Str 1",
            "--city",
            "Berlin",
            "--vat-id",
            "DE999999999",
        ],
    );
    let c = open(&db);
    let raw = vec![json!("Beratung @ 1 @ 100.00 @R")];
    let v = bukio::invoice::create_invoice(
        &c,
        1,
        "2026-07-10",
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        &raw,
        "agent:test",
        false,
    )
    .unwrap();
    let id = v["id"]
        .as_i64()
        .or_else(|| v["invoice"]["id"].as_i64())
        .unwrap();
    let invoice = bukio::invoice::get_invoice(&c, id).unwrap().unwrap();
    let xml = bukio::ubl::invoice_to_ubl(&c, &invoice).unwrap();
    assert!(
        xml.contains("<cbc:Percent>19.00</cbc:Percent>"),
        "DE rate on the AE line: {xml}"
    );
    assert!(
        !xml.contains("<cbc:Percent>21.00</cbc:Percent>"),
        "not the NL 21.00: {xml}"
    );
    assert!(
        xml.contains("<cbc:IdentificationCode>DE</cbc:IdentificationCode>"),
        "buyer country falls back to the profile: {xml}"
    );
}

#[test]
fn jur_be_vat_book_lands_on_the_profile_ledger() {
    let (_d, db) = tmp("be-vat");
    cli(
        &db,
        &[
            "init",
            "--name",
            "Test BV",
            "--country",
            "BE",
            "--legal-form",
            "bv",
            "--vat",
            "on",
        ],
    );
    cli(
        &db,
        &[
            "vat",
            "book",
            "--date",
            "2026-08-15",
            "--desc",
            "omzet",
            "--postings",
            "700:-1000@21,400:1210",
            "--post",
        ],
    );
    let tb = cli(&db, &["report", "trial-balance"]);
    let accounts = &tb["data"]["accounts"];
    assert_eq!(
        net(accounts, "451"),
        Some(-21000),
        "output VAT on the BE ledger 451: {tb}"
    );
    assert!(
        net(accounts, "2500").is_none() && net(accounts, "1500").is_none(),
        "no NL clearing accounts"
    );
    assert_eq!(tb["data"]["balanced"], json!(true));
}

#[test]
fn jur_fr_vat_book_accepts_dotted_codes() {
    let (_d, db) = tmp("fr-vat");
    cli(
        &db,
        &[
            "init",
            "--name",
            "SARL Test",
            "--country",
            "FR",
            "--legal-form",
            "sarl",
            "--vat",
            "on",
        ],
    );
    cli(
        &db,
        &[
            "vat",
            "book",
            "--date",
            "2026-08-15",
            "--desc",
            "omzet",
            "--postings",
            "701:-1000@5.5,411:1055",
            "--post",
        ],
    );
    let tb = cli(&db, &["report", "trial-balance"]);
    assert_eq!(net(&tb["data"]["accounts"], "44571"), Some(-5500), "{tb}");
    assert_eq!(tb["data"]["balanced"], json!(true));
}

#[test]
fn jur_be_vat_file_defaults_to_the_profile_account() {
    let (_d, db) = tmp("be-vatfile");
    cli(
        &db,
        &[
            "init",
            "--name",
            "Test BV",
            "--country",
            "BE",
            "--legal-form",
            "bv",
            "--vat",
            "on",
        ],
    );
    cli(
        &db,
        &[
            "entry",
            "add",
            "--date",
            "2026-06-30",
            "--desc",
            "omzet",
            "--postings",
            "400:1210,700:-1000,451:-210",
            "--post",
        ],
    );
    let plan = cli(&db, &["vat", "file", "--dry-run"]);
    assert_eq!(
        plan["data"]["account"],
        json!("451"),
        "BE fileDefault, not the NL 2510: {plan}"
    );
    assert_eq!(plan["data"]["owe"], json!(true));
    let filed = cli(&db, &["vat", "file"]);
    assert!(
        filed["data"]["entry_id"].as_i64().is_some(),
        "vat file posted: {filed}"
    );
    let c = open(&db);
    assert_eq!(
        bukio::accounts::resolve_profile(&c).unwrap()["tax"]["accounts"]["differenceDefault"],
        json!("648")
    );
}

#[test]
fn jur_de_bank_add_defaults_to_the_profile_bank_account() {
    let (_d, db) = tmp("de-bank");
    cli(
        &db,
        &[
            "init",
            "--name",
            "Test GmbH",
            "--country",
            "DE",
            "--legal-form",
            "gmbh",
            "--vat",
            "on",
        ],
    );
    let r = cli(
        &db,
        &[
            "bank",
            "add",
            "--iban",
            CLI_IBAN,
            "--name",
            "Zakelijk",
            "--dry-run",
        ],
    );
    let account = r["data"]["plan"]["account_code"]
        .as_str()
        .unwrap_or("")
        .to_string();
    assert_eq!(
        account, "1200",
        "DE profile bank account, not the NL 1100: {r}"
    );
}

#[test]
fn jur_nl_bank_add_still_defaults_to_1100() {
    let (_d, db) = tmp("nl-bank");
    cli(&db, &["init", "--name", "Test BV", "--vat", "on"]);
    let r = cli(
        &db,
        &[
            "bank",
            "add",
            "--iban",
            CLI_IBAN,
            "--name",
            "Zakelijk",
            "--dry-run",
        ],
    );
    let account = r["data"]["plan"]["account_code"]
        .as_str()
        .unwrap_or("")
        .to_string();
    assert_eq!(account, "1100", "{r}");
}
