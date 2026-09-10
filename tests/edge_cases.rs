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

/// parsed item specs (the object form create_invoice takes alongside line strings)
fn item_spec(v: &[&str]) -> Vec<Value> {
    v.iter()
        .map(|s| bukio::invoice::parse_item_spec(s).unwrap())
        .collect()
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
        None,
        &lines(&["1x Werk @ 100.00 @21"]),
        "agent:test",
        false,
    )
    .unwrap();
    let id = inv["id"].as_i64().unwrap();
    finalize_invoice(&d, id, "agent:test", false).unwrap();
    mark_paid(&d, id, "2026-07-20", 12100, "transfer", "agent:test", false).unwrap();
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
        None,
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

// ==== ported from test/reports-v014.test.js =================================
// aging / contact statement / sales, plus a CLI + MCP e2e (part B below).

fn vat_off() -> Connection {
    setup_with_vat(false)
}

fn make_finalized(
    db: &Connection,
    contact_id: i64,
    date: &str,
    due_days: Option<i64>,
    lines_raw: &[&str],
) -> Value {
    let inv = create_invoice(
        db,
        contact_id,
        date,
        due_days,
        None,
        None,
        None,
        None,
        None,
        None,
        &lines(lines_raw),
        "agent:test",
        false,
    )
    .unwrap();
    let id = inv["id"].as_i64().unwrap();
    finalize_invoice(db, id, "agent:test", false).unwrap();
    get_invoice(db, id).unwrap().unwrap()
}

