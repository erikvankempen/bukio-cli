/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

import { test, beforeEach } from 'node:test';
import assert from 'node:assert/strict';
import { openDb } from '../src/core/db.js';
import { seedDefaultChart, getAccountByCode } from '../src/core/accounts.js';
import { createEntry, postEntry, listEntries } from '../src/core/entries.js';
import { createAccount } from '../src/core/accounts.js';
import {
  createScheme, ensureDefaultScheme, addAsset, runDue, register,
  disposeAsset, scheduleDepreciation, listAssets,
} from '../src/assets/index.js';

let db;

beforeEach(() => {
  db = openDb(':memory:');
  seedDefaultChart(db);
  createAccount(db, { code: '1500', name: 'Cumulatieve afschrijvingen', type: 'asset', normalBalance: 'debit' });
  // note: the default chart already contains 1850 Vervoermiddelen
});

function post(db, date, description, postings) {
  const e = createEntry(db, { date, description, postings, actor: 'agent:test' });
  return postEntry(db, { id: e.id, actor: 'agent:test' });
}

// --- schemes -----------------------------------------------------------------

test('ensureDefaultScheme: creates the standard 5y linear scheme lazily', () => {
  const s = ensureDefaultScheme(db);
  assert.equal(s.name, 'Standaard 5 jaar lineair');
  assert.equal(s.method, 'lineair');
  assert.equal(s.life_months, 60);
  assert.equal(s.residual_bp, 0);
  // idempotent
  assert.equal(ensureDefaultScheme(db).id, s.id);
});

test('createScheme: rejects duplicate names and bad methods', () => {
  createScheme(db, { name: 'A' });
  assert.throws(() => createScheme(db, { name: 'A' }), { code: 'SCHEME_NAME_TAKEN' });
  assert.throws(() => createScheme(db, { name: 'B', method: 'weird' }), { code: 'INVALID_METHOD' });
  assert.throws(() => createScheme(db, { name: 'C', lifeMonths: 0 }), { code: 'INVALID_LIFE' });
});

// --- schedule math -----------------------------------------------------------

test('scheduleDepreciation: linear 60m is cents-exact and remainder-adjusted', () => {
  const s = scheduleDepreciation({ costCents: 100000, lifeMonths: 60, method: 'lineair', firstPeriod: '2026-01', asOf: '2030-12' });
  assert.equal(s.length, 60);
  assert.equal(s[0].amountCents, 1667);
  assert.equal(s[59].amountCents, 100000 - s.slice(0, 59).reduce((a, x) => a + x.amountCents, 0));
  assert.equal(s.reduce((a, x) => a + x.amountCents, 0), 100000); // cents-exact
  assert.ok(s.every((x) => Math.abs(x.amountCents - 1667) <= 2)); // no drift beyond rounding
});

test('scheduleDepreciation: degressief double-declining with switch to linear', () => {
  const s = scheduleDepreciation({ costCents: 60000, lifeMonths: 12, method: 'degressief', firstPeriod: '2025-02', asOf: '2026-02' });
  assert.equal(s.length, 12);
  assert.equal(s[0].amountCents, 10000); // 600 * 2/12
  assert.equal(s[6].amountCents, 3349);  // switched to the linear view
  assert.equal(s.reduce((a, x) => a + x.amountCents, 0), 60000); // exactly to residual
});

test('scheduleDepreciation: stops at the residual, never overshoots', () => {
  const s = scheduleDepreciation({ costCents: 10000, residualCents: 1000, lifeMonths: 36, method: 'lineair', firstPeriod: '2026-01', asOf: '2035-12' });
  const total = s.reduce((a, x) => a + x.amountCents, 0);
  assert.equal(total, 9000);
  assert.ok(s.length <= 36);
});

// --- addAsset ----------------------------------------------------------------

