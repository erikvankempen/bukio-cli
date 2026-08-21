/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across thirty-one jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// ICP readout (Phase 4) — Intracommunautaire prestaties listing.
// Per EU customer (contact with a foreign VAT id), the total value of
// btw-verlegde EU supplies (invoice lines with vat_code 'RE') in a quarter.
// Aid for the quarterly ICP listing — bukio never submits anything.
import { parsePeriod } from '../vat/index.js';
import { formatAmount } from '../core/money.js';
import { getInvoice, computeInvoiceTotals } from '../invoice/index.js';
import { makeError } from '../core/errors.js';

export function icpError(code, message) {
  return makeError(code, message);
}

export function icpReadout(db, { period }) {
  const { from, to, label } = parsePeriod(period);

  const invoiceRows = db.prepare(`
    SELECT DISTINCT i.id, i.invoice_type, i.invoice_number, i.date, i.contact_id,
           c.name, c.vat_id, c.country
    FROM invoices i
    JOIN invoice_lines l ON l.invoice_id = i.id
    JOIN contacts c ON c.id = i.contact_id
    WHERE i.invoice_number IS NOT NULL
      AND i.status IN ('sent', 'paid', 'overdue')
      AND i.date >= ? AND i.date <= ?
      AND l.vat_code = 'RE'
    ORDER BY c.name, i.id
  `).all(from, to);

  const perContact = new Map();
  for (const row of invoiceRows) {
    const inv = getInvoice(db, row.id);
    // the DISCOUNTED RE base: the per-rate-group allocation spreads line and
    // total discounts across groups, so the RE group net is exactly the base
    // the OB readout reports in 2a — the ICP listing must agree with it
    // (raw line sums would ignore v0.13 discounts and overstate the listing)
    const totals = computeInvoiceTotals(inv.lines, inv.discount_type, inv.discount_value);
    const reNet = totals.groups.find((g) => g.code === 'RE')?.discountedNet ?? 0;
    if (reNet === 0) continue;
    const signed = row.invoice_type === 'credit' ? -reNet : reNet;
    const cur = perContact.get(row.contact_id) ?? {
      contact_id: row.contact_id, name: row.name, vat_id: row.vat_id,
      country: row.country, amount_cents: 0, invoice_numbers: [],
    };
    cur.amount_cents += signed;
    cur.invoice_numbers.push(row.invoice_number);
    perContact.set(row.contact_id, cur);
  }

  const customers = [...perContact.values()].map((c) => ({
    ...c,
    amount: formatAmount(c.amount_cents),
    invoice_numbers: [...new Set(c.invoice_numbers)],
  }));

  const missingVat = customers.filter((c) => !c.vat_id);
  if (missingVat.length) {
    throw icpError(
      'ICP_VAT_ID_MISSING',
      `EU customers without a btw-id (required for the ICP listing): ${missingVat.map((c) => c.name).join(', ')} — add it with contact add / an update`,
    );
  }

  return {
    period: label,
    from,
    to,
    customers,
    total_cents: customers.reduce((s, c) => s + c.amount_cents, 0),
    total: formatAmount(customers.reduce((s, c) => s + c.amount_cents, 0)),
    note: 'Manual filing aid only — bukio never submits the ICP listing.',
  };
}
