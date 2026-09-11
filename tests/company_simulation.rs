//! Ported from test/company-simulation.test.js (468 lines, 12 tests).
//! Full-year simulation of a fictitious company (Noordwind Handel BV),
//! driven through the bukio binary via std::process::Command.
//!
//! Every stage builds on the previous one — they must run sequentially in
//! one #[test] fn with a shared database, exactly like the JS before()/after().

use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Once;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Once-guarded BUKIO_CONFIG_DIR so no test ever touches ~/.bukio.
static ISOLATION: Once = Once::new();

fn isolate() {
    ISOLATION.call_once(|| {
        let d = std::env::temp_dir().join(format!("bukio-sim-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::env::set_var("BUKIO_CONFIG_DIR", &d);
    });
}

fn tmpdir(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("bukio-sim-{}-{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

const IBAN: &str = "NL91ABNA0417164300";

struct Sim {
    dir: PathBuf,
    db: PathBuf,
}

impl Sim {
    fn new(tag: &str) -> Self {
        isolate();
        let dir = tmpdir(tag);
        let db = dir.join("noordwind.db");
        Self { dir, db }
    }

    /// Run a bukio command; return (json_value, exit_ok).
    fn run(&self, args: &[&str]) -> (Value, bool) {
        let exe = env!("CARGO_BIN_EXE_bukio");
        let mut all = vec![
            "--db",
            self.db.to_str().unwrap(),
            "--actor",
            "agent:bartholomeus",
        ];
        all.extend_from_slice(args);
        all.push("--json");
        let out = Command::new(exe)
            .args(&all)
            .output()
            .unwrap_or_else(|e| panic!("bukio {all:?}: {e}"));
        let stdout = String::from_utf8_lossy(&out.stdout);
        let ok = out.status.success();
        let v: Value = serde_json::from_str(&stdout)
            .unwrap_or_else(|_| serde_json::json!({"raw": stdout.to_string()}));
        (v, ok)
    }

    /// Run expecting success; panic otherwise.
    fn ok(&self, args: &[&str]) -> Value {
        let (v, ok) = self.run(args);
        assert!(ok, "command failed ({args:?}), got: {v}");
        v
    }

    /// Trial-balance net_cents for one account code.
    fn tb(&self, code: &str) -> i64 {
        let v = self.ok(&["report", "trial-balance"]);
        v["data"]["accounts"]
            .as_array()
            .and_then(|a| a.iter().find(|a| a["code"].as_str() == Some(code)))
            .map_or(0, |a| a["net_cents"].as_i64().unwrap_or(0))
    }

    /// Build CAMT.053 XML, write to file, return path.
    fn camt(&self, name: &str, entries: &[CamtEntry]) -> String {
        let p = self.dir.join(name);
        std::fs::write(&p, camt_xml(entries)).unwrap();
        p.to_str().unwrap().to_string()
    }
}

struct CamtEntry<'a> {
    dir: &'a str,
    cents: i64,
    date: &'a str,
    party: &'a str,
    desc: &'a str,
    ref_: &'a str,
}

fn camt_xml(entries: &[CamtEntry]) -> String {
    let ntries: Vec<String> = entries
        .iter()
        .map(|e| {
            let amt = format!("{}.{:02}", e.cents / 100, e.cents % 100);
            let party_xml = if e.dir == "DBIT" {
                format!("<Cdtr><Nm>{}</Nm></Cdtr>", e.party)
            } else {
                format!("<Dbtr><Nm>{}</Nm></Dbtr>", e.party)
            };
            format!(
                concat!(
                    "    <Ntry>\n",
                    "      <Amt>{}</Amt><CdtDbtInd>{}</CdtDbtInd>\n",
                    "      <AcctSvcrRef>{}</AcctSvcrRef>\n",
                    "      <BookgDt><Dt>{}</Dt></BookgDt>\n",
                    "      <NtryDtls><TxDtls><RltdPties>{}</RltdPties>\n",
                    "        <RmtInf><Ustrd>{}</Ustrd></RmtInf>",
                    "</TxDtls></NtryDtls>\n",
                    "    </Ntry>",
                ),
                amt, e.dir, e.ref_, e.date, party_xml, e.desc
            )
        })
        .collect();
    format!(
        concat!(
            "<?xml version=\"1.0\"?>\n",
            "<Document xmlns=\"urn:iso:std:iso:20022:tech:xsd:camt.053.001.02\">\n",
            "<BkToCstmrStmt><Stmt><Acct><Id><IBAN>{iban}</IBAN></Id></Acct>\n",
            "{ntries}\n",
            "</Stmt></BkToCstmrStmt></Document>",
        ),
        iban = IBAN,
        ntries = ntries.join("\n"),
    )
}

// ---------------------------------------------------------------------------
// Stage 1 — company setup
// ---------------------------------------------------------------------------

#[test]
fn company_simulation() {
    let s = Sim::new("company-sim");

    // ── Stage 1: init (dry-run then real), capital, bank, company profile ──

    // dry-run creates nothing
    let plan = s.ok(&[
        "init",
        "--name",
        "Noordwind Handel BV",
        "--registration-id",
        "81234567",
        "--legal-form",
        "bv",
        "--vat",
        "on",
        "--dry-run",
    ]);
    assert_eq!(plan["data"]["dryRun"], true);
    assert!(!s.db.exists(), "dry-run must not create the db");

    // real init
    let init = s.ok(&[
        "init",
        "--name",
        "Noordwind Handel BV",
        "--registration-id",
        "81234567",
        "--legal-form",
        "bv",
        "--vat",
        "on",
    ]);
    assert_eq!(init["data"]["company"]["name"], "Noordwind Handel BV");
    assert_eq!(init["data"]["company"]["vat_module"], 1);

    // startkapitaal 25.000,00
    s.ok(&[
        "entry",
        "add",
        "--date",
        "2026-01-02",
        "--desc",
        "Startkapitaal",
        "--postings",
        "1100:25000.00,3000:-25000.00",
        "--post",
    ]);

    // bank account
    s.ok(&[
        "bank",
        "add",
        "--iban",
        IBAN,
        "--name",
        "Zakelijke rekening",
    ]);

    // company profile (12-vereisten before any invoice finalize)
    s.ok(&[
        "company",
        "update",
        "--address",
        "Industrieweg 12",
        "--postal-code",
        "2712 CD",
        "--city",
        "Zoetermeer",
        "--tax-id",
        "NL812345678B01",
        "--iban",
        IBAN,
    ]);

    assert_eq!(s.tb("1100"), 2500000);
    assert_eq!(s.tb("3000"), -2500000);

    // ── Stage 2: contacts + items ──

    let acme = s.ok(&[
        "contact",
        "add",
        "--name",
        "ACME B.V.",
        "--address",
        "Straat 1",
        "--postal-code",
        "1000 AA",
        "--city",
        "Amsterdam",
        "--vat-id",
        "NL999999999B01",
    ]);
    let berlin = s.ok(&[
        "contact",
        "add",
        "--name",
        "Berliner Handel GmbH",
        "--address",
        "Hauptstrasse 5",
        "--postal-code",
        "10115",
        "--city",
        "Berlin",
        "--country",
        "DE",
        "--vat-id",
        "DE123456789",
    ]);
    let kantoorx = s.ok(&[
        "contact",
        "add",
        "--name",
        "KantoorX B.V.",
        "--address",
        "Kantoorlaan 3",
        "--postal-code",
        "5611 AA",
        "--city",
        "Eindhoven",
        "--vat-id",
        "NL888888888B01",
        "--iban",
        "NL02ABNA0123456789",
    ]);
    let softhaus = s.ok(&[
        "contact",
        "add",
        "--name",
        "Softwarehaus GmbH",
        "--address",
        "IT-Park 9",
        "--postal-code",
        "80331",
        "--city",
        "Muenchen",
        "--country",
        "DE",
        "--vat-id",
        "DE987654321",
        "--iban",
        "DE89370400440532013000",
    ]);
    assert_eq!(acme["data"]["contact"]["id"], 1);
    assert_eq!(berlin["data"]["contact"]["id"], 2);
    assert_eq!(kantoorx["data"]["contact"]["id"], 3);
    assert_eq!(softhaus["data"]["contact"]["id"], 4);

    let c1 = s.ok(&[
        "item",
        "add",
        "--name",
        "Consultancy",
        "--unit",
        "h",
        "--price",
        "150.00",
        "--vat",
        "21",
    ]);
    let c2 = s.ok(&[
        "item", "add", "--name", "Boek", "--unit", "unit", "--price", "20.00", "--vat", "9",
    ]);
    let c3 = s.ok(&[
        "item",
        "add",
        "--name",
        "ICT Support",
        "--unit",
        "h",
        "--price",
        "90.00",
        "--vat",
        "21",
    ]);
    assert_eq!(c1["data"]["item"]["id"], 1);
    assert_eq!(c2["data"]["item"]["id"], 2);
    assert_eq!(c3["data"]["item"]["id"], 3);

    // ── Stage 3: sales ──

    // S1 — ACME: line discount 10% on consultancy + 9% line
    let s1 = s.ok(&[
        "invoice",
        "create",
        "--contact",
        "1",
        "--date",
        "2026-01-10",
        "--lines",
        "2x Consultancy @ 150.00 @21 @-10%,1x Boek @ 20.00 @9",
    ]);
    assert_eq!(s1["data"]["invoice"]["gross_cents"], 34850);
    let f1 = s.ok(&["invoice", "finalize", "--id", "1"]);
    assert_eq!(f1["data"]["invoice"]["invoice_number"], "2026-0001");

    // S2 — Berliner: invoice-level discount 5%
    let s2 = s.ok(&[
        "invoice",
        "create",
        "--contact",
        "2",
        "--date",
        "2026-01-15",
        "--lines",
        "1x Consultancy @ 150.00 @21,3x Boek @ 20.00 @9",
        "--discount-pct",
        "5",
    ]);
    assert_eq!(s2["data"]["invoice"]["net_cents"], 19950);
    assert_eq!(s2["data"]["invoice"]["vat_cents"], 3506);
    assert_eq!(s2["data"]["invoice"]["gross_cents"], 23456);
    assert_eq!(s2["data"]["invoice"]["discount_cents"], 1050);
    let f2 = s.ok(&["invoice", "finalize", "--id", "2"]);
    assert_eq!(f2["data"]["invoice"]["invoice_number"], "2026-0002");

    // S3 — Berliner (EU): btw-verlegde levering @RE — no VAT
    let s3 = s.ok(&[
        "invoice",
        "create",
        "--contact",
        "2",
        "--date",
        "2026-01-20",
        "--lines",
        "2x ICT Support @ 90.00 @RE",
    ]);
    assert_eq!(s3["data"]["invoice"]["net_cents"], 18000);
    assert_eq!(s3["data"]["invoice"]["vat_cents"], 0);
    assert_eq!(s3["data"]["invoice"]["gross_cents"], 18000);
    let f3 = s.ok(&["invoice", "finalize", "--id", "3"]);
    assert_eq!(f3["data"]["invoice"]["invoice_number"], "2026-0003");

    // S4 — ACME: line discount 25% on books
    let s4 = s.ok(&[
        "invoice",
        "create",
        "--contact",
        "1",
        "--date",
        "2026-02-05",
        "--lines",
        "4x Boek @ 20.00 @9 @-25%",
    ]);
    assert_eq!(s4["data"]["invoice"]["gross_cents"], 6540);
    let f4 = s.ok(&["invoice", "finalize", "--id", "4"]);
    assert_eq!(f4["data"]["invoice"]["invoice_number"], "2026-0004");

    // S5 — ACME: full-price consultancy → gets a credit note later
    let s5 = s.ok(&[
        "invoice",
        "create",
        "--contact",
        "1",
        "--date",
        "2026-02-10",
        "--lines",
        "1x Consultancy @ 150.00 @21",
    ]);
    assert_eq!(s5["data"]["invoice"]["gross_cents"], 18150);
    let f5 = s.ok(&["invoice", "finalize", "--id", "5"]);
    assert_eq!(f5["data"]["invoice"]["invoice_number"], "2026-0005");

    // credit note for S5
    let cr = s.ok(&[
        "invoice",
        "credit",
        "--id",
        "5",
        "--date",
        "2026-02-12",
        "--reason",
        "tarief gecorrigeerd",
    ]);
    assert_eq!(cr["data"]["invoice"]["invoice_type"], "credit");
    assert_eq!(cr["data"]["invoice"]["credit_for_invoice_id"], 5);
    let f6 = s.ok(&["invoice", "finalize", "--id", "6"]);
    assert_eq!(f6["data"]["invoice"]["invoice_number"], "2026-0006");

    // sales net: S1 29000 + S2 19950 + S3 18000 + S4 6000 = 72950
    assert_eq!(s.tb("8000"), -(29000 + 19950 + 18000 + 6000));
    // VAT output: S5 21% + S1+S2+S4 21% + S1+S2 9% — S5+credit net to zero
    assert_eq!(s.tb("2500"), -(5850 + 3506 + 540));
    // Debtors: S1+S2+S3+S4 = gross of paid invoices
    assert_eq!(s.tb("1200"), 34850 + 23456 + 18000 + 6540);

    // ── Stage 4: purchases ──

    // P1 — kantoorartikelen 21%
    s.ok(&[
        "vat",
        "book",
        "--date",
        "2026-01-08",
        "--desc",
        "KantoorX - KX-2026-001",
        "--postings",
        "4300:50.00@21,1100:-60.50",
        "--post",
    ]);
    // P2 — EU software, btw verlegd
    s.ok(&[
        "vat",
        "book",
        "--date",
        "2026-01-12",
        "--desc",
        "Softwarehaus - SH-2026-042 (EU verlegd)",
        "--postings",
        "4340:300.00@RE,1100:-300.00",
        "--post",
    ]);
    // P3 — binnenlands verlegd
    s.ok(&[
        "vat",
        "book",
        "--date",
        "2026-01-18",
        "--desc",
        "Bouwbedrijf De Lier - 2026-013 (verlegd)",
        "--postings",
        "4000:400.00@R,1100:-400.00",
        "--post",
    ]);
    // P4 — marketing 21%
    s.ok(&[
        "vat",
        "book",
        "--date",
        "2026-02-03",
        "--desc",
        "KantoorX - KX-2026-014",
        "--postings",
        "4300:100.00@21,1100:-121.00",
        "--post",
    ]);
    // P5 — verzekering 21%
    s.ok(&[
        "vat",
        "book",
        "--date",
        "2026-02-15",
        "--desc",
        "Verzekeraar - polis 2026",
        "--postings",
        "4320:40.00@21,1100:-48.40",
        "--post",
    ]);

    // Only standard input VAT lands in 1500: 1050+2100+840
    assert_eq!(s.tb("1500"), 1050 + 2100 + 840);
    assert_eq!(
        s.tb("1100"),
        2500000 - (6050 + 30000 + 40000 + 12100 + 4840)
    );

    // ── Stage 5: bank import + auto-match ──

    let stmt = s.camt(
        "q1.xml",
        &[
            CamtEntry {
                dir: "DBIT",
                cents: 6050,
                date: "2026-01-08",
                party: "KantoorX B.V.",
                desc: "KX-2026-001",
                ref_: "NTRY001",
            },
            CamtEntry {
                dir: "DBIT",
                cents: 30000,
                date: "2026-01-12",
                party: "Softwarehaus GmbH",
                desc: "SH-2026-042",
                ref_: "NTRY002",
            },
            CamtEntry {
                dir: "DBIT",
                cents: 40000,
                date: "2026-01-18",
                party: "Bouwbedrijf De Lier",
                desc: "2026-013",
                ref_: "NTRY003",
            },
            CamtEntry {
                dir: "DBIT",
                cents: 12100,
                date: "2026-02-03",
                party: "KantoorX B.V.",
                desc: "KX-2026-014",
                ref_: "NTRY004",
            },
            CamtEntry {
                dir: "DBIT",
                cents: 4840,
                date: "2026-02-15",
                party: "Verzekeraar",
                desc: "polis 2026",
                ref_: "NTRY005",
            },
            CamtEntry {
                dir: "CRDT",
                cents: 34850,
                date: "2026-02-01",
                party: "ACME B.V.",
                desc: "2026-0001",
                ref_: "NTRY006",
            },
            CamtEntry {
                dir: "CRDT",
                cents: 23456,
                date: "2026-02-15",
                party: "Berliner Handel GmbH",
                desc: "2026-0002",
                ref_: "NTRY007",
            },
            CamtEntry {
                dir: "CRDT",
                cents: 18000,
                date: "2026-02-15",
                party: "Berliner Handel GmbH",
                desc: "2026-0003",
                ref_: "NTRY008",
            },
            CamtEntry {
                dir: "CRDT",
                cents: 6540,
                date: "2026-03-01",
                party: "ACME B.V.",
                desc: "2026-0004",
                ref_: "NTRY009",
            },
        ],
    );
    let imp = s.ok(&["bank", "import", "--file", &stmt, "--iban", IBAN]);
    assert_eq!(imp["ok"], true);

    let m = s.ok(&["bank", "match", "auto"]);
    assert_eq!(m["data"]["matched"].as_array().unwrap().len(), 9);
    assert_eq!(m["data"]["unmatched_remaining"], 0);

    // invoices 1-4 paid
    let list = s.ok(&["invoice", "list"]);
    for id in [1, 2, 3, 4] {
        let inv = list["data"]["invoices"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["id"].as_i64() == Some(id))
            .unwrap();
        assert_eq!(inv["status"], "paid", "invoice {id} must be paid");
        assert_eq!(inv["outstanding_cents"], 0);
    }
    assert_eq!(s.tb("1200"), 18150 - 18150);
    assert_eq!(
        s.tb("1100"),
        2500000 - (6050 + 30000 + 40000 + 12100 + 4840) + (34850 + 23456 + 18000 + 6540)
    );

    // ── Stage 6: Q1 OB readout ──

    let r = s.ok(&["vat", "readout", "--period", "2026-Q1"]);
    let f = &r["data"]["fields"];
    assert_eq!(f["1a"]["cents"], 41250);
    assert_eq!(f["1b"]["cents"], 13700);
    assert_eq!(f["1c"]["cents"], 0);
    assert_eq!(f["2a"]["cents"], 18000);
    assert_eq!(f["3a"]["cents"], 59000);
    assert_eq!(f["3b"]["cents"], 30000);
    assert_eq!(f["4a"]["cents"], 8400);
    assert_eq!(f["4b"]["cents"], 6300);
    assert_eq!(f["5a"]["cents"], 9896);
    assert_eq!(f["5b"]["cents"], 18690);
    assert_eq!(f["5d"]["cents"], 5906);
    assert_eq!(r["data"]["to_pay"], "59.06");

    // ── Stage 7: Q1 filing + settlement ──

    let plan = s.ok(&["vat", "file", "--period", "2026-Q1", "--dry-run"]);
    assert_eq!(plan["data"]["dryRun"], true);
    assert_eq!(plan["data"]["liability_cents"], 5906);

    let file = s.ok(&["vat", "file", "--period", "2026-Q1"]);
    assert_eq!(file["data"]["owe"], true);
    assert_eq!(file["data"]["liability_cents"], 5906);
    assert_eq!(file["data"]["account"], "2510");
    assert_eq!(s.tb("2510"), -5906);
    assert_eq!(s.tb("1500"), 0);
    assert_eq!(s.tb("2500"), 0);

    // pay the VAT return (59,00 → 6-cent rounding gain)
    let stmt2 = s.camt(
        "q1-vat.xml",
        &[CamtEntry {
            dir: "DBIT",
            cents: 5900,
            date: "2026-04-10",
            party: "Belastingdienst",
            desc: "OB-aangifte 2026-Q1",
            ref_: "NTRY010",
        }],
    );
    s.ok(&["bank", "import", "--file", &stmt2, "--iban", IBAN]);
    let txs = s.ok(&["bank", "transactions", "--state", "unmatched"]);
    let vat_tx = txs["data"]["transactions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["counterparty"].as_str() == Some("Belastingdienst"))
        .expect("Belastingdienst tx not found");
    let tx_id = vat_tx["id"].to_string();

    let s_plan = s.ok(&[
        "vat",
        "settle",
        "--tx",
        &tx_id,
        "--period",
        "2026-Q1",
        "--dry-run",
    ]);
    assert_eq!(s_plan["data"]["difference_cents"], -6);

    let settle = s.ok(&["vat", "settle", "--tx", &tx_id, "--period", "2026-Q1"]);
    assert_eq!(settle["data"]["difference_cents"], -6);
    assert_eq!(s.tb("2510"), 0);
    assert_eq!(s.tb("4700"), -6);
    assert_eq!(
        s.tb("1100"),
        2500000 - (6050 + 30000 + 40000 + 12100 + 4840) + (34850 + 23456 + 18000 + 6540) - 5900
    );

    // ── Stage 8: P&L, sales, aging, statement, month-end ──

    let pnl = s.ok(&["report", "pnl", "--year", "2026"]);
    assert_eq!(pnl["data"]["revenue_cents"], 72950);
    assert_eq!(pnl["data"]["costs_cents"], 88994);
    assert_eq!(pnl["data"]["result_cents"], -16044);

    let sales = s.ok(&["report", "sales", "--year", "2026", "--by", "contact"]);
    let acme_g = sales["data"]["groups"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"].as_str() == Some("ACME B.V."))
        .unwrap();
    let berlin_g = sales["data"]["groups"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"].as_str() == Some("Berliner Handel GmbH"))
        .unwrap();
    assert_eq!(acme_g["gross_cents"], 59540);
    assert_eq!(berlin_g["gross_cents"], 41456);
    assert_eq!(
        acme_g["gross_cents"].as_i64().unwrap() + berlin_g["gross_cents"].as_i64().unwrap(),
        100996
    );

    let aging = s.ok(&[
        "report",
        "aging",
        "--as-of",
        "2026-12-31",
        "--kind",
        "debtors",
    ]);
    let aging_acme = aging["data"]["debtors"]["contacts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"].as_str() == Some("ACME B.V."))
        .unwrap();
    assert_eq!(aging_acme["total_cents"], 0);
    assert_eq!(aging_acme["items"][0]["ref"], "2026-0005");
    assert_eq!(aging_acme["items"][0]["outstanding_cents"], 0);

    let stmt_r = s.ok(&["contact", "statement", "--id", "1", "--as-of", "2026-12-31"]);
    assert_eq!(stmt_r["data"]["balance_cents"], 0);

    let me = s.ok(&["month-end", "--period", "2026-03"]);
    assert_eq!(me["data"]["period"], "2026-03");
    assert!(me["data"]["warnings"].is_array());

    // ── Stage 9: Q2 — continuity, second filing cycle ──

    // P6 — hosting 21%
    s.ok(&[
        "vat",
        "book",
        "--date",
        "2026-04-08",
        "--desc",
        "MijnHosting - hosting Q2",
        "--postings",
        "4340:80.00@21,1100:-96.80",
        "--post",
    ]);
    // S6 — ACME: 2x consultancy
    let s6 = s.ok(&[
        "invoice",
        "create",
        "--contact",
        "1",
        "--date",
        "2026-04-05",
        "--lines",
        "2x Consultancy @ 150.00 @21",
    ]);
    assert_eq!(s6["data"]["invoice"]["gross_cents"], 36300);
    let f7 = s.ok(&["invoice", "finalize", "--id", "7"]);
    assert_eq!(f7["data"]["invoice"]["invoice_number"], "2026-0007");

    // bank import + match Q2
    let q2_stmt = s.camt(
        "q2.xml",
        &[
            CamtEntry {
                dir: "DBIT",
                cents: 9680,
                date: "2026-04-08",
                party: "MijnHosting",
                desc: "hosting Q2",
                ref_: "NTRY011",
            },
            CamtEntry {
                dir: "CRDT",
                cents: 36300,
                date: "2026-04-20",
                party: "ACME B.V.",
                desc: "2026-0007",
                ref_: "NTRY012",
            },
        ],
    );
    s.ok(&["bank", "import", "--file", &q2_stmt, "--iban", IBAN]);
    let m2 = s.ok(&["bank", "match", "auto"]);
    assert_eq!(m2["data"]["matched"].as_array().unwrap().len(), 2);
    assert_eq!(m2["data"]["unmatched_remaining"], 0);

    // Q2 readout — only Q2 activity
    let r2 = s.ok(&["vat", "readout", "--period", "2026-Q2"]);
    let f2 = &r2["data"]["fields"];
    assert_eq!(f2["1a"]["cents"], 30000);
    assert_eq!(f2["2a"]["cents"], 0);
    assert_eq!(f2["3a"]["cents"], 8000);
    assert_eq!(f2["5a"]["cents"], 6300);
    assert_eq!(f2["5b"]["cents"], 1680);
    assert_eq!(f2["5d"]["cents"], 4620);

    // file + settle Q2
    let file2 = s.ok(&["vat", "file", "--period", "2026-Q2"]);
    assert_eq!(file2["data"]["liability_cents"], 4620);
    assert_eq!(s.tb("2510"), -4620);

    let q2_vat_stmt = s.camt(
        "q2-vat.xml",
        &[CamtEntry {
            dir: "DBIT",
            cents: 4600,
            date: "2026-04-25",
            party: "Belastingdienst",
            desc: "OB-aangifte 2026-Q2",
            ref_: "NTRY013",
        }],
    );
    s.ok(&["bank", "import", "--file", &q2_vat_stmt, "--iban", IBAN]);
    let txs2 = s.ok(&["bank", "transactions", "--state", "unmatched"]);
    let vat_tx2 = txs2["data"]["transactions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["counterparty"].as_str() == Some("Belastingdienst"))
        .unwrap();
    let tx_id2 = vat_tx2["id"].to_string();
    let settle2 = s.ok(&["vat", "settle", "--tx", &tx_id2, "--period", "2026-Q2"]);
    assert_eq!(settle2["data"]["difference_cents"], -20);
    assert_eq!(s.tb("2510"), 0);
    assert_eq!(s.tb("4700"), -26);

    // ── Stage 10: payables + SEPA batch ──

    s.ok(&[
        "payments",
        "payables",
        "add",
        "--contact",
        "3",
        "--ref",
        "KX-2026-014",
        "--date",
        "2026-02-03",
        "--due",
        "2026-03-03",
        "--amount",
        "121.00",
    ]);
    s.ok(&[
        "payments",
        "payables",
        "add",
        "--contact",
        "4",
        "--ref",
        "SH-2026-042",
        "--date",
        "2026-01-12",
        "--due",
        "2026-02-12",
        "--amount",
        "300.00",
    ]);

    let batch = s.ok(&["payments", "batch", "create", "--from-invoices"]);
    assert_eq!(batch["data"]["id"], 1);
    assert_eq!(batch["data"]["lines"].as_array().unwrap().len(), 2);

    let pain_path = s.dir.join("betalingen.xml");
    let pain_str = pain_path.to_str().unwrap();
    let exp = s.ok(&[
        "payments", "batch", "export", "--id", "1", "--schema", "001.03", "--out", pain_str,
    ]);
    assert_eq!(exp["data"]["batch_id"], 1);
    assert!(!exp["data"]["msg_id"].is_null());
    assert!(pain_path.exists(), "pain.001 file must be written");
    let xml = std::fs::read_to_string(&pain_path).unwrap();
    assert!(xml.contains("PmtInf"));

    // mark both payables paid
    s.ok(&["payments", "payables", "pay", "--id", "1"]);
    s.ok(&["payments", "payables", "pay", "--id", "2"]);
    let paid = s.ok(&["payments", "payables", "list", "--status", "paid"]);
    assert_eq!(paid["data"]["payables"].as_array().unwrap().len(), 2);

    // ── Stage 11: year-end close, jaarrekening, ICP ──

    let status = s.ok(&["year-end", "status", "--year", "2026"]);
    assert_eq!(status["data"]["status"]["closed"], false);
    assert_eq!(status["data"]["status"]["result_cents"], 5976);

    let yp = s.ok(&["year-end", "close", "--year", "2026", "--dry-run"]);
    assert_eq!(yp["data"]["dryRun"], true);
    let close = s.ok(&["year-end", "close", "--year", "2026"]);
    assert_eq!(close["data"]["closed"], true);
    // result 59.76 moves to equity: 25000.00 + 59.76
    assert_eq!(s.tb("3000"), -(2500000 + 5976));
    assert_eq!(s.tb("9900"), 0);

    // micro jaarekening
    let jk = s.ok(&[
        "financial-statements",
        "report",
        "--year",
        "2026",
        "--model",
        "micro",
        "--format",
        "json",
    ]);
    assert_eq!(
        jk["data"]["financial_statements"]["balans"]["balanced"],
        true
    );
    assert_eq!(
        jk["data"]["financial_statements"]["balans"]["total_activa_cents"],
        2505976
    );
    assert_eq!(
        jk["data"]["financial_statements"]["balans"]["total_passiva_cents"],
        2505976
    );

    // ICP
    let icp = s.ok(&["icp", "readout", "--period", "2026-Q1"]);
    assert_eq!(icp["data"]["total_cents"], 18000);
    assert_eq!(icp["data"]["customers"][0]["vat_id"], "DE123456789");
    assert_eq!(icp["data"]["customers"][0]["amount_cents"], 18000);

    // ── Stage 12: final verification ──

    assert_eq!(s.tb("1100"), 2505976);
    assert_eq!(s.tb("1200"), 0);
    assert_eq!(s.tb("1500"), 0);
    assert_eq!(s.tb("2500"), 0);
    assert_eq!(s.tb("2510"), 0);

    let audit = s.ok(&["audit"]);
    assert!(audit["data"]["entries"].as_array().unwrap().len() > 20);
    for e in audit["data"]["entries"].as_array().unwrap() {
        let actor = e["actor"].as_str().unwrap_or("");
        assert!(!actor.is_empty(), "every audit entry must have an actor");
    }

    let backup_path = s.dir.join("backup.db");
    let bp = backup_path.to_str().unwrap();
    let bk = s.ok(&["backup", "--out", bp]);
    assert_eq!(bk["ok"], true);
    assert!(backup_path.exists());
}
