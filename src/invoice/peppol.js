/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across thirty-one jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Peppol send (FR3.8): POST the UBL 2.1 / Peppol BIS 3.0 document to an
// access-point provider (e.g. Storecove-class). The provider contract is
// deliberately thin: POST the XML with a bearer token, expect 2xx.
// Configuration comes from the environment — never hard-code credentials:
//   BUKIO_PEPPOL_ENDPOINT  e.g. https://api.storecove.com/api/v2/invoices
//   BUKIO_PEPPOL_TOKEN     the provider API token

import { invoiceToUbl } from './ubl.js';

export function peppolError(code, message) {
  const e = new Error(message);
  e.code = code;
  return e;
}

export function peppolConfig() {
  return {
    endpoint: process.env.BUKIO_PEPPOL_ENDPOINT ?? null,
    token: process.env.BUKIO_PEPPOL_TOKEN ?? null,
  };
}

/**
 * Send a finalized invoice to the Peppol access point. dryRun validates the
 * configuration + payload without making the request.
 */
export async function sendPeppolInvoice(db, invoice, { endpoint = null, dryRun = false } = {}) {
  const cfg = peppolConfig();
  const ep = endpoint ?? cfg.endpoint;
  if (!ep) {
    throw peppolError('PEPPOL_NOT_CONFIGURED', 'no Peppol endpoint — set BUKIO_PEPPOL_ENDPOINT (or pass --endpoint)');
  }
  // Peppol BIS 3.0 BT-49: the buyer's electronic address (cbc:EndpointID) is
  // mandatory (1..1) — a buyer without a KVK number cannot receive a Peppol
  // document (the access point validates this). Fail early with a clear
  // message instead of a Schematron rejection at the provider.
  if (!invoice.contact?.kvk) {
    throw peppolError('PEPPOL_BUYER_MISSING_ID', `buyer '${invoice.contact?.name ?? 'unknown'}' has no KVK number — Peppol BIS 3.0 requires the buyer electronic address (BT-49); set it via 'contact update --id <id> --kvk <number>'`);
  }
  // Peppol BIS 3.0 BT-10 (PEPPOL-EN16931-R003, fatal; DE-R-015 for NL): the
  // buyer reference must be provided (there is no order-reference element in
  // this document). The invoice 'reference' field (klantkenmerk) is it.
  if (!invoice.reference) {
    throw peppolError('PEPPOL_BUYER_REFERENCE_MISSING', `invoice ${invoice.invoice_number} has no buyer reference — Peppol BIS 3.0 requires cbc:BuyerReference (BT-10); set it at creation with 'invoice create ... --reference <text>' or recreate the invoice`);
  }
  if (dryRun) {
    return {
      dryRun: true,
      invoice_number: invoice.invoice_number,
      endpoint: ep,
      configured: Boolean(cfg.token),
      bytes: invoiceToUbl(db, invoice).length,
    };
  }
  const xml = invoiceToUbl(db, invoice);
  let res;
  try {
    res = await fetch(ep, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/xml; charset=utf-8',
        ...(cfg.token ? { Authorization: `Bearer ${cfg.token}` } : {}),
      },
      body: xml,
      // never hang the CLI on a dead access point
      signal: AbortSignal.timeout(30_000),
    });
  } catch (err) {
    throw peppolError('PEPPOL_SEND_FAILED', `provider unreachable at ${ep}: ${err.message}`);
  }
  const text = await res.text();
  if (!res.ok) {
    throw peppolError('PEPPOL_SEND_FAILED', `provider returned ${res.status}: ${text.slice(0, 300)}`);
  }
  return {
    dryRun: false,
    invoice_number: invoice.invoice_number,
    endpoint: ep,
    status: res.status,
    response: text.slice(0, 500) || null,
  };
}
