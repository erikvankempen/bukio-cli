/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Invoicing module (Phase 3, FR3) — outgoing invoices compliant with the
// 12 verplichte factuurvereisten, lifecycle draft -> sent -> paid, credit
// notes, booking integration, UBL/PDF export hooks.
import { createAccount, getAccountByCode } from '../core/accounts.js';
import { createEntry, postEntry } from '../core/entries.js';
import { record } from '../audit/index.js';
import { resolveProfile, allTaxCodes } from '../jurisdictions/index.js';
import { isValidIban, normalizeIban } from '../core/iban.js';
import { isVatEnabled, listVatCodes } from '../vat/index.js';
import { getItem } from '../items/index.js';
import { parseBankAmount } from '../bank/csv.js';
import { formatAmount } from '../core/money.js';

export function invoiceError(code, message) {
  const e = new Error(message);
  e.code = code;
  return e;
}

// --- line spec parsing ----------------------------------------------------

const QTY_RE = /^(-?)(\d+(?:\.\d{1,3})?)\s*x\s+(.+)$/;
const DISC_RE = /^-(\d+(?:\.\d{1,2})?)(%?)$/;
// The 8 known VAT codes (src/vat/index.js VAT_CODES). A trailing token is a
// VAT code when it is one of these, or looks like a code (1-2 letters, or a
// 1-2 digit number like '9'/'21'); anything else — e.g. '100' or 'nope' — is
// the price, so price-only lines ("DESC @ 100") parse correctly while
// unknown codes ('@99') still fail validation with VAT_CODE_NOT_FOUND.
// union of all registered profiles' codes (validation against the ACTIVE
// profile's list still happens downstream — see VAT_CODE_NOT_FOUND).
// NOTE: a price-only line ending in a bare dotted rate ('Dienst @ 5.5')
// now tokenizes the rate as a VAT code and fails INVALID_LINE — write two
// decimals ('Dienst @ 5.50') or pass the code explicitly ('@ 5.50 @5.5').
const KNOWN_VAT_CODES = new Set(allTaxCodes());
function isVatCodeToken(token) {
  const t = token.toUpperCase();
  if (KNOWN_VAT_CODES.has(t)) return true;
  return /^[A-Z]{1,2}$/.test(t) || /^\d{1,2}$/.test(t);
}

/** Format a milli-quantity for humans: 2000 -> '2', 1500 -> '1.5'. */
export function formatQty(milli) {
  const n = milli / 1000;
  if (Number.isInteger(n)) return String(n);
  return n.toFixed(3).replace(/0+$/, '').replace(/\.$/, '');
}

/**
 * Parse a line spec: "[QTYx] DESCRIPTION @ PRICE [@ VATCODE] [@ -DISCOUNT]"
 * e.g. "2x Consultancy @ 150.00 @21" -> { qtyMilli: 2000, description: 'Consultancy',
 * priceCents: 15000, vatCode: '21' }; "1.5x Coaching @ 100.00 @9 @-10%" adds
 * { discountType: 'pct', discountValue: 1000 } (bp).
 * Splits from the right so descriptions may contain '@' (e.g. "Email @ adres").
 * Discounts are the LAST @-token and must start with '-': "-10%" (pct) or "-25.00".
 */
export function parseLineSpec(spec) {
  const s = String(spec).trim();
  const qtyMatch = s.match(QTY_RE);
  if (qtyMatch && qtyMatch[1] === '-') {
    throw invoiceError('INVALID_LINE', `line '${spec}': quantity must be a positive number`);
  }
  const qtyMilli = qtyMatch ? Math.round(parseFloat(qtyMatch[2]) * 1000) : 1000;
  if (!Number.isInteger(qtyMilli) || qtyMilli < 1) {
    throw invoiceError('INVALID_LINE', `line '${spec}': quantity must be a positive number`);
  }
  const rest = qtyMatch ? qtyMatch[3] : s;
  const parts = rest.split('@').map((p) => p.trim());
  let discountType = null;
  let discountValue = null;
  let last = parts[parts.length - 1];
  const dm = last.match(DISC_RE);
  if (dm) {
    discountType = dm[2] === '%' ? 'pct' : 'amount';
    discountValue = dm[2] === '%' ? Math.round(parseFloat(dm[1]) * 100) : parseBankAmount(dm[1]);
    parts.pop();
    last = parts[parts.length - 1];
  }
  let vatCode = null;
  let pricePart = last;
  if (isVatCodeToken(last)) {
    vatCode = last.toUpperCase();
    pricePart = parts[parts.length - 2];
  }
  const description = parts.slice(0, parts.length - (vatCode ? 2 : 1)).join('@').trim();
  const priceCents = parseBankAmount(pricePart);
  if (!description || !priceCents || priceCents <= 0) {
    throw invoiceError('INVALID_LINE', `line '${spec}' must be "[QTYx] DESCRIPTION @ PRICE [@ VATCODE] [@ -DISCOUNT]"`);
  }
  return { qtyMilli, qty: qtyMilli / 1000, description, priceCents, vatCode, discountType, discountValue };
}

/** Split a (possibly comma-separated) line-spec list into individual specs. */
export function splitLineSpecs(lines) {
  return lines.flatMap((spec) => (typeof spec === 'string'
    ? String(spec).split(',').map((s) => s.trim()).filter(Boolean)
    : [spec]));
}

/**
 * Parse an item spec: "ID[:QTY][@PRICE][@VATCODE][@-DISCOUNT]"
 * e.g. "1:2" (catalog price), "1:2@140.00", "1:1.5@140.00@21@-10%".
 * Price/VAT overrides apply to THIS invoice only — the catalog is untouched.
 */
export function parseItemSpec(spec) {
  const s = String(spec).trim();
  const [idPart, ...restParts] = s.split('@');
  const m = idPart.trim().match(/^(\d+)(?::(\d+(?:\.\d{1,3})?))?$/);
  if (!m) {
    throw invoiceError('INVALID_ITEM_SPEC', `item spec '${spec}' must be "ID[:QTY][@PRICE][@VATCODE][@-DISCOUNT]"`);
  }
  const itemId = parseInt(m[1], 10);
  const qtyMilli = m[2] ? Math.round(parseFloat(m[2]) * 1000) : 1000;
  if (!Number.isInteger(qtyMilli) || qtyMilli < 1) {
    throw invoiceError('INVALID_LINE', `item spec '${spec}': quantity must be a positive number`);
  }
  const parts = restParts.map((p) => p.trim()).filter((p) => p !== '');
  let discountType = null;
  let discountValue = null;
  let vatCode = null;
  let priceCents = null;
  if (parts.length) {
    let last = parts[parts.length - 1];
    const dm = last.match(DISC_RE);
    if (dm) {
      discountType = dm[2] === '%' ? 'pct' : 'amount';
      discountValue = dm[2] === '%' ? Math.round(parseFloat(dm[1]) * 100) : parseBankAmount(dm[1]);
      parts.pop();
      last = parts[parts.length - 1];
    }
    if (parts.length && isVatCodeToken(last)) {
      vatCode = last.toUpperCase();
      parts.pop();
      last = parts[parts.length - 1];
    }
    if (parts.length) priceCents = parseBankAmount(last);
  }
  return { itemId, qtyMilli, priceCents, vatCode, discountType, discountValue };
}

