/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Year-end close (Phase 4) — afsluiting van het boekjaar.
// Two posted entries, both source='closing' with source_ref='fy:YYYY':
//   1. Afsluiting boekjaar: every income/expense account reversed into
//      9900 Resultaat boekjaar (created on demand).
//   2. Resultaatbestemming: 9900 -> 3000 Eigen vermogen.
// The P&L report excludes source='closing' so the year's flow stays visible
// after closing; the balance sheet (which includes the entries) then shows
// equity including the result. Deterministic, dry-run first, fully audited.
import { createAccount, getAccountByCode } from '../core/accounts.js';
import { createEntry, postEntry } from '../core/entries.js';
import { record } from '../audit/index.js';

export function yearEndError(code, message) {
  const e = new Error(message);
  e.code = code;
  return e;
}

/** Net per income/expense account in the year (posted, excluding closing). */
export function resultAccounts(db, year) {
  const [from, to] = fiscalYearWindow(db, year);
  return db.prepare(`
    SELECT a.code, a.name, a.type,
      COALESCE(SUM(p.amount_cents), 0) AS net_cents
    FROM accounts a
    LEFT JOIN (
      SELECT p.account_id, p.amount_cents
      FROM postings p
      JOIN journal_entries e ON e.id = p.entry_id
      WHERE e.state = 'posted' AND e.source != 'closing'
        AND e.date >= ? AND e.date <= ?
    ) p ON p.account_id = a.id
    WHERE a.type IN ('income', 'expense')
    GROUP BY a.id
    HAVING net_cents != 0
    ORDER BY a.code
  `).all(from, to);
}

/**
 * The fiscal-year window a closing year refers to: [start, end] ISO dates.
 * Default fiscal year 12-31 -> the calendar year; anything else (e.g. 06-30)
 * -> the 12 months ending in that year (year 2026, FY 06-30 ->
 * 2025-07-01..2026-06-30). The year parameter is the year the FY ENDS in,
 * matching compliance/jaarrekening and the balans peildatum.
 */
export function fiscalYearWindow(db, year) {
  const fy = db.prepare('SELECT fiscal_year_end FROM company WHERE id = 1').get()?.fiscal_year_end || '12-31';
  // tolerant parse: '12-31' (canonical) or a full '2026-12-31' value
  const parts = String(fy).split('-');
  const mm = Number(parts[parts.length - 2]);
  const dd = Number(parts[parts.length - 1]);
  if (mm === 12 && dd === 31) return [`${year}-01-01`, `${year}-12-31`];
  const end = `${year}-${String(mm).padStart(2, '0')}-${String(dd).padStart(2, '0')}`;
  const start = new Date(Date.UTC(Number(year) - 1, mm - 1, dd + 1)).toISOString().slice(0, 10);
  return [start, end];
}

export function isYearClosed(db, year) {
  return Boolean(db.prepare(
    "SELECT 1 FROM journal_entries WHERE source = 'closing' AND source_ref = ?",
  ).get(`fy:${year}`));
}

export function yearEndStatus(db, { year }) {
  if (!/^\d{4}$/.test(String(year))) throw yearEndError('INVALID_YEAR', `year '${year}' must be YYYY`);
  const rows = db.prepare("SELECT * FROM journal_entries WHERE source = 'closing' AND source_ref = ? ORDER BY id").all(`fy:${year}`);
  const accounts = resultAccounts(db, year);
  // `-0` must normalize to 0 (Object.is treats them differently; it leaks
  // into JSON consumers and renders as '-0.00' in edge cases)
  const rawResult = accounts.reduce((s, a) => s + a.net_cents, 0);
  const resultCents = rawResult === 0 ? 0 : -rawResult;
  return {
    year,
    closed: rows.length > 0,
    result_cents: resultCents,
    accounts: accounts.map((a) => ({ code: a.code, name: a.name, net_cents: a.net_cents })),
    closing_entries: rows.map((r) => ({ id: r.id, date: r.date, description: r.description, state: r.state })),
  };
}

/**
 * Close the fiscal year: guard, compute the result, create 9900 if needed,
 * post the closing + appropriation entries. Dry-run returns the plan.
 */
