/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Inbound e-invoice intake (EN 16931 / Peppol BIS 3.0 UBL 2.1 invoices) into
// the payables register — the receive-half of the 2027 e-invoicing mandate.
// Same contract as every importer: validate the WHOLE document first (all
// problems collected, nothing written), idempotent by
// source_ref '<supplier-key>:<invoice-number>', one transaction, audited.
//
// Scope note: registers a PAYABLE only (amount + dates + supplier + VAT
// breakdown). GL mapping and the actual booking stay with the agent/human
// booking workflow — an UBL has no account codes. Credit notes (type 381)
// are rejected in v1: payables are positive-only by schema.
import { XMLParser } from 'fast-xml-parser';
import { createContact, listContacts } from '../invoice/index.js';
import { record } from '../audit/index.js';
import { importError, parseImportAmount } from './index.js';

const DATE_RE = /^\d{4}-\d{2}-\d{2}$/;
const UBL_NS = 'urn:oasis:names:specification:ubl:schema:xsd:Invoice-2';

function validDate(s) {
  if (typeof s !== 'string' || !DATE_RE.test(s)) return false;
  const d = new Date(`${s}T00:00:00Z`);
  return !Number.isNaN(d.getTime()) && d.toISOString().slice(0, 10) === s;
}

function addDays(date, days) {
  const d = new Date(`${date}T00:00:00Z`);
  d.setUTCDate(d.getUTCDate() + days);
  return d.toISOString().slice(0, 10);
}

function asArray(x) {
  if (x == null) return [];
  return Array.isArray(x) ? x : [x];
}

function normalizeName(name) {
  return String(name).toLowerCase().replace(/[^a-z0-9]/g, '');
}

/** Walk a nested path like ['cac:PartyName','cbc:Name'] → trimmed string. */
function pickPath(obj, path) {
  let cur = obj;
  for (const k of path) {
    if (cur == null) return '';
    cur = cur[k];
  }
  return cur == null ? '' : String(cur).trim();
}

/** First non-empty value among several paths (for optional supplier fields). */
function pick(obj, ...paths) {
  for (const p of paths) {
    const v = pickPath(obj, p);
    if (v) return v;
  }
  return '';
}

/**
 * Import one EN 16931/Peppol UBL invoice as a payable.
 * contact: explicit contact id (must exist). Else auto-match by vat-id, then
 * normalized name; createMissing creates the supplier contact from the file.
 * Returns { imported, duplicates, contacts_created, supplier, invoice_ref,
 * date, due_date, amount_cents, vat_by_rate, contact }.
 */
