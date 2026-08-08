/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, existsSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { openDb } from '../src/core/db.js';

const BIN = path.join(path.dirname(fileURLToPath(import.meta.url)), '..', 'bin', 'bukio.js');

function run(dbPath, args, { expectFail = false } = {}) {
  const env = { ...process.env, BUKIO_DB: dbPath, BUKIO_ACTOR: 'agent:test' };
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

/** Run and return raw stdout (for non-JSON output like CSV). */
function runText(dbPath, args) {
  const env = { ...process.env, BUKIO_DB: dbPath, BUKIO_ACTOR: 'agent:test' };
  return execFileSync(process.execPath, [BIN, ...args], { env, encoding: 'utf8' });
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

test('init: creates company + 30-account chart with VAT on', () => {
  const dbPath = tmpDb();
  const { out } = run(dbPath, ['init', '--name', 'Demo BV', '--kvk', '12345678', '--legal-form', 'bv', '--vat', 'on', '--json']);
  assert.equal(out.ok, true);
  assert.equal(out.data.chart.accounts, 31); // 29 default + 2 VAT accounts
  assert.equal(out.data.chart.created, 31);

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
    '--actor', 'agent:test', '--json',
  ]);
  const audit = run(dbPath, ['audit', '--by', 'agent:test', '--json']).out.data.entries;
  assert.ok(audit.length >= 2);
  assert.ok(audit.every((a) => a.actor === 'agent:test'));
});

test('account add/list/show/deactivate flow', () => {
  const dbPath = tmpDb();
  run(dbPath, ['init', '--name', 'A', '--json']);

  const added = run(dbPath, ['account', 'add', '--code', '5000', '--name', 'Testkosten', '--type', 'expense', '--normal-balance', 'debit', '--json']).out.data;
  assert.equal(added.code, '5000');
  assert.equal(added.rgs_code, null);

  const dup = run(dbPath, ['account', 'add', '--code', '5000', '--name', 'x', '--type', 'expense', '--normal-balance', 'debit', '--json'], { expectFail: true });
  assert.equal(dup.out.error.code, 'ACCOUNT_EXISTS');

  const list = run(dbPath, ['account', 'list', '--type', 'expense', '--json']).out.data.accounts;
  assert.equal(list.length, 14); // 13 default (incl. 4840 Koersverschillen) + 1 new

  run(dbPath, ['account', 'deactivate', '--code', '5000', '--json']);
  const blocked = run(dbPath, ['entry', 'add', '--desc', 'x', '--postings', '5000:1.00,1100:-1.00', '--json'], { expectFail: true });
  assert.equal(blocked.out.error.code, 'ACCOUNT_INACTIVE');

  run(dbPath, ['account', 'reactivate', '--code', '5000', '--json']);
  run(dbPath, ['entry', 'add', '--desc', 'x', '--postings', '5000:1.00,1100:-1.00', '--post', '--json']);
});

test('account import: dry-run validates, real import creates', () => {
  const dbPath = tmpDb();
  run(dbPath, ['init', '--name', 'A', '--json']);
  const csvPath = path.join(path.dirname(dbPath), 'chart.csv');
  writeFileSync(csvPath, [
    'code,name,type,normal_balance,rgs_code',
    '5000,Testkosten,expense,debit,WBED.42',
    '5100,Verkeerd,weird,debit,',
  ].join('\n'));

  const dry = run(dbPath, ['account', 'import', '--file', csvPath, '--dry-run', '--json']).out.data;
  assert.equal(dry.created, 1);
  assert.equal(dry.skipped, 1);

  const real = run(dbPath, ['account', 'import', '--file', csvPath, '--json']).out.data;
  assert.equal(real.created, 1);
  assert.equal(real.skipped, 1);
});

