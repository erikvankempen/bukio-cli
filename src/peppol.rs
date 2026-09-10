// bukio-cli — agent-first double-entry bookkeeping for SMEs.
// Copyright (c) 2026 Erik van Kempen.
// SPDX-License-Identifier: Apache-2.0
//
// Peppol send (FR3.8): POST the UBL 2.1 / Peppol BIS 3.0 document to an
// access-point provider. The provider contract is deliberately thin: POST the
// XML with a bearer token, expect 2xx. Configuration comes from the
// environment — never hard-code credentials:
//   BUKIO_PEPPOL_ENDPOINT  e.g. https://api.storecove.com/api/v2/invoices
//   BUKIO_PEPPOL_TOKEN     the provider API token

use crate::money::{BukioError, Result};
use crate::ubl::invoice_to_ubl;
use rusqlite::Connection;
use serde_json::{json, Value};
use std::time::Duration;

fn peppol_error(code: &'static str, message: impl Into<String>) -> BukioError {
    BukioError::new(code, message)
}

/// (endpoint, token) from the environment.
pub fn peppol_config() -> (Option<String>, Option<String>) {
    (
        std::env::var("BUKIO_PEPPOL_ENDPOINT").ok(),
        std::env::var("BUKIO_PEPPOL_TOKEN").ok(),
    )
}

fn truncate(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// Send a finalized invoice to the Peppol access point. dry_run validates the
/// configuration + payload without making the request.
pub fn send_peppol_invoice(
    db: &Connection,
    invoice: &Value,
    endpoint: Option<&str>,
    dry_run: bool,
) -> Result<Value> {
    let (cfg_endpoint, cfg_token) = peppol_config();
    let ep = match endpoint.or(cfg_endpoint.as_deref()) {
        Some(e) => e.to_string(),
        None => {
            return Err(peppol_error(
                "PEPPOL_NOT_CONFIGURED",
                "no Peppol endpoint — set BUKIO_PEPPOL_ENDPOINT (or pass --endpoint)",
            ))
        }
    };
    // Peppol BIS 3.0 BT-49: the buyer's electronic address (cbc:EndpointID) is
    // mandatory — a buyer without a KVK number cannot receive a Peppol
    // document. Fail early instead of taking a Schematron rejection later.
    if invoice["contact"]["kvk"].as_str().unwrap_or("").is_empty() {
        let name = invoice["contact"]["name"].as_str().unwrap_or("unknown");
        return Err(peppol_error(
            "PEPPOL_BUYER_MISSING_ID",
            format!("buyer '{name}' has no KVK number — Peppol BIS 3.0 requires the buyer electronic address (BT-49); set it via 'contact update --id <id> --kvk <number>'"),
        ));
    }
    // Peppol BIS 3.0 BT-10 (PEPPOL-EN16931-R003, fatal): the buyer reference is
    // required. The invoice `reference` field (klantkenmerk) is it.
    if invoice["reference"].as_str().unwrap_or("").is_empty() {
        return Err(peppol_error(
            "PEPPOL_BUYER_REFERENCE_MISSING",
            format!(
                "invoice {} has no buyer reference — Peppol BIS 3.0 requires cbc:BuyerReference (BT-10); set it at creation with 'invoice create ... --reference <text>' or recreate the invoice",
                invoice["invoice_number"].as_str().unwrap_or("")
            ),
        ));
    }
    let xml = invoice_to_ubl(db, invoice)?;
    let invoice_number = invoice["invoice_number"].clone();
    if dry_run {
        return Ok(json!({
            "dryRun": true,
            "invoice_number": invoice_number,
            "endpoint": ep,
            "configured": cfg_token.is_some(),
            "bytes": xml.chars().count(),
        }));
    }
    let config = ureq::config::Config::builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(30)))
        .build();
    let agent = ureq::Agent::new_with_config(config);
    let mut req = agent
        .post(&ep)
        .header("Content-Type", "application/xml; charset=utf-8");
    if let Some(ref tok) = cfg_token {
        req = req.header("Authorization", &format!("Bearer {tok}"));
    }
    let resp = req.send(xml.as_bytes()).map_err(|e| {
        peppol_error(
            "PEPPOL_SEND_FAILED",
            format!("provider unreachable at {ep}: {e}"),
        )
    })?;
    let status = resp.status().as_u16();
    let text = resp.into_body().read_to_string().map_err(|e| {
        peppol_error(
            "PEPPOL_SEND_FAILED",
            format!("failed to read provider response: {e}"),
        )
    })?;
    if !(200..300).contains(&status) {
        return Err(peppol_error(
            "PEPPOL_SEND_FAILED",
            format!("provider returned {status}: {}", truncate(&text, 300)),
        ));
    }
    Ok(json!({
        "dryRun": false,
        "invoice_number": invoice_number,
        "endpoint": ep,
        "status": status,
        "response": if text.is_empty() { Value::Null } else { json!(truncate(&text, 500)) },
    }))
}