/** Split a (possibly comma-separated) item-spec list into individual specs. */
export function splitItemSpecs(items) {
  return items.flatMap((spec) => (typeof spec === 'string'
    ? String(spec).split(',').map((s) => s.trim()).filter(Boolean)
    : [spec]));
}

function assertLineDiscount(type, value) {
  if (type === null && value === null) return;
  if (type === 'pct') {
    if (!Number.isInteger(value) || value <= 0 || value > 10000) {
      throw invoiceError('INVALID_LINE_DISCOUNT', 'percentage discount must be in (0, 100]');
    }
  } else if (type === 'amount') {
    if (!Number.isInteger(value) || value <= 0) {
      throw invoiceError('INVALID_LINE_DISCOUNT', 'fixed discount must be positive cents');
    }
  } else {
    throw invoiceError('INVALID_LINE_DISCOUNT', `discount type '${type}' must be 'pct' or 'amount'`);
  }
}

/**
 * Validate a line-spec list without inserting anything (used by recurring
 * invoice templates at creation). Returns the parsed lines with vat_rate_bp.
 */
export function validateInvoiceLines(db, lines) {
  const vatOn = isVatEnabled(db);
  const vatCodes = new Map(listVatCodes(db).map((c) => [c.code, c]));
  return splitLineSpecs(lines).map((spec) => {
    const p = typeof spec === 'string' ? parseLineSpec(spec) : spec;
    if (!p.description || !p.priceCents) throw invoiceError('INVALID_LINE', `line '${JSON.stringify(spec)}' is not parseable`);
    if (!Number.isInteger(p.qtyMilli) || p.qtyMilli < 1) throw invoiceError('INVALID_LINE', `line '${JSON.stringify(spec)}': quantity must be a positive number`);
    assertLineDiscount(p.discountType, p.discountValue);
    if (p.vatCode) {
      if (!vatOn) throw invoiceError('VAT_MODULE_OFF', 'line has a VAT code but the VAT module is off for this company');
      if (!vatCodes.has(p.vatCode)) throw invoiceError('VAT_CODE_NOT_FOUND', `vat code '${p.vatCode}' does not exist`);
    }
    return { ...p, vat_rate_bp: p.vatCode ? vatCodes.get(p.vatCode).rate_bp : 0 };
  });
}

// --- discount + VAT allocation --------------------------------------------

/**
 * Allocate `total` cents across weights by the largest-remainder method:
 * every share is floored, then the remaining cents go to the largest
 * fractional parts (ties broken by index). Σ result == total exactly.
 */
export function allocateLargestRemainder(total, weights) {
  const n = weights.length;
  if (n === 0 || total === 0) return weights.map(() => 0);
  const wSum = weights.reduce((a, b) => a + b, 0);
  if (wSum <= 0) return weights.map(() => 0);
  const exact = weights.map((w) => (w * total) / wSum);
  const alloc = exact.map(Math.floor);
  let rest = total - alloc.reduce((a, b) => a + b, 0);
  const order = exact
    .map((e, i) => [i, e - alloc[i]])
    .sort((a, b) => (b[1] - a[1]) || (a[0] - b[0]));
  for (let k = 0; k < rest; k++) alloc[order[k % order.length][0]]++;
  return alloc;
}

/** Per-line discount in cents (single source of truth — shared by the UBL
 *  builder, the sales report and computeInvoiceTotals). */
export function lineDiscountCents(line) {
  if (line.discount_type === 'pct') return Math.round(line.amount_cents * line.discount_value / 10000);
  if (line.discount_type === 'amount') return Math.min(line.discount_value, line.amount_cents);
  return 0;
}

/**
 * Compute invoice totals from its lines + optional total discount. Pure and
 * deterministic — getInvoice derives net/vat/gross/breakdown from the SAME
 * computation createInvoice used, so there is exactly one source of truth.
 *
 * The total discount is allocated across VAT-code groups proportionally
 * (largest remainder) so each rate group's VAT base is reduced exactly, and
 * per-line VAT is redistributed within each group to sum to the group VAT —
 * the OB readout, jaarrekening, XAF and UBL all reconcile to the cent.
 *
 * Returns { lineNets, groups (by vat code), breakdown (by rate), net_cents,
 * vat_cents, gross_cents, discount_cents }.
 */
export function computeInvoiceTotals(lines, discountType = null, discountValue = null) {
  const lineNets = lines.map((l) => {
    const amount = l.amount_cents;
    const disc = lineDiscountCents(l);
    return { ...l, lineNet: amount - disc, lineDiscount: disc };
  });
  const netBefore = lineNets.reduce((s, l) => s + l.lineNet, 0);
  const discountCents = discountType === 'pct'
    ? Math.round(netBefore * discountValue / 10000)
    : discountType === 'amount' ? Math.min(discountValue, netBefore) : 0;

  // group lines by (vat code, rate, gl account) — the allocation unit
  const groupKey = (l) => `${l.vat_code ?? 'none'}|${l.vat_rate_bp}|${l.gl_account ?? ''}`;
  const groups = new Map();
  for (const l of lineNets) {
    const key = groupKey(l);
    if (!groups.has(key)) groups.set(key, { key, code: l.vat_code, rateBp: l.vat_rate_bp, gl: l.gl_account ?? null, net: 0, lines: [] });
    groups.get(key).net += l.lineNet;
    groups.get(key).lines.push(l);
  }
  const list = [...groups.values()].filter((g) => g.net !== 0);
  const alloc = allocateLargestRemainder(discountCents, list.map((g) => Math.max(0, g.net)));

  for (let i = 0; i < list.length; i++) {
    const g = list[i];
    g.discountedNet = g.net - alloc[i];
    g.vat = Math.round(Math.abs(g.discountedNet) * g.rateBp / 10000);
    // distribute the group VAT across its lines (largest remainder) so
    // Σ line vat == group vat exactly
    const lineShares = allocateLargestRemainder(g.vat, g.lines.map((l) => Math.max(0, l.lineNet)));
    g.lines.forEach((l, k) => { l.vatAmount = lineShares[k]; });
  }

  const net = list.reduce((s, g) => s + g.discountedNet, 0);
  const vat = list.reduce((s, g) => s + g.vat, 0);

  // per-rate breakdown for the invoice (only rates that actually charge VAT)
  const byRate = new Map();
  for (const g of list) {
    if (g.vat === 0) continue;
    const rk = String(g.rateBp);
    if (!byRate.has(rk)) byRate.set(rk, { rate_bp: g.rateBp, base_cents: 0, vat_cents: 0 });
    byRate.get(rk).base_cents += g.discountedNet;
    byRate.get(rk).vat_cents += g.vat;
  }
  const breakdown = [...byRate.values()].sort((a, b) => b.rate_bp - a.rate_bp);

  return { lineNets, groups: list, breakdown, net_cents: net, vat_cents: vat, gross_cents: net + vat, discount_cents: discountCents, net_before_cents: netBefore };
}

// --- contacts -------------------------------------------------------------

