// bukio-cli — agent-first double-entry bookkeeping for SMEs.
// Copyright (c) 2026 Erik van Kempen.
// SPDX-License-Identifier: Apache-2.0
//
// VAT module (mirrors src/vat/index.js): codes, VAT-aware booking,
// OB-aangifte readout, filing & settlement.

use crate::accounts::{create_account, get_account_by_code, resolve_profile, NewAccount};
use crate::actor::now_iso;
use crate::audit::{record, RecordArgs};
use crate::dates::today_iso;
use crate::entries::{create_entry, post_entry, CreateEntry, PostingSpec};
use crate::money::{format_amount, parse_amount, BukioError, Result};
use rusqlite::Connection;
use serde_json::{json, Value};

pub fn is_vat_enabled(db: &Connection) -> bool {
    db.query_row(
        "SELECT vat_module, kor_flag FROM company WHERE id = 1",
        [],
        |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
    )
    .map(|(v, k)| v == 1 && k == 0)
    .unwrap_or(false)
}

fn require_vat(db: &Connection) -> Result<()> {
    if !is_vat_enabled(db) {
        return Err(BukioError::new(
            "VAT_MODULE_OFF",
            "the VAT module is not enabled for this company (enable with `bukio vat enable`)",
        ));
    }
    Ok(())
}

/// Enable the VAT module: flag + VAT accounts + VAT codes. Idempotent.
pub fn enable_vat_module(db: &Connection, actor: &str) -> Result<Value> {
    let kor: i64 = db
        .query_row("SELECT kor_flag FROM company WHERE id = 1", [], |r| {
            r.get(0)
        })
        .unwrap_or(0);
    if kor == 1 {
        return Err(BukioError::new("KOR_ACTIVE", "this company uses the KOR (kleineondernemersregeling) — the VAT module cannot be enabled"));
    }
    let profile = resolve_profile(db)?;
    let tax = &profile["tax"];
    let tx = db.unchecked_transaction().map_err(sql_err)?;
    {
        tx.execute("UPDATE company SET vat_module = 1 WHERE id = 1", [])
            .map_err(sql_err)?;
        for a in tax["accounts"]["ledger"]
            .as_array()
            .cloned()
            .unwrap_or_default()
        {
            let code = a["code"].as_str().unwrap_or_default();
            if get_account_by_code(&tx, code).is_none() {
                create_account(
                    &tx,
                    &NewAccount {
                        code,
                        name: a["name"].as_str().unwrap_or_default(),
                        type_: a["type"].as_str().unwrap_or("liability"),
                        normal_balance: a["normalBalance"].as_str().unwrap_or("credit"),
                        taxonomy_code: a["taxonomyCode"].as_str(),
                    },
                )?;
            }
        }
        for c in tax["codes"].as_array().cloned().unwrap_or_default() {
            tx.execute(
                "INSERT OR IGNORE INTO vat_codes (code, rate_bp, type, eu_reverse, description) VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![
                    c["code"].as_str().unwrap_or_default(),
                    c["rateBp"].as_i64().unwrap_or(0),
                    c["type"].as_str().unwrap_or("standard"),
                    (c["euReverse"].as_bool().unwrap_or(false) || c["euReverse"].as_i64().unwrap_or(0) == 1),
                    c["description"].as_str().unwrap_or_default(),
                ],
            )
            .map_err(sql_err)?;
        }
        record(
            &tx,
            RecordArgs {
                actor,
                action: "vat.enable",
                command: Some("vat enable"),
                args: Some(json!({})),
                outcome: "ok",
                entry_ids: vec![],
            },
        )?;
    }
    tx.commit().map_err(sql_err)?;
    let accounts: Vec<String> = tax["accounts"]["ledger"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|x| x["code"].as_str().unwrap_or_default().to_string())
                .collect()
        })
        .unwrap_or_default();
    let codes: Vec<String> = tax["codes"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|x| x["code"].as_str().unwrap_or_default().to_string())
                .collect()
        })
        .unwrap_or_default();
    Ok(json!({ "vat_module": 1, "accounts": accounts, "codes": codes }))
}

pub fn list_vat_codes(db: &Connection) -> Result<Vec<Value>> {
    let mut stmt = db
        .prepare("SELECT id, code, rate_bp, type, eu_reverse, description FROM vat_codes ORDER BY rate_bp DESC, code")
        .map_err(sql_err)?;
    let rows = stmt
        .query_map([], |r| {
            Ok(json!({
                "id": r.get::<_, i64>(0)?,
                "code": r.get::<_, String>(1)?,
                "rate_bp": r.get::<_, i64>(2)?,
                "type": r.get::<_, String>(3)?,
                "eu_reverse": r.get::<_, i64>(4)? == 1,
                "description": r.get::<_, String>(5)?,
            }))
        })
        .map_err(sql_err)?
        .filter_map(|x| x.ok())
        .collect();
    Ok(rows)
}

#[derive(Debug, Clone)]
pub struct VatSpec {
    pub code: String,
    pub amount_cents: i64,
    pub vat_code: Option<String>,
    /// FX passthrough (see PostingSpec) — the JS carries these onto the base
    /// legs so a foreign-currency VAT split keeps the original amounts.
    pub fx_currency: Option<String>,
    pub fx_amount_cents: Option<i64>,
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
            let bad = || {
                BukioError::new(
                    "INVALID_POSTING",
                    format!("posting '{t}' must be CODE:AMOUNT[@VATCODE] (e.g. 8000:-100.00@21)"),
                )
            };
            let Some((code, rest)) = t.split_once(':') else {
                return Err(bad());
            };
            if code.is_empty() || code.len() > 6 || !code.bytes().all(|b| b.is_ascii_digit()) {
                return Err(bad());
            }
            let (amount, vat) = match rest.rsplit_once('@') {
                Some((a, v)) => (a, Some(v)),
                None => (rest, None),
            };
            if let Some(v) = vat {
                if v.is_empty() || !v.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.') {
                    return Err(bad());
                }
            }
            out.push(VatSpec {
                code: code.to_string(),
                amount_cents: parse_amount(amount)?,
                vat_code: vat.map(String::from),
                fx_currency: None,
                fx_amount_cents: None,
            });
        }
    }
    Ok(out)
}

