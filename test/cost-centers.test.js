/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across thirty-one jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */
import { test, beforeEach } from 'node:test';
import assert from 'node:assert/strict';
import { openDb } from '../src/core/db.js';
import { seedDefaultChart } from '../src/core/accounts.js';
import { createEntry, postEntry, reverseEntry, getEntry, parsePostingSpecs } from '../src/core/entries.js';
import {
  createCostCenter, getCostCenterByCode, listCostCenters, deactivateCostCenter,
  reactivateCostCenter, parsePostingSpecsWithCostCenter, resolveCostCenterIds,
} from '../src/core/cost-centers.js';
import { trialBalance } from '../src/report/trial-balance.js';
import { costCenterReport } from '../src/report/cost-center.js';

let db;

beforeEach(() => {
  db = openDb(':memory:');
  seedDefaultChart(db);
});

// ── registry ────────────────────────────────────────────────────────────
test('createCostCenter: basic CRUD', () => {
  const cc = createCostCenter(db, { code: 'ADM', name: 'Administration' });
  assert.equal(cc.code, 'ADM');
  assert.equal(cc.name, 'Administration');
  assert.equal(cc.active, 1);
  assert.equal(getCostCenterByCode(db, 'ADM') != null, true);
  assert.equal(listCostCenters(db).length, 1);
});

test('createCostCenter: rejects duplicate code', () => {
  createCostCenter(db, { code: 'ADM', name: 'Administration' });
  assert.throws(() => createCostCenter(db, { code: 'ADM', name: 'Admin' }), { code: 'COST_CENTER_EXISTS' });
});

test('createCostCenter: invalid code is rejected', () => {
  assert.throws(() => createCostCenter(db, { code: 'x', name: 'X' }), { code: 'INVALID_CODE' });
});

test('deactivateCostCenter: blocks new bookings but history stays', () => {
  createCostCenter(db, { code: 'ADM', name: 'Administration' });
  const updated = deactivateCostCenter(db, 'ADM');
  assert.equal(updated.active, 0);
  // resolveCostCenterIds rejects inactive
  assert.throws(() => resolveCostCenterIds(db, { costCenterCode: 'ADM' }), { code: 'COST_CENTER_INACTIVE' });
  reactivateCostCenter(db, 'ADM');
  // after reactivation, resolvable again
  const ids = resolveCostCenterIds(db, { costCenterCode: 'ADM' });
  assert.ok(ids.costCenterId > 0);
});

// ── posting spec parsing ────────────────────────────────────────────────
test('parsePostingSpecsWithCostCenter: plain specs (no CC)', () => {
  const specs = parsePostingSpecsWithCostCenter(['8000:-100.00,3000:100.00']);
  assert.equal(specs.length, 2);
  assert.equal(specs[0].code, '8000');
  assert.equal(specs[0].amountCents, -10000);
  assert.equal(specs[0].costCenterCode, null);
});

test('parsePostingSpecsWithCostCenter: @CC suffix', () => {
  const specs = parsePostingSpecsWithCostCenter(['8000:-100.00@ADM,3000:100.00']);
  assert.equal(specs[0].costCenterCode, 'ADM');
  assert.equal(specs[1].costCenterCode, null);
});

// ── entry engine ────────────────────────────────────────────────────────
test('createEntry: cost center is carried through and surfaced', () => {
  createCostCenter(db, { code: 'ADM', name: 'Admin' });
  const e = createEntry(db, {
    date: '2026-08-04', description: 'CC test',
    postings: [
      { code: '8000', amountCents: -10000, costCenterCode: 'ADM' },
      { code: '3000', amountCents: 10000 },
    ],
  });
  assert.equal(e.postings[0].cost_center_code, 'ADM');
  assert.equal(e.postings[1].cost_center_code, null);
});

test('reverseEntry: cost center carried to contra-entry', () => {
  createCostCenter(db, { code: 'ADM', name: 'Admin' });
  const e = createEntry(db, {
    date: '2026-08-04', description: 'CC test',
    postings: [
      { code: '8000', amountCents: -10000, costCenterCode: 'ADM' },
      { code: '3000', amountCents: 10000 },
    ],
  });
  postEntry(db, { id: e.id });
  const rev = reverseEntry(db, { id: e.id });
  assert.equal(rev.postings[0].cost_center_code, 'ADM');
  // reversal balances
  const sum = rev.postings.reduce((s, p) => s + p.amount_cents, 0);
  assert.equal(sum, 0);
});

