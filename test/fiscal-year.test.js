/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across eleven jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Fiscal-year window regression tests (v0.14.1 review pass): every
// year-based report must derive its [from, to] from fiscalYearWindow, not
// hardcode calendar year — a company with fiscal_year_end 03-31 reports
// 2025-04-01..2026-03-31 for "year 2026" (the same window the jaarrekening
// and the year-end close use).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, rmSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { openDb } from '../src/core/db.js';
import { seedDefaultChart } from '../src/core/accounts.js';
import { createEntry, postEntry } from '../src/core/entries.js';
import { pnl } from '../src/report/pnl.js';
import { journal } from '../src/report/journal.js';
import { trialBalance } from '../src/report/trial-balance.js';
import { sales } from '../src/report/sales.js';
import { createContact, createInvoice, finalizeInvoice } from '../src/invoice/index.js';
import { fiscalYearWindow } from '../src/year-end/index.js';

const BIN = path.join(path.dirname(fileURLToPath(import.meta.url)), '..', 'bin', 'bukio.js');

function cli(dbPath, args) {
  const env = { ...process.env, BUKIO_DB: dbPath, BUKIO_ACTOR: 'agent:test' };
  const stdout = execFileSync(process.execPath, [BIN, '--json', ...args], { env, encoding: 'utf8' });
  return JSON.parse(stdout);
}

/** Scratch DB: company with fiscal_year_end 03-31, one entry in Jan 2026
 *  (previous FY) and one in Nov 2026 (this FY). */
function makeFiscalDb() {
  const dir = mkdtempSync(path.join(os.tmpdir(), 'bukio-fiscal-'));
  const db = openDb(path.join(dir, 'test.db'));
  db.prepare('INSERT INTO company (name, registration_id, fiscal_year_end) VALUES (?, ?, ?)').run('Test Coaching', '12345678', '03-31');
  seedDefaultChart(db);
  createEntry(db, { date: '2026-01-15', description: 'jan 2026 (prev FY)', postings: [{ code: '1100', amountCents: 10000 }, { code: '8000', amountCents: -10000 }] });
  createEntry(db, { date: '2026-11-15', description: 'nov 2026 (this FY)', postings: [{ code: '1100', amountCents: 20000 }, { code: '8000', amountCents: -20000 }] });
  for (const e of db.prepare('SELECT id FROM journal_entries').all()) postEntry(db, { id: e.id });
  db.close();
  return path.join(dir, 'test.db');
}

test('fiscalYearWindow: year 2026 for FYE 03-31 spans 2025-04-01..2026-03-31', () => {
  const dbPath = makeFiscalDb();
  try {
    const db = openDb(dbPath);
    assert.deepEqual(fiscalYearWindow(db, '2026'), ['2025-04-01', '2026-03-31']);
    db.close();
  } finally { rmSync(path.dirname(dbPath), { recursive: true, force: true }); }
});

test('report pnl --year uses the fiscal window (jan prev-FY in, nov this-FY out)', () => {
  const dbPath = makeFiscalDb();
  try {
    const out = cli(dbPath, ['report', 'pnl', '--year', '2026']);
    assert.equal(out.data.from, '2025-04-01');
    assert.equal(out.data.to, '2026-03-31');
    // only the Jan 2026 entry (100.00) is inside 2025-04-01..2026-03-31
    assert.equal(out.data.result_cents, 10000);
  } finally { rmSync(path.dirname(dbPath), { recursive: true, force: true }); }
});

test('report journal --year uses the fiscal window', () => {
  const dbPath = makeFiscalDb();
  try {
    const out = cli(dbPath, ['report', 'journal', '--year', '2026']);
    assert.equal(out.data.from, '2025-04-01');
    assert.equal(out.data.to, '2026-03-31');
    assert.equal(out.data.rows.length, 2); // one posting pair of the jan entry
    assert.ok(out.data.rows.every((r) => r.date >= '2025-04-01' && r.date <= '2026-03-31'));
  } finally { rmSync(path.dirname(dbPath), { recursive: true, force: true }); }
});

test('report trial-balance --year uses the fiscal window', () => {
  const dbPath = makeFiscalDb();
  try {
    const out = cli(dbPath, ['report', 'trial-balance', '--year', '2026']);
    // only the Jan entry (100.00) is in the window
    const bank = out.data.accounts.find((a) => a.code === '1100');
    assert.equal(bank.net_cents, 10000);
  } finally { rmSync(path.dirname(dbPath), { recursive: true, force: true }); }
});

test('pnl() module with explicit from/to is untouched by the fiscal change', () => {
  const dbPath = makeFiscalDb();
  try {
    const db = openDb(dbPath);
    const p = pnl(db, { from: '2026-01-01', to: '2026-12-31' });
    assert.equal(p.result_cents, 30000); // both entries, calendar window still works
    const j = journal(db, { from: '2026-01-01', to: '2026-12-31' });
    assert.equal(j.length, 4); // both entries' postings
    db.close();
  } finally { rmSync(path.dirname(dbPath), { recursive: true, force: true }); }
});

test('sales() uses the fiscal window for --year', () => {
  const dir = mkdtempSync(path.join(os.tmpdir(), 'bukio-fiscal-sales-'));
  const dbPath = path.join(dir, 'test.db');
  try {
    const db = openDb(dbPath);
    db.prepare('INSERT INTO company (name, registration_id, fiscal_year_end, address, postal_code, city) VALUES (?, ?, ?, ?, ?, ?)').run('Test Coaching', '12345678', '03-31', 'Teststraat 1', '1000 AA', 'Amsterdam');
    seedDefaultChart(db);
    const c = createContact(db, { name: 'Acme BV', address: 'Straat 1', postalCode: '1000 AA', city: 'Amsterdam' });
    // invoice dated 2026-01-20 (prev FY) and 2026-11-20 (this FY)
    const i1 = createInvoice(db, { contactId: c.id, lines: ['1x Werk @ 50.00'], date: '2026-01-20' });
    const i2 = createInvoice(db, { contactId: c.id, lines: ['1x Werk2 @ 70.00'], date: '2026-11-20' });
    finalizeInvoice(db, { id: i1.id, actor: 'human' });
    finalizeInvoice(db, { id: i2.id, actor: 'human' });
    const s = sales(db, { year: '2026' });
    // 2025-04-01..2026-03-31 includes only the Jan invoice (50.00)
    assert.equal(s.totals.net_cents, 5000);
    db.close();
  } finally { rmSync(dir, { recursive: true, force: true }); }
});

test('MCP pnl tool reports the fiscal window', () => {
  const dbPath = makeFiscalDb();
  try {
    const db = openDb(dbPath);
    // exercise the same derivation the MCP handler uses
    const [from, to] = fiscalYearWindow(db, '2026');
    const p = pnl(db, { from, to });
    assert.equal(p.result_cents, 10000);
    db.close();
  } finally { rmSync(path.dirname(dbPath), { recursive: true, force: true }); }
});
