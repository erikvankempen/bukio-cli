/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

import { test, beforeEach } from 'node:test';
import assert from 'node:assert/strict';
import { openDb } from '../src/core/db.js';
import { seedDefaultChart, getAccountByCode } from '../src/core/accounts.js';
import { listEntries, getEntry, createEntry, postEntry, reverseEntry } from '../src/core/entries.js';
import {
  importOpeningBalances, importJournalCsv, importXaf, importContacts, parseImportAmount,
} from '../src/import/index.js';
import { listContacts } from '../src/invoice/index.js';
import { inferRgs } from '../src/core/chart.js';
import { importChartCsv, createAccount } from '../src/core/accounts.js';

let db;

function setup() {
  db = openDb(':memory:');
  seedDefaultChart(db);
  db.prepare(
    "INSERT INTO company (name, legal_form, registration_id, tax_id, vat_module) VALUES ('Demo BV', 'bv', '12345678', 'NL123456789B01', 0)",
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

test('opening-balances: re-import succeeds after reversing the opening entry (correction path)', () => {
  const res = importOpeningBalances(db, { csvText: '1100,1.00\n3000,-1.00\n', date: '2026-01-01', actor: 'agent:test' });
  reverseEntry(db, { id: res.entry.id, actor: 'agent:test' });
  // the reversal nets the old balances to zero — a fresh import is allowed
  const res2 = importOpeningBalances(db, { csvText: '1100,2.00\n3000,-2.00\n', date: '2026-01-02', actor: 'agent:test' });
  assert.notEqual(res2.entry.id, res.entry.id, 'a NEW opening entry is created');
  const sums = {};
  for (const p of getEntry(db, res2.entry.id).postings) sums[p.account_code] = (sums[p.account_code] ?? 0) + p.amount_cents;
  assert.deepEqual(sums, { 1100: 200, 3000: -200 });
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

test('journal: comma-delimited file with a semicolon inside a quoted field parses (delimiter decided once)', () => {
  // the per-line heuristic used to flip this row to ';'-mode because the
  // quoted description contains a semicolon, misparsing the row; the
  // delimiter must come from the first (header) line only
  const csv = 'datum,boekstuknummer,rekening,tegenrekening,bedrag,omschrijving\n'
    + '2026-01-05,J1,1100,8000,100.00,"Consultancy; tweede termijn"\n'
    + '2026-01-06,J2,1100,8000,50.00,Zonder puntkomma\n';
  const res = importJournalCsv(db, { csvText: csv, actor: 'agent:test' });
  assert.equal(res.imported, 2);
  const entries = listEntries(db, { state: 'posted' });
  assert.equal(entries.length, 2);
  const j1 = getEntry(db, entries.find((e) => e.source_ref === 'journal:J1').id);
  assert.equal(j1.description, 'Consultancy; tweede termijn', 'the quoted field must survive intact');
  const sums = {};
  for (const p of j1.postings) sums[p.account_code] = (sums[p.account_code] ?? 0) + p.amount_cents;
  assert.deepEqual(sums, { 1100: 10000, 8000: -10000 });
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

// --- XAF 4.0 AuditFile layout (root <AuditFile>) ----------------------------

const AUDITFILE_XAF = `<?xml version="1.0" encoding="UTF-8"?>
<AuditFile xmlns="https://www.bukio.nl/xaf/4.0" version="4.0" exportedAt="2026-08-06T08:48:22Z">
  <Header>
    <AuditFileVersion>4.0</AuditFileVersion>
    <CompanyID>1</CompanyID>
    <CompanyName>Demo BV</CompanyName>
    <FiscalYear>2026</FiscalYear>
    <StartDate>2026-01-01</StartDate>
    <EndDate>2026-12-31</EndDate>
    <CurrencyCode>EUR</CurrencyCode>
    <SoftwareDescription>Bukio</SoftwareDescription>
  </Header>
  <MasterFiles>
    <GeneralLedgerAccounts>
      <Account><AccountID>1100</AccountID><AccountDescription>Gebouwen</AccountDescription><AccountType>Asset</AccountType></Account>
      <Account><AccountID>8000</AccountID><AccountDescription>Omzet</AccountDescription><AccountType>Revenue</AccountType></Account>
      <Account><AccountID>5100</AccountID><AccountDescription>Crediteuren</AccountDescription><AccountType>Liability</AccountType></Account>
      <Account><AccountID>7150</AccountID><AccountDescription>Platformkosten</AccountDescription><AccountType>Expense</AccountType></Account>
    </GeneralLedgerAccounts>
  </MasterFiles>
  <GeneralLedgerEntries>
    <Journal>
      <JournalID>SAL</JournalID>
      <Transaction>
        <TransactionID>2026-00001</TransactionID>
        <TransactionDate>2026-01-15</TransactionDate>
        <Description>Factuur 2026-0001</Description>
        <Line>
          <RecordID>1</RecordID>
          <AccountID>1100</AccountID>
          <Description>Factuur 2026-0001</Description>
          <DebitAmount>121.00</DebitAmount>
          <TaxInformation><TaxType>VAT</TaxType><TaxCode>NOVAT</TaxCode><TaxPercentage>0.00</TaxPercentage></TaxInformation>
        </Line>
        <Line>
          <RecordID>2</RecordID>
          <AccountID>8000</AccountID>
          <Description>Factuur 2026-0001</Description>
          <CreditAmount>121.00</CreditAmount>
        </Line>
      </Transaction>
    </Journal>
  </GeneralLedgerEntries>
</AuditFile>`;

test('xaf (AuditFile layout): imports transaction, creates + renames chart accounts', () => {
  const res = importXaf(db, { xmlText: AUDITFILE_XAF, actor: 'agent:test' });
  assert.equal(res.imported, 1);
  assert.equal(res.header.company_name, 'Demo BV');
  assert.equal(res.header.software, 'Bukio');
  // file accounts missing from the starter chart are created with mapped types
  assert.equal(getAccountByCode(db, '5100').type, 'liability');
  assert.equal(getAccountByCode(db, '7150').type, 'expense');
  // colliding codes are renamed on the empty ledger (1100 Bank -> Gebouwen)
  assert.equal(getAccountByCode(db, '1100').name, 'Gebouwen');
  assert.deepEqual(res.accounts_updated, [{ code: '1100', from: 'Bank', to: 'Gebouwen', type: 'asset', normal_balance: 'debit', taxonomy_code: 'BMVA.02' }]);
  // NOVAT reported, not booked
  assert.deepEqual(res.ignored_btw_codes, ['NOVAT']);
  const e = getEntry(db, listEntries(db, { state: 'posted' })[0].id);
  assert.equal(e.source, 'xaf');
  assert.equal(e.source_ref, '2026-00001');
  assert.equal(e.description, 'Factuur 2026-0001');
  const sums = {};
  for (const p of e.postings) sums[p.account_code] = (sums[p.account_code] ?? 0) + p.amount_cents;
  assert.deepEqual(sums, { 1100: 12100, 8000: -12100 });
});

test('xaf (AuditFile layout): accounts with postings are NOT renamed', () => {
  postEntry(db, { id: createEntry(db, {
    date: '2026-01-01', description: 'Bestaat al', postings: [{ code: '1100', amountCents: 100 }, { code: '3000', amountCents: -100 }],
  }).id });
  const res = importXaf(db, { xmlText: AUDITFILE_XAF });
  assert.equal(getAccountByCode(db, '1100').name, 'Bank'); // untouched
  assert.ok(res.chart_warnings.some((w) => w.includes('1100')));
});

test('xaf (AuditFile layout): unbalanced transaction fails whole-file validation', () => {
  const bad = AUDITFILE_XAF.replace('</Line>\n        <Line>\n          <RecordID>2</RecordID>\n          <AccountID>8000</AccountID>\n          <Description>Factuur 2026-0001</Description>\n          <CreditAmount>121.00</CreditAmount>\n        </Line>', '</Line>');
  assert.throws(
    () => importXaf(db, { xmlText: bad }),
    (err) => {
      assert.equal(err.code, 'IMPORT_VALIDATION_FAILED');
      assert.ok(err.details.some((d) => d.error.startsWith('UNBALANCED')));
      return true;
    },
  );
  assert.equal(listEntries(db).length, 0);
});

test('xaf (AuditFile layout): idempotent per TransactionID', () => {
  importXaf(db, { xmlText: AUDITFILE_XAF });
  const res = importXaf(db, { xmlText: AUDITFILE_XAF });
  assert.equal(res.imported, 0);
  assert.equal(res.duplicates, 1);
});

test('xaf (AuditFile layout): dry-run lists renames and writes nothing', () => {
  const plan = importXaf(db, { xmlText: AUDITFILE_XAF, dryRun: true });
  assert.equal(plan.mutaties, 1);
  assert.equal(plan.accounts_to_create, 2);
  assert.deepEqual(plan.accounts_to_rename.map((r) => r.code), ['1100']);
  assert.equal(listEntries(db).length, 0);
  assert.equal(getAccountByCode(db, '1100').name, 'Bank');
});

test('xaf (AuditFile layout): 8-digit CompanyID mismatch is an error', () => {
  assert.throws(
    () => importXaf(db, { xmlText: AUDITFILE_XAF.replace('<CompanyID>1</CompanyID>', '<CompanyID>99999999</CompanyID>') }),
    { code: 'COMPANY_MISMATCH' },
  );
});

test('xaf (AuditFile layout): company name mismatch is only a warning', () => {
  const res = importXaf(db, { xmlText: AUDITFILE_XAF.replace('<CompanyName>Demo BV</CompanyName>', '<CompanyName>Demo B.V. Rotterdam</CompanyName>') });
  assert.equal(res.imported, 1);
  assert.ok(res.company_mismatch.some((w) => w.includes('name differs')));
});

// --- contacts from audit files ----------------------------------------------

const CONTACTS_XAF = `<?xml version="1.0" encoding="UTF-8"?>
<AuditFile xmlns="https://www.bukio.nl/xaf/4.0" version="4.0">
  <Header>
    <AuditFileVersion>4.0</AuditFileVersion>
    <CompanyName>Demo BV</CompanyName>
  </Header>
  <MasterFiles>
    <Customers>
      <Customer>
        <CustomerID>1</CustomerID>
        <CompanyName>Daan van der Leen</CompanyName>
        <Email>daanleen@gmail.com</Email>
        <Address>
          <StreetName>Lamarckhof 9-1</StreetName>
          <PostalCode>1098TK</PostalCode>
          <City>Amsterdam</City>
          <Country>NL</Country>
        </Address>
      </Customer>
    </Customers>
    <Suppliers>
      <Supplier>
        <SupplierID>13</SupplierID>
        <CompanyName>Anomaly</CompanyName>
        <Contact>Matt</Contact>
        <Email>help@anoma.ly</Email>
        <Address>
          <StreetName>2443 Fillmore Street</StreetName>
          <PostalCode>94115</PostalCode>
          <City>San Francisco</City>
          <Country>us</Country>
        </Address>
      </Supplier>
      <Supplier>
        <SupplierID>14</SupplierID>
        <CompanyName>DeluxHost</CompanyName>
        <Email></Email>
      </Supplier>
    </Suppliers>
  </MasterFiles>
</AuditFile>`;

test('import contacts: suppliers + customers mapped to contacts', () => {
  const res = importContacts(db, { xmlText: CONTACTS_XAF, actor: 'agent:test' });
  assert.equal(res.imported, 3);
  assert.equal(res.suppliers, 2);
  assert.equal(res.customers, 1);
  const [anomaly, daan, delux] = listContacts(db).sort((a, b) => a.name.localeCompare(b.name));
  assert.equal(daan.name, 'Daan van der Leen');
  assert.equal(daan.address, 'Lamarckhof 9-1');
  assert.equal(daan.postal_code, '1098TK');
  assert.equal(daan.city, 'Amsterdam');
  assert.equal(daan.country, 'NL');
  assert.equal(daan.email, 'daanleen@gmail.com');
  assert.equal(anomaly.country, 'US'); // normalised to uppercase
  assert.equal(delux.name, 'DeluxHost');
  assert.equal(delux.address, null); // no address in the file
  const audit = db.prepare("SELECT * FROM audit_log WHERE action = 'import.contacts' ORDER BY id DESC LIMIT 1").get();
  assert.equal(audit.actor, 'agent:test');
});

test('import contacts: idempotent by name', () => {
  importContacts(db, { xmlText: CONTACTS_XAF });
  const res = importContacts(db, { xmlText: CONTACTS_XAF });
  assert.equal(res.imported, 0);
  assert.equal(res.duplicates, 3);
});

test('import contacts: entry without a name fails whole-file validation', () => {
  const bad = CONTACTS_XAF.replace('<CompanyName>DeluxHost</CompanyName>', '');
  assert.throws(
    () => importContacts(db, { xmlText: bad }),
    (err) => {
      assert.equal(err.code, 'IMPORT_VALIDATION_FAILED');
      assert.ok(err.details.some((d) => d.error.startsWith('CONTACT_REQUIRED')));
      return true;
    },
  );
  assert.equal(listContacts(db).length, 0);
});

// --- RGS enforcement on import ----------------------------------------------

test('inferRgs: keywords within type, then type-based fallbacks', () => {
  assert.equal(inferRgs('income', 'Omzet diensten'), 'WOVB.82'); // diensten before omzet
  assert.equal(inferRgs('income', 'Omzet goederen'), 'WOMZ.80');
  assert.equal(inferRgs('income', 'Overige opbrengsten'), 'WOVB.82');
  assert.equal(inferRgs('expense', 'Afschrijvingskosten'), 'WAFS.41');
  assert.equal(inferRgs('expense', 'Bankkosten'), 'WFBE.84');
  assert.equal(inferRgs('expense', 'Rentebaten'), 'WFBE.84');
  assert.equal(inferRgs('expense', 'Kosten IT'), 'WBED.42');
  assert.equal(inferRgs('expense', 'Inkoopwaarde'), 'WKPR.70');
  assert.equal(inferRgs('expense', 'Voorraadmutatie'), 'WKPR.70');
  assert.equal(inferRgs('expense', 'Kosten uitbesteed werk'), 'WKPR.70');
  assert.equal(inferRgs('expense', 'Pensioenlasten'), 'WPER.40');
  assert.equal(inferRgs('asset', 'Bank Rabobank ZZP'), 'BLIM.10');
  assert.equal(inferRgs('asset', 'Hardware'), 'BMVA.02');
  assert.equal(inferRgs('asset', 'Te vorderen btw hoog 21%'), 'BVOR.11');
  assert.equal(inferRgs('asset', 'Vraagposten'), 'BVOR.11');
  assert.equal(inferRgs('asset', 'Kruisposten'), 'BVOR.11');
  assert.equal(inferRgs('asset', 'Cumulatieve afschrijvingen'), 'BMVA.02'); // contra-MVA
  assert.equal(inferRgs('liability', 'Crediteuren'), 'BSCH.12');
  assert.equal(inferRgs('equity', 'Privéstortingen'), 'BEIV.05');
});

test('import xaf (AuditFile): created accounts carry inferred RGS codes', () => {
  const res = importXaf(db, { xmlText: AUDITFILE_XAF, actor: 'agent:test' });
  // 5100 Crediteuren + 7150 Platformkosten are new; 1100/8000 exist in the seed
  assert.equal(res.accounts_created.length, 2);
  assert.equal(getAccountByCode(db, '1100').taxonomy_code, 'BMVA.02'); // renamed Gebouwen: BLIM.10 -> BMVA.02
  assert.equal(getAccountByCode(db, '8000').taxonomy_code, 'WOMZ.80'); // Omzet (revenue)
  assert.equal(getAccountByCode(db, '5100').taxonomy_code, 'BSCH.12'); // Crediteuren
  assert.equal(getAccountByCode(db, '7150').taxonomy_code, 'WBED.42'); // Platformkosten
});

test('import xaf: re-import backfills RGS codes on accounts that lack them', () => {
  // pre-fix chart state: 8000 exists WITHOUT an rgs code
  db.prepare("UPDATE accounts SET taxonomy_code = NULL WHERE code = '8000'").run();
  const res = importXaf(db, { xmlText: AUDITFILE_XAF, actor: 'agent:test' });
  assert.equal(getAccountByCode(db, '8000').taxonomy_code, 'WOMZ.80');
  assert.ok(res.accounts_rgs_backfilled.some((a) => a.code === '8000' && a.taxonomy_code === 'WOMZ.80'));
  // idempotent: second re-run backfills nothing
  const res2 = importXaf(db, { xmlText: AUDITFILE_XAF, actor: 'agent:test' });
  assert.equal(res2.accounts_rgs_backfilled.length, 0);
});

test('import journal: --create-missing accounts also get RGS codes', () => {
  const csv = JOURNAL.replaceAll('3000', '9999').replaceAll('8000', '9998');
  importJournalCsv(db, { csvText: csv, createMissing: true });
  assert.equal(getAccountByCode(db, '9999').taxonomy_code, 'WBED.42'); // expense fallback
  assert.equal(getAccountByCode(db, '9998').taxonomy_code, 'WOVB.82'); // income fallback
});

test('import chart CSV without an rgs column infers RGS codes', () => {
  const csv = 'code,name,type,normal_balance\n1350,Hardware,asset,debit\n8300,Omzet diensten,income,credit';
  const res = importChartCsv(db, csv);
  assert.equal(res.created, 2);
  assert.equal(getAccountByCode(db, '1350').taxonomy_code, 'BMVA.02');
  assert.equal(getAccountByCode(db, '8300').taxonomy_code, 'WOVB.82');
});

test('import contacts: dry-run writes nothing', () => {
  const plan = importContacts(db, { xmlText: CONTACTS_XAF, dryRun: true });
  assert.equal(plan.contacts_to_create, 3);
  assert.equal(plan.duplicates, 0);
  assert.equal(listContacts(db).length, 0);
});
