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
  const res = await fetch(ep, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/xml; charset=utf-8',
      ...(cfg.token ? { Authorization: `Bearer ${cfg.token}` } : {}),
    },
    body: xml,
  });
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