export function createContact(db, {
  name, address = null, postalCode = null, city = null, country = null,
  email = null, vatId = null, kvk = null, iban = null, actor = 'human', dryRun = false,
}) {
  if (!name || typeof name !== 'string' || !name.trim()) throw invoiceError('INVALID_NAME', 'contact needs a name');
  if (iban != null && !isValidIban(iban)) throw invoiceError('INVALID_IBAN', `'${iban}' is not a valid IBAN`);
  // a contact without an explicit country is assumed to be in the issuer's
  // market (NL contacts stay 'NL'; a LU company's customers default to 'LU'
  // — the old hardcoded 'NL' default put a wrong BT-40 country on
  // e-invoices in every non-NL market)
  const cc = country ?? resolveProfile(db).meta.country;
  // canonical storage form — same as updateContact and every consumer
  // (normalizeIban strips spaces AND dashes; a space-only strip kept dashes,
  // so create vs update stored the same IBAN in two different shapes)
  const cleanIban = iban != null ? normalizeIban(iban) : null;
  if (dryRun) {
    return {
      action: 'contact.create', name, address, postalCode, city, country: cc, email,
      vatId, kvk, iban: cleanIban, dryRun: true,
    };
  }
  const info = db.prepare(`
    INSERT INTO contacts (name, address, postal_code, city, country, email, vat_id, kvk, iban, created_by)
    VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
  `).run(name, address, postalCode, city, cc, email, vatId, kvk, cleanIban, actor);
  record(db, { actor, action: 'contact.create', command: 'contact add', args: { name, iban: iban ? true : false }, outcome: 'ok' });
  return getContact(db, info.lastInsertRowid);
}

export function updateContact(db, {
  id, name = null, address = null, postalCode = null, city = null, country = null,
  email = null, vatId = null, kvk = null, iban = null, actor = 'human', dryRun = false,
}) {
  const existing = getContact(db, id);
  if (!existing) throw invoiceError('CONTACT_NOT_FOUND', `contact ${id} does not exist`);
  const changes = {
    name: name ?? existing.name, address: address ?? existing.address,
    postalCode: postalCode ?? existing.postal_code, city: city ?? existing.city,
    country: country ?? existing.country, email: email ?? existing.email,
    vatId: vatId ?? existing.vat_id, kvk: kvk ?? existing.kvk,
    iban: iban != null ? normalizeIban(iban) : existing.iban,
  };
  if (changes.iban != null && !isValidIban(changes.iban)) throw invoiceError('INVALID_IBAN', `'${changes.iban}' is not a valid IBAN`);
  const before = { ...existing };
  if (dryRun) {
    return { action: 'contact.update', id, changes, before, dryRun: true };
  }
  db.prepare(`
    UPDATE contacts SET name = ?, address = ?, postal_code = ?, city = ?, country = ?, email = ?, vat_id = ?, kvk = ?, iban = ?
    WHERE id = ?
  `).run(changes.name, changes.address, changes.postalCode, changes.city, changes.country, changes.email, changes.vatId, changes.kvk, changes.iban ? String(changes.iban).replace(/\s+/g, '') : null, id);
  // audit which fields actually changed (all 9 — email/vat_id/kvk/country
  // used to be silently absent from the audit trail)
  const FIELD_MAP = [
    ['name', 'name'], ['address', 'address'], ['postal_code', 'postalCode'],
    ['city', 'city'], ['country', 'country'], ['email', 'email'],
    ['vat_id', 'vatId'], ['kvk', 'kvk'], ['iban', 'iban'],
  ];
  const changed = FIELD_MAP
    .filter(([dbKey, camelKey]) => changes[camelKey] !== before[dbKey])
    .map(([dbKey]) => dbKey);
  record(db, {
    actor, action: 'contact.update', command: 'contact update',
    args: { contact_id: id, changed },
    outcome: 'ok',
  });
  return getContact(db, id);
}

export function getContact(db, id) {
  return db.prepare('SELECT * FROM contacts WHERE id = ?').get(id) ?? null;
}

export function listContacts(db) {
  return db.prepare('SELECT * FROM contacts ORDER BY name').all();
}

// --- invoice helpers ------------------------------------------------------

function vatCodeRate(db, code) {
  if (!code) return 0;
  const row = db.prepare('SELECT rate_bp FROM vat_codes WHERE code = ?').get(code);
  return row ? row.rate_bp : 0;
}

export function getInvoice(db, id) {
  const inv = db.prepare('SELECT * FROM invoices WHERE id = ?').get(id);
  if (!inv) return null;
  inv.lines = db.prepare('SELECT * FROM invoice_lines WHERE invoice_id = ? ORDER BY line_no').all(id);
  inv.payments = db.prepare('SELECT * FROM invoice_payments WHERE invoice_id = ? ORDER BY date, id').all(id);
  inv.contact = getContact(db, inv.contact_id);
  const t = computeInvoiceTotals(inv.lines, inv.discount_type, inv.discount_value);
  inv.net_cents = t.net_cents;
  inv.vat_cents = t.vat_cents;
  inv.gross_cents = t.gross_cents;
  inv.discount_cents = t.discount_cents;
  inv.vat_breakdown = t.breakdown;
  inv.paid_cents = inv.payments.reduce((s, p) => s + p.amount_cents, 0);
  // derived status
  inv.status = inv.status === 'sent' && inv.due_date && inv.due_date < new Date().toISOString().slice(0, 10)
    ? 'overdue' : inv.status;
  return inv;
}

export function listInvoices(db, { status = null, type = null } = {}) {
  const clauses = [];
  const params = [];
  // 'overdue' is a DERIVED status (sent + past due) — never stored — so it
  // cannot be filtered in SQL; the rest filter on the stored status
  if (status && status !== 'overdue') { clauses.push('status = ?'); params.push(status); }
  if (type) { clauses.push('invoice_type = ?'); params.push(type); }
  const where = clauses.length ? `WHERE ${clauses.join(' AND ')}` : '';
  const rows = db.prepare(`SELECT * FROM invoices ${where} ORDER BY id DESC`).all(...params);
  let invoices = rows.map((r) => getInvoice(db, r.id));
  if (status === 'overdue') invoices = invoices.filter((i) => i.status === 'overdue');
  return invoices;
}

/**
 * Per-contact statement (opgave): finalized sales invoices + their payments +
 * payables, merged by date with a running balance. Positive balance = the
 * contact owes us; negative = we owe the contact. Read-only.
 */
