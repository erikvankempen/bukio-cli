import { test, beforeEach } from 'node:test';
import assert from 'node:assert/strict';
import { openDb } from '../src/core/db.js';
import { seedDefaultChart, createAccount } from '../src/core/accounts.js';
import { createEntry, postEntry, reverseEntry } from '../src/core/entries.js';
import { balans } from '../src/report/balans.js';
import { pnl } from '../src/report/pnl.js';
import { journal } from '../src/report/journal.js';

let db;

beforeEach(() => {
  db = openDb(':memory:');
  seedDefaultChart(db);
});

function post(date, description, postings) {
  const e = createEntry(db, { date, description, postings });
  return postEntry(db, { id: e.id });
}

function seedScenario() {
  post('2026-01-05', 'Startkapitaal', [{ code: '1100', amountCents: 1000000 }, { code: '3000', amountCents: -1000000 }]);
  post('2026-02-10', 'Omzet', [{ code: '1100', amountCents: 121000 }, { code: '8000', amountCents: -121000 }]);
  post('2026-03-01', 'Kantoorartikelen', [{ code: '4300', amountCents: 25000 }, { code: '1100', amountCents: -25000 }]);
}

test('balans: assets = liabilities + equity + result', () => {
  seedScenario();
  const b = balans(db, { asOf: '2026-03-31' });
  assert.equal(b.balanced, true);
  assert.equal(b.assets.total_cents, 1096000); // 1.000.000 + 121.000 - 25.000
  assert.equal(b.liabilities_and_equity.total_cents, 1096000);
  assert.equal(b.liabilities_and_equity.result_cents, 96000); // 121.000 - 25.000

  const blim = b.assets.sections.find((s) => s.rgs_code === 'BLIM.10');
  assert.equal(blim.label, 'Liquide middelen');
  assert.equal(blim.total_cents, 1096000);
  const ev = b.liabilities_and_equity.sections.find((s) => s.rgs_code === 'BEIV.05');
  assert.equal(ev.total_cents, 1000000);
});

test('balans: before any income/expense, result is zero', () => {
  post('2026-01-05', 'Startkapitaal', [{ code: '1100', amountCents: 1000000 }, { code: '3000', amountCents: -1000000 }]);
  const b = balans(db, { asOf: '2026-01-31' });
  assert.equal(b.balanced, true);
  assert.equal(b.assets.total_cents, 1000000);
  assert.equal(b.liabilities_and_equity.result_cents, 0);
});

test('balans: empty books balance at zero', () => {
  const b = balans(db, { asOf: '2026-01-31' });
  assert.equal(b.balanced, true);
  assert.equal(b.assets.total_cents, 0);
  assert.equal(b.liabilities_and_equity.total_cents, 0);
});

test('balans: drafts excluded, reversal nets out', () => {
  createEntry(db, {
    date: '2026-02-01', description: 'draft',
    postings: [{ code: '1100', amountCents: 999 }, { code: '3000', amountCents: -999 }],
  }); // never posted — must not appear
  const posted = post('2026-02-02', 'Omzet', [{ code: '1100', amountCents: 50000 }, { code: '8000', amountCents: -50000 }]);
  reverseEntry(db, { id: posted.id, reason: 'credit note' });

  const b = balans(db, { asOf: '2026-12-31' });
  assert.equal(b.balanced, true);
  assert.equal(b.assets.total_cents, 0); // omzet + reversal cancel out
  assert.equal(b.liabilities_and_equity.result_cents, 0);
});

test('pnl: revenue, costs and result', () => {
  seedScenario();
  const p = pnl(db, { from: '2026-01-01', to: '2026-12-31' });
  assert.equal(p.revenue_cents, 121000);
  assert.equal(p.costs_cents, 25000);
  assert.equal(p.result_cents, 96000);
  const omzet = p.sections.find((s) => s.rgs_code === 'WOMZ.80');
  assert.equal(omzet.total_cents, 121000);
  const kosten = p.sections.find((s) => s.rgs_code === 'WBED.42');
  assert.equal(kosten.total_cents, 25000);
});

test('pnl: empty period gives zero result and no sections', () => {
  const p = pnl(db, { from: '2025-01-01', to: '2025-12-31' });
  assert.equal(p.result_cents, 0);
  assert.equal(p.sections.length, 0);
});

test('pnl: legacy chart without RGS codes still splits revenue/costs by type', () => {
  // accounts created from an audit-file import carry NO rgs codes (pre-fix
  // data); revenue/costs must be driven by account type, not rgs_code
  for (const [code, name, type] of [
    ['8200', 'Omzet diensten', 'income'],
    ['6531', 'Kosten IT', 'expense'],
    ['6710', 'Afschrijvingskosten', 'expense'],
    ['9100', 'Rentebaten', 'expense'], // contra-expense account
  ]) {
    createAccount(db, { code, name, type, normalBalance: type === 'income' ? 'credit' : 'debit', rgsCode: null });
  }
  post('2026-03-01', 'Factuur', [{ code: '1100', amountCents: 29420 }, { code: '8200', amountCents: -29420 }]);
  post('2026-03-02', 'Hosting', [{ code: '6531', amountCents: 24036 }, { code: '1100', amountCents: -24036 }]);
  post('2026-03-03', 'Afschrijving', [{ code: '6710', amountCents: 29976 }, { code: '1100', amountCents: -29976 }]);
  post('2026-03-04', 'Rente', [{ code: '1100', amountCents: 11716 }, { code: '9100', amountCents: -11716 }]);

  const p = pnl(db, { from: '2026-01-01', to: '2026-12-31' });
  assert.equal(p.revenue_cents, 29420); // 8200 only (9100 is typed expense)
  assert.equal(p.costs_cents, 24036 + 29976 - 11716); // contra-expense nets out
  assert.equal(p.result_cents, 29420 - (24036 + 29976 - 11716)); // -319.51-style
});

test('pnl: catch-all section for accounts with unknown rgs_code', () => {
  createAccount(db, { code: '5000', name: 'Testkosten', type: 'expense', normalBalance: 'debit' });
  post('2026-05-01', 'Testkosten', [{ code: '5000', amountCents: 1000 }, { code: '1100', amountCents: -1000 }]);
  const p = pnl(db, { from: '2026-01-01', to: '2026-12-31' });
  const overig = p.sections.find((s) => s.label === 'Overig');
  assert.equal(overig.total_cents, 1000);
  assert.equal(p.costs_cents, 1000);
});

test('journal: one row per posting, ordered by date', () => {
  seedScenario();
  const rows = journal(db, { from: '2026-01-01', to: '2026-12-31' });
  assert.equal(rows.length, 6); // 3 entries x 2 postings
  assert.equal(rows[0].entry_id, 1);
  assert.equal(rows[0].account_code, '1100');
  assert.equal(rows[0].amount_cents, 1000000);
  assert.equal(rows[0].state, 'posted');
});
