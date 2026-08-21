/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across thirty-one jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Trial balance — per-account debit/credit/net from posted entries.
import { fiscalYearWindow } from '../year-end/index.js';

export function trialBalance(db, { year = null } = {}) {
  if (year != null && !/^\d{4}$/.test(String(year))) {
    const e = new Error(`year '${year}' must be YYYY`);
    e.code = 'INVALID_YEAR';
    throw e;
  }
  const [from, to] = year != null ? fiscalYearWindow(db, String(year)) : [null, null];
  const rows = db.prepare(`
    SELECT a.code, a.name, a.type,
      SUM(CASE WHEN p.amount_cents > 0 THEN p.amount_cents ELSE 0 END) AS debit_cents,
      SUM(CASE WHEN p.amount_cents < 0 THEN -p.amount_cents ELSE 0 END) AS credit_cents,
      SUM(p.amount_cents) AS net_cents
    FROM postings p
    JOIN journal_entries e ON e.id = p.entry_id
    JOIN accounts a ON a.id = p.account_id
    WHERE e.state = 'posted'
      AND (? IS NULL OR e.date >= ?)
      AND (? IS NULL OR e.date <= ?)
    GROUP BY a.id
    ORDER BY a.code
  `).all(from, from, to, to);

  const totalDebit = rows.reduce((s, r) => s + r.debit_cents, 0);
  const totalCredit = rows.reduce((s, r) => s + r.credit_cents, 0);

  return {
    accounts: rows,
    total_debit_cents: totalDebit,
    total_credit_cents: totalCredit,
    balanced: totalDebit === totalCredit,
  };
}