export function contactStatement(db, { contactId, asOf = null }) {
  const contact = getContact(db, contactId);
  if (!contact) throw invoiceError('CONTACT_NOT_FOUND', `contact ${contactId} does not exist`);
  const asOfDate = asOf ?? new Date().toISOString().slice(0, 10);
  if (!/^\d{4}-\d{2}-\d{2}$/.test(asOfDate)) throw invoiceError('INVALID_DATE', `as-of '${asOfDate}' must be YYYY-MM-DD`);
  {
    const d = new Date(`${asOfDate}T00:00:00Z`);
    if (Number.isNaN(d.getTime()) || d.toISOString().slice(0, 10) !== asOfDate) {
      throw invoiceError('INVALID_DATE', `as-of '${asOfDate}' is not a valid calendar date`);
    }
  }

  const rows = [];
  const invoices = listInvoices(db).filter((i) =>
    i.contact_id === contactId && ['sales', 'credit'].includes(i.invoice_type)
    && i.status !== 'draft' && i.status !== 'void'
    && i.date <= asOfDate);
  for (const i of invoices) {
    // credit notes (invoice_type='credit') reduce what the contact owes:
    // their gross books as a credit row instead of a debit
    const isCredit = i.invoice_type === 'credit';
    rows.push({
      date: i.date, kind: isCredit ? 'credit' : 'invoice', ref: i.invoice_number ?? `#${i.id}`,
      description: isCredit ? `Credit note ${i.invoice_number ?? ''}`.trim() : (i.description ?? `Invoice ${i.invoice_number}`),
      debit_cents: isCredit ? 0 : i.gross_cents, credit_cents: isCredit ? i.gross_cents : 0, balance_cents: 0,
    });
    for (const p of i.payments.filter((x) => x.date <= asOfDate)) {
      // payments on a credit note are refunds we paid — reversed polarity
      rows.push({
        date: p.date, kind: 'payment', ref: i.invoice_number,
        description: isCredit ? `Refund ${i.invoice_number}` : `Payment ${i.invoice_number}`,
        debit_cents: isCredit ? p.amount_cents : 0, credit_cents: isCredit ? 0 : p.amount_cents, balance_cents: 0,
      });
    }
  }
  const payables = db.prepare(
    'SELECT id, invoice_ref, date, amount_cents FROM payables WHERE contact_id = ? AND date <= ? ORDER BY date, id',
  ).all(contactId, asOfDate);
  for (const p of payables) {
    rows.push({
      date: p.date, kind: 'payable', ref: p.invoice_ref,
      description: `Purchase invoice ${p.invoice_ref}`,
      debit_cents: 0, credit_cents: p.amount_cents, balance_cents: 0,
    });
  }

  rows.sort((a, b) => a.date.localeCompare(b.date) || a.kind.localeCompare(b.kind));
  let balance = 0;
  for (const r of rows) {
    balance += r.debit_cents - r.credit_cents;
    r.balance_cents = balance;
  }
  return { contact: { id: contact.id, name: contact.name }, as_of: asOfDate, rows, balance_cents: balance };
}

/**
 * Reminder candidates: sales invoices that are overdue or due within
 * `withinDays` days. Read-only. Returns reminders sorted most-overdue first.
 * The `--draft-emails` enrichment happens in the CLI (needs company data).
 */
export function invoiceReminders(db, { withinDays = 7 } = {}) {
  if (!Number.isInteger(withinDays) || withinDays < 0) {
    throw invoiceError('INVALID_WINDOW', `within-days must be a non-negative integer, got '${withinDays}'`);
  }
  const today = new Date().toISOString().slice(0, 10);
  // UTC day arithmetic — Date.now() + N*86400000 would drift across DST
  const now = new Date();
  const dueSoon = new Date(Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate() + withinDays)).toISOString().slice(0, 10);
  const reminders = listInvoices(db)
    .filter((i) => i.invoice_type === 'sales')
    .filter((i) => i.status === 'overdue' || (i.status === 'sent' && i.due_date && i.due_date <= dueSoon))
    .map((i) => {
      const daysOverdue = i.status === 'overdue' && i.due_date
        ? Math.max(0, Math.floor((Date.parse(today) - Date.parse(i.due_date)) / 86400000))
        : 0;
      return {
        invoice_id: i.id,
        invoice_number: i.invoice_number,
        contact_name: i.contact?.name ?? null,
        contact_email: i.contact?.email ?? null,
        due_date: i.due_date,
        days_overdue: daysOverdue,
        outstanding_cents: i.gross_cents - i.paid_cents,
        outstanding: formatAmount(i.gross_cents - i.paid_cents),
        gross: formatAmount(i.gross_cents),
        status: i.status,
        remind: i.status === 'overdue' ? 'overdue' : 'due_soon',
      };
    })
    .sort((a, b) => b.days_overdue - a.days_overdue || a.invoice_id - b.invoice_id);
  return { as_of: today, within_days: withinDays, count: reminders.length, reminders };
}

/** Normalize an object-style line (from credit notes / recurring templates). */
function normalizeLineObject(l) {
  return {
    qtyMilli: l.qtyMilli ?? (l.qty != null ? Math.round(l.qty * 1000) : 1000),
    description: l.description,
    priceCents: l.priceCents,
    vatCode: l.vatCode ?? null,
    discountType: l.discountType ?? null,
    discountValue: l.discountValue ?? null,
    glAccount: l.glAccount ?? null,
    unit: l.unit ?? null,
    itemId: l.itemId ?? null,
  };
}

/**
 * Create a draft invoice. Lines carry a VAT rate snapshot (vat_rate_bp) so
 * exports stay correct even if vat_codes change later. VAT codes are only
 * allowed when the VAT module is on.
 *
 * Lines come from `lines` (free-form line specs) OR `items` (catalog item
 * specs "ID[:QTY][@PRICE][@VATCODE][@-DISCOUNT]" — price/VAT overrides apply
 * to this invoice only). `discountType/discountValue` apply to the TOTAL,
 * before VAT. `language` is 'nl' or 'en'; when omitted it follows the
 * company profile (Dutch for NL/BE companies, English for every other
 * market) — no market is the de facto base.
 */