test('report balans/pnl/journal: JSON + CSV + XLSX export', () => {
  const dbPath = tmpDb();
  run(dbPath, ['init', '--name', 'A', '--json']);
  run(dbPath, ['entry', 'add', '--date', '2026-01-05', '--desc', 'Startkapitaal', '--postings', '1100:10000.00,3000:-10000.00', '--post', '--json']);
  run(dbPath, ['entry', 'add', '--date', '2026-02-10', '--desc', 'Omzet', '--postings', '1100:1210.00,8000:-1210.00', '--post', '--json']);
  run(dbPath, ['entry', 'add', '--date', '2026-03-01', '--desc', 'Kantoorartikelen', '--postings', '4300:250.00,1100:-250.00', '--post', '--json']);

  const b = run(dbPath, ['report', 'balans', '--as-of', '2026-12-31', '--json']).out.data;
  assert.equal(b.balanced, true);
  assert.equal(b.assets.total, '10960.00');
  assert.equal(b.liabilities_and_equity.result, '960.00');

  const p = run(dbPath, ['report', 'pnl', '--year', '2026', '--json']).out.data;
  assert.equal(p.revenue, '1210.00');
  assert.equal(p.costs, '250.00');
  assert.equal(p.result, '960.00');

  // CSV to stdout
  const csv = runText(dbPath, ['report', 'pnl', '--year', '2026', '--format', 'csv']);
  assert.match(csv, /^rgs,group,code,name,amount/m);

  // CSV to file
  const csvOut = path.join(path.dirname(dbPath), 'pnl.csv');
  runText(dbPath, ['report', 'journal', '--year', '2026', '--format', 'csv', '--out', csvOut]);
  const csvContent = readFileSync(csvOut, 'utf8');
  assert.match(csvContent, /^date,entry,description/s);
  assert.match(csvContent, /Kantoorartikelen/);

  // XLSX to file
  const xlsxOut = path.join(path.dirname(dbPath), 'pnl.xlsx');
  runText(dbPath, ['report', 'pnl', '--year', '2026', '--format', 'xlsx', '--out', xlsxOut]);
  assert.equal(existsSync(xlsxOut), true);
  assert.ok(statSync(xlsxOut).size > 1000);

  // xlsx without --out fails cleanly
  const noOut = run(dbPath, ['report', 'pnl', '--format', 'xlsx', '--json'], { expectFail: true });
  assert.equal(noOut.out.error.code, 'OUT_REQUIRED');
});

test('report balans --as-of is respected', () => {
  const dbPath = tmpDb();
  run(dbPath, ['init', '--name', 'A', '--json']);
  run(dbPath, ['entry', 'add', '--date', '2026-01-05', '--desc', 'Startkapitaal', '--postings', '1100:1000.00,3000:-1000.00', '--post', '--json']);
  run(dbPath, ['entry', 'add', '--date', '2026-06-01', '--desc', 'Omzet', '--postings', '1100:500.00,8000:-500.00', '--post', '--json']);

  const early = run(dbPath, ['report', 'balans', '--as-of', '2026-03-01', '--json']).out.data;
  assert.equal(early.as_of, '2026-03-01');
  assert.equal(early.assets.total, '1000.00'); // omzet not yet booked
  assert.equal(early.liabilities_and_equity.result, '0.00');

  const late = run(dbPath, ['report', 'balans', '--as-of', '2026-12-31', '--json']).out.data;
  assert.equal(late.as_of, '2026-12-31');
  assert.equal(late.assets.total, '1500.00');
  assert.equal(late.liabilities_and_equity.result, '500.00');
});

test('backup + restore roundtrip', () => {
  const dbPath = tmpDb();
  run(dbPath, ['init', '--name', 'A', '--json']);
  run(dbPath, ['entry', 'add', '--desc', 'x', '--postings', '1100:100.00,3000:-100.00', '--post', '--json']);

  const backupPath = path.join(path.dirname(dbPath), 'backup.db');
  const backup = run(dbPath, ['backup', '--out', backupPath, '--json']).out.data;
  assert.equal(existsSync(backup.path), true);
  assert.ok(backup.bytes > 0);

  // restore into a fresh file
  const restored = path.join(path.dirname(dbPath), 'restored.db');
  run(dbPath, ['restore', '--from', backupPath, '--to', restored, '--json']);
  const db = openDb(restored);
  assert.equal(db.prepare('SELECT name FROM company').get().name, 'A');
  assert.equal(db.prepare("SELECT COUNT(*) c FROM journal_entries WHERE state='posted'").get().c, 1);
  db.close();

  // restore into an existing initialised target requires --force
  const conflict = run(dbPath, ['restore', '--from', backupPath, '--to', dbPath, '--json'], { expectFail: true });
  assert.equal(conflict.out.error.code, 'RESTORE_EXISTS');
  run(dbPath, ['restore', '--from', backupPath, '--to', dbPath, '--force', '--json']);

  // invalid inputs
  const bad = run(dbPath, ['restore', '--from', '/nonexistent.db', '--json'], { expectFail: true });
  assert.equal(bad.out.error.code, 'FILE_NOT_FOUND');
  const txt = path.join(path.dirname(dbPath), 'junk.txt');
  writeFileSync(txt, 'not a database');
  const bad2 = run(dbPath, ['restore', '--from', txt, '--json'], { expectFail: true });
  assert.equal(bad2.out.error.code, 'INVALID_BACKUP');
  const same = run(dbPath, ['restore', '--from', backupPath, '--to', backupPath, '--json'], { expectFail: true });
  assert.equal(same.out.error.code, 'SAME_FILE');
});

