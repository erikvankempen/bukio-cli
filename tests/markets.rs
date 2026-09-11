//! Per-market behaviour, table-driven over every profile in profiles.json.
//!
//! The JS suite has a hand-written block per market asserting the profile's
//! *values* plus the engine's behaviour. The values are already guarded once,
//! for all 31 markets, by test/profiles-drift.test.js — so what is asserted
//! here is that the port's *engine* honours each profile: the chart it seeds,
//! the taxonomy discriminator it stamps, the strict dispatch that must not
//! fall back to Dutch output, and a calendar that only carries that market's
//! own filing types.

use serde_json::{json, Value};
use std::path::PathBuf;

fn bin() -> String {
    std::env::var("CARGO_BIN_EXE_bukio").unwrap_or_else(|_| "target/release/bukio".to_string())
}

fn tmp(tag: &str) -> (PathBuf, String) {
    let dir = std::env::temp_dir().join(format!("bukio-mkt-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join("test.db").to_string_lossy().to_string();
    (dir, db)
}

fn cli(db: &str, args: &[&str]) -> Value {
    let out = std::process::Command::new(bin())
        .args(args)
        .args(["--db", db, "--json"])
        .env("BUKIO_ACTOR", "agent:test")
        .output()
        .expect("cli runs");
    serde_json::from_slice(&out.stdout).unwrap_or_else(|_| {
        let code = String::from_utf8_lossy(&out.stderr).to_string();
        json!({ "ok": false, "error": { "code": "NO_JSON" }, "stderr": code })
    })
}

fn code(v: &Value) -> String {
    v["error"]["code"].as_str().unwrap_or("").to_string()
}

/// The market codes in profiles.json.
fn markets() -> Vec<String> {
    let mut v: Vec<String> = bukio::accounts::profiles()
        .as_object()
        .expect("profiles are an object")
        .keys()
        .cloned()
        .collect();
    v.sort();
    v
}

#[test]
fn mkt_profiles_cover_thirty_one_markets() {
    let m = markets();
    assert_eq!(m.len(), 31, "{m:?}");
    for cc in ["NL", "BE", "DE", "FR", "GB", "US", "LU", "SE", "NO", "XK"] {
        assert!(m.contains(&cc.to_string()), "{cc} missing from {m:?}");
    }
}

#[test]
fn mkt_init_seeds_each_market_chart_with_its_own_taxonomy() {
    let mut checked = 0;
    for cc in markets() {
        let p = bukio::accounts::get_profile(&cc).unwrap();
        // every market here has a chart; only some carry a statutory taxonomy
        // (NL rgs, LU pcn) — the rest store NULL, exactly like the JS
        let taxonomy = p["reporting"]["taxonomy"].as_str();
        if p["reporting"]["defaultChart"]
            .as_array()
            .map(|c| c.is_empty())
            .unwrap_or(true)
        {
            continue; // PLANNED markets have no chart yet
        }
        let form = p["meta"]["legalForms"]
            .as_array()
            .and_then(|f| f.first())
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| panic!("{cc} has no legal form"));
        let (_d, db) = tmp(&format!("init-{cc}"));
        let mut args = vec![
            "init",
            "--name",
            "Test Co",
            "--country",
            &cc,
            "--legal-form",
            form,
        ];
        if p["tax"]["system"].as_str() == Some("vat") {
            args.push("--vat");
            args.push("on");
        }
        let r = cli(&db, &args);
        assert_eq!(r["ok"], json!(true), "{cc} init failed: {r}");

        let company = &r["data"]["company"];
        assert_eq!(company["country"], json!(cc), "{cc}: {r}");
        assert_eq!(
            company["base_currency"], p["meta"]["baseCurrency"],
            "{cc} base currency"
        );
        assert_eq!(company["locale"], p["meta"]["locale"], "{cc} locale");

        let c = bukio::db::open_db(&db).unwrap();
        let mut stmt = c
            .prepare("SELECT code, taxonomy FROM accounts WHERE active = 1")
            .unwrap();
        let rows: Vec<(String, Option<String>)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        drop(stmt);
        assert!(!rows.is_empty(), "{cc} seeded no accounts");
        for (code, tax) in &rows {
            assert_eq!(
                tax.as_deref(),
                taxonomy,
                "{cc}: account {code} carries the wrong taxonomy discriminator"
            );
        }
        // every chart account the profile declares is seeded under its own name
        for want in p["reporting"]["defaultChart"].as_array().unwrap() {
            let code = want["code"].as_str().unwrap();
            let name = want["name"].as_str().unwrap();
            assert!(
                rows.iter().any(|(c, _)| c == code),
                "{cc}: profile account {code} ({name}) was not seeded"
            );
        }
        checked += 1;
    }
    assert!(checked >= 28, "only {checked} markets exercised");
}

#[test]
fn mkt_strict_dispatch_never_falls_back_to_dutch_output() {
    for cc in markets() {
        let p = bukio::accounts::get_profile(&cc).unwrap();
        if p["reporting"]["defaultChart"]
            .as_array()
            .map(|c| c.is_empty())
            .unwrap_or(true)
        {
            continue;
        }
        if cc == "NL" {
            continue; // the reference market is covered by its own tests
        }
        let form = p["meta"]["legalForms"].as_array().unwrap()[0]
            .as_str()
            .unwrap();
        let (_d, db) = tmp(&format!("dispatch-{cc}"));
        let mut args = vec![
            "init",
            "--name",
            "Test Co",
            "--country",
            &cc,
            "--legal-form",
            form,
        ];
        if p["tax"]["system"].as_str() == Some("vat") {
            args.push("--vat");
            args.push("on");
        }
        let r = cli(&db, &args);
        assert_eq!(r["ok"], json!(true), "{cc} init failed: {r}");

        let out = cli(&db, &["vat", "readout", "--period", "2026-Q1"]);
        let has_layout = p["tax"]["returnLayout"].is_string()
            || p["tax"]["returnLayout"]
                .as_object()
                .map(|o| !o.is_empty())
                .unwrap_or(false);
        if has_layout {
            assert_eq!(
                out["ok"],
                json!(true),
                "{cc} declares a return layout: {out}"
            );
            continue;
        }
        // Fail loudly, and never render NL fields. A company whose VAT module is
        // off hears VAT_MODULE_OFF first (the JS order); the rest must report
        // that their country has no return layout.
        let expected = if p["tax"]["system"].as_str() == Some("vat") {
            "FORMAT_NOT_SUPPORTED"
        } else {
            "VAT_MODULE_OFF"
        };
        assert_eq!(
            code(&out),
            expected,
            "{cc} must not fall back to a Dutch VAT return: {out}"
        );
    }
}

#[test]
fn mkt_compliance_calendar_only_carries_the_markets_own_filing_types() {
    for cc in markets() {
        let p = bukio::accounts::get_profile(&cc).unwrap();
        let declared: Vec<&str> = p["compliance"]["filingTypes"]
            .as_array()
            .map(|a| a.iter().filter_map(|f| f["type"].as_str()).collect())
            .unwrap_or_default();
        let mut args = vec!["init", "--name", "Test Co", "--country", &cc];
        if p["reporting"]["defaultChart"]
            .as_array()
            .map(|c| c.is_empty())
            .unwrap_or(true)
        {
            continue;
        }
        args.push("--legal-form");
        args.push(
            p["meta"]["legalForms"].as_array().unwrap()[0]
                .as_str()
                .unwrap(),
        );
        if p["tax"]["system"].as_str() == Some("vat") {
            args.push("--vat");
            args.push("on");
        }
        let (_d, db) = tmp(&format!("cal-{cc}"));
        let r = cli(&db, &args);
        assert_eq!(r["ok"], json!(true), "{cc} init failed: {r}");
        let cal = cli(&db, &["compliance", "status", "--year", "2026"]);
        let obs = cal["data"]["compliance"]["obligations"]
            .as_array()
            .cloned()
            .unwrap_or_else(|| {
                panic!("{cc}: no obligations in {cal}");
            });
        for o in &obs {
            let t = o["type"].as_str().unwrap_or("");
            assert!(
                declared.contains(&t),
                "{cc}: unexpected filing type '{t}' in {cal}"
            );
            let deadline = o["deadline"].as_str().unwrap_or("");
            assert!(
                deadline.len() == 10 && deadline.as_bytes()[4] == b'-',
                "{cc}: deadline '{deadline}' is not a date"
            );
            assert!(
                ["open", "overdue", "filed", "due"].contains(&o["status"].as_str().unwrap_or("")),
                "{cc}: odd status in {o}"
            );
        }
        // a market that declares no filings reports none, rather than the NL set
        if declared.is_empty() {
            assert!(
                obs.is_empty(),
                "{cc} declares no filings but reported {obs:?}"
            );
        }
        // NL filing types only ever appear for NL
        if cc != "NL" {
            for o in &obs {
                assert!(
                    !["OB", "ICP", "JAARREKENING"].contains(&o["type"].as_str().unwrap_or("")),
                    "{cc} leaked an NL filing type: {o}"
                );
            }
        }
    }
}
