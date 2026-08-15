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
import { complianceStatus, markFiled } from '../src/compliance/index.js';
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
  // all ten markets are implemented — PLANNED is empty
  assert.deepEqual([...PLANNED].sort(), []);
});

test('getProfile throws PROFILE_NOT_FOUND for unknown valid codes', () => {
  assert.throws(() => getProfile('ZZ'), (e) => e.code === 'PROFILE_NOT_FOUND');
  assert.throws(() => getProfile('LT'), (e) => e.code === 'PROFILE_NOT_FOUND');
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
  const dbLT = scratchDbAt(21, { sql: "INSERT INTO company (name, country) VALUES (?, ?)", params: ['Test BV', 'LT'] });
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

test('M3 init: --country LT (valid code, no profile) is rejected with PROFILE_NOT_FOUND', () => {
  const dbPath = tmpDb();
  const r = cli(dbPath, ['init', '--name', 'Test BV', '--country', 'LT'], { expectFail: true });
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

test('B1: LU is implemented — PLANNED is empty (all ten markets landed)', () => {
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
    // Q3 2026 is still open (deadline after today)
    assert.equal(obs.find((o) => o.type === 'TVA' && o.period === '2026-Q3').status, 'open');
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
  assert.equal(p.meta.locale, 'en-GB');
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
  assert.equal(p.documents.invoiceCompliance, undefined);
  assert.equal(p.documents.eInvoicing, undefined); // 2029 mandate, no Peppol scheme yet
  assert.equal(p.documents.auditFile, undefined);
  assert.deepEqual(p.exchange.paymentFormats, []); // SEPA is not a domestic rail
  // GB deadlines: annual accounts 9 months after FYE, CT600 12 months
  assert.deepEqual(p.compliance.filingTypes.map((ft) => ft.deadlineRule), ['gb-9-months', 'gb-ct600']);
});

test('GB: PLANNED is empty (all ten markets landed)', () => {
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
  assert.equal(r.out.data.company.locale, 'en-GB');
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
  assert.equal(p.documents.invoiceCompliance, undefined);
  assert.equal(p.documents.auditFile, undefined); // FEC is a B-milestone
  assert.deepEqual(p.compliance.filingTypes, []);
  // e-invoicing registered: EN 16931 UBL accepted in the FR mandate
  assert.equal(p.documents.eInvoicing, 'peppol-bis-3.0');
  assert.deepEqual(p.exchange.paymentFormats, ['sepa-pain.001', 'sepa-pain.008']);
});

test('FR: PLANNED is empty (all ten markets landed)', () => {
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
    assert.throws(
      () => validateCompliance(db, { invoice_type: 'invoice', lines: [] }),
      (e) => e.code === 'FORMAT_NOT_SUPPORTED',
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
  assert.equal(p.meta.locale, 'en-US');
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
  assert.equal(p.documents.invoiceCompliance, undefined);
  assert.equal(p.documents.eInvoicing, undefined);
  assert.equal(p.documents.auditFile, undefined);
  assert.deepEqual(p.exchange.bankStatementFormats, ['csv']); // CAMT.053 unverified
  assert.deepEqual(p.exchange.paymentFormats, []); // no SEPA; ACH is a B-milestone
  // US deadlines: 1120 (15th of 4th month after FYE) + 941 (quarterly)
  assert.deepEqual(p.compliance.filingTypes.map((ft) => ft.deadlineRule), ['us-1120', 'us-941']);
});

test('US: PLANNED is empty (all ten markets landed)', () => {
  assert.deepEqual([...PLANNED].sort(), []);
  assert.equal(getProfile('US').meta.country, 'US');
});

test('US: init --country US creates a USD company with the US chart', () => {
  const dbPath = tmpDb();
  const r = cli(dbPath, ['init', '--name', 'Acme LLC', '--country', 'US', '--legal-form', 'llc']);
  assert.equal(r.out.data.company.country, 'US');
  assert.equal(r.out.data.company.base_currency, 'USD');
  assert.equal(r.out.data.company.locale, 'en-US');
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
  assert.equal(p.documents.invoiceCompliance, undefined);
  assert.equal(p.documents.auditFile, undefined);
  assert.deepEqual(p.exchange.bankStatementFormats, ['camt.053', 'csv']); // CODA not registered
  assert.deepEqual(p.exchange.paymentFormats, ['sepa-pain.001', 'sepa-pain.008']);
  // e-invoicing: CONFIRMED mandatory B2B via Peppol since 1 Jan 2026
  assert.equal(p.documents.eInvoicing, 'peppol-bis-3.0');
  // BE deadlines: VAT monthly (20th), annual accounts 7 months
  assert.deepEqual(p.compliance.filingTypes.map((ft) => ft.deadlineRule), ['be-vat-monthly', 'be-7-months']);
});

test('BE: PLANNED is empty (all ten markets landed)', () => {
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
    assert.throws(
      () => validateCompliance(db, { invoice_type: 'invoice', lines: [] }),
      (e) => e.code === 'FORMAT_NOT_SUPPORTED',
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
  assert.equal(p.documents.invoiceCompliance, undefined);
  assert.equal(p.documents.auditFile, undefined);
  assert.deepEqual(p.exchange.paymentFormats, ['sepa-pain.001', 'sepa-pain.008']);
  // e-invoicing: EN 16931 accepted (Peppol BIS 3.0 UBL is one)
  assert.equal(p.documents.eInvoicing, 'peppol-bis-3.0');
  // DE deadlines: UStVA quarterly (10th), annual VAT (31 Jul), accounts (12 mo)
  assert.deepEqual(p.compliance.filingTypes.map((ft) => ft.deadlineRule), ['de-ustva-quarterly', 'de-annual-vat', 'de-12-months']);
});

test('DE: PLANNED is empty (all ten markets landed)', () => {
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
    assert.throws(
      () => validateCompliance(db, { invoice_type: 'invoice', lines: [] }),
      (e) => e.code === 'FORMAT_NOT_SUPPORTED',
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
  assert.equal(p.documents.invoiceCompliance, undefined);
  assert.equal(p.documents.auditFile, undefined); // DK SAF-T v2.0 (2027) is a B-milestone
  assert.deepEqual(p.exchange.paymentFormats, ['sepa-pain.001', 'sepa-pain.008']);
  assert.equal(p.documents.eInvoicing, 'peppol-bis-3.0'); // voluntary B2B
  // DK deadlines: quarterly VAT (1st of 3rd month), accounts 5 months
  assert.deepEqual(p.compliance.filingTypes.map((ft) => ft.deadlineRule), ['dk-quarterly', 'dk-5-months']);
});

test('DK: PLANNED is empty (all ten markets landed)', () => {
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
    assert.throws(
      () => validateCompliance(db, { invoice_type: 'invoice', lines: [] }),
      (e) => e.code === 'FORMAT_NOT_SUPPORTED',
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
  assert.ok(p.reporting.defaultChart.some((a) => a.code === '1910' && a.name === 'Pankkitili'));
  assert.ok(p.reporting.defaultChart.some((a) => a.code === '3000' && a.name === 'Myynti ALV 25,5%'));
  assert.ok(p.reporting.defaultChart.some((a) => a.code === '2350' && a.normalBalance === 'debit'));
  assert.ok(p.reporting.defaultChart.length >= 50);
  // B-milestones stay unregistered (strict dispatch fails loudly)
  assert.equal(p.tax.returnLayout, undefined);
  assert.equal(p.reporting.format, undefined);
  assert.equal(p.documents.invoiceCompliance, undefined);
  assert.equal(p.documents.auditFile, undefined);
  assert.deepEqual(p.exchange.paymentFormats, ['sepa-pain.001', 'sepa-pain.008']);
  // e-invoicing: no B2B mandate; Peppol BIS accepted voluntarily
  assert.equal(p.documents.eInvoicing, 'peppol-bis-3.0');
  // FI deadlines: quarterly VAT (12th of 2nd month), accounts filed in 8 months
  assert.deepEqual(p.compliance.filingTypes.map((ft) => ft.deadlineRule), ['fi-quarterly', 'fi-8-months']);
});

test('FI: PLANNED is empty (all ten markets landed)', () => {
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
    assert.ok(accounts.some((a) => a.code === '1910' && a.name === 'Pankkitili'));
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
    assert.throws(
      () => validateCompliance(db, { invoice_type: 'invoice', lines: [] }),
      (e) => e.code === 'FORMAT_NOT_SUPPORTED',
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
  assert.equal(p.documents.invoiceCompliance, undefined);
  assert.equal(p.documents.auditFile, undefined); // SAF-T Accounting is a B-milestone
  assert.deepEqual(p.exchange.paymentFormats, ['sepa-pain.001', 'sepa-pain.008']);
  // e-invoicing: EHF 3.0 = Peppol BIS 3.0 UBL profile
  assert.equal(p.documents.eInvoicing, 'peppol-bis-3.0');
  // NO deadlines: bi-monthly VAT + accounts 7 months
  assert.deepEqual(p.compliance.filingTypes.map((ft) => ft.deadlineRule), ['no-bimonthly', 'no-7-months']);
  assert.equal(p.compliance.filingTypes[0].periodShape, 'YYYY-Pn'); // bi-monthly shape
});

test('NO: PLANNED is empty (all ten markets landed)', () => {
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
    assert.throws(
      () => validateCompliance(db, { invoice_type: 'invoice', lines: [] }),
      (e) => e.code === 'FORMAT_NOT_SUPPORTED',
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
  assert.equal(p.documents.invoiceCompliance, undefined);
  assert.equal(p.documents.auditFile, undefined);
  assert.deepEqual(p.exchange.paymentFormats, ['sepa-pain.001', 'sepa-pain.008']);
  // e-invoicing: B2G mandatory via Peppol (Peppol BIS 3.0)
  assert.equal(p.documents.eInvoicing, 'peppol-bis-3.0');
  // SE deadlines: quarterly VAT (12th of 2nd month, Aug 17th), accounts 7 months
  assert.deepEqual(p.compliance.filingTypes.map((ft) => ft.deadlineRule), ['se-quarterly', 'se-7-months']);
});

test('SE: PLANNED is empty — every expansion market is implemented', () => {
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
    assert.throws(
      () => validateCompliance(db, { invoice_type: 'invoice', lines: [] }),
      (e) => e.code === 'FORMAT_NOT_SUPPORTED',
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