export function importUblInvoice(db, {
  xmlText, contact = null, createMissing = false, actor = 'human', dryRun = false,
}) {
  let doc;
  try {
    doc = new XMLParser({ parseTagValue: false, trimValues: true }).parse(xmlText);
  } catch (err) {
    throw importError('INVALID_UBL_INVOICE', `cannot parse XML: ${err.message}`);
  }
  const inv = doc.Invoice;
  if (!inv) {
    throw importError('INVALID_UBL_INVOICE', `root element must be <Invoice> in the UBL namespace (${UBL_NS}) — this is not an EN 16931 invoice`);
  }

  const errors = [];

  // --- identity -------------------------------------------------------------
  const invoiceRef = pick(inv, ['cbc:ID']);
  if (!invoiceRef) errors.push({ line: 0, error: 'INVALID_UBL_INVOICE: cbc:ID (invoice number) is missing' });

  const typeCode = pick(inv, ['cbc:InvoiceTypeCode']);
  // EN 16931 BT-3 is mandatory (1..1) — an absent type code is a malformed
  // document, not an implicit 380
  if (!typeCode) {
    throw importError('INVALID_UBL_INVOICE', 'cbc:InvoiceTypeCode is missing (EN 16931 BT-3)');
  }
  if (typeCode !== '380') {
    throw importError('UNSUPPORTED_UBL_DOCUMENT', `InvoiceTypeCode '${typeCode}' is not supported (380 = invoice; credit notes 381 are not imported yet)`);
  }

  // --- dates ----------------------------------------------------------------
  const issueDate = pick(inv, ['cbc:IssueDate']);
  const dueDate = pick(inv, ['cbc:DueDate']);
  if (!issueDate || !validDate(issueDate)) {
    errors.push({ line: 0, error: `INVALID_DATE: cbc:IssueDate '${issueDate}' must be yyyy-mm-dd` });
  }
  if (dueDate && !validDate(dueDate)) {
    errors.push({ line: 0, error: `INVALID_DATE: cbc:DueDate '${dueDate}' must be yyyy-mm-dd` });
  }

  // --- supplier -------------------------------------------------------------
  const sup = inv['cac:AccountingSupplierParty']?.['cac:Party'];
  const supName = pick(sup, ['cac:PartyName', 'cbc:Name'], ['cac:PartyLegalEntity', 'cbc:RegistrationName']);
  // The supplier's VAT number lives in PartyTaxScheme/cbc:CompanyID (the
  // outgoing UBL writes it there too). PartyTaxScheme/TaxScheme/cbc:ID is
  // the tax SCHEME identifier — always the literal 'VAT' — and reading it
  // here would store 'VAT' as every supplier's vat_id, collapsing the
  // idempotency key (vat:<invoiceRef>) and the vat-id contact match
  // across all vendors into one.
  const vatId = (() => {
    // pickPath stringifies — grab the PartyTaxScheme OBJECT(s) directly. UBL
    // allows 0..n PartyTaxScheme (e.g. a German supplier with a local
    // Steuernummer AND a USt-IdNr); fast-xml-parser yields an ARRAY for
    // repeats, and pickPath on an array returns undefined. Take the entry
    // whose TaxScheme/cbc:ID is 'VAT' (the tax SCHEME identifier — the
    // literal 'VAT', not the number) and read its CompanyID.
    const schemes = asArray(sup?.['cac:PartyTaxScheme']);
    if (schemes.length === 0) return null;
    // prefer the scheme explicitly marked VAT; fall back to a lone scheme
    // with NO TaxScheme id (lenient real-world shape), but never trust a
    // scheme carrying a different tax identifier (e.g. a German Steuernummer)
    const target = schemes.find((s) => {
      const schemeId = pickPath(s, ['cac:TaxScheme', 'cbc:ID']);
      return schemeId && schemeId.toUpperCase() === 'VAT';
    }) ?? (schemes.length === 1 && !pickPath(schemes[0], ['cac:TaxScheme', 'cbc:ID']) ? schemes[0] : null);
    if (!target) return null;
    const id = pickPath(target, ['cbc:CompanyID']);
    return id || null;
  })();
  const email = pick(sup, ['cac:Contact', 'cbc:ElectronicMail']) || null;
  const street = pick(sup, ['cac:PostalAddress', 'cbc:StreetName']) || null;
  const city = pick(sup, ['cac:PostalAddress', 'cbc:CityName']) || null;
  const postal = pick(sup, ['cac:PostalAddress', 'cbc:PostalZone']) || null;
  const countryRaw = pick(sup, ['cac:PostalAddress', 'cac:Country', 'cbc:IdentificationCode']);
  const country = (countryRaw || 'NL').toUpperCase();
  if (!supName) {
    errors.push({ line: 0, error: 'INVALID_UBL_INVOICE: supplier name missing (PartyName/Name or PartyLegalEntity/RegistrationName)' });
  }

  // --- monetary totals ------------------------------------------------------
  const totals = inv['cac:LegalMonetaryTotal'];
  const currencyRaw = inv['cbc:DocumentCurrencyCode'];
  // EN 16931 BT-5 is mandatory (1..1) — an absent element is a malformed
  // document, not an implicit EUR (a missing currency on a non-EUR invoice
  // would silently create a payable in the wrong currency)
  if (!currencyRaw) {
    errors.push({ line: 0, error: 'INVALID_UBL_INVOICE: cbc:DocumentCurrencyCode is missing (EN 16931 BT-5)' });
  }
  const currency = currencyRaw ? String(currencyRaw).trim().toUpperCase() : 'EUR';
  if (currency !== 'EUR') {
    errors.push({ line: 0, error: `INVALID_UBL_INVOICE: cbc:DocumentCurrencyCode '${currency}' — only EUR invoices can be imported (payables are EUR-only)` });
  }
  const payableRaw = pick(totals, ['cbc:PayableAmount']);
  let payableCents = null;
  if (!payableRaw) {
    errors.push({ line: 0, error: 'INVALID_UBL_INVOICE: cbc:PayableAmount is missing' });
  } else {
    payableCents = parseImportAmount(payableRaw);
    if (!Number.isInteger(payableCents) || payableCents <= 0) {
      errors.push({ line: 0, error: `INVALID_AMOUNT: cbc:PayableAmount '${payableRaw}' must be a positive amount` });
    }
  }

  // VAT breakdown per rate (informational — no VAT legs are booked)
  const vatByRate = {};
  const subtotals = asArray(inv['cac:TaxTotal']?.['cac:TaxSubtotal']);
  for (const st of subtotals) {
    const pct = pick(st, ['cac:TaxCategory', 'cbc:Percent']);
    const taxRaw = pick(st, ['cbc:TaxAmount']);
    const taxCents = taxRaw ? parseImportAmount(taxRaw) : null;
    if (pct && Number.isInteger(taxCents)) vatByRate[pct] = (vatByRate[pct] ?? 0) + taxCents;
  }

  if (errors.length > 0) {
    throw importError('IMPORT_VALIDATION_FAILED', 'UBL invoice failed validation — nothing was imported', errors);
  }

  // --- idempotency ----------------------------------------------------------
  const supplierKey = vatId ? vatId.toLowerCase() : normalizeName(supName);
  const sourceRef = `${supplierKey}:${invoiceRef}`;
  const existingRefs = new Set(
    db.prepare("SELECT source_ref FROM payables WHERE source = 'ubl'").all().map((r) => r.source_ref),
  );

  // --- contact resolution ---------------------------------------------------
  let resolved = null;
  let contactCreated = false;
  if (contact != null) {
    const byId = listContacts(db).find((c) => c.id === Number(contact));
    if (!byId) throw importError('CONTACT_NOT_FOUND', `contact ${contact} does not exist`);
    resolved = byId;
  } else {
    const all = listContacts(db);
    if (vatId) {
      resolved = all.find((c) => c.vat_id && c.vat_id.toLowerCase() === vatId.toLowerCase());
    }
    if (!resolved) {
      const want = normalizeName(supName);
      resolved = all.find((c) => normalizeName(c.name) === want);
    }
    if (!resolved && createMissing) {
      contactCreated = true;
      if (!dryRun) {
        // dry-run must not write: the plan below names the would-be contact
        resolved = createContact(db, {
          name: supName, address: street, postalCode: postal, city, country,
          email, vatId, actor,
        });
      }
    }
    if (!resolved && !(createMissing && dryRun)) {
      throw importError('CONTACT_NOT_FOUND', `no contact matches supplier '${supName}' — pass --contact <id> or --create-missing to create it`);
    }
  }

  const finalDue = dueDate || addDays(issueDate, 30);

  if (dryRun) {
    return {
      action: 'import.invoice', file: null, supplier: supName, vat_id: vatId,
      invoice_ref: invoiceRef, date: issueDate, due_date: finalDue,
      amount_cents: payableCents, vat_by_rate: vatByRate,
      contact: { id: resolved?.id ?? null, name: resolved?.name ?? supName, created: contactCreated },
      dryRun: true,
    };
  }

  let imported = 0;
  let duplicates = 0;
  if (existingRefs.has(sourceRef)) {
    duplicates += 1;
  } else {
    db.transaction(() => {
      const info = db.prepare(
        `INSERT INTO payables (contact_id, invoice_ref, date, due_date, amount_cents, payment_method, source, source_ref, created_by)
         VALUES (?, ?, ?, ?, ?, 'transfer', 'ubl', ?, ?)`,
      ).run(resolved.id, invoiceRef, issueDate, finalDue, payableCents, sourceRef, actor);
      record(db, {
        actor, action: 'import.invoice', command: 'import invoice',
        args: {
          payable_id: Number(info.lastInsertRowid), supplier: supName, vat_id: vatId,
          invoice_ref: invoiceRef, date: issueDate, due_date: finalDue,
          amount_cents: payableCents, vat_by_rate: vatByRate, contact_id: resolved.id,
        },
        outcome: 'ok',
      });
    })();
    existingRefs.add(sourceRef);
    imported += 1;
  }

  return {
    imported, duplicates, contacts_created: contactCreated ? 1 : 0,
    supplier: supName, vat_id: vatId, invoice_ref: invoiceRef,
    date: issueDate, due_date: finalDue, amount_cents: payableCents,
    vat_by_rate: vatByRate, contact: { id: resolved.id, name: resolved.name },
    dryRun: false,
  };
}
