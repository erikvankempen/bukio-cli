import { test, beforeEach } from 'node:test';
import assert from 'node:assert/strict';
import { openDb } from '../src/core/db.js';
import { seedDefaultChart, getAccountByCode } from '../src/core/accounts.js';
import { listEntries, getEntry } from '../src/core/entries.js';
import {
  importOpeningBalances, importJournalCsv, importXaf, parseImportAmount,
} from '../src/import/index.js';

let db;

function setup() {
  db = openDb(':memory:');
  seedDefaultChart(db);
  db.prepare(
    "INSERT INTO company (name, legal_form, kvk, btw_id, vat_module) VALUES ('Demo BV', 'bv', '12345678', 'NL123456789B01', 0)",
  ).run();
}

beforeEach(() => {
  setup();
});

// --- parseImportAmount ------------------------------------------------------

test('parseImportAmount: international, Dutch comma, thousands-dot forms', () => {
  assert.equal(parseImportAmount('1234.56'), 123456);
  assert.equal(parseImportAmount('1234,56'), 123456);
  assert.equal(parseImportAmount('1.234,56'), 123456);
  assert.equal(parseImportAmount('0,50'), 50);
  assert.equal(parseImportAmount('-12,50'), -1250);
  assert.throws(() => parseImportAmount('abc'), { code: 'INVALID_AMOUNT' });
  assert.throws(() => parseImportAmount('1.234'), { code: 'INVALID_AMOUNT' }); // 3 decimals
  assert.throws(() => parseImportAmount(''), { code: 'INVALID_AMOUNT' });
});

// --- opening balances -------------------------------------------------------

test('opening-balances: imports ONE posted Beginbalans entry (source import)', () => {
  const res = importOpeningBalances(db, {
    csvText: '1100,10000.00\n3000,-10000.00\n', date: '2026-01-01', actor: 'agent:test',
  });
  assert.equal(res.entry.state, 'posted');
  assert.equal(res.accounts, 2);
  const e = getEntry(db, res.entry.id);
  assert.equal(e.description, 'Beginbalans');
  assert.equal(e.source, 'import');
  assert.equal(e.source_ref, 'opening-balances');
  assert.equal(e.created_by, 'agent:test');
});

test('opening-balances: Dutch code,debet,credit layout', () => {
  importOpeningBalances(db, { csvText: '1100,10000.00,\n3000,,10000.00\n', date: '2026-01-01' });
  const e = getEntry(db, listEntries(db, { state: 'posted' })[0].id);
  const sums = {};
  for (const p of e.postings) sums[p.account_code] = (sums[p.account_code] ?? 0) + p.amount_cents;
  assert.deepEqual(sums, { 1100: 1000000, 3000: -1000000 });
});

test('opening-balances: validation collects ALL errors, writes nothing', () => {
  const csv = '1100,5000.00\n9999,3000.00\n1100,abc\n';
  assert.throws(
    () => importOpeningBalances(db, { csvText: csv, date: '2026-01-01' }),
    (err) => {
      assert.equal(err.code, 'IMPORT_VALIDATION_FAILED');
      assert.equal(err.details.length, 3); // ACCOUNT_NOT_FOUND, INVALID_AMOUNT, UNBALANCED
      assert.ok(err.details.some((d) => d.error.includes('9999')));
      assert.ok(err.details.some((d) => d.error.includes('abc')));
      assert.ok(err.details.some((d) => d.error.startsWith('UNBALANCED')));
      return true;
    },
  );
  assert.equal(listEntries(db).length, 0);
});

test('opening-balances: re-import is rejected', () => {
  importOpeningBalances(db, { csvText: '1100,1.00\n3000,-1.00\n', date: '2026-01-01' });
  assert.throws(
    () => importOpeningBalances(db, { csvText: '1100,2.00\n3000,-2.00\n', date: '2026-01-02' }),
    { code: 'OPENING_ALREADY_IMPORTED' },
  );
});

test('opening-balances: dry-run validates and writes nothing', () => {
  const plan = importOpeningBalances(db, { csvText: '1100,1.00\n3000,-1.00\n', date: '2026-01-01', dryRun: true });
  assert.equal(plan.accounts, 2);
  assert.equal(plan.total_debit_cents, 100);
  assert.equal(listEntries(db).length, 0);
});

