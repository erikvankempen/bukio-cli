/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Round-3 review regression tests: recurring pause/resume dry-run crash,
// audit --format json without --json, bank match post dry-run validation.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { openDb } from '../src/core/db.js';
import { seedDefaultChart } from '../src/core/accounts.js';
import { createEntry, postEntry } from '../src/core/entries.js';
import { importTransactions } from '../src/bank/index.js';

const BIN = path.join(path.dirname(fileURLToPath(import.meta.url)), '..', 'bin', 'bukio.js');

function run(dbPath, args, { expectFail = false } = {}) {
  const env = { ...process.env, BUKIO_DB: dbPath, BUKIO_ACTOR: 'agent:test' };
  try {
    const stdout = execFileSync(process.execPath, [BIN, ...args], { env, encoding: 'utf8' });
    return { code: 0, out: JSON.parse(stdout) };
  } catch (err) {
    if (expectFail) return { code: err.status, out: JSON.parse(err.stdout), err: err.stderr };
    throw err;
  }
}

function tmpDb() {
  const dir = mkdtempSync(path.join(os.tmpdir(), 'bukio-r3-'));
  return path.join(dir, 'test.db');
}

test('recurring pause --dry-run and resume --dry-run render a plan (no fmtTemplate crash)', () => {
  const dbPath = tmpDb();
  run(dbPath, ['init', '--name', 'Demo BV', '--kvk', '12345678', '--json']);
  run(dbPath, ['recurring', 'add', '--name', 'Huur', '--postings', '4300:1000.00,1100:-1000.00', '--frequency', 'monthly', '--start', '2026-01-01', '--json']);
  // dry-run: must return a plan, not crash on fmtTemplate(plan-without-postings)
  const p = run(dbPath, ['recurring', 'pause', '--id', '1', '--dry-run', '--json']);
  assert.equal(p.code, 0);
  assert.equal(p.out.data.template.dryRun, true);
  assert.equal(p.out.data.template.action, 'recurring.paused');
  assert.equal(p.out.data.template.id, '1');
  const r = run(dbPath, ['recurring', 'resume', '--id', '1', '--dry-run', '--json']);
  assert.equal(r.code, 0);
  assert.equal(r.out.data.template.dryRun, true);
  assert.equal(r.out.data.template.action, 'recurring.active');
  assert.equal(r.out.data.template.id, '1');
  // execute still works
  const real = run(dbPath, ['recurring', 'pause', '--id', '1', '--json']);
  assert.equal(real.code, 0);
  assert.equal(real.out.data.template.status, 'paused');
});

test('audit --format json prints JSON even without the global --json flag', () => {
  const dbPath = tmpDb();
  run(dbPath, ['init', '--name', 'Demo BV', '--kvk', '12345678', '--json']);
  const env = { ...process.env, BUKIO_DB: dbPath, BUKIO_ACTOR: 'agent:test' };
  const stdout = execFileSync(process.execPath, [BIN, 'audit', '--format', 'json'], { env, encoding: 'utf8' });
  const parsed = JSON.parse(stdout);
  assert.equal(parsed.ok, true);
  assert.ok(Array.isArray(parsed.data.entries));
  assert.ok(parsed.data.entries.length > 0);
});

test('bank match post --dry-run rejects an already-matched transaction and a missing account', () => {
  const dbPath = tmpDb();
  const db = openDb(dbPath);
  db.prepare('INSERT INTO company (name, registration_id) VALUES (?, ?)').run('Demo BV', '12345678');
  seedDefaultChart(db);
  db.prepare(`INSERT INTO bank_accounts (iban, account_code, name) VALUES (?, ?, ?)`).run('NL91ABNA0417164300', '1100', 'Betaalrekening');
  importTransactions(db, {
    iban: 'NL91ABNA0417164300',
    transactions: [
      { date: '2026-07-01', amount_cents: -50000, counterparty: 'Leverancier', description: 'F1', bank_ref: 'REF-A', iban: 'NL91ABNA0417164300' },
    ],
  });
  // match the tx first → state becomes matched
  const e = createEntry(db, {
    date: '2026-06-25', description: 'Betaling A',
    postings: [{ code: '1100', amountCents: -50000 }, { code: '4300', amountCents: 50000 }],
  });
  postEntry(db, { id: e.id });
  db.prepare("UPDATE bank_transactions SET state = 'matched' WHERE id = 1").run();
  db.close();

  // already-matched tx → ALREADY_MATCHED even in dry-run
  const r1 = run(dbPath, ['bank', 'match', 'post', '--tx', '1', '--account', '4300', '--dry-run', '--json'], { expectFail: true });
  assert.equal(r1.code, 1);
  assert.equal(r1.out.error.code, 'ALREADY_MATCHED');

  // reset to unmatched, then a nonexistent account → ACCOUNT_NOT_FOUND
  const db2 = openDb(dbPath);
  db2.prepare("UPDATE bank_transactions SET state = 'unmatched' WHERE id = 1").run();
  db2.close();
  const r2 = run(dbPath, ['bank', 'match', 'post', '--tx', '1', '--account', '9999', '--dry-run', '--json'], { expectFail: true });
  assert.equal(r2.code, 1);
  assert.equal(r2.out.error.code, 'ACCOUNT_NOT_FOUND');
});

test('vat book --dry-run rejects unbalanced postings (parity with entry add)', () => {
  const dbPath = tmpDb();
  run(dbPath, ['init', '--name', 'Demo BV', '--kvk', '12345678', '--legal-form', 'bv', '--vat', 'on', '--json']);
  // a spec that cannot balance: 100 debit with no contra leg
  const r = run(dbPath, ['vat', 'book', '--date', '2026-01-10', '--desc', 'x', '--postings', '8000:-100.00@21', '--dry-run', '--json'], { expectFail: true });
  assert.equal(r.code, 1);
  assert.equal(r.out.error.code, 'UNBALANCED');
});
