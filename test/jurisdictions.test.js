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
  // Phase B expansion wave is planned (research/implementation in progress)
  assert.deepEqual([...PLANNED].sort(), ['BE', 'DE', 'DK', 'FI', 'NO', 'SE']);
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

test('B1: LU is implemented — PLANNED is the Phase B expansion wave', () => {
  assert.ok(!PLANNED.includes('LU'));
  assert.deepEqual([...PLANNED].sort(), ['BE', 'DE', 'DK', 'FI', 'NO', 'SE']);
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

test('GB: PLANNED is the Phase B expansion wave (BE/DE/DK/FI/NO/SE)', () => {
  assert.ok(!PLANNED.includes('GB'));
  assert.deepEqual([...PLANNED].sort(), ['BE', 'DE', 'DK', 'FI', 'NO', 'SE']);
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

test('FR: PLANNED is the Phase B expansion wave (BE/DE/DK/FI/NO/SE)', () => {
  assert.ok(!PLANNED.includes('FR'));
  assert.deepEqual([...PLANNED].sort(), ['BE', 'DE', 'DK', 'FI', 'NO', 'SE']);
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

test('US: PLANNED is the Phase B expansion wave (BE/DE/DK/FI/NO/SE)', () => {
  assert.deepEqual([...PLANNED].sort(), ['BE', 'DE', 'DK', 'FI', 'NO', 'SE']);
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