test('opening-balances: unknown account and zero amount rejected', () => {
  assert.throws(
    () => importOpeningBalances(db, { csvText: '1100,0.00\n3000,-0.00\n', date: '2026-01-01' }),
    (err) => {
      assert.equal(err.code, 'IMPORT_VALIDATION_FAILED');
      assert.ok(err.details.some((d) => d.error.includes('non-zero')));
      return true;
    },
  );
});

// --- journal CSV ------------------------------------------------------------

const JOURNAL = `Datum;Boekstuknummer;Rekening;Tegenrekening;Bedrag;Omschrijving
2026-01-05;J1;1100;8000;1210,00;Verkoop 1
2026-01-05;J1;3000;1100;210,00;correctie eigen vermogen
2026-01-20;J2;1100;8000;605,00;Verkoop 2
2026-01-20;J2;3000;1100;105,00;correctie eigen vermogen`;

test('journal: one posted entry per boekstuk, two postings per line', () => {
  const res = importJournalCsv(db, { csvText: JOURNAL, actor: 'agent:test' });
  assert.equal(res.imported, 2);
  const entries = listEntries(db, { state: 'posted' });
  assert.equal(entries.length, 2);
  const j1 = getEntry(db, entries.find((e) => e.source_ref === 'journal:J1').id);
  assert.equal(j1.source, 'import');
  assert.equal(j1.description, 'Verkoop 1');
  const sums = {};
  for (const p of j1.postings) sums[p.account_code] = (sums[p.account_code] ?? 0) + p.amount_cents;
  assert.deepEqual(sums, { 1100: 100000, 8000: -121000, 3000: 21000 });
});

test('journal: idempotent re-import skips existing boekstukken', () => {
  assert.equal(importJournalCsv(db, { csvText: JOURNAL }).imported, 2);
  const res = importJournalCsv(db, { csvText: JOURNAL });
  assert.equal(res.imported, 0);
  assert.equal(res.duplicates, 2);
  assert.equal(listEntries(db).length, 2);
});

test('journal: unknown account fails whole-file validation without --create-missing', () => {
  assert.throws(
    () => importJournalCsv(db, { csvText: JOURNAL.replace('3000', '9999') }),
    (err) => {
      assert.equal(err.code, 'IMPORT_VALIDATION_FAILED');
      assert.ok(err.details.some((d) => d.error.includes('9999')));
      return true;
    },
  );
  assert.equal(listEntries(db).length, 0);
});

test('journal: --create-missing infers type from net movement', () => {
  const csv = JOURNAL.replaceAll('3000', '9999').replaceAll('8000', '9998');
  const res = importJournalCsv(db, { csvText: csv, createMissing: true });
  assert.equal(res.imported, 2);
  assert.ok(res.accounts_created.length >= 2);
  // 9999 net +315 (debet) -> expense; 9998 net -1815 (credit) -> income
  assert.equal(getAccountByCode(db, '9999').type, 'expense');
  assert.equal(getAccountByCode(db, '9998').type, 'income');
});

test('journal: bad amount and date mismatch per boekstuk are both collected', () => {
  const csv = `Datum;Boekstuknummer;Rekening;Tegenrekening;Bedrag
2026-01-05;J1;1100;8000;abc
2026-01-06;J1;1100;8000;10.00`;
  assert.throws(
    () => importJournalCsv(db, { csvText: csv }),
    (err) => {
      assert.equal(err.code, 'IMPORT_VALIDATION_FAILED');
      assert.ok(err.details.some((d) => d.error.startsWith('INVALID_AMOUNT')));
      assert.ok(err.details.some((d) => d.error.startsWith('DATE_MISMATCH')));
      return true;
    },
  );
});

test('journal: missing required header column rejected', () => {
  assert.throws(
    () => importJournalCsv(db, { csvText: 'Datum;Boekstuknummer;Rekening;Bedrag\n2026-01-05;J1;1100;10.00' }),
    { code: 'INVALID_CSV_HEADER' },
  );
});

// --- XAF 4.0 ----------------------------------------------------------------

