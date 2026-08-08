/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// ICP readout (Phase 4) — Intracommunautaire prestaties listing.
// Per EU customer (contact with a foreign VAT id), the total value of
// btw-verlegde EU supplies (invoice lines with vat_code 'RE') in a quarter.
// Aid for the quarterly ICP listing — bukio never submits anything.
import { parsePeriod } from '../vat/index.js';
import { formatAmount } from '../core/money.js';

export function icpError(code, message) {
  const e = new Error(message);
  e.code = code;
  return e;
}

export function icpReadout(db, { period }) {
  const { from, to, label } = parsePeriod(period);

  const rows = db.prepare(`
    SELECT c.id AS contact_id, c.name, c.vat_id, c.country,
      SUM(CASE WHEN i.invoice_type = 'credit' THEN -l.amount_cents ELSE l.amount_cents END) AS amount_cents,
      GROUP_CONCAT(DISTINCT i.invoice_number) AS invoice_numbers
    FROM invoices i
    JOIN invoice_lines l ON l.invoice_id = i.id
    JOIN contacts c ON c.id = i.contact_id
    WHERE i.invoice_number IS NOT NULL
      AND i.status IN ('sent', 'paid', 'overdue')
      AND i.date >= ? AND i.date <= ?
      AND l.vat_code = 'RE'
    GROUP BY c.id
    ORDER BY c.name
  `).all(from, to);

  const customers = rows.map((r) => ({
    contact_id: r.contact_id,
    name: r.name,
    vat_id: r.vat_id,
    country: r.country,
    amount_cents: r.amount_cents,
    amount: formatAmount(r.amount_cents),
    invoice_numbers: r.invoice_numbers ? r.invoice_numbers.split(',') : [],
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