test('addAsset: standard 5y linear, first run on the 1st of the month', () => {
  const { asset, warnings } = addAsset(db, {
    name: 'Laptop', purchaseDate: '2024-01-15', purchasePriceCents: 200000,
    depreciationStartDate: '2024-02-01', recognitionDate: '2024-02-01',
    assetAccount: '1800', expenseAccount: '4600', actor: 'agent:test',
  });
  assert.equal(asset.status, 'active');
  assert.equal(asset.scheme.name, 'Standaard 5 jaar lineair');
  assert.equal(asset.cum_dep_account_code, null); // booked on the asset account
  // GL reconciliation warnings (the purchase isn't booked in this test)
  assert.ok(warnings.some((w) => w.includes('asset account 1800')));
});

test('addAsset: mid-life adoption keeps only the remaining depreciation', () => {
  // bought 2023, recognised 2024-06 with 250.00 already depreciated:
  // remaining 950.00 over 48 months = 19.79
  const { asset } = addAsset(db, {
    name: 'Fotocamera', purchaseDate: '2023-05-10', purchasePriceCents: 120000,
    depreciationStartDate: '2023-06-01', recognitionDate: '2024-06-01',
    cumDepAtRecognitionCents: 25000,
    assetAccount: '1800', cumDepAccount: '1500', expenseAccount: '4600',
  });
  assert.equal(asset.status, 'active');
  assert.equal(asset.scheme.life_months, 60);
  const r = runDue(db, { period: '2024-06', actor: 'agent:test' });
  assert.equal(r.booked.length, 1);
  assert.equal(r.booked[0].amount_cents, 1979); // round(950/48)
  const reg = register(db, { asOf: '2024-06-30' }).assets.find((a) => a.id === asset.id);
  assert.equal(reg.total_cum_dep_cents, 25000 + 1979);
  assert.equal(reg.book_value_cents, 120000 - 25000 - 1979);
});

test('addAsset: cum-dep at recognition above cost minus residual is rejected', () => {
  assert.throws(() => addAsset(db, {
    name: 'X', purchaseDate: '2024-01-01', purchasePriceCents: 100000,
    depreciationStartDate: '2024-01-01', recognitionDate: '2024-06-01',
    cumDepAtRecognitionCents: 100001, assetAccount: '1800', expenseAccount: '4600',
  }), { code: 'INVALID_DEPRECIATION' });
});

test('addAsset: recognises an already fully depreciated asset as fully_depreciated', () => {
  const { asset } = addAsset(db, {
    name: 'Oud', purchaseDate: '2019-01-01', purchasePriceCents: 100000,
    depreciationStartDate: '2019-02-01', recognitionDate: '2025-02-01',
    cumDepAtRecognitionCents: 100000, assetAccount: '1800', expenseAccount: '4600',
  });
  assert.equal(asset.status, 'fully_depreciated');
  const r = runDue(db, { asOf: '2026-01-01' });
  assert.equal(r.booked.length, 0); // nothing left to depreciate
});

test('addAsset: account type validation', () => {
  assert.throws(() => addAsset(db, {
    name: 'X', purchaseDate: '2024-01-01', purchasePriceCents: 100000,
    depreciationStartDate: '2024-01-01', recognitionDate: '2024-01-01',
    assetAccount: '8000', expenseAccount: '4600', // 8000 is income
  }), { code: 'ACCOUNT_TYPE' });
});

test('addAsset: missing entry link fails ENTRY_NOT_FOUND', () => {
  assert.throws(() => addAsset(db, {
    name: 'X', purchaseDate: '2024-01-01', purchasePriceCents: 100000,
    depreciationStartDate: '2024-01-01', recognitionDate: '2024-01-01',
    assetAccount: '1800', expenseAccount: '4600', entryId: 999,
  }), { code: 'ENTRY_NOT_FOUND' });
});

test('addAsset: dry-run writes nothing', () => {
  const r = addAsset(db, {
    name: 'Laptop', purchaseDate: '2024-01-15', purchasePriceCents: 200000,
    depreciationStartDate: '2024-02-01', recognitionDate: '2024-02-01',
    assetAccount: '1800', expenseAccount: '4600', dryRun: true,
  });
  assert.equal(r.dryRun, true);
  assert.equal(r.asset.months_left, 60);
  assert.equal(listAssets(db).length, 0);
});

