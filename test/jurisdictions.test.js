/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across eleven jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Jurisdiction profile registry tests (Phase A, M1).
// Pure module-level tests — no DB required (resolveProfile lands in M2 with
// migration 021). The NL profile is the reference: its values must match the
// legacy hardcoded constants exactly ("moved, not changed").
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, readdirSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import Database from 'better-sqlite3';
import { openDb } from '../src/core/db.js';
import {
  createContact, createInvoice, finalizeInvoice, getInvoice, validateCompliance,
} from '../src/invoice/index.js';
import { invoiceToUbl } from '../src/invoice/ubl.js';
import { XMLParser } from 'fast-xml-parser';
import { exportXaf } from '../src/export/index.js';
import { complianceStatus, markFiled } from '../src/compliance/index.js';
import { vatSettle } from '../src/vat/index.js';
import { getProfile, PLANNED, normalizeCountry, resolveProfile } from '../src/jurisdictions/index.js';

const MIGRATIONS_DIR = path.join(import.meta.dirname, '..', 'migrations');

/** Scratch DB migrated to `version` (all migrations ≤ version, verbatim). */
function scratchDbAt(version, seedCompany = null) {
  const dir = mkdtempSync(path.join(tmpdir(), 'bukio-jur-'));
  const file = path.join(dir, 'test.db');
  const db = new Database(file);
  for (const f of readdirSync(MIGRATIONS_DIR)
    .filter((x) => /^\d+_.*\.sql$/.test(x))
    .sort((a, b) => parseInt(a, 10) - parseInt(b, 10))) {
    const v = parseInt(f.split('_')[0], 10);
    if (v > version) continue;
    db.exec(readFileSync(path.join(MIGRATIONS_DIR, f), 'utf8'));
  }
  db.pragma(`user_version = ${version}`);
  if (seedCompany) {
    // seedCompany = { sql, params } — caller provides the full INSERT
    db.prepare(seedCompany.sql).run(...seedCompany.params);
  }
  return db;
}

test('getProfile returns the NL profile for NL (any case)', () => {
  for (const cc of ['NL', 'nl', ' Nl ']) {
    const p = getProfile(cc);
    assert.equal(p.meta.country, 'NL');
    assert.equal(p.meta.baseCurrency, 'EUR');
    assert.equal(p.meta.locale, 'nl');
  }
});

test('getProfile rejects malformed country input with INVALID_COUNTRY', () => {
  for (const bad of ['NETHERLANDS', 'N', 'NLD', 'NL!', '', ' ', 42, null, undefined]) {
    assert.throws(() => getProfile(bad), (e) => e.code === 'INVALID_COUNTRY');
  }
});

test('getProfile throws COUNTRY_NOT_SUPPORTED for valid-but-planned countries', () => {
  for (const cc of PLANNED) {
    assert.throws(() => getProfile(cc), (e) => e.code === 'COUNTRY_NOT_SUPPORTED');
  }
  // all sixteen markets are implemented — PLANNED is empty
  assert.deepEqual([...PLANNED].sort(), []);
});

test('getProfile throws PROFILE_NOT_FOUND for unknown valid codes', () => {
  assert.throws(() => getProfile('ZZ'), (e) => e.code === 'PROFILE_NOT_FOUND');
  assert.throws(() => getProfile('IS'), (e) => e.code === 'PROFILE_NOT_FOUND');
});

test('profiles are deep-frozen (static data — no consumer may mutate)', () => {
  const p = getProfile('NL');
  assert.ok(Object.isFrozen(p));
  assert.ok(Object.isFrozen(p.tax));
  assert.ok(Object.isFrozen(p.tax.codes));
  assert.ok(Object.isFrozen(p.reporting.defaultChart));
  assert.throws(() => { p.tax.standardRateBp = 9999; }, TypeError);
});

test('NL profile integrity — tax section matches the legacy VAT module', () => {
  const p = getProfile('NL');
  assert.equal(p.tax.system, 'vat');
  assert.equal(p.tax.standardRateBp, 2100);
  assert.equal(p.tax.reverseChargeEffectiveRateBp, 2100);
  assert.equal(p.tax.smallBusinessScheme, 'kor');
  // current VAT_CODES: 21/9/0/V/R/RE/M/P
  assert.equal(p.tax.codes.length, 8);
  for (const c of p.tax.codes) {
    assert.ok(['standard', 'exempt', 'reverse', 'margin', 'private'].includes(c.type));
    assert.equal(typeof c.rateBp, 'number');
    assert.equal(typeof c.description, 'string');
  }
  assert.deepEqual(p.tax.codes.map((c) => c.code), ['21', '9', '0', 'V', 'R', 'RE', 'M', 'P']);
  // legacy VAT_ACCOUNTS 1500/2500
  assert.deepEqual(p.tax.accounts.ledger.map((a) => a.code), ['1500', '2500']);
  assert.equal(p.tax.accounts.fileDefault, '2510');
  assert.equal(p.tax.accounts.differenceDefault, '4700');
  assert.equal(p.tax.accounts.settlementAccountName, 'Af te dragen omzetbelasting');
});

test('NL profile integrity — reporting section matches the legacy chart', () => {
  const p = getProfile('NL');
  assert.equal(p.reporting.taxonomy, 'rgs');
  assert.equal(p.reporting.defaultChart.length, 29);
  for (const a of p.reporting.defaultChart) {
    assert.ok(['asset', 'liability', 'equity', 'income', 'expense'].includes(a.type));
    assert.ok(['debit', 'credit'].includes(a.normalBalance));
    assert.equal(typeof a.taxonomyCode, 'string');
    assert.ok(a.taxonomyCode.length > 0);
  }
  // legacy RGS_LABELS count
  assert.equal(Object.keys(p.reporting.labels).length, 16);
  assert.equal(p.reporting.labels['WOMZ.80'], 'Omzet');
  // jaarrekening line lists (Titel 9 BW2 micro/klein)
  assert.deepEqual(p.reporting.statutoryAccounts.models, ['micro', 'klein']);
  assert.equal(p.reporting.statutoryAccounts.lines.activa.length, 6);
  assert.equal(p.reporting.statutoryAccounts.lines.passiva.length, 4);
  assert.equal(p.reporting.statutoryAccounts.lines.pnl.length, 7);
});

test('NL profile integrity — identifiers, compliance, documents, closing', () => {
  const p = getProfile('NL');
  assert.equal(p.identifiers.companyIdLabel, 'registration_id');
  assert.equal(p.identifiers.vatIdLabel, 'tax_id');
  assert.equal(p.identifiers.peppolSchemeId, '9944');
  assert.deepEqual(p.identifiers.accountNumber, { kind: 'iban' });
  assert.deepEqual(p.meta.legalForms, ['eenmanszaak', 'vof', 'bv', 'nv', 'stichting', 'vereniging']);
  // legacy compliance trio
  assert.deepEqual(p.compliance.filingTypes.map((f) => f.type), ['OB', 'ICP', 'JAARREKENING']);
  assert.equal(p.compliance.filingTypes[0].deadlineRule, 'nl-quarterly');
  assert.equal(p.compliance.filingTypes[2].deadlineRule, 'nl-13-months');
  assert.equal(p.documents.invoiceCompliance, 'nl-12-vereisten');
  assert.equal(p.documents.eInvoicing, 'peppol-bis-3.0');
  assert.equal(p.documents.auditFile, 'xaf-auditfile-4.0');
  assert.equal(p.exchange.fxSource, 'ecb');
  assert.equal(p.exchange.baseCurrency, 'EUR');
  assert.deepEqual(p.closing, { resultAccount: '9900', equityAccount: '3000' });
});

test('normalizeCountry trims and uppercases', () => {
  assert.equal(normalizeCountry(' nl '), 'NL');
  assert.equal(normalizeCountry('GB'), 'GB');
  assert.equal(normalizeCountry(''), null);
  assert.equal(normalizeCountry(null), null);
});

test('resolveProfile returns the NL profile for a company with country NL', () => {
  const db = scratchDbAt(21, { sql: "INSERT INTO company (name, country) VALUES (?, ?)", params: ['Test BV', 'NL'] });
  try {
    const p = resolveProfile(db);
    assert.equal(p.meta.country, 'NL');
  } finally {
    db.close();
  }
});

test('resolveProfile defaults to NL on a pre-021 DB (no country column)', () => {
  const db = scratchDbAt(20, { sql: 'INSERT INTO company (name) VALUES (?)', params: ['Test BV'] });
  try {
    const p = resolveProfile(db);
    assert.equal(p.meta.country, 'NL');
  } finally {
    db.close();
  }
});

test('resolveProfile defaults to NL when no company row exists yet', () => {
  const db = scratchDbAt(21);
  try {
    const p = resolveProfile(db);
    assert.equal(p.meta.country, 'NL');
  } finally {
    db.close();
  }
});

test('resolveProfile throws for unsupported / unknown company countries (decision §9.1.6)', () => {
  const dbLT = scratchDbAt(21, { sql: "INSERT INTO company (name, country) VALUES (?, ?)", params: ['Test BV', 'IS'] });
  try {
    assert.throws(() => resolveProfile(dbLT), (e) => e.code === 'PROFILE_NOT_FOUND');
  } finally {
    dbLT.close();
  }
  const dbZZ = scratchDbAt(21, { sql: "INSERT INTO company (name, country) VALUES (?, ?)", params: ['Test BV', 'ZZ'] });
  try {
    assert.throws(() => resolveProfile(dbZZ), (e) => e.code === 'PROFILE_NOT_FOUND');
  } finally {
    dbZZ.close();
  }
});

// --- Phase A M3: init --country + generic identifier flags (CLI level) -----

const BIN = path.join(path.dirname(fileURLToPath(import.meta.url)), '..', 'bin', 'bukio.js');

function cli(dbPath, args, { expectFail = false } = {}) {
  const env = { ...process.env, BUKIO_DB: dbPath, BUKIO_ACTOR: 'agent:test' };
  const fullArgs = args.includes('--json') ? args : [...args, '--json'];
  try {
    const stdout = execFileSync(process.execPath, [BIN, ...fullArgs], { env, encoding: 'utf8' });
    return { code: 0, out: JSON.parse(stdout) };
  } catch (err) {
    if (expectFail) return { code: err.status, out: JSON.parse(err.stdout || '{}'), err: err.stderr };
    throw err;
  }
}

function tmpDb() {
  const dir = mkdtempSync(path.join(tmpdir(), 'bukio-jur-cli-'));
  return path.join(dir, 'test.db');
}

test('M3 init: --country IS (valid code, no profile) is rejected with PROFILE_NOT_FOUND', () => {
  const dbPath = tmpDb();
  const r = cli(dbPath, ['init', '--name', 'Test BV', '--country', 'IS'], { expectFail: true });
  assert.equal(r.code, 1);
  assert.equal(r.out.error.code, 'PROFILE_NOT_FOUND');
});

test('M3 init: --country ZZ (valid code, no profile) is rejected with PROFILE_NOT_FOUND', () => {
  const dbPath = tmpDb();
  const r = cli(dbPath, ['init', '--name', 'Test BV', '--country', 'ZZ'], { expectFail: true });
  assert.equal(r.code, 1);
  assert.equal(r.out.error.code, 'PROFILE_NOT_FOUND');
});

test('M3 init: --country nl (lowercase) normalizes to NL and stores profile fields', () => {
  const dbPath = tmpDb();
  const r = cli(dbPath, ['init', '--name', 'Test BV', '--country', 'nl', '--vat', 'on']);
  assert.equal(r.out.data.company.country, 'NL');
  assert.equal(r.out.data.company.base_currency, 'EUR');
  assert.equal(r.out.data.company.locale, 'nl');
  assert.equal(r.out.data.company.profile_version, 1);
  const show = cli(dbPath, ['company', 'show']);
  assert.equal(show.out.data.company.country, 'NL');
  assert.equal(show.out.data.company.base_currency, 'EUR');
  assert.equal(show.out.data.company.locale, 'nl');
  assert.equal(show.out.data.company.profile_version, 1);
});

test('M3 init: generic --registration-id/--tax-id are stored; no deprecation warning', () => {
  const dbPath = tmpDb();
  const r = cli(dbPath, ['init', '--name', 'Test BV', '--registration-id', '12345678', '--tax-id', 'NL123456789B01']);
  assert.equal(r.out.data.company.registration_id, '12345678');
  assert.equal(r.out.data.company.tax_id, 'NL123456789B01');
  assert.equal(r.out.data.warnings, undefined);
});

test('M3 init: legacy --kvk/--btw-id aliases map to the generic fields and warn', () => {
  const dbPath = tmpDb();
  const r = cli(dbPath, ['init', '--name', 'Test BV', '--kvk', '12345678', '--btw-id', 'NL123456789B01']);
  assert.equal(r.out.data.company.registration_id, '12345678');
  assert.equal(r.out.data.company.tax_id, 'NL123456789B01');
  assert.ok(r.out.data.warnings.some((w) => w.includes('--kvk is deprecated')));
  assert.ok(r.out.data.warnings.some((w) => w.includes('--btw-id is deprecated')));
});

test('M3 company update: changing country is rejected with COUNTRY_IMMUTABLE', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Test BV']);
  const r = cli(dbPath, ['company', 'update', '--country', 'US'], { expectFail: true });
  assert.equal(r.code, 1);
  assert.equal(r.out.error.code, 'COUNTRY_IMMUTABLE');
});

test('M3 company update: --country with the SAME value passes the immutability gate', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Test BV']);
  const r = cli(dbPath, ['company', 'update', '--country', 'nl', '--city', 'Amsterdam']);
  assert.equal(r.out.data.company.country, 'NL');
  assert.equal(r.out.data.company.city, 'Amsterdam');
});

test('M3 company update: --kvk alias warns and updates registration_id', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Test BV']);
  const r = cli(dbPath, ['company', 'update', '--kvk', '87654321']);
  assert.equal(r.out.data.company.registration_id, '87654321');
  assert.ok(r.out.data.warnings.some((w) => w.includes('--kvk is deprecated')));
});

test('M3 company update: generic --registration-id/--tax-id work without warnings', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Test BV']);
  const r = cli(dbPath, ['company', 'update', '--registration-id', '11112222', '--tax-id', 'NL999999999B01']);
  assert.equal(r.out.data.company.registration_id, '11112222');
  assert.equal(r.out.data.company.tax_id, 'NL999999999B01');
  assert.equal(r.out.data.warnings, undefined);
});

// --- Phase A M4: profile indirection is live in the VAT + compliance paths --

test('M4: obReadout resolves the profile (unknown company country -> PROFILE_NOT_FOUND)', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Test BV', '--vat', 'on']);
  const db = openDb(dbPath);
  db.prepare("UPDATE company SET country = 'ZZ' WHERE id = 1").run();
  db.close();
  const r = cli(dbPath, ['vat', 'readout', '--period', '2026-Q2'], { expectFail: true });
  assert.equal(r.out.error.code, 'PROFILE_NOT_FOUND');
});

test('M4: validateCompliance resolves the profile (unknown company country -> PROFILE_NOT_FOUND)', () => {
  const db = scratchDbAt(22);
  try {
    db.prepare("INSERT INTO company (name, country) VALUES ('Test BV', 'ZZ')").run();
    assert.throws(
      () => validateCompliance(db, { contact: { name: 'X', address: 'A', city: 'C' }, lines: [] }),
      (e) => e.code === 'PROFILE_NOT_FOUND',
    );
  } finally {
    db.close();
  }
});

// --- Phase A M5: reporting path resolves the profile ------------------------

test('M5: jaarrekening resolves the profile (unknown company country -> PROFILE_NOT_FOUND)', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Test BV', '--vat', 'on']);
  const db = openDb(dbPath);
  db.prepare("UPDATE company SET country = 'ZZ' WHERE id = 1").run();
  db.close();
  const r = cli(dbPath, ['financial-statements', 'report', '--year', '2025'], { expectFail: true });
  assert.equal(r.out.error.code, 'PROFILE_NOT_FOUND');
});

test('M5: deprecated alias `jaarrekening report` still works and warns', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Test BV', '--vat', 'on']);
  const r = cli(dbPath, ['jaarrekening', 'report', '--year', '2026', '--format', 'json']);
  assert.equal(r.out.data.financial_statements.year, '2026');
  assert.ok(r.out.data.warnings.some((w) => w.includes('jaarrekening is deprecated')));
});

// --- Phase A M6: compliance calendar resolves the profile -------------------

test('M6: compliance status resolves the profile (unknown company country -> PROFILE_NOT_FOUND)', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Test BV', '--vat', 'on']);
  const db = openDb(dbPath);
  db.prepare("UPDATE company SET country = 'ZZ' WHERE id = 1").run();
  db.close();
  const r = cli(dbPath, ['compliance', 'status', '--year', '2026'], { expectFail: true });
  assert.equal(r.out.error.code, 'PROFILE_NOT_FOUND');
});

// --- Phase A M7: e-invoicing resolves the profile ---------------------------

test('M7: invoiceToUbl resolves the profile (unknown company country -> PROFILE_NOT_FOUND)', () => {
  const db = scratchDbAt(22);
  try {
    db.prepare("INSERT INTO company (name, registration_id, tax_id, country) VALUES ('Test BV', '12345678', 'NL123456789B01', 'ZZ')").run();
    assert.throws(
      () => invoiceToUbl(db, { invoice_type: 'invoice', lines: [], contact: {} }),
      (e) => e.code === 'PROFILE_NOT_FOUND',
    );
  } finally {
    db.close();
  }
});

// --- Phase A M8: year-end close + payments resolve the profile --------------

test('M8: year-end close resolves the profile (unknown company country -> PROFILE_NOT_FOUND)', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Test BV', '--vat', 'on']);
  const db = openDb(dbPath);
  db.prepare("UPDATE company SET country = 'ZZ' WHERE id = 1").run();
  db.close();
  const r = cli(dbPath, ['year-end', 'close', '--year', '2026'], { expectFail: true });
  assert.equal(r.out.error.code, 'PROFILE_NOT_FOUND');
});

// --- Phase A M9: export + bank import resolve the profile -------------------

test('M9: exportXaf resolves the profile (unknown company country -> PROFILE_NOT_FOUND)', () => {
  const db = scratchDbAt(22);
  try {
    db.prepare("INSERT INTO company (name, registration_id, tax_id, country) VALUES ('Test BV', '12345678', 'NL123456789B01', 'ZZ')").run();
    assert.throws(
      () => exportXaf(db, { year: 2026, out: '/tmp/xaf-test.xml' }),
      (e) => e.code === 'PROFILE_NOT_FOUND',
    );
  } finally {
    db.close();
  }
});

test('M9: bank import resolves the profile (unknown company country -> PROFILE_NOT_FOUND)', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Test BV', '--vat', 'on']);
  const db = openDb(dbPath);
  db.prepare("UPDATE company SET country = 'ZZ' WHERE id = 1").run();
  db.close();
  const csvPath = path.join(path.dirname(dbPath), 'tx.csv');
  writeFileSync(csvPath, 'date,description,amount,iban\n2026-01-05,test,100.00,IBAN123\n');
  const r = cli(dbPath, ['bank', 'import', '--file', csvPath, '--iban', 'NL00BANK0123456789', '--dry-run'], { expectFail: true });
  assert.equal(r.out.error.code, 'PROFILE_NOT_FOUND');
});

// --- review-fix: account taxonomy flag (regression + alias coverage) --------

test('review-fix: account add --taxonomy-code works; --rgs-code alias maps and warns', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Test BV']);
  const primary = cli(dbPath, ['account', 'add', '--code', '1300', '--name', 'Testrekening', '--type', 'asset', '--normal-balance', 'debit', '--taxonomy-code', 'BMVA.02', '--dry-run']);
  assert.equal(primary.out.data.account.taxonomy_code, 'BMVA.02');
  const alias = cli(dbPath, ['account', 'add', '--code', '1300', '--name', 'Testrekening', '--type', 'asset', '--normal-balance', 'debit', '--rgs-code', 'BMVA.02', '--dry-run']);
  assert.equal(alias.out.data.account.taxonomy_code, 'BMVA.02');
  assert.ok(alias.out.data.warnings.some((w) => w.includes('--rgs-code is deprecated')));
  const conflict = cli(dbPath, ['account', 'add', '--code', '1300', '--name', 'Testrekening', '--type', 'asset', '--normal-balance', 'debit', '--taxonomy-code', 'BMVA.02', '--rgs-code', 'BFVA.03', '--dry-run']);
  assert.equal(conflict.out.data.account.taxonomy_code, 'BMVA.02'); // primary wins
  assert.ok(conflict.out.data.warnings.some((w) => w.includes('--rgs-code ignored')));
});

// --- Phase B B1: Luxembourg profile (PCN 2020, French labels) ---------------

test('B1: getProfile returns the LU profile (French, PCN 2020 data)', () => {
  const p = getProfile('LU');
  assert.equal(p.meta.country, 'LU');
  assert.equal(p.meta.baseCurrency, 'EUR');
  assert.equal(p.meta.locale, 'fr-lu');
  assert.ok(p.meta.legalForms.includes('sarl'));
  assert.ok(!p.meta.legalForms.includes('bv')); // NL legal form rejected for LU
  assert.equal(p.identifiers.peppolSchemeId, '0195'); // RCS registry code (BT-34/BT-49)
  assert.ok(p.identifiers.vatIdFormat.test('LU12345678'));
  assert.equal(p.tax.standardRateBp, 1700);
  assert.equal(p.tax.smallBusinessScheme, 'franchise'); // €50K franchise en base
  assert.deepEqual(p.tax.codes.map((c) => c.code), ['17', '14', '8', '3', '0', 'V', 'R', 'RE', 'M', 'P']);
  // PCN VAT ledger (official RGD annex): 421611 TVA en amont / 461411 TVA en aval
  assert.deepEqual(p.tax.accounts.ledger.map((a) => a.code), ['421611', '461411']);
  assert.equal(p.tax.accounts.fileDefault, '461412');
  // PCN has NO class 8: the annual result is 142, appropriation to 1411
  assert.equal(p.closing.resultAccount, '142');
  assert.equal(p.closing.equityAccount, '1411');
  assert.equal(p.reporting.taxonomy, 'pcn');
  // chart codes verbatim from the official annex (hierarchical 2-6 digits)
  assert.ok(p.reporting.defaultChart.some((a) => a.code === '516' && a.name === 'Caisse'));
  assert.ok(p.reporting.defaultChart.some((a) => a.code === '421611' && a.name === 'TVA en amont'));
  assert.ok(p.reporting.defaultChart.some((a) => a.code === '7021' && a.name === 'Ventes de produits finis'));
  // B1 scope: formats without an engine are deliberately unregistered
  // (strict dispatch fails loudly instead of producing Dutch output)
  assert.equal(p.tax.returnLayout, undefined);
  // B3: the FAIA audit-file format is registered
  assert.equal(p.documents.auditFile, 'faia-2.01-reduced-b');
  // B5: the LU compliance calendar is registered (quarterly TVA default band
  // + annual accounts within ~7 months of the FY end)
  assert.deepEqual(p.compliance.filingTypes, [
    { type: 'TVA', periodShape: 'YYYY-Qn', deadlineRule: 'lu-quarterly' },
    { type: 'COMPTES_ANNUELS', periodShape: 'YYYY', deadlineRule: 'lu-7-months' },
  ]);
  // B2: the LU LSC statutory layout is registered (abridged model)
  assert.equal(p.reporting.format, 'lu-lsc');
  assert.deepEqual(p.reporting.statutoryAccounts.models, ['abrege']);
  assert.ok(p.reporting.statutoryAccounts.lines.activa.length >= 5);
  // B6: the LU invoice compliance rule set is registered
  assert.equal(p.documents.invoiceCompliance, 'lu-invoice-vereisten');
  // e-invoicing IS registered: B2G mandatory in LU since 18 Mar 2023
  assert.equal(p.documents.eInvoicing, 'peppol-bis-3.0');
  assert.deepEqual(p.exchange.paymentFormats, ['sepa-pain.001', 'sepa-pain.008']);
});

test('B1: LU is implemented — PLANNED is empty (all thirty-one markets landed)', () => {
  assert.ok(!PLANNED.includes('LU'));
  assert.deepEqual([...PLANNED].sort(), []);
  assert.equal(getProfile('LU').meta.country, 'LU');
  for (const cc of PLANNED) {
    assert.throws(() => getProfile(cc), (e) => e.code === 'COUNTRY_NOT_SUPPORTED');
  }
});

test('B1: the LU profile is deep-frozen', () => {
  const p = getProfile('LU');
  assert.ok(Object.isFrozen(p));
  assert.ok(Object.isFrozen(p.tax));
  assert.ok(Object.isFrozen(p.tax.codes));
  assert.ok(Object.isFrozen(p.reporting.defaultChart));
  assert.throws(() => { p.tax.standardRateBp = 9999; }, TypeError);
});