export function createInvoice(db, {
  contactId, lines = null, items = null, date, dueDays = 30, deliveryDate = null,
  description = null, reference = null, notes = null, discountType = null,
  discountValue = null, language = null, actor = 'human', dryRun = false,
}) {
  const contact = getContact(db, contactId);
  if (language == null) {
    // Document language follows the company profile, not a hardcoded market:
    // NL/BE (Dutch-speaking) default to 'nl', every other market to 'en'.
    const comp = db.prepare('SELECT locale FROM company WHERE id = 1').get();
    language = comp && comp.locale && comp.locale.startsWith('nl') ? 'nl' : 'en';
  }
  if (!contact) throw invoiceError('CONTACT_NOT_FOUND', `contact ${contactId} does not exist`);
  if (!/^\d{4}-\d{2}-\d{2}$/.test(date)) throw invoiceError('INVALID_DATE', `date '${date}' must be YYYY-MM-DD`);
  {
    const d = new Date(`${date}T00:00:00Z`);
    if (Number.isNaN(d.getTime()) || d.toISOString().slice(0, 10) !== date) {
      throw invoiceError('INVALID_DATE', `date '${date}' is not a valid calendar date`);
    }
  }
  const hasLines = Array.isArray(lines) ? lines.length > 0 : Boolean(lines);
  const hasItems = Array.isArray(items) ? items.length > 0 : Boolean(items);
  if (!hasLines && !hasItems) throw invoiceError('NO_LINES', 'an invoice needs lines (--lines) or items (--items)');
  if (hasLines && hasItems) throw invoiceError('CONFLICTING_LINES', 'pass either --lines or --items, not both');
  if (dueDays != null && (!Number.isInteger(dueDays) || dueDays < 0)) {
    throw invoiceError('INVALID_DUE_DAYS', `due-days must be a non-negative integer, got '${dueDays}'`);
  }
  if (deliveryDate != null) {
    if (!/^\d{4}-\d{2}-\d{2}$/.test(deliveryDate)) {
      throw invoiceError('INVALID_DATE', `delivery-date '${deliveryDate}' must be YYYY-MM-DD`);
    }
    const dd = new Date(`${deliveryDate}T00:00:00Z`);
    if (Number.isNaN(dd.getTime()) || dd.toISOString().slice(0, 10) !== deliveryDate) {
      throw invoiceError('INVALID_DATE', `delivery-date '${deliveryDate}' is not a valid calendar date`);
    }
  }
  if (!['nl', 'en'].includes(language)) {
    throw invoiceError('INVALID_LANGUAGE', "language must be 'nl' or 'en'");
  }
  if (discountType != null) assertLineDiscount(discountType, discountValue);

  const vatOn = isVatEnabled(db);
  const vatCodes = new Map(listVatCodes(db).map((c) => [c.code, c]));

  // resolve item specs against the catalog (snapshot price/VAT/unit/gl now)
  let rawLines;
  if (items) {
    rawLines = splitItemSpecs(items).map((spec) => {
      const p = typeof spec === 'string' ? parseItemSpec(spec) : spec;
      const item = getItem(db, p.itemId);
      if (!item) throw invoiceError('ITEM_NOT_FOUND', `item ${p.itemId} does not exist`);
      if (item.active !== 1) throw invoiceError('ITEM_INACTIVE', `item ${p.itemId} is deactivated`);
      const priceCents = p.priceCents ?? item.unit_price_cents;
      if (!Number.isInteger(priceCents) || priceCents <= 0) {
        throw invoiceError('INVALID_ITEM_OVERRIDE', `item spec '${JSON.stringify(spec)}': price override must be positive`);
      }
      const vatCode = p.vatCode ?? item.vat_code;
      return {
        description: item.description ?? item.name,
        qtyMilli: p.qtyMilli,
        priceCents,
        vatCode,
        discountType: p.discountType,
        discountValue: p.discountValue,
        unit: item.unit,
        itemId: item.id,
        glAccount: item.gl_account,
      };
    });
  } else {
    rawLines = splitLineSpecs(lines);
  }

  const parsedLines = rawLines.map((spec) => {
    const p = typeof spec === 'string' ? parseLineSpec(spec) : normalizeLineObject(spec);
    if (!p.description || !p.priceCents) throw invoiceError('INVALID_LINE', `line '${JSON.stringify(spec)}' is not parseable`);
    if (!Number.isInteger(p.qtyMilli) || p.qtyMilli < 1) throw invoiceError('INVALID_LINE', `line '${JSON.stringify(spec)}': quantity must be a positive number`);
    assertLineDiscount(p.discountType, p.discountValue);
    if (p.vatCode) {
      if (!vatOn) throw invoiceError('VAT_MODULE_OFF', 'line has a VAT code but the VAT module is off for this company');
      if (!vatCodes.has(p.vatCode)) throw invoiceError('VAT_CODE_NOT_FOUND', `vat code '${p.vatCode}' does not exist`);
    }
    const rateBp = p.vatCode ? vatCodes.get(p.vatCode).rate_bp : 0;
    const amountCents = Math.round(p.qtyMilli * p.priceCents / 1000);
    if (p.discountType === 'amount' && p.discountValue >= amountCents) {
      throw invoiceError('INVALID_LINE_DISCOUNT', `line '${JSON.stringify(spec)}': fixed discount must be less than the line amount`);
    }
    return {
      line_no: 0, description: p.description, quantity: p.qtyMilli,
      unit_price_cents: p.priceCents, vat_code: p.vatCode, vat_rate_bp: rateBp,
      amount_cents: amountCents, vat_amount_cents: 0,
      item_id: p.itemId ?? null, unit: p.unit ?? null, gl_account: p.glAccount ?? null,
      discount_type: p.discountType, discount_value: p.discountValue,
    };
  });

  // one source of truth: totals + per-rate VAT allocation
  const totals = computeInvoiceTotals(parsedLines, discountType, discountValue);
  totals.lineNets.forEach((ln, i) => { parsedLines[i].vat_amount_cents = ln.vatAmount ?? 0; });
  if (parsedLines.length === 0) throw invoiceError('NO_LINES', 'an invoice needs at least one line');

  const dueDate = dueDays != null
    ? new Date(Date.UTC(Number(date.slice(0, 4)), Number(date.slice(5, 7)) - 1, Number(date.slice(8, 10)) + dueDays)).toISOString().slice(0, 10)
    : null;

  if (dryRun) {
    // validate-everything-first, write nothing: all checks above already ran
    return {
      action: 'invoice.create', contact_id: contactId, date, due_days: dueDays,
      delivery_date: deliveryDate, description, reference, notes,
      language, discount_type: discountType, discount_value: discountValue,
      discount_cents: totals.discount_cents,
      lines: parsedLines.map((l) => ({
        qty: l.quantity / 1000, description: l.description, priceCents: l.unit_price_cents,
        vatCode: l.vat_code, discountType: l.discount_type, discountValue: l.discount_value,
      })),
      net_cents: totals.net_cents, vat_cents: totals.vat_cents, gross_cents: totals.gross_cents,
      vat_breakdown: totals.breakdown,
      due_date: dueDate, dryRun: true,
    };
  }

  const tx = db.transaction(() => {
    const info = db.prepare(`
      INSERT INTO invoices (contact_id, date, due_date, delivery_date, description, reference, notes, discount_type, discount_value, language, created_by)
      VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
    `).run(contactId, date, dueDate, deliveryDate, description, reference, notes,
      discountType, discountValue, language, actor);
    const invoiceId = info.lastInsertRowid;
    const insertLine = db.prepare(`
      INSERT INTO invoice_lines
        (invoice_id, line_no, description, quantity, unit_price_cents, vat_code, vat_rate_bp,
         amount_cents, vat_amount_cents, item_id, unit, gl_account, discount_type, discount_value)
      VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
    `);
    parsedLines.forEach((l, i) => {
      insertLine.run(invoiceId, i + 1, l.description, l.quantity, l.unit_price_cents,
        l.vat_code, l.vat_rate_bp, l.amount_cents, l.vat_amount_cents,
        l.item_id, l.unit, l.gl_account, l.discount_type, l.discount_value);
    });
    record(db, {
      actor, action: 'invoice.create', command: 'invoice create',
      args: {
        contactId, date, lines: parsedLines.length,
        net: totals.net_cents, discount: totals.discount_cents, language,
      },
      outcome: 'ok', invoiceId,
    });
    return invoiceId;
  });
  return getInvoice(db, tx());
}

// --- compliance (12 factuurvereisten) -------------------------------------

/**
 * Validate the 12 verplichte factuurvereisten (art. 35c/35d Wet OB + KVK).
 * Returns the list of missing vereisten; throws SUPPLIER_INCOMPLETE /
 * CUSTOMER_INCOMPLETE / CUSTOMER_VAT_REQUIRED when they matter.
 */
// Compliance rule-sets keyed by profile.documents.invoiceCompliance. NL is
// the only rule in Phase A ('nl-12-vereisten' — art. 35c/35d Wet OB + KVK).
const INVOICE_COMPLIANCE_RULES = {
  'nl-12-vereisten': validateNl12Vereisten,
  // Phase B6: Luxembourg invoice requirements (loi TVA + RCS). French error
  // messages, same error-code contract as the NL rule set.
  'lu-invoice-vereisten': validateLuVereisten,
};

