import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, existsSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { openDb } from '../src/core/db.js';

const BIN = path.join(path.dirname(fileURLToPath(import.meta.url)), '..', 'bin', 'bukio.js');

function run(dbPath, args, { expectFail = false } = {}) {
  const env = { ...process.env, BUKIO_DB: dbPath };
  try {
    const stdout = execFileSync(process.execPath, [BIN, ...args], { env, encoding: 'utf8' });
    return { code: 0, out: JSON.parse(stdout) };
  } catch (err) {
    if (expectFail) {
      return { code: err.status, out: JSON.parse(err.stdout), err: err.stderr };
    }
    throw err;
  }
}

function tmpDb() {
  const dir = mkdtempSync(path.join(os.tmpdir(), 'bukio-cli-test-'));
  return path.join(dir, 'test.db');
}

test('init --dry-run: shows plan, creates nothing', () => {
  const dbPath = tmpDb();
  const { code, out } = run(dbPath, ['init', '--name', 'Demo BV', '--kvk', '12345678', '--legal-form', 'bv', '--vat', 'on', '--dry-run', '--json']);
  assert.equal(code, 0);
  assert.equal(out.ok, true);
  assert.equal(out.data.dryRun, true);
  assert.equal(out.data.company.name, 'Demo BV');
  assert.equal(out.data.company.vat_module, 1);
  assert.equal(existsSync(dbPath), false);
});

test('init: creates company + 14-account chart', () => {
  const dbPath = tmpDb();
  const { out } = run(dbPath, ['init', '--name', 'Demo BV', '--kvk', '12345678', '--legal-form', 'bv', '--vat', 'on', '--json']);
  assert.equal(out.ok, true);
  assert.equal(out.data.chart.accounts, 14);
  assert.equal(out.data.chart.created, 14);

  const db = openDb(dbPath);
  const company = db.prepare('SELECT * FROM company').get();
  assert.equal(company.name, 'Demo BV');
  assert.equal(company.legal_form, 'bv');
  assert.equal(company.vat_module, 1);
  db.close();
});

test('init: second init fails with ALREADY_INITIALISED', () => {
  const dbPath = tmpDb();
  run(dbPath, ['init', '--name', 'A', '--json']);
  const { code, out } = run(dbPath, ['init', '--name', 'B', '--json'], { expectFail: true });
  assert.equal(code, 1);
  assert.equal(out.ok, false);
  assert.equal(out.error.code, 'ALREADY_INITIALISED');
});

test('entry add --dry-run: plans without writing', () => {
  const dbPath = tmpDb();
  run(dbPath, ['init', '--name', 'A', '--json']);
  const { out } = run(dbPath, [
    'entry', 'add', '--date', '2026-08-04', '--desc', 'Startkapitaal',
    '--postings', '1100:10000.00,3000:-10000.00', '--dry-run', '--json',
  ]);
  assert.equal(out.ok, true);
  assert.equal(out.data.dryRun, true);
  assert.equal(out.data.sum, '0.00');
  assert.equal(out.data.account_validation, 'ok');

  const db = openDb(dbPath);
  assert.equal(db.prepare('SELECT COUNT(*) c FROM journal_entries').get().c, 0);
  db.close();
});

test('entry add: rejects malformed posting spec and unknown account', () => {
  const dbPath = tmpDb();
  run(dbPath, ['init', '--name', 'A', '--json']);
  const bad1 = run(dbPath, ['entry', 'add', '--desc', 'x', '--postings', 'garbage', '--json'], { expectFail: true });
  assert.equal(bad1.out.error.code, 'INVALID_POSTING');
  const bad2 = run(dbPath, ['entry', 'add', '--desc', 'x', '--postings', '9999:1.00,3000:-1.00', '--json'], { expectFail: true });
  assert.equal(bad2.out.error.code, 'ACCOUNT_NOT_FOUND');
  const bad3 = run(dbPath, ['entry', 'add', '--desc', 'x', '--postings', '1100:5.00,3000:-4.00', '--json'], { expectFail: true });
  assert.equal(bad3.out.error.code, 'UNBALANCED');
});

test('entry add --post + trial balance + audit end-to-end', () => {
  const dbPath = tmpDb();
  run(dbPath, ['init', '--name', 'Demo BV', '--legal-form', 'bv', '--json']);
  run(dbPath, [
    'entry', 'add', '--date', '2026-08-04', '--desc', 'Startkapitaal',
    '--postings', '1100:10000.00,3000:-10000.00', '--post', '--json',
  ]);
  run(dbPath, [
    'entry', 'add', '--date', '2026-08-05', '--desc', 'Kantoorartikelen',
    '--postings', '4300:250.00,1100:-250.00', '--post', '--json',
  ]);

  const tb = run(dbPath, ['report', 'trial-balance', '--json']).out.data;
  assert.equal(tb.balanced, true);
  assert.equal(tb.total_debit, '10250.00');
  assert.equal(tb.total_credit, '10250.00');
  assert.equal(tb.accounts.find((a) => a.code === '1100').net, '9750.00');

  // audit log is newest-first
  const audit = run(dbPath, ['audit', '--json']).out.data.entries;
  assert.deepEqual(audit.map((a) => a.action),
    ['entry.post', 'entry.create', 'entry.post', 'entry.create', 'company.init']);
});

test('entry reverse: contra-entry keeps the trial balance balanced', () => {
  const dbPath = tmpDb();
  run(dbPath, ['init', '--name', 'A', '--json']);
  const created = run(dbPath, [
    'entry', 'add', '--desc', 'Omzet', '--postings', '1100:121.00,8000:-121.00', '--post', '--json',
  ]).out.data;
  const id = created.id;

  const rev = run(dbPath, ['entry', 'reverse', '--id', String(id), '--reason', 'credit note', '--json']).out.data;
  assert.equal(rev.state, 'posted');
  assert.equal(rev.source, 'reversal');

  // A reversal is itself a balanced mirror entry: totals stay equal, nets go to zero.
  const tb = run(dbPath, ['report', 'trial-balance', '--json']).out.data;
  assert.equal(tb.balanced, true);
  assert.equal(tb.accounts.find((a) => a.code === '1100').net, '0.00');
  assert.equal(tb.accounts.find((a) => a.code === '8000').net, '0.00');
});

test('commands fail cleanly when no database exists', () => {
  const dbPath = tmpDb();
  const { code, out } = run(dbPath, ['report', 'trial-balance', '--json'], { expectFail: true });
  assert.equal(code, 1);
  assert.equal(out.error.code, 'NO_DATABASE');
});

test('--actor is recorded on entries and audit', () => {
  const dbPath = tmpDb();
  run(dbPath, ['init', '--name', 'A', '--json']);
  run(dbPath, [
    'entry', 'add', '--desc', 'agent posting', '--postings', '1100:10.00,3000:-10.00', '--post',
    '--actor', 'agent:hermes', '--json',
  ]);
  const audit = run(dbPath, ['audit', '--by', 'agent:hermes', '--json']).out.data.entries;
  assert.ok(audit.length >= 2);
  assert.ok(audit.every((a) => a.actor === 'agent:hermes'));
});