test('B1: init --country LU creates a French LU company with the PCN chart', () => {
  const dbPath = tmpDb();
  const r = cli(dbPath, ['init', '--name', 'Sàrl Test', '--country', 'LU', '--legal-form', 'sarl', '--vat', 'on']);
  assert.equal(r.out.data.company.country, 'LU');
  assert.equal(r.out.data.company.base_currency, 'EUR');
  assert.equal(r.out.data.company.locale, 'fr-lu');
  const db = openDb(dbPath);
  try {
    const c = db.prepare('SELECT country, base_currency, locale, profile_version FROM company WHERE id = 1').get();
    assert.deepEqual(c, { country: 'LU', base_currency: 'EUR', locale: 'fr-lu', profile_version: 1 });
    const accounts = db.prepare('SELECT code, name, taxonomy FROM accounts WHERE active = 1').all();
    assert.ok(accounts.some((a) => a.code === '516' && a.name === 'Caisse'));
    assert.ok(accounts.some((a) => a.code === '421611' && a.name === 'TVA en amont'));
    assert.ok(accounts.some((a) => a.code === '7021'));
    assert.ok(accounts.length >= 40);
    // LU accounts carry the pcn taxonomy discriminator (createAccount fix)
    for (const a of accounts) assert.equal(a.taxonomy, 'pcn');
  } finally {
    db.close();
  }
  // NL legal forms are rejected for an LU company
  const bad = cli(tmpDb(), ['init', '--name', 'X', '--country', 'LU', '--legal-form', 'bv'], { expectFail: true });
  assert.equal(bad.out.error.code, 'INVALID_LEGAL_FORM');
  // KOR is an NL-only scheme — rejected for LU (franchise en base instead)
  const kor = cli(tmpDb(), ['init', '--name', 'X', '--country', 'LU', '--legal-form', 'sarl', '--kor'], { expectFail: true });
  assert.equal(kor.out.error.code, 'INVALID_VAT_CHOICE');
  // NL companies are unchanged: seeded accounts keep the 'rgs' discriminator
  const nlPath = tmpDb();
  cli(nlPath, ['init', '--name', 'Test BV', '--vat', 'on']);
  const nlDb = openDb(nlPath);
  try {
    assert.equal(nlDb.prepare("SELECT taxonomy FROM accounts WHERE code = '1000'").get().taxonomy, 'rgs');
  } finally {
    nlDb.close();
  }
});

test('B1: LU strict dispatch — unregistered formats fail loudly (no NL fallback)', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Sàrl Test', '--country', 'LU', '--legal-form', 'sarl', '--vat', 'on']);
  // VAT readout: the LU eCDF return layout is a B-milestone
  let r = cli(dbPath, ['vat', 'readout', '--period', '2026-Q1'], { expectFail: true });
  assert.equal(r.out.error.code, 'FORMAT_NOT_SUPPORTED');
  // invoice compliance is registered since B6 (tested in the B6 section)
});

test('B1: LU UBL invoice emits the RCS scheme 0195 and country LU (never 9944)', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Sàrl Test', '--country', 'LU', '--legal-form', 'sarl', '--registration-id', 'B123456', '--tax-id', 'LU12345678', '--vat', 'on']);
  cli(dbPath, ['company', 'update', '--address', '1 rue du Test', '--postal-code', 'L-1234', '--city', 'Luxembourg']);
  const db = openDb(dbPath);
  try {
    createContact(db, {
      name: 'Client SARL', address: '1 rue du Test', postalCode: 'L-1234', city: 'Luxembourg',
      vatId: 'LU99999999', kvk: 'B654321', actor: 'agent:test',
    });
    const inv = createInvoice(db, {
      contactId: 1, date: '2026-08-15', lines: ['1x Prestation @ 100.00 @17'], actor: 'agent:test',
    });
    finalizeInvoice(db, { id: inv.id, actor: 'agent:test' }); // B6: LU compliance passes
    const xml = invoiceToUbl(db, getInvoice(db, inv.id));
    // BT-34/BT-49: RCS scheme 0195, never the Dutch KVK scheme 9944
    assert.ok(xml.includes('schemeID="0195"'), 'endpoints use the RCS scheme 0195');
    assert.ok(!xml.includes('schemeID="9944"'), 'never the Dutch KVK scheme');
    assert.match(xml, /<cbc:IdentificationCode>LU<\/cbc:IdentificationCode>/);
    assert.ok(xml.includes('LU99999999'), 'buyer carries the LU TVA id');
  } finally {
    db.close();
  }
});

// --- Phase B B6: LU invoice compliance rule set -----------------------------

test('B6: LU invoice finalizes end-to-end (compliance rule set registered)', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Sàrl Test', '--country', 'LU', '--legal-form', 'sarl', '--registration-id', 'B123456', '--tax-id', 'LU12345678', '--vat', 'on']);
  cli(dbPath, ['company', 'update', '--address', '1 rue du Test', '--postal-code', 'L-1234', '--city', 'Luxembourg']);
  const db = openDb(dbPath);
  try {
    createContact(db, {
      name: 'Client SARL', address: '1 rue du Test', postalCode: 'L-1234', city: 'Luxembourg',
      vatId: 'LU99999999', actor: 'agent:test',
    });
    const inv = createInvoice(db, {
      contactId: 1, date: '2026-08-15', lines: ['1x Prestation @ 100.00 @17'], actor: 'agent:test',
    });
    const fin = finalizeInvoice(db, { id: inv.id, actor: 'agent:test' });
    assert.equal(fin.invoice.invoice_number, '2026-0001');
    assert.equal(fin.invoice.status, 'sent'); // finalized
    assert.equal(fin.entry.state, 'posted');
    // books: debiteuren 4011 / omzet 7021 / TVA en aval 461411 (LU chart)
    const postings = db.prepare(
      "SELECT a.code, p.amount_cents FROM postings p JOIN accounts a ON a.id = p.account_id JOIN journal_entries e ON e.id = p.entry_id WHERE e.id = ? ORDER BY a.code",
    ).all(fin.entry.id);
    const codes = postings.map((p) => `${p.code}:${p.amount_cents}`).join(' ');
    assert.ok(codes.includes('4011:11700'), `debiteuren leg on the PCN debtors account: ${codes}`);
    assert.ok(codes.includes('7021:-10000'), `revenue on 7021: ${codes}`);
    assert.ok(codes.includes('461411:-1700'), `output VAT on 461411: ${codes}`);
  } finally {
    db.close();
  }
});

test('B6: LU supplier requirements — missing RCS / TVA fail with French messages', () => {
  // LU company WITHOUT a registration id (RCS number)
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Sàrl Test', '--country', 'LU', '--legal-form', 'sarl', '--vat', 'on']);
  cli(dbPath, ['company', 'update', '--address', '1 rue du Test', '--postal-code', 'L-1234', '--city', 'Luxembourg']);
  const db = openDb(dbPath);
  try {
    createContact(db, {
      name: 'Client SARL', address: '1 rue du Test', postalCode: 'L-1234', city: 'Luxembourg',
      vatId: 'LU99999999', actor: 'agent:test',
    });
    const inv = createInvoice(db, {
      contactId: 1, date: '2026-08-15', lines: ['1x Prestation @ 100.00 @17'], actor: 'agent:test',
    });
    assert.throws(
      () => finalizeInvoice(db, { id: inv.id, actor: 'agent:test' }),
      (e) => e.code === 'SUPPLIER_INCOMPLETE' && e.message.includes('numéro RCS'),
    );
    // a TVA-registered LU supplier without a TVA number also fails
    db.prepare("UPDATE company SET registration_id = 'B123456' WHERE id = 1").run();
    assert.throws(
      () => finalizeInvoice(db, { id: inv.id, actor: 'agent:test' }),
      (e) => e.code === 'SUPPLIER_INCOMPLETE' && e.message.includes('numéro de TVA'),
    );
  } finally {
    db.close();
  }
});

test('B6: LU reverse charge requires the customer TVA number (auto-liquidation)', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Sàrl Test', '--country', 'LU', '--legal-form', 'sarl', '--registration-id', 'B123456', '--tax-id', 'LU12345678', '--vat', 'on']);
  cli(dbPath, ['company', 'update', '--address', '1 rue du Test', '--postal-code', 'L-1234', '--city', 'Luxembourg']);
  const db = openDb(dbPath);
  try {
    createContact(db, {
      name: 'Client SARL', address: '1 rue du Test', postalCode: 'L-1234', city: 'Luxembourg',
      actor: 'agent:test', // no vat_id
    });
    const inv = createInvoice(db, {
      contactId: 1, date: '2026-08-15', lines: ['1x Prestation @ 100.00 @RE'], actor: 'agent:test',
    });
    assert.throws(
      () => finalizeInvoice(db, { id: inv.id, actor: 'agent:test' }),
      (e) => e.code === 'CUSTOMER_VAT_REQUIRED' && e.message.includes('auto-liquidation'),
    );
    // with the customer TVA id the same invoice finalizes
    db.prepare("UPDATE contacts SET vat_id = 'FR12345678901' WHERE id = 1").run();
    const fin = finalizeInvoice(db, { id: inv.id, actor: 'agent:test' });
    assert.equal(fin.invoice.status, 'sent');
    assert.equal(fin.entry.state, 'posted');
  } finally {
    db.close();
  }
});

test('B6: NL invoice compliance is unchanged (byte-identical, nl-12-vereisten)', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Test BV', '--registration-id', '12345678', '--tax-id', 'NL123456789B01', '--vat', 'on']);
  cli(dbPath, ['company', 'update', '--address', 'Industrieweg 12', '--postal-code', '2712 CD', '--city', 'Zoetermeer']);
  const db = openDb(dbPath);
  try {
    createContact(db, {
      name: 'ACME B.V.', address: 'Straat 1', postalCode: '1000 AA', city: 'Amsterdam',
      vatId: 'NL999999999B01', actor: 'agent:test',
    });
    const inv = createInvoice(db, {
      contactId: 1, date: '2026-08-15', lines: ['2x Consultancy @ 150.00 @21'], actor: 'agent:test',
    });
    const fin = finalizeInvoice(db, { id: inv.id, actor: 'agent:test' });
    assert.equal(fin.invoice.status, 'sent');
    assert.equal(fin.entry.state, 'posted');
    // the NL rule set still reports 12 vereisten
    assert.equal(validateCompliance(db, getInvoice(db, inv.id)).vereisten, 12);
  } finally {
    db.close();
  }
});

// --- Phase B B2: LU statutory accounts (LSC abridged layout) -----------------

test('B2: LU financial statements report the LSC abridged layout', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Sàrl Test', '--country', 'LU', '--legal-form', 'sarl', '--registration-id', 'B123456', '--tax-id', 'LU12345678', '--vat', 'on']);
  cli(dbPath, ['company', 'update', '--address', '1 rue du Test', '--postal-code', 'L-1234', '--city', 'Luxembourg']);
  const db = openDb(dbPath);
  try {
    createContact(db, {
      name: 'Client SARL', address: '1 rue du Test', postalCode: 'L-1234', city: 'Luxembourg',
      vatId: 'LU99999999', actor: 'agent:test',
    });
    const inv = createInvoice(db, {
      contactId: 1, date: '2026-08-15', lines: ['1x Prestation @ 100.00 @17'], actor: 'agent:test',
    });
    finalizeInvoice(db, { id: inv.id, actor: 'agent:test' }); // books 4011 +11700 / 7021 -10000 / 461411 -1700
  } finally {
    db.close();
  }
  const r = cli(dbPath, ['financial-statements', 'report', '--year', '2026', '--format', 'json']);
  const fs = r.out.data.financial_statements;
  assert.equal(fs.model, 'abrege'); // profile-driven default, not 'klein'
  assert.equal(fs.as_of, '2026-12-31');
  assert.equal(fs.balans.balanced, true);
  const activa = fs.balans.activa;
  const passiva = fs.balans.passiva;
  assert.ok(activa.some((l) => l.label === 'Actif circulant' && l.total_cents === 11700), 'debtors leg on Actif circulant');
  assert.ok(passiva.some((l) => l.label === 'Dettes' && l.total_cents === 1700), 'output VAT on Dettes');
  assert.ok(passiva.some((l) => l.label === 'Capitaux propres' && l.total_cents === 10000), 'unclosed result folded into Capitaux propres');
  const ca = fs.pnl.lines.find((l) => l.label === "Chiffre d'affaires net");
  assert.ok(ca && ca.total_cents === 10000, 'revenue on the CA line');
  assert.equal(fs.pnl.resultat_cents, 10000);
});

test('DE: UBL reverse-charge line percent is profile-driven, not NL 21.00 (review fix)', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Test GmbH', '--country', 'DE', '--legal-form', 'gmbh', '--vat', 'on']);
  const db = openDb(dbPath);
  try {
    const contact = createContact(db, { name: 'Kunde', address: 'Str 1', postalCode: '10115', city: 'Berlin', vatId: 'DE999999999', actor: 'agent:test' });
    const inv = createInvoice(db, { contactId: contact.id, date: '2026-07-10', lines: ['Beratung @ 1 @ 100.00 @R'] });
    const xml = invoiceToUbl(db, getInvoice(db, inv.id));
    const parser = new XMLParser({ ignoreAttributes: false, removeNSPrefix: true, parseTagValue: false });
    const root = parser.parse(xml)['Invoice'];
    // the AE line percent must follow the DE profile (19%), not the NL 21.00
    assert.equal(root['InvoiceLine']['Item']['ClassifiedTaxCategory']['Percent'], '19.00');
    // the buyer country falls back to the profile country (DE), not NL
    assert.equal(root['AccountingCustomerParty']['Party']['PostalAddress']['Country']['IdentificationCode'], 'DE');
    const subtotal = root['TaxTotal']['TaxSubtotal'];
    const sub = Array.isArray(subtotal) ? subtotal.find((s) => s['TaxCategory']['ID'] === 'AE') : subtotal;
    assert.ok(sub, 'AE TaxSubtotal present');
    assert.equal(sub['TaxCategory']['Percent'], '19.00');
  } finally {
    db.close();
  }
});

test('BE: vat book auto VAT legs land on the profile ledger, not NL 2500/1500 (review fix)', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Test BV', '--country', 'BE', '--legal-form', 'bv', '--vat', 'on']);
  cli(dbPath, ['vat', 'book', '--date', '2026-08-15', '--desc', 'omzet', '--postings', '700:-1000@21,400:1210', '--post']);
  const tb = cli(dbPath, ['report', 'trial-balance']);
  const accounts = tb.out.data.accounts;
  const out = accounts.find((a) => a.code === '451');
  assert.ok(out && out.net_cents === -21000, 'output VAT leg on 451 (BE ledger), not 2500');
  assert.ok(!accounts.some((a) => a.code === '2500' || a.code === '1500'), 'no NL clearing accounts created');
  assert.equal(tb.out.data.balanced, true);
});

test('FR: vat book accepts dotted VAT codes (@5.5) and posts to 44571 (review fix)', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'SARL Test', '--country', 'FR', '--legal-form', 'sarl', '--vat', 'on']);
  cli(dbPath, ['vat', 'book', '--date', '2026-08-15', '--desc', 'omzet', '--postings', '701:-1000@5.5,411:1055', '--post']);
  const tb = cli(dbPath, ['report', 'trial-balance']);
  const accounts = tb.out.data.accounts;
  const out = accounts.find((a) => a.code === '44571');
  assert.ok(out && out.net_cents === -5500, '5.5% output leg on 44571');
  assert.equal(tb.out.data.balanced, true);
});

test('BE: vat file/settle resolve the profile defaults via the CLI (review fix)', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Test BV', '--country', 'BE', '--legal-form', 'bv', '--vat', 'on']);
  cli(dbPath, ['entry', 'add', '--date', '2026-06-30', '--desc', 'omzet', '--postings', '400:1210,700:-1000,451:-210', '--post']);
  // file: the af-te-dragen default is the BE profile's fileDefault (451) —
  // the CLI used to hardcode the NL 2510
  const plan = cli(dbPath, ['vat', 'file', '--dry-run']);
  assert.equal(plan.out.data.account, '451');
  assert.equal(plan.out.data.owe, true);
  const filed = cli(dbPath, ['vat', 'file']);
  assert.ok(filed.out.data.entry_id, 'vat file posted');
  // settle: the difference account defaults to the BE profile's 648
  const db = openDb(dbPath);
  try {
    const settle = vatSettle(db, { txAmountCents: -21000, txDate: '2026-07-01', bankAccountCode: '550', dryRun: true });
    assert.equal(settle.difference_account, '648');
    assert.equal(settle.difference_cents, 0);
  } finally {
    db.close();
  }
});

test('B2: LU P&L — mixed leftover (custom expense + custom income) reconciles (review fix)', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Sàrl Test', '--country', 'LU', '--legal-form', 'sarl', '--vat', 'on']);
  cli(dbPath, ['account', 'add', '--code', '6600', '--name', 'Autres charges', '--type', 'expense', '--normal-balance', 'debit']);
  cli(dbPath, ['account', 'add', '--code', '7700', '--name', 'Autres produits', '--type', 'income', '--normal-balance', 'credit']);
  cli(dbPath, ['entry', 'add', '--date', '2026-06-30', '--desc', 'exercice', '--postings',
    '101:-1000,5131:1000,4011:11700,7021:-10000,461411:-1700,6600:500,5131:-500,7700:-200,5131:200', '--post']);
  const r = cli(dbPath, ['financial-statements', 'report', '--year', '2026', '--format', 'json']);
  const fs = r.out.data.financial_statements;
  assert.equal(fs.balans.balanced, true);
  const autres = fs.pnl.lines.find((l) => l.label === 'Autres');
  assert.ok(autres && autres.total_cents === 70000, 'display total is the raw sum (500 expense + 200 income)');
  // resultat uses the signed net: CA 10000 + custom income 200 - custom expense 500
  assert.equal(fs.pnl.resultat_cents, 970000, 'resultat reconciles for mixed leftovers');
});

test('B2: LU P&L — 73x subventions on line 4 and custom expenses subtract (review fix)', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Sàrl Test', '--country', 'LU', '--legal-form', 'sarl', '--vat', 'on']);
  // custom accounts outside the default chart: 7300 subventions (income)
  // and 6600 a custom expense account
  cli(dbPath, ['account', 'add', '--code', '7300', '--name', "Subventions d'exploitation", '--type', 'income', '--normal-balance', 'credit']);
  cli(dbPath, ['account', 'add', '--code', '6600', '--name', 'Autres charges', '--type', 'expense', '--normal-balance', 'debit']);
  // capital in, sales + VAT, custom expense paid from bank, subvention received
  cli(dbPath, ['entry', 'add', '--date', '2026-06-30', '--desc', 'exercice', '--postings',
    '101:-1000,5131:1000,4011:11700,7021:-10000,461411:-1700,6600:500,5131:-500,7300:-200,5131:200',
    '--post']);
  const r = cli(dbPath, ['financial-statements', 'report', '--year', '2026', '--format', 'json']);
  const fs = r.out.data.financial_statements;
  assert.equal(fs.balans.balanced, true);
  // 73x subventions map to line 4 'Autres produits d'exploitation'
  const autresProduits = fs.pnl.lines.find((l) => l.label === "Autres produits d'exploitation");
  assert.ok(autresProduits && autresProduits.total_cents === 20000, '73x subventions on line 4');
  // the custom 6600 expense lands in the 'Autres' catch-all and is
  // SUBTRACTED (sign -1) — not added to the result
  const autres = fs.pnl.lines.find((l) => l.label === 'Autres');
  assert.ok(autres && autres.total_cents === 50000, 'custom expense on the Autres catch-all');
  assert.equal(fs.pnl.resultat_cents, 970000, 'resultat = CA + subventions - charges - custom expense');
});

test('B2: cross-border buyer EndpointID uses the BUYER country scheme (review fix)', () => {
  // LU seller + NL buyer: the buyer's KVK was issued by the Dutch KVK
  // registry -> scheme 9944, NOT the seller's LU RCS scheme (0195)
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Sàrl Test', '--country', 'LU', '--legal-form', 'sarl', '--registration-id', 'B123456', '--tax-id', 'LU12345678', '--vat', 'on']);
  cli(dbPath, ['company', 'update', '--address', '1 rue du Test', '--postal-code', 'L-1234', '--city', 'Luxembourg']);
  const db = openDb(dbPath);
  try {
    const nl = createContact(db, {
      name: 'ACME B.V.', address: 'Straat 1', postalCode: '1000 AA', city: 'Amsterdam',
      country: 'NL', vatId: 'NL999999999B01', kvk: '98765432', actor: 'agent:test',
    });
    const inv = createInvoice(db, {
      contactId: nl.id, date: '2026-07-10', lines: ['1x Prestation @ 100.00 @17'], actor: 'agent:test',
    });
    finalizeInvoice(db, { id: inv.id, actor: 'agent:test' });
    const xml = invoiceToUbl(db, getInvoice(db, inv.id));
    assert.match(xml, /<cbc:EndpointID schemeID="9944">98765432<\/cbc:EndpointID>/);
    // same-market buyer (no country -> LU) still gets the seller's 0195
    const lu = createContact(db, {
      name: 'Sàrl LU', address: '1 rue du Test', postalCode: 'L-1234', city: 'Luxembourg',
      vatId: 'LU99999999', kvk: 'B123456', actor: 'agent:test',
    });
    const inv2 = createInvoice(db, {
      contactId: lu.id, date: '2026-07-10', lines: ['1x Prestation @ 100.00 @17'], actor: 'agent:test',
    });
    finalizeInvoice(db, { id: inv2.id, actor: 'agent:test' });
    const xml2 = invoiceToUbl(db, getInvoice(db, inv2.id));
    assert.match(xml2, /<cbc:EndpointID schemeID="0195">B123456<\/cbc:EndpointID>/);
    // unregistered-market buyer (PL is not among the 11 profiles): the
    // buyerSchemeId catch falls back to the seller's scheme (0195) for
    // unregistered markets (IS Iceland: valid code, no profile)
    const is = createContact(db, {
      name: 'Island ehf.', address: 'Gata 1', postalCode: '101', city: 'Reykjavik',
      country: 'IS', vatId: 'IS123456', kvk: '0000123456', actor: 'agent:test',
    });
    const inv3 = createInvoice(db, {
      contactId: is.id, date: '2026-07-10', lines: ['1x Prestation @ 100.00 @17'], actor: 'agent:test',
    });
    finalizeInvoice(db, { id: inv3.id, actor: 'agent:test' });
    const xml3 = invoiceToUbl(db, getInvoice(db, inv3.id));
    assert.match(xml3, /<cbc:EndpointID schemeID="0195">0000123456<\/cbc:EndpointID>/);
  } finally {
    db.close();
  }
});

test('B2: LU financial statements reject the NL model (INVALID_MODEL)', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Sàrl Test', '--country', 'LU', '--legal-form', 'sarl', '--vat', 'on']);
  const r = cli(dbPath, ['financial-statements', 'report', '--year', '2026', '--model', 'klein'], { expectFail: true });
  assert.equal(r.out.error.code, 'INVALID_MODEL');
  // the abridged model is the default for LU — no --model needed
  const ok = cli(dbPath, ['financial-statements', 'report', '--year', '2026']);
  assert.equal(ok.out.data.financial_statements.model, 'abrege');
});

test('B2: NL financial statements keep the klein default (byte-identical)', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Test BV', '--vat', 'on']);
  const r = cli(dbPath, ['financial-statements', 'report', '--year', '2026']);
  assert.equal(r.out.data.financial_statements.model, 'klein');
  // explicit micro still works
  const m = cli(dbPath, ['financial-statements', 'report', '--year', '2026', '--model', 'micro']);
  assert.equal(m.out.data.financial_statements.model, 'micro');
});

// --- Phase B B5: LU compliance calendar -------------------------------------

test('B5: LU compliance calendar — TVA on the 15th + annual accounts in 7 months', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Sàrl Test', '--country', 'LU', '--legal-form', 'sarl', '--vat', 'on']);
  const db = openDb(dbPath);
  try {
    const r = complianceStatus(db, { year: 2026 });
    const obs = r.obligations;
    // TVA quarterly on the 15th of the month after the quarter (not the NL
    // month-end deadlines)
    assert.equal(obs.find((o) => o.type === 'TVA' && o.period === '2026-Q1').deadline, '2026-04-15');
    assert.equal(obs.find((o) => o.type === 'TVA' && o.period === '2026-Q2').deadline, '2026-07-15');
    assert.equal(obs.find((o) => o.type === 'TVA' && o.period === '2026-Q3').deadline, '2026-10-15');
    assert.equal(obs.find((o) => o.type === 'TVA' && o.period === '2026-Q4').deadline, '2027-01-15');
    // the previous year's Q4 return falls in this calendar year (15 Jan)
    assert.equal(obs.find((o) => o.type === 'TVA' && o.period === '2025-Q4').deadline, '2026-01-15');
    // annual accounts: deposit within ~7 months of the FY end (LSC 2002 law)
    const ac = obs.find((o) => o.type === 'COMPTES_ANNUELS' && o.period === '2026');
    assert.equal(ac.deadline, '2027-07-31');
    // Q3 2026 deadline 2026-10-15: open until then, overdue after (date-aware,
    // so the suite does not flap when run after mid-October 2026; compared
    // against the engine's UTC today, not local time)
    const q3 = obs.find((o) => o.type === 'TVA' && o.period === '2026-Q3');
    // exact mirror of the engine rule (deadline < today -> overdue): on the
    // deadline day itself the obligation is still 'open'
    assert.equal(q3.status, '2026-10-15' < new Date().toISOString().slice(0, 10) ? 'overdue' : 'open');
    // no NL types leak into the LU calendar
    assert.ok(!obs.some((o) => ['OB', 'ICP', 'JAARREKENING'].includes(o.type)), 'no NL filing types in the LU calendar');
  } finally {
    db.close();
  }
});

test('B5: LU TVA filings mark through the registry and flip the status', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Sàrl Test', '--country', 'LU', '--legal-form', 'sarl', '--vat', 'on']);
  const db = openDb(dbPath);
  try {
    const marked = markFiled(db, { type: 'TVA', period: '2026-Q1', date: '2026-04-10', actor: 'agent:test' });
    assert.equal(marked.type, 'TVA');
    const r = complianceStatus(db, { year: 2026 });
    assert.equal(r.obligations.find((o) => o.type === 'TVA' && o.period === '2026-Q1').status, 'filed');
    // marking an unregistered type fails loudly
    assert.throws(() => markFiled(db, { type: 'OB', period: '2026-Q1' }), { code: 'INVALID_TYPE' });
  } finally {
    db.close();
  }
});

// --- Phase B B3: FAIA audit-file export -------------------------------------