// --- runs --------------------------------------------------------------------

test('runDue: books monthly depreciation, idempotent per asset-month', () => {
  addAsset(db, {
    name: 'Laptop', purchaseDate: '2024-01-15', purchasePriceCents: 200000,
    depreciationStartDate: '2024-02-01', recognitionDate: '2024-02-01',
    assetAccount: '1800', expenseAccount: '4600',
  });
  const r1 = runDue(db, { period: '2024-03', actor: 'agent:test' });
  assert.equal(r1.booked.length, 2); // 2024-02 + 2024-03
  const r2 = runDue(db, { period: '2024-03', actor: 'agent:test' });
  assert.equal(r2.booked.length, 0); // idempotent

  const entries = listEntries(db, { state: 'posted' });
  assert.equal(entries.length, 2);
  for (const e of entries) {
    assert.equal(e.source, 'assets');
    assert.ok(e.source_ref.startsWith('asset:'));
  }
});

test('runDue: paused assets do not run; resume restarts them', () => {
  const { asset } = addAsset(db, {
    name: 'Laptop', purchaseDate: '2024-01-15', purchasePriceCents: 200000,
    depreciationStartDate: '2024-02-01', recognitionDate: '2024-02-01',
    assetAccount: '1800', expenseAccount: '4600',
  });
  db.prepare("UPDATE assets SET status = 'paused' WHERE id = ?").run(asset.id);
  assert.equal(runDue(db, { period: '2024-03' }).booked.length, 0);
  db.prepare("UPDATE assets SET status = 'active' WHERE id = ?").run(asset.id);
  const r = runDue(db, { period: '2024-03' });
  assert.equal(r.booked.length, 2);
});

test('runDue: dry-run plans but books nothing', () => {
  addAsset(db, {
    name: 'Laptop', purchaseDate: '2024-01-15', purchasePriceCents: 200000,
    depreciationStartDate: '2024-02-01', recognitionDate: '2024-02-01',
    assetAccount: '1800', expenseAccount: '4600',
  });
  const plan = runDue(db, { period: '2024-03', dryRun: true });
  assert.equal(plan.plan.length, 1);
  assert.equal(plan.plan[0].periods.length, 2);
  assert.equal(listEntries(db).length, 0);
});

test('runDue: auto-completes to fully_depreciated at the residual', () => {
  addAsset(db, {
    name: 'Klein', purchaseDate: '2026-01-01', purchasePriceCents: 12000, // 120.00
    depreciationStartDate: '2026-02-01', recognitionDate: '2026-02-01',
    lifeMonths: 12, assetAccount: '1800', expenseAccount: '4600',
  });
  const r = runDue(db, { asOf: '2027-02-01' });
  assert.equal(r.booked.length, 12);
  const a = listAssets(db)[0];
  assert.equal(a.status, 'fully_depreciated');
  const total = register(db, { asOf: '2027-02-01' }).assets[0].total_cum_dep_cents;
  assert.equal(total, 12000);
});

test('runDue: books on cum-dep account when provided, else on the asset account', () => {
  addAsset(db, {
    name: 'A', purchaseDate: '2024-01-01', purchasePriceCents: 120000,
    depreciationStartDate: '2024-02-01', recognitionDate: '2024-02-01',
    assetAccount: '1800', cumDepAccount: '1500', expenseAccount: '4600',
  });
  addAsset(db, {
    name: 'B', purchaseDate: '2024-01-01', purchasePriceCents: 120000,
    depreciationStartDate: '2024-02-01', recognitionDate: '2024-02-01',
    assetAccount: '1850', expenseAccount: '4600',
  });
  runDue(db, { period: '2024-02' });
  const entries = listEntries(db, { state: 'posted' }).sort((a, b) => a.id - b.id);
  const legOf = (entry) => db.prepare('SELECT a.code FROM postings p JOIN accounts a ON a.id = p.account_id WHERE p.entry_id = ?').all(entry.id).map((x) => x.code).sort();
  assert.deepEqual(legOf(entries[0]), ['1500', '4600']); // asset A: cum-dep account leg
  assert.deepEqual(legOf(entries[1]), ['1850', '4600']); // asset B: asset account leg
});