const CAMT_FIXTURE = (iban) => `<?xml version="1.0" encoding="UTF-8"?>
<Document xmlns="urn:iso:std:iso:20022:tech:xsd:camt.053.001.02">
  <BkToCstmrStmt><Stmt>
    <Acct><Id><IBAN>${iban}</IBAN></Id></Acct>
    <Ntry>
      <Amt>100.00</Amt><CdtDbtInd>CRDT</CdtDbtInd><BookgDt><Dt>2026-06-01</Dt></BookgDt>
      <NtryDtls><TxDtls><RltdPties><Dbtr><Nm>ACME B.V.</Nm></Dbtr></RltdPties>
      <RmtInf><Ustrd>Factuur 2026-001</Ustrd></RmtInf></TxDtls></NtryDtls>
    </Ntry>
    <Ntry>
      <Amt>25.50</Amt><CdtDbtInd>DBIT</CdtDbtInd><BookgDt><Dt>2026-06-02</Dt></BookgDt>
      <NtryDtls><TxDtls><RltdPties><Cdtr><Nm>Kantoorwinkel BV</Nm></Cdtr></RltdPties>
      <RmtInf><Ustrd>Kantoorartikelen</Ustrd></RmtInf></TxDtls></NtryDtls>
    </Ntry>
  </Stmt></BkToCstmrStmt>
</Document>`;

const RABO_CSV_FIXTURE = (iban) => [
  'Datum;Naam / Omschrijving;Rekening;Tegenrekening;Code;Af Bij;Bedrag (EUR);MutatieSoort;Mededelingen',
  `2026-06-01;ACME B.V.;${iban};NL00RABO0123456789;GT;Bij;100,00;Overschrijving;Factuur 2026-001`,
  `2026-06-02;Kantoorwinkel BV;${iban};NL00RABO9876543210;GT;Af;25,50;Overschrijving;Kantoorartikelen`,
].join('\n');

const IBAN = 'NL91ABNA0417164300';

test('bank import (CAMT + CSV), idempotency, match --post, ignore', () => {
  const dbPath = tmpDb();
  run(dbPath, ['init', '--name', 'A', '--json']);
  const dir = path.dirname(dbPath);

  const camtPath = path.join(dir, 'stmt.xml');
  writeFileSync(camtPath, CAMT_FIXTURE(IBAN));
  const imp = run(dbPath, ['bank', 'import', '--file', camtPath, '--iban', IBAN, '--json']).out.data;
  assert.equal(imp.imported, 2);

  // idempotent re-import
  const again = run(dbPath, ['bank', 'import', '--file', camtPath, '--iban', IBAN, '--json']).out.data;
  assert.equal(again.imported, 0);
  assert.equal(again.duplicates, 2);

  // dry-run preview
  const dry = run(dbPath, ['bank', 'import', '--file', camtPath, '--iban', IBAN, '--dry-run', '--json']).out.data;
  assert.equal(dry.imported, 0);
  assert.equal(dry.duplicates, 2);

  // CSV import with Dutch amounts
  const csvPath = path.join(dir, 'rabo.csv');
  writeFileSync(csvPath, RABO_CSV_FIXTURE(IBAN));
  const csvImp = run(dbPath, ['bank', 'import', '--file', csvPath, '--iban', IBAN, '--json']).out.data;
  assert.equal(csvImp.imported, 0); // same transactions, hashes match

  const txs = run(dbPath, ['bank', 'transactions', '--state', 'unmatched', '--json']).out.data.transactions;
  assert.equal(txs.length, 2);
  const income = txs.find((t) => t.amount_cents > 0);

  const posted = run(dbPath, ['bank', 'match', 'post', '--tx', String(income.id), '--account', '8000', '--json']).out.data;
  assert.equal(posted.entry_id, 1);
  assert.equal(posted.state, 'posted');

  const accounts = run(dbPath, ['bank', 'list', '--json']).out.data.accounts;
  assert.equal(accounts[0].balance, '74.50');
  assert.equal(accounts[0].unmatched_count, 1);

  const remaining = run(dbPath, ['bank', 'transactions', '--state', 'unmatched', '--json']).out.data.transactions[0];
  run(dbPath, ['bank', 'ignore', '--tx', String(remaining.id), '--json']);
  assert.equal(run(dbPath, ['bank', 'transactions', '--state', 'unmatched', '--json']).out.data.transactions.length, 0);

  const tb = run(dbPath, ['report', 'trial-balance', '--json']).out.data;
  assert.equal(tb.balanced, true);
});