test('B3: LU export xaf produces the FAIA 2.01 reduced-B audit file', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Sàrl Test', '--country', 'LU', '--legal-form', 'sarl', '--registration-id', 'B123456', '--tax-id', 'LU12345678', '--vat', 'on']);
  cli(dbPath, ['company', 'update', '--address', '1 rue du Test', '--postal-code', 'L-1234', '--city', 'Luxembourg']);
  const db = openDb(dbPath);
  try {
    createContact(db, {
      name: 'Client SARL', address: '1 rue du Test', postalCode: 'L-1234', city: 'Luxembourg',
      vatId: 'LU99999999', actor: 'agent:test',
    });
    const inv = createInvoice(db, {
      contactId: 1, date: '2026-08-15', lines: ['1x Prestation @ 100.00 @17'], actor: 'agent:test',
    });
    finalizeInvoice(db, { id: inv.id, actor: 'agent:test' }); // 4011 +117.00 / 7021 -100.00 / 461411 -17.00
  } finally {
    db.close();
  }
  const outPath = path.join(tmpdir(), `faia-${Date.now()}.xml`);
  const r = cli(dbPath, ['export', 'xaf', '--year', '2026', '--out', outPath]);
  assert.equal(r.out.data.mutaties, 1);
  const xml = readFileSync(outPath, 'utf8');
  // FAIA 2.01 reduced B: no namespace, AuditFile root, English elements
  assert.ok(xml.startsWith('<?xml version="1.0" encoding="UTF-8"?>'));
  assert.match(xml, /<AuditFile>/);
  assert.ok(!xml.includes('xmlns='), 'reduced B has no targetNamespace');
  assert.match(xml, /<AuditFileVersion>2\.01<\/AuditFileVersion>/);
  assert.match(xml, /<AuditFileCountry>LU<\/AuditFileCountry>/);
  assert.match(xml, /<RegistrationNumber>B123456<\/RegistrationNumber>/);
  assert.match(xml, /<TaxRegistrationNumber>LU12345678<\/TaxRegistrationNumber>/);
  assert.match(xml, /<TaxNumber>LU12345678<\/TaxNumber>/);
  // civil-year selection (FAIA requires complete civil years)
  assert.match(xml, /<SelectionStartDate>2026-01-01<\/SelectionStartDate>/);
  assert.match(xml, /<SelectionEndDate>2026-12-31<\/SelectionEndDate>/);
  // chart of accounts with French AccountType values + required balances
  assert.match(xml, /<AccountID>4011<\/AccountID>\s*<AccountDescription>Clients<\/AccountDescription>\s*<AccountType>Actif<\/AccountType>/);
  assert.match(xml, /<AccountID>7021<\/AccountID>\s*<AccountDescription>Ventes de produits finis<\/AccountDescription>\s*<AccountType>Produit<\/AccountType>/);
  assert.match(xml, /<AccountID>461411<\/AccountID>[\s\S]*?<AccountType>Passif<\/AccountType>/);
  assert.match(xml, /<ClosingDebitBalance>117\.00<\/ClosingDebitBalance>/);
  assert.match(xml, /<ClosingCreditBalance>100\.00<\/ClosingCreditBalance>/);
  // entries: one transaction, debit/credit pairs, balanced totals
  assert.match(xml, /<NumberOfEntries>1<\/NumberOfEntries>/);
  assert.match(xml, /<TotalDebit>117\.00<\/TotalDebit>/);
  assert.match(xml, /<TotalCredit>117\.00<\/TotalCredit>/);
  assert.match(xml, /<TransactionID>\d+<\/TransactionID>\s*<Period>8<\/Period>\s*<PeriodYear>2026<\/PeriodYear>/);
  assert.match(xml, /<DebitAmount><Amount>117\.00<\/Amount><\/DebitAmount>/);
  assert.match(xml, /<CreditAmount><Amount>100\.00<\/Amount><\/CreditAmount>/);
  // never the Dutch XAF root/namespace
  assert.ok(!xml.includes('<Xaf '), 'never the Dutch XAF root');
});


test('B3: FAIA omits the TaxTable for a TVA-less company (review fix)', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Sàrl Test', '--country', 'LU', '--legal-form', 'sarl', '--registration-id', 'B123456', '--tax-id', 'LU12345678', '--vat', 'on']);
  cli(dbPath, ['company', 'update', '--address', '1 rue du Test', '--postal-code', 'L-1234', '--city', 'Luxembourg']);
  const db = openDb(dbPath);
  try {
    db.prepare('UPDATE company SET tax_id = NULL WHERE id = 1').run();
  } finally {
    db.close();
  }
  cli(dbPath, ['entry', 'add', '--date', '2026-08-15', '--desc', 'test', '--postings', '4011:11700,7021:-10000,461411:-1700', '--post']);
  const outPath = path.join(tmpdir(), `faia-notax-${Date.now()}.xml`);
  const r = cli(dbPath, ['export', 'xaf', '--year', '2026', '--out', outPath]);
  assert.equal(r.out.data.mutaties, 1);
  const xml = readFileSync(outPath, 'utf8');
  // the TVA TaxTable keyrefs the TaxRegistration — a TVA-less company must
  // not declare a TVA table entry it cannot back with a registration
  assert.ok(!xml.includes('<TaxTable'), 'no TaxTable for a TVA-less company');
  assert.ok(!xml.includes('<TaxRegistration'), 'no TaxRegistration without a TVA number');
});
test('B3: NL XAF export is unchanged (byte-identical, xaf-auditfile-4.0)', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Test BV', '--registration-id', '12345678', '--tax-id', 'NL123456789B01', '--vat', 'on']);
  cli(dbPath, ['company', 'update', '--address', 'Industrieweg 12', '--postal-code', '2712 CD', '--city', 'Zoetermeer']);
  const db = openDb(dbPath);
  try {
    createContact(db, {
      name: 'ACME B.V.', address: 'Straat 1', postalCode: '1000 AA', city: 'Amsterdam',
      vatId: 'NL999999999B01', actor: 'agent:test',
    });
    const inv = createInvoice(db, {
      contactId: 1, date: '2026-08-15', lines: ['2x Consultancy @ 150.00 @21'], actor: 'agent:test',
    });
    finalizeInvoice(db, { id: inv.id, actor: 'agent:test' });
  } finally {
    db.close();
  }
  const outPath = path.join(tmpdir(), `xaf-${Date.now()}.xml`);
  const r = cli(dbPath, ['export', 'xaf', '--year', '2026', '--out', outPath]);
  assert.equal(r.out.data.mutaties, 1);
  const xml = readFileSync(outPath, 'utf8');
  assert.match(xml, /<Xaf xmlns="http:\/\/www\.auditfiles\.nl\/XAF\/4\.0">/);
  assert.match(xml, /<RekeningCode>1200<\/RekeningCode>/);
  assert.ok(!xml.includes('<AuditFile>'), 'NL still emits the Dutch XAF, never FAIA');
});

// --- Phase B: GB profile (UK conventions, GBP) ------------------------------

test('GB: getProfile returns the GB profile (GBP, en-GB, UK conventions)', () => {
  const p = getProfile('GB');
  assert.equal(p.meta.country, 'GB');
  assert.equal(p.meta.baseCurrency, 'GBP');
  assert.equal(p.meta.locale, 'en');
  assert.equal(p.meta.defaultFiscalYearEnd, '03-31'); // tax-year aligned
  assert.ok(p.meta.legalForms.includes('private-limited-company'));
  assert.ok(!p.meta.legalForms.includes('bv')); // NL form rejected
  assert.equal(p.tax.standardRateBp, 2000);
  assert.equal(p.tax.smallBusinessScheme, 'flat-rate');
  assert.deepEqual(p.tax.codes.map((c) => c.code), ['20', '5', '0', 'V', 'R', 'M', 'P']);
  // VAT control accounts per the UK chart convention
  assert.deepEqual(p.tax.accounts.ledger.map((a) => a.code), ['2110', '2100']);
  assert.equal(p.reporting.debtorsAccount, '1100');
  // UK closing: current-year result 3300 -> retained earnings 3200
  assert.equal(p.closing.resultAccount, '3300');
  assert.equal(p.closing.equityAccount, '3200');
  assert.equal(p.reporting.taxonomy, null); // no statutory taxonomy
  // chart per the QuickBooks/Xero convention (research §7)
  assert.ok(p.reporting.defaultChart.some((a) => a.code === '1000' && a.name === 'Bank — current account'));
  assert.ok(p.reporting.defaultChart.some((a) => a.code === '4000' && a.name === 'Sales — goods'));
  assert.ok(p.reporting.defaultChart.length >= 40);
  // B-milestones stay unregistered (strict dispatch fails loudly)
  assert.equal(p.tax.returnLayout, undefined);
  assert.equal(p.reporting.format, undefined);
  assert.equal(p.documents.invoiceCompliance, undefined); // B-milestone (no EU art. 226 baseline)
  assert.equal(p.documents.eInvoicing, undefined); // 2029 mandate, no Peppol scheme yet
  assert.equal(p.documents.auditFile, undefined);
  assert.deepEqual(p.exchange.paymentFormats, []); // SEPA is not a domestic rail
  // GB deadlines: annual accounts 9 months after FYE, CT600 12 months
  assert.deepEqual(p.compliance.filingTypes.map((ft) => ft.deadlineRule), ['gb-9-months', 'gb-ct600']);
});

test('GB: PLANNED is empty (all thirty-one markets landed)', () => {
  assert.ok(!PLANNED.includes('GB'));
  assert.deepEqual([...PLANNED].sort(), []);
  for (const cc of PLANNED) {
    assert.throws(() => getProfile(cc), (e) => e.code === 'COUNTRY_NOT_SUPPORTED');
  }
});

test('GB: init --country GB creates a GBP company with the UK chart', () => {
  const dbPath = tmpDb();
  const r = cli(dbPath, ['init', '--name', 'Test Ltd', '--country', 'GB', '--legal-form', 'private-limited-company', '--vat', 'on']);
  assert.equal(r.out.data.company.country, 'GB');
  assert.equal(r.out.data.company.base_currency, 'GBP');
  assert.equal(r.out.data.company.locale, 'en');
  const db = openDb(dbPath);
  try {
    const accounts = db.prepare('SELECT code, name, taxonomy FROM accounts WHERE active = 1').all();
    assert.ok(accounts.some((a) => a.code === '1000' && a.name === 'Bank — current account'));
    assert.ok(accounts.some((a) => a.code === '4000' && a.name === 'Sales — goods'));
    assert.ok(accounts.some((a) => a.code === '1100' && a.name === 'Trade debtors (accounts receivable)'));
    // no statutory taxonomy: GB account rows carry null
    for (const a of accounts) assert.equal(a.taxonomy, null);
  } finally {
    db.close();
  }
  // NL legal form rejected for GB
  const bad = cli(tmpDb(), ['init', '--name', 'X', '--country', 'GB', '--legal-form', 'bv'], { expectFail: true });
  assert.equal(bad.out.error.code, 'INVALID_LEGAL_FORM');
  // KOR is an NL-only scheme
  const kor = cli(tmpDb(), ['init', '--name', 'X', '--country', 'GB', '--legal-form', 'private-limited-company', '--kor'], { expectFail: true });
  assert.equal(kor.out.error.code, 'INVALID_VAT_CHOICE');
});

test('GB: strict dispatch — unregistered formats fail loudly (no fallback)', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Test Ltd', '--country', 'GB', '--legal-form', 'private-limited-company', '--vat', 'on']);
  // financial statements: FRS 102/105 iXBRL is a B-milestone
  let r = cli(dbPath, ['financial-statements', 'report', '--year', '2026'], { expectFail: true });
  assert.equal(r.out.error.code, 'FORMAT_NOT_SUPPORTED');
  // XAF export: no UK SAF-T
  r = cli(dbPath, ['export', 'xaf', '--year', '2026', '--out', '/tmp/xaf-gb.xml'], { expectFail: true });
  assert.equal(r.out.error.code, 'FORMAT_NOT_SUPPORTED');
  // VAT readout: the 9-box return engine is a B-milestone
  r = cli(dbPath, ['vat', 'readout', '--period', '2026-Q1'], { expectFail: true });
  assert.equal(r.out.error.code, 'FORMAT_NOT_SUPPORTED');
  // invoice compliance: reg. 14 rule set is a B-milestone
  const db = openDb(dbPath);
  try {
    assert.throws(
      () => validateCompliance(db, { invoice_type: 'invoice', lines: [] }),
      (e) => e.code === 'FORMAT_NOT_SUPPORTED',
    );
  } finally {
    db.close();
  }
});

test('GB: compliance calendar — annual accounts in 9 months, CT600 in 12', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Test Ltd', '--country', 'GB', '--legal-form', 'private-limited-company', '--vat', 'on']);
  const db = openDb(dbPath);
  try {
    const r = complianceStatus(db, { year: 2026 });
    const obs = r.obligations;
    // FY ending 2026-03-31: accounts due 2026-12-31 (+9 months), CT600 2027-03-31 (+12)
    assert.equal(obs.find((o) => o.type === 'ANNUAL_ACCOUNTS' && o.period === '2026').deadline, '2026-12-31');
    assert.equal(obs.find((o) => o.type === 'CT600' && o.period === '2026').deadline, '2027-03-31');
    // no NL or LU types leak into the GB calendar
    assert.ok(!obs.some((o) => ['OB', 'ICP', 'JAARREKENING', 'TVA', 'COMPTES_ANNUELS'].includes(o.type)));
  } finally {
    db.close();
  }
});

// --- Phase B: FR profile (PCG, TVA 20/10/5.5/2.1) ---------------------------

test('FR: getProfile returns the FR profile (EUR, fr, PCG data)', () => {
  const p = getProfile('FR');
  assert.equal(p.meta.country, 'FR');
  assert.equal(p.meta.baseCurrency, 'EUR');
  assert.equal(p.meta.locale, 'fr');
  assert.ok(p.meta.legalForms.includes('sarl'));
  assert.ok(!p.meta.legalForms.includes('bv')); // NL form rejected
  assert.equal(p.identifiers.peppolSchemeId, '0002'); // SIREN scheme
  assert.ok(p.identifiers.vatIdFormat.test('FR12345678901'));
  assert.equal(p.tax.standardRateBp, 2000);
  assert.equal(p.tax.smallBusinessScheme, 'franchise'); // €85K/€37.5K
  assert.deepEqual(p.tax.codes.map((c) => c.code), ['20', '10', '5.5', '2.1', '0', 'V', 'R', 'RE', 'M', 'P']);
  // PCG TVA accounts: 44566 input / 44571 output; settlement 44551
  assert.deepEqual(p.tax.accounts.ledger.map((a) => a.code), ['44566', '44571']);
  assert.equal(p.tax.accounts.fileDefault, '44551');
  assert.equal(p.reporting.debtorsAccount, '411');
  // PCG closing: 120 résultat -> 110 report à nouveau
  assert.equal(p.closing.resultAccount, '120');
  assert.equal(p.closing.equityAccount, '110');
  assert.equal(p.reporting.taxonomy, null);
  // chart: verified PCG codes incl. the contra-asset 281 (migration 023)
  assert.ok(p.reporting.defaultChart.some((a) => a.code === '512' && a.name === 'Banques'));
  assert.ok(p.reporting.defaultChart.some((a) => a.code === '706' && a.name === 'Prestations de services'));
  assert.ok(p.reporting.defaultChart.some((a) => a.code === '281' && a.normalBalance === 'credit'));
  assert.ok(p.reporting.defaultChart.length >= 40);
  // B-milestones stay unregistered (strict dispatch fails loudly)
  assert.equal(p.tax.returnLayout, undefined);
  assert.equal(p.reporting.format, undefined);
  assert.equal(p.documents.invoiceCompliance, 'eu-invoice-vereisten'); // art. 226 baseline (CGI additions B-milestone)
  assert.equal(p.documents.auditFile, undefined); // FEC is a B-milestone
  assert.deepEqual(p.compliance.filingTypes, []);
  // e-invoicing registered: EN 16931 UBL accepted in the FR mandate
  assert.equal(p.documents.eInvoicing, 'peppol-bis-3.0');
  assert.deepEqual(p.exchange.paymentFormats, ['sepa-pain.001', 'sepa-pain.008']);
});

test('FR: PLANNED is empty (all thirty-one markets landed)', () => {
  assert.ok(!PLANNED.includes('FR'));
  assert.deepEqual([...PLANNED].sort(), []);
});

test('FR: init --country FR creates a French company with the PCG chart', () => {
  const dbPath = tmpDb();
  const r = cli(dbPath, ['init', '--name', 'SAS Test', '--country', 'FR', '--legal-form', 'sas', '--vat', 'on']);
  assert.equal(r.out.data.company.country, 'FR');
  assert.equal(r.out.data.company.base_currency, 'EUR');
  assert.equal(r.out.data.company.locale, 'fr');
  const db = openDb(dbPath);
  try {
    const accounts = db.prepare('SELECT code, name, taxonomy FROM accounts WHERE active = 1').all();
    assert.ok(accounts.some((a) => a.code === '512' && a.name === 'Banques'));
    assert.ok(accounts.some((a) => a.code === '706' && a.name === 'Prestations de services'));
    assert.ok(accounts.some((a) => a.code === '281')); // contra-asset accepted (023)
    for (const a of accounts) assert.equal(a.taxonomy, null);
  } finally {
    db.close();
  }
  // NL legal form rejected for FR
  const bad = cli(tmpDb(), ['init', '--name', 'X', '--country', 'FR', '--legal-form', 'bv'], { expectFail: true });
  assert.equal(bad.out.error.code, 'INVALID_LEGAL_FORM');
  // KOR is an NL-only scheme
  const kor = cli(tmpDb(), ['init', '--name', 'X', '--country', 'FR', '--legal-form', 'sas', '--kor'], { expectFail: true });
  assert.equal(kor.out.error.code, 'INVALID_VAT_CHOICE');
});

test('FR: dotted VAT codes (5.5/2.1) parse in the invoice line spec (review fix)', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'SARL Test', '--country', 'FR', '--legal-form', 'sarl', '--vat', 'on']);
  const db = openDb(dbPath);
  try {
    const contact = createContact(db, { name: 'Client', address: 'Rue 1', postalCode: '75001', city: 'Paris', vatId: 'FR99999999999', actor: 'agent:test' });
    // '@5.5' must be recognised as the reduced VAT code, not mis-parsed as
    // a €5.50 price (the old parser only knew the NL codes + \d{1,2})
    const inv = createInvoice(db, { contactId: contact.id, date: '2026-07-10', lines: ['Prestation @ 1 @ 100.00 @5.5'] });
    assert.equal(inv.lines[0].vat_code, '5.5');
    assert.equal(inv.lines[0].vat_rate_bp, 550);
    assert.equal(inv.lines[0].amount_cents, 10000);
    // note: validateCompliance for FR fails FORMAT_NOT_SUPPORTED by design
    // (no FR compliance engine yet — strict dispatch) — the parse is what
    // this regression covers
  } finally {
    db.close();
  }
});

test('FR: strict dispatch — unregistered formats fail loudly (no fallback)', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'SAS Test', '--country', 'FR', '--legal-form', 'sas', '--vat', 'on']);
  let r = cli(dbPath, ['financial-statements', 'report', '--year', '2026'], { expectFail: true });
  assert.equal(r.out.error.code, 'FORMAT_NOT_SUPPORTED');
  r = cli(dbPath, ['export', 'xaf', '--year', '2026', '--out', '/tmp/xaf-fr.xml'], { expectFail: true });
  assert.equal(r.out.error.code, 'FORMAT_NOT_SUPPORTED'); // FEC is the FR audit file, not XAF
  r = cli(dbPath, ['vat', 'readout', '--period', '2026-Q1'], { expectFail: true });
  assert.equal(r.out.error.code, 'FORMAT_NOT_SUPPORTED'); // CA3 return engine is a B-milestone
  const db = openDb(dbPath);
  try {
    // the art. 226 EU baseline is registered — the rule RUNS and enforces
    // (the test company is minimal, so the supplier party checks fire)
    assert.throws(
      () => validateCompliance(db, { invoice_type: 'invoice', lines: [] }),
      (e) => e.code === 'SUPPLIER_INCOMPLETE',
    );
  } finally {
    db.close();
  }
});

// --- Phase B: US profile (no VAT, state-level sales tax) ---------------------

test('US: getProfile returns the US profile (USD, en-US, no federal VAT)', () => {
  const p = getProfile('US');
  assert.equal(p.meta.country, 'US');
  assert.equal(p.meta.baseCurrency, 'USD');
  assert.equal(p.meta.locale, 'en');
  assert.ok(p.meta.legalForms.includes('llc'));
  assert.ok(!p.meta.legalForms.includes('bv')); // NL form rejected
  // no federal VAT: system 'none', no codes, no ledger
  assert.equal(p.tax.system, 'none');
  assert.deepEqual(p.tax.codes, []);
  assert.deepEqual(p.tax.accounts.ledger, []);
  assert.ok(p.identifiers.vatIdFormat.test('12-3456789')); // EIN
  assert.ok(!p.identifiers.vatIdFormat.test('123456789'));
  assert.equal(p.reporting.debtorsAccount, '1100');
  assert.equal(p.closing.resultAccount, '3300');
  assert.equal(p.closing.equityAccount, '3200');
  assert.equal(p.reporting.taxonomy, null);
  // QuickBooks-convention chart incl. the 1410 contra-asset (migration 023)
  assert.ok(p.reporting.defaultChart.some((a) => a.code === '1000' && a.name === 'Checking account'));
  assert.ok(p.reporting.defaultChart.some((a) => a.code === '4000' && a.name === 'Sales — goods'));
  assert.ok(p.reporting.defaultChart.some((a) => a.code === '1410' && a.normalBalance === 'credit'));
  assert.ok(p.reporting.defaultChart.length >= 38);
  // B-milestones stay unregistered (strict dispatch fails loudly)
  assert.equal(p.tax.returnLayout, undefined);
  assert.equal(p.reporting.format, undefined);
  assert.equal(p.documents.invoiceCompliance, undefined); // B-milestone (no federal invoice rule set)
  assert.equal(p.documents.eInvoicing, undefined);
  assert.equal(p.documents.auditFile, undefined);
  assert.deepEqual(p.exchange.bankStatementFormats, ['csv']); // CAMT.053 unverified
  assert.deepEqual(p.exchange.paymentFormats, []); // no SEPA; ACH is a B-milestone
  // US deadlines: 1120 (15th of 4th month after FYE) + 941 (quarterly)
  assert.deepEqual(p.compliance.filingTypes.map((ft) => ft.deadlineRule), ['us-1120', 'us-941']);
});

test('US: PLANNED is empty (all thirty-one markets landed)', () => {
  assert.deepEqual([...PLANNED].sort(), []);
  assert.equal(getProfile('US').meta.country, 'US');
});

test('US: init --country US creates a USD company with the US chart', () => {
  const dbPath = tmpDb();
  const r = cli(dbPath, ['init', '--name', 'Acme LLC', '--country', 'US', '--legal-form', 'llc']);
  assert.equal(r.out.data.company.country, 'US');
  assert.equal(r.out.data.company.base_currency, 'USD');
  assert.equal(r.out.data.company.locale, 'en');
  // --vat on must not crash the init render: the US profile has NO VAT ledger
  // (system 'none', ledger []) — the profile-aware clearing-account message
  // falls back to a plain "module enabled" line (Phase C review finding).
  const rVat = cli(tmpDb(), ['init', '--name', 'Acme LLC', '--country', 'US', '--legal-form', 'llc', '--vat', 'on']);
  assert.equal(rVat.out.data.company.vat_module, 1);
  const db = openDb(dbPath);
  try {
    const accounts = db.prepare('SELECT code, name, taxonomy FROM accounts WHERE active = 1').all();
    assert.ok(accounts.some((a) => a.code === '1000' && a.name === 'Checking account'));
    assert.ok(accounts.some((a) => a.code === '4000' && a.name === 'Sales — goods'));
    assert.ok(accounts.some((a) => a.code === '1410')); // contra-asset accepted (023)
    for (const a of accounts) assert.equal(a.taxonomy, null);
  } finally {
    db.close();
  }
  // NL legal form rejected for US
  const bad = cli(tmpDb(), ['init', '--name', 'X', '--country', 'US', '--legal-form', 'bv'], { expectFail: true });
  assert.equal(bad.out.error.code, 'INVALID_LEGAL_FORM');
  // KOR is an NL-only scheme
  const kor = cli(tmpDb(), ['init', '--name', 'X', '--country', 'US', '--legal-form', 'llc', '--kor'], { expectFail: true });
  assert.equal(kor.out.error.code, 'INVALID_VAT_CHOICE');
});

test('US: strict dispatch — unregistered formats fail loudly (no fallback)', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Acme LLC', '--country', 'US', '--legal-form', 'llc']);
  let r = cli(dbPath, ['financial-statements', 'report', '--year', '2026'], { expectFail: true });
  assert.equal(r.out.error.code, 'FORMAT_NOT_SUPPORTED');
  r = cli(dbPath, ['export', 'xaf', '--year', '2026', '--out', '/tmp/xaf-us.xml'], { expectFail: true });
  assert.equal(r.out.error.code, 'FORMAT_NOT_SUPPORTED');
  const db = openDb(dbPath);
  try {
    assert.throws(
      () => validateCompliance(db, { invoice_type: 'invoice', lines: [] }),
      (e) => e.code === 'FORMAT_NOT_SUPPORTED',
    );
  } finally {
    db.close();
  }
});

test('US: compliance calendar — 1120 on 15 Apr + 941 quarterly (month-end)', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Acme LLC', '--country', 'US', '--legal-form', 'llc']);
  const db = openDb(dbPath);
  try {
    const r = complianceStatus(db, { year: 2026 });
    const obs = r.obligations;
    // FY 12-31: 1120 due 15 Apr next year
    assert.equal(obs.find((o) => o.type === 'FEDERAL_INCOME_TAX' && o.period === '2026').deadline, '2027-04-15');
    // 941 quarterly, last day of the month after the quarter
    assert.equal(obs.find((o) => o.type === 'PAYROLL_941' && o.period === '2026-Q3').deadline, '2026-10-31');
    assert.equal(obs.find((o) => o.type === 'PAYROLL_941' && o.period === '2026-Q4').deadline, '2027-01-31');
    // no NL/LU/GB/FR types leak into the US calendar
    assert.ok(!obs.some((o) => ['OB', 'ICP', 'JAARREKENING', 'TVA', 'COMPTES_ANNUELS', 'ANNUAL_ACCOUNTS', 'CT600'].includes(o.type)));
  } finally {
    db.close();
  }
});

