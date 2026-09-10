//! Ported from test/edge-cases.test.js — edge cases across the whole surface:
//! guards, rounding, boundaries, lifecycle violations, idempotency.
use bukio::entries::{create_entry, post_entry, reverse_entry, CreateEntry, PostingSpec};
use bukio::invoice::{
    create_invoice, credit_invoice, finalize_invoice, get_invoice, mark_paid, parse_line_spec,
};
use rusqlite::Connection;
use serde_json::{json, Value};

fn setup_with_vat(vat: bool) -> Connection {
    let d = bukio::db::open_db(":memory:").unwrap();
    bukio::accounts::seed_default_chart(&d).unwrap();
    d.execute(
        "INSERT INTO company (name, registration_id, legal_form, tax_id, iban, address, postal_code, city, vat_module)
         VALUES ('Demo BV','12345678','bv','NL123456789B01','NL91ABNA0417164300','Industrieweg 12','2712 CD','Zoetermeer', ?1)",
        [vat as i64],
    )
    .unwrap();
    if vat {
        bukio::vat::enable_vat_module(&d, "agent:test").unwrap();
    }
    d
}

fn setup() -> Connection {
    setup_with_vat(true)
}

fn specs(pairs: &[(&str, i64)]) -> Vec<PostingSpec> {
    pairs
        .iter()
        .map(|(code, amount_cents)| PostingSpec {
            code: (*code).to_string(),
            amount_cents: *amount_cents,
            cost_center_code: None,
            vat_code: None,
            vat_amount_cents: None,
        })
        .collect()
}

/// create + post, like the JS `entry()` helper
fn entry(db: &Connection, date: &str, desc: &str, postings: Vec<PostingSpec>) -> Value {
    let e = create_entry(
        db,
        CreateEntry {
            date,
            description: desc,
            postings,
            source: "manual",
            source_ref: None,
            actor: "agent:test",
        },
    )
    .unwrap();
    serde_json::to_value(post_entry(db, e.id, "agent:test").unwrap()).unwrap()
}

fn draft(db: &Connection, date: &str, desc: &str, postings: Vec<PostingSpec>) -> i64 {
    create_entry(
        db,
        CreateEntry {
            date,
            description: desc,
            postings,
            source: "manual",
            source_ref: None,
            actor: "agent:test",
        },
    )
    .unwrap()
    .id
}

fn add_contact(db: &Connection, vat_id: Option<&str>) -> i64 {
    bukio::contacts::create_contact(
        db,
        "ACME B.V.",
        Some("Straat 1"),
        Some("1000 AA"),
        Some("Amsterdam"),
        None,
        None,
        vat_id,
        None,
        None,
        "agent:test",
        false,
    )
    .unwrap()["id"]
        .as_i64()
        .unwrap()
}

fn days_from_now(days: i64) -> String {
    (chrono::Local::now().date_naive() + chrono::Duration::days(days))
        .format("%Y-%m-%d")
        .to_string()
}

fn code_of<T>(r: Result<T, bukio::money::BukioError>) -> String {
    match r {
        Ok(_) => panic!("expected an error"),
        Err(e) => e.code.to_string(),
    }
}

fn lines(v: &[&str]) -> Vec<Value> {
    v.iter().map(|s| json!(s)).collect()
}

// --- ledger guards ---------------------------------------------------------

#[test]
fn ledger_unbalanced_zero_amount_too_few_postings_rejected() {
    let d = setup();
    assert_eq!(
        code_of(create_entry(
            &d,
            CreateEntry {
                date: "2026-01-01",
                description: "x",
                postings: specs(&[("1100", 100), ("3000", -99)]),
                source: "manual",
                source_ref: None,
                actor: "a"
            }
        )),
        "UNBALANCED"
    );
    assert_eq!(
        code_of(create_entry(
            &d,
            CreateEntry {
                date: "2026-01-01",
                description: "x",
                postings: specs(&[("1100", 0), ("3000", 0)]),
                source: "manual",
                source_ref: None,
                actor: "a"
            }
        )),
        "INVALID_AMOUNT_CENTS"
    );
    assert_eq!(
        code_of(create_entry(
            &d,
            CreateEntry {
                date: "2026-01-01",
                description: "x",
                postings: specs(&[("1100", 100)]),
                source: "manual",
                source_ref: None,
                actor: "a"
            }
        )),
        "TOO_FEW_POSTINGS"
    );
    assert_eq!(
        code_of(create_entry(
            &d,
            CreateEntry {
                date: "2026-01-01",
                description: "x",
                postings: specs(&[("9999", 100), ("3000", -100)]),
                source: "manual",
                source_ref: None,
                actor: "a"
            }
        )),
        "ACCOUNT_NOT_FOUND"
    );
    assert_eq!(
        code_of(create_entry(
            &d,
            CreateEntry {
                date: "2026-01-01",
                description: "x",
                postings: specs(&[("1100", 100), ("3000", -100)]),
                source: "bogus",
                source_ref: None,
                actor: "a"
            }
        )),
        "INVALID_SOURCE"
    );
}

#[test]
fn ledger_same_account_on_both_sides_is_legal() {
    let d = setup();
    let e = entry(
        &d,
        "2026-01-01",
        "Kas naar bank",
        specs(&[("1100", 10000), ("1100", -10000)]),
    );
    assert_eq!(e["state"], json!("posted"));
}

