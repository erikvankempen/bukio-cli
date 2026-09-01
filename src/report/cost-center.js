/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across thirty-one jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Cost-center analysis report: per-cost-center net debit/credit and a P&L-style
// breakdown (revenue vs costs). Postings without a cost center roll up under
// 'unassigned'. The report covers ONLY income/expense accounts (asset/liability/
// equity are static-balance accounts and don't belong in a P&L cost-center
// view — but we include them in the full break-down for completeness).
import { fiscalYearWindow } from '../year-end/index.js';

/**
 * Analytical cost-center report for a period.
 * Returns per-center: accounts with net_cents, plus revenue/costs/result.
 * @param {object} opts
 * @param {string} [opts.year] - fiscal year shorthand (overrides from/to)
 * @param {string} [opts.from] - period start (inclusive)
 * @param {string} [opts.to] - period end (inclusive)
 * @param {string} [opts.costCenter] - filter to a single cost center code
 */
export function costCenterReport(db, { year = null, from = null, to = null, costCenter = null } = {}) {
  if (year != null && !/^\d{4}$/.test(String(year))) {
    const e = new Error(`year '${year}' must be YYYY`);
    e.code = 'INVALID_YEAR';
    throw e;
  }
  // year overrides from/to; from/to used directly when no year.
  const [fyFrom, fyTo] = year != null ? fiscalYearWindow(db, String(year)) : [null, null];
  const effectiveFrom = from ?? fyFrom;
  const effectiveTo = to ?? fyTo;

  // One query: all posted income/expense postings with cost center.
  // LEFT JOIN so unassigned (NULL cc) still appear.
  const rows = db.prepare(`
    SELECT p.account_id, p.amount_cents,
           a.code AS account_code, a.name AS account_name, a.type AS account_type,
           cc.code AS cost_center_code, cc.name AS cost_center_name
    FROM postings p
    JOIN journal_entries e ON e.id = p.entry_id
    JOIN accounts a ON a.id = p.account_id
    LEFT JOIN cost_centers cc ON cc.id = p.cost_center_id
    WHERE e.state = 'posted'
      AND e.source != 'closing'
      AND a.type IN ('income','expense')
      AND (? IS NULL OR e.date >= ?)
      AND (? IS NULL OR e.date <= ?)
      AND (? IS NULL OR cc.code = ?)
  `).all(effectiveFrom, effectiveFrom, effectiveTo, effectiveTo, costCenter, costCenter);

  // Group by cost center.
  const byCc = new Map(); // ccKey → { code, name, accounts: Map<accountId, {code,name,type,net}> }
  for (const r of rows) {
    const key = r.cost_center_code ?? '__unassigned__';
    let bucket = byCc.get(key);
    if (!bucket) {
      bucket = {
        cost_center_code: r.cost_center_code,
        cost_center_name: r.cost_center_name,
        accounts: new Map(),
      };
      byCc.set(key, bucket);
    }
    const accKey = r.account_id;
    const acc = bucket.accounts.get(accKey) ?? { code: r.account_code, name: r.account_name, type: r.account_type, net_cents: 0 };
    acc.net_cents += r.amount_cents;
    bucket.accounts.set(accKey, acc);
  }

  // Build final structure.
  const centers = [];
  for (const bucket of byCc.values()) {
    const accs = [...bucket.accounts.values()]
      .map((a) => ({
        code: a.code,
        name: a.name,
        type: a.type,
        net_cents: a.net_cents,
        net: (a.net_cents / 100).toFixed(2),
        amount_cents: a.type === 'income' ? -a.net_cents : a.net_cents,
      }))
      .filter((a) => a.amount_cents !== 0)
      .sort((a, b) => a.code.localeCompare(b.code));

    const revenue = accs.filter((a) => a.type === 'income').reduce((s, a) => s + a.amount_cents, 0);
    const costs = accs.filter((a) => a.type === 'expense').reduce((s, a) => s + a.amount_cents, 0);

    centers.push({
      cost_center_code: bucket.cost_center_code,
      cost_center_name: bucket.cost_center_name ?? (bucket.cost_center_code == null ? 'Unassigned' : null),
      accounts: accs,
      revenue_cents: revenue,
      costs_cents: costs,
      result_cents: revenue - costs,
    });
  }

  // Sort: named centers first, unassigned last.
  centers.sort((a, b) => {
    if (a.cost_center_code == null) return 1;
    if (b.cost_center_code == null) return -1;
    return a.cost_center_code.localeCompare(b.cost_center_code);
  });

  return { year, from: effectiveFrom, to: effectiveTo, centers };
}