// --- Phase B expansion: BE profile (PCN-BE, EUR) -----------------------------

test('BE: getProfile returns the BE profile (EUR, nl-BE, PCN-BE data)', () => {
  const p = getProfile('BE');
  assert.equal(p.meta.country, 'BE');
  assert.equal(p.meta.baseCurrency, 'EUR');
  assert.equal(p.meta.locale, 'nl-BE');
  assert.ok(p.meta.legalForms.includes('bv'));
  assert.ok(!p.meta.legalForms.includes('gmbh')); // DE form rejected
  // Peppol scheme: KBO (0208) — NOT 0106 (that is the NL KvK)
  assert.equal(p.identifiers.peppolSchemeId, '0208');
  assert.ok(p.identifiers.vatIdFormat.test('BE0123456789'));
  assert.equal(p.tax.standardRateBp, 2100);
  assert.equal(p.tax.smallBusinessScheme, 'franchise'); // €25K, from 1 Jan 2025
  assert.deepEqual(p.tax.codes.map((c) => c.code), ['21', '12', '6', '0', 'V', 'R', 'RE', 'M', 'P']);
  // PCMN VAT accounts: 411 input / 451 output
  assert.deepEqual(p.tax.accounts.ledger.map((a) => a.code), ['411', '451']);
  assert.equal(p.tax.accounts.fileDefault, '451');
  assert.equal(p.reporting.debtorsAccount, '400'); // Clients
  // BE closing: result 140 (overgedragen winst) — 12x is revaluation
  assert.equal(p.closing.resultAccount, '140');
  assert.equal(p.closing.equityAccount, '140');
  assert.equal(p.reporting.taxonomy, null);
  // chart: PCN-BE minimum plan codes (3-4 digits)
  assert.ok(p.reporting.defaultChart.some((a) => a.code === '550' && a.name.includes('Banques')));
  assert.ok(p.reporting.defaultChart.some((a) => a.code === '700'));
  assert.ok(p.reporting.defaultChart.length >= 43);
  // B-milestones stay unregistered (strict dispatch fails loudly)
  assert.equal(p.tax.returnLayout, undefined);
  assert.equal(p.reporting.format, undefined);
  assert.equal(p.documents.invoiceCompliance, 'eu-invoice-vereisten'); // art. 226 baseline
  assert.equal(p.documents.auditFile, undefined);
  assert.deepEqual(p.exchange.bankStatementFormats, ['camt.053', 'csv']); // CODA not registered
  assert.deepEqual(p.exchange.paymentFormats, ['sepa-pain.001', 'sepa-pain.008']);
  // e-invoicing: CONFIRMED mandatory B2B via Peppol since 1 Jan 2026
  assert.equal(p.documents.eInvoicing, 'peppol-bis-3.0');
  // BE deadlines: VAT monthly (20th), annual accounts 7 months
  assert.deepEqual(p.compliance.filingTypes.map((ft) => ft.deadlineRule), ['be-vat-monthly', 'be-7-months']);
});

test('BE: PLANNED is empty (all thirty-one markets landed)', () => {
  assert.ok(!PLANNED.includes('BE'));
  assert.deepEqual([...PLANNED].sort(), []);
});

test('BE: init --country BE creates a Belgian company with the PCMN chart', () => {
  const dbPath = tmpDb();
  const r = cli(dbPath, ['init', '--name', 'Test BV', '--country', 'BE', '--legal-form', 'bv', '--vat', 'on']);
  assert.equal(r.out.data.company.country, 'BE');
  assert.equal(r.out.data.company.base_currency, 'EUR');
  assert.equal(r.out.data.company.locale, 'nl-BE');
  const db = openDb(dbPath);
  try {
    const accounts = db.prepare('SELECT code, name, taxonomy FROM accounts WHERE active = 1').all();
    assert.ok(accounts.some((a) => a.code === '550' && a.name.includes('Banques')));
    assert.ok(accounts.some((a) => a.code === '700'));
    assert.ok(accounts.some((a) => a.code === '400' && a.name.includes('Clients')));
    for (const a of accounts) assert.equal(a.taxonomy, null);
  } finally {
    db.close();
  }
  // DE legal form rejected for BE
  const bad = cli(tmpDb(), ['init', '--name', 'X', '--country', 'BE', '--legal-form', 'gmbh'], { expectFail: true });
  assert.equal(bad.out.error.code, 'INVALID_LEGAL_FORM');
  // KOR is an NL-only scheme
  const kor = cli(tmpDb(), ['init', '--name', 'X', '--country', 'BE', '--legal-form', 'bv', '--kor'], { expectFail: true });
  assert.equal(kor.out.error.code, 'INVALID_VAT_CHOICE');
});

test('BE: strict dispatch — unregistered formats fail loudly (no fallback)', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Test BV', '--country', 'BE', '--legal-form', 'bv', '--vat', 'on']);
  let r = cli(dbPath, ['financial-statements', 'report', '--year', '2026'], { expectFail: true });
  assert.equal(r.out.error.code, 'FORMAT_NOT_SUPPORTED');
  r = cli(dbPath, ['export', 'xaf', '--year', '2026', '--out', '/tmp/xaf-be.xml'], { expectFail: true });
  assert.equal(r.out.error.code, 'FORMAT_NOT_SUPPORTED');
  r = cli(dbPath, ['vat', 'readout', '--period', '2026-Q1'], { expectFail: true });
  assert.equal(r.out.error.code, 'FORMAT_NOT_SUPPORTED'); // Intervat return engine is a B-milestone
  const db = openDb(dbPath);
  try {
    // the art. 226 EU baseline is registered — the rule RUNS and enforces
    // (the test company is minimal, so the supplier party checks fire)
    assert.throws(
      () => validateCompliance(db, { invoice_type: 'invoice', lines: [] }),
      (e) => e.code === 'SUPPLIER_INCOMPLETE',
    );
  } finally {
    db.close();
  }
});

test('BE: compliance calendar — VAT on the 20th + annual accounts in 7 months', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Test BV', '--country', 'BE', '--legal-form', 'bv', '--vat', 'on']);
  const db = openDb(dbPath);
  try {
    const r = complianceStatus(db, { year: 2026 });
    const obs = r.obligations;
    // monthly VAT due the 20th of the following month (Dec's return is due
    // in the next calendar year, so it appears there, not here)
    assert.equal(obs.find((o) => o.type === 'VAT' && o.period === '2026-01').deadline, '2026-02-20');
    assert.equal(obs.find((o) => o.type === 'VAT' && o.period === '2025-12').deadline, '2026-01-20');
    // annual accounts with the NBB within 7 months of FYE (12-31 -> 07-31)
    assert.equal(obs.find((o) => o.type === 'ANNUAL_ACCOUNTS' && o.period === '2026').deadline, '2027-07-31');
    // no NL/LU/GB/FR/US types leak into the BE calendar
    assert.ok(!obs.some((o) => ['OB', 'ICP', 'JAARREKENING', 'TVA', 'COMPTES_ANNUELS', 'CT600', 'FEDERAL_INCOME_TAX', 'PAYROLL_941'].includes(o.type)));
  } finally {
    db.close();
  }
});

// --- Phase B expansion: DE profile (DATEV SKR 03, EUR) -----------------------

test('DE: bank add defaults to the profile bank account (1200), not NL 1100 (review fix)', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Test GmbH', '--country', 'DE', '--legal-form', 'gmbh', '--registration-id', 'HRB123456', '--tax-id', 'DE123456789', '--vat', 'on']);
  const r = cli(dbPath, ['bank', 'add', '--iban', 'DE89370400440532013000', '--name', 'Geschäftskonto']);
  assert.equal(r.out.data.bank_account.account_code, '1200');
  assert.equal(getProfile('DE').reporting.bankAccountDefault, '1200');
});

test('NL: bank add still defaults to 1100 (byte-identity)', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Test BV', '--country', 'NL', '--legal-form', 'bv', '--vat', 'on']);
  const r = cli(dbPath, ['bank', 'add', '--iban', 'NL91ABNA0417164300', '--name', 'Betaalrekening']);
  assert.equal(r.out.data.bank_account.account_code, '1100');
  assert.equal(getProfile('NL').reporting.bankAccountDefault, '1100');
});

test('DE: getProfile returns the DE profile (EUR, de-DE, SKR 03 data)', () => {
  const p = getProfile('DE');
  assert.equal(p.meta.country, 'DE');
  assert.equal(p.meta.baseCurrency, 'EUR');
  assert.equal(p.meta.locale, 'de-DE');
  assert.ok(p.meta.legalForms.includes('gmbh'));
  assert.ok(!p.meta.legalForms.includes('bv')); // BE form rejected
  // Peppol scheme: 9930 = USt-IdNr (NOT 0204 Leitweg-ID / NOT 0210 Italy)
  assert.equal(p.identifiers.peppolSchemeId, '9930');
  assert.ok(p.identifiers.vatIdFormat.test('DE123456789'));
  assert.equal(p.tax.standardRateBp, 1900);
  assert.equal(p.tax.smallBusinessScheme, 'kleinunternehmer'); // €25k/€100k since 2025
  // the first income account is the default sales account (postingDefaults)
  // — must be the standard 19% sales account, NOT the §19 Kleinunternehmer one
  assert.equal(p.reporting.defaultChart.find((a) => a.type === 'income').code, '8400');
  assert.deepEqual(p.tax.codes.map((c) => c.code), ['19', '7', '0', 'V', 'R', 'RE', 'M', 'P']);
  // SKR 03 VAT accounts: 1570 input / 1776 output; settlement 1780
  assert.deepEqual(p.tax.accounts.ledger.map((a) => a.code), ['1570', '1776']);
  assert.equal(p.tax.accounts.fileDefault, '1780');
  assert.equal(p.reporting.debtorsAccount, '1400'); // Forderungen L+L
  // SKR 03 closing: 0860 Gewinnvortrag
  assert.equal(p.closing.resultAccount, '0860');
  assert.equal(p.closing.equityAccount, '0860');
  assert.equal(p.reporting.taxonomy, null);
  // chart: SKR 03 codes (4 digits; no 9000 clearing account — not a bukio type)
  assert.ok(p.reporting.defaultChart.some((a) => a.code === '1200' && a.name === 'Bank'));
  assert.ok(p.reporting.defaultChart.some((a) => a.code === '8400' && a.name === 'Erlöse 19 % USt'));
  assert.ok(!p.reporting.defaultChart.some((a) => a.code === '9000'));
  assert.ok(p.reporting.defaultChart.length >= 40);
  // B-milestones stay unregistered (strict dispatch fails loudly)
  assert.equal(p.tax.returnLayout, undefined);
  assert.equal(p.reporting.format, undefined);
  assert.equal(p.documents.invoiceCompliance, 'eu-invoice-vereisten'); // art. 226 baseline
  assert.equal(p.documents.auditFile, undefined);
  assert.deepEqual(p.exchange.paymentFormats, ['sepa-pain.001', 'sepa-pain.008']);
  // e-invoicing: EN 16931 accepted (Peppol BIS 3.0 UBL is one)
  assert.equal(p.documents.eInvoicing, 'peppol-bis-3.0');
  // DE deadlines: UStVA quarterly (10th), annual VAT (31 Jul), accounts (12 mo)
  assert.deepEqual(p.compliance.filingTypes.map((ft) => ft.deadlineRule), ['de-ustva-quarterly', 'de-annual-vat', 'de-12-months']);
});

test('DE: PLANNED is empty (all thirty-one markets landed)', () => {
  assert.ok(!PLANNED.includes('DE'));
  assert.deepEqual([...PLANNED].sort(), []);
});

test('DE: init --country DE creates a German company with the SKR 03 chart', () => {
  const dbPath = tmpDb();
  const r = cli(dbPath, ['init', '--name', 'Test GmbH', '--country', 'DE', '--legal-form', 'gmbh', '--vat', 'on']);
  assert.equal(r.out.data.company.country, 'DE');
  assert.equal(r.out.data.company.base_currency, 'EUR');
  assert.equal(r.out.data.company.locale, 'de-DE');
  const db = openDb(dbPath);
  try {
    const accounts = db.prepare('SELECT code, name, taxonomy FROM accounts WHERE active = 1').all();
    assert.ok(accounts.some((a) => a.code === '1200' && a.name === 'Bank'));
    assert.ok(accounts.some((a) => a.code === '8400' && a.name === 'Erlöse 19 % USt'));
    for (const a of accounts) assert.equal(a.taxonomy, null);
    // Document language follows the profile — DE defaults to its own language (de)
    // (no market is the de facto base; NL is the only Dutch-speaking default).
    const c = createContact(db, { name: 'Kunde GmbH' });
    const inv = createInvoice(db, { contactId: c.id, lines: ['Dienst @ 10.00'], date: '2026-08-10', actor: 'agent:test' });
    assert.equal(inv.language, 'de'); // the profile's language (fully localised PDF)
    const invNl = createInvoice(db, { contactId: c.id, lines: ['Dienst @ 10.00'], date: '2026-08-11', language: 'nl', actor: 'agent:test' });
    assert.equal(invNl.language, 'nl'); // explicit --language still overrides
  } finally {
    db.close();
  }
  // BE legal form rejected for DE
  const bad = cli(tmpDb(), ['init', '--name', 'X', '--country', 'DE', '--legal-form', 'bv'], { expectFail: true });
  assert.equal(bad.out.error.code, 'INVALID_LEGAL_FORM');
  // KOR is an NL-only scheme
  const kor = cli(tmpDb(), ['init', '--name', 'X', '--country', 'DE', '--legal-form', 'gmbh', '--kor'], { expectFail: true });
  assert.equal(kor.out.error.code, 'INVALID_VAT_CHOICE');
});

test('DE: strict dispatch — unregistered formats fail loudly (no fallback)', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Test GmbH', '--country', 'DE', '--legal-form', 'gmbh', '--vat', 'on']);
  let r = cli(dbPath, ['financial-statements', 'report', '--year', '2026'], { expectFail: true });
  assert.equal(r.out.error.code, 'FORMAT_NOT_SUPPORTED');
  r = cli(dbPath, ['export', 'xaf', '--year', '2026', '--out', '/tmp/xaf-de.xml'], { expectFail: true });
  assert.equal(r.out.error.code, 'FORMAT_NOT_SUPPORTED');
  r = cli(dbPath, ['vat', 'readout', '--period', '2026-Q1'], { expectFail: true });
  assert.equal(r.out.error.code, 'FORMAT_NOT_SUPPORTED'); // ELSTER UStVA engine is a B-milestone
  const db = openDb(dbPath);
  try {
    // the art. 226 EU baseline is registered — the rule RUNS and enforces
    // (the test company is minimal, so the supplier party checks fire)
    assert.throws(
      () => validateCompliance(db, { invoice_type: 'invoice', lines: [] }),
      (e) => e.code === 'SUPPLIER_INCOMPLETE',
    );
  } finally {
    db.close();
  }
});

test('DE: compliance calendar — UStVA 10th + annual VAT 31 Jul + accounts 12 mo', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Test GmbH', '--country', 'DE', '--legal-form', 'gmbh', '--vat', 'on']);
  const db = openDb(dbPath);
  try {
    const r = complianceStatus(db, { year: 2026 });
    const obs = r.obligations;
    // quarterly UStVA due the 10th of the month after the quarter
    assert.equal(obs.find((o) => o.type === 'UMSATZSTEUER_VORANMELDUNG' && o.period === '2026-Q1').deadline, '2026-04-10');
    assert.equal(obs.find((o) => o.type === 'UMSATZSTEUER_VORANMELDUNG' && o.period === '2025-Q4').deadline, '2026-01-10');
    // annual VAT return 31 July of the following year
    assert.equal(obs.find((o) => o.type === 'UMSATZSTEUER_JAHRESERKLAERUNG' && o.period === '2026').deadline, '2027-07-31');
    // Offenlegung 12 months after the balance-sheet date (12-31 -> 2027-12-31)
    assert.equal(obs.find((o) => o.type === 'ANNUAL_ACCOUNTS' && o.period === '2026').deadline, '2027-12-31');
    // no NL/LU/GB/FR/US/BE types leak into the DE calendar
    assert.ok(!obs.some((o) => ['OB', 'ICP', 'JAARREKENING', 'TVA', 'COMPTES_ANNUELS', 'VAT', 'CT600', 'FEDERAL_INCOME_TAX', 'PAYROLL_941'].includes(o.type)));
  } finally {
    db.close();
  }
});

// --- Phase B expansion: DK profile (Standardkontoplan-aligned, DKK) ----------

test('DK: getProfile returns the DK profile (DKK, da-DK, 25% VAT only)', () => {
  const p = getProfile('DK');
  assert.equal(p.meta.country, 'DK');
  assert.equal(p.meta.baseCurrency, 'DKK');
  assert.equal(p.meta.locale, 'da-DK');
  assert.ok(p.meta.legalForms.includes('aps'));
  assert.ok(!p.meta.legalForms.includes('bv')); // BE form rejected
  // Peppol scheme: 0184 DK:CVR
  assert.equal(p.identifiers.peppolSchemeId, '0184');
  assert.ok(p.identifiers.vatIdFormat.test('DK12345678'));
  assert.equal(p.tax.standardRateBp, 2500);
  assert.equal(p.tax.smallBusinessScheme, null); // NO small-business scheme
  assert.deepEqual(p.tax.codes.map((c) => c.code), ['25', '0', 'V', 'R', 'RE', 'M', 'P']);
  // VAT accounts: 2720 Købsmoms input / 2710 Salgsmoms output; 2730 settlement
  assert.deepEqual(p.tax.accounts.ledger.map((a) => a.code), ['2720', '2710']);
  assert.equal(p.tax.accounts.fileDefault, '2730');
  assert.equal(p.reporting.debtorsAccount, '1210'); // Debitorer
  // DK closing: 3990 Årets resultat -> 3120 Overført resultat
  assert.equal(p.closing.resultAccount, '3990');
  assert.equal(p.closing.equityAccount, '3120');
  assert.equal(p.reporting.taxonomy, null);
  // chart: 1xxx assets / 2xxx liabilities / 3xxx equity / 4xxx income / 5xxx costs
  assert.ok(p.reporting.defaultChart.some((a) => a.code === '1110' && a.name === 'Bank'));
  assert.ok(p.reporting.defaultChart.some((a) => a.code === '4100' && a.name === 'Salg af varer'));
  assert.ok(p.reporting.defaultChart.length >= 40);
  // B-milestones stay unregistered (strict dispatch fails loudly)
  assert.equal(p.tax.returnLayout, undefined);
  assert.equal(p.reporting.format, undefined);
  assert.equal(p.documents.invoiceCompliance, 'eu-invoice-vereisten'); // art. 226 baseline
  assert.equal(p.documents.auditFile, undefined); // DK SAF-T v2.0 (2027) is a B-milestone
  assert.deepEqual(p.exchange.paymentFormats, ['sepa-pain.001', 'sepa-pain.008']);
  assert.equal(p.documents.eInvoicing, 'peppol-bis-3.0'); // voluntary B2B
  // DK deadlines: quarterly VAT (1st of 3rd month), accounts 5 months
  assert.deepEqual(p.compliance.filingTypes.map((ft) => ft.deadlineRule), ['dk-quarterly', 'dk-5-months']);
});

test('DK: PLANNED is empty (all thirty-one markets landed)', () => {
  assert.ok(!PLANNED.includes('DK'));
  assert.deepEqual([...PLANNED].sort(), []);
});

test('DK: init --country DK creates a Danish company with the kontoplan', () => {
  const dbPath = tmpDb();
  const r = cli(dbPath, ['init', '--name', 'Test ApS', '--country', 'DK', '--legal-form', 'aps', '--vat', 'on']);
  assert.equal(r.out.data.company.country, 'DK');
  assert.equal(r.out.data.company.base_currency, 'DKK');
  assert.equal(r.out.data.company.locale, 'da-DK');
  const db = openDb(dbPath);
  try {
    const accounts = db.prepare('SELECT code, name, taxonomy FROM accounts WHERE active = 1').all();
    assert.ok(accounts.some((a) => a.code === '1110' && a.name === 'Bank'));
    assert.ok(accounts.some((a) => a.code === '4100' && a.name === 'Salg af varer'));
    assert.ok(accounts.some((a) => a.code === '1620')); // contra-asset accepted (023)
    for (const a of accounts) assert.equal(a.taxonomy, null);
  } finally {
    db.close();
  }
  // BE legal form rejected for DK
  const bad = cli(tmpDb(), ['init', '--name', 'X', '--country', 'DK', '--legal-form', 'bv'], { expectFail: true });
  assert.equal(bad.out.error.code, 'INVALID_LEGAL_FORM');
  // KOR is an NL-only scheme
  const kor = cli(tmpDb(), ['init', '--name', 'X', '--country', 'DK', '--legal-form', 'aps', '--kor'], { expectFail: true });
  assert.equal(kor.out.error.code, 'INVALID_VAT_CHOICE');
});

test('DK: strict dispatch — unregistered formats fail loudly (no fallback)', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Test ApS', '--country', 'DK', '--legal-form', 'aps', '--vat', 'on']);
  let r = cli(dbPath, ['financial-statements', 'report', '--year', '2026'], { expectFail: true });
  assert.equal(r.out.error.code, 'FORMAT_NOT_SUPPORTED');
  r = cli(dbPath, ['export', 'xaf', '--year', '2026', '--out', '/tmp/xaf-dk.xml'], { expectFail: true });
  assert.equal(r.out.error.code, 'FORMAT_NOT_SUPPORTED');
  r = cli(dbPath, ['vat', 'readout', '--period', '2026-Q1'], { expectFail: true });
  assert.equal(r.out.error.code, 'FORMAT_NOT_SUPPORTED'); // TastSelv momsangivelse is a B-milestone
  const db = openDb(dbPath);
  try {
    // the art. 226 EU baseline is registered — the rule RUNS and enforces
    // (the test company is minimal, so the supplier party checks fire)
    assert.throws(
      () => validateCompliance(db, { invoice_type: 'invoice', lines: [] }),
      (e) => e.code === 'SUPPLIER_INCOMPLETE',
    );
  } finally {
    db.close();
  }
});

test('DK: compliance calendar — quarterly VAT 1st of 3rd month + accounts 5 months', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Test ApS', '--country', 'DK', '--legal-form', 'aps', '--vat', 'on']);
  const db = openDb(dbPath);
  try {
    const r = complianceStatus(db, { year: 2026 });
    const obs = r.obligations;
    // quarterly VAT due the 1st of the 3rd following month (Q1 -> 1 Jun, Q2 -> 1 Sep)
    assert.equal(obs.find((o) => o.type === 'VAT' && o.period === '2026-Q1').deadline, '2026-06-01');
    assert.equal(obs.find((o) => o.type === 'VAT' && o.period === '2026-Q2').deadline, '2026-09-01');
    assert.equal(obs.find((o) => o.type === 'VAT' && o.period === '2026-Q3').deadline, '2026-12-01');
    // annual report (class B) within 5 months of FYE (12-31 -> 05-31)
    assert.equal(obs.find((o) => o.type === 'ANNUAL_ACCOUNTS' && o.period === '2026').deadline, '2027-05-31');
    // no NL/LU/GB/FR/US/BE/DE types leak into the DK calendar
    assert.ok(!obs.some((o) => ['OB', 'ICP', 'JAARREKENING', 'TVA', 'COMPTES_ANNUELS', 'CT600', 'FEDERAL_INCOME_TAX', 'PAYROLL_941', 'UMSATZSTEUER_VORANMELDUNG', 'UMSATZSTEUER_JAHRESERKLAERUNG'].includes(o.type)));
  } finally {
    db.close();
  }
});

// --- Phase B expansion: FI profile (Liikekirjuri chart, EUR) ------------------

test('FI: getProfile returns the FI profile (EUR, fi-FI, 25.5% VAT)', () => {
  const p = getProfile('FI');
  assert.equal(p.meta.country, 'FI');
  assert.equal(p.meta.baseCurrency, 'EUR');
  assert.equal(p.meta.locale, 'fi-FI');
  assert.ok(p.meta.legalForms.includes('oy'));
  assert.ok(!p.meta.legalForms.includes('bv')); // BE form rejected
  // Peppol scheme: 0037 (LY-tunnus)
  assert.equal(p.identifiers.peppolSchemeId, '0037');
  assert.ok(p.identifiers.vatIdFormat.test('FI01120389'));
  assert.equal(p.tax.standardRateBp, 2550);
  assert.equal(p.tax.smallBusinessScheme, 'franchise'); // €20K exemption
  // 25.5 / 13.5 (since 1 Jan 2026) / 10 / 0
  assert.deepEqual(p.tax.codes.map((c) => c.code), ['25.5', '13.5', '10', '0', 'V', 'R', 'RE', 'M', 'P']);
  // Finnish VAT accounts: 1763 input / 2939 output
  assert.deepEqual(p.tax.accounts.ledger.map((a) => a.code), ['1763', '2939']);
  assert.equal(p.tax.accounts.fileDefault, '2939');
  assert.equal(p.reporting.debtorsAccount, '1701'); // Myyntisaamiset
  // FI closing: 2375 Tilikauden voitto -> 2251 Ed. tilikausien voitto
  assert.equal(p.closing.resultAccount, '2375');
  assert.equal(p.closing.equityAccount, '2251');
  assert.equal(p.reporting.taxonomy, null);
  // chart: Liikekirjuri codes incl. the 2350 Yksityistili contra-equity
  assert.ok(p.reporting.defaultChart.some((a) => a.code === '1910' && a.name === 'Pankkitili (Nordea)'));
  assert.ok(p.reporting.defaultChart.some((a) => a.code === '3000' && a.name === 'Myynti ALV 25,5%'));
  assert.ok(p.reporting.defaultChart.some((a) => a.code === '2350' && a.normalBalance === 'debit'));
  assert.ok(p.reporting.defaultChart.length >= 50);
  // B-milestones stay unregistered (strict dispatch fails loudly)
  assert.equal(p.tax.returnLayout, undefined);
  assert.equal(p.reporting.format, undefined);
  assert.equal(p.documents.invoiceCompliance, 'eu-invoice-vereisten'); // art. 226 baseline
  assert.equal(p.documents.auditFile, undefined);
  assert.deepEqual(p.exchange.paymentFormats, ['sepa-pain.001', 'sepa-pain.008']);
  // e-invoicing: no B2B mandate; Peppol BIS accepted voluntarily
  assert.equal(p.documents.eInvoicing, 'peppol-bis-3.0');
  // FI deadlines: quarterly VAT (12th of 2nd month), accounts filed in 8 months
  assert.deepEqual(p.compliance.filingTypes.map((ft) => ft.deadlineRule), ['fi-quarterly', 'fi-8-months']);
});

