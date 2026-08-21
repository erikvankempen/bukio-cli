/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across thirty-one jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Open-items aging: outstanding debtor invoices and unpaid payables per
// contact, bucketed by days past due (current / 30 / 60 / 90 / 90+).
// Read-only. `report aging` is the dunning and jaarrekening-notes tool.
import { listInvoices } from '../invoice/index.js';
import { validateLabeledDate, todayIso } from '../core/dates.js';

export function reportError(code, message) {
  const e = new Error(message);
  e.code = code;
  return e;
}


function bucketFor(daysPastDue) {
  if (daysPastDue <= 0) return 'current';
  if (daysPastDue <= 30) return 'd30';
  if (daysPastDue <= 60) return 'd60';
  if (daysPastDue <= 90) return 'd90';
  return 'd90plus';
}

const EMPTY_BUCKETS = { current: 0, d30: 0, d60: 0, d90: 0, d90plus: 0 };

function emptyTotals() {
  return { ...EMPTY_BUCKETS, total_cents: 0 };
}

/** Outstanding sales invoices (sent/overdue, not fully paid) per contact. */
function debtorsAging(db, asOf) {
  const byContact = new Map();
  const invoices = listInvoices(db)
    .filter((i) => i.invoice_type === 'sales' && (i.status === 'sent' || i.status === 'overdue'))
    // an as-of dated report must not show invoices issued AFTER that date
    .filter((i) => i.date <= asOf);

  for (const i of invoices) {
    const outstanding = i.gross_cents - i.paid_cents;
    if (outstanding <= 0) continue;
    const due = i.due_date ?? i.date;
    const daysPastDue = Math.max(0, Math.floor((Date.parse(asOf) - Date.parse(due)) / 86400000));
    const bucket = bucketFor(daysPastDue);
    if (!byContact.has(i.contact_id)) {
      byContact.set(i.contact_id, {
        contact_id: i.contact_id,
        name: i.contact?.name ?? null,
        buckets: { ...EMPTY_BUCKETS },
        in_batch_cents: 0,
        total_cents: 0,
        items: [],
      });
    }
    const c = byContact.get(i.contact_id);
    c.buckets[bucket] += outstanding;
    c.total_cents += outstanding;
    c.items.push({
      ref: i.invoice_number ?? `#${i.id}`,
      date: i.date,
      due_date: due,
      days_past_due: daysPastDue,
      outstanding_cents: outstanding,
    });
  }

  // Net finalized credit notes against the outstanding: the ledger already
  // reduced Debiteuren when the credit note was booked, so a debtor with a
  // €1000 invoice and a €400 credit note owes €600 — the aging must not
  // overstate the dunning amount. Credits offset the OLDEST debt first
  // (FIFO), so bucket sums keep reconciling with each contact's total.
  const credits = listInvoices(db)
    .filter((i) => i.invoice_type === 'credit' && !['draft', 'void'].includes(i.status))
    // a credit note dated after the as-of date did not exist yet at as-of
    .filter((i) => i.date <= asOf);
  for (const cr of credits) {
    const c = byContact.get(cr.contact_id);
    if (!c || c.total_cents <= 0) continue;
    // FIFO-offset the items too so item-level outstanding reconciles with the
    // contact/bucket totals after netting (a €400 credit on a €1000 invoice
    // must show the item at €600, not €1000). Independent pass: the buckets
    // below consume their OWN copy of the credit so both stay correct.
    let itemRemaining = cr.gross_cents;
    // items are newest-first (listInvoices ORDER BY id DESC) — offset the
    // OLDEST debt first (FIFO), matching the bucket pass below
    for (let n = c.items.length - 1; n >= 0 && itemRemaining > 0; n -= 1) {
      const item = c.items[n];
      const take = Math.min(item.outstanding_cents, itemRemaining);
      item.outstanding_cents -= take;
      itemRemaining -= take;
    }
    let bucketRemaining = cr.gross_cents;
    for (const b of ['d90plus', 'd90', 'd60', 'd30', 'current']) {
      if (bucketRemaining <= 0) break;
      const take = Math.min(c.buckets[b], bucketRemaining);
      c.buckets[b] -= take;
      bucketRemaining -= take;
    }
    c.total_cents = Object.values(c.buckets).reduce((s, v) => s + v, 0);
  }

  const contacts = [...byContact.values()].sort((a, b) => b.total_cents - a.total_cents);
  const totals = emptyTotals();
  for (const c of contacts) {
    for (const b of Object.keys(EMPTY_BUCKETS)) totals[b] += c.buckets[b];
    totals.total_cents += c.total_cents;
  }
  return { contacts, totals };
}

/** Unpaid payables per contact; in_batch shown separately (money is leaving). */
function creditorsAging(db, asOf) {
  // as-of semantics: a payable dated AFTER the as-of date did not exist yet
  // at as-of — the debtors leg filters invoices by date <= asOf, so the
  // creditors leg must filter payables the same way (a historical aging
  // would otherwise overstate what was owed at that date)
  const rows = db.prepare(`
    SELECT p.id, p.contact_id, p.invoice_ref, p.date, p.due_date, p.amount_cents, p.status,
           c.name AS contact_name
    FROM payables p LEFT JOIN contacts c ON c.id = p.contact_id
    WHERE p.status IN ('unpaid','in_batch')
      AND p.date <= ?
    ORDER BY p.due_date, p.id
  `).all(asOf);

  const byContact = new Map();
  for (const p of rows) {
    const due = p.due_date ?? p.date;
    const daysPastDue = Math.max(0, Math.floor((Date.parse(asOf) - Date.parse(due)) / 86400000));
    const bucket = bucketFor(daysPastDue);
    if (!byContact.has(p.contact_id)) {
      byContact.set(p.contact_id, {
        contact_id: p.contact_id,
        name: p.contact_name ?? null,
        buckets: { ...EMPTY_BUCKETS },
        in_batch_cents: 0,
        total_cents: 0,
        items: [],
      });
    }
    const c = byContact.get(p.contact_id);
    if (p.status === 'in_batch') {
      c.in_batch_cents += p.amount_cents;
    } else {
      c.buckets[bucket] += p.amount_cents;
    }
    c.total_cents += p.amount_cents;
    c.items.push({
      payable_id: p.id,
      ref: p.invoice_ref,
      date: p.date,
      due_date: due,
      days_past_due: daysPastDue,
      amount_cents: p.amount_cents,
      status: p.status,
    });
  }

  const contacts = [...byContact.values()].sort((a, b) => b.total_cents - a.total_cents);
  const totals = emptyTotals();
  for (const c of contacts) {
    for (const b of Object.keys(EMPTY_BUCKETS)) totals[b] += c.buckets[b];
    totals.total_cents += c.total_cents;
  }
  return { contacts, totals };
}

/**
 * Open items as of a date. kind: 'debtors' | 'creditors' | 'both' (default).
 * Buckets are days past due relative to asOf.
 */
export function aging(db, { asOf = null, kind = 'both' } = {}) {
  const asOfDate = asOf ?? todayIso();
  validateLabeledDate(asOfDate, 'as-of');
  if (!['debtors', 'creditors', 'both'].includes(kind)) {
    throw reportError('INVALID_KIND', `kind must be 'debtors', 'creditors' or 'both', got '${kind}'`);
  }
  const result = { as_of: asOfDate, kind };
  if (kind === 'debtors' || kind === 'both') result.debtors = debtorsAging(db, asOfDate);
  if (kind === 'creditors' || kind === 'both') result.creditors = creditorsAging(db, asOfDate);
  return result;
}
