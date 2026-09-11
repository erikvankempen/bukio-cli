// bukio-cli — agent-first double-entry bookkeeping for SMEs.
// Copyright (c) 2026 Erik van Kempen.
// SPDX-License-Identifier: Apache-2.0
//
// Invoice engine (mirrors src/invoice/index.js — core functions).
// Skipped: compliance rule-sets (localization-only), paymentFromBank
// (bank-internal), invoiceReminders (read-only, add when needed).

use crate::accounts::resolve_profile;
use crate::audit::{record, RecordArgs};
use crate::contacts::get_contact;
use crate::entries::PostingSpec;
use crate::money::{format_amount, BukioError, Result};
use crate::vat::{is_vat_enabled, list_vat_codes};
use rusqlite::Connection;
use serde_json::{json, Value};

fn invoice_error(code: &'static str, msg: impl Into<String>) -> BukioError {
    BukioError::new(code, msg.into())
}

fn sql_err(e: rusqlite::Error) -> BukioError {
    BukioError::new("DB_ERROR", e.to_string())
}

// --- line spec parsing ----------------------------------------------------

fn parse_dutch_amount(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    // Dutch format: 1.234,56 (dot=thousands, comma=decimal)
    // English format: 1,234.56 or 150.00 (no comma = dot is decimal)
    let normalized = if s.contains(',') {
        s.replace('.', "").replace(',', ".")
    } else {
        s.to_string()
    };
    let val: f64 = normalized.parse().ok()?;
    Some((val * 100.0).round() as i64)
}

/// Parse "[QTYx] DESCRIPTION @ PRICE [@ VATCODE] [@ -DISCOUNT]"
pub fn parse_line_spec(spec: &str) -> Result<Value> {
    let s = spec.trim();
    // Try to extract quantity prefix
    // Try to extract quantity prefix — only when the text before the 'x' is a
    // number. The old code used the FIRST 'x' anywhere, so a description
    // containing an x ("x @ 1 @21", "Box @ 10") was read as a broken quantity
    // and the line was dropped.
    let (qty_milli, rest) = if let Some(idx) = s.find('x') {
        let qty_str = s[..idx].trim();
        let numeric = !qty_str.is_empty()
            && qty_str
                .trim_start_matches('-')
                .chars()
                .all(|c| c.is_ascii_digit() || c == '.' || c == ',');
        if !numeric {
            (1000, s)
        } else {
            if qty_str.starts_with('-') {
                return Err(invoice_error(
                    "INVALID_LINE",
                    format!("line '{spec}': quantity must be positive"),
                ));
            }
            let qty_f: f64 = qty_str.replace(',', ".").parse().map_err(|_| {
                invoice_error("INVALID_LINE", format!("line '{spec}': invalid quantity"))
            })?;
            let q = (qty_f * 1000.0).round() as i64;
            if q < 1 {
                return Err(invoice_error(
                    "INVALID_LINE",
                    format!("line '{spec}': quantity must be positive"),
                ));
            }
            (q, s[idx + 1..].trim_start())
        }
    } else {
        (1000, s)
    };

    // Split from right on '@'
    let parts: Vec<&str> = rest.split('@').map(|p| p.trim()).collect();
    if parts.len() < 2 {
        return Err(invoice_error(
            "INVALID_LINE",
            format!(
                "line '{spec}' must be \"[QTYx] DESCRIPTION @ PRICE [@ VATCODE] [@ -DISCOUNT]\""
            ),
        ));
    }

    // Check if last part is a discount
    let (discount_type, discount_value, parts) = {
        let last = *parts.last().unwrap();
        if last.starts_with('-') {
            let is_pct = last.ends_with('%');
            let num_str = if is_pct {
                &last[1..last.len() - 1]
            } else {
                &last[1..]
            };
            let val = parse_dutch_amount(num_str).ok_or_else(|| {
                invoice_error("INVALID_LINE", format!("line '{spec}': invalid discount"))
            })?;
            let dt = if is_pct { "pct" } else { "amount" };
            let dv = if is_pct { val } else { val };
            (
                Some(dt.to_string()),
                Some(dv),
                parts[..parts.len() - 1].to_vec(),
            )
        } else {
            (None, None, parts.to_vec())
        }
    };

    // Check if last remaining part is a VAT code (1-2 alphanumeric or 1-2 digit)
    let (vat_code, parts) = {
        let last = *parts.last().unwrap();
        let upper = last.to_uppercase();
        let is_code = upper.len() <= 2 && (upper.chars().all(|c| c.is_ascii_alphanumeric()));
        if is_code && !last.is_empty() {
            (Some(upper), parts[..parts.len() - 1].to_vec())
        } else {
            (None, parts)
        }
    };

    // Price is the second-to-last part
    if parts.len() < 2 {
        return Err(invoice_error(
            "INVALID_LINE",
            format!(
                "line '{spec}' must be \"[QTYx] DESCRIPTION @ PRICE [@ VATCODE] [@ -DISCOUNT]\""
            ),
        ));
    }
    let price_str = parts.last().unwrap();
    let price_cents = parse_dutch_amount(price_str)
        .ok_or_else(|| invoice_error("INVALID_LINE", format!("line '{spec}': invalid price")))?;
    if price_cents <= 0 {
        return Err(invoice_error(
            "INVALID_LINE",
            format!("line '{spec}': price must be positive"),
        ));
    }

    let description = parts[..parts.len() - 1].join("@").trim().to_string();
    if description.is_empty() {
        return Err(invoice_error(
            "INVALID_LINE",
            format!("line '{spec}': description required"),
        ));
    }

    Ok(json!({
        "qtyMilli": qty_milli,
        // the JS's qty is a plain JS number: JSON.stringify(1) is "1", not
        // "1.0" — serde prints the f64 form, which broke byte parity of --json
        "qty": if qty_milli % 1000 == 0 { json!(qty_milli / 1000) } else { json!(qty_milli as f64 / 1000.0) },
        "description": description, "priceCents": price_cents,
        "vatCode": vat_code, "discountType": discount_type, "discountValue": discount_value,
    }))
}

/// Normalise a parsed/object line to the engine's internal snake_case keys.
/// The JS accepts object lines in camelCase ({qtyMilli, priceCents, glAccount})
/// while our DB rows are snake_case — both must work.
fn to_snake_line(line: Value) -> Value {
    let mut o = line;
    if let Some(map) = o.as_object_mut() {
        for (camel, snake) in [
            ("qtyMilli", "qty_milli"),
            ("priceCents", "price_cents"),
            ("vatCode", "vat_code"),
            ("discountType", "discount_type"),
            ("discountValue", "discount_value"),
            ("glAccount", "gl_account"),
            ("itemId", "item_id"),
        ] {
            if !map.contains_key(snake) {
                if let Some(v) = map.get(camel).cloned() {
                    map.insert(snake.to_string(), v);
                }
            }
        }
    }
    o
}

pub fn split_line_specs(lines: &[Value]) -> Vec<Value> {
    // the JS only SPLITS here (String(spec).split(',').map(trim).filter(Boolean));
    // the parsing happens in create_invoice and MUST propagate. This used to
    // parse and silently drop failures, so a typo'd line vanished from the
    // invoice instead of raising INVALID_LINE.
    let mut result = Vec::new();
    for line in lines {
        if let Some(s) = line.as_str() {
            for part in s.split(',') {
                let trimmed = part.trim();
                if !trimmed.is_empty() {
                    result.push(Value::String(trimmed.to_string()));
                }
            }
        } else {
            result.push(line.clone());
        }
    }
    result
}

/// Parse item spec: "ID[:QTY][@PRICE][@VATCODE][@-DISCOUNT]"
pub fn parse_item_spec(spec: &str) -> Result<Value> {
    let s = spec.trim();
    let parts: Vec<&str> = s.split('@').collect();
    let id_part = parts[0].trim();

    // Parse ID[:QTY]
    let (item_id, qty_milli) = if let Some(colon_idx) = id_part.find(':') {
        let id: i64 = id_part[..colon_idx].parse().map_err(|_| {
            invoice_error(
                "INVALID_ITEM_SPEC",
                format!("item spec '{spec}': invalid item id"),
            )
        })?;
        let qty_f: f64 = id_part[colon_idx + 1..].parse().map_err(|_| {
            invoice_error(
                "INVALID_ITEM_SPEC",
                format!("item spec '{spec}': invalid quantity"),
            )
        })?;
        let q = (qty_f * 1000.0).round() as i64;
        if q < 1 {
            return Err(invoice_error(
                "INVALID_LINE",
                format!("item spec '{spec}': quantity must be positive"),
            ));
        }
        (id, q)
    } else {
        let id: i64 = id_part.parse().map_err(|_| {
            invoice_error(
                "INVALID_ITEM_SPEC",
                format!("item spec '{spec}': invalid item id"),
            )
        })?;
        (id, 1000)
    };

    let mut price_cents = None;
    let mut vat_code = None;
    let mut discount_type = None;
    let mut discount_value = None;

    // Parse @-separated overrides (skip first = id part)
    let overrides: Vec<&str> = parts[1..]
        .iter()
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .collect();
    let mut consumed = 0;
    if !overrides.is_empty() {
        let last = *overrides.last().unwrap();
        if last.starts_with('-') {
            let is_pct = last.ends_with('%');
            let num_str = if is_pct {
                &last[1..last.len() - 1]
            } else {
                &last[1..]
            };
            // a percentage is a plain number (10% -> 1000 bp), NOT an amount:
            // running it through parse_dutch_amount gave 10.00 EUR = 1000 cents
            // and the old *100 then made 100000, a 100% discount
            let val = if is_pct {
                let pct: f64 = num_str.parse().map_err(|_| {
                    invoice_error(
                        "INVALID_LINE",
                        format!("item spec '{spec}': invalid discount"),
                    )
                })?;
                (pct * 100.0).round() as i64
            } else {
                parse_dutch_amount(num_str).ok_or_else(|| {
                    invoice_error(
                        "INVALID_LINE",
                        format!("item spec '{spec}': invalid discount"),
                    )
                })?
            };
            discount_type = Some(if is_pct {
                "pct".to_string()
            } else {
                "amount".to_string()
            });
            discount_value = Some(val);
            consumed += 1;
        }
        let remaining = overrides.len() - consumed;
        if remaining > 0 {
            let candidate = overrides[remaining - 1];
            let upper = candidate.to_uppercase();
            if upper.len() <= 2 && upper.chars().all(|c| c.is_ascii_alphanumeric()) {
                vat_code = Some(upper);
                consumed += 1;
            }
        }
        let remaining = overrides.len() - consumed;
        if remaining > 0 {
            price_cents = Some(parse_dutch_amount(overrides[remaining - 1]).ok_or_else(|| {
                invoice_error("INVALID_LINE", format!("item spec '{spec}': invalid price"))
            })?);
        }
    }

    // the JS parseItemSpec returns camelCase — the invoice engine's
    // to_snake_line converts it back where it consumes the spec
    Ok(json!({
        "itemId": item_id, "qtyMilli": qty_milli,
        "priceCents": price_cents, "vatCode": vat_code,
        "discountType": discount_type, "discountValue": discount_value,
    }))
}

pub fn split_item_specs(items: &[Value]) -> Vec<Value> {
    let mut result = Vec::new();
    for item in items {
        if let Some(s) = item.as_str() {
            for part in s.split(',') {
                let trimmed = part.trim();
                if !trimmed.is_empty() {
                    if let Ok(parsed) = parse_item_spec(trimmed) {
                        result.push(parsed);
                    }
                }
            }
        } else {
            result.push(item.clone());
        }
    }
    result
}

// --- discount + VAT allocation -------------------------------------------