test('FI: PLANNED is empty (all thirty-one markets landed)', () => {
  assert.ok(!PLANNED.includes('FI'));
  assert.deepEqual([...PLANNED].sort(), []);
});

test('FI: init --country FI creates a Finnish company with the model chart', () => {
  const dbPath = tmpDb();
  const r = cli(dbPath, ['init', '--name', 'Test Oy', '--country', 'FI', '--legal-form', 'oy', '--vat', 'on']);
  assert.equal(r.out.data.company.country, 'FI');
  assert.equal(r.out.data.company.base_currency, 'EUR');
  assert.equal(r.out.data.company.locale, 'fi-FI');
  const db = openDb(dbPath);
  try {
    const accounts = db.prepare('SELECT code, name, taxonomy FROM accounts WHERE active = 1').all();
    assert.ok(accounts.some((a) => a.code === '1910' && a.name === 'Pankkitili (Nordea)'));
    assert.ok(accounts.some((a) => a.code === '3000' && a.name === 'Myynti ALV 25,5%'));
    assert.ok(accounts.some((a) => a.code === '2350')); // contra-equity accepted (024)
    for (const a of accounts) assert.equal(a.taxonomy, null);
  } finally {
    db.close();
  }
  // BE legal form rejected for FI
  const bad = cli(tmpDb(), ['init', '--name', 'X', '--country', 'FI', '--legal-form', 'bv'], { expectFail: true });
  assert.equal(bad.out.error.code, 'INVALID_LEGAL_FORM');
  // KOR is an NL-only scheme
  const kor = cli(tmpDb(), ['init', '--name', 'X', '--country', 'FI', '--legal-form', 'oy', '--kor'], { expectFail: true });
  assert.equal(kor.out.error.code, 'INVALID_VAT_CHOICE');
});

test('FI: strict dispatch — unregistered formats fail loudly (no fallback)', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Test Oy', '--country', 'FI', '--legal-form', 'oy', '--vat', 'on']);
  let r = cli(dbPath, ['financial-statements', 'report', '--year', '2026'], { expectFail: true });
  assert.equal(r.out.error.code, 'FORMAT_NOT_SUPPORTED');
  r = cli(dbPath, ['export', 'xaf', '--year', '2026', '--out', '/tmp/xaf-fi.xml'], { expectFail: true });
  assert.equal(r.out.error.code, 'FORMAT_NOT_SUPPORTED');
  r = cli(dbPath, ['vat', 'readout', '--period', '2026-Q1'], { expectFail: true });
  assert.equal(r.out.error.code, 'FORMAT_NOT_SUPPORTED'); // OmaVero kausiveroilmoitus is a B-milestone
  const db = openDb(dbPath);
  try {
    // the art. 226 EU baseline is registered — the rule RUNS and enforces
    // (the test company is minimal, so the supplier party checks fire)
    assert.throws(
      () => validateCompliance(db, { invoice_type: 'invoice', lines: [] }),
      (e) => e.code === 'SUPPLIER_INCOMPLETE',
    );
  } finally {
    db.close();
  }
});

test('FI: compliance calendar — quarterly VAT 12th of 2nd month + accounts in 8 months', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Test Oy', '--country', 'FI', '--legal-form', 'oy', '--vat', 'on']);
  const db = openDb(dbPath);
  try {
    const r = complianceStatus(db, { year: 2026 });
    const obs = r.obligations;
    // quarterly VAT due the 12th of the second month after the quarter
    assert.equal(obs.find((o) => o.type === 'VAT' && o.period === '2026-Q1').deadline, '2026-05-12');
    assert.equal(obs.find((o) => o.type === 'VAT' && o.period === '2026-Q2').deadline, '2026-08-12');
    // annual accounts filed within 8 months of FYE (12-31 -> 08-31)
    assert.equal(obs.find((o) => o.type === 'ANNUAL_ACCOUNTS' && o.period === '2026').deadline, '2027-08-31');
    // no NL/LU/GB/FR/US/BE/DE/DK types leak into the FI calendar
    assert.ok(!obs.some((o) => ['OB', 'ICP', 'JAARREKENING', 'TVA', 'COMPTES_ANNUELS', 'CT600', 'FEDERAL_INCOME_TAX', 'PAYROLL_941', 'UMSATZSTEUER_VORANMELDUNG', 'UMSATZSTEUER_JAHRESERKLAERUNG'].includes(o.type)));
  } finally {
    db.close();
  }
});

// --- Phase B expansion: NO profile (NS 4102, NOK, bi-monthly VAT) -------------

test('NO: getProfile returns the NO profile (NOK, nb-NO, NS 4102)', () => {
  const p = getProfile('NO');
  assert.equal(p.meta.country, 'NO');
  assert.equal(p.meta.baseCurrency, 'NOK');
  assert.equal(p.meta.locale, 'nb-NO');
  assert.ok(p.meta.legalForms.includes('as'));
  assert.ok(!p.meta.legalForms.includes('bv')); // BE form rejected
  // Peppol scheme: 0192 NO:ORG
  assert.equal(p.identifiers.peppolSchemeId, '0192');
  assert.ok(p.identifiers.vatIdFormat.test('NO123456789MVA'));
  assert.ok(!p.identifiers.vatIdFormat.test('NO123456789')); // MVA suffix required
  assert.equal(p.tax.standardRateBp, 2500);
  assert.equal(p.tax.smallBusinessScheme, null); // no exemption — only a registration threshold
  // NO is NOT in the EU: no RE code
  assert.deepEqual(p.tax.codes.map((c) => c.code), ['25', '15', '12', '0', 'V', 'R', 'M', 'P']);
  // NS 4102 MVA accounts: 2710 input / 2700 output; 2740 settlement
  assert.deepEqual(p.tax.accounts.ledger.map((a) => a.code), ['2710', '2700']);
  assert.equal(p.tax.accounts.fileDefault, '2740');
  assert.equal(p.reporting.debtorsAccount, '1500'); // Kundefordringer
  // NO closing: 8960 Overført til egenkapital
  assert.equal(p.closing.resultAccount, '8960');
  assert.equal(p.closing.equityAccount, '8960');
  assert.equal(p.reporting.taxonomy, null);
  // chart: NS 4102 codes incl. the 2060 Privatuttak contra-equity
  assert.ok(p.reporting.defaultChart.some((a) => a.code === '1920' && a.name === 'Bankinnskudd'));
  assert.ok(p.reporting.defaultChart.some((a) => a.code === '3000' && a.name === 'Salgsinntekt, avgiftspliktig'));
  assert.ok(p.reporting.defaultChart.some((a) => a.code === '2060' && a.normalBalance === 'debit'));
  assert.ok(p.reporting.defaultChart.length >= 45);
  // B-milestones stay unregistered (strict dispatch fails loudly)
  assert.equal(p.tax.returnLayout, undefined);
  assert.equal(p.reporting.format, undefined);
  assert.equal(p.documents.invoiceCompliance, 'eu-invoice-vereisten'); // art. 226 baseline
  assert.equal(p.documents.auditFile, undefined); // SAF-T Accounting is a B-milestone
  assert.deepEqual(p.exchange.paymentFormats, ['sepa-pain.001', 'sepa-pain.008']);
  // e-invoicing: EHF 3.0 = Peppol BIS 3.0 UBL profile
  assert.equal(p.documents.eInvoicing, 'peppol-bis-3.0');
  // NO deadlines: bi-monthly VAT + accounts 7 months
  assert.deepEqual(p.compliance.filingTypes.map((ft) => ft.deadlineRule), ['no-bimonthly', 'no-7-months']);
  assert.equal(p.compliance.filingTypes[0].periodShape, 'YYYY-Pn'); // bi-monthly shape
});

test('NO: PLANNED is empty (all thirty-one markets landed)', () => {
  assert.ok(!PLANNED.includes('NO'));
  assert.deepEqual([...PLANNED].sort(), []);
});

test('NO: init --country NO creates a Norwegian company with the NS 4102 chart', () => {
  const dbPath = tmpDb();
  const r = cli(dbPath, ['init', '--name', 'Test AS', '--country', 'NO', '--legal-form', 'as', '--vat', 'on']);
  assert.equal(r.out.data.company.country, 'NO');
  assert.equal(r.out.data.company.base_currency, 'NOK');
  assert.equal(r.out.data.company.locale, 'nb-NO');
  const db = openDb(dbPath);
  try {
    const accounts = db.prepare('SELECT code, name, taxonomy FROM accounts WHERE active = 1').all();
    assert.ok(accounts.some((a) => a.code === '1920' && a.name === 'Bankinnskudd'));
    assert.ok(accounts.some((a) => a.code === '3000' && a.name === 'Salgsinntekt, avgiftspliktig'));
    assert.ok(accounts.some((a) => a.code === '2060')); // contra-equity accepted (024)
    for (const a of accounts) assert.equal(a.taxonomy, null);
  } finally {
    db.close();
  }
  // BE legal form rejected for NO
  const bad = cli(tmpDb(), ['init', '--name', 'X', '--country', 'NO', '--legal-form', 'bv'], { expectFail: true });
  assert.equal(bad.out.error.code, 'INVALID_LEGAL_FORM');
  // KOR is an NL-only scheme
  const kor = cli(tmpDb(), ['init', '--name', 'X', '--country', 'NO', '--legal-form', 'as', '--kor'], { expectFail: true });
  assert.equal(kor.out.error.code, 'INVALID_VAT_CHOICE');
});

test('NO: strict dispatch — unregistered formats fail loudly (no fallback)', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Test AS', '--country', 'NO', '--legal-form', 'as', '--vat', 'on']);
  let r = cli(dbPath, ['financial-statements', 'report', '--year', '2026'], { expectFail: true });
  assert.equal(r.out.error.code, 'FORMAT_NOT_SUPPORTED');
  r = cli(dbPath, ['export', 'xaf', '--year', '2026', '--out', '/tmp/xaf-no.xml'], { expectFail: true });
  assert.equal(r.out.error.code, 'FORMAT_NOT_SUPPORTED');
  r = cli(dbPath, ['vat', 'readout', '--period', '2026-Q1'], { expectFail: true });
  assert.equal(r.out.error.code, 'FORMAT_NOT_SUPPORTED'); // Altinn mva-meldingen is a B-milestone
  const db = openDb(dbPath);
  try {
    // the art. 226 EU baseline is registered — the rule RUNS and enforces
    // (the test company is minimal, so the supplier party checks fire)
    assert.throws(
      () => validateCompliance(db, { invoice_type: 'invoice', lines: [] }),
      (e) => e.code === 'SUPPLIER_INCOMPLETE',
    );
  } finally {
    db.close();
  }
});

test('NO: compliance calendar — bi-monthly VAT (6/yr) + accounts by 31 July', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Test AS', '--country', 'NO', '--legal-form', 'as', '--vat', 'on']);
  const db = openDb(dbPath);
  try {
    const r = complianceStatus(db, { year: 2026 });
    const obs = r.obligations;
    // bi-monthly mva-meldingen: 6 periods, 1 month + 10 days (P3 summer exception)
    assert.equal(obs.find((o) => o.type === 'VAT' && o.period === '2026-P1').deadline, '2026-04-10');
    assert.equal(obs.find((o) => o.type === 'VAT' && o.period === '2026-P3').deadline, '2026-08-31');
    assert.equal(obs.find((o) => o.type === 'VAT' && o.period === '2026-P5').deadline, '2026-12-10');
    // P6 due 10 Feb next year -> appears in the 2027 calendar as prev-P6;
    // the previous year's P6 is due 10 Feb 2026 (in this calendar)
    assert.equal(obs.find((o) => o.type === 'VAT' && o.period === '2025-P6').deadline, '2026-02-10');
    // annual accounts filed by 31 July (approved <= 6 months + filed <= 1 month)
    assert.equal(obs.find((o) => o.type === 'ANNUAL_ACCOUNTS' && o.period === '2026').deadline, '2027-07-31');
    // no NL/LU/GB/FR/US/BE/DE/DK/FI types leak into the NO calendar
    assert.ok(!obs.some((o) => ['OB', 'ICP', 'JAARREKENING', 'TVA', 'COMPTES_ANNUELS', 'CT600', 'FEDERAL_INCOME_TAX', 'PAYROLL_941', 'UMSATZSTEUER_VORANMELDUNG', 'UMSATZSTEUER_JAHRESERKLAERUNG'].includes(o.type)));
  } finally {
    db.close();
  }
});

// --- Phase B expansion: SE profile (BAS 2023, SEK) ----------------------------

test('SE: getProfile returns the SE profile (SEK, sv-SE, BAS 2023)', () => {
  const p = getProfile('SE');
  assert.equal(p.meta.country, 'SE');
  assert.equal(p.meta.baseCurrency, 'SEK');
  assert.equal(p.meta.locale, 'sv-SE');
  assert.ok(p.meta.legalForms.includes('ab'));
  assert.ok(!p.meta.legalForms.includes('bv')); // BE form rejected
  // Peppol scheme: 0007 Organisationsnummer
  assert.equal(p.identifiers.peppolSchemeId, '0007');
  assert.ok(p.identifiers.vatIdFormat.test('SE556677889901'));
  assert.equal(p.tax.standardRateBp, 2500);
  // SME exemption SEK 120,000 (the 80k figure is superseded)
  assert.equal(p.tax.smallBusinessScheme, 'franchise');
  assert.deepEqual(p.tax.codes.map((c) => c.code), ['25', '12', '6', '0', 'V', 'R', 'RE', 'M', 'P']);
  // BAS VAT accounts: 2641 input / 2611 output; 2650 settlement
  assert.deepEqual(p.tax.accounts.ledger.map((a) => a.code), ['2641', '2611']);
  assert.equal(p.tax.accounts.fileDefault, '2650');
  assert.equal(p.reporting.debtorsAccount, '1510'); // Kundfordringar
  // SE closing: 2099 Årets resultat -> 2098 Föregående år
  assert.equal(p.closing.resultAccount, '2099');
  assert.equal(p.closing.equityAccount, '2098');
  assert.equal(p.reporting.taxonomy, null);
  // chart: BAS 2023 codes (no 8999 P&L summary - not a bukio type)
  assert.ok(p.reporting.defaultChart.some((a) => a.code === '1930' && a.name.includes('Företagskonto')));
  assert.ok(p.reporting.defaultChart.some((a) => a.code === '3001' && a.name === 'Försäljning inom Sverige, 25 % moms'));
  assert.ok(!p.reporting.defaultChart.some((a) => a.code === '8999'));
  assert.ok(p.reporting.defaultChart.length >= 40);
  // B-milestones stay unregistered (strict dispatch fails loudly)
  assert.equal(p.tax.returnLayout, undefined);
  assert.equal(p.reporting.format, undefined);
  assert.equal(p.documents.invoiceCompliance, 'eu-invoice-vereisten'); // art. 226 baseline
  assert.equal(p.documents.auditFile, undefined);
  assert.deepEqual(p.exchange.paymentFormats, ['sepa-pain.001', 'sepa-pain.008']);
  // e-invoicing: B2G mandatory via Peppol (Peppol BIS 3.0)
  assert.equal(p.documents.eInvoicing, 'peppol-bis-3.0');
  // SE deadlines: quarterly VAT (12th of 2nd month, Aug 17th), accounts 7 months
  assert.deepEqual(p.compliance.filingTypes.map((ft) => ft.deadlineRule), ['se-quarterly', 'se-7-months']);
});

test('SE: PLANNED is empty (all thirty-one markets landed)', () => {
  assert.ok(!PLANNED.includes('SE'));
  assert.deepEqual([...PLANNED].sort(), []);
});

test('SE: init --country SE creates a Swedish company with the BAS chart', () => {
  const dbPath = tmpDb();
  const r = cli(dbPath, ['init', '--name', 'Test AB', '--country', 'SE', '--legal-form', 'ab', '--vat', 'on']);
  assert.equal(r.out.data.company.country, 'SE');
  assert.equal(r.out.data.company.base_currency, 'SEK');
  assert.equal(r.out.data.company.locale, 'sv-SE');
  const db = openDb(dbPath);
  try {
    const accounts = db.prepare('SELECT code, name, taxonomy FROM accounts WHERE active = 1').all();
    assert.ok(accounts.some((a) => a.code === '1930' && a.name.includes('Företagskonto')));
    assert.ok(accounts.some((a) => a.code === '3001' && a.name === 'Försäljning inom Sverige, 25 % moms'));
    for (const a of accounts) assert.equal(a.taxonomy, null);
  } finally {
    db.close();
  }
  // BE legal form rejected for SE
  const bad = cli(tmpDb(), ['init', '--name', 'X', '--country', 'SE', '--legal-form', 'bv'], { expectFail: true });
  assert.equal(bad.out.error.code, 'INVALID_LEGAL_FORM');
  // KOR is an NL-only scheme
  const kor = cli(tmpDb(), ['init', '--name', 'X', '--country', 'SE', '--legal-form', 'ab', '--kor'], { expectFail: true });
  assert.equal(kor.out.error.code, 'INVALID_VAT_CHOICE');
});

test('SE: strict dispatch — unregistered formats fail loudly (no fallback)', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Test AB', '--country', 'SE', '--legal-form', 'ab', '--vat', 'on']);
  let r = cli(dbPath, ['financial-statements', 'report', '--year', '2026'], { expectFail: true });
  assert.equal(r.out.error.code, 'FORMAT_NOT_SUPPORTED');
  r = cli(dbPath, ['export', 'xaf', '--year', '2026', '--out', '/tmp/xaf-se.xml'], { expectFail: true });
  assert.equal(r.out.error.code, 'FORMAT_NOT_SUPPORTED');
  r = cli(dbPath, ['vat', 'readout', '--period', '2026-Q1'], { expectFail: true });
  assert.equal(r.out.error.code, 'FORMAT_NOT_SUPPORTED'); // Skatteverket momsredovisning is a B-milestone
  const db = openDb(dbPath);
  try {
    // the art. 226 EU baseline is registered — the rule RUNS and enforces
    // (the test company is minimal, so the supplier party checks fire)
    assert.throws(
      () => validateCompliance(db, { invoice_type: 'invoice', lines: [] }),
      (e) => e.code === 'SUPPLIER_INCOMPLETE',
    );
  } finally {
    db.close();
  }
});

test('SE: compliance calendar — quarterly VAT 12th of 2nd month (Aug 17th) + accounts 7 months', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Test AB', '--country', 'SE', '--legal-form', 'ab', '--vat', 'on']);
  const db = openDb(dbPath);
  try {
    const r = complianceStatus(db, { year: 2026 });
    const obs = r.obligations;
    // quarterly momsredovisning: 12th of the 2nd month after the quarter,
    // August exception (17th)
    assert.equal(obs.find((o) => o.type === 'VAT' && o.period === '2026-Q1').deadline, '2026-05-12');
    assert.equal(obs.find((o) => o.type === 'VAT' && o.period === '2026-Q2').deadline, '2026-08-17'); // August
    assert.equal(obs.find((o) => o.type === 'VAT' && o.period === '2026-Q3').deadline, '2026-11-12');
    // annual report filed with Bolagsverket within 7 months of FYE (12-31 -> 07-31)
    assert.equal(obs.find((o) => o.type === 'ANNUAL_ACCOUNTS' && o.period === '2026').deadline, '2027-07-31');
    // no NL/LU/GB/FR/US/BE/DE/DK/FI/NO types leak into the SE calendar
    assert.ok(!obs.some((o) => ['OB', 'ICP', 'JAARREKENING', 'TVA', 'COMPTES_ANNUELS', 'CT600', 'FEDERAL_INCOME_TAX', 'PAYROLL_941', 'UMSATZSTEUER_VORANMELDUNG', 'UMSATZSTEUER_JAHRESERKLAERUNG'].includes(o.type)));
  } finally {
    db.close();
  }
});

// --- Phase C: AT profile (Einheitskontenrahmen, EUR) ------------------------

test('AT: getProfile returns the AT profile (EUR, de-AT, EKR data)', () => {
  const p = getProfile('AT');
  assert.equal(p.meta.country, 'AT');
  assert.equal(p.meta.baseCurrency, 'EUR');
  assert.equal(p.meta.locale, 'de-AT');
  assert.ok(p.meta.legalForms.includes('gmbh'));
  assert.ok(!p.meta.legalForms.includes('bv')); // NL form rejected
  // Peppol scheme: 9914 = Österreichische UID (not 9930 DE / not 9944 NL)
  assert.equal(p.identifiers.peppolSchemeId, '9914');
  assert.ok(p.identifiers.vatIdFormat.test('ATU12345678'));
  assert.ok(!p.identifiers.vatIdFormat.test('DE123456789'));
  assert.equal(p.tax.standardRateBp, 2000);
  assert.equal(p.tax.smallBusinessScheme, 'kleinunternehmer'); // €55K since 2025
  // the first income account is the default sales account — the 20% Erlöse
  // account for a VAT-registered GmbH (not a § 6 exemption account)
  assert.equal(p.reporting.defaultChart.find((a) => a.type === 'income').code, '4000');
  assert.deepEqual(p.tax.codes.map((c) => c.code), ['20', '13', '10', 'V', 'R', 'RE', 'M', 'P']);
  // EKR VAT accounts: 2500 Vorsteuer (input) / 3500 Umsatzsteuer (output);
  // settlement on 3500 (the EKR has single VAT accounts, no per-rate split)
  assert.deepEqual(p.tax.accounts.ledger.map((a) => a.code), ['2500', '3500']);
  assert.equal(p.tax.accounts.fileDefault, '3500');
  assert.equal(p.reporting.debtorsAccount, '2000'); // Forderungen L+L Inland
  // EKR closing: Jahresgewinn/-verlust 9350 -> Gewinnvortrag 9380
  assert.equal(p.closing.resultAccount, '9350');
  assert.equal(p.closing.equityAccount, '9380');
  assert.equal(p.reporting.taxonomy, null);
  // chart: EKR codes (3-digit codes zero-padded to bukio's 4-digit form)
  assert.ok(p.reporting.defaultChart.some((a) => a.code === '2800' && a.name === 'Guthaben bei Kreditinstituten'));
  assert.ok(p.reporting.defaultChart.some((a) => a.code === '2700' && a.name === 'Kassa'));
  assert.ok(p.reporting.defaultChart.some((a) => a.code === '4000' && a.name === 'Erlöse 20 %'));
  assert.ok(p.reporting.defaultChart.some((a) => a.code === '0620')); // zero-padded Büromaschinen
  assert.ok(p.reporting.defaultChart.length >= 28);
  // B-milestones stay unregistered (strict dispatch fails loudly)
  assert.equal(p.tax.returnLayout, undefined);
  assert.equal(p.reporting.format, undefined);
  assert.equal(p.documents.invoiceCompliance, 'eu-invoice-vereisten'); // art. 226 baseline
  assert.equal(p.documents.auditFile, undefined);
  assert.deepEqual(p.exchange.paymentFormats, ['sepa-pain.001', 'sepa-pain.008']);
  assert.equal(p.documents.eInvoicing, 'peppol-bis-3.0');
  // AT deadlines: UVA quarterly (15th of the second following month), annual
  // VAT 30 June (electronic)
  assert.deepEqual(p.compliance.filingTypes.map((ft) => ft.deadlineRule), ['at-uva-quarterly', 'at-annual-vat']);
});

test('AT: PLANNED is empty (all thirty-one markets landed)', () => {
  assert.ok(!PLANNED.includes('AT'));
  assert.deepEqual([...PLANNED].sort(), []);
});

test('AT: init --country AT creates an Austrian company with the EKR chart', () => {
  const dbPath = tmpDb();
  const r = cli(dbPath, ['init', '--name', 'Muster GmbH', '--country', 'AT', '--legal-form', 'gmbh', '--vat', 'on']);
  assert.equal(r.out.data.company.country, 'AT');
  assert.equal(r.out.data.company.base_currency, 'EUR');
  assert.equal(r.out.data.company.locale, 'de-AT');
  const db = openDb(dbPath);
  try {
    const accounts = db.prepare('SELECT code, name, taxonomy FROM accounts WHERE active = 1').all();
    assert.ok(accounts.some((a) => a.code === '2800' && a.name === 'Guthaben bei Kreditinstituten'));
    assert.ok(accounts.some((a) => a.code === '2500' && a.name === 'Vorsteuer'));
    assert.ok(accounts.some((a) => a.code === '3500' && a.name === 'Umsatzsteuer'));
    for (const a of accounts) assert.equal(a.taxonomy, null);
    // Document language follows the profile — AT defaults to de (de-AT -> de)
    // nl* locales default to Dutch), explicit --language still overrides.
    const c = createContact(db, { name: 'Kunde GmbH' });
    const inv = createInvoice(db, { contactId: c.id, lines: ['Dienst @ 10.00'], date: '2026-08-10', actor: 'agent:test' });
    assert.equal(inv.language, 'de'); // de-AT -> de (fully localised PDF)
    const invNl = createInvoice(db, { contactId: c.id, lines: ['Dienst @ 10.00'], date: '2026-08-11', language: 'nl', actor: 'agent:test' });
    assert.equal(invNl.language, 'nl');
  } finally {
    db.close();
  }
  // NL legal form rejected for AT
  const bad = cli(tmpDb(), ['init', '--name', 'X', '--country', 'AT', '--legal-form', 'bv'], { expectFail: true });
  assert.equal(bad.out.error.code, 'INVALID_LEGAL_FORM');
  // KOR is an NL-only scheme
  const kor = cli(tmpDb(), ['init', '--name', 'X', '--country', 'AT', '--legal-form', 'gmbh', '--kor'], { expectFail: true });
  assert.equal(kor.out.error.code, 'INVALID_VAT_CHOICE');
});