export function validateCompliance(db, invoice) {
  const profile = resolveProfile(db);
  const rule = INVOICE_COMPLIANCE_RULES[profile.documents.invoiceCompliance];
  if (!rule) {
    throw invoiceError('FORMAT_NOT_SUPPORTED', `invoice compliance rule '${profile.documents.invoiceCompliance}' has no implementation (registered: ${Object.keys(INVOICE_COMPLIANCE_RULES).join(', ')})`);
  }
  return rule(db, invoice);
}

function validateNl12Vereisten(db, invoice) {
  const company = db.prepare('SELECT * FROM company WHERE id = 1').get();
  const contact = invoice.contact;

  const missingSupplier = [];
  // The supplier btw-id is a factuurvereiste only when the supplier HAS one
  // (art. 35a Wet OB) — a VAT-exempt business (vat module off, no btw-id)
  // must still be able to invoice.
  const supplierHasVat = company.vat_module === 1 || Boolean(company.tax_id);
  if (!company.name) missingSupplier.push('company name');
  if (supplierHasVat && !company.tax_id) missingSupplier.push('btw-id');
  if (!company.registration_id) missingSupplier.push('registration number');
  if (!company.address) missingSupplier.push('address');
  if (!company.postal_code) missingSupplier.push('postal code');
  if (!company.city) missingSupplier.push('city');
  if (missingSupplier.length) {
    throw invoiceError('SUPPLIER_INCOMPLETE', `supplier details missing (requirements 1-3): ${missingSupplier.join(', ')} — set them with init/company update`);
  }

  if (!contact.name || !contact.address || !contact.city) {
    throw invoiceError('CUSTOMER_INCOMPLETE', 'customer details missing (requirement 6): name, address and city are required');
  }

  const hasReverse = invoice.lines.some((l) => l.vat_code === 'R' || l.vat_code === 'RE');
  if (hasReverse && !contact.vat_id) {
    throw invoiceError('CUSTOMER_VAT_REQUIRED', 'reverse-charge invoice: the customer VAT id is required (requirement 7)');
  }

  return { ok: true, vereisten: 12 };
}

/**
 * Validate the Luxembourg invoice requirements (loi modifiée du 12 février
 * 1979 art. 66; RCS) — the checks implemented here cover the PARTY fields:
 * seller name/legal form/seat, "R.C.S. Luxembourg" + number, TVA number
 * (when the supplier is registered), customer name/address/city, and the
 * customer TVA id on reverse-charge lines. The content-level rules from the
 * law (issue date, sequential number, supply date, qty/nature, price excl.
 * VAT, exemption reason, auto-liquidation note) are enforced by the invoice
 * builder itself, not re-checked here. The autorisation d'établissement has
 * no schema field yet (documented, not validated).
 */
function validateLuVereisten(db, invoice) {
  const company = db.prepare('SELECT * FROM company WHERE id = 1').get();
  const contact = invoice.contact;

  const missingSupplier = [];
  // the supplier TVA number is a requirement only when the supplier is
  // registered (franchise en base: below €50K no TVA number exists)
  const supplierHasVat = company.vat_module === 1 || Boolean(company.tax_id);
  if (!company.name) missingSupplier.push('dénomination');
  if (!company.legal_form) missingSupplier.push('forme juridique');
  if (supplierHasVat && !company.tax_id) missingSupplier.push('numéro de TVA');
  if (!company.registration_id) missingSupplier.push('numéro RCS');
  if (!company.address) missingSupplier.push('adresse');
  if (!company.postal_code) missingSupplier.push('code postal');
  if (!company.city) missingSupplier.push('ville');
  if (missingSupplier.length) {
    throw invoiceError('SUPPLIER_INCOMPLETE', `données du fournisseur manquantes: ${missingSupplier.join(', ')} — définissez-les avec init/company update`);
  }

  if (!contact.name || !contact.address || !contact.city) {
    throw invoiceError('CUSTOMER_INCOMPLETE', 'données du client manquantes: nom, adresse et ville sont obligatoires');
  }

  const hasReverse = invoice.lines.some((l) => l.vat_code === 'R' || l.vat_code === 'RE');
  if (hasReverse && !contact.vat_id) {
    throw invoiceError('CUSTOMER_VAT_REQUIRED', 'auto-liquidation sur la facture: le numéro de TVA du client est obligatoire');
  }

  return { ok: true, vereisten: 12 };
}

// --- numbering + finalize -------------------------------------------------

export function nextInvoiceNumber(db, year) {
  const row = db.prepare(
    "SELECT MAX(CAST(SUBSTR(invoice_number, 6) AS INTEGER)) AS m FROM invoices WHERE invoice_number LIKE ?",
  ).get(`${year}-%`);
  return `${year}-${String((row?.m ?? 0) + 1).padStart(4, '0')}`;
}

/**
 * Posting defaults derived from the RESOLVED profile (Phase B6): the default
 * sales account (first income account of the default chart — NL 8000, LU
 * 7021), the VAT liability clearing account (tax.accounts.ledger — NL 2500,
 * LU 461411) and the debtors account (reporting.debtorsAccount — NL 1200,
 * LU 4011). These were hardcoded NL chart codes; a profile without an income
 * account or VAT liability fails loudly instead of posting to a nonexistent
 * account.
 */
function postingDefaults(db) {
  const profile = resolveProfile(db);
  const sales = profile.reporting.defaultChart.find((a) => a.type === 'income');
  if (!sales) {
    throw invoiceError('FORMAT_NOT_SUPPORTED', `profile ${profile.meta.country} declares no income account in its default chart — invoice postings need one`);
  }
  // the VAT liability is only required when the VAT module is enabled — a
  // no-VAT market (US: tax.system 'none') books plain sales vs debtors
  let vatLiabilityCode = null;
  if (isVatEnabled(db)) {
    const vatLiability = profile.tax.accounts.ledger.find((a) => a.type === 'liability');
    if (!vatLiability) {
      throw invoiceError('FORMAT_NOT_SUPPORTED', `profile ${profile.meta.country} declares no VAT liability clearing account — VAT-enabled invoice postings need one`);
    }
    vatLiabilityCode = vatLiability.code;
  }
  return { salesCode: sales.code, vatLiabilityCode, debtorsCode: profile.reporting.debtorsAccount };
}

/**
 * Build the booking postings for an invoice: Debiteuren vs Omzet + Te betalen
 * btw (VAT module on) or Debiteuren vs Omzet (module off). Credit notes use
 * the reversed signs. The VAT groups carry the discount-allocated nets and
 * per-rate VAT, so the books match the invoice exactly.
 */