const XAF = `<?xml version="1.0" encoding="UTF-8"?>
<Xaf xmlns="http://www.auditfiles.nl/XAF/4.0">
  <XafHeader>
    <Version>4.0</Version>
    <CompanyName>Demo BV</CompanyName>
    <CompanyID>12345678</CompanyID>
    <FiscalYear>2026</FiscalYear>
    <StartDate>2026-01-01</StartDate>
    <EndDate>2026-12-31</EndDate>
    <SoftwareName>OudPakket</SoftwareName>
    <SoftwareVersion>1.0</SoftwareVersion>
  </XafHeader>
  <Rekeningen>
    <Rekening><RekeningCode>1250</RekeningCode><RekeningOmschrijving>Kas klein</RekeningOmschrijving><RekeningSoort>Balans</RekeningSoort></Rekening>
    <Rekening><RekeningCode>8000</RekeningCode><RekeningOmschrijving>Omzet</RekeningOmschrijving><RekeningSoort>Winst en Verlies</RekeningSoort></Rekening>
  </Rekeningen>
  <Mutaties>
    <Mutatie>
      <Boekstuknummer>2026-0001</Boekstuknummer>
      <Datum>2026-01-15</Datum>
      <Boekingen>
        <Boeking><RekeningCode>1250</RekeningCode><TegenrekeningCode>8000</TegenrekeningCode><Bedrag>100,00</Bedrag><BtwCode>0</BtwCode><Omschrijving>Contante verkoop</Omschrijving></Boeking>
      </Boekingen>
    </Mutatie>
  </Mutaties>
</Xaf>`;

test('xaf: imports mutaties and creates file-chart accounts', () => {
  const res = importXaf(db, { xmlText: XAF, actor: 'agent:test' });
  assert.equal(res.imported, 1);
  assert.equal(res.header.company_name, 'Demo BV');
  assert.equal(res.header.fiscal_year, '2026');
  const kas = getAccountByCode(db, '1250'); // created from the file (debet net -> asset)
  assert.ok(kas, '1250 Kas klein should be created from the file chart');
  assert.equal(kas.type, 'asset');
  assert.ok(res.accounts_created.some((a) => a.code === '1250'));
  const e = listEntries(db, { state: 'posted' })[0];
  assert.equal(e.source, 'xaf');
  assert.equal(e.source_ref, '2026-0001');
});

test('xaf: btw codes are reported, not booked', () => {
  const res = importXaf(db, { xmlText: XAF });
  assert.deepEqual(res.ignored_btw_codes, ['0']);
  const e = getEntry(db, listEntries(db)[0].id);
  assert.ok(e.postings.every((p) => p.vat_code_id == null));
});

test('xaf: idempotent per boekstuknummer', () => {
  importXaf(db, { xmlText: XAF });
  const res = importXaf(db, { xmlText: XAF });
  assert.equal(res.imported, 0);
  assert.equal(res.duplicates, 1);
});

test('xaf: rekening not in file chart nor chart of accounts -> validation error', () => {
  const bad = XAF.replace(
    '</Boeking>',
    '</Boeking><Boeking><RekeningCode>9999</RekeningCode><TegenrekeningCode>8000</TegenrekeningCode><Bedrag>50,00</Bedrag></Boeking>',
  );
  assert.throws(
    () => importXaf(db, { xmlText: bad }),
    (err) => {
      assert.equal(err.code, 'IMPORT_VALIDATION_FAILED');
      assert.ok(err.details.some((d) => d.error.includes('9999')));
      return true;
    },
  );
  assert.equal(listEntries(db).length, 0);
});

test('xaf: unsupported version rejected', () => {
  assert.throws(
    () => importXaf(db, { xmlText: XAF.replace('<Version>4.0</Version>', '<Version>3.1</Version>') }),
    { code: 'INVALID_XAF' },
  );
});

test('xaf: COMPANY_MISMATCH blocks importing another company', () => {
  assert.throws(
    () => importXaf(db, { xmlText: XAF.replace('<CompanyID>12345678</CompanyID>', '<CompanyID>99999999</CompanyID>') }),
    { code: 'COMPANY_MISMATCH' },
  );
});

test('xaf: name mismatch is only a warning', () => {
  const res = importXaf(db, { xmlText: XAF.replace('<CompanyName>Demo BV</CompanyName>', '<CompanyName>Demo B.V. Rotterdam</CompanyName>') });
  assert.equal(res.imported, 1);
  assert.ok(res.company_mismatch.some((w) => w.includes('name differs')));
});

test('xaf: dry-run validates and writes nothing', () => {
  const plan = importXaf(db, { xmlText: XAF, dryRun: true });
  assert.equal(plan.mutaties, 1);
  assert.equal(plan.accounts_to_create, 1);
  assert.equal(listEntries(db).length, 0);
  assert.equal(getAccountByCode(db, '1250'), null);
});