test('AT: strict dispatch — unregistered formats fail loudly (no fallback)', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Muster GmbH', '--country', 'AT', '--legal-form', 'gmbh', '--vat', 'on']);
  let r = cli(dbPath, ['financial-statements', 'report', '--year', '2026'], { expectFail: true });
  assert.equal(r.out.error.code, 'FORMAT_NOT_SUPPORTED');
  r = cli(dbPath, ['export', 'xaf', '--year', '2026', '--out', '/tmp/xaf-at.xml'], { expectFail: true });
  assert.equal(r.out.error.code, 'FORMAT_NOT_SUPPORTED'); // SAF-T AT is a different schema
  r = cli(dbPath, ['vat', 'readout', '--period', '2026-Q1'], { expectFail: true });
  assert.equal(r.out.error.code, 'FORMAT_NOT_SUPPORTED'); // UVA engine is a B-milestone
  const db = openDb(dbPath);
  try {
    // the art. 226 EU baseline is registered — the rule RUNS and enforces
    // (the test company is minimal, so the supplier party checks fire)
    assert.throws(
      () => validateCompliance(db, { invoice_type: 'invoice', lines: [] }),
      (e) => e.code === 'SUPPLIER_INCOMPLETE',
    );
  } finally {
    db.close();
  }
});

test('AT: compliance calendar — UVA 15th of second following month + annual VAT 30 Jun', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Muster GmbH', '--country', 'AT', '--legal-form', 'gmbh', '--vat', 'on']);
  const db = openDb(dbPath);
  try {
    const r = complianceStatus(db, { year: 2026 });
    const obs = r.obligations;
    // quarterly UVA due the 15th of the SECOND following month (§ 21 UStG)
    assert.equal(obs.find((o) => o.type === 'UMSATZSTEUER_VORANMELDUNG' && o.period === '2026-Q1').deadline, '2026-05-15');
    assert.equal(obs.find((o) => o.type === 'UMSATZSTEUER_VORANMELDUNG' && o.period === '2026-Q2').deadline, '2026-08-15');
    assert.equal(obs.find((o) => o.type === 'UMSATZSTEUER_VORANMELDUNG' && o.period === '2026-Q3').deadline, '2026-11-15');
    assert.equal(obs.find((o) => o.type === 'UMSATZSTEUER_VORANMELDUNG' && o.period === '2026-Q4').deadline, '2027-02-15');
    assert.equal(obs.find((o) => o.type === 'UMSATZSTEUER_VORANMELDUNG' && o.period === '2025-Q4').deadline, '2026-02-15');
    // annual USt-Erklärung 30 June of the following year (electronic filing)
    assert.equal(obs.find((o) => o.type === 'UMSATZSTEUER_JAHRESERKLAERUNG' && o.period === '2026').deadline, '2027-06-30');
    // no NL/LU/GB/FR/US/BE/DE/DK/FI/NO/SE types leak into the AT calendar
    assert.ok(!obs.some((o) => ['OB', 'ICP', 'JAARREKENING', 'TVA', 'COMPTES_ANNUELS', 'CT600', 'FEDERAL_INCOME_TAX', 'PAYROLL_941', 'VAT3', 'ANNUAL_ACCOUNTS', 'VAT'].includes(o.type)));
  } finally {
    db.close();
  }
});

// --- Phase C: IE profile (UK-style chart, EUR, VAT3 bi-monthly) -------------

test('IE: getProfile returns the IE profile (EUR, en, UK-style chart)', () => {
  const p = getProfile('IE');
  assert.equal(p.meta.country, 'IE');
  assert.equal(p.meta.baseCurrency, 'EUR');
  assert.equal(p.meta.locale, 'en');
  assert.ok(p.meta.legalForms.includes('ltd'));
  assert.ok(!p.meta.legalForms.includes('gmbh')); // AT form rejected
  // Peppol scheme: 9935 = Ireland VAT number (not 9932 GB / 9944 NL)
  assert.equal(p.identifiers.peppolSchemeId, '9935');
  assert.ok(p.identifiers.vatIdFormat.test('IE1234567T'));
  assert.ok(p.identifiers.vatIdFormat.test('IE1234567TW'));
  assert.ok(!p.identifiers.vatIdFormat.test('DE123456789'));
  assert.equal(p.tax.standardRateBp, 2300);
  assert.equal(p.tax.smallBusinessScheme, 'threshold'); // €85K goods / €42.5K services
  // the first income account is the default sales account
  assert.equal(p.reporting.defaultChart.find((a) => a.type === 'income').code, '4000');
  assert.deepEqual(p.tax.codes.map((c) => c.code), ['23', '13.5', '9', '4.8', '0', 'V', 'R', 'RE', 'M', 'P']);
  // UK-style VAT control accounts: 2110 input / 2100 output; settlement 2120
  assert.deepEqual(p.tax.accounts.ledger.map((a) => a.code), ['2110', '2100']);
  assert.equal(p.tax.accounts.fileDefault, '2120');
  assert.equal(p.reporting.debtorsAccount, '1100');
  // closing: Profit/(loss) for the year 3300 -> P&L account 3200
  assert.equal(p.closing.resultAccount, '3300');
  assert.equal(p.closing.equityAccount, '3200');
  assert.equal(p.reporting.taxonomy, null);
  // chart: UK-style codes (1000s assets ... 7000s expenses)
  assert.ok(p.reporting.defaultChart.some((a) => a.code === '1000' && a.name === 'Bank — current account'));
  assert.ok(p.reporting.defaultChart.some((a) => a.code === '2100' && a.name === 'VAT on sales (output)'));
  assert.ok(p.reporting.defaultChart.some((a) => a.code === '4000' && a.name === 'Sales — goods'));
  assert.ok(p.reporting.defaultChart.length >= 35);
  // B-milestones stay unregistered (strict dispatch fails loudly)
  assert.equal(p.tax.returnLayout, undefined);
  assert.equal(p.reporting.format, undefined);
  assert.equal(p.documents.invoiceCompliance, 'eu-invoice-vereisten'); // art. 226 baseline
  assert.equal(p.documents.auditFile, undefined);
  assert.deepEqual(p.exchange.paymentFormats, ['sepa-pain.001', 'sepa-pain.008']);
  assert.equal(p.documents.eInvoicing, 'peppol-bis-3.0');
  // IE deadlines: VAT3 bi-monthly (23rd), annual accounts + CT1 9 months
  assert.deepEqual(p.compliance.filingTypes.map((ft) => ft.deadlineRule), ['ie-bimonthly', 'ie-9-months', 'ie-9-months']);
});

test('IE: PLANNED is empty (all thirty-one markets landed)', () => {
  assert.ok(!PLANNED.includes('IE'));
  assert.deepEqual([...PLANNED].sort(), []);
});

test('IE: init --country IE creates an Irish company with the UK-style chart', () => {
  const dbPath = tmpDb();
  const r = cli(dbPath, ['init', '--name', 'Acme Ltd', '--country', 'IE', '--legal-form', 'ltd', '--vat', 'on']);
  assert.equal(r.out.data.company.country, 'IE');
  assert.equal(r.out.data.company.base_currency, 'EUR');
  assert.equal(r.out.data.company.locale, 'en');
  const db = openDb(dbPath);
  try {
    const accounts = db.prepare('SELECT code, name, taxonomy FROM accounts WHERE active = 1').all();
    assert.ok(accounts.some((a) => a.code === '1000' && a.name === 'Bank — current account'));
    assert.ok(accounts.some((a) => a.code === '2100' && a.name === 'VAT on sales (output)'));
    assert.ok(accounts.some((a) => a.code === '2110' && a.name === 'VAT on purchases (reclaimable)'));
    for (const a of accounts) assert.equal(a.taxonomy, null);
    // Document language follows the profile — IE defaults to en (en-IE)
    const c = createContact(db, { name: 'Customer Ltd' });
    const inv = createInvoice(db, { contactId: c.id, lines: ['Service @ 10.00'], date: '2026-08-10', actor: 'agent:test' });
    assert.equal(inv.language, 'en');
  } finally {
    db.close();
  }
  // AT legal form rejected for IE
  const bad = cli(tmpDb(), ['init', '--name', 'X', '--country', 'IE', '--legal-form', 'gmbh'], { expectFail: true });
  assert.equal(bad.out.error.code, 'INVALID_LEGAL_FORM');
  // KOR is an NL-only scheme
  const kor = cli(tmpDb(), ['init', '--name', 'X', '--country', 'IE', '--legal-form', 'ltd', '--kor'], { expectFail: true });
  assert.equal(kor.out.error.code, 'INVALID_VAT_CHOICE');
});

test('IE: strict dispatch — unregistered formats fail loudly (no fallback)', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Acme Ltd', '--country', 'IE', '--legal-form', 'ltd', '--vat', 'on']);
  let r = cli(dbPath, ['financial-statements', 'report', '--year', '2026'], { expectFail: true });
  assert.equal(r.out.error.code, 'FORMAT_NOT_SUPPORTED');
  r = cli(dbPath, ['export', 'xaf', '--year', '2026', '--out', '/tmp/xaf-ie.xml'], { expectFail: true });
  assert.equal(r.out.error.code, 'FORMAT_NOT_SUPPORTED');
  r = cli(dbPath, ['vat', 'readout', '--period', '2026-P1'], { expectFail: true });
  assert.equal(r.out.error.code, 'FORMAT_NOT_SUPPORTED'); // VAT3 9-box engine is a B-milestone
  const db = openDb(dbPath);
  try {
    // the art. 226 EU baseline is registered — the rule RUNS and enforces
    // (the test company is minimal, so the supplier party checks fire)
    assert.throws(
      () => validateCompliance(db, { invoice_type: 'invoice', lines: [] }),
      (e) => e.code === 'SUPPLIER_INCOMPLETE',
    );
  } finally {
    db.close();
  }
});

test('IE: compliance calendar — VAT3 bi-monthly 23rd + annual accounts/CT1 9 months', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Acme Ltd', '--country', 'IE', '--legal-form', 'ltd', '--vat', 'on']);
  const db = openDb(dbPath);
  try {
    const r = complianceStatus(db, { year: 2026 });
    const obs = r.obligations;
    // VAT3 bi-monthly, due the 23rd of the month after the period end
    assert.equal(obs.find((o) => o.type === 'VAT3' && o.period === '2026-P1').deadline, '2026-03-23');
    assert.equal(obs.find((o) => o.type === 'VAT3' && o.period === '2026-P2').deadline, '2026-05-23');
    assert.equal(obs.find((o) => o.type === 'VAT3' && o.period === '2026-P5').deadline, '2026-11-23');
    assert.equal(obs.find((o) => o.type === 'VAT3' && o.period === '2026-P6').deadline, '2027-01-23');
    assert.equal(obs.find((o) => o.type === 'VAT3' && o.period === '2025-P6').deadline, '2026-01-23');
    // annual accounts + CT1 due 9 months after the FYE (12-31 -> 09-30)
    assert.equal(obs.find((o) => o.type === 'ANNUAL_ACCOUNTS' && o.period === '2026').deadline, '2027-09-30');
    assert.equal(obs.find((o) => o.type === 'CT1' && o.period === '2026').deadline, '2027-09-30');
    // no NL/LU/GB/FR/US/BE/DE/DK/FI/NO/SE/AT types leak into the IE calendar
    assert.ok(!obs.some((o) => ['OB', 'ICP', 'JAARREKENING', 'TVA', 'COMPTES_ANNUELS', 'CT600', 'FEDERAL_INCOME_TAX', 'PAYROLL_941', 'UMSATZSTEUER_VORANMELDUNG', 'UMSATZSTEUER_JAHRESERKLAERUNG', 'VAT'].includes(o.type)));
  } finally {
    db.close();
  }
});

// --- Phase D: IT profile (commercialisti convention chart, EUR) -------------

test('IT: getProfile returns the IT profile (EUR, it, convention chart)', () => {
  const p = getProfile('IT');
  assert.equal(p.meta.country, 'IT');
  assert.equal(p.meta.baseCurrency, 'EUR');
  assert.equal(p.meta.locale, 'it');
  assert.ok(p.meta.legalForms.includes('srl'));
  assert.ok(!p.meta.legalForms.includes('bv')); // NL form rejected
  // Peppol scheme: 0211 = Partita IVA (not 9920 ES / 9944 NL)
  assert.equal(p.identifiers.peppolSchemeId, '0211');
  assert.ok(p.identifiers.vatIdFormat.test('IT12345678901'));
  assert.ok(!p.identifiers.vatIdFormat.test('DE123456789'));
  assert.equal(p.tax.standardRateBp, 2200);
  assert.equal(p.tax.smallBusinessScheme, 'forfettario'); // flat-rate ≤ €85K
  // the first income account is the default sales account
  assert.equal(p.reporting.defaultChart.find((a) => a.type === 'income').code, '4000');
  assert.deepEqual(p.tax.codes.map((c) => c.code), ['22', '10', '5', '4', 'V', 'R', 'RE', 'M', 'P']);
  // convention VAT accounts: 1300 IVA a credito / 2100 IVA a debito;
  // settlement on 2400 Erario c/IVA
  assert.deepEqual(p.tax.accounts.ledger.map((a) => a.code), ['1300', '2100']);
  assert.equal(p.tax.accounts.fileDefault, '2400');
  assert.equal(p.reporting.debtorsAccount, '1200'); // Crediti v/clienti
  // closing: Utile (perdita) dell'esercizio 3200 -> Utili a nuovo 3100
  assert.equal(p.closing.resultAccount, '3200');
  assert.equal(p.closing.equityAccount, '3100');
  assert.equal(p.reporting.taxonomy, null);
  // chart: commercialisti convention codes
  assert.ok(p.reporting.defaultChart.some((a) => a.code === '1100' && a.name === 'Banca c/c'));
  assert.ok(p.reporting.defaultChart.some((a) => a.code === '1000' && a.name === 'Cassa contanti'));
  assert.ok(p.reporting.defaultChart.some((a) => a.code === '4000'));
  assert.ok(p.reporting.defaultChart.length >= 24);
  // B-milestones stay unregistered (strict dispatch fails loudly)
  assert.equal(p.tax.returnLayout, undefined);
  assert.equal(p.reporting.format, undefined);
  assert.equal(p.documents.invoiceCompliance, 'eu-invoice-vereisten'); // art. 226 baseline
  assert.equal(p.documents.auditFile, undefined);
  assert.deepEqual(p.exchange.paymentFormats, ['sepa-pain.001', 'sepa-pain.008']);
  // cross-border Peppol registered; domestic FatturaPA/SdI is a B-milestone
  assert.equal(p.documents.eInvoicing, 'peppol-bis-3.0');
  // IT deadlines: liquidazione quarterly (16th of 2nd month), Dichiarazione
  // IVA 30 Apr, bilancio 5 months
  assert.deepEqual(p.compliance.filingTypes.map((ft) => ft.deadlineRule), ['it-liquidazione-quarterly', 'it-dichiarazione-iva', 'it-bilancio']);
});

test('IT: init --country IT creates an Italian company with the convention chart', () => {
  const dbPath = tmpDb();
  const r = cli(dbPath, ['init', '--name', 'Rossi SRL', '--country', 'IT', '--legal-form', 'srl', '--vat', 'on']);
  assert.equal(r.out.data.company.country, 'IT');
  assert.equal(r.out.data.company.base_currency, 'EUR');
  assert.equal(r.out.data.company.locale, 'it');
  const db = openDb(dbPath);
  try {
    const accounts = db.prepare('SELECT code, name, taxonomy FROM accounts WHERE active = 1').all();
    assert.ok(accounts.some((a) => a.code === '1100' && a.name === 'Banca c/c'));
    assert.ok(accounts.some((a) => a.code === '1300' && a.name === 'IVA a credito'));
    assert.ok(accounts.some((a) => a.code === '2100' && a.name === 'IVA a debito'));
    for (const a of accounts) assert.equal(a.taxonomy, null);
    // Document language follows the profile — IT defaults to it (fully localised PDF)
    // nl* locales default to Dutch); explicit --language still overrides.
    const c = createContact(db, { name: 'Cliente SRL' });
    const inv = createInvoice(db, { contactId: c.id, lines: ['Servizio @ 10.00'], date: '2026-08-10', actor: 'agent:test' });
    assert.equal(inv.language, 'it'); // the profile's language (fully localised PDF)
    const invNl = createInvoice(db, { contactId: c.id, lines: ['Servizio @ 10.00'], date: '2026-08-11', language: 'nl', actor: 'agent:test' });
    assert.equal(invNl.language, 'nl'); // explicit --language still overrides (documents are nl|en)
  } finally {
    db.close();
  }
  // NL legal form rejected for IT
  const bad = cli(tmpDb(), ['init', '--name', 'X', '--country', 'IT', '--legal-form', 'bv'], { expectFail: true });
  assert.equal(bad.out.error.code, 'INVALID_LEGAL_FORM');
  // KOR is an NL-only scheme
  const kor = cli(tmpDb(), ['init', '--name', 'X', '--country', 'IT', '--legal-form', 'srl', '--kor'], { expectFail: true });
  assert.equal(kor.out.error.code, 'INVALID_VAT_CHOICE');
});

test('IT: strict dispatch — unregistered formats fail loudly (no fallback)', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Rossi SRL', '--country', 'IT', '--legal-form', 'srl', '--vat', 'on']);
  let r = cli(dbPath, ['financial-statements', 'report', '--year', '2026'], { expectFail: true });
  assert.equal(r.out.error.code, 'FORMAT_NOT_SUPPORTED');
  r = cli(dbPath, ['export', 'xaf', '--year', '2026', '--out', '/tmp/xaf-it.xml'], { expectFail: true });
  assert.equal(r.out.error.code, 'FORMAT_NOT_SUPPORTED');
  r = cli(dbPath, ['vat', 'readout', '--period', '2026-Q1'], { expectFail: true });
  assert.equal(r.out.error.code, 'FORMAT_NOT_SUPPORTED'); // liquidazione engine is a B-milestone
  const db = openDb(dbPath);
  try {
    // the art. 226 EU baseline is registered — the rule RUNS and enforces
    // (the test company is minimal, so the supplier party checks fire)
    assert.throws(
      () => validateCompliance(db, { invoice_type: 'invoice', lines: [] }),
      (e) => e.code === 'SUPPLIER_INCOMPLETE',
    );
  } finally {
    db.close();
  }
});

test('IT: compliance calendar — liquidazione 16th of 2nd month + Dichiarazione 30 Apr + bilancio', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Rossi SRL', '--country', 'IT', '--legal-form', 'srl', '--vat', 'on']);
  const db = openDb(dbPath);
  try {
    const r = complianceStatus(db, { year: 2026 });
    const obs = r.obligations;
    // quarterly liquidazione: 16th of the SECOND month after the quarter
    assert.equal(obs.find((o) => o.type === 'LIQUIDAZIONE_IVA' && o.period === '2026-Q1').deadline, '2026-05-16');
    assert.equal(obs.find((o) => o.type === 'LIQUIDAZIONE_IVA' && o.period === '2026-Q2').deadline, '2026-08-16');
    assert.equal(obs.find((o) => o.type === 'LIQUIDAZIONE_IVA' && o.period === '2026-Q3').deadline, '2026-11-16');
    assert.equal(obs.find((o) => o.type === 'LIQUIDAZIONE_IVA' && o.period === '2026-Q4').deadline, '2027-02-16');
    assert.equal(obs.find((o) => o.type === 'LIQUIDAZIONE_IVA' && o.period === '2025-Q4').deadline, '2026-02-16');
    // annual Dichiarazione IVA 30 April; bilancio deposit ~5 months
    assert.equal(obs.find((o) => o.type === 'DICHIARAZIONE_IVA' && o.period === '2026').deadline, '2027-04-30');
    assert.equal(obs.find((o) => o.type === 'BILANCIO' && o.period === '2026').deadline, '2027-05-31');
    // no NL/LU/GB/FR/US/BE/DE/DK/FI/NO/SE/AT/IE/ES/PT types leak into IT
    assert.ok(!obs.some((o) => ['OB', 'ICP', 'JAARREKENING', 'TVA', 'COMPTES_ANNUELS', 'CT600', 'FEDERAL_INCOME_TAX', 'PAYROLL_941', 'VAT3', 'ANNUAL_ACCOUNTS', 'UMSATZSTEUER_VORANMELDUNG', 'IVA_TRIMESTRAL', 'IVA_ANUAL', 'IMPUESTO_SOCIEDADES', 'CUENTAS_ANUALES', 'IVA_DP', 'IRC', 'CONTAS_ANUAIS'].includes(o.type)));
  } finally {
    db.close();
  }
});

// --- Phase D: ES profile (PGC official chart, EUR) --------------------------

test('ES: getProfile returns the ES profile (EUR, es, PGC chart)', () => {
  const p = getProfile('ES');
  assert.equal(p.meta.country, 'ES');
  assert.equal(p.meta.baseCurrency, 'EUR');
  assert.equal(p.meta.locale, 'es');
  assert.ok(p.meta.legalForms.includes('sl'));
  assert.ok(!p.meta.legalForms.includes('gmbh')); // AT form rejected
  // Peppol scheme: 9920 = AEAT NIF
  assert.equal(p.identifiers.peppolSchemeId, '9920');
  assert.ok(p.identifiers.vatIdFormat.test('ESB12345678'));
  assert.ok(!p.identifiers.vatIdFormat.test('DE123456789'));
  assert.equal(p.tax.standardRateBp, 2100);
  assert.equal(p.tax.smallBusinessScheme, 'recargo-equivalencia');
  assert.equal(p.reporting.defaultChart.find((a) => a.type === 'income').code, '700');
  assert.deepEqual(p.tax.codes.map((c) => c.code), ['21', '10', '4', 'V', 'R', 'RE', 'M', 'P']);
  // PGC VAT accounts: 472 IVA soportado / 477 IVA repercutido; settlement 475
  assert.deepEqual(p.tax.accounts.ledger.map((a) => a.code), ['472', '477']);
  assert.equal(p.tax.accounts.fileDefault, '475');
  assert.equal(p.reporting.debtorsAccount, '430'); // Clientes
  // closing: Resultado del ejercicio 129 -> Resultados negativos 121
  assert.equal(p.closing.resultAccount, '129');
  assert.equal(p.closing.equityAccount, '121');
  assert.equal(p.reporting.taxonomy, null);
  // chart: official PGC codes
  assert.ok(p.reporting.defaultChart.some((a) => a.code === '570' && a.name === 'Caja, euros'));
  assert.ok(p.reporting.defaultChart.some((a) => a.code === '572' && a.name === 'Bancos e instituciones de crédito c/c'));
  assert.ok(p.reporting.defaultChart.some((a) => a.code === '700' && a.name === 'Ventas de mercaderías'));
  assert.ok(p.reporting.defaultChart.length >= 28);
  // B-milestones stay unregistered
  assert.equal(p.tax.returnLayout, undefined);
  assert.equal(p.reporting.format, undefined);
  assert.equal(p.documents.invoiceCompliance, 'eu-invoice-vereisten'); // art. 226 baseline
  assert.equal(p.documents.auditFile, undefined);
  assert.deepEqual(p.exchange.paymentFormats, ['sepa-pain.001', 'sepa-pain.008']);
  assert.equal(p.documents.eInvoicing, 'peppol-bis-3.0'); // cross-border; Verifactu B-milestone
  assert.deepEqual(p.compliance.filingTypes.map((ft) => ft.deadlineRule), ['es-303-quarterly', 'es-390', 'es-200', 'es-7-months']);
});

test('ES: init --country ES creates a Spanish company with the PGC chart', () => {
  const dbPath = tmpDb();
  const r = cli(dbPath, ['init', '--name', 'Perez SL', '--country', 'ES', '--legal-form', 'sl', '--vat', 'on']);
  assert.equal(r.out.data.company.country, 'ES');
  assert.equal(r.out.data.company.base_currency, 'EUR');
  assert.equal(r.out.data.company.locale, 'es');
  const db = openDb(dbPath);
  try {
    const accounts = db.prepare('SELECT code, name, taxonomy FROM accounts WHERE active = 1').all();
    assert.ok(accounts.some((a) => a.code === '572' && a.name === 'Bancos e instituciones de crédito c/c'));
    assert.ok(accounts.some((a) => a.code === '472' && a.name === 'H.P. IVA soportado'));
    assert.ok(accounts.some((a) => a.code === '477' && a.name === 'H.P. IVA repercutido'));
    for (const a of accounts) assert.equal(a.taxonomy, null);
    // Document language defaults to English (no nl* locale); explicit wins
    const c = createContact(db, { name: 'Cliente SL' });
    const inv = createInvoice(db, { contactId: c.id, lines: ['Servicio @ 10.00'], date: '2026-08-10', actor: 'agent:test' });
    assert.equal(inv.language, 'es'); // the profile's language (fully localised PDF)
  } finally {
    db.close();
  }
  const bad = cli(tmpDb(), ['init', '--name', 'X', '--country', 'ES', '--legal-form', 'bv'], { expectFail: true });
  assert.equal(bad.out.error.code, 'INVALID_LEGAL_FORM');
  const kor = cli(tmpDb(), ['init', '--name', 'X', '--country', 'ES', '--legal-form', 'sl', '--kor'], { expectFail: true });
  assert.equal(kor.out.error.code, 'INVALID_VAT_CHOICE');
});