#[test]
fn ledger_reversal_guards_draft_and_double_reversal_rejected() {
    let d = setup();
    let dr = draft(
        &d,
        "2026-01-01",
        "d",
        specs(&[("1100", 100), ("3000", -100)]),
    );
    assert_eq!(code_of(reverse_entry(&d, dr, "a", None)), "NOT_POSTED");

    let posted = entry(
        &d,
        "2026-01-01",
        "p",
        specs(&[("1100", 100), ("3000", -100)]),
    );
    let id = posted["id"].as_i64().unwrap();
    reverse_entry(&d, id, "a", None).unwrap();
    assert_eq!(
        code_of(reverse_entry(&d, id, "a", None)),
        "ALREADY_REVERSED"
    );

    // reversal nets the account to zero
    let net: i64 = d
        .query_row(
            "SELECT COALESCE(SUM(p.amount_cents),0) FROM postings p
               JOIN journal_entries e ON e.id = p.entry_id
              WHERE e.state = 'posted'
                AND p.account_id = (SELECT id FROM accounts WHERE code = '1100')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(net, 0);
}

#[test]
fn ledger_posted_entries_are_immutable() {
    let d = setup();
    entry(
        &d,
        "2026-01-01",
        "p",
        specs(&[("1100", 100), ("3000", -100)]),
    );
    assert!(d
        .execute(
            "UPDATE journal_entries SET description = 'hack' WHERE id = 1",
            []
        )
        .is_err());
    assert!(d
        .execute("DELETE FROM postings WHERE entry_id = 1", [])
        .is_err());
    // the description survived the attempted UPDATE
    let desc: String = d
        .query_row(
            "SELECT description FROM journal_entries WHERE id = 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(desc, "p");
}

#[test]
fn ledger_drafts_excluded_from_balans_and_pnl() {
    let d = setup();
    draft(
        &d,
        "2026-01-01",
        "draft",
        specs(&[("1100", 500), ("3000", -500)]),
    );
    entry(
        &d,
        "2026-01-01",
        "posted",
        specs(&[("1100", 10000), ("8000", -10000)]),
    );
    let b = bukio::reports::balans(&d, "2026-12-31").unwrap();
    assert_eq!(b["assets"]["total_cents"].as_i64(), Some(10000));
    let p = bukio::reports::pnl(&d, "2026-01-01", "2026-12-31").unwrap();
    assert_eq!(p["revenue_cents"].as_i64(), Some(10000));
}

// --- invoice edge cases ----------------------------------------------------

#[test]
fn invoice_quantity_and_price_guards() {
    let d = setup();
    add_contact(&d, None);
    for spec in ["0x Ding @ 10.00", "Ding @ 0.00", "Ding @ -5.00"] {
        assert_eq!(
            code_of(create_invoice(
                &d,
                1,
                "2026-07-10",
                None,
                None,
                None,
                None,
                None,
                None,
                &lines(&[spec]),
                "agent:test",
                false
            )),
            "INVALID_LINE",
            "spec {spec}"
        );
    }
}

#[test]
fn invoice_line_parser_dutch_comma_and_at_in_description() {
    let a = parse_line_spec("Papier @ 45,50 @21").unwrap();
    assert_eq!(a["qtyMilli"], json!(1000));
    assert_eq!(a["qty"], json!(1));
    assert_eq!(a["description"], json!("Papier"));
    assert_eq!(a["priceCents"], json!(4550));
    assert_eq!(a["vatCode"], json!("21"));
    assert!(a["discountType"].is_null());
    assert!(a["discountValue"].is_null());

    let b = parse_line_spec("2x Email @ adres @ 5.00").unwrap();
    assert_eq!(b["qtyMilli"], json!(2000));
    assert_eq!(b["qty"], json!(2));
    assert_eq!(b["description"], json!("Email@adres"));
    assert_eq!(b["priceCents"], json!(500));
    assert!(b["vatCode"].is_null());

    let c = parse_line_spec("1000x Zeer lange omschrijving met @ tekens erin @ 0.99 @9").unwrap();
    assert_eq!(c["qtyMilli"], json!(1000000));
    assert_eq!(
        c["description"],
        json!("Zeer lange omschrijving met@tekens erin")
    );
    assert_eq!(c["priceCents"], json!(99));
    assert_eq!(c["vatCode"], json!("9"));
}

#[test]
fn invoice_line_parser_regressions() {
    // "DESC @ 100" used to fail: "100" was misread as a VAT code
    let a = parse_line_spec("Consultancy @ 100").unwrap();
    assert_eq!(a["priceCents"], json!(10000));
    assert!(a["vatCode"].is_null());
    // lowercase vat codes are normalised
    let b = parse_line_spec("Uren @ 75.00 @re").unwrap();
    assert_eq!(b["priceCents"], json!(7500));
    assert_eq!(b["vatCode"], json!("RE"));
    // a non-vat-code word fails cleanly
    assert_eq!(
        code_of(parse_line_spec("Uren @ 75.00 @nope")),
        "INVALID_LINE"
    );
    // negative quantity is rejected
    assert_eq!(code_of(parse_line_spec("-2x Uren @ 75.00")), "INVALID_LINE");
}

#[test]
fn invoice_per_line_rounding_three_pennies() {
    let d = setup();
    add_contact(&d, None);
    let inv = create_invoice(
        &d,
        1,
        "2026-07-10",
        None,
        None,
        None,
        None,
        None,
        None,
        &lines(&["3x Pennen @ 0.01 @21"]),
        "agent:test",
        false,
    )
    .unwrap();
    // vat on the LINE total: round(0.03 * 21%) = 1 cent
    assert_eq!(inv["lines"][0]["vat_amount_cents"].as_i64(), Some(1));
    assert_eq!(inv["net_cents"].as_i64(), Some(3));
    assert_eq!(inv["gross_cents"].as_i64(), Some(4));
}

#[test]
fn invoice_zero_and_exempt_lines_book_without_vat() {
    let d = setup();
    add_contact(&d, None);
    let inv = create_invoice(
        &d,
        1,
        "2026-07-10",
        None,
        None,
        None,
        None,
        None,
        None,
        &lines(&["1x Export @ 500.00 @0", "1x Vrijstelling @ 100.00 @V"]),
        "agent:test",
        false,
    )
    .unwrap();
    let id = inv["id"].as_i64().unwrap();
    finalize_invoice(&d, id, "agent:test", false).unwrap();

    let e = serde_json::to_value(bukio::entries::get_entry(&d, 1).unwrap()).unwrap();
    let postings = e["postings"].as_array().unwrap();
    let tagged: Vec<&Value> = postings
        .iter()
        .filter(|p| !p["vat_code"].is_null())
        .collect();
    assert_eq!(tagged.len(), 2, "{e}");
    assert!(tagged.iter().all(|p| p["vat_amount_cents"] == json!(0)));
    let bank = postings
        .iter()
        .find(|p| p["account_code"] == json!("1200"))
        .unwrap();
    assert_eq!(bank["amount_cents"].as_i64(), Some(60000));
    let omzet: i64 = postings
        .iter()
        .filter(|p| p["account_code"] == json!("8000"))
        .map(|p| p["amount_cents"].as_i64().unwrap())
        .sum();
    assert_eq!(omzet, -60000);

    let r = bukio::vat::ob_readout(&d, "2026-Q3").unwrap();
    assert_eq!(r["fields"]["1c"].as_i64(), Some(60000));
    assert_eq!(r["fields"]["5a"].as_i64(), Some(0));
}

#[test]
fn invoice_credit_note_of_a_paid_invoice_and_credit_of_credit_rejected() {
    let d = setup();
    add_contact(&d, None);
    let inv = create_invoice(
        &d,
        1,
        "2026-07-10",
        None,
        None,
        None,
        None,
        None,
        None,
        &lines(&["1x Werk @ 100.00 @21"]),
        "agent:test",
        false,
    )
    .unwrap();
    let id = inv["id"].as_i64().unwrap();
    finalize_invoice(&d, id, "agent:test", false).unwrap();
    mark_paid(&d, id, "2026-07-20", 12100, "bank", "agent:test", false).unwrap();
    let credit = credit_invoice(&d, id, None, None, "agent:test", false).unwrap();
    let cid = credit["id"].as_i64().unwrap();
    finalize_invoice(&d, cid, "agent:test", false).unwrap();
    assert_eq!(
        code_of(credit_invoice(&d, cid, None, None, "agent:test", false)),
        "NOT_SALES_INVOICE"
    );
}

#[test]
fn invoice_lifecycle_pay_draft_overpay_overdue() {
    let d = setup();
    add_contact(&d, None);
    let inv = create_invoice(
        &d,
        1,
        "2026-01-01",
        None,
        None,
        None,
        None,
        None,
        None,
        &lines(&["1x X @ 100.00 @21"]),
        "agent:test",
        false,
    )
    .unwrap();
    let id = inv["id"].as_i64().unwrap();
    assert_eq!(
        code_of(mark_paid(
            &d,
            id,
            "2026-01-05",
            100,
            "bank",
            "agent:test",
            false
        )),
        "NOT_PAYABLE"
    );
    finalize_invoice(&d, id, "agent:test", false).unwrap();
    assert_eq!(
        code_of(mark_paid(
            &d,
            id,
            "2026-01-05",
            999999,
            "bank",
            "agent:test",
            false
        )),
        "OVERPAYMENT"
    );
    // overdue is DERIVED at read time (due 2026-01-31 is in the past)
    let got = get_invoice(&d, id).unwrap().unwrap();
    assert_eq!(got["status"], json!("overdue"));
}

#[test]
fn invoice_ubl_escaping_and_verlegd_category() {
    let d = setup();
    add_contact(&d, Some("DE123456789"));
    let inv = create_invoice(
        &d,
        1,
        "2026-07-10",
        None,
        None,
        None,
        None,
        None,
        None,
        &lines(&[
            "1x IT & Support <urgent> @ 500.00 @RE",
            "1x Normaal @ 100.00 @21",
        ]),
        "agent:test",
        false,
    )
    .unwrap();
    let id = inv["id"].as_i64().unwrap();
    finalize_invoice(&d, id, "agent:test", false).unwrap();
    let full = get_invoice(&d, id).unwrap().unwrap();
    let xml = bukio::ubl::invoice_to_ubl(&d, &full).unwrap();
    assert!(xml.contains("IT &amp; Support &lt;urgent&gt;"), "{xml}");
    assert!(xml.contains("<cbc:ID>AE</cbc:ID>"), "verlegd line -> AE");
    assert!(xml.contains("<cbc:ID>S</cbc:ID>"), "standard line");
    assert!(
        xml.contains("<cbc:TaxAmount currencyID=\"EUR\">21.00</cbc:TaxAmount>"),
        "only the 21% line carries tax"
    );
}

#[test]
fn invoice_due_date_crosses_the_year_boundary() {
    let d = setup();
    add_contact(&d, None);
    let inv = create_invoice(
        &d,
        1,
        "2026-12-20",
        Some(30),
        None,
        None,
        None,
        None,
        None,
        &lines(&["1x X @ 10.00"]),
        "agent:test",
        false,
    )
    .unwrap();
    assert_eq!(inv["due_date"], json!("2027-01-19"));
}

// --- recurring edge cases --------------------------------------------------

/// create_template takes the serialized postings array (the CLI converts the
/// compact "code:amount" spec string into this before calling the engine)
fn postings_json(pairs: &[(&str, i64)]) -> String {
    let v: Vec<Value> = pairs
        .iter()
        .map(|(c, a)| json!({ "code": c, "amountCents": a }))
        .collect();
    serde_json::to_string(&v).unwrap()
}

fn tpl(
    db: &Connection,
    name: &str,
    postings: String,
    freq: &str,
    day: u32,
    start: &str,
    end: Option<&str>,
) -> Value {
    bukio::recurring::create_template(
        db, name, None, freq, day, start, end, None, &postings, false, "a", "entry", false,
    )
    .unwrap()
}

#[test]
fn recurring_day_28_keeps_the_28th_every_month() {
    let d = setup();
    let t = tpl(
        &d,
        "Huur",
        postings_json(&[("4300", 100000), ("1100", -100000)]),
        "monthly",
        28,
        "2026-01-28",
        None,
    );
    assert_eq!(t["next_run_date"], json!("2026-01-28"));
    bukio::recurring::run_due(&d, Some("2026-12-31"), None, "a", false).unwrap();
    let mut stmt = d
        .prepare(
            "SELECT DISTINCT date FROM journal_entries WHERE source = 'recurring' ORDER BY date",
        )
        .unwrap();
    let dates: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert_eq!(dates.len(), 12, "{dates:?}");
    assert!(dates.iter().all(|d| d.ends_with("-28")), "{dates:?}");
}

#[test]
fn recurring_quarterly_and_yearly_frequencies() {
    let d = setup();
    tpl(
        &d,
        "Q",
        postings_json(&[("4300", 1000), ("1100", -1000)]),
        "quarterly",
        1,
        "2026-01-01",
        None,
    );
    tpl(
        &d,
        "Y",
        postings_json(&[("4300", 1000), ("1100", -1000)]),
        "yearly",
        1,
        "2026-01-01",
        None,
    );
    bukio::recurring::run_due(&d, Some("2027-06-30"), None, "a", false).unwrap();
    let mut stmt = d
        .prepare("SELECT date FROM journal_entries WHERE source = 'recurring' ORDER BY date")
        .unwrap();
    let dates: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    // quarterly 2026 Q1..Q4 + 2027 Q1..Q2 = 6; yearly 2026 + 2027 = 2
    assert_eq!(
        dates.iter().filter(|d| d.starts_with("2026-")).count(),
        5,
        "{dates:?}"
    );
    assert_eq!(dates.len(), 8, "{dates:?}");
}

#[test]
fn recurring_end_date_stops_the_schedule() {
    let d = setup();
    tpl(
        &d,
        "Tijdelijk",
        postings_json(&[("4300", 1000), ("1100", -1000)]),
        "monthly",
        1,
        "2026-01-01",
        Some("2026-03-15"),
    );
    bukio::recurring::run_due(&d, Some("2026-12-31"), None, "a", false).unwrap();
    let c: i64 = d
        .query_row(
            "SELECT COUNT(*) FROM journal_entries WHERE source = 'recurring'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(c, 3);
}

#[test]
fn recurring_template_id_runs_only_that_template() {
    let d = setup();
    tpl(
        &d,
        "A",
        postings_json(&[("4300", 1000), ("1100", -1000)]),
        "monthly",
        1,
        "2026-01-01",
        None,
    );
    tpl(
        &d,
        "B",
        postings_json(&[("4300", 2000), ("1100", -2000)]),
        "monthly",
        1,
        "2026-03-01",
        None,
    );
    let r = bukio::recurring::run_due(&d, Some("2026-01-31"), Some(2), "a", false).unwrap();
    assert_eq!(r["templates"].as_array().unwrap().len(), 0, "B not due yet");
    let r2 = bukio::recurring::run_due(&d, Some("2026-03-31"), Some(2), "a", false).unwrap();
    let runs = r2["templates"][0]["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(
        runs[0]["generated"][0]["entry"]["description"],
        json!("B 2026-03-01")
    );
}

#[test]
fn recurring_depreciation_with_residual_absorbs_the_remainder_in_the_final_run() {
    let d = setup();
    let t = bukio::recurring::build_depreciation_template(
        &d,
        "Auto",
        "1800",
        "4600",
        1200000,
        100000,
        12,
        "2026-01-01",
        None,
        "a",
        false,
    )
    .unwrap();
    // (12000 - 1000) / 12 = 916.67/mo; final = 11000 - 916.67*11 = 916.63
    let monthly = t["template"]["postings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["code"] == json!("4600"))
        .unwrap();
    assert_eq!(monthly["amountCents"].as_i64(), Some(91667));
    let fin = t["template"]["final_postings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["code"] == json!("4600"))
        .unwrap();
    assert_eq!(fin["amountCents"].as_i64(), Some(91663));
    assert_eq!(t["total_cents"].as_i64(), Some(1100000));
    let bad = bukio::recurring::build_depreciation_template(
        &d,
        "x",
        "1800",
        "4600",
        1000,
        1000,
        36,
        "2026-01-01",
        None,
        "a",
        false,
    );
    assert_eq!(code_of(bad), "INVALID_RESIDUAL");
}

// --- bank edge cases -------------------------------------------------------

const IBAN: &str = "NL91ABNA0417164300";

fn camt(amount: &str, date: &str, desc: &str) -> String {
    format!(
        r#"<?xml version="1.0"?>
<Document xmlns="urn:iso:std:iso:20022:tech:xsd:camt.053.001.02">
  <BkToCstmrStmt><Stmt><Acct><Id><IBAN>{IBAN}</IBAN></Id></Acct>
    <Ntry><Amt>{amount}</Amt><CdtDbtInd>CRDT</CdtDbtInd><BookgDt><Dt>{date}</Dt></BookgDt>
      <NtryDtls><TxDtls><RltdPties><Dbtr><Nm>ACME B.V.</Nm></Dbtr></RltdPties>
      <RmtInf><Ustrd>{desc}</Ustrd></RmtInf></TxDtls></NtryDtls></Ntry>
  </Stmt></BkToCstmrStmt>
</Document>"#
    )
}

#[test]
fn bank_import_is_idempotent() {
    let d = setup();
    let xml = camt("100.00", "2026-06-01", "Factuur 1");
    let first = bukio::bank::import_transactions(
        &d,
        IBAN,
        &bukio::bank::parse_camt053(&xml).unwrap(),
        None,
        "1100",
        "agent:test",
    )
    .unwrap();
    assert_eq!(first["imported"].as_i64(), Some(1));
    let again = bukio::bank::import_transactions(
        &d,
        IBAN,
        &bukio::bank::parse_camt053(&xml).unwrap(),
        None,
        "1100",
        "agent:test",
    )
    .unwrap();
    assert_eq!(again["imported"].as_i64(), Some(0));
    assert_eq!(again["duplicates"].as_i64(), Some(1));
    assert_eq!(again["total"].as_i64(), Some(1));
}

#[test]
fn bank_rabo_csv_with_af_bij_and_dutch_decimals() {
    let csv = [
        "Datum;Naam / Omschrijving;Rekening;Tegenrekening;Code;Af Bij;Bedrag (EUR);MutatieSoort;Mededelingen",
        &format!("2026-06-01;ACME B.V.;{IBAN};NL00RABO0123456789;GT;Bij;100,00;Overschrijving;Factuur 1"),
        &format!("2026-06-02;Kantoorwinkel BV;{IBAN};NL00RABO9876543210;GT;Af;25,50;Overschrijving;Kantoorartikelen"),
    ]
    .join("\n");
    let txs = bukio::bank::parse_bank_csv(&csv, IBAN).unwrap();
    assert_eq!(txs.transactions.len(), 2, "{txs:?}");
    assert_eq!(txs.transactions[0].amount_cents, 10000);
    assert_eq!(txs.transactions[1].amount_cents, -2550);
}

#[test]
fn bank_auto_match_prefers_an_exact_entry_over_an_invoice() {
    let d = setup();
    add_contact(&d, None);
    let inv = create_invoice(
        &d,
        1,
        &days_from_now(-2),
        None,
        None,
        None,
        None,
        None,
        None,
        &lines(&["1x Werk @ 100.00 @21"]),
        "agent:test",
        false,
    )
    .unwrap();
    let iid = inv["id"].as_i64().unwrap();
    finalize_invoice(&d, iid, "agent:test", false).unwrap(); // gross 121.00 on 1200
                                                             // manually book the receipt on the bank account
    entry(
        &d,
        &days_from_now(-1),
        "Ontvangst",
        specs(&[("1100", 12100), ("1200", -12100)]),
    );

    let xml = camt("121.00", &days_from_now(-1), "Factuur 2026-0001");
    bukio::bank::import_transactions(
        &d,
        IBAN,
        &bukio::bank::parse_camt053(&xml).unwrap(),
        None,
        "1100",
        "agent:test",
    )
    .unwrap();
    let r = bukio::bank::auto_match(&d, 30, "agent:test", false).unwrap();
    assert_eq!(
        r["matched"][0]["kind"],
        json!("entry"),
        "entry wins over invoice"
    );
    let untouched = get_invoice(&d, iid).unwrap().unwrap();
    assert_eq!(untouched["status"], json!("sent"));
}

#[test]
fn bank_partial_payment_does_not_auto_match_the_invoice() {
    let d = setup();
    add_contact(&d, None);
    let inv = create_invoice(
        &d,
        1,
        "2026-07-10",
        None,
        None,
        None,
        None,
        None,
        None,
        &lines(&["1x Werk @ 100.00 @21"]),
        "agent:test",
        false,
    )
    .unwrap();
    finalize_invoice(&d, inv["id"].as_i64().unwrap(), "agent:test", false).unwrap(); // 121.00 open
    let xml = camt("50.00", "2026-07-20", "Factuur 1");
    bukio::bank::import_transactions(
        &d,
        IBAN,
        &bukio::bank::parse_camt053(&xml).unwrap(),
        None,
        "1100",
        "agent:test",
    )
    .unwrap();
    let r = bukio::bank::auto_match(&d, 30, "agent:test", false).unwrap();
    assert_eq!(
        r["matched"].as_array().unwrap().len(),
        0,
        "50.00 != 121.00 open"
    );
    assert_eq!(r["unmatched_remaining"].as_i64(), Some(1));
}

// --- VAT edge cases --------------------------------------------------------

fn book_vat(d: &Connection, desc: &str, spec: &str) {
    bukio::vat::book_vat_entry(
        d,
        "2026-07-01",
        desc,
        &bukio::vat::parse_vat_posting_specs(&[spec.to_string()]).unwrap(),
        "manual",
        None,
        "agent:test",
        true,
    )
    .unwrap();
}

#[test]
fn vat_mixed_rates_in_one_entry_monthly_readout() {
    let d = setup();
    book_vat(&d, "Gemengd", "1100:131.90,8000:-100.00@21,8000:-10.00@9");
    let r = bukio::vat::ob_readout(&d, "2026-07").unwrap();
    assert_eq!(r["fields"]["1a"].as_i64(), Some(10000));
    assert_eq!(r["fields"]["1b"].as_i64(), Some(1000));
    assert_eq!(r["fields"]["5a"].as_i64(), Some(2190));
    assert_eq!(r["fields"]["5d"].as_i64(), Some(2190));
}

#[test]
fn vat_private_use_p_goes_to_1d_and_5a_at_the_standard_rate() {
    let d = setup();
    book_vat(&d, "Privégebruik", "1100:60.50,8000:-50.00@P");
    let r = bukio::vat::ob_readout(&d, "2026-07").unwrap();
    assert_eq!(r["fields"]["1d"].as_i64(), Some(5000));
    assert_eq!(r["fields"]["5a"].as_i64(), Some(1050));
    assert_eq!(r["fields"]["5d"].as_i64(), Some(1050));
}

#[test]
fn vat_private_use_is_always_owed_regardless_of_the_posting_sign() {
    let d = setup();
    // regression: a DEBIT-signed private-use booking used to produce a 2500
    // DEBIT that reduced te-betalen and made 1d/5a negative
    book_vat(
        &d,
        "Privégebruik (old masked form)",
        "1100:-121.00,4700:100.00@P",
    );
    let legs: Vec<(String, i64)> = {
        let mut stmt = d
            .prepare(
                "SELECT a.code, p.amount_cents FROM postings p
                   JOIN accounts a ON a.id = p.account_id
                  WHERE p.entry_id = (SELECT MAX(id) FROM journal_entries)
                  ORDER BY a.code",
            )
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
    };
    let vat_leg = legs
        .iter()
        .find(|(c, _)| c == "2500")
        .unwrap_or_else(|| panic!("private use must book a 2500 VAT leg: {legs:?}"));
    assert_eq!(vat_leg.1, -2100, "the 2500 leg must be a CREDIT (you owe)");
    let r = bukio::vat::ob_readout(&d, "2026-07").unwrap();
    assert_eq!(
        r["fields"]["1d"].as_i64(),
        Some(10000),
        "base always positive"
    );
    assert_eq!(r["fields"]["5a"].as_i64(), Some(2100));
    assert_eq!(r["fields"]["5d"].as_i64(), Some(2100));
}

#[test]
fn vat_reverse_charge_income_reports_the_base_in_1c() {
    let d = setup();
    book_vat(
        &d,
        "Verlegd binnenland uitgaand",
        "1100:121.00,8000:-100.00@R",
    );
    let r = bukio::vat::ob_readout(&d, "2026-07").unwrap();
    assert_eq!(r["fields"]["1c"].as_i64(), Some(10000));
    assert_eq!(r["fields"]["1a"].as_i64(), Some(0));
    assert_eq!(r["fields"]["2a"].as_i64(), Some(0));
    assert_eq!(r["fields"]["5a"].as_i64(), Some(0));
}

// --- year-end / jaarrekening edge cases ------------------------------------

#[test]
fn year_end_loss_year_closes_with_negative_result_into_equity() {
    let d = setup();
    entry(
        &d,
        "2026-03-01",
        "Kosten",
        specs(&[("4300", 5000), ("1100", -5000)]),
    );
    let r = bukio::year_end::year_end_close(&d, "2026", "a", false).unwrap();
    assert_eq!(r["result_cents"].as_i64(), Some(-5000));
    let eq: i64 = d
        .query_row(
            "SELECT COALESCE(SUM(p.amount_cents),0) FROM postings p
               JOIN journal_entries e ON e.id = p.entry_id
              WHERE e.state = 'posted'
                AND p.account_id = (SELECT id FROM accounts WHERE code = '3000')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(eq, 5000, "verlies deblokkeert EV");
}

#[test]
fn year_end_fiscal_year_end_0630_drives_the_jaarrekening_as_of() {
    let d = setup();
    d.execute("UPDATE company SET fiscal_year_end = '06-30'", [])
        .unwrap();
    entry(
        &d,
        "2026-03-01",
        "Omzet",
        specs(&[("1100", 12100), ("8000", -10000), ("2500", -2100)]),
    );
    let r = bukio::reports::jaarrekening(&d, "2026", Some("micro")).unwrap();
    assert_eq!(r["as_of"], json!("2026-06-30"));
    assert_eq!(r["balans"]["total_activa_cents"].as_i64(), Some(12100));
}

#[test]
fn jaarrekening_custom_account_lands_in_overig_and_still_balances() {
    let d = setup();
    bukio::accounts::create_account(
        &d,
        &bukio::accounts::NewAccount {
            code: "1999",
            name: "Crypto",
            type_: "asset",
            normal_balance: "debit",
            taxonomy_code: None,
        },
    )
    .unwrap();
    entry(
        &d,
        "2026-03-01",
        "Omzet",
        specs(&[("1999", 10000), ("8000", -10000)]),
    );
    let r = bukio::reports::jaarrekening(&d, "2026", Some("klein")).unwrap();
    let overig = r["balans"]["activa"]
        .as_array()
        .unwrap()
        .iter()
        .find(|g| g["label"] == json!("Overig"))
        .cloned()
        .unwrap_or_else(|| panic!("no Overig group: {r}"));
    assert_eq!(overig["total_cents"].as_i64(), Some(10000));
    assert_eq!(
        r["balans"]["total_activa_cents"].as_i64(),
        r["balans"]["total_passiva_cents"].as_i64()
    );
}

#[test]
fn jaarrekening_micro_with_no_activity_is_zero_and_balanced() {
    let d = setup();
    let r = bukio::reports::jaarrekening(&d, "2026", Some("micro")).unwrap();
    assert_eq!(r["balans"]["total_activa_cents"].as_i64(), Some(0));
    assert_eq!(r["balans"]["total_passiva_cents"].as_i64(), Some(0));
    assert_eq!(r["balans"]["balanced"], json!(true));
}

#[test]
fn year_end_closing_two_years_works_independently() {
    let d = setup();
    entry(
        &d,
        "2025-03-01",
        "Omzet 2025",
        specs(&[("1100", 5000), ("8000", -5000)]),
    );
    entry(
        &d,
        "2026-03-01",
        "Omzet 2026",
        specs(&[("1100", 8000), ("8000", -8000)]),
    );
    bukio::year_end::year_end_close(&d, "2025", "a", false).unwrap();
    bukio::year_end::year_end_close(&d, "2026", "a", false).unwrap();
    let p26 = bukio::reports::pnl(&d, "2026-01-01", "2026-12-31").unwrap();
    assert_eq!(
        p26["revenue_cents"].as_i64(),
        Some(8000),
        "2025 close untouched 2026"
    );
}

// --- ICP edge cases --------------------------------------------------------

#[test]
fn icp_credit_note_reduces_the_customer_total_and_period_boundaries_hold() {
    let d = setup();
    let de = bukio::contacts::create_contact(
        &d,
        "GmbH Berlin",
        Some("H 1"),
        None,
        Some("Berlin"),
        Some("DE"),
        None,
        Some("DE123456789"),
        None,
        None,
        "a",
        false,
    )
    .unwrap();
    let cid = de["id"].as_i64().unwrap();
    let inv = create_invoice(
        &d,
        cid,
        "2026-07-10",
        None,
        None,
        None,
        None,
        None,
        None,
        &lines(&["1x Advies @ 2000.00 @RE"]),
        "a",
        false,
    )
    .unwrap();
    let iid = inv["id"].as_i64().unwrap();
    finalize_invoice(&d, iid, "a", false).unwrap();
    let credit = credit_invoice(&d, iid, None, None, "a", false).unwrap();
    finalize_invoice(&d, credit["id"].as_i64().unwrap(), "a", false).unwrap();

    let r = bukio::reports::icp_readout(&d, "2026-Q3").unwrap();
    assert_eq!(
        r["customers"][0]["amount_cents"].as_i64(),
        Some(0),
        "2000 - 2000"
    );
    assert_eq!(r["total_cents"].as_i64(), Some(0));
    let q2 = bukio::reports::icp_readout(&d, "2026-Q2").unwrap();
    assert_eq!(q2["customers"].as_array().unwrap().len(), 0);
}

#[test]
fn icp_re_base_uses_the_discounted_amount() {
    let d = setup();
    let de = bukio::contacts::create_contact(
        &d,
        "GmbH Hamburg",
        Some("H 1"),
        None,
        Some("Hamburg"),
        Some("DE"),
        None,
        Some("DE987654321"),
        None,
        None,
        "a",
        false,
    )
    .unwrap();
    let inv = create_invoice(
        &d,
        de["id"].as_i64().unwrap(),
        "2026-07-10",
        None,
        None,
        None,
        None,
        Some("pct"),
        Some(1000),
        &lines(&["1x Levering @ 1000.00 @RE"]),
        "a",
        false,
    )
    .unwrap();
    let iid = inv["id"].as_i64().unwrap();
    finalize_invoice(&d, iid, "a", false).unwrap();
    let r = bukio::reports::icp_readout(&d, "2026-Q3").unwrap();
    assert_eq!(
        r["customers"][0]["amount_cents"].as_i64(),
        Some(90000),
        "900.00 not 1000.00"
    );

    let credit = credit_invoice(&d, iid, None, None, "a", false).unwrap();
    // the credit must carry the ORIGINAL invoice-level discount, not just the
    // line amounts: booking it undiscounted over-credits the customer by 100.00
    assert_eq!(credit["discount_type"], json!("pct"));
    assert_eq!(credit["discount_value"].as_i64(), Some(1000));
    assert_eq!(
        credit["net_cents"].as_i64(),
        Some(90000),
        "discounted credit base"
    );
    finalize_invoice(&d, credit["id"].as_i64().unwrap(), "a", false).unwrap();
    let r2 = bukio::reports::icp_readout(&d, "2026-Q3").unwrap();
    assert_eq!(r2["customers"][0]["amount_cents"].as_i64(), Some(0));
}

// --- fx --------------------------------------------------------------------

#[test]
fn fx_rate_raw_float_parses_as_1_0875_not_scaled_again() {
    let d = setup();
    let stored = bukio::fx::set_fx_rate(
        &d,
        "USD",
        "2026-08-01",
        "1.0875",
        "manual",
        "agent:test",
        false,
    )
    .unwrap();
    assert_eq!(stored["rate_x10000"].as_i64(), Some(10875));
    assert_eq!(stored["rate"], json!("1.0875"));
    assert_eq!(bukio::fx::convert_fx(10000, 10875).unwrap(), 9195); // $100 -> EUR 91.95
                                                                    // NOTE: the JS also accepts an already-scaled INTEGER when the caller
                                                                    // passes a JS number (the ECB/MCP path). The port's set_fx_rate takes a
                                                                    // string, so there is no number-vs-string distinction to make; a string
                                                                    // "10875" goes through parse_rate and is rejected on BOTH implementations
                                                                    // (`fx set --rate 10875`), which is the parity that matters.
    assert_eq!(
        code_of(bukio::fx::set_fx_rate(
            &d,
            "USD",
            "2026-08-02",
            "10875",
            "manual",
            "agent:test",
            false
        )),
        "INVALID_RATE"
    );
}

// --- dry-run hygiene -------------------------------------------------------

#[test]
fn all_mutating_paths_leave_no_trace_in_dry_run() {
    let d = setup();
    add_contact(&d, None);
    let inv = create_invoice(
        &d,
        1,
        "2026-07-10",
        None,
        None,
        None,
        None,
        None,
        None,
        &lines(&["1x X @ 100.00 @21"]),
        "agent:test",
        false,
    )
    .unwrap();
    let iid = inv["id"].as_i64().unwrap();
    finalize_invoice(&d, iid, "agent:test", true).unwrap();
    assert!(
        get_invoice(&d, iid).unwrap().unwrap()["invoice_number"].is_null(),
        "dry-run finalize must not number the invoice"
    );

    tpl(
        &d,
        "T",
        postings_json(&[("4300", 1000), ("1100", -1000)]),
        "monthly",
        1,
        "2026-01-01",
        None,
    );
    bukio::recurring::run_due(&d, Some("2026-12-31"), None, "a", true).unwrap();
    let rec: i64 = d
        .query_row(
            "SELECT COUNT(*) FROM journal_entries WHERE source = 'recurring'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(rec, 0, "dry-run recurring wrote entries");

    bukio::year_end::year_end_close(&d, "2026", "a", true).unwrap();
    let closing: i64 = d
        .query_row(
            "SELECT COUNT(*) FROM journal_entries WHERE source = 'closing'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(closing, 0, "dry-run year-end wrote the closing entry");
}

// --- a missing database must never be created by a read --------------------

#[test]
fn a_read_command_never_creates_a_missing_database() {
    // JS: ensureDb(ctx, {mustExist:false}) returns null and leaves no file. The
    // port's equivalent lives in the CLI's open_existing, so this drives the
    // binary — same guarantee, the real surface.
    let dir = std::env::temp_dir().join(format!("bukio-nodb-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let missing = dir.join("absent.db");

    let exe = env!("CARGO_BIN_EXE_bukio");
    let out = std::process::Command::new(exe)
        .args([
            "invoice",
            "list",
            "--db",
            missing.to_str().unwrap(),
            "--json",
            "--actor",
            "human:test",
        ])
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "a missing db must fail, not yield a handle"
    );
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["error"]["code"], json!("NO_DATABASE"), "{v}");
    assert!(!missing.exists(), "the file must not be created");
    let _ = std::fs::remove_dir_all(&dir);
}

// ==== ported from test/year-end.test.js =====================================
// NOTE: 3 of the 21 JS tests assert the jaarrekening HTML/PDF renderer
// (jaarrekeningHtml / jaarrekeningToPdf, playwright). The port has no renderer
// (no pdf/html module), so those are an unported FEATURE, not a test failure.
// The 18 engine tests below cover the accounting behaviour.

#[test]
fn year_end_close_posts_closing_and_appropriation() {
    let d = setup();
    entry(
        &d,
        "2026-03-01",
        "Omzet A",
        specs(&[("1100", 12100), ("8000", -10000), ("2500", -2100)]),
    );
    entry(
        &d,
        "2026-04-01",
        "Software",
        specs(&[("4300", 3000), ("1100", -3000)]),
    );

    let result = bukio::year_end::year_end_close(&d, "2026", "agent:test", false).unwrap();
    assert_eq!(result["closed"], json!(true));
    assert_eq!(result["result_cents"].as_i64(), Some(7000));

    // 9900 created on demand (equity)
    let acc = bukio::accounts::get_account_by_code(&d, "9900").expect("9900 created");
    assert_eq!(acc["type"], json!("equity"));

    // two closing entries, posted, tagged
    let mut stmt = d
        .prepare("SELECT id, date, description, state, source_ref FROM journal_entries WHERE source = 'closing' ORDER BY id")
        .unwrap();
    let closing: Vec<(i64, String, String, String, String)> = stmt
        .query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert_eq!(closing.len(), 2, "{closing:?}");
    assert!(closing.iter().all(|c| c.3 == "posted" && c.4 == "fy:2026"));
    assert_eq!(closing[0].2, "Afsluiting boekjaar 2026");
    assert_eq!(closing[1].2, "Resultaatbestemming 2026");

    // income closed to zero, equity credited with the result, ledger balanced
    let balance = |code: &str| -> i64 {
        d.query_row(
            "SELECT COALESCE(SUM(p.amount_cents),0) FROM postings p
               JOIN journal_entries e ON e.id = p.entry_id
              WHERE e.state = 'posted' AND p.account_id = (SELECT id FROM accounts WHERE code = ?1)",
            [code],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert_eq!(balance("8000"), 0, "omzet closed out");
    assert_eq!(balance("3000"), -7000, "equity credited with the result");
    let totals: i64 = d
        .query_row(
            "SELECT COALESCE(SUM(amount_cents),0) FROM postings",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(totals, 0);

    // second close rejected
    assert_eq!(
        code_of(bukio::year_end::year_end_close(
            &d,
            "2026",
            "agent:test",
            false
        )),
        "ALREADY_CLOSED"
    );
}

#[test]
fn year_end_reversing_the_closing_entries_reopens_the_year() {
    let d = setup();
    entry(
        &d,
        "2026-03-01",
        "Omzet",
        specs(&[("1100", 12100), ("8000", -10000), ("2500", -2100)]),
    );
    let closed = bukio::year_end::year_end_close(&d, "2026", "agent:test", false).unwrap();
    assert_eq!(closed["closed"], json!(true));
    assert!(bukio::year_end::is_year_closed(&d, "2026").unwrap());

    // the documented undo: reverse the closing entries
    let ids: Vec<i64> = {
        let mut stmt = d
            .prepare("SELECT id FROM journal_entries WHERE source = 'closing' AND source_ref = 'fy:2026'")
            .unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
    };
    for id in ids {
        reverse_entry(&d, id, "agent:test", None).unwrap();
    }

    // regression: a reversed closing entry used to keep the year locked forever
    assert!(
        !bukio::year_end::is_year_closed(&d, "2026").unwrap(),
        "reversing the closing entries must re-open the year"
    );
    let status = bukio::compliance::compliance_status(&d, 2026).unwrap();
    let ar = status["obligations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["type"] == json!("JAARREKENING") && o["period"] == json!("2026"))
        .cloned()
        .unwrap_or_else(|| panic!("no JAARREKENING obligation: {status}"));
    assert_eq!(
        ar["books_closed"],
        json!(false),
        "calendar must show the year open"
    );

    // and the year can be closed again
    let reopened = bukio::year_end::year_end_close(&d, "2026", "agent:test", false).unwrap();
    assert_eq!(reopened["closed"], json!(true));
}

#[test]
fn year_end_guards_drafts_block_and_empty_year_reports() {
    let d = setup();
    draft(
        &d,
        "2026-05-01",
        "draft",
        specs(&[("1100", 100), ("3000", -100)]),
    );
    assert_eq!(
        code_of(bukio::year_end::year_end_close(
            &d,
            "2026",
            "agent:test",
            false
        )),
        "INCOMPLETE_YEAR"
    );

    let empty = bukio::year_end::year_end_close(&d, "2025", "agent:test", false).unwrap();
    assert_eq!(empty["closed"], json!(false));
    assert_eq!(empty["reason"], json!("EMPTY_YEAR"));
    assert_eq!(
        code_of(bukio::year_end::year_end_close(
            &d,
            "bad",
            "agent:test",
            false
        )),
        "INVALID_YEAR"
    );
}

#[test]
fn year_end_dry_run_writes_nothing() {
    let d = setup();
    entry(
        &d,
        "2026-03-01",
        "Omzet",
        specs(&[("1100", 12100), ("8000", -10000), ("2500", -2100)]),
    );
    let plan = bukio::year_end::year_end_close(&d, "2026", "agent:test", true).unwrap();
    assert_eq!(plan["dryRun"], json!(true));
    assert_eq!(plan["result_cents"].as_i64(), Some(10000));
    assert_eq!(plan["create_9900"], json!(true));
    assert_eq!(plan["entries"].as_array().unwrap().len(), 2);
    let c: i64 = d
        .query_row(
            "SELECT COUNT(*) FROM journal_entries WHERE source = 'closing'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(c, 0);
    assert!(bukio::accounts::get_account_by_code(&d, "9900").is_none());
}

#[test]
fn pnl_still_shows_the_year_result_after_closing() {
    let d = setup();
    entry(
        &d,
        "2026-03-01",
        "Omzet",
        specs(&[("1100", 12100), ("8000", -10000), ("2500", -2100)]),
    );
    entry(
        &d,
        "2026-04-01",
        "Kosten",
        specs(&[("4300", 3000), ("1100", -3000)]),
    );
    bukio::year_end::year_end_close(&d, "2026", "agent:test", false).unwrap();
    let r = bukio::reports::pnl(&d, "2026-01-01", "2026-12-31").unwrap();
    assert_eq!(r["revenue_cents"].as_i64(), Some(10000));
    assert_eq!(r["costs_cents"].as_i64(), Some(3000));
    assert_eq!(r["result_cents"].as_i64(), Some(7000));
}

#[test]
fn jaarrekening_klein_statutory_balans_and_wv() {
    let d = setup();
    entry(
        &d,
        "2026-03-01",
        "Omzet",
        specs(&[("1100", 12100), ("8000", -10000), ("2500", -2100)]),
    );
    entry(
        &d,
        "2026-04-01",
        "Kosten",
        specs(&[("4300", 3000), ("1100", -3000)]),
    );

    let r = bukio::reports::jaarrekening(&d, "2026", Some("klein")).unwrap();
    assert_eq!(r["model"], json!("klein"));
    assert_eq!(r["balans"]["balanced"], json!(true));
    assert_eq!(
        r["balans"]["total_activa_cents"].as_i64(),
        r["balans"]["total_passiva_cents"].as_i64()
    );
    let labels: Vec<&str> = r["balans"]["activa"]
        .as_array()
        .unwrap()
        .iter()
        .map(|g| g["label"].as_str().unwrap_or(""))
        .collect();
    assert!(labels.contains(&"Liquide middelen"), "{labels:?}");
    let ev = r["balans"]["passiva"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["taxonomy_code"] == json!("BEIV.05"))
        .cloned()
        .expect("BEIV.05 eigen vermogen");
    assert_eq!(
        ev["total_cents"].as_i64(),
        Some(7000),
        "onverdeeld resultaat pre-close"
    );
    assert_eq!(r["pnl"]["omzet_cents"].as_i64(), Some(10000));
    assert_eq!(r["pnl"]["inkoop_cents"].as_i64(), Some(0));
    assert_eq!(r["pnl"]["resultaat_cents"].as_i64(), Some(7000));
}

#[test]
fn jaarrekening_klein_counts_inkoop_once_and_adds_overige_opbrengsten() {
    let d = setup();
    entry(
        &d,
        "2026-03-01",
        "Omzet",
        specs(&[("1100", 12100), ("8000", -10000), ("2500", -2100)]),
    );
    entry(
        &d,
        "2026-04-01",
        "Inkoop",
        specs(&[("4000", 4000), ("1100", -4000)]),
    );
    entry(
        &d,
        "2026-05-01",
        "Overige opbrengst",
        specs(&[("8100", -500), ("1100", 500)]),
    );
    entry(
        &d,
        "2026-06-01",
        "Kosten",
        specs(&[("4300", 2000), ("1100", -2000)]),
    );

    let r = bukio::reports::jaarrekening(&d, "2026", Some("klein")).unwrap();
    let p = &r["pnl"];
    assert_eq!(p["omzet_cents"].as_i64(), Some(10000));
    assert_eq!(p["overige_opbrengsten_cents"].as_i64(), Some(500));
    assert_eq!(p["inkoop_cents"].as_i64(), Some(4000));
    assert_eq!(p["bruto_marge_cents"].as_i64(), Some(6000));
    assert_eq!(
        p["kosten_cents"].as_i64(),
        Some(2000),
        "operating costs only, no inkoop"
    );
    assert_eq!(p["resultaat_cents"].as_i64(), Some(4500));
    assert_eq!(p["resultaat"], json!("45.00"));
}

#[test]
fn jaarrekening_after_closing_result_sits_in_equity_and_micro_has_no_wv() {
    let d = setup();
    entry(
        &d,
        "2026-03-01",
        "Omzet",
        specs(&[("1100", 12100), ("8000", -10000), ("2500", -2100)]),
    );
    bukio::year_end::year_end_close(&d, "2026", "agent:test", false).unwrap();
    let r = bukio::reports::jaarrekening(&d, "2026", Some("micro")).unwrap();
    let ev = r["balans"]["passiva"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["taxonomy_code"] == json!("BEIV.05"))
        .cloned()
        .expect("BEIV.05");
    assert_eq!(
        ev["total_cents"].as_i64(),
        Some(10000),
        "result closed into equity"
    );
    assert!(
        !ev["sections"]
            .as_array()
            .map(|ss| ss
                .iter()
                .any(|s| s["label"] == json!("Onverdeeld resultaat")))
            .unwrap_or(false),
        "no onverdeeld after the close: {ev}"
    );
    assert!(r.get("pnl").is_none(), "micro has no W&V");
}

#[test]
fn jaarrekening_klein_pnl_follows_the_fiscal_year() {
    let d = setup();
    d.execute("UPDATE company SET fiscal_year_end = '06-30'", [])
        .unwrap();
    entry(
        &d,
        "2025-06-15",
        "Omzet te vroeg",
        specs(&[("1100", 1000), ("8000", -1000)]),
    );
    entry(
        &d,
        "2025-09-01",
        "Omzet 1",
        specs(&[("1100", 2000), ("8000", -2000)]),
    );
    entry(
        &d,
        "2026-06-30",
        "Omzet 2",
        specs(&[("1100", 3000), ("8000", -3000)]),
    );
    entry(
        &d,
        "2026-07-15",
        "Omzet te laat",
        specs(&[("1100", 4000), ("8000", -4000)]),
    );

    let r = bukio::reports::jaarrekening(&d, "2026", Some("klein")).unwrap();
    assert_eq!(r["as_of"], json!("2026-06-30"));
    assert_eq!(r["balans"]["balanced"], json!(true));
    assert_eq!(
        r["pnl"]["omzet_cents"].as_i64(),
        Some(5000),
        "fy window only"
    );
    assert_eq!(r["pnl"]["resultaat_cents"].as_i64(), Some(5000));
}

#[test]
fn year_end_close_follows_the_fiscal_year() {
    let d = setup();
    d.execute("UPDATE company SET fiscal_year_end = '06-30'", [])
        .unwrap();
    entry(
        &d,
        "2025-06-15",
        "Te vroeg",
        specs(&[("1100", 1000), ("8000", -1000)]),
    );
    entry(
        &d,
        "2025-09-01",
        "Omzet 1",
        specs(&[("1100", 2000), ("8000", -2000)]),
    );
    entry(
        &d,
        "2026-06-30",
        "Omzet 2",
        specs(&[("1100", 3000), ("8000", -3000)]),
    );
    entry(
        &d,
        "2026-07-15",
        "Te laat",
        specs(&[("1100", 4000), ("8000", -4000)]),
    );

    bukio::year_end::year_end_close(&d, "2026", "agent:test", false).unwrap();
    let first_date: String = d
        .query_row(
            "SELECT date FROM journal_entries WHERE source = 'closing' ORDER BY id LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(first_date, "2026-06-30", "dated at the FISCAL year end");
    let status = bukio::year_end::year_end_status(&d, "2026").unwrap();
    assert_eq!(
        status["result_cents"].as_i64(),
        Some(5000),
        "in-window result only"
    );

    // outside-window entries were NOT closed
    let leftover: i64 = d
        .query_row(
            "SELECT COALESCE(SUM(p.amount_cents),0) FROM postings p
               JOIN journal_entries e ON e.id = p.entry_id AND e.state = 'posted' AND e.source != 'closing'
               JOIN accounts a ON a.id = p.account_id WHERE a.code = '8000'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(leftover, -10000);
}

#[test]
fn jaarrekening_invalid_model_rejected() {
    let d = setup();
    assert_eq!(
        code_of(bukio::reports::jaarrekening(&d, "2026", Some("groot"))),
        "INVALID_MODEL"
    );
}

#[test]
fn jaarrekening_account_amounts_are_numbers_never_nan() {
    let d = setup();
    entry(
        &d,
        "2026-03-01",
        "Omzet",
        specs(&[("1100", 12100), ("8000", -10000), ("2500", -2100)]),
    );
    entry(
        &d,
        "2026-03-05",
        "Laptop",
        specs(&[("1800", 537000), ("1100", -537000)]),
    );
    let r = bukio::reports::jaarrekening(&d, "2026", Some("klein")).unwrap();

    let mut accounts: Vec<Value> = Vec::new();
    for side in ["activa", "passiva"] {
        for g in r["balans"][side].as_array().unwrap() {
            accounts.extend(g["accounts"].as_array().cloned().unwrap_or_default());
            for sec in g["sections"].as_array().unwrap_or(&vec![]) {
                accounts.extend(sec["accounts"].as_array().cloned().unwrap_or_default());
            }
        }
    }
    if let Some(lines) = r
        .get("pnl")
        .and_then(|p| p.get("lines"))
        .and_then(|l| l.as_array())
    {
        for l in lines {
            for s in l["sections"].as_array().unwrap_or(&vec![]) {
                accounts.extend(s["accounts"].as_array().cloned().unwrap_or_default());
            }
        }
    }
    assert!(
        accounts.len() >= 3,
        "expected account detail rows: {accounts:?}"
    );
    for a in &accounts {
        let n = a["amount_cents"]
            .as_i64()
            .unwrap_or_else(|| panic!("{} amount_cents must be a number: {a}", a["name"]));
        assert_ne!(n, i64::MIN);
    }
}

#[test]
fn jaarrekening_pnl_includes_the_afschrijvingen_line() {
    let d = setup();
    entry(
        &d,
        "2026-03-01",
        "Afschr",
        specs(&[("1800", -10000), ("4600", 10000)]),
    );
    let r = bukio::reports::jaarrekening(&d, "2026", Some("klein")).unwrap();
    let line = r["pnl"]["lines"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["taxonomy_code"] == json!("WAFS.41"))
        .cloned()
        .unwrap_or_else(|| panic!("WAFS.41 must map to an Afschrijvingen line: {r}"));
    assert_eq!(line["label"], json!("Afschrijvingen"));
    assert_eq!(line["total_cents"].as_i64(), Some(10000));
}

#[test]
fn ob_readout_verlegde_inkoop_and_verkoop() {
    let d = setup();
    // binnenlandse verlegde inkoop
    book_vat(
        &d,
        "Inkoop verlegd binnenland",
        "4300:100.00@R,1100:-100.00,2500:-21.00",
    );
    let mut r = bukio::vat::ob_readout(&d, "2026-Q3").unwrap();
    assert_eq!(r["fields"]["3a"].as_i64(), Some(10000));
    assert_eq!(r["fields"]["4a"].as_i64(), Some(2100));
    assert_eq!(r["fields"]["5b"].as_i64(), Some(2100));
    assert_eq!(r["fields"]["5d"].as_i64(), Some(0));

    // EU verlegde inkoop
    book_vat(
        &d,
        "Inkoop verlegd EU",
        "4300:500.00@RE,1100:-500.00,2500:-105.00",
    );
    r = bukio::vat::ob_readout(&d, "2026-Q3").unwrap();
    assert_eq!(r["fields"]["3b"].as_i64(), Some(50000));
    assert_eq!(r["fields"]["4b"].as_i64(), Some(10500));
    assert_eq!(r["fields"]["5b"].as_i64(), Some(12600));
    assert_eq!(r["fields"]["5d"].as_i64(), Some(0));
}

#[test]
fn ob_readout_verlegde_eu_sale_reports_2a() {
    let d = setup();
    let c = bukio::contacts::create_contact(
        &d,
        "GmbH Berlin",
        Some("Hauptstr 1"),
        None,
        Some("Berlin"),
        Some("DE"),
        None,
        Some("DE123456789"),
        None,
        None,
        "agent:test",
        false,
    )
    .unwrap();
    let inv = create_invoice(
        &d,
        c["id"].as_i64().unwrap(),
        "2026-07-10",
        None,
        None,
        None,
        None,
        None,
        None,
        &lines(&["1x Advies @ 2000.00 @RE"]),
        "agent:test",
        false,
    )
    .unwrap();
    finalize_invoice(&d, inv["id"].as_i64().unwrap(), "agent:test", false).unwrap();
    let r = bukio::vat::ob_readout(&d, "2026-Q3").unwrap();
    assert_eq!(r["fields"]["2a"].as_i64(), Some(200000));
    assert_eq!(r["fields"]["1a"].as_i64(), Some(0));
}

#[test]
fn icp_readout_totals_per_eu_customer() {
    let d = setup();
    let mk = |name: &str, vat_id: &str, country: &str| -> i64 {
        bukio::contacts::create_contact(
            &d,
            name,
            Some("Str 1"),
            None,
            Some("City"),
            Some(country),
            None,
            Some(vat_id),
            None,
            None,
            "agent:test",
            false,
        )
        .unwrap()["id"]
            .as_i64()
            .unwrap()
    };
    let de = mk("GmbH Berlin", "DE123456789", "DE");
    let be = mk("NV Brussel", "BE0123456789", "BE");
    for (cid, date, line) in [
        (de, "2026-07-10", "1x Advies @ 2000.00 @RE"),
        (de, "2026-08-01", "1x Advies @ 500.00 @RE"),
        (be, "2026-09-05", "1x Support @ 300.00 @RE"),
    ] {
        let inv = create_invoice(
            &d,
            cid,
            date,
            None,
            None,
            None,
            None,
            None,
            None,
            &lines(&[line]),
            "agent:test",
            false,
        )
        .unwrap();
        finalize_invoice(&d, inv["id"].as_i64().unwrap(), "agent:test", false).unwrap();
    }

    let r = bukio::reports::icp_readout(&d, "2026-Q3").unwrap();
    let customers = r["customers"].as_array().unwrap();
    assert_eq!(customers.len(), 2, "{r}");
    let de_row = customers
        .iter()
        .find(|c| c["name"] == json!("GmbH Berlin"))
        .unwrap();
    assert_eq!(de_row["amount_cents"].as_i64(), Some(250000));
    assert_eq!(de_row["vat_id"], json!("DE123456789"));
    assert_eq!(de_row["invoice_numbers"].as_array().unwrap().len(), 2);
    let be_row = customers
        .iter()
        .find(|c| c["name"] == json!("NV Brussel"))
        .unwrap();
    assert_eq!(be_row["amount_cents"].as_i64(), Some(30000));
    assert_eq!(r["total_cents"].as_i64(), Some(280000));
}

#[test]
fn icp_readout_missing_customer_vat_id_fails_loudly() {
    let d = setup();
    let c = bukio::contacts::create_contact(
        &d,
        "GmbH Ohne Vat",
        Some("Hauptstr 1"),
        None,
        Some("Berlin"),
        Some("DE"),
        None,
        Some("DE123456789"),
        None,
        None,
        "agent:test",
        false,
    )
    .unwrap();
    let cid = c["id"].as_i64().unwrap();
    let inv = create_invoice(
        &d,
        cid,
        "2026-07-10",
        None,
        None,
        None,
        None,
        None,
        None,
        &lines(&["1x Advies @ 2000.00 @RE"]),
        "agent:test",
        false,
    )
    .unwrap();
    finalize_invoice(&d, inv["id"].as_i64().unwrap(), "agent:test", false).unwrap();
    // compliance guarantees a vat-id at finalize — simulate it being lost later
    d.execute("UPDATE contacts SET vat_id = NULL WHERE id = ?1", [cid])
        .unwrap();
    assert_eq!(
        code_of(bukio::reports::icp_readout(&d, "2026-Q3")),
        "ICP_VAT_ID_MISSING"
    );
}

#[test]
fn icp_readout_no_re_lines_gives_an_empty_listing() {
    let d = setup();
    let r = bukio::reports::icp_readout(&d, "2026-Q3").unwrap();
    assert_eq!(r["customers"].as_array().unwrap().len(), 0);
    assert_eq!(r["total_cents"].as_i64(), Some(0));
}