export function buildInvoicePostings(db, invoice) {
  const pd = postingDefaults(db);
  const vatOn = isVatEnabled(db);
  const isCredit = invoice.invoice_type === 'credit';
  const sign = isCredit ? 1 : -1; // sales: omzet is credit (-), credit note: debit (+)
  const gross = invoice.gross_cents;
  const postings = [];

  if (vatOn) {
    const { groups } = computeInvoiceTotals(invoice.lines, invoice.discount_type, invoice.discount_value);
    for (const g of groups) {
      if (g.discountedNet === 0) continue;
      const gl = g.gl ?? pd.salesCode;
      if (g.code && g.vat > 0) {
        // btw-plichtige omzet: tagged posting so the OB readout picks it up
        postings.push({
          code: gl, amountCents: sign * g.discountedNet,
          vatCode: g.code, vatAmountCents: sign * g.vat,
        });
      } else if (g.code) {
        // 0% / vrijgesteld / verlegd (RE/R): no VAT on the books, but tagged
        // so the OB readout reports the base (1c for 0%/vrijgesteld, 2a for
        // verlegde EU leveringen) — zero vat amount
        postings.push({
          code: gl, amountCents: sign * g.discountedNet,
          vatCode: g.code, vatAmountCents: 0,
        });
      } else {
        // line without a VAT code: no VAT on the books
        postings.push({ code: gl, amountCents: sign * g.discountedNet });
      }
    }
    if (invoice.vat_cents > 0) {
      postings.push({ code: pd.vatLiabilityCode, amountCents: sign * invoice.vat_cents });
    }
  } else {
    // VAT module off: still honor per-line GL accounts (a line with
    // glAccount '8050' must land on 8050, not the 8000 default) — same
    // grouping as the VAT-on branch so both modes book identically
    const { groups } = computeInvoiceTotals(invoice.lines, invoice.discount_type, invoice.discount_value);
    for (const g of groups) {
      if (g.discountedNet === 0) continue;
      postings.push({ code: g.gl ?? pd.salesCode, amountCents: sign * g.discountedNet });
    }
  }
  // debiteuren leg: sales -> debit (+gross), credit note -> credit (-gross)
  postings.push({ code: pd.debtorsCode, amountCents: isCredit ? -gross : gross });
  return postings;
}

/** Finalize a draft invoice: assign the sequential number and book the entry. */
export function finalizeInvoice(db, { id, actor = 'human', dryRun = false }) {
  const invoice = getInvoice(db, id);
  if (!invoice) throw invoiceError('NOT_FOUND', `invoice ${id} does not exist`);
  if (invoice.status !== 'draft') throw invoiceError('ALREADY_FINALIZED', `invoice ${id} is already ${invoice.status}`);

  validateCompliance(db, invoice);
  const year = invoice.date.slice(0, 4);
  const postings = buildInvoicePostings(db, invoice);

  if (dryRun) {
    return {
      // read-only peek: the real run allocates INSIDE its transaction, so a
      // concurrent finalize may advance the sequence before this plan runs
      invoice_number: nextInvoiceNumber(db, year), postings,
      net: invoice.net_cents, vat: invoice.vat_cents, gross: invoice.gross_cents,
      dryRun: true,
    };
  }

  // The number is allocated INSIDE the transaction, atomically with the
  // booking: reading MAX before the tx would let two concurrent finalizes
  // (two processes on a WAL db) compute the same number — the loser's UPDATE
  // trips the UNIQUE constraint and surfaces a raw SQLite error instead of
  // just picking the next number. The retry loop absorbs that exact race:
  // a collision rolls the whole tx back (entry included), the next attempt
  // re-reads MAX (now including the winner's number) and books cleanly.
  let posted;
  let attempts = 0;
  for (;;) {
    try {
      posted = db.transaction(() => {
        const number = nextInvoiceNumber(db, year);
        const entry = createEntry(db, {
          date: invoice.date,
          description: `Invoice ${number}${invoice.contact ? ` - ${invoice.contact.name}` : ''}`,
          postings,
          source: 'invoice',
          sourceRef: `inv:${id}`,
          actor,
        });
        const p = postEntry(db, { id: entry.id, actor });
        db.prepare('UPDATE invoices SET invoice_number = ?, status = ?, entry_id = ? WHERE id = ?')
          .run(number, 'sent', p.id, id);
        record(db, {
          actor, action: 'invoice.finalize', command: 'invoice finalize',
          args: { id, invoice_number: number, gross: invoice.gross_cents }, outcome: 'ok', entryIds: [p.id],
        });
        return p;
      })();
      break;
    } catch (err) {
      const collision = String(err.message).includes('UNIQUE constraint failed: invoices.invoice_number');
      if (!collision || ++attempts >= 5) throw err;
    }
  }
  return { invoice: getInvoice(db, id), entry: posted, dryRun: false };
}

/** Create a credit note (draft) from a finalized sales invoice. */
export function creditInvoice(db, { id, date = null, reason = null, actor = 'human', dryRun = false }) {
  const original = getInvoice(db, id);
  if (!original) throw invoiceError('NOT_FOUND', `invoice ${id} does not exist`);
  if (original.invoice_type !== 'sales') throw invoiceError('NOT_SALES_INVOICE', 'only sales invoices can be credited');
  if (!['sent', 'paid', 'overdue'].includes(original.status)) throw invoiceError('NOT_FINALIZED', 'the invoice must be finalized before crediting');

  if (dryRun) {
    return {
      action: 'invoice.credit', for_invoice: id, reason,
      date: date ?? new Date().toISOString().slice(0, 10),
      description: reason ?? `Credit note for ${original.invoice_number}`,
      reference: original.reference ?? original.invoice_number, dryRun: true,
    };
  }

  const creditId = createInvoice(db, {
    contactId: original.contact_id,
    lines: original.lines.map((l) => ({
      qtyMilli: l.quantity, description: l.description,
      priceCents: l.unit_price_cents, vatCode: l.vat_code,
      discountType: l.discount_type, discountValue: l.discount_value,
      glAccount: l.gl_account, unit: l.unit, itemId: l.item_id,
    })),
    date: date ?? new Date().toISOString().slice(0, 10),
    dueDays: null,
    description: reason ?? `Credit note for ${original.invoice_number}`,
    // carry the buyer reference (klantkenmerk) so BT-10 on the credit note
    // matches the original — the preceding-invoice number for BT-25 is
    // derived in the UBL builder via credit_for_invoice_id
    reference: original.reference ?? original.invoice_number,
    discountType: original.discount_type, discountValue: original.discount_value,
    language: original.language ?? 'nl',
    actor,
  }).id;
  db.prepare("UPDATE invoices SET invoice_type = 'credit', credit_for_invoice_id = ? WHERE id = ?")
    .run(id, creditId);
  record(db, { actor, action: 'invoice.credit', command: 'invoice credit', args: { id, creditId }, outcome: 'ok' });
  return getInvoice(db, creditId);
}

// --- payments -------------------------------------------------------------

/**
 * Record a payment (tracking only — the posting comes from the bank flow).
 * When fully paid, the invoice status becomes 'paid'.
 */