test('ES: strict dispatch — unregistered formats fail loudly (no fallback)', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Perez SL', '--country', 'ES', '--legal-form', 'sl', '--vat', 'on']);
  let r = cli(dbPath, ['financial-statements', 'report', '--year', '2026'], { expectFail: true });
  assert.equal(r.out.error.code, 'FORMAT_NOT_SUPPORTED');
  r = cli(dbPath, ['export', 'xaf', '--year', '2026', '--out', '/tmp/xaf-es.xml'], { expectFail: true });
  assert.equal(r.out.error.code, 'FORMAT_NOT_SUPPORTED');
  r = cli(dbPath, ['vat', 'readout', '--period', '2026-Q1'], { expectFail: true });
  assert.equal(r.out.error.code, 'FORMAT_NOT_SUPPORTED'); // Modelo 303 engine is a B-milestone
  const db = openDb(dbPath);
  try {
    // the art. 226 EU baseline is registered — the rule RUNS and enforces
    // (the test company is minimal, so the supplier party checks fire)
    assert.throws(
      () => validateCompliance(db, { invoice_type: 'invoice', lines: [] }),
      (e) => e.code === 'SUPPLIER_INCOMPLETE',
    );
  } finally {
    db.close();
  }
});

test('ES: compliance calendar — Modelo 303 quarterly + 390 + 200 + cuentas anuales', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Perez SL', '--country', 'ES', '--legal-form', 'sl', '--vat', 'on']);
  const db = openDb(dbPath);
  try {
    const r = complianceStatus(db, { year: 2026 });
    const obs = r.obligations;
    // Modelo 303: first 20 days after the quarter; Q4 until 30 Jan next year
    assert.equal(obs.find((o) => o.type === 'IVA_TRIMESTRAL' && o.period === '2026-Q1').deadline, '2026-04-20');
    assert.equal(obs.find((o) => o.type === 'IVA_TRIMESTRAL' && o.period === '2026-Q2').deadline, '2026-07-20');
    assert.equal(obs.find((o) => o.type === 'IVA_TRIMESTRAL' && o.period === '2026-Q3').deadline, '2026-10-20');
    assert.equal(obs.find((o) => o.type === 'IVA_TRIMESTRAL' && o.period === '2026-Q4').deadline, '2027-01-30');
    assert.equal(obs.find((o) => o.type === 'IVA_TRIMESTRAL' && o.period === '2025-Q4').deadline, '2026-01-30');
    // Modelo 390 annual 30 Jan; Modelo 200 25 Jul; cuentas anuales 31 Jul
    assert.equal(obs.find((o) => o.type === 'IVA_ANUAL' && o.period === '2026').deadline, '2027-01-30');
    assert.equal(obs.find((o) => o.type === 'IMPUESTO_SOCIEDADES' && o.period === '2026').deadline, '2027-07-25');
    assert.equal(obs.find((o) => o.type === 'CUENTAS_ANUALES' && o.period === '2026').deadline, '2027-07-31');
    // no other markets' types leak
    assert.ok(!obs.some((o) => ['OB', 'ICP', 'JAARREKENING', 'TVA', 'COMPTES_ANNUELS', 'CT600', 'FEDERAL_INCOME_TAX', 'PAYROLL_941', 'VAT3', 'ANNUAL_ACCOUNTS', 'LIQUIDAZIONE_IVA', 'DICHIARAZIONE_IVA', 'BILANCIO', 'IVA_DP', 'IRC', 'CONTAS_ANUAIS'].includes(o.type)));
  } finally {
    db.close();
  }
});

// --- Phase D: PT profile (SNC official chart, EUR) --------------------------

test('PT: getProfile returns the PT profile (EUR, pt, SNC chart)', () => {
  const p = getProfile('PT');
  assert.equal(p.meta.country, 'PT');
  assert.equal(p.meta.baseCurrency, 'EUR');
  assert.equal(p.meta.locale, 'pt');
  assert.ok(p.meta.legalForms.includes('lda'));
  assert.ok(!p.meta.legalForms.includes('gmbh')); // AT form rejected
  // Peppol scheme: 9946 = Portugal VAT number
  assert.equal(p.identifiers.peppolSchemeId, '9946');
  assert.ok(p.identifiers.vatIdFormat.test('PT501234567'));
  assert.ok(!p.identifiers.vatIdFormat.test('DE123456789'));
  assert.equal(p.tax.standardRateBp, 2300);
  assert.equal(p.tax.smallBusinessScheme, 'threshold'); // art. 53 CIVA isenção
  assert.equal(p.reporting.defaultChart.find((a) => a.type === 'income').code, '0071');
  assert.deepEqual(p.tax.codes.map((c) => c.code), ['23', '13', '6', 'V', 'R', 'RE', 'M', 'P']);
  // SNC VAT accounts: 2432 IVA dedutível / 2433 IVA liquidado; settlement 2434
  assert.deepEqual(p.tax.accounts.ledger.map((a) => a.code), ['2432', '2433']);
  assert.equal(p.tax.accounts.fileDefault, '2434');
  assert.equal(p.reporting.debtorsAccount, '0021'); // Clientes (zero-padded SNC)
  // closing: Resultado líquido do período 8181 -> Resultados transitados 0056
  assert.equal(p.closing.resultAccount, '8181');
  assert.equal(p.closing.equityAccount, '0056');
  assert.equal(p.reporting.taxonomy, null);
  // chart: SNC codes (2-digit bases zero-padded to 4 digits)
  assert.ok(p.reporting.defaultChart.some((a) => a.code === '0012' && a.name === 'Depósitos à ordem'));
  assert.ok(p.reporting.defaultChart.some((a) => a.code === '0011' && a.name === 'Caixa'));
  assert.ok(p.reporting.defaultChart.some((a) => a.code === '0071' && a.name === 'Vendas'));
  assert.ok(p.reporting.defaultChart.some((a) => a.code === '2432' && a.name === 'IVA dedutível'));
  assert.ok(p.reporting.defaultChart.length >= 20);
  // B-milestones stay unregistered
  assert.equal(p.tax.returnLayout, undefined);
  assert.equal(p.reporting.format, undefined);
  assert.equal(p.documents.invoiceCompliance, 'eu-invoice-vereisten'); // art. 226 baseline
  assert.equal(p.documents.auditFile, undefined);
  assert.deepEqual(p.exchange.paymentFormats, ['sepa-pain.001', 'sepa-pain.008']);
  assert.equal(p.documents.eInvoicing, 'peppol-bis-3.0'); // cross-border; ATCUD B-milestone
  assert.deepEqual(p.compliance.filingTypes.map((ft) => ft.deadlineRule), ['pt-dp-quarterly', 'pt-irc', 'pt-ies']);
});

test('PT: init --country PT creates a Portuguese company with the SNC chart', () => {
  const dbPath = tmpDb();
  const r = cli(dbPath, ['init', '--name', 'Silva Lda', '--country', 'PT', '--legal-form', 'lda', '--vat', 'on']);
  assert.equal(r.out.data.company.country, 'PT');
  assert.equal(r.out.data.company.base_currency, 'EUR');
  assert.equal(r.out.data.company.locale, 'pt');
  const db = openDb(dbPath);
  try {
    const accounts = db.prepare('SELECT code, name, taxonomy FROM accounts WHERE active = 1').all();
    assert.ok(accounts.some((a) => a.code === '0012' && a.name === 'Depósitos à ordem'));
    assert.ok(accounts.some((a) => a.code === '2432' && a.name === 'IVA dedutível'));
    assert.ok(accounts.some((a) => a.code === '2433' && a.name === 'IVA liquidado'));
    for (const a of accounts) assert.equal(a.taxonomy, null);
    // Document language defaults to English (no nl* locale); explicit wins
    const c = createContact(db, { name: 'Cliente Lda' });
    const inv = createInvoice(db, { contactId: c.id, lines: ['Serviço @ 10.00'], date: '2026-08-10', actor: 'agent:test' });
    assert.equal(inv.language, 'pt'); // the profile's language (fully localised PDF)
  } finally {
    db.close();
  }
  const bad = cli(tmpDb(), ['init', '--name', 'X', '--country', 'PT', '--legal-form', 'gmbh'], { expectFail: true });
  assert.equal(bad.out.error.code, 'INVALID_LEGAL_FORM');
  const kor = cli(tmpDb(), ['init', '--name', 'X', '--country', 'PT', '--legal-form', 'lda', '--kor'], { expectFail: true });
  assert.equal(kor.out.error.code, 'INVALID_VAT_CHOICE');
});

test('PT: strict dispatch — unregistered formats fail loudly (no fallback)', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Silva Lda', '--country', 'PT', '--legal-form', 'lda', '--vat', 'on']);
  let r = cli(dbPath, ['financial-statements', 'report', '--year', '2026'], { expectFail: true });
  assert.equal(r.out.error.code, 'FORMAT_NOT_SUPPORTED');
  r = cli(dbPath, ['export', 'xaf', '--year', '2026', '--out', '/tmp/xaf-pt.xml'], { expectFail: true });
  assert.equal(r.out.error.code, 'FORMAT_NOT_SUPPORTED'); // SAF-T PT is a different schema
  r = cli(dbPath, ['vat', 'readout', '--period', '2026-Q1'], { expectFail: true });
  assert.equal(r.out.error.code, 'FORMAT_NOT_SUPPORTED'); // Declaração Periódica is a B-milestone
  const db = openDb(dbPath);
  try {
    // the art. 226 EU baseline is registered — the rule RUNS and enforces
    // (the test company is minimal, so the supplier party checks fire)
    assert.throws(
      () => validateCompliance(db, { invoice_type: 'invoice', lines: [] }),
      (e) => e.code === 'SUPPLIER_INCOMPLETE',
    );
  } finally {
    db.close();
  }
});

test('PT: compliance calendar — Declaração Periódica 20th of 2nd month + IRC + IES', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Silva Lda', '--country', 'PT', '--legal-form', 'lda', '--vat', 'on']);
  const db = openDb(dbPath);
  try {
    const r = complianceStatus(db, { year: 2026 });
    const obs = r.obligations;
    // quarterly DP: 20th of the SECOND month after the quarter
    assert.equal(obs.find((o) => o.type === 'IVA_DP' && o.period === '2026-Q1').deadline, '2026-05-20');
    assert.equal(obs.find((o) => o.type === 'IVA_DP' && o.period === '2026-Q2').deadline, '2026-08-20');
    assert.equal(obs.find((o) => o.type === 'IVA_DP' && o.period === '2026-Q3').deadline, '2026-11-20');
    assert.equal(obs.find((o) => o.type === 'IVA_DP' && o.period === '2026-Q4').deadline, '2027-02-20');
    assert.equal(obs.find((o) => o.type === 'IVA_DP' && o.period === '2025-Q4').deadline, '2026-02-20');
    // Modelo 22 (IRC) 31 May; IES 15 July
    assert.equal(obs.find((o) => o.type === 'IRC' && o.period === '2026').deadline, '2027-05-31');
    assert.equal(obs.find((o) => o.type === 'CONTAS_ANUAIS' && o.period === '2026').deadline, '2027-07-15');
    // no other markets' types leak
    assert.ok(!obs.some((o) => ['OB', 'ICP', 'JAARREKENING', 'TVA', 'COMPTES_ANNUELS', 'CT600', 'FEDERAL_INCOME_TAX', 'PAYROLL_941', 'VAT3', 'ANNUAL_ACCOUNTS', 'LIQUIDAZIONE_IVA', 'DICHIARAZIONE_IVA', 'BILANCIO', 'IVA_TRIMESTRAL', 'IVA_ANUAL', 'IMPUESTO_SOCIEDADES', 'CUENTAS_ANUALES'].includes(o.type)));
  } finally {
    db.close();
  }
});

test('EU baseline: a DE company finalizes invoices end-to-end (art. 226 rule + de document language)', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Muster GmbH', '--country', 'DE', '--legal-form', 'gmbh', '--vat', 'on',
    '--registration-id', 'HRB 123456', '--tax-id', 'DE123456789', '--address', 'Musterstr. 1',
    '--postal-code', '10115', '--city', 'Berlin']);
  const db = openDb(dbPath);
  try {
    const c = createContact(db, { name: 'Kunde GmbH', address: 'Testweg 2', city: 'Hamburg' });
    const inv = createInvoice(db, { contactId: c.id, lines: ['Dienstleistung @ 100.00 @19'], date: '2026-08-10', actor: 'agent:test' });
    assert.equal(inv.language, 'de'); // the profile's language (fully localised PDF)
    finalizeInvoice(db, { id: inv.id, actor: 'agent:test' });
    const row = db.prepare('SELECT status, invoice_number FROM invoices WHERE id = ?').get(inv.id);
    assert.equal(row.status, 'sent');
    assert.ok(row.invoice_number.startsWith('2026-'));
    // reverse-charge still requires the customer VAT id (art. 226(14))
    const rev = createInvoice(db, { contactId: c.id, lines: ['Dienstleistung @ 100.00 @R'], date: '2026-08-11', actor: 'agent:test' });
    assert.throws(
      () => finalizeInvoice(db, { id: rev.id, actor: 'agent:test' }),
      (e) => e.code === 'CUSTOMER_VAT_REQUIRED',
    );
  } finally {
    db.close();
  }
});

test('BG: getProfile returns the BG profile (EUR, bg, NSS chart)', () => {
  const p = getProfile('BG');
  assert.equal(p.meta.country, 'BG');
  assert.equal(p.meta.baseCurrency, 'EUR');
  assert.ok(p.meta.legalForms.includes('eood'));
  assert.ok(!p.meta.legalForms.includes('bv')); // NL form rejected
  assert.equal(p.identifiers.peppolSchemeId, '9926'); // BG VAT EAS
  assert.ok(p.identifiers.vatIdFormat.test('BG123456789'));
  assert.equal(p.tax.standardRateBp, 2000);
  assert.deepEqual(p.tax.codes.map((c) => c.code), ['20', '9', '0', 'V', 'R', 'RE']);
  assert.deepEqual(p.tax.accounts.ledger.map((a) => a.code), ['1500', '1510']);
  assert.equal(p.tax.accounts.fileDefault, '1520');
  assert.equal(p.reporting.debtorsAccount, '1200');
  assert.equal(p.reporting.bankAccountDefault, '1010');
  assert.equal(p.closing.resultAccount, '2200');
  assert.equal(p.closing.equityAccount, '2100');
  assert.equal(p.documents.invoiceCompliance, 'eu-invoice-vereisten'); // art. 226 baseline
  assert.equal(p.documents.eInvoicing, 'peppol-bis-3.0'); // cross-border Peppol
  assert.equal(p.documents.defaultLanguage, 'bg'); // BG documents (full i18n table)
  assert.equal(p.tax.returnLayout, undefined);
  assert.equal(p.reporting.format, undefined);
  assert.equal(p.documents.auditFile, undefined);
  assert.deepEqual(p.compliance.filingTypes.map((ft) => ft.deadlineRule), ['bg-vat-monthly', 'bg-annual-accounts', 'bg-cit']);
});

test('BG: init --country BG creates a Bulgarian company with the NSS chart', () => {
  const dbPath = tmpDb();
  const r = cli(dbPath, ['init', '--name', 'Test EOOD', '--country', 'BG', '--legal-form', 'eood', '--vat', 'on']);
  assert.equal(r.out.data.company.country, 'BG');
  assert.equal(r.out.data.company.base_currency, 'EUR');
  const db = openDb(dbPath);
  try {
    const accounts = db.prepare('SELECT code, name FROM accounts WHERE active = 1').all();
    assert.ok(accounts.some((a) => a.code === '1010' && a.name === 'Банкови сметки'));
    assert.ok(accounts.some((a) => a.code === '1500' && a.name === 'ДДС за възстановяване'));
    assert.ok(accounts.some((a) => a.code === '1510' && a.name === 'ДДС за внасяне'));
    const c = createContact(db, { name: 'Kunde EOOD' });
    const inv = createInvoice(db, { contactId: c.id, lines: ['Usluga @ 10.00'], date: '2026-08-10', actor: 'agent:test' });
    assert.equal(inv.language, 'bg'); // BG documents (full i18n table)
  } finally {
    db.close();
  }
  const bad = cli(tmpDb(), ['init', '--name', 'X', '--country', 'BG', '--legal-form', 'bv'], { expectFail: true });
  assert.equal(bad.out.error.code, 'INVALID_LEGAL_FORM');
});

test('BG: strict dispatch — unregistered formats fail loudly (no fallback)', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Test EOOD', '--country', 'BG', '--legal-form', 'eood', '--vat', 'on',
    '--registration-id', '123456789', '--tax-id', 'BG123456789', '--address', 'ul. 1', '--postal-code', '1000', '--city', 'Sofia']);
  let r = cli(dbPath, ['financial-statements', 'report', '--year', '2026'], { expectFail: true });
  assert.equal(r.out.error.code, 'FORMAT_NOT_SUPPORTED');
  r = cli(dbPath, ['export', 'xaf', '--year', '2026', '--out', '/tmp/xaf-bg.xml'], { expectFail: true });
  assert.equal(r.out.error.code, 'FORMAT_NOT_SUPPORTED');
  r = cli(dbPath, ['vat', 'readout', '--period', '2026-01'], { expectFail: true });
  assert.equal(r.out.error.code, 'FORMAT_NOT_SUPPORTED'); // ДДС return engine is a B-milestone
  const db = openDb(dbPath);
  try {
    const c = createContact(db, { name: 'Kunde EOOD', address: 'ul. 2', city: 'Plovdiv' });
    const inv = createInvoice(db, { contactId: c.id, lines: ['Usluga @ 10.00'], date: '2026-08-10', actor: 'agent:test' });
    finalizeInvoice(db, { id: inv.id, actor: 'agent:test' }); // art. 226 baseline works
  } finally {
    db.close();
  }
});

test('HR: getProfile returns the HR profile (EUR, hr, Računski plan)', () => {
  const p = getProfile('HR');
  assert.equal(p.meta.country, 'HR');
  assert.equal(p.meta.baseCurrency, 'EUR');
  assert.ok(p.meta.legalForms.includes('doo'));
  assert.equal(p.identifiers.peppolSchemeId, '9934'); // HR VAT EAS
  assert.ok(p.identifiers.vatIdFormat.test('HR12345678901'));
  assert.equal(p.tax.standardRateBp, 2500);
  assert.deepEqual(p.tax.codes.map((c) => c.code), ['25', '13', '5', '0', 'V', 'R', 'RE']);
  assert.deepEqual(p.tax.accounts.ledger.map((a) => a.code), ['1500', '1510']);
  assert.equal(p.reporting.debtorsAccount, '1200');
  assert.equal(p.reporting.bankAccountDefault, '1000');
  assert.equal(p.closing.resultAccount, '2200');
  assert.equal(p.documents.invoiceCompliance, 'eu-invoice-vereisten');
  assert.equal(p.documents.defaultLanguage, 'hr');
  assert.deepEqual(p.compliance.filingTypes.map((ft) => ft.deadlineRule), ['hr-vat-monthly', 'hr-annual-accounts', 'hr-cit']);
});

test('HR: init --country HR creates a Croatian company with the Računski plan chart', () => {
  const dbPath = tmpDb();
  const r = cli(dbPath, ['init', '--name', 'Test d.o.o.', '--country', 'HR', '--legal-form', 'doo', '--vat', 'on']);
  assert.equal(r.out.data.company.country, 'HR');
  const db = openDb(dbPath);
  try {
    const accounts = db.prepare('SELECT code, name FROM accounts WHERE active = 1').all();
    assert.ok(accounts.some((a) => a.code === '1000' && a.name === 'Banka — žiro račun'));
    assert.ok(accounts.some((a) => a.code === '1500' && a.name === 'Potraživanja za PDV'));
    assert.ok(accounts.some((a) => a.code === '1510' && a.name === 'Obveze za PDV'));
    const c = createContact(db, { name: 'Kupac d.o.o.' });
    const inv = createInvoice(db, { contactId: c.id, lines: ['Usluga @ 10.00'], date: '2026-08-10', actor: 'agent:test' });
    assert.equal(inv.language, 'hr'); // HR documents (full i18n table)
  } finally {
    db.close();
  }
});

test('SI: getProfile returns the SI profile (EUR, si, SRS 30 kontni načrt)', () => {
  const p = getProfile('SI');
  assert.equal(p.meta.country, 'SI');
  assert.equal(p.meta.baseCurrency, 'EUR');
  assert.ok(p.meta.legalForms.includes('doo'));
  assert.equal(p.identifiers.peppolSchemeId, '9949'); // SI VAT EAS
  assert.ok(p.identifiers.vatIdFormat.test('SI12345678'));
  assert.equal(p.tax.standardRateBp, 2200);
  assert.deepEqual(p.tax.codes.map((c) => c.code), ['22', '9.5', '5', '0', 'V', 'R', 'RE']); // dotted 9.5
  assert.deepEqual(p.tax.accounts.ledger.map((a) => a.code), ['1500', '1510']);
  assert.equal(p.reporting.debtorsAccount, '1200');
  assert.equal(p.closing.resultAccount, '2200');
  assert.equal(p.documents.invoiceCompliance, 'eu-invoice-vereisten');
  assert.equal(p.documents.defaultLanguage, 'sl');
  assert.deepEqual(p.compliance.filingTypes.map((ft) => ft.deadlineRule), ['si-vat-monthly', 'si-annual-accounts', 'si-ddpo']);
});

test('SI: init --country SI creates a Slovenian company (language defaults to sl)', () => {
  const dbPath = tmpDb();
  const r = cli(dbPath, ['init', '--name', 'Test d.o.o.', '--country', 'SI', '--legal-form', 'doo', '--vat', 'on']);
  assert.equal(r.out.data.company.country, 'SI');
  const db = openDb(dbPath);
  try {
    const accounts = db.prepare('SELECT code, name FROM accounts WHERE active = 1').all();
    assert.ok(accounts.some((a) => a.code === '1000' && a.name === 'Poslovni račun'));
    assert.ok(accounts.some((a) => a.code === '1500' && a.name === 'Vstopni DDV'));
    assert.ok(accounts.some((a) => a.code === '1510' && a.name === 'Izstopni DDV'));
    const c = createContact(db, { name: 'Kupac d.o.o.' });
    const inv = createInvoice(db, { contactId: c.id, lines: ['Storitev @ 10.00'], date: '2026-08-10', actor: 'agent:test' });
    assert.equal(inv.language, 'sl'); // SI documents (full i18n table)
  } finally {
    db.close();
  }
});

test('EE: getProfile returns the EE profile (EUR, ee, RMP convention chart)', () => {
  const p = getProfile('EE');
  assert.equal(p.meta.country, 'EE');
  assert.equal(p.meta.baseCurrency, 'EUR');
  assert.ok(p.meta.legalForms.includes('ou'));
  assert.equal(p.identifiers.peppolSchemeId, '9931'); // EE VAT EAS
  assert.ok(p.identifiers.vatIdFormat.test('EE123456789'));
  assert.equal(p.tax.standardRateBp, 2400);
  assert.deepEqual(p.tax.codes.map((c) => c.code), ['24', '9', '0', 'V', 'R', 'RE']);
  assert.deepEqual(p.tax.accounts.ledger.map((a) => a.code), ['1510', '1520']);
  assert.equal(p.reporting.debtorsAccount, '1200');
  assert.equal(p.closing.resultAccount, '2200');
  assert.equal(p.documents.invoiceCompliance, 'eu-invoice-vereisten');
  assert.equal(p.documents.defaultLanguage, 'et');
  // CIT is on distributions only — no annual CIT return
  assert.deepEqual(p.compliance.filingTypes.map((ft) => ft.deadlineRule), ['ee-vat-monthly', 'ee-annual-accounts']);
});

test('EE: init --country EE creates an Estonian company (language defaults to et)', () => {
  const dbPath = tmpDb();
  const r = cli(dbPath, ['init', '--name', 'Test OÜ', '--country', 'EE', '--legal-form', 'ou', '--vat', 'on']);
  assert.equal(r.out.data.company.country, 'EE');
  const db = openDb(dbPath);
  try {
    const accounts = db.prepare('SELECT code, name FROM accounts WHERE active = 1').all();
    assert.ok(accounts.some((a) => a.code === '1000' && a.name === 'Arvelduskonto'));
    assert.ok(accounts.some((a) => a.code === '1510' && a.name === 'Sisendkäibemaks'));
    assert.ok(accounts.some((a) => a.code === '1520' && a.name === 'Väljundkäibemaks'));
    const c = createContact(db, { name: 'Ostja OÜ' });
    const inv = createInvoice(db, { contactId: c.id, lines: ['Teenus @ 10.00'], date: '2026-08-10', actor: 'agent:test' });
    assert.equal(inv.language, 'et'); // EE documents (full i18n table)
  } finally {
    db.close();
  }
});

test('LV: getProfile returns the LV profile (EUR, lv, standard kontu plāns)', () => {
  const p = getProfile('LV');
  assert.equal(p.meta.country, 'LV');
  assert.equal(p.meta.baseCurrency, 'EUR');
  assert.ok(p.meta.legalForms.includes('sia'));
  assert.equal(p.identifiers.peppolSchemeId, '9939'); // LV VAT EAS
  assert.ok(p.identifiers.vatIdFormat.test('LV12345678901'));
  assert.equal(p.tax.standardRateBp, 2100);
  assert.deepEqual(p.tax.codes.map((c) => c.code), ['21', '12', '5', '0', 'V', 'R', 'RE']);
  assert.deepEqual(p.tax.accounts.ledger.map((a) => a.code), ['1510', '1520']);
  assert.equal(p.reporting.debtorsAccount, '1200');
  assert.equal(p.closing.resultAccount, '2200');
  assert.equal(p.documents.invoiceCompliance, 'eu-invoice-vereisten');
  assert.equal(p.documents.defaultLanguage, 'lv');
  // CIT on distributions only — no annual CIT return
  assert.deepEqual(p.compliance.filingTypes.map((ft) => ft.deadlineRule), ['lv-vat-monthly', 'lv-annual-accounts']);
});

