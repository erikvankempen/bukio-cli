/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Sales analytics: revenue from posted sales invoices in a year, grouped by
// contact (net/vat/gross via computeInvoiceTotals — the engine's single
// source of truth) or by catalog item / ad-hoc line description (net only,
// after per-line discounts; invoice-level discounts are NOT allocated down to
// lines — documented approximation). Credit notes are excluded.
// Read-only. `report sales` is the agent's weekly-briefing number.
import { listInvoices, lineDiscountCents } from '../invoice/index.js';

export function salesError(code, message) {
  const e = new Error(message);
  e.code = code;
  return e;
}

/**
 * Revenue from posted sales invoices in `year`, by contact or by item.
 * by 'contact': {contact_id, name, invoice_count, net_cents, vat_cents, gross_cents}
 * by 'item':    {key, item_id, name, line_count, net_cents}
 */
export function sales(db, { year, by = 'contact' } = {}) {
  if (typeof year !== 'string' || !/^\d{4}$/.test(year)) {
    throw salesError('INVALID_YEAR', `year '${year}' must be YYYY`);
  }
  if (!['contact', 'item'].includes(by)) {
    throw salesError('INVALID_KIND', `by must be 'contact' or 'item', got '${by}'`);
  }
  const from = `${year}-01-01`;
  const to = `${year}-12-31`;
  const invoices = listInvoices(db)
    .filter((i) => i.invoice_type === 'sales')
    .filter((i) => i.status !== 'draft' && i.status !== 'void')
    .filter((i) => i.date >= from && i.date <= to);

  if (by === 'contact') {
    const map = new Map();
    for (const i of invoices) {
      if (!map.has(i.contact_id)) {
        map.set(i.contact_id, {
          contact_id: i.contact_id, name: i.contact?.name ?? null,
          invoice_count: 0, net_cents: 0, vat_cents: 0, gross_cents: 0,
        });
      }
      const g = map.get(i.contact_id);
      g.invoice_count += 1;
      g.net_cents += i.net_cents;
      g.vat_cents += i.vat_cents;
      g.gross_cents += i.gross_cents;
    }
    const groups = [...map.values()].sort((a, b) => b.gross_cents - a.gross_cents);
    const totals = groups.reduce((t, g) => {
      t.invoice_count += g.invoice_count;
      t.net_cents += g.net_cents;
      t.vat_cents += g.vat_cents;
      t.gross_cents += g.gross_cents;
      return t;
    }, { invoice_count: 0, net_cents: 0, vat_cents: 0, gross_cents: 0 });
    return { year, by, groups, totals };
  }

  // by item: net after per-line discounts; invoice-level discounts excluded
  const itemNames = new Map();
  const map = new Map();
  for (const i of invoices) {
    for (const l of i.lines) {
      const key = l.item_id != null ? `item:${l.item_id}` : `desc:${l.description}`;
      if (!map.has(key)) {
        let name = l.description;
        if (l.item_id != null) {
          if (!itemNames.has(l.item_id)) {
            const item = db.prepare('SELECT name FROM items WHERE id = ?').get(l.item_id);
            itemNames.set(l.item_id, item?.name ?? l.description);
          }
          name = itemNames.get(l.item_id);
        }
        map.set(key, { key, item_id: l.item_id ?? null, name, line_count: 0, net_cents: 0 });
      }
      const g = map.get(key);
      g.line_count += 1;
      g.net_cents += l.amount_cents - lineDiscountCents(l);
    }
  }
  const groups = [...map.values()].sort((a, b) => b.net_cents - a.net_cents);
  const totals = groups.reduce((t, g) => {
    t.line_count += g.line_count;
    t.net_cents += g.net_cents;
    return t;
  }, { line_count: 0, net_cents: 0 });
  return { year, by, groups, totals };
}