// --- disposal ----------------------------------------------------------------

test('disposeAsset: sale with winst books the full entry and closes the asset', () => {
  const { asset } = addAsset(db, {
    name: 'Fotocamera', purchaseDate: '2023-05-10', purchasePriceCents: 120000,
    depreciationStartDate: '2023-06-01', recognitionDate: '2024-06-01',
    cumDepAtRecognitionCents: 25000, assetAccount: '1800', cumDepAccount: '1500',
    expenseAccount: '4600',
  });
  runDue(db, { period: '2024-07' }); // 2 runs of 19.79
  const r = disposeAsset(db, { id: asset.id, date: '2024-08-15', proceedsCents: 95000, actor: 'agent:test' });
  // book value = 1200 - (250 + 39.58) = 910.42 -> winst 39.58
  assert.equal(r.book_value_cents, 91042);
  assert.equal(r.result_cents, 3958);
  const sums = {};
  for (const p of r.postings) sums[p.code] = (sums[p.code] ?? 0) + p.amountCents;
  assert.deepEqual(sums, { 1100: 95000, 1500: 28958, 1800: -120000, 8100: -3958 });
  assert.equal(listAssets(db)[0].status, 'disposed');
  assert.equal(listAssets(db)[0].disposed_proceeds_cents, 95000);
  assert.equal(r.entry.state, 'posted');
  assert.equal(listEntries(db, { state: 'posted' }).length, 3); // 2 runs + disposal
});

test('disposeAsset: scrap (proceeds 0) books a verlies', () => {
  const { asset } = addAsset(db, {
    name: 'Printer', purchaseDate: '2024-01-01', purchasePriceCents: 50000,
    depreciationStartDate: '2024-02-01', recognitionDate: '2024-02-01',
    assetAccount: '1800', expenseAccount: '4600',
  });
  runDue(db, { period: '2024-02' });
  const r = disposeAsset(db, { id: asset.id, date: '2024-03-01', proceedsCents: 0, actor: 'agent:test' });
  // book value = 500 - 8.33 = 491.67 -> verlies 491.67 (debit 8100);
  // no cum-dep account -> the cum-dep leg lands on the asset account itself
  assert.equal(r.result_cents, -49167);
  const sums = {};
  for (const p of r.postings) sums[p.code] = (sums[p.code] ?? 0) + p.amountCents;
  assert.deepEqual(sums, { 1800: -49167, 8100: 49167 });
});

test('disposeAsset: rejects double disposal and bad dates', () => {
  const { asset } = addAsset(db, {
    name: 'X', purchaseDate: '2024-01-01', purchasePriceCents: 50000,
    depreciationStartDate: '2024-02-01', recognitionDate: '2024-02-01',
    assetAccount: '1800', expenseAccount: '4600',
  });
  disposeAsset(db, { id: asset.id, date: '2024-06-01', proceedsCents: 0 });
  assert.throws(() => disposeAsset(db, { id: asset.id, date: '2024-07-01', proceedsCents: 0 }), { code: 'ALREADY_DISPOSED' });
});

test('disposeAsset: dry-run books nothing', () => {
  const { asset } = addAsset(db, {
    name: 'X', purchaseDate: '2024-01-01', purchasePriceCents: 50000,
    depreciationStartDate: '2024-02-01', recognitionDate: '2024-02-01',
    assetAccount: '1800', expenseAccount: '4600',
  });
  const r = disposeAsset(db, { id: asset.id, date: '2024-06-01', proceedsCents: 0, dryRun: true });
  assert.equal(r.dryRun, true);
  assert.equal(listAssets(db)[0].status, 'active');
  assert.equal(listEntries(db).length, 0);
});