/// Expand VAT-aware specs into core postings (mirrors expandVatPostings).
pub fn expand_vat_postings(
    db: &Connection,
    specs: &[VatSpec],
) -> Result<(Vec<PostingSpec>, Vec<(Option<String>, Option<i64>)>)> {
    require_vat(db)?;
    let profile = resolve_profile(db)?;
    let tax = &profile["tax"];
    let reverse_rate = tax["reverseChargeEffectiveRateBp"].as_i64().unwrap_or(2100);
    let ledger = tax["accounts"]["ledger"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let input_acc = ledger.iter().find(|a| a["type"] == "asset");
    let output_acc = ledger.iter().find(|a| a["type"] == "liability");
    let (input_code, output_code) = match (input_acc, output_acc) {
        (Some(i), Some(o)) => (
            i["code"].as_str().unwrap_or_default().to_string(),
            o["code"].as_str().unwrap_or_default().to_string(),
        ),
        _ => {
            return Err(BukioError::new(
                "FORMAT_NOT_SUPPORTED",
                "the jurisdiction profile's VAT ledger must declare one asset and one liability clearing account",
            ))
        }
    };

    #[derive(Clone)]
    struct Expanded {
        code: String,
        amount_cents: i64,
        vat_code: Option<String>,
        vat_amount_cents: Option<i64>,
        fx_currency: Option<String>,
        fx_amount_cents: Option<i64>,
    }
    let mut expanded: Vec<Expanded> = Vec::new();
    let mut vat_legs: Vec<PostingSpec> = Vec::new();

    for spec in specs {
        let account = get_account_by_code(db, &spec.code).ok_or_else(|| {
            BukioError::new(
                "ACCOUNT_NOT_FOUND",
                format!("account {} does not exist", spec.code),
            )
        })?;
        match &spec.vat_code {
            Some(vc) => {
                let vat: Option<(i64, String, bool)> = db
                    .query_row(
                        "SELECT rate_bp, type, eu_reverse FROM vat_codes WHERE code = ?1",
                        [vc.as_str()],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, i64>(2)? == 1)),
                    )
                    .ok();
                let Some((rate_bp, vtype, eu_reverse)) = vat else {
                    return Err(BukioError::new(
                        "VAT_CODE_NOT_FOUND",
                        format!("vat code '{vc}' does not exist"),
                    ));
                };
                if vtype == "margin" {
                    return Err(BukioError::new(
                        "VAT_MARGIN_NOT_SUPPORTED",
                        "margeregeling cannot be split automatically — book it manually",
                    ));
                }
                let effective_rate = if vtype == "reverse" || vtype == "private" {
                    reverse_rate
                } else {
                    rate_bp
                };
                let num =
                    (spec.amount_cents.abs() as i128 * effective_rate as i128) as f64 / 10000.0;
                let rounded = num.round() as i64;
                let vat_amount = if vtype == "private" {
                    -rounded
                } else {
                    rounded * spec.amount_cents.signum()
                };
                let is_output = account["type"] == "income" || vtype == "private";
                let vat_account = if is_output {
                    output_code.clone()
                } else {
                    input_code.clone()
                };
                expanded.push(Expanded {
                    code: spec.code.clone(),
                    amount_cents: spec.amount_cents,
                    vat_code: Some(vc.clone()),
                    vat_amount_cents: Some(vat_amount),
                    fx_currency: spec.fx_currency.clone(),
                    fx_amount_cents: spec.fx_amount_cents,
                });
                if vat_amount != 0 && vtype != "reverse" {
                    vat_legs.push(PostingSpec {
                        code: vat_account,
                        amount_cents: vat_amount,
                        cost_center_code: None,
                        vat_code: None,
                        vat_amount_cents: None,
                        fx_currency: None,
                        fx_amount_cents: None,
                    });
                }
            }
            None => expanded.push(Expanded {
                code: spec.code.clone(),
                amount_cents: spec.amount_cents,
                vat_code: None,
                vat_amount_cents: None,
                fx_currency: spec.fx_currency.clone(),
                fx_amount_cents: spec.fx_amount_cents,
            }),
        }
    }

    // rounding-drift absorption into the largest untagged user leg
    let mut result: Vec<PostingSpec> = expanded
        .iter()
        .map(|e| PostingSpec {
            code: e.code.clone(),
            amount_cents: e.amount_cents,
            cost_center_code: e.vat_code.clone().and(None),
            vat_code: None,
            vat_amount_cents: None,
            fx_currency: e.fx_currency.clone(),
            fx_amount_cents: e.fx_amount_cents,
        })
        .collect();
    // carry vat info separately — resolved inside create_entry via vat_code? No:
    // bukio stores vat_code_id on the posting; extend PostingSpec use below.
    let vat_info: Vec<(Option<String>, Option<i64>)> = expanded
        .iter()
        .map(|e| (e.vat_code.clone(), e.vat_amount_cents))
        .collect();
    result.extend(vat_legs);
    let sum: i64 = result.iter().map(|p| p.amount_cents).sum();
    if sum != 0 {
        // absorb into the largest untagged leg
        let mut target_idx: Option<usize> = None;
        for (i, e) in expanded.iter().enumerate() {
            if e.vat_code.is_none() {
                match target_idx {
                    Some(t) if expanded[t].amount_cents.abs() >= e.amount_cents.abs() => {}
                    _ => target_idx = Some(i),
                }
            }
        }
        if let Some(i) = target_idx {
            result[i].amount_cents -= sum;
        }
    }
    Ok((result, vat_info))
}

/// Book a VAT-aware entry (mirrors bookVatEntry). VAT metadata is attached to
/// the postings after creation via direct column updates (the JS engine
/// accepts vatCode/vatAmountCents on spec objects; PostingSpec carries only
/// the analytical cost-center dim, so we patch the rows).
pub fn book_vat_entry(
    db: &Connection,
    date: &str,
    description: &str,
    specs: &[VatSpec],
    source: &str,
    source_ref: Option<&str>,
    actor: &str,
    post: bool,
) -> Result<Value> {
    let (expanded_specs, vat_info) = expand_vat_postings(db, specs)?;
    let entry = create_entry(
        db,
        CreateEntry {
            date,
            description,
            postings: expanded_specs,
            source,
            source_ref,
            actor,
        },
    )?;
    // attach vat code + amount to the ORIGINAL (tagged) postings
    for spec in specs {
        if let Some(vc) = &spec.vat_code {
            let vat_row: Option<(i64, i64, String)> = db
                .query_row(
                    "SELECT id, rate_bp, type FROM vat_codes WHERE code = ?1",
                    [vc.as_str()],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .ok();
            if let Some((vat_id, rate_bp, vtype)) = vat_row {
                let profile = resolve_profile(db)?;
                let reverse_rate = profile["tax"]["reverseChargeEffectiveRateBp"]
                    .as_i64()
                    .unwrap_or(2100);
                let effective = if vtype == "reverse" || vtype == "private" {
                    reverse_rate
                } else {
                    rate_bp
                };
                let num = (spec.amount_cents.abs() as i128 * effective as i128) as f64 / 10000.0;
                let rounded = num.round() as i64;
                let vat_amount = if vtype == "private" {
                    -rounded
                } else {
                    rounded * spec.amount_cents.signum()
                };
                // find the posting row: account code + amount match on this entry
                let posting_id: Option<i64> = db
                    .query_row(
                        "SELECT p.id FROM postings p JOIN accounts a ON a.id = p.account_id
                         WHERE p.entry_id = ?1 AND a.code = ?2 AND p.amount_cents = ?3 ORDER BY p.id LIMIT 1",
                        rusqlite::params![entry.id, spec.code, spec.amount_cents],
                        |r| r.get(0),
                    )
                    .ok();
                if let Some(pid) = posting_id {
                    db.execute(
                        "UPDATE postings SET vat_code_id = ?1, vat_amount_cents = ?2 WHERE id = ?3",
                        rusqlite::params![vat_id, vat_amount, pid],
                    )
                    .map_err(sql_err)?;
                }
            }
        }
    }
    let entry = if post {
        post_entry(db, entry.id, actor)?
    } else {
        entry
    };
    // fmtEntry shape: postings carry vat/fx fields from the DB
    let enriched: Vec<Value> = entry
        .postings
        .iter()
        .map(|p| {
            let (vat_code, vat_amount_cents): (Option<String>, Option<i64>) = db
                .query_row(
                    "SELECT vc.code, p.vat_amount_cents FROM postings p
                     LEFT JOIN vat_codes vc ON vc.id = p.vat_code_id WHERE p.id = ?1",
                    [p.id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap_or((None, None));
            json!({
                "account_code": p.account_code, "account_name": p.account_name,
                "amount_cents": p.amount_cents, "amount": format_amount(p.amount_cents),
                "vat_code": vat_code,
                "vat_amount_cents": vat_amount_cents,
                "vat_amount": vat_amount_cents.map(format_amount),
                "fx_currency": p.fx_currency.clone(),
                "fx_amount_cents": p.fx_amount_cents,
                "fx_amount": p.fx_amount_cents.map(format_amount),
            })
        })
        .collect();
    Ok(json!({
        "id": entry.id, "date": entry.date, "description": entry.description,
        "state": entry.state, "postings": enriched,
    }))
}

/// Parse '2026-Q2' or '2026-07' into { from, to }.
pub fn parse_period(period: &str) -> Result<(String, String)> {
    let bad = || {
        BukioError::new(
            "INVALID_PERIOD",
            format!("period '{period}' must be YYYY-Qn or YYYY-MM"),
        )
    };
    let parts: Vec<&str> = period.split('-').collect();
    if parts.len() == 2 {
        let y = parts[0];
        if y.len() == 4 && y.bytes().all(|b| b.is_ascii_digit()) {
            if let Some(qn) = parts[1].strip_prefix('Q') {
                if let Ok(q) = qn.parse::<u32>() {
                    if (1..=4).contains(&q) {
                        let from = format!("{y}-{:02}-01", (q - 1) * 3 + 1);
                        let to_month = q * 3;
                        let to = format!(
                            "{y}-{:02}-{:02}",
                            to_month,
                            dim(y.parse().unwrap(), to_month)
                        );
                        return Ok((from, to));
                    }
                }
                return Err(bad());
            }
            if let Ok(m) = parts[1].parse::<u32>() {
                // the month must be two digits (the JS rejects '2026-1')
                if (1..=12).contains(&m) && parts[1].len() == 2 {
                    let y_num: i32 = y.parse().unwrap();
                    return Ok((
                        format!("{y}-{:02}-01", m),
                        format!("{y}-{:02}-{:02}", m, dim(y_num, m)),
                    ));
                }
                return Err(BukioError::new(
                    "INVALID_PERIOD",
                    format!("period '{period}' must be YYYY-Qn or YYYY-MM (month 01-12)"),
                ));
            }
        }
    }
    Err(bad())
}

fn dim(y: i32, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 {
                29
            } else {
                28
            }
        }
        _ => 30,
    }
}

/// OB-aangifte readout for a period (mirrors buildObReadoutNl).
pub fn ob_readout(db: &Connection, period: &str) -> Result<Value> {
    // The JS checks the module before the country/format dispatch: a company
    // with VAT off must hear VAT_MODULE_OFF, not "no layout for your country".
    require_vat(db)?;
    require_vat(db)?;
    let profile = resolve_profile(db)?;
    if profile["tax"]["returnLayout"].as_str() != Some("ob-1a-5d") {
        return Err(BukioError::new(
            "FORMAT_NOT_SUPPORTED",
            "no VAT-return layout for this jurisdiction yet",
        ));
    }
    let (from, to) = parse_period(period)?;
    let clearing: Vec<String> = profile["tax"]["accounts"]["ledger"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|x| format!("'{}'", x["code"].as_str().unwrap_or_default()))
                .collect()
        })
        .unwrap_or_default();
    let sql = format!(
        "SELECT p.amount_cents, p.vat_amount_cents, vc.code, vc.rate_bp, vc.type, vc.eu_reverse, a.type, a.code
         FROM postings p
         JOIN journal_entries e ON e.id = p.entry_id AND e.state = 'posted'
         JOIN vat_codes vc ON vc.id = p.vat_code_id
         JOIN accounts a ON a.id = p.account_id
         WHERE e.date >= ?1 AND e.date <= ?2
           AND p.vat_code_id IS NOT NULL
           AND a.code NOT IN ({})
         ORDER BY e.id, p.id",
        clearing.join(",")
    );
    let mut stmt = db.prepare(&sql).map_err(sql_err)?;
    let rows: Vec<(i64, Option<i64>, String, i64, String, bool, String, String)> = stmt
        .query_map(rusqlite::params![from, to], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get::<_, i64>(5)? == 1,
                r.get(6)?,
                r.get(7)?,
            ))
        })
        .map_err(sql_err)?
        .filter_map(|x| x.ok())
        .collect();

    let mut f1a = 0i64;
    let mut f1b = 0i64;
    let mut f1c = 0i64;
    let mut f1d = 0i64;
    let mut f2a = 0i64;
    let mut f3a = 0i64;
    let mut f3b = 0i64;
    let mut f3c = 0i64;
    let mut f4a = 0i64;
    let mut f4b = 0i64;
    let mut f5a = 0i64;
    let mut f5b = 0i64;
    for (amount, vat_amount, _code, rate_bp, vtype, eu_reverse, account_type, _acc_code) in rows {
        let vat_amount = vat_amount.unwrap_or(0);
        match vtype.as_str() {
            "standard" | "exempt" => {
                if account_type == "income" {
                    let base = -amount;
                    match rate_bp {
                        2100 => f1a += base,
                        900 => f1b += base,
                        _ => f1c += base,
                    }
                    if vtype == "standard" {
                        f5a += -vat_amount;
                    }
                } else {
                    let base = amount;
                    match rate_bp {
                        2100 => f3a += base,
                        900 => f3b += base,
                        _ => f3c += base,
                    }
                    if vtype == "standard" {
                        f5b += vat_amount;
                    }
                }
            }
            "reverse" => {
                if account_type == "income" && eu_reverse {
                    f2a += -amount;
                } else if account_type == "income" {
                    f1c += -amount;
                } else if account_type == "expense" {
                    if eu_reverse {
                        f3b += amount;
                        f4b += vat_amount;
                    } else {
                        f3a += amount;
                        f4a += vat_amount;
                    }
                    f5b += vat_amount;
                }
            }
            "private" => {
                f1d += amount.abs();
                f5a += -vat_amount;
            }
            _ => {}
        }
    }
    let f5d = f5a + f4a + f4b - f5b; // 5c = 0
    Ok(json!({
        "period": period,
        "from": from,
        "to": to,
        "fields": {
            "1a": f1a, "1b": f1b, "1c": f1c, "1d": f1d, "2a": f2a, "2b": 0,
            "3a": f3a, "3b": f3b, "3c": f3c, "4a": f4a, "4b": f4b,
            "5a": f5a, "5b": f5b, "5c": 0, "5d": f5d,
        },
        "to_pay_cents": f5d,
        "to_pay": format_amount(f5d),
        "note": "Manual filing aid only — bukio never submits. Fields 2b (non-EU exports) and 5c are not tracked and shown as 0.",
    }))
}

