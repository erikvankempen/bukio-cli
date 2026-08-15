/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across eleven jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, existsSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import crypto from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { openDb } from '../src/core/db.js';
import { record, setPendingSignature } from '../src/audit/index.js';
import { buildDigest } from '../src/core/canonical.js';
import { sign } from '../src/core/sign.js';

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

test('report trial-balance csv: TOTAAL row net is 0.00 for a balanced ledger (regression)', () => {
  const dbPath = tmpDb();
  run(dbPath, ['init', '--name', 'A', '--json']);
  run(dbPath, [
    'entry', 'add', '--desc', 'Startkapitaal', '--postings', '1100:10000.00,3000:-10000.00', '--post', '--json',
  ]);
  const csvPath = path.join(path.dirname(dbPath), 'tb.csv');
  execFileSync(process.execPath, [BIN, '--json', 'report', 'trial-balance', '--format', 'csv', '--out', csvPath], {
    env: { ...process.env, BUKIO_DB: dbPath, BUKIO_ACTOR: 'agent:test' },
    encoding: 'utf8',
  });
  const csv = readFileSync(csvPath, 'utf8');
  const totalRow = csv.split('\n').find((l) => l.includes('TOTAAL'));
  assert.ok(totalRow, 'TOTAAL row must be present');
  // columns: code,account,type,debit,credit,net — net must be the difference, not the debit
  const cols = totalRow.split(',');
  assert.equal(cols[3], '10000.00'); // debit
  assert.equal(cols[4], '10000.00'); // credit
  assert.equal(cols[5], '0.00');     // net — used to wrongly show 10000.00
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
  assert.equal(added.taxonomy_code, null);

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
    'code,name,type,normal_balance,taxonomy_code',
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

test('report balance-sheet/pnl/journal: JSON + CSV + XLSX export', () => {
  const dbPath = tmpDb();
  run(dbPath, ['init', '--name', 'A', '--json']);
  run(dbPath, ['entry', 'add', '--date', '2026-01-05', '--desc', 'Startkapitaal', '--postings', '1100:10000.00,3000:-10000.00', '--post', '--json']);
  run(dbPath, ['entry', 'add', '--date', '2026-02-10', '--desc', 'Omzet', '--postings', '1100:1210.00,8000:-1210.00', '--post', '--json']);
  run(dbPath, ['entry', 'add', '--date', '2026-03-01', '--desc', 'Kantoorartikelen', '--postings', '4300:250.00,1100:-250.00', '--post', '--json']);

  const b = run(dbPath, ['report', 'balance-sheet', '--as-of', '2026-12-31', '--json']).out.data;
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

test('report balance-sheet --as-of is respected', () => {
  const dbPath = tmpDb();
  run(dbPath, ['init', '--name', 'A', '--json']);
  run(dbPath, ['entry', 'add', '--date', '2026-01-05', '--desc', 'Startkapitaal', '--postings', '1100:1000.00,3000:-1000.00', '--post', '--json']);
  run(dbPath, ['entry', 'add', '--date', '2026-06-01', '--desc', 'Omzet', '--postings', '1100:500.00,8000:-500.00', '--post', '--json']);

  const early = run(dbPath, ['report', 'balance-sheet', '--as-of', '2026-03-01', '--json']).out.data;
  assert.equal(early.as_of, '2026-03-01');
  assert.equal(early.assets.total, '1000.00'); // omzet not yet booked
  assert.equal(early.liabilities_and_equity.result, '0.00');

  const late = run(dbPath, ['report', 'balance-sheet', '--as-of', '2026-12-31', '--json']).out.data;
  assert.equal(late.as_of, '2026-12-31');
  assert.equal(late.assets.total, '1500.00');
  assert.equal(late.liabilities_and_equity.result, '500.00');
});

test('report balans stays available as a deprecated alias', () => {
  const dbPath = tmpDb();
  run(dbPath, ['init', '--name', 'A', '--json']);
  run(dbPath, ['entry', 'add', '--date', '2026-01-05', '--desc', 'Startkapitaal', '--postings', '1100:1000.00,3000:-1000.00', '--post', '--json']);
  const legacy = run(dbPath, ['report', 'balans', '--as-of', '2026-12-31', '--json']).out.data;
  const modern = run(dbPath, ['report', 'balance-sheet', '--as-of', '2026-12-31', '--json']).out.data;
  assert.equal(legacy.balanced, true);
  assert.equal(legacy.assets.total, modern.assets.total);
  assert.equal(legacy.liabilities_and_equity.result, modern.liabilities_and_equity.result);
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

test('account list: human mode renders without crashing (table import regression)', () => {
  const dbPath = tmpDb();
  run(dbPath, ['init', '--name', 'Demo BV', '--kvk', '12345678', '--legal-form', 'bv', '--vat', 'off', '--json']);
  // human mode (no --json): the render callback calls table() — a missing
  // import crashed with 'table is not defined' instead of listing accounts
  const out = execFileSync(process.execPath, [BIN, '--db', dbPath, 'account', 'list'], {
    env: { ...process.env, BUKIO_ACTOR: 'agent:test' }, encoding: 'utf8',
  });
  assert.match(out, /1100/);
});

test('update: fetches from origin/main via --repo (fixture only, never the live repo)', () => {
  const dir = mkdtempSync(path.join(os.tmpdir(), 'bukio-upd-cli-'));
  const origin = path.join(dir, 'origin.git');
  const sh = (cwd, args) => execFileSync('git', args, { cwd, encoding: 'utf8' }).replace(/\s+$/, '');
  sh(dir, ['init', '--bare', 'origin.git']);
  sh(dir, ['--git-dir=origin.git', 'symbolic-ref', 'HEAD', 'refs/heads/main']);
  const work1 = path.join(dir, 'work1');
  sh(dir, ['clone', origin, 'work1']);
  sh(work1, ['config', 'user.email', 't@t']);
  sh(work1, ['config', 'user.name', 'T']);
  writeFileSync(path.join(work1, 'package.json'), '{"name":"bukio-cli","version":"1.0.0"}\n');
  writeFileSync(path.join(work1, 'README.md'), 'v1\n');
  sh(work1, ['add', '.']);
  sh(work1, ['commit', '-m', 'v1']);
  sh(work1, ['push', '-u', 'origin', 'main']);
  const work2 = path.join(dir, 'work2');
  sh(dir, ['clone', origin, 'work2']);
  sh(work2, ['config', 'user.email', 't@t']);
  sh(work2, ['config', 'user.name', 'T']);
  writeFileSync(path.join(work2, 'README.md'), 'v2\n');
  sh(work2, ['add', '.']);
  sh(work2, ['commit', '-m', 'v2']);
  sh(work2, ['push', 'origin', 'main']);

  const dbPath = tmpDb(); // no company db exists -> update still works (audit skipped)
  const dry = run(dbPath, ['update', '--repo', work1, '--trust-remote', '--dry-run', '--json']);
  assert.equal(dry.out.data.incoming_count, 1);
  assert.equal(dry.out.data.warning, null);
  assert.equal(dry.out.data.up_to_date, false);

  // without --yes the overwrite is refused
  const refused = run(dbPath, ['update', '--repo', work1, '--trust-remote', '--json'], { expectFail: true });
  assert.equal(refused.out.error.code, 'UPDATE_CONFIRM_REQUIRED');

  const done = run(dbPath, ['update', '--repo', work1, '--trust-remote', '--yes', '--json']);
  assert.equal(done.out.data.updated, true);
  assert.equal(done.out.data.commits_applied, 1);
  assert.equal(readFileSync(path.join(work1, 'README.md'), 'utf8'), 'v2\n');
});

test('vat file + settle with a custom af-te-dragen account (--account 2515)', () => {
  const dbPath = tmpDb();
  run(dbPath, ['init', '--name', 'Demo BV', '--kvk', '12345678', '--legal-form', 'bv', '--vat', 'on', '--json']);
  run(dbPath, ['vat', 'book', '--date', '2026-07-01', '--desc', 'Omzet', '--postings', '1100:121.00,8000:-100.00@21', '--post', '--json']);
  // file to a custom account — the dry-run must show the same code
  const dry = run(dbPath, ['vat', 'file', '--account', '2515', '--period', '2026-Q3', '--dry-run', '--json']);
  assert.equal(dry.out.data.account, '2515');
  const filed = run(dbPath, ['vat', 'file', '--account', '2515', '--period', '2026-Q3', '--json']);
  assert.equal(filed.out.data.account, '2515');
  assert.equal(filed.out.data.liability_cents, 2100);
  // payment tx arrives; settle against the custom account
  const dir = mkdtempSync(path.join(os.tmpdir(), 'bukio-ob2-'));
  const camt = path.join(dir, 'ob.camt.xml');
  writeFileSync(camt, `<?xml version="1.0"?>
<Document xmlns="urn:iso:std:iso:20022:tech:xsd:camt.053.001.02">
  <BkToCstmrStmt><Stmt><Acct><Id><IBAN>NL91ABNA0417164300</IBAN></Id></Acct>
    <Ntry><Amt>21.00</Amt><CdtDbtInd>DBIT</CdtDbtInd><BookgDt><Dt>2026-07-25</Dt></BookgDt>
      <NtryDtls><TxDtls><RltdPties><Dbtr><Nm>Belastingdienst</Nm></Dbtr></RltdPties>
      <RmtInf><Ustrd>OB aangifte</Ustrd></RmtInf></TxDtls></NtryDtls></Ntry>
  </Stmt></BkToCstmrStmt>
</Document>`);
  run(dbPath, ['bank', 'add', '--iban', 'NL91ABNA0417164300', '--name', 'Rabobank', '--json']);
  run(dbPath, ['bank', 'import', '--file', camt, '--iban', 'NL91ABNA0417164300', '--json']);
  const txId = run(dbPath, ['bank', 'transactions', '--json']).out.data.transactions[0].id;
  const settled = run(dbPath, ['vat', 'settle', '--tx', String(txId), '--account', '2515', '--period', '2026-Q3', '--json']);
  assert.equal(settled.out.data.account, '2515');
  assert.equal(settled.out.data.difference_cents, 0); // 21.00 filed = 21.00 booked
  assert.equal(settled.out.data.tx.state, 'matched');
  // the default account was never touched
  const tb = run(dbPath, ['report', 'trial-balance', '--json']);
  assert.equal(tb.out.data.balanced, true);
});

test('vat file + vat settle end-to-end: filing moves the position, the payment cancels it with the rounding difference in the P&L', () => {
  const dbPath = tmpDb();
  run(dbPath, ['init', '--name', 'Demo BV', '--kvk', '12345678', '--legal-form', 'bv', '--vat', 'on', '--json']);
  run(dbPath, ['vat', 'book', '--date', '2026-07-01', '--desc', 'Omzet', '--postings', '1100:121.00,8000:-100.00@21', '--post', '--json']);
  run(dbPath, ['vat', 'book', '--date', '2026-07-05', '--desc', 'Inkoop', '--postings', '1100:-60.50,4300:50.00@21', '--post', '--json']);

  // dry-run file first — nothing written
  const dry = run(dbPath, ['vat', 'file', '--period', '2026-Q3', '--dry-run', '--json']);
  assert.equal(dry.out.data.dryRun, true);
  assert.equal(dry.out.data.liability_cents, 1050);

  const filed = run(dbPath, ['vat', 'file', '--period', '2026-Q3', '--json']);
  assert.ok(filed.out.data.entry_id > 0);
  assert.equal(filed.out.data.liability_cents, 1050);
  assert.equal(filed.out.data.owe, true);

  // the Belastingdienst payment arrives: €10.00 outgoing (filed 10.50 rounded down)
  const dir = mkdtempSync(path.join(os.tmpdir(), 'bukio-ob-'));
  const camt = path.join(dir, 'ob.camt.xml');
  writeFileSync(camt, `<?xml version="1.0"?>
<Document xmlns="urn:iso:std:iso:20022:tech:xsd:camt.053.001.02">
  <BkToCstmrStmt><Stmt><Acct><Id><IBAN>NL91ABNA0417164300</IBAN></Id></Acct>
    <Ntry><Amt>10.00</Amt><CdtDbtInd>DBIT</CdtDbtInd><BookgDt><Dt>2026-07-25</Dt></BookgDt>
      <NtryDtls><TxDtls><RltdPties><Dbtr><Nm>Belastingdienst</Nm></Dbtr></RltdPties>
      <RmtInf><Ustrd>OB aangifte</Ustrd></RmtInf></TxDtls></NtryDtls></Ntry>
  </Stmt></BkToCstmrStmt>
</Document>`);
  run(dbPath, ['bank', 'add', '--iban', 'NL91ABNA0417164300', '--name', 'Rabobank', '--json']);
  run(dbPath, ['bank', 'import', '--file', camt, '--iban', 'NL91ABNA0417164300', '--json']);
  const txId = run(dbPath, ['bank', 'transactions', '--json']).out.data.transactions[0].id;

  // dry-run settle — tx stays unmatched
  const settleDry = run(dbPath, ['vat', 'settle', '--tx', String(txId), '--period', '2026-Q3', '--dry-run', '--json']);
  assert.equal(settleDry.out.data.dryRun, true);
  assert.equal(settleDry.out.data.difference_cents, -50);
  assert.equal(run(dbPath, ['bank', 'transactions', '--json']).out.data.transactions[0].state, 'unmatched');

  const settled = run(dbPath, ['vat', 'settle', '--tx', String(txId), '--period', '2026-Q3', '--json']);
  assert.equal(settled.out.data.difference_cents, -50); // paid 50 cents less -> P&L gain
  assert.equal(settled.out.data.tx.state, 'matched'); // payment tx reconciled to the entry

  // the matched tx cannot be settled again
  const again = run(dbPath, ['vat', 'settle', '--tx', String(txId), '--json'], { expectFail: true });
  assert.equal(again.code, 1);
  assert.equal(again.out.error.code, 'ALREADY_MATCHED');

  // books stay balanced and the P&L carries the rounding gain
  const tb = run(dbPath, ['report', 'trial-balance', '--json']);
  assert.equal(tb.out.data.balanced, true);
  const pnl = run(dbPath, ['report', 'pnl', '--year', '2026', '--json']);
  assert.equal(pnl.out.data.sections.some((s) => s.accounts.some((r) => r.code === '4700' && r.amount_cents === -50)), true);
});

test('entry post --dry-run: rejects non-draft entries instead of a green plan', () => {
  const dbPath = tmpDb();
  run(dbPath, ['init', '--name', 'Demo BV', '--kvk', '12345678', '--legal-form', 'bv', '--vat', 'off', '--json']);
  const { out } = run(dbPath, ['entry', 'add', '--date', '2026-01-01', '--desc', 'x', '--postings', '1100:100.00,3000:-100.00', '--post', '--json']);
  const id = out.data.id;
  // already posted -> dry-run must fail with ALREADY_POSTED, not "(no change)"
  const r = run(dbPath, ['entry', 'post', '--id', String(id), '--dry-run', '--json'], { expectFail: true });
  assert.equal(r.code, 1);
  assert.equal(r.out.error.code, 'ALREADY_POSTED');
});

test('entry reverse --dry-run: rejects drafts (NOT_POSTED) and double reversals', () => {
  const dbPath = tmpDb();
  run(dbPath, ['init', '--name', 'Demo BV', '--kvk', '12345678', '--legal-form', 'bv', '--vat', 'off', '--json']);
  // draft entry -> reversal plan must NOT be shown
  const { out } = run(dbPath, ['entry', 'add', '--date', '2026-01-01', '--desc', 'x', '--postings', '1100:100.00,3000:-100.00', '--json']);
  const draftId = out.data.id;
  const r1 = run(dbPath, ['entry', 'reverse', '--id', String(draftId), '--dry-run', '--json'], { expectFail: true });
  assert.equal(r1.code, 1);
  assert.equal(r1.out.error.code, 'NOT_POSTED');
  // posted then reversed -> second reversal dry-run fails ALREADY_REVERSED
  run(dbPath, ['entry', 'post', '--id', String(draftId), '--json']);
  run(dbPath, ['entry', 'reverse', '--id', String(draftId), '--reason', 'correctie', '--json']);
  const r2 = run(dbPath, ['entry', 'reverse', '--id', String(draftId), '--dry-run', '--json'], { expectFail: true });
  assert.equal(r2.code, 1);
  assert.equal(r2.out.error.code, 'ALREADY_REVERSED');
});

test('vat book --dry-run: validates date and description like the execute path', () => {
  const dbPath = tmpDb();
  run(dbPath, ['init', '--name', 'Demo BV', '--kvk', '12345678', '--legal-form', 'bv', '--vat', 'on', '--json']);
  const r = run(dbPath, ['vat', 'book', '--date', '2026-99-99', '--desc', 'x', '--postings', '1100:121.00,8000:-100.00@21', '--dry-run', '--json'], { expectFail: true });
  assert.equal(r.code, 1);
  assert.equal(r.out.error.code, 'INVALID_DATE');
});

test('actor: help lists the identity subcommands', () => {
  const help = runText(tmpDb(), ['actor', '--help']);
  for (const cmd of ['keygen', 'register', 'list', 'revoke', 'enforce', 'unlock', 'lock', 'verify']) {
    assert.ok(help.includes(cmd), `actor help must mention '${cmd}'`);
  }
  // an unknown subcommand exits non-zero with help
  assert.throws(() => runText(tmpDb(), ['actor', 'frobnicate']));
});

test('actor enforce: needs exactly one of --on/--off (INVALID_ENFORCE, JSON contract)', () => {
  const dbPath = tmpDb();
  const { code, out } = run(dbPath, ['actor', 'enforce', '--json'], { expectFail: true });
  assert.equal(code, 1);
  assert.equal(out.ok, false);
  assert.equal(out.error.code, 'INVALID_ENFORCE');
});

// --- audit verify (Task 7) --------------------------------------------------

/** Signed-company setup: init + keygen agent + register, enforce still off. */
function setupSignedCompany(cfg) {
  const dbPath = tmpDb();
  const env = { ...process.env, BUKIO_DB: dbPath, BUKIO_ACTOR: 'agent:test', BUKIO_CONFIG_DIR: cfg };
  execFileSync(process.execPath, [BIN, '--actor', 'human:erik', 'init', '--name', 'X', '--json'], { env, encoding: 'utf8' });
  execFileSync(process.execPath, [BIN, '--json', '--actor', 'agent:bartholomeus', 'actor', 'keygen'], { env, encoding: 'utf8' });
  execFileSync(process.execPath, [BIN, '--json', '--actor', 'agent:bartholomeus', 'actor', 'register'], { env, encoding: 'utf8' });
  execFileSync(process.execPath, [BIN, '--json', '--actor', 'agent:bartholomeus', 'entry', 'add', '--date', '2026-08-10', '--desc', 'Signed', '--postings', '1100:100.00,8000:-100.00', '--post'], { env, encoding: 'utf8' });
  return { dbPath, env };
}

/** Record a row whose digest does not match its stored args (tampering). */
function injectTamperedRow(dbPath) {
  const db = openDb(dbPath);
  try {
    const ts = new Date().toISOString();
    const nonce = crypto.randomUUID();
    const args = { date: '2026-08-10', desc: 'original' };
    const realDigest = buildDigest({ actor: 'agent:bartholomeus', cmd: 'entry add', args, ts, nonce });
    const tamperedDigest = buildDigest({ actor: 'agent:bartholomeus', cmd: 'entry add', args: { date: '2026-08-10', desc: 'HACKED' }, ts, nonce });
    const kp = crypto.generateKeyPairSync('ed25519');
    setPendingSignature({
      digestHash: tamperedDigest, sigKeyid: 'ff'.repeat(16), sigNonce: nonce, sigTs: ts,
      sig: sign(realDigest, kp.privateKey.export({ type: 'pkcs8', format: 'pem' })), sigStatus: 'verified', signedArgs: args,
    });
    record(db, { actor: 'agent:bartholomeus', action: 'test.tampered', command: 'entry add', outcome: 'ok' });
  } finally {
    db.close();
  }
}

test('audit verify: clean signed trail -> JSON summary, exit 0', () => {
  const cfg = mkdtempSync(path.join(os.tmpdir(), 'bukio-verify-cli-'));
  const { dbPath, env } = setupSignedCompany(cfg);
  const out = execFileSync(process.execPath, [BIN, '--json', 'audit', 'verify'], { env, encoding: 'utf8' });
  const data = JSON.parse(out).data;
  assert.ok(data.summary.ok >= 1, `expected at least one verified row, got ${JSON.stringify(data.summary)}`);
  assert.equal(data.summary.tampered, 0);
  assert.equal(data.summary.invalid_signature, 0);
  assert.equal(data.summary.unknown_key, 0);
  assert.ok(data.rows.every((r) => ['ok', 'unsigned'].includes(r.status)));
});

test('audit verify: tampered row -> exit 1 with per-row status and counts', () => {
  const cfg = mkdtempSync(path.join(os.tmpdir(), 'bukio-verify-cli-'));
  const { dbPath, env } = setupSignedCompany(cfg);
  injectTamperedRow(dbPath);
  try {
    execFileSync(process.execPath, [BIN, '--json', 'audit', 'verify'], { env, encoding: 'utf8' });
    assert.fail('audit verify should exit 1 when the trail has problems');
  } catch (err) {
    assert.equal(err.status, 1);
    const data = JSON.parse(err.stdout).data;
    assert.equal(data.summary.tampered, 1);
    const bad = data.rows.find((r) => r.status === 'tampered');
    assert.ok(bad);
    assert.equal(bad.action, 'test.tampered');
  }
});

test('version: --version and the MCP serverInfo match package.json (drift guard)', () => {
  const pkg = JSON.parse(readFileSync(path.join(path.dirname(BIN), '..', 'package.json'), 'utf8'));
  // the CLI version string comes from the binary itself
  const out = execFileSync(process.execPath, [BIN, '--version'], { encoding: 'utf8' }).trim();
  assert.equal(out, pkg.version, `bukio --version must equal package.json (${pkg.version})`);
  // the MCP serverInfo version is a separate literal — grep the source so a
  // release bump cannot silently drift one of the three version strings
  const mcpSrc = readFileSync(path.join(path.dirname(BIN), '..', 'src', 'cli', 'mcp.js'), 'utf8');
  assert.ok(mcpSrc.includes(`version: '${pkg.version}'`), `mcp.js serverInfo must carry ${pkg.version}`);
});
