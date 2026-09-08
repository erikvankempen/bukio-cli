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
use crate::items::get_item;
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
    let (qty_milli, rest) = if let Some(idx) = s.find("x ") {
        let qty_str = s[..idx].trim();
        if qty_str.starts_with('-') {
            return Err(invoice_error(
                "INVALID_LINE",
                format!("line '{spec}': quantity must be positive"),
            ));
        }
        let qty_f: f64 = qty_str.parse().map_err(|_| {
            invoice_error("INVALID_LINE", format!("line '{spec}': invalid quantity"))
        })?;
        let q = (qty_f * 1000.0).round() as i64;
        if q < 1 {
            return Err(invoice_error(
                "INVALID_LINE",
                format!("line '{spec}': quantity must be positive"),
            ));
        }
        (q, s[idx + 2..].trim())
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
        "qty_milli": qty_milli, "qty": qty_milli as f64 / 1000.0,
        "description": description, "price_cents": price_cents,
        "vat_code": vat_code, "discount_type": discount_type, "discount_value": discount_value,
    }))
}

pub fn split_line_specs(lines: &[Value]) -> Vec<Value> {
    let mut result = Vec::new();
    for line in lines {
        if let Some(s) = line.as_str() {
            for part in s.split(',') {
                let trimmed = part.trim();
                if !trimmed.is_empty() {
                    if let Ok(parsed) = parse_line_spec(trimmed) {
                        result.push(parsed);
                    }
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
            let val = parse_dutch_amount(num_str).ok_or_else(|| {
                invoice_error(
                    "INVALID_LINE",
                    format!("item spec '{spec}': invalid discount"),
                )
            })?;
            discount_type = Some(if is_pct {
                "pct".to_string()
            } else {
                "amount".to_string()
            });
            discount_value = Some(if is_pct { val * 100 } else { val });
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

    Ok(json!({
        "item_id": item_id, "qty_milli": qty_milli,
        "price_cents": price_cents, "vat_code": vat_code,
        "discount_type": discount_type, "discount_value": discount_value,
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
        format!("{:.3}", qty as f64 / 1000.0)
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
    let line_nets: Vec<Value> = lines
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
    // Column order: id, type, number, contact_id, date, due_date, status,
    // language, currency, discount_type, discount_value, notes, subtotal_cents,
    // vat_cents, total_cents, reference, created_by, created_at, description,
    // delivery_date, invoice_type, invoice_number, entry_id, credit_for_invoice_id
    Ok(json!({
        "id": row.get::<_, i64>(0)?,
        "type": row.get::<_, Option<String>>(1)?,
        "invoice_number": row.get::<_, Option<String>>(2)?,
        "contact_id": row.get::<_, Option<i64>>(3)?,
        "date": row.get::<_, Option<String>>(4)?,
        "due_date": row.get::<_, Option<String>>(5)?,
        "status": row.get::<_, Option<String>>(6)?,
        "language": row.get::<_, Option<String>>(7)?,
        "currency": row.get::<_, Option<String>>(8)?,
        "discount_type": row.get::<_, Option<String>>(9)?,
        "discount_value": row.get::<_, Option<i64>>(10)?,
        "notes": row.get::<_, Option<String>>(11)?,
        "subtotal_cents": row.get::<_, Option<i64>>(12)?,
        "vat_cents": row.get::<_, Option<i64>>(13)?,
        "total_cents": row.get::<_, Option<i64>>(14)?,
        "reference": row.get::<_, Option<String>>(15)?,
        "created_by": row.get::<_, Option<String>>(16)?,
        "created_at": row.get::<_, Option<String>>(17)?,
        "description": row.get::<_, Option<String>>(18)?,
        "delivery_date": row.get::<_, Option<String>>(19)?,
        "invoice_type": row.get::<_, Option<String>>(20)?,
        "entry_id": row.get::<_, Option<i64>>(22)?,
        "credit_for_invoice_id": row.get::<_, Option<i64>>(23)?,
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
        "SELECT id, invoice_id, date, amount_cents, method, reference, bank_tx_id, created_by FROM invoice_payments WHERE invoice_id = ?1 ORDER BY date, id"
    ).map_err(sql_err)?;
    let rows = stmt.query_map([invoice_id], |r| {
        Ok(json!({
            "id": r.get::<_, i64>(0)?, "invoice_id": r.get::<_, i64>(1)?, "date": r.get::<_, String>(2)?,
            "amount_cents": r.get::<_, i64>(3)?, "method": r.get::<_, String>(4)?,
            "reference": r.get::<_, Option<String>>(5)?, "bank_tx_id": r.get::<_, Option<i64>>(6)?,
            "created_by": r.get::<_, String>(7)?,
        }))
    }).map_err(sql_err)?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

pub fn get_invoice(db: &Connection, id: i64) -> Result<Option<Value>> {
    let inv = db.query_row("SELECT * FROM invoices WHERE id = ?1", [id], |r| {
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
    let mut sql = "SELECT * FROM invoices".to_string();
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
    let mut invoices: Vec<Value> = rows.filter_map(|r| r.ok()).collect();
    // Enrich each invoice
    for inv in invoices.iter_mut() {
        let iid = inv["id"].as_i64().unwrap_or(0);
        let lines = get_invoice_lines(db, iid)?;
        let payments = get_invoice_payments(db, iid)?;
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
        if inv["status"].as_str() == Some("sent") {
            if let Some(due) = inv["due_date"].as_str() {
                let today = crate::dates::today_iso();
                if due < today.as_str() {
                    inv["status"] = json!("overdue");
                }
            }
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
    let is_credit = invoice["invoice_type"].as_str() == Some("credit");
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
            "no invoice compliance rule set for this profile",
        ));
    }
    // Simplified: check supplier and customer party fields
    let company = db.query_row("SELECT * FROM company WHERE id = 1", [], |r| {
        Ok(json!({
            "name": r.get::<_, Option<String>>(1)?, "tax_id": r.get::<_, Option<String>>(5)?,
            "registration_id": r.get::<_, Option<String>>(2)?, "address": r.get::<_, Option<String>>(10)?,
            "postal_code": r.get::<_, Option<String>>(11)?, "city": r.get::<_, Option<String>>(12)?,
            "vat_module": r.get::<_, Option<i64>>(7)?,
        }))
    }).map_err(sql_err)?;

    let mut missing = Vec::new();
    if company["name"].as_str().is_none() {
        missing.push("company name");
    }
    let supplier_has_vat =
        company["vat_module"].as_i64() == Some(1) || company["tax_id"].as_str().is_some();
    if supplier_has_vat && company["tax_id"].as_str().is_none() {
        missing.push("tax id");
    }
    if company["registration_id"].as_str().is_none() {
        missing.push("registration number");
    }
    if company["address"].as_str().is_none() {
        missing.push("address");
    }
    if company["postal_code"].as_str().is_none() {
        missing.push("postal code");
    }
    if company["city"].as_str().is_none() {
        missing.push("city");
    }
    if !missing.is_empty() {
        return Err(invoice_error(
            "SUPPLIER_INCOMPLETE",
            format!(
                "supplier details missing: {} — set them with init/company update",
                missing.join(", ")
            ),
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
                "customer details missing: name, address and city are required",
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
                    "reverse-charge invoice: the customer VAT id is required",
                ));
            }
        }
    }
    Ok(())
}

pub fn create_invoice(
    db: &Connection,
    contact_id: i64,
    date: &str,
    due_days: Option<i64>,
    description: Option<&str>,
    reference: Option<&str>,
    notes: Option<&str>,
    discount_type: Option<&str>,
    discount_value: Option<i64>,
    lines_raw: &[Value],
    actor: &str,
    dry_run: bool,
) -> Result<Value> {
    let contact = get_contact(db, contact_id)?.ok_or_else(|| {
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

    if lines_raw.is_empty() {
        return Err(invoice_error("NO_LINES", "an invoice needs lines"));
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

    // Compute due date
    let due_date = due_days.map(|dd| {
        let y: i64 = date[..4].parse().unwrap_or(2026);
        let m: i64 = date[5..7].parse().unwrap_or(1);
        let d: i64 = date[8..10].parse().unwrap_or(1);
        let total_days = d + dd;
        // Simple date arithmetic (handles month overflow)
        let dt = chrono::NaiveDate::from_ymd_opt(y as i32, m as u32, d as u32)
            .unwrap_or(chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap());
        let dt = dt + chrono::Duration::days(dd);
        dt.format("%Y-%m-%d").to_string()
    });

    if dry_run {
        return Ok(json!({
            "action": "invoice.create", "contact_id": contact_id, "date": date,
            "due_days": due_days, "description": description, "reference": reference,
            "notes": notes, "discount_type": discount_type, "discount_value": discount_value,
            "discount_cents": totals["discount_cents"],
            "lines": parsed_lines, "net_cents": totals["net_cents"],
            "vat_cents": totals["vat_cents"], "gross_cents": totals["gross_cents"],
            "vat_breakdown": totals["breakdown"], "due_date": due_date, "dryRun": true,
        }));
    }

    // Insert invoice
    db.execute(
        "INSERT INTO invoices (contact_id, date, due_date, description, reference, notes, discount_type, discount_value, created_by) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        rusqlite::params![contact_id, date, due_date, description, reference, notes, discount_type, discount_value, actor],
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
                if e.message.contains("UNIQUE constraint failed") && attempts < 5 {
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

    let lines: Vec<Value> = original["lines"].as_array().cloned().unwrap_or_default();
    let contact_id = original["contact_id"].as_i64().unwrap_or(0);
    let credit = create_invoice(
        db,
        contact_id,
        credit_date,
        None,
        Some(reason.unwrap_or(&format!(
            "Credit note for {}",
            original["invoice_number"].as_str().unwrap_or("")
        ))),
        original["reference"].as_str(),
        None,
        original["discount_type"].as_str(),
        original["discount_value"].as_i64(),
        &lines,
        actor,
        false,
    )?;

    let credit_id = credit["id"].as_i64().unwrap_or(0);
    db.execute(
        "UPDATE invoices SET invoice_type = 'credit', credit_for_invoice_id = ?1 WHERE id = ?2",
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

    db.execute(
        "INSERT INTO invoice_payments (invoice_id, date, amount_cents, method, created_by) VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![id, date, amount_cents, method, actor],
    ).map_err(sql_err)?;

    if paid + amount_cents >= gross {
        db.execute("UPDATE invoices SET status = 'paid' WHERE id = ?1", [id])
            .map_err(sql_err)?;
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_line_basic() {
        let r = parse_line_spec("2x Consultancy @ 150.00 @21").unwrap();
        assert_eq!(r["qty_milli"], 2000);
        assert_eq!(r["description"], "Consultancy");
        assert_eq!(r["price_cents"], 15000);
        assert_eq!(r["vat_code"], "21");
    }

    #[test]
    fn parse_line_with_discount() {
        let r = parse_line_spec("1x Service @ 100.00 @9 @-10%").unwrap();
        assert_eq!(r["discount_type"], "pct");
        assert_eq!(r["discount_value"], 1000);
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
        assert_eq!(r["item_id"], 1);
        assert_eq!(r["qty_milli"], 2000);
        assert_eq!(r["price_cents"], 14000);
        assert_eq!(r["vat_code"], "21");
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
}