export function yearEndClose(db, { year, actor = 'human', dryRun = false }) {
  if (!/^\d{4}$/.test(String(year))) throw yearEndError('INVALID_YEAR', `year '${year}' must be YYYY`);
  const company = db.prepare('SELECT * FROM company WHERE id = 1').get();
  if (!company) throw yearEndError('NOT_INITIALISED', 'company database not initialised');

  const [fyFrom, fyTo] = fiscalYearWindow(db, year);
  const drafts = db.prepare(
    "SELECT COUNT(*) c FROM journal_entries WHERE state = 'draft' AND date >= ? AND date <= ?",
  ).get(fyFrom, fyTo).c;
  if (drafts > 0) throw yearEndError('INCOMPLETE_YEAR', `${drafts} draft entry/entries in ${year} — post or reverse them before closing`);

  if (isYearClosed(db, year)) throw yearEndError('ALREADY_CLOSED', `${year} is already closed (undo with entry reverse on the closing entries)`);

  const accounts = resultAccounts(db, year);
  if (accounts.length === 0) {
    return { closed: false, year, reason: 'EMPTY_YEAR', result_cents: 0, message: `no income/expense activity in ${year} — nothing to close` };
  }
  // `-0` must normalize to 0 (Object.is treats them differently; it leaks
  // into JSON consumers and renders as '-0.00' in edge cases)
  const rawResult = accounts.reduce((s, a) => s + a.net_cents, 0);
  const resultCents = rawResult === 0 ? 0 : -rawResult;

  // entry 1: reverse every result account into 9900. When the year nets to
  // zero (income == expense) the reversals alone sum to zero — there is no
  // 9900 balance to carry, so the zero-amount 9900/3000 legs are skipped
  // (createEntry rejects zero postings; a zero-result year closes cleanly).
  const closingPostings = accounts.map((a) => ({ code: a.code, amountCents: -a.net_cents }));
  if (resultCents !== 0) {
    closingPostings.push({ code: '9900', amountCents: -resultCents });
  }
  // entry 2: appropriation to equity (only when there is a result)
  const appropriationPostings = resultCents !== 0
    ? [
      { code: '9900', amountCents: resultCents },
      { code: '3000', amountCents: -resultCents },
    ]
    : [];

  if (dryRun) {
    return {
      closed: false, year, dryRun: true, result_cents: resultCents,
      create_9900: resultCents !== 0 && !getAccountByCode(db, '9900'),
      entries: [
        { description: `Afsluiting boekjaar ${year}`, postings: closingPostings },
        ...(appropriationPostings.length ? [{ description: `Resultaatbestemming ${year}`, postings: appropriationPostings }] : []),
      ],
    };
  }

  // closing entries are dated at the FISCAL year end (12-31 default, or the
  // company's fiscal_year_end) — not hardcoded to 12-31
  const closeDate = fyTo;
  const tx = db.transaction(() => {
    if (resultCents !== 0 && !getAccountByCode(db, '9900')) {
      createAccount(db, { code: '9900', name: 'Resultaat boekjaar', type: 'equity', normalBalance: 'credit', rgsCode: 'BEIV.05' });
    }
    const e1 = createEntry(db, {
      date: closeDate, description: `Afsluiting boekjaar ${year}`,
      postings: closingPostings, source: 'closing', sourceRef: `fy:${year}`, actor: 'closing',
    });
    const p1 = postEntry(db, { id: e1.id, actor: 'closing' });
    const results = { e1: p1 };
    if (appropriationPostings.length) {
      const e2 = createEntry(db, {
        date: closeDate, description: `Resultaatbestemming ${year}`,
        postings: appropriationPostings, source: 'closing', sourceRef: `fy:${year}`, actor: 'closing',
      });
      const p2 = postEntry(db, { id: e2.id, actor: 'closing' });
      results.e2 = p2;
    }
    record(db, {
      actor, action: 'year_end.close', command: 'year-end close',
      args: { year, result_cents: resultCents }, outcome: 'ok', entryIds: [results.e1.id, ...(results.e2 ? [results.e2.id] : [])],
    });
    return results;
  });
  const { e1, e2 } = tx();
  return { closed: true, year, result_cents: resultCents, entries: e2 ? [e1, e2] : [e1] };
}