/// Record that a period was filed manually.
pub fn mark_filed(db: &Connection, period: &str, actor: &str) -> Result<Value> {
    require_vat(db)?;
    let (from, to) = parse_period(period)?;
    let readout = ob_readout(db, period)?;
    let fields = serde_json::to_string(&readout["fields"]).unwrap();
    db.execute(
        "INSERT INTO vat_returns (type, period, status, fields_json, filed_at)
         VALUES ('OB', ?1, 'filed', ?2, ?3)
         ON CONFLICT(type, period) DO UPDATE SET status = 'filed', fields_json = excluded.fields_json, filed_at = excluded.filed_at",
        rusqlite::params![period, fields, now_iso()],
    )
    .map_err(sql_err)?;
    record(
        db,
        RecordArgs {
            actor,
            action: "vat.filed",
            command: Some("vat readout --mark-filed"),
            args: Some(json!({ "period": period })),
            outcome: "ok",
            entry_ids: vec![],
        },
    )?;
    Ok(json!({ "period": period, "from": from, "to": to, "status": "filed" }))
}

/// Reclassify the outstanding VAT position to the af-te-dragen account at
/// filing (mirrors vatFile). VAT leg amounts use exact booked cents.
pub fn vat_file(
    db: &Connection,
    account: Option<&str>,
    period: Option<&str>,
    desc: Option<&str>,
    actor: &str,
    dry_run: bool,
    locale: &str,
) -> Result<Value> {
    require_vat(db)?;
    let profile = resolve_profile(db)?;
    let tax = &profile["tax"];
    let mut account = account.map(String::from).unwrap_or_else(|| {
        tax["accounts"]["fileDefault"]
            .as_str()
            .unwrap_or("2510")
            .to_string()
    });
    let ledger = tax["accounts"]["ledger"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let input_acc = ledger.iter().find(|a| a["type"] == "asset");
    let output_acc = ledger.iter().find(|a| a["type"] == "liability");
    let (input_code, output_code) = match (input_acc, output_acc) {
        (Some(i), Some(o)) => (
            i["code"].as_str().unwrap_or_default().to_string(),
            o["code"].as_str().unwrap_or_default().to_string(),
        ),
        _ => return Err(BukioError::new("FORMAT_NOT_SUPPORTED", "the jurisdiction profile's VAT ledger must declare one asset and one liability clearing account")),
    };
    let bal_input = account_balance(db, &input_code)?;
    let bal_output = account_balance(db, &output_code)?;
    let net = -(bal_output + bal_input);
    if net == 0 {
        return Err(BukioError::new(
            "VAT_NOTHING_TO_FILE",
            format!("no outstanding VAT position to reclassify ({output_code}/{input_code} net is zero)"),
        ));
    }
    account = resolve_vat_settlement_account(db, &account)?;

    let mut postings = vec![
        PostingSpec {
            code: output_code.clone(),
            amount_cents: -bal_output,
            cost_center_code: None,
            vat_code: None,
            vat_amount_cents: None,
            fx_currency: None,
            fx_amount_cents: None,
        },
        PostingSpec {
            code: input_code.clone(),
            amount_cents: -bal_input,
            cost_center_code: None,
            vat_code: None,
            vat_amount_cents: None,
            fx_currency: None,
            fx_amount_cents: None,
        },
        PostingSpec {
            code: account.clone(),
            amount_cents: bal_output + bal_input,
            cost_center_code: None,
            vat_code: None,
            vat_amount_cents: None,
            fx_currency: None,
            fx_amount_cents: None,
        },
    ];
    postings.retain(|p| p.amount_cents != 0);
    let owe = net > 0;
    let liability = net.abs();
    // the JS builds this from the locale tables (vat.file.description), with the
    // direction itself translated first — the port hardcoded different English
    let description = desc.map(String::from).unwrap_or_else(|| {
        let direction = if owe {
            crate::i18n::t("dir.payable", &[], locale)
        } else {
            crate::i18n::t("dir.receivable", &[], locale)
        };
        let period_part = period.map(|p| format!(" {p}")).unwrap_or_default();
        crate::i18n::t(
            "vat.file.description",
            &[
                ("period", &period_part),
                ("account", &account),
                ("direction", &direction),
            ],
            locale,
        )
    });

    if dry_run {
        return Ok(json!({
            "action": "vat.file", "dryRun": true, "account": account, "owe": owe,
            "liability_cents": liability,
            "postings": postings.iter().map(|p| json!({ "code": p.code, "amount_cents": p.amount_cents })).collect::<Vec<_>>(),
        }));
    }

    ensure_vat_settlement_account(db, &account)?;
    let postings_json: Vec<Value> = postings
        .iter()
        .map(|p| json!({ "code": p.code, "amountCents": p.amount_cents }))
        .collect();
    let created = create_entry(
        db,
        CreateEntry {
            date: &today_iso(),
            description: &description,
            postings,
            source: "manual",
            source_ref: None,
            actor,
        },
    )?;
    post_entry(db, created.id, actor)?;
    record(
        db,
        RecordArgs {
            actor,
            action: "vat.file",
            command: Some("vat file"),
            args: Some(
                json!({ "account": account, "period": period, "owe": owe, "liability_cents": liability, "description": description }),
            ),
            outcome: "ok",
            entry_ids: vec![created.id],
        },
    )?;
    Ok(json!({
        "action": "vat.file", "entry_id": created.id, "account": account, "owe": owe,
        "liability_cents": liability,
        "postings": postings_json,
    }))
}

fn settlement_account_name(db: &Connection) -> String {
    resolve_profile(db).unwrap_or_else(|_| crate::accounts::get_profile("NL").unwrap())["tax"]
        ["accounts"]["settlementAccountName"]
        .as_str()
        .unwrap_or("Af te dragen omzetbelasting")
        .to_string()
}

fn is_vat_settlement_account(db: &Connection, code: &str) -> bool {
    let Some(a) = get_account_by_code(db, code) else {
        return false;
    };
    let profile = resolve_profile(db).unwrap();
    let file_default = profile["tax"]["accounts"]["fileDefault"]
        .as_str()
        .unwrap_or("");
    let seeded = profile["reporting"]["defaultChart"]
        .as_array()
        .and_then(|chart| {
            chart
                .iter()
                .find(|c| c["code"].as_str() == Some(file_default))
        });
    if let Some(seeded_row) = seeded {
        if code == file_default
            && a["name"].as_str() == seeded_row["name"].as_str()
            && a["type"] == "liability"
            && a["normal_balance"] == "credit"
        {
            return true;
        }
    }
    a["name"].as_str() == Some(&settlement_account_name(db))
        && a["type"] == "liability"
        && a["normal_balance"] == "credit"
}

fn resolve_vat_settlement_account(db: &Connection, account: &str) -> Result<String> {
    if get_account_by_code(db, account).is_none() {
        return Ok(account.to_string());
    }
    if is_vat_settlement_account(db, account) {
        return Ok(account.to_string());
    }
    if !account.bytes().all(|b| b.is_ascii_digit()) {
        return Err(BukioError::new(
            "VAT_ACCOUNT_COLLISION",
            format!("account {account} exists but is not '{}' and has no numeric successor — pick a free code with --account", settlement_account_name(db)),
        ));
    }
    let mut code: i64 = account.parse().unwrap_or(0);
    let mut guard = 0;
    loop {
        code += 1;
        let code_s = code.to_string();
        if get_account_by_code(db, &code_s).is_none() {
            return Ok(code_s);
        }
        if is_vat_settlement_account(db, &code_s) {
            return Ok(code_s);
        }
        guard += 1;
        if guard > 999 {
            return Err(BukioError::new(
                "VAT_ACCOUNT_COLLISION",
                format!(
                    "no free numeric successor after {account} — pick a free code with --account"
                ),
            ));
        }
    }
}

fn ensure_vat_settlement_account(db: &Connection, account: &str) -> Result<()> {
    let resolved = resolve_vat_settlement_account(db, account)?;
    if get_account_by_code(db, &resolved).is_none() {
        let profile = resolve_profile(db)?;
        let uses_taxonomy = profile["reporting"]["defaultChart"]
            .as_array()
            .map(|c| c.iter().any(|a| a["taxonomyCode"].is_string()))
            .unwrap_or(false);
        create_account(
            db,
            &NewAccount {
                code: &resolved,
                name: &settlement_account_name(db),
                type_: "liability",
                normal_balance: "credit",
                taxonomy_code: if uses_taxonomy { Some("BSCH.12") } else { None },
            },
        )?;
    }
    Ok(())
}

/// Book the bank payment that cancels the af-te-dragen balance
/// (mirrors vatSettle). Rounding difference to the P&L.
pub fn vat_settle(
    db: &Connection,
    tx_amount_cents: i64,
    tx_date: Option<&str>,
    bank_account_code: &str,
    account: Option<&str>,
    difference_account: Option<&str>,
    period: Option<&str>,
    desc: Option<&str>,
    actor: &str,
    dry_run: bool,
    locale: &str,
) -> Result<Value> {
    require_vat(db)?;
    let tax = &resolve_profile(db)?["tax"];
    let account = account.map(String::from).unwrap_or_else(|| {
        tax["accounts"]["fileDefault"]
            .as_str()
            .unwrap_or("2510")
            .to_string()
    });
    let difference_account = difference_account.map(String::from).unwrap_or_else(|| {
        tax["accounts"]["differenceDefault"]
            .as_str()
            .unwrap_or("4700")
            .to_string()
    });
    let balance = account_balance(db, &account)?;
    if balance == 0 {
        return Err(BukioError::new(
            "VAT_SETTLE_NOTHING",
            format!(
                "no outstanding balance on {account} ({}) to settle",
                settlement_account_name(db)
            ),
        ));
    }
    let owe = balance < 0;
    let liability = balance.abs();
    let paid = tx_amount_cents.abs();
    if owe && tx_amount_cents >= 0 {
        return Err(BukioError::new("VAT_SETTLE_DIRECTION", format!("paying {account} (payable) requires an OUTGOING bank transaction, got +{paid} cents")));
    }
    if !owe && tx_amount_cents <= 0 {
        return Err(BukioError::new("VAT_SETTLE_DIRECTION", format!("receiving a refund on {account} (receivable) requires an INCOMING bank transaction, got {tx_amount_cents} cents")));
    }
    if get_account_by_code(db, &difference_account).is_none() {
        return Err(BukioError::new(
            "INVALID_DIFFERENCE_ACCOUNT",
            format!("difference account {difference_account} does not exist (pick an expense account, e.g. {})", tax["accounts"]["differenceDefault"].as_str().unwrap_or("4700")),
        ));
    }
    let difference: i64 = if owe { 1 } else { -1 } * (paid - liability);
    if difference.abs() > 500 {
        return Err(BukioError::new(
            "VAT_SETTLE_DIFFERENCE_TOO_LARGE",
            format!("settlement difference is {} cents vs liability {liability} — a VAT filing rounds per line (< \u{20ac}0.50/line), so this looks like the wrong amount; max allowed is 500 cents", difference.abs()),
        ));
    }
    let mut postings = vec![
        PostingSpec {
            code: account.clone(),
            amount_cents: if owe { liability } else { -liability },
            cost_center_code: None,
            vat_code: None,
            vat_amount_cents: None,
            fx_currency: None,
            fx_amount_cents: None,
        },
        PostingSpec {
            code: bank_account_code.to_string(),
            amount_cents: tx_amount_cents,
            cost_center_code: None,
            vat_code: None,
            vat_amount_cents: None,
            fx_currency: None,
            fx_amount_cents: None,
        },
    ];
    if difference != 0 {
        postings.push(PostingSpec {
            code: difference_account.clone(),
            amount_cents: difference,
            cost_center_code: None,
            vat_code: None,
            vat_amount_cents: None,
            fx_currency: None,
            fx_amount_cents: None,
        });
    }
    let description = desc.map(String::from).unwrap_or_else(|| {
        let period_part = period.map(|p| format!(" {p}")).unwrap_or_default();
        crate::i18n::t(
            "vat.settle.description",
            &[
                ("period", &period_part),
                ("account", &settlement_account_name(db)),
                ("amount", &format_amount(difference)),
            ],
            locale,
        )
    });
    let date = tx_date.map(String::from).unwrap_or_else(today_iso);
    if dry_run {
        return Ok(json!({
            "action": "vat.settle", "dryRun": true, "account": account, "owe": owe,
            "liability_cents": liability, "paid_cents": paid, "difference_cents": difference,
            "difference_account": difference_account, "date": date,
            "postings": postings.iter().map(|p| json!({ "code": p.code, "amount_cents": p.amount_cents })).collect::<Vec<_>>(),
        }));
    }
    let postings_json: Vec<Value> = postings
        .iter()
        .map(|p| json!({ "code": p.code, "amountCents": p.amount_cents }))
        .collect();
    let created = create_entry(
        db,
        CreateEntry {
            date: &date,
            description: &description,
            postings,
            source: "manual",
            source_ref: None,
            actor,
        },
    )?;
    post_entry(db, created.id, actor)?;
    record(
        db,
        RecordArgs {
            actor,
            action: "vat.settle",
            command: Some("vat settle"),
            args: Some(
                json!({ "account": account, "period": period, "owe": owe, "liability_cents": liability, "paid_cents": paid, "difference_cents": difference, "difference_account": difference_account, "description": description }),
            ),
            outcome: "ok",
            entry_ids: vec![created.id],
        },
    )?;
    Ok(json!({
        "action": "vat.settle", "entry_id": created.id, "account": account, "owe": owe,
        "liability_cents": liability, "paid_cents": paid, "difference_cents": difference,
        "difference_account": difference_account,
        "postings": postings_json,
    }))
}

fn account_balance(db: &Connection, code: &str) -> Result<i64> {
    db.query_row(
        "SELECT COALESCE(SUM(p.amount_cents), 0) FROM postings p
         JOIN journal_entries e ON e.id = p.entry_id AND e.state = 'posted'
         JOIN accounts a ON a.id = p.account_id
         WHERE a.code = ?1",
        [code],
        |r| r.get(0),
    )
    .map_err(sql_err)
}

fn sql_err(e: rusqlite::Error) -> BukioError {
    BukioError::new("DB_ERROR", e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_db;

    fn vat_company() -> Connection {
        let db = open_db(":memory:").unwrap();
        db.execute(
            "INSERT INTO company (name, vat_module) VALUES ('VAT BV', 1)",
            [],
        )
        .unwrap();
        crate::accounts::seed_default_chart(&db).unwrap();
        enable_vat_module(&db, "human:erik").unwrap();
        db
    }

    // ── ported from test/vat.test.js ──
    // NOTE: the Rust engine returns the entry directly from book_vat_entry (the
    // JS engine wraps it as {entry}); the CLI wrappers match, which is what
    // parity checks. JS's vatNetPosition has no public Rust equivalent — the
    // position is computed inside vat_file/vat_settle — so that one assertion
    // is not reproducible here.

    fn specs(raw: &[&str]) -> Vec<VatSpec> {
        let owned: Vec<String> = raw.iter().map(|s| s.to_string()).collect();
        parse_vat_posting_specs(&owned).unwrap()
    }

    fn fresh_vat_company(kor: bool) -> Connection {
        let db = open_db(":memory:").unwrap();
        db.execute(
            "INSERT INTO company (name, legal_form, kor_flag, vat_module) VALUES ('Test BV', 'bv', ?1, 0)",
            rusqlite::params![if kor { 1 } else { 0 }],
        )
        .unwrap();
        crate::accounts::seed_default_chart(&db).unwrap();
        db
    }

    fn seed_q2_scenario(db: &Connection) {
        // sale 121.00 incl 21%; purchase 60.50 incl 21%; sale 109.00 incl 9%
        for (date, desc, raw) in [
            (
                "2026-04-10",
                "Factuur 2026-001",
                "1100:121.00,8000:-100.00@21",
            ),
            (
                "2026-05-15",
                "Kantoorartikelen",
                "4300:50.00@21,1100:-60.50",
            ),
            (
                "2026-06-01",
                "Factuur 2026-002",
                "1100:109.00,8000:-100.00@9",
            ),
        ] {
            book_vat_entry(
                db,
                date,
                desc,
                &specs(&[raw]),
                "manual",
                None,
                "human:erik",
                true,
            )
            .unwrap();
        }
    }

    #[test]
    fn enable_seeds_accounts_and_codes_then_is_idempotent() {
        let db = fresh_vat_company(false);
        enable_vat_module(&db, "human:erik").unwrap();
        assert!(is_vat_enabled(&db));
        let name = |code: &str| {
            crate::accounts::get_account_by_code(&db, code).unwrap()["name"]
                .as_str()
                .unwrap()
                .to_string()
        };
        assert_eq!(name("1500"), "Te vorderen omzetbelasting");
        assert_eq!(name("2500"), "Te betalen omzetbelasting");
        let codes = list_vat_codes(&db).unwrap();
        assert_eq!(codes.len(), 8);
        assert!(codes
            .iter()
            .any(|c| c["code"] == "21" && c["rate_bp"] == 2100 && c["type"] == "standard"));
        assert!(codes
            .iter()
            .any(|c| c["code"] == "RE" && c["eu_reverse"] == true));
        // idempotent
        enable_vat_module(&db, "human:erik").unwrap();
        assert_eq!(list_vat_codes(&db).unwrap().len(), 8);
    }

    #[test]
    fn enable_refuses_on_a_kor_company() {
        let db = fresh_vat_company(true);
        assert_eq!(
            enable_vat_module(&db, "human:erik").unwrap_err().code,
            "KOR_ACTIVE"
        );
    }

    #[test]
    fn parse_specs_handles_the_vat_suffix() {
        let parsed = specs(&["1100:121.00,8000:-100.00@21"]);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].code, "1100");
        assert_eq!(parsed[0].amount_cents, 12100);
        assert_eq!(parsed[0].vat_code, None);
        assert_eq!(parsed[1].code, "8000");
        assert_eq!(parsed[1].amount_cents, -10000);
        assert_eq!(parsed[1].vat_code.as_deref(), Some("21"));

        let owned = vec!["nonsense".to_string()];
        assert_eq!(
            parse_vat_posting_specs(&owned).unwrap_err().code,
            "INVALID_POSTING"
        );
    }

    #[test]
    fn expand_adds_the_output_vat_leg() {
        let db = vat_company();
        let (p, vat_info) =
            expand_vat_postings(&db, &specs(&["1100:121.00,8000:-100.00@21"])).unwrap();
        assert_eq!(p.len(), 3);
        assert_eq!(p[0].code, "1100");
        assert_eq!(p[0].amount_cents, 12100);
        assert_eq!(p[0].vat_amount_cents, None);
        assert_eq!(p[1].code, "8000");
        assert_eq!(p[1].amount_cents, -10000);
        assert_eq!(p[2].code, "2500"); // te betalen btw
        assert_eq!(p[2].amount_cents, -2100);
        assert_eq!(p.iter().map(|x| x.amount_cents).sum::<i64>(), 0);
        // the Rust API carries the per-input VAT info in a SECOND vector
        // (JS attaches vatCode/vatAmountCents inline on the posting); the CLI
        // shapes agree.
        assert_eq!(vat_info.len(), 2);
        assert_eq!(vat_info[0], (None, None));
        assert_eq!(vat_info[1], (Some("21".to_string()), Some(-2100)));
    }

    #[test]
    fn expand_routes_the_input_side_to_1500() {
        let db = vat_company();
        let (p, _) = expand_vat_postings(&db, &specs(&["4340:100.00@21,1100:-121.00"])).unwrap();
        let vat_leg = p.iter().find(|x| x.code == "1500").expect("1500 leg");
        assert_eq!(vat_leg.amount_cents, 2100);
        assert_eq!(p.iter().map(|x| x.amount_cents).sum::<i64>(), 0);
    }

    #[test]
    fn book_posts_a_three_leg_entry_with_vat_fields_persisted() {
        let db = vat_company();
        let entry = book_vat_entry(
            &db,
            "2026-06-01",
            "Factuur 2026-001",
            &specs(&["1100:121.00,8000:-100.00@21"]),
            "manual",
            None,
            "human:erik",
            true,
        )
        .unwrap();
        assert_eq!(entry["state"], "posted");
        let postings = entry["postings"].as_array().unwrap();
        assert_eq!(postings.len(), 3);
        let omzet = postings
            .iter()
            .find(|p| p["account_code"] == "8000")
            .expect("8000 posting");
        assert_eq!(omzet["vat_amount_cents"], -2100);
        assert!(omzet["vat_code"].is_string());
    }

    #[test]
    fn book_guards_module_off_and_unknown_code() {
        let off = fresh_vat_company(false);
        assert_eq!(
            book_vat_entry(
                &off,
                "2026-06-01",
                "x",
                &specs(&["1100:121.00,8000:-100.00@21"]),
                "manual",
                None,
                "human:erik",
                false
            )
            .unwrap_err()
            .code,
            "VAT_MODULE_OFF"
        );

        let db = vat_company();
        assert_eq!(
            book_vat_entry(
                &db,
                "2026-06-01",
                "x",
                &specs(&["1100:121.00,8000:-100.00@25"]),
                "manual",
                None,
                "human:erik",
                false
            )
            .unwrap_err()
            .code,
            "VAT_CODE_NOT_FOUND"
        );
    }

    #[test]
    fn parse_period_quarters_months_and_rejects() {
        assert_eq!(
            parse_period("2026-Q2").unwrap(),
            ("2026-04-01".to_string(), "2026-06-30".to_string())
        );
        assert_eq!(
            parse_period("2026-Q4").unwrap(),
            ("2026-10-01".to_string(), "2026-12-31".to_string())
        );
        assert_eq!(
            parse_period("2026-07").unwrap(),
            ("2026-07-01".to_string(), "2026-07-31".to_string())
        );
        assert_eq!(parse_period("2026-Q5").unwrap_err().code, "INVALID_PERIOD");
        assert_eq!(parse_period("2026").unwrap_err().code, "INVALID_PERIOD");
    }

    #[test]
    fn ob_readout_full_scenario_fields() {
        let db = vat_company();
        seed_q2_scenario(&db);
        let r = ob_readout(&db, "2026-Q2").unwrap();
        let f = &r["fields"];
        assert_eq!(f["1a"], 10000); // omzet 21%
        assert_eq!(f["1b"], 10000); // omzet 9%
        assert_eq!(f["1c"], 0);
        assert_eq!(f["3a"], 5000); // inkopen 21%
        assert_eq!(f["5a"], 3000); // 2100 + 900
        assert_eq!(f["5b"], 1050); // voorbelasting
        assert_eq!(f["5d"], 1950); // 3000 - 1050
        assert_eq!(r["to_pay"], "19.50");
    }

    #[test]
    fn ob_readout_is_period_isolated_and_ignores_drafts() {
        let db = vat_company();
        seed_q2_scenario(&db);
        assert_eq!(ob_readout(&db, "2026-Q1").unwrap()["fields"]["5d"], 0);

        // a balanced DRAFT carrying VAT fields must not leak into the readout
        crate::entries::create_entry(
            &db,
            crate::entries::CreateEntry {
                date: "2026-04-20",
                description: "draft sale",
                postings: vec![
                    crate::entries::PostingSpec {
                        code: "1100".into(),
                        amount_cents: 12100,
                        cost_center_code: None,
                        vat_code: None,
                        vat_amount_cents: None,
                        fx_currency: None,
                        fx_amount_cents: None,
                    },
                    crate::entries::PostingSpec {
                        code: "8000".into(),
                        amount_cents: -10000,
                        cost_center_code: None,
                        vat_code: Some("21".into()),
                        vat_amount_cents: Some(-2100),
                        fx_currency: None,
                        fx_amount_cents: None,
                    },
                    crate::entries::PostingSpec {
                        code: "2500".into(),
                        amount_cents: -2100,
                        cost_center_code: None,
                        vat_code: Some("21".into()),
                        vat_amount_cents: Some(-2100),
                        fx_currency: None,
                        fx_amount_cents: None,
                    },
                ],
                source: "manual",
                source_ref: None,
                actor: "human:erik",
            },
        )
        .unwrap();

        assert_eq!(ob_readout(&db, "2026-Q2").unwrap()["fields"]["5a"], 3000);
    }

    #[test]
    fn ob_readout_reverse_charge_nets_out() {
        let db = vat_company();
        // reverse charge books NO auto VAT leg; the tagged posting feeds the readout
        let entry = book_vat_entry(
            &db,
            "2026-06-01",
            "Inkoop verlegd",
            &specs(&["4300:100.00@R,1100:-100.00"]),
            "manual",
            None,
            "human:erik",
            true,
        )
        .unwrap();
        let mut legs: Vec<String> = entry["postings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| {
                format!(
                    "{}:{}",
                    p["account_code"].as_str().unwrap(),
                    p["amount_cents"]
                )
            })
            .collect();
        legs.sort();
        assert_eq!(legs, vec!["1100:-10000", "4300:10000"]);

        let r = ob_readout(&db, "2026-Q2").unwrap();
        assert_eq!(r["fields"]["3a"], 10000); // binnenlandse verlegde inkoop
        assert_eq!(r["fields"]["4a"], 2100); // 21% derived from the tagged posting
        assert_eq!(r["fields"]["5b"], 2100); // claimed back — nets out
        assert_eq!(r["fields"]["5d"], 0);
    }

    #[test]
    fn ob_readout_guards_module_off_and_bad_period() {
        let off = fresh_vat_company(false);
        assert_eq!(
            ob_readout(&off, "2026-Q2").unwrap_err().code,
            "VAT_MODULE_OFF"
        );
        let db = vat_company();
        assert_eq!(ob_readout(&db, "2026").unwrap_err().code, "INVALID_PERIOD");
    }

    #[test]
    fn mark_filed_records_the_filing_and_is_an_idempotent_upsert() {
        let db = vat_company();
        seed_q2_scenario(&db);
        let result = mark_filed(&db, "2026-Q2", "agent:test").unwrap();
        assert_eq!(result["status"], "filed");

        let (status, fields_json): (String, Option<String>) = db
            .query_row(
                "SELECT status, fields_json FROM vat_returns WHERE type='OB' AND period='2026-Q2'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "filed");
        let fields: Value = serde_json::from_str(&fields_json.unwrap()).unwrap();
        assert_eq!(fields["5d"], 1950);

        mark_filed(&db, "2026-Q2", "agent:test").unwrap();
        let count: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM vat_returns WHERE period='2026-Q2'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    // ── ported from test/vat-settle.test.js ──

    const SETTLE_IBAN: &str = "NL91ABNA0417164300";

    fn settle_company() -> Connection {
        let db = open_db(":memory:").unwrap();
        crate::accounts::seed_default_chart(&db).unwrap();
        db.execute(
            "INSERT INTO company (name, registration_id, legal_form, tax_id, iban, vat_module)
             VALUES ('Demo BV', '12345678', 'bv', 'NL123456789B01', ?1, 1)",
            rusqlite::params![SETTLE_IBAN],
        )
        .unwrap();
        enable_vat_module(&db, "human:erik").unwrap();
        db
    }

    fn camt_payment(amount: &str, direction: &str) -> String {
        format!(
            r#"<?xml version="1.0"?>
<Document xmlns="urn:iso:std:iso:20022:tech:xsd:camt.053.001.02">
  <BkToCstmrStmt><Stmt><Acct><Id><IBAN>{iban}</IBAN></Id></Acct>
    <Ntry><Amt>{amount}</Amt><CdtDbtInd>{direction}</CdtDbtInd><BookgDt><Dt>2026-07-25</Dt></BookgDt>
      <NtryDtls><TxDtls><RltdPties><Dbtr><Nm>Belastingdienst</Nm></Dbtr></RltdPties>
      <RmtInf><Ustrd>OB aangifte</Ustrd></RmtInf></TxDtls></NtryDtls></Ntry>
  </Stmt></BkToCstmrStmt>
</Document>"#,
            iban = SETTLE_IBAN,
            amount = amount,
            direction = direction
        )
    }

    /// Import one OB payment; returns (amount_cents, date, account_code, state).
    fn import_payment(
        db: &Connection,
        amount: &str,
        direction: &str,
    ) -> (i64, String, String, String) {
        let txs = crate::bank::parse_camt053(&camt_payment(amount, direction)).unwrap();
        crate::bank::import_transactions(db, SETTLE_IBAN, &txs, None, "1100", "human:erik")
            .unwrap();
        last_tx(db)
    }

    fn last_tx(db: &Connection) -> (i64, String, String, String) {
        db.query_row(
            "SELECT bt.amount_cents, bt.date, ba.account_code, bt.state
             FROM bank_transactions bt JOIN bank_accounts ba ON ba.id = bt.bank_account_id
             ORDER BY bt.id DESC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap()
    }

    fn account_bal(db: &Connection, code: &str) -> i64 {
        db.query_row(
            "SELECT COALESCE(SUM(p.amount_cents), 0) FROM postings p
             JOIN journal_entries e ON e.id = p.entry_id AND e.state = 'posted'
             JOIN accounts a ON a.id = p.account_id WHERE a.code = ?1",
            [code],
            |r| r.get(0),
        )
        .unwrap()
    }

    /// JS vatNetPosition = -(sum of the profile's ledger account balances).
    fn vat_net_position(db: &Connection) -> i64 {
        -(account_bal(db, "1500") + account_bal(db, "2500"))
    }

    fn book_quarter(db: &Connection) {
        // 121.00 sale (21 VAT) + 60.50 purchase (10.50 input) -> owe 10.50
        book_vat_entry(
            db,
            "2026-07-01",
            "Omzet Q3",
            &specs(&["1100:121.00,8000:-100.00@21"]),
            "manual",
            None,
            "human:erik",
            true,
        )
        .unwrap();
        book_vat_entry(
            db,
            "2026-07-05",
            "Inkoop Q3",
            &specs(&["1100:-60.50,4300:50.00@21"]),
            "manual",
            None,
            "human:erik",
            true,
        )
        .unwrap();
    }

    fn add_account(db: &Connection, code: &str, name: &str) {
        crate::accounts::create_account(
            db,
            &crate::accounts::NewAccount {
                code,
                name,
                type_: "liability",
                normal_balance: "credit",
                taxonomy_code: None,
            },
        )
        .unwrap();
    }

    fn audit_args(db: &Connection, action: &str) -> Option<Value> {
        db.query_row(
            "SELECT args_json FROM audit_log WHERE action = ?1",
            [action],
            |r| r.get::<_, Option<String>>(0),
        )
        .unwrap()
        .map(|j| serde_json::from_str(&j).unwrap())
    }

    #[test]
    fn file_owe_clears_2500_and_books_the_liability_to_2510() {
        let db = settle_company();
        book_quarter(&db);
        assert_eq!(vat_net_position(&db), 1050); // 2100 output - 1050 input
        let r = vat_file(&db, None, Some("2026-Q3"), None, "agent:test", false, "en").unwrap();
        assert_eq!(r["owe"], true);
        assert_eq!(r["liability_cents"], 1050);
        assert_eq!(account_bal(&db, "2500"), 0); // clearing account emptied
        assert_eq!(account_bal(&db, "2510"), -1050); // af te dragen (credit)
        let n: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM audit_log WHERE action = 'vat.file'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1);
        let args = audit_args(&db, "vat.file").unwrap();
        assert_eq!(args["period"], "2026-Q3");
        assert_eq!(args["liability_cents"], 1050);
    }

    #[test]
    fn file_refund_position_clears_1500_and_debits_2510() {
        let db = settle_company();
        // only input VAT: 121.00 purchase -> 21.00 voorbelasting, no sales
        book_vat_entry(
            &db,
            "2026-07-01",
            "Inkoop",
            &specs(&["1100:-121.00,4300:100.00@21"]),
            "manual",
            None,
            "human:erik",
            true,
        )
        .unwrap();
        assert_eq!(vat_net_position(&db), -2100);
        let r = vat_file(&db, None, Some("2026-Q3"), None, "agent:test", false, "en").unwrap();
        assert_eq!(r["owe"], false);
        assert_eq!(r["liability_cents"], 2100);
        assert_eq!(account_bal(&db, "1500"), 0);
        assert_eq!(account_bal(&db, "2510"), 2100); // debit = terug te ontvangen
    }

    #[test]
    fn file_nothing_to_file_when_the_position_is_zero() {
        let db = settle_company();
        assert_eq!(
            vat_file(&db, None, None, None, "agent:test", false, "en")
                .unwrap_err()
                .code,
            "VAT_NOTHING_TO_FILE"
        );
    }

    #[test]
    fn file_dry_run_writes_nothing_and_does_not_create_the_account() {
        let db = settle_company();
        book_quarter(&db);
        let r = vat_file(&db, None, Some("2026-Q3"), None, "agent:test", true, "en").unwrap();
        assert_eq!(r["dryRun"], true);
        assert_eq!(r["liability_cents"], 1050);
        assert_eq!(account_bal(&db, "2500"), -2100); // untouched
        assert_eq!(account_bal(&db, "2510"), 0);
        let created: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM accounts WHERE code = '2510'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(created, 0);
        let audits: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM audit_log WHERE action = 'vat.file'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(audits, 0);
    }

    #[test]
    fn settle_rounding_in_your_favour_books_a_gain_to_4700() {
        let db = settle_company();
        book_quarter(&db);
        vat_file(&db, None, Some("2026-Q3"), None, "agent:test", false, "en").unwrap();
        let (amt, date, code, state) = import_payment(&db, "10.00", "DBIT");
        assert_eq!(state, "unmatched");
        let r = vat_settle(
            &db,
            amt,
            Some(&date),
            &code,
            None,
            None,
            Some("2026-Q3"),
            None,
            "agent:test",
            false,
            "en",
        )
        .unwrap();
        assert_eq!(r["difference_cents"], -50); // paid 50c less -> gain
        assert_eq!(r["difference_account"], "4700");
        assert_eq!(account_bal(&db, "2510"), 0);
        assert_eq!(account_bal(&db, "4700"), -50);
        assert_eq!(account_bal(&db, "1100"), 5050); // 121.00 - 60.50 - 10.00
        let args = audit_args(&db, "vat.settle").unwrap();
        assert_eq!(args["difference_cents"], -50);
    }

    #[test]
    fn settle_refund_received_in_your_favour_books_a_gain() {
        let db = settle_company();
        book_vat_entry(
            &db,
            "2026-07-01",
            "Inkoop",
            &specs(&["1100:-121.00,8000:100.00@21"]),
            "manual",
            None,
            "human:erik",
            true,
        )
        .unwrap();
        vat_file(&db, None, Some("2026-Q3"), None, "agent:test", false, "en").unwrap();
        let (amt, date, code, _) = import_payment(&db, "22.00", "CRDT");
        let r = vat_settle(
            &db,
            amt,
            Some(&date),
            &code,
            None,
            None,
            Some("2026-Q3"),
            None,
            "agent:test",
            false,
            "en",
        )
        .unwrap();
        assert_eq!(r["difference_cents"], -100); // received 1.00 more -> gain
        assert_eq!(account_bal(&db, "2510"), 0);
        assert_eq!(account_bal(&db, "4700"), -100);
        assert_eq!(account_bal(&db, "1100"), -9900); // -121.00 + 22.00
    }

    #[test]
    fn settle_paying_more_than_booked_books_a_loss() {
        let db = settle_company();
        book_quarter(&db);
        vat_file(&db, None, Some("2026-Q3"), None, "agent:test", false, "en").unwrap();
        let (amt, date, code, _) = import_payment(&db, "11.00", "DBIT");
        let r = vat_settle(
            &db,
            amt,
            Some(&date),
            &code,
            None,
            None,
            Some("2026-Q3"),
            None,
            "agent:test",
            false,
            "en",
        )
        .unwrap();
        assert_eq!(r["difference_cents"], 50); // loss (debit)
        assert_eq!(account_bal(&db, "4700"), 50);
    }

    #[test]
    fn settle_difference_beyond_five_euro_is_rejected() {
        let db = settle_company();
        book_quarter(&db);
        vat_file(&db, None, Some("2026-Q3"), None, "agent:test", false, "en").unwrap();
        let (amt, date, code, _) = import_payment(&db, "20.00", "DBIT");
        assert_eq!(
            vat_settle(
                &db,
                amt,
                Some(&date),
                &code,
                None,
                None,
                None,
                None,
                "agent:test",
                false,
                "en",
            )
            .unwrap_err()
            .code,
            "VAT_SETTLE_DIFFERENCE_TOO_LARGE"
        );
    }

    #[test]
    fn settle_nothing_without_a_filed_balance() {
        let db = settle_company();
        book_quarter(&db);
        let (amt, date, code, _) = import_payment(&db, "10.50", "DBIT");
        assert_eq!(
            vat_settle(
                &db,
                amt,
                Some(&date),
                &code,
                None,
                None,
                None,
                None,
                "agent:test",
                false,
                "en",
            )
            .unwrap_err()
            .code,
            "VAT_SETTLE_NOTHING"
        );
    }

    #[test]
    fn settle_direction_guard_blocks_incoming_against_a_payable() {
        let db = settle_company();
        book_quarter(&db);
        vat_file(&db, None, Some("2026-Q3"), None, "agent:test", false, "en").unwrap();
        let (amt, date, code, _) = import_payment(&db, "10.50", "CRDT");
        assert_eq!(
            vat_settle(
                &db,
                amt,
                Some(&date),
                &code,
                None,
                None,
                None,
                None,
                "agent:test",
                false,
                "en",
            )
            .unwrap_err()
            .code,
            "VAT_SETTLE_DIRECTION"
        );
    }

    #[test]
    fn settle_rejects_an_invalid_difference_account() {
        let db = settle_company();
        book_quarter(&db);
        vat_file(&db, None, Some("2026-Q3"), None, "agent:test", false, "en").unwrap();
        let (amt, date, code, _) = import_payment(&db, "10.50", "DBIT");
        assert_eq!(
            vat_settle(
                &db,
                amt,
                Some(&date),
                &code,
                None,
                Some("9999"),
                None,
                None,
                "agent:test",
                false,
                "en",
            )
            .unwrap_err()
            .code,
            "INVALID_DIFFERENCE_ACCOUNT"
        );
    }

    #[test]
    fn settle_dry_run_books_nothing_and_leaves_the_tx_unmatched() {
        let db = settle_company();
        book_quarter(&db);
        vat_file(&db, None, Some("2026-Q3"), None, "agent:test", false, "en").unwrap();
        let (amt, date, code, _) = import_payment(&db, "10.00", "DBIT");
        let r = vat_settle(
            &db,
            amt,
            Some(&date),
            &code,
            None,
            None,
            Some("2026-Q3"),
            None,
            "agent:test",
            true,
            "en",
        )
        .unwrap();
        assert_eq!(r["dryRun"], true);
        assert_eq!(r["difference_cents"], -50);
        assert_eq!(account_bal(&db, "2510"), -1050); // untouched
        let entries: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM journal_entries WHERE description LIKE '%Betaling OB%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(entries, 0);
        assert_eq!(last_tx(&db).3, "unmatched");
    }

    #[test]
    fn settle_custom_difference_account() {
        let db = settle_company();
        crate::accounts::create_account(
            &db,
            &crate::accounts::NewAccount {
                code: "4850",
                name: "Afrondingsverschillen",
                type_: "expense",
                normal_balance: "debit",
                taxonomy_code: None,
            },
        )
        .unwrap();
        book_quarter(&db);
        vat_file(&db, None, Some("2026-Q3"), None, "agent:test", false, "en").unwrap();
        let (amt, date, code, _) = import_payment(&db, "10.00", "DBIT");
        let r = vat_settle(
            &db,
            amt,
            Some(&date),
            &code,
            None,
            Some("4850"),
            None,
            None,
            "agent:test",
            false,
            "en",
        )
        .unwrap();
        assert_eq!(r["difference_account"], "4850");
        assert_eq!(account_bal(&db, "4850"), -50);
        assert_eq!(account_bal(&db, "4700"), 0);
    }

    #[test]
    fn readout_5d_agrees_with_the_booked_net_position() {
        let db = settle_company();
        book_quarter(&db);
        let readout = ob_readout(&db, "2026-07").unwrap();
        assert_eq!(readout["to_pay_cents"], 1050);
        assert_eq!(
            vat_net_position(&db),
            readout["to_pay_cents"].as_i64().unwrap()
        );
    }

    #[test]
    fn file_falls_to_the_next_free_code_when_2510_is_taken() {
        let db = settle_company();
        add_account(&db, "2510", "Te betalen omzetbelasting 2025");
        book_quarter(&db);
        let r = vat_file(&db, None, Some("2026-Q3"), None, "agent:test", false, "en").unwrap();
        assert_eq!(r["account"], "2511"); // next best numeric code
        assert_eq!(account_bal(&db, "2510"), 0); // the foreign 2510 is untouched
        assert_eq!(account_bal(&db, "2511"), -1050);
        let (name, type_, nb): (String, String, String) = db
            .query_row(
                "SELECT name, type, normal_balance FROM accounts WHERE code = '2511'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(name, "Af te dragen omzetbelasting");
        assert_eq!(type_, "liability");
        assert_eq!(nb, "credit");

        // and the settlement can target that account explicitly
        let (amt, date, code, _) = import_payment(&db, "10.00", "DBIT");
        let s = vat_settle(
            &db,
            amt,
            Some(&date),
            &code,
            Some("2511"),
            None,
            None,
            None,
            "agent:test",
            false,
            "en",
        )
        .unwrap();
        assert_eq!(s["account"], "2511");
        assert_eq!(s["difference_cents"], -50);
        assert_eq!(account_bal(&db, "2511"), 0);
    }

    #[test]
    fn file_uses_a_custom_account_when_requested_and_settle_cancels_it() {
        let db = settle_company();
        book_quarter(&db);
        let r = vat_file(
            &db,
            Some("2515"),
            Some("2026-Q3"),
            None,
            "agent:test",
            false,
            "en",
        )
        .unwrap();
        assert_eq!(r["account"], "2515");
        assert_eq!(account_bal(&db, "2515"), -1050);
        assert_eq!(account_bal(&db, "2510"), 0); // default account untouched
        let (amt, date, code, _) = import_payment(&db, "10.00", "DBIT");
        let s = vat_settle(
            &db,
            amt,
            Some(&date),
            &code,
            Some("2515"),
            None,
            None,
            None,
            "agent:test",
            false,
            "en",
        )
        .unwrap();
        assert_eq!(s["account"], "2515");
        assert_eq!(account_bal(&db, "2515"), 0);
    }

    #[test]
    fn file_dry_run_plans_the_next_free_code_without_creating_anything() {
        let db = settle_company();
        add_account(&db, "2510", "Oude schuld");
        book_quarter(&db);
        let r = vat_file(&db, None, Some("2026-Q3"), None, "agent:test", true, "en").unwrap();
        assert_eq!(r["dryRun"], true);
        assert_eq!(r["account"], "2511"); // the plan shows where it WOULD land
        let created: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM accounts WHERE code = '2511'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(created, 0);
        assert_eq!(account_bal(&db, "2510"), 0);
    }

    #[test]
    fn file_reuses_the_same_reassigned_account_across_filings() {
        let db = settle_company();
        add_account(&db, "2510", "Oude schuld");
        book_quarter(&db);
        let first = vat_file(&db, None, Some("2026-Q3"), None, "agent:test", false, "en").unwrap();
        assert_eq!(first["account"], "2511");
        // a second quarter must find the SAME 2511 again
        book_vat_entry(
            &db,
            "2026-10-01",
            "Omzet Q4",
            &specs(&["1100:121.00,8000:-100.00@21"]),
            "manual",
            None,
            "human:erik",
            true,
        )
        .unwrap();
        let second = vat_file(&db, None, Some("2026-Q4"), None, "agent:test", false, "en").unwrap();
        assert_eq!(second["account"], "2511"); // reuse, not 2512
        assert_eq!(account_bal(&db, "2511"), -3150); // -1050 (Q3) + -2100 (Q4)
    }

    #[test]
    fn enable_is_idempotent_and_seeds_codes() {
        let db = vat_company();
        let again = enable_vat_module(&db, "human:erik").unwrap();
        assert_eq!(again["vat_module"], 1);
        let codes = list_vat_codes(&db).unwrap();
        assert!(codes.iter().any(|c| c["code"] == "21"));
        assert!(codes.iter().any(|c| c["code"] == "V"));
    }

    #[test]
    fn vat_booking_and_readout() {
        let db = vat_company();
        // sale: bank 121.00 (gross), omzet -100.00 @21 -> VAT leg 2500 -21.00
        let e = book_vat_entry(
            &db,
            "2026-04-10",
            "Verkoop",
            &[
                VatSpec {
                    code: "1100".into(),
                    amount_cents: 12100,
                    vat_code: None,
                    fx_currency: None,
                    fx_amount_cents: None,
                },
                VatSpec {
                    code: "8000".into(),
                    amount_cents: -10000,
                    vat_code: Some("21".into()),
                    fx_currency: None,
                    fx_amount_cents: None,
                },
            ],
            "manual",
            None,
            "human:erik",
            true,
        )
        .unwrap();
        assert_eq!(e["state"], "posted");
        let ro = ob_readout(&db, "2026-Q2").unwrap();
        assert_eq!(ro["fields"]["1a"], 10000);
        assert_eq!(ro["fields"]["5a"], 2100);
        assert_eq!(ro["to_pay_cents"], 2100);
    }

    #[test]
    fn file_and_settle_roundtrip() {
        let db = vat_company();
        book_vat_entry(
            &db,
            "2026-04-10",
            "Verkoop",
            &[
                VatSpec {
                    code: "1100".into(),
                    amount_cents: 12100,
                    vat_code: None,
                    fx_currency: None,
                    fx_amount_cents: None,
                },
                VatSpec {
                    code: "8000".into(),
                    amount_cents: -10000,
                    vat_code: Some("21".into()),
                    fx_currency: None,
                    fx_amount_cents: None,
                },
            ],
            "manual",
            None,
            "human:erik",
            true,
        )
        .unwrap();
        // nothing outstanding on 2510 yet -> file moves 2500 position
        let filed = vat_file(&db, None, Some("2026-Q2"), None, "human:erik", false, "en").unwrap();
        assert_eq!(filed["owe"], true);
        assert_eq!(filed["liability_cents"], 2100);
        // settle the rounded whole-euro payment (21.00 exact -> difference 0)
        let settled = vat_settle(
            &db,
            -2100,
            Some("2026-07-01"),
            "1100",
            None,
            None,
            Some("2026-Q2"),
            None,
            "human:erik",
            false,
            "en",
        )
        .unwrap();
        assert_eq!(settled["difference_cents"], 0);
        // nothing left to settle
        assert_eq!(
            vat_settle(
                &db,
                -100,
                None,
                "1100",
                None,
                None,
                None,
                None,
                "human:erik",
                false,
                "en",
            )
            .unwrap_err()
            .code,
            "VAT_SETTLE_NOTHING"
        );
    }

    #[test]
    fn period_parsing() {
        assert_eq!(
            parse_period("2026-Q2").unwrap(),
            ("2026-04-01".into(), "2026-06-30".into())
        );
        assert_eq!(
            parse_period("2026-02").unwrap(),
            ("2026-02-01".into(), ("2026-02-28").into())
        );
        assert_eq!(
            parse_period("2024-02").unwrap(),
            ("2024-02-01".into(), "2024-02-29".into())
        );
        assert!(parse_period("2026-13").is_err());
        assert!(parse_period("garbage").is_err());
    }
}
