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
import { mkdtempSync, readdirSync, readFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import Database from 'better-sqlite3';
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
  assert.ok(PLANNED.includes('LU'));
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
