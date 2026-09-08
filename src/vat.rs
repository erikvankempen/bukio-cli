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
                });
                if vat_amount != 0 && vtype != "reverse" {
                    vat_legs.push(PostingSpec {
                        code: vat_account,
                        amount_cents: vat_amount,
                        cost_center_code: None,
                    });
                }
            }
            None => expanded.push(Expanded {
                code: spec.code.clone(),
                amount_cents: spec.amount_cents,
                vat_code: None,
                vat_amount_cents: None,
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
                "fx_currency": null, "fx_amount_cents": null, "fx_amount": null,
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
                if (1..=12).contains(&m) {
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
        },
        PostingSpec {
            code: input_code.clone(),
            amount_cents: -bal_input,
            cost_center_code: None,
        },
        PostingSpec {
            code: account.clone(),
            amount_cents: bal_output + bal_input,
            cost_center_code: None,
        },
    ];
    postings.retain(|p| p.amount_cents != 0);
    let owe = net > 0;
    let liability = net.abs();
    let description = desc.map(String::from).unwrap_or_else(|| {
        let direction = if owe { "payable" } else { "receivable" };
        format!(
            "VAT filing{period_part} — reclassify to {account} ({direction})",
            period_part = period.map(|p| format!(" {p}")).unwrap_or_default()
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
        },
        PostingSpec {
            code: bank_account_code.to_string(),
            amount_cents: tx_amount_cents,
            cost_center_code: None,
        },
    ];
    if difference != 0 {
        postings.push(PostingSpec {
            code: difference_account.clone(),
            amount_cents: difference,
            cost_center_code: None,
        });
    }
    let description = desc.map(String::from).unwrap_or_else(|| {
        format!(
            "VAT settlement{period_part} — {} (difference {diff})",
            settlement_account_name(db),
            period_part = period.map(|p| format!(" {p}")).unwrap_or_default(),
            diff = format_amount(difference)
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
                },
                VatSpec {
                    code: "8000".into(),
                    amount_cents: -10000,
                    vat_code: Some("21".into()),
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
                },
                VatSpec {
                    code: "8000".into(),
                    amount_cents: -10000,
                    vat_code: Some("21".into()),
                },
            ],
            "manual",
            None,
            "human:erik",
            true,
        )
        .unwrap();
        // nothing outstanding on 2510 yet -> file moves 2500 position
        let filed = vat_file(&db, None, Some("2026-Q2"), None, "human:erik", false).unwrap();
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
                false
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