test('LV: init --country LV creates a Latvian company (language defaults to lv)', () => {
  const dbPath = tmpDb();
  const r = cli(dbPath, ['init', '--name', 'Test SIA', '--country', 'LV', '--legal-form', 'sia', '--vat', 'on']);
  assert.equal(r.out.data.company.country, 'LV');
  const db = openDb(dbPath);
  try {
    const accounts = db.prepare('SELECT code, name FROM accounts WHERE active = 1').all();
    assert.ok(accounts.some((a) => a.code === '1000' && a.name === 'Norēķinu konti bankā'));
    assert.ok(accounts.some((a) => a.code === '1510' && a.name === 'Priekšnodoklis'));
    assert.ok(accounts.some((a) => a.code === '1520' && a.name === 'PVN budžetā'));
    const c = createContact(db, { name: 'Pircējs SIA' });
    const inv = createInvoice(db, { contactId: c.id, lines: ['Pakalpojums @ 10.00'], date: '2026-08-10', actor: 'agent:test' });
    assert.equal(inv.language, 'lv'); // LV documents (full i18n table)
  } finally {
    db.close();
  }
});

test('LT: getProfile returns the LT profile (EUR, lt, Įmonių sąskaitų planas)', () => {
  const p = getProfile('LT');
  assert.equal(p.meta.country, 'LT');
  assert.equal(p.meta.baseCurrency, 'EUR');
  assert.ok(p.meta.legalForms.includes('uab'));
  assert.equal(p.identifiers.peppolSchemeId, '9937'); // LT VAT EAS
  assert.ok(p.identifiers.vatIdFormat.test('LT123456789'));
  assert.equal(p.tax.standardRateBp, 2100);
  assert.deepEqual(p.tax.codes.map((c) => c.code), ['21', '9', '5', '0', 'V', 'R', 'RE']);
  assert.deepEqual(p.tax.accounts.ledger.map((a) => a.code), ['1500', '1510']);
  assert.equal(p.reporting.debtorsAccount, '1200');
  assert.equal(p.closing.resultAccount, '2200');
  assert.equal(p.documents.invoiceCompliance, 'eu-invoice-vereisten');
  assert.equal(p.documents.defaultLanguage, 'lt');
  assert.deepEqual(p.compliance.filingTypes.map((ft) => ft.deadlineRule), ['lt-vat-monthly', 'lt-annual-accounts', 'lt-cit']);
});

test('LT: init --country LT creates a Lithuanian company (language defaults to lt)', () => {
  const dbPath = tmpDb();
  const r = cli(dbPath, ['init', '--name', 'Test UAB', '--country', 'LT', '--legal-form', 'uab', '--vat', 'on']);
  assert.equal(r.out.data.company.country, 'LT');
  const db = openDb(dbPath);
  try {
    const accounts = db.prepare('SELECT code, name FROM accounts WHERE active = 1').all();
    assert.ok(accounts.some((a) => a.code === '1000' && a.name === 'Pinigai banko sąskaitoje'));
    assert.ok(accounts.some((a) => a.code === '1500' && a.name === 'Pirkimo PVM'));
    assert.ok(accounts.some((a) => a.code === '1510' && a.name === 'Pardavimo PVM'));
    const c = createContact(db, { name: 'Pirkėjas UAB' });
    const inv = createInvoice(db, { contactId: c.id, lines: ['Paslauga @ 10.00'], date: '2026-08-10', actor: 'agent:test' });
    assert.equal(inv.language, 'lt'); // LT documents (full i18n table)
  } finally {
    db.close();
  }
});

test('MT: getProfile returns the MT profile (EUR, mt, convention chart)', () => {
  const p = getProfile('MT');
  assert.equal(p.meta.country, 'MT');
  assert.equal(p.meta.baseCurrency, 'EUR');
  assert.ok(p.meta.legalForms.includes('ltd'));
  assert.equal(p.identifiers.peppolSchemeId, '9943'); // MT VAT EAS
  assert.ok(p.identifiers.vatIdFormat.test('MT12345678'));
  assert.equal(p.tax.standardRateBp, 1800);
  assert.deepEqual(p.tax.codes.map((c) => c.code), ['18', '12', '7', '5', '0', 'V', 'R', 'RE']);
  assert.deepEqual(p.tax.accounts.ledger.map((a) => a.code), ['2410', '2420']);
  assert.equal(p.reporting.debtorsAccount, '1100');
  assert.equal(p.closing.resultAccount, '3300');
  assert.equal(p.closing.equityAccount, '3200');
  assert.equal(p.documents.invoiceCompliance, 'eu-invoice-vereisten');
  assert.equal(p.documents.defaultLanguage, 'mt');
  assert.deepEqual(p.compliance.filingTypes.map((ft) => ft.deadlineRule), ['mt-vat-quarterly', 'mt-annual-accounts', 'mt-cit']);
});

test('MT: init --country MT creates a Maltese company (language defaults to mt)', () => {
  const dbPath = tmpDb();
  const r = cli(dbPath, ['init', '--name', 'Test Ltd', '--country', 'MT', '--legal-form', 'ltd', '--vat', 'on']);
  assert.equal(r.out.data.company.country, 'MT');
  const db = openDb(dbPath);
  try {
    const accounts = db.prepare('SELECT code, name FROM accounts WHERE active = 1').all();
    assert.ok(accounts.some((a) => a.code === '1000' && a.name === 'Bank — current account'));
    assert.ok(accounts.some((a) => a.code === '2410' && a.name === 'VAT input (on purchases)'));
    assert.ok(accounts.some((a) => a.code === '2420' && a.name === 'VAT output (on sales)'));
    const c = createContact(db, { name: 'Client Ltd' });
    const inv = createInvoice(db, { contactId: c.id, lines: ['Service @ 10.00'], date: '2026-08-10', actor: 'agent:test' });
    assert.equal(inv.language, 'mt'); // MT documents (full i18n table)
  } finally {
    db.close();
  }
});

test('CY: getProfile returns the CY profile (EUR, cy, convention chart)', () => {
  const p = getProfile('CY');
  assert.equal(p.meta.country, 'CY');
  assert.equal(p.meta.baseCurrency, 'EUR');
  assert.ok(p.meta.legalForms.includes('ltd'));
  assert.equal(p.identifiers.peppolSchemeId, '9928'); // CY VAT EAS
  assert.ok(p.identifiers.vatIdFormat.test('CY12345678X'));
  assert.equal(p.tax.standardRateBp, 1900);
  assert.deepEqual(p.tax.codes.map((c) => c.code), ['19', '9', '5', '3', '0', 'V', 'R', 'RE']);
  assert.deepEqual(p.tax.accounts.ledger.map((a) => a.code), ['2410', '2420']);
  assert.equal(p.reporting.debtorsAccount, '1100');
  assert.equal(p.closing.resultAccount, '3300');
  assert.equal(p.closing.equityAccount, '3200');
  assert.equal(p.documents.invoiceCompliance, 'eu-invoice-vereisten');
  assert.equal(p.documents.defaultLanguage, 'cy');
  assert.deepEqual(p.compliance.filingTypes.map((ft) => ft.deadlineRule), ['cy-vat-quarterly', 'cy-annual-accounts', 'cy-td4']);
});

test('CY: init --country CY creates a Cypriot company (language defaults to cy)', () => {
  const dbPath = tmpDb();
  const r = cli(dbPath, ['init', '--name', 'Test Ltd', '--country', 'CY', '--legal-form', 'ltd', '--vat', 'on']);
  assert.equal(r.out.data.company.country, 'CY');
  const db = openDb(dbPath);
  try {
    const accounts = db.prepare('SELECT code, name FROM accounts WHERE active = 1').all();
    assert.ok(accounts.some((a) => a.code === '1000' && a.name === 'Bank — current account'));
    assert.ok(accounts.some((a) => a.code === '2410' && a.name === 'VAT input (on purchases)'));
    assert.ok(accounts.some((a) => a.code === '2420' && a.name === 'VAT output (on sales)'));
    const c = createContact(db, { name: 'Client Ltd' });
    const inv = createInvoice(db, { contactId: c.id, lines: ['Service @ 10.00'], date: '2026-08-10', actor: 'agent:test' });
    assert.equal(inv.language, 'cy'); // CY documents (full i18n table)
  } finally {
    db.close();
  }
});

test('CZ: getProfile returns the CZ profile (CZK, cz, směrná účtová osnova)', () => {
  const p = getProfile('CZ');
  assert.equal(p.meta.country, 'CZ');
  assert.equal(p.meta.baseCurrency, 'CZK'); // koruna — per-profile baseCurrency
  assert.ok(p.meta.legalForms.includes('sro'));
  assert.equal(p.identifiers.peppolSchemeId, '9929'); // CZ VAT EAS
  assert.ok(p.identifiers.vatIdFormat.test('CZ12345678'));
  assert.equal(p.tax.standardRateBp, 2100);
  assert.deepEqual(p.tax.codes.map((c) => c.code), ['21', '12', '0', 'V', 'R', 'RE']);
  assert.deepEqual(p.tax.accounts.ledger.map((a) => a.code), ['3431', '3432']);
  assert.equal(p.tax.accounts.fileDefault, '3433');
  assert.equal(p.reporting.debtorsAccount, '3110');
  assert.equal(p.closing.resultAccount, '4310');
  assert.equal(p.documents.invoiceCompliance, 'eu-invoice-vereisten');
  assert.equal(p.documents.defaultLanguage, 'cs');
  assert.deepEqual(p.compliance.filingTypes.map((ft) => ft.deadlineRule), ['cz-vat-monthly', 'cz-annual-accounts', 'cz-cit']);
});

test('CZ: init --country CZ creates a Czech company (language defaults to cs)', () => {
  const dbPath = tmpDb();
  const r = cli(dbPath, ['init', '--name', 'Test s.r.o.', '--country', 'CZ', '--legal-form', 'sro', '--vat', 'on']);
  assert.equal(r.out.data.company.country, 'CZ');
  assert.equal(r.out.data.company.base_currency, 'CZK');
  const db = openDb(dbPath);
  try {
    const accounts = db.prepare('SELECT code, name FROM accounts WHERE active = 1').all();
    assert.ok(accounts.some((a) => a.code === '2210' && a.name === 'Bankovní účty'));
    assert.ok(accounts.some((a) => a.code === '3431' && a.name === 'DPH na vstupu'));
    assert.ok(accounts.some((a) => a.code === '3432' && a.name === 'DPH na výstupu'));
    const c = createContact(db, { name: 'Odběratel s.r.o.' });
    const inv = createInvoice(db, { contactId: c.id, lines: ['Služba @ 10.00'], date: '2026-08-10', actor: 'agent:test' });
    assert.equal(inv.language, 'cs'); // CZ documents (full i18n table)
  } finally {
    db.close();
  }
});

test('SK: getProfile returns the SK profile (EUR, sk, směrná účtová osnova)', () => {
  const p = getProfile('SK');
  assert.equal(p.meta.country, 'SK');
  assert.equal(p.meta.baseCurrency, 'EUR');
  assert.ok(p.meta.legalForms.includes('sro'));
  assert.equal(p.identifiers.peppolSchemeId, '9950'); // SK VAT EAS
  assert.ok(p.identifiers.vatIdFormat.test('SK1234567890'));
  assert.equal(p.tax.standardRateBp, 2300);
  assert.deepEqual(p.tax.codes.map((c) => c.code), ['23', '19', '5', '0', 'V', 'R', 'RE']);
  assert.deepEqual(p.tax.accounts.ledger.map((a) => a.code), ['3431', '3432']);
  assert.equal(p.reporting.debtorsAccount, '3110');
  assert.equal(p.closing.resultAccount, '4310');
  assert.equal(p.documents.invoiceCompliance, 'eu-invoice-vereisten');
  assert.equal(p.documents.defaultLanguage, 'sk');
  assert.deepEqual(p.compliance.filingTypes.map((ft) => ft.deadlineRule), ['sk-vat-monthly', 'sk-annual-accounts', 'sk-cit']);
});

test('SK: init --country SK creates a Slovak company (language defaults to sk)', () => {
  const dbPath = tmpDb();
  const r = cli(dbPath, ['init', '--name', 'Test s.r.o.', '--country', 'SK', '--legal-form', 'sro', '--vat', 'on']);
  assert.equal(r.out.data.company.country, 'SK');
  const db = openDb(dbPath);
  try {
    const accounts = db.prepare('SELECT code, name FROM accounts WHERE active = 1').all();
    assert.ok(accounts.some((a) => a.code === '2210' && a.name === 'Bankové účty'));
    assert.ok(accounts.some((a) => a.code === '3431' && a.name === 'DPH na vstupe'));
    const c = createContact(db, { name: 'Odberateľ s.r.o.' });
    const inv = createInvoice(db, { contactId: c.id, lines: ['Služba @ 10.00'], date: '2026-08-10', actor: 'agent:test' });
    assert.equal(inv.language, 'sk'); // SK documents (full i18n table)
  } finally {
    db.close();
  }
});

test('GR: getProfile returns the GR profile (EUR, gr, ΕΓΛΣ chart; EL prefix)', () => {
  const p = getProfile('GR');
  assert.equal(p.meta.country, 'GR');
  assert.equal(p.meta.baseCurrency, 'EUR');
  assert.ok(p.meta.legalForms.includes('ike'));
  assert.equal(p.identifiers.peppolSchemeId, '9933'); // GR VAT EAS
  assert.ok(p.identifiers.vatIdFormat.test('EL123456789')); // EL, not GR
  assert.ok(!p.identifiers.vatIdFormat.test('GR123456789'));
  assert.equal(p.tax.standardRateBp, 2400);
  assert.deepEqual(p.tax.codes.map((c) => c.code), ['24', '13', '6', '0', 'V', 'R', 'RE']);
  assert.deepEqual(p.tax.accounts.ledger.map((a) => a.code), ['5400', '5450']);
  assert.equal(p.reporting.debtorsAccount, '3000');
  assert.equal(p.closing.resultAccount, '4300');
  assert.equal(p.documents.invoiceCompliance, 'eu-invoice-vereisten');
  assert.equal(p.documents.defaultLanguage, 'el');
  assert.deepEqual(p.compliance.filingTypes.map((ft) => ft.deadlineRule), ['gr-vat-monthly', 'gr-annual-accounts', 'gr-cit']);
});

test('GR: init --country GR creates a Greek company (language defaults to el)', () => {
  const dbPath = tmpDb();
  const r = cli(dbPath, ['init', '--name', 'Test IKE', '--country', 'GR', '--legal-form', 'ike', '--vat', 'on']);
  assert.equal(r.out.data.company.country, 'GR');
  const db = openDb(dbPath);
  try {
    const accounts = db.prepare('SELECT code, name FROM accounts WHERE active = 1').all();
    assert.ok(accounts.some((a) => a.code === '3802' && a.name === 'Τράπεζες'));
    assert.ok(accounts.some((a) => a.code === '5400' && a.name === 'ΦΠΑ εισροών'));
    assert.ok(accounts.some((a) => a.code === '5450' && a.name === 'ΦΠΑ εκροών'));
    const c = createContact(db, { name: 'Πελάτης IKE' });
    const inv = createInvoice(db, { contactId: c.id, lines: ['Υπηρεσία @ 10.00'], date: '2026-08-10', actor: 'agent:test' });
    assert.equal(inv.language, 'el'); // GR documents (full i18n table)
  } finally {
    db.close();
  }
});

test('PL: getProfile returns the PL profile (PLN, pl, Rozporządzenie MF chart)', () => {
  const p = getProfile('PL');
  assert.equal(p.meta.country, 'PL');
  assert.equal(p.meta.baseCurrency, 'PLN'); // złoty — per-profile baseCurrency
  assert.ok(p.meta.legalForms.includes('sp-zoo'));
  assert.equal(p.identifiers.peppolSchemeId, '9945'); // PL VAT EAS
  assert.ok(p.identifiers.vatIdFormat.test('PL1234567890'));
  assert.equal(p.tax.standardRateBp, 2300);
  assert.deepEqual(p.tax.codes.map((c) => c.code), ['23', '8', '5', '0', 'V', 'R', 'RE']);
  assert.deepEqual(p.tax.accounts.ledger.map((a) => a.code), ['2210', '2220']);
  assert.equal(p.tax.accounts.fileDefault, '2230');
  assert.equal(p.reporting.debtorsAccount, '2010');
  assert.equal(p.closing.resultAccount, '4030');
  assert.equal(p.documents.invoiceCompliance, 'eu-invoice-vereisten');
  assert.equal(p.documents.defaultLanguage, 'pl');
  assert.deepEqual(p.compliance.filingTypes.map((ft) => ft.deadlineRule), ['pl-vat-monthly', 'pl-annual-accounts', 'pl-cit']);
});

test('PL: init --country PL creates a Polish company (language defaults to pl)', () => {
  const dbPath = tmpDb();
  const r = cli(dbPath, ['init', '--name', 'Test sp. z o.o.', '--country', 'PL', '--legal-form', 'sp-zoo', '--vat', 'on']);
  assert.equal(r.out.data.company.country, 'PL');
  assert.equal(r.out.data.company.base_currency, 'PLN');
  const db = openDb(dbPath);
  try {
    const accounts = db.prepare('SELECT code, name FROM accounts WHERE active = 1').all();
    assert.ok(accounts.some((a) => a.code === '1310' && a.name === 'Rachunek bankowy'));
    assert.ok(accounts.some((a) => a.code === '2210' && a.name === 'VAT naliczony'));
    assert.ok(accounts.some((a) => a.code === '2220' && a.name === 'VAT należny'));
    const c = createContact(db, { name: 'Odbiorca sp. z o.o.' });
    const inv = createInvoice(db, { contactId: c.id, lines: ['Usługa @ 10.00'], date: '2026-08-10', actor: 'agent:test' });
    assert.equal(inv.language, 'pl'); // PL documents (full i18n table)
  } finally {
    db.close();
  }
});

test('HU: getProfile returns the HU profile (HUF, hu, Szt. chart)', () => {
  const p = getProfile('HU');
  assert.equal(p.meta.country, 'HU');
  assert.equal(p.meta.baseCurrency, 'HUF'); // forint — per-profile baseCurrency
  assert.ok(p.meta.legalForms.includes('kft'));
  assert.equal(p.identifiers.peppolSchemeId, '9910'); // HU VAT EAS
  assert.ok(p.identifiers.vatIdFormat.test('HU12345678'));
  assert.equal(p.tax.standardRateBp, 2700); // the EU's highest
  assert.deepEqual(p.tax.codes.map((c) => c.code), ['27', '18', '5', '0', 'V', 'R', 'RE']);
  assert.deepEqual(p.tax.accounts.ledger.map((a) => a.code), ['4660', '4670']);
  assert.equal(p.tax.accounts.fileDefault, '4680');
  assert.equal(p.reporting.debtorsAccount, '3110');
  assert.equal(p.closing.resultAccount, '4190');
  assert.equal(p.closing.equityAccount, '4130');
  assert.equal(p.documents.invoiceCompliance, 'eu-invoice-vereisten');
  assert.equal(p.documents.defaultLanguage, 'hu');
  assert.deepEqual(p.compliance.filingTypes.map((ft) => ft.deadlineRule), ['hu-vat-monthly', 'hu-annual-accounts', 'hu-cit']);
});

test('HU: init --country HU creates a Hungarian company (language defaults to hu)', () => {
  const dbPath = tmpDb();
  const r = cli(dbPath, ['init', '--name', 'Test Kft.', '--country', 'HU', '--legal-form', 'kft', '--vat', 'on']);
  assert.equal(r.out.data.company.country, 'HU');
  assert.equal(r.out.data.company.base_currency, 'HUF');
  const db = openDb(dbPath);
  try {
    const accounts = db.prepare('SELECT code, name FROM accounts WHERE active = 1').all();
    assert.ok(accounts.some((a) => a.code === '3810' && a.name === 'Bank'));
    assert.ok(accounts.some((a) => a.code === '4660' && a.name === 'Előzetesen felszámított áfa'));
    assert.ok(accounts.some((a) => a.code === '4670' && a.name === 'Fizetendő áfa'));
    const c = createContact(db, { name: 'Vevő Kft.' });
    const inv = createInvoice(db, { contactId: c.id, lines: ['Szolgáltatás @ 10.00'], date: '2026-08-10', actor: 'agent:test' });
    assert.equal(inv.language, 'hu'); // HU documents (full i18n table)
  } finally {
    db.close();
  }
});

test('RO: getProfile returns the RO profile (RON, ro, Planul de conturi)', () => {
  const p = getProfile('RO');
  assert.equal(p.meta.country, 'RO');
  assert.equal(p.meta.baseCurrency, 'RON'); // leu — per-profile baseCurrency
  assert.ok(p.meta.legalForms.includes('srl'));
  assert.equal(p.identifiers.peppolSchemeId, '9947'); // RO VAT EAS (cross-border ref; RO is not a Peppol participant)
  assert.ok(p.identifiers.vatIdFormat.test('RO12345678'));
  assert.equal(p.tax.standardRateBp, 1800); // cut from 19% on 1 Jan 2026
  assert.deepEqual(p.tax.codes.map((c) => c.code), ['18', '9', '0', 'V', 'R', 'RE']);
  assert.deepEqual(p.tax.accounts.ledger.map((a) => a.code), ['4424', '4423']);
  assert.equal(p.tax.accounts.fileDefault, '4426');
  assert.equal(p.reporting.debtorsAccount, '4111');
  assert.equal(p.closing.resultAccount, '1211');
  assert.equal(p.closing.equityAccount, '1171');
  assert.equal(p.documents.invoiceCompliance, 'eu-invoice-vereisten');
  assert.equal(p.documents.defaultLanguage, 'ro');
  assert.deepEqual(p.compliance.filingTypes.map((ft) => ft.deadlineRule), ['ro-vat-monthly', 'ro-annual-accounts', 'ro-cit']);
});

test('RO: init --country RO creates a Romanian company (language defaults to ro)', () => {
  const dbPath = tmpDb();
  const r = cli(dbPath, ['init', '--name', 'Test SRL', '--country', 'RO', '--legal-form', 'srl', '--vat', 'on']);
  assert.equal(r.out.data.company.country, 'RO');
  assert.equal(r.out.data.company.base_currency, 'RON');
  const db = openDb(dbPath);
  try {
    const accounts = db.prepare('SELECT code, name FROM accounts WHERE active = 1').all();
    assert.ok(accounts.some((a) => a.code === '5121' && a.name === 'Conturi la bănci'));
    assert.ok(accounts.some((a) => a.code === '4424' && a.name === 'TVA de recuperat'));
    assert.ok(accounts.some((a) => a.code === '4423' && a.name === 'TVA de plată'));
    const c = createContact(db, { name: 'Client SRL' });
    const inv = createInvoice(db, { contactId: c.id, lines: ['Serviciu @ 10.00'], date: '2026-08-10', actor: 'agent:test' });
    assert.equal(inv.language, 'ro'); // RO documents (full i18n table)
  } finally {
    db.close();
  }
});

test('XK: getProfile returns the XK profile (EUR, sq, SKRFI convention chart)', () => {
  const p = getProfile('XK');
  assert.equal(p.meta.country, 'XK');
  assert.equal(p.meta.baseCurrency, 'EUR'); // unilateral euro adoption
  assert.equal(p.meta.locale, 'sq');
  assert.deepEqual(p.meta.legalForms, ['shpk', 'sha', 'op', 'kp', 'bi']);
  assert.equal(p.identifiers.vatIdLabel, 'Numri i TVSH-së');
  assert.match('K12345678', p.identifiers.vatIdFormat); // 'K'-prefixed VAT number
  assert.equal(p.identifiers.peppolSchemeId, null); // not a Peppol participant
  assert.deepEqual(p.tax.codes.map((c) => c.code), ['18', '8', '0', 'V', 'R', 'RE']);
  assert.equal(p.tax.codes[0].rateBp, 1800); // 18% standard
  assert.deepEqual(p.tax.accounts.ledger.map((a) => a.code), ['2210', '2220']);
  assert.equal(p.tax.accounts.fileDefault, '2230');
  assert.equal(p.documents.invoiceCompliance, 'eu-invoice-vereisten'); // non-EU baseline
  assert.equal(p.documents.eInvoicing, 'peppol-bis-3.0'); // cross-border EN 16931 only
  assert.deepEqual(p.documents.languages, ['sq']);
  assert.equal(p.documents.defaultLanguage, 'sq'); // Albanian documents (full i18n table)
  assert.deepEqual(p.compliance.filingTypes.map((f) => f.deadlineRule),
    ['xk-vat-monthly', 'xk-annual-accounts', 'xk-cit']);
});

test('XK: init --country XK creates a Kosovar company (language defaults to sq)', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Test Shpk', '--country', 'XK', '--legal-form', 'shpk', '--vat', 'on',
    '--registration-id', '81234567', '--tax-id', 'K12345678', '--address', 'Rr. 1', '--postal-code', '10000', '--city', 'Prishtina']);
  const db = openDb(dbPath);
  try {
    const c = createContact(db, { name: 'Blerësi Test' });
    const inv = createInvoice(db, { contactId: c.id, lines: ['Shërbim @ 10.00'], date: '2026-08-10', actor: 'agent:test' });
    assert.equal(inv.language, 'sq'); // Albanian documents (full i18n table)
    const p = getProfile('XK');
    assert.ok(p.reporting.defaultChart.some((a) => a.code === '4010' && a.name === 'Të hyrat nga shitjet'));
  } finally {
    db.close();
  }
});
