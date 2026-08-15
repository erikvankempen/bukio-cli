/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
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
import { exportXaf } from '../src/export/index.js';
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
  assert.ok(PLANNED.includes('GB'));
  assert.ok(PLANNED.includes('US'));
  assert.ok(PLANNED.includes('FR'));
  // LU is implemented (Phase B1) — it must NOT be in PLANNED
  assert.ok(!PLANNED.includes('LU'));
});

test('getProfile throws PROFILE_NOT_FOUND for unknown valid codes', () => {
  assert.throws(() => getProfile('ZZ'), (e) => e.code === 'PROFILE_NOT_FOUND');
  assert.throws(() => getProfile('DE'), (e) => e.code === 'PROFILE_NOT_FOUND');
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
  assert.equal(p.tax.accounts.afTeDragenName, 'Af te dragen omzetbelasting');
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
  const dbGB = scratchDbAt(21, { sql: "INSERT INTO company (name, country) VALUES (?, ?)", params: ['Test BV', 'GB'] });
  try {
    assert.throws(() => resolveProfile(dbGB), (e) => e.code === 'COUNTRY_NOT_SUPPORTED');
  } finally {
    dbGB.close();
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

test('M3 init: --country GB is rejected with COUNTRY_NOT_SUPPORTED', () => {
  const dbPath = tmpDb();
  const r = cli(dbPath, ['init', '--name', 'Test BV', '--country', 'GB'], { expectFail: true });
  assert.equal(r.code, 1);
  assert.equal(r.out.error.code, 'COUNTRY_NOT_SUPPORTED');
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
  assert.equal(p.meta.locale, 'fr');
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
  assert.equal(p.reporting.format, undefined);
  assert.equal(p.documents.invoiceCompliance, undefined);
  assert.equal(p.documents.auditFile, undefined);
  assert.deepEqual(p.compliance.filingTypes, []);
  // e-invoicing IS registered: B2G mandatory in LU since 18 Mar 2023
  assert.equal(p.documents.eInvoicing, 'peppol-bis-3.0');
  assert.deepEqual(p.exchange.paymentFormats, ['sepa-pain.001', 'sepa-pain.008']);
});

test('B1: LU is implemented — PLANNED is now GB/US/FR only', () => {
  assert.ok(!PLANNED.includes('LU'));
  assert.deepEqual([...PLANNED].sort(), ['FR', 'GB', 'US']);
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
  assert.equal(r.out.data.company.locale, 'fr');
  const db = openDb(dbPath);
  try {
    const c = db.prepare('SELECT country, base_currency, locale, profile_version FROM company WHERE id = 1').get();
    assert.deepEqual(c, { country: 'LU', base_currency: 'EUR', locale: 'fr', profile_version: 1 });
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
  // financial statements: the LU LSC layout is B2 — never a Dutch one
  let r = cli(dbPath, ['financial-statements', 'report', '--year', '2026'], { expectFail: true });
  assert.equal(r.out.error.code, 'FORMAT_NOT_SUPPORTED');
  // XAF export: FAIA is B3 — never the Dutch audit file
  r = cli(dbPath, ['export', 'xaf', '--year', '2026', '--out', '/tmp/xaf-lu.xml'], { expectFail: true });
  assert.equal(r.out.error.code, 'FORMAT_NOT_SUPPORTED');
  // VAT readout: the LU eCDF return layout is a B-milestone
  r = cli(dbPath, ['vat', 'readout', '--period', '2026-Q1'], { expectFail: true });
  assert.equal(r.out.error.code, 'FORMAT_NOT_SUPPORTED');
  // invoice compliance: the LU rule set is B6
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

test('B1: LU UBL invoice emits the RCS scheme 0195 and country LU (never 9944)', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Sàrl Test', '--country', 'LU', '--legal-form', 'sarl', '--registration-id', 'B123456', '--vat', 'on']);
  const db = openDb(dbPath);
  try {
    createContact(db, {
      name: 'Client SARL', address: '1 rue du Test', postalCode: 'L-1234', city: 'Luxembourg',
      vatId: 'LU99999999', kvk: 'B654321', actor: 'agent:test',
    });
    // NOTE: LU invoices cannot be finalized in B1 (finalizeInvoice runs the
    // compliance rule set, which is deliberately unregistered until B6) —
    // UBL generation is tested on the created invoice, which is all the
    // builder needs (it reads company + invoice fields, not posting state)
    const inv = createInvoice(db, {
      contactId: 1, date: '2026-08-15', lines: ['1x Prestation @ 100.00 @17'], actor: 'agent:test',
    });
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