// ── report ──────────────────────────────────────────────────────────────
test('costCenterReport: groups postings by cost center', () => {
  createCostCenter(db, { code: 'ADM', name: 'Admin' });
  createCostCenter(db, { code: 'SALES', name: 'Sales' });
  // Two entries: one ADM expense, one SALES revenue, one unassigned
  createEntry(db, {
    date: '2026-08-04', description: 'adm expense',
    postings: [{ code: '4700', amountCents: 20000, costCenterCode: 'ADM' }, { code: '1100', amountCents: -20000 }],
  });
  createEntry(db, {
    date: '2026-08-05', description: 'sales revenue',
    postings: [{ code: '1100', amountCents: 30000 }, { code: '8000', amountCents: -30000, costCenterCode: 'SALES' }],
  });
  createEntry(db, {
    date: '2026-08-06', description: 'unassigned expense',
    postings: [{ code: '4700', amountCents: 5000 }, { code: '1100', amountCents: -5000 }],
  });
  // Post all
  for (const e of db.prepare('SELECT id FROM journal_entries').all()) {
    postEntry(db, { id: e.id });
  }
  const r = costCenterReport(db, { year: '2026' });
  assert.equal(r.centers.length, 3); // ADM, SALES, unassigned
  const adm = r.centers.find((c) => c.cost_center_code === 'ADM');
  assert.ok(adm);
  assert.equal(adm.result_cents < 0, true); // expense only
  const sales = r.centers.find((c) => c.cost_center_code === 'SALES');
  assert.ok(sales);
  assert.equal(sales.result_cents > 0, true); // revenue only
  const unassigned = r.centers.find((c) => c.cost_center_code == null);
  assert.ok(unassigned);
});

test('trial-balance: still balanced after CC-tagged entries', () => {
  createCostCenter(db, { code: 'ADM', name: 'Admin' });
  createEntry(db, {
    date: '2026-08-04', description: 'CC entry',
    postings: [{ code: '8000', amountCents: -50000, costCenterCode: 'ADM' }, { code: '3000', amountCents: 50000 }],
  });
  const e = db.prepare('SELECT id FROM journal_entries').get();
  postEntry(db, { id: e.id });
  const tb = trialBalance(db, { year: '2026' });
  assert.equal(tb.balanced, true);
});

test('costCenterReport: period filtering (--from/--to)', () => {
  createCostCenter(db, { code: 'ADM', name: 'Admin' });
  // Jan entry (outside filter) + Aug entry (inside filter)
  createEntry(db, {
    date: '2026-01-15', description: 'jan',
    postings: [{ code: '4700', amountCents: 10000, costCenterCode: 'ADM' }, { code: '1100', amountCents: -10000 }],
  });
  createEntry(db, {
    date: '2026-08-04', description: 'aug',
    postings: [{ code: '4700', amountCents: 20000, costCenterCode: 'ADM' }, { code: '1100', amountCents: -20000 }],
  });
  for (const e of db.prepare('SELECT id FROM journal_entries').all()) postEntry(db, { id: e.id });
  // Filter to Aug only
  const r = costCenterReport(db, { from: '2026-08-01', to: '2026-08-31' });
  const adm = r.centers.find((c) => c.cost_center_code === 'ADM');
  assert.ok(adm);
  // Only the Aug expense (20000) should appear — not Jan
  const total = adm.accounts.reduce((s, a) => s + a.amount_cents, 0);
  assert.equal(total, 20000);
});

test('costCenterReport: --cost-center filter returns only that center', () => {
  createCostCenter(db, { code: 'ADM', name: 'Admin' });
  createCostCenter(db, { code: 'SALES', name: 'Sales' });
  createEntry(db, {
    date: '2026-08-04', description: 'adm',
    postings: [{ code: '4700', amountCents: 10000, costCenterCode: 'ADM' }, { code: '1100', amountCents: -10000 }],
  });
  createEntry(db, {
    date: '2026-08-05', description: 'sales',
    postings: [{ code: '4700', amountCents: 20000, costCenterCode: 'SALES' }, { code: '1100', amountCents: -20000 }],
  });
  for (const e of db.prepare('SELECT id FROM journal_entries').all()) postEntry(db, { id: e.id });
  const r = costCenterReport(db, { year: '2026', costCenter: 'ADM' });
  assert.equal(r.centers.length, 1);
  assert.equal(r.centers[0].cost_center_code, 'ADM');
});
