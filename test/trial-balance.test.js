/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across eleven jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

import { test, beforeEach } from 'node:test';
import assert from 'node:assert/strict';
import { openDb } from '../src/core/db.js';
import { seedDefaultChart } from '../src/core/accounts.js';
import { createEntry, postEntry, reverseEntry } from '../src/core/entries.js';
import { trialBalance } from '../src/report/trial-balance.js';

let db;

beforeEach(() => {
  db = openDb(':memory:');
  seedDefaultChart(db);
});

function post(date, description, postings) {
  const e = createEntry(db, { date, description, postings });
  return postEntry(db, { id: e.id });
}

test('trial balance: startkapitaal + expense, per-account totals', () => {
  post('2026-01-10', 'Startkapitaal', [{ code: '1100', amountCents: 1000000 }, { code: '3000', amountCents: -1000000 }]);
  post('2026-01-15', 'Kantoorartikelen', [{ code: '4300', amountCents: 50000 }, { code: '1100', amountCents: -50000 }]);

  const tb = trialBalance(db);
  assert.equal(tb.balanced, true);
  const byCode = Object.fromEntries(tb.accounts.map((a) => [a.code, a]));

  assert.equal(byCode['1100'].net_cents, 950000);  // 1.000.000 - 50.000
  assert.equal(byCode['3000'].net_cents, -1000000);
  assert.equal(byCode['4300'].net_cents, 50000);
  assert.equal(tb.total_debit_cents, 1050000);
  assert.equal(tb.total_credit_cents, 1050000);
});

test('trial balance: year filter', () => {
  post('2026-01-10', 'y2026', [{ code: '1100', amountCents: 100 }, { code: '3000', amountCents: -100 }]);
  post('2025-12-31', 'y2025', [{ code: '1100', amountCents: 500 }, { code: '3000', amountCents: -500 }]);

  const tb2026 = trialBalance(db, { year: '2026' });
  assert.equal(tb2026.total_debit_cents, 100);
  const tb2025 = trialBalance(db, { year: '2025' });
  assert.equal(tb2025.total_debit_cents, 500);
  const tbNone = trialBalance(db);
  assert.equal(tbNone.total_debit_cents, 600);
});

test('trial balance: drafts are excluded, reversals net out', () => {
  const e = createEntry(db, {
    date: '2026-02-01', description: 'draft only',
    postings: [{ code: '1100', amountCents: 999 }, { code: '3000', amountCents: -999 }],
  }); // stays draft
  const posted = post('2026-02-02', 'Omzet', [{ code: '1100', amountCents: 12100 }, { code: '8000', amountCents: -12100 }]);
  reverseEntry(db, { id: posted.id, reason: 'credit note' });

  // The reversal is itself a balanced mirror entry: totals stay equal,
  // every account nets to zero. Drafts are excluded.
  const tb = trialBalance(db);
  assert.equal(tb.balanced, true);
  assert.equal(tb.total_debit_cents, tb.total_credit_cents);
  assert.ok(tb.accounts.every((a) => a.net_cents === 0));
});