/// Largest-remainder allocation: every share floored, remaining cents go to
/// largest fractional parts. Σ result == total exactly.
pub fn allocate_largest_remainder(total: i64, weights: &[i64]) -> Vec<i64> {
    let n = weights.len();
    if n == 0 || total == 0 {
        return vec![0; n];
    }
    let w_sum: i64 = weights.iter().sum();
    if w_sum <= 0 {
        return vec![0; n];
    }
    let exact: Vec<f64> = weights
        .iter()
        .map(|&w| (w as f64 * total as f64) / w_sum as f64)
        .collect();
    let mut alloc: Vec<i64> = exact.iter().map(|e| *e as i64).collect();
    let mut rest = total - alloc.iter().sum::<i64>();
    let mut order: Vec<(usize, f64)> = exact
        .iter()
        .enumerate()
        .map(|(i, e)| (i, e - alloc[i] as f64))
        .collect();
    order.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.0.cmp(&b.0))
    });
    let mut k = 0;
    while rest > 0 {
        alloc[order[k % order.len()].0] += 1;
        rest -= 1;
        k += 1;
    }
    alloc
}

/// Per-line discount in cents
pub fn format_qty(qty: i64) -> String {
    if qty % 1000 == 0 {
        format!("{}", qty / 1000)
    } else {
        // the JS toFixed(3) then strips trailing zeros: 1500 -> "1.5", not "1.500"
        format!("{:.3}", qty as f64 / 1000.0)
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_string()
    }
}

pub fn line_discount_cents(line: &Value) -> i64 {
    let amount = line["amount_cents"].as_i64().unwrap_or(0);
    match line["discount_type"].as_str() {
        Some("pct") => {
            let val = line["discount_value"].as_i64().unwrap_or(0);
            ((amount as f64 * val as f64) / 10000.0).round() as i64
        }
        Some("amount") => {
            let val = line["discount_value"].as_i64().unwrap_or(0);
            val.min(amount)
        }
        _ => 0,
    }
}

/// Compute invoice totals from lines + optional total discount.
/// Returns { lineNets, groups, breakdown, net_cents, vat_cents, gross_cents, discount_cents }
pub fn compute_invoice_totals(
    lines: &[Value],
    discount_type: Option<&str>,
    discount_value: Option<i64>,
) -> Value {
    // Compute line nets
    let mut line_nets: Vec<Value> = lines
        .iter()
        .map(|l| {
            let disc = line_discount_cents(l);
            let line_net = l["amount_cents"].as_i64().unwrap_or(0) - disc;
            let mut ln = l.clone();
            ln["lineNet"] = json!(line_net);
            ln["lineDiscount"] = json!(disc);
            ln
        })
        .collect();

    let net_before: i64 = line_nets
        .iter()
        .map(|l| l["lineNet"].as_i64().unwrap_or(0))
        .sum();
    let discount_cents = match discount_type {
        Some("pct") => {
            ((net_before as f64 * discount_value.unwrap_or(0) as f64) / 10000.0).round() as i64
        }
        Some("amount") => discount_value.unwrap_or(0).min(net_before),
        _ => 0,
    };

    // Group lines by (vat_code, rate_bp, gl_account)
    let mut groups: Vec<Value> = Vec::new();
    for l in &line_nets {
        let code = l["vat_code"].as_str().unwrap_or("");
        let rate = l["vat_rate_bp"].as_i64().unwrap_or(0);
        let gl = l["gl_account"].as_str().unwrap_or("");
        let key = format!("{code}|{rate}|{gl}");
        if let Some(g) = groups.iter_mut().find(|g| g["key"].as_str() == Some(&key)) {
            g["net"] = json!(g["net"].as_i64().unwrap_or(0) + l["lineNet"].as_i64().unwrap_or(0));
            if let Some(arr) = g["lines"].as_array_mut() {
                arr.push(l.clone());
            }
        } else {
            groups.push(json!({
                "key": key, "code": if code.is_empty() { Value::Null } else { json!(code) },
                "rateBp": rate, "gl": if gl.is_empty() { Value::Null } else { json!(gl) },
                "net": l["lineNet"], "lines": vec![l.clone()],
            }));
        }
    }
    groups.retain(|g| g["net"].as_i64().unwrap_or(0) != 0);

    // Allocate total discount across groups
    let group_weights: Vec<i64> = groups
        .iter()
        .map(|g| g["net"].as_i64().unwrap_or(0).max(0))
        .collect();
    let alloc = allocate_largest_remainder(discount_cents, &group_weights);

    for (i, g) in groups.iter_mut().enumerate() {
        let g_net = g["net"].as_i64().unwrap_or(0);
        let discounted = g_net - alloc[i];
        g["discountedNet"] = json!(discounted);
        let vat = ((discounted.abs() as f64 * g["rateBp"].as_i64().unwrap_or(0) as f64) / 10000.0)
            .round() as i64;
        g["vat"] = json!(vat);

        // Distribute group VAT across lines
        let line_weights: Vec<i64> = g["lines"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .map(|l| l["lineNet"].as_i64().unwrap_or(0).max(0))
            .collect();
        let line_shares = allocate_largest_remainder(vat, &line_weights);
        if let Some(lines_arr) = g["lines"].as_array_mut() {
            for (k, l) in lines_arr.iter_mut().enumerate() {
                l["vatAmount"] = json!(line_shares.get(k).unwrap_or(&0));
            }
        }
    }

    // Propagate the per-line VAT back onto lineNets. The groups hold CLONES of
    // the lines, so without this every stored invoice line kept vat 0 and the
    // line-level VAT never reached the DB (createInvoice writes lineNets).
    let mut vat_by_line: std::collections::HashMap<i64, i64> = std::collections::HashMap::new();
    for g in &groups {
        if let Some(arr) = g["lines"].as_array() {
            for l in arr {
                if let (Some(no), Some(v)) = (l["line_no"].as_i64(), l["vatAmount"].as_i64()) {
                    vat_by_line.insert(no, v);
                }
            }
        }
    }
    for ln in line_nets.iter_mut() {
        if let Some(no) = ln["line_no"].as_i64() {
            ln["vatAmount"] = json!(vat_by_line.get(&no).copied().unwrap_or(0));
        }
    }

    let net: i64 = groups
        .iter()
        .map(|g| g["discountedNet"].as_i64().unwrap_or(0))
        .sum();
    let vat: i64 = groups.iter().map(|g| g["vat"].as_i64().unwrap_or(0)).sum();

    // Per-rate breakdown
    let mut breakdown: Vec<Value> = Vec::new();
    for g in &groups {
        let v = g["vat"].as_i64().unwrap_or(0);
        if v == 0 {
            continue;
        }
        let rate = g["rateBp"].as_i64().unwrap_or(0);
        if let Some(b) = breakdown.iter_mut().find(|b| b["rate_bp"] == rate) {
            b["base_cents"] = json!(
                b["base_cents"].as_i64().unwrap_or(0) + g["discountedNet"].as_i64().unwrap_or(0)
            );
            b["vat_cents"] = json!(b["vat_cents"].as_i64().unwrap_or(0) + v);
        } else {
            breakdown
                .push(json!({"rate_bp": rate, "base_cents": g["discountedNet"], "vat_cents": v}));
        }
    }
    breakdown.sort_by(|a, b| {
        b["rate_bp"]
            .as_i64()
            .unwrap_or(0)
            .cmp(&a["rate_bp"].as_i64().unwrap_or(0))
    });

    json!({
        "lineNets": line_nets, "groups": groups, "breakdown": breakdown,
        "net_cents": net, "vat_cents": vat, "gross_cents": net + vat,
        "discount_cents": discount_cents, "net_before_cents": net_before,
    })
}

// --- invoice DB operations ------------------------------------------------

fn serialize_invoice_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    // Matches explicit SELECT in get_invoice: id, invoice_type, invoice_number,
    // contact_id, date, due_date, status, language, currency, discount_type,
    // discount_value, notes, subtotal_cents, vat_cents, total_cents, reference,
    // created_by, created_at, description, delivery_date, entry_id, credit_for_invoice_id
    let id: i64 = row.get(0)?;
    let inv_type: Option<String> = row.get(1)?;
    let inv_number: Option<String> = row.get(2)?;
    let subtotal: Option<i64> = row.get(12)?;
    let vat: Option<i64> = row.get(13)?;
    let total: Option<i64> = row.get(14)?;
    Ok(json!({
        "id": id,
        "invoice_type": inv_type,
        "invoice_number": inv_number,
        "contact_id": row.get::<_, Option<i64>>(3)?,
        "date": row.get::<_, Option<String>>(4)?,
        "due_date": row.get::<_, Option<String>>(5)?,
        "status": row.get::<_, Option<String>>(6)?,
        "language": row.get::<_, Option<String>>(7)?,
        "currency": row.get::<_, Option<String>>(8)?,
        "discount_type": row.get::<_, Option<String>>(9)?,
        "discount_value": row.get::<_, Option<i64>>(10)?,
        "notes": row.get::<_, Option<String>>(11)?,
        "subtotal_cents": subtotal.unwrap_or(0),
        "vat_cents": vat.unwrap_or(0),
        "total_cents": total.unwrap_or(0),
        "reference": row.get::<_, Option<String>>(15)?,
        "created_by": row.get::<_, Option<String>>(16)?,
        "created_at": row.get::<_, Option<String>>(17)?,
        "description": row.get::<_, Option<String>>(18)?,
        "delivery_date": row.get::<_, Option<String>>(19)?,
        "entry_id": row.get::<_, Option<i64>>(20)?,
        "credit_for_invoice_id": row.get::<_, Option<i64>>(21)?,
    }))
}

