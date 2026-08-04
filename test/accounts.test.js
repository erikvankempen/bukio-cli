import { test, beforeEach } from 'node:test';
import assert from 'node:assert/strict';
import { openDb } from '../src/core/db.js';
import {
  createAccount, deactivateAccount, reactivateAccount, getAccountByCode,
  listAccounts, importChartCsv, seedDefaultChart,
} from '../src/core/accounts.js';
import { createEntry, postEntry } from '../src/core/entries.js';

let db;

beforeEach(() => {
  db = openDb(':memory:');
  seedDefaultChart(db);
});

test('createAccount: valid account lands in the chart', () => {
  const a = createAccount(db, { code: '5000', name: 'Testkosten', type: 'expense', normalBalance: 'debit' });
  assert.equal(a.code, '5000');
  assert.equal(getAccountByCode(db, '5000').name, 'Testkosten');
});

test('createAccount: rejects duplicates and invalid input', () => {
  assert.throws(() => createAccount(db, { code: '1100', name: 'x', type: 'asset', normalBalance: 'debit' }),
    { code: 'ACCOUNT_EXISTS' });
  assert.throws(() => createAccount(db, { code: 'abc', name: 'x', type: 'asset', normalBalance: 'debit' }),
    { code: 'INVALID_CODE' });
  assert.throws(() => createAccount(db, { code: '5001', name: 'x', type: 'weird', normalBalance: 'debit' }),
    { code: 'INVALID_TYPE' });
  assert.throws(() => createAccount(db, { code: '5001', name: 'x', type: 'expense', normalBalance: 'credit' }),
    { code: 'INVALID_COMBINATION' });
  assert.throws(() => createAccount(db, { code: '5001', name: 'x', type: 'expense', normalBalance: 'debit', rgsCode: 'BMVA' }),
    { code: 'INVALID_RGS_CODE' });
});

test('deactivate/reactivate lifecycle blocks new postings', () => {
  const account = createAccount(db, { code: '5000', name: 'Testkosten', type: 'expense', normalBalance: 'debit' });
  deactivateAccount(db, '5000');
  assert.equal(getAccountByCode(db, '5000').active, 0);
  assert.throws(() => deactivateAccount(db, '5000'), { code: 'ALREADY_INACTIVE' });
  assert.throws(() => deactivateAccount(db, '9999'), { code: 'ACCOUNT_NOT_FOUND' });

  assert.throws(() => createEntry(db, {
    date: '2026-08-04', description: 'x',
    postings: [{ code: '5000', amountCents: 100 }, { code: '1100', amountCents: -100 }],
  }), { code: 'ACCOUNT_INACTIVE' });

  reactivateAccount(db, '5000');
  assert.equal(getAccountByCode(db, '5000').active, 1);
  assert.throws(() => reactivateAccount(db, '5000'), { code: 'ALREADY_ACTIVE' });

  const e = createEntry(db, {
    date: '2026-08-04', description: 'x',
    postings: [{ code: '5000', amountCents: 100 }, { code: '1100', amountCents: -100 }],
  });
  postEntry(db, { id: e.id });
});

test('importChartCsv: imports valid rows, skips duplicates and invalid rows', () => {
  const csv = [
    'code,name,type,normal_balance,rgs_code',
    '5000,Testkosten,expense,debit,WBED.42',
    '5100,Andere kosten,expense,debit,WBED.42',
    '5200,Verkeerd type,weird,debit,',
    '1100,Bank duplicaat,asset,debit,BLIM.10',
    '',
  ].join('\n');
  const result = importChartCsv(db, csv);
  assert.equal(result.created, 2);
  assert.equal(result.skipped, 2);
  assert.equal(result.total, 4);
  assert.equal(result.errors.length, 2);
  assert.ok(result.errors.some((e) => /INVALID_TYPE/.test(e.error)));
  assert.ok(result.errors.some((e) => /ACCOUNT_EXISTS/.test(e.error)));
  assert.equal(getAccountByCode(db, '5100').name, 'Andere kosten');
});

test('importChartCsv: header validation', () => {
  assert.throws(() => importChartCsv(db, 'foo,bar\n1,2\n'), { code: 'INVALID_CSV_HEADER' });
  assert.throws(() => importChartCsv(db, 'code,name,type,normal_balance\n'), { code: 'EMPTY_CSV' });
});

test('importChartCsv: quoted values with commas parse', () => {
  const csv = [
    'code,name,type,normal_balance,rgs_code',
    '"5000","Kosten, algemeen",expense,debit,WBED.42',
  ].join('\n');
  const result = importChartCsv(db, csv);
  assert.equal(result.created, 1);
  assert.equal(getAccountByCode(db, '5000').name, 'Kosten, algemeen');
});

test('listAccounts: type filter and includeInactive', () => {
  assert.equal(listAccounts(db, { type: 'income' }).length, 2);
  assert.equal(listAccounts(db, { type: 'expense' }).length, 12);
  createAccount(db, { code: '5000', name: 'x', type: 'expense', normalBalance: 'debit' });
  deactivateAccount(db, '5000');
  assert.equal(listAccounts(db, { type: 'expense' }).length, 12); // inactive hidden
  assert.equal(listAccounts(db, { type: 'expense', includeInactive: true }).length, 13);
});
