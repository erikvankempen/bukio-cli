/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across eleven jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Winst- en verliesrekening (profit & loss) for a period, grouped by RGS hoofdgroep.
import { validateLabeledDate } from '../core/dates.js';
import { rgsLabel } from '../core/chart.js';

// Display order: revenue first, then costs.
const PNL_GROUPS = ['WOMZ.80', 'WOVB.82', 'WKPR.70', 'WPER.40', 'WAFS.41', 'WBED.42', 'WFBE.84'];

/**
 * P&L for entries with date in [from, to] (inclusive).
 * Section value: income sections show positive revenue (-net), expense
 * sections show positive cost (net). result = revenue - costs.
 */
export function pnl(db, { from, to }) {
  // a garbage year builds ranges like 'abc-01-01' — reject before the query
  // (same class as the exportXaf/jaarrekening year fix)
    // a garbage year builds ranges like 'abc-01-01' — reject before the query
    // (same class as the exportXaf/jaarrekening year fix)
  for (const [label, v] of [['from', from], ['to', to]]) validateLabeledDate(v, label);
  const rows = db.prepare(`
    SELECT a.id, a.code, a.name, a.type, a.taxonomy_code,
      COALESCE(SUM(p.amount_cents), 0) AS net_cents
    FROM accounts a
    LEFT JOIN (
      SELECT p.account_id, p.amount_cents
      FROM postings p
      JOIN journal_entries e ON e.id = p.entry_id
      WHERE e.state = 'posted' AND e.source != 'closing' AND e.date >= ? AND e.date <= ?
    ) p ON p.account_id = a.id
    WHERE a.type IN ('income','expense')
    GROUP BY a.id
    ORDER BY a.code
  `).all(from, to);

  const known = new Set(PNL_GROUPS);
  const sections = PNL_GROUPS.map((code) => {
    const accounts = rows
      .filter((r) => (r.taxonomy_code || 'overig') === code)
      .map((r) => ({
        code: r.code,
        name: r.name,
        type: r.type,
        amount_cents: r.type === 'income' ? -r.net_cents : r.net_cents,
      }))
      .filter((a) => a.amount_cents !== 0);
    return {
      taxonomy_code: code,
      label: rgsLabel(code),
      accounts,
      total_cents: accounts.reduce((s, a) => s + a.amount_cents, 0),
    };
  }).filter((s) => s.accounts.length > 0);

  // catch-all: income/expense accounts with an unknown taxonomy_code
  const leftover = rows
    .filter((r) => !known.has(r.taxonomy_code || 'overig'))
    .map((r) => ({
      code: r.code,
      name: r.name,
      type: r.type,
      amount_cents: r.type === 'income' ? -r.net_cents : r.net_cents,
    }))
    .filter((a) => a.amount_cents !== 0);
  if (leftover.length) {
    sections.push({
      taxonomy_code: null,
      label: 'Overig',
      accounts: leftover,
      total_cents: leftover.reduce((s, a) => s + a.amount_cents, 0),
    });
  }

  // revenue/costs are driven by account TYPE, not taxonomy_code: imported legacy
  // charts often have no RGS codes, and a type-based split keeps the P&L
  // correct there (rgs only shapes the display sections).
  const revenue = rows.filter((r) => r.type === 'income').reduce((s, r) => s - r.net_cents, 0);
  const costs = rows.filter((r) => r.type === 'expense').reduce((s, r) => s + r.net_cents, 0);

  return {
    from,
    to,
    sections,
    revenue_cents: revenue,
    costs_cents: costs,
    result_cents: revenue - costs,
  };
}