// --- register ----------------------------------------------------------------

test('register: book values and totals as of a date', () => {
  addAsset(db, {
    name: 'Laptop', purchaseDate: '2024-01-15', purchasePriceCents: 200000,
    depreciationStartDate: '2024-02-01', recognitionDate: '2024-02-01',
    assetAccount: '1800', expenseAccount: '4600',
  });
  runDue(db, { period: '2024-04' }); // 3 runs of 33.33
  const reg = register(db, { asOf: '2024-04-30' });
  assert.equal(reg.assets.length, 1);
  assert.equal(reg.assets[0].total_cum_dep_cents, 9999); // 3 x 33.33
  assert.equal(reg.assets[0].book_value_cents, 200000 - 9999);
  assert.equal(reg.assets[0].next_run_period, '2024-05');
  assert.equal(reg.totals.purchase_price_cents, 200000);
});

test('register: disposal dates and proceeds surface in the register', () => {
  const { asset } = addAsset(db, {
    name: 'X', purchaseDate: '2024-01-01', purchasePriceCents: 50000,
    depreciationStartDate: '2024-02-01', recognitionDate: '2024-02-01',
    assetAccount: '1800', expenseAccount: '4600',
  });
  disposeAsset(db, { id: asset.id, date: '2024-06-01', proceedsCents: 10000 });
  const a = register(db, { asOf: '2024-06-30' }).assets[0];
  assert.equal(a.status, 'disposed');
  assert.equal(a.disposed_date, '2024-06-01');
  assert.equal(a.disposed_proceeds_cents, 10000);
});

test('trial balance stays balanced through the whole lifecycle', () => {
  post(db, '2024-01-15', 'Aankoop', [{ code: '1800', amountCents: 200000 }, { code: '1100', amountCents: -200000 }]);
  const { asset } = addAsset(db, {
    name: 'Laptop', purchaseDate: '2024-01-15', purchasePriceCents: 200000,
    depreciationStartDate: '2024-02-01', recognitionDate: '2024-02-01',
    assetAccount: '1800', expenseAccount: '4600',
  });
  runDue(db, { asOf: '2024-12-01' });
  disposeAsset(db, { id: asset.id, date: '2025-01-02', proceedsCents: 90000 });
  const tb = db.prepare(`
    SELECT COALESCE(SUM(CASE WHEN p.amount_cents > 0 THEN p.amount_cents ELSE 0 END),0) AS d,
           COALESCE(SUM(CASE WHEN p.amount_cents < 0 THEN -p.amount_cents ELSE 0 END),0) AS c
    FROM postings p JOIN journal_entries e ON e.id = p.entry_id AND e.state = 'posted'
  `).get();
  assert.equal(tb.d, tb.c);
});

test('disposeAsset: entry + asset status are atomic (no orphaned entry on rollback)', () => {
  const { asset } = addAsset(db, {
    name: 'Atomic', purchaseDate: '2024-01-01', purchasePriceCents: 50000,
    depreciationStartDate: '2024-02-01', recognitionDate: '2024-02-01',
    assetAccount: '1800', expenseAccount: '4600',
  });
  const before = listEntries(db, { state: 'posted' }).length;
  // force the UPDATE assets step (inside the disposal transaction) to fail:
  // a trigger error on the status update rolls the whole tx back
  assert.throws(() => {
    db.prepare(`
      CREATE TRIGGER IF NOT EXISTS trg_fail_disposal BEFORE UPDATE OF status ON assets
      WHEN NEW.status = 'disposed' AND OLD.name = 'Atomic'
      BEGIN SELECT RAISE(ABORT, 'boom'); END
    `).run();
    disposeAsset(db, { id: asset.id, date: '2024-06-01', proceedsCents: 0 });
  }, /boom/);
  db.prepare('DROP TRIGGER trg_fail_disposal').run();
  // nothing persisted: no new posted entry, asset still active
  assert.equal(listEntries(db, { state: 'posted' }).length, before);
  assert.equal(listAssets(db)[0].status, 'active');
});
