//! VAT module (optional) — codes, VAT-aware booking, OB manual-filing readout.
//! Only active when company.vat_module = 1. The core ledger stays VAT-agnostic:
//! this module expands "@code" posting specs into full entries (including the
//! VAT ledger legs) and computes the OB-aangifte fields for manual filing.
//! Port of the Node `src/vat/index.js`.

use crate::core::accounts::{create_account_from_seed, get_account_by_code};
use crate::core::chart::AccountSeed;
use crate::core::entries::{create_entry, post_entry, Entry, PostingSpec};
use crate::core::money::parse_amount;
use crate::error::{AppError, Result};
use regex::Regex;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::sync::OnceLock;

pub const VAT_ACCOUNTS: &[AccountSeed] = &[
    AccountSeed { code: "1500", name: "Te vorderen omzetbelasting", account_type: "asset", normal_balance: "debit", rgs_code: "BVOR.11" },
    AccountSeed { code: "2500", name: "Te betalen omzetbelasting", account_type: "liability", normal_balance: "credit", rgs_code: "BSCH.12" },
];

#[derive(Debug, Clone, Copy)]
pub struct VatCodeSeed {
    pub code: &'static str,
    pub rate_bp: i64,
    pub vat_type: &'static str,
    pub eu_reverse: i64,
    pub description: &'static str,
}

pub const VAT_CODES: &[VatCodeSeed] = &[
    VatCodeSeed { code: "21", rate_bp: 2100, vat_type: "standard", eu_reverse: 0, description: "21% hoog tarief" },
    VatCodeSeed { code: "9", rate_bp: 900, vat_type: "standard", eu_reverse: 0, description: "9% laag tarief" },
    VatCodeSeed { code: "0", rate_bp: 0, vat_type: "standard", eu_reverse: 0, description: "0% nultarief" },
    VatCodeSeed { code: "V", rate_bp: 0, vat_type: "exempt", eu_reverse: 0, description: "Vrijgesteld" },
    VatCodeSeed { code: "R", rate_bp: 0, vat_type: "reverse", eu_reverse: 0, description: "Verlegd (binnenland)" },
    VatCodeSeed { code: "RE", rate_bp: 0, vat_type: "reverse", eu_reverse: 1, description: "Verlegd (EU)" },
    VatCodeSeed { code: "M", rate_bp: 0, vat_type: "margin", eu_reverse: 0, description: "Marge" },
    VatCodeSeed { code: "P", rate_bp: 0, vat_type: "private", eu_reverse: 0, description: "Privégebruik" },
];