fn get_invoice_lines(db: &Connection, invoice_id: i64) -> Result<Vec<Value>> {
    let mut stmt = db.prepare(
        "SELECT id, invoice_id, line_no, description, quantity, unit_price_cents, vat_code, vat_rate_bp, amount_cents, vat_amount_cents, item_id, unit, gl_account, discount_type, discount_value FROM invoice_lines WHERE invoice_id = ?1 ORDER BY line_no"
    ).map_err(sql_err)?;
    let rows = stmt.query_map([invoice_id], |r| {
        Ok(json!({
            "id": r.get::<_, i64>(0)?, "invoice_id": r.get::<_, i64>(1)?, "line_no": r.get::<_, i64>(2)?,
            "description": r.get::<_, String>(3)?, "quantity": r.get::<_, i64>(4)?,
            "unit_price_cents": r.get::<_, i64>(5)?, "vat_code": r.get::<_, Option<String>>(6)?,
            "vat_rate_bp": r.get::<_, i64>(7)?, "amount_cents": r.get::<_, i64>(8)?,
            "vat_amount_cents": r.get::<_, i64>(9)?, "item_id": r.get::<_, Option<i64>>(10)?,
            "unit": r.get::<_, Option<String>>(11)?, "gl_account": r.get::<_, Option<String>>(12)?,
            "discount_type": r.get::<_, Option<String>>(13)?, "discount_value": r.get::<_, Option<i64>>(14)?,
        }))
    }).map_err(sql_err)?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

fn get_invoice_payments(db: &Connection, invoice_id: i64) -> Result<Vec<Value>> {
    let mut stmt = db.prepare(
        "SELECT id, invoice_id, date, amount_cents, method, bank_tx_id, created_by FROM invoice_payments WHERE invoice_id = ?1 ORDER BY date, id"
    ).map_err(sql_err)?;
    let rows = stmt.query_map([invoice_id], |r| {
        Ok(json!({
            "id": r.get::<_, i64>(0)?, "invoice_id": r.get::<_, i64>(1)?, "date": r.get::<_, String>(2)?,
            "amount_cents": r.get::<_, i64>(3)?, "method": r.get::<_, String>(4)?,
            "bank_tx_id": r.get::<_, Option<i64>>(5)?,
            "created_by": r.get::<_, String>(6)?,
        }))
    }).map_err(sql_err)?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

/// The JS CLI's fmtLine projection (src/cli/invoice.js).
pub fn fmt_line(l: &Value) -> Value {
    let qty = l["quantity"].as_i64().unwrap_or(1000);
    let opt = |k: &str| {
        if l[k].is_null() {
            Value::Null
        } else {
            l[k].clone()
        }
    };
    json!({
        "line_no": l["line_no"], "description": l["description"],
        "quantity": format_qty(qty), "quantity_milli": qty,
        "unit": opt("unit"), "item_id": opt("item_id"),
        "unit_price_cents": l["unit_price_cents"],
        "unit_price": format_amount(l["unit_price_cents"].as_i64().unwrap_or(0)),
        "vat_code": l["vat_code"], "vat_rate_bp": l["vat_rate_bp"],
        "discount_type": l["discount_type"], "discount_value": l["discount_value"],
        "amount_cents": l["amount_cents"],
        "amount": format_amount(l["amount_cents"].as_i64().unwrap_or(0)),
        "vat_amount_cents": l["vat_amount_cents"],
        "vat_amount": format_amount(l["vat_amount_cents"].as_i64().unwrap_or(0)),
    })
}

/// The JS CLI's fmtInvoice projection (src/cli/invoice.js) — every invoice
/// command emits this shape, NOT the raw row from get_invoice.
pub fn fmt_invoice(i: &Value) -> Value {
    let net = i["net_cents"].as_i64().unwrap_or(0);
    let vat = i["vat_cents"].as_i64().unwrap_or(0);
    let gross = i["gross_cents"].as_i64().unwrap_or(0);
    let paid = i["paid_cents"].as_i64().unwrap_or(0);
    let lines: Vec<Value> = i["lines"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(fmt_line)
        .collect();
    let breakdown: Vec<Value> = i["vat_breakdown"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|b| {
            let bp = b["rate_bp"].as_i64().unwrap_or(0);
            json!({
                "rate_bp": bp,
                "rate": if bp % 100 == 0 { json!(bp / 100) } else { json!(bp as f64 / 100.0) },
                "base": format_amount(b["base_cents"].as_i64().unwrap_or(0)),
                "vat": format_amount(b["vat_cents"].as_i64().unwrap_or(0)),
            })
        })
        .collect();
    let payments: Vec<Value> = i["payments"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|p| {
            json!({
                "date": p["date"],
                "amount": format_amount(p["amount_cents"].as_i64().unwrap_or(0)),
                "method": p["method"],
            })
        })
        .collect();
    let contact_name = i["contact"].get("name").cloned().unwrap_or(Value::Null);
    json!({
        "id": i["id"], "invoice_number": i["invoice_number"], "invoice_type": i["invoice_type"],
        "contact_id": i["contact_id"], "contact_name": contact_name,
        "date": i["date"], "due_date": i["due_date"], "delivery_date": i["delivery_date"],
        "status": i["status"], "reference": i["reference"], "notes": i["notes"],
        "language": if i["language"].is_null() { json!("nl") } else { i["language"].clone() },
        "entry_id": i["entry_id"], "credit_for_invoice_id": i["credit_for_invoice_id"],
        "net_cents": net, "vat_cents": vat, "gross_cents": gross,
        "discount_type": i["discount_type"], "discount_value": i["discount_value"],
        "discount_cents": i["discount_cents"],
        "paid_cents": paid, "outstanding_cents": gross - paid,
        "net": format_amount(net), "vat": format_amount(vat),
        "gross": format_amount(gross), "paid": format_amount(paid),
        "lines": lines, "vat_breakdown": breakdown, "payments": payments,
    })
}

pub fn get_invoice(db: &Connection, id: i64) -> Result<Option<Value>> {
    let inv = db.query_row(
        "SELECT i.id, i.invoice_type, i.invoice_number, i.contact_id, i.date, i.due_date, i.status,
                i.language, i.currency, i.discount_type, i.discount_value, i.notes,
                COALESCE((SELECT SUM(amount_cents) FROM invoice_lines WHERE invoice_id = i.id), 0) AS subtotal_cents,
                COALESCE((SELECT SUM(vat_amount_cents) FROM invoice_lines WHERE invoice_id = i.id), 0) AS vat_cents,
                COALESCE((SELECT SUM(amount_cents + vat_amount_cents) FROM invoice_lines WHERE invoice_id = i.id), 0) AS total_cents,
                i.reference, i.created_by, i.created_at, i.description, i.delivery_date,
                i.entry_id, i.credit_for_invoice_id
         FROM invoices i WHERE i.id = ?1", [id], |r| {
        serialize_invoice_row(r)
    });
    let mut inv = match inv {
        Ok(v) => v,
        Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(None),
        Err(e) => return Err(sql_err(e)),
    };
    let lines = get_invoice_lines(db, id)?;
    let payments = get_invoice_payments(db, id)?;
    let contact = inv["contact_id"]
        .as_i64()
        .and_then(|cid| get_contact(db, cid).ok().flatten());
    let t = compute_invoice_totals(
        &lines,
        inv["discount_type"].as_str(),
        inv["discount_value"].as_i64(),
    );
    inv["lines"] = json!(lines);
    inv["payments"] = json!(payments);
    inv["contact"] = contact.unwrap_or(Value::Null);
    inv["net_cents"] = t["net_cents"].clone();
    inv["vat_cents"] = t["vat_cents"].clone();
    inv["gross_cents"] = t["gross_cents"].clone();
    inv["discount_cents"] = t["discount_cents"].clone();
    inv["vat_breakdown"] = t["breakdown"].clone();
    let paid: i64 = payments
        .iter()
        .map(|p| p["amount_cents"].as_i64().unwrap_or(0))
        .sum();
    inv["paid_cents"] = json!(paid);
    // Formatted amount strings (JS parity: fmtInvoice adds these)
    inv["net"] = json!(format_amount(inv["net_cents"].as_i64().unwrap_or(0)));
    inv["vat"] = json!(format_amount(inv["vat_cents"].as_i64().unwrap_or(0)));
    inv["gross"] = json!(format_amount(inv["gross_cents"].as_i64().unwrap_or(0)));
    inv["paid"] = json!(format_amount(paid));
    inv["outstanding_cents"] = json!(inv["gross_cents"].as_i64().unwrap_or(0) - paid);
    // Derived status: overdue
    if inv["status"].as_str() == Some("sent") {
        if let Some(due) = inv["due_date"].as_str() {
            let today = crate::dates::today_iso();
            if due < today.as_str() {
                inv["status"] = json!("overdue");
            }
        }
    }
    Ok(Some(inv))
}

pub fn list_invoices(
    db: &Connection,
    status: Option<&str>,
    invoice_type: Option<&str>,
) -> Result<Vec<Value>> {
    let mut sql = "SELECT i.id, i.invoice_type, i.invoice_number, i.contact_id, i.date, i.due_date, i.status,
        i.language, i.currency, i.discount_type, i.discount_value, i.notes,
        COALESCE((SELECT SUM(amount_cents) FROM invoice_lines WHERE invoice_id = i.id), 0) AS subtotal_cents,
        COALESCE((SELECT SUM(vat_amount_cents) FROM invoice_lines WHERE invoice_id = i.id), 0) AS vat_cents,
        COALESCE((SELECT SUM(amount_cents + vat_amount_cents) FROM invoice_lines WHERE invoice_id = i.id), 0) AS total_cents,
        i.reference, i.created_by, i.created_at, i.description, i.delivery_date,
        i.entry_id, i.credit_for_invoice_id FROM invoices i".to_string();
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let mut clauses = Vec::new();
    if let Some(s) = status {
        if s != "overdue" {
            clauses.push("status = ?".to_string());
            params.push(Box::new(s.to_string()));
        }
    }
    if let Some(t) = invoice_type {
        clauses.push("invoice_type = ?".to_string());
        params.push(Box::new(t.to_string()));
    }
    if !clauses.is_empty() {
        sql.push_str(&format!(" WHERE {}", clauses.join(" AND ")));
    }
    sql.push_str(" ORDER BY id DESC");
    let mut stmt = db.prepare(&sql).map_err(sql_err)?;
    let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
    let rows = stmt
        .query_map(param_refs.as_slice(), |r| serialize_invoice_row(r))
        .map_err(sql_err)?;
    // Enrich each invoice through the ONE serializer (JS maps getInvoice over
    // the rows) — a hand-rolled enrichment drifts: it dropped outstanding_cents
    // and the formatted amount strings.
    let mut invoices: Vec<Value> = Vec::new();
    for inv in rows.filter_map(|r| r.ok()) {
        let iid = inv["id"].as_i64().unwrap_or(0);
        if let Some(full) = get_invoice(db, iid)? {
            invoices.push(full);
        }
    }
    // Filter overdue in-Rust (can't filter derived status in SQL)
    if status == Some("overdue") {
        invoices.retain(|i| i["status"].as_str() == Some("overdue"));
    }
    Ok(invoices)
}

pub fn next_invoice_number(db: &Connection, year: i64) -> Result<String> {
    let pattern = format!("{year}-%");
    let row = db.query_row(
        "SELECT MAX(CAST(SUBSTR(invoice_number, 6) AS INTEGER)) FROM invoices WHERE invoice_number LIKE ?1",
        [&pattern], |r| r.get::<_, Option<i64>>(0),
    ).map_err(sql_err)?;
    let next = row.unwrap_or(0) + 1;
    Ok(format!("{year}-{next:04}"))
}

fn posting_defaults(db: &Connection) -> Result<Value> {
    let profile = resolve_profile(db)?;
    let sales = profile["reporting"]["defaultChart"]
        .as_array()
        .and_then(|arr| arr.iter().find(|a| a["type"] == "income"));
    let sales_code = sales.ok_or_else(|| {
        invoice_error("FORMAT_NOT_SUPPORTED", "profile declares no income account")
    })?["code"]
        .as_str()
        .ok_or_else(|| invoice_error("FORMAT_NOT_SUPPORTED", "income account has no code"))?;

    let mut vat_liability_code = None;
    if is_vat_enabled(db) {
        let vat_liab = profile["tax"]["accounts"]["ledger"]
            .as_array()
            .and_then(|arr| arr.iter().find(|a| a["type"] == "liability"));
        vat_liability_code = Some(
            vat_liab.ok_or_else(|| {
                invoice_error(
                    "FORMAT_NOT_SUPPORTED",
                    "profile declares no VAT liability account",
                )
            })?["code"]
                .as_str()
                .ok_or_else(|| {
                    invoice_error("FORMAT_NOT_SUPPORTED", "VAT liability account has no code")
                })?
                .to_string(),
        );
    }

    Ok(json!({
        "salesCode": sales_code,
        "vatLiabilityCode": vat_liability_code,
        "debtorsCode": profile["reporting"]["debtorsAccount"],
    }))
}

pub fn build_invoice_postings(db: &Connection, invoice: &Value) -> Result<Vec<Value>> {
    let pd = posting_defaults(db)?;
    let vat_on = is_vat_enabled(db);
    // the JS keys the sign off invoice_type (a credit note row carries
    // invoice_type: 'credit'); fall back to `type` for raw row shapes
    let is_credit = match invoice["invoice_type"].as_str() {
        Some(t) => t == "credit",
        None => invoice["type"].as_str() == Some("credit"),
    };
    let sign = if is_credit { 1i64 } else { -1i64 };
    let gross = invoice["gross_cents"].as_i64().unwrap_or(0);
    let mut postings: Vec<Value> = Vec::new();

    let lines = invoice["lines"].as_array().cloned().unwrap_or_default();
    let t = compute_invoice_totals(
        &lines,
        invoice["discount_type"].as_str(),
        invoice["discount_value"].as_i64(),
    );
    let groups = t["groups"].as_array().cloned().unwrap_or_default();

    if vat_on {
        for g in &groups {
            let dn = g["discountedNet"].as_i64().unwrap_or(0);
            if dn == 0 {
                continue;
            }
            let gl = g["gl"]
                .as_str()
                .or_else(|| pd["salesCode"].as_str())
                .unwrap_or("8000");
            let code = g["code"].as_str();
            let v = g["vat"].as_i64().unwrap_or(0);
            if let Some(c) = code {
                postings.push(json!({"code": gl, "amountCents": sign * dn, "vatCode": c, "vatAmountCents": if v > 0 { sign * v } else { 0 }}));
            } else {
                postings.push(json!({"code": gl, "amountCents": sign * dn}));
            }
        }
        if invoice["vat_cents"].as_i64().unwrap_or(0) > 0 {
            let vl = pd["vatLiabilityCode"].as_str().unwrap_or("2500");
            postings.push(json!({"code": vl, "amountCents": sign * invoice["vat_cents"].as_i64().unwrap_or(0)}));
        }
    } else {
        for g in &groups {
            let dn = g["discountedNet"].as_i64().unwrap_or(0);
            if dn == 0 {
                continue;
            }
            let gl = g["gl"]
                .as_str()
                .or_else(|| pd["salesCode"].as_str())
                .unwrap_or("8000");
            postings.push(json!({"code": gl, "amountCents": sign * dn}));
        }
    }
    // Debiteuren leg
    let dc = pd["debtorsCode"].as_str().unwrap_or("1200");
    postings.push(json!({"code": dc, "amountCents": if is_credit { -gross } else { gross }}));
    Ok(postings)
}

pub fn validate_compliance(db: &Connection, invoice: &Value) -> Result<()> {
    let profile = resolve_profile(db)?;
    let rule_key = profile["documents"]["invoiceCompliance"]
        .as_str()
        .unwrap_or("");
    if rule_key.is_empty() {
        return Err(invoice_error(
            "FORMAT_NOT_SUPPORTED",
            "no invoice compliance rule set for this profile yet (a B-milestone; \
             registered: nl-12-vereisten, eu-invoice-vereisten, lu-invoice-vereisten)",
        ));
    }
    // the JS has one validator per rule, each with its own wording (NL counts
    // the 12 vereisten, the EU rule cites art. 226 and labels taxId "tax id")
    let is_nl = rule_key == "nl-12-vereisten";
    let is_lu = rule_key == "lu-invoice-vereisten";
    // Per-rule wording, mirroring the JS validators: NL counts the twelve
    // vereisten, LU speaks French (loi du 12 février 1979 art. 66 / RCS), the
    // rest cite art. 226 of the EU VAT Directive.
    let (
        supplier_msg,
        fix_hint,
        name_label,
        tax_label,
        reg_label,
        addr_label,
        postal_label,
        city_label,
    ) = if is_nl {
        (
            "supplier details missing (requirements 1-3)",
            "set them with init/company update",
            "company name",
            "btw-id",
            "registration number",
            "address",
            "postal code",
            "city",
        )
    } else if is_lu {
        (
            "données du fournisseur manquantes",
            "définissez-les avec init/company update",
            "dénomination",
            "numéro de TVA",
            "numéro RCS",
            "adresse",
            "code postal",
            "ville",
        )
    } else {
        (
            "supplier details missing (art. 226(a)-(c) EU VAT Directive)",
            "set them with init/company update",
            "company name",
            "tax id",
            "registration number",
            "address",
            "postal code",
            "city",
        )
    };
    // Simplified: check supplier and customer party fields
    // select by NAME: the positional `SELECT *` indices drifted from the live
    // schema (migration 021 rebuilt the table), so the address was never seen
    // and an incomplete supplier passed validation
    let company = db.query_row(
        "SELECT name, tax_id, registration_id, address, postal_code, city, vat_module FROM company WHERE id = 1",
        [], |r| {
        Ok(json!({
            "name": r.get::<_, Option<String>>(0)?, "tax_id": r.get::<_, Option<String>>(1)?,
            "registration_id": r.get::<_, Option<String>>(2)?, "address": r.get::<_, Option<String>>(3)?,
            "postal_code": r.get::<_, Option<String>>(4)?, "city": r.get::<_, Option<String>>(5)?,
            "vat_module": r.get::<_, Option<i64>>(6)?,
        }))
    }).map_err(sql_err)?;

    let mut missing = Vec::new();
    if company["name"].as_str().is_none() {
        missing.push(name_label);
    }
    let supplier_has_vat =
        company["vat_module"].as_i64() == Some(1) || company["tax_id"].as_str().is_some();
    if supplier_has_vat && company["tax_id"].as_str().is_none() {
        missing.push(tax_label);
    }
    if company["registration_id"].as_str().is_none() {
        missing.push(reg_label);
    }
    if company["address"].as_str().is_none() {
        missing.push(addr_label);
    }
    if company["postal_code"].as_str().is_none() {
        missing.push(postal_label);
    }
    if company["city"].as_str().is_none() {
        missing.push(city_label);
    }
    if !missing.is_empty() {
        return Err(invoice_error(
            "SUPPLIER_INCOMPLETE",
            format!("{}: {} — {}", supplier_msg, missing.join(", "), fix_hint),
        ));
    }

    let contact = invoice["contact"].as_object();
    if let Some(c) = contact {
        if c.get("name").and_then(|v| v.as_str()).is_none()
            || c.get("address").and_then(|v| v.as_str()).is_none()
            || c.get("city").and_then(|v| v.as_str()).is_none()
        {
            return Err(invoice_error(
                "CUSTOMER_INCOMPLETE",
                if is_nl {
                    "customer details missing (requirement 6): name, address and city are required"
                } else if is_lu {
                    "données du client manquantes: nom, adresse et ville sont obligatoires"
                } else {
                    "customer details missing (art. 226(5) EU VAT Directive): name, address and city are required"
                },
            ));
        }
    }

    // Reverse charge check
    let lines = invoice["lines"].as_array().cloned().unwrap_or_default();
    let has_reverse = lines
        .iter()
        .any(|l| matches!(l["vat_code"].as_str(), Some("R") | Some("RE")));
    if has_reverse {
        let contact = invoice["contact"].as_object();
        if let Some(c) = contact {
            if c.get("vat_id").and_then(|v| v.as_str()).is_none() {
                return Err(invoice_error(
                    "CUSTOMER_VAT_REQUIRED",
                    if is_nl {
                        "reverse-charge invoice: the customer VAT id is required (requirement 7)"
                    } else if is_lu {
                        "auto-liquidation sur la facture: le numéro de TVA du client est obligatoire"
                    } else {
                        "reverse-charge invoice: the customer VAT id is required (art. 226(14) EU VAT Directive)"
                    },
                ));
            }
        }
    }
    Ok(())
}

/// The company's default document language: the base of `company.locale`, when
/// that base is an i18n table — otherwise 'en' (mirrors the JS).
pub fn default_document_language(db: &Connection) -> String {
    let locale: Option<String> = db
        .query_row("SELECT locale FROM company WHERE id = 1", [], |r| r.get(0))
        .ok()
        .flatten();
    match locale {
        Some(l) => {
            let base = l.split('-').next().unwrap_or("en").to_string();
            if crate::i18n::get_table(&base).is_some() {
                base
            } else {
                "en".to_string()
            }
        }
        None => "en".to_string(),
    }
}

pub fn create_invoice(
    db: &Connection,
    contact_id: i64,
    date: &str,
    due_days: Option<i64>,
    // the JS createInvoice({deliveryDate}) — the service/delivery date when it
    // differs from the invoice date; validated and stored, not just read
    delivery_date: Option<&str>,
    description: Option<&str>,
    reference: Option<&str>,
    notes: Option<&str>,
    discount_type: Option<&str>,
    discount_value: Option<i64>,
    language: Option<&str>,
    lines_raw: &[Value],
    actor: &str,
    dry_run: bool,
) -> Result<Value> {
    // the JS validates the date at create; a 2026-02-30 used to be stored
    crate::dates::validate_date(date)?;
    // and it rejects a negative payment term before touching the database —
    // Option<i64> already makes a non-integer unrepresentable, so only the sign
    // needs guarding (the CLI checks it too, but MCP and the recurring engine
    // call the engine directly)
    if let Some(dd) = due_days {
        if dd < 0 {
            return Err(invoice_error(
                "INVALID_DUE_DAYS",
                format!("due-days must be a non-negative integer, got '{dd}'"),
            ));
        }
    }
    // the JS checks both the shape and that the date exists — a 2026-02-30
    // passes a naive YYYY-MM-DD test and must not be stored
    if let Some(dd) = delivery_date {
        if dd.len() != 10 || crate::dates::validate_date(dd).is_err() {
            return Err(invoice_error(
                "INVALID_DATE",
                format!("delivery-date '{dd}' must be a valid YYYY-MM-DD date"),
            ));
        }
    }
    // every i18n table is a valid document language; anything else is rejected
    // (the stored column may be NULL — get_invoice then reports the 'nl' default)
    if let Some(l) = language {
        if crate::i18n::get_table(l).is_none() {
            return Err(invoice_error(
                "INVALID_LANGUAGE",
                format!("'{l}' is not a supported document language"),
            ));
        }
    }
    let _contact = get_contact(db, contact_id)?.ok_or_else(|| {
        invoice_error(
            "CONTACT_NOT_FOUND",
            format!("contact {contact_id} does not exist"),
        )
    })?;

    // Validate date
    if date.len() != 10 || date.as_bytes()[4] != b'-' || date.as_bytes()[7] != b'-' {
        return Err(invoice_error(
            "INVALID_DATE",
            format!("date '{date}' must be YYYY-MM-DD"),
        ));
    }

    // Split comma-separated line specs
    let lines_raw: Vec<Value> = split_line_specs(lines_raw);
    if lines_raw.is_empty() {
        return Err(invoice_error("NO_LINES", "an invoice needs lines"));
    }

    // the JS takes either --lines or --items, never both
    let has_item_spec = lines_raw
        .iter()
        .any(|v| v.get("item_id").or_else(|| v.get("itemId")).is_some());
    let has_line_spec = lines_raw.iter().any(|v| v.is_string());
    if has_item_spec && has_line_spec {
        return Err(invoice_error(
            "CONFLICTING_LINES",
            "pass either lines or items, not both",
        ));
    }

    let vat_on = is_vat_enabled(db);
    let vat_codes: std::collections::HashMap<String, Value> = list_vat_codes(db)?
        .into_iter()
        .filter_map(|c| {
            let code = c.get("code")?.as_str()?.to_string();
            Some((code, c))
        })
        .collect();

    // Parse and validate lines
    let mut parsed_lines: Vec<Value> = Vec::new();
    for (i, spec) in lines_raw.iter().enumerate() {
        let p = if spec.is_string() {
            parse_line_spec(spec.as_str().unwrap())?
        } else {
            spec.clone()
        };
        // both the JS camelCase shape and our snake_case must resolve
        let p = to_snake_line(p);
        // an item spec (--items) resolves against the catalog: snapshot the
        // description/price/VAT/unit/gl, applying any per-invoice override
        let p = if p.get("item_id").map(|v| !v.is_null()).unwrap_or(false) {
            let item_id = p["item_id"].as_i64().unwrap_or(0);
            let item = crate::items::get_item(db, item_id)?.ok_or_else(|| {
                invoice_error("ITEM_NOT_FOUND", format!("item {item_id} does not exist"))
            })?;
            let item_active = item["active"]
                .as_i64()
                .map(|v| v == 1)
                .or_else(|| item["active"].as_bool())
                .unwrap_or(false);
            if !item_active {
                return Err(invoice_error(
                    "ITEM_INACTIVE",
                    format!("item {item_id} is deactivated"),
                ));
            }
            let price = p["price_cents"]
                .as_i64()
                .unwrap_or_else(|| item["unit_price_cents"].as_i64().unwrap_or(0));
            if price <= 0 {
                return Err(invoice_error(
                    "INVALID_ITEM_OVERRIDE",
                    "item price override must be positive".to_string(),
                ));
            }
            let description = item["description"]
                .as_str()
                .filter(|d| !d.is_empty())
                .map(String::from)
                .unwrap_or_else(|| item["name"].as_str().unwrap_or("").to_string());
            json!({
                "description": description,
                "qty_milli": p["qty_milli"],
                "price_cents": price,
                "vat_code": if p["vat_code"].is_null() { item["vat_code"].clone() } else { p["vat_code"].clone() },
                "discount_type": p["discount_type"],
                "discount_value": p["discount_value"],
                "unit": item["unit"],
                "item_id": item_id,
                "gl_account": item["gl_account"],
            })
        } else {
            p
        };
        let desc = p["description"].as_str().unwrap_or("");
        let price = p["price_cents"].as_i64().unwrap_or(0);
        if desc.is_empty() || price <= 0 {
            return Err(invoice_error(
                "INVALID_LINE",
                format!("line {i}: not parseable"),
            ));
        }
        let qty = p["qty_milli"].as_i64().unwrap_or(1000);
        if qty < 1 {
            return Err(invoice_error(
                "INVALID_LINE",
                format!("line {i}: quantity must be positive"),
            ));
        }

        // the JS's assertLineDiscount: a null/null pair is fine, a percentage
        // must be in (0, 100] and a fixed discount must be positive — the port
        // accepted both a >100% discount and one that swallowed the whole line
        let (d_type, d_val) = (p["discount_type"].as_str(), p["discount_value"].as_i64());
        match (d_type, d_val) {
            (None, None) => {}
            (Some("pct"), Some(v)) if v > 0 && v <= 10000 => {}
            (Some("amount"), Some(v)) if v > 0 => {}
            _ => {
                return Err(invoice_error(
                    "INVALID_LINE_DISCOUNT",
                    format!(
                        "line {i}: discount must be a percentage in (0, 100] or a positive amount"
                    ),
                ))
            }
        }
        let line_amount = ((qty as f64 * price as f64) / 1000.0).round() as i64;
        if d_type == Some("amount") && d_val.unwrap_or(0) >= line_amount {
            return Err(invoice_error(
                "INVALID_LINE_DISCOUNT",
                format!("line {i}: fixed discount must be less than the line amount"),
            ));
        }
        let vat_code = p["vat_code"].as_str().map(String::from);
        if let Some(ref vc) = vat_code {
            if !vat_on {
                return Err(invoice_error(
                    "VAT_MODULE_OFF",
                    "line has a VAT code but the VAT module is off",
                ));
            }
            if !vat_codes.contains_key(vc) {
                return Err(invoice_error(
                    "VAT_CODE_NOT_FOUND",
                    format!("vat code '{vc}' does not exist"),
                ));
            }
        }
        let rate_bp = vat_code
            .as_ref()
            .and_then(|vc| vat_codes.get(vc).and_then(|c| c["rate_bp"].as_i64()))
            .unwrap_or(0);
        let amount = ((qty as f64 * price as f64) / 1000.0).round() as i64;

        parsed_lines.push(json!({
            "line_no": i + 1, "description": desc, "quantity": qty,
            "unit_price_cents": price, "vat_code": vat_code, "vat_rate_bp": rate_bp,
            "amount_cents": amount, "vat_amount_cents": 0,
            "item_id": p["item_id"], "unit": p["unit"], "gl_account": p["gl_account"],
            "discount_type": p["discount_type"], "discount_value": p["discount_value"],
        }));
    }

    let totals = compute_invoice_totals(&parsed_lines, discount_type, discount_value);
    // Set vat_amount_cents from totals
    if let Some(line_nets) = totals["lineNets"].as_array() {
        for (i, ln) in line_nets.iter().enumerate() {
            if let Some(line) = parsed_lines.get_mut(i) {
                line["vat_amount_cents"] = ln["vatAmount"].clone();
            }
        }
    }

    // Compute due date — the JS defaults dueDays to 30 when the caller omits it
    let due_days = Some(due_days.unwrap_or(30));
    let due_date = due_days.map(|dd| {
        let y: i64 = date[..4].parse().unwrap_or(2026);
        let m: i64 = date[5..7].parse().unwrap_or(1);
        let d: i64 = date[8..10].parse().unwrap_or(1);
        let _total_days = d + dd;
        // Simple date arithmetic (handles month overflow)
        let dt = chrono::NaiveDate::from_ymd_opt(y as i32, m as u32, d as u32)
            .unwrap_or(chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap());
        let dt = dt + chrono::Duration::days(dd);
        dt.format("%Y-%m-%d").to_string()
    });

    if dry_run {
        return Ok(json!({
            "action": "invoice.create", "contact_id": contact_id, "date": date,
            "due_days": due_days, "delivery_date": delivery_date,
            "description": description, "reference": reference,
            "notes": notes, "discount_type": discount_type, "discount_value": discount_value,
            "discount_cents": totals["discount_cents"],
            "lines": parsed_lines, "net_cents": totals["net_cents"],
            "vat_cents": totals["vat_cents"], "gross_cents": totals["gross_cents"],
            "vat_breakdown": totals["breakdown"], "due_date": due_date, "dryRun": true,
        }));
    }

    // Insert invoice
    db.execute(
        "INSERT INTO invoices (contact_id, date, due_date, delivery_date, description, reference, notes, discount_type, discount_value, language, created_by) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        // the column is NOT NULL; the JS default is 'nl' (get_invoice keeps the same fallback for old rows)
        rusqlite::params![contact_id, date, due_date, delivery_date, description, reference, notes, discount_type, discount_value, language
            .map(String::from)
            .unwrap_or_else(|| default_document_language(db)), actor],
    ).map_err(sql_err)?;
    let invoice_id = db.last_insert_rowid();

    // Insert lines
    for (i, l) in parsed_lines.iter().enumerate() {
        db.execute(
            "INSERT INTO invoice_lines (invoice_id, line_no, description, quantity, unit_price_cents, vat_code, vat_rate_bp, amount_cents, vat_amount_cents, item_id, unit, gl_account, discount_type, discount_value) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            rusqlite::params![
                invoice_id, (i + 1) as i64,
                l["description"].as_str().unwrap_or(""),
                l["quantity"].as_i64().unwrap_or(0),
                l["unit_price_cents"].as_i64().unwrap_or(0),
                l["vat_code"].as_str(),
                l["vat_rate_bp"].as_i64().unwrap_or(0),
                l["amount_cents"].as_i64().unwrap_or(0),
                l["vat_amount_cents"].as_i64().unwrap_or(0),
                l["item_id"].as_i64(),
                l["unit"].as_str(),
                l["gl_account"].as_str(),
                l["discount_type"].as_str(),
                l["discount_value"].as_i64(),
            ],
        ).map_err(sql_err)?;
    }

    record(
        db,
        RecordArgs {
            actor,
            action: "invoice.create",
            command: Some("invoice create"),
            args: Some(
                json!({"contactId": contact_id, "date": date, "lines": parsed_lines.len(), "net": totals["net_cents"], "gross": totals["gross_cents"]}),
            ),
            outcome: "ok",
            entry_ids: vec![],
        },
    )?;

    get_invoice(db, invoice_id)?
        .ok_or_else(|| invoice_error("DB_ERROR", "invoice not found after insert"))
}

pub fn finalize_invoice(db: &Connection, id: i64, actor: &str, dry_run: bool) -> Result<Value> {
    let invoice = get_invoice(db, id)?
        .ok_or_else(|| invoice_error("NOT_FOUND", format!("invoice {id} does not exist")))?;
    let status = invoice["status"].as_str().unwrap_or("draft");
    if status != "draft" {
        return Err(invoice_error(
            "ALREADY_FINALIZED",
            format!("invoice {id} is already {status}"),
        ));
    }

    validate_compliance(db, &invoice)?;
    let year_str = invoice["date"].as_str().unwrap_or("2026");
    let year: i64 = year_str[..4].parse().unwrap_or(2026);
    let postings = build_invoice_postings(db, &invoice)?;

    if dry_run {
        return Ok(json!({
            "invoice_number": next_invoice_number(db, year)?, "postings": postings,
            "net": invoice["net_cents"], "vat": invoice["vat_cents"], "gross": invoice["gross_cents"],
            "dryRun": true,
        }));
    }

    // Transaction with retry for invoice_number collision
    let mut attempts = 0;
    loop {
        match (|| -> Result<Value> {
            let number = next_invoice_number(db, year)?;
            let contact_name = invoice["contact"]
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let desc = format!("Invoice {number} - {contact_name}");

            let entry = crate::entries::create_entry(
                db,
                crate::entries::CreateEntry {
                    date: year_str,
                    description: &desc,
                    postings: postings
                        .iter()
                        .map(|p| crate::entries::PostingSpec {
                            code: p["code"].as_str().unwrap_or("").to_string(),
                            amount_cents: p["amountCents"].as_i64().unwrap_or(0),
                            cost_center_code: p
                                .get("costCenterCode")
                                .and_then(|v| v.as_str())
                                .map(String::from),
                            // carry the VAT tag through — the OB readout derives
                            // 1a/1b/1c/2a from tagged income postings
                            vat_code: p.get("vatCode").and_then(|v| v.as_str()).map(String::from),
                            vat_amount_cents: p.get("vatAmountCents").and_then(|v| v.as_i64()),
                            fx_currency: None,
                            fx_amount_cents: None,
                        })
                        .collect(),
                    source: "invoice",
                    source_ref: Some(&format!("inv:{id}")),
                    actor,
                },
            )?;
            let posted = crate::entries::post_entry(db, entry.id, actor)?;

            db.execute(
                "UPDATE invoices SET invoice_number = ?1, status = 'sent', entry_id = ?2 WHERE id = ?3",
                rusqlite::params![number, posted.id, id],
            ).map_err(sql_err)?;

            record(
                db,
                RecordArgs {
                    actor,
                    action: "invoice.finalize",
                    command: Some("invoice finalize"),
                    args: Some(json!({"id": id, "invoice_number": number})),
                    outcome: "ok",
                    entry_ids: vec![posted.id],
                },
            )?;
            Ok(
                json!({"invoice": get_invoice(db, id)?, "entry": crate::entries::entry_to_json(&posted)}),
            )
        })() {
            Ok(result) => return Ok(result),
            Err(e) => {
                if e.message
                    .contains("UNIQUE constraint failed: invoices.invoice_number")
                    && attempts < 5
                {
                    attempts += 1;
                    continue;
                }
                return Err(e);
            }
        }
    }
}

pub fn credit_invoice(
    db: &Connection,
    id: i64,
    date: Option<&str>,
    reason: Option<&str>,
    actor: &str,
    dry_run: bool,
) -> Result<Value> {
    let original = get_invoice(db, id)?
        .ok_or_else(|| invoice_error("NOT_FOUND", format!("invoice {id} does not exist")))?;
    if original["invoice_type"].as_str() != Some("sales") {
        return Err(invoice_error(
            "NOT_SALES_INVOICE",
            "only sales invoices can be credited",
        ));
    }
    let status = original["status"].as_str().unwrap_or("draft");
    if !["sent", "paid", "overdue"].contains(&status) {
        return Err(invoice_error(
            "NOT_FINALIZED",
            "the invoice must be finalized before crediting",
        ));
    }

    let today = crate::dates::today_iso();
    let credit_date = date.unwrap_or(&today);
    if dry_run {
        return Ok(json!({
            "action": "invoice.credit", "for_invoice": id, "reason": reason,
            "date": credit_date, "dryRun": true,
        }));
    }

    let orig_lines: Vec<Value> = original["lines"].as_array().cloned().unwrap_or_default();
    // Pass structured lines directly — create_invoice handles non-string specs
    // but needs 'price_cents' not 'unit_price_cents'
    let credit_lines_raw: Vec<Value> = orig_lines
        .iter()
        .map(|l| {
            let mut line = l.clone();
            // rename unit_price_cents -> price_cents for create_invoice
            if let Some(upc) = line.get("unit_price_cents").cloned() {
                line["price_cents"] = upc;
            }
            // rename quantity -> qty_milli (milliunits)
            if let Some(qty) = line.get("quantity").cloned() {
                line["qty_milli"] = qty;
            }
            line
        })
        .collect();
    let contact_id = original["contact_id"].as_i64().unwrap_or(0);
    let credit = create_invoice(
        db,
        contact_id,
        credit_date,
        None,
        // a credit note carries no service date of its own
        None,
        Some(reason.unwrap_or(&format!(
            "Credit note for {}",
            original["invoice_number"].as_str().unwrap_or("")
        ))),
        // carry the buyer reference (klantkenmerk) so BT-10 on the credit note
        // matches the original; fall back to the original invoice number
        original["reference"]
            .as_str()
            .or_else(|| original["invoice_number"].as_str()),
        None,
        // the invoice-level discount is re-applied on the credit, like the JS
        // (original.discount_type/discount_value) — dropping it credited the
        // UNDISCOUNTED amount and over-credited the customer
        original["discount_type"].as_str(),
        original["discount_value"].as_i64(),
        // the credit note inherits the source document language
        original["language"].as_str().or(Some("nl")),
        &credit_lines_raw,
        actor,
        false,
    )?;

    let credit_id = credit["id"].as_i64().unwrap_or(0);
    db.execute(
        "UPDATE invoices SET invoice_type = 'credit', credit_for_invoice_id = ?1, due_date = NULL WHERE id = ?2",
        rusqlite::params![id, credit_id],
    )
    .map_err(sql_err)?;

    record(
        db,
        RecordArgs {
            actor,
            action: "invoice.credit",
            command: Some("invoice credit"),
            args: Some(json!({"id": id, "creditId": credit_id})),
            outcome: "ok",
            entry_ids: vec![],
        },
    )?;
    get_invoice(db, credit_id)?
        .ok_or_else(|| BukioError::new("DB_ERROR", "invoice not found after credit"))
}

pub fn mark_paid(
    db: &Connection,
    id: i64,
    date: &str,
    amount_cents: i64,
    method: &str,
    actor: &str,
    dry_run: bool,
    // the reconciliation link (JS markPaid takes bankTxId); None for a manual
    // payment, Some(tx) when the payment comes off a bank transaction
    bank_tx_id: Option<i64>,
) -> Result<Value> {
    let invoice = get_invoice(db, id)?
        .ok_or_else(|| invoice_error("NOT_FOUND", format!("invoice {id} does not exist")))?;
    if invoice["invoice_type"].as_str() == Some("credit") {
        return Err(invoice_error(
            "CREDIT_NOT_PAYABLE",
            "credit notes are not payable",
        ));
    }
    let status = invoice["status"].as_str().unwrap_or("draft");
    if !["sent", "overdue"].contains(&status) {
        return Err(invoice_error(
            "NOT_PAYABLE",
            format!("invoice {id} is {status}"),
        ));
    }
    if amount_cents <= 0 {
        return Err(invoice_error(
            "INVALID_AMOUNT",
            "payment amount must be positive",
        ));
    }
    if date.len() != 10 || date.as_bytes()[4] != b'-' || date.as_bytes()[7] != b'-' {
        return Err(invoice_error(
            "INVALID_DATE",
            format!("payment date '{date}' must be YYYY-MM-DD"),
        ));
    }

    let gross = invoice["gross_cents"].as_i64().unwrap_or(0);
    let paid = invoice["paid_cents"].as_i64().unwrap_or(0);
    let remaining = gross - paid;
    if amount_cents > remaining {
        return Err(invoice_error(
            "OVERPAYMENT",
            format!("payment {amount_cents} exceeds the outstanding {remaining}"),
        ));
    }

    if dry_run {
        return Ok(json!({
            "action": "invoice.pay", "invoice_id": id, "date": date,
            "amount_cents": amount_cents, "method": method, "remaining_cents": remaining, "dryRun": true,
        }));
    }

    // the payment and the status flip must be one unit: a failure between them
    // used to leave the payment recorded with the invoice still 'sent'
    let tx = crate::entries::begin(db)?;
    tx.execute(
        "INSERT INTO invoice_payments (invoice_id, date, amount_cents, method, bank_tx_id, created_by) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![id, date, amount_cents, method, bank_tx_id, actor],
    ).map_err(sql_err)?;

    if paid + amount_cents >= gross {
        tx.execute("UPDATE invoices SET status = 'paid' WHERE id = ?1", [id])
            .map_err(sql_err)?;
    }
    tx.commit()?;

    record(
        db,
        RecordArgs {
            actor,
            action: "invoice.pay",
            command: Some("invoice pay"),
            args: Some(json!({"id": id, "amountCents": amount_cents})),
            outcome: "ok",
            entry_ids: vec![],
        },
    )?;
    get_invoice(db, id)?.ok_or_else(|| BukioError::new("DB_ERROR", "invoice not found"))
}

/// FX-match sanity bound shared by the autoMatch SQL (src/bank.rs) and
/// payment_from_bank: a payment may differ from an invoice's outstanding by up
/// to 2% (floor 25 cents) and the gap is booked to 4840 Koersverschillen. Keep
/// the two consumers in sync — drift would make autoMatch match what
/// payment_from_bank then rejects (FX_DIFFERENCE_TOO_LARGE).
pub const FX_MATCH_TOLERANCE_BP: i64 = 200; // 2%
pub const FX_MATCH_FLOOR_CENTS: i64 = 25; // small-invoice absolute floor

/// Apply a bank payment to an invoice: record the payment, post the bank entry
/// (Bank / Debiteuren) and reconcile the transaction. Mirrors the JS
/// paymentFromBank, which the bank auto-match engine routes through.
///
/// An invoice booked from a foreign-currency price is translated at the invoice
/// date and the incoming transfer converted at the PAYMENT date, so the amount
/// received can differ. Within the sanity bound it is booked to 4840
/// Koersverschillen and the invoice settles in full; beyond it the difference
/// is not an FX move but a wrong amount, and is rejected. The whole flow is one
/// unit (entries::begin nests when the caller already holds a transaction, like
/// better-sqlite3): a half-written payment with no entry would double-pay on a
/// re-match.
pub fn payment_from_bank(
    db: &Connection,
    invoice_id: i64,
    bank_tx_id: i64,
    actor: &str,
    fx_tolerance_bp: i64,
) -> Result<Value> {
    let tx = crate::entries::begin(db)?;
    let invoice = get_invoice(&tx, invoice_id)?.ok_or_else(|| {
        invoice_error("NOT_FOUND", format!("invoice {invoice_id} does not exist"))
    })?;
    let row: rusqlite::Result<(String, i64, i64, String)> = tx.query_row(
        "SELECT date, amount_cents, bank_account_id, state FROM bank_transactions WHERE id = ?1",
        [bank_tx_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    );
    let (tx_date, tx_amount, tx_account_id, tx_state) = row.map_err(|_| {
        invoice_error(
            "NOT_FOUND",
            format!("bank transaction {bank_tx_id} does not exist"),
        )
    })?;
    if tx_state != "unmatched" {
        return Err(invoice_error(
            "ALREADY_MATCHED",
            format!("bank transaction {bank_tx_id} is already {tx_state}"),
        ));
    }
    let bank_code: Option<String> = tx
        .query_row(
            "SELECT account_code FROM bank_accounts WHERE id = ?1",
            [tx_account_id],
            |r| r.get::<_, Option<String>>(0),
        )
        .unwrap_or(None)
        .filter(|c| !c.is_empty());
    let bank_code = bank_code.ok_or_else(|| {
        invoice_error(
            "ACCOUNT_NOT_FOUND",
            format!("bank account {tx_account_id} has no ledger account — set one before matching"),
        )
    })?;

    let outstanding =
        invoice["gross_cents"].as_i64().unwrap_or(0) - invoice["paid_cents"].as_i64().unwrap_or(0);
    let delta = tx_amount - outstanding; // + paid more (FX gain), - paid less (FX loss)
    let mut fx_cents = 0i64;
    if delta != 0 {
        // floor 25 cents: absorbs cent-level rounding on tiny invoices, and
        // stays small enough that a €10 invoice paid €9 is still rejected
        let tolerance = std::cmp::max(
            ((outstanding as f64 * fx_tolerance_bp as f64) / 10000.0).round() as i64,
            FX_MATCH_FLOOR_CENTS,
        );
        if delta.abs() > tolerance {
            return Err(invoice_error(
                "FX_DIFFERENCE_TOO_LARGE",
                format!("payment {tx_amount} differs from the outstanding {outstanding} by {delta} cents — beyond the {fx_tolerance_bp}bp sanity bound; check the amount before booking"),
            ));
        }
        fx_cents = delta;
    }

    // settle the invoice in full — the FX difference absorbs the cent-level gap
    let paid = mark_paid(
        &tx,
        invoice_id,
        &tx_date,
        outstanding,
        "bank",
        actor,
        false,
        Some(bank_tx_id),
    )?;
    let debtors_code = crate::accounts::resolve_profile(&tx)
        .ok()
        .and_then(|p| p["reporting"]["debtorsAccount"].as_str().map(String::from))
        .unwrap_or_else(|| "1200".into());
    let mut postings = vec![
        PostingSpec {
            code: bank_code,
            amount_cents: tx_amount,
            cost_center_code: None,
            vat_code: None,
            vat_amount_cents: None,
            fx_currency: None,
            fx_amount_cents: None,
        },
        PostingSpec {
            code: debtors_code,
            amount_cents: -outstanding,
            cost_center_code: None,
            vat_code: None,
            vat_amount_cents: None,
            fx_currency: None,
            fx_amount_cents: None,
        },
    ];
    let number = invoice["invoice_number"].as_str().unwrap_or("");
    let mut description = if number.is_empty() {
        format!("Payment {invoice_id}")
    } else {
        format!("Payment {number}")
    };
    if let Some(name) = invoice["contact"]["name"].as_str() {
        if !name.is_empty() {
            description.push_str(&format!(" - {name}"));
        }
    }
    if fx_cents != 0 {
        let fx_code = crate::bank::ensure_fx_difference_account(&tx, actor)?;
        postings.push(PostingSpec {
            code: fx_code,
            amount_cents: -fx_cents,
            cost_center_code: None,
            vat_code: None,
            vat_amount_cents: None,
            fx_currency: None,
            fx_amount_cents: None,
        });
        description.push_str(&format!(
            " (fx difference {})",
            crate::money::format_amount(fx_cents)
        ));
    }

    let source_ref = format!("tx:{bank_tx_id}");
    let entry = crate::entries::create_entry(
        &tx,
        crate::entries::CreateEntry {
            date: &tx_date,
            description: &description,
            postings,
            source: "bank",
            source_ref: Some(&source_ref),
            actor,
        },
    )?;
    let posted = crate::entries::post_entry(&tx, entry.id, actor)?;
    let method = if actor.starts_with("agent") {
        "agent"
    } else {
        "manual"
    };
    tx.execute(
        "INSERT INTO reconciliations (bank_tx_id, target_type, target_id, method, confidence, created_by)
         VALUES (?1, 'invoice', ?2, ?3, 1.0, ?4)",
        rusqlite::params![bank_tx_id, invoice_id, method, actor],
    )
    .map_err(sql_err)?;
    tx.execute(
        "UPDATE bank_transactions SET state = 'matched' WHERE id = ?1",
        [bank_tx_id],
    )
    .map_err(sql_err)?;
    crate::audit::record(
        &tx,
        crate::audit::RecordArgs {
            actor,
            action: "invoice.payment_bank",
            command: Some("bank match"),
            args: Some(json!({ "invoiceId": invoice_id, "bankTxId": bank_tx_id })),
            outcome: "ok",
            entry_ids: vec![posted.id],
        },
    )?;
    tx.commit()?;

    Ok(json!({
        "invoice": paid,
        "entry": serde_json::to_value(&posted).unwrap_or(Value::Null),
    }))
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_line_basic() {
        let r = parse_line_spec("2x Consultancy @ 150.00 @21").unwrap();
        assert_eq!(r["qtyMilli"], 2000);
        assert_eq!(r["description"], "Consultancy");
        assert_eq!(r["priceCents"], 15000);
        assert_eq!(r["vatCode"], "21");
    }

    #[test]
    fn parse_line_with_discount() {
        let r = parse_line_spec("1x Service @ 100.00 @9 @-10%").unwrap();
        assert_eq!(r["discountType"], "pct");
        assert_eq!(r["discountValue"], 1000);
    }

    #[test]
    fn allocate_largest_remainder_exact() {
        let a = allocate_largest_remainder(100, &[1, 1, 1]);
        assert_eq!(a.iter().sum::<i64>(), 100);
        assert_eq!(a, vec![34, 33, 33]);
    }

    #[test]
    fn line_discount_pct() {
        let line = json!({"amount_cents": 10000, "discount_type": "pct", "discount_value": 1000});
        assert_eq!(line_discount_cents(&line), 1000);
    }

    #[test]
    fn line_discount_amount() {
        let line = json!({"amount_cents": 5000, "discount_type": "amount", "discount_value": 500});
        assert_eq!(line_discount_cents(&line), 500);
    }

    #[test]
    fn compute_totals_no_discount() {
        let lines = vec![
            json!({"amount_cents": 10000, "vat_code": "21", "vat_rate_bp": 2100, "lineNet": 10000, "lineDiscount": 0}),
            json!({"amount_cents": 5000, "vat_code": "9", "vat_rate_bp": 900, "lineNet": 5000, "lineDiscount": 0}),
        ];
        let t = compute_invoice_totals(&lines, None, None);
        assert_eq!(t["net_cents"], 15000);
        assert_eq!(t["gross_cents"], 17550);
        assert_eq!(t["discount_cents"], 0);
    }

    #[test]
    fn parse_item_spec_basic() {
        let r = parse_item_spec("1:2@140.00@21").unwrap();
        assert_eq!(r["itemId"], 1);
        assert_eq!(r["qtyMilli"], 2000);
        assert_eq!(r["priceCents"], 14000);
        assert_eq!(r["vatCode"], "21");
    }

    #[test]
    fn next_invoice_number_sequential() {
        let d = crate::db::open_db(":memory:").unwrap();
        d.execute("INSERT INTO company (name) VALUES ('TestCo')", [])
            .unwrap();
        // Empty table -> 2026-0001
        let n = next_invoice_number(&d, 2026).unwrap();
        assert_eq!(n, "2026-0001");
    }

    // ==== ported from test/invoice.test.js ==================================

    fn company_db(vat: bool, complete: bool) -> Connection {
        let db = crate::db::open_db(":memory:").unwrap();
        crate::accounts::seed_default_chart(&db).unwrap();
        let address = if complete {
            Some("Industrieweg 12")
        } else {
            None
        };
        db.execute(
            "INSERT INTO company (name, registration_id, legal_form, tax_id, iban, address, postal_code, city, vat_module) \
             VALUES ('Demo BV', '12345678', 'bv', 'NL123456789B01', 'NL91ABNA0417164300', ?1, '2712 CD', 'Zoetermeer', ?2)",
            rusqlite::params![address, if vat { 1 } else { 0 }],
        )
        .unwrap();
        if vat {
            crate::vat::enable_vat_module(&db, "human:erik").unwrap();
        }
        db
    }

    fn mk_contact(db: &Connection, vat_id: Option<&str>) -> i64 {
        crate::contacts::create_contact(
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

    const DEFAULT_LINE: &str = "2x Consultancy @ 150.00 @21";

    fn new_invoice(db: &Connection, contact_id: i64, date: &str, lines: &[Value]) -> Result<Value> {
        create_invoice(
            db,
            contact_id,
            date,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            lines,
            "agent:test",
            false,
        )
    }

    fn mk_invoice(db: &Connection) -> Value {
        new_invoice(db, 1, &days_from_now(-2), &[json!(DEFAULT_LINE)]).unwrap()
    }

    /// The JS test reads postings through getEntry (the ENGINE shape), which
    /// carries the per-posting VAT; the CLI's entry projection does not.
    fn entry_json(db: &Connection, id: i64) -> Value {
        serde_json::to_value(crate::entries::get_entry(db, id).unwrap()).unwrap()
    }

    fn postings_of<'a>(entry: &'a Value, code: &str) -> Option<&'a Value> {
        entry["postings"]
            .as_array()?
            .iter()
            .find(|p| p["account_code"].as_str() == Some(code))
    }

    #[test]
    fn parse_line_spec_qty_description_price_vat() {
        let l = parse_line_spec("2x Consultancy @ 150.00 @21").unwrap();
        assert_eq!(l["qtyMilli"].as_i64(), Some(2000));
        assert_eq!(l["qty"].as_f64(), Some(2.0));
        assert_eq!(l["description"].as_str(), Some("Consultancy"));
        assert_eq!(l["priceCents"].as_i64(), Some(15000));
        assert_eq!(l["vatCode"].as_str(), Some("21"));
        assert!(l["discountType"].is_null());

        let l2 = parse_line_spec("Kantoorartikelen @ 45,50").unwrap();
        assert_eq!(l2["qtyMilli"].as_i64(), Some(1000));
        assert_eq!(l2["priceCents"].as_i64(), Some(4550));
        assert!(l2["vatCode"].is_null());

        assert_eq!(parse_line_spec("garbage").unwrap_err().code, "INVALID_LINE");
    }

    #[test]
    fn create_invoice_draft_line_math_and_due_date() {
        let db = company_db(true, true);
        mk_contact(&db, None);
        let inv = create_invoice(
            &db,
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
            &[json!("2x Consultancy @ 150.00 @21")],
            "agent:test",
            false,
        )
        .unwrap();
        assert_eq!(inv["status"].as_str(), Some("draft"));
        assert!(inv["invoice_number"].is_null());
        assert_eq!(inv["lines"].as_array().unwrap().len(), 1);
        assert_eq!(inv["lines"][0]["amount_cents"].as_i64(), Some(30000));
        assert_eq!(inv["lines"][0]["vat_amount_cents"].as_i64(), Some(6300));
        assert_eq!(inv["net_cents"].as_i64(), Some(30000));
        assert_eq!(inv["vat_cents"].as_i64(), Some(6300));
        assert_eq!(inv["gross_cents"].as_i64(), Some(36300));
        assert_eq!(inv["due_date"].as_str(), Some("2026-08-09")); // +30 days (NL profile)
    }

    #[test]
    fn create_invoice_guards() {
        // unknown contact
        let db = company_db(true, true);
        assert_eq!(
            new_invoice(&db, 99, "2026-07-10", &[json!("x @ 1 @21")])
                .unwrap_err()
                .code,
            "CONTACT_NOT_FOUND"
        );
        mk_contact(&db, None);
        assert_eq!(
            new_invoice(&db, 1, "2026-07-10", &[]).unwrap_err().code,
            "NO_LINES"
        );
        assert_eq!(
            new_invoice(&db, 1, "bad", &[json!("x @ 1")])
                .unwrap_err()
                .code,
            "INVALID_DATE"
        );
        // a vat code with the module off
        let off = company_db(false, true);
        mk_contact(&off, None);
        assert_eq!(
            new_invoice(&off, 1, &days_from_now(-2), &[json!(DEFAULT_LINE)])
                .unwrap_err()
                .code,
            "VAT_MODULE_OFF"
        );
        // unknown vat code (module on)
        let on = company_db(true, true);
        mk_contact(&on, None);
        assert_eq!(
            new_invoice(&on, 1, "2026-07-10", &[json!("x @ 1 @99")])
                .unwrap_err()
                .code,
            "VAT_CODE_NOT_FOUND"
        );
    }

    #[test]
    fn validate_compliance_requires_supplier_and_customer_data() {
        // missing supplier address
        let db = company_db(true, false);
        mk_contact(&db, None);
        let inv = new_invoice(&db, 1, "2026-07-10", &[json!("x @ 1 @21")]).unwrap();
        assert_eq!(
            validate_compliance(&db, &inv).unwrap_err().code,
            "SUPPLIER_INCOMPLETE"
        );
        // missing customer address
        let db = company_db(true, true);
        crate::contacts::create_contact(
            &db,
            "Zonder Adres",
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
        let inv = new_invoice(&db, 1, "2026-07-10", &[json!("x @ 1 @21")]).unwrap();
        assert_eq!(
            validate_compliance(&db, &inv).unwrap_err().code,
            "CUSTOMER_INCOMPLETE"
        );
        // reverse-charged (verlegd) needs the customer's vat id
        let db = company_db(true, true);
        mk_contact(&db, None);
        let inv = new_invoice(&db, 1, "2026-07-10", &[json!("x @ 1 @R")]).unwrap();
        assert_eq!(
            validate_compliance(&db, &inv).unwrap_err().code,
            "CUSTOMER_VAT_REQUIRED"
        );
        // complete passes
        let db = company_db(true, true);
        mk_contact(&db, None);
        let inv = mk_invoice(&db);
        assert!(validate_compliance(&db, &inv).is_ok());
    }

    #[test]
    fn finalize_assigns_a_sequential_number_and_books_the_invoice() {
        let db = company_db(true, true);
        mk_contact(&db, None);
        let inv = mk_invoice(&db);
        let r = finalize_invoice(&db, inv["id"].as_i64().unwrap(), "agent:test", false).unwrap();
        assert_eq!(r["invoice"]["invoice_number"].as_str(), Some("2026-0001"));
        assert_eq!(r["invoice"]["status"].as_str(), Some("sent"));
        assert_eq!(r["invoice"]["entry_id"].as_i64(), r["entry"]["id"].as_i64());

        let entry = entry_json(&db, r["entry"]["id"].as_i64().unwrap());
        let entry = &entry;
        assert_eq!(entry["source"].as_str(), Some("invoice"));
        assert_eq!(entry["state"].as_str(), Some("posted"));
        assert_eq!(
            postings_of(entry, "1200").unwrap()["amount_cents"].as_i64(),
            Some(36300)
        ); // debiteuren
        assert_eq!(
            postings_of(entry, "8000").unwrap()["amount_cents"].as_i64(),
            Some(-30000)
        );
        assert_eq!(
            postings_of(entry, "8000").unwrap()["vat_amount_cents"].as_i64(),
            Some(-6300)
        );
        assert_eq!(
            postings_of(entry, "2500").unwrap()["amount_cents"].as_i64(),
            Some(-6300)
        );

        // the second invoice continues the sequence
        let inv2 = mk_invoice(&db);
        finalize_invoice(&db, inv2["id"].as_i64().unwrap(), "agent:test", false).unwrap();
        let shown = get_invoice(&db, inv2["id"].as_i64().unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(shown["invoice_number"].as_str(), Some("2026-0002"));
    }

    #[test]
    fn finalize_multiple_vat_rates_gives_per_rate_postings_with_exact_vat() {
        let db = company_db(true, true);
        mk_contact(&db, None);
        let inv = create_invoice(
            &db,
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
            &[
                json!("1x Dienstverlening @ 100.00 @21"),
                json!("2x Maaltijd @ 25.00 @9"),
            ],
            "agent:test",
            false,
        )
        .unwrap();
        let r = finalize_invoice(&db, inv["id"].as_i64().unwrap(), "agent:test", false).unwrap();
        let entry = entry_json(&db, r["entry"]["id"].as_i64().unwrap());
        let omzet: Vec<&Value> = entry["postings"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|p| p["account_code"].as_str() == Some("8000"))
            .collect();
        assert_eq!(omzet.len(), 2);
        assert!(omzet
            .iter()
            .any(|p| p["vat_amount_cents"].as_i64() == Some(-2100)));
        assert!(omzet
            .iter()
            .any(|p| p["vat_amount_cents"].as_i64() == Some(-450)));
        // gross = 100 + 21 + 50 + 4.50
        assert_eq!(
            postings_of(&entry, "1200").unwrap()["amount_cents"].as_i64(),
            Some(17550)
        );
    }

    #[test]
    fn finalize_with_the_vat_module_off_books_net_only() {
        let db = company_db(false, true);
        mk_contact(&db, None);
        let inv = new_invoice(&db, 1, "2026-07-10", &[json!("1x Dienst @ 100.00")]).unwrap();
        let r = finalize_invoice(&db, inv["id"].as_i64().unwrap(), "agent:test", false).unwrap();
        let entry = entry_json(&db, r["entry"]["id"].as_i64().unwrap());
        let postings = entry["postings"].as_array().unwrap();
        assert_eq!(postings.len(), 2);
        assert_eq!(
            postings_of(&entry, "1200").unwrap()["amount_cents"].as_i64(),
            Some(10000)
        );
        assert_eq!(
            postings_of(&entry, "8000").unwrap()["amount_cents"].as_i64(),
            Some(-10000)
        );
    }

    #[test]
    fn finalize_rejects_a_finalized_invoice_and_a_dry_run_writes_nothing() {
        let db = company_db(true, true);
        mk_contact(&db, None);
        let inv = mk_invoice(&db);
        finalize_invoice(&db, inv["id"].as_i64().unwrap(), "agent:test", false).unwrap();
        assert_eq!(
            finalize_invoice(&db, inv["id"].as_i64().unwrap(), "agent:test", false)
                .unwrap_err()
                .code,
            "ALREADY_FINALIZED"
        );

        let inv2 = mk_invoice(&db);
        let plan = finalize_invoice(&db, inv2["id"].as_i64().unwrap(), "agent:test", true).unwrap();
        assert_eq!(plan["dryRun"].as_bool(), Some(true));
        assert_eq!(plan["invoice_number"].as_str(), Some("2026-0002"));
        let after = get_invoice(&db, inv2["id"].as_i64().unwrap())
            .unwrap()
            .unwrap();
        assert!(after["invoice_number"].is_null());
        let entries: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM journal_entries WHERE source = 'invoice'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(entries, 1);
    }

    #[test]
    fn credit_note_reverses_the_booking_and_the_sequence_continues() {
        let db = company_db(true, true);
        mk_contact(&db, None);
        let inv = mk_invoice(&db);
        finalize_invoice(&db, inv["id"].as_i64().unwrap(), "agent:test", false).unwrap();

        let credit = credit_invoice(
            &db,
            inv["id"].as_i64().unwrap(),
            None,
            Some("verkeerde tarief"),
            "agent:test",
            false,
        )
        .unwrap();
        assert_eq!(credit["invoice_type"].as_str(), Some("credit"));
        assert_eq!(credit["credit_for_invoice_id"].as_i64(), inv["id"].as_i64());
        assert_eq!(credit["reference"].as_str(), Some("2026-0001"));
        assert_eq!(credit["lines"].as_array().unwrap().len(), 1);

        let r = finalize_invoice(&db, credit["id"].as_i64().unwrap(), "agent:test", false).unwrap();
        assert_eq!(r["invoice"]["invoice_number"].as_str(), Some("2026-0002"));
        let entry = entry_json(&db, r["entry"]["id"].as_i64().unwrap());
        assert_eq!(
            postings_of(&entry, "1200").unwrap()["amount_cents"].as_i64(),
            Some(-36300)
        ); // debiteuren credit
        assert_eq!(
            postings_of(&entry, "8000").unwrap()["amount_cents"].as_i64(),
            Some(30000)
        ); // omzet debit
        assert_eq!(
            postings_of(&entry, "2500").unwrap()["amount_cents"].as_i64(),
            Some(6300)
        ); // vat debit
    }

    #[test]
    fn payments_partial_then_full_then_overpayment_is_rejected() {
        let db = company_db(true, true);
        mk_contact(&db, None);
        let inv = mk_invoice(&db);
        let id = inv["id"].as_i64().unwrap();
        finalize_invoice(&db, id, "agent:test", false).unwrap();

        let partial = mark_paid(
            &db,
            id,
            &days_from_now(-1),
            20000,
            "bank",
            "agent:test",
            false,
            None,
        )
        .unwrap();
        assert_eq!(partial["status"].as_str(), Some("sent"));
        assert_eq!(partial["paid_cents"].as_i64(), Some(20000));

        let paid = mark_paid(
            &db,
            id,
            &days_from_now(-1),
            16300,
            "bank",
            "agent:test",
            false,
            None,
        )
        .unwrap();
        assert_eq!(paid["status"].as_str(), Some("paid"));

        assert_eq!(
            mark_paid(
                &db,
                id,
                &days_from_now(-1),
                100,
                "bank",
                "agent:test",
                false,
                None
            )
            .unwrap_err()
            .code,
            "NOT_PAYABLE"
        );

        let inv2 = mk_invoice(&db);
        let id2 = inv2["id"].as_i64().unwrap();
        finalize_invoice(&db, id2, "agent:test", false).unwrap();
        assert_eq!(
            mark_paid(
                &db,
                id2,
                &days_from_now(-1),
                40000,
                "bank",
                "agent:test",
                false,
                None
            )
            .unwrap_err()
            .code,
            "OVERPAYMENT"
        );
    }

    #[test]
    fn next_invoice_number_is_year_scoped() {
        let db = company_db(true, true);
        mk_contact(&db, None);
        assert_eq!(next_invoice_number(&db, 2026).unwrap(), "2026-0001");
        let inv = mk_invoice(&db);
        finalize_invoice(&db, inv["id"].as_i64().unwrap(), "agent:test", false).unwrap();
        assert_eq!(next_invoice_number(&db, 2026).unwrap(), "2026-0002");
        assert_eq!(next_invoice_number(&db, 2027).unwrap(), "2027-0001");
    }

    #[test]
    fn mark_paid_payment_and_status_update_are_atomic() {
        // the JS simulates a crash between the payment INSERT and the status
        // UPDATE by monkey-patching db.prepare; rusqlite cannot be patched, so
        // the same failure is forced with a SQLite trigger — if the port ever
        // loses the transaction, the payment survives and this fails.
        let db = company_db(true, true);
        mk_contact(&db, None);
        let inv = mk_invoice(&db);
        let id = inv["id"].as_i64().unwrap();
        finalize_invoice(&db, id, "agent:test", false).unwrap();

        db.execute(
            "CREATE TRIGGER boom BEFORE UPDATE ON invoices BEGIN SELECT RAISE(ABORT, 'simulated crash'); END",
            [],
        )
        .unwrap();
        let result = mark_paid(
            &db,
            id,
            &days_from_now(-1),
            36300,
            "bank",
            "agent:test",
            false,
            None,
        );
        db.execute("DROP TRIGGER boom", []).unwrap();
        assert!(result.is_err());

        let payments: i64 = db
            .query_row("SELECT COUNT(*) FROM invoice_payments", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            payments, 0,
            "the payment must not survive a failed status update"
        );
        let after = get_invoice(&db, id).unwrap().unwrap();
        assert_eq!(after["status"].as_str(), Some("sent"));
    }

    #[test]
    fn finalize_never_reuses_an_existing_invoice_number() {
        // the JS test monkey-patches the first UPDATE to throw a UNIQUE error,
        // which Rust cannot do. Instead a row holds the number the sequence
        // would compute while sitting outside the year the sequence scans: the
        // retry (or the MAX read) must still land on a free number, never
        // surfacing a raw constraint error and never double-booking.
        let db = company_db(true, true);
        mk_contact(&db, None);
        db.execute(
            "INSERT INTO invoices (contact_id, date, status, invoice_number, invoice_type, created_by) \
             VALUES (1, '2025-07-01', 'sent', '2026-0001', 'sales', 'agent:test')",
            [],
        )
        .unwrap();
        let inv = mk_invoice(&db);
        let r = finalize_invoice(&db, inv["id"].as_i64().unwrap(), "agent:test", false).unwrap();
        let number = r["invoice"]["invoice_number"].as_str().unwrap().to_string();
        assert_ne!(number, "2026-0001", "a taken number must not be reused");
        assert_eq!(r["invoice"]["status"].as_str(), Some("sent"));
        let entries: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM journal_entries WHERE source = 'invoice'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(entries, 1, "exactly one booking entry");
    }

    #[test]
    fn build_invoice_postings_sales_vs_credit_sign_flip() {
        let db = company_db(true, true);
        mk_contact(&db, None);
        let inv = mk_invoice(&db);
        let sales = build_invoice_postings(&db, &inv).unwrap();
        assert_eq!(
            sales
                .iter()
                .find(|p| p["code"].as_str() == Some("1200"))
                .unwrap()["amountCents"]
                .as_i64(),
            Some(36300)
        );
        let mut credit = inv.clone();
        credit["invoice_type"] = json!("credit");
        let reversed = build_invoice_postings(&db, &credit).unwrap();
        assert_eq!(
            reversed
                .iter()
                .find(|p| p["code"].as_str() == Some("1200"))
                .unwrap()["amountCents"]
                .as_i64(),
            Some(-36300)
        );
    }

    #[test]
    fn build_invoice_postings_honours_the_line_gl_account_with_the_vat_module_off() {
        let db = company_db(false, true);
        mk_contact(&db, None);
        let inv = new_invoice(
            &db,
            1,
            "2026-07-10",
            &[json!({
                "qtyMilli": 1000, "description": "Zonder btw",
                "priceCents": 10000, "glAccount": "8050"
            })],
        )
        .unwrap();
        let postings = build_invoice_postings(&db, &inv).unwrap();
        assert_eq!(
            postings
                .iter()
                .find(|p| p["code"].as_str() == Some("1200"))
                .unwrap()["amountCents"]
                .as_i64(),
            Some(10000)
        );
        assert_eq!(
            postings
                .iter()
                .find(|p| p["code"].as_str() == Some("8050"))
                .unwrap()["amountCents"]
                .as_i64(),
            Some(-10000),
            "the line GL must be honoured with the VAT module off"
        );
        assert!(
            !postings.iter().any(|p| p["code"].as_str() == Some("8000")),
            "the hardcoded 8000 default must not be used when a line GL exists"
        );
    }
}