test('bank match --auto links posted entries (exact)', () => {
  const dbPath = tmpDb();
  run(dbPath, ['init', '--name', 'A', '--json']);
  run(dbPath, ['entry', 'add', '--date', '2026-06-01', '--desc', 'Factuur 2026-001', '--postings', '1100:100.00,8000:-100.00', '--post', '--json']);
  const camtPath = path.join(path.dirname(dbPath), 'stmt.xml');
  writeFileSync(camtPath, CAMT_FIXTURE(IBAN));
  run(dbPath, ['bank', 'import', '--file', camtPath, '--iban', IBAN, '--json']);

  const dry = run(dbPath, ['bank', 'match', 'auto', '--dry-run', '--json']).out.data;
  assert.equal(dry.matched.length, 1);
  assert.equal(dry.matched[0].method, 'exact');
  assert.equal(dry.matched[0].entry_id, 1);

  const real = run(dbPath, ['bank', 'match', 'auto', '--json']).out.data;
  assert.equal(real.matched.length, 1);
  assert.equal(run(dbPath, ['bank', 'transactions', '--state', 'matched', '--json']).out.data.transactions.length, 1);
});

test('vat enable/book/readout/mark-filed end-to-end', () => {
  const dbPath = tmpDb();
  run(dbPath, ['init', '--name', 'A', '--vat', 'on', '--json']);

  const codes = run(dbPath, ['vat', 'codes', '--json']).out.data.codes;
  assert.equal(codes.length, 8);

  run(dbPath, ['vat', 'book', '--date', '2026-04-10', '--desc', 'Factuur 2026-001', '--postings', '1100:121.00,8000:-100.00@21', '--post', '--json']);
  run(dbPath, ['vat', 'book', '--date', '2026-05-15', '--desc', 'Kantoorartikelen', '--postings', '4300:50.00@21,1100:-60.50', '--post', '--json']);

  const r = run(dbPath, ['vat', 'readout', '--period', '2026-Q2', '--json']).out.data;
  assert.equal(r.fields['1a'].amount, '100.00');
  assert.equal(r.fields['5a'].amount, '21.00');
  assert.equal(r.fields['5b'].amount, '10.50');
  assert.equal(r.to_pay, '10.50');

  run(dbPath, ['vat', 'readout', '--period', '2026-Q2', '--mark-filed', '--json']);

  const tb = run(dbPath, ['report', 'trial-balance', '--json']).out.data;
  assert.equal(tb.balanced, true);
});

test('vat: module off blocks book, enable works on existing company', () => {
  const dbPath = tmpDb();
  run(dbPath, ['init', '--name', 'B', '--json']); // vat off

  const err = run(dbPath, ['vat', 'book', '--date', '2026-04-10', '--desc', 'x', '--postings', '1100:121.00,8000:-100.00@21', '--json'], { expectFail: true });
  assert.equal(err.out.error.code, 'VAT_MODULE_OFF');

  run(dbPath, ['vat', 'enable', '--json']);
  assert.equal(run(dbPath, ['vat', 'codes', '--json']).out.data.codes.length, 8);
  run(dbPath, ['vat', 'book', '--date', '2026-04-10', '--desc', 'x', '--postings', '1100:121.00,8000:-100.00@21', '--post', '--json']);
});