pub fn is_vat_enabled(conn: &Connection) -> Result<bool> {
    let row: Option<(i64, i64)> = conn
        .query_row("SELECT vat_module, kor_flag FROM company", [], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?;
    Ok(match row {
        Some((vat_module, kor_flag)) => vat_module == 1 && kor_flag == 0,
        None => false,
    })
}

pub fn require_vat(conn: &Connection) -> Result<()> {
    if !is_vat_enabled(conn)? {
        return Err(AppError::new(
            "VAT_MODULE_OFF",
            "the VAT module is not enabled for this company (enable with `bukio vat enable`)",
        ));
    }
    Ok(())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct VatEnableResult {
    pub vat_module: i64,
    pub accounts: Vec<String>,
    pub codes: Vec<String>,
}

/// Enable the VAT module: flag + VAT accounts + VAT codes. Idempotent.
pub fn enable_vat_module(conn: &Connection, actor: &str) -> Result<VatEnableResult> {
    let (_vat_module, kor_flag): (i64, i64) = conn.query_row(
        "SELECT vat_module, kor_flag FROM company",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    if kor_flag == 1 {
        return Err(AppError::new(
            "KOR_ACTIVE",
            "this company uses the KOR (kleineondernemersregeling) — the VAT module cannot be enabled",
        ));
    }
    let tx = conn.unchecked_transaction()?;
    tx.execute("UPDATE company SET vat_module = 1 WHERE id = 1", [])?;
    for a in VAT_ACCOUNTS {
        if get_account_by_code(&tx, a.code)?.is_none() {
            create_account_from_seed(&tx, a)?;
        }
    }
    let insert_code = "INSERT OR IGNORE INTO vat_codes (code, rate_bp, type, eu_reverse, description) VALUES (?1, ?2, ?3, ?4, ?5)";
    for c in VAT_CODES {
        tx.execute(insert_code, params![c.code, c.rate_bp, c.vat_type, c.eu_reverse, c.description])?;
    }
    crate::audit::record(&tx, actor, "vat.enable", Some("vat enable"), Some(&serde_json::json!({})), "ok", &[])?;
    tx.commit()?;
    Ok(VatEnableResult {
        vat_module: 1,
        accounts: VAT_ACCOUNTS.iter().map(|a| a.code.to_string()).collect(),
        codes: VAT_CODES.iter().map(|c| c.code.to_string()).collect(),
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct VatCodeRow {
    pub code: String,
    pub rate_bp: i64,
    pub rate: String,
    #[serde(rename = "type")]
    pub vat_type: String,
    pub eu_reverse: bool,
    pub description: String,
}

pub fn list_vat_codes(conn: &Connection) -> Result<Vec<VatCodeRow>> {
    let mut stmt = conn.prepare("SELECT * FROM vat_codes ORDER BY rate_bp DESC, code")?;
    let rows = stmt
        .query_map([], |r| {
            let rate_bp: i64 = r.get(2)?;
            let eu_reverse: i64 = r.get(4)?;
            Ok(VatCodeRow {
                code: r.get(1)?,
                rate_bp,
                rate: format!("{:.1}%", rate_bp as f64 / 100.0),
                vat_type: r.get(3)?,
                eu_reverse: eu_reverse != 0,
                description: r.get(5)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// VAT codes with their type labels (used by the readout later).
pub fn vat_type_of(conn: &Connection, code: &str) -> Result<Option<String>> {
    let t: Option<String> = conn
        .query_row("SELECT type FROM vat_codes WHERE code = ?1", [code], |r| r.get(0))
        .optional()?;
    Ok(t)
}

/// A raw VAT-aware posting spec: CODE:AMOUNT[@VATCODE] (fx fields set later).
#[derive(Debug, Clone)]
pub struct VatSpec {
    pub code: String,
    pub amount_cents: i64,
    pub vat_code: Option<String>,
    pub fx_currency: Option<String>,
    pub fx_amount_cents: Option<i64>,
}

fn vat_posting_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^(\d{1,6}):(.+?)(?:@([A-Z0-9]+))?$").unwrap())
}

/// Parse posting specs with optional VAT: "CODE:AMOUNT[@VATCODE]".
pub fn parse_vat_posting_specs(raw: &[String]) -> Result<Vec<VatSpec>> {
    let mut out = Vec::new();
    for item in raw {
        for token in item.split(',') {
            let t = token.trim();
            if t.is_empty() {
                continue;
            }
            let caps = vat_posting_re().captures(t).ok_or_else(|| {
                AppError::new(
                    "INVALID_POSTING",
                    format!("posting '{t}' must be CODE:AMOUNT[@VATCODE] (e.g. 8000:-100.00@21)"),
                )
            })?;
            let code = caps.get(1).unwrap().as_str().to_string();
            let amount_cents = parse_amount(caps.get(2).unwrap().as_str())?;
            let vat_code = caps.get(3).map(|m| m.as_str().to_string());
            out.push(VatSpec { code, amount_cents, vat_code, fx_currency: None, fx_amount_cents: None });
        }
    }
    Ok(out)
}

/// An expanded posting (core posting + computed VAT fields).
#[derive(Debug, Clone)]
pub struct ExpandedPosting {
    pub code: String,
    pub amount_cents: i64,
    pub vat_code: Option<String>,
    pub vat_amount_cents: Option<i64>,
    pub fx_currency: Option<String>,
    pub fx_amount_cents: Option<i64>,
    /// true for the auto-added VAT ledger leg (1500/2500). Node's CLI
    /// serialises vatCode as `null` for plain postings but omits the key for
    /// the auto legs (the JS property is undefined there) — this flag lets us
    /// reproduce that exactly.
    pub vat_leg: bool,
}

/// Expand VAT-aware posting specs into core postings:
/// - '@code' postings get vat_code + computed vat_amount (net = amount).
/// - A VAT ledger leg is added automatically (2500 te betalen for output /
///   reverse / private, 1500 te vorderen for input).
/// The caller's other postings must balance the gross amounts.
pub fn expand_vat_postings(conn: &Connection, specs: &[VatSpec]) -> Result<Vec<ExpandedPosting>> {
    require_vat(conn)?;
    let mut expanded: Vec<ExpandedPosting> = Vec::new();
    let mut vat_legs: Vec<ExpandedPosting> = Vec::new();

    for spec in specs {
        let account = get_account_by_code(conn, &spec.code)?.ok_or_else(|| {
            AppError::new("ACCOUNT_NOT_FOUND", format!("account {} does not exist", spec.code))
        })?;
        if let Some(vat_code) = &spec.vat_code {
            let vat: VatCodeLookup = conn
                .query_row(
                    "SELECT code, rate_bp, type, eu_reverse FROM vat_codes WHERE code = ?1",
                    [vat_code],
                    |r| Ok(VatCodeLookup { code: r.get(0)?, rate_bp: r.get(1)?, vat_type: r.get(2)?, eu_reverse: r.get(3)? }),
                )
                .optional()?
                .ok_or_else(|| {
                    AppError::new("VAT_CODE_NOT_FOUND", format!("vat code '{vat_code}' does not exist"))
                })?;
            if vat.vat_type == "margin" {
                return Err(AppError::new(
                    "VAT_MARGIN_NOT_SUPPORTED",
                    "margeregeling cannot be split automatically — book it manually",
                ));
            }
            // Reverse charge / privégebruik: 0% codes, but the VAT due is
            // computed at the standard rate (21%).
            let effective_rate_bp = if vat.vat_type == "reverse" || vat.vat_type == "private" {
                2100
            } else {
                vat.rate_bp
            };
            let vat_amount = round_half_up(spec.amount_cents.abs() * effective_rate_bp, 10000)
                * spec.amount_cents.signum();

            let is_output = account.account_type == "income" || vat.vat_type == "private";
            let vat_account_code = if vat.vat_type == "reverse" || is_output { "2500" } else { "1500" };
            expanded.push(ExpandedPosting {
                code: spec.code.clone(),
                amount_cents: spec.amount_cents,
                vat_code: Some(vat.code.clone()),
                vat_amount_cents: Some(vat_amount),
                fx_currency: spec.fx_currency.clone(),
                fx_amount_cents: spec.fx_amount_cents,
                vat_leg: false,
            });
            // The VAT leg carries the SAME sign as the tagged posting.
            vat_legs.push(ExpandedPosting {
                code: vat_account_code.to_string(),
                amount_cents: vat_amount,
                vat_code: None,
                vat_amount_cents: None,
                fx_currency: None,
                fx_amount_cents: None,
                vat_leg: true,
            });
        } else {
            expanded.push(ExpandedPosting {
                code: spec.code.clone(),
                amount_cents: spec.amount_cents,
                vat_code: None,
                vat_amount_cents: None,
                fx_currency: spec.fx_currency.clone(),
                fx_amount_cents: spec.fx_amount_cents,
                vat_leg: false,
            });
        }
    }
    expanded.extend(vat_legs);
    Ok(expanded)
}

struct VatCodeLookup {
    code: String,
    rate_bp: i64,
    vat_type: String,
    eu_reverse: i64,
}

/// Integer round-half-up: round(n / denom).
fn round_half_up(n: i64, denom: i64) -> i64 {
    (n + denom / 2) / denom
}

fn expanded_to_spec(p: &ExpandedPosting) -> PostingSpec {
    PostingSpec {
        code: p.code.clone(),
        amount_cents: p.amount_cents,
        vat_code: p.vat_code.clone(),
        vat_amount_cents: p.vat_amount_cents,
        fx_currency: p.fx_currency.clone(),
        fx_amount_cents: p.fx_amount_cents,
    }
}

/// Book a VAT-aware entry via the core engine.
#[allow(clippy::too_many_arguments)]
pub fn book_vat_entry(
    conn: &Connection,
    date: &str,
    description: &str,
    postings: &[VatSpec],
    source: &str,
    source_ref: Option<&str>,
    actor: &str,
    post: bool,
) -> Result<(Entry, Vec<ExpandedPosting>)> {
    let expanded = expand_vat_postings(conn, postings)?;
    let core_postings: Vec<PostingSpec> = expanded.iter().map(expanded_to_spec).collect();
    let mut entry = create_entry(conn, date, description, &core_postings, source, source_ref, actor)?;
    if post {
        entry = post_entry(conn, entry.meta.id, actor)?;
    }
    Ok((entry, expanded))
}

/// Parse '2026-Q2' or '2026-07' into { from, to } (ISO dates).
pub struct Period {
    pub from: String,
    pub to: String,
    pub label: String,
}

pub fn parse_period(period: &str) -> Result<Period> {
    let q_re = Regex::new(r"^(\d{4})-Q([1-4])$").unwrap();
    let m_re = Regex::new(r"^(\d{4})-(\d{2})$").unwrap();
    if let Some(caps) = q_re.captures(period) {
        let y: i32 = caps[1].parse().unwrap();
        let qn: u32 = caps[2].parse().unwrap();
        let from = format!("{y}-{:02}-01", (qn - 1) * 3 + 1);
        let to = month_end(y, qn * 3);
        return Ok(Period { from, to, label: period.to_string() });
    }
    if let Some(caps) = m_re.captures(period) {
        let y: i32 = caps[1].parse().unwrap();
        let mn: u32 = caps[2].parse().unwrap();
        if !(1..=12).contains(&mn) {
            return Err(AppError::new("INVALID_PERIOD", format!("period '{period}' must be YYYY-Qn or YYYY-MM")));
        }
        let from = format!("{y}-{mn:02}-01");
        let to = month_end(y, mn);
        return Ok(Period { from, to, label: period.to_string() });
    }
    Err(AppError::new("INVALID_PERIOD", format!("period '{period}' must be YYYY-Qn or YYYY-MM")))
}

fn month_end(year: i32, month: u32) -> String {
    let d = chrono::NaiveDate::from_ymd_opt(year, month, 1)
        .and_then(|d| d.checked_add_months(chrono::Months::new(1)))
        .and_then(|d| d.checked_sub_days(chrono::Days::new(1)))
        .unwrap();
    d.format("%Y-%m-%d").to_string()
}

#[derive(Debug, Clone)]
pub struct ObFieldRow {
    pub amount_cents: i64,
    pub vat_amount_cents: Option<i64>,
    pub vat_code: String,
    pub rate_bp: i64,
    pub vat_type: String,
    pub eu_reverse: i64,
    pub account_type: String,
}

/// OB-aangifte manual-filing readout for a period (fields 1a-5d).
/// This is an aid for MANUAL filing in Mijn Belastingdienst — bukio never
/// submits anything.
pub struct ObReadout {
    pub period: String,
    pub from: String,
    pub to: String,
    pub fields: Vec<(String, i64)>, // ordered field -> cents
    pub to_pay_cents: i64,
    pub to_pay: String,
    pub note: String,
}

pub fn ob_readout(conn: &Connection, period: &str) -> Result<ObReadout> {
    require_vat(conn)?;
    let p = parse_period(period)?;

    let mut stmt = conn.prepare(
        "SELECT p.amount_cents, p.vat_amount_cents, vc.code AS vat_code, vc.rate_bp, vc.type, vc.eu_reverse,\n\
                a.type AS account_type, a.code AS account_code\n\
         FROM postings p\n\
         JOIN journal_entries e ON e.id = p.entry_id AND e.state = 'posted'\n\
         JOIN vat_codes vc ON vc.id = p.vat_code_id\n\
         JOIN accounts a ON a.id = p.account_id\n\
         WHERE e.date >= ?1 AND e.date <= ?2\n\
           AND p.vat_code_id IS NOT NULL\n\
           AND a.code NOT IN ('1500','2500')\n\
         ORDER BY e.id, p.id",
    )?;
    let rows: Vec<(i64, Option<i64>, String, i64, String, i64, String, String)> = stmt
        .query_map(params![p.from, p.to], |r| {
            Ok((
                r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut f: std::collections::BTreeMap<&str, i64> = [
        ("1a", 0), ("1b", 0), ("1c", 0), ("1d", 0),
        ("2a", 0), ("2b", 0),
        ("3a", 0), ("3b", 0), ("3c", 0),
        ("4a", 0), ("4b", 0),
        ("5a", 0), ("5b", 0), ("5c", 0),
    ]
    .into_iter()
    .collect();

    for (amount_cents, vat_amount_cents, _vat_code, rate_bp, vat_type, eu_reverse, account_type, _account_code) in rows {
        let vat_amount = vat_amount_cents.unwrap_or(0);
        if vat_type == "standard" || vat_type == "exempt" {
            if account_type == "income" {
                let base = -amount_cents;
                match rate_bp {
                    2100 => *f.get_mut("1a").unwrap() += base,
                    900 => *f.get_mut("1b").unwrap() += base,
                    _ => *f.get_mut("1c").unwrap() += base,
                }
                if vat_type == "standard" {
                    *f.get_mut("5a").unwrap() += -vat_amount;
                }
            } else {
                let base = amount_cents;
                match rate_bp {
                    2100 => *f.get_mut("3a").unwrap() += base,
                    900 => *f.get_mut("3b").unwrap() += base,
                    _ => *f.get_mut("3c").unwrap() += base,
                }
                if vat_type == "standard" {
                    *f.get_mut("5b").unwrap() += vat_amount;
                }
            }
        } else if vat_type == "reverse" {
            if account_type == "income" && eu_reverse != 0 {
                *f.get_mut("2a").unwrap() += -amount_cents;
            } else if account_type == "expense" {
                let key = if eu_reverse != 0 { "3b" } else { "3a" };
                *f.get_mut(key).unwrap() += amount_cents;
                let vat_key = if eu_reverse != 0 { "4b" } else { "4a" };
                *f.get_mut(vat_key).unwrap() += vat_amount;
                *f.get_mut("5b").unwrap() += vat_amount;
            }
        } else if vat_type == "private" {
            *f.get_mut("1d").unwrap() += -amount_cents;
            *f.get_mut("5a").unwrap() += -vat_amount;
        }
        // type 'margin' is excluded from the return
    }

    let f5a = f["5a"];
    let f4a = f["4a"];
    let f4b = f["4b"];
    let f5c = f["5c"];
    let f5b = f["5b"];
    let f5d = f5a + f4a + f4b + f5c - f5b;
    f.insert("5d", f5d);

    let fields: Vec<(String, i64)> = [
        "1a", "1b", "1c", "1d", "2a", "2b", "3a", "3b", "3c", "4a", "4b", "5a", "5b", "5c", "5d",
    ]
    .iter()
    .map(|k| (k.to_string(), f[*k]))
    .collect();

    Ok(ObReadout {
        period: p.label,
        from: p.from,
        to: p.to,
        fields,
        to_pay_cents: f5d,
        to_pay: crate::core::money::format_amount(f5d),
        note: "Manual filing aid only — bukio never submits. Fields 2b (non-EU exports) and 5c are not tracked and shown as 0.".to_string(),
    })
}

/// Record that a period was filed manually.
pub struct MarkFiledResult {
    pub period: String,
    pub from: String,
    pub to: String,
    pub status: String,
}

pub fn mark_filed(conn: &Connection, period: &str, actor: &str) -> Result<MarkFiledResult> {
    require_vat(conn)?;
    let p = parse_period(period)?;
    let readout = ob_readout(conn, &p.label)?;
    let fields_json = serde_json::to_string(
        &readout.fields.iter().map(|(k, v)| (k, v)).collect::<std::collections::BTreeMap<_, _>>(),
    )
    .unwrap();
    conn.execute(
        "INSERT INTO vat_returns (type, period, status, fields_json, filed_at)
         VALUES ('OB', ?1, 'filed', ?2, ?3)
         ON CONFLICT(type, period) DO UPDATE SET status = 'filed', fields_json = excluded.fields_json, filed_at = excluded.filed_at",
        params![p.label, fields_json, crate::core::entries::now_iso()],
    )?;
    crate::audit::record(
        conn,
        actor,
        "vat.filed",
        Some("vat readout --mark-filed"),
        Some(&serde_json::json!({ "period": p.label })),
        "ok",
        &[],
    )?;
    Ok(MarkFiledResult { period: p.label, from: p.from, to: p.to, status: "filed".to_string() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::accounts::seed_default_chart;
    use crate::core::db::open_in_memory;

    fn vat_db() -> Connection {
        let conn = open_in_memory().unwrap();
        seed_default_chart(&conn).unwrap();
        conn.execute(
            "INSERT INTO company (name, kvk, legal_form, vat_module, kor_flag)
             VALUES ('Test BV', '00000000', 'bv', 1, 0)",
            [],
        )
        .unwrap();
        let insert = "INSERT OR IGNORE INTO vat_codes (code, rate_bp, type, eu_reverse, description) VALUES (?1, ?2, ?3, ?4, ?5)";
        for c in VAT_CODES {
            conn.execute(insert, params![c.code, c.rate_bp, c.vat_type, c.eu_reverse, c.description]).unwrap();
        }
        create_account_from_seed(&conn, &VAT_ACCOUNTS[0]).unwrap();
        create_account_from_seed(&conn, &VAT_ACCOUNTS[1]).unwrap();
        conn
    }

    #[test]
    fn parse_vat_specs() {
        let specs = parse_vat_posting_specs(&["8000:-100.00@21".to_string(), "1100:121.00".to_string()]).unwrap();
        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].code, "8000");
        assert_eq!(specs[0].amount_cents, -10000);
        assert_eq!(specs[0].vat_code.as_deref(), Some("21"));
        assert_eq!(specs[1].vat_code, None);
        let err = parse_vat_posting_specs(&["8000".to_string()]).unwrap_err();
        assert_eq!(err.code(), "INVALID_POSTING");
    }

    #[test]
    fn expand_output_vat_leg() {
        let conn = vat_db();
        let specs = parse_vat_posting_specs(&["8000:-100.00@21".to_string(), "1100:121.00".to_string()]).unwrap();
        let expanded = expand_vat_postings(&conn, &specs).unwrap();
        assert_eq!(expanded.len(), 3);
        assert_eq!(expanded[0].vat_code.as_deref(), Some("21"));
        assert_eq!(expanded[0].vat_amount_cents, Some(-2100));
        // vat leg on 2500 with the same sign
        assert_eq!(expanded[2].code, "2500");
        assert_eq!(expanded[2].amount_cents, -2100);
    }

    #[test]
    fn expand_input_vat_leg() {
        let conn = vat_db();
        let specs = parse_vat_posting_specs(&["4300:100.00@21".to_string(), "1100:-121.00".to_string()]).unwrap();
        let expanded = expand_vat_postings(&conn, &specs).unwrap();
        assert_eq!(expanded[0].vat_amount_cents, Some(2100));
        // vat leg on 1500 (te vorderen)
        assert_eq!(expanded[2].code, "1500");
        assert_eq!(expanded[2].amount_cents, 2100);
    }

    #[test]
    fn book_posts_vat_entry() {
        let conn = vat_db();
        let specs = parse_vat_posting_specs(&["8000:-100.00@21".to_string(), "1100:121.00".to_string()]).unwrap();
        let (entry, expanded) = book_vat_entry(&conn, "2026-08-01", "Verkoop", &specs, "manual", None, "human", true).unwrap();
        assert_eq!(entry.meta.state, "posted");
        assert_eq!(entry.postings.len(), 3);
        assert_eq!(expanded.len(), 3);
        // the vat posting carries vat_code_id + vat_amount_cents in the DB
        let (vat_id, vat_amt): (Option<i64>, Option<i64>) = conn
            .query_row(
                "SELECT p.vat_code_id, p.vat_amount_cents FROM postings p JOIN accounts a ON a.id = p.account_id WHERE p.entry_id = ?1 AND a.code = '8000'",
                [entry.meta.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert!(vat_id.is_some());
        assert_eq!(vat_amt, Some(-2100));
    }

    #[test]
    fn reverse_charge_and_private_use_standard_rate() {
        let conn = vat_db();
        // verlegde inkoop: 0% code but 21% VAT computed
        let specs = parse_vat_posting_specs(&["4300:100.00@RE".to_string(), "1100:-121.00".to_string()]).unwrap();
        let expanded = expand_vat_postings(&conn, &specs).unwrap();
        assert_eq!(expanded[0].vat_amount_cents, Some(2100));
        // reverse -> vat leg on 2500
        assert_eq!(expanded[2].code, "2500");
        // privégebruik: income? no — private type forces 2500 + standard rate
        let specs = parse_vat_posting_specs(&["8100:-50.00@P".to_string(), "1100:60.50".to_string()]).unwrap();
        let expanded = expand_vat_postings(&conn, &specs).unwrap();
        assert_eq!(expanded[0].vat_amount_cents, Some(-1050));
        assert_eq!(expanded[2].code, "2500");
    }

    #[test]
    fn margin_requires_manual_booking() {
        let conn = vat_db();
        let specs = parse_vat_posting_specs(&["8000:-100.00@M".to_string(), "1100:100.00".to_string()]).unwrap();
        let err = expand_vat_postings(&conn, &specs).unwrap_err();
        assert_eq!(err.code(), "VAT_MARGIN_NOT_SUPPORTED");
    }

    #[test]
    fn readout_fields() {
        let conn = vat_db();
        // omzet 1000 @21 -> 1a +100000, 5a +21000
        let s1 = parse_vat_posting_specs(&["8000:-1000.00@21".to_string(), "1100:1210.00".to_string()]).unwrap();
        book_vat_entry(&conn, "2026-07-01", "Omzet", &s1, "manual", None, "human", true).unwrap();
        // inkoop 300 @21 -> 3a +30000, 5b +6300
        let s2 = parse_vat_posting_specs(&["4300:300.00@21".to_string(), "1100:-363.00".to_string()]).unwrap();
        book_vat_entry(&conn, "2026-07-15", "Inkoop", &s2, "manual", None, "human", true).unwrap();
        let r = ob_readout(&conn, "2026-Q3").unwrap();
        let get = |k: &str| r.fields.iter().find(|(f, _)| f == k).unwrap().1;
        assert_eq!(get("1a"), 100000);
        assert_eq!(get("5a"), 21000);
        assert_eq!(get("3a"), 30000);
        assert_eq!(get("5b"), 6300);
        assert_eq!(r.to_pay_cents, 21000 - 6300);
        assert_eq!(r.to_pay, "147.00");
        // period bounds: Q3 = 2026-07-01 .. 2026-09-30
        assert_eq!(r.from, "2026-07-01");
        assert_eq!(r.to, "2026-09-30");
    }

    #[test]
    fn readout_excludes_unposted_and_1500_2500() {
        let conn = vat_db();
        // draft entry should not appear
        let s = parse_vat_posting_specs(&["8000:-100.00@21".to_string(), "1100:121.00".to_string()]).unwrap();
        book_vat_entry(&conn, "2026-07-01", "Draft", &s, "manual", None, "human", false).unwrap();
        let r = ob_readout(&conn, "2026-Q3").unwrap();
        assert_eq!(r.to_pay_cents, 0);
        assert_eq!(r.fields.iter().find(|(f, _)| f == "1a").unwrap().1, 0);
    }

    #[test]
    fn mark_filed_records_return() {
        let conn = vat_db();
        let s = parse_vat_posting_specs(&["8000:-100.00@21".to_string(), "1100:121.00".to_string()]).unwrap();
        book_vat_entry(&conn, "2026-07-01", "Omzet", &s, "manual", None, "human", true).unwrap();
        let r = mark_filed(&conn, "2026-Q3", "human").unwrap();
        assert_eq!(r.status, "filed");
        let (status, fields_json): (String, String) = conn
            .query_row("SELECT status, fields_json FROM vat_returns WHERE type='OB' AND period='2026-Q3'", [], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap();
        assert_eq!(status, "filed");
        assert!(fields_json.contains("\"5a\""));
    }
}
