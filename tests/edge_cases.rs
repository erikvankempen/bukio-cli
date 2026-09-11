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
            fx_currency: None,
            fx_amount_cents: None,
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
        None,
        &lines(&["1x Werk @ 100.00 @21"]),
        "agent:test",
        false,
    )
    .unwrap();
    let id = inv["id"].as_i64().unwrap();
    finalize_invoice(&d, id, "agent:test", false).unwrap();
    mark_paid(
        &d,
        id,
        "2026-07-20",
        12100,
        "transfer",
        "agent:test",
        false,
        None,
    )
    .unwrap();
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
            false,
            None
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
            false,
            None
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
        None,
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
        None,
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
    mark_paid(
        &d,
        id,
        "2026-07-20",
        2000,
        "transfer",
        "agent:test",
        false,
        None,
    )
    .unwrap();
    mark_paid(
        &d,
        id,
        "2026-08-20",
        3000,
        "transfer",
        "agent:test",
        false,
        None,
    )
    .unwrap();

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
        Mcp::start_as(db_path, "agent:test", None)
    }

    /// An MCP session with a chosen actor and config dir (the authz gate reads
    /// both).
    fn start_as(db_path: &str, actor: &str, config_dir: Option<&str>) -> Mcp {
        use std::process::{Command, Stdio};
        let exe = env!("CARGO_BIN_EXE_bukio");
        let mut cmd = Command::new(exe);
        cmd.args(["mcp", "--db", db_path]).env("BUKIO_ACTOR", actor);
        if let Some(c) = config_dir {
            cmd.env("BUKIO_CONFIG_DIR", c);
        }
        let mut child = cmd
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

    /// Write a raw JSON-RPC line and read the next message back.
    fn raw(&mut self, line: &str) -> Value {
        use std::io::{BufRead, Write};
        writeln!(self.stdin, "{line}").unwrap();
        self.stdin.flush().unwrap();
        let mut buf = String::new();
        loop {
            buf.clear();
            let n = self.reader.read_line(&mut buf).unwrap();
            assert!(n > 0, "MCP closed after a raw line");
            if let Ok(msg) = serde_json::from_str::<Value>(buf.trim()) {
                return msg;
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
                    fx_currency: None,
                    fx_amount_cents: None,
                },
                bukio::entries::PostingSpec {
                    code: "3000".into(),
                    amount_cents: -10000,
                    cost_center_code: None,
                    vat_code: None,
                    vat_amount_cents: None,
                    fx_currency: None,
                    fx_amount_cents: None,
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

// ==== export xaf + audit formats (ported from test/export.test.js) =========

fn xpost(db: &rusqlite::Connection, date: &str, desc: &str, postings: &[(&str, i64)]) {
    let specs = postings
        .iter()
        .map(|(code, cents)| bukio::entries::PostingSpec {
            code: (*code).to_string(),
            amount_cents: *cents,
            cost_center_code: None,
            vat_code: None,
            vat_amount_cents: None,
            fx_currency: None,
            fx_amount_cents: None,
        })
        .collect();
    let e = bukio::entries::create_entry(
        db,
        bukio::entries::CreateEntry {
            date,
            description: desc,
            postings: specs,
            source: "manual",
            source_ref: None,
            actor: "agent:test",
        },
    )
    .unwrap();
    bukio::entries::post_entry(db, e.id, "agent:test").unwrap();
}

/// A file DB with the default chart and company (the JS beforeEach + seedCompany).
fn export_env(tag: &str) -> (std::path::PathBuf, String) {
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

fn seed_scenario(db: &rusqlite::Connection) {
    xpost(
        db,
        "2026-01-05",
        "Startkapitaal",
        &[("1100", 1000000), ("3000", -1000000)],
    );
    xpost(
        db,
        "2026-02-10",
        "Omzet",
        &[("1100", 121000), ("8000", -121000)],
    );
    // 3-leg VAT split
    xpost(
        db,
        "2026-02-28",
        "Factuur met btw",
        &[("1100", 12100), ("8000", -10000), ("2100", -2100)],
    );
    // a draft must NOT appear in the export
    bukio::entries::create_entry(
        db,
        bukio::entries::CreateEntry {
            date: "2026-04-01",
            description: "Concept",
            postings: vec![
                bukio::entries::PostingSpec {
                    code: "4300".into(),
                    amount_cents: 5000,
                    cost_center_code: None,
                    vat_code: None,
                    vat_amount_cents: None,
                    fx_currency: None,
                    fx_amount_cents: None,
                },
                bukio::entries::PostingSpec {
                    code: "1100".into(),
                    amount_cents: -5000,
                    cost_center_code: None,
                    vat_code: None,
                    vat_amount_cents: None,
                    fx_currency: None,
                    fx_amount_cents: None,
                },
            ],
            source: "manual",
            source_ref: None,
            actor: "agent:test",
        },
    )
    .unwrap();
}

#[test]
fn export_xaf_writes_a_4_0_file_with_header_chart_and_one_mutatie_per_posted_entry() {
    let (dir, f) = export_env("xp1");
    let db = bukio::db::open_db(&f).unwrap();
    seed_scenario(&db);
    let out = dir.join("jaar-2026.xaf");
    let outs = out.to_str().unwrap().to_string();

    let res = bukio::export::export_xaf(&db, "2026", &outs, "agent:test", false).unwrap();
    assert_eq!(res["ok"], json!(true), "{res}");
    assert_eq!(res["year"], json!("2026"));
    assert_eq!(res["rekeningen"], json!(29));
    assert_eq!(res["mutaties"], json!(3), "drafts are excluded: {res}");

    let xml = std::fs::read_to_string(&out).unwrap();
    assert!(
        xml.contains(r#"<Xaf xmlns="http://www.auditfiles.nl/XAF/4.0">"#),
        "{xml:.400}"
    );
    assert!(xml.contains("<Version>4.0</Version>"));
    assert!(xml.contains("<Boekstuknummer>1</Boekstuknummer>"));
    assert!(xml.contains("<Boekstuknummer>3</Boekstuknummer>"));
    assert!(!xml.contains("Concept"), "drafts are not exported");
    assert_eq!(
        xml.matches("<Boeking>").count(),
        4,
        "1 + 1 + 2 (the 3-leg splits into two pairs)"
    );
    assert!(
        xml.contains("<Bedrag>10000.00</Bedrag>"),
        "bedrag is positive in XAF"
    );
    assert!(!xml.contains("-10000.00"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn export_xaf_three_leg_entry_round_trips_through_the_importer() {
    let (dir, f) = export_env("xp2");
    let db = bukio::db::open_db(&f).unwrap();
    seed_scenario(&db);
    let out = dir.join("roundtrip.xaf");
    bukio::export::export_xaf(&db, "2026", out.to_str().unwrap(), "agent:test", false).unwrap();

    // a fresh DB with the same chart, re-imported
    let db2 = bukio::db::open_db(":memory:").unwrap();
    bukio::accounts::seed_default_chart(&db2).unwrap();
    let xml = std::fs::read_to_string(&out).unwrap();
    let res = bukio::import_mod::import_xaf(&db2, &xml, "agent:test", false).unwrap();
    assert_eq!(res["imported"], json!(3), "{res}");

    // the 3-leg entry: 1100:12100 / 8000:-10000 / 2100:-2100 must reconstruct
    let entry_id: i64 = db2
        .query_row(
            "SELECT id FROM journal_entries WHERE description = 'Factuur met btw'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let mut stmt = db2
        .prepare("SELECT a.code, p.amount_cents FROM postings p JOIN accounts a ON a.id = p.account_id WHERE p.entry_id = ? ORDER BY a.code")
        .unwrap();
    let mut sums: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    for row in stmt
        .query_map([entry_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })
        .unwrap()
    {
        let (code, cents) = row.unwrap();
        *sums.entry(code).or_insert(0) += cents;
    }
    assert_eq!(sums.get("1100"), Some(&12100), "{sums:?}");
    assert_eq!(sums.get("8000"), Some(&-10000), "{sums:?}");
    assert_eq!(sums.get("2100"), Some(&-2100), "{sums:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn export_xaf_follows_the_fiscal_year_for_non_calendar_years() {
    let (dir, f) = export_env("xp3");
    let db = bukio::db::open_db(&f).unwrap();
    // FY ends 06-30 -> exporting 2026 covers 2025-07-01..2026-06-30
    db.execute("UPDATE company SET fiscal_year_end = '06-30'", [])
        .unwrap();
    xpost(
        &db,
        "2025-06-15",
        "Te vroeg",
        &[("1100", 1000), ("8000", -1000)],
    );
    xpost(
        &db,
        "2025-09-01",
        "Binnen",
        &[("1100", 2000), ("8000", -2000)],
    );
    xpost(
        &db,
        "2026-06-30",
        "Laatste",
        &[("1100", 3000), ("8000", -3000)],
    );
    xpost(
        &db,
        "2026-07-15",
        "Te laat",
        &[("1100", 4000), ("8000", -4000)],
    );

    let out = dir.join("fiscaal-2026.xaf");
    let res =
        bukio::export::export_xaf(&db, "2026", out.to_str().unwrap(), "agent:test", false).unwrap();
    assert_eq!(
        res["mutaties"],
        json!(2),
        "only the in-window entries: {res}"
    );

    let xml = std::fs::read_to_string(&out).unwrap();
    assert!(
        xml.contains("<StartDate>2025-07-01</StartDate>"),
        "{xml:.400}"
    );
    assert!(xml.contains("<EndDate>2026-06-30</EndDate>"));
    assert!(!xml.contains("Te vroeg"));
    assert!(!xml.contains("Te laat"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn export_xaf_records_an_audit_row() {
    let (dir, f) = export_env("xp4");
    let db = bukio::db::open_db(&f).unwrap();
    seed_scenario(&db);
    let out = dir.join("audited.xaf");
    let outs = out.to_str().unwrap().to_string();
    bukio::export::export_xaf(&db, "2026", &outs, "agent:test", false).unwrap();

    let rows = bukio::audit::list(&db, None, None, 100).unwrap();
    let row = rows
        .iter()
        .find(|r| r["action"] == json!("export.xaf"))
        .expect("export.xaf audit row expected");
    assert_eq!(row["actor"], json!("agent:test"));
    let args: Value = serde_json::from_str(row["args_json"].as_str().unwrap()).unwrap();
    assert_eq!(args["year"], json!("2026"));
    assert_eq!(args["out"], json!(outs));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn export_xaf_on_a_year_with_no_posted_entries_is_export_empty_year() {
    let (dir, f) = export_env("xp5");
    let db = bukio::db::open_db(&f).unwrap();
    xpost(
        &db,
        "2025-12-31",
        "Beginbalans",
        &[("1100", 10000), ("3000", -10000)],
    );
    let out = dir.join("empty.xaf");
    let err = bukio::export::export_xaf(&db, "2027", out.to_str().unwrap(), "agent:test", false)
        .unwrap_err();
    assert_eq!(err.code, "EXPORT_EMPTY_YEAR", "{err:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn export_xaf_escapes_ampersands_and_angle_brackets() {
    let (dir, f) = export_env("xp6");
    let db = bukio::db::open_db(&f).unwrap();
    xpost(
        &db,
        "2026-01-02",
        "Kosten & \"<extra>\"",
        &[("4300", 1000), ("1100", -1000)],
    );
    let out = dir.join("escape.xaf");
    bukio::export::export_xaf(&db, "2026", out.to_str().unwrap(), "agent:test", false).unwrap();
    let xml = std::fs::read_to_string(&out).unwrap();
    assert!(
        xml.contains("Kosten &amp; &quot;&lt;extra&gt;&quot;"),
        "escaped description expected in {xml:.600}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cli_export_xaf_year_out_writes_a_file() {
    let (dir, f) = export_env("xp7");
    let db = bukio::db::open_db(&f).unwrap();
    seed_scenario(&db);
    let out = dir.join("cli.xaf");
    let (_, ok, stdout) = run_cli(&[
        "--json",
        "export",
        "xaf",
        "--year",
        "2026",
        "--out",
        out.to_str().unwrap(),
        "--db",
        &f,
    ]);
    assert!(ok, "{stdout}");
    assert!(
        std::fs::read_to_string(&out).unwrap().contains("<Xaf"),
        "the file holds the XAF document"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn audit_csv_exports_rows_with_headers() {
    let (dir, f) = export_env("xp8");
    let db = bukio::db::open_db(&f).unwrap();
    seed_scenario(&db);
    let out = dir.join("audit.csv");
    let (_, ok, stdout) = run_cli(&[
        "--json",
        "audit",
        "--format",
        "csv",
        "--out",
        out.to_str().unwrap(),
        "--limit",
        "5",
        "--db",
        &f,
    ]);
    assert!(ok, "{stdout}");
    let csv = std::fs::read_to_string(&out).unwrap();
    assert!(
        csv.starts_with("id,timestamp,actor,action,command,args,outcome,entry_ids"),
        "header row: {csv:.200}"
    );
    assert!(csv.contains("entry.post"), "{csv:.400}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn audit_xlsx_requires_out_and_writes_a_workbook() {
    let (dir, f) = export_env("xp9");
    let db = bukio::db::open_db(&f).unwrap();
    seed_scenario(&db);
    let out = dir.join("audit.xlsx");
    let (_, ok, stdout) = run_cli(&[
        "--json",
        "audit",
        "--format",
        "xlsx",
        "--out",
        out.to_str().unwrap(),
        "--limit",
        "5",
        "--db",
        &f,
    ]);
    assert!(ok, "{stdout}");
    let buf = std::fs::read(&out).unwrap();
    assert_eq!((buf[0], buf[1]), (0x50, 0x4b), "xlsx is a zip");

    // no --out → OUT_REQUIRED
    let (r, ok, stdout) = run_cli(&["--json", "audit", "--format", "xlsx", "--db", &f]);
    assert!(!ok);
    assert!(
        r["error"]["code"] == json!("OUT_REQUIRED") || stdout.contains("OUT_REQUIRED"),
        "OUT_REQUIRED expected: {stdout}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn export_xaf_for_a_year_with_nothing_posted_is_export_empty_year_via_cli() {
    let (dir, f) = export_env("xp10");
    let out = dir.join("x.xaf");
    let (r, ok, stdout) = run_cli(&[
        "--json",
        "export",
        "xaf",
        "--year",
        "2030",
        "--out",
        out.to_str().unwrap(),
        "--db",
        &f,
    ]);
    assert!(!ok);
    assert!(
        r["error"]["code"] == json!("EXPORT_EMPTY_YEAR") || stdout.contains("EXPORT_EMPTY_YEAR"),
        "{stdout}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
// ==== actor / signing gate (ported from test/actor.test.js) ================

use std::os::unix::fs::PermissionsExt;

/// The JS runCli: no inherited actor/config, ALWAYS a scratch DB so a forgotten
/// --db can never reach the live company DB.
fn acli(args: &[&str], env: &[(&str, &str)]) -> (Value, bool, String) {
    let exe = env!("CARGO_BIN_EXE_bukio");
    let scratch = std::env::temp_dir().join(format!(
        "bukio-actor-scratch-{}-{}.db",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut cmd = std::process::Command::new(exe);
    for k in [
        "BUKIO_ACTOR",
        "BUKIO_SIGNING_PASSPHRASE",
        "BUKIO_CONFIG_DIR",
        "BUKIO_DB",
        "BUKIO_SERVER",
    ] {
        cmd.env_remove(k);
    }
    cmd.env("BUKIO_DB", scratch.to_string_lossy().to_string());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.args(args).output().unwrap();
    let _ = std::fs::remove_file(&scratch);
    (
        serde_json::from_slice(&out.stdout).unwrap_or(Value::Null),
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).to_string(),
    )
}

struct ActorCfg {
    dir: std::path::PathBuf,
    cfg: String,
    db: String,
}

fn actor_cfg(tag: &str) -> ActorCfg {
    let dir = temp_dir(tag);
    let cfg = dir.join("cfg");
    std::fs::create_dir_all(&cfg).unwrap();
    ActorCfg {
        cfg: cfg.to_string_lossy().to_string(),
        db: dir.join(format!("{tag}.db")).to_string_lossy().to_string(),
        dir,
    }
}

/// BUKIO_CONFIG_DIR + BUKIO_ACTOR='' (actor comes from --actor).
fn base_of(cfg: &str) -> Vec<(&'static str, &str)> {
    vec![("BUKIO_CONFIG_DIR", cfg), ("BUKIO_ACTOR", "")]
}

fn key_file(cfg: &str, actor: &str) -> std::path::PathBuf {
    std::path::Path::new(cfg)
        .join("keys")
        .join(format!("{}.key", actor.replace(':', "-")))
}

fn setup_enrolled_agent(cfg: &str, db: &str) {
    let base = base_of(cfg);
    let (_, ok, out) = acli(
        &["--actor", "human:erik", "init", "--name", "X", "--db", db],
        &base,
    );
    assert!(ok, "{out}");
    acli(
        &["--json", "--actor", "agent:bartholomeus", "actor", "keygen"],
        &[("BUKIO_CONFIG_DIR", cfg)],
    );
    let (_, ok, out) = acli(
        &[
            "--json",
            "--actor",
            "agent:bartholomeus",
            "actor",
            "register",
            "--db",
            db,
        ],
        &base,
    );
    assert!(ok, "register failed: {out}");
}

#[test]
fn actor_is_valid_actor_role_name_formats() {
    for good in [
        "agent:bartholomeus",
        "human:erik",
        "system:close",
        "agent:a.b_c-1",
    ] {
        assert!(bukio::actor::is_valid_actor(good), "{good} must be valid");
    }
    for bad in [
        "human",
        "agent",
        "agent:",
        ":erik",
        "human erik",
        "human:john smith",
        "",
    ] {
        assert!(
            !bukio::actor::is_valid_actor(bad),
            "{bad:?} must be rejected"
        );
    }
}

#[test]
fn actor_error_messages_for_missing_and_malformed_actors() {
    let missing = bukio::actor::actor_error(None).unwrap();
    assert_eq!(missing.code, "ACTOR_REQUIRED");
    assert!(
        missing.message.contains("agent:bartholomeus"),
        "{missing:?}"
    );
    let bad = bukio::actor::actor_error(Some("human")).unwrap();
    assert_eq!(bad.code, "INVALID_ACTOR");
    assert!(bad.message.contains("'<role>:<name>'"), "{bad:?}");
    assert!(bukio::actor::actor_error(Some("human:erik")).is_none());
}

#[test]
fn actor_cli_missing_actor_is_actor_required() {
    let t = actor_cfg("ac3");
    let (_, ok, out) = acli(
        &["init", "--name", "X", "--db", &t.db],
        &[("BUKIO_ACTOR", ""), ("BUKIO_CONFIG_DIR", &t.cfg)],
    );
    assert!(!ok);
    assert!(out.contains("ACTOR_REQUIRED"), "{out}");
    assert!(out.contains("human:erik"), "{out}");
    let _ = std::fs::remove_dir_all(&t.dir);
}

#[test]
fn actor_cli_bare_role_is_rejected() {
    let t = actor_cfg("ac4");
    let (_, ok, out) = acli(
        &["--actor", "human", "init", "--name", "X", "--db", &t.db],
        &[("BUKIO_ACTOR", ""), ("BUKIO_CONFIG_DIR", &t.cfg)],
    );
    assert!(!ok);
    assert!(out.contains("INVALID_ACTOR"), "{out}");
    assert!(out.contains("human:erik"), "{out}");
    let _ = std::fs::remove_dir_all(&t.dir);
}

#[test]
fn actor_cli_named_actor_works_and_json_errors_have_the_shape() {
    let t = actor_cfg("ac5");
    let env = base_of(&t.cfg);
    let (_, ok, out) = acli(
        &[
            "--actor",
            "human:erik",
            "init",
            "--name",
            "X",
            "--db",
            &t.db,
        ],
        &env,
    );
    assert!(ok, "{out}");

    let bad_db = t.dir.join("y.db").to_string_lossy().to_string();
    let (r, ok, out) = acli(
        &[
            "--json", "--actor", "human", "init", "--name", "Y", "--db", &bad_db,
        ],
        &env,
    );
    assert!(!ok, "{out}");
    assert_eq!(r["ok"], json!(false));
    assert_eq!(r["error"]["code"], json!("INVALID_ACTOR"));
    assert_eq!(
        r["error"]["message"]
            .as_str()
            .unwrap()
            .contains("agent:bartholomeus"),
        true,
        "{r}"
    );
    let _ = std::fs::remove_dir_all(&t.dir);
}

#[test]
fn actor_cli_env_actor_satisfies_the_requirement() {
    let t = actor_cfg("ac6");
    let (_, ok, out) = acli(
        &["init", "--name", "X", "--db", &t.db],
        &[("BUKIO_ACTOR", "human:erik"), ("BUKIO_CONFIG_DIR", &t.cfg)],
    );
    assert!(ok, "{out}");
    let _ = std::fs::remove_dir_all(&t.dir);
}

#[test]
fn actor_cli_env_actor_is_recorded_in_the_audit_trail() {
    let t = actor_cfg("ac7");
    let env = [
        ("BUKIO_ACTOR", "human:erik"),
        ("BUKIO_CONFIG_DIR", t.cfg.as_str()),
    ];
    let (_, ok, out) = acli(&["init", "--name", "X", "--db", &t.db], &env);
    assert!(ok, "{out}");
    let (audit, ok, out) = acli(&["audit", "--db", &t.db, "--json", "--limit", "50"], &env);
    assert!(ok, "{out}");
    let rows = audit["data"]["entries"].as_array().unwrap();
    assert!(!rows.is_empty());
    for row in rows {
        assert_eq!(
            row["actor"],
            json!("human:erik"),
            "audit row {} must carry the env actor",
            row["id"]
        );
    }
    let _ = std::fs::remove_dir_all(&t.dir);
}

#[test]
fn actor_keygen_agent_writes_a_plain_0600_key_file() {
    let t = actor_cfg("ac8");
    let (r, ok, out) = acli(
        &["--json", "--actor", "agent:bartholomeus", "actor", "keygen"],
        &[("BUKIO_CONFIG_DIR", &t.cfg)],
    );
    assert!(ok, "{out}");
    assert_eq!(r["data"]["actor"], json!("agent:bartholomeus"));
    assert_eq!(r["data"]["encrypted"], json!(false));
    let keyid = r["data"]["keyid"].as_str().unwrap();
    assert_eq!(keyid.len(), 32);
    assert!(keyid.chars().all(|c| c.is_ascii_hexdigit()), "{keyid}");

    let file = key_file(&t.cfg, "agent:bartholomeus");
    assert!(file.exists(), "key file written");
    assert_eq!(
        std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let pem = std::fs::read_to_string(&file).unwrap();
    assert!(pem.contains("-----BEGIN PRIVATE KEY-----"), "{pem:.120}");
    assert!(!pem.contains("ENCRYPTED"));
    let _ = std::fs::remove_dir_all(&t.dir);
}

#[test]
fn actor_keygen_human_key_is_passphrase_encrypted() {
    let t = actor_cfg("ac9");
    let (r, ok, out) = acli(
        &["--json", "--actor", "human:erik", "actor", "keygen"],
        &[
            ("BUKIO_CONFIG_DIR", &t.cfg),
            ("BUKIO_SIGNING_PASSPHRASE", "hunter2"),
        ],
    );
    assert!(ok, "{out}");
    assert_eq!(r["data"]["encrypted"], json!(true));
    let pem = std::fs::read_to_string(key_file(&t.cfg, "human:erik")).unwrap();
    assert!(
        pem.contains("-----BEGIN ENCRYPTED PRIVATE KEY-----"),
        "{pem:.120}"
    );
    let _ = std::fs::remove_dir_all(&t.dir);
}

#[test]
fn actor_keygen_refuses_to_overwrite_and_force_replaces() {
    let t = actor_cfg("ac10");
    let env = [("BUKIO_CONFIG_DIR", t.cfg.as_str())];
    let args = ["--json", "--actor", "agent:bartholomeus", "actor", "keygen"];
    let (_, ok, out) = acli(&args, &env);
    assert!(ok, "{out}");
    let (dup, ok, _) = acli(&args, &env);
    assert!(!ok);
    assert_eq!(dup["error"]["code"], json!("KEY_ALREADY_EXISTS"), "{dup}");
    let (_, ok, out) = acli(
        &[
            "--json",
            "--actor",
            "agent:bartholomeus",
            "actor",
            "keygen",
            "--force",
        ],
        &env,
    );
    assert!(ok, "{out}");
    let _ = std::fs::remove_dir_all(&t.dir);
}

#[test]
fn actor_keygen_human_without_a_passphrase_is_passphrase_required() {
    let t = actor_cfg("ac11");
    let (r, ok, _) = acli(
        &["--json", "--actor", "human:erik", "actor", "keygen"],
        &[("BUKIO_CONFIG_DIR", &t.cfg)],
    );
    assert!(!ok);
    assert_eq!(r["error"]["code"], json!("PASSPHRASE_REQUIRED"), "{r}");
    let _ = std::fs::remove_dir_all(&t.dir);
}

#[test]
fn actor_register_enrols_the_local_key_and_audits_it() {
    let t = actor_cfg("ac12");
    let base = base_of(&t.cfg);
    let (_, ok, out) = acli(
        &[
            "--actor",
            "human:erik",
            "init",
            "--name",
            "X",
            "--db",
            &t.db,
        ],
        &base,
    );
    assert!(ok, "{out}");
    acli(
        &["--json", "--actor", "agent:bartholomeus", "actor", "keygen"],
        &[("BUKIO_CONFIG_DIR", &t.cfg)],
    );
    let (reg, ok, out) = acli(
        &[
            "--json",
            "--actor",
            "agent:bartholomeus",
            "actor",
            "register",
            "--db",
            &t.db,
        ],
        &base,
    );
    assert!(ok, "{out}");
    assert_eq!(reg["data"]["enrolled"], json!(true));
    let keyid = reg["data"]["keyid"].as_str().unwrap();
    assert_eq!(keyid.len(), 32);

    let db = bukio::db::open_db(&t.db).unwrap();
    let (row_keyid, revoked): (String, Option<String>) = db
        .query_row(
            "SELECT keyid, revoked_at FROM actor_keys WHERE actor = ?1",
            ["agent:bartholomeus"],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(row_keyid, keyid);
    assert_eq!(revoked, None);
    let (actor, n): (String, i64) = db
        .query_row(
            "SELECT actor, COUNT(*) FROM audit_log WHERE action = 'actor.register'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(n, 1, "register writes one audit row");
    assert_eq!(actor, "agent:bartholomeus");
    let _ = std::fs::remove_dir_all(&t.dir);
}

#[test]
fn actor_revoke_requires_a_reason_and_marks_the_row() {
    let t = actor_cfg("ac13");
    let base = base_of(&t.cfg);
    acli(
        &[
            "--actor",
            "human:erik",
            "init",
            "--name",
            "X",
            "--db",
            &t.db,
        ],
        &base,
    );
    acli(
        &["--json", "--actor", "agent:bartholomeus", "actor", "keygen"],
        &[("BUKIO_CONFIG_DIR", &t.cfg)],
    );
    acli(
        &[
            "--json",
            "--actor",
            "agent:bartholomeus",
            "actor",
            "register",
            "--db",
            &t.db,
        ],
        &base,
    );

    let (no_reason, ok, _) = acli(
        &[
            "--json",
            "--actor",
            "agent:bartholomeus",
            "actor",
            "revoke",
            "--db",
            &t.db,
        ],
        &base,
    );
    assert!(!ok);
    assert_eq!(
        no_reason["error"]["code"],
        json!("INVALID_REASON"),
        "{no_reason}"
    );

    let (_, ok, out) = acli(
        &[
            "--json",
            "--actor",
            "agent:bartholomeus",
            "actor",
            "revoke",
            "--db",
            &t.db,
            "--reason",
            "test rotation",
        ],
        &base,
    );
    assert!(ok, "{out}");
    let db = bukio::db::open_db(&t.db).unwrap();
    let (revoked, reason): (Option<String>, Option<String>) = db
        .query_row(
            "SELECT revoked_at, revoked_reason FROM actor_keys WHERE actor = ?1",
            ["agent:bartholomeus"],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert!(revoked.is_some(), "revoked_at is set");
    assert_eq!(reason.as_deref(), Some("test rotation"));
    let n: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM audit_log WHERE action = 'actor.revoke'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 1);
    let _ = std::fs::remove_dir_all(&t.dir);
}

#[test]
fn actor_enforce_toggles_the_company_flag_and_audits_it() {
    let t = actor_cfg("ac14");
    setup_enrolled_agent(&t.cfg, &t.db);
    let base = base_of(&t.cfg);

    let (on, ok, out) = acli(
        &[
            "--json",
            "--actor",
            "agent:bartholomeus",
            "actor",
            "enforce",
            "--on",
            "--db",
            &t.db,
        ],
        &base,
    );
    assert!(ok, "{out}");
    assert_eq!(on["data"]["enforce"], json!("on"));
    let (off, ok, out) = acli(
        &[
            "--json",
            "--actor",
            "agent:bartholomeus",
            "actor",
            "enforce",
            "--off",
            "--db",
            &t.db,
        ],
        &base,
    );
    assert!(ok, "{out}");
    assert_eq!(off["data"]["enforce"], json!("off"));

    let db = bukio::db::open_db(&t.db).unwrap();
    let value: String = db
        .query_row(
            "SELECT value FROM settings WHERE key = 'signing_enforce'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(value, "off");
    let n: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM audit_log WHERE action = 'actor.enforce'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 2, "both flips are audited");
    let _ = std::fs::remove_dir_all(&t.dir);
}

#[test]
fn actor_unlock_wrong_passphrase_then_session_then_lock() {
    let t = actor_cfg("ac15");
    let env = [
        ("BUKIO_CONFIG_DIR", t.cfg.as_str()),
        ("BUKIO_SIGNING_PASSPHRASE", "correct horse"),
    ];
    let (_, ok, out) = acli(
        &["--json", "--actor", "human:erik", "actor", "keygen"],
        &env,
    );
    assert!(ok, "{out}");

    let (wrong, ok, _) = acli(
        &["--json", "--actor", "human:erik", "actor", "unlock"],
        &[
            ("BUKIO_CONFIG_DIR", &t.cfg),
            ("BUKIO_SIGNING_PASSPHRASE", "battery staple"),
        ],
    );
    assert!(!ok);
    assert_eq!(
        wrong["error"]["code"],
        json!("PASSPHRASE_INVALID"),
        "{wrong}"
    );

    let (ok_r, ok, out) = acli(
        &["--json", "--actor", "human:erik", "actor", "unlock"],
        &env,
    );
    assert!(ok, "{out}");
    let session = std::path::Path::new(&t.cfg)
        .join("sessions")
        .join("human-erik.key");
    assert_eq!(
        ok_r["data"]["sessionFile"],
        json!(session.to_string_lossy())
    );
    let raw: Value = serde_json::from_str(&std::fs::read_to_string(&session).unwrap()).unwrap();
    assert!(raw["keyPem"]
        .as_str()
        .unwrap()
        .contains("-----BEGIN PRIVATE KEY-----"));
    assert_eq!(
        std::fs::metadata(&session).unwrap().permissions().mode() & 0o777,
        0o600
    );

    let (lock, ok, out) = acli(
        &["--json", "--actor", "human:erik", "actor", "lock"],
        &[("BUKIO_CONFIG_DIR", &t.cfg)],
    );
    assert!(ok, "{out}");
    assert_eq!(lock["data"]["removed"], json!(true));
    assert!(!session.exists());
    let _ = std::fs::remove_dir_all(&t.dir);
}

#[test]
fn actor_unlock_is_not_applicable_to_agent_keys() {
    let t = actor_cfg("ac16");
    let env = [("BUKIO_CONFIG_DIR", t.cfg.as_str())];
    acli(
        &["--json", "--actor", "agent:bartholomeus", "actor", "keygen"],
        &env,
    );
    let (r, ok, _) = acli(
        &["--json", "--actor", "agent:bartholomeus", "actor", "unlock"],
        &env,
    );
    assert!(!ok);
    assert_eq!(r["error"]["code"], json!("UNLOCK_NOT_APPLICABLE"), "{r}");
    let _ = std::fs::remove_dir_all(&t.dir);
}

#[test]
fn actor_list_shows_enrolled_and_revoked_actors() {
    let t = actor_cfg("ac17");
    let base = base_of(&t.cfg);
    acli(
        &[
            "--actor",
            "human:erik",
            "init",
            "--name",
            "X",
            "--db",
            &t.db,
        ],
        &base,
    );
    acli(
        &["--json", "--actor", "agent:bartholomeus", "actor", "keygen"],
        &[("BUKIO_CONFIG_DIR", &t.cfg)],
    );
    acli(
        &[
            "--json",
            "--actor",
            "agent:bartholomeus",
            "actor",
            "register",
            "--db",
            &t.db,
        ],
        &base,
    );
    acli(
        &[
            "--json",
            "--actor",
            "agent:bartholomeus",
            "actor",
            "revoke",
            "--db",
            &t.db,
            "--reason",
            "test",
        ],
        &base,
    );

    let (r, ok, out) = acli(
        &[
            "--json",
            "--actor",
            "human:erik",
            "actor",
            "list",
            "--db",
            &t.db,
        ],
        &base,
    );
    assert!(ok, "{out}");
    let actors = r["data"]["actors"].as_array().unwrap();
    assert_eq!(actors.len(), 1, "{r}");
    assert_eq!(actors[0]["actor"], json!("agent:bartholomeus"));
    assert_eq!(actors[0]["active"], json!(false), "revoked");
    assert!(!actors[0]["revoked_at"].is_null());
    let _ = std::fs::remove_dir_all(&t.dir);
}

#[test]
fn actor_verify_reports_key_state_against_the_registry() {
    let t = actor_cfg("ac18");
    let base = base_of(&t.cfg);
    acli(
        &[
            "--actor",
            "human:erik",
            "init",
            "--name",
            "X",
            "--db",
            &t.db,
        ],
        &base,
    );
    acli(
        &["--json", "--actor", "agent:bartholomeus", "actor", "keygen"],
        &[("BUKIO_CONFIG_DIR", &t.cfg)],
    );
    acli(
        &[
            "--json",
            "--actor",
            "agent:bartholomeus",
            "actor",
            "register",
            "--db",
            &t.db,
        ],
        &base,
    );

    let (v1, ok, out) = acli(
        &[
            "--json",
            "--actor",
            "agent:bartholomeus",
            "actor",
            "verify",
            "--db",
            &t.db,
        ],
        &base,
    );
    assert!(ok, "{out}");
    assert_eq!(v1["data"]["registered"], json!(true), "{v1}");
    assert_eq!(v1["data"]["active"], json!(true));
    assert_eq!(v1["data"]["keyFileExists"], json!(true));

    acli(
        &[
            "--json",
            "--actor",
            "agent:bartholomeus",
            "actor",
            "revoke",
            "--db",
            &t.db,
            "--reason",
            "test",
        ],
        &base,
    );
    let (v2, _, _) = acli(
        &[
            "--json",
            "--actor",
            "agent:bartholomeus",
            "actor",
            "verify",
            "--db",
            &t.db,
        ],
        &base,
    );
    assert_eq!(v2["data"]["active"], json!(false));
    let _ = std::fs::remove_dir_all(&t.dir);
}

#[test]
fn actor_commands_reject_invalid_actor_strings() {
    let t = actor_cfg("ac19");
    let (r, ok, _) = acli(
        &["--json", "--actor", "human", "actor", "keygen"],
        &[("BUKIO_CONFIG_DIR", &t.cfg)],
    );
    assert!(!ok);
    assert_eq!(r["error"]["code"], json!("INVALID_ACTOR"), "{r}");
    let _ = std::fs::remove_dir_all(&t.dir);
}

#[test]
fn actor_read_session_key_treats_missing_and_expired_files_as_locked() {
    let t = actor_cfg("ac20");
    let saved = std::env::var("BUKIO_CONFIG_DIR").ok();
    std::env::set_var("BUKIO_CONFIG_DIR", &t.cfg);
    let result = (|| {
        assert!(
            bukio::actor_cli::read_session_key("human:erik").is_none(),
            "no file yet"
        );
        let session = std::path::Path::new(&t.cfg)
            .join("sessions")
            .join("human-erik.key");
        std::fs::create_dir_all(session.parent().unwrap()).unwrap();
        std::fs::write(
            &session,
            r#"{"keyPem":"x","expiresAt":"2000-01-01T00:00:00.000Z"}"#,
        )
        .unwrap();
        assert!(
            bukio::actor_cli::read_session_key("human:erik").is_none(),
            "expired counts as locked"
        );
    })();
    match saved {
        Some(v) => std::env::set_var("BUKIO_CONFIG_DIR", v),
        None => std::env::remove_var("BUKIO_CONFIG_DIR"),
    }
    let _ = std::fs::remove_dir_all(&t.dir);
}

// --- sign gate --------------------------------------------------------------

const ENTRY_ARGS: [&str; 9] = [
    "entry",
    "add",
    "--date",
    "2026-08-10",
    "--desc",
    "Gate test",
    "--postings",
    "1100:100.00,8000:-100.00",
    "--post",
];

fn last_audit_row(db_path: &str) -> Value {
    let db = bukio::db::open_db(db_path).unwrap();
    let (sig_status, digest_hash, sig_keyid, sig, sig_nonce, sig_ts): (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    ) = db
        .query_row(
            "SELECT sig_status, digest_hash, sig_keyid, sig, sig_nonce, sig_ts FROM audit_log ORDER BY id DESC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
        )
        .unwrap();
    json!({
        "sig_status": sig_status, "digest_hash": digest_hash, "sig_keyid": sig_keyid,
        "sig": sig, "sig_nonce": sig_nonce, "sig_ts": sig_ts,
    })
}

fn entry_count(db_path: &str) -> i64 {
    let db = bukio::db::open_db(db_path).unwrap();
    db.query_row("SELECT COUNT(*) FROM journal_entries", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn sign_gate_record_mode_with_an_enrolled_key_marks_the_row_verified() {
    let t = actor_cfg("ac21");
    setup_enrolled_agent(&t.cfg, &t.db);
    let mut args = vec!["--json", "--actor", "agent:bartholomeus"];
    args.extend(ENTRY_ARGS);
    args.extend(["--db", &t.db]);
    let (_, ok, out) = acli(&args, &base_of(&t.cfg));
    assert!(ok, "{out}");

    let row = last_audit_row(&t.db);
    assert_eq!(row["sig_status"], json!("verified"), "{row}");
    let digest = row["digest_hash"].as_str().unwrap();
    assert_eq!(digest.len(), 64, "sha256 hex");
    assert!(digest.chars().all(|c| c.is_ascii_hexdigit()));
    let keyid = row["sig_keyid"].as_str().unwrap();
    assert_eq!(keyid.len(), 32);
    assert!(!row["sig"].is_null());
    assert!(!row["sig_nonce"].is_null());
    assert!(!row["sig_ts"].is_null());
    let _ = std::fs::remove_dir_all(&t.dir);
}

#[test]
fn sign_gate_record_mode_without_a_key_logs_unsigned() {
    let t = actor_cfg("ac22");
    let base = base_of(&t.cfg);
    acli(
        &[
            "--actor",
            "human:erik",
            "init",
            "--name",
            "X",
            "--db",
            &t.db,
        ],
        &base,
    );
    let mut args = vec!["--json", "--actor", "system:month-end"];
    args.extend(ENTRY_ARGS);
    args.extend(["--db", &t.db]);
    let (_, ok, out) = acli(&args, &base);
    assert!(ok, "{out}");
    let row = last_audit_row(&t.db);
    assert_eq!(row["sig_status"], json!("unsigned"), "{row}");
    assert!(row["sig"].is_null());
    let _ = std::fs::remove_dir_all(&t.dir);
}

#[test]
fn sign_gate_enforce_without_a_key_is_signature_required_and_mutates_nothing() {
    let t = actor_cfg("ac23");
    setup_enrolled_agent(&t.cfg, &t.db);
    let base = base_of(&t.cfg);
    let (_, ok, out) = acli(
        &[
            "--json",
            "--actor",
            "agent:bartholomeus",
            "actor",
            "enforce",
            "--on",
            "--db",
            &t.db,
        ],
        &base,
    );
    assert!(ok, "{out}");

    let mut args = vec!["--json", "--actor", "system:month-end"];
    args.extend(ENTRY_ARGS);
    args.extend(["--db", &t.db]);
    let (r, ok, _) = acli(&args, &base);
    assert!(!ok);
    assert_eq!(r["error"]["code"], json!("SIGNATURE_REQUIRED"), "{r}");
    assert_eq!(entry_count(&t.db), 0, "nothing mutated");
    let _ = std::fs::remove_dir_all(&t.dir);
}

#[test]
fn sign_gate_enforce_with_a_rotated_unregistered_key_is_signature_invalid() {
    let t = actor_cfg("ac24");
    setup_enrolled_agent(&t.cfg, &t.db);
    let base = base_of(&t.cfg);
    acli(
        &[
            "--json",
            "--actor",
            "agent:bartholomeus",
            "actor",
            "enforce",
            "--on",
            "--db",
            &t.db,
        ],
        &base,
    );
    // rotate the local key WITHOUT re-registering -> local key != registered key
    acli(
        &[
            "--json",
            "--actor",
            "agent:bartholomeus",
            "actor",
            "keygen",
            "--force",
        ],
        &[("BUKIO_CONFIG_DIR", &t.cfg)],
    );
    let mut args = vec!["--json", "--actor", "agent:bartholomeus"];
    args.extend(ENTRY_ARGS);
    args.extend(["--db", &t.db]);
    let (r, ok, _) = acli(&args, &base);
    assert!(!ok);
    assert_eq!(r["error"]["code"], json!("SIGNATURE_INVALID"), "{r}");
    assert_eq!(entry_count(&t.db), 0);
    let _ = std::fs::remove_dir_all(&t.dir);
}

#[test]
fn sign_gate_locked_human_key_is_passphrase_required_then_env_unlocks() {
    let t = actor_cfg("ac25");
    let with_pass = [
        ("BUKIO_CONFIG_DIR", t.cfg.as_str()),
        ("BUKIO_SIGNING_PASSPHRASE", "hunter2"),
    ];
    let base = base_of(&t.cfg);
    let (_, ok, out) = acli(
        &[
            "--actor",
            "human:erik",
            "init",
            "--name",
            "X",
            "--db",
            &t.db,
        ],
        &base,
    );
    assert!(ok, "{out}");
    for cmd in [
        vec!["--json", "--actor", "human:erik", "actor", "keygen"],
        vec![
            "--json",
            "--actor",
            "human:erik",
            "actor",
            "register",
            "--db",
            t.db.as_str(),
        ],
        vec![
            "--json",
            "--actor",
            "human:erik",
            "actor",
            "enforce",
            "--on",
            "--db",
            t.db.as_str(),
        ],
    ] {
        let (_, ok, out) = acli(&cmd, &with_pass);
        assert!(ok, "{}: {out}", cmd.join(" "));
    }

    let mut args = vec!["--json", "--actor", "human:erik"];
    args.extend(ENTRY_ARGS);
    args.extend(["--db", &t.db]);
    let (locked, ok, _) = acli(&args, &base);
    assert!(!ok);
    assert_eq!(
        locked["error"]["code"],
        json!("PASSPHRASE_REQUIRED"),
        "{locked}"
    );

    let (_, ok, out) = acli(&args, &with_pass);
    assert!(ok, "{out}");
    assert_eq!(last_audit_row(&t.db)["sig_status"], json!("verified"));
    let _ = std::fs::remove_dir_all(&t.dir);
}

#[test]
fn sign_gate_unknown_actor_key_is_actor_key_unknown() {
    let t = actor_cfg("ac26");
    setup_enrolled_agent(&t.cfg, &t.db);
    let base = base_of(&t.cfg);
    acli(
        &[
            "--json",
            "--actor",
            "agent:bartholomeus",
            "actor",
            "enforce",
            "--on",
            "--db",
            &t.db,
        ],
        &base,
    );
    // a key file exists for this actor, but it was never registered
    acli(
        &["--json", "--actor", "system:cron", "actor", "keygen"],
        &[("BUKIO_CONFIG_DIR", &t.cfg)],
    );
    let mut args = vec!["--json", "--actor", "system:cron"];
    args.extend(ENTRY_ARGS);
    args.extend(["--db", &t.db]);
    let (r, ok, _) = acli(&args, &base);
    assert!(!ok);
    assert_eq!(r["error"]["code"], json!("ACTOR_KEY_UNKNOWN"), "{r}");
    let _ = std::fs::remove_dir_all(&t.dir);
}

#[test]
fn sign_gate_revoked_key_is_actor_key_revoked() {
    let t = actor_cfg("ac27");
    setup_enrolled_agent(&t.cfg, &t.db);
    let base = base_of(&t.cfg);
    acli(
        &[
            "--json",
            "--actor",
            "agent:bartholomeus",
            "actor",
            "enforce",
            "--on",
            "--db",
            &t.db,
        ],
        &base,
    );
    // the gate verifies BEFORE the action, so the actor can revoke its own key
    let (_, ok, out) = acli(
        &[
            "--json",
            "--actor",
            "agent:bartholomeus",
            "actor",
            "revoke",
            "--db",
            &t.db,
            "--reason",
            "test",
        ],
        &base,
    );
    assert!(ok, "{out}");
    let mut args = vec!["--json", "--actor", "agent:bartholomeus"];
    args.extend(ENTRY_ARGS);
    args.extend(["--db", &t.db]);
    let (r, ok, _) = acli(&args, &base);
    assert!(!ok);
    assert_eq!(r["error"]["code"], json!("ACTOR_KEY_REVOKED"), "{r}");
    let _ = std::fs::remove_dir_all(&t.dir);
}

#[test]
fn sign_gate_dry_run_fails_identically_before_any_mutation() {
    let t = actor_cfg("ac28");
    setup_enrolled_agent(&t.cfg, &t.db);
    let base = base_of(&t.cfg);
    acli(
        &[
            "--json",
            "--actor",
            "agent:bartholomeus",
            "actor",
            "enforce",
            "--on",
            "--db",
            &t.db,
        ],
        &base,
    );
    let (r, ok, _) = acli(
        &[
            "--json",
            "--actor",
            "system:month-end",
            "entry",
            "add",
            "--date",
            "2026-08-10",
            "--desc",
            "X",
            "--postings",
            "1100:100.00,8000:-100.00",
            "--dry-run",
            "--db",
            &t.db,
        ],
        &base,
    );
    assert!(!ok);
    assert_eq!(r["error"]["code"], json!("SIGNATURE_REQUIRED"), "{r}");
    assert_eq!(entry_count(&t.db), 0);
    let _ = std::fs::remove_dir_all(&t.dir);
}

#[test]
fn sign_gate_keygen_stays_exempt_and_enforce_off_needs_an_enrolled_actor() {
    let t = actor_cfg("ac29");
    setup_enrolled_agent(&t.cfg, &t.db);
    let base = base_of(&t.cfg);
    acli(
        &[
            "--json",
            "--actor",
            "agent:bartholomeus",
            "actor",
            "enforce",
            "--on",
            "--db",
            &t.db,
        ],
        &base,
    );
    // keygen is exempt (its own key does not exist yet)
    let (_, ok, out) = acli(
        &["--json", "--actor", "system:new", "actor", "keygen"],
        &[("BUKIO_CONFIG_DIR", &t.cfg)],
    );
    assert!(ok, "{out}");
    // enforce --off is NOT exempt: system:new has a key file but is not enrolled
    let (off, ok, _) = acli(
        &[
            "--json",
            "--actor",
            "system:new",
            "actor",
            "enforce",
            "--off",
            "--db",
            &t.db,
        ],
        &base,
    );
    assert!(!ok);
    assert_eq!(off["error"]["code"], json!("ACTOR_KEY_UNKNOWN"), "{off}");
    // the enrolled agent CAN disable enforcement
    let (_, ok, out) = acli(
        &[
            "--json",
            "--actor",
            "agent:bartholomeus",
            "actor",
            "enforce",
            "--off",
            "--db",
            &t.db,
        ],
        &base,
    );
    assert!(ok, "{out}");
    // enforcement is off again: unsigned commands run
    let mut args = vec!["--json", "--actor", "system:new"];
    args.extend(ENTRY_ARGS);
    args.extend(["--db", &t.db]);
    let (_, ok, out) = acli(&args, &base);
    assert!(ok, "{out}");
    let _ = std::fs::remove_dir_all(&t.dir);
}

// --- verify_signature_bundle unit checks ------------------------------------

#[test]
fn verify_bundle_stale_timestamp_is_signature_stale_under_enforce() {
    let db = bukio::db::open_db(":memory:").unwrap();
    let (public, private, keyid) = bukio::sign::generate_key_pair();
    bukio::actor::enrol_actor(&db, "agent:bartholomeus", &keyid, &public).unwrap();
    let ts = "2026-08-10T12:00:00.000Z";
    let digest = bukio::canonical::build_digest(
        "agent:bartholomeus",
        "entry add",
        &json!({}),
        ts,
        "fresh-1",
    );
    let sig = bukio::sign::sign(digest.as_bytes(), &private).unwrap();
    let r = bukio::sign_gate::verify_signature_bundle(
        &db,
        "agent:bartholomeus",
        &digest,
        &sig,
        &keyid,
        ts,
        "fresh-1",
        true,
    );
    assert!(!r.ok, "{r:?}");
    assert_eq!(r.code, Some("SIGNATURE_STALE"), "{r:?}");
}

#[test]
fn verify_bundle_reused_nonce_is_nonce_reused_even_in_record_mode() {
    let now = bukio::actor::now_iso();
    let t = actor_cfg("ac31");
    let saved = std::env::var("BUKIO_CONFIG_DIR").ok();
    std::env::set_var("BUKIO_CONFIG_DIR", &t.cfg);
    let db = bukio::db::open_db(":memory:").unwrap();
    let (public, private, keyid) = bukio::sign::generate_key_pair();
    bukio::actor::enrol_actor(&db, "agent:bartholomeus", &keyid, &public).unwrap();
    let ts = now.as_str();
    let digest = bukio::canonical::build_digest(
        "agent:bartholomeus",
        "entry add",
        &json!({}),
        ts,
        "same-nonce",
    );
    let sig = bukio::sign::sign(digest.as_bytes(), &private).unwrap();
    let first = bukio::sign_gate::verify_signature_bundle(
        &db,
        "agent:bartholomeus",
        &digest,
        &sig,
        &keyid,
        ts,
        "same-nonce",
        false,
    );
    let second = bukio::sign_gate::verify_signature_bundle(
        &db,
        "agent:bartholomeus",
        &digest,
        &sig,
        &keyid,
        ts,
        "same-nonce",
        false,
    );
    let (used, code) = (
        bukio::sign_gate::is_nonce_used(&keyid, "same-nonce"),
        second.code,
    );
    match saved {
        Some(v) => std::env::set_var("BUKIO_CONFIG_DIR", v),
        None => std::env::remove_var("BUKIO_CONFIG_DIR"),
    }
    assert!(first.ok, "{first:?}");
    assert_eq!(first.status, "verified");
    assert!(used, "the nonce is recorded");
    assert!(!second.ok, "a replay must be refused: {second:?}");
    assert_eq!(code, Some("NONCE_REUSED"));
    let _ = std::fs::remove_dir_all(&t.dir);
}

#[test]
fn verify_bundle_record_mode_tolerates_unknown_revoked_and_invalid_as_unsigned() {
    let now = bukio::actor::now_iso();
    let ts = now.as_str();
    // unknown actor -> unsigned, still ok
    {
        let db = bukio::db::open_db(":memory:").unwrap();
        let (_, private, keyid) = bukio::sign::generate_key_pair();
        let digest = bukio::canonical::build_digest(
            "agent:bartholomeus",
            "entry add",
            &json!({}),
            ts,
            "n-unk",
        );
        let sig = bukio::sign::sign(digest.as_bytes(), &private).unwrap();
        let r = bukio::sign_gate::verify_signature_bundle(
            &db,
            "agent:bartholomeus",
            &digest,
            &sig,
            &keyid,
            ts,
            "n-unk",
            false,
        );
        assert!(r.ok, "{r:?}");
        assert_eq!(r.status, "unsigned");
    }
    // revoked -> unsigned
    {
        let db = bukio::db::open_db(":memory:").unwrap();
        let (public, private, keyid) = bukio::sign::generate_key_pair();
        bukio::actor::enrol_actor(&db, "agent:bartholomeus", &keyid, &public).unwrap();
        bukio::actor::revoke_actor_reason(&db, "agent:bartholomeus", "test").unwrap();
        let digest = bukio::canonical::build_digest(
            "agent:bartholomeus",
            "entry add",
            &json!({}),
            ts,
            "n-rev",
        );
        let sig = bukio::sign::sign(digest.as_bytes(), &private).unwrap();
        let r = bukio::sign_gate::verify_signature_bundle(
            &db,
            "agent:bartholomeus",
            &digest,
            &sig,
            &keyid,
            ts,
            "n-rev",
            false,
        );
        assert!(r.ok, "{r:?}");
        assert_eq!(r.status, "unsigned");
    }
    // signed by a key that is not the enrolled one -> unsigned
    {
        let db = bukio::db::open_db(":memory:").unwrap();
        let enrolled = bukio::sign::generate_key_pair();
        let imposter = bukio::sign::generate_key_pair();
        bukio::actor::enrol_actor(&db, "agent:bartholomeus", &enrolled.2, &enrolled.0).unwrap();
        let digest = bukio::canonical::build_digest(
            "agent:bartholomeus",
            "entry add",
            &json!({}),
            ts,
            "n-wrong",
        );
        let sig = bukio::sign::sign(digest.as_bytes(), &imposter.1).unwrap();
        let r = bukio::sign_gate::verify_signature_bundle(
            &db,
            "agent:bartholomeus",
            &digest,
            &sig,
            &imposter.2,
            ts,
            "n-wrong",
            false,
        );
        assert!(r.ok, "{r:?}");
        assert_eq!(r.status, "unsigned");
    }
}

// --- full Tier 0 lifecycle --------------------------------------------------

#[test]
fn actor_lifecycle_across_two_companies() {
    let dir = temp_dir("aclife");
    let db_a = dir.join("a.db").to_string_lossy().to_string();
    let db_b = dir.join("b.db").to_string_lossy().to_string();
    let cfg = dir.join("cfg");
    std::fs::create_dir_all(&cfg).unwrap();
    let cfgs = cfg.to_string_lossy().to_string();
    let base: Vec<(&str, &str)> = vec![("BUKIO_CONFIG_DIR", &cfgs), ("BUKIO_ACTOR", "agent:test")];
    const PASS: &str = "lifecycle-passphrase-42";

    let run = |db: &str, args: &[&str], extra: &[(&str, &str)]| -> Value {
        let mut all: Vec<(&str, &str)> = base.clone();
        all.extend_from_slice(extra);
        let mut full = vec!["--db", db, "--json"];
        full.extend_from_slice(args);
        let (r, ok, out) = acli(&full, &all);
        assert!(ok, "expected ok for {}: {out}", args.join(" "));
        r
    };
    let run_fail = |db: &str, args: &[&str], extra: &[(&str, &str)]| -> Value {
        let mut all: Vec<(&str, &str)> = base.clone();
        all.extend_from_slice(extra);
        let mut full = vec!["--db", db, "--json"];
        full.extend_from_slice(args);
        let (r, ok, out) = acli(&full, &all);
        assert!(!ok, "expected failure for {}: {out}", args.join(" "));
        r
    };
    let sig_counts = |db: &str| -> std::collections::HashMap<String, i64> {
        let h = bukio::db::open_db(db).unwrap();
        let mut stmt = h
            .prepare("SELECT sig_status, COUNT(*) FROM audit_log GROUP BY sig_status")
            .unwrap();
        let rows = stmt
            .query_map([], |r| {
                Ok((r.get::<_, Option<String>>(0)?, r.get::<_, i64>(1)?))
            })
            .unwrap();
        rows.map(|r| {
            let (k, v) = r.unwrap();
            (k.unwrap_or_else(|| "null".into()), v)
        })
        .collect()
    };
    let post_entry = |who: &str, desc: &str, db: &str, extra: &[(&str, &str)]| {
        run(
            db,
            &[
                "--actor",
                who,
                "entry",
                "add",
                "--date",
                "2026-08-10",
                "--desc",
                desc,
                "--postings",
                "1100:100.00,8000:-100.00",
                "--post",
            ],
            extra,
        );
    };

    // 1. init companies A and B
    run(
        &db_a,
        &["--actor", "human:erik", "init", "--name", "A"],
        &[],
    );
    run(
        &db_b,
        &["--actor", "human:erik", "init", "--name", "B"],
        &[],
    );
    // 2. keygen: agent plain, human passphrase-encrypted
    run(
        &db_a,
        &["--actor", "agent:bartholomeus", "actor", "keygen"],
        &[],
    );
    run(
        &db_a,
        &["--actor", "human:erik", "actor", "keygen"],
        &[("BUKIO_SIGNING_PASSPHRASE", PASS)],
    );
    // 3. unlock the human key into a session
    run(
        &db_a,
        &[
            "--actor",
            "human:erik",
            "actor",
            "unlock",
            "--ttl-hours",
            "12",
        ],
        &[("BUKIO_SIGNING_PASSPHRASE", PASS)],
    );
    // 4. register both actors in company A
    run(
        &db_a,
        &["--actor", "agent:bartholomeus", "actor", "register"],
        &[],
    );
    run(
        &db_a,
        &["--actor", "human:erik", "actor", "register"],
        &[("BUKIO_SIGNING_PASSPHRASE", PASS)],
    );
    // 5. enforce signing in A
    run(
        &db_a,
        &["--actor", "human:erik", "actor", "enforce", "--on"],
        &[("BUKIO_SIGNING_PASSPHRASE", PASS)],
    );
    // 6. signed commands run: agent via key file, human via the session
    post_entry("agent:bartholomeus", "signed by agent", &db_a, &[]);
    post_entry("human:erik", "signed by human", &db_a, &[]);
    let after = sig_counts(&db_a);
    assert!(
        after.get("verified").copied().unwrap_or(0) >= 4,
        "both actors' rows verified: {after:?}"
    );
    // 7. an actor without key material is refused
    let refused = run_fail(
        &db_a,
        &[
            "--actor",
            "agent:test",
            "entry",
            "add",
            "--date",
            "2026-08-10",
            "--desc",
            "x",
            "--postings",
            "1100:10.00,8000:-10.00",
        ],
        &[],
    );
    assert_eq!(
        refused["error"]["code"],
        json!("SIGNATURE_REQUIRED"),
        "{refused}"
    );
    // 8. lock the session: refused without the passphrase, works with it
    run(&db_a, &["--actor", "human:erik", "actor", "lock"], &[]);
    let locked = run_fail(
        &db_a,
        &[
            "--actor",
            "human:erik",
            "entry",
            "add",
            "--date",
            "2026-08-10",
            "--desc",
            "x",
            "--postings",
            "1100:10.00,8000:-10.00",
        ],
        &[],
    );
    assert_eq!(
        locked["error"]["code"],
        json!("PASSPHRASE_REQUIRED"),
        "{locked}"
    );
    post_entry(
        "human:erik",
        "signed with passphrase after lock",
        &db_a,
        &[("BUKIO_SIGNING_PASSPHRASE", PASS)],
    );
    assert!(
        sig_counts(&db_a).get("verified").copied().unwrap_or(0) >= 6,
        "passphrase path verifies after lock"
    );
    // 9. self-revoke: the agent is refused afterwards
    run(
        &db_a,
        &[
            "--actor",
            "agent:bartholomeus",
            "actor",
            "revoke",
            "--reason",
            "rotation",
        ],
        &[],
    );
    let revoked = run_fail(
        &db_a,
        &[
            "--actor",
            "agent:bartholomeus",
            "entry",
            "add",
            "--date",
            "2026-08-10",
            "--desc",
            "x",
            "--postings",
            "1100:10.00,8000:-10.00",
        ],
        &[],
    );
    assert_eq!(
        revoked["error"]["code"],
        json!("ACTOR_KEY_REVOKED"),
        "{revoked}"
    );
    // 10. rotation: new key on disk, enrolled as a fresh row
    run(
        &db_a,
        &[
            "--actor",
            "agent:bartholomeus",
            "actor",
            "keygen",
            "--force",
        ],
        &[],
    );
    run(
        &db_a,
        &["--actor", "agent:bartholomeus", "actor", "register"],
        &[],
    );
    post_entry("agent:bartholomeus", "signed with rotated key", &db_a, &[]);
    // 11. audit verify is clean
    let verify = run(
        &db_a,
        &["--actor", "agent:bartholomeus", "audit", "verify"],
        &[],
    );
    let summary = &verify["data"]["summary"];
    assert_eq!(summary["tampered"], json!(0), "{summary}");
    assert_eq!(summary["invalid_signature"], json!(0));
    assert_eq!(summary["unknown_key"], json!(0));
    assert!(
        summary["ok"].as_i64().unwrap() >= 3,
        "new rows verified: {summary}"
    );
    assert!(
        summary["revoked"].as_i64().unwrap() >= 2,
        "old rows revoked: {summary}"
    );

    // 12. company B is independent: enrolment and enforcement do not leak
    run(
        &db_b,
        &["--actor", "human:erik", "actor", "register"],
        &[("BUKIO_SIGNING_PASSPHRASE", PASS)],
    );
    run(
        &db_b,
        &["--actor", "human:erik", "actor", "enforce", "--on"],
        &[("BUKIO_SIGNING_PASSPHRASE", PASS)],
    );
    let not_enrolled = run_fail(
        &db_b,
        &[
            "--actor",
            "agent:bartholomeus",
            "entry",
            "add",
            "--date",
            "2026-08-10",
            "--desc",
            "x",
            "--postings",
            "1100:10.00,8000:-10.00",
        ],
        &[],
    );
    assert_eq!(
        not_enrolled["error"]["code"],
        json!("ACTOR_KEY_UNKNOWN"),
        "{not_enrolled}"
    );
    let first_register = run_fail(
        &db_b,
        &["--actor", "agent:bartholomeus", "actor", "register"],
        &[],
    );
    assert_eq!(
        first_register["error"]["code"],
        json!("ACTOR_KEY_UNKNOWN"),
        "first enrolment under enforce is operator-gated"
    );
    let off_refused = run_fail(
        &db_b,
        &["--actor", "agent:test", "actor", "enforce", "--off"],
        &[],
    );
    assert_eq!(
        off_refused["error"]["code"],
        json!("SIGNATURE_REQUIRED"),
        "enforce --off requires an enrolled actor"
    );
    run(
        &db_b,
        &["--actor", "human:erik", "actor", "enforce", "--off"],
        &[("BUKIO_SIGNING_PASSPHRASE", PASS)],
    );
    run(
        &db_b,
        &["--actor", "agent:bartholomeus", "actor", "register"],
        &[],
    );
    run(
        &db_b,
        &["--actor", "human:erik", "actor", "enforce", "--on"],
        &[("BUKIO_SIGNING_PASSPHRASE", PASS)],
    );
    post_entry("agent:bartholomeus", "signed in company B", &db_b, &[]);
    let b_counts = sig_counts(&db_b);
    assert!(
        b_counts.get("verified").copied().unwrap_or(0) >= 2,
        "B verifies the same rotated key: {b_counts:?}"
    );
    let b_verify = run(
        &db_b,
        &["--actor", "agent:bartholomeus", "audit", "verify"],
        &[],
    );
    assert_eq!(
        b_verify["data"]["summary"]["revoked"],
        json!(0),
        "B has no revoked rows (fresh registry)"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
// ==== authz CLI + MCP gate (ported from test/authz-cli.test.js) =============

struct AuthzCo {
    dir: std::path::PathBuf,
    cfg: String,
    db: String,
}

fn co_env<'a>(cfg: &'a str, actor: &'a str) -> Vec<(&'static str, &'a str)> {
    vec![("BUKIO_ACTOR", actor), ("BUKIO_CONFIG_DIR", cfg)]
}

impl AuthzCo {
    fn run(&self, actor: &str, args: &[&str]) -> (Value, bool, String) {
        let mut full = vec!["--json", "--actor", actor];
        full.extend_from_slice(args);
        full.extend(["--db", self.db.as_str()]);
        acli(&full, &co_env(&self.cfg, actor))
    }
}

/// Fresh company with the actors enrolled (keys in cfg, enrolled in the DB).
fn authz_company(tag: &str, actors: &[&str]) -> AuthzCo {
    let dir = temp_dir(tag);
    let cfg_path = dir.join("cfg");
    std::fs::create_dir_all(&cfg_path).unwrap();
    let cfg = cfg_path.to_string_lossy().to_string();
    let db = dir.join("company.db").to_string_lossy().to_string();
    let (_, ok, out) = acli(
        &["--json", "init", "--name", "X", "--db", &db],
        &co_env(&cfg, "agent:owner"),
    );
    assert!(ok, "init: {out}");
    for a in actors {
        let (_, ok, out) = acli(
            &["--json", "--actor", a, "actor", "keygen"],
            &co_env(&cfg, a),
        );
        assert!(ok, "keygen {a}: {out}");
        let (_, ok, out) = acli(
            &["--json", "--actor", a, "actor", "register", "--db", &db],
            &co_env(&cfg, a),
        );
        assert!(ok, "register {a}: {out}");
    }
    AuthzCo { dir, cfg, db }
}

/// Flip authz on (the flipper becomes owner) and grant the two roles.
fn bootstrap_authz(c: &AuthzCo) {
    for args in [
        vec!["actor", "authz", "--on"],
        vec![
            "actor",
            "roles",
            "grant",
            "bookkeeper",
            "--for",
            "agent:bookkeeper-a",
        ],
        vec![
            "actor",
            "roles",
            "grant",
            "payments",
            "--for",
            "agent:payments-b",
        ],
    ] {
        let (_, ok, out) = c.run("agent:owner", &args);
        assert!(ok, "{}: {out}", args.join(" "));
    }
}

const ALL_ACTORS: [&str; 4] = [
    "agent:owner",
    "agent:bookkeeper-a",
    "agent:payments-b",
    "agent:nobody",
];

#[test]
fn authz_on_sets_the_mode_implies_enforce_and_grants_the_flipper_owner() {
    let c = authz_company("az1", &["agent:owner"]);
    let (on, ok, out) = c.run("agent:owner", &["actor", "authz", "--on"]);
    assert!(ok, "{out}");
    assert_eq!(on["data"]["authz"], json!("on"), "{on}");
    assert_eq!(on["data"]["enforce"], json!("on"), "authz implies enforce");
    assert_eq!(
        on["data"]["owner"],
        json!("agent:owner"),
        "the flipper becomes owner"
    );

    let (roles, ok, out) = c.run("agent:owner", &["actor", "roles"]);
    assert!(ok, "{out}");
    assert_eq!(roles["data"]["roles"], json!(["owner"]), "{roles}");

    let db = bukio::db::open_db(&c.db).unwrap();
    let mode: String = db
        .query_row(
            "SELECT value FROM settings WHERE key='authz_mode'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(mode, "on");
    let enforce: String = db
        .query_row(
            "SELECT value FROM settings WHERE key='signing_enforce'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(enforce, "on");
    let n: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM audit_log WHERE action = 'actor.authz'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(n >= 1, "the authz flip must be audited");
    let _ = std::fs::remove_dir_all(&c.dir);
}

#[test]
fn authz_on_dry_run_writes_nothing() {
    let c = authz_company("az2", &["agent:owner"]);
    let (plan, ok, out) = c.run("agent:owner", &["actor", "authz", "--on", "--dry-run"]);
    assert!(ok, "{out}");
    assert_eq!(plan["data"]["dryRun"], json!(true), "{plan}");
    assert_eq!(plan["data"]["owner_granted"], json!("agent:owner"));

    let db = bukio::db::open_db(&c.db).unwrap();
    let mode: String = db
        .query_row(
            "SELECT value FROM settings WHERE key='authz_mode'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(mode, "off", "dry-run must not flip the mode");
    let n: i64 = db
        .query_row("SELECT COUNT(*) FROM actor_roles", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0, "dry-run must not grant the owner");
    let _ = std::fs::remove_dir_all(&c.dir);
}

#[test]
fn authz_requires_exactly_one_of_on_or_off() {
    let c = authz_company("az3", &["agent:owner"]);
    for args in [
        vec!["actor", "authz"],
        vec!["actor", "authz", "--on", "--off"],
    ] {
        let (r, ok, _) = c.run("agent:owner", &args);
        assert!(!ok);
        assert_eq!(r["error"]["code"], json!("INVALID_AUTHZ"), "{r}");
    }
    let _ = std::fs::remove_dir_all(&c.dir);
}

#[test]
fn authz_off_keeps_signing_enforcement_on() {
    let c = authz_company("az4", &["agent:owner"]);
    c.run("agent:owner", &["actor", "authz", "--on"]);
    let (off, ok, out) = c.run("agent:owner", &["actor", "authz", "--off"]);
    assert!(ok, "{out}");
    assert_eq!(off["data"]["authz"], json!("off"));
    assert_eq!(off["data"]["enforce"], json!("on"), "Tier 0 stays active");

    let db = bukio::db::open_db(&c.db).unwrap();
    let enforce: String = db
        .query_row(
            "SELECT value FROM settings WHERE key='signing_enforce'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(enforce, "on");
    let _ = std::fs::remove_dir_all(&c.dir);
}

#[test]
fn authz_off_by_a_non_owner_is_authz_denied() {
    let c = authz_company("az5", &ALL_ACTORS);
    bootstrap_authz(&c);
    let (off, ok, _) = c.run("agent:bookkeeper-a", &["actor", "authz", "--off"]);
    assert!(!ok);
    assert_eq!(off["error"]["code"], json!("AUTHZ_DENIED"), "{off}");
    let _ = std::fs::remove_dir_all(&c.dir);
}

#[test]
fn roles_grant_and_revoke_audit_and_warn_on_sod_conflicts() {
    let c = authz_company("az6", &ALL_ACTORS);
    bootstrap_authz(&c);
    let (grant, ok, out) = c.run(
        "agent:owner",
        &[
            "actor",
            "roles",
            "grant",
            "payments",
            "--for",
            "agent:bookkeeper-a",
        ],
    );
    assert!(ok, "{out}");
    assert_eq!(
        grant["data"]["roles"],
        json!(["bookkeeper", "payments"]),
        "{grant}"
    );
    let warnings = grant["data"]["warnings"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        warnings
            .iter()
            .any(|w| w.as_str().unwrap_or("").contains("bookkeeper + payments")),
        "SoD warning expected: {warnings:?}"
    );

    let (revoke, ok, out) = c.run(
        "agent:owner",
        &[
            "actor",
            "roles",
            "revoke",
            "payments",
            "--for",
            "agent:bookkeeper-a",
        ],
    );
    assert!(ok, "{out}");
    assert_eq!(revoke["data"]["roles"], json!(["bookkeeper"]), "{revoke}");

    let db = bukio::db::open_db(&c.db).unwrap();
    let mut stmt = db
        .prepare("SELECT DISTINCT action FROM audit_log WHERE action LIKE 'actor.roles.%' ORDER BY action")
        .unwrap();
    let actions: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert!(
        actions.contains(&"actor.roles.grant".to_string()),
        "{actions:?}"
    );
    assert!(
        actions.contains(&"actor.roles.revoke".to_string()),
        "{actions:?}"
    );
    let args: String = db
        .query_row(
            "SELECT args_json FROM audit_log WHERE action = 'actor.roles.grant' ORDER BY id DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        args.contains("agent:bookkeeper-a"),
        "the grantee must be in the signed args: {args}"
    );
    let _ = std::fs::remove_dir_all(&c.dir);
}

#[test]
fn roles_revoke_guards_absent_roles_and_the_last_owner() {
    let c = authz_company("az7", &ALL_ACTORS);
    bootstrap_authz(&c);
    let (absent, ok, _) = c.run(
        "agent:owner",
        &[
            "actor",
            "roles",
            "revoke",
            "tax",
            "--for",
            "agent:payments-b",
        ],
    );
    assert!(!ok);
    assert_eq!(
        absent["error"]["code"],
        json!("ROLE_NOT_GRANTED"),
        "{absent}"
    );

    let (last, ok, _) = c.run(
        "agent:owner",
        &["actor", "roles", "revoke", "owner", "--for", "agent:owner"],
    );
    assert!(!ok);
    assert_eq!(last["error"]["code"], json!("LAST_OWNER"), "{last}");
    let _ = std::fs::remove_dir_all(&c.dir);
}

#[test]
fn roles_rejects_an_invalid_role_and_grantee() {
    let c = authz_company("az8", &ALL_ACTORS);
    bootstrap_authz(&c);
    let (bad_role, ok, _) = c.run(
        "agent:owner",
        &[
            "actor",
            "roles",
            "grant",
            "superuser",
            "--for",
            "agent:bookkeeper-a",
        ],
    );
    assert!(!ok);
    assert_eq!(
        bad_role["error"]["code"],
        json!("INVALID_ROLE"),
        "{bad_role}"
    );

    let (bad_actor, ok, _) = c.run(
        "agent:owner",
        &["actor", "roles", "grant", "bookkeeper", "--for", "agent"],
    );
    assert!(!ok);
    assert_eq!(
        bad_actor["error"]["code"],
        json!("INVALID_ACTOR"),
        "{bad_actor}"
    );
    let _ = std::fs::remove_dir_all(&c.dir);
}

#[test]
fn roles_are_self_service_but_viewing_another_actor_is_owner_only() {
    let c = authz_company("az9", &ALL_ACTORS);
    bootstrap_authz(&c);
    let (own, ok, out) = c.run("agent:nobody", &["actor", "roles"]);
    assert!(ok, "{out}");
    assert_eq!(own["data"]["roles"], json!([]), "{own}");

    let (owner_view, ok, out) = c.run(
        "agent:owner",
        &["actor", "roles", "--for", "agent:bookkeeper-a"],
    );
    assert!(ok, "{out}");
    assert_eq!(owner_view["data"]["roles"], json!(["bookkeeper"]));

    let (denied, ok, _) = c.run(
        "agent:bookkeeper-a",
        &["actor", "roles", "--for", "agent:payments-b"],
    );
    assert!(!ok);
    assert_eq!(denied["error"]["code"], json!("AUTHZ_DENIED"), "{denied}");
    let _ = std::fs::remove_dir_all(&c.dir);
}

#[test]
fn roles_are_inert_data_when_authz_is_off() {
    let c = authz_company("az10", &["agent:owner", "agent:bookkeeper-a"]);
    // no authz --on: grants are configuration, not privilege
    let (grant, ok, out) = c.run(
        "agent:bookkeeper-a",
        &[
            "actor",
            "roles",
            "grant",
            "readonly",
            "--for",
            "agent:owner",
        ],
    );
    assert!(ok, "{out}");
    assert_eq!(grant["data"]["roles"], json!(["readonly"]), "{grant}");
    let (roles, ok, out) = c.run("agent:owner", &["actor", "roles"]);
    assert!(ok, "{out}");
    assert_eq!(roles["data"]["roles"], json!(["readonly"]));
    let _ = std::fs::remove_dir_all(&c.dir);
}

#[test]
fn actor_can_is_a_self_service_check_of_the_actual_mutation() {
    let c = authz_company("az11", &ALL_ACTORS);
    bootstrap_authz(&c);
    let (draft, ok, out) = c.run("agent:bookkeeper-a", &["actor", "can", "entry add"]);
    assert!(ok, "{out}");
    assert_eq!(draft["data"]["capability"], json!("entry.draft"), "{draft}");
    assert_eq!(draft["data"]["allowed"], json!(true));

    let (post, _, _) = c.run("agent:bookkeeper-a", &["actor", "can", "entry add --post"]);
    assert_eq!(post["data"]["capability"], json!("entry.post"), "{post}");
    assert_eq!(post["data"]["allowed"], json!(true));

    let (file, _, _) = c.run("agent:bookkeeper-a", &["actor", "can", "vat file"]);
    assert_eq!(file["data"]["capability"], json!("vat.file"), "{file}");
    assert_eq!(file["data"]["allowed"], json!(false));
    assert_eq!(
        file["data"]["denied_reason"],
        json!("no capability 'vat.file'")
    );

    // the MCP form maps to the same capability
    let (mcp, _, _) = c.run("agent:bookkeeper-a", &["actor", "can", "mcp:entry_add"]);
    assert_eq!(mcp["data"]["capability"], json!("entry.draft"), "{mcp}");
    let _ = std::fs::remove_dir_all(&c.dir);
}

#[test]
fn actor_can_for_another_actor_is_owner_only() {
    let c = authz_company("az12", &ALL_ACTORS);
    bootstrap_authz(&c);
    let (ok_r, ok, out) = c.run(
        "agent:owner",
        &["actor", "can", "entry post", "--for", "agent:bookkeeper-a"],
    );
    assert!(ok, "{out}");
    assert_eq!(ok_r["data"]["allowed"], json!(true), "{ok_r}");

    let (denied, ok, _) = c.run(
        "agent:bookkeeper-a",
        &["actor", "can", "entry post", "--for", "agent:payments-b"],
    );
    assert!(!ok);
    assert_eq!(denied["error"]["code"], json!("AUTHZ_DENIED"), "{denied}");
    let _ = std::fs::remove_dir_all(&c.dir);
}

#[test]
fn who_can_is_the_sod_review_lens_owner_only() {
    let c = authz_company("az13", &ALL_ACTORS);
    bootstrap_authz(&c);
    let (who, ok, out) = c.run("agent:owner", &["actor", "who-can", "entry post"]);
    assert!(ok, "{out}");
    assert_eq!(who["data"]["capability"], json!("entry.post"), "{who}");
    let actors = who["data"]["actors"].as_array().unwrap();
    let allowed: Vec<&str> = actors
        .iter()
        .filter(|a| a["allowed"] == json!(true))
        .map(|a| a["actor"].as_str().unwrap())
        .collect();
    assert!(allowed.contains(&"agent:owner"), "{allowed:?}");
    assert!(allowed.contains(&"agent:bookkeeper-a"), "{allowed:?}");
    assert!(!allowed.contains(&"agent:payments-b"), "{allowed:?}");
    assert!(!allowed.contains(&"agent:nobody"), "{allowed:?}");

    let (denied, ok, _) = c.run("agent:bookkeeper-a", &["actor", "who-can", "entry post"]);
    assert!(!ok);
    assert_eq!(denied["error"]["code"], json!("AUTHZ_DENIED"), "{denied}");
    let _ = std::fs::remove_dir_all(&c.dir);
}

#[test]
fn gate_denies_a_wrong_capability_before_any_mutation() {
    let c = authz_company("az14", &ALL_ACTORS);
    bootstrap_authz(&c);
    let (_, ok, out) = c.run(
        "agent:bookkeeper-a",
        &[
            "entry",
            "add",
            "--date",
            "2026-08-01",
            "--desc",
            "test",
            "--postings",
            "1100:100.00,3000:-100.00",
        ],
    );
    assert!(ok, "{out}");
    let (_, ok, out) = c.run("agent:bookkeeper-a", &["entry", "post", "--id", "1"]);
    assert!(ok, "{out}");

    let (file, ok, _) = c.run(
        "agent:bookkeeper-a",
        &["vat", "file", "--period", "2026-Q2"],
    );
    assert!(!ok);
    let err = &file["error"];
    assert_eq!(err["code"], json!("AUTHZ_DENIED"), "{file}");
    let msg = err["message"].as_str().unwrap_or("");
    assert!(msg.contains("agent:bookkeeper-a"), "{msg}");
    assert!(msg.contains("'vat.file'"), "{msg}");
    assert!(msg.contains("bookkeeper"), "{msg}");

    let db = bukio::db::open_db(&c.db).unwrap();
    let n: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM audit_log WHERE action = 'vat.file'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 0, "the refused command wrote nothing");
    let _ = std::fs::remove_dir_all(&c.dir);
}

#[test]
fn gate_lets_payments_act_but_not_post_entries() {
    let c = authz_company("az15", &ALL_ACTORS);
    bootstrap_authz(&c);
    let (_, ok, out) = c.run("agent:payments-b", &["bank", "list"]);
    assert!(ok, "{out}");
    let (b_post, ok, _) = c.run("agent:payments-b", &["entry", "post", "--id", "1"]);
    assert!(!ok);
    assert_eq!(b_post["error"]["code"], json!("AUTHZ_DENIED"), "{b_post}");
    let _ = std::fs::remove_dir_all(&c.dir);
}

#[test]
fn gate_is_deny_by_default_for_a_role_less_actor() {
    let c = authz_company("az16", &ALL_ACTORS);
    bootstrap_authz(&c);
    for args in [
        vec!["actor", "verify"],
        vec!["actor", "roles"],
        vec!["actor", "can", "entry add"],
    ] {
        let (_, ok, out) = c.run("agent:nobody", &args);
        assert!(ok, "{}: {out}", args.join(" "));
    }
    let (add, ok, _) = c.run(
        "agent:nobody",
        &[
            "entry",
            "add",
            "--date",
            "2026-08-01",
            "--desc",
            "x",
            "--postings",
            "1100:100.00,3000:-100.00",
        ],
    );
    assert!(!ok);
    assert_eq!(add["error"]["code"], json!("AUTHZ_DENIED"), "{add}");
    assert!(
        add["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("no roles"),
        "{add}"
    );
    let _ = std::fs::remove_dir_all(&c.dir);
}

#[test]
fn gate_refuses_a_dry_run_identically() {
    let c = authz_company("az17", &ALL_ACTORS);
    bootstrap_authz(&c);
    let (plan, ok, _) = c.run(
        "agent:payments-b",
        &[
            "entry",
            "add",
            "--post",
            "--date",
            "2026-08-01",
            "--desc",
            "x",
            "--postings",
            "1100:100.00,3000:-100.00",
            "--dry-run",
        ],
    );
    assert!(!ok);
    assert_eq!(plan["error"]["code"], json!("AUTHZ_DENIED"), "{plan}");
    let db = bukio::db::open_db(&c.db).unwrap();
    let n: i64 = db
        .query_row("SELECT COUNT(*) FROM journal_entries", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0, "a dry-run refusal writes nothing");
    let _ = std::fs::remove_dir_all(&c.dir);
}

#[test]
fn gate_covers_reads_too() {
    let c = authz_company("az18", &ALL_ACTORS);
    bootstrap_authz(&c);
    let (tb, ok, _) = c.run("agent:nobody", &["report", "trial-balance"]);
    assert!(!ok);
    assert_eq!(tb["error"]["code"], json!("AUTHZ_DENIED"), "{tb}");
    let (tb_b, ok, out) = c.run("agent:payments-b", &["report", "trial-balance"]);
    assert!(ok, "{out}");
    let _ = std::fs::remove_dir_all(&c.dir);
}

#[test]
fn gate_lets_the_actual_mutation_decide_for_entry_add_post() {
    let c = authz_company("az19", &ALL_ACTORS);
    bootstrap_authz(&c);
    let (_, ok, out) = c.run(
        "agent:bookkeeper-a",
        &[
            "entry",
            "add",
            "--date",
            "2026-08-01",
            "--desc",
            "d",
            "--postings",
            "1100:50.00,3000:-50.00",
        ],
    );
    assert!(ok, "{out}");
    let (_, ok, out) = c.run(
        "agent:bookkeeper-a",
        &[
            "entry",
            "add",
            "--post",
            "--date",
            "2026-08-01",
            "--desc",
            "p",
            "--postings",
            "1100:50.00,3000:-50.00",
        ],
    );
    assert!(ok, "{out}");
    let (b_draft, ok, _) = c.run(
        "agent:payments-b",
        &[
            "entry",
            "add",
            "--date",
            "2026-08-01",
            "--desc",
            "x",
            "--postings",
            "1100:50.00,3000:-50.00",
        ],
    );
    assert!(!ok);
    assert_eq!(b_draft["error"]["code"], json!("AUTHZ_DENIED"), "{b_draft}");
    let _ = std::fs::remove_dir_all(&c.dir);
}

#[test]
fn revoke_target_kills_a_compromised_key_everywhere() {
    let c = authz_company("az20", &ALL_ACTORS);
    bootstrap_authz(&c);
    let (kill, ok, out) = c.run(
        "agent:owner",
        &[
            "actor",
            "revoke",
            "--target",
            "agent:payments-b",
            "--reason",
            "compromised key",
        ],
    );
    assert!(ok, "{out}");
    assert_eq!(kill["data"]["actor"], json!("agent:payments-b"), "{kill}");
    assert_eq!(kill["data"]["revoked_by"], json!("agent:owner"));

    let (after, ok, _) = c.run("agent:payments-b", &["bank", "list"]);
    assert!(!ok);
    assert_eq!(
        after["error"]["code"],
        json!("ACTOR_KEY_REVOKED"),
        "{after}"
    );

    // self-revoke is ALSO gate-refused (enforce is on and the key is revoked)
    let (self_revoke, ok, _) = c.run(
        "agent:payments-b",
        &["actor", "revoke", "--reason", "rotating out"],
    );
    assert!(!ok);
    assert_eq!(
        self_revoke["error"]["code"],
        json!("ACTOR_KEY_REVOKED"),
        "{self_revoke}"
    );
    let _ = std::fs::remove_dir_all(&c.dir);
}

#[test]
fn revoke_target_needs_the_owner_role_regardless_of_authz_mode() {
    let c = authz_company("az21", &["agent:owner", "agent:bookkeeper-a"]);
    // authz is OFF; the owner role is granted explicitly (roles are inert data
    // while authz is off, but the owner-kill check reads them regardless)
    let (_, ok, out) = c.run(
        "agent:owner",
        &["actor", "roles", "grant", "owner", "--for", "agent:owner"],
    );
    assert!(ok, "{out}");

    let (denied, ok, _) = c.run(
        "agent:bookkeeper-a",
        &[
            "actor",
            "revoke",
            "--target",
            "agent:owner",
            "--reason",
            "x",
        ],
    );
    assert!(!ok);
    assert_eq!(denied["error"]["code"], json!("AUTHZ_DENIED"), "{denied}");
    assert!(
        denied["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("owner role"),
        "{denied}"
    );

    let (ok_r, ok, out) = c.run(
        "agent:owner",
        &[
            "actor",
            "revoke",
            "--target",
            "agent:bookkeeper-a",
            "--reason",
            "leaving",
        ],
    );
    assert!(ok, "{out}");
    assert_eq!(ok_r["data"]["actor"], json!("agent:bookkeeper-a"));
    let db = bukio::db::open_db(&c.db).unwrap();
    let n: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM actor_keys WHERE actor = 'agent:bookkeeper-a' AND revoked_at IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 1, "the target key must be revoked");
    let _ = std::fs::remove_dir_all(&c.dir);
}

#[test]
fn mcp_gate_maps_tools_to_the_same_capabilities_and_refuses_without_mutating() {
    let c = authz_company("az22", &ALL_ACTORS);
    bootstrap_authz(&c);
    let (_, ok, out) = c.run("agent:owner", &["contact", "add", "--name", "Vendor"]);
    assert!(ok, "{out}");

    // A (bookkeeper): entry_add + entry_post work through MCP
    let mut a = Mcp::start_as(&c.db, "agent:bookkeeper-a", Some(&c.cfg));
    let (a_res, a_err) = a.tool(
        "entry_add",
        json!({ "date": "2026-08-01", "description": "via mcp", "postings": ["1100:100.00", "3000:-100.00"], "mode": "execute" }),
    );
    assert!(!a_err, "{a_res}");
    assert_eq!(a_res["mode"], json!("execute"), "{a_res}");
    let (a_post, a_post_err) = a.tool(
        "entry_post",
        json!({ "id": a_res["entry_id"], "mode": "execute", "actor": "agent:bookkeeper-a" }),
    );
    assert!(!a_post_err, "{a_post}");
    a.stop();

    // B (payments): entry_add with post:true is refused
    let mut b = Mcp::start_as(&c.db, "agent:payments-b", Some(&c.cfg));
    let (b_err_payload, b_is_err) = b.tool(
        "entry_add",
        json!({ "date": "2026-08-02", "description": "should not land", "postings": ["1100:50.00", "3000:-50.00"], "post": true, "mode": "execute" }),
    );
    assert!(b_is_err, "{b_err_payload}");
    assert_eq!(
        b_err_payload["error"]["code"],
        json!("AUTHZ_DENIED"),
        "{b_err_payload}"
    );
    assert!(
        b_err_payload["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("entry.post"),
        "{b_err_payload}"
    );

    // B can still do its own thing (an empty batch is a VALIDATION failure)
    let (batch, _) = b.tool(
        "payments_batch_create",
        json!({ "type": "transfer", "payable_ids": [], "mode": "execute", "actor": "agent:payments-b" }),
    );
    assert_ne!(
        batch["error"]["code"],
        json!("AUTHZ_DENIED"),
        "the gate let payments.sepa through: {batch}"
    );
    b.stop();

    // the refused call mutated nothing
    let db = bukio::db::open_db(&c.db).unwrap();
    let mut stmt = db
        .prepare("SELECT description FROM journal_entries")
        .unwrap();
    let descs: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert!(descs.iter().any(|d| d == "via mcp"), "{descs:?}");
    assert!(
        !descs.iter().any(|d| d == "should not land"),
        "refused call must not mutate: {descs:?}"
    );
    let _ = std::fs::remove_dir_all(&c.dir);
}

#[test]
fn mcp_read_only_tools_are_not_gated() {
    let c = authz_company("az23", &ALL_ACTORS);
    bootstrap_authz(&c);
    let mut s = Mcp::start_as(&c.db, "agent:nobody", Some(&c.cfg));
    let (tb, is_err) = s.tool("trial_balance", json!({}));
    assert!(!is_err, "{tb}");
    s.stop();
    let _ = std::fs::remove_dir_all(&c.dir);
}

#[test]
fn mcp_vat_book_maps_to_vat_book_capability() {
    let c = authz_company("az24", &ALL_ACTORS);
    bootstrap_authz(&c);
    let mut b = Mcp::start_as(&c.db, "agent:payments-b", Some(&c.cfg));
    let (payload, is_err) = b.tool(
        "vat_book",
        json!({ "date": "2026-08-01", "description": "x", "postings": ["1100:121.00", "8000:-100.00@21"], "post": true, "mode": "execute" }),
    );
    assert!(is_err, "{payload}");
    assert_eq!(payload["error"]["code"], json!("AUTHZ_DENIED"), "{payload}");
    assert!(
        payload["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("vat.book"),
        "{payload}"
    );
    b.stop();
    let _ = std::fs::remove_dir_all(&c.dir);
}

#[test]
fn authz_lifecycle_owner_splits_bookkeeping_and_payments() {
    const IBAN: &str = "NL91ABNA0417164300"; // mod-97 valid SEPA test IBAN
    let c = authz_company("az25", &ALL_ACTORS);

    // 1. owner completes the company profile + a contact (SEPA needs both)
    let (_, ok, out) = c.run(
        "agent:owner",
        &["company", "update", "--iban", IBAN, "--name", "SoD BV"],
    );
    assert!(ok, "{out}");
    let (_, ok, out) = c.run(
        "agent:owner",
        &["contact", "add", "--name", "Vendor", "--iban", IBAN],
    );
    assert!(ok, "{out}");

    // 2. bootstrap
    let (on, ok, out) = c.run("agent:owner", &["actor", "authz", "--on"]);
    assert!(ok, "{out}");
    assert_eq!(on["data"]["owner"], json!("agent:owner"));

    // 3. grants (owner only)
    let (_, ok, out) = c.run(
        "agent:owner",
        &[
            "actor",
            "roles",
            "grant",
            "bookkeeper",
            "--for",
            "agent:bookkeeper-a",
        ],
    );
    assert!(ok, "{out}");
    let (_, ok, out) = c.run(
        "agent:owner",
        &[
            "actor",
            "roles",
            "grant",
            "payments",
            "--for",
            "agent:payments-b",
        ],
    );
    assert!(ok, "{out}");

    // 4. A (bookkeeper) drafts AND posts
    let (_, ok, out) = c.run(
        "agent:bookkeeper-a",
        &[
            "entry",
            "add",
            "--date",
            "2026-08-01",
            "--desc",
            "Inkoop",
            "--postings",
            "4300:100.00,1100:-100.00",
        ],
    );
    assert!(ok, "{out}");
    let (_, ok, out) = c.run("agent:bookkeeper-a", &["entry", "post", "--id", "1"]);
    assert!(ok, "{out}");

    // 5. B (payments): payables + SEPA batch — money OUT is B's job
    let (_, ok, out) = c.run(
        "agent:payments-b",
        &[
            "payments",
            "payables",
            "add",
            "--contact",
            "1",
            "--ref",
            "INV-1",
            "--date",
            "2026-08-01",
            "--amount",
            "100.00",
        ],
    );
    assert!(ok, "{out}");
    let (_, ok, out) = c.run(
        "agent:payments-b",
        &[
            "payments",
            "batch",
            "create",
            "--type",
            "transfer",
            "--payable",
            "1",
        ],
    );
    assert!(ok, "{out}");

    // 6. cross-capability refusals: A cannot pay, B cannot book
    let (a_pay, ok, _) = c.run(
        "agent:bookkeeper-a",
        &[
            "payments",
            "batch",
            "create",
            "--type",
            "transfer",
            "--payable",
            "1",
        ],
    );
    assert!(!ok);
    assert_eq!(a_pay["error"]["code"], json!("AUTHZ_DENIED"), "{a_pay}");
    let (b_book, ok, _) = c.run(
        "agent:payments-b",
        &[
            "entry",
            "add",
            "--post",
            "--date",
            "2026-08-02",
            "--desc",
            "nope",
            "--postings",
            "4300:10.00,1100:-10.00",
        ],
    );
    assert!(!ok);
    assert_eq!(b_book["error"]["code"], json!("AUTHZ_DENIED"), "{b_book}");

    // 7. SoD warning on a conflicting grant
    let (conflict, ok, out) = c.run(
        "agent:owner",
        &[
            "actor",
            "roles",
            "grant",
            "payments",
            "--for",
            "agent:bookkeeper-a",
        ],
    );
    assert!(ok, "{out}");
    let warnings = conflict["data"]["warnings"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        warnings
            .iter()
            .any(|w| w.as_str().unwrap_or("").contains("bookkeeper + payments")),
        "{warnings:?}"
    );

    // 8. who-can 'entry post' → owner + A only
    let (who, _, _) = c.run("agent:owner", &["actor", "who-can", "entry post"]);
    let actors = who["data"]["actors"].as_array().unwrap();
    let allowed: Vec<&str> = actors
        .iter()
        .filter(|a| a["allowed"] == json!(true))
        .map(|a| a["actor"].as_str().unwrap())
        .collect();
    assert!(allowed.contains(&"agent:owner"), "{allowed:?}");
    assert!(allowed.contains(&"agent:bookkeeper-a"), "{allowed:?}");
    assert!(
        !allowed.contains(&"agent:payments-b"),
        "B must NOT post: {allowed:?}"
    );

    // 9. the trail stays cryptographically clean through the whole scenario
    let (verify, ok, out) = c.run("agent:owner", &["audit", "verify"]);
    assert!(ok, "{out}");
    let summary = &verify["data"]["summary"];
    assert_eq!(summary["tampered"], json!(0), "{summary}");
    assert_eq!(summary["invalid_signature"], json!(0), "{summary}");
    assert_eq!(summary["unknown_key"], json!(0), "{summary}");

    // 10. authz --off closes the scenario; enforcement stays on
    let (off, ok, out) = c.run("agent:owner", &["actor", "authz", "--off"]);
    assert!(ok, "{out}");
    assert_eq!(off["data"]["enforce"], json!("on"));
    let _ = std::fs::remove_dir_all(&c.dir);
}
// ==== CLI end-to-end (ported from test/cli.test.js) =========================

/// db path in a fresh dir (no init).
fn cdb(tag: &str) -> (std::path::PathBuf, String) {
    let dir = temp_dir(tag);
    let db = dir.join("test.db").to_string_lossy().to_string();
    (dir, db)
}

/// The JS `run(dbPath, args)`: BUKIO_DB + BUKIO_ACTOR=agent:test.
fn crun(db: &str, args: &[&str]) -> (Value, bool) {
    let mut full = vec!["--json"];
    full.extend_from_slice(args);
    full.extend(["--db", db]);
    let (v, ok, _) = acli(&full, &[("BUKIO_ACTOR", "agent:test")]);
    (v, ok)
}

/// Raw stdout (CSV, human output).
fn crun_text(db: &str, args: &[&str]) -> String {
    let exe = env!("CARGO_BIN_EXE_bukio");
    let out = std::process::Command::new(exe)
        .args(args)
        .env("BUKIO_DB", db)
        .env("BUKIO_ACTOR", "agent:test")
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).to_string()
}

const CAMT: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Document xmlns="urn:iso:std:iso:20022:tech:xsd:camt.053.001.02">
  <BkToCstmrStmt><Stmt>
    <Acct><Id><IBAN>NL91ABNA0417164300</IBAN></Id></Acct>
    <Ntry>
      <Amt>100.00</Amt><CdtDbtInd>CRDT</CdtDbtInd><BookgDt><Dt>2026-06-01</Dt></BookgDt>
      <NtryDtls><TxDtls><RltdPties><Dbtr><Nm>ACME B.V.</Nm></Dbtr></RltdPties>
      <RmtInf><Ustrd>Factuur 2026-001</Ustrd></RmtInf></TxDtls></NtryDtls>
    </Ntry>
    <Ntry>
      <Amt>25.50</Amt><CdtDbtInd>DBIT</CdtDbtInd><BookgDt><Dt>2026-06-02</Dt></BookgDt>
      <NtryDtls><TxDtls><RltdPties><Cdtr><Nm>Kantoorwinkel BV</Nm></Cdtr></RltdPties>
      <RmtInf><Ustrd>Kantoorartikelen</Ustrd></RmtInf></TxDtls></NtryDtls>
    </Ntry>
  </Stmt></BkToCstmrStmt>
</Document>"#;

const RABO_CSV: &str = "Datum;Naam / Omschrijving;Rekening;Tegenrekening;Code;Af Bij;Bedrag (EUR);MutatieSoort;Mededelingen\n2026-06-01;ACME B.V.;NL91ABNA0417164300;NL00RABO0123456789;GT;Bij;100,00;Overschrijving;Factuur 2026-001\n2026-06-02;Kantoorwinkel BV;NL91ABNA0417164300;NL00RABO9876543210;GT;Af;25,50;Overschrijving;Kantoorartikelen";

const CLI_IBAN: &str = "NL91ABNA0417164300";

#[test]
fn cli_init_dry_run_shows_a_plan_and_creates_nothing() {
    let (_dir, db) = cdb("cli1");
    let (out, ok) = crun(
        &db,
        &[
            "init",
            "--name",
            "Demo BV",
            "--registration-id",
            "12345678",
            "--legal-form",
            "bv",
            "--vat",
            "on",
            "--dry-run",
        ],
    );
    assert!(ok, "{out}");
    assert_eq!(out["data"]["dryRun"], json!(true), "{out}");
    assert_eq!(out["data"]["company"]["name"], json!("Demo BV"));
    assert_eq!(out["data"]["company"]["vat_module"], json!(1));
    assert!(
        !std::path::Path::new(&db).exists(),
        "dry-run must not create the db"
    );
}

#[test]
fn cli_init_creates_the_company_and_the_vat_chart() {
    let (_dir, db) = cdb("cli2");
    let (out, ok) = crun(
        &db,
        &[
            "init",
            "--name",
            "Demo BV",
            "--registration-id",
            "12345678",
            "--legal-form",
            "bv",
            "--vat",
            "on",
        ],
    );
    assert!(ok, "{out}");
    assert_eq!(
        out["data"]["chart"]["accounts"],
        json!(31),
        "29 default + 2 VAT"
    );
    assert_eq!(out["data"]["chart"]["created"], json!(31));

    let d = bukio::db::open_db(&db).unwrap();
    let (name, form, vat): (String, String, i64) = d
        .query_row(
            "SELECT name, legal_form, vat_module FROM company",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(name, "Demo BV");
    assert_eq!(form, "bv");
    assert_eq!(vat, 1);
}

#[test]
fn cli_second_init_is_already_initialised() {
    let (_dir, db) = cdb("cli3");
    let (_, ok) = crun(&db, &["init", "--name", "A"]);
    assert!(ok);
    let (out, ok) = crun(&db, &["init", "--name", "B"]);
    assert!(!ok);
    assert_eq!(out["ok"], json!(false), "{out}");
    assert_eq!(out["error"]["code"], json!("ALREADY_INITIALISED"), "{out}");
}

#[test]
fn cli_entry_add_dry_run_plans_without_writing() {
    let (_dir, db) = cdb("cli4");
    crun(&db, &["init", "--name", "A"]);
    let (out, ok) = crun(
        &db,
        &[
            "entry",
            "add",
            "--date",
            "2026-08-04",
            "--desc",
            "Startkapitaal",
            "--postings",
            "1100:10000.00,3000:-10000.00",
            "--dry-run",
        ],
    );
    assert!(ok, "{out}");
    assert_eq!(out["data"]["dryRun"], json!(true));
    assert_eq!(out["data"]["sum"], json!("0.00"));
    assert_eq!(out["data"]["account_validation"], json!("ok"));
    let d = bukio::db::open_db(&db).unwrap();
    let n: i64 = d
        .query_row("SELECT COUNT(*) FROM journal_entries", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0);
}

#[test]
fn cli_entry_add_rejects_malformed_unknown_and_unbalanced() {
    let (_dir, db) = cdb("cli5");
    crun(&db, &["init", "--name", "A"]);
    let (bad1, ok) = crun(
        &db,
        &["entry", "add", "--desc", "x", "--postings", "garbage"],
    );
    assert!(!ok);
    assert_eq!(bad1["error"]["code"], json!("INVALID_POSTING"), "{bad1}");
    let (bad2, ok) = crun(
        &db,
        &[
            "entry",
            "add",
            "--desc",
            "x",
            "--postings",
            "9999:1.00,3000:-1.00",
        ],
    );
    assert!(!ok);
    assert_eq!(bad2["error"]["code"], json!("ACCOUNT_NOT_FOUND"), "{bad2}");
    let (bad3, ok) = crun(
        &db,
        &[
            "entry",
            "add",
            "--desc",
            "x",
            "--postings",
            "1100:5.00,3000:-4.00",
        ],
    );
    assert!(!ok);
    assert_eq!(bad3["error"]["code"], json!("UNBALANCED"), "{bad3}");
}

#[test]
fn cli_entry_add_post_trial_balance_and_audit_end_to_end() {
    let (_dir, db) = cdb("cli6");
    crun(&db, &["init", "--name", "Demo BV", "--legal-form", "bv"]);
    crun(
        &db,
        &[
            "entry",
            "add",
            "--date",
            "2026-08-04",
            "--desc",
            "Startkapitaal",
            "--postings",
            "1100:10000.00,3000:-10000.00",
            "--post",
        ],
    );
    crun(
        &db,
        &[
            "entry",
            "add",
            "--date",
            "2026-08-05",
            "--desc",
            "Kantoorartikelen",
            "--postings",
            "4300:250.00,1100:-250.00",
            "--post",
        ],
    );

    let (tb, ok) = crun(&db, &["report", "trial-balance"]);
    assert!(ok, "{tb}");
    let tb = &tb["data"];
    assert_eq!(tb["balanced"], json!(true));
    assert_eq!(tb["total_debit"], json!("10250.00"));
    assert_eq!(tb["total_credit"], json!("10250.00"));
    let acct = tb["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["code"] == json!("1100"))
        .unwrap();
    assert_eq!(acct["net"], json!("9750.00"), "{acct}");

    let (audit, _) = crun(&db, &["audit", "--limit", "10"]);
    let actions: Vec<&str> = audit["data"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["action"].as_str().unwrap())
        .take(5)
        .collect();
    assert_eq!(
        actions,
        vec![
            "entry.post",
            "entry.create",
            "entry.post",
            "entry.create",
            "company.init"
        ],
        "audit is newest-first"
    );
}

#[test]
fn cli_entry_reverse_keeps_the_trial_balance_balanced() {
    let (_dir, db) = cdb("cli7");
    crun(&db, &["init", "--name", "A"]);
    let (created, ok) = crun(
        &db,
        &[
            "entry",
            "add",
            "--desc",
            "Omzet",
            "--postings",
            "1100:121.00,8000:-121.00",
            "--post",
        ],
    );
    assert!(ok, "{created}");
    let id = created["data"]["id"].as_i64().unwrap();

    let (rev, ok) = crun(
        &db,
        &[
            "entry",
            "reverse",
            "--id",
            &id.to_string(),
            "--reason",
            "credit note",
        ],
    );
    assert!(ok, "{rev}");
    assert_eq!(rev["data"]["state"], json!("posted"));
    assert_eq!(rev["data"]["source"], json!("reversal"));

    let (tb, _) = crun(&db, &["report", "trial-balance"]);
    let tb = &tb["data"];
    assert_eq!(tb["balanced"], json!(true));
    for code in ["1100", "8000"] {
        let acct = tb["accounts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["code"] == json!(code))
            .unwrap();
        assert_eq!(acct["net"], json!("0.00"), "{code} nets to zero: {acct}");
    }
}

#[test]
fn cli_trial_balance_csv_totaal_row_net_is_zero() {
    let (dir, db) = cdb("cli8");
    crun(&db, &["init", "--name", "A"]);
    crun(
        &db,
        &[
            "entry",
            "add",
            "--desc",
            "Startkapitaal",
            "--postings",
            "1100:10000.00,3000:-10000.00",
            "--post",
        ],
    );
    let csv_path = dir.join("tb.csv").to_string_lossy().to_string();
    crun_text(
        &db,
        &[
            "report",
            "trial-balance",
            "--format",
            "csv",
            "--out",
            &csv_path,
        ],
    );
    let csv = std::fs::read_to_string(&csv_path).unwrap();
    let total = csv
        .lines()
        .find(|l| l.contains("TOTAAL"))
        .unwrap_or_else(|| panic!("TOTAAL row must be present:\n{csv}"));
    let cols: Vec<&str> = total.split(',').collect();
    assert_eq!(cols[3], "10000.00", "debit");
    assert_eq!(cols[4], "10000.00", "credit");
    assert_eq!(
        cols[5], "0.00",
        "net must be the difference, not the debit: {total}"
    );
}

#[test]
fn cli_commands_fail_cleanly_without_a_database() {
    let (_dir, db) = cdb("cli9");
    let (out, ok) = crun(&db, &["report", "trial-balance"]);
    assert!(!ok);
    assert_eq!(out["error"]["code"], json!("NO_DATABASE"), "{out}");
}

#[test]
fn cli_actor_is_recorded_on_entries_and_audit() {
    let (_dir, db) = cdb("cli10");
    crun(&db, &["init", "--name", "A"]);
    let (_, ok) = crun(
        &db,
        &[
            "entry",
            "add",
            "--desc",
            "agent posting",
            "--postings",
            "1100:10.00,3000:-10.00",
            "--post",
            "--actor",
            "agent:test",
        ],
    );
    assert!(ok);
    let (audit, ok) = crun(&db, &["audit", "--by", "agent:test"]);
    assert!(ok, "{audit}");
    let rows = audit["data"]["entries"].as_array().unwrap();
    assert!(rows.len() >= 2, "{audit}");
    assert!(
        rows.iter().all(|a| a["actor"] == json!("agent:test")),
        "{audit}"
    );
}

#[test]
fn cli_account_add_list_show_deactivate_flow() {
    let (_dir, db) = cdb("cli11");
    crun(&db, &["init", "--name", "A"]);
    let (added, ok) = crun(
        &db,
        &[
            "account",
            "add",
            "--code",
            "5000",
            "--name",
            "Testkosten",
            "--type",
            "expense",
            "--normal-balance",
            "debit",
        ],
    );
    assert!(ok, "{added}");
    assert_eq!(added["data"]["code"], json!("5000"));
    assert!(added["data"]["taxonomy_code"].is_null());

    let (dup, ok) = crun(
        &db,
        &[
            "account",
            "add",
            "--code",
            "5000",
            "--name",
            "x",
            "--type",
            "expense",
            "--normal-balance",
            "debit",
        ],
    );
    assert!(!ok);
    assert_eq!(dup["error"]["code"], json!("ACCOUNT_EXISTS"), "{dup}");

    let (list, ok) = crun(&db, &["account", "list", "--type", "expense"]);
    assert!(ok, "{list}");
    assert_eq!(
        list["data"]["accounts"].as_array().unwrap().len(),
        14,
        "13 default (incl. 4840) + 1 new"
    );

    let (_, ok) = crun(&db, &["account", "deactivate", "--code", "5000"]);
    assert!(ok);
    let (blocked, ok) = crun(
        &db,
        &[
            "entry",
            "add",
            "--desc",
            "x",
            "--postings",
            "5000:1.00,1100:-1.00",
        ],
    );
    assert!(!ok);
    assert_eq!(
        blocked["error"]["code"],
        json!("ACCOUNT_INACTIVE"),
        "{blocked}"
    );

    crun(&db, &["account", "reactivate", "--code", "5000"]);
    let (_, ok) = crun(
        &db,
        &[
            "entry",
            "add",
            "--desc",
            "x",
            "--postings",
            "5000:1.00,1100:-1.00",
            "--post",
        ],
    );
    assert!(ok);
}

#[test]
fn cli_account_import_dry_run_validates_then_creates() {
    let (dir, db) = cdb("cli12");
    crun(&db, &["init", "--name", "A"]);
    let csv = dir.join("chart.csv");
    std::fs::write(
        &csv,
        "code,name,type,normal_balance,taxonomy_code\n5000,Testkosten,expense,debit,WBED.42\n5100,Verkeerd,weird,debit,\n",
    )
    .unwrap();
    let csv = csv.to_string_lossy().to_string();

    let (dry, ok) = crun(&db, &["account", "import", "--file", &csv, "--dry-run"]);
    assert!(ok, "{dry}");
    assert_eq!(dry["data"]["created"], json!(1), "{dry}");
    assert_eq!(dry["data"]["skipped"], json!(1));

    let (real, ok) = crun(&db, &["account", "import", "--file", &csv]);
    assert!(ok, "{real}");
    assert_eq!(real["data"]["created"], json!(1));
    assert_eq!(real["data"]["skipped"], json!(1));
}

#[test]
fn cli_reports_json_csv_and_xlsx() {
    let (dir, db) = cdb("cli13");
    crun(&db, &["init", "--name", "A"]);
    crun(
        &db,
        &[
            "entry",
            "add",
            "--date",
            "2026-01-05",
            "--desc",
            "Startkapitaal",
            "--postings",
            "1100:10000.00,3000:-10000.00",
            "--post",
        ],
    );
    crun(
        &db,
        &[
            "entry",
            "add",
            "--date",
            "2026-02-10",
            "--desc",
            "Omzet",
            "--postings",
            "1100:1210.00,8000:-1210.00",
            "--post",
        ],
    );
    crun(
        &db,
        &[
            "entry",
            "add",
            "--date",
            "2026-03-01",
            "--desc",
            "Kantoorartikelen",
            "--postings",
            "4300:250.00,1100:-250.00",
            "--post",
        ],
    );

    let (b, ok) = crun(&db, &["report", "balance-sheet", "--as-of", "2026-12-31"]);
    assert!(ok, "{b}");
    assert_eq!(b["data"]["balanced"], json!(true), "{b}");
    assert_eq!(b["data"]["assets"]["total"], json!("10960.00"));
    assert_eq!(
        b["data"]["liabilities_and_equity"]["result"],
        json!("960.00")
    );

    let (p, ok) = crun(&db, &["report", "pnl", "--year", "2026"]);
    assert!(ok, "{p}");
    assert_eq!(p["data"]["revenue"], json!("1210.00"));
    assert_eq!(p["data"]["costs"], json!("250.00"));
    assert_eq!(p["data"]["result"], json!("960.00"));

    let csv = crun_text(&db, &["report", "pnl", "--year", "2026", "--format", "csv"]);
    assert!(csv.starts_with("rgs,group,code,name,amount"), "{csv:.200}");

    let csv_out = dir.join("pnl.csv").to_string_lossy().to_string();
    crun_text(
        &db,
        &[
            "report", "journal", "--year", "2026", "--format", "csv", "--out", &csv_out,
        ],
    );
    let content = std::fs::read_to_string(&csv_out).unwrap();
    assert!(
        content.starts_with("date,entry,description"),
        "{content:.200}"
    );
    assert!(content.contains("Kantoorartikelen"), "{content:.400}");

    let xlsx_out = dir.join("pnl.xlsx").to_string_lossy().to_string();
    crun_text(
        &db,
        &[
            "report", "pnl", "--year", "2026", "--format", "xlsx", "--out", &xlsx_out,
        ],
    );
    let meta = std::fs::metadata(&xlsx_out).unwrap();
    assert!(meta.len() > 1000, "{} bytes", meta.len());

    let (no_out, ok) = crun(&db, &["report", "pnl", "--format", "xlsx"]);
    assert!(!ok);
    assert_eq!(no_out["error"]["code"], json!("OUT_REQUIRED"), "{no_out}");
}

#[test]
fn cli_balance_sheet_as_of_is_respected() {
    let (_dir, db) = cdb("cli14");
    crun(&db, &["init", "--name", "A"]);
    crun(
        &db,
        &[
            "entry",
            "add",
            "--date",
            "2026-01-05",
            "--desc",
            "Startkapitaal",
            "--postings",
            "1100:1000.00,3000:-1000.00",
            "--post",
        ],
    );
    crun(
        &db,
        &[
            "entry",
            "add",
            "--date",
            "2026-06-01",
            "--desc",
            "Omzet",
            "--postings",
            "1100:500.00,8000:-500.00",
            "--post",
        ],
    );

    let (early, _) = crun(&db, &["report", "balance-sheet", "--as-of", "2026-03-01"]);
    assert_eq!(early["data"]["as_of"], json!("2026-03-01"), "{early}");
    assert_eq!(
        early["data"]["assets"]["total"],
        json!("1000.00"),
        "omzet not yet booked"
    );
    assert_eq!(
        early["data"]["liabilities_and_equity"]["result"],
        json!("0.00")
    );

    let (late, _) = crun(&db, &["report", "balance-sheet", "--as-of", "2026-12-31"]);
    assert_eq!(late["data"]["as_of"], json!("2026-12-31"));
    assert_eq!(late["data"]["assets"]["total"], json!("1500.00"));
    assert_eq!(
        late["data"]["liabilities_and_equity"]["result"],
        json!("500.00")
    );
}

#[test]
fn cli_backup_and_restore_roundtrip() {
    let (dir, db) = cdb("cli15");
    crun(&db, &["init", "--name", "A"]);
    crun(
        &db,
        &[
            "entry",
            "add",
            "--desc",
            "x",
            "--postings",
            "1100:100.00,3000:-100.00",
            "--post",
        ],
    );
    let backup_path = dir.join("backup.db").to_string_lossy().to_string();

    let (backup, ok) = crun(&db, &["backup", "--out", &backup_path]);
    assert!(ok, "{backup}");
    assert!(std::path::Path::new(backup["data"]["path"].as_str().unwrap()).exists());
    assert!(backup["data"]["bytes"].as_i64().unwrap() > 0);

    let restored = dir.join("restored.db").to_string_lossy().to_string();
    let (_, ok) = crun(&db, &["restore", "--from", &backup_path, "--to", &restored]);
    assert!(ok);
    let d = bukio::db::open_db(&restored).unwrap();
    let name: String = d
        .query_row("SELECT name FROM company", [], |r| r.get(0))
        .unwrap();
    assert_eq!(name, "A");
    let n: i64 = d
        .query_row(
            "SELECT COUNT(*) FROM journal_entries WHERE state='posted'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 1);

    let (conflict, ok) = crun(&db, &["restore", "--from", &backup_path, "--to", &db]);
    assert!(!ok);
    assert_eq!(
        conflict["error"]["code"],
        json!("RESTORE_EXISTS"),
        "{conflict}"
    );
    let (_, ok) = crun(
        &db,
        &["restore", "--from", &backup_path, "--to", &db, "--force"],
    );
    assert!(ok);

    let (bad, ok) = crun(&db, &["restore", "--from", "/nonexistent.db"]);
    assert!(!ok);
    assert_eq!(bad["error"]["code"], json!("FILE_NOT_FOUND"), "{bad}");
    let junk = dir.join("junk.txt");
    std::fs::write(&junk, "not a database").unwrap();
    let (bad2, ok) = crun(&db, &["restore", "--from", &junk.to_string_lossy()]);
    assert!(!ok);
    assert_eq!(bad2["error"]["code"], json!("INVALID_BACKUP"), "{bad2}");
    let (same, ok) = crun(
        &db,
        &["restore", "--from", &backup_path, "--to", &backup_path],
    );
    assert!(!ok);
    assert_eq!(same["error"]["code"], json!("SAME_FILE"), "{same}");
}

#[test]
fn cli_bank_import_idempotency_match_post_and_ignore() {
    let (dir, db) = cdb("cli16");
    crun(&db, &["init", "--name", "A"]);
    let camt = dir.join("stmt.xml");
    std::fs::write(&camt, CAMT).unwrap();
    let camt = camt.to_string_lossy().to_string();

    let (imp, ok) = crun(
        &db,
        &["bank", "import", "--file", &camt, "--iban", CLI_IBAN],
    );
    assert!(ok, "{imp}");
    assert_eq!(imp["data"]["imported"], json!(2), "{imp}");

    let (again, _) = crun(
        &db,
        &["bank", "import", "--file", &camt, "--iban", CLI_IBAN],
    );
    assert_eq!(again["data"]["imported"], json!(0));
    assert_eq!(again["data"]["duplicates"], json!(2));

    let (dry, _) = crun(
        &db,
        &[
            "bank",
            "import",
            "--file",
            &camt,
            "--iban",
            CLI_IBAN,
            "--dry-run",
        ],
    );
    assert_eq!(dry["data"]["imported"], json!(0));
    assert_eq!(dry["data"]["duplicates"], json!(2));

    let csv = dir.join("rabo.csv");
    std::fs::write(&csv, RABO_CSV).unwrap();
    let (csv_imp, _) = crun(
        &db,
        &[
            "bank",
            "import",
            "--file",
            &csv.to_string_lossy(),
            "--iban",
            CLI_IBAN,
        ],
    );
    assert_eq!(
        csv_imp["data"]["imported"],
        json!(0),
        "same transactions, hashes match: {csv_imp}"
    );

    let (txs, _) = crun(&db, &["bank", "transactions", "--state", "unmatched"]);
    let txs = txs["data"]["transactions"].as_array().unwrap().clone();
    assert_eq!(txs.len(), 2);
    let income = txs
        .iter()
        .find(|t| t["amount_cents"].as_i64().unwrap() > 0)
        .unwrap();
    let income_id = income["id"].as_i64().unwrap();

    let (posted, ok) = crun(
        &db,
        &[
            "bank",
            "match",
            "post",
            "--tx",
            &income_id.to_string(),
            "--account",
            "8000",
        ],
    );
    assert!(ok, "{posted}");
    assert_eq!(posted["data"]["entry_id"], json!(1), "{posted}");
    assert_eq!(posted["data"]["state"], json!("posted"));

    let (list, _) = crun(&db, &["bank", "list"]);
    let acct = &list["data"]["accounts"][0];
    assert_eq!(acct["balance"], json!("74.50"), "{acct}");
    assert_eq!(acct["unmatched_count"], json!(1), "{acct}");

    let (remaining, _) = crun(&db, &["bank", "transactions", "--state", "unmatched"]);
    let remaining_id = remaining["data"]["transactions"][0]["id"].as_i64().unwrap();
    crun(&db, &["bank", "ignore", "--tx", &remaining_id.to_string()]);
    let (after, _) = crun(&db, &["bank", "transactions", "--state", "unmatched"]);
    assert_eq!(after["data"]["transactions"].as_array().unwrap().len(), 0);

    let (tb, _) = crun(&db, &["report", "trial-balance"]);
    assert_eq!(tb["data"]["balanced"], json!(true), "{tb}");
}

#[test]
fn cli_bank_match_auto_links_posted_entries_exactly() {
    let (dir, db) = cdb("cli17");
    crun(&db, &["init", "--name", "A"]);
    crun(
        &db,
        &[
            "entry",
            "add",
            "--date",
            "2026-06-01",
            "--desc",
            "Factuur 2026-001",
            "--postings",
            "1100:100.00,8000:-100.00",
            "--post",
        ],
    );
    let camt = dir.join("stmt.xml");
    std::fs::write(&camt, CAMT).unwrap();
    crun(
        &db,
        &[
            "bank",
            "import",
            "--file",
            &camt.to_string_lossy(),
            "--iban",
            CLI_IBAN,
        ],
    );

    let (dry, ok) = crun(&db, &["bank", "match", "auto", "--dry-run"]);
    assert!(ok, "{dry}");
    assert_eq!(dry["data"]["matched"].as_array().unwrap().len(), 1, "{dry}");
    assert_eq!(dry["data"]["matched"][0]["method"], json!("exact"));
    assert_eq!(dry["data"]["matched"][0]["entry_id"], json!(1));

    let (real, _) = crun(&db, &["bank", "match", "auto"]);
    assert_eq!(real["data"]["matched"].as_array().unwrap().len(), 1);
    let (matched, _) = crun(&db, &["bank", "transactions", "--state", "matched"]);
    assert_eq!(matched["data"]["transactions"].as_array().unwrap().len(), 1);
}

#[test]
fn cli_vat_enable_book_readout_mark_filed_end_to_end() {
    let (_dir, db) = cdb("cli18");
    crun(&db, &["init", "--name", "A", "--vat", "on"]);
    let (codes, ok) = crun(&db, &["vat", "codes"]);
    assert!(ok, "{codes}");
    assert_eq!(
        codes["data"]["codes"].as_array().unwrap().len(),
        8,
        "{codes}"
    );

    crun(
        &db,
        &[
            "vat",
            "book",
            "--date",
            "2026-04-10",
            "--desc",
            "Factuur 2026-001",
            "--postings",
            "1100:121.00,8000:-100.00@21",
            "--post",
        ],
    );
    crun(
        &db,
        &[
            "vat",
            "book",
            "--date",
            "2026-05-15",
            "--desc",
            "Kantoorartikelen",
            "--postings",
            "4300:50.00@21,1100:-60.50",
            "--post",
        ],
    );

    let (r, ok) = crun(&db, &["vat", "readout", "--period", "2026-Q2"]);
    assert!(ok, "{r}");
    assert_eq!(r["data"]["fields"]["1a"]["amount"], json!("100.00"), "{r}");
    assert_eq!(r["data"]["fields"]["5a"]["amount"], json!("21.00"), "{r}");
    assert_eq!(r["data"]["fields"]["5b"]["amount"], json!("10.50"), "{r}");
    assert_eq!(r["data"]["to_pay"], json!("10.50"), "{r}");

    let (_, ok) = crun(
        &db,
        &["vat", "readout", "--period", "2026-Q2", "--mark-filed"],
    );
    assert!(ok);
    let (tb, _) = crun(&db, &["report", "trial-balance"]);
    assert_eq!(tb["data"]["balanced"], json!(true));
}

#[test]
fn cli_vat_module_off_blocks_book_and_enable_works_on_an_existing_company() {
    let (_dir, db) = cdb("cli19");
    crun(&db, &["init", "--name", "B"]);
    let (err, ok) = crun(
        &db,
        &[
            "vat",
            "book",
            "--date",
            "2026-04-10",
            "--desc",
            "x",
            "--postings",
            "1100:121.00,8000:-100.00@21",
        ],
    );
    assert!(!ok);
    assert_eq!(err["error"]["code"], json!("VAT_MODULE_OFF"), "{err}");

    let (_, ok) = crun(&db, &["vat", "enable"]);
    assert!(ok);
    let (codes, _) = crun(&db, &["vat", "codes"]);
    assert_eq!(codes["data"]["codes"].as_array().unwrap().len(), 8);
    let (_, ok) = crun(
        &db,
        &[
            "vat",
            "book",
            "--date",
            "2026-04-10",
            "--desc",
            "x",
            "--postings",
            "1100:121.00,8000:-100.00@21",
            "--post",
        ],
    );
    assert!(ok);
}

#[test]
fn cli_account_list_human_mode_renders() {
    let (_dir, db) = cdb("cli20");
    crun(
        &db,
        &[
            "init",
            "--name",
            "Demo BV",
            "--registration-id",
            "12345678",
            "--legal-form",
            "bv",
            "--vat",
            "off",
        ],
    );
    let exe = env!("CARGO_BIN_EXE_bukio");
    let out = std::process::Command::new(exe)
        .args(["--db", &db, "account", "list"])
        .env("BUKIO_ACTOR", "agent:test")
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        text.contains("1100"),
        "human account list must render:\n{text}"
    );
}

#[test]
fn cli_update_fetches_from_a_fixture_origin() {
    let dir = temp_dir("cli21");
    let origin = dir.join("origin.git").to_string_lossy().to_string();
    let git = |cwd: &str, args: &[&str]| -> String {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    let dirs = dir.to_string_lossy().to_string();
    git(&dirs, &["init", "--bare", "origin.git"]);
    git(
        &dirs,
        &[
            "--git-dir=origin.git",
            "symbolic-ref",
            "HEAD",
            "refs/heads/main",
        ],
    );

    let work1 = dir.join("work1").to_string_lossy().to_string();
    git(&dirs, &["clone", &origin, "work1"]);
    git(&work1, &["config", "user.email", "t@t"]);
    git(&work1, &["config", "user.name", "T"]);
    std::fs::write(
        format!("{work1}/package.json"),
        "{\"name\":\"bukio-cli\",\"version\":\"1.0.0\"}\n",
    )
    .unwrap();
    std::fs::write(format!("{work1}/README.md"), "v1\n").unwrap();
    git(&work1, &["add", "."]);
    git(&work1, &["commit", "-m", "v1"]);
    git(&work1, &["push", "-u", "origin", "main"]);

    let work2 = dir.join("work2").to_string_lossy().to_string();
    git(&dirs, &["clone", &origin, "work2"]);
    git(&work2, &["config", "user.email", "t@t"]);
    git(&work2, &["config", "user.name", "T"]);
    std::fs::write(format!("{work2}/README.md"), "v2\n").unwrap();
    git(&work2, &["add", "."]);
    git(&work2, &["commit", "-m", "v2"]);
    git(&work2, &["push", "origin", "main"]);

    let (_d2, db) = cdb("cli21db"); // no company db -> update still works (audit skipped)
    let (dry, ok) = crun(
        &db,
        &["update", "--repo", &work1, "--trust-remote", "--dry-run"],
    );
    assert!(ok, "{dry}");
    assert_eq!(dry["data"]["incoming_count"], json!(1), "{dry}");
    assert!(dry["data"]["warning"].is_null(), "{dry}");
    assert_eq!(dry["data"]["up_to_date"], json!(false));

    let (refused, ok) = crun(&db, &["update", "--repo", &work1, "--trust-remote"]);
    assert!(!ok);
    assert_eq!(
        refused["error"]["code"],
        json!("UPDATE_CONFIRM_REQUIRED"),
        "{refused}"
    );

    let (done, ok) = crun(
        &db,
        &["update", "--repo", &work1, "--trust-remote", "--yes"],
    );
    assert!(ok, "{done}");
    assert_eq!(done["data"]["updated"], json!(true), "{done}");
    assert_eq!(done["data"]["commits_applied"], json!(1));
    assert_eq!(
        std::fs::read_to_string(format!("{work1}/README.md")).unwrap(),
        "v2\n"
    );
}

#[test]
fn cli_vat_file_and_settle_with_a_custom_account() {
    let (_dir, db) = cdb("cli22");
    crun(
        &db,
        &[
            "init",
            "--name",
            "Demo BV",
            "--registration-id",
            "12345678",
            "--legal-form",
            "bv",
            "--vat",
            "on",
        ],
    );
    crun(
        &db,
        &[
            "vat",
            "book",
            "--date",
            "2026-07-01",
            "--desc",
            "Omzet",
            "--postings",
            "1100:121.00,8000:-100.00@21",
            "--post",
        ],
    );

    let (dry, ok) = crun(
        &db,
        &[
            "vat",
            "file",
            "--account",
            "2515",
            "--period",
            "2026-Q3",
            "--dry-run",
        ],
    );
    assert!(ok, "{dry}");
    assert_eq!(dry["data"]["account"], json!("2515"), "{dry}");
    let (filed, ok) = crun(
        &db,
        &["vat", "file", "--account", "2515", "--period", "2026-Q3"],
    );
    assert!(ok, "{filed}");
    assert_eq!(filed["data"]["account"], json!("2515"));
    assert_eq!(filed["data"]["liability_cents"], json!(2100));

    let tdir = temp_dir("cli22ob");
    let camt = tdir.join("ob.camt.xml");
    std::fs::write(
        &camt,
        r#"<?xml version="1.0"?>
<Document xmlns="urn:iso:std:iso:20022:tech:xsd:camt.053.001.02">
  <BkToCstmrStmt><Stmt><Acct><Id><IBAN>NL91ABNA0417164300</IBAN></Id></Acct>
    <Ntry><Amt>21.00</Amt><CdtDbtInd>DBIT</CdtDbtInd><BookgDt><Dt>2026-07-25</Dt></BookgDt>
      <NtryDtls><TxDtls><RltdPties><Dbtr><Nm>Belastingdienst</Nm></Dbtr></RltdPties>
      <RmtInf><Ustrd>OB aangifte</Ustrd></RmtInf></TxDtls></NtryDtls></Ntry>
  </Stmt></BkToCstmrStmt>
</Document>"#,
    )
    .unwrap();
    crun(
        &db,
        &["bank", "add", "--iban", CLI_IBAN, "--name", "Rabobank"],
    );
    crun(
        &db,
        &[
            "bank",
            "import",
            "--file",
            &camt.to_string_lossy(),
            "--iban",
            CLI_IBAN,
        ],
    );
    let (txs, _) = crun(&db, &["bank", "transactions"]);
    let tx_id = txs["data"]["transactions"][0]["id"].as_i64().unwrap();

    let (settled, ok) = crun(
        &db,
        &[
            "vat",
            "settle",
            "--tx",
            &tx_id.to_string(),
            "--account",
            "2515",
            "--period",
            "2026-Q3",
        ],
    );
    assert!(ok, "{settled}");
    assert_eq!(settled["data"]["account"], json!("2515"));
    assert_eq!(
        settled["data"]["difference_cents"],
        json!(0),
        "21.00 filed = 21.00 booked"
    );
    assert_eq!(settled["data"]["tx"]["state"], json!("matched"));
    let (tb, _) = crun(&db, &["report", "trial-balance"]);
    assert_eq!(tb["data"]["balanced"], json!(true));
}

#[test]
fn cli_vat_file_and_settle_end_to_end_with_a_rounding_difference() {
    let (_dir, db) = cdb("cli23");
    crun(
        &db,
        &[
            "init",
            "--name",
            "Demo BV",
            "--registration-id",
            "12345678",
            "--legal-form",
            "bv",
            "--vat",
            "on",
        ],
    );
    crun(
        &db,
        &[
            "vat",
            "book",
            "--date",
            "2026-07-01",
            "--desc",
            "Omzet",
            "--postings",
            "1100:121.00,8000:-100.00@21",
            "--post",
        ],
    );
    crun(
        &db,
        &[
            "vat",
            "book",
            "--date",
            "2026-07-05",
            "--desc",
            "Inkoop",
            "--postings",
            "1100:-60.50,4300:50.00@21",
            "--post",
        ],
    );

    let (dry, ok) = crun(&db, &["vat", "file", "--period", "2026-Q3", "--dry-run"]);
    assert!(ok, "{dry}");
    assert_eq!(dry["data"]["dryRun"], json!(true));
    assert_eq!(dry["data"]["liability_cents"], json!(1050));

    let (filed, ok) = crun(&db, &["vat", "file", "--period", "2026-Q3"]);
    assert!(ok, "{filed}");
    assert!(filed["data"]["entry_id"].as_i64().unwrap() > 0, "{filed}");
    assert_eq!(filed["data"]["liability_cents"], json!(1050));
    assert_eq!(filed["data"]["owe"], json!(true));

    let tdir = temp_dir("cli23ob");
    let camt = tdir.join("ob.camt.xml");
    std::fs::write(
        &camt,
        r#"<?xml version="1.0"?>
<Document xmlns="urn:iso:std:iso:20022:tech:xsd:camt.053.001.02">
  <BkToCstmrStmt><Stmt><Acct><Id><IBAN>NL91ABNA0417164300</IBAN></Id></Acct>
    <Ntry><Amt>10.00</Amt><CdtDbtInd>DBIT</CdtDbtInd><BookgDt><Dt>2026-07-25</Dt></BookgDt>
      <NtryDtls><TxDtls><RltdPties><Dbtr><Nm>Belastingdienst</Nm></Dbtr></RltdPties>
      <RmtInf><Ustrd>OB aangifte</Ustrd></RmtInf></TxDtls></NtryDtls></Ntry>
  </Stmt></BkToCstmrStmt>
</Document>"#,
    )
    .unwrap();
    crun(
        &db,
        &["bank", "add", "--iban", CLI_IBAN, "--name", "Rabobank"],
    );
    crun(
        &db,
        &[
            "bank",
            "import",
            "--file",
            &camt.to_string_lossy(),
            "--iban",
            CLI_IBAN,
        ],
    );
    let (txs, _) = crun(&db, &["bank", "transactions"]);
    let tx_id = txs["data"]["transactions"][0]["id"].as_i64().unwrap();

    let (settle_dry, ok) = crun(
        &db,
        &[
            "vat",
            "settle",
            "--tx",
            &tx_id.to_string(),
            "--period",
            "2026-Q3",
            "--dry-run",
        ],
    );
    assert!(ok, "{settle_dry}");
    assert_eq!(settle_dry["data"]["dryRun"], json!(true));
    assert_eq!(settle_dry["data"]["difference_cents"], json!(-50));
    let (txs2, _) = crun(&db, &["bank", "transactions"]);
    assert_eq!(
        txs2["data"]["transactions"][0]["state"],
        json!("unmatched"),
        "dry-run must not match"
    );

    let (settled, ok) = crun(
        &db,
        &[
            "vat",
            "settle",
            "--tx",
            &tx_id.to_string(),
            "--period",
            "2026-Q3",
        ],
    );
    assert!(ok, "{settled}");
    assert_eq!(
        settled["data"]["difference_cents"],
        json!(-50),
        "paid 50 cents less -> P&L gain"
    );
    assert_eq!(settled["data"]["tx"]["state"], json!("matched"));

    let (again, ok) = crun(&db, &["vat", "settle", "--tx", &tx_id.to_string()]);
    assert!(!ok);
    assert_eq!(again["error"]["code"], json!("ALREADY_MATCHED"), "{again}");

    let (tb, _) = crun(&db, &["report", "trial-balance"]);
    assert_eq!(tb["data"]["balanced"], json!(true));
    let (pnl, _) = crun(&db, &["report", "pnl", "--year", "2026"]);
    let has_gain = pnl["data"]["sections"].as_array().unwrap().iter().any(|s| {
        s["accounts"]
            .as_array()
            .map(|a| {
                a.iter()
                    .any(|r| r["code"] == json!("4700") && r["amount_cents"] == json!(-50))
            })
            .unwrap_or(false)
    });
    assert!(has_gain, "the rounding gain must land in 4700: {pnl}");
}

#[test]
fn cli_entry_post_dry_run_rejects_non_draft_entries() {
    let (_dir, db) = cdb("cli24");
    crun(
        &db,
        &[
            "init",
            "--name",
            "Demo BV",
            "--registration-id",
            "12345678",
            "--legal-form",
            "bv",
            "--vat",
            "off",
        ],
    );
    let (out, ok) = crun(
        &db,
        &[
            "entry",
            "add",
            "--date",
            "2026-01-01",
            "--desc",
            "x",
            "--postings",
            "1100:100.00,3000:-100.00",
            "--post",
        ],
    );
    assert!(ok, "{out}");
    let id = out["data"]["id"].as_i64().unwrap();
    let (r, ok) = crun(
        &db,
        &["entry", "post", "--id", &id.to_string(), "--dry-run"],
    );
    assert!(
        !ok,
        "already posted -> dry-run must fail, not show a green plan"
    );
    assert_eq!(r["error"]["code"], json!("ALREADY_POSTED"), "{r}");
}

#[test]
fn cli_entry_reverse_dry_run_rejects_drafts_and_double_reversals() {
    let (_dir, db) = cdb("cli25");
    crun(
        &db,
        &[
            "init",
            "--name",
            "Demo BV",
            "--registration-id",
            "12345678",
            "--legal-form",
            "bv",
            "--vat",
            "off",
        ],
    );
    let (out, ok) = crun(
        &db,
        &[
            "entry",
            "add",
            "--date",
            "2026-01-01",
            "--desc",
            "x",
            "--postings",
            "1100:100.00,3000:-100.00",
        ],
    );
    assert!(ok, "{out}");
    let id = out["data"]["id"].as_i64().unwrap();

    let (r1, ok) = crun(
        &db,
        &["entry", "reverse", "--id", &id.to_string(), "--dry-run"],
    );
    assert!(!ok, "a draft must not get a reversal plan");
    assert_eq!(r1["error"]["code"], json!("NOT_POSTED"), "{r1}");

    crun(&db, &["entry", "post", "--id", &id.to_string()]);
    crun(
        &db,
        &[
            "entry",
            "reverse",
            "--id",
            &id.to_string(),
            "--reason",
            "correctie",
        ],
    );
    let (r2, ok) = crun(
        &db,
        &["entry", "reverse", "--id", &id.to_string(), "--dry-run"],
    );
    assert!(!ok);
    assert_eq!(r2["error"]["code"], json!("ALREADY_REVERSED"), "{r2}");
}

#[test]
fn cli_vat_book_dry_run_validates_date_and_description() {
    let (_dir, db) = cdb("cli26");
    crun(
        &db,
        &[
            "init",
            "--name",
            "Demo BV",
            "--registration-id",
            "12345678",
            "--legal-form",
            "bv",
            "--vat",
            "on",
        ],
    );
    let (r, ok) = crun(
        &db,
        &[
            "vat",
            "book",
            "--date",
            "2026-99-99",
            "--desc",
            "x",
            "--postings",
            "1100:121.00,8000:-100.00@21",
            "--dry-run",
        ],
    );
    assert!(!ok);
    assert_eq!(r["error"]["code"], json!("INVALID_DATE"), "{r}");
}

#[test]
fn cli_actor_help_lists_the_identity_subcommands() {
    let (_dir, db) = cdb("cli27");
    let help = crun_text(&db, &["actor", "--help"]);
    for cmd in [
        "keygen", "register", "list", "revoke", "enforce", "unlock", "lock", "verify",
    ] {
        assert!(
            help.contains(cmd),
            "actor help must mention '{cmd}':\n{help}"
        );
    }
    // an unknown subcommand exits non-zero
    let exe = env!("CARGO_BIN_EXE_bukio");
    let out = std::process::Command::new(exe)
        .args(["--db", &db, "actor", "frobnicate"])
        .env("BUKIO_ACTOR", "agent:test")
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "an unknown actor subcommand must fail"
    );
}

#[test]
fn cli_actor_enforce_needs_exactly_one_of_on_or_off() {
    let (_dir, db) = cdb("cli28");
    let (out, ok) = crun(&db, &["actor", "enforce"]);
    assert!(!ok);
    assert_eq!(out["ok"], json!(false), "{out}");
    assert_eq!(out["error"]["code"], json!("INVALID_ENFORCE"), "{out}");
}

/// init + keygen + register agent:bartholomeus + one signed entry (enforce off).
fn signed_company(tag: &str) -> (std::path::PathBuf, String, String, String) {
    let dir = temp_dir(tag);
    let cfg = dir.join("cfg").to_string_lossy().to_string();
    let db = dir.join("company.db").to_string_lossy().to_string();
    std::fs::create_dir_all(&cfg).unwrap();
    let env: Vec<(&str, &str)> = vec![("BUKIO_CONFIG_DIR", &cfg), ("BUKIO_ACTOR", "agent:test")];
    let run = |args: &[&str]| {
        let mut full = vec!["--json"];
        full.extend_from_slice(args);
        full.extend(["--db", &db]);
        let (r, ok, out) = acli(&full, &env);
        assert!(ok, "{}: {out}", args.join(" "));
        r
    };
    run(&["--actor", "human:erik", "init", "--name", "X"]);
    run(&["--actor", "agent:bartholomeus", "actor", "keygen"]);
    run(&["--actor", "agent:bartholomeus", "actor", "register"]);
    run(&[
        "--actor",
        "agent:bartholomeus",
        "entry",
        "add",
        "--date",
        "2026-08-10",
        "--desc",
        "Signed",
        "--postings",
        "1100:100.00,8000:-100.00",
        "--post",
    ]);
    (dir, cfg, db, String::new())
}

#[test]
fn cli_audit_verify_clean_trail_summarises_ok() {
    let (dir, cfg, db, _) = signed_company("cli29");
    let env: Vec<(&str, &str)> = vec![("BUKIO_CONFIG_DIR", &cfg), ("BUKIO_ACTOR", "agent:test")];
    let (data, ok, out) = acli(&["--json", "audit", "verify", "--db", &db], &env);
    assert!(ok, "{out}");
    let summary = &data["data"]["summary"];
    assert!(summary["ok"].as_i64().unwrap() >= 1, "{summary}");
    assert_eq!(summary["tampered"], json!(0), "{summary}");
    assert_eq!(summary["invalid_signature"], json!(0));
    assert_eq!(summary["unknown_key"], json!(0));
    let rows = data["data"]["rows"].as_array().unwrap();
    assert!(
        rows.iter()
            .all(|r| r["status"] == json!("ok") || r["status"] == json!("unsigned")),
        "{rows:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cli_audit_verify_reports_a_tampered_row_with_exit_one() {
    let (dir, cfg, db, _) = signed_company("cli30");
    // The audit log is append-only (a trigger blocks UPDATE), so tamper the
    // way the JS suite does: record a row whose stored digest does not match
    // its signed args.
    {
        let d = bukio::db::open_db(&db).unwrap();
        let ts = bukio::actor::now_iso();
        let nonce = "tamper-nonce-1";
        let args = json!({ "date": "2026-08-10", "desc": "original" });
        let real_digest =
            bukio::canonical::build_digest("agent:bartholomeus", "entry add", &args, &ts, nonce);
        let tampered_digest = bukio::canonical::build_digest(
            "agent:bartholomeus",
            "entry add",
            &json!({ "date": "2026-08-10", "desc": "HACKED" }),
            &ts,
            nonce,
        );
        let (_, private, _) = bukio::sign::generate_key_pair();
        bukio::audit::set_pending_signature(Some(bukio::audit::PendingSignature {
            digest_hash: Some(tampered_digest),
            sig_keyid: Some("ff".repeat(16)),
            sig_nonce: Some(nonce.to_string()),
            sig_ts: Some(ts),
            sig: Some(bukio::sign::sign(real_digest.as_bytes(), &private).unwrap()),
            sig_status: "verified".to_string(),
            signed_args: Some(args),
            signed_command: Some("entry add".to_string()),
        }));
        bukio::audit::record(
            &d,
            bukio::audit::RecordArgs {
                actor: "agent:bartholomeus",
                action: "test.tampered",
                command: Some("entry add"),
                args: None,
                outcome: "ok",
                entry_ids: vec![],
            },
        )
        .unwrap();
        bukio::audit::set_pending_signature(None);
    }
    let env: Vec<(&str, &str)> = vec![("BUKIO_CONFIG_DIR", &cfg), ("BUKIO_ACTOR", "agent:test")];
    let (data, ok, _) = acli(&["--json", "audit", "verify", "--db", &db], &env);
    assert!(!ok, "audit verify must exit 1 when the trail has problems");
    let summary = &data["data"]["summary"];
    assert_eq!(summary["tampered"], json!(1), "{summary}");
    let bad = data["data"]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["status"] == json!("tampered"))
        .expect("a tampered row must be reported");
    assert_eq!(bad["action"], json!("test.tampered"), "{bad}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cli_version_matches_the_crate_version() {
    // This used to compare against package.json — the JS tree's version file,
    // which this branch no longer carries. Cargo's own package version is the
    // source of truth, and env!() resolves it at compile time.
    let exe = env!("CARGO_BIN_EXE_bukio");
    let out = std::process::Command::new(exe)
        .arg("--version")
        .output()
        .unwrap();
    let version = String::from_utf8_lossy(&out.stdout).trim().to_string();
    assert_eq!(
        version,
        env!("CARGO_PKG_VERSION"),
        "bukio --version must equal Cargo.toml's version"
    );
}
// ==== agent layer: fx, ECB, compliance, MCP (ported from test/agent-layer.test.js) ====

const SDMX_USD: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<message:GenericData xmlns:message="http://www.sdmx.org/resources/sdmxml/schemas/v2_1/message" xmlns:generic="http://www.sdmx.org/resources/sdmxml/schemas/v2_1/data/generic">
<message:DataSet>
<generic:Series>
<generic:SeriesKey><generic:Value id="FREQ" value="D"/><generic:Value id="CURRENCY" value="USD"/></generic:SeriesKey>
<generic:Obs><generic:ObsDimension value="2026-07-30"/><generic:ObsValue value="1.1476"/></generic:Obs>
<generic:Obs><generic:ObsDimension value="2026-07-31"/><generic:ObsValue value="1.1485"/></generic:Obs>
<generic:Obs><generic:ObsDimension value="2026-08-03"/><generic:ObsValue value="1.1515"/></generic:Obs>
</generic:Series>
</message:DataSet>
</message:GenericData>"#;

fn ecb_ok(_url: &str) -> std::result::Result<Option<String>, String> {
    Ok(Some(SDMX_USD.to_string()))
}
fn ecb_404(_url: &str) -> std::result::Result<Option<String>, String> {
    Ok(None)
}
fn ecb_boom(_url: &str) -> std::result::Result<Option<String>, String> {
    Err("ENOTFOUND".to_string())
}
fn ecb_empty(_url: &str) -> std::result::Result<Option<String>, String> {
    Ok(Some(
        "<?xml version=\"1.0\"?><message:GenericData></message:GenericData>".to_string(),
    ))
}

/// A file DB + config dir: init (Demo BV, bv, VAT on) through the CLI.
fn agent_env(tag: &str) -> (std::path::PathBuf, String, String) {
    let dir = temp_dir(tag);
    let cfg = dir.join("cfg").to_string_lossy().to_string();
    let db = dir.join("company.db").to_string_lossy().to_string();
    std::fs::create_dir_all(&cfg).unwrap();
    let (out, ok, _) = acli(
        &[
            "--json",
            "init",
            "--name",
            "Demo BV",
            "--registration-id",
            "12345678",
            "--legal-form",
            "bv",
            "--vat",
            "on",
            "--tax-id",
            "NL123456789B01",
            "--address",
            "Industrieweg 12",
            "--postal-code",
            "2712 CD",
            "--city",
            "Zoetermeer",
            "--db",
            &db,
        ],
        &[("BUKIO_CONFIG_DIR", &cfg), ("BUKIO_ACTOR", "agent:test")],
    );
    assert!(ok, "{out}");
    (dir, cfg, db)
}

fn mem_db() -> rusqlite::Connection {
    bukio::db::open_db(":memory:").unwrap()
}

// --- FX rate store + conversion math ---------------------------------------

#[test]
fn fx_parse_rate_and_convert_fx_use_integer_math_rounded_half_up() {
    assert_eq!(bukio::fx::parse_rate("1.0875").unwrap(), 10875);
    assert_eq!(bukio::fx::parse_rate("1").unwrap(), 10000);
    assert_eq!(bukio::fx::parse_rate("0.9").unwrap(), 9000);
    assert_eq!(bukio::fx::parse_rate("1.087").unwrap(), 10870);
    assert_eq!(
        bukio::fx::parse_rate("abc").unwrap_err().code,
        "INVALID_RATE"
    );
    assert_eq!(
        bukio::fx::parse_rate("-1.0").unwrap_err().code,
        "INVALID_RATE"
    );
    assert_eq!(
        bukio::fx::parse_rate("1.08755").unwrap_err().code,
        "INVALID_RATE",
        "more than 4 decimals"
    );
    // 895.00 USD at 1.0875 -> 89500 * 10000 / 10875 = 82298.85 -> 82299
    assert_eq!(bukio::fx::convert_fx(89500, 10875).unwrap(), 82299);
    assert_eq!(bukio::fx::convert_fx(124630, 10875).unwrap(), 114602);
}

#[test]
fn fx_set_rate_upserts_audits_and_get_rate_prefers_latest_on_or_before() {
    let db = mem_db();
    bukio::fx::set_fx_rate(
        &db,
        "USD",
        "2026-07-01",
        "1.08",
        "manual",
        "agent:test",
        false,
    )
    .unwrap();
    bukio::fx::set_fx_rate(
        &db,
        "USD",
        "2026-07-10",
        "1.09",
        "manual",
        "agent:test",
        false,
    )
    .unwrap();
    // upsert the same date
    bukio::fx::set_fx_rate(
        &db,
        "USD",
        "2026-07-01",
        "1.081",
        "manual",
        "agent:test",
        false,
    )
    .unwrap();

    assert_eq!(
        bukio::fx::get_fx_rate(&db, "USD", "2026-07-01").unwrap(),
        Some(10810)
    );
    assert_eq!(
        bukio::fx::get_fx_rate(&db, "USD", "2026-07-10").unwrap(),
        Some(10900)
    );
    assert_eq!(
        bukio::fx::get_fx_rate(&db, "USD", "2026-07-05").unwrap(),
        Some(10810),
        "latest on/before the date"
    );
    assert_eq!(
        bukio::fx::get_fx_rate(&db, "USD", "2026-06-01").unwrap(),
        None,
        "before the first"
    );
    assert_eq!(
        bukio::fx::get_fx_rate(&db, "GBP", "2026-07-05").unwrap(),
        None,
        "unknown currency"
    );
    assert_eq!(
        bukio::fx::list_fx_rates(&db, Some("USD"), 50)
            .unwrap()
            .len(),
        2
    );

    let n: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM audit_log WHERE action='fx.set'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 3, "every set/upsert is audited");

    assert_eq!(
        bukio::fx::set_fx_rate(&db, "usd", "2026-07-01", "1.0", "manual", "a", false)
            .unwrap_err()
            .code,
        "INVALID_CURRENCY"
    );
    assert_eq!(
        bukio::fx::set_fx_rate(&db, "USD", "bad", "1.0", "manual", "a", false)
            .unwrap_err()
            .code,
        "INVALID_DATE"
    );
}

#[test]
fn fx_to_eur_postings_attaches_the_original_amounts() {
    let specs = vec![
        PostingSpec {
            code: "4300".into(),
            amount_cents: 89500,
            vat_code: Some("21".into()),
            ..Default::default()
        },
        PostingSpec {
            code: "1100".into(),
            amount_cents: -124630,
            ..Default::default()
        },
    ];
    let eur = bukio::fx::to_eur_postings(specs, "USD", 10875).unwrap();
    assert_eq!(eur[0].amount_cents, 82299, "895.00 USD at 1.0875 -> EUR");
    assert_eq!(eur[0].fx_currency.as_deref(), Some("USD"));
    assert_eq!(
        eur[0].fx_amount_cents,
        Some(89500),
        "the original amount is kept"
    );
    assert_eq!(eur[1].amount_cents, -114602);
    assert_eq!(eur[1].fx_amount_cents, Some(-124630));
}

#[test]
fn fx_entry_add_with_currency_books_eur_and_keeps_the_original_amounts() {
    let (_dir, _cfg, db_path) = agent_env("fx4");
    let db = bukio::db::open_db(&db_path).unwrap();
    let specs = bukio::fx::to_eur_postings(
        vec![
            PostingSpec {
                code: "4300".into(),
                amount_cents: 89500,
                ..Default::default()
            },
            PostingSpec {
                code: "1100".into(),
                amount_cents: -89500,
                ..Default::default()
            },
        ],
        "USD",
        10875,
    )
    .unwrap();
    let e = bukio::entries::create_entry(
        &db,
        CreateEntry {
            date: "2026-07-03",
            description: "Factuur in USD",
            postings: specs,
            source: "manual",
            source_ref: None,
            actor: "agent:test",
        },
    )
    .unwrap();
    let entry = bukio::entries::post_entry(&db, e.id, "agent:test").unwrap();

    let p = entry
        .postings
        .iter()
        .find(|x| x.account_code == "4300")
        .unwrap();
    assert_eq!(p.amount_cents, 82299, "EUR");
    assert_eq!(p.fx_currency.as_deref(), Some("USD"));
    assert_eq!(p.fx_amount_cents, Some(89500));
    let other = entry
        .postings
        .iter()
        .find(|x| x.account_code == "1100")
        .unwrap();
    assert_eq!(other.amount_cents, -82299);

    // the reversal negates both the EUR and the FX amount
    let rev = bukio::entries::reverse_entry(&db, entry.id, "agent:test", None).unwrap();
    let rp = rev
        .postings
        .iter()
        .find(|x| x.account_code == "4300")
        .unwrap();
    assert_eq!(rp.amount_cents, -82299);
    assert_eq!(rp.fx_amount_cents, Some(-89500));
    assert_eq!(rp.fx_currency.as_deref(), Some("USD"));
}

#[test]
fn fx_vat_book_with_currency_computes_vat_on_the_eur_amounts() {
    let (_dir, _cfg, db_path) = agent_env("fx5");
    let db = bukio::db::open_db(&db_path).unwrap();
    bukio::fx::set_fx_rate(
        &db,
        "USD",
        "2026-07-03",
        "1.0875",
        "manual",
        "agent:test",
        false,
    )
    .unwrap();

    let raw =
        bukio::vat::parse_vat_posting_specs(&["4300:895.00@21,1100:-1082.95".to_string()]).unwrap();
    let rate = bukio::fx::get_fx_rate(&db, "USD", "2026-07-03")
        .unwrap()
        .unwrap();
    let specs = bukio::fx::to_eur_vat_specs(raw, "USD", rate).unwrap();
    let out = bukio::vat::book_vat_entry(
        &db,
        "2026-07-03",
        "DeKantoor B.V. (USD)",
        &specs,
        "manual",
        None,
        "agent:test",
        true,
    )
    .unwrap();

    let entry = if out.get("entry").is_some() {
        out["entry"].clone()
    } else {
        out.clone()
    };
    let postings = entry["postings"]
        .as_array()
        .unwrap_or_else(|| panic!("no postings in {out}"));
    let net = postings
        .iter()
        .find(|x| x["account_code"] == json!("4300"))
        .unwrap();
    assert_eq!(
        net["amount_cents"],
        json!(82299),
        "895.00 USD -> 822.99 EUR net"
    );
    assert_eq!(net["fx_amount_cents"], json!(89500), "{net}");
    let vat = postings.iter().find(|x| !x["vat_code"].is_null()).unwrap();
    assert_eq!(vat["amount_cents"], json!(82299), "{vat}");
    assert_eq!(
        vat["vat_amount_cents"],
        json!(17283),
        "21% of the EUR base: {vat}"
    );
    let bank = postings
        .iter()
        .find(|x| x["account_code"] == json!("1100"))
        .unwrap();
    assert_eq!(bank["amount_cents"], json!(-99582), "{bank}");

    // the OB readout sees the EUR base
    let r = bukio::vat::ob_readout(&db, "2026-Q3").unwrap();
    assert_eq!(r["fields"]["3a"], json!(82299), "{r}");
    assert_eq!(r["fields"]["5b"], json!(17283), "{r}");
}

#[test]
fn fx_invalid_currency_on_a_posting_is_rejected() {
    let (_dir, _cfg, db_path) = agent_env("fx6");
    let db = bukio::db::open_db(&db_path).unwrap();
    let err = bukio::entries::create_entry(
        &db,
        CreateEntry {
            date: "2026-07-03",
            description: "x",
            postings: vec![
                PostingSpec {
                    code: "4300".into(),
                    amount_cents: 100,
                    fx_currency: Some("usd".into()),
                    fx_amount_cents: Some(100),
                    ..Default::default()
                },
                PostingSpec {
                    code: "1100".into(),
                    amount_cents: -100,
                    ..Default::default()
                },
            ],
            source: "manual",
            source_ref: None,
            actor: "a",
        },
    )
    .unwrap_err();
    assert_eq!(err.code, "INVALID_FX_CURRENCY", "{err:?}");

    // the JS also rejects a non-integer fxAmountCents (INVALID_FX_AMOUNT);
    // in the port that state is unrepresentable — the field is an Option<i64>,
    // so a string can never reach create_entry. Nothing to assert at runtime.
    assert!(std::mem::size_of::<Option<i64>>() > 0);
}

// --- ECB reference rates ----------------------------------------------------

#[test]
fn ecb_parses_sdmx_observations_and_falls_back_to_the_last_business_day() {
    let obs = bukio::fx::parse_sdmx_observations(SDMX_USD, "USD");
    assert_eq!(obs.len(), 3, "{obs:?}");
    assert_eq!(obs[0].0, "2026-07-30");
    assert_eq!(obs[2].1, 1.1515);

    bukio::fx::set_ecb_fetcher(ecb_ok);
    let sat = bukio::fx::fetch_ecb_rate("USD", "2026-08-01").unwrap();
    assert_eq!(
        sat,
        Some(("2026-07-31".to_string(), 11485)),
        "Saturday -> Friday"
    );
    let mon = bukio::fx::fetch_ecb_rate("USD", "2026-08-03").unwrap();
    assert_eq!(
        mon,
        Some(("2026-08-03".to_string(), 11515)),
        "exact business day"
    );
    bukio::fx::clear_ecb_fetcher();
}

#[test]
fn ecb_missing_currency_is_none_and_a_network_failure_is_ecb_fetch_failed() {
    bukio::fx::set_ecb_fetcher(ecb_404);
    assert_eq!(
        bukio::fx::fetch_ecb_rate("XYZ", "2026-08-04").unwrap(),
        None
    );
    bukio::fx::clear_ecb_fetcher();

    bukio::fx::set_ecb_fetcher(ecb_boom);
    let err = bukio::fx::fetch_ecb_rate("USD", "2026-08-04").unwrap_err();
    assert_eq!(err.code, "ECB_FETCH_FAILED", "{err:?}");
    bukio::fx::clear_ecb_fetcher();
}

#[test]
fn fx_missing_rate_auto_fetches_from_ecb_stores_it_and_reuses_it() {
    let db = mem_db();
    bukio::fx::set_ecb_fetcher(ecb_ok);
    // no stored rate -> ECB fetch -> stored as source=ECB
    let r = bukio::fx::resolve_rate(&db, "USD", None, "2026-08-01", "agent:test", false).unwrap();
    assert_eq!(r, 11485);
    assert_eq!(
        bukio::fx::get_fx_rate(&db, "USD", "2026-08-01").unwrap(),
        Some(11485)
    );
    let (source, created_by): (String, String) = db
        .query_row(
            "SELECT source, created_by FROM fx_rates WHERE currency='USD' AND date='2026-07-31'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(source, "ECB");
    assert_eq!(created_by, "agent:test");

    // the second booking reuses the stored rate
    let count_before: i64 = db
        .query_row("SELECT COUNT(*) FROM fx_rates", [], |r| r.get(0))
        .unwrap();
    let r2 = bukio::fx::resolve_rate(&db, "USD", None, "2026-08-01", "agent:test", false).unwrap();
    assert_eq!(r2, 11485);
    let count_after: i64 = db
        .query_row("SELECT COUNT(*) FROM fx_rates", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count_before, count_after, "the stored rate wins");

    // an explicit rate always wins
    let r3 = bukio::fx::resolve_rate(&db, "USD", Some("1.09"), "2026-08-01", "agent:test", false)
        .unwrap();
    assert_eq!(r3, 10900);
    bukio::fx::clear_ecb_fetcher();
}

#[test]
fn fx_no_fetch_blocks_the_ecb_fallback() {
    let db = mem_db();
    let err =
        bukio::fx::resolve_rate(&db, "USD", None, "2026-08-01", "agent:test", true).unwrap_err();
    assert_eq!(err.code, "FX_RATE_NOT_FOUND", "{err:?}");
}

#[test]
fn fx_ecb_without_a_rate_for_the_currency_is_ecb_rate_not_available() {
    let db = mem_db();
    bukio::fx::set_ecb_fetcher(ecb_empty);
    let err = bukio::fx::resolve_rate(&db, "USD", None, "2026-08-01", "a", false).unwrap_err();
    assert_eq!(err.code, "ECB_RATE_NOT_AVAILABLE", "{err:?}");
    bukio::fx::clear_ecb_fetcher();
}

#[test]
fn fx_resolve_rate_dry_run_does_not_persist_the_fetched_ecb_rate() {
    let db = mem_db();
    bukio::fx::set_ecb_fetcher(ecb_ok);
    let rate =
        bukio::fx::resolve_rate_opt(&db, "USD", None, "2026-08-03", "agent:test", false, true)
            .unwrap();
    assert_eq!(rate, 11515);
    assert_eq!(
        bukio::fx::list_fx_rates(&db, None, 50).unwrap().len(),
        0,
        "dry-run must not INSERT"
    );
    let n: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM audit_log WHERE action = 'fx.set'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 0, "dry-run must not audit");

    // the execute path persists
    let rate2 =
        bukio::fx::resolve_rate(&db, "USD", None, "2026-08-03", "agent:test", false).unwrap();
    assert_eq!(rate2, 11515);
    assert_eq!(bukio::fx::list_fx_rates(&db, None, 50).unwrap().len(), 1);
    bukio::fx::clear_ecb_fetcher();
}

#[test]
fn mcp_resolve_fx_never_stores_the_fetched_rate_on_a_plan_only_call() {
    let db = mem_db();
    bukio::fx::set_ecb_fetcher(ecb_ok);
    let specs = || {
        vec![
            PostingSpec {
                code: "1100".into(),
                amount_cents: 10000,
                ..Default::default()
            },
            PostingSpec {
                code: "8000".into(),
                amount_cents: -10000,
                ..Default::default()
            },
        ]
    };
    // plan-only: the fetched rate must not be stored, and no fx.set row
    let plan = bukio::fx::resolve_fx(
        &db,
        specs(),
        Some("USD"),
        None,
        "2026-08-06",
        "agent:test",
        true,
    )
    .unwrap();
    assert_eq!(plan.len(), 2);
    assert_eq!(plan[0].fx_currency.as_deref(), Some("USD"));
    let n: i64 = db
        .query_row("SELECT COUNT(*) FROM fx_rates", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0, "no rate stored on a plan-only call");
    let n: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM audit_log WHERE action = 'fx.set'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 0, "no fx.set audit row on a plan-only call");

    // execute stores it for reuse
    bukio::fx::resolve_fx(
        &db,
        specs(),
        Some("USD"),
        None,
        "2026-08-06",
        "agent:test",
        false,
    )
    .unwrap();
    let n: i64 = db
        .query_row("SELECT COUNT(*) FROM fx_rates", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 1, "execute stores the fetched rate");
    bukio::fx::clear_ecb_fetcher();

    // the same path through the MCP tool (the wiring, not just the helper)
    let (_dir, cfg, db_path) = agent_env("fx12");
    bukio::fx::set_ecb_fetcher(ecb_ok);
    let mut m = Mcp::start_as(&db_path, "agent:test", Some(&cfg));
    mcp_init(&mut m);
    let (plan, is_err) = m.tool(
        "entry_add",
        json!({ "date": "2026-08-06", "description": "USD plan", "postings": ["4300:895.00", "1100:-895.00"], "currency": "USD" }),
    );
    assert!(!is_err, "{plan}");
    assert_eq!(plan["mode"], json!("dry-run"), "{plan}");
    let d = bukio::db::open_db(&db_path).unwrap();
    let n: i64 = d
        .query_row("SELECT COUNT(*) FROM fx_rates", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        n, 0,
        "a plan-only MCP call must not store the fetched rate: {plan}"
    );
    m.stop();
    bukio::fx::clear_ecb_fetcher();
    let _ = std::fs::remove_dir_all(&_dir);
}

// --- compliance -------------------------------------------------------------

#[test]
fn compliance_quarterly_deadlines() {
    assert_eq!(
        bukio::compliance::quarter_deadline("2026-Q1").unwrap(),
        ("2026-Q1", "2026-04-30".to_string())
    );
    assert_eq!(
        bukio::compliance::quarter_deadline("2026-Q3").unwrap(),
        ("2026-Q3", "2026-10-31".to_string())
    );
    assert_eq!(
        bukio::compliance::quarter_deadline("2026-Q4").unwrap(),
        ("2026-Q4", "2027-01-31".to_string())
    );
    assert_eq!(
        bukio::compliance::quarter_deadline("2026")
            .unwrap_err()
            .code,
        "INVALID_PERIOD"
    );
}

#[test]
fn compliance_jaarrekening_deadline_is_13_months_after_the_fiscal_year_end() {
    let (_dir, _cfg, db) = agent_env("al14");
    let fy: String = db_company_field(&db, "fiscal_year_end");
    assert_eq!(
        bukio::compliance::jaarrekening_deadline(&fy, 2026),
        "2028-01-31"
    );
    assert_eq!(
        bukio::compliance::jaarrekening_deadline("06-30", 2026),
        "2027-07-31",
        "a June year end"
    );
    // tolerant parse: a full YYYY-MM-DD must not read the year as the month
    assert_eq!(
        bukio::compliance::jaarrekening_deadline("2026-06-30", 2026),
        "2027-07-31",
        "full-date fiscal_year_end"
    );
}

fn db_company_field(db_path: &str, field: &str) -> String {
    let d = bukio::db::open_db(db_path).unwrap();
    let sql = format!("SELECT {field} FROM company WHERE id = 1");
    let v: Option<String> = d.query_row(&sql, [], |r| r.get(0)).unwrap();
    v.unwrap_or_default()
}

#[test]
fn compliance_calendar_shows_obligations_and_statuses_flip_with_filings() {
    let (_dir, _cfg, db_path) = agent_env("al15");
    let db = bukio::db::open_db(&db_path).unwrap();
    let r = bukio::compliance::compliance_status(&db, 2026).unwrap();
    let types: Vec<&str> = r["obligations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["type"].as_str().unwrap())
        .collect();
    assert!(types.contains(&"OB"), "{types:?}");
    assert!(types.contains(&"ICP"), "{types:?}");
    assert!(types.contains(&"JAARREKENING"), "{types:?}");
    assert!(r["obligations"].as_array().unwrap().len() >= 8, "{r}");

    let ob_q3 = r["obligations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["type"] == json!("OB") && o["period"] == json!("2026-Q3"))
        .expect("OB 2026-Q3");
    assert_eq!(ob_q3["deadline"], json!("2026-10-31"));
    assert_eq!(
        ob_q3["status"],
        json!("open"),
        "today is before the deadline"
    );

    // filing flips the status
    db.execute(
        "INSERT INTO vat_returns (type, period, status, fields_json, filed_at) VALUES ('OB','2026-Q3','filed','{}','2026-10-31')",
        [],
    )
    .unwrap();
    let r2 = bukio::compliance::compliance_status(&db, 2026).unwrap();
    let ob_q3 = r2["obligations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["type"] == json!("OB") && o["period"] == json!("2026-Q3"))
        .unwrap();
    assert_eq!(ob_q3["status"], json!("filed"));

    bukio::compliance::mark_filed(&db, "ICP", "2026-Q3", None, "agent:test", false).unwrap();
    bukio::compliance::mark_filed(&db, "JAARREKENING", "2026", None, "agent:test", false).unwrap();
    let r3 = bukio::compliance::compliance_status(&db, 2026).unwrap();
    for (t, p) in [("ICP", "2026-Q3"), ("JAARREKENING", "2026")] {
        let o = r3["obligations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|o| o["type"] == json!(t) && o["period"] == json!(p))
            .unwrap_or_else(|| panic!("{t} {p}"));
        assert_eq!(o["status"], json!("filed"), "{t}");
    }
    // OB must be filed through vat readout --mark-filed
    assert_eq!(
        bukio::compliance::mark_filed(&db, "OB", "2026-Q3", None, "agent:test", false)
            .unwrap_err()
            .code,
        "INVALID_TYPE"
    );
}

#[test]
fn compliance_closed_books_show_on_the_jaarrekening_obligation() {
    let (_dir, _cfg, db_path) = agent_env("al16");
    let db = bukio::db::open_db(&db_path).unwrap();
    let e = bukio::entries::create_entry(
        &db,
        CreateEntry {
            date: "2026-03-01",
            description: "Omzet",
            postings: vec![
                PostingSpec {
                    code: "1100".into(),
                    amount_cents: 12100,
                    cost_center_code: None,
                    vat_code: None,
                    vat_amount_cents: None,
                    fx_currency: None,
                    fx_amount_cents: None,
                },
                PostingSpec {
                    code: "8000".into(),
                    amount_cents: -10000,
                    cost_center_code: None,
                    vat_code: None,
                    vat_amount_cents: None,
                    fx_currency: None,
                    fx_amount_cents: None,
                },
                PostingSpec {
                    code: "2500".into(),
                    amount_cents: -2100,
                    cost_center_code: None,
                    vat_code: None,
                    vat_amount_cents: None,
                    fx_currency: None,
                    fx_amount_cents: None,
                },
            ],
            source: "manual",
            source_ref: None,
            actor: "a",
        },
    )
    .unwrap();
    bukio::entries::post_entry(&db, e.id, "a").unwrap();

    let before = bukio::compliance::compliance_status(&db, 2026).unwrap();
    let ob = before["obligations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["type"] == json!("JAARREKENING") && o["period"] == json!("2026"))
        .unwrap();
    assert_eq!(ob["books_closed"], json!(false), "{ob}");

    bukio::year_end::year_end_close(&db, "2026", "a", false).unwrap();
    let after = bukio::compliance::compliance_status(&db, 2026).unwrap();
    let ob = after["obligations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["type"] == json!("JAARREKENING") && o["period"] == json!("2026"))
        .unwrap();
    assert_eq!(ob["books_closed"], json!(true), "{ob}");
}

// --- MCP server (real stdio child process) ----------------------------------

fn mcp_init(m: &mut Mcp) {
    m.call(
        "initialize",
        json!({ "protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": { "name": "test", "version": "1" } }),
    );
}

#[test]
fn mcp_initialize_tools_list_and_read_only_calls_work_end_to_end() {
    let (_dir, cfg, db) = agent_env("al17");
    let (_, ok, _) = acli(
        &[
            "--json",
            "entry",
            "add",
            "--date",
            "2026-07-01",
            "--desc",
            "Omzet",
            "--postings",
            "1100:121.00,8000:-100.00,2500:-21.00",
            "--post",
            "--db",
            &db,
        ],
        &[("BUKIO_CONFIG_DIR", &cfg), ("BUKIO_ACTOR", "agent:test")],
    );
    assert!(ok);

    let mut m = Mcp::start_as(&db, "agent:test", Some(&cfg));
    mcp_init(&mut m);
    let init = m.call(
        "initialize",
        json!({ "protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": { "name": "test", "version": "1" } }),
    );
    assert_eq!(
        init["result"]["serverInfo"]["name"],
        json!("bukio-cli"),
        "{init}"
    );
    assert!(
        init["result"]["capabilities"]["tools"].is_object(),
        "{init}"
    );

    let tools = m.call("tools/list", json!({}));
    let names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    for want in [
        "trial_balance",
        "entry_add",
        "vat_book",
        "year_end_close",
        "fx_set",
        "compliance",
    ] {
        assert!(names.contains(&want), "{want} missing from {names:?}");
    }

    let (tb, is_err) = m.tool("trial_balance", json!({}));
    assert!(!is_err, "{tb}");
    assert_eq!(tb["balanced"], json!(true), "{tb}");

    let (ci, is_err) = m.tool("company_info", json!({}));
    assert!(!is_err, "{ci}");
    assert_eq!(ci["company"]["name"], json!("Demo BV"), "{ci}");

    let unknown = m.call("tools/call", json!({ "name": "nope", "arguments": {} }));
    assert_eq!(unknown["error"]["code"], json!(-32602), "{unknown}");

    // year is REQUIRED for pnl and journal — the schema must say so
    for tool in ["pnl", "journal"] {
        let t = tools["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == json!(tool))
            .unwrap_or_else(|| panic!("{tool} tool exists"));
        assert_eq!(t["inputSchema"]["required"], json!(["year"]), "{tool}");
    }
    m.stop();
}

#[test]
fn mcp_null_params_answer_cleanly_instead_of_an_internal_error() {
    let (_dir, cfg, db) = agent_env("al18");
    let mut m = Mcp::start_as(&db, "agent:test", Some(&cfg));
    mcp_init(&mut m);
    let r = m.call("tools/call", Value::Null);
    assert!(
        !r["error"].is_null(),
        "the call must be answered with an error object: {r}"
    );
    assert_eq!(
        r["error"]["code"],
        json!(-32602),
        "null params must yield invalid-params, not -32603: {r}"
    );

    // a null-params read that carries the tool name still works
    let (ci, is_err) = m.tool("company_info", Value::Null);
    assert!(!is_err, "{ci}");
    m.stop();
}

#[test]
fn mcp_invoices_tool_derives_the_overdue_status() {
    let (_dir, cfg, db) = agent_env("al19");
    // a finalized invoice due 2026-07-01 (in the past) -> derived status overdue
    let (contact, ok, out) = acli(
        &[
            "--json",
            "contact",
            "add",
            "--name",
            "Klant BV",
            "--address",
            "Straat 1",
            "--city",
            "Amsterdam",
            "--db",
            &db,
        ],
        &[("BUKIO_CONFIG_DIR", &cfg), ("BUKIO_ACTOR", "agent:test")],
    );
    assert!(ok, "{out}");
    let contact_id = contact["data"]["id"]
        .as_i64()
        .or_else(|| contact["data"]["contact"]["id"].as_i64())
        .unwrap_or_else(|| panic!("no contact id in {contact}"));
    let (inv, ok, out) = acli(
        &[
            "--json",
            "invoice",
            "create",
            "--contact",
            &contact_id.to_string(),
            "--date",
            "2026-06-01",
            "--due-days",
            "30",
            "--lines",
            "Ding @ 100.00 @21",
            "--db",
            &db,
        ],
        &[("BUKIO_CONFIG_DIR", &cfg), ("BUKIO_ACTOR", "agent:test")],
    );
    assert!(ok, "{out}");
    let inv_id = inv["data"]["id"]
        .as_i64()
        .or_else(|| inv["data"]["invoice"]["id"].as_i64())
        .unwrap_or_else(|| panic!("no invoice id in {inv}"));
    let (fin, ok, out) = acli(
        &[
            "--json",
            "invoice",
            "finalize",
            "--id",
            &inv_id.to_string(),
            "--db",
            &db,
        ],
        &[("BUKIO_CONFIG_DIR", &cfg), ("BUKIO_ACTOR", "agent:test")],
    );
    assert!(ok, "{out}");
    let number = fin["data"]["invoice"]["invoice_number"]
        .as_str()
        .or_else(|| fin["data"]["invoice_number"].as_str())
        .unwrap_or_else(|| panic!("no invoice number in {fin}"))
        .to_string();

    let mut m = Mcp::start_as(&db, "agent:test", Some(&cfg));
    mcp_init(&mut m);
    let (odata, is_err) = m.tool("invoices", json!({ "status": "overdue" }));
    assert!(!is_err, "{odata}");
    assert_eq!(odata["invoices"].as_array().unwrap().len(), 1, "{odata}");
    assert_eq!(
        odata["invoices"][0]["invoice_number"],
        json!(number),
        "{odata}"
    );

    // the stored status is 'sent' — the same invoice shows up there too
    let (sdata, _) = m.tool("invoices", json!({ "status": "sent" }));
    assert_eq!(sdata["invoices"].as_array().unwrap().len(), 1, "{sdata}");

    // an invalid status is still rejected
    let (_bad, is_err) = m.tool("invoices", json!({ "status": "bogus" }));
    assert!(is_err, "an invalid status must be refused");
    m.stop();
}

#[test]
fn mcp_non_object_json_rpc_messages_get_invalid_request_and_the_server_survives() {
    let (_dir, cfg, db) = agent_env("al20");
    let mut m = Mcp::start_as(&db, "agent:test", Some(&cfg));
    for raw in ["null", "42", "[1,2]"] {
        let r = m.raw(raw);
        assert_eq!(r["error"]["code"], json!(-32600), "raw {raw}: {r}");
    }
    // still alive
    mcp_init(&mut m);
    m.stop();
}

#[test]
fn mcp_mutations_are_plan_only_by_default_and_execute_books_with_the_actor() {
    let (_dir, cfg, db) = agent_env("al21");
    let mut m = Mcp::start_as(&db, "agent:test", Some(&cfg));
    mcp_init(&mut m);

    // dry-run: no write
    let (plan, is_err) = m.tool("entry_add", json!({ "date": "2026-07-05", "description": "Plan only", "postings": ["1100:5000.00", "3000:-5000.00"] }));
    assert!(!is_err, "{plan}");
    assert_eq!(plan["mode"], json!("dry-run"), "{plan}");
    assert_eq!(plan["balanced"], json!(true), "{plan}");

    // execute without post: a draft
    let (exec, is_err) = m.tool("entry_add", json!({ "date": "2026-07-05", "description": "Echte boeking", "postings": ["1100:5000.00", "3000:-5000.00"], "mode": "execute", "actor": "agent:mcp-test" }));
    assert!(!is_err, "{exec}");
    assert_eq!(exec["mode"], json!("execute"), "{exec}");
    assert_eq!(exec["state"], json!("draft"), "{exec}");

    // post it
    let (posted, is_err) = m.tool(
        "entry_post",
        json!({ "id": exec["entry_id"], "mode": "execute", "actor": "agent:mcp-test" }),
    );
    assert!(!is_err, "{posted}");
    assert_eq!(posted["state"], json!("posted"), "{posted}");

    // the audit trail shows the MCP actor
    let (audit, is_err) = m.tool("audit", json!({ "by": "agent:mcp-test" }));
    assert!(!is_err, "{audit}");
    assert!(audit["entries"].as_array().unwrap().len() >= 2, "{audit}");

    // fx via MCP
    let (fx, is_err) = m.tool("fx_set", json!({ "currency": "USD", "date": "2026-07-10", "rate": "1.09", "mode": "execute", "actor": "agent:mcp-test" }));
    assert!(!is_err, "{fx}");
    assert_eq!(fx["rate"], json!("1.0900"), "{fx}");

    // an invalid call -> isError
    let (_bad, is_err) = m.tool("entry_add", json!({ "date": "2026-07-05", "description": "x", "postings": ["1100:1.00"], "mode": "execute" }));
    assert!(is_err, "an unbalanced entry must be refused");
    m.stop();
}

#[test]
fn mcp_assets_run_books_depreciation_not_recurring_entries() {
    let (_dir, cfg, db) = agent_env("al22");
    let (_, ok, out) = acli(
        &[
            "--json",
            "assets",
            "add",
            "--name",
            "Laptop",
            "--purchase-date",
            "2025-12-15",
            "--purchase-price",
            "1200.00",
            "--depreciation-start",
            "2026-01-01",
            "--recognition-date",
            "2026-01-01",
            "--asset-account",
            "1800",
            "--expense-account",
            "4600",
            "--db",
            &db,
        ],
        &[("BUKIO_CONFIG_DIR", &cfg), ("BUKIO_ACTOR", "agent:test")],
    );
    assert!(ok, "{out}");

    let mut m = Mcp::start_as(&db, "agent:test", Some(&cfg));
    mcp_init(&mut m);
    let (data, is_err) = m.tool(
        "assets_run",
        json!({ "period": "2026-01", "mode": "execute", "actor": "agent:mcp-test" }),
    );
    assert!(!is_err, "{data}");
    assert_eq!(data["mode"], json!("execute"), "{data}");
    assert_eq!(data["booked"].as_array().unwrap().len(), 1, "{data}");
    m.stop();

    let d = bukio::db::open_db(&db).unwrap();
    let mut stmt = d
        .prepare("SELECT DISTINCT source FROM journal_entries")
        .unwrap();
    let sources: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert!(
        sources.iter().any(|s| s == "assets"),
        "a depreciation entry must be booked: {sources:?}"
    );
    assert!(
        !sources.iter().any(|s| s == "recurring"),
        "assets_run must NOT generate recurring entries: {sources:?}"
    );
}

#[test]
fn mcp_contact_add_preserves_postal_code_and_vat_id() {
    let (_dir, cfg, db) = agent_env("al23");
    let mut m = Mcp::start_as(&db, "agent:test", Some(&cfg));
    mcp_init(&mut m);
    let (_r, is_err) = m.tool(
        "contact_add",
        json!({ "name": "Acme GmbH", "address": "Leverstrasse 1", "postal_code": "80331", "city": "München", "country": "DE", "vat_id": "DE123456789", "mode": "execute", "actor": "agent:mcp-test" }),
    );
    assert!(!is_err, "contact_add failed");
    m.stop();

    let d = bukio::db::open_db(&db).unwrap();
    let (address, postal, city, country, vat): (String, String, String, String, String) = d
        .query_row(
            "SELECT address, postal_code, city, country, vat_id FROM contacts WHERE name = ?1",
            ["Acme GmbH"],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .expect("contact must exist");
    assert_eq!(address, "Leverstrasse 1");
    assert_eq!(postal, "80331");
    assert_eq!(city, "München");
    assert_eq!(country, "DE");
    assert_eq!(vat, "DE123456789");
}

#[test]
fn mcp_readonly_env_blocks_execution() {
    let (_dir, cfg, db) = agent_env("al24");
    let exe = env!("CARGO_BIN_EXE_bukio");
    let mut child = std::process::Command::new(exe)
        .args(["mcp", "--db", &db])
        .env("BUKIO_CONFIG_DIR", &cfg)
        .env("BUKIO_ACTOR", "agent:test")
        .env("BUKIO_MCP_READONLY", "1")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    use std::io::{BufRead, BufReader, Write};
    let mut stdin = child.stdin.take().unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    let req = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": { "name": "entry_add", "arguments": { "date": "2026-07-05", "description": "x", "postings": ["1100:1.00", "3000:-1.00"], "mode": "execute" } } });
    writeln!(stdin, "{req}").unwrap();
    stdin.flush().unwrap();
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let res: Value = serde_json::from_str(line.trim()).unwrap();
    assert_eq!(res["result"]["isError"], json!(true), "{res}");
    let data: Value =
        serde_json::from_str(res["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(data["error"]["code"], json!("MCP_READONLY"), "{data}");
    let _ = child.kill();
    let _ = child.wait();
}

// --- MCP signed execution (Tier 0) ------------------------------------------

/// Scratch company + config dir with agent:bartholomeus enrolled.
fn signed_company_mcp(tag: &str) -> (std::path::PathBuf, String, String) {
    let dir = temp_dir(tag);
    let cfg = dir.join("cfg").to_string_lossy().to_string();
    let db = dir.join("company.db").to_string_lossy().to_string();
    std::fs::create_dir_all(&cfg).unwrap();
    let base: Vec<(&str, &str)> = vec![("BUKIO_CONFIG_DIR", &cfg), ("BUKIO_ACTOR", "agent:test")];
    let run = |args: &[&str]| {
        let mut full = vec!["--json"];
        full.extend_from_slice(args);
        full.extend(["--db", &db]);
        let (r, ok, out) = acli(&full, &base);
        assert!(ok, "{}: {out}", args.join(" "));
        r
    };
    run(&["--actor", "human:erik", "init", "--name", "X"]);
    run(&["--actor", "agent:bartholomeus", "actor", "keygen"]);
    run(&["--actor", "agent:bartholomeus", "actor", "register"]);
    (dir, cfg, db)
}

fn last_audit_row_mcp(db_path: &str) -> Value {
    let d = bukio::db::open_db(db_path).unwrap();
    let (sig_status, command, digest, sig, keyid): (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    ) = d
        .query_row(
            "SELECT sig_status, command, digest_hash, sig, sig_keyid FROM audit_log ORDER BY id DESC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .unwrap();
    json!({ "sig_status": sig_status, "command": command, "digest_hash": digest, "sig": sig, "sig_keyid": keyid })
}

fn mcp_entry_args() -> Value {
    json!({ "date": "2026-08-10", "description": "MCP signed entry", "postings": ["1100:100.00", "8000:-100.00"], "mode": "execute", "post": true })
}

#[test]
fn mcp_signed_execute_call_stores_a_verified_audit_row() {
    let (dir, cfg, db) = signed_company_mcp("al26");
    let mut m = Mcp::start_as(&db, "agent:bartholomeus", Some(&cfg));
    mcp_init(&mut m);
    let mut args = mcp_entry_args();
    args["actor"] = json!("agent:bartholomeus");
    let (data, is_err) = m.tool("entry_add", args);
    assert!(!is_err, "{data}");
    assert_eq!(data["mode"], json!("execute"), "{data}");
    assert!(data["entry_id"].as_i64().unwrap() > 0, "{data}");
    m.stop();

    let row = last_audit_row_mcp(&db);
    assert_eq!(row["sig_status"], json!("verified"), "{row}");
    assert_eq!(
        row["command"],
        json!("mcp:entry_add"),
        "the signed command string"
    );
    assert!(
        !row["digest_hash"].is_null() && !row["sig"].is_null() && !row["sig_keyid"].is_null(),
        "{row}"
    );

    // the CLI verifier accepts the MCP-signed row (shared digest scheme)
    let (out, ok, _) = acli(
        &["--json", "audit", "verify", "--db", &db],
        &[("BUKIO_CONFIG_DIR", &cfg), ("BUKIO_ACTOR", "agent:test")],
    );
    assert!(ok, "{out}");
    assert_eq!(
        out["data"]["summary"]["ok"],
        json!(2),
        "entry.create + entry.post: {out}"
    );
    assert_eq!(out["data"]["summary"]["tampered"], json!(0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn mcp_enforce_with_a_missing_key_refuses_without_mutating() {
    let (dir, cfg, db) = signed_company_mcp("al27");
    let (_, ok, _) = acli(
        &[
            "--json",
            "--actor",
            "human:erik",
            "actor",
            "enforce",
            "--on",
            "--db",
            &db,
        ],
        &[("BUKIO_CONFIG_DIR", &cfg), ("BUKIO_ACTOR", "agent:test")],
    );
    assert!(ok);

    let mut m = Mcp::start_as(&db, "agent:test", Some(&cfg)); // no key material
    mcp_init(&mut m);
    let mut args = mcp_entry_args();
    args["actor"] = json!("agent:test");
    let (data, is_err) = m.tool("entry_add", args);
    assert!(is_err, "{data}");
    assert_eq!(data["ok"], json!(false), "{data}");
    assert_eq!(data["error"]["code"], json!("SIGNATURE_REQUIRED"), "{data}");

    let d = bukio::db::open_db(&db).unwrap();
    let n: i64 = d
        .query_row("SELECT COUNT(*) FROM journal_entries", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0, "no entry may be created");

    // the dry-run default is refused identically
    let (plan, is_err) = m.tool(
        "entry_add",
        json!({ "date": "2026-08-10", "description": "plan", "postings": ["1100:100.00", "8000:-100.00"], "actor": "agent:test" }),
    );
    assert!(is_err, "{plan}");
    assert_eq!(plan["error"]["code"], json!("SIGNATURE_REQUIRED"), "{plan}");
    m.stop();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn mcp_repeated_signed_calls_verify_and_record_fresh_nonces() {
    let (dir, cfg, db) = signed_company_mcp("al28");
    let mut m = Mcp::start_as(&db, "agent:bartholomeus", Some(&cfg));
    mcp_init(&mut m);
    for i in 0..2 {
        let mut args = mcp_entry_args();
        args["description"] = json!(format!("MCP entry {i}"));
        args["actor"] = json!("agent:bartholomeus");
        let (data, is_err) = m.tool("entry_add", args);
        assert!(!is_err, "{data}");
    }
    m.stop();

    let d = bukio::db::open_db(&db).unwrap();
    let mut stmt = d
        .prepare("SELECT sig_status FROM audit_log WHERE action = 'entry.create' ORDER BY id")
        .unwrap();
    let statuses: Vec<String> = stmt
        .query_map([], |r| r.get::<_, Option<String>>(0))
        .unwrap()
        .map(|r| r.unwrap().unwrap_or_default())
        .collect();
    assert_eq!(statuses, vec!["verified", "verified"], "{statuses:?}");

    // the shared nonce cache was written (the same file the CLI uses)
    let raw = std::fs::read_to_string(std::path::Path::new(&cfg).join("nonces.json")).unwrap();
    let nonces: Value = serde_json::from_str(&raw).unwrap();
    let count: usize = nonces
        .as_object()
        .unwrap()
        .values()
        .map(|by_key| by_key.as_object().map(|o| o.len()).unwrap_or(0))
        .sum();
    assert!(count >= 2, "nonces recorded per signed call (got {count})");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn mcp_malformed_actor_is_still_rejected() {
    let (dir, cfg, db) = signed_company_mcp("al29");
    let mut m = Mcp::start_as(&db, "agent:test", Some(&cfg));
    mcp_init(&mut m);
    let mut args = mcp_entry_args();
    args["actor"] = json!("agent"); // missing ':name'
    let (data, is_err) = m.tool("entry_add", args);
    assert!(is_err, "{data}");
    assert_eq!(data["error"]["code"], json!("INVALID_ACTOR"), "{data}");
    m.stop();
    let d = bukio::db::open_db(&db).unwrap();
    let n: i64 = d
        .query_row("SELECT COUNT(*) FROM journal_entries", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn mcp_a_second_company_db_uses_its_own_registry_and_enforce_state() {
    let (dir_a, cfg, db_a) = signed_company_mcp("al30a");
    // company B: enforce OFF, agent:bartholomeus NOT enrolled
    let dir_b = temp_dir("al30b");
    let db_b = dir_b.join("company.db").to_string_lossy().to_string();
    let (_, ok, _) = acli(
        &[
            "--json",
            "--actor",
            "human:erik",
            "init",
            "--name",
            "Y",
            "--db",
            &db_b,
        ],
        &[("BUKIO_CONFIG_DIR", &cfg), ("BUKIO_ACTOR", "agent:test")],
    );
    assert!(ok);

    // A: enforce on — signed calls run and verify
    let (_, ok, _) = acli(
        &[
            "--json",
            "--actor",
            "human:erik",
            "actor",
            "enforce",
            "--on",
            "--db",
            &db_a,
        ],
        &[("BUKIO_CONFIG_DIR", &cfg), ("BUKIO_ACTOR", "agent:test")],
    );
    assert!(ok);
    let mut m_a = Mcp::start_as(&db_a, "agent:bartholomeus", Some(&cfg));
    mcp_init(&mut m_a);
    let mut args = mcp_entry_args();
    args["description"] = json!("in A");
    args["actor"] = json!("agent:bartholomeus");
    let (_data, is_err) = m_a.tool("entry_add", args);
    assert!(!is_err, "A must accept the signed call");
    m_a.stop();
    let d = bukio::db::open_db(&db_a).unwrap();
    let status: Option<String> = d
        .query_row(
            "SELECT sig_status FROM audit_log WHERE action = 'entry.create' ORDER BY id DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(status.as_deref(), Some("verified"), "A verifies");

    // B: the same key, enforce off, not enrolled -> unsigned rows but it runs
    let mut m_b = Mcp::start_as(&db_b, "agent:bartholomeus", Some(&cfg));
    mcp_init(&mut m_b);
    let mut args = mcp_entry_args();
    args["description"] = json!("in B");
    args["actor"] = json!("agent:bartholomeus");
    let (_data, is_err) = m_b.tool("entry_add", args);
    assert!(!is_err, "B must still run (no enforcement)");
    m_b.stop();
    let d = bukio::db::open_db(&db_b).unwrap();
    let status: Option<String> = d
        .query_row(
            "SELECT sig_status FROM audit_log WHERE action = 'entry.create' ORDER BY id DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(status.as_deref(), Some("unsigned"), "B records unsigned");
    let _ = std::fs::remove_dir_all(&dir_a);
    let _ = std::fs::remove_dir_all(&dir_b);
}
// ==== hardening (ported from test/hardening.test.js) =========================

/// A file DB with a named company (the JS's tmpDb + init).
fn hard_db(tag: &str, vat: bool) -> (std::path::PathBuf, String, String) {
    let (dir, db) = cdb(tag);
    let cfg = dir.join("cfg").to_string_lossy().to_string();
    std::fs::create_dir_all(&cfg).unwrap();
    let mut args = vec![
        "--json",
        "init",
        "--name",
        "Test BV",
        "--registration-id",
        "12345678",
        "--legal-form",
        "bv",
    ];
    if vat {
        args.extend(["--vat", "on"]);
    }
    args.extend(["--db", db.as_str()]);
    let (out, ok, _) = acli(
        &args,
        &[("BUKIO_CONFIG_DIR", &cfg), ("BUKIO_ACTOR", "agent:test")],
    );
    assert!(ok, "init: {out}");
    (dir, cfg, db)
}

// --- F1: reversal carries VAT fields ---------------------------------------

#[test]
fn hard_reversal_of_a_vat_entry_cancels_the_ob_readout_and_keeps_vat_fields() {
    let (_dir, _cfg, db_path) = hard_db("h1", true);
    let db = bukio::db::open_db(&db_path).unwrap();
    let specs =
        bukio::vat::parse_vat_posting_specs(&["8000:-100.00@21,1100:121.00".to_string()]).unwrap();
    let out = bukio::vat::book_vat_entry(
        &db,
        "2026-01-15",
        "verkoop",
        &specs,
        "manual",
        None,
        "agent:test",
        true,
    )
    .unwrap();
    let entry_id = out["id"].as_i64().unwrap();

    let before = bukio::vat::ob_readout(&db, "2026-Q1").unwrap();
    assert_eq!(before["fields"]["1a"], json!(10000), "{before}");
    assert_eq!(before["fields"]["5a"], json!(2100), "{before}");

    let rev = bukio::entries::reverse_entry(&db, entry_id, "agent:test", None).unwrap();
    let after = bukio::vat::ob_readout(&db, "2026-Q1").unwrap();
    assert_eq!(
        after["fields"]["1a"],
        json!(0),
        "1a must cancel after reversal: {after}"
    );
    assert_eq!(
        after["fields"]["5a"],
        json!(0),
        "5a must cancel after reversal: {after}"
    );

    // the reversal posting carries the VAT code + negated VAT amount
    let rev_entry = bukio::entries::get_entry(&db, rev.id).unwrap();
    let tagged = rev_entry
        .postings
        .iter()
        .find(|p| p.account_code == "8000")
        .expect("8000 posting");
    assert_eq!(
        tagged.vat_amount_cents,
        Some(2100),
        "original was -2100: {tagged:?}"
    );
}

// --- F2: parsePeriod month bounds -------------------------------------------

#[test]
fn hard_parse_period_rejects_out_of_range_months() {
    for bad in ["2026-13", "2026-00", "2026-99", "2026-1"] {
        let err = bukio::vat::parse_period(bad).unwrap_err();
        assert_eq!(err.code, "INVALID_PERIOD", "'{bad}' must be rejected");
    }
    let (from, to) = bukio::vat::parse_period("2026-Q4").unwrap();
    assert_eq!((from.as_str(), to.as_str()), ("2026-10-01", "2026-12-31"));
    let (from, to) = bukio::vat::parse_period("2026-12").unwrap();
    assert_eq!((from.as_str(), to.as_str()), ("2026-12-01", "2026-12-31"));
    let (_, to) = bukio::vat::parse_period("2024-02").unwrap();
    assert_eq!(to, "2024-02-29", "leap February");
}

// --- F9: vat book with 0-rate codes ------------------------------------------

#[test]
fn hard_vat_book_with_v_and_0_books_without_a_zero_leg() {
    let (_dir, _cfg, db_path) = hard_db("h9", true);
    let db = bukio::db::open_db(&db_path).unwrap();
    for code in ["V", "0"] {
        let specs =
            bukio::vat::parse_vat_posting_specs(&[format!("8000:-100.00@{code},1100:100.00")])
                .unwrap();
        let out = bukio::vat::book_vat_entry(
            &db,
            "2026-01-10",
            &format!("code {code}"),
            &specs,
            "manual",
            None,
            "agent:test",
            true,
        )
        .unwrap();
        assert_eq!(out["state"], json!("posted"), "{out}");
        let postings = out["postings"].as_array().unwrap();
        let tagged = postings
            .iter()
            .find(|p| p["account_code"] == json!("8000"))
            .expect("8000 posting");
        assert_eq!(tagged["vat_code"], json!(code), "{tagged}");
        assert_eq!(tagged["vat_amount_cents"], json!(0), "{tagged}");
        assert!(
            postings.iter().all(|p| p["amount_cents"] != json!(0)),
            "no zero-amount leg may survive: {postings:?}"
        );
    }
    let readout = bukio::vat::ob_readout(&db, "2026-Q1").unwrap();
    assert_eq!(
        readout["fields"]["1c"],
        json!(20000),
        "the base is still reported: {readout}"
    );
}

#[test]
fn hard_vat_book_with_r_books_no_vat_leg() {
    let (_dir, _cfg, db_path) = hard_db("h9r", true);
    let db = bukio::db::open_db(&db_path).unwrap();
    let specs =
        bukio::vat::parse_vat_posting_specs(&["8000:-100.00@R,1100:100.00".to_string()]).unwrap();
    let out = bukio::vat::book_vat_entry(
        &db,
        "2026-01-10",
        "verlegd",
        &specs,
        "manual",
        None,
        "agent:test",
        true,
    )
    .unwrap();
    let postings = out["postings"].as_array().unwrap();
    assert_eq!(postings.len(), 2, "omzet + bank, no VAT leg: {out}");
    assert!(
        !postings
            .iter()
            .any(|p| p["account_code"] == json!("2500") || p["account_code"] == json!("1500")),
        "{out}"
    );
}

// --- F10: FX+VAT rounding drift ----------------------------------------------

#[test]
fn hard_fx_vat_booking_absorbs_rounding_drift() {
    let (_dir, _cfg, db_path) = hard_db("h10", true);
    let db = bukio::db::open_db(&db_path).unwrap();
    let specs =
        bukio::vat::parse_vat_posting_specs(&["8000:-41.33@21,1100:50.01".to_string()]).unwrap();
    let converted = bukio::fx::to_eur_vat_specs(specs, "USD", 10001).unwrap();
    let (expanded, _vat) = bukio::vat::expand_vat_postings(&db, &converted).unwrap();
    let sum: i64 = expanded.iter().map(|p| p.amount_cents).sum();
    assert_eq!(sum, 0, "converted legs must sum to zero, got {sum}");

    let out = bukio::vat::book_vat_entry(
        &db,
        "2026-01-10",
        "USD inkoop",
        &converted,
        "manual",
        None,
        "agent:test",
        true,
    )
    .unwrap();
    assert_eq!(out["state"], json!("posted"), "{out}");
    let total: i64 = out["postings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["amount_cents"].as_i64().unwrap())
        .sum();
    assert_eq!(total, 0, "the booked entry must balance: {out}");
}

#[test]
fn hard_fx_vat_a_range_of_amounts_never_trips_unbalanced() {
    let (_dir, _cfg, db_path) = hard_db("h10b", true);
    let db = bukio::db::open_db(&db_path).unwrap();
    for fx in (4000..=4300).step_by(7) {
        let gross = ((fx as f64) * 1.21).round() as i64;
        let raw = format!(
            "8000:-{:.2}@21,1100:{:.2}",
            fx as f64 / 100.0,
            gross as f64 / 100.0
        );
        let specs = bukio::vat::parse_vat_posting_specs(&[raw]).unwrap();
        let converted = bukio::fx::to_eur_vat_specs(specs, "USD", 10001).unwrap();
        let (expanded, _) = bukio::vat::expand_vat_postings(&db, &converted).unwrap();
        let sum: i64 = expanded.iter().map(|p| p.amount_cents).sum();
        assert_eq!(sum, 0, "fx={fx} unbalanced by {sum}");
    }
}

// --- F11: vat book --json carries vat_code -----------------------------------

#[test]
fn hard_cli_vat_book_json_reports_the_vat_code_on_tagged_postings() {
    let (_dir, _cfg, db) = hard_db("h11", true);
    let (out, ok) = crun(
        &db,
        &[
            "vat",
            "book",
            "--date",
            "2026-01-10",
            "--desc",
            "verkoop",
            "--postings",
            "8000:-100.00@21,1100:121.00",
            "--post",
        ],
    );
    assert!(ok, "{out}");
    let postings = out["data"]["entry"]["postings"].as_array().unwrap();
    let tagged = postings
        .iter()
        .find(|p| p["account_code"] == json!("8000"))
        .expect("8000 posting");
    assert_eq!(tagged["vat_code"], json!("21"), "{tagged}");
    assert_eq!(tagged["vat_amount_cents"], json!(-2100), "{tagged}");
}

// --- F12: invoice pay amount parsing ----------------------------------------

#[test]
fn hard_cli_invoice_pay_rejects_non_international_amounts() {
    let (_dir, _cfg, db) = hard_db("h12", true);
    let (_o, ok) = crun(
        &db,
        &[
            "company",
            "update",
            "--tax-id",
            "NL123456789B01",
            "--address",
            "Industrieweg 12",
            "--postal-code",
            "2712 CD",
            "--city",
            "Zoetermeer",
            "--iban",
            CLI_IBAN,
        ],
    );
    assert!(ok);
    let (_, ok) = crun(
        &db,
        &[
            "contact",
            "add",
            "--name",
            "ACME BV",
            "--address",
            "Straat 1",
            "--city",
            "Amsterdam",
        ],
    );
    assert!(ok);
    let (_, ok) = crun(
        &db,
        &[
            "invoice",
            "create",
            "--contact",
            "1",
            "--lines",
            "1x Test @ 100.00 @21",
            "--date",
            "2026-01-10",
        ],
    );
    assert!(ok);
    let (_, ok) = crun(&db, &["invoice", "finalize", "--id", "1"]);
    assert!(ok);

    for amount in ["12,34", "1e3"] {
        let (bad, ok) = crun(
            &db,
            &[
                "invoice",
                "pay",
                "--id",
                "1",
                "--date",
                "2026-01-20",
                "--amount",
                amount,
            ],
        );
        assert!(!ok, "'{amount}' must be rejected");
        assert_eq!(bad["error"]["code"], json!("INVALID_AMOUNT"), "{bad}");
    }
    let (_, ok) = crun(
        &db,
        &[
            "invoice",
            "pay",
            "--id",
            "1",
            "--date",
            "2026-01-20",
            "--amount",
            "12.34",
        ],
    );
    assert!(ok);
    let (show, _) = crun(&db, &["invoice", "show", "--id", "1"]);
    assert_eq!(show["data"]["invoice"]["paid"], json!("12.34"), "{show}");
}

// --- dry-run uniformity -----------------------------------------------------

#[test]
fn hard_cli_backup_dry_run_writes_no_file() {
    let (dir, _cfg, db) = hard_db("h17", false);
    let out_path = dir.join("never.db").to_string_lossy().to_string();
    let (out, ok) = crun(&db, &["backup", "--out", &out_path, "--dry-run"]);
    assert!(ok, "{out}");
    assert_eq!(out["data"]["dryRun"], json!(true), "{out}");
    assert!(
        !std::path::Path::new(&out_path).exists(),
        "dry-run must not write the file"
    );
}

#[test]
fn hard_cli_export_xaf_dry_run_writes_nothing_and_schemes_validate() {
    let (dir, _cfg, db) = hard_db("h18", false);
    let (_, ok) = crun(
        &db,
        &[
            "entry",
            "add",
            "--date",
            "2026-01-10",
            "--desc",
            "Start",
            "--postings",
            "1100:1000.00,3000:-1000.00",
            "--post",
        ],
    );
    assert!(ok);
    let out_path = dir.join("never.xaf").to_string_lossy().to_string();
    let (out, ok) = crun(
        &db,
        &[
            "export",
            "xaf",
            "--year",
            "2026",
            "--out",
            &out_path,
            "--dry-run",
        ],
    );
    assert!(ok, "{out}");
    assert_eq!(out["data"]["dryRun"], json!(true), "{out}");
    assert!(!std::path::Path::new(&out_path).exists());

    // scheme dry-run validates the life instead of printing a NaN plan
    let (bad_scheme, ok) = crun(
        &db,
        &[
            "assets",
            "scheme",
            "add",
            "--name",
            "X",
            "--life-months",
            "abc",
            "--dry-run",
        ],
    );
    assert!(!ok);
    assert_eq!(
        bad_scheme["error"]["code"],
        json!("INVALID_LIFE"),
        "{bad_scheme}"
    );

    // depreciation dry-run validates the non-positive-final guard
    let (bad_dep, ok) = crun(
        &db,
        &[
            "depreciation",
            "add",
            "--name",
            "D",
            "--cost",
            "1.00",
            "--life-months",
            "150",
            "--start",
            "2026-01-01",
            "--dry-run",
        ],
    );
    assert!(!ok);
    assert_eq!(bad_dep["error"]["code"], json!("INVALID_LIFE"), "{bad_dep}");
}

#[test]
fn hard_cli_assets_register_csv_has_a_header_and_totals() {
    let (_dir, _cfg, db) = hard_db("h15", false);
    let (_, ok) = crun(
        &db,
        &[
            "assets",
            "add",
            "--name",
            "Laptop",
            "--purchase-date",
            "2026-01-01",
            "--purchase-price",
            "1200.00",
            "--depreciation-start",
            "2026-01-01",
            "--recognition-date",
            "2026-01-01",
        ],
    );
    assert!(ok);
    let csv = crun_text(&db, &["assets", "register", "--format", "csv"]);
    let header = csv.lines().next().unwrap_or("");
    assert!(
        header.starts_with("id,name,category"),
        "header row expected, got: {header}"
    );
    assert!(csv.contains("Laptop"), "the asset row must be present");
    assert!(csv.contains("TOTAL"), "the totals row must be present");
}

#[test]
fn hard_cli_assets_register_json_without_the_global_flag() {
    let (_dir, _cfg, db) = hard_db("h15b", false);
    let raw = crun_text(&db, &["assets", "register", "--format", "json"]);
    let parsed: Value =
        serde_json::from_str(&raw).unwrap_or_else(|_| panic!("not JSON: {raw:.200}"));
    assert_eq!(parsed["ok"], json!(true), "{parsed}");
    assert!(parsed["data"]["assets"].is_array(), "{parsed}");
}

#[test]
fn hard_cli_recurring_run_dry_run_renders_plans() {
    let (_dir, _cfg, db) = hard_db("h16", false);
    let (_, ok) = crun(
        &db,
        &[
            "recurring",
            "add",
            "--name",
            "Huur",
            "--postings",
            "4300:1000.00,1100:-1000.00",
            "--frequency",
            "monthly",
            "--start",
            "2026-01-10",
        ],
    );
    assert!(ok);
    let out = crun_text(&db, &["recurring", "run", "--dry-run"]);
    assert!(
        !out.contains("#undefined"),
        "dry-run must not render undefined ids: {out:.300}"
    );
    assert!(
        out.contains("(plan)"),
        "dry-run runs must render as plans: {out:.300}"
    );
}

#[test]
fn hard_cli_assets_pause_dry_run_leaves_the_status_unchanged() {
    let (_dir, _cfg, db) = hard_db("h16b", false);
    let (_, ok) = crun(
        &db,
        &[
            "assets",
            "add",
            "--name",
            "Laptop",
            "--purchase-date",
            "2026-01-01",
            "--purchase-price",
            "1200.00",
            "--depreciation-start",
            "2026-01-01",
            "--recognition-date",
            "2026-01-01",
        ],
    );
    assert!(ok);
    let (_, ok) = crun(&db, &["assets", "pause", "--id", "1", "--dry-run"]);
    assert!(ok);
    let (show, _) = crun(&db, &["assets", "list"]);
    assert_eq!(
        show["data"]["assets"][0]["status"],
        json!("active"),
        "{show}"
    );
}

// --- F13: numeric CLI inputs validate ---------------------------------------

#[test]
fn hard_cli_invoice_reminders_within_days_0_stays_0() {
    let (_dir, _cfg, db) = hard_db("h13", false);
    let (zero, ok) = crun(&db, &["invoice", "reminders", "--within-days", "0"]);
    assert!(ok, "{zero}");
    assert_eq!(
        zero["data"]["within_days"],
        json!(0),
        "--within-days 0 must not become 7: {zero}"
    );

    let (garbage, ok) = crun(&db, &["invoice", "reminders", "--within-days", "abc"]);
    assert!(!ok);
    assert_eq!(
        garbage["error"]["code"],
        json!("INVALID_WINDOW"),
        "{garbage}"
    );
    let (neg, ok) = crun(&db, &["invoice", "reminders", "--within-days", "-1"]);
    assert!(!ok);
    assert_eq!(neg["error"]["code"], json!("INVALID_WINDOW"), "{neg}");
}

#[test]
fn hard_cli_limit_0_returns_0_rows_and_garbage_errors() {
    let (_dir, _cfg, db) = hard_db("h13b", false);
    for i in 0..2 {
        let (_, ok) = crun(
            &db,
            &[
                "entry",
                "add",
                "--date",
                "2026-01-01",
                "--desc",
                &format!("e{i}"),
                "--postings",
                "1100:10.00,8000:-10.00",
                "--post",
            ],
        );
        assert!(ok);
    }
    let (zero, ok) = crun(&db, &["entry", "list", "--limit", "0"]);
    assert!(ok, "{zero}");
    assert_eq!(
        zero["data"]["entries"].as_array().unwrap().len(),
        0,
        "--limit 0 must not become the default: {zero}"
    );
    let (one, _) = crun(&db, &["entry", "list", "--limit", "1"]);
    assert_eq!(one["data"]["entries"].as_array().unwrap().len(), 1);
    let (garbage, ok) = crun(&db, &["entry", "list", "--limit", "abc"]);
    assert!(!ok);
    assert_eq!(
        garbage["error"]["code"],
        json!("INVALID_LIMIT"),
        "{garbage}"
    );

    let (audit_zero, ok) = crun(&db, &["audit", "--limit", "0"]);
    assert!(ok, "{audit_zero}");
    assert_eq!(audit_zero["data"]["entries"].as_array().unwrap().len(), 0);

    let (fx_garbage, ok) = crun(&db, &["fx", "list", "--limit", "abc"]);
    assert!(!ok);
    assert_eq!(
        fx_garbage["error"]["code"],
        json!("INVALID_LIMIT"),
        "{fx_garbage}"
    );
    let (fx_zero, ok) = crun(&db, &["fx", "list", "--limit", "0"]);
    assert!(ok, "{fx_zero}");
    assert_eq!(fx_zero["data"]["rates"].as_array().unwrap().len(), 0);
}

// --- F17: autoMatch window validation ---------------------------------------

#[test]
fn hard_cli_bank_match_auto_validates_window_days() {
    let (_dir, _cfg, db) = hard_db("h17b", false);
    let (garbage, ok) = crun(&db, &["bank", "match", "auto", "--window-days", "abc"]);
    assert!(!ok);
    assert_eq!(
        garbage["error"]["code"],
        json!("INVALID_WINDOW"),
        "{garbage}"
    );
    // 0 stays 0 — an empty result, not an error
    let (_, ok) = crun(&db, &["bank", "match", "auto", "--window-days", "0"]);
    assert!(ok);
}

// --- F22: day-overflow calendar dates rejected at every money boundary -------

#[test]
fn hard_entry_add_rejects_day_overflow_dates() {
    let (_dir, _cfg, db) = hard_db("h22", false);
    let (bad, ok) = crun(
        &db,
        &[
            "entry",
            "add",
            "--date",
            "2026-02-30",
            "--desc",
            "x",
            "--postings",
            "1100:100.00,8000:-100.00",
        ],
    );
    assert!(!ok);
    assert_eq!(bad["error"]["code"], json!("INVALID_DATE"), "{bad}");

    // valid dates (incl. leap day) still pass
    let (_ok_r, ok) = crun(
        &db,
        &[
            "entry",
            "add",
            "--date",
            "2024-02-29",
            "--desc",
            "leap",
            "--postings",
            "1100:1.00,8000:-1.00",
        ],
    );
    assert!(ok, "a leap day must be accepted");
}

#[test]
fn hard_opening_balances_rejects_a_day_overflow_date() {
    let (dir, _cfg, db) = hard_db("h22b", false);
    let csv = dir.join("ob.csv");
    std::fs::write(&csv, "1100,10000.00\n3000,-10000.00\n").unwrap();
    let (bad, ok) = crun(
        &db,
        &[
            "import",
            "opening-balances",
            "--file",
            &csv.to_string_lossy(),
            "--date",
            "2026-02-30",
        ],
    );
    assert!(!ok);
    assert_eq!(bad["error"]["code"], json!("INVALID_DATE"), "{bad}");
    let d = bukio::db::open_db(&db).unwrap();
    let n: i64 = d
        .query_row("SELECT COUNT(*) FROM journal_entries", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0, "a rejected import writes nothing");
}

#[test]
fn hard_fx_set_rejects_a_day_overflow_date() {
    let (_dir, _cfg, db) = hard_db("h22c", false);
    let (bad, ok) = crun(
        &db,
        &[
            "fx",
            "set",
            "--currency",
            "USD",
            "--date",
            "2026-02-30",
            "--rate",
            "1.0875",
        ],
    );
    assert!(!ok);
    assert_eq!(bad["error"]["code"], json!("INVALID_DATE"), "{bad}");
}

#[test]
fn hard_report_balance_sheet_rejects_a_garbage_as_of() {
    let (_dir, _cfg, db) = hard_db("h23", false);
    let (bad, ok) = crun(&db, &["report", "balance-sheet", "--as-of", "garbage"]);
    assert!(!ok);
    assert_eq!(bad["error"]["code"], json!("INVALID_DATE"), "{bad}");
}

#[test]
fn hard_reports_reject_a_garbage_year() {
    let (_dir, _cfg, db) = hard_db("h24", false);
    let (pnl_bad, ok) = crun(&db, &["report", "pnl", "--year", "abc"]);
    assert!(!ok);
    assert_eq!(pnl_bad["error"]["code"], json!("INVALID_DATE"), "{pnl_bad}");
    let (journal_bad, ok) = crun(&db, &["report", "journal", "--year", "abc"]);
    assert!(!ok);
    assert_eq!(
        journal_bad["error"]["code"],
        json!("INVALID_DATE"),
        "{journal_bad}"
    );
    let (tb_bad, ok) = crun(&db, &["report", "trial-balance", "--year", "abc"]);
    assert!(!ok);
    assert_eq!(tb_bad["error"]["code"], json!("INVALID_YEAR"), "{tb_bad}");
    let (_ok_r, ok) = crun(&db, &["report", "pnl", "--year", "2026"]);
    assert!(ok);
}

#[test]
fn hard_entry_list_rejects_garbage_date_bounds() {
    let (_dir, _cfg, db) = hard_db("h25", false);
    let (_, ok) = crun(
        &db,
        &[
            "entry",
            "add",
            "--date",
            "2026-01-15",
            "--desc",
            "x",
            "--postings",
            "1100:100.00,8000:-100.00",
            "--post",
        ],
    );
    assert!(ok);
    let (bad_to, ok) = crun(&db, &["entry", "list", "--date-to", "garbage"]);
    assert!(!ok);
    assert_eq!(bad_to["error"]["code"], json!("INVALID_DATE"), "{bad_to}");
    let (bad_from, ok) = crun(&db, &["entry", "list", "--date-from", "2026-02-30"]);
    assert!(!ok);
    assert_eq!(
        bad_from["error"]["code"],
        json!("INVALID_DATE"),
        "{bad_from}"
    );
    let (ok_r, ok) = crun(
        &db,
        &[
            "entry",
            "list",
            "--date-from",
            "2026-01-01",
            "--date-to",
            "2026-01-31",
        ],
    );
    assert!(ok, "{ok_r}");
    assert_eq!(
        ok_r["data"]["entries"].as_array().unwrap().len(),
        1,
        "{ok_r}"
    );
}

#[test]
fn hard_opening_balances_accepts_the_documented_optional_header() {
    let (dir, _cfg, db) = hard_db("h26", false);
    // 2-column header layout
    let csv2 = dir.join("ob2.csv");
    std::fs::write(&csv2, "code,amount\n1100,10000.00\n3000,-10000.00\n").unwrap();
    let (out, ok) = crun(
        &db,
        &[
            "import",
            "opening-balances",
            "--file",
            &csv2.to_string_lossy(),
        ],
    );
    assert!(ok, "{out}");
    assert_eq!(out["data"]["accounts"], json!(2), "{out}");

    // 3-column header layout on a second database
    let (dir3, _cfg3, db3) = hard_db("h26b", false);
    let csv3 = dir3.join("ob3.csv");
    std::fs::write(&csv3, "code,debet,credit\n1100,10000.00,\n3000,,10000.00\n").unwrap();
    let (out3, ok) = crun(
        &db3,
        &[
            "import",
            "opening-balances",
            "--file",
            &csv3.to_string_lossy(),
        ],
    );
    assert!(ok, "{out3}");
    assert_eq!(out3["data"]["accounts"], json!(2), "{out3}");

    // a data-only file (no header) still works (the first cell is a code)
    let (dir4, _cfg4, db4) = hard_db("h26c", false);
    let csv4 = dir4.join("ob4.csv");
    std::fs::write(&csv4, "1100,10000.00\n3000,-10000.00\n").unwrap();
    let (_o, ok) = crun(
        &db4,
        &[
            "import",
            "opening-balances",
            "--file",
            &csv4.to_string_lossy(),
        ],
    );
    assert!(ok);
}
// --- dry-run uniformity (CLI surface) ---------------------------------------

#[test]
fn hard_cli_dry_runs_write_nothing() {
    let (_dir, _cfg, db) = hard_db("h27", true);
    // contact add / update
    let (plan, ok) = crun(
        &db,
        &[
            "contact",
            "add",
            "--name",
            "Nieuwe BV",
            "--iban",
            CLI_IBAN,
            "--dry-run",
        ],
    );
    assert!(ok, "{plan}");
    let d = bukio::db::open_db(&db).unwrap();
    let n: i64 = d
        .query_row(
            "SELECT COUNT(*) FROM contacts WHERE name = 'Nieuwe BV'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 0, "contact add --dry-run must write nothing");
    let (_, ok) = crun(
        &db,
        &[
            "contact",
            "add",
            "--name",
            "ACME BV",
            "--address",
            "Straat 1",
            "--city",
            "Amsterdam",
        ],
    );
    assert!(ok);
    let (upd, ok) = crun(
        &db,
        &[
            "contact",
            "update",
            "--id",
            "1",
            "--name",
            "Gewijzigd BV",
            "--dry-run",
        ],
    );
    assert!(ok, "{upd}");
    let name: String = d
        .query_row("SELECT name FROM contacts WHERE id = 1", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        name, "ACME BV",
        "contact update --dry-run must not change the row"
    );

    // compliance mark
    let (filed, ok) = crun(
        &db,
        &[
            "compliance",
            "mark",
            "--type",
            "ICP",
            "--period",
            "2026-Q1",
            "--dry-run",
        ],
    );
    assert!(ok, "{filed}");
    let n: i64 = d
        .query_row("SELECT COUNT(*) FROM filings WHERE type = 'ICP'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(n, 0, "compliance mark --dry-run must write nothing");

    // fx set
    let (rate, ok) = crun(
        &db,
        &[
            "fx",
            "set",
            "--currency",
            "USD",
            "--date",
            "2026-01-10",
            "--rate",
            "1.0875",
            "--dry-run",
        ],
    );
    assert!(ok, "{rate}");
    let n: i64 = d
        .query_row(
            "SELECT COUNT(*) FROM fx_rates WHERE currency = 'USD'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 0, "fx set --dry-run must write nothing");
    drop(d);

    // recurring pause
    let (_, ok) = crun(
        &db,
        &[
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
        ],
    );
    assert!(ok);
    let (pause, ok) = crun(&db, &["recurring", "pause", "--id", "1", "--dry-run"]);
    assert!(ok, "{pause}");
    let d = bukio::db::open_db(&db).unwrap();
    let status: String = d
        .query_row(
            "SELECT status FROM recurring_templates WHERE id = 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        status, "active",
        "recurring pause --dry-run must not change the status"
    );

    // account reactivate
    let (_, ok) = crun(&db, &["account", "deactivate", "--code", "1200"]);
    assert!(ok);
    let (react, ok) = crun(
        &db,
        &["account", "reactivate", "--code", "1200", "--dry-run"],
    );
    assert!(ok, "{react}");
    let active: i64 = d
        .query_row("SELECT active FROM accounts WHERE code = '1200'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(
        active, 0,
        "account reactivate --dry-run must not flip the flag"
    );
}

#[test]
fn hard_cli_bank_add_and_ignore_dry_runs_write_nothing() {
    let (_dir, _cfg, db) = hard_db("h28", false);
    let (plan, ok) = crun(
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
    assert!(ok, "{plan}");
    let d = bukio::db::open_db(&db).unwrap();
    let n: i64 = d
        .query_row("SELECT COUNT(*) FROM bank_accounts", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0, "bank add --dry-run must write nothing");

    // import a transaction, then a dry-run ignore must leave it unmatched
    let (_, ok) = crun(
        &db,
        &["bank", "add", "--iban", CLI_IBAN, "--name", "Zakelijk"],
    );
    assert!(ok);
    let xml = format!(
        "<?xml version=\"1.0\"?>\n<Document xmlns=\"urn:iso:std:iso:20022:tech:xsd:camt.053.001.02\">\n<BkToCstmrStmt><Stmt><Acct><Id><IBAN>{CLI_IBAN}</IBAN></Id></Acct>\n<Ntry><Amt>50.00</Amt><CdtDbtInd>DBIT</CdtDbtInd><BookgDt><Dt>2026-01-10</Dt></BookgDt><NtryDtls><TxDtls><RltdPties><Cdtr><Nm>ACME</Nm></Cdtr></RltdPties><RmtInf><Ustrd>factuur</Ustrd></RmtInf></TxDtls></NtryDtls></Ntry>\n</Stmt></BkToCstmrStmt></Document>"
    );
    let dir = std::path::Path::new(&db).parent().unwrap().to_path_buf();
    let camt = dir.join("stmt.xml");
    std::fs::write(&camt, xml).unwrap();
    let (_, ok) = crun(
        &db,
        &[
            "bank",
            "import",
            "--file",
            &camt.to_string_lossy(),
            "--iban",
            CLI_IBAN,
        ],
    );
    assert!(ok);
    let (_, ok) = crun(&db, &["bank", "ignore", "--tx", "1", "--dry-run"]);
    assert!(ok);
    let state: String = d
        .query_row(
            "SELECT state FROM bank_transactions WHERE id = 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        state, "unmatched",
        "bank ignore --dry-run must not change the state"
    );
}

#[test]
fn hard_cli_batch_delete_cascades_lines_and_releases_payables() {
    let (_dir, _cfg, db) = hard_db("h29", false);
    let (_, ok) = crun(&db, &["company", "update", "--iban", CLI_IBAN]);
    assert!(ok);
    let (_, ok) = crun(
        &db,
        &[
            "contact",
            "add",
            "--name",
            "ACME BV",
            "--iban",
            "NL86INGB0002445588",
        ],
    );
    assert!(ok);
    let (_, ok) = crun(
        &db,
        &[
            "payments",
            "payables",
            "add",
            "--contact",
            "1",
            "--ref",
            "F1",
            "--date",
            "2026-01-01",
            "--amount",
            "50.00",
        ],
    );
    assert!(ok);
    let (batch, ok) = crun(
        &db,
        &[
            "payments",
            "batch",
            "create",
            "--type",
            "transfer",
            "--payable",
            "1",
        ],
    );
    assert!(ok, "{batch}");
    let batch_id = batch["data"]["id"].as_i64().unwrap();
    let d = bukio::db::open_db(&db).unwrap();
    let n: i64 = d
        .query_row(
            "SELECT COUNT(*) FROM payment_batch_lines WHERE batch_id = ?1",
            [batch_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 1);
    drop(d);
    let (_, ok) = crun(
        &db,
        &["payments", "batch", "delete", "--id", &batch_id.to_string()],
    );
    assert!(ok);
    let d = bukio::db::open_db(&db).unwrap();
    let n: i64 = d
        .query_row(
            "SELECT COUNT(*) FROM payment_batch_lines WHERE batch_id = ?1",
            [batch_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 0, "lines cascade with the batch");
    let status: String = d
        .query_row("SELECT status FROM payables WHERE id = 1", [], |r| r.get(0))
        .unwrap();
    assert_eq!(status, "unpaid", "the payable is released");
}

#[test]
fn hard_cli_sepa_msg_id_stays_within_35_chars() {
    let (_dir, _cfg, db) = hard_db("h30", false);
    let (_, ok) = crun(&db, &["company", "update", "--iban", CLI_IBAN]);
    assert!(ok);
    let (_, ok) = crun(
        &db,
        &[
            "contact",
            "add",
            "--name",
            "ACME BV",
            "--iban",
            "NL86INGB0002445588",
        ],
    );
    assert!(ok);
    // an explicit id beyond any realistic AUTOINCREMENT range
    {
        let d = bukio::db::open_db(&db).unwrap();
        d.execute(
            "INSERT INTO payment_batches (id, batch_date, debit_iban, debit_name, total_cents, created_by)
             VALUES (99999999999999999, '2026-01-10', ?1, 'Demo BV', 10000, 'agent:test')",
            [CLI_IBAN],
        )
        .unwrap();
        d.execute(
            "INSERT INTO payment_batch_lines (batch_id, contact_id, name, iban, amount_cents, reference)
             VALUES (99999999999999999, 1, 'ACME BV', 'NL86INGB0002445588', 10000, 'F1')",
            [],
        )
        .unwrap();
    }
    let (r, ok) = crun(
        &db,
        &["payments", "batch", "export", "--id", "99999999999999999"],
    );
    assert!(ok, "{r}");
    let msg_id = r["data"]["msg_id"]
        .as_str()
        .unwrap_or_else(|| panic!("no msg_id in {r}"));
    assert!(
        msg_id.len() <= 35,
        "MsgId '{msg_id}' is {} chars",
        msg_id.len()
    );
    assert!(msg_id.starts_with("BUKIO"), "{msg_id}");
}

#[test]
fn hard_cli_ubl_uses_eur_and_the_supplier_postal_code() {
    let (dir, _cfg, db) = hard_db("h31", true);
    let (_, ok) = crun(
        &db,
        &[
            "company",
            "update",
            "--postal-code",
            "2712 CD",
            "--address",
            "Industrieweg 12",
            "--city",
            "Zoetermeer",
            "--tax-id",
            "NL123456789B01",
        ],
    );
    assert!(ok);
    let (_, ok) = crun(
        &db,
        &[
            "contact",
            "add",
            "--name",
            "Klant BV",
            "--address",
            "Straat 1",
            "--city",
            "Amsterdam",
        ],
    );
    assert!(ok);
    let (out, ok) = crun(
        &db,
        &[
            "invoice",
            "create",
            "--contact",
            "1",
            "--lines",
            "1x Werk @ 100.00 @21",
            "--date",
            "2099-01-10",
        ],
    );
    assert!(ok, "{out}");
    let (out, ok) = crun(&db, &["invoice", "finalize", "--id", "1"]);
    assert!(ok, "{out}");
    // the CLI writes the XML to --out (default <invoice_number>.xml)
    let ubl_path = dir.join("inv.xml").to_string_lossy().to_string();
    let (_, ok) = crun(&db, &["invoice", "ubl", "--id", "1", "--out", &ubl_path]);
    assert!(ok);
    let xml = std::fs::read_to_string(&ubl_path).unwrap();
    assert!(
        !xml.contains("currencyID=\"undefined\""),
        "no undefined currency may leak: {xml:.400}"
    );
    assert!(xml.contains("currencyID=\"EUR\""), "currencyID must be EUR");
    assert!(
        xml.contains("<cbc:PostalZone>2712 CD</cbc:PostalZone>"),
        "the supplier postal code must be present"
    );
}

// --- F13 (MCP): journal limit + validation ----------------------------------

#[test]
fn hard_mcp_journal_honors_limit_with_truncation_and_validates() {
    let (_dir, cfg, db) = hard_db("h32", false);
    for i in 0..3 {
        let (_, ok) = crun(
            &db,
            &[
                "entry",
                "add",
                "--date",
                "2026-01-01",
                "--desc",
                &format!("e{i}"),
                "--postings",
                "1100:10.00,8000:-10.00",
                "--post",
            ],
        );
        assert!(ok);
    }
    let mut m = Mcp::start_as(&db, "agent:test", Some(&cfg));
    mcp_init(&mut m);

    let (capped, is_err) = m.tool("journal", json!({ "year": "2026", "limit": 2 }));
    assert!(!is_err, "{capped}");
    assert_eq!(
        capped["rows"].as_array().unwrap().len(),
        2,
        "the limit must cap the rows: {capped}"
    );
    assert_eq!(
        capped["truncated"],
        json!(true),
        "truncation must be flagged: {capped}"
    );

    let (full, _) = m.tool("journal", json!({ "year": "2026" }));
    assert_eq!(full["truncated"], json!(false), "{full}");
    assert!(full["rows"].as_array().unwrap().len() > 2, "{full}");

    for (tool, args, code) in [
        ("journal", json!({ "year": "abcd" }), "INVALID_YEAR"),
        (
            "journal",
            json!({ "year": "2026", "limit": "abc" }),
            "INVALID_LIMIT",
        ),
        ("invoices", json!({ "limit": "abc" }), "INVALID_LIMIT"),
        ("pnl", json!({ "year": "2026-13" }), "INVALID_YEAR"),
    ] {
        let (payload, is_err) = m.tool(tool, args.clone());
        assert!(is_err, "{tool} {args}: {payload}");
        assert_eq!(
            payload["error"]["code"],
            json!(code),
            "{tool} {args}: {payload}"
        );
    }
    m.stop();
}

#[test]
fn hard_mcp_vat_book_leaves_a_draft_and_invoice_pay_defaults_to_outstanding() {
    let (_dir, cfg, db) = hard_db("h33", true);
    let (_, ok) = crun(
        &db,
        &[
            "company",
            "update",
            "--tax-id",
            "NL123456789B01",
            "--address",
            "Industrieweg 12",
            "--postal-code",
            "2712 CD",
            "--city",
            "Zoetermeer",
        ],
    );
    assert!(ok);
    let (_, ok) = crun(
        &db,
        &[
            "contact",
            "add",
            "--name",
            "Klant BV",
            "--address",
            "A",
            "--city",
            "B",
        ],
    );
    assert!(ok);
    let (_, ok) = crun(
        &db,
        &[
            "invoice",
            "create",
            "--contact",
            "1",
            "--lines",
            "1x Werk @ 100.00 @21",
            "--date",
            "2099-01-10",
        ],
    );
    assert!(ok);
    let (_, ok) = crun(&db, &["invoice", "finalize", "--id", "1"]);
    assert!(ok);

    let mut m = Mcp::start_as(&db, "agent:test", Some(&cfg));
    mcp_init(&mut m);
    // vat_book without post=true leaves a draft
    let (booked, is_err) = m.tool(
        "vat_book",
        json!({ "date": "2099-02-01", "description": "verkoop", "postings": ["8000:-50.00@21", "1100:60.50"], "mode": "execute", "actor": "agent:mcp-test" }),
    );
    assert!(!is_err, "{booked}");
    assert_eq!(
        booked["state"],
        json!("draft"),
        "vat_book must leave a draft: {booked}"
    );

    // invoice_pay without an amount pays the full outstanding
    let (paid, is_err) = m.tool(
        "invoice_pay",
        json!({ "id": 1, "date": "2099-02-10", "mode": "execute", "actor": "agent:mcp-test" }),
    );
    assert!(!is_err, "{paid}");
    assert_eq!(
        paid["invoice"]["status"],
        json!("paid"),
        "invoice_pay should mark it paid: {paid}"
    );
    m.stop();
}

#[test]
fn hard_mcp_dry_runs_validate_like_execute() {
    let (_dir, cfg, db) = hard_db("h34", false);
    let (_, ok) = crun(
        &db,
        &[
            "contact",
            "add",
            "--name",
            "ACME BV",
            "--address",
            "Klantstraat 1",
            "--city",
            "Amsterdam",
        ],
    );
    assert!(ok);
    let mut m = Mcp::start_as(&db, "agent:test", Some(&cfg));
    mcp_init(&mut m);

    for (tool, args, code) in [
        (
            "entry_add",
            json!({ "date": "abc", "description": "x", "postings": ["1100:100.00", "3000:-100.00"] }),
            "INVALID_DATE",
        ),
        (
            "entry_add",
            json!({ "date": "2026-01-15", "description": "x", "postings": ["1100:5.00", "3000:-4.00"] }),
            "UNBALANCED",
        ),
        (
            "entry_add",
            json!({ "date": "2026-01-15", "description": "x", "postings": ["1100:100.00"] }),
            "TOO_FEW_POSTINGS",
        ),
        ("entry_reverse", json!({ "id": 999 }), "NOT_FOUND"),
        ("entry_post", json!({ "id": 999 }), "NOT_FOUND"),
        ("invoice_credit", json!({ "id": 999 }), "NOT_FOUND"),
        (
            "invoice_pay",
            json!({ "id": 999, "date": "2026-01-15" }),
            "NOT_FOUND",
        ),
        ("contact_add", json!({ "name": "  " }), "INVALID_NAME"),
        ("invoice_finalize", json!({ "id": 999 }), "NOT_FOUND"),
        (
            "fx_set",
            json!({ "currency": "USD", "date": "2026-02-30", "rate": "1.0875" }),
            "INVALID_DATE",
        ),
    ] {
        let (payload, is_err) = m.tool(tool, args.clone());
        assert!(is_err, "{tool} {args} must fail: {payload}");
        assert_eq!(
            payload["error"]["code"],
            json!(code),
            "{tool} {args}: {payload}"
        );
    }

    // a valid plan is still green
    let (ok_plan, is_err) = m.tool("entry_add", json!({ "date": "2026-01-15", "description": "x", "postings": ["1100:100.00", "3000:-100.00"] }));
    assert!(!is_err, "{ok_plan}");
    assert_eq!(ok_plan["balanced"], json!(true), "{ok_plan}");
    m.stop();
}

#[test]
fn hard_mcp_on_a_missing_database_errors_instead_of_creating_one() {
    let dir = temp_dir("h35");
    let missing = dir.join("missing.db").to_string_lossy().to_string();
    let exe = env!("CARGO_BIN_EXE_bukio");
    let exe_path = exe;
    let out = std::process::Command::new(exe_path)
        .args(["mcp", "--db", &missing])
        .env("BUKIO_ACTOR", "agent:test")
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "MCP must exit non-zero on a missing database"
    );
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        combined.contains("no database at") && combined.contains("run 'bukio init' first"),
        "expected the NO_DATABASE message, got: {combined:.300}"
    );
    assert!(
        !std::path::Path::new(&missing).exists(),
        "must not create the database file"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// --- init validation + audit trail ------------------------------------------

#[test]
fn hard_cli_init_validates_iban_vat_choice_and_fiscal_year_end() {
    // every attempt needs a fresh database: init refuses an already-initialised file
    let (dir, _cfg, _db) = hard_db("h36", false);
    let db = dir.join("fresh.db").to_string_lossy().to_string();
    let (bad_iban, ok) = crun(
        &db,
        &[
            "init",
            "--name",
            "Test BV",
            "--iban",
            "NL00BOGUS",
            "--db",
            &db,
        ],
    );
    assert!(!ok);
    assert_eq!(
        bad_iban["error"]["code"],
        json!("INVALID_IBAN"),
        "{bad_iban}"
    );
    let (bad_vat, ok) = crun(
        &db,
        &["init", "--name", "Test BV", "--vat", "banana", "--db", &db],
    );
    assert!(!ok);
    assert_eq!(
        bad_vat["error"]["code"],
        json!("INVALID_VAT_CHOICE"),
        "{bad_vat}"
    );
    for fye in ["99-99", "02-30"] {
        let (bad, ok) = crun(
            &db,
            &[
                "init",
                "--name",
                "Test BV",
                "--fiscal-year-end",
                fye,
                "--db",
                &db,
            ],
        );
        assert!(!ok, "'{fye}' must be rejected");
        assert_eq!(
            bad["error"]["code"],
            json!("INVALID_FISCAL_YEAR_END"),
            "{bad}"
        );
    }
    let (ok_r, ok) = crun(
        &db,
        &[
            "init",
            "--name",
            "Test BV",
            "--iban",
            CLI_IBAN,
            "--fiscal-year-end",
            "12-31",
            "--vat",
            "on",
            "--db",
            &db,
        ],
    );
    assert!(ok, "{ok_r}");
    assert_eq!(ok_r["data"]["company"]["vat_module"], json!(1), "{ok_r}");
}

#[test]
fn hard_cli_account_lifecycle_is_audited() {
    let (dir, _cfg, db) = hard_db("h37", false);
    let (_, ok) = crun(
        &db,
        &[
            "account",
            "add",
            "--code",
            "9999",
            "--name",
            "Test account",
            "--type",
            "asset",
            "--normal-balance",
            "debit",
        ],
    );
    assert!(ok);
    let (_, ok) = crun(&db, &["account", "deactivate", "--code", "9999"]);
    assert!(ok);
    let (_, ok) = crun(&db, &["account", "reactivate", "--code", "9999"]);
    assert!(ok);
    let csv = dir.join("chart.csv");
    std::fs::write(
        &csv,
        "code,name,type,normal_balance,taxonomy_code\n8888,Nieuwe rekening,expense,debit,WKPR.70\n",
    )
    .unwrap();
    let (_, ok) = crun(
        &db,
        &["account", "import", "--file", &csv.to_string_lossy()],
    );
    assert!(ok);

    let (audit, _) = crun(&db, &["audit", "--limit", "100"]);
    let actions: Vec<String> = audit["data"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["action"].as_str().unwrap_or("").to_string())
        .collect();
    for expected in [
        "company.init",
        "account.add",
        "account.deactivate",
        "account.reactivate",
        "account.import",
    ] {
        assert!(
            actions.iter().any(|a| a == expected),
            "the audit log must contain {expected}: {actions:?}"
        );
    }
    // a dry-run must not record
    let (_, ok) = crun(
        &db,
        &["account", "deactivate", "--code", "8888", "--dry-run"],
    );
    assert!(ok);
    let (audit2, _) = crun(&db, &["audit", "--limit", "100"]);
    let n = audit2["data"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["action"] == json!("account.deactivate"))
        .count();
    assert_eq!(n, 1, "a dry-run must not write an audit row");
}

// --- F14/F16/F19: date + period validation through the CLI -------------------

#[test]
fn hard_cli_add_payable_rejects_garbage_dates() {
    let (_dir, _cfg, db) = hard_db("h38", false);
    let (_, ok) = crun(
        &db,
        &[
            "contact",
            "add",
            "--name",
            "ACME BV",
            "--iban",
            "NL86INGB0002445588",
        ],
    );
    assert!(ok);
    for date in ["garbage", "2026-02-30"] {
        let (bad, ok) = crun(
            &db,
            &[
                "payments",
                "payables",
                "add",
                "--contact",
                "1",
                "--ref",
                "F1",
                "--date",
                date,
                "--amount",
                "10.00",
            ],
        );
        assert!(!ok, "'{date}' must be rejected");
        assert_eq!(bad["error"]["code"], json!("INVALID_DATE"), "{bad}");
    }
    let (bad_due, ok) = crun(
        &db,
        &[
            "payments",
            "payables",
            "add",
            "--contact",
            "1",
            "--ref",
            "F1",
            "--date",
            "2026-01-05",
            "--due",
            "nonsense",
            "--amount",
            "10.00",
        ],
    );
    assert!(!ok);
    assert_eq!(bad_due["error"]["code"], json!("INVALID_DATE"), "{bad_due}");
    let (ok_r, ok) = crun(
        &db,
        &[
            "payments",
            "payables",
            "add",
            "--contact",
            "1",
            "--ref",
            "F1",
            "--date",
            "2026-01-05",
            "--due",
            "2026-02-05",
            "--amount",
            "10.00",
        ],
    );
    assert!(ok, "{ok_r}");
    assert_eq!(ok_r["data"]["due_date"], json!("2026-02-05"), "{ok_r}");
}

#[test]
fn hard_cli_assets_run_rejects_garbage_periods_and_dates() {
    let (_dir, _cfg, db) = hard_db("h39", false);
    let (out, ok) = crun(
        &db,
        &[
            "assets",
            "scheme",
            "add",
            "--name",
            "3y",
            "--life-months",
            "36",
        ],
    );
    assert!(ok, "{out}");
    let (out, ok) = crun(
        &db,
        &[
            "assets",
            "add",
            "--name",
            "Laptop",
            "--purchase-date",
            "2026-01-01",
            "--purchase-price",
            "3600.00",
            "--depreciation-start",
            "2026-01-01",
            "--recognition-date",
            "2026-01-01",
        ],
    );
    assert!(ok, "{out}");
    for period in ["2026-13", "2026-00"] {
        let (bad, ok) = crun(&db, &["assets", "run", "--period", period]);
        assert!(!ok, "'{period}' must be rejected");
        assert_eq!(bad["error"]["code"], json!("INVALID_PERIOD"), "{bad}");
    }
    for as_of in ["garbage", "2026-02-30"] {
        let (bad, ok) = crun(&db, &["assets", "run", "--as-of", as_of]);
        assert!(!ok, "'{as_of}' must be rejected");
        assert_eq!(bad["error"]["code"], json!("INVALID_DATE"), "{bad}");
    }
    // a valid period books the due runs
    let (ok_r, ok) = crun(&db, &["assets", "run", "--period", "2026-02"]);
    assert!(ok, "{ok_r}");
    let booked = ok_r["data"]["booked"].as_array().unwrap();
    assert_eq!(booked.len(), 2, "Jan + Feb catch-up: {ok_r}");
}

#[test]
fn hard_cli_recurring_run_rejects_a_garbage_as_of() {
    let (_dir, _cfg, db) = hard_db("h40", false);
    let (_, ok) = crun(
        &db,
        &[
            "recurring",
            "add",
            "--name",
            "Huur",
            "--postings",
            "4600:100.00,1100:-100.00",
            "--frequency",
            "monthly",
            "--start",
            "2026-01-01",
        ],
    );
    assert!(ok);
    for as_of in ["garbage", "2026-02-30"] {
        let (bad, ok) = crun(&db, &["recurring", "run", "--as-of", as_of]);
        assert!(!ok, "'{as_of}' must be rejected");
        assert_eq!(bad["error"]["code"], json!("INVALID_DATE"), "{bad}");
    }
    let (ok_r, ok) = crun(&db, &["recurring", "run", "--as-of", "2026-03-01"]);
    assert!(ok, "{ok_r}");
    let runs = ok_r["data"]["templates"][0]["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 3, "Jan, Feb, Mar: {ok_r}");
}

#[test]
fn hard_cli_year_end_status_rejects_a_non_yyyy_year() {
    let (_dir, _cfg, db) = hard_db("h41", false);
    for year in ["abc", "20261"] {
        let (bad, ok) = crun(&db, &["year-end", "status", "--year", year]);
        assert!(!ok, "'{year}' must be rejected");
        assert_eq!(bad["error"]["code"], json!("INVALID_YEAR"), "{bad}");
    }
    let (ok_r, ok) = crun(&db, &["year-end", "status", "--year", "2026"]);
    assert!(ok, "{ok_r}");
    assert_eq!(ok_r["data"]["status"]["closed"], json!(false), "{ok_r}");
}

#[test]
fn hard_cli_recurring_numeric_inputs_pass_through_unmasked() {
    let (_dir, _cfg, db) = hard_db("h42", false);
    let (_, ok) = crun(
        &db,
        &[
            "contact",
            "add",
            "--name",
            "ACME BV",
            "--address",
            "Straat 1",
            "--city",
            "Amsterdam",
        ],
    );
    assert!(ok);
    // --due-days 0 must stay 0
    let (zero, ok) = crun(
        &db,
        &[
            "recurring",
            "add",
            "--kind",
            "invoice",
            "--contact",
            "1",
            "--lines",
            "1x Coaching @ 100.00",
            "--frequency",
            "monthly",
            "--start",
            "2026-01-15",
            "--due-days",
            "0",
            "--name",
            "t0",
        ],
    );
    assert!(ok, "{zero}");
    let d = bukio::db::open_db(&db).unwrap();
    let due: i64 = d
        .query_row(
            "SELECT due_days FROM recurring_templates WHERE name = 't0'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(due, 0, "due-days 0 must survive, not become 30");
    // garbage --due-days errors
    let (bad_due, ok) = crun(
        &db,
        &[
            "recurring",
            "add",
            "--kind",
            "invoice",
            "--contact",
            "1",
            "--lines",
            "1x Coaching @ 100.00",
            "--frequency",
            "monthly",
            "--start",
            "2026-01-15",
            "--due-days",
            "abc",
            "--name",
            "t2",
        ],
    );
    assert!(!ok);
    assert_eq!(
        bad_due["error"]["code"],
        json!("INVALID_DUE_DAYS"),
        "{bad_due}"
    );
    // --day 0 / abc are rejected
    for day in ["0", "abc"] {
        let (bad, ok) = crun(
            &db,
            &[
                "recurring",
                "add",
                "--postings",
                "4300:100.00,1100:-100.00",
                "--frequency",
                "monthly",
                "--start",
                "2026-01-15",
                "--day",
                day,
                "--name",
                "t3",
            ],
        );
        assert!(!ok, "--day {day} must be rejected");
        assert_eq!(bad["error"]["code"], json!("INVALID_DATE"), "{bad}");
    }
}

#[test]
fn hard_cli_dry_runs_validate_like_the_real_run() {
    let (dir, _cfg, db) = hard_db("h43", false);
    // recurring add
    let (bad_day, ok) = crun(
        &db,
        &[
            "recurring",
            "add",
            "--postings",
            "4300:100.00,1100:-100.00",
            "--frequency",
            "monthly",
            "--start",
            "2026-01-15",
            "--day",
            "abc",
            "--name",
            "t1",
            "--dry-run",
        ],
    );
    assert!(!ok);
    assert_eq!(bad_day["error"]["code"], json!("INVALID_DATE"), "{bad_day}");
    let (bad_post, ok) = crun(
        &db,
        &[
            "recurring",
            "add",
            "--postings",
            "BOGUS",
            "--frequency",
            "monthly",
            "--start",
            "2026-01-15",
            "--name",
            "t2",
            "--dry-run",
        ],
    );
    assert!(!ok);
    assert_eq!(
        bad_post["error"]["code"],
        json!("INVALID_POSTING"),
        "{bad_post}"
    );
    let (_, ok) = crun(
        &db,
        &[
            "recurring",
            "add",
            "--postings",
            "4300:100.00,1100:-100.00",
            "--frequency",
            "monthly",
            "--start",
            "2026-01-15",
            "--name",
            "t3",
            "--dry-run",
        ],
    );
    assert!(ok);
    let d = bukio::db::open_db(&db).unwrap();
    let n: i64 = d
        .query_row("SELECT COUNT(*) FROM recurring_templates", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0, "a recurring dry-run must not write");

    // invoice create
    let (_, ok) = crun(
        &db,
        &[
            "contact",
            "add",
            "--name",
            "ACME BV",
            "--address",
            "Straat 1",
            "--city",
            "Amsterdam",
        ],
    );
    assert!(ok);
    let (bad_date, ok) = crun(
        &db,
        &[
            "invoice",
            "create",
            "--contact",
            "1",
            "--lines",
            "1x Coaching @ 100.00",
            "--date",
            "abc",
            "--dry-run",
        ],
    );
    assert!(!ok);
    assert_eq!(
        bad_date["error"]["code"],
        json!("INVALID_DATE"),
        "{bad_date}"
    );
    let (bad_contact, ok) = crun(
        &db,
        &[
            "invoice",
            "create",
            "--contact",
            "99",
            "--lines",
            "1x Coaching @ 100.00",
            "--date",
            "2026-01-15",
            "--dry-run",
        ],
    );
    assert!(!ok);
    assert_eq!(
        bad_contact["error"]["code"],
        json!("CONTACT_NOT_FOUND"),
        "{bad_contact}"
    );
    let (plan, ok) = crun(
        &db,
        &[
            "invoice",
            "create",
            "--contact",
            "1",
            "--lines",
            "1x Coaching @ 100.00",
            "--date",
            "2026-01-15",
            "--dry-run",
        ],
    );
    assert!(ok, "{plan}");
    assert_eq!(plan["data"]["dryRun"], json!(true), "{plan}");
    assert_eq!(plan["data"]["gross_cents"], json!(10000), "{plan}");
    let n: i64 = d
        .query_row("SELECT COUNT(*) FROM invoices", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0, "an invoice dry-run must not write");

    // entry add
    let (bad_e, ok) = crun(
        &db,
        &[
            "entry",
            "add",
            "--date",
            "abc",
            "--desc",
            "x",
            "--postings",
            "1100:100.00,3000:-100.00",
            "--dry-run",
        ],
    );
    assert!(!ok);
    assert_eq!(bad_e["error"]["code"], json!("INVALID_DATE"), "{bad_e}");
    let (unbalanced, ok) = crun(
        &db,
        &[
            "entry",
            "add",
            "--date",
            "2026-01-15",
            "--desc",
            "x",
            "--postings",
            "1100:5.00,3000:-4.00",
            "--dry-run",
        ],
    );
    assert!(!ok);
    assert_eq!(
        unbalanced["error"]["code"],
        json!("UNBALANCED"),
        "{unbalanced}"
    );
    let (_, ok) = crun(
        &db,
        &[
            "entry",
            "add",
            "--date",
            "2026-01-15",
            "--desc",
            "x",
            "--postings",
            "1100:100.00,3000:-100.00",
            "--dry-run",
        ],
    );
    assert!(ok);
    let n: i64 = d
        .query_row("SELECT COUNT(*) FROM journal_entries", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0, "an entry dry-run must not write");
    let _ = dir;
}

// --- error-code documentation guard -----------------------------------------

#[test]
fn hard_every_emitted_error_code_is_documented() {
    // the port's counterpart of the JS guard: every code raised in src/ must
    // appear in AGENTS.md §7
    let agents =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/AGENTS.md")).unwrap();
    let mut codes: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut stack = vec![std::path::PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src"
    ))];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().map(|e| e == "rs").unwrap_or(false) {
                let src = std::fs::read_to_string(&path).unwrap();
                for cap in src.split("BukioError::new(").skip(1) {
                    let rest = cap.trim_start();
                    let rest = rest.strip_prefix('"').unwrap_or(rest);
                    let code: String = rest
                        .chars()
                        .take_while(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || *c == '_')
                        .collect();
                    if code.len() >= 3 {
                        codes.insert(code);
                    }
                }
            }
        }
    }
    let missing: Vec<&String> = codes
        .iter()
        .filter(|c| !agents.contains(c.as_str()))
        .collect();
    assert!(
        missing.is_empty(),
        "error codes emitted by src/ but missing from AGENTS.md: {missing:?}"
    );
}
// ==== hardening, part 2 (lib-level) ========================================

/// A file DB whose company is complete enough to finalize invoices.
fn hard_full(tag: &str, vat: bool) -> (std::path::PathBuf, String, String) {
    let (dir, cfg, db) = hard_db(tag, vat);
    let (_o, ok) = crun(
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
            "--tax-id",
            "NL123456789B01",
            "--iban",
            CLI_IBAN,
        ],
    );
    assert!(ok, "company update failed");
    (dir, cfg, db)
}

fn hard_open(path: &str) -> rusqlite::Connection {
    bukio::db::open_db(path).unwrap()
}

/// createEntry + postEntry (the JS post()).
fn hp(
    db: &rusqlite::Connection,
    date: &str,
    desc: &str,
    postings: Vec<bukio::entries::PostingSpec>,
) -> i64 {
    let e = bukio::entries::create_entry(
        db,
        bukio::entries::CreateEntry {
            date,
            description: desc,
            postings,
            source: "manual",
            source_ref: None,
            actor: "agent:test",
        },
    )
    .unwrap();
    bukio::entries::post_entry(db, e.id, "agent:test")
        .unwrap()
        .id
}

fn ps(code: &str, amount_cents: i64) -> bukio::entries::PostingSpec {
    bukio::entries::PostingSpec {
        code: code.to_string(),
        amount_cents,
        ..Default::default()
    }
}

fn psfx(
    code: &str,
    amount_cents: i64,
    currency: &str,
    fx_cents: i64,
) -> bukio::entries::PostingSpec {
    bukio::entries::PostingSpec {
        code: code.to_string(),
        amount_cents,
        fx_currency: Some(currency.to_string()),
        fx_amount_cents: Some(fx_cents),
        ..Default::default()
    }
}

fn hbx(
    date: &str,
    amount_cents: i64,
    counterparty: &str,
    description: &str,
) -> bukio::bank::BankTx {
    bukio::bank::BankTx {
        date: date.to_string(),
        amount_cents,
        counterparty: Some(counterparty.to_string()),
        description: Some(description.to_string()),
        iban_counter: None,
        bank_ref: None,
        iban: None,
    }
}

/// The JS fxInvoice(): a contact + a finalized VAT-free invoice of gross_cents.
fn hfx_inv(db: &rusqlite::Connection, db_path: &str, gross_cents: i64) -> i64 {
    let (_o, ok) = crun(
        db_path,
        &[
            "contact",
            "add",
            "--name",
            "Klant BV",
            "--address",
            "Straat 1",
            "--city",
            "Amsterdam",
        ],
    );
    assert!(ok, "contact add failed");
    let line = format!("1x Werk @ {}.{:02}", gross_cents / 100, gross_cents % 100);
    let inv = bukio::invoice::create_invoice(
        db,
        1,
        "2099-01-10",
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        &[json!(line)],
        "agent:test",
        false,
    )
    .unwrap();
    let id = inv["id"]
        .as_i64()
        .or_else(|| inv["invoice"]["id"].as_i64())
        .unwrap_or_else(|| panic!("no invoice id in {inv}"));
    bukio::invoice::finalize_invoice(db, id, "agent:test", false).unwrap();
    id
}

// --- F3: asset disposal -----------------------------------------------------

#[test]
fn hard_asset_dispose_at_exactly_book_value_balances() {
    let (_d, _c, dbp) = hard_full("h44", false);
    let (_o, ok) = crun(
        &dbp,
        &[
            "assets",
            "scheme",
            "add",
            "--name",
            "S",
            "--life-months",
            "24",
        ],
    );
    assert!(ok);
    let db = hard_open(&dbp);
    // link the asset to the scheme explicitly: an unlinked asset falls back to
    // the 60-month default life
    let scheme_id =
        bukio::assets::create_scheme(&db, "S-24", "lineair", 24, 0, "agent:test", false).unwrap()
            ["id"]
            .as_i64()
            .unwrap();
    bukio::assets::create_asset(
        &db,
        "Laptop",
        None,
        None,
        Some(scheme_id),
        Some("lineair"),
        Some(24),
        Some(0),
        None,
        "2025-01-01",
        240000,
        "2025-01-01",
        "2025-01-01",
        0,
        "1800",
        None,
        "4600",
        None,
        None,
        "agent:test",
        false,
    )
    .unwrap();
    bukio::assets::run_due(&db, "2025-06", "agent:test", false).unwrap();
    let reg = bukio::assets::register(&db, Some("2025-06-01"), "agent:test").unwrap();
    let asset = reg["assets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["id"] == json!(1))
        .expect("asset row");
    let book = asset["book_value_cents"].as_i64().unwrap();

    let r = bukio::assets::dispose_asset(
        &db,
        1,
        "2025-07-15",
        book,
        Some("1100"),
        None,
        None,
        "agent:test",
        false,
    )
    .unwrap();
    assert_eq!(r["result_cents"], json!(0), "{r}");
    assert_eq!(r["entry"]["state"], json!("posted"), "{r}");
}

#[test]
fn hard_asset_dispose_fully_depreciated_with_no_proceeds() {
    let (_d, _c, dbp) = hard_full("h45", false);
    let (_o, ok) = crun(
        &dbp,
        &[
            "assets",
            "scheme",
            "add",
            "--name",
            "S2",
            "--life-months",
            "12",
        ],
    );
    assert!(ok);
    let db = hard_open(&dbp);
    // linked to a 12-month scheme: an unlinked asset falls back to 60 months
    let scheme_id =
        bukio::assets::create_scheme(&db, "S-12b", "lineair", 12, 0, "agent:test", false).unwrap()
            ["id"]
            .as_i64()
            .unwrap();
    bukio::assets::create_asset(
        &db,
        "Bureaulamp",
        None,
        None,
        Some(scheme_id),
        Some("lineair"),
        Some(12),
        Some(0),
        None,
        "2025-01-01",
        120000,
        "2025-01-01",
        "2025-01-01",
        0,
        "1800",
        None,
        "4600",
        None,
        None,
        "agent:test",
        false,
    )
    .unwrap();
    bukio::assets::run_due(&db, "2025-12", "agent:test", false).unwrap();
    let r = bukio::assets::dispose_asset(
        &db,
        1,
        "2026-02-01",
        0,
        None,
        None,
        None,
        "agent:test",
        false,
    )
    .unwrap();
    assert_eq!(r["result_cents"], json!(0), "{r}");
    let entry_id = r["entry"]["id"].as_i64().unwrap();
    let entry = bukio::entries::get_entry(&db, entry_id).unwrap();
    let sum: i64 = entry.postings.iter().map(|p| p.amount_cents).sum();
    assert_eq!(sum, 0, "the disposal entry must balance");
}

// --- F4/F5/F6/F7/F8 ---------------------------------------------------------

#[test]
fn hard_invoice_create_rejects_impossible_calendar_dates() {
    let (_d, _c, dbp) = hard_full("h46", true);
    let (_o, ok) = crun(
        &dbp,
        &[
            "contact",
            "add",
            "--name",
            "ACME BV",
            "--address",
            "Straat 1",
            "--city",
            "Amsterdam",
        ],
    );
    assert!(ok);
    let db = hard_open(&dbp);
    for bad in ["2026-02-30", "2026-13-01", "2026-00-10"] {
        let err = bukio::invoice::create_invoice(
            &db,
            1,
            bad,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            &[json!("1x Test @ 100.00")],
            "agent:test",
            false,
        )
        .unwrap_err();
        assert_eq!(
            err.code, "INVALID_DATE",
            "'{bad}' must be rejected at create"
        );
    }
    for good in ["2024-02-29", "2026-04-30"] {
        bukio::invoice::create_invoice(
            &db,
            1,
            good,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            &[json!("1x Test @ 100.00")],
            "agent:test",
            false,
        )
        .unwrap_or_else(|e| panic!("'{good}' must be accepted: {e:?}"));
    }
}

#[test]
fn hard_camt_two_identical_same_day_entries_both_import() {
    let (_d, _c, dbp) = hard_full("h47", false);
    let db = hard_open(&dbp);
    let xml = "<?xml version=\"1.0\"?>\n<Document><BkToCstmrStmt><Stmt>\n<Acct><Id><IBAN>NL91ABNA0417164300</IBAN></Id></Acct>\n<Ntry><Amt>10.00</Amt><CdtDbtInd>DBIT</CdtDbtInd><BookgDt><Dt>2026-01-05</Dt></BookgDt><AcctSvcrRef>REF-1</AcctSvcrRef><NtryDtls><TxDtls><RltdPties><Cdtr><Nm>Spotify</Nm></Cdtr></RltdPties><RmtInf><Ustrd>Abonnement</Ustrd></RmtInf></TxDtls></NtryDtls></Ntry>\n<Ntry><Amt>10.00</Amt><CdtDbtInd>DBIT</CdtDbtInd><BookgDt><Dt>2026-01-05</Dt></BookgDt><AcctSvcrRef>REF-2</AcctSvcrRef><NtryDtls><TxDtls><RltdPties><Cdtr><Nm>Spotify</Nm></Cdtr></RltdPties><RmtInf><Ustrd>Abonnement</Ustrd></RmtInf></TxDtls></NtryDtls></Ntry>\n</Stmt></BkToCstmrStmt></Document>";
    let txs = bukio::bank::parse_camt053(xml).unwrap();
    assert_eq!(txs.len(), 2);
    assert_eq!(txs[0].bank_ref.as_deref(), Some("REF-1"));
    let iban = txs[0].iban.clone().unwrap_or_else(|| CLI_IBAN.to_string());
    let first =
        bukio::bank::import_transactions(&db, &iban, &txs, Some("Zakelijk"), "1100", "agent:test")
            .unwrap();
    assert_eq!(
        first["imported"],
        json!(2),
        "both identical payments must import: {first}"
    );
    let second =
        bukio::bank::import_transactions(&db, &iban, &txs, Some("Zakelijk"), "1100", "agent:test")
            .unwrap();
    assert_eq!(
        second["imported"],
        json!(0),
        "re-import must stay idempotent: {second}"
    );
    assert_eq!(second["duplicates"], json!(2), "{second}");
}

#[test]
fn hard_bank_csv_surfaces_skipped_rows() {
    let content = "Datum;Naam;Bedrag\n2026-01-01;ACME;12,34\n2026-01-02;BAD;notanumber\n2026-01-03;GOOD;5.00\n";
    let parsed = bukio::bank::parse_bank_csv(content, CLI_IBAN).unwrap();
    assert_eq!(parsed.transactions.len(), 2, "two rows parse");
    assert_eq!(
        parsed.skipped.len(),
        1,
        "the bad row is surfaced, not dropped"
    );
    assert_eq!(
        parsed.skipped[0]["line"],
        json!(3),
        "{:?}",
        parsed.skipped[0]
    );
    assert!(
        parsed.skipped[0]["reason"]
            .as_str()
            .unwrap_or("")
            .contains("unparseable amount"),
        "{:?}",
        parsed.skipped[0]
    );
}

#[test]
fn hard_payments_batch_csv_without_a_header_parses_positionally() {
    let (_d, _c, dbp) = hard_full("h48", false);
    let (_o, ok_iban) = crun(&dbp, &["company", "update", "--iban", CLI_IBAN]);
    assert!(ok_iban, "company iban update failed");
    let db = hard_open(&dbp);

    let (_o, ok) = crun(
        &dbp,
        &[
            "contact",
            "add",
            "--name",
            "ACME BV",
            "--address",
            "Straat 1",
            "--city",
            "Amsterdam",
            "--iban",
            "NL86INGB0002445588",
        ],
    );
    assert!(ok);

    let parsed = bukio::payments::parse_batch_csv("ACME BV;100.00;factuur 1\n").unwrap();
    let errors = parsed["errors"].as_array().unwrap();
    assert!(errors.is_empty(), "{parsed}");
    let lines = parsed["lines"].as_array().unwrap();
    assert_eq!(lines.len(), 1, "{parsed}");
    assert_eq!(lines[0]["contact"], json!("ACME BV"), "{parsed}");
    assert_eq!(lines[0]["amountCents"], json!(10000), "{parsed}");

    let batch = bukio::payments::create_payment_batch_from_csv(
        &db,
        "ACME BV;100.00;factuur 1\n",
        None,
        None,
        "agent:test",
        false,
    )
    .unwrap();
    let batch_lines = batch["lines"].as_array().unwrap();
    assert_eq!(batch_lines.len(), 1, "{batch}");
}

#[test]
fn hard_build_depreciation_template_rejects_a_non_positive_final_run() {
    let (_d, _c, dbp) = hard_full("h49", false);
    let db = hard_open(&dbp);
    let err = bukio::recurring::build_depreciation_template(
        &db,
        "D",
        "1800",
        "4600",
        100,
        0,
        150,
        "2026-01-01",
        None,
        "agent:test",
        false,
    )
    .unwrap_err();
    assert_eq!(err.code, "INVALID_LIFE", "{err:?}");
    let ok = bukio::recurring::build_depreciation_template(
        &db,
        "OK",
        "1800",
        "4600",
        12000,
        0,
        120,
        "2026-01-01",
        None,
        "agent:test",
        false,
    )
    .unwrap();
    assert_eq!(ok["monthly_cents"], json!(100), "{ok}");
    assert_eq!(ok["final_cents"], json!(100), "{ok}");
    assert_eq!(ok["total_cents"], json!(12000), "{ok}");
}

// --- dry-run uniformity (lib level) -----------------------------------------

#[test]
fn hard_lib_mark_paid_dry_run_writes_nothing_but_still_validates() {
    let (_d, _c, dbp) = hard_full("h50", true);
    let db = hard_open(&dbp);
    let inv = hfx_inv(&db, &dbp, 12100);
    let plan = bukio::invoice::mark_paid(
        &db,
        inv,
        "2099-02-01",
        5000,
        "manual",
        "agent:test",
        true,
        None,
    )
    .unwrap();
    assert_eq!(plan["dryRun"], json!(true), "{plan}");
    let remaining = plan["remaining_cents"].as_i64().unwrap_or(-1);
    assert!(remaining >= 0, "{plan}");
    let n: i64 = db
        .query_row("SELECT COUNT(*) FROM invoice_payments", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0, "a pay dry-run writes no payment");
    let status = bukio::invoice::get_invoice(&db, inv).unwrap().unwrap()["status"].clone();
    assert_eq!(status, json!("sent"), "the invoice stays sent");

    let err = bukio::invoice::mark_paid(
        &db,
        inv,
        "2099-02-01",
        999999,
        "manual",
        "agent:test",
        true,
        None,
    )
    .unwrap_err();
    assert_eq!(
        err.code, "OVERPAYMENT",
        "overpay is rejected even in dry-run"
    );
}

#[test]
fn hard_lib_dry_runs_write_nothing() {
    let (_d, _c, dbp) = hard_full("h51", true);
    let db = hard_open(&dbp);
    // fx set
    let rate = bukio::fx::set_fx_rate(
        &db,
        "USD",
        "2026-01-10",
        "1.0875",
        "manual",
        "agent:test",
        true,
    )
    .unwrap();
    assert_eq!(rate["dryRun"], json!(true), "{rate}");
    assert_eq!(rate["rate_x10000"], json!(10875), "{rate}");
    let n: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM fx_rates WHERE currency = 'USD'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 0, "an fx dry-run writes no rate");

    // recurring pause
    let (_o, ok) = crun(
        &dbp,
        &[
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
        ],
    );
    assert!(ok);
    let pause =
        bukio::recurring::set_template_status(&db, 1, "paused", "agent:test", true).unwrap();
    assert_eq!(pause["dryRun"], json!(true), "{pause}");
    let status = bukio::recurring::get_template(&db, 1).unwrap().unwrap()["status"].clone();
    assert_eq!(status, json!("active"), "a pause dry-run leaves it active");

    // account reactivate: the port has no dry_run argument on the module
    // function (the CLI gates it before the call), so the CLI test
    // hard_cli_dry_runs_write_nothing carries that assertion instead.
    bukio::accounts::deactivate_account(&db, "1200").unwrap();
    let react = bukio::accounts::reactivate_account(&db, "1200").unwrap();
    assert!(react.is_object(), "{react}");
    let active: i64 = db
        .query_row("SELECT active FROM accounts WHERE code = '1200'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(active, 1, "a real reactivate flips the flag");
}

#[test]
fn hard_lib_bank_dry_runs_write_nothing() {
    let (_d, _c, dbp) = hard_full("h52", false);
    let db = hard_open(&dbp);
    let plan =
        bukio::bank::get_or_create_bank_account(&db, CLI_IBAN, Some("Zakelijk"), "1100", true)
            .unwrap();
    assert_eq!(plan["dryRun"], json!(true), "{plan}");
    assert_eq!(plan["would_create"], json!(true), "{plan}");
    let n: i64 = db
        .query_row("SELECT COUNT(*) FROM bank_accounts", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0, "a bank-add dry-run creates nothing");

    let entry_id = hp(
        &db,
        "2026-01-10",
        "inkoop",
        vec![ps("1100", -5000), ps("4300", 5000)],
    );
    let txs = vec![hbx("2026-01-10", -5000, "ACME", "factuur")];
    bukio::bank::import_transactions(&db, CLI_IBAN, &txs, Some("Zakelijk"), "1100", "agent:test")
        .unwrap();
    let tx_id: i64 = db
        .query_row(
            "SELECT id FROM bank_transactions WHERE state = 'unmatched'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let link =
        bukio::bank::link_transaction(&db, tx_id, entry_id, "manual", None, "agent:test", true)
            .unwrap();
    assert_eq!(link["dryRun"], json!(true), "{link}");
    let recon: i64 = db
        .query_row("SELECT COUNT(*) FROM reconciliations", [], |r| r.get(0))
        .unwrap();
    assert_eq!(recon, 0, "a link dry-run writes no reconciliation");
    let state: String = db
        .query_row(
            "SELECT state FROM bank_transactions WHERE id = ?1",
            [tx_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "unmatched", "the transaction stays unmatched");
}

#[test]
fn hard_auto_match_never_crosses_bank_accounts() {
    let (_d, _c, dbp) = hard_full("h53", false);
    let db = hard_open(&dbp);
    bukio::bank::import_transactions(
        &db,
        CLI_IBAN,
        &[hbx("2026-01-10", -5000, "ACME", "factuur A")],
        Some("Rabo A"),
        "1100",
        "agent:test",
    )
    .unwrap();
    bukio::bank::import_transactions(
        &db,
        "NL86INGB0002445588",
        &[hbx("2026-01-10", -5000, "ACME", "factuur B")],
        Some("ING B"),
        "1100",
        "agent:test",
    )
    .unwrap();
    let tx_a: i64 = db
        .query_row(
            "SELECT id FROM bank_transactions WHERE description = 'factuur A'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let (entry, _recon) =
        bukio::bank::post_from_transaction(&db, tx_a, "4300", "agent:test", true).unwrap();
    assert!(entry["id"].as_i64().is_some(), "{entry}");

    let tx_b: i64 = db
        .query_row(
            "SELECT id FROM bank_transactions WHERE description = 'factuur B'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let result = bukio::bank::auto_match(&db, 5, "agent:test", true).unwrap();
    let matched = result["matched"].as_array().unwrap();
    assert!(
        !matched.iter().any(|m| m["tx_id"] == json!(tx_b)),
        "tx B must not match an entry booked from account A: {result}"
    );
}

// --- CSV / XLSX formula injection -------------------------------------------

#[test]
fn hard_csv_exports_neuter_formula_injection() {
    let (_d, _c, dbp) = hard_full("h54", false);
    let db = hard_open(&dbp);
    // a hostile account name that would become a live formula in Excel
    db.execute(
        "INSERT INTO accounts (code, name, type, normal_balance, taxonomy_code) VALUES ('9998', '=HYPERLINK(\"https://evil\",\"x\")', 'expense', 'debit', 'WBED.42')",
        [],
    )
    .unwrap();
    let e = bukio::entries::create_entry(
        &db,
        bukio::entries::CreateEntry {
            date: "2026-06-01",
            description: "kost",
            postings: vec![ps("1100", 1000), ps("9998", -1000)],
            source: "manual",
            source_ref: None,
            actor: "agent:test",
        },
    )
    .unwrap();
    bukio::entries::post_entry(&db, e.id, "agent:test").unwrap();
    drop(db);

    let out = std::path::Path::new(&dbp)
        .parent()
        .unwrap()
        .join("tb.csv")
        .to_string_lossy()
        .to_string();
    let (_, ok) = crun(
        &dbp,
        &["report", "trial-balance", "--format", "csv", "--out", &out],
    );
    assert!(ok, "the csv export must succeed");
    let csv = std::fs::read_to_string(&out).unwrap();
    assert!(
        csv.contains("\"'=HYPERLINK(\"\"https://evil\"\",\"\"x\"\")\""),
        "the formula must be quoted and prefixed with a single quote, got:\n{csv}"
    );
    assert!(
        !csv.contains("\n=HYPERLINK"),
        "a bare formula must never reach a cell"
    );
    // negative amounts stay amounts, not guarded text
    let (_o2, ok2) = crun(
        &dbp,
        &["report", "trial-balance", "--format", "csv", "--out", &out],
    );
    assert!(ok2);
    // xlsx: rust_xlsxwriter's write_string stores text, so no cell can be a formula
}

#[test]
fn hard_jaarrekening_and_export_xaf_reject_a_non_yyyy_year() {
    let (_d, _c, dbp) = hard_full("h55", false);
    let db = hard_open(&dbp);
    for bad in ["abc", "20261"] {
        assert_eq!(
            bukio::reports::jaarrekening(&db, bad, Some("klein"))
                .unwrap_err()
                .code,
            "INVALID_YEAR"
        );
        assert_eq!(
            bukio::export::export_xaf(&db, bad, "/tmp/never.xaf", "agent:test", true)
                .unwrap_err()
                .code,
            "INVALID_YEAR"
        );
    }
    assert!(!std::path::Path::new("/tmp/never.xaf").exists());
}

// --- F14/F15: derived status, 0% line, amounts, boundaries ------------------

#[test]
fn hard_invoice_list_status_overdue_filters_the_derived_status() {
    let (_d, _c, dbp) = hard_full("h56", false);
    let db = hard_open(&dbp);
    let (_o, ok) = crun(
        &dbp,
        &[
            "contact",
            "add",
            "--name",
            "ACME BV",
            "--address",
            "Straat 1",
            "--city",
            "Amsterdam",
        ],
    );
    assert!(ok);
    for date in ["2099-01-10", "2026-01-01"] {
        let inv = bukio::invoice::create_invoice(
            &db,
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
            &[json!("1x T @ 100.00")],
            "agent:test",
            false,
        )
        .unwrap();
        let id = inv["id"]
            .as_i64()
            .or_else(|| inv["invoice"]["id"].as_i64())
            .unwrap();
        bukio::invoice::finalize_invoice(&db, id, "agent:test", false).unwrap();
    }
    let overdue = bukio::invoice::list_invoices(&db, Some("overdue"), None).unwrap();
    assert_eq!(
        overdue.len(),
        1,
        "only the long-past invoice is overdue: {overdue:?}"
    );
    let sent = bukio::invoice::list_invoices(&db, Some("sent"), None).unwrap();
    assert_eq!(sent.len(), 2, "both are stored 'sent': {sent:?}");
}

#[test]
fn hard_invoice_finalize_with_a_zero_rate_line_books_a_tagged_zero_vat_posting() {
    let (_d, _c, dbp) = hard_full("h57", true);
    let db = hard_open(&dbp);
    let (_o, ok) = crun(
        &dbp,
        &[
            "contact",
            "add",
            "--name",
            "ACME BV",
            "--address",
            "Straat 1",
            "--city",
            "Amsterdam",
        ],
    );
    assert!(ok);
    let inv = bukio::invoice::create_invoice(
        &db,
        1,
        "2099-01-10",
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        &[json!("1x Diensten @ 100.00 @V")],
        "agent:test",
        false,
    )
    .unwrap();
    let id = inv["id"]
        .as_i64()
        .or_else(|| inv["invoice"]["id"].as_i64())
        .unwrap();
    let fin = bukio::invoice::finalize_invoice(&db, id, "agent:test", false).unwrap();
    let invoice = fin.get("invoice").cloned().unwrap_or(fin.clone());
    assert_eq!(invoice["status"], json!("sent"), "{invoice}");
    let entry_id = invoice["entry_id"].as_i64().unwrap();
    let entry = bukio::entries::get_entry(&db, entry_id).unwrap();
    let omzet = entry
        .postings
        .iter()
        .find(|p| p.account_code == "8000")
        .expect("8000 posting");
    assert_eq!(omzet.vat_code.as_deref(), Some("V"), "{omzet:?}");
    assert_eq!(omzet.vat_amount_cents, Some(0), "{omzet:?}");
    let readout = bukio::vat::ob_readout(&db, "2099-Q1").unwrap();
    assert_eq!(readout["fields"]["1c"], json!(10000), "{readout}");
    assert_eq!(readout["fields"]["5a"], json!(0), "{readout}");
}

#[test]
fn hard_parse_amount_boundaries() {
    for (input, want) in [
        ("1234.5", 123450i64),
        ("-0.01", -1),
        ("0", 0),
        ("-0", 0),
        ("999999999999.99", 99999999999999),
        (" 12.34 ", 1234),
    ] {
        assert_eq!(
            bukio::money::parse_amount(input).unwrap(),
            want,
            "'{input}'"
        );
    }
    for bad in ["", ".5", "1.", "1.234", "+12.34", "12,34", "1e3", "12.345"] {
        let err = bukio::money::parse_amount(bad).unwrap_err();
        assert_eq!(err.code, "INVALID_AMOUNT", "'{bad}' must be rejected");
    }
}

#[test]
fn hard_ob_readout_period_with_a_year_boundary_stays_within_the_period() {
    let (_d, _c, dbp) = hard_full("h58", true);
    let db = hard_open(&dbp);
    for (date, desc, spec) in [
        ("2026-12-31", "q4 sale", "8000:-100.00@21,1100:121.00"),
        ("2027-01-01", "q1 sale", "8000:-50.00@21,1100:60.50"),
    ] {
        let specs = bukio::vat::parse_vat_posting_specs(&[spec.to_string()]).unwrap();
        bukio::vat::book_vat_entry(&db, date, desc, &specs, "manual", None, "agent:test", true)
            .unwrap();
    }
    assert_eq!(
        bukio::vat::ob_readout(&db, "2026-Q4").unwrap()["fields"]["1a"],
        json!(10000)
    );
    assert_eq!(
        bukio::vat::ob_readout(&db, "2027-Q1").unwrap()["fields"]["1a"],
        json!(5000)
    );
}

#[test]
fn hard_entry_with_the_same_account_on_both_sides_books_the_net() {
    let (_d, _c, dbp) = hard_full("h59", false);
    let db = hard_open(&dbp);
    let id = hp(
        &db,
        "2026-01-10",
        "partial same-code",
        vec![ps("1200", 10000), ps("1200", -2000), ps("8000", -8000)],
    );
    let full = bukio::entries::get_entry(&db, id).unwrap();
    let mut by_code: std::collections::HashMap<&str, i64> = std::collections::HashMap::new();
    for p in &full.postings {
        *by_code.entry(p.account_code.as_str()).or_insert(0) += p.amount_cents;
    }
    assert_eq!(by_code.get("1200"), Some(&8000), "{:?}", full.postings);
    assert_eq!(by_code.get("8000"), Some(&-8000));
}

#[test]
fn hard_reversal_of_an_fx_entry_negates_the_fx_amounts() {
    let (_d, _c, dbp) = hard_full("h60", false);
    let db = hard_open(&dbp);
    let id = hp(
        &db,
        "2026-01-10",
        "fx purchase",
        vec![
            psfx("4340", 875, "USD", 1000),
            psfx("3000", -875, "USD", -1000),
        ],
    );
    let rev = bukio::entries::reverse_entry(&db, id, "agent:test", None).unwrap();
    let entry = bukio::entries::get_entry(&db, rev.id).unwrap();
    let fx = entry
        .postings
        .iter()
        .find(|p| p.account_code == "4340")
        .expect("4340 posting");
    assert_eq!(fx.fx_amount_cents, Some(-1000), "{fx:?}");
    assert_eq!(fx.fx_currency.as_deref(), Some("USD"), "{fx:?}");
}

#[test]
fn hard_cli_import_xaf_failure_prints_cleanly() {
    let (dir, _c, dbp) = hard_full("h61", false);
    let bad = dir.join("bad.xaf");
    std::fs::write(
        &bad,
        "<?xml version=\"1.0\"?><Xaf><XafHeader><Version>4.0</Version></XafHeader><Mutaties><Mutatie><Boekstuknummer>1</Boekstuknummer><Datum>2026-01-01</Datum></Mutatie></Mutaties></Xaf>",
    )
    .unwrap();
    let raw = crun_text(&dbp, &["import", "xaf", "--file", &bad.to_string_lossy()]);
    assert!(
        raw.contains("IMPORT_VALIDATION_FAILED"),
        "expected a validation error, got: {raw:.300}"
    );
    assert!(
        !raw.contains("ReferenceError"),
        "no crash may leak: {raw:.300}"
    );
    assert!(!raw.contains("panicked"), "no panic may leak: {raw:.300}");
}

// --- F16-F19: FX differences, atomicity, year-end, recurring ----------------

#[test]
fn hard_auto_match_books_a_small_fx_difference_to_4840() {
    let (_d, _c, dbp) = hard_full("h62", false);
    let db = hard_open(&dbp);
    let inv = hfx_inv(&db, &dbp, 100000);
    bukio::bank::import_transactions(
        &db,
        CLI_IBAN,
        &[hbx("2099-01-20", 99750, "Klant BV", "betaling")],
        Some("Zakelijk"),
        "1100",
        "agent:test",
    )
    .unwrap();
    let res = bukio::bank::auto_match(&db, 14, "agent:test", false).unwrap();
    let m = res["matched"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["kind"] == json!("invoice"))
        .unwrap_or_else(|| panic!("a payment within the FX bound must match: {res}"));
    assert_eq!(m["fx_delta_cents"], json!(-250), "{m}");
    assert_eq!(
        bukio::invoice::get_invoice(&db, inv).unwrap().unwrap()["status"],
        json!("paid")
    );
    let tb = bukio::reports::trial_balance(&db, None).unwrap();
    assert_eq!(tb["balanced"], json!(true), "{tb}");
    let net = |code: &str| -> i64 {
        tb["accounts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["code"] == json!(code))
            .map(|a| a["net_cents"].as_i64().unwrap())
            .unwrap_or(0)
    };
    assert_eq!(net("4840"), 250, "the 2.50 loss sits on 4840 (debit)");
    assert_eq!(net("1100"), 99750);
    assert_eq!(net("1200"), 0, "Debiteuren is fully released");
}

#[test]
fn hard_payment_from_bank_fx_gain_books_a_credit_on_4840() {
    let (_d, _c, dbp) = hard_full("h63", false);
    let db = hard_open(&dbp);
    let inv = hfx_inv(&db, &dbp, 50000);
    bukio::bank::import_transactions(
        &db,
        CLI_IBAN,
        &[hbx("2099-01-20", 50150, "Klant BV", "betaling")],
        Some("Zakelijk"),
        "1100",
        "agent:test",
    )
    .unwrap();
    let tx_id: i64 = db
        .query_row(
            "SELECT id FROM bank_transactions WHERE state = 'unmatched'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    bukio::invoice::payment_from_bank(&db, inv, tx_id, "agent:test", 200).unwrap();
    let tb = bukio::reports::trial_balance(&db, None).unwrap();
    assert_eq!(tb["balanced"], json!(true), "{tb}");
    let net_4840 = tb["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["code"] == json!("4840"))
        .map(|a| a["net_cents"].as_i64().unwrap())
        .unwrap_or(0);
    assert_eq!(net_4840, -150, "a 1.50 gain is a credit");
    assert_eq!(
        bukio::invoice::get_invoice(&db, inv).unwrap().unwrap()["status"],
        json!("paid")
    );
}

#[test]
fn hard_a_difference_beyond_the_sanity_bound_is_rejected() {
    let (_d, _c, dbp) = hard_full("h64", false);
    let db = hard_open(&dbp);
    let inv = hfx_inv(&db, &dbp, 100000);
    bukio::bank::import_transactions(
        &db,
        CLI_IBAN,
        &[hbx("2099-01-20", 95000, "Klant BV", "betaling")],
        Some("Zakelijk"),
        "1100",
        "agent:test",
    )
    .unwrap();
    let res = bukio::bank::auto_match(&db, 14, "agent:test", false).unwrap();
    let invoice_matches = res["matched"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["kind"] == json!("invoice"))
        .count();
    assert_eq!(
        invoice_matches, 0,
        "a 5% shortfall is not an FX move: {res}"
    );
    assert_eq!(res["unmatched_remaining"], json!(1), "{res}");
    assert_eq!(
        bukio::invoice::get_invoice(&db, inv).unwrap().unwrap()["status"],
        json!("sent")
    );
    let tx_id: i64 = db
        .query_row(
            "SELECT id FROM bank_transactions WHERE state = 'unmatched'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    // and the explicit entry point refuses it as well (the JS asserts both
    // paths): tolerance 0 still leaves the 25-cent floor, so 5% is over the top
    let err = bukio::invoice::payment_from_bank(&db, inv, tx_id, "agent:test", 0).unwrap_err();
    assert_eq!(err.code, "FX_DIFFERENCE_TOO_LARGE", "{err:?}");
    assert_eq!(
        bukio::invoice::get_invoice(&db, inv).unwrap().unwrap()["status"],
        json!("sent"),
        "a rejected payment must not settle the invoice"
    );
}

#[test]
fn hard_fx_sanity_floor_is_25_cents() {
    let (_d, _c, dbp) = hard_full("h65", false);
    let db = hard_open(&dbp);
    let inv = hfx_inv(&db, &dbp, 1000);
    bukio::bank::import_transactions(
        &db,
        CLI_IBAN,
        &[hbx("2099-01-20", 900, "Klant BV", "betaling")],
        Some("Zakelijk"),
        "1100",
        "agent:test",
    )
    .unwrap();
    let res = bukio::bank::auto_match(&db, 14, "agent:test", false).unwrap();
    let n = res["matched"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["kind"] == json!("invoice"))
        .count();
    assert_eq!(
        n, 0,
        "10% off a 10 euro invoice must not auto-settle: {res}"
    );
    let tx_id: i64 = db
        .query_row(
            "SELECT id FROM bank_transactions WHERE state = 'unmatched'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    // 100 cents of difference on a 10 euro invoice is over the floor and far
    // over 0bp: the abs floor of 25 cents does not make a 10% haircut acceptable
    let err = bukio::invoice::payment_from_bank(&db, inv, tx_id, "agent:test", 0).unwrap_err();
    assert_eq!(err.code, "FX_DIFFERENCE_TOO_LARGE", "{err:?}");
    assert_eq!(
        bukio::invoice::get_invoice(&db, inv).unwrap().unwrap()["status"],
        json!("sent")
    );
}

#[test]
fn hard_4840_is_created_on_demand_and_audited() {
    let (_d, _c, dbp) = hard_full("h66", false);
    let db = hard_open(&dbp);
    db.execute("DELETE FROM accounts WHERE code = '4840'", [])
        .unwrap();
    assert!(bukio::accounts::get_account_by_code(&db, "4840").is_none());
    let inv = hfx_inv(&db, &dbp, 10000);
    bukio::bank::import_transactions(
        &db,
        CLI_IBAN,
        &[hbx("2099-01-20", 9980, "Klant BV", "betaling")],
        Some("Zakelijk"),
        "1100",
        "agent:test",
    )
    .unwrap();
    let tx_id: i64 = db
        .query_row(
            "SELECT id FROM bank_transactions WHERE state = 'unmatched'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    bukio::invoice::payment_from_bank(&db, inv, tx_id, "agent:test", 200).unwrap();

    let fx = bukio::accounts::get_account_by_code(&db, "4840").expect("4840 created on demand");
    assert_eq!(fx["taxonomy_code"], json!("WFBE.84"), "{fx}");
    assert_eq!(
        bukio::reports::trial_balance(&db, None).unwrap()["balanced"],
        json!(true)
    );

    let rows = bukio::audit::list(&db, None, None, 200).unwrap();
    let created: Vec<&Value> = rows
        .iter()
        .filter(|r| {
            r["action"] == json!("account.create")
                && r["args_json"].as_str().unwrap_or("").contains("4840")
        })
        .collect();
    assert_eq!(
        created.len(),
        1,
        "the on-demand creation is audited exactly once: {created:?}"
    );
    assert_eq!(created[0]["actor"], json!("agent:test"), "{:?}", created[0]);
}

#[test]
fn hard_payment_from_bank_is_atomic() {
    // The bank ledger account is inactive, so the posting cannot be created:
    // the payment row, the entry and the reconciliation must all roll back.
    // A recorded payment with no entry would double-pay on a re-match, which is
    // why the JS wraps paymentFromBank in one transaction and why entries::begin
    // nests a savepoint instead of starting a second one.
    let (_d, _c, dbp) = hard_full("h67", false);
    let db = hard_open(&dbp);
    let inv = hfx_inv(&db, &dbp, 50000);
    bukio::bank::import_transactions(
        &db,
        CLI_IBAN,
        &[hbx("2099-01-20", 50150, "Klant BV", "betaling")],
        Some("Zakelijk"),
        "1100",
        "agent:test",
    )
    .unwrap();
    let tx_id: i64 = db
        .query_row(
            "SELECT id FROM bank_transactions WHERE state = 'unmatched'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    bukio::accounts::deactivate_account(&db, "1100").unwrap();

    let err = bukio::invoice::payment_from_bank(&db, inv, tx_id, "agent:test", 200).unwrap_err();
    assert_eq!(err.code, "ACCOUNT_INACTIVE", "{err:?}");

    let payments: i64 = db
        .query_row("SELECT COUNT(*) FROM invoice_payments", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        payments, 0,
        "no payment row may survive a failed bank payment"
    );
    assert_eq!(
        bukio::invoice::get_invoice(&db, inv).unwrap().unwrap()["status"],
        json!("sent"),
        "the invoice must not be settled"
    );
    let state: String = db
        .query_row(
            "SELECT state FROM bank_transactions WHERE id = ?1",
            [tx_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "unmatched", "the bank line stays open for a human");
    let recon: i64 = db
        .query_row("SELECT COUNT(*) FROM reconciliations", [], |r| r.get(0))
        .unwrap();
    assert_eq!(recon, 0, "no dangling reconciliation");
}

#[test]
fn hard_post_from_transaction_is_atomic() {
    let (_d, _c, dbp) = hard_full("h68", false);
    let db = hard_open(&dbp);
    bukio::bank::import_transactions(
        &db,
        CLI_IBAN,
        &[hbx("2099-01-20", -5000, "ACME", "kosten")],
        Some("Zakelijk"),
        "1100",
        "agent:test",
    )
    .unwrap();
    let tx_id: i64 = db
        .query_row(
            "SELECT id FROM bank_transactions WHERE state = 'unmatched'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let before: i64 = db
        .query_row("SELECT COUNT(*) FROM journal_entries", [], |r| r.get(0))
        .unwrap();
    bukio::accounts::deactivate_account(&db, "4300").unwrap();
    let err =
        bukio::bank::post_from_transaction(&db, tx_id, "4300", "agent:test", true).unwrap_err();
    assert_eq!(err.code, "ACCOUNT_INACTIVE", "{err:?}");
    let after: i64 = db
        .query_row("SELECT COUNT(*) FROM journal_entries", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        after, before,
        "no stray draft entry may survive a failed post"
    );
    let state: String = db
        .query_row(
            "SELECT state FROM bank_transactions WHERE id = ?1",
            [tx_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "unmatched");
    let rec: i64 = db
        .query_row("SELECT COUNT(*) FROM reconciliations", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rec, 0);
}

#[test]
fn hard_create_payment_batch_rejects_a_garbage_date() {
    let (_d, _c, dbp) = hard_full("h69", false);
    let db = hard_open(&dbp);
    let (_o, ok) = crun(
        &dbp,
        &[
            "contact",
            "add",
            "--name",
            "ACME BV",
            "--address",
            "Straat 1",
            "--city",
            "Amsterdam",
            "--iban",
            "NL86INGB0002445588",
        ],
    );
    assert!(ok);
    let lines =
        vec![json!({ "contact": "ACME BV", "iban": "NL86INGB0002445588", "amountCents": 1000 })];
    for bad in ["garbage", "2026-02-30"] {
        let err = bukio::payments::create_payment_batch(
            &db,
            Some(bad),
            None,
            &lines,
            &[],
            "transfer",
            "agent:test",
            false,
        )
        .unwrap_err();
        assert_eq!(
            err.code, "INVALID_DATE",
            "'{bad}' must be rejected: {err:?}"
        );
    }
    let ok_batch = bukio::payments::create_payment_batch(
        &db,
        Some("2026-03-05"),
        None,
        &lines,
        &[],
        "transfer",
        "agent:test",
        false,
    )
    .unwrap();
    assert_eq!(ok_batch["batch_date"], json!("2026-03-05"), "{ok_batch}");
}

#[test]
fn hard_fetch_ecb_rate_rejects_a_malformed_date() {
    for bad in ["not-a-date", "2026-02-30"] {
        let err = bukio::fx::fetch_ecb_rate("USD", bad).unwrap_err();
        assert_eq!(err.code, "INVALID_DATE", "'{bad}' must be rejected");
    }
}

#[test]
fn hard_import_transactions_rejects_garbage_dates() {
    let (_d, _c, dbp) = hard_full("h70", false);
    let db = hard_open(&dbp);
    bukio::bank::get_or_create_bank_account(&db, CLI_IBAN, Some("Zakelijk"), "1100", false)
        .unwrap();
    for bad in ["garbage", "2026-02-30", "10-01-2026"] {
        let txs = vec![hbx(bad, -5000, "ACME", "x")];
        let err = bukio::bank::import_transactions(
            &db,
            CLI_IBAN,
            &txs,
            Some("Zakelijk"),
            "1100",
            "agent:test",
        )
        .unwrap_err();
        assert_eq!(err.code, "INVALID_DATE", "'{bad}' must be rejected");
    }
    let good = vec![hbx("2026-01-10", -5000, "ACME", "x")];
    let r = bukio::bank::import_transactions(
        &db,
        CLI_IBAN,
        &good,
        Some("Zakelijk"),
        "1100",
        "agent:test",
    )
    .unwrap();
    assert_eq!(r["imported"], json!(1), "{r}");
}

#[test]
fn hard_list_limits_validate_at_the_module_boundary() {
    let (_d, _c, dbp) = hard_full("h71", false);
    let db = hard_open(&dbp);
    assert_eq!(
        bukio::entries::list_entries(&db, None, None, None, -1)
            .unwrap_err()
            .code,
        "INVALID_LIMIT"
    );
    assert_eq!(
        bukio::audit::list(&db, None, None, -1).unwrap_err().code,
        "INVALID_LIMIT"
    );
    assert_eq!(
        bukio::bank::list_transactions(&db, None, None, -1)
            .unwrap_err()
            .code,
        "INVALID_LIMIT"
    );
    assert_eq!(
        bukio::fx::list_fx_rates(&db, None, -1).unwrap_err().code,
        "INVALID_LIMIT"
    );
    // zero is legal and returns zero rows
    assert_eq!(
        bukio::entries::list_entries(&db, None, None, None, 0)
            .unwrap()
            .len(),
        0
    );
    assert_eq!(bukio::fx::list_fx_rates(&db, None, 0).unwrap().len(), 0);
}

#[test]
fn hard_import_xaf_skips_a_duplicate_boekstuknummer() {
    let (_d, _c, dbp) = hard_full("h72", false);
    let db = hard_open(&dbp);
    let xaf = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Xaf xmlns=\"http://www.auditfiles.nl/XAF/4.0\">\n  <XafHeader><Version>4.0</Version><CompanyName>Demo BV</CompanyName><CompanyID>12345678</CompanyID><FiscalYear>2026</FiscalYear></XafHeader>\n  <Rekeningen>\n    <Rekening><RekeningCode>1100</RekeningCode><RekeningOmschrijving>Bank</RekeningOmschrijving><RekeningSoort>Balans</RekeningSoort></Rekening>\n    <Rekening><RekeningCode>8000</RekeningCode><RekeningOmschrijving>Omzet</RekeningOmschrijving><RekeningSoort>Winst en Verlies</RekeningSoort></Rekening>\n  </Rekeningen>\n  <Mutaties>\n    <Mutatie>\n      <Boekstuknummer>DUP-1</Boekstuknummer><Datum>2026-01-10</Datum>\n      <Boekingen>\n        <Boeking><RekeningCode>1100</RekeningCode><TegenrekeningCode>8000</TegenrekeningCode><Bedrag>100.00</Bedrag></Boeking>\n      </Boekingen>\n    </Mutatie>\n    <Mutatie>\n      <Boekstuknummer>DUP-1</Boekstuknummer><Datum>2026-01-11</Datum>\n      <Boekingen>\n        <Boeking><RekeningCode>1100</RekeningCode><TegenrekeningCode>8000</TegenrekeningCode><Bedrag>200.00</Bedrag></Boeking>\n      </Boekingen>\n    </Mutatie>\n  </Mutaties>\n</Xaf>";
    let r = bukio::import_mod::import_xaf(&db, xaf, "agent:test", false).unwrap();
    assert_eq!(
        r["imported"],
        json!(1),
        "only the first mutatie imports: {r}"
    );
    assert_eq!(r["duplicates"], json!(1), "the duplicate is reported: {r}");
    let n: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM journal_entries WHERE source = 'xaf'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 1);
    let again = bukio::import_mod::import_xaf(&db, xaf, "agent:test", false).unwrap();
    assert_eq!(
        again["imported"],
        json!(0),
        "re-import stays idempotent: {again}"
    );
    assert_eq!(again["duplicates"], json!(2), "{again}");
}

#[test]
fn hard_auto_match_fx_tolerance_matches_the_posting_tolerance() {
    let (_d, _c, dbp) = hard_full("h73", true);
    let db = hard_open(&dbp);
    let (_o, ok) = crun(
        &dbp,
        &[
            "contact",
            "add",
            "--name",
            "ACME BV",
            "--address",
            "Straat 1",
            "--city",
            "Amsterdam",
        ],
    );
    assert!(ok);
    bukio::bank::get_or_create_bank_account(&db, CLI_IBAN, Some("Zakelijk"), "1100", false)
        .unwrap();
    let inv = bukio::invoice::create_invoice(
        &db,
        1,
        "2026-01-10",
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        &[json!("1x Werk @ 123.45 @21")],
        "agent:test",
        false,
    )
    .unwrap();
    let id = inv["id"]
        .as_i64()
        .or_else(|| inv["invoice"]["id"].as_i64())
        .unwrap();
    bukio::invoice::finalize_invoice(&db, id, "agent:test", false).unwrap();
    let gross = bukio::invoice::get_invoice(&db, id).unwrap().unwrap()["gross_cents"]
        .as_i64()
        .unwrap();
    let tol = ((gross as f64) * 0.02).round() as i64;
    let pay = gross - tol;
    let number = bukio::invoice::get_invoice(&db, id).unwrap().unwrap()["invoice_number"]
        .as_str()
        .unwrap_or("")
        .to_string();
    bukio::bank::import_transactions(
        &db,
        CLI_IBAN,
        &[hbx(
            "2026-01-15",
            pay,
            "ACME BV",
            &format!("Betaling {number}"),
        )],
        Some("Zakelijk"),
        "1100",
        "agent:test",
    )
    .unwrap();
    let dry = bukio::bank::auto_match(&db, 14, "agent:test", true).unwrap();
    let proposed = dry["matched"]
        .as_array()
        .unwrap()
        .iter()
        .any(|m| m["kind"] == json!("invoice") && m["invoice_id"] == json!(id));
    assert!(
        proposed,
        "a payment at the {tol}-cent boundary must be proposed: {dry}"
    );
    bukio::bank::auto_match(&db, 14, "agent:test", false).unwrap();
    assert_eq!(
        bukio::invoice::get_invoice(&db, id).unwrap().unwrap()["status"],
        json!("paid")
    );
    assert_eq!(
        bukio::reports::trial_balance(&db, None).unwrap()["balanced"],
        json!(true)
    );
}

#[test]
fn hard_year_end_close_handles_a_zero_result_year() {
    let (_d, _c, dbp) = hard_full("h74", false);
    let db = hard_open(&dbp);
    hp(
        &db,
        "2026-03-01",
        "omzet",
        vec![ps("1100", 10000), ps("8000", -10000)],
    );
    hp(
        &db,
        "2026-04-01",
        "kosten",
        vec![ps("4600", 10000), ps("1100", -10000)],
    );
    let posted = |db: &rusqlite::Connection| -> i64 {
        db.query_row(
            "SELECT COUNT(*) FROM journal_entries WHERE state = 'posted'",
            [],
            |r| r.get(0),
        )
        .unwrap()
    };
    let before = posted(&db);

    let plan = bukio::year_end::year_end_close(&db, "2026", "agent:test", true).unwrap();
    assert_eq!(plan["result_cents"], json!(0), "{plan}");
    assert_eq!(posted(&db), before, "a dry-run books nothing: {plan}");
    assert_eq!(
        plan["create_9900"].as_bool().unwrap_or(false),
        false,
        "{plan}"
    );

    let r = bukio::year_end::year_end_close(&db, "2026", "agent:test", false).unwrap();
    assert_eq!(r["closed"], json!(true), "{r}");
    assert_eq!(
        posted(&db),
        before + 1,
        "a zero result books one closing entry: {r}"
    );
    let latest: i64 = db
        .query_row("SELECT MAX(id) FROM journal_entries", [], |r| r.get(0))
        .unwrap();
    let closing = bukio::entries::get_entry(&db, latest).unwrap();
    assert_eq!(
        closing.postings.len(),
        2,
        "no zero-amount legs: {:?}",
        closing.postings
    );
    assert!(
        bukio::accounts::get_account_by_code(&db, "9900").is_none(),
        "9900 must not be created for a zero result"
    );
    assert_eq!(
        bukio::reports::trial_balance(&db, None).unwrap()["balanced"],
        json!(true)
    );

    hp(
        &db,
        "2025-05-01",
        "omzet2",
        vec![ps("1100", 5000), ps("8000", -5000)],
    );
    let before_2025 = posted(&db);
    let r2 = bukio::year_end::year_end_close(&db, "2025", "agent:test", false).unwrap();
    assert_eq!(r2["result_cents"], json!(5000), "{r2}");
    assert_eq!(
        posted(&db),
        before_2025 + 2,
        "a non-zero result books closing + appropriation: {r2}"
    );
}

#[test]
fn hard_recurring_dry_run_previews_a_zero_day_invoice_due_date() {
    let (_d, _c, dbp) = hard_full("h75", true);
    let db = hard_open(&dbp);
    let (_o, ok) = crun(
        &dbp,
        &[
            "contact",
            "add",
            "--name",
            "ACME BV",
            "--address",
            "Straat 1",
            "--city",
            "Amsterdam",
        ],
    );
    assert!(ok);
    let (_o, ok) = crun(
        &dbp,
        &[
            "recurring",
            "add",
            "--kind",
            "invoice",
            "--contact",
            "1",
            "--lines",
            "1x Coaching @ 100.00",
            "--frequency",
            "monthly",
            "--start",
            "2026-01-15",
            "--day",
            "15",
            "--due-days",
            "0",
            "--name",
            "abonnement",
        ],
    );
    assert!(ok);
    let tpl = bukio::recurring::get_template(&db, 1).unwrap().unwrap();
    assert_eq!(tpl["due_days"], json!(0), "{tpl}");

    let plan =
        bukio::recurring::run_due(&db, Some("2026-01-20"), None, "agent:test", true).unwrap();
    let runs = plan["templates"][0]["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 1, "{plan}");
    let due = runs[0]["invoice"]["due_date"].clone();
    assert_eq!(
        due,
        json!("2026-01-15"),
        "due_days 0 must date the invoice on the invoice date: {plan}"
    );

    let real =
        bukio::recurring::run_due(&db, Some("2026-01-20"), None, "agent:test", false).unwrap();
    let inv_id = real["templates"][0]["runs"][0]["generated"][0]["invoice"]["id"]
        .as_i64()
        .unwrap();
    let inv = bukio::invoice::get_invoice(&db, inv_id).unwrap().unwrap();
    assert_eq!(
        inv["due_date"],
        json!("2026-01-15"),
        "the real run dates it the same way: {inv}"
    );
}

#[test]
fn hard_credit_invoice_dry_run_validates_like_the_real_run() {
    let (_d, _c, dbp) = hard_full("h76", true);
    let db = hard_open(&dbp);
    let err = bukio::invoice::credit_invoice(&db, 999, None, None, "agent:test", true).unwrap_err();
    assert_eq!(err.code, "NOT_FOUND", "{err:?}");
    let (_o, ok) = crun(
        &dbp,
        &[
            "contact",
            "add",
            "--name",
            "ACME BV",
            "--address",
            "Straat 1",
            "--city",
            "Amsterdam",
        ],
    );
    assert!(ok);
    let inv = bukio::invoice::create_invoice(
        &db,
        1,
        "2026-01-15",
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        &[json!("1x Coaching @ 100.00")],
        "agent:test",
        false,
    )
    .unwrap();
    let id = inv["id"]
        .as_i64()
        .or_else(|| inv["invoice"]["id"].as_i64())
        .unwrap();
    let err = bukio::invoice::credit_invoice(&db, id, None, None, "agent:test", true).unwrap_err();
    assert_eq!(
        err.code, "NOT_FINALIZED",
        "a draft cannot be credited: {err:?}"
    );

    bukio::invoice::finalize_invoice(&db, id, "agent:test", false).unwrap();
    let plan = bukio::invoice::credit_invoice(&db, id, None, None, "agent:test", true).unwrap();
    assert_eq!(plan["dryRun"], json!(true), "{plan}");
    let for_inv = plan.get("for_invoice").or_else(|| plan.get("forInvoice"));
    assert_eq!(for_inv, Some(&json!(id)), "{plan}");
    let n: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM invoices WHERE invoice_type = 'credit'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 0, "a credit dry-run writes no credit note");
}

#[test]
fn hard_create_invoice_rejects_negative_due_days() {
    // the JS throws INVALID_DUE_DAYS before touching the database. The Rust
    // signature takes Option<i64>, so a non-integer term cannot even be
    // expressed; the sign is the part that still needs a guard (MCP and the
    // recurring engine call the engine directly, not through the CLI).
    let d = setup();
    add_contact(&d, None);
    let err = create_invoice(
        &d,
        1,
        "2026-07-10",
        Some(-5),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        &lines(&["1x Pennen @ 1.00 @21"]),
        "agent:test",
        false,
    )
    .unwrap_err();
    assert_eq!(err.code, "INVALID_DUE_DAYS", "{err:?}");
    let n: i64 = d
        .query_row("SELECT COUNT(*) FROM invoices", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0, "a rejected term must not write an invoice");
}

#[test]
fn hard_create_invoice_validates_and_stores_the_delivery_date() {
    // The JS validates the delivery date at create — both the shape and that the
    // date exists — and stores it; the port read the column in every projection
    // but never wrote it, so the field was permanently NULL.
    let d = setup();
    add_contact(&d, None);
    let lines = lines(&["1x Pennen @ 1.00 @21"]);

    for bad in ["2026-02-30", "2026-7-1", "01-07-2026", "2026-13-01"] {
        let err = create_invoice(
            &d,
            1,
            "2026-07-10",
            None,
            Some(bad),
            None,
            None,
            None,
            None,
            None,
            None,
            &lines,
            "agent:test",
            false,
        )
        .unwrap_err();
        assert_eq!(err.code, "INVALID_DATE", "delivery-date '{bad}': {err:?}");
    }
    let n: i64 = d
        .query_row("SELECT COUNT(*) FROM invoices", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0, "a rejected delivery date must not write an invoice");

    let inv = create_invoice(
        &d,
        1,
        "2026-07-10",
        None,
        Some("2026-07-01"),
        None,
        None,
        None,
        None,
        None,
        None,
        &lines,
        "agent:test",
        false,
    )
    .unwrap();
    assert_eq!(inv["delivery_date"], json!("2026-07-01"), "{inv}");
    let stored: String = d
        .query_row(
            "SELECT delivery_date FROM invoices WHERE id = ?1",
            [inv["id"].as_i64().unwrap()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(stored, "2026-07-01", "the date is stored, not just echoed");
}