fn contact_id(db: &Connection, name: &str) -> i64 {
    bukio::contacts::create_contact(
        db,
        name,
        Some("Klantstraat 1"),
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
    .unwrap()["id"]
        .as_i64()
        .unwrap()
}

#[test]
fn aging_debtors_buckets_totals_paid_excluded_sorted() {
    let d = vat_off();
    let acme = contact_id(&d, "Acme BV");
    let beta = contact_id(&d, "Beta BV");

    // due 2026-06-01 -> 68 days past the as-of -> d90 (61-90)
    make_finalized(&d, acme, "2026-05-01", Some(31), &["Ding @ 100.00"]);
    // due 2026-07-20 -> 19 days past -> d30
    make_finalized(&d, acme, "2026-06-20", Some(30), &["Ding @ 100.00"]);
    // fully paid — must NOT appear
    let paid = make_finalized(&d, acme, "2026-07-01", Some(30), &["Ding @ 100.00"]);
    mark_paid(
        &d,
        paid["id"].as_i64().unwrap(),
        "2026-07-10",
        paid["gross_cents"].as_i64().unwrap(),
        "bank",
        "agent:test",
        false,
    )
    .unwrap();
    // due after the as-of -> current
    make_finalized(&d, beta, "2026-08-01", Some(30), &["Ding @ 100.00"]);

    let r = bukio::reports::aging(&d, "2026-08-08", "debtors").unwrap();
    assert_eq!(r["kind"], json!("debtors"));
    let contacts = r["debtors"]["contacts"].as_array().unwrap();
    assert_eq!(contacts.len(), 2, "{r}");
    let ac = contacts
        .iter()
        .find(|c| c["contact_id"] == json!(acme))
        .unwrap();
    assert_eq!(ac["buckets"]["d90"].as_i64(), Some(10000));
    assert_eq!(ac["buckets"]["d30"].as_i64(), Some(10000));
    assert_eq!(ac["buckets"]["current"].as_i64(), Some(0));
    assert_eq!(ac["buckets"]["d60"].as_i64(), Some(0));
    assert_eq!(ac["total_cents"].as_i64(), Some(20000));
    assert_eq!(ac["items"].as_array().unwrap().len(), 2);
    assert!(ac["items"]
        .as_array()
        .unwrap()
        .iter()
        .all(|i| i["outstanding_cents"].as_i64() == Some(10000)));
    let be = contacts
        .iter()
        .find(|c| c["contact_id"] == json!(beta))
        .unwrap();
    assert_eq!(be["buckets"]["current"].as_i64(), Some(10000));
    assert_eq!(r["debtors"]["totals"]["total_cents"].as_i64(), Some(30000));
    assert_eq!(r["debtors"]["totals"]["d90"].as_i64(), Some(10000));
    assert_eq!(r["debtors"]["totals"]["d30"].as_i64(), Some(10000));
    assert_eq!(r["debtors"]["totals"]["current"].as_i64(), Some(10000));
}

#[test]
fn aging_debtors_excludes_invoices_after_as_of_and_nets_credits_fifo() {
    let d = vat_off();
    let acme = contact_id(&d, "Acme BV");
    make_finalized(&d, acme, "2026-05-01", Some(31), &["Ding @ 1000.00"]);
    // an invoice dated AFTER the as-of did not exist at as-of
    make_finalized(&d, acme, "2026-09-01", Some(30), &["Later @ 500.00"]);
    // a credit note dated after as-of must not net against the as-of position
    let cred = credit_invoice(
        &d,
        1,
        Some("2026-09-02"),
        Some("later"),
        "agent:test",
        false,
    )
    .unwrap();
    finalize_invoice(&d, cred["id"].as_i64().unwrap(), "agent:test", false).unwrap();

    let mut r = bukio::reports::aging(&d, "2026-08-08", "debtors").unwrap();
    let mut ac = r["debtors"]["contacts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["contact_id"] == json!(acme))
        .unwrap()
        .clone();
    assert_eq!(
        ac["total_cents"].as_i64(),
        Some(100000),
        "only the pre-as-of invoice"
    );
    assert_eq!(ac["buckets"]["d90"].as_i64(), Some(100000));

    // at a later as-of the 1000 credit nets the OLDEST invoice (FIFO)
    r = bukio::reports::aging(&d, "2026-09-30", "debtors").unwrap();
    ac = r["debtors"]["contacts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["contact_id"] == json!(acme))
        .unwrap()
        .clone();
    assert_eq!(ac["total_cents"].as_i64(), Some(50000));
    let items = ac["items"].as_array().unwrap();
    let first = items
        .iter()
        .find(|i| i["ref"] == json!("2026-0001"))
        .unwrap();
    assert_eq!(
        first["outstanding_cents"].as_i64(),
        Some(0),
        "credit offsets the oldest"
    );
    let later = items
        .iter()
        .find(|i| i["ref"] == json!("2026-0002"))
        .unwrap();
    assert_eq!(later["outstanding_cents"].as_i64(), Some(50000));
}

#[test]
fn aging_debtors_finalized_credits_reduce_drafts_do_not() {
    let d = vat_off();
    let acme = contact_id(&d, "Acme BV");
    make_finalized(&d, acme, "2026-05-01", Some(31), &["Ding @ 1000.00"]);
    let cred = credit_invoice(
        &d,
        1,
        Some("2026-07-01"),
        Some("retour"),
        "agent:test",
        false,
    )
    .unwrap();

    // a DRAFT credit note must not reduce anything yet
    let mut r = bukio::reports::aging(&d, "2026-08-08", "debtors").unwrap();
    let find = |r: &Value| -> Value {
        r["debtors"]["contacts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["contact_id"] == json!(acme))
            .unwrap()
            .clone()
    };
    let mut ac = find(&r);
    assert_eq!(ac["buckets"]["d90"].as_i64(), Some(100000));
    assert_eq!(ac["total_cents"].as_i64(), Some(100000));

    finalize_invoice(&d, cred["id"].as_i64().unwrap(), "agent:test", false).unwrap();
    r = bukio::reports::aging(&d, "2026-08-08", "debtors").unwrap();
    ac = find(&r);
    assert_eq!(ac["buckets"]["d90"].as_i64(), Some(0));
    assert_eq!(ac["total_cents"].as_i64(), Some(0));
    assert_eq!(r["debtors"]["totals"]["total_cents"].as_i64(), Some(0));
}

#[test]
fn aging_creditors_buckets_and_in_batch_separately() {
    let d = vat_off();
    let sup = contact_id(&d, "Lever BV");
    for (r, date, due, amt) in [
        ("F-1", "2026-05-01", Some("2026-06-01"), 5000i64),
        ("F-2", "2026-08-01", Some("2026-08-20"), 2500),
    ] {
        bukio::payments::add_payable(
            &d,
            &sup.to_string(),
            r,
            date,
            due,
            amt,
            "transfer",
            "agent:test",
            false,
        )
        .unwrap();
    }
    // move one payable into a batch (simulating batch create)
    d.execute(
        "UPDATE payables SET status = 'in_batch' WHERE invoice_ref = 'F-1'",
        [],
    )
    .unwrap();

    let r = bukio::reports::aging(&d, "2026-08-08", "creditors").unwrap();
    let cs = r["creditors"]["contacts"].as_array().unwrap();
    assert_eq!(cs.len(), 1, "{r}");
    let s = &cs[0];
    assert_eq!(s["in_batch_cents"].as_i64(), Some(5000));
    assert_eq!(s["buckets"]["current"].as_i64(), Some(2500));
    assert_eq!(s["total_cents"].as_i64(), Some(7500));
    assert_eq!(r["creditors"]["totals"]["total_cents"].as_i64(), Some(7500));
    let f1 = s["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["ref"] == json!("F-1"))
        .unwrap();
    assert_eq!(f1["status"], json!("in_batch"));
}

#[test]
fn aging_creditors_excludes_payables_after_as_of() {
    let d = vat_off();
    let sup = contact_id(&d, "Lever BV");
    bukio::payments::add_payable(
        &d,
        &sup.to_string(),
        "F-1",
        "2026-05-01",
        None,
        5000,
        "transfer",
        "agent:test",
        false,
    )
    .unwrap();
    bukio::payments::add_payable(
        &d,
        &sup.to_string(),
        "F-2",
        "2026-09-15",
        None,
        2500,
        "transfer",
        "agent:test",
        false,
    )
    .unwrap();

    let r = bukio::reports::aging(&d, "2026-08-08", "creditors").unwrap();
    let cs = r["creditors"]["contacts"].as_array().unwrap();
    assert_eq!(cs.len(), 1, "{r}");
    assert_eq!(cs[0]["total_cents"].as_i64(), Some(5000));
    assert_eq!(cs[0]["items"].as_array().unwrap().len(), 1);
    assert_eq!(cs[0]["items"][0]["ref"], json!("F-1"));
    assert_eq!(r["creditors"]["totals"]["total_cents"].as_i64(), Some(5000));

    let later = bukio::reports::aging(&d, "2026-09-30", "creditors").unwrap();
    assert_eq!(
        later["creditors"]["contacts"][0]["total_cents"].as_i64(),
        Some(7500)
    );
}

#[test]
fn aging_validation_rejects_bad_as_of_and_kind() {
    let d = vat_off();
    assert_eq!(
        code_of(bukio::reports::aging(&d, "garbage", "debtors")),
        "INVALID_DATE"
    );
    assert_eq!(
        code_of(bukio::reports::aging(&d, "2026-02-30", "debtors")),
        "INVALID_DATE"
    );
    assert_eq!(
        code_of(bukio::reports::aging(&d, "2026-08-08", "bogus")),
        "INVALID_KIND"
    );
}

#[test]
fn contact_statement_running_balance_and_supplier_side() {
    let d = vat_off();
    let acme = contact_id(&d, "Acme BV");
    let sup = contact_id(&d, "Lever BV");
    let inv = make_finalized(&d, acme, "2026-07-01", Some(30), &["Ding @ 100.00"]);
    let gross = inv["gross_cents"].as_i64().unwrap();
    mark_paid(
        &d,
        inv["id"].as_i64().unwrap(),
        "2026-07-20",
        4000,
        "bank",
        "agent:test",
        false,
    )
    .unwrap();

    let r = bukio::contacts::contact_statement(&d, acme, Some("2026-08-08")).unwrap();
    assert_eq!(r["contact"]["name"], json!("Acme BV"));
    let rows = r["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 2, "invoice + payment: {r}");
    assert_eq!(rows[0]["kind"], json!("invoice"));
    assert_eq!(rows[0]["debit_cents"].as_i64(), Some(gross));
    assert_eq!(rows[1]["kind"], json!("payment"));
    assert_eq!(rows[1]["credit_cents"].as_i64(), Some(4000));
    assert_eq!(r["balance_cents"].as_i64(), Some(gross - 4000));
    assert_eq!(rows[rows.len() - 1]["balance_cents"], r["balance_cents"]);
    assert_eq!(
        code_of(bukio::contacts::contact_statement(&d, 999999, None)),
        "CONTACT_NOT_FOUND"
    );
    assert_eq!(
        code_of(bukio::contacts::contact_statement(&d, acme, Some("abc"))),
        "INVALID_DATE"
    );

    // supplier: a payable makes the balance negative (we owe them)
    bukio::payments::add_payable(
        &d,
        &sup.to_string(),
        "F-9",
        "2026-07-05",
        None,
        12345,
        "transfer",
        "agent:test",
        false,
    )
    .unwrap();
    let s = bukio::contacts::contact_statement(&d, sup, Some("2026-08-08")).unwrap();
    assert_eq!(s["rows"][0]["kind"], json!("payable"));
    assert_eq!(s["balance_cents"].as_i64(), Some(-12345));
}

#[test]
fn contact_statement_credit_notes_reduce_the_balance() {
    let d = vat_off();
    let acme = contact_id(&d, "Acme BV");
    let inv = make_finalized(&d, acme, "2026-07-01", None, &["Ding @ 100.00"]);
    let st = bukio::contacts::contact_statement(&d, acme, Some("2026-08-08")).unwrap();
    assert_eq!(st["balance_cents"].as_i64(), Some(10000));

    let credit = credit_invoice(
        &d,
        inv["id"].as_i64().unwrap(),
        Some("2026-07-15"),
        None,
        "agent:test",
        false,
    )
    .unwrap();
    finalize_invoice(&d, credit["id"].as_i64().unwrap(), "agent:test", false).unwrap();

    let st = bukio::contacts::contact_statement(&d, acme, Some("2026-08-08")).unwrap();
    let credit_row = st["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["kind"] == json!("credit"))
        .cloned()
        .unwrap_or_else(|| panic!("credit note must appear on the opgave: {st}"));
    assert_eq!(credit_row["credit_cents"].as_i64(), Some(10000));
    assert_eq!(st["balance_cents"].as_i64(), Some(0));
}

#[test]
fn contact_statement_excludes_payments_after_as_of() {
    let d = vat_off();
    let acme = contact_id(&d, "Acme BV");
    let inv = make_finalized(&d, acme, "2026-07-01", None, &["Ding @ 100.00"]);
    let id = inv["id"].as_i64().unwrap();
    mark_paid(&d, id, "2026-07-20", 2000, "transfer", "agent:test", false).unwrap();
    mark_paid(&d, id, "2026-08-20", 3000, "transfer", "agent:test", false).unwrap();

    let r = bukio::contacts::contact_statement(&d, acme, Some("2026-08-08")).unwrap();
    let payments: Vec<&Value> = r["rows"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|x| x["kind"] == json!("payment"))
        .collect();
    assert_eq!(payments.len(), 1, "only the pre-as-of payment shows: {r}");
    assert_eq!(payments[0]["credit_cents"].as_i64(), Some(2000));
    assert_eq!(r["balance_cents"].as_i64(), Some(10000 - 2000));

    let full = bukio::contacts::contact_statement(&d, acme, Some("2026-09-30")).unwrap();
    assert_eq!(
        full["rows"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|x| x["kind"] == json!("payment"))
            .count(),
        2
    );
    assert_eq!(full["balance_cents"].as_i64(), Some(10000 - 2000 - 3000));
}

#[test]
fn sales_by_contact_net_vat_gross_and_credits_excluded() {
    let d = vat_off();
    let acme = contact_id(&d, "Acme BV");
    let beta = contact_id(&d, "Beta BV");
    make_finalized(&d, acme, "2026-01-10", None, &["Ding @ 100.00"]);
    make_finalized(&d, acme, "2026-03-15", Some(30), &["Ding @ 50.00"]);
    // a draft must not count
    let draft = create_invoice(
        &d,
        acme,
        "2026-02-01",
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        &lines(&["Ding @ 999.00"]),
        "agent:test",
        false,
    )
    .unwrap();
    assert_eq!(draft["status"], json!("draft"));
    // outside the year
    make_finalized(&d, beta, "2025-12-31", Some(30), &["Ding @ 100.00"]);

    let r = bukio::reports::sales(&d, "2026", "contact").unwrap();
    let groups = r["groups"].as_array().unwrap();
    assert_eq!(groups.len(), 1, "{r}");
    assert_eq!(groups[0]["contact_id"], json!(acme));
    assert_eq!(groups[0]["invoice_count"].as_i64(), Some(2));
    assert_eq!(groups[0]["net_cents"].as_i64(), Some(15000));
    assert_eq!(groups[0]["gross_cents"].as_i64(), Some(15000));
    assert_eq!(r["totals"]["net_cents"].as_i64(), Some(15000));

    assert_eq!(
        code_of(bukio::reports::sales(&d, "abc", "contact")),
        "INVALID_YEAR"
    );
    assert_eq!(
        code_of(bukio::reports::sales(&d, "2026", "bogus")),
        "INVALID_KIND"
    );
}

#[test]
fn sales_by_item_groups_catalog_items_and_ad_hoc_lines() {
    let d = vat_off();
    let acme = contact_id(&d, "Acme BV");
    let beta = contact_id(&d, "Beta BV");
    bukio::items::create_item(
        &d,
        "Coaching uur",
        None,
        "h",
        8000,
        None,
        None,
        "agent:test",
        false,
    )
    .unwrap();
    make_finalized(
        &d,
        acme,
        "2026-02-01",
        None,
        &["1x Coaching uur @ 80.00", "Materiaal @ 20.00"],
    );
    make_finalized(&d, beta, "2026-02-02", None, &["1x Coaching uur @ 80.00"]);

    let r = bukio::reports::sales(&d, "2026", "item").unwrap();
    let groups = r["groups"].as_array().unwrap();
    let find = |n: &str| groups.iter().find(|g| g["name"] == json!(n)).cloned();
    let coaching = find("Coaching uur").unwrap_or_else(|| panic!("no Coaching uur group: {r}"));
    assert_eq!(coaching["line_count"].as_i64(), Some(2));
    assert_eq!(coaching["net_cents"].as_i64(), Some(16000));
    let materiaal = find("Materiaal").unwrap_or_else(|| panic!("no Materiaal group: {r}"));
    assert_eq!(materiaal["net_cents"].as_i64(), Some(2000));
    assert_eq!(r["totals"]["line_count"].as_i64(), Some(3));
    assert_eq!(r["totals"]["net_cents"].as_i64(), Some(18000));
}

// ---- CLI + MCP e2e (needs a real file database the binary can open) --------

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!(
        "bukio-rep14-{}-{}-{}",
        tag,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn run_cli(args: &[&str]) -> (Value, bool, String) {
    let exe = env!("CARGO_BIN_EXE_bukio");
    let out = std::process::Command::new(exe)
        .env("BUKIO_ACTOR", "agent:test")
        .args(args)
        .output()
        .unwrap();
    (
        serde_json::from_slice(&out.stdout).unwrap_or(Value::Null),
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).to_string(),
    )
}

#[test]
fn cli_aging_sales_statement_e2e_with_csv_export() {
    let dir = temp_dir("cli");
    let file = dir.join("test.db");
    let f = file.to_str().unwrap().to_string();

    let (_, ok, out) = run_cli(&[
        "--json",
        "init",
        "--name",
        "Test Coaching",
        "--registration-id",
        "12345678",
        "--legal-form",
        "eenmanszaak",
        "--vat",
        "off",
        "--db",
        &f,
    ]);
    assert!(ok, "init failed: {out}");
    let (_, ok, out) = run_cli(&[
        "--json",
        "company",
        "update",
        "--address",
        "Teststraat 1",
        "--postal-code",
        "1000 AA",
        "--city",
        "Amsterdam",
        "--db",
        &f,
    ]);
    assert!(ok, "company update failed: {out}");

    // seed through the engine on the same file (like the JS suite does)
    let d = bukio::db::open_db(&f).unwrap();
    let acme = contact_id(&d, "Acme BV");
    make_finalized(&d, acme, "2026-07-01", Some(30), &["Ding @ 100.00"]);
    drop(d);

    let (v, ok, out) = run_cli(&[
        "--json",
        "report",
        "aging",
        "--as-of",
        "2026-08-08",
        "--kind",
        "debtors",
        "--db",
        &f,
    ]);
    assert!(ok, "{out}");
    assert_eq!(
        v["data"]["debtors"]["totals"]["total_cents"].as_i64(),
        Some(10000),
        "{v}"
    );

    let (v, ok, out) = run_cli(&["--json", "report", "sales", "--year", "2026", "--db", &f]);
    assert!(ok, "{out}");
    assert_eq!(
        v["data"]["totals"]["invoice_count"].as_i64(),
        Some(1),
        "{v}"
    );

    let (v, ok, out) = run_cli(&[
        "--json",
        "contact",
        "statement",
        "--id",
        &acme.to_string(),
        "--db",
        &f,
    ]);
    assert!(ok, "{out}");
    assert_eq!(v["data"]["rows"].as_array().unwrap().len(), 1, "{v}");
    assert_eq!(v["data"]["balance_cents"].as_i64(), Some(10000));

    // csv export: --out writes the file (the command still prints a human line)
    let csv = dir.join("aging.csv");
    let (_, ok, out) = run_cli(&[
        "--json",
        "report",
        "aging",
        "--format",
        "csv",
        "--out",
        csv.to_str().unwrap(),
        "--db",
        &f,
    ]);
    assert!(ok, "{out}");
    let content = std::fs::read_to_string(&csv).unwrap();
    assert!(content.contains("debtors"), "{content}");
    assert!(
        !content.contains("=HYPERLINK"),
        "formula-injection guard tripped: {content}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn mcp_report_aging_and_report_sales_share_the_shapes() {
    use std::io::{BufRead, BufReader, Write};
    use std::process::{Command, Stdio};

    let dir = temp_dir("mcp");
    let file = dir.join("test.db");
    let f = file.to_str().unwrap().to_string();
    let (_, ok, out) = run_cli(&[
        "--json",
        "init",
        "--name",
        "Test Coaching",
        "--registration-id",
        "12345678",
        "--legal-form",
        "eenmanszaak",
        "--vat",
        "off",
        "--db",
        &f,
    ]);
    assert!(ok, "init failed: {out}");
    let (_, ok, out) = run_cli(&[
        "--json",
        "company",
        "update",
        "--address",
        "Teststraat 1",
        "--postal-code",
        "1000 AA",
        "--city",
        "Amsterdam",
        "--db",
        &f,
    ]);
    assert!(ok, "company update failed: {out}");
    let d = bukio::db::open_db(&f).unwrap();
    let acme = contact_id(&d, "Acme BV");
    make_finalized(&d, acme, "2026-07-01", Some(30), &["Ding @ 100.00"]);
    drop(d);

    let exe = env!("CARGO_BIN_EXE_bukio");
    let mut child = Command::new(exe)
        .args(["mcp", "--db", &f])
        .env("BUKIO_ACTOR", "agent:test")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap());

    let mut call = |stdin: &mut dyn Write,
                    reader: &mut BufReader<_>,
                    id: u64,
                    method: &str,
                    params: Value|
     -> Value {
        let req = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        writeln!(stdin, "{req}").unwrap();
        stdin.flush().unwrap();
        let mut line = String::new();
        loop {
            line.clear();
            let n = reader.read_line(&mut line).unwrap();
            assert!(n > 0, "MCP closed before answering {method}");
            let Ok(msg) = serde_json::from_str::<Value>(line.trim()) else {
                continue;
            };
            if msg["id"] == json!(id) {
                return msg;
            }
        }
    };

    call(
        &mut stdin,
        &mut reader,
        1,
        "initialize",
        json!({ "protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": { "name": "t", "version": "1" } }),
    );

    let r = call(
        &mut stdin,
        &mut reader,
        2,
        "tools/call",
        json!({ "name": "report_aging", "arguments": { "as_of": "2026-08-08", "kind": "debtors" } }),
    );
    let txt = r["result"]["content"][0]["text"].as_str().unwrap_or("");
    let data: Value =
        serde_json::from_str(txt).unwrap_or_else(|_| panic!("bad tools/call payload: {r}"));
    assert_eq!(
        data["debtors"]["totals"]["total_cents"].as_i64(),
        Some(10000),
        "{r}"
    );

    let r = call(
        &mut stdin,
        &mut reader,
        3,
        "tools/call",
        json!({ "name": "report_sales", "arguments": { "year": "2026" } }),
    );
    let txt = r["result"]["content"][0]["text"].as_str().unwrap_or("");
    let data: Value =
        serde_json::from_str(txt).unwrap_or_else(|_| panic!("bad tools/call payload: {r}"));
    assert_eq!(data["totals"]["invoice_count"].as_i64(), Some(1), "{r}");

    let r = call(
        &mut stdin,
        &mut reader,
        4,
        "tools/call",
        json!({ "name": "report_sales", "arguments": { "year": "abc" } }),
    );
    assert_eq!(r["result"]["isError"], json!(true), "{r}");

    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&dir);
}

// ==== jaarrekening HTML/PDF renderer (the 3 tests year-end could not port) ==

#[test]
fn jaarrekening_html_renders_account_detail_without_nan() {
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
    let html = bukio::pdf::jaarrekening_html(&r);
    assert!(
        !html.contains("NaN"),
        "the HTML template must not contain NaN"
    );
    assert!(html.contains("5370.00"), "account detail amount rendered");
    assert!(html.contains("Jaarrekening 2026"), "title");
    assert!(html.contains("Totaal activa"), "balans total");
}

#[test]
fn jaarrekening_pdf_renders_bytes() {
    let d = setup();
    entry(
        &d,
        "2026-03-01",
        "Omzet",
        specs(&[("1100", 12100), ("8000", -10000), ("2500", -2100)]),
    );
    let r = bukio::reports::jaarrekening(&d, "2026", Some("klein")).unwrap();
    let dir = temp_dir("pdf");
    let out = dir.join("test-jaarrekening.pdf");
    // the port writes the PDF natively — no browser, so this cannot be skipped
    let res = bukio::pdf::jaarrekening_to_pdf(&r, Some(out.to_str().unwrap()))
        .expect("native PDF render");
    // The port writes the PDF natively (no Chromium), so it is a valid ~3KB
    // document rather than the JS's chromium-inflated 48KB. Check the structure
    // instead of the size: header, EOF, and every xref offset pointing at its
    // own "N 0 obj" — the thing that breaks when the writer's offsets drift.
    assert!(res["bytes"].as_u64().unwrap_or(0) > 1000, "{res}");
    let bytes = std::fs::read(&out).unwrap();
    assert!(bytes.starts_with(b"%PDF-1.4"), "must be a PDF");
    assert!(bytes.ends_with(b"%%EOF\n"), "must be terminated");
    let text = String::from_utf8_lossy(&bytes).to_string();
    assert!(text.contains("Jaarrekening 2026"), "title in the document");
    // WinAnsi single-byte encoding: "Materiële" must carry 0xEB, not the two
    // UTF-8 bytes — otherwise the PDF shows "MateriÃ«le"
    assert!(
        !text.contains('\u{c3}') && !text.contains('\u{c2}'),
        "content stream must be single-byte (WinAnsi) encoded — a UTF-8 push showed 'MateriÃ«le'"
    );
    assert!(
        text.contains("Totaal activa"),
        "balans total in the document"
    );
    // xref offsets are BYTE offsets — parse them from the raw bytes (the lossy
    // string conversion above shifts positions for non-ASCII content)
    let marker = b"startxref";
    let at = bytes
        .windows(marker.len())
        .rposition(|w| w == marker)
        .unwrap();
    let tail = String::from_utf8_lossy(&bytes[at + marker.len()..]).to_string();
    let start: usize = tail.trim().lines().next().unwrap().trim().parse().unwrap();
    assert!(
        bytes[start..].starts_with(b"xref"),
        "startxref must point at the table"
    );
    let table = String::from_utf8_lossy(&bytes[start..]).to_string();
    let mut it = table.lines();
    assert_eq!(it.next().map(str::trim), Some("xref"));
    let header = it.next().unwrap_or("");
    let count: usize = header.split_whitespace().nth(1).unwrap().parse().unwrap();
    for n in 0..count {
        let chunk = it.next().expect("xref table shorter than its count");
        let off: usize = chunk
            .split_whitespace()
            .next()
            .unwrap_or("0")
            .parse()
            .unwrap_or(0);
        if off == 0 {
            continue; // object 0 is the free head
        }
        let want = format!("{n} 0 obj");
        assert!(
            bytes[off..].starts_with(want.as_bytes()),
            "xref entry {n} points at {:?}",
            String::from_utf8_lossy(&bytes[off..(off + 12).min(bytes.len())])
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn jaarrekening_html_escapes_quotes_in_the_company_name() {
    let d = setup();
    entry(
        &d,
        "2026-03-01",
        "Omzet",
        specs(&[("1100", 12100), ("8000", -10000), ("2500", -2100)]),
    );
    d.execute(
        "UPDATE company SET name = ?1 WHERE id = 1",
        ["Test \"Bedrijf\" BV"],
    )
    .unwrap();
    let r = bukio::reports::jaarrekening(&d, "2026", Some("klein")).unwrap();
    let html = bukio::pdf::jaarrekening_html(&r);
    assert!(
        html.contains("Test &quot;Bedrijf&quot; BV"),
        "company name must be escaped"
    );
    assert!(
        !html.contains("Test \"Bedrijf\" BV"),
        "raw quotes must not reach the HTML"
    );
}

// ==== invoice HTML/PDF renderer (native, no browser) ========================

#[test]
fn invoice_html_and_pdf_are_native_documents() {
    let d = setup();
    let cid = add_contact(&d, None);
    let inv = create_invoice(
        &d,
        cid,
        "2026-07-01",
        Some(30),
        None,
        Some("PO-77"),
        None,
        None,
        None,
        None,
        &lines(&["3x Consultancy @ 125.00 @21"]),
        "agent:test",
        false,
    )
    .unwrap();
    let id = inv["id"].as_i64().unwrap();
    finalize_invoice(&d, id, "agent:test", false).unwrap();
    let full = get_invoice(&d, id).unwrap().unwrap();

    let html = bukio::pdf::invoice_html(&d, &full);
    assert!(html.contains("2026-0001"), "invoice number on the document");
    assert!(html.contains("Consultancy"), "line description");
    assert!(html.contains("ACME B.V."), "customer");
    assert!(!html.contains("NaN"), "no NaN in the HTML");
    assert!(html.contains("375.00"), "line amount rendered");

    let dir = temp_dir("invpdf");
    let out = dir.join("inv.pdf");
    let res = bukio::pdf::invoice_to_pdf(&d, &full, Some(out.to_str().unwrap())).unwrap();
    let bytes = std::fs::read(&out).unwrap();
    assert_eq!(res["bytes"].as_u64().unwrap() as usize, bytes.len());
    assert!(bytes.len() > 800, "{res}");
    assert!(bytes.starts_with(b"%PDF-1.4"), "must be a PDF");
    assert!(bytes.ends_with(b"%%EOF\n"), "must be terminated");
    let text = String::from_utf8_lossy(&bytes).to_string();
    assert!(text.contains("2026-0001"), "number on the PDF");
    assert!(text.contains("Consultancy"), "line on the PDF");
    assert!(
        !text.contains('\u{c3}') && !text.contains('\u{c2}'),
        "single-byte (WinAnsi) content stream"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ==== ported from test/fiscal-year.test.js, test/company.test.js, ===========
// ==== and test/review-round3.test.js (all CLI-level) =======================

/// a fresh file DB initialised through the CLI, like the JS suites do
fn cli_db(tag: &str, init_args: &[&str]) -> (std::path::PathBuf, String) {
    let dir = temp_dir(tag);
    let file = dir.join("test.db");
    let f = file.to_str().unwrap().to_string();
    let mut args = vec!["--json", "init", "--name", "Test Coaching"];
    args.extend_from_slice(init_args);
    args.extend_from_slice(&["--db", &f]);
    let (_, ok, out) = run_cli(&args);
    assert!(ok, "init failed: {out}");
    (dir, f)
}

// ---- fiscal-year (7) -------------------------------------------------------

/// company with fiscal_year_end 03-31 and one entry in each half:
/// 2026-01-15 (previous FY) and 2026-11-15 (this FY)
fn fiscal_db(tag: &str) -> (std::path::PathBuf, String) {
    let (dir, f) = cli_db(tag, &["--registration-id", "12345678", "--vat", "off"]);
    {
        let d = bukio::db::open_db(&f).unwrap();
        d.execute("UPDATE company SET fiscal_year_end = '03-31'", [])
            .unwrap();
        for (date, desc, amt) in [
            ("2026-01-15", "jan 2026 (prev FY)", 10000i64),
            ("2026-11-15", "nov 2026 (this FY)", 20000),
        ] {
            let e = create_entry(
                &d,
                bukio::entries::CreateEntry {
                    date: date.into(),
                    description: desc.into(),
                    postings: specs(&[("1100", amt), ("8000", -amt)]),
                    source: "manual",
                    source_ref: None,
                    actor: "agent:test",
                },
            )
            .unwrap();
            post_entry(&d, e.id, "agent:test").unwrap();
        }
    }
    (dir, f)
}

#[test]
fn fiscal_year_window_for_march_year_end_spans_previous_april_to_march() {
    let (dir, f) = fiscal_db("fy");
    let d = bukio::db::open_db(&f).unwrap();
    let (from, to) = bukio::year_end::fiscal_year_window(&d, "2026").unwrap();
    assert_eq!(from, "2025-04-01");
    assert_eq!(to, "2026-03-31");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn report_pnl_year_uses_the_fiscal_window() {
    let (dir, f) = fiscal_db("fypnl");
    let (v, ok, out) = run_cli(&["--json", "report", "pnl", "--year", "2026", "--db", &f]);
    assert!(ok, "{out}");
    assert_eq!(v["data"]["from"], json!("2025-04-01"), "{v}");
    assert_eq!(v["data"]["to"], json!("2026-03-31"), "{v}");
    assert_eq!(
        v["data"]["result_cents"].as_i64(),
        Some(10000),
        "only the Jan entry is in window"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn report_journal_year_uses_the_fiscal_window() {
    let (dir, f) = fiscal_db("fyjrnl");
    let (v, ok, out) = run_cli(&["--json", "report", "journal", "--year", "2026", "--db", &f]);
    assert!(ok, "{out}");
    assert_eq!(v["data"]["from"], json!("2025-04-01"), "{v}");
    assert_eq!(v["data"]["to"], json!("2026-03-31"), "{v}");
    let rows = v["data"]["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 2, "one posting pair of the jan entry: {v}");
    assert!(rows.iter().all(|r| {
        let d = r["date"].as_str().unwrap_or("");
        d >= "2025-04-01" && d <= "2026-03-31"
    }));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn report_trial_balance_year_uses_the_fiscal_window() {
    let (dir, f) = fiscal_db("fytb");
    let (v, ok, out) = run_cli(&[
        "--json",
        "report",
        "trial-balance",
        "--year",
        "2026",
        "--db",
        &f,
    ]);
    assert!(ok, "{out}");
    let bank = v["data"]["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["code"] == json!("1100"))
        .cloned()
        .unwrap_or_else(|| panic!("no 1100: {v}"));
    assert_eq!(bank["net_cents"].as_i64(), Some(10000));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn pnl_and_journal_with_explicit_dates_ignore_the_fiscal_window() {
    let (dir, f) = fiscal_db("fyexplicit");
    let d = bukio::db::open_db(&f).unwrap();
    let p = bukio::reports::pnl(&d, "2026-01-01", "2026-12-31").unwrap();
    assert_eq!(
        p["result_cents"].as_i64(),
        Some(30000),
        "calendar window still works"
    );
    let j = bukio::reports::journal(&d, "2026-01-01", "2026-12-31", None).unwrap();
    assert_eq!(j.len(), 4, "both entries' postings");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn sales_uses_the_fiscal_window() {
    let (dir, f) = cli_db(
        "fysales",
        &[
            "--registration-id",
            "12345678",
            "--vat",
            "off",
            // finalizing needs a complete supplier, like the JS suite's seed
            "--address",
            "Teststraat 1",
            "--postal-code",
            "1000 AA",
            "--city",
            "Amsterdam",
        ],
    );
    {
        let d = bukio::db::open_db(&f).unwrap();
        d.execute("UPDATE company SET fiscal_year_end = '03-31'", [])
            .unwrap();
        let c = bukio::contacts::create_contact(
            &d,
            "Acme BV",
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
        .unwrap()["id"]
            .as_i64()
            .unwrap();
        for (date, line) in [
            ("2026-01-20", "1x Werk @ 50.00"),
            ("2026-11-20", "1x Werk2 @ 70.00"),
        ] {
            let inv = create_invoice(
                &d,
                c,
                date,
                None,
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
        let s = bukio::reports::sales(&d, "2026", "contact").unwrap();
        assert_eq!(
            s["totals"]["net_cents"].as_i64(),
            Some(5000),
            "only the Jan invoice is in window"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn mcp_pnl_reports_the_fiscal_window() {
    let (dir, f) = fiscal_db("fymcp");
    let d = bukio::db::open_db(&f).unwrap();
    let (from, to) = bukio::year_end::fiscal_year_window(&d, "2026").unwrap();
    let p = bukio::reports::pnl(&d, &from, &to).unwrap();
    assert_eq!(p["result_cents"].as_i64(), Some(10000));
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- company (6) -----------------------------------------------------------

#[test]
fn company_update_sets_fields_and_audits() {
    let (dir, f) = cli_db(
        "co1",
        &[
            "--registration-id",
            "12345678",
            "--legal-form",
            "eenmanszaak",
            "--vat",
            "off",
        ],
    );
    let (v, ok, out) = run_cli(&[
        "--json",
        "company",
        "update",
        "--address",
        "Teststraat 1",
        "--postal-code",
        "1000 AA",
        "--city",
        "Amsterdam",
        "--iban",
        "NL91ABNA0417164300",
        "--db",
        &f,
    ]);
    assert!(ok, "{out}");
    assert_eq!(
        v["data"]["company"]["address"],
        json!("Teststraat 1"),
        "{v}"
    );
    let d = bukio::db::open_db(&f).unwrap();
    let (postal, city, iban): (Option<String>, Option<String>, Option<String>) = d
        .query_row(
            "SELECT postal_code, city, iban FROM company WHERE id = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(postal.as_deref(), Some("1000 AA"));
    assert_eq!(city.as_deref(), Some("Amsterdam"));
    assert_eq!(iban.as_deref(), Some("NL91ABNA0417164300"));
    let (actor, command): (String, String) = d
        .query_row(
            "SELECT actor, command FROM audit_log WHERE action = 'company.update' ORDER BY id DESC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(actor, "agent:test");
    assert_eq!(command, "company update");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn company_update_dry_run_writes_nothing() {
    let (dir, f) = cli_db("co2", &["--registration-id", "12345678", "--vat", "off"]);
    let (_, ok, out) = run_cli(&[
        "--json",
        "company",
        "update",
        "--address",
        "Teststraat 1",
        "--city",
        "Amsterdam",
        "--dry-run",
        "--db",
        &f,
    ]);
    assert!(ok, "{out}");
    let d = bukio::db::open_db(&f).unwrap();
    let (addr, city): (Option<String>, Option<String>) = d
        .query_row("SELECT address, city FROM company WHERE id = 1", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!(addr, None);
    assert_eq!(city, None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn company_update_without_options_is_rejected() {
    let (dir, f) = cli_db("co3", &["--registration-id", "12345678", "--vat", "off"]);
    let (v, ok, out) = run_cli(&["--json", "company", "update", "--db", &f]);
    assert!(!ok, "{out}");
    assert_eq!(v["error"]["code"], json!("NOTHING_TO_UPDATE"), "{v}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn company_update_rejects_an_invalid_iban() {
    let (dir, f) = cli_db("co4", &["--registration-id", "12345678", "--vat", "off"]);
    let (v, ok, out) = run_cli(&["--json", "company", "update", "--iban", "nope", "--db", &f]);
    assert!(!ok, "{out}");
    assert_eq!(v["error"]["code"], json!("INVALID_IBAN"), "{v}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn company_show_returns_the_record() {
    let (dir, f) = cli_db("co5", &["--registration-id", "12345678", "--vat", "off"]);
    run_cli(&[
        "--json",
        "company",
        "update",
        "--city",
        "Amsterdam",
        "--db",
        &f,
    ]);
    let (v, ok, out) = run_cli(&["--json", "company", "show", "--db", &f]);
    assert!(ok, "{out}");
    let c = &v["data"]["company"];
    assert_eq!(c["name"], json!("Test Coaching"));
    assert_eq!(c["registration_id"], json!("12345678"));
    assert_eq!(c["city"], json!("Amsterdam"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn company_show_without_a_company_row_is_refused() {
    let (dir, f) = cli_db("co6", &["--registration-id", "12345678", "--vat", "off"]);
    {
        let d = bukio::db::open_db(&f).unwrap();
        d.execute("DELETE FROM company", []).unwrap();
    }
    let (v, ok, out) = run_cli(&["--json", "company", "show", "--db", &f]);
    assert!(!ok, "{out}");
    assert_eq!(v["ok"], json!(false));
    assert_eq!(v["error"]["code"], json!("NO_COMPANY"), "{v}");
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("run bukio init"),
        "{v}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- review-round3 (4) -----------------------------------------------------

#[test]
fn recurring_pause_and_resume_dry_run_render_a_plan() {
    let (dir, f) = cli_db("r3a", &["--registration-id", "12345678", "--vat", "off"]);
    let (_, ok, out) = run_cli(&[
        "--json",
        "recurring",
        "add",
        "--name",
        "Huur",
        "--postings",
        "4300:1000.00,1100:-1000.00",
        "--frequency",
        "monthly",
        "--start",
        "2026-01-01",
        "--db",
        &f,
    ]);
    assert!(ok, "{out}");

    // the dry-run plan must render (used to crash rendering a plan with no postings)
    let (v, ok, out) = run_cli(&[
        "--json",
        "recurring",
        "pause",
        "--id",
        "1",
        "--dry-run",
        "--db",
        &f,
    ]);
    assert!(ok, "{out}");
    assert_eq!(v["data"]["template"]["dryRun"], json!(true), "{v}");
    assert_eq!(
        v["data"]["template"]["action"],
        json!("recurring.paused"),
        "{v}"
    );
    assert_eq!(v["data"]["template"]["id"], json!("1"), "{v}");
    let (v, ok, out) = run_cli(&[
        "--json",
        "recurring",
        "resume",
        "--id",
        "1",
        "--dry-run",
        "--db",
        &f,
    ]);
    assert!(ok, "{out}");
    assert_eq!(v["data"]["template"]["dryRun"], json!(true), "{v}");
    assert_eq!(
        v["data"]["template"]["action"],
        json!("recurring.active"),
        "{v}"
    );
    assert_eq!(v["data"]["template"]["id"], json!("1"), "{v}");

    let (v, ok, out) = run_cli(&["--json", "recurring", "pause", "--id", "1", "--db", &f]);
    assert!(ok, "{out}");
    assert_eq!(v["data"]["template"]["status"], json!("paused"), "{v}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn audit_format_json_prints_json_without_the_global_flag() {
    let (dir, f) = cli_db("r3b", &["--registration-id", "12345678", "--vat", "off"]);
    // NOTE: no --json at all — --format json must be enough
    let (v, ok, out) = run_cli(&["audit", "--format", "json", "--db", &f]);
    assert!(ok, "{out}");
    assert_eq!(v["ok"], json!(true), "{v}");
    let entries = v["data"]["entries"]
        .as_array()
        .unwrap_or_else(|| panic!("{v}"));
    assert!(!entries.is_empty(), "{v}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bank_match_post_dry_run_rejects_matched_tx_and_missing_account() {
    let (dir, f) = cli_db("r3c", &["--registration-id", "12345678", "--vat", "off"]);
    {
        let d = bukio::db::open_db(&f).unwrap();
        d.execute(
            "INSERT INTO bank_accounts (iban, account_code, name) VALUES (?1, '1100', 'Betaalrekening')",
            ["NL91ABNA0417164300"],
        )
        .unwrap();
        bukio::bank::import_transactions(
            &d,
            "NL91ABNA0417164300",
            &[bukio::bank::BankTx {
                date: "2026-07-01".into(),
                amount_cents: -50000,
                counterparty: Some("Leverancier".into()),
                description: Some("F1".into()),
                iban_counter: None,
                bank_ref: Some("REF-A".into()),
                iban: Some("NL91ABNA0417164300".into()),
            }],
            Some("Betaalrekening"),
            "1100",
            "agent:test",
        )
        .unwrap();
        let e = create_entry(
            &d,
            bukio::entries::CreateEntry {
                date: "2026-06-25".into(),
                description: "Betaling A".into(),
                postings: specs(&[("1100", -50000), ("4300", 50000)]),
                source: "manual",
                source_ref: None,
                actor: "agent:test",
            },
        )
        .unwrap();
        post_entry(&d, e.id, "agent:test").unwrap();
        d.execute(
            "UPDATE bank_transactions SET state = 'matched' WHERE id = 1",
            [],
        )
        .unwrap();
    }

    let (v, ok, out) = run_cli(&[
        "--json",
        "bank",
        "match",
        "post",
        "--tx",
        "1",
        "--account",
        "4300",
        "--dry-run",
        "--db",
        &f,
    ]);
    assert!(!ok, "{out}");
    assert_eq!(v["error"]["code"], json!("ALREADY_MATCHED"), "{v}");

    {
        let d = bukio::db::open_db(&f).unwrap();
        d.execute(
            "UPDATE bank_transactions SET state = 'unmatched' WHERE id = 1",
            [],
        )
        .unwrap();
    }
    let (v, ok, out) = run_cli(&[
        "--json",
        "bank",
        "match",
        "post",
        "--tx",
        "1",
        "--account",
        "9999",
        "--dry-run",
        "--db",
        &f,
    ]);
    assert!(!ok, "{out}");
    assert_eq!(v["error"]["code"], json!("ACCOUNT_NOT_FOUND"), "{v}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn vat_book_dry_run_rejects_unbalanced_postings() {
    let (dir, f) = cli_db(
        "r3d",
        &[
            "--registration-id",
            "12345678",
            "--legal-form",
            "bv",
            "--vat",
            "on",
        ],
    );
    let (v, ok, out) = run_cli(&[
        "--json",
        "vat",
        "book",
        "--date",
        "2026-01-10",
        "--desc",
        "x",
        "--postings",
        "8000:-100.00@21",
        "--dry-run",
        "--db",
        &f,
    ]);
    assert!(!ok, "{out}");
    assert_eq!(v["error"]["code"], json!("UNBALANCED"), "{v}");
    let _ = std::fs::remove_dir_all(&dir);
}

// ==== ported from test/direct-debit.test.js =================================

/// JSON-RPC session over `bukio mcp` — reused by the MCP-shaped tests ahead.
struct Mcp {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    reader: std::io::BufReader<std::process::ChildStdout>,
    next: u64,
}

impl Mcp {
    fn start(db_path: &str) -> Mcp {
        use std::process::{Command, Stdio};
        let exe = env!("CARGO_BIN_EXE_bukio");
        let mut child = Command::new(exe)
            .args(["mcp", "--db", db_path])
            .env("BUKIO_ACTOR", "agent:test")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let reader = std::io::BufReader::new(child.stdout.take().unwrap());
        let mut m = Mcp {
            child,
            stdin,
            reader,
            next: 1,
        };
        m.call(
            "initialize",
            json!({ "protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": { "name": "t", "version": "1" } }),
        );
        m
    }

    fn call(&mut self, method: &str, params: Value) -> Value {
        use std::io::{BufRead, Write};
        let id = self.next;
        self.next += 1;
        let req = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        writeln!(self.stdin, "{req}").unwrap();
        self.stdin.flush().unwrap();
        let mut line = String::new();
        loop {
            line.clear();
            let n = self.reader.read_line(&mut line).unwrap();
            assert!(n > 0, "MCP closed before answering {method}");
            if let Ok(msg) = serde_json::from_str::<Value>(line.trim()) {
                if msg["id"] == json!(id) {
                    return msg;
                }
            }
        }
    }

    /// tools/call + parse the JSON payload out of the content block
    fn tool(&mut self, name: &str, args: Value) -> (Value, bool) {
        let r = self.call("tools/call", json!({ "name": name, "arguments": args }));
        let is_err = r["result"]["isError"] == json!(true);
        let txt = r["result"]["content"][0]["text"].as_str().unwrap_or("");
        let data =
            serde_json::from_str(txt).unwrap_or_else(|_| panic!("bad payload from {name}: {r}"));
        (data, is_err)
    }

    fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// the JS suite's direct-debit fixture: company with an IBAN + complete address
fn dd_db(tag: &str) -> (std::path::PathBuf, String) {
    let (dir, f) = cli_db(
        tag,
        &[
            "--registration-id",
            "12345678",
            "--legal-form",
            "eenmanszaak",
            "--vat",
            "off",
        ],
    );
    let (_, ok, out) = run_cli(&[
        "--json",
        "company",
        "update",
        "--address",
        "Teststraat 1",
        "--postal-code",
        "1000 AA",
        "--city",
        "Amsterdam",
        "--iban",
        "NL91ABNA0417164300",
        "--db",
        &f,
    ]);
    assert!(ok, "company update failed: {out}");
    (dir, f)
}

fn dd_contact(db: &Connection, name: &str) -> i64 {
    bukio::contacts::create_contact(
        db,
        name,
        Some("Klantstraat 1"),
        None,
        Some("Amsterdam"),
        None,
        None,
        None,
        None,
        // direct debit needs the debtor's IBAN
        Some("NL91ABNA0417164300"),
        "agent:test",
        false,
    )
    .unwrap()["id"]
        .as_i64()
        .unwrap()
}

fn dd_payable(db: &Connection, contact: i64, inv_ref: &str, method: &str) -> i64 {
    bukio::payments::add_payable(
        db,
        &contact.to_string(),
        inv_ref,
        "2026-08-01",
        Some("2026-08-31"),
        12100,
        method,
        "agent:test",
        false,
    )
    .unwrap()["id"]
        .as_i64()
        .unwrap()
}

#[test]
fn mandates_add_list_remove_with_guards_and_audit() {
    let (dir, f) = dd_db("dd1");
    let d = bukio::db::open_db(&f).unwrap();
    let c = dd_contact(&d, "Debiteur BV");

    let m = bukio::payments::add_mandate(
        &d,
        c,
        "NL01ZZZ123456789012",
        Some("2026-07-01"),
        "b2b",
        "agent:test",
        false,
    )
    .unwrap();
    assert_eq!(m["scheme"], json!("b2b"));
    assert_eq!(m["contact_name"], json!("Debiteur BV"));

    assert_eq!(bukio::payments::list_mandates(&d, None).unwrap().len(), 1);
    let rows = bukio::payments::list_mandates(&d, None).unwrap();
    assert_eq!(rows[0]["mandate_ref"], json!("NL01ZZZ123456789012"));
    assert_eq!(
        bukio::payments::list_mandates(&d, Some(c)).unwrap().len(),
        1
    );
    assert_eq!(
        bukio::payments::list_mandates(&d, Some(999999))
            .unwrap()
            .len(),
        0
    );

    // duplicate ref for the same contact
    assert_eq!(
        code_of(bukio::payments::add_mandate(
            &d,
            c,
            "NL01ZZZ123456789012",
            None,
            "core",
            "agent:test",
            false
        )),
        "MANDATE_DUPLICATE"
    );
    // the same ref for ANOTHER contact is fine
    let c2 = dd_contact(&d, "Tweede BV");
    bukio::payments::add_mandate(
        &d,
        c2,
        "NL01ZZZ123456789012",
        None,
        "core",
        "agent:test",
        false,
    )
    .unwrap();
    assert_eq!(bukio::payments::list_mandates(&d, None).unwrap().len(), 2);

    // guards
    assert_eq!(
        code_of(bukio::payments::add_mandate(
            &d,
            999999,
            "R1",
            None,
            "core",
            "agent:test",
            false
        )),
        "CONTACT_NOT_FOUND"
    );
    assert_eq!(
        code_of(bukio::payments::add_mandate(
            &d,
            c,
            "",
            None,
            "core",
            "agent:test",
            false
        )),
        "INVALID_MANDATE_REF"
    );
    assert_eq!(
        code_of(bukio::payments::add_mandate(
            &d,
            c,
            &"x".repeat(36),
            None,
            "core",
            "agent:test",
            false
        )),
        "INVALID_MANDATE_REF"
    );
    assert_eq!(
        code_of(bukio::payments::add_mandate(
            &d,
            c,
            "R2",
            None,
            "sct",
            "agent:test",
            false
        )),
        "INVALID_SCHEME"
    );
    assert_eq!(
        code_of(bukio::payments::add_mandate(
            &d,
            c,
            "R3",
            Some("2026-02-30"),
            "core",
            "agent:test",
            false
        )),
        "INVALID_DATE"
    );

    // dry-run writes nothing
    let plan = bukio::payments::add_mandate(&d, c, "R4", None, "core", "agent:test", true).unwrap();
    assert_eq!(plan["dryRun"], json!(true));
    assert_eq!(bukio::payments::list_mandates(&d, None).unwrap().len(), 2);

    // remove
    let r = bukio::payments::remove_mandate(&d, m["id"].as_i64().unwrap(), "agent:test", false)
        .unwrap();
    assert_eq!(r["status"], json!("deleted"));
    assert_eq!(bukio::payments::list_mandates(&d, None).unwrap().len(), 1);
    assert_eq!(
        code_of(bukio::payments::remove_mandate(
            &d,
            999999,
            "agent:test",
            false
        )),
        "MANDATE_NOT_FOUND"
    );

    let audit: i64 = d
        .query_row(
            "SELECT COUNT(*) FROM audit_log WHERE action IN ('payments.mandate.add','payments.mandate.remove')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(audit, 3, "1 add + 1 remove (dry-runs do not audit)");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn direct_debit_batch_first_then_recurrent_and_mandate_snapshot() {
    let (dir, f) = dd_db("dd2");
    let d = bukio::db::open_db(&f).unwrap();
    let c = dd_contact(&d, "Debiteur BV");
    bukio::payments::add_mandate(
        &d,
        c,
        "NL01ZZZ999",
        Some("2026-07-01"),
        "core",
        "agent:test",
        false,
    )
    .unwrap();
    let p1 = dd_payable(&d, c, "INV-1", "direct_debit");

    let batch = bukio::payments::create_payment_batch(
        &d,
        Some("2026-08-10"),
        None,
        &[],
        &[p1],
        "direct_debit",
        "agent:test",
        false,
    )
    .unwrap();
    assert_eq!(batch["batch_kind"], json!("direct_debit"));
    let lines = batch["lines"].as_array().unwrap();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["mandate_ref"], json!("NL01ZZZ999"));
    assert_eq!(lines[0]["mandate_seq"], json!("FRST"));
    assert_eq!(lines[0]["scheme"], json!("core"));

    // second batch for the same contact -> RCUR
    let p2 = dd_payable(&d, c, "INV-2", "direct_debit");
    let batch2 = bukio::payments::create_payment_batch(
        &d,
        Some("2026-09-10"),
        None,
        &[],
        &[p2],
        "direct_debit",
        "agent:test",
        false,
    )
    .unwrap();
    assert_eq!(batch2["lines"][0]["mandate_seq"], json!("RCUR"));

    // a NEW mandate starts at FRST again (SEPA is per-mandate)
    bukio::payments::add_mandate(
        &d,
        c,
        "NL01ZZZ888",
        Some("2026-10-01"),
        "core",
        "agent:test",
        false,
    )
    .unwrap();
    let p3 = dd_payable(&d, c, "INV-3", "direct_debit");
    let batch3 = bukio::payments::create_payment_batch(
        &d,
        Some("2026-11-10"),
        None,
        &[],
        &[p3],
        "direct_debit",
        "agent:test",
        false,
    )
    .unwrap();
    assert_eq!(batch3["lines"][0]["mandate_ref"], json!("NL01ZZZ888"));
    assert_eq!(
        batch3["lines"][0]["mandate_seq"],
        json!("FRST"),
        "a brand-new mandate starts at FRST"
    );

    // removed + re-added with the SAME ref = a new mandate: FRST, not RCUR.
    // Counting by the ref snapshot alone used to emit RCUR here.
    let m1 = bukio::payments::list_mandates(&d, Some(c))
        .unwrap()
        .into_iter()
        .find(|m| m["mandate_ref"] == json!("NL01ZZZ999"))
        .unwrap();
    bukio::payments::remove_mandate(&d, m1["id"].as_i64().unwrap(), "agent:test", false).unwrap();
    bukio::payments::add_mandate(
        &d,
        c,
        "NL01ZZZ999",
        Some("2026-12-01"),
        "core",
        "agent:test",
        false,
    )
    .unwrap();
    let p4 = dd_payable(&d, c, "INV-4", "direct_debit");
    let batch4 = bukio::payments::create_payment_batch(
        &d,
        Some("2026-12-10"),
        None,
        &[],
        &[p4],
        "direct_debit",
        "agent:test",
        false,
    )
    .unwrap();
    assert_eq!(batch4["lines"][0]["mandate_ref"], json!("NL01ZZZ999"));
    assert_eq!(
        batch4["lines"][0]["mandate_seq"],
        json!("FRST"),
        "a re-created mandate with the same ref must start at FRST (SEPA per-mandate rule)"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn direct_debit_batch_without_a_mandate_is_refused() {
    let (dir, f) = dd_db("dd3");
    let d = bukio::db::open_db(&f).unwrap();
    let c = dd_contact(&d, "Debiteur BV");
    let p = dd_payable(&d, c, "F2026-10", "direct_debit");
    let err = bukio::payments::create_payment_batch(
        &d,
        Some("2026-08-10"),
        None,
        &[],
        &[p],
        "direct_debit",
        "agent:test",
        false,
    )
    .unwrap_err();
    assert_eq!(err.code, "BATCH_VALIDATION_FAILED");
    let details = err
        .details
        .clone()
        .and_then(|d| d.as_array().cloned())
        .unwrap_or_default();
    assert!(
        details.iter().any(|d| d["error"]
            .as_str()
            .unwrap_or("")
            .starts_with("MANDATE_REQUIRED")),
        "{details:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn payment_term_isolation_between_transfer_and_direct_debit() {
    let (dir, f) = dd_db("dd4");
    let d = bukio::db::open_db(&f).unwrap();
    let c = dd_contact(&d, "Debiteur BV");
    bukio::payments::add_mandate(&d, c, "M1", None, "core", "agent:test", false).unwrap();
    let dd = dd_payable(&d, c, "DD-1", "direct_debit");
    let tr = dd_payable(&d, c, "TR-1", "transfer");

    let e1 = bukio::payments::create_payment_batch(
        &d,
        None,
        None,
        &[],
        &[dd],
        "transfer",
        "agent:test",
        false,
    )
    .unwrap_err();
    assert_eq!(e1.code, "BATCH_VALIDATION_FAILED");
    assert!(e1
        .details
        .clone()
        .and_then(|d| d.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .any(|x| x["error"]
            .as_str()
            .unwrap_or("")
            .starts_with("PAYABLE_DIRECT_DEBIT")));

    let e2 = bukio::payments::create_payment_batch(
        &d,
        None,
        None,
        &[],
        &[tr],
        "direct_debit",
        "agent:test",
        false,
    )
    .unwrap_err();
    assert_eq!(e2.code, "BATCH_VALIDATION_FAILED");
    assert!(e2
        .details
        .clone()
        .and_then(|d| d.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .any(|x| x["error"]
            .as_str()
            .unwrap_or("")
            .starts_with("PAYABLE_NOT_DIRECT_DEBIT")));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn build_pain008_structure_mandate_data_agents_and_scheme_split() {
    let xml = bukio::payments::build_pain008(
        "BUKIO20260810123456",
        "2026-08-10T10:00:00Z",
        "Test Coaching",
        "NL91ABNA0417164300",
        "2026-08-12",
        &[
            json!({ "name": "Debiteur BV", "iban": "NL91ABNA0417164300", "amount_cents": 12100, "reference": "Factuur INV-1", "mandate_ref": "NL01ZZZ999", "mandate_date": "2026-07-01", "mandate_seq": "FRST", "scheme": "core" }),
            json!({ "name": "Grootbedrijf NV", "iban": "NL91ABNA0417164300", "amount_cents": 25000, "reference": "Factuur B2B-2", "mandate_ref": "B2BMAND1", "mandate_date": "2026-06-15", "mandate_seq": "RCUR", "scheme": "b2b" }),
        ],
    );
    for needle in [
        "xmlns=\"urn:iso:std:iso:20022:tech:xsd:pain.008.001.02\"",
        "<CstmrDrctDbtInitn>",
        "<PmtMtd>DD</PmtMtd>",
        "<CtrlSum>371.00</CtrlSum>",
        "<ReqdColltnDt>2026-08-12</ReqdColltnDt>",
        "<LclInstrm><Cd>CORE</Cd></LclInstrm>",
        "<LclInstrm><Cd>B2B</Cd></LclInstrm>",
        "<MndtId>NL01ZZZ999</MndtId>",
        "<DtOfSgntr>2026-07-01</DtOfSgntr>",
        "<DbtrAgt><FinInstnId><Othr><Id>NOTPROVIDED</Id></Othr></FinInstnId></DbtrAgt>",
        "<Dbtr><Nm>Grootbedrijf NV</Nm></Dbtr>",
    ] {
        assert!(xml.contains(needle), "missing {needle} in:\n{xml}");
    }
    assert_eq!(xml.matches("<PmtInf>").count(), 2, "one PmtInf per scheme");
    assert_eq!(xml.matches("<DrctDbtTxInf>").count(), 2);
}

#[test]
fn export_direct_debit_batch_uses_pain008_and_transfer_still_pain001() {
    let (dir, f) = dd_db("dd6");
    let d = bukio::db::open_db(&f).unwrap();
    let c = dd_contact(&d, "Debiteur BV");
    bukio::payments::add_mandate(&d, c, "M1", None, "core", "agent:test", false).unwrap();
    let p = dd_payable(&d, c, "F2026-10", "direct_debit");
    let batch = bukio::payments::create_payment_batch(
        &d,
        Some("2026-08-10"),
        None,
        &[],
        &[p],
        "direct_debit",
        "agent:test",
        false,
    )
    .unwrap();
    let id = batch["id"].as_i64().unwrap();

    let plan = bukio::payments::export_payment_batch(&d, id, "agent:test", true, None).unwrap();
    assert_eq!(plan["dryRun"], json!(true));
    assert_eq!(plan["schema"], json!("pain.008.001.02"));
    assert!(plan["xml"]
        .as_str()
        .unwrap_or("")
        .contains("pain.008.001.02"));

    let r = bukio::payments::export_payment_batch(&d, id, "agent:test", false, None).unwrap();
    assert_eq!(r["status"], json!("exported"));
    assert_eq!(r["schema"], json!("pain.008.001.02"));
    assert!(r["msg_id"].as_str().unwrap_or("").len() <= 35);

    let (schema, msg_id): (Option<String>, Option<String>) = d
        .query_row(
            "SELECT schema, msg_id FROM payment_batches WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(schema.as_deref(), Some("pain.008.001.02"));
    assert!(msg_id.is_some());

    // re-export blocked
    assert_eq!(
        code_of(bukio::payments::export_payment_batch(
            &d,
            id,
            "agent:test",
            false,
            None
        )),
        "BATCH_ALREADY_EXPORTED"
    );
    // wrong schema for a DD batch
    let c2 = dd_contact(&d, "Tweede BV");
    bukio::payments::add_mandate(&d, c2, "M2", None, "core", "agent:test", false).unwrap();
    let p2 = dd_payable(&d, c2, "INV-2", "direct_debit");
    let b2 = bukio::payments::create_payment_batch(
        &d,
        Some("2026-08-10"),
        None,
        &[],
        &[p2],
        "direct_debit",
        "agent:test",
        false,
    )
    .unwrap();
    assert_eq!(
        code_of(bukio::payments::export_payment_batch(
            &d,
            b2["id"].as_i64().unwrap(),
            "agent:test",
            false,
            Some("001.03")
        )),
        "INVALID_SCHEMA"
    );

    // a transfer batch still exports pain.001
    let tr = dd_payable(&d, c2, "TR-1", "transfer");
    let tb = bukio::payments::create_payment_batch(
        &d,
        Some("2026-08-10"),
        None,
        &[],
        &[tr],
        "transfer",
        "agent:test",
        false,
    )
    .unwrap();
    assert_eq!(tb["batch_kind"], json!("transfer"));
    let exp = bukio::payments::export_payment_batch(
        &d,
        tb["id"].as_i64().unwrap(),
        "agent:test",
        false,
        None,
    )
    .unwrap();
    assert_eq!(exp["schema"], json!("pain.001.001.03"));
    assert!(exp["xml"]
        .as_str()
        .unwrap_or("")
        .contains("pain.001.001.03"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cli_mandate_and_direct_debit_batch_end_to_end() {
    let (dir, f) = dd_db("dd7");
    let (_, ok, out) = run_cli(&[
        "--json",
        "contact",
        "add",
        "--name",
        "Debiteur BV",
        "--address",
        "Klantstraat 1",
        "--city",
        "Amsterdam",
        "--iban",
        "NL91ABNA0417164300",
        "--db",
        &f,
    ]);
    assert!(ok, "{out}");

    let (v, ok, out) = run_cli(&[
        "--json",
        "payments",
        "mandate",
        "add",
        "--contact",
        "1",
        "--ref",
        "NL01ZZZ999",
        "--date",
        "2026-07-01",
        "--type",
        "b2b",
        "--db",
        &f,
    ]);
    assert!(ok, "{out}");
    assert_eq!(v["data"]["scheme"], json!("b2b"), "{v}");

    let (v, ok, out) = run_cli(&["--json", "payments", "mandate", "list", "--db", &f]);
    assert!(ok, "{out}");
    assert_eq!(v["data"]["mandates"].as_array().unwrap().len(), 1, "{v}");

    let (_, ok, out) = run_cli(&[
        "--json",
        "payments",
        "payables",
        "add",
        "--contact",
        "1",
        "--ref",
        "INV-1",
        "--date",
        "2026-08-01",
        "--due",
        "2026-08-31",
        "--amount",
        "121.00",
        "--method",
        "direct-debit",
        "--db",
        &f,
    ]);
    assert!(ok, "{out}");

    let (v, ok, out) = run_cli(&[
        "--json",
        "payments",
        "batch",
        "create",
        "--type",
        "direct-debit",
        "--from-invoices",
        "--date",
        "2026-08-10",
        "--dry-run",
        "--db",
        &f,
    ]);
    assert!(ok, "{out}");
    assert_eq!(v["data"]["dryRun"], json!(true), "{v}");
    assert_eq!(v["data"]["batch_kind"], json!("direct_debit"), "{v}");
    assert_eq!(
        v["data"]["lines"][0]["mandate_ref"],
        json!("NL01ZZZ999"),
        "{v}"
    );

    let (v, ok, out) = run_cli(&[
        "--json",
        "payments",
        "batch",
        "create",
        "--type",
        "direct-debit",
        "--from-invoices",
        "--date",
        "2026-08-10",
        "--db",
        &f,
    ]);
    assert!(ok, "{out}");
    assert_eq!(v["data"]["lines"][0]["mandate_seq"], json!("FRST"), "{v}");
    let batch_id = v["data"]["id"].as_i64().unwrap();

    let (v, ok, out) = run_cli(&[
        "--json",
        "payments",
        "batch",
        "export",
        "--id",
        &batch_id.to_string(),
        "--db",
        &f,
    ]);
    assert!(ok, "{out}");
    assert_eq!(v["data"]["schema"], json!("pain.008.001.02"), "{v}");

    // the DD payable is now in_batch — a transfer batch must refuse it
    let (v, ok, out) = run_cli(&[
        "--json",
        "payments",
        "batch",
        "create",
        "--type",
        "transfer",
        "--payable",
        "1",
        "--date",
        "2026-08-10",
        "--db",
        &f,
    ]);
    assert!(!ok, "{out}");
    assert_eq!(v["error"]["code"], json!("PAYABLE_NOT_ELIGIBLE"), "{v}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn mcp_mandate_and_batch_tools() {
    let (dir, f) = dd_db("dd8");
    run_cli(&[
        "--json",
        "contact",
        "add",
        "--name",
        "Debiteur BV",
        "--address",
        "Klantstraat 1",
        "--city",
        "Amsterdam",
        "--iban",
        "NL91ABNA0417164300",
        "--db",
        &f,
    ]);
    run_cli(&[
        "--json",
        "payments",
        "payables",
        "add",
        "--contact",
        "1",
        "--ref",
        "INV-1",
        "--date",
        "2026-08-01",
        "--due",
        "2026-08-31",
        "--amount",
        "121.00",
        "--method",
        "direct-debit",
        "--db",
        &f,
    ]);

    let mut mcp = Mcp::start(&f);
    let (mand, is_err) = mcp.tool(
        "payments_mandate_add",
        json!({ "contact_id": 1, "mandate_ref": "NL01ZZZ999", "scheme": "b2b", "mode": "execute" }),
    );
    assert!(!is_err, "{mand}");
    assert_eq!(mand["mode"], json!("execute"));
    assert_eq!(mand["scheme"], json!("b2b"));

    let (list, _) = mcp.tool("payments_mandate_list", json!({}));
    assert_eq!(list["mandates"].as_array().unwrap().len(), 1, "{list}");

    let (plan, _) = mcp.tool(
        "payments_batch_create",
        json!({ "type": "direct_debit", "batch_date": "2026-08-10", "payable_ids": [1] }),
    );
    assert_eq!(plan["mode"], json!("dry-run"), "{plan}");
    assert_eq!(plan["batch_kind"], json!("direct_debit"), "{plan}");

    let (exec, _) = mcp.tool("payments_batch_create", json!({ "type": "direct_debit", "batch_date": "2026-08-10", "payable_ids": [1], "mode": "execute" }));
    assert_eq!(exec["mode"], json!("execute"), "{exec}");
    assert_eq!(exec["batch_kind"], json!("direct_debit"), "{exec}");

    let (exp, _) = mcp.tool(
        "payments_batch_export",
        json!({ "batch_id": exec["batch_id"], "mode": "execute" }),
    );
    assert_eq!(exp["schema"], json!("pain.008.001.02"), "{exp}");
    assert!(exp["xml"]
        .as_str()
        .unwrap_or("")
        .contains("pain.008.001.02"));
    mcp.stop();
    let _ = std::fs::remove_dir_all(&dir);
}

// ==== ported from test/import-invoice.test.js ===============================

/// Minimal-but-valid EN 16931 UBL fixture. Instead of the JS's regex surgery,
/// each omitted field is a flag — same document shapes, no string munging.
#[allow(clippy::too_many_arguments)]
fn ubl_invoice(
    id: &str,
    type_code: Option<&str>,
    issue_date: &str,
    due_date: Option<&str>,
    supplier_name: &str,
    vat_id: Option<&str>,
    payable: &str,
    tax_exclusive: &str,
    tax_amount: &str,
    percent: &str,
    currency: Option<&str>,
) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<Invoice xmlns="urn:oasis:names:specification:ubl:schema:xsd:Invoice-2"
         xmlns:cac="urn:oasis:names:specification:ubl:schema:xsd:CommonAggregateComponents-2"
         xmlns:cbc="urn:oasis:names:specification:ubl:schema:xsd:CommonBasicComponents-2">
  <cbc:CustomizationID>urn:cen.eu:en16931:2017</cbc:CustomizationID>
  <cbc:ID>{id}</cbc:ID>
  <cbc:IssueDate>{issue_date}</cbc:IssueDate>
  {due}
  {type_code}
  {currency}
  <cac:AccountingSupplierParty>
    <cac:Party>
      <cac:PartyName><cbc:Name>{supplier_name}</cbc:Name></cac:PartyName>
      <cac:PostalAddress>
        <cbc:StreetName>Leverstraat 3</cbc:StreetName>
        <cbc:CityName>Rotterdam</cbc:CityName>
        <cbc:PostalZone>3000 AA</cbc:PostalZone>
      </cac:PostalAddress>
      <cac:PartyTaxScheme>
        {company_id}
        <cac:TaxScheme><cbc:ID>VAT</cbc:ID></cac:TaxScheme>
      </cac:PartyTaxScheme>
      <cac:Contact><cbc:ElectronicMail>billing@acme.example</cbc:ElectronicMail></cac:Contact>
    </cac:Party>
  </cac:AccountingSupplierParty>
  <cac:AccountingCustomerParty>
    <cac:Party><cac:PartyName><cbc:Name>Test Coaching</cbc:Name></cac:PartyName></cac:Party>
  </cac:AccountingCustomerParty>
  <cac:TaxTotal>
    <cbc:TaxAmount currencyID="EUR">{tax_amount}</cbc:TaxAmount>
    <cac:TaxSubtotal>
      <cbc:TaxableAmount currencyID="EUR">{tax_exclusive}</cbc:TaxableAmount>
      <cbc:TaxAmount currencyID="EUR">{tax_amount}</cbc:TaxAmount>
      <cac:TaxCategory><cbc:ID>S</cbc:ID><cbc:Percent>{percent}</cbc:Percent></cac:TaxCategory>
    </cac:TaxSubtotal>
  </cac:TaxTotal>
  <cac:LegalMonetaryTotal>
    <cbc:LineExtensionAmount currencyID="EUR">{tax_exclusive}</cbc:LineExtensionAmount>
    <cbc:TaxExclusiveAmount currencyID="EUR">{tax_exclusive}</cbc:TaxExclusiveAmount>
    <cbc:TaxInclusiveAmount currencyID="EUR">{payable}</cbc:TaxInclusiveAmount>
    <cbc:PayableAmount currencyID="EUR">{payable}</cbc:PayableAmount>
  </cac:LegalMonetaryTotal>
  <cac:InvoiceLine>
    <cbc:ID>1</cbc:ID>
    <cbc:InvoicedQuantity unitCode="HUR">2.0</cbc:InvoicedQuantity>
    <cbc:LineExtensionAmount currencyID="EUR">{tax_exclusive}</cbc:LineExtensionAmount>
    <cac:Item><cbc:Name>Consultancy</cbc:Name></cac:Item>
    <cac:Price><cbc:PriceAmount currencyID="EUR">50.00</cbc:PriceAmount></cac:Price>
  </cac:InvoiceLine>
</Invoice>"#,
        id = id,
        due = due_date
            .map(|d| format!("<cbc:DueDate>{d}</cbc:DueDate>"))
            .unwrap_or_default(),
        type_code = type_code
            .map(|t| format!("<cbc:InvoiceTypeCode>{t}</cbc:InvoiceTypeCode>"))
            .unwrap_or_default(),
        currency = currency
            .map(|c| format!("<cbc:DocumentCurrencyCode>{c}</cbc:DocumentCurrencyCode>"))
            .unwrap_or_default(),
        supplier_name = supplier_name,
        company_id = vat_id
            .map(|v| format!("<cbc:CompanyID schemeID=\"VAT\">{v}</cbc:CompanyID>"))
            .unwrap_or_default(),
        payable = payable,
        tax_exclusive = tax_exclusive,
        tax_amount = tax_amount,
        percent = percent,
    )
}

/// the default fixture, exactly the JS default parameters
fn ubl_default() -> String {
    ubl_invoice(
        "F2026-123",
        Some("380"),
        "2026-08-01",
        Some("2026-08-31"),
        "Acme BV",
        Some("NL123456789B01"),
        "121.00",
        "100.00",
        "21.00",
        "21",
        Some("EUR"),
    )
}

fn import_db(tag: &str) -> (std::path::PathBuf, String) {
    cli_db(
        tag,
        &[
            "--registration-id",
            "12345678",
            "--legal-form",
            "eenmanszaak",
            "--vat",
            "off",
        ],
    )
}

fn payable_rows(db: &Connection) -> Vec<Value> {
    let mut stmt = db
        .prepare("SELECT invoice_ref, amount_cents, contact_id, source, source_ref, payment_method FROM payables ORDER BY id")
        .unwrap();
    stmt.query_map([], |r| {
        Ok(json!({
            "invoice_ref": r.get::<_, String>(0)?,
            "amount_cents": r.get::<_, i64>(1)?,
            "contact_id": r.get::<_, i64>(2)?,
            "source": r.get::<_, Option<String>>(3)?,
            "source_ref": r.get::<_, Option<String>>(4)?,
            "payment_method": r.get::<_, Option<String>>(5)?,
        }))
    })
    .unwrap()
    .map(|r| r.unwrap())
    .collect()
}

#[test]
fn import_ubl_registers_a_payable_matches_by_vat_id_and_parses_vat() {
    let (dir, f) = import_db("ubl1");
    let d = bukio::db::open_db(&f).unwrap();
    let existing = bukio::contacts::create_contact(
        &d,
        "Acme BV",
        Some("Leverstraat 3"),
        None,
        Some("Rotterdam"),
        None,
        None,
        Some("NL123456789B01"),
        None,
        None,
        "agent:test",
        false,
    )
    .unwrap();
    let existing_id = existing["id"].as_i64().unwrap();

    let r = bukio::import_mod::import_invoice(&d, &ubl_default(), None, false, "agent:test", false)
        .unwrap();
    assert_eq!(r["imported"].as_i64(), Some(1), "{r}");
    assert_eq!(r["duplicates"].as_i64(), Some(0));
    assert_eq!(r["amount_cents"].as_i64(), Some(12100));
    assert_eq!(r["due_date"], json!("2026-08-31"));
    assert_eq!(r["contact"]["id"].as_i64(), Some(existing_id));
    assert_eq!(r["vat_by_rate"]["21"].as_i64(), Some(2100), "{r}");

    let rows = payable_rows(&d);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["invoice_ref"], json!("F2026-123"));
    assert_eq!(rows[0]["amount_cents"].as_i64(), Some(12100));
    assert_eq!(rows[0]["contact_id"].as_i64(), Some(existing_id));
    assert_eq!(rows[0]["source"], json!("ubl"));
    assert_eq!(rows[0]["source_ref"], json!("nl123456789b01:F2026-123"));
    assert_eq!(rows[0]["payment_method"], json!("transfer"));

    let (actor, args): (String, String) = d
        .query_row(
            "SELECT actor, args_json FROM audit_log WHERE action = 'import.invoice'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(actor, "agent:test");
    let args: Value = serde_json::from_str(&args).unwrap();
    assert!(args["payable_id"].as_i64().is_some(), "{args}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn import_ubl_reimport_is_a_duplicate() {
    let (dir, f) = import_db("ubl2");
    let d = bukio::db::open_db(&f).unwrap();
    bukio::contacts::create_contact(
        &d,
        "Acme BV",
        None,
        None,
        None,
        None,
        None,
        Some("NL123456789B01"),
        None,
        None,
        "agent:test",
        false,
    )
    .unwrap();
    bukio::import_mod::import_invoice(&d, &ubl_default(), None, false, "agent:test", false)
        .unwrap();
    let r2 =
        bukio::import_mod::import_invoice(&d, &ubl_default(), None, false, "agent:test", false)
            .unwrap();
    assert_eq!(r2["imported"].as_i64(), Some(0), "{r2}");
    assert_eq!(r2["duplicates"].as_i64(), Some(1));
    assert_eq!(payable_rows(&d).len(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn import_ubl_create_missing_makes_the_supplier_contact() {
    let (dir, f) = import_db("ubl3");
    let d = bukio::db::open_db(&f).unwrap();
    let r = bukio::import_mod::import_invoice(&d, &ubl_default(), None, true, "agent:test", false)
        .unwrap();
    assert_eq!(r["imported"].as_i64(), Some(1), "{r}");
    assert_eq!(r["contacts_created"].as_i64(), Some(1), "{r}");
    let cid = r["contact"]["id"].as_i64().unwrap();
    let (name, vat_id, city, email): (String, Option<String>, Option<String>, Option<String>) = d
        .query_row(
            "SELECT name, vat_id, city, email FROM contacts WHERE id = ?1",
            [cid],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(name, "Acme BV");
    assert_eq!(vat_id.as_deref(), Some("NL123456789B01"));
    assert_eq!(city.as_deref(), Some("Rotterdam"));
    assert_eq!(email.as_deref(), Some("billing@acme.example"));

    // idempotent on re-import via the new contact's key
    let r2 = bukio::import_mod::import_invoice(&d, &ubl_default(), None, true, "agent:test", false)
        .unwrap();
    assert_eq!(r2["duplicates"].as_i64(), Some(1), "{r2}");
    assert_eq!(r2["contacts_created"].as_i64(), Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn import_ubl_tax_scheme_id_is_not_the_vat_number() {
    let (dir, f) = import_db("ubl4");
    let d = bukio::db::open_db(&f).unwrap();
    // a supplier block with only the scheme id (no CompanyID) must NOT store
    // 'VAT' as the vat_id — that would collapse the idempotency key and the
    // vat-id matching across every vendor carrying a PartyTaxScheme
    let xml = ubl_invoice(
        "F2026-123",
        Some("380"),
        "2026-08-01",
        Some("2026-08-31"),
        "Acme BV",
        None,
        "121.00",
        "100.00",
        "21.00",
        "21",
        Some("EUR"),
    );
    let r = bukio::import_mod::import_invoice(&d, &xml, None, true, "agent:test", false).unwrap();
    assert_eq!(r["imported"].as_i64(), Some(1), "{r}");
    let cid = r["contact"]["id"].as_i64().unwrap();
    let (name, vat): (String, Option<String>) = d
        .query_row(
            "SELECT name, vat_id FROM contacts WHERE id = ?1",
            [cid],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(name, "Acme BV");
    assert_eq!(vat, None, "the scheme id must not become the vat_id");
    // the idempotency key falls back to the normalized name
    assert_eq!(payable_rows(&d)[0]["source_ref"], json!("acmebv:F2026-123"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn import_ubl_explicit_contact_wins_and_missing_contact_is_refused() {
    let (dir, f) = import_db("ubl5");
    let d = bukio::db::open_db(&f).unwrap();
    let other = bukio::contacts::create_contact(
        &d,
        "Iets Anders BV",
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        "agent:test",
        false,
    )
    .unwrap()["id"]
        .as_i64()
        .unwrap();
    let r = bukio::import_mod::import_invoice(
        &d,
        &ubl_default(),
        Some(other),
        false,
        "agent:test",
        false,
    )
    .unwrap();
    assert_eq!(r["contact"]["id"].as_i64(), Some(other));
    assert_eq!(r["contact"]["name"], json!("Iets Anders BV"));

    let xml124 = ubl_invoice(
        "F2026-124",
        Some("380"),
        "2026-08-01",
        Some("2026-08-31"),
        "Acme BV",
        Some("NL123456789B01"),
        "121.00",
        "100.00",
        "21.00",
        "21",
        Some("EUR"),
    );
    assert_eq!(
        code_of(bukio::import_mod::import_invoice(
            &d,
            &xml124,
            None,
            false,
            "agent:test",
            false
        )),
        "CONTACT_NOT_FOUND"
    );
    assert_eq!(
        code_of(bukio::import_mod::import_invoice(
            &d,
            &ubl_default(),
            Some(999999),
            false,
            "agent:test",
            false
        )),
        "CONTACT_NOT_FOUND"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn import_ubl_validation_failures_write_nothing() {
    let (dir, f) = import_db("ubl6");
    let d = bukio::db::open_db(&f).unwrap();
    let missing_amount = ubl_default().replace(
        "<cbc:PayableAmount currencyID=\"EUR\">121.00</cbc:PayableAmount>",
        "",
    );
    let cases: Vec<(&str, String, &str)> = vec![
        ("not xml", "hello world".to_string(), "INVALID_UBL_INVOICE"),
        (
            "wrong root",
            "<AuditFile><Xaf/></AuditFile>".to_string(),
            "INVALID_UBL_INVOICE",
        ),
        ("missing amount", missing_amount, "IMPORT_VALIDATION_FAILED"),
        (
            "bad date",
            ubl_invoice(
                "F2026-123",
                Some("380"),
                "2026-02-30",
                Some("2026-08-31"),
                "Acme BV",
                Some("NL123456789B01"),
                "121.00",
                "100.00",
                "21.00",
                "21",
                Some("EUR"),
            ),
            "IMPORT_VALIDATION_FAILED",
        ),
        (
            "negative amount",
            ubl_invoice(
                "F2026-123",
                Some("380"),
                "2026-08-01",
                Some("2026-08-31"),
                "Acme BV",
                Some("NL123456789B01"),
                "-5.00",
                "100.00",
                "21.00",
                "21",
                Some("EUR"),
            ),
            "IMPORT_VALIDATION_FAILED",
        ),
        (
            "credit note",
            ubl_invoice(
                "F2026-123",
                Some("381"),
                "2026-08-01",
                Some("2026-08-31"),
                "Acme BV",
                Some("NL123456789B01"),
                "121.00",
                "100.00",
                "21.00",
                "21",
                Some("EUR"),
            ),
            "UNSUPPORTED_UBL_DOCUMENT",
        ),
        (
            "non-EUR currency",
            ubl_invoice(
                "F2026-123",
                Some("380"),
                "2026-08-01",
                Some("2026-08-31"),
                "Acme BV",
                Some("NL123456789B01"),
                "121.00",
                "100.00",
                "21.00",
                "21",
                Some("USD"),
            ),
            "IMPORT_VALIDATION_FAILED",
        ),
    ];
    for (label, xml, code) in cases {
        assert_eq!(
            code_of(bukio::import_mod::import_invoice(
                &d,
                &xml,
                None,
                true,
                "agent:test",
                false
            )),
            code,
            "{label}"
        );
    }
    assert_eq!(payable_rows(&d).len(), 0);
    let contacts: i64 = d
        .query_row("SELECT COUNT(*) FROM contacts", [], |r| r.get(0))
        .unwrap();
    assert_eq!(contacts, 0, "no contact was created by a failing import");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn import_ubl_due_date_defaults_to_issue_plus_30_days() {
    let (dir, f) = import_db("ubl7");
    let d = bukio::db::open_db(&f).unwrap();
    bukio::contacts::create_contact(
        &d,
        "Acme BV",
        None,
        None,
        None,
        None,
        None,
        Some("NL123456789B01"),
        None,
        None,
        "agent:test",
        false,
    )
    .unwrap();
    let xml = ubl_invoice(
        "F2026-123",
        Some("380"),
        "2026-08-01",
        None,
        "Acme BV",
        Some("NL123456789B01"),
        "121.00",
        "100.00",
        "21.00",
        "21",
        Some("EUR"),
    );
    let r = bukio::import_mod::import_invoice(&d, &xml, None, false, "agent:test", false).unwrap();
    assert_eq!(r["due_date"], json!("2026-08-31"), "{r}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn import_ubl_dry_run_validates_but_writes_nothing() {
    let (dir, f) = import_db("ubl8");
    let d = bukio::db::open_db(&f).unwrap();
    let r = bukio::import_mod::import_invoice(&d, &ubl_default(), None, true, "agent:test", true)
        .unwrap();
    assert_eq!(r["dryRun"], json!(true), "{r}");
    assert_eq!(r["action"], json!("import.invoice"));
    assert_eq!(r["contact"]["created"], json!(true));
    assert_eq!(payable_rows(&d).len(), 0);
    let contacts: i64 = d
        .query_row("SELECT COUNT(*) FROM contacts", [], |r| r.get(0))
        .unwrap();
    assert_eq!(contacts, 0);
    let audit: i64 = d
        .query_row(
            "SELECT COUNT(*) FROM audit_log WHERE action = 'import.invoice'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(audit, 0);

    // garbage still fails in dry-run
    assert_eq!(
        code_of(bukio::import_mod::import_invoice(
            &d,
            "garbage",
            None,
            true,
            "agent:test",
            true
        )),
        "INVALID_UBL_INVOICE"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cli_import_invoice_end_to_end() {
    let (dir, f) = import_db("ubl9");
    let xml_path = dir.join("invoice.xml");
    std::fs::write(&xml_path, ubl_default()).unwrap();
    let xp = xml_path.to_str().unwrap().to_string();

    let (v, ok, out) = run_cli(&[
        "--json",
        "import",
        "invoice",
        "--file",
        &xp,
        "--create-missing",
        "--dry-run",
        "--db",
        &f,
    ]);
    assert!(ok, "{out}");
    assert_eq!(v["data"]["dryRun"], json!(true), "{v}");
    assert_eq!(v["data"]["contact"]["created"], json!(true), "{v}");
    let d = bukio::db::open_db(&f).unwrap();
    assert_eq!(payable_rows(&d).len(), 0);
    drop(d);

    let (v, ok, out) = run_cli(&[
        "--json",
        "import",
        "invoice",
        "--file",
        &xp,
        "--create-missing",
        "--db",
        &f,
    ]);
    assert!(ok, "{out}");
    assert_eq!(v["data"]["imported"].as_i64(), Some(1), "{v}");

    let (v, ok, out) = run_cli(&["--json", "payments", "payables", "list", "--db", &f]);
    assert!(ok, "{out}");
    let payables = v["data"]["payables"].as_array().unwrap();
    assert_eq!(payables.len(), 1, "{v}");
    assert_eq!(payables[0]["invoice_ref"], json!("F2026-123"));

    let (v, ok, out) = run_cli(&[
        "--json",
        "import",
        "invoice",
        "--file",
        &xp,
        "--create-missing",
        "--db",
        &f,
    ]);
    assert!(ok, "{out}");
    assert_eq!(v["data"]["duplicates"].as_i64(), Some(1), "{v}");

    // missing file
    let nope = dir.join("nope.xml").to_str().unwrap().to_string();
    let (v, ok, out) = run_cli(&["--json", "import", "invoice", "--file", &nope, "--db", &f]);
    assert!(!ok, "{out}");
    assert_eq!(v["error"]["code"], json!("FILE_NOT_FOUND"), "{v}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn mcp_invoice_import_dry_run_and_execute() {
    let (dir, f) = import_db("ubl10");
    let xml_path = dir.join("invoice.xml");
    std::fs::write(&xml_path, ubl_default()).unwrap();
    let xp = xml_path.to_str().unwrap().to_string();

    let mut mcp = Mcp::start(&f);
    let (_, is_err) = mcp.tool("invoice_import", json!({ "file_path": "/nope/nope.xml" }));
    assert!(is_err, "a missing file must be an MCP error");

    let (plan, is_err) = mcp.tool(
        "invoice_import",
        json!({ "file_path": xp, "create_missing": true }),
    );
    assert!(!is_err, "{plan}");
    assert_eq!(plan["mode"], json!("dry-run"), "{plan}");
    {
        let d = bukio::db::open_db(&f).unwrap();
        assert_eq!(payable_rows(&d).len(), 0, "dry-run wrote a payable");
    }

    let (exec, is_err) = mcp.tool(
        "invoice_import",
        json!({ "file_path": xp, "create_missing": true, "mode": "execute" }),
    );
    assert!(!is_err, "{exec}");
    assert_eq!(exec["mode"], json!("execute"), "{exec}");
    assert_eq!(exec["imported"].as_i64(), Some(1), "{exec}");
    {
        let d = bukio::db::open_db(&f).unwrap();
        assert_eq!(payable_rows(&d).len(), 1);
    }
    mcp.stop();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn import_ubl_multiple_party_tax_scheme_entries_still_extract_the_vat_number() {
    let (dir, f) = import_db("ubl11");
    let d = bukio::db::open_db(&f).unwrap();
    let existing = bukio::contacts::create_contact(
        &d,
        "Acme BV",
        Some("Leverstraat 3"),
        None,
        Some("Rotterdam"),
        None,
        None,
        Some("NL123456789B01"),
        None,
        None,
        "agent:test",
        false,
    )
    .unwrap()["id"]
        .as_i64()
        .unwrap();
    // a supplier with BOTH a local tax number and a USt-IdNr (two
    // PartyTaxScheme siblings): the vat-id extraction must pick the VAT scheme
    let xml = ubl_default().replace(
        "</cac:PartyTaxScheme>",
        "</cac:PartyTaxScheme>\n      <cac:PartyTaxScheme>\n        <cbc:CompanyID schemeID=\"TIN\">DE123456789</cbc:CompanyID>\n        <cac:TaxScheme><cbc:ID>TIN</cbc:ID></cac:TaxScheme>\n      </cac:PartyTaxScheme>",
    );
    let r = bukio::import_mod::import_invoice(&d, &xml, None, false, "agent:test", false).unwrap();
    assert_eq!(r["imported"].as_i64(), Some(1), "{r}");
    assert_eq!(
        r["contact"]["id"].as_i64(),
        Some(existing),
        "matched by vat-id"
    );
    let rows = payable_rows(&d);
    assert_eq!(rows[0]["source_ref"], json!("nl123456789b01:F2026-123"));
    assert_eq!(rows[0]["contact_id"].as_i64(), Some(existing));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn import_ubl_missing_invoice_type_code_is_rejected() {
    let (dir, f) = import_db("ubl12");
    let d = bukio::db::open_db(&f).unwrap();
    let xml = ubl_invoice(
        "F2026-123",
        None,
        "2026-08-01",
        Some("2026-08-31"),
        "Acme BV",
        Some("NL123456789B01"),
        "121.00",
        "100.00",
        "21.00",
        "21",
        Some("EUR"),
    );
    let err =
        bukio::import_mod::import_invoice(&d, &xml, None, false, "agent:test", false).unwrap_err();
    assert_eq!(err.code, "INVALID_UBL_INVOICE");
    assert!(
        err.message.contains("InvoiceTypeCode is missing"),
        "{err:?}"
    );
    assert_eq!(payable_rows(&d).len(), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn import_ubl_missing_document_currency_code_is_rejected() {
    let (dir, f) = import_db("ubl13");
    let d = bukio::db::open_db(&f).unwrap();
    let xml = ubl_invoice(
        "F2026-123",
        Some("380"),
        "2026-08-01",
        Some("2026-08-31"),
        "Acme BV",
        Some("NL123456789B01"),
        "121.00",
        "100.00",
        "21.00",
        "21",
        None,
    );
    let err =
        bukio::import_mod::import_invoice(&d, &xml, None, false, "agent:test", false).unwrap_err();
    assert_eq!(err.code, "IMPORT_VALIDATION_FAILED");
    let details = err
        .details
        .clone()
        .and_then(|d| d.as_array().cloned())
        .unwrap_or_default();
    assert!(
        details.iter().any(|x| x["error"]
            .as_str()
            .unwrap_or("")
            .contains("DocumentCurrencyCode is missing")),
        "{details:?}"
    );
    assert_eq!(payable_rows(&d).len(), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn import_ubl_malformed_payable_amount_is_collected_with_the_other_errors() {
    let (dir, f) = import_db("ubl14");
    let d = bukio::db::open_db(&f).unwrap();
    // regression: a malformed amount used to THROW mid-parse, aborting before the
    // collected errors were checked — co-occurring problems were never reported
    let xml = ubl_invoice(
        "F2026-123",
        Some("380"),
        "2026-08-01",
        Some("2026-08-31"),
        "Acme BV",
        Some("NL123456789B01"),
        "1,2,3",
        "100.00",
        "21.00",
        "21",
        Some("EUR"),
    );
    let err =
        bukio::import_mod::import_invoice(&d, &xml, None, false, "agent:test", false).unwrap_err();
    assert_eq!(err.code, "IMPORT_VALIDATION_FAILED", "{err:?}");
    let details = err
        .details
        .clone()
        .and_then(|d| d.as_array().cloned())
        .unwrap_or_default();
    assert!(
        details
            .iter()
            .any(|x| x["error"].as_str().unwrap_or("").contains("PayableAmount")),
        "{details:?}"
    );
    assert_eq!(payable_rows(&d).len(), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

// ==== ported from test/invoice-features.test.js =============================

fn if_contact(db: &Connection, vat_id: Option<&str>) -> i64 {
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

fn if_item(db: &Connection, over: &[(&str, Value)]) -> Value {
    let get = |k: &str, d: Value| -> Value {
        over.iter()
            .find(|(n, _)| *n == k)
            .map(|(_, v)| v.clone())
            .unwrap_or(d)
    };
    bukio::items::create_item(
        db,
        get("name", json!("Consultancy")).as_str().unwrap(),
        get("description", Value::Null).as_str(),
        get("unit", json!("h")).as_str().unwrap(),
        get("unitPriceCents", json!(15000)).as_i64().unwrap(),
        get("vatCode", json!("21")).as_str(),
        get("glAccount", Value::Null).as_str(),
        "agent:test",
        false,
    )
    .unwrap()
}

fn inv_lines(db: &Connection, c: i64, ls: &[&str], date: &str) -> Value {
    bukio::invoice::create_invoice(
        db,
        c,
        date,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        &lines(ls),
        "agent:test",
        false,
    )
    .unwrap()
}

fn inv_disc(db: &Connection, c: i64, ls: &[&str], date: &str, dt: &str, dv: i64) -> Value {
    bukio::invoice::create_invoice(
        db,
        c,
        date,
        None,
        None,
        None,
        None,
        Some(dt),
        Some(dv),
        None,
        &lines(ls),
        "agent:test",
        false,
    )
    .unwrap()
}

#[test]
fn if_fractional_quantities_parse_to_milli_units() {
    assert_eq!(
        bukio::invoice::parse_line_spec("1.5x Coaching @ 100.00 @9").unwrap()["qtyMilli"],
        json!(1500)
    );
    assert_eq!(
        bukio::invoice::parse_line_spec("0.5x Coaching @ 100.00 @9").unwrap()["qtyMilli"],
        json!(500)
    );
    assert_eq!(
        bukio::invoice::parse_line_spec("2x Coaching @ 100.00").unwrap()["qtyMilli"],
        json!(2000)
    );
    assert_eq!(bukio::invoice::format_qty(2000), "2");
    assert_eq!(bukio::invoice::format_qty(1500), "1.5");
    assert_eq!(bukio::invoice::format_qty(1250), "1.25");
    assert_eq!(
        code_of(bukio::invoice::parse_line_spec("0x Ding @ 5.00")),
        "INVALID_LINE"
    );
}

#[test]
fn if_line_discounts_parse_and_over_100_pct_is_rejected_at_creation() {
    let d = setup();
    let parsed = bukio::invoice::parse_line_spec("2x Ding @ 10.00 @21 @-10%").unwrap();
    assert_eq!(parsed["qtyMilli"], json!(2000));
    assert_eq!(parsed["priceCents"], json!(1000));
    assert_eq!(parsed["vatCode"], json!("21"));
    assert_eq!(parsed["discountType"], json!("pct"));
    assert_eq!(parsed["discountValue"], json!(1000));
    let amount = bukio::invoice::parse_line_spec("Ding @ 10.00 @-2.50").unwrap();
    assert_eq!(amount["discountType"], json!("amount"));
    assert_eq!(amount["discountValue"], json!(250));
    // pct > 100 parses but is rejected at creation
    assert_eq!(
        bukio::invoice::parse_line_spec("Ding @ 10.00 @-101%").unwrap()["discountValue"],
        json!(10100)
    );
    let c = if_contact(&d, Some("NL999999999B01"));
    assert_eq!(
        code_of(bukio::invoice::create_invoice(
            &d,
            c,
            "2026-08-10",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            &lines(&["Ding @ 10.00 @-101%"]),
            "agent:test",
            false
        )),
        "INVALID_LINE_DISCOUNT"
    );
}

#[test]
fn if_item_specs_parse() {
    let p = bukio::invoice::parse_item_spec("1:2").unwrap();
    assert_eq!(p["itemId"], json!(1));
    assert_eq!(p["qtyMilli"], json!(2000));
    assert_eq!(p["priceCents"], Value::Null);
    assert_eq!(p["vatCode"], Value::Null);
    assert_eq!(p["discountType"], Value::Null);
    assert_eq!(p["discountValue"], Value::Null);
    let o = bukio::invoice::parse_item_spec("1:1.5@140.00@21@-10%").unwrap();
    assert_eq!(o["qtyMilli"], json!(1500));
    assert_eq!(o["priceCents"], json!(14000));
    assert_eq!(o["vatCode"], json!("21"));
    assert_eq!(o["discountType"], json!("pct"));
    assert_eq!(o["discountValue"], json!(1000));
    assert_eq!(
        code_of(bukio::invoice::parse_item_spec("x:2")),
        "INVALID_ITEM_SPEC"
    );
}

#[test]
fn if_allocate_largest_remainder_sums_exactly_and_is_deterministic() {
    let a = bukio::invoice::allocate_largest_remainder(100, &[700, 200, 100]);
    assert_eq!(a.iter().sum::<i64>(), 100);
    assert_eq!(
        a,
        bukio::invoice::allocate_largest_remainder(100, &[700, 200, 100])
    );
    assert_eq!(
        bukio::invoice::allocate_largest_remainder(0, &[1, 2]),
        vec![0, 0]
    );
    // zero-weight share never gets a cent
    assert_eq!(
        bukio::invoice::allocate_largest_remainder(1, &[100, 0]),
        vec![1, 0]
    );
}

#[test]
fn if_item_crud_with_audit() {
    let d = setup();
    if_contact(&d, Some("NL999999999B01"));
    let item = if_item(&d, &[]);
    assert_eq!(item["name"], json!("Consultancy"));
    assert_eq!(item["unit_price_cents"], json!(15000));
    let id = item["id"].as_i64().unwrap();

    assert_eq!(bukio::items::list_items(&d, true).unwrap().len(), 1);
    assert_eq!(
        bukio::items::get_item(&d, id).unwrap().unwrap()["name"],
        json!("Consultancy")
    );

    let updated = bukio::items::update_item(
        &d,
        id,
        None,
        None,
        None,
        Some(16000),
        None,
        None,
        false,
        "agent:test",
        false,
    )
    .unwrap();
    assert_eq!(updated["unit_price_cents"], json!(16000));
    assert_eq!(
        bukio::items::get_item(&d, id).unwrap().unwrap()["active"],
        json!(true)
    );

    bukio::items::update_item(
        &d,
        id,
        None,
        None,
        None,
        None,
        None,
        None,
        true,
        "agent:test",
        false,
    )
    .unwrap();
    assert_eq!(
        bukio::items::get_item(&d, id).unwrap().unwrap()["active"],
        json!(false)
    );
    assert_eq!(
        bukio::items::list_items(&d, true).unwrap().len(),
        0,
        "activeOnly by default"
    );
    assert_eq!(bukio::items::list_items(&d, false).unwrap().len(), 1);

    let mut stmt = d
        .prepare("SELECT action FROM audit_log WHERE action LIKE 'item.%' ORDER BY id")
        .unwrap();
    let actions: Vec<String> = stmt
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert_eq!(actions, vec!["item.create", "item.update", "item.update"]);
}

#[test]
fn if_item_update_empty_string_clears_vat_code_and_gl_account() {
    let d = setup();
    if_contact(&d, Some("NL999999999B01"));
    let item = if_item(
        &d,
        &[("vatCode", json!("21")), ("glAccount", json!("8000"))],
    );
    let id = item["id"].as_i64().unwrap();
    assert_eq!(
        bukio::items::get_item(&d, id).unwrap().unwrap()["vat_code"],
        json!("21")
    );
    assert_eq!(
        bukio::items::get_item(&d, id).unwrap().unwrap()["gl_account"],
        json!("8000")
    );

    // empty strings mean "clear" — they must not be kept or stored verbatim
    let updated = bukio::items::update_item(
        &d,
        id,
        None,
        None,
        None,
        None,
        Some(String::new()),
        Some(String::new()),
        false,
        "agent:test",
        false,
    )
    .unwrap();
    assert_eq!(
        updated["vat_code"],
        Value::Null,
        "empty vatCode must clear the code"
    );
    assert_eq!(
        updated["gl_account"],
        Value::Null,
        "empty glAccount must clear the account"
    );
    assert_eq!(
        bukio::items::get_item(&d, id).unwrap().unwrap()["gl_account"],
        Value::Null
    );
}

#[test]
fn if_item_guards() {
    let d = setup();
    let mk = |name: &str, unit: &str, price: i64, vat: Option<&str>, gl: Option<&str>| {
        bukio::items::create_item(&d, name, None, unit, price, vat, gl, "agent:test", false)
    };
    assert_eq!(code_of(mk("", "h", 100, None, None)), "INVALID_NAME");
    assert_eq!(code_of(mk("X", "weeks", 100, None, None)), "INVALID_UNIT");
    assert_eq!(code_of(mk("X", "h", 0, None, None)), "INVALID_PRICE");
    assert_eq!(
        code_of(mk("X", "h", 100, Some("999"), None)),
        "VAT_CODE_NOT_FOUND"
    );
    // dotted rates are FORMAT-valid after the parser fix — the NL fixture still
    // rejects them semantically, while malformed codes stay INVALID_VAT_CODE
    assert_eq!(
        code_of(mk("X", "h", 100, Some("5.5"), None)),
        "VAT_CODE_NOT_FOUND"
    );
    assert_eq!(
        code_of(mk("X", "h", 100, Some("5..5"), None)),
        "INVALID_VAT_CODE"
    );
    assert_eq!(
        code_of(mk("X", "h", 100, None, Some("9999"))),
        "ACCOUNT_NOT_FOUND"
    );
    assert_eq!(
        code_of(bukio::items::update_item(
            &d,
            999,
            None,
            None,
            None,
            None,
            None,
            None,
            false,
            "agent:test",
            false
        )),
        "ITEM_NOT_FOUND"
    );
    // dry-run writes nothing
    let plan = bukio::items::create_item(&d, "Dry", None, "h", 100, None, None, "agent:test", true)
        .unwrap();
    assert_eq!(plan["dryRun"], json!(true));
    assert_eq!(bukio::items::list_items(&d, true).unwrap().len(), 0);
}

#[test]
fn if_item_without_vat_code_is_allowed_when_the_vat_module_is_off() {
    let d = setup_with_vat(false);
    let item = bukio::items::create_item(
        &d,
        "Coaching",
        None,
        "session",
        7500,
        None,
        None,
        "agent:test",
        false,
    )
    .unwrap();
    assert_eq!(item["vat_code"], Value::Null);
    assert_eq!(
        code_of(bukio::items::create_item(
            &d,
            "X",
            None,
            "h",
            100,
            Some("21"),
            None,
            "agent:test",
            false
        )),
        "VAT_MODULE_OFF"
    );
}

#[test]
fn if_unit_labels_localize() {
    assert_eq!(bukio::i18n::unit_label("h", "nl"), "uur");
    assert_eq!(bukio::i18n::unit_label("h", "en"), "h");
    assert_eq!(bukio::i18n::unit_label("month", "en"), "month");
    assert_eq!(bukio::i18n::unit_label("unit", "nl"), "stuks");
}

#[test]
fn if_invoice_create_from_items_snapshots_catalog_values() {
    let d = setup();
    let c = if_contact(&d, Some("NL999999999B01"));
    let item = if_item(
        &d,
        &[
            ("name", json!("Advisory")),
            ("description", json!("Ad-hoc advisory")),
            ("unitPriceCents", json!(20000)),
        ],
    );
    let id = item["id"].as_i64().unwrap();
    let inv = bukio::invoice::create_invoice(
        &d,
        c,
        "2026-08-10",
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        &item_spec(&[&format!("{id}:2")]),
        "agent:test",
        false,
    )
    .unwrap();
    let l = &inv["lines"][0];
    assert_eq!(l["description"], json!("Ad-hoc advisory"));
    assert_eq!(l["quantity"], json!(2000));
    assert_eq!(l["unit"], json!("h"));
    assert_eq!(l["item_id"], json!(id));
    assert_eq!(l["unit_price_cents"], json!(20000));
    assert_eq!(l["amount_cents"], json!(40000));
    assert_eq!(inv["net_cents"], json!(40000));
    assert_eq!(inv["vat_cents"], json!(8400));
    assert_eq!(inv["gross_cents"], json!(48400));

    // price edits after creation never rewrite the invoice
    bukio::items::update_item(
        &d,
        id,
        None,
        None,
        None,
        Some(99900),
        None,
        None,
        false,
        "agent:test",
        false,
    )
    .unwrap();
    let again = bukio::invoice::get_invoice(&d, inv["id"].as_i64().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(again["lines"][0]["unit_price_cents"], json!(20000));
}

#[test]
fn if_invoice_create_from_items_per_invoice_overrides() {
    let d = setup();
    let c = if_contact(&d, Some("NL999999999B01"));
    let item = if_item(&d, &[("unitPriceCents", json!(20000))]);
    let id = item["id"].as_i64().unwrap();
    let inv = bukio::invoice::create_invoice(
        &d,
        c,
        "2026-08-10",
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        &item_spec(&[&format!("{id}:1.5@180.00@9@-10%")]),
        "agent:test",
        false,
    )
    .unwrap();
    let l = &inv["lines"][0];
    assert_eq!(l["unit_price_cents"], json!(18000), "override, not catalog");
    assert_eq!(l["vat_code"], json!("9"));
    assert_eq!(l["quantity"], json!(1500));
    assert_eq!(l["discount_type"], json!("pct"));
    assert_eq!(l["discount_value"], json!(1000));
    // 1.5 × 180.00 = 270.00, −10% = 243.00 net, 9% vat = 21.87
    assert_eq!(inv["net_cents"], json!(24300));
    assert_eq!(inv["vat_cents"], json!(2187));
    assert_eq!(inv["gross_cents"], json!(26487));
    assert_eq!(
        bukio::items::get_item(&d, id).unwrap().unwrap()["unit_price_cents"],
        json!(20000),
        "catalog untouched"
    );
}

#[test]
fn if_item_guards_on_invoices() {
    let d = setup();
    let c = if_contact(&d, Some("NL999999999B01"));
    let item = if_item(&d, &[]);
    let id = item["id"].as_i64().unwrap();
    let mk = |items: &[&str], ls: &[&str]| {
        let mut raw = item_spec(items);
        raw.extend(lines(ls));
        bukio::invoice::create_invoice(
            &d,
            c,
            "2026-08-10",
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
    };
    assert_eq!(code_of(mk(&["999:1"], &[])), "ITEM_NOT_FOUND");
    assert_eq!(
        code_of(mk(&[&format!("{id}:1@0.00")], &[])),
        "INVALID_ITEM_OVERRIDE"
    );
    assert_eq!(
        code_of(mk(&[&format!("{id}:1")], &["X @ 5.00"])),
        "CONFLICTING_LINES"
    );
    bukio::items::update_item(
        &d,
        id,
        None,
        None,
        None,
        None,
        None,
        None,
        true,
        "agent:test",
        false,
    )
    .unwrap();
    assert_eq!(code_of(mk(&[&format!("{id}:1")], &[])), "ITEM_INACTIVE");
}

#[test]
fn if_fractional_quantity_line_math() {
    let d = setup();
    let c = if_contact(&d, Some("NL999999999B01"));
    let inv = inv_lines(&d, c, &["1.5x Coaching @ 100.00 @9"], "2026-08-10");
    assert_eq!(inv["lines"][0]["quantity"], json!(1500));
    assert_eq!(inv["lines"][0]["amount_cents"], json!(15000));
    assert_eq!(inv["net_cents"], json!(15000));
    assert_eq!(inv["vat_cents"], json!(1350), "9%");
}

#[test]
fn if_line_discount_pct_and_amount_reduce_net_and_vat() {
    let d = setup();
    let c = if_contact(&d, Some("NL999999999B01"));
    let pct = inv_lines(&d, c, &["2x Ding @ 100.00 @21 @-10%"], "2026-08-10");
    assert_eq!(pct["lines"][0]["discount_type"], json!("pct"));
    assert_eq!(pct["lines"][0]["discount_value"], json!(1000));
    assert_eq!(pct["net_cents"], json!(18000)); // 200 − 20
    assert_eq!(pct["vat_cents"], json!(3780)); // 21% of 180
    assert_eq!(
        pct["discount_cents"],
        json!(0),
        "line discounts are per-line; invoice-level total is 0"
    );

    let amt = inv_lines(&d, c, &["2x Ding @ 100.00 @21 @-25.00"], "2026-08-10");
    assert_eq!(amt["net_cents"], json!(17500));
    assert_eq!(amt["vat_cents"], json!(3675));
    assert_eq!(amt["discount_cents"], json!(0));

    assert_eq!(
        code_of(bukio::invoice::create_invoice(
            &d,
            c,
            "2026-08-10",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            &lines(&["2x Ding @ 100.00 @-250.00"]),
            "agent:test",
            false
        )),
        "INVALID_LINE_DISCOUNT"
    );
}

#[test]
fn if_total_discount_single_rate_pct_and_amount() {
    let d = setup();
    let c = if_contact(&d, Some("NL999999999B01"));
    let pct = inv_disc(&d, c, &["2x Ding @ 100.00 @21"], "2026-08-10", "pct", 500); // 5%
    assert_eq!(pct["discount_cents"], json!(1000)); // 5% of 200.00
    assert_eq!(pct["net_cents"], json!(19000));
    assert_eq!(pct["vat_cents"], json!(3990));
    assert_eq!(pct["gross_cents"], json!(22990));

    let amt = inv_disc(
        &d,
        c,
        &["2x Ding @ 100.00 @21"],
        "2026-08-10",
        "amount",
        1000,
    );
    assert_eq!(amt["net_cents"], json!(19000));
    assert_eq!(amt["vat_cents"], json!(3990));
    assert_eq!(amt["discount_cents"], json!(1000));
}

#[test]
fn if_total_discount_across_mixed_vat_rates_allocates_to_the_cent() {
    let d = setup();
    let c = if_contact(&d, Some("NL999999999B01"));
    let inv = inv_disc(
        &d,
        c,
        &[
            "3x Hoog @ 100.00 @21",
            "2x Laag @ 100.00 @9",
            "1x Nul @ 100.00 @0",
        ],
        "2026-08-10",
        "amount",
        6000,
    );
    assert_eq!(inv["discount_cents"], json!(6000));
    assert_eq!(inv["net_cents"], json!(54000)); // 600 − 60
    assert_eq!(inv["vat_cents"], json!(7290)); // 21% of 270 + 9% of 180
    assert_eq!(inv["gross_cents"], json!(61290));
    // per-line VAT sums exactly to the invoice VAT
    let line_vat: i64 = inv["lines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["vat_amount_cents"].as_i64().unwrap())
        .sum();
    assert_eq!(line_vat, 7290);
    // the breakdown covers only rates that charge VAT (the 0% base is outside it)
    let sum_base: i64 = inv["vat_breakdown"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["base_cents"].as_i64().unwrap())
        .sum();
    assert_eq!(sum_base, 45000, "270 + 180 (21% + 9% bases only)");
    let sum_vat: i64 = inv["vat_breakdown"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["vat_cents"].as_i64().unwrap())
        .sum();
    assert_eq!(sum_vat, inv["vat_cents"].as_i64().unwrap());
}

#[test]
fn if_total_discount_with_awkward_split_still_balances() {
    let d = setup();
    let c = if_contact(&d, Some("NL999999999B01"));
    // 21% line 101.00 + 9% line 1.00 → a 1.00 discount must split 0.99/0.01
    let inv = inv_disc(
        &d,
        c,
        &["1x A @ 101.00 @21", "1x B @ 1.00 @9"],
        "2026-08-10",
        "amount",
        100,
    );
    assert_eq!(inv["discount_cents"], json!(100));
    assert_eq!(inv["net_cents"], json!(10100));
    assert_eq!(inv["vat_cents"], json!(2109), "21% of 100.00 + 9% of 1.00");
    let line_vat: i64 = inv["lines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["vat_amount_cents"].as_i64().unwrap())
        .sum();
    assert_eq!(line_vat, 2109);
}

#[test]
fn if_compute_invoice_totals_is_deterministic_across_recomputes() {
    let d = setup();
    let c = if_contact(&d, Some("NL999999999B01"));
    let inv = inv_disc(
        &d,
        c,
        &["3x A @ 33.33 @21", "2x B @ 12.50 @9", "1x C @ 7.77 @21"],
        "2026-08-10",
        "pct",
        750,
    );
    let fresh = bukio::invoice::get_invoice(&d, inv["id"].as_i64().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(fresh["net_cents"], inv["net_cents"]);
    assert_eq!(fresh["vat_cents"], inv["vat_cents"]);
    assert_eq!(fresh["vat_breakdown"], inv["vat_breakdown"]);
}

#[test]
fn if_booking_with_discounts_uses_discounted_nets_and_vat_per_rate() {
    let d = setup();
    let c = if_contact(&d, Some("NL999999999B01"));
    let inv = inv_disc(
        &d,
        c,
        &["3x Hoog @ 100.00 @21", "2x Laag @ 100.00 @9"],
        "2026-08-10",
        "amount",
        5000,
    );
    let postings = bukio::invoice::build_invoice_postings(&d, &inv).unwrap();
    let omzet: Vec<&Value> = postings
        .iter()
        .filter(|p| p["code"] == json!("8000"))
        .collect();
    assert_eq!(omzet.len(), 2, "one omzet leg per VAT rate");
    let omzet_sum: i64 = omzet
        .iter()
        .map(|p| p["amountCents"].as_i64().unwrap())
        .sum();
    assert_eq!(
        omzet_sum, -45000,
        "270 + 180 — the discount is allocated per rate"
    );
    let vat_leg = postings
        .iter()
        .find(|p| p["code"] == json!("2500"))
        .unwrap();
    assert_eq!(vat_leg["amountCents"], json!(-7290));
}

#[test]
fn if_finalize_with_discounts_books_a_balanced_entry() {
    let d = setup();
    let c = if_contact(&d, Some("NL999999999B01"));
    let inv = inv_disc(&d, c, &["2x Ding @ 100.00 @21"], "2026-08-10", "pct", 1000);
    let result =
        bukio::invoice::finalize_invoice(&d, inv["id"].as_i64().unwrap(), "agent:test", false)
            .unwrap();
    assert_eq!(result["invoice"]["invoice_number"], json!("2026-0001"));
    assert_eq!(result["invoice"]["gross_cents"], json!(21780)); // (200−20) + 21% of 180
    let entry_id = result["entry"]["id"].as_i64().unwrap();
    let state: String = d
        .query_row(
            "SELECT state FROM journal_entries WHERE id = ?1",
            [entry_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "posted");
    let sum: i64 = d
        .query_row(
            "SELECT COALESCE(SUM(amount_cents),0) FROM postings WHERE entry_id = ?1",
            [entry_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(sum, 0, "balanced");
}

#[test]
fn if_invoice_language_defaults_accepted_and_rejected() {
    let d = setup();
    let c = if_contact(&d, Some("NL999999999B01"));
    let mk = |date: &str, lang: Option<&str>| {
        bukio::invoice::create_invoice(
            &d,
            c,
            date,
            None,
            None,
            None,
            None,
            None,
            None,
            lang,
            &lines(&["Ding @ 10.00"]),
            "agent:test",
            false,
        )
    };
    assert_eq!(
        mk("2026-08-10", None).unwrap()["language"],
        json!("nl"),
        "nl default"
    );
    assert_eq!(
        mk("2026-08-11", Some("en")).unwrap()["language"],
        json!("en")
    );
    // every i18n table is a valid document language
    assert_eq!(
        mk("2026-08-11", Some("de")).unwrap()["language"],
        json!("de")
    );
    assert_eq!(
        mk("2026-08-12", Some("it")).unwrap()["language"],
        json!("it")
    );
    assert_eq!(code_of(mk("2026-08-11", Some("xx"))), "INVALID_LANGUAGE");
}

#[test]
fn if_cli_rejects_discount_pct_and_amount_together() {
    let (dir, f) = cli_db("ifdisc", &["--registration-id", "12345678", "--vat", "on"]);
    run_cli(&[
        "--json",
        "contact",
        "add",
        "--name",
        "ACME B.V.",
        "--address",
        "Straat 1",
        "--city",
        "Amsterdam",
        "--db",
        &f,
    ]);
    let (v, ok, out) = run_cli(&[
        "--json",
        "invoice",
        "create",
        "--contact",
        "1",
        "--lines",
        "1x Ding @ 10.00",
        "--date",
        "2026-08-10",
        "--discount-pct",
        "5",
        "--discount-amount",
        "5.00",
        "--db",
        &f,
    ]);
    assert!(!ok, "expected INVALID_DISCOUNT, got: {out}");
    assert_eq!(v["error"]["code"], json!("INVALID_DISCOUNT"), "{v}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn if_credit_note_inherits_language_and_discounts() {
    let d = setup();
    let c = if_contact(&d, Some("NL999999999B01"));
    let inv = bukio::invoice::create_invoice(
        &d,
        c,
        "2026-08-10",
        None,
        None,
        None,
        None,
        Some("pct"),
        Some(500),
        Some("en"),
        &lines(&["2x Ding @ 100.00 @21 @-10%"]),
        "agent:test",
        false,
    )
    .unwrap();
    let id = inv["id"].as_i64().unwrap();
    bukio::invoice::finalize_invoice(&d, id, "agent:test", false).unwrap();
    let credit = bukio::invoice::credit_invoice(&d, id, None, None, "agent:test", false).unwrap();
    assert_eq!(
        credit["language"],
        json!("en"),
        "inherits the source language"
    );
    assert_eq!(credit["discount_type"], json!("pct"));
    assert_eq!(credit["discount_value"], json!(500));
    assert_eq!(credit["lines"][0]["discount_type"], json!("pct"));
    assert_eq!(credit["net_cents"], inv["net_cents"]);
    assert_eq!(credit["vat_cents"], inv["vat_cents"]);
}

// ==== invoice-features, second half (UBL / PDF / logo / recurring / MCP / bank)

fn png_bytes(width: u32, height: u32) -> Vec<u8> {
    let mut b = vec![0u8; 33];
    b[0..8].copy_from_slice(&[0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
    b[8..12].copy_from_slice(&13u32.to_be_bytes());
    b[12..16].copy_from_slice(b"IHDR");
    b[16..20].copy_from_slice(&width.to_be_bytes());
    b[20..24].copy_from_slice(&height.to_be_bytes());
    b
}

fn jpeg_bytes(width: u16, height: u16) -> Vec<u8> {
    let mut b = vec![0u8; 41];
    b[0..2].copy_from_slice(&0xffd8u16.to_be_bytes());
    b[2..4].copy_from_slice(&0xffe0u16.to_be_bytes());
    b[4..6].copy_from_slice(&16u16.to_be_bytes());
    b[6..11].copy_from_slice(b"JFIF\0");
    b[20..22].copy_from_slice(&0xffc0u16.to_be_bytes());
    b[22..24].copy_from_slice(&17u16.to_be_bytes());
    b[24] = 8;
    b[25..27].copy_from_slice(&height.to_be_bytes());
    b[27..29].copy_from_slice(&width.to_be_bytes());
    b
}

fn ubl_of(db: &Connection, inv: &Value) -> String {
    let id = inv["id"].as_i64().unwrap();
    bukio::invoice::finalize_invoice(db, id, "agent:test", false).unwrap();
    let full = bukio::invoice::get_invoice(db, id).unwrap().unwrap();
    bukio::ubl::invoice_to_ubl(db, &full).unwrap()
}

#[test]
fn if_ubl_formatted_quantity_unit_code_language_and_discounted_bases() {
    let d = setup();
    let c = if_contact(&d, Some("NL999999999B01"));
    let inv = bukio::invoice::create_invoice(
        &d,
        c,
        "2026-08-10",
        None,
        None,
        None,
        None,
        Some("pct"),
        Some(1000),
        Some("en"),
        &lines(&["1.5x Consultancy @ 100.00 @21 @-10%"]),
        "agent:test",
        false,
    )
    .unwrap();
    let xml = ubl_of(&d, &inv);
    assert!(
        xml.contains(r#"<cbc:InvoicedQuantity unitCode="C62">1.5</cbc:InvoicedQuantity>"#),
        "{xml}"
    );
    // Peppol BIS 3.0 has NO top-level cbc:LanguageID (not in the UBL 2.1 content model)
    assert!(
        !xml.contains("<cbc:LanguageID>"),
        "UBL must not carry cbc:LanguageID"
    );
    // element order: InvoiceTypeCode → Note → DocumentCurrencyCode
    if let Some(note) = xml.find("<cbc:Note>") {
        let cur = xml.find("<cbc:DocumentCurrencyCode>").unwrap_or(usize::MAX);
        assert!(note < cur, "cbc:Note must precede cbc:DocumentCurrencyCode");
    }
    // the 10% line discount is a line allowance of 15.00
    assert!(xml.contains("<cbc:ChargeIndicator>false</cbc:ChargeIndicator>"));
    assert!(
        xml.contains(r#"<cbc:Amount currencyID="EUR">15.00</cbc:Amount>"#),
        "{xml}"
    );
    // BR-26: LineExtensionAmount is net of the line allowance
    assert!(xml
        .contains(r#"<cbc:LineExtensionAmount currencyID="EUR">135.00</cbc:LineExtensionAmount>"#));
    // BT-107 covers only the document-level allowance
    assert!(xml.contains(
        r#"<cbc:AllowanceTotalAmount currencyID="EUR">13.50</cbc:AllowanceTotalAmount>"#
    ));
    // UBL 2.1 LegalMonetaryTotal child order
    let lmt_start = xml.find("<cac:LegalMonetaryTotal>").unwrap();
    let lmt_end = xml.find("</cac:LegalMonetaryTotal>").unwrap();
    let lmt = &xml[lmt_start..lmt_end];
    let mut last = 0usize;
    for t in [
        "LineExtensionAmount",
        "TaxExclusiveAmount",
        "TaxInclusiveAmount",
        "AllowanceTotalAmount",
        "PayableAmount",
    ] {
        let i = lmt
            .find(&format!("<cbc:{t}"))
            .unwrap_or_else(|| panic!("{t} missing from {lmt}"));
        assert!(i >= last, "LegalMonetaryTotal children out of order at {t}");
        last = i;
    }
    // BR-CO-11: the document allowance (reason 95, 10%) sits after PaymentTerms, before TaxTotal
    assert!(xml.contains("<cbc:AllowanceChargeReasonCode>95</cbc:AllowanceChargeReasonCode>"));
    assert!(xml.contains("<cbc:MultiplierFactorNumeric>10</cbc:MultiplierFactorNumeric>"));
    let pt = xml.find("<cac:PaymentTerms>").unwrap_or(0);
    let ac = xml.find("<cac:AllowanceCharge>").unwrap();
    let tt = xml.find("<cac:TaxTotal>").unwrap();
    assert!(
        ac > pt && ac < tt,
        "document AllowanceCharge must sit between PaymentTerms and TaxTotal"
    );
    assert!(
        xml.contains(r#"<cbc:TaxExclusiveAmount currencyID="EUR">121.50</cbc:TaxExclusiveAmount>"#)
    );
    assert!(xml.contains(r#"<cbc:TaxableAmount currencyID="EUR">121.50</cbc:TaxableAmount>"#));
}

#[test]
fn if_ubl_line_only_discounts_and_category_mapping() {
    let d = setup();
    let c = if_contact(&d, Some("NL999999999B01"));
    let inv = bukio::invoice::create_invoice(
        &d,
        c,
        "2026-08-10",
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        &lines(&["1.5x Consultancy @ 100.00 @21 @-10%"]),
        "agent:test",
        false,
    )
    .unwrap();
    let xml = ubl_of(&d, &inv);
    // the line discount folds into the line net: 150 − 15 = 135
    assert!(xml
        .contains(r#"<cbc:LineExtensionAmount currencyID="EUR">135.00</cbc:LineExtensionAmount>"#));
    // no document-level allowance -> no AllowanceTotalAmount element
    assert!(!xml.contains("<cbc:AllowanceTotalAmount"), "{xml}");
    assert!(
        xml.contains(r#"<cbc:TaxExclusiveAmount currencyID="EUR">135.00</cbc:TaxExclusiveAmount>"#)
    );

    // category mapping: @0 -> Z, @V -> E, @21 -> S
    let c2 = if_contact(&d, Some("NL888888888B01"));
    let inv2 = bukio::invoice::create_invoice(
        &d,
        c2,
        "2026-08-10",
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        &lines(&[
            "1x Nul @ 10.00 @0",
            "1x Vrij @ 10.00 @V",
            "1x Normaal @ 10.00 @21",
        ]),
        "agent:test",
        false,
    )
    .unwrap();
    let xml2 = ubl_of(&d, &inv2);
    for cat in [
        "<cbc:ID>Z</cbc:ID>",
        "<cbc:ID>E</cbc:ID>",
        "<cbc:ID>S</cbc:ID>",
    ] {
        assert!(xml2.contains(cat), "missing {cat} in {xml2}");
    }
}

#[test]
fn if_ubl_zero_vat_categories_still_emit_a_tax_subtotal() {
    let d = setup();
    let c = if_contact(&d, Some("NL999999999B01"));
    let inv = bukio::invoice::create_invoice(
        &d,
        c,
        "2026-08-10",
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        &lines(&["1x Dienst @ 500.00 @RE", "1x Vrij @ 100.00 @V"]),
        "agent:test",
        false,
    )
    .unwrap();
    let xml = ubl_of(&d, &inv);
    // BT-5: the document currency code is mandatory
    assert!(xml.contains("<cbc:DocumentCurrencyCode>EUR</cbc:DocumentCurrencyCode>"));
    for (base, cat) in [("500.00", "AE"), ("100.00", "E")] {
        let marker = format!(r#"<cbc:TaxableAmount currencyID="EUR">{base}</cbc:TaxableAmount>"#);
        let i = xml
            .find(&marker)
            .unwrap_or_else(|| panic!("no {cat} base {base} in {xml}"));
        let rest = &xml[i..];
        let j = rest.find("<cbc:TaxAmount").unwrap();
        let zero = &rest[j..j + 60];
        assert!(
            zero.contains(">0.00<"),
            "{cat} subtotal must carry a zero TaxAmount: {zero}"
        );
        assert!(rest[j..].contains(&format!("<cbc:ID>{cat}</cbc:ID>")));
    }
    assert!(
        xml.contains(r#"<cbc:TaxExclusiveAmount currencyID="EUR">600.00</cbc:TaxExclusiveAmount>"#)
    );
    assert!(
        xml.contains(r#"<cbc:TaxInclusiveAmount currencyID="EUR">600.00</cbc:TaxInclusiveAmount>"#)
    );
}

#[test]
fn if_ubl_hour_unit_maps_to_hur() {
    let d = setup();
    let c = if_contact(&d, Some("NL999999999B01"));
    let inv = bukio::invoice::create_invoice(
        &d,
        c,
        "2026-08-10",
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        &lines(&["2x Coaching @ 50.00 @21"]),
        "agent:test",
        false,
    )
    .unwrap();
    d.execute(
        "UPDATE invoice_lines SET unit = ?1 WHERE invoice_id = ?2",
        rusqlite::params!["h", inv["id"].as_i64().unwrap()],
    )
    .unwrap();
    let xml = ubl_of(&d, &inv);
    assert!(
        xml.contains(r#"unitCode="HUR">2</cbc:InvoicedQuantity>"#),
        "{xml}"
    );
}

#[test]
fn if_pdf_dutch_labels_unit_column_vat_breakdown_and_discount_row() {
    let d = setup();
    let c = if_contact(&d, Some("NL999999999B01"));
    let inv = bukio::invoice::create_invoice(
        &d,
        c,
        "2026-08-10",
        None,
        None,
        None,
        None,
        Some("pct"),
        Some(500),
        None,
        &lines(&["2x Consultancy @ 100.00 @21 @-10%", "1x Maand @ 50.00 @9"]),
        "agent:test",
        false,
    )
    .unwrap();
    d.execute(
        "UPDATE invoice_lines SET unit = ?1 WHERE invoice_id = ?2 AND line_no = 1",
        rusqlite::params!["h", inv["id"].as_i64().unwrap()],
    )
    .unwrap();
    let full = bukio::invoice::get_invoice(&d, inv["id"].as_i64().unwrap())
        .unwrap()
        .unwrap();
    let html = bukio::pdf::invoice_html(&d, &full);
    for needle in [
        "FACTUUR",
        "Factuur aan",
        "Omschrijving",
        "Aantal",
        "Eenheid",
        "Btw over 21%",
        "Btw over 9%",
        "Totaal btw",
        "Korting",
        "Totaal (incl. btw)",
    ] {
        assert!(html.contains(needle), "missing {needle} in the Dutch PDF");
    }
    assert!(html.contains(">2<"), "formatted quantity, not 2000");
    assert!(html.contains(">uur<"), "localized unit");
    assert!(
        !html.contains("2000</td>"),
        "milli must never leak to the PDF"
    );
    let totals = bukio::invoice::compute_invoice_totals(
        full["lines"].as_array().unwrap(),
        full["discount_type"].as_str(),
        full["discount_value"].as_i64(),
    );
    let net = bukio::money::format_amount(totals["net_before_cents"].as_i64().unwrap());
    let gross = bukio::money::format_amount(totals["gross_cents"].as_i64().unwrap());
    assert!(
        html.contains(&net),
        "net {net} missing from the rendered totals"
    );
    assert!(
        html.contains(&gross),
        "gross {gross} missing from the rendered totals"
    );
}

#[test]
fn if_pdf_english_labels() {
    let d = setup();
    let c = if_contact(&d, Some("NL999999999B01"));
    let inv = bukio::invoice::create_invoice(
        &d,
        c,
        "2026-08-10",
        None,
        None,
        None,
        None,
        None,
        None,
        Some("en"),
        &lines(&["1x Ding @ 100.00 @21"]),
        "agent:test",
        false,
    )
    .unwrap();
    let html = bukio::pdf::invoice_html(&d, &inv);
    for needle in [
        "INVOICE",
        "Billed to",
        "Description",
        "Qty",
        "Unit",
        "Subtotal excl. VAT",
        "Total (incl. VAT)",
    ] {
        assert!(html.contains(needle), "missing {needle} in the English PDF");
    }
}

#[test]
fn if_pdf_company_logo_renders_as_a_data_uri() {
    let d = setup();
    let c = if_contact(&d, Some("NL999999999B01"));
    let inv = bukio::invoice::create_invoice(
        &d,
        c,
        "2026-08-10",
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        &lines(&["1x Ding @ 10.00"]),
        "agent:test",
        false,
    )
    .unwrap();
    let png = png_bytes(120, 60);
    d.execute(
        "UPDATE company SET logo = ?1, logo_mime = ?2 WHERE id = 1",
        rusqlite::params![png, "image/png"],
    )
    .unwrap();
    let html = bukio::pdf::invoice_html(&d, &inv);
    assert!(
        html.contains(r#"<img class="logo" src="data:image/png;base64,"#),
        "{html}"
    );
    // and without a logo there is no <img>
    d.execute(
        "UPDATE company SET logo = NULL, logo_mime = NULL WHERE id = 1",
        [],
    )
    .unwrap();
    assert!(!bukio::pdf::invoice_html(&d, &inv).contains("<img"));
}

#[test]
fn if_pdf_native_renderer_produces_a_valid_pdf() {
    let d = setup();
    let c = if_contact(&d, Some("NL999999999B01"));
    let inv = bukio::invoice::create_invoice(
        &d,
        c,
        "2026-08-10",
        None,
        None,
        None,
        None,
        Some("pct"),
        Some(1000),
        Some("en"),
        &lines(&["1.5x Consultancy @ 100.00 @21 @-10%"]),
        "agent:test",
        false,
    )
    .unwrap();
    let dir = temp_dir("ifpdf");
    let out = dir.join("inv.pdf");
    let res = bukio::pdf::invoice_to_pdf(&d, &inv, Some(out.to_str().unwrap())).unwrap();
    assert!(res["bytes"].as_u64().unwrap_or(0) > 1000, "{res}");
    let bytes = std::fs::read(&out).unwrap();
    assert!(bytes.len() > 1000);
    assert!(bytes.starts_with(b"%PDF-"));
    assert!(dir.exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn if_recurring_invoice_template_snapshots_catalog_prices_per_run() {
    let (dir, f) = cli_db("ifrec", &["--registration-id", "12345678", "--vat", "on"]);
    run_cli(&[
        "--json",
        "contact",
        "add",
        "--name",
        "ACME B.V.",
        "--address",
        "Straat 1",
        "--city",
        "Amsterdam",
        "--db",
        &f,
    ]);
    let (v, ok, out) = run_cli(&[
        "--json", "item", "add", "--name", "SaaS", "--unit", "month", "--price", "99.00", "--vat",
        "21", "--db", &f,
    ]);
    assert!(ok, "{out}");
    let item_id = v["data"]["item_id"].as_i64().unwrap_or(1);

    let (v, ok, out) = run_cli(&[
        "--json",
        "recurring",
        "add",
        "--kind",
        "invoice",
        "--name",
        "SaaS abonnement",
        "--contact",
        "1",
        "--items",
        &format!("{item_id}:1"),
        "--frequency",
        "monthly",
        "--start",
        "2026-08-01",
        "--due-days",
        "14",
        "--db",
        &f,
    ]);
    assert!(ok, "{out}");
    assert_eq!(v["data"]["template"]["vat_aware"], json!(1), "{v}");

    let (v, ok, out) = run_cli(&[
        "--json",
        "recurring",
        "run",
        "--as-of",
        "2026-08-31",
        "--db",
        &f,
    ]);
    assert!(ok, "{out}");
    let d = bukio::db::open_db(&f).unwrap();
    let aug_id: i64 = d
        .query_row("SELECT id FROM invoices ORDER BY id LIMIT 1", [], |r| {
            r.get(0)
        })
        .unwrap();
    let aug = bukio::invoice::get_invoice(&d, aug_id).unwrap().unwrap();
    assert_eq!(aug["lines"][0]["item_id"], json!(item_id));
    assert_eq!(aug["lines"][0]["unit_price_cents"], json!(9900));
    assert_eq!(aug["lines"][0]["quantity"], json!(1000));

    // a price change applies from the NEXT run (snapshot semantics)
    let (_, ok, out) = run_cli(&[
        "--json",
        "item",
        "update",
        "--id",
        &item_id.to_string(),
        "--price",
        "119.00",
        "--db",
        &f,
    ]);
    assert!(ok, "{out}");
    let (_, ok, out) = run_cli(&[
        "--json",
        "recurring",
        "run",
        "--as-of",
        "2026-09-30",
        "--db",
        &f,
    ]);
    assert!(ok, "{out}");
    let sep_id: i64 = d
        .query_row(
            "SELECT id FROM invoices WHERE date = '2026-09-01'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let sep = bukio::invoice::get_invoice(&d, sep_id).unwrap().unwrap();
    assert_eq!(
        sep["lines"][0]["unit_price_cents"],
        json!(11900),
        "the new price applies this run"
    );
    let aug_again = bukio::invoice::get_invoice(&d, aug_id).unwrap().unwrap();
    assert_eq!(
        aug_again["lines"][0]["unit_price_cents"],
        json!(9900),
        "past drafts untouched"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn if_mcp_item_tools_and_invoice_create_with_items_discount_and_language() {
    let (dir, f) = cli_db("ifmcp", &["--registration-id", "12345678", "--vat", "on"]);
    run_cli(&[
        "--json",
        "contact",
        "add",
        "--name",
        "ACME B.V.",
        "--address",
        "Straat 1",
        "--city",
        "Amsterdam",
        "--db",
        &f,
    ]);

    let mut mcp = Mcp::start(&f);
    let (added, is_err) = mcp.tool(
        "item_add",
        json!({ "name": "Consultancy", "unit": "h", "unit_price": "150.00", "vat_code": "21", "mode": "execute", "actor": "agent:mcp-test" }),
    );
    assert!(!is_err, "{added}");
    assert_eq!(added["action"], json!("item.create"));

    let (list, _) = mcp.tool("item_list", json!({}));
    assert!(list.to_string().contains("Consultancy"), "{list}");

    let (inv, is_err) = mcp.tool(
        "invoice_create",
        json!({ "contact_id": 1, "items": ["1:2@140.00"], "date": "2026-08-10", "discount_pct": 5, "language": "en", "mode": "execute", "actor": "agent:mcp-test" }),
    );
    assert!(!is_err, "{inv}");
    assert_eq!(inv["invoice_id"], json!(1), "{inv}");
    assert_eq!(inv["totals"]["discount"], json!(1400), "5% of 280.00");
    assert_eq!(inv["totals"]["net"], json!(26600));
    assert_eq!(inv["totals"]["vat"], json!(5586));

    let (deact, _) = mcp.tool(
        "item_update",
        json!({ "id": 1, "deactivate": true, "mode": "execute", "actor": "agent:mcp-test" }),
    );
    assert_eq!(deact["active"], json!(false), "item_update must deactivate");

    // item_add dry-run writes nothing
    let (plan, _) = mcp.tool(
        "item_add",
        json!({ "name": "X", "unit_price": "10.00", "mode": "dry-run" }),
    );
    assert_eq!(plan["dryRun"], json!(true));
    mcp.stop();

    let check = bukio::db::open_db(&f).unwrap();
    let n: i64 = check
        .query_row("SELECT COUNT(*) FROM items", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 1, "dry-run wrote nothing");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn if_bank_auto_match_matches_a_discounted_invoice_at_its_discounted_gross() {
    let d = setup();
    let c = if_contact(&d, Some("NL999999999B01"));
    // 2x 100 @21 with a 10% total discount -> gross 217.80 (line sums say 242.00)
    let inv = bukio::invoice::create_invoice(
        &d,
        c,
        "2026-08-01",
        None,
        None,
        None,
        None,
        Some("pct"),
        Some(1000),
        None,
        &lines(&["2x Dienst @ 100.00 @21"]),
        "agent:test",
        false,
    )
    .unwrap();
    let id = inv["id"].as_i64().unwrap();
    bukio::invoice::finalize_invoice(&d, id, "agent:test", false).unwrap();
    assert_eq!(
        bukio::invoice::get_invoice(&d, id).unwrap().unwrap()["gross_cents"],
        json!(21780)
    );

    bukio::bank::get_or_create_bank_account(&d, "NL91ABNA0417164300", None, "1100", false).unwrap();
    let tx = vec![bukio::bank::BankTx {
        date: "2026-08-05".into(),
        amount_cents: 21780,
        counterparty: Some("ACME B.V.".into()),
        description: Some("betaling factuur".into()),
        iban_counter: None,
        bank_ref: None,
        iban: None,
    }];
    bukio::bank::import_transactions(
        &d,
        "NL91ABNA0417164300",
        &tx,
        Some("Betaalrekening"),
        "1100",
        "agent:test",
    )
    .unwrap();

    let dry = bukio::bank::auto_match(&d, 14, "agent:test", true).unwrap();
    let matched = dry["matched"].as_array().unwrap();
    assert_eq!(matched.len(), 1, "{dry}");
    assert_eq!(matched[0]["kind"], json!("invoice"));
    assert_eq!(matched[0]["fx_delta_cents"], json!(0));

    bukio::bank::auto_match(&d, 14, "agent:test", false).unwrap();
    let paid = bukio::invoice::get_invoice(&d, id).unwrap().unwrap();
    assert_eq!(paid["status"], json!("paid"));
    assert_eq!(paid["paid_cents"], json!(21780));
    assert_eq!(
        bukio::reports::trial_balance(&d, None).unwrap()["balanced"],
        json!(true)
    );
}

#[test]
fn if_bank_auto_match_does_not_match_a_pre_discount_payment() {
    let d = setup();
    let c = if_contact(&d, Some("NL999999999B01"));
    let inv = bukio::invoice::create_invoice(
        &d,
        c,
        "2026-08-01",
        None,
        None,
        None,
        None,
        Some("pct"),
        Some(1000),
        None,
        &lines(&["2x Dienst @ 100.00 @21"]),
        "agent:test",
        false,
    )
    .unwrap();
    bukio::invoice::finalize_invoice(&d, inv["id"].as_i64().unwrap(), "agent:test", false).unwrap();
    bukio::bank::get_or_create_bank_account(&d, "NL91ABNA0417164300", None, "1100", false).unwrap();
    let tx = vec![bukio::bank::BankTx {
        date: "2026-08-05".into(),
        amount_cents: 24200,
        counterparty: Some("ACME B.V.".into()),
        description: Some("pre-discount amount".into()),
        iban_counter: None,
        bank_ref: None,
        iban: None,
    }];
    bukio::bank::import_transactions(
        &d,
        "NL91ABNA0417164300",
        &tx,
        Some("Betaalrekening"),
        "1100",
        "agent:test",
    )
    .unwrap();
    let dry = bukio::bank::auto_match(&d, 14, "agent:test", true).unwrap();
    assert_eq!(
        dry["matched"].as_array().unwrap().len(),
        0,
        "242.00 != 217.80 and outside tolerance: {dry}"
    );
}

#[test]
fn if_company_logo_set_extract_round_trip_and_remove() {
    let (dir, f) = cli_db("iflogo", &["--registration-id", "12345678", "--vat", "on"]);
    let logo = dir.join("logo.png");
    let bytes = png_bytes(200, 80);
    std::fs::write(&logo, &bytes).unwrap();

    let (v, ok, out) = run_cli(&[
        "--json",
        "company",
        "update",
        "--logo",
        logo.to_str().unwrap(),
        "--db",
        &f,
    ]);
    assert!(ok, "{out}");
    assert_eq!(v["data"]["company"]["logo_mime"], json!("image/png"), "{v}");
    assert_eq!(v["data"]["company"]["logo_bytes"], json!(33));

    let extract = dir.join("out.png");
    let (v, ok, out) = run_cli(&[
        "--json",
        "company",
        "logo",
        "--out",
        extract.to_str().unwrap(),
        "--db",
        &f,
    ]);
    assert!(ok, "{out}");
    assert_eq!(
        std::fs::read(&extract).unwrap(),
        bytes,
        "byte-identical round-trip"
    );

    let (_, ok, out) = run_cli(&["--json", "company", "update", "--remove-logo", "--db", &f]);
    assert!(ok, "{out}");
    let (v, ok, out) = run_cli(&["--json", "company", "show", "--db", &f]);
    assert!(ok, "{out}");
    assert_eq!(v["data"]["company"]["logo_mime"], Value::Null, "{v}");
    assert_eq!(v["data"]["company"]["logo_bytes"], Value::Null, "{v}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn if_company_logo_format_size_and_dimension_guards() {
    let (dir, f) = cli_db("iflogo2", &["--registration-id", "12345678", "--vat", "on"]);
    let bad = dir.join("logo.txt");
    std::fs::write(&bad, b"not an image at all").unwrap();
    let (v, ok, _) = run_cli(&[
        "--json",
        "company",
        "update",
        "--logo",
        bad.to_str().unwrap(),
        "--db",
        &f,
    ]);
    assert!(!ok);
    assert_eq!(v["error"]["code"], json!("LOGO_UNSUPPORTED_FORMAT"), "{v}");

    let big = dir.join("big.png");
    std::fs::write(&big, png_bytes(4096, 100)).unwrap();
    let (v, ok, _) = run_cli(&[
        "--json",
        "company",
        "update",
        "--logo",
        big.to_str().unwrap(),
        "--db",
        &f,
    ]);
    assert!(!ok);
    assert_eq!(
        v["error"]["code"],
        json!("LOGO_DIMENSIONS_TOO_LARGE"),
        "{v}"
    );

    let huge = dir.join("huge.png");
    let mut oversized = png_bytes(100, 100);
    oversized.extend(std::iter::repeat(0u8).take(1_100_000));
    std::fs::write(&huge, oversized).unwrap();
    let (v, ok, _) = run_cli(&[
        "--json",
        "company",
        "update",
        "--logo",
        huge.to_str().unwrap(),
        "--db",
        &f,
    ]);
    assert!(!ok);
    assert_eq!(v["error"]["code"], json!("LOGO_TOO_LARGE"), "{v}");

    let nope = dir.join("nope.png");
    let (v, ok, _) = run_cli(&[
        "--json",
        "company",
        "update",
        "--logo",
        nope.to_str().unwrap(),
        "--db",
        &f,
    ]);
    assert!(!ok);
    assert_eq!(v["error"]["code"], json!("LOGO_FILE_NOT_FOUND"), "{v}");

    // JPEG + SVG accepted; SVG dimensions come from width/height
    let jpg = dir.join("logo.jpg");
    std::fs::write(&jpg, jpeg_bytes(120, 60)).unwrap();
    let (v, ok, out) = run_cli(&[
        "--json",
        "company",
        "update",
        "--logo",
        jpg.to_str().unwrap(),
        "--db",
        &f,
    ]);
    assert!(ok, "{out}");
    assert_eq!(
        v["data"]["company"]["logo_mime"],
        json!("image/jpeg"),
        "{v}"
    );

    let svg = dir.join("logo.svg");
    std::fs::write(
        &svg,
        br#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="50"></svg>"#,
    )
    .unwrap();
    let (v, ok, out) = run_cli(&[
        "--json",
        "company",
        "update",
        "--logo",
        svg.to_str().unwrap(),
        "--db",
        &f,
    ]);
    assert!(ok, "{out}");
    assert_eq!(
        v["data"]["company"]["logo_mime"],
        json!("image/svg+xml"),
        "{v}"
    );

    // an XML declaration + a long comment before <svg (real-world logo files)
    let commented = dir.join("commented.svg");
    std::fs::write(
        &commented,
        b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!-- a long comment block that pushes <svg far past the first 200 bytes of the file -->\n<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"100\" height=\"50\"></svg>",
    )
    .unwrap();
    let (v, ok, out) = run_cli(&[
        "--json",
        "company",
        "update",
        "--logo",
        commented.to_str().unwrap(),
        "--db",
        &f,
    ]);
    assert!(ok, "{out}");
    assert_eq!(
        v["data"]["company"]["logo_mime"],
        json!("image/svg+xml"),
        "{v}"
    );

    let outpng = dir.join("x.png");
    let (v, ok, out) = run_cli(&[
        "--json",
        "company",
        "logo",
        "--out",
        outpng.to_str().unwrap(),
        "--db",
        &f,
    ]);
    assert!(ok, "{out}");
    assert_eq!(v["data"]["mime"], json!("image/svg+xml"), "{v}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn if_review_fix_reverse_charge_label_and_email_language_follow_the_document() {
    // an ES invoice with a reverse-charge line: the PDF must show the Spanish
    // label, never the Dutch 'verlegd'
    let (dir, f) = cli_db(
        "ifes",
        &[
            "--country",
            "ES",
            "--legal-form",
            "sl",
            "--vat",
            "on",
            "--registration-id",
            "M-123456",
            "--tax-id",
            "ESB12345678",
            "--address",
            "Calle 1",
            "--postal-code",
            "28001",
            "--city",
            "Madrid",
        ],
    );
    let d = bukio::db::open_db(&f).unwrap();
    let c = bukio::contacts::create_contact(
        &d,
        "Cliente SL",
        Some("Calle 2"),
        None,
        Some("Barcelona"),
        None,
        None,
        None,
        None,
        None,
        "agent:test",
        false,
    )
    .unwrap()["id"]
        .as_i64()
        .unwrap();
    let inv = bukio::invoice::create_invoice(
        &d,
        c,
        "2026-08-10",
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        &lines(&["Servicio @ 100.00 @R"]),
        "agent:test",
        false,
    )
    .unwrap();
    assert_eq!(
        inv["language"],
        json!("es"),
        "an ES company invoices in Spanish"
    );
    let html = bukio::pdf::invoice_html(&d, &inv);
    assert!(html.contains("inversión del sujeto pasivo"), "{html}");
    assert!(
        !html.contains("verlegd"),
        "no Dutch fallback on a Spanish PDF"
    );

    let en = bukio::invoice::create_invoice(
        &d,
        c,
        "2026-08-11",
        None,
        None,
        None,
        None,
        None,
        None,
        Some("en"),
        &lines(&["Servicio @ 100.00 @R"]),
        "agent:test",
        false,
    )
    .unwrap();
    let en_html = bukio::pdf::invoice_html(&d, &en);
    assert!(en_html.contains("reverse charge"));
    assert!(!en_html.contains("inversión del sujeto pasivo"));

    // invoice emails follow the document language too
    assert_eq!(
        bukio::pdf::default_subject("it", "2026-0001", "Rossi SRL"),
        "Fattura 2026-0001 — Rossi SRL"
    );
    assert_eq!(
        bukio::pdf::default_subject("es", "2026-0001", "Perez SL"),
        "Factura 2026-0001 — Perez SL"
    );
    assert_eq!(
        bukio::pdf::default_subject("de", "2026-0001", "Muster GmbH"),
        "Rechnung 2026-0001 — Muster GmbH"
    );
    assert_eq!(
        bukio::pdf::default_subject("nl", "2026-0001", "Demo BV"),
        "Factuur 2026-0001 — Demo BV"
    );
    assert_eq!(
        bukio::pdf::default_subject("xx", "2026-0001", "X"),
        "Invoice 2026-0001 — X",
        "unknown -> the English table"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ==== attachments (ported from test/attachments.test.js) ====================

/// A company + contact + draft invoice + draft entry, on a file DB (file-mode
/// attachments need a real DB path).
fn attach_env(tag: &str) -> (std::path::PathBuf, String, i64, i64) {
    let (dir, f) = cli_db(
        tag,
        &[
            "--registration-id",
            "12345678",
            "--legal-form",
            "eenmanszaak",
            "--vat",
            "off",
        ],
    );
    run_cli(&[
        "--json",
        "company",
        "update",
        "--address",
        "Teststraat 1",
        "--postal-code",
        "1000 AA",
        "--city",
        "Amsterdam",
        "--db",
        &f,
    ]);
    let db = bukio::db::open_db(&f).unwrap();
    let c = bukio::contacts::create_contact(
        &db,
        "Acme BV",
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        "agent:test",
        false,
    )
    .unwrap();
    let inv = bukio::invoice::create_invoice(
        &db,
        c["id"].as_i64().unwrap(),
        "2026-08-10",
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        &[json!("Ding @ 10.00")],
        "agent:test",
        false,
    )
    .unwrap();
    let entry = bukio::entries::create_entry(
        &db,
        bukio::entries::CreateEntry {
            date: "2026-08-10",
            description: "Startkapitaal",
            postings: vec![
                bukio::entries::PostingSpec {
                    code: "1100".into(),
                    amount_cents: 10000,
                    cost_center_code: None,
                    vat_code: None,
                    vat_amount_cents: None,
                },
                bukio::entries::PostingSpec {
                    code: "3000".into(),
                    amount_cents: -10000,
                    cost_center_code: None,
                    vat_code: None,
                    vat_amount_cents: None,
                },
            ],
            source: "manual",
            source_ref: None,
            actor: "agent:test",
        },
    )
    .unwrap();
    (dir, f, inv["id"].as_i64().unwrap(), entry.id)
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}

fn b64(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn unb64(s: &str) -> Vec<u8> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.decode(s).unwrap()
}

#[test]
fn attach_add_db_mode_stores_a_blob_round_trips_and_infers_mime() {
    let (dir, f, inv_id, _) = attach_env("att1");
    let doc = b"%PDF-1.4 fake invoice bytes".to_vec();
    let src = dir.join("F2026-123.pdf");
    std::fs::write(&src, &doc).unwrap();

    let db = bukio::db::open_db(&f).unwrap();
    let a = bukio::attachments::add_attachment(
        &db,
        "invoice",
        inv_id,
        src.to_str().unwrap(),
        Some("originel"),
        "db",
        "agent:test",
        false,
    )
    .unwrap();
    assert!(a["id"].as_i64().unwrap() > 0, "{a}");
    assert_eq!(a["mode"], json!("db"));
    assert_eq!(a["mime"], json!("application/pdf"));
    assert_eq!(a["sha256"], json!(sha256_hex(&doc)));
    assert_eq!(a["size"], json!(doc.len()));

    // the row holds the BLOB and no path
    let (mode, data, path): (String, Option<Vec<u8>>, Option<String>) = db
        .query_row(
            "SELECT mode, data, path FROM attachments WHERE id = ?1",
            [a["id"].as_i64().unwrap()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(mode, "db");
    assert_eq!(data.unwrap(), doc);
    assert_eq!(path, None);

    // get_attachment returns the bytes
    let got = bukio::attachments::get_attachment(&db, a["id"].as_i64().unwrap()).unwrap();
    assert_eq!(unb64(got["data"].as_str().unwrap()), doc);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn attach_add_works_for_entries_too() {
    let (dir, f, _, entry_id) = attach_env("att2");
    let src = dir.join("factuur.xml");
    std::fs::write(&src, b"some xml invoice").unwrap();
    let db = bukio::db::open_db(&f).unwrap();
    let a = bukio::attachments::add_attachment(
        &db,
        "entry",
        entry_id,
        src.to_str().unwrap(),
        None,
        "db",
        "agent:test",
        false,
    )
    .unwrap();
    assert_eq!(a["kind"], json!("entry"));
    assert_eq!(a["ref_id"], json!(entry_id));
    assert_eq!(a["mime"], json!("application/xml"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn attach_add_validation_errors() {
    let (dir, f, inv_id, _) = attach_env("att3");
    let src = dir.join("x.pdf");
    std::fs::write(&src, b"data").unwrap();
    let p = src.to_str().unwrap().to_string();
    let db = bukio::db::open_db(&f).unwrap();
    let err = |r: bukio::money::Result<Value>| r.unwrap_err().code;

    assert_eq!(
        err(bukio::attachments::add_attachment(
            &db,
            "bogus",
            inv_id,
            &p,
            None,
            "db",
            "agent:test",
            false
        )),
        "INVALID_KIND"
    );
    assert_eq!(
        err(bukio::attachments::add_attachment(
            &db,
            "invoice",
            0,
            &p,
            None,
            "db",
            "agent:test",
            false
        )),
        "REF_REQUIRED"
    );
    assert_eq!(
        err(bukio::attachments::add_attachment(
            &db,
            "invoice",
            999999,
            &p,
            None,
            "db",
            "agent:test",
            false
        )),
        "NOT_FOUND"
    );
    let missing = dir.join("nope.pdf");
    assert_eq!(
        err(bukio::attachments::add_attachment(
            &db,
            "invoice",
            inv_id,
            missing.to_str().unwrap(),
            None,
            "db",
            "agent:test",
            false
        )),
        "ATTACHMENT_FILE_NOT_FOUND"
    );
    assert_eq!(
        err(bukio::attachments::add_attachment(
            &db,
            "invoice",
            inv_id,
            &p,
            None,
            "bogus",
            "agent:test",
            false
        )),
        "INVALID_STORE"
    );

    bukio::attachments::add_attachment(&db, "invoice", inv_id, &p, None, "db", "agent:test", false)
        .unwrap();
    assert_eq!(
        err(bukio::attachments::add_attachment(
            &db,
            "invoice",
            inv_id,
            &p,
            None,
            "db",
            "agent:test",
            false
        )),
        "ATTACHMENT_DUPLICATE"
    );

    let big = dir.join("big.pdf");
    std::fs::write(
        &big,
        vec![0u8; bukio::attachments::MAX_ATTACHMENT_BYTES + 1],
    )
    .unwrap();
    assert_eq!(
        err(bukio::attachments::add_attachment(
            &db,
            "invoice",
            inv_id,
            big.to_str().unwrap(),
            None,
            "db",
            "agent:test",
            false
        )),
        "ATTACHMENT_TOO_LARGE"
    );

    // an empty file is a friendly error, not a raw CHECK-constraint failure
    let empty = dir.join("empty.pdf");
    std::fs::write(&empty, b"").unwrap();
    assert_eq!(
        err(bukio::attachments::add_attachment(
            &db,
            "invoice",
            inv_id,
            empty.to_str().unwrap(),
            None,
            "db",
            "agent:test",
            false
        )),
        "ATTACHMENT_EMPTY"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn attach_list_is_metadata_only_without_the_blob() {
    let (dir, f, inv_id, _) = attach_env("att4");
    let doc = b"hello".to_vec();
    let src = dir.join("a.pdf");
    std::fs::write(&src, &doc).unwrap();
    let db = bukio::db::open_db(&f).unwrap();
    bukio::attachments::add_attachment(
        &db,
        "invoice",
        inv_id,
        src.to_str().unwrap(),
        Some("n1"),
        "db",
        "agent:test",
        false,
    )
    .unwrap();
    // the same bytes again is a duplicate, so exactly one row remains
    let dup = bukio::attachments::add_attachment(
        &db,
        "invoice",
        inv_id,
        src.to_str().unwrap(),
        Some("n2"),
        "db",
        "agent:test",
        false,
    )
    .unwrap_err();
    assert_eq!(dup.code, "ATTACHMENT_DUPLICATE");

    let rows = bukio::attachments::list_attachments(&db, "invoice", inv_id).unwrap();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0]["file_name"], json!("a.pdf"));
    assert_eq!(rows[0]["mode"], json!("db"));
    assert_eq!(rows[0]["sha256"], json!(sha256_hex(&doc)));
    assert!(
        rows[0].get("data").is_none(),
        "list must not carry the BLOB: {:?}",
        rows[0]
    );
    assert_eq!(
        bukio::attachments::list_attachments(&db, "invoice", 999999)
            .unwrap()
            .len(),
        0
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn attach_remove_deletes_and_audits_and_unknown_ids_error() {
    let (dir, f, inv_id, _) = attach_env("att5");
    let src = dir.join("a.pdf");
    std::fs::write(&src, b"hello").unwrap();
    let db = bukio::db::open_db(&f).unwrap();
    let a = bukio::attachments::add_attachment(
        &db,
        "invoice",
        inv_id,
        src.to_str().unwrap(),
        None,
        "db",
        "agent:test",
        false,
    )
    .unwrap();
    let id = a["id"].as_i64().unwrap();

    let r = bukio::attachments::remove_attachment(&db, id, "agent:test", false).unwrap();
    assert_eq!(r["id"], json!(id));
    let n: i64 = db
        .query_row("SELECT COUNT(*) FROM attachments", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0);

    let (actor, args): (String, String) = db
        .query_row(
            "SELECT actor, args_json FROM audit_log WHERE action = 'attachments.remove'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(actor, "agent:test");
    let args: Value = serde_json::from_str(&args).unwrap();
    assert_eq!(args["attachment_id"], json!(id));

    assert_eq!(
        bukio::attachments::remove_attachment(&db, id, "agent:test", false)
            .unwrap_err()
            .code,
        "ATTACHMENT_NOT_FOUND"
    );
    assert_eq!(
        bukio::attachments::get_attachment(&db, id)
            .unwrap_err()
            .code,
        "ATTACHMENT_NOT_FOUND"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn attach_add_dry_run_writes_nothing_and_audits_nothing() {
    let (dir, f, inv_id, _) = attach_env("att6");
    let src = dir.join("a.pdf");
    std::fs::write(&src, b"hello").unwrap();
    let db = bukio::db::open_db(&f).unwrap();

    let plan = bukio::attachments::add_attachment(
        &db,
        "invoice",
        inv_id,
        src.to_str().unwrap(),
        None,
        "db",
        "agent:test",
        true,
    )
    .unwrap();
    assert_eq!(plan["dryRun"], json!(true), "{plan}");
    assert_eq!(plan["action"], json!("attachments.add"));
    assert_eq!(plan["sha256"], json!(sha256_hex(b"hello")));
    let n: i64 = db
        .query_row("SELECT COUNT(*) FROM attachments", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0);
    let audited: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM audit_log WHERE action = 'attachments.add'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(audited, 0);

    // a dry-run remove plans without deleting
    let a = bukio::attachments::add_attachment(
        &db,
        "invoice",
        inv_id,
        src.to_str().unwrap(),
        None,
        "db",
        "agent:test",
        false,
    )
    .unwrap();
    let r =
        bukio::attachments::remove_attachment(&db, a["id"].as_i64().unwrap(), "agent:test", true)
            .unwrap();
    assert_eq!(r["dryRun"], json!(true));
    let n: i64 = db
        .query_row("SELECT COUNT(*) FROM attachments", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 1);
    let audited: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM audit_log WHERE action = 'attachments.remove'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(audited, 0);

    // a dry-run remove of a nonexistent id still validates
    assert_eq!(
        bukio::attachments::remove_attachment(&db, 999999, "agent:test", true)
            .unwrap_err()
            .code,
        "ATTACHMENT_NOT_FOUND"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn attach_file_mode_copies_to_sha256_and_remove_deletes_the_copy() {
    let (dir, f, inv_id, _) = attach_env("att7");
    let doc = b"%PDF-1.4 file-mode doc".to_vec();
    let src = dir.join("F2026-124.pdf");
    std::fs::write(&src, &doc).unwrap();
    let db = bukio::db::open_db(&f).unwrap();

    let a = bukio::attachments::add_attachment(
        &db,
        "invoice",
        inv_id,
        src.to_str().unwrap(),
        None,
        "file",
        "agent:test",
        false,
    )
    .unwrap();
    assert_eq!(a["mode"], json!("file"));
    let path = a["path"].as_str().unwrap().to_string();
    assert!(
        path.ends_with(&sha256_hex(&doc)),
        "the copy is stored under its sha256: {path}"
    );
    assert!(
        std::path::Path::new(&path).exists(),
        "copy must exist on disk"
    );
    assert_eq!(std::fs::read(&path).unwrap(), doc);

    // the DB row carries the path, no BLOB
    let (data, row_path): (Option<Vec<u8>>, Option<String>) = db
        .query_row(
            "SELECT data, path FROM attachments WHERE id = ?1",
            [a["id"].as_i64().unwrap()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(data, None);
    assert_eq!(row_path.as_deref(), Some(path.as_str()));

    // show round-trips
    let got = bukio::attachments::get_attachment(&db, a["id"].as_i64().unwrap()).unwrap();
    assert_eq!(unb64(got["data"].as_str().unwrap()), doc);

    // remove deletes the copy
    bukio::attachments::remove_attachment(&db, a["id"].as_i64().unwrap(), "agent:test", false)
        .unwrap();
    assert!(
        !std::path::Path::new(&path).exists(),
        "the file-mode copy must be deleted with the row"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn attach_get_file_mode_with_a_missing_copy_is_attachment_file_missing() {
    let (dir, f, inv_id, _) = attach_env("att8");
    let src = dir.join("a.pdf");
    std::fs::write(&src, b"hello").unwrap();
    let db = bukio::db::open_db(&f).unwrap();
    let a = bukio::attachments::add_attachment(
        &db,
        "invoice",
        inv_id,
        src.to_str().unwrap(),
        None,
        "file",
        "agent:test",
        false,
    )
    .unwrap();
    let copy_dir = std::path::Path::new(a["path"].as_str().unwrap())
        .parent()
        .unwrap()
        .to_path_buf();
    std::fs::remove_dir_all(&copy_dir).unwrap();

    let err = bukio::attachments::get_attachment(&db, a["id"].as_i64().unwrap()).unwrap_err();
    assert_eq!(err.code, "ATTACHMENT_FILE_MISSING", "{err:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn attach_cli_round_trip_with_audit() {
    let (dir, f, inv_id, _) = attach_env("att9");
    let doc = b"%PDF-1.4 cli doc".to_vec();
    let src = dir.join("F2026-125.pdf");
    std::fs::write(&src, &doc).unwrap();

    let (added, ok, out) = run_cli(&[
        "--json",
        "attach",
        "add",
        "--invoice",
        &inv_id.to_string(),
        "--file",
        src.to_str().unwrap(),
        "--note",
        "cli test",
        "--db",
        &f,
    ]);
    assert!(ok, "{out}");
    let id = added["data"]["id"].as_i64().unwrap();
    assert_eq!(added["data"]["mode"], json!("db"), "{added}");
    assert_eq!(added["data"]["file_name"], json!("F2026-125.pdf"));

    let (listed, _, out) = run_cli(&[
        "--json",
        "attach",
        "list",
        "--invoice",
        &inv_id.to_string(),
        "--db",
        &f,
    ]);
    assert_eq!(
        listed["data"]["attachments"].as_array().unwrap().len(),
        1,
        "{out}"
    );
    assert_eq!(listed["data"]["attachments"][0]["id"], json!(id));

    let out_dir = dir.join("out");
    let out_file = out_dir.join("extracted.pdf");
    let (_, ok, out) = run_cli(&[
        "--json",
        "attach",
        "show",
        "--id",
        &id.to_string(),
        "--out",
        out_file.to_str().unwrap(),
        "--db",
        &f,
    ]);
    assert!(ok, "{out}");
    assert_eq!(std::fs::read(&out_file).unwrap(), doc);

    // --out that exists without --force → FILE_EXISTS
    let (blocked, ok, _) = run_cli(&[
        "--json",
        "attach",
        "show",
        "--id",
        &id.to_string(),
        "--out",
        out_file.to_str().unwrap(),
        "--db",
        &f,
    ]);
    assert!(!ok);
    assert_eq!(blocked["error"]["code"], json!("FILE_EXISTS"), "{blocked}");
    let (_, ok, out) = run_cli(&[
        "--json",
        "attach",
        "show",
        "--id",
        &id.to_string(),
        "--out",
        out_file.to_str().unwrap(),
        "--force",
        "--db",
        &f,
    ]);
    assert!(ok, "{out}");

    let db = bukio::db::open_db(&f).unwrap();
    let (n, actor, command): (i64, String, String) = db
        .query_row(
            "SELECT COUNT(*), MAX(actor), MAX(command) FROM audit_log WHERE action = 'attachments.add'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(n, 1);
    assert_eq!(actor, "agent:test");
    assert_eq!(command, "attach add");

    let (_, ok, out) = run_cli(&[
        "--json",
        "attach",
        "remove",
        "--id",
        &id.to_string(),
        "--db",
        &f,
    ]);
    assert!(ok, "{out}");
    let (listed, _, _) = run_cli(&[
        "--json",
        "attach",
        "list",
        "--invoice",
        &inv_id.to_string(),
        "--db",
        &f,
    ]);
    assert_eq!(listed["data"]["attachments"].as_array().unwrap().len(), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn attach_cli_rejects_both_refs_and_an_unknown_store() {
    let (dir, f, inv_id, entry_id) = attach_env("att10");
    let src = dir.join("a.pdf");
    std::fs::write(&src, b"x").unwrap();
    let p = src.to_str().unwrap().to_string();

    let (both, ok, _) = run_cli(&[
        "--json",
        "attach",
        "add",
        "--invoice",
        &inv_id.to_string(),
        "--entry",
        &entry_id.to_string(),
        "--file",
        &p,
        "--db",
        &f,
    ]);
    assert!(!ok);
    assert_eq!(both["error"]["code"], json!("REF_REQUIRED"), "{both}");

    let (none, ok, _) = run_cli(&["--json", "attach", "add", "--file", &p, "--db", &f]);
    assert!(!ok);
    assert_eq!(none["error"]["code"], json!("REF_REQUIRED"), "{none}");

    let (bad, ok, _) = run_cli(&[
        "--json",
        "attach",
        "add",
        "--invoice",
        &inv_id.to_string(),
        "--file",
        &p,
        "--store",
        "bogus",
        "--db",
        &f,
    ]);
    assert!(!ok);
    assert_eq!(bad["error"]["code"], json!("INVALID_STORE"), "{bad}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn attach_cli_dry_run_writes_nothing() {
    let (dir, f, inv_id, _) = attach_env("att11");
    let src = dir.join("a.pdf");
    std::fs::write(&src, b"x").unwrap();
    let (r, ok, out) = run_cli(&[
        "--json",
        "attach",
        "add",
        "--invoice",
        &inv_id.to_string(),
        "--file",
        src.to_str().unwrap(),
        "--dry-run",
        "--db",
        &f,
    ]);
    assert!(ok, "{out}");
    assert_eq!(r["data"]["dryRun"], json!(true), "{r}");
    let db = bukio::db::open_db(&f).unwrap();
    let n: i64 = db
        .query_row("SELECT COUNT(*) FROM attachments", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn attach_cli_file_mode_end_to_end() {
    let (dir, f, inv_id, _) = attach_env("att12");
    let doc = b"%PDF-1.4 cli file mode".to_vec();
    let src = dir.join("F2026-126.pdf");
    std::fs::write(&src, &doc).unwrap();
    let (r, ok, out) = run_cli(&[
        "--json",
        "attach",
        "add",
        "--invoice",
        &inv_id.to_string(),
        "--file",
        src.to_str().unwrap(),
        "--store",
        "file",
        "--db",
        &f,
    ]);
    assert!(ok, "{out}");
    assert_eq!(r["data"]["mode"], json!("file"), "{r}");
    let id = r["data"]["id"].as_i64().unwrap();
    let out_file = dir.join("extracted2.pdf");
    let (_, ok, out) = run_cli(&[
        "--json",
        "attach",
        "show",
        "--id",
        &id.to_string(),
        "--out",
        out_file.to_str().unwrap(),
        "--db",
        &f,
    ]);
    assert!(ok, "{out}");
    assert_eq!(std::fs::read(&out_file).unwrap(), doc);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn attach_migration_created_the_table_with_its_columns() {
    let (dir, f, _, _) = attach_env("att13");
    let db = bukio::db::open_db(&f).unwrap();
    let mut stmt = db.prepare("PRAGMA table_info('attachments')").unwrap();
    let cols: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(1))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    for c in [
        "kind",
        "ref_id",
        "file_name",
        "mime",
        "size",
        "sha256",
        "mode",
        "data",
        "path",
        "note",
        "created_by",
    ] {
        assert!(
            cols.iter().any(|x| x == c),
            "missing column {c} in {cols:?}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn attach_dir_convention_is_the_db_name_with_an_attachments_suffix() {
    let dir = temp_dir("att14");
    let file = dir.join("demo.db");
    let f = file.to_str().unwrap().to_string();
    let (_, ok, out) = run_cli(&[
        "--json",
        "init",
        "--name",
        "Test Coaching",
        "--registration-id",
        "12345678",
        "--legal-form",
        "eenmanszaak",
        "--vat",
        "off",
        "--db",
        &f,
    ]);
    assert!(ok, "{out}");
    run_cli(&[
        "--json",
        "company",
        "update",
        "--address",
        "Teststraat 1",
        "--postal-code",
        "1000 AA",
        "--city",
        "Amsterdam",
        "--db",
        &f,
    ]);
    let db = bukio::db::open_db(&f).unwrap();
    let c = bukio::contacts::create_contact(
        &db,
        "Acme BV",
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        "agent:test",
        false,
    )
    .unwrap();
    let inv = bukio::invoice::create_invoice(
        &db,
        c["id"].as_i64().unwrap(),
        "2026-08-10",
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        &[json!("Ding @ 10.00")],
        "agent:test",
        false,
    )
    .unwrap();
    let src = dir.join("a.pdf");
    std::fs::write(&src, b"x").unwrap();
    let a = bukio::attachments::add_attachment(
        &db,
        "invoice",
        inv["id"].as_i64().unwrap(),
        src.to_str().unwrap(),
        None,
        "file",
        "agent:test",
        false,
    )
    .unwrap();
    let parent = std::path::Path::new(a["path"].as_str().unwrap())
        .parent()
        .unwrap();
    assert_eq!(
        parent,
        dir.join("demo-attachments"),
        "demo.db → demo-attachments/"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn attach_file_mode_dir_is_created_next_to_a_nested_db() {
    let dir = temp_dir("att15");
    let nested = dir.join("nested");
    std::fs::create_dir_all(&nested).unwrap();
    let file = nested.join("sub.db");
    let f = file.to_str().unwrap().to_string();
    let (_, ok, out) = run_cli(&[
        "--json",
        "init",
        "--name",
        "Test Coaching",
        "--registration-id",
        "12345678",
        "--legal-form",
        "eenmanszaak",
        "--vat",
        "off",
        "--db",
        &f,
    ]);
    assert!(ok, "{out}");
    run_cli(&[
        "--json",
        "company",
        "update",
        "--address",
        "Teststraat 1",
        "--postal-code",
        "1000 AA",
        "--city",
        "Amsterdam",
        "--db",
        &f,
    ]);
    let db = bukio::db::open_db(&f).unwrap();
    let c = bukio::contacts::create_contact(
        &db,
        "Acme BV",
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        "agent:test",
        false,
    )
    .unwrap();
    let inv = bukio::invoice::create_invoice(
        &db,
        c["id"].as_i64().unwrap(),
        "2026-08-10",
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        &[json!("Ding @ 10.00")],
        "agent:test",
        false,
    )
    .unwrap();
    let src = dir.join("a.pdf");
    std::fs::write(&src, b"x").unwrap();
    let a = bukio::attachments::add_attachment(
        &db,
        "invoice",
        inv["id"].as_i64().unwrap(),
        src.to_str().unwrap(),
        None,
        "file",
        "agent:test",
        false,
    )
    .unwrap();
    let path = a["path"].as_str().unwrap();
    assert_eq!(
        std::path::Path::new(path).parent().unwrap(),
        nested.join("sub-attachments")
    );
    assert!(std::path::Path::new(path).exists());
    let _ = std::fs::remove_dir_all(&dir);
}

// ==== backup (ported from test/backup.test.js) ==============================

/// The CLI with an extra environment (HOME decides ~/.bukio/backups).
fn run_cli_env(args: &[&str], env: &[(&str, &str)]) -> (Value, bool, String) {
    let exe = env!("CARGO_BIN_EXE_bukio");
    let mut cmd = std::process::Command::new(exe);
    cmd.env("BUKIO_ACTOR", "agent:test");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.args(args).output().unwrap();
    (
        serde_json::from_slice(&out.stdout).unwrap_or(Value::Null),
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).to_string(),
    )
}

fn trial_balance_sum(file: &str) -> i64 {
    let db = bukio::db::open_db(file).unwrap();
    db.query_row(
        "SELECT COALESCE(SUM(amount_cents),0) FROM postings p JOIN journal_entries e ON e.id = p.entry_id WHERE e.state = 'posted'",
        [],
        |r| r.get(0),
    )
    .unwrap()
}

/// init + one posted entry, the JS beforeEach.
fn backup_env(tag: &str) -> (std::path::PathBuf, String) {
    let dir = temp_dir(tag);
    let file = dir.join("test.db");
    let f = file.to_str().unwrap().to_string();
    let (_, ok, out) = run_cli(&[
        "--json",
        "init",
        "--name",
        "Test Coaching",
        "--registration-id",
        "12345678",
        "--legal-form",
        "eenmanszaak",
        "--vat",
        "off",
        "--db",
        &f,
    ]);
    assert!(ok, "{out}");
    let (_, ok, out) = run_cli(&[
        "--json",
        "entry",
        "add",
        "--date",
        "2026-08-10",
        "--desc",
        "Startkapitaal",
        "--postings",
        "1100:10000.00,3000:-10000.00",
        "--post",
        "--db",
        &f,
    ]);
    assert!(ok, "{out}");
    (dir, f)
}

#[test]
fn backup_encrypt_writes_the_magic_header_and_restores_byte_identical() {
    let (dir, db_path) = backup_env("bk1");
    let enc = dir.join("enc.db.enc");
    let encs = enc.to_str().unwrap().to_string();

    let (r, ok, out) = run_cli(&[
        "--json",
        "backup",
        "--out",
        &encs,
        "--encrypt",
        "--passphrase",
        "hunter2",
        "--db",
        &db_path,
    ]);
    assert!(ok, "{out}");
    assert_eq!(r["data"]["encrypted"], json!(true), "{r}");
    assert!(enc.exists());

    // the on-disk format starts with the magic
    let head = &std::fs::read(&enc).unwrap()[..9];
    assert_eq!(head, b"BUKIOENC1", "encrypted backups start with the magic");
    assert!(bukio::backup::is_encrypted_backup(&enc));

    let restored = dir.join("restored.db");
    let rs = restored.to_str().unwrap().to_string();
    let (rr, ok, out) = run_cli(&[
        "--json",
        "restore",
        "--from",
        &encs,
        "--to",
        &rs,
        "--passphrase",
        "hunter2",
        "--db",
        &db_path,
    ]);
    assert!(ok, "{out}");
    assert_eq!(rr["data"]["encrypted"], json!(true), "{rr}");
    assert_eq!(trial_balance_sum(&db_path), trial_balance_sum(&rs));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn restore_encrypted_needs_a_passphrase_and_rejects_a_wrong_one() {
    let (dir, db_path) = backup_env("bk2");
    let enc = dir.join("enc.db.enc");
    let encs = enc.to_str().unwrap().to_string();
    run_cli(&[
        "--json",
        "backup",
        "--out",
        &encs,
        "--encrypt",
        "--passphrase",
        "hunter2",
        "--db",
        &db_path,
    ]);
    let restored = dir.join("r.db");
    let rs = restored.to_str().unwrap().to_string();

    let (no_pass, ok, _) = run_cli(&[
        "--json", "restore", "--from", &encs, "--to", &rs, "--db", &db_path,
    ]);
    assert!(!ok);
    assert_eq!(
        no_pass["error"]["code"],
        json!("BACKUP_PASSPHRASE_REQUIRED"),
        "{no_pass}"
    );
    assert!(
        !restored.exists(),
        "a refused restore must not create the file"
    );

    let (wrong, ok, _) = run_cli(&[
        "--json",
        "restore",
        "--from",
        &encs,
        "--to",
        &rs,
        "--passphrase",
        "wrong",
        "--db",
        &db_path,
    ]);
    assert!(!ok);
    assert_eq!(
        wrong["error"]["code"],
        json!("BACKUP_PASSPHRASE_WRONG"),
        "{wrong}"
    );
    assert!(!restored.exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn restore_takes_the_passphrase_from_the_environment() {
    let (dir, db_path) = backup_env("bk3");
    let enc = dir.join("enc.db.enc");
    let encs = enc.to_str().unwrap().to_string();
    run_cli(&[
        "--json",
        "backup",
        "--out",
        &encs,
        "--encrypt",
        "--passphrase",
        "envpass",
        "--db",
        &db_path,
    ]);
    let restored = dir.join("r.db");
    let rs = restored.to_str().unwrap().to_string();
    let (r, ok, out) = run_cli_env(
        &[
            "--json", "restore", "--from", &encs, "--to", &rs, "--db", &db_path,
        ],
        &[("BUKIO_BACKUP_PASSPHRASE", "envpass")],
    );
    assert!(ok, "{out}");
    assert_eq!(r["data"]["encrypted"], json!(true), "{r}");
    assert_eq!(trial_balance_sum(&db_path), trial_balance_sum(&rs));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_tampered_encrypted_backup_is_a_passphrase_wrong() {
    let (dir, db_path) = backup_env("bk4");
    let enc = dir.join("enc.db.enc");
    let encs = enc.to_str().unwrap().to_string();
    run_cli(&[
        "--json",
        "backup",
        "--out",
        &encs,
        "--encrypt",
        "--passphrase",
        "hunter2",
        "--db",
        &db_path,
    ]);
    let mut bytes = std::fs::read(&enc).unwrap();
    let last = bytes.len() - 5;
    bytes[last] ^= 0xff; // flip a ciphertext byte
    std::fs::write(&enc, &bytes).unwrap();

    let restored = dir.join("r.db");
    let rs = restored.to_str().unwrap().to_string();
    let (r, ok, _) = run_cli(&[
        "--json",
        "restore",
        "--from",
        &encs,
        "--to",
        &rs,
        "--passphrase",
        "hunter2",
        "--db",
        &db_path,
    ]);
    assert!(!ok);
    // GCM authenticates, so tampering surfaces as the wrong-passphrase code
    assert_eq!(r["error"]["code"], json!("BACKUP_PASSPHRASE_WRONG"), "{r}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn backup_encrypt_decrypt_unit_round_trip_and_wrong_key() {
    let dir = temp_dir("bk5");
    let plain = dir.join("plain.db");
    let enc = dir.join("plain.db.enc");
    std::fs::write(&plain, b"sqlite bytes 123").unwrap();

    let size = bukio::backup::encrypt_file(&plain, &enc, "pass").unwrap();
    assert!(size > 0);
    let dec = bukio::backup::decrypt_bytes(&std::fs::read(&enc).unwrap(), "pass").unwrap();
    assert_eq!(dec, b"sqlite bytes 123");
    let err = bukio::backup::decrypt_bytes(&std::fs::read(&enc).unwrap(), "nope").unwrap_err();
    assert_eq!(err.code, "BACKUP_PASSPHRASE_WRONG", "{err:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn backup_keep_prunes_the_oldest_and_a_dry_run_deletes_nothing() {
    let (dir, db_path) = backup_env("bk6");
    let home = dir.join("home");
    let backup_dir = home.join(".bukio").join("backups");
    std::fs::create_dir_all(&backup_dir).unwrap();
    for n in [
        "bukio-2026-08-01T00-00-00.db",
        "bukio-2026-08-02T00-00-00.db",
        "bukio-2026-08-03T00-00-00.db",
        "bukio-2026-08-04T00-00-00.db",
    ] {
        std::fs::write(backup_dir.join(n), "x").unwrap();
    }
    // an unrelated file must never be pruned
    std::fs::write(backup_dir.join("notes.txt"), "keep me").unwrap();
    let home_s = home.to_str().unwrap().to_string();

    let (dry, ok, out) = run_cli_env(
        &[
            "--json",
            "backup",
            "--keep",
            "2",
            "--dry-run",
            "--db",
            &db_path,
        ],
        &[("HOME", &home_s)],
    );
    assert!(ok, "{out}");
    assert_eq!(dry["data"]["pruned"].as_array().unwrap().len(), 2, "{dry}");
    assert_eq!(
        std::fs::read_dir(&backup_dir).unwrap().count(),
        5,
        "dry-run deletes nothing"
    );

    // the real run prunes AFTER the new backup exists — keep N TOTAL, not N+1
    let (real, ok, out) = run_cli_env(
        &["--json", "backup", "--keep", "2", "--db", &db_path],
        &[("HOME", &home_s)],
    );
    assert!(ok, "{out}");
    assert_eq!(
        real["data"]["pruned"].as_array().unwrap().len(),
        3,
        "{real}"
    );
    let mut remaining: Vec<String> = std::fs::read_dir(&backup_dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    remaining.sort();
    for gone in [
        "bukio-2026-08-01T00-00-00.db",
        "bukio-2026-08-02T00-00-00.db",
        "bukio-2026-08-03T00-00-00.db",
    ] {
        assert!(
            !remaining.iter().any(|f| f == gone),
            "{gone} should be pruned: {remaining:?}"
        );
    }
    assert!(
        remaining
            .iter()
            .any(|f| f == "bukio-2026-08-04T00-00-00.db"),
        "{remaining:?}"
    );
    assert_eq!(
        remaining
            .iter()
            .filter(|f| f.starts_with("bukio-") && f.ends_with(".db"))
            .count(),
        2,
        "exactly the 2 newest remain: {remaining:?}"
    );
    assert!(remaining.iter().any(|f| f == "notes.txt"), "{remaining:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn backup_keep_validation_rejects_non_integer_zero_and_out() {
    let (dir, db_path) = backup_env("bk7");
    for bad in ["abc", "0"] {
        let (r, ok, _) = run_cli(&["--json", "backup", "--keep", bad, "--db", &db_path]);
        assert!(!ok);
        assert_eq!(r["error"]["code"], json!("INVALID_KEEP"), "keep={bad}: {r}");
    }
    let x = dir.join("x.db");
    let (r, ok, _) = run_cli(&[
        "--json",
        "backup",
        "--keep",
        "2",
        "--out",
        x.to_str().unwrap(),
        "--db",
        &db_path,
    ]);
    assert!(!ok);
    assert_eq!(r["error"]["code"], json!("INVALID_KEEP"), "{r}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn backup_and_restore_plain_work_and_both_are_audited() {
    let (dir, db_path) = backup_env("bk8");
    let backup_path = dir.join("plain.db");
    let bs = backup_path.to_str().unwrap().to_string();

    let (r, ok, out) = run_cli(&["--json", "backup", "--out", &bs, "--db", &db_path]);
    assert!(ok, "{out}");
    assert_eq!(r["data"]["encrypted"], json!(false), "{r}");
    assert!(!bukio::backup::is_encrypted_backup(&backup_path));

    let restored = dir.join("restored-plain.db");
    let rs = restored.to_str().unwrap().to_string();
    let (_, ok, out) = run_cli(&[
        "--json", "restore", "--from", &bs, "--to", &rs, "--db", &db_path,
    ]);
    assert!(ok, "{out}");
    assert_eq!(trial_balance_sum(&db_path), trial_balance_sum(&rs));

    // audit rows in the SOURCE (backup) and the RESTORED (restore) DBs
    let db = bukio::db::open_db(&db_path).unwrap();
    let (actor, args): (String, String) = db
        .query_row(
            "SELECT actor, args_json FROM audit_log WHERE action = 'backup' ORDER BY id DESC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(actor, "agent:test");
    assert_eq!(
        serde_json::from_str::<Value>(&args).unwrap()["encrypted"],
        json!(false)
    );

    let db = bukio::db::open_db(&rs).unwrap();
    let (actor, args): (String, String) = db
        .query_row(
            "SELECT actor, args_json FROM audit_log WHERE action = 'restore' ORDER BY id DESC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(actor, "agent:test");
    assert_eq!(
        serde_json::from_str::<Value>(&args).unwrap()["encrypted"],
        json!(false)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn prune_backups_on_a_missing_folder_is_a_noop() {
    let dir = temp_dir("bk9");
    let nohome = dir.join("nohome");
    let saved = std::env::var("HOME").ok();
    std::env::set_var("HOME", &nohome);
    let r = bukio::backup::prune_backups(3, false).unwrap();
    match saved {
        Some(h) => std::env::set_var("HOME", h),
        None => std::env::remove_var("HOME"),
    }
    assert_eq!(r.len(), 0, "{r:?}");
    let _ = std::fs::remove_dir_all(&dir);
}