export function markPaid(db, { id, date, amountCents, method = 'bank', bankTxId = null, actor = 'human', dryRun = false }) {
  const invoice = getInvoice(db, id);
  if (!invoice) throw invoiceError('NOT_FOUND', `invoice ${id} does not exist`);
  if (invoice.invoice_type === 'credit') throw invoiceError('CREDIT_NOT_PAYABLE', 'credit notes are not payable');
  if (!['sent', 'overdue'].includes(invoice.status)) throw invoiceError('NOT_PAYABLE', `invoice ${id} is ${invoice.status}`);
  if (!Number.isInteger(amountCents) || amountCents <= 0) throw invoiceError('INVALID_AMOUNT', 'payment amount must be positive cents');
  if (typeof date !== 'string' || !/^\d{4}-\d{2}-\d{2}$/.test(date)) {
    throw invoiceError('INVALID_DATE', `payment date '${date}' must be YYYY-MM-DD`);
  }
  {
    const d = new Date(`${date}T00:00:00Z`);
    if (Number.isNaN(d.getTime()) || d.toISOString().slice(0, 10) !== date) {
      throw invoiceError('INVALID_DATE', `payment date '${date}' is not a valid calendar date`);
    }
  }

  const remaining = invoice.gross_cents - invoice.paid_cents;
  if (amountCents > remaining) throw invoiceError('OVERPAYMENT', `payment ${amountCents} exceeds the outstanding ${remaining}`);

  if (dryRun) {
    return {
      action: 'invoice.pay', invoice_id: id, date, amount_cents: amountCents,
      method, remaining_cents: remaining, dryRun: true,
    };
  }

  // payment insert + status transition commit together: a crash between the
  // two would leave a recorded payment with the invoice still 'sent' — the
  // books would then disagree with the invoice status, and a re-run of the
  // same payment would be blocked by OVERPAYMENT with no clear explanation
  return db.transaction(() => {
    db.prepare('INSERT INTO invoice_payments (invoice_id, date, amount_cents, method, bank_tx_id, created_by) VALUES (?, ?, ?, ?, ?, ?)')
      .run(id, date, amountCents, method, bankTxId, actor);
    const paidNow = invoice.paid_cents + amountCents;
    if (paidNow >= invoice.gross_cents) {
      db.prepare("UPDATE invoices SET status = 'paid' WHERE id = ?").run(id);
    }
    record(db, { actor, action: 'invoice.pay', command: 'invoice pay', args: { id, amountCents }, outcome: 'ok' });
    return getInvoice(db, id);
  })();
}

/**
 * Ensure the FX-differences account exists (4840 Koersverschillen).
 * The default chart has it since 2026-08-07; databases seeded before that
 * get it created on demand, the same way year-end creates account 9900.
 * The creation is audited (chart mutations must be traceable).
 */
export function ensureFxDifferenceAccount(db, { actor = 'human' } = {}) {
  let account = getAccountByCode(db, '4840');
  if (!account) {
    account = createAccount(db, {
      code: '4840', name: 'Koersverschillen', type: 'expense',
      normalBalance: 'debit', taxonomyCode: 'WFBE.84',
    });
    record(db, {
      actor, action: 'account.create', command: 'bank match',
      args: { code: '4840', name: 'Koersverschillen' }, outcome: 'ok',
    });
  }
  return account;
}

/**
 * FX-match sanity bound shared by the autoMatch SQL (src/bank/index.js) and
 * paymentFromBank: a payment may differ from an invoice's outstanding by up to
 * 2% (floor 25 cents) and the gap is booked to 4840 Koersverschillen. Keep the
 * two consumers in sync — drift would make autoMatch match what paymentFromBank
 * then rejects (FX_DIFFERENCE_TOO_LARGE).
 */
export const FX_MATCH_TOLERANCE_BP = 200; // 2%
export const FX_MATCH_FLOOR_CENTS = 25;   // small-invoice absolute floor

/**
 * Apply a bank payment to an invoice: record the payment, post the bank entry
 * (Bank / Debiteuren) and reconcile the transaction. Used by the bank
 * auto-match engine.
 *
 * An invoice booked from a foreign-currency price is translated to EUR at the
 * invoice date; the incoming bank transfer is converted at the PAYMENT date,
 * so the received amount can differ. When the difference is within the sanity
 * bound (fxToleranceBp, default 200bp = 2%, floor 25 cents) it is booked to
 * 4840 Koersverschillen and the invoice settles in full. Beyond the bound it
 * is rejected — a large difference is not an FX move, it is a wrong amount.
 * The whole flow is one transaction: payment, entry, reconciliation and state
 * update all-or-nothing (a half-written payment with no entry would double-pay
 * on re-match).
 */
export function paymentFromBank(db, { invoiceId, bankTxId, actor = 'human', fxToleranceBp = FX_MATCH_TOLERANCE_BP }) {
  return db.transaction(() => {
    const invoice = getInvoice(db, invoiceId);
    if (!invoice) throw invoiceError('NOT_FOUND', `invoice ${invoiceId} does not exist`);
    const txRow = db.prepare('SELECT * FROM bank_transactions WHERE id = ?').get(bankTxId);
    if (!txRow) throw invoiceError('NOT_FOUND', `bank transaction ${bankTxId} does not exist`);
    if (txRow.state !== 'unmatched') throw invoiceError('ALREADY_MATCHED', `bank transaction ${bankTxId} is already ${txRow.state}`);
    const bankAccount = db.prepare('SELECT * FROM bank_accounts WHERE id = ?').get(txRow.bank_account_id);
    if (!bankAccount?.account_code) {
      throw invoiceError('ACCOUNT_NOT_FOUND', `bank account ${txRow.bank_account_id} has no ledger account — set one before matching`);
    }

    const outstanding = invoice.gross_cents - invoice.paid_cents;
    const delta = txRow.amount_cents - outstanding; // + paid more (FX gain), - paid less (FX loss)
    let fxCents = 0;
    if (delta !== 0) {
      // floor 25 cents: enough to absorb cent-level rounding on tiny invoices,
      // small enough that a €10 invoice paid €9 (10% off) is still rejected
      const tolerance = Math.max(Math.round((outstanding * fxToleranceBp) / 10000), 25);
      if (Math.abs(delta) > tolerance) {
        throw invoiceError(
          'FX_DIFFERENCE_TOO_LARGE',
          `payment ${txRow.amount_cents} differs from the outstanding ${outstanding} by ${delta} cents — beyond the ${fxToleranceBp}bp sanity bound; check the amount before booking`,
        );
      }
      fxCents = delta;
    }

    // settle the invoice in full — the FX difference absorbs the cent-level gap
    const paid = markPaid(db, { id: invoiceId, date: txRow.date, amountCents: outstanding, method: 'bank', bankTxId, actor });
    const postings = [
      { code: bankAccount.account_code, amountCents: txRow.amount_cents },
      { code: postingDefaults(db).debtorsCode, amountCents: -outstanding },
    ];
    let description = `Payment ${invoice.invoice_number ?? invoiceId}${invoice.contact ? ` - ${invoice.contact.name}` : ''}`;
    if (fxCents !== 0) {
      const fxAccount = ensureFxDifferenceAccount(db, { actor });
      postings.push({ code: fxAccount.code, amountCents: -fxCents });
      description += ` (fx difference ${formatAmount(fxCents)})`;
    }
    const entry = createEntry(db, {
      date: txRow.date,
      description,
      postings,
      source: 'bank',
      sourceRef: `tx:${bankTxId}`,
      actor,
    });
    const posted = postEntry(db, { id: entry.id, actor });
    db.prepare(`
      INSERT INTO reconciliations (bank_tx_id, target_type, target_id, method, confidence, created_by)
      VALUES (?, 'invoice', ?, ?, 1.0, ?)
    `).run(bankTxId, invoiceId, actor.startsWith('agent') ? 'agent' : 'manual', actor);
    db.prepare("UPDATE bank_transactions SET state = 'matched' WHERE id = ?").run(bankTxId);
    record(db, {
      actor, action: 'invoice.payment_bank', command: 'bank match',
      args: { invoiceId, bankTxId }, outcome: 'ok', entryIds: [posted.id],
    });
    return { invoice: paid, entry: posted };
  })();
}
