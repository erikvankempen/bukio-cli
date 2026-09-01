/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across thirty-one jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Migration 021 tests (Phase A, M2) — jurisdiction-profile schema.
// Builds a scratch DB at user_version 20 (applies migrations 001–020
// verbatim), seeds NL-shaped data, then runs the real migration runner
// (openDb) and asserts the upgrade path: column renames, new columns,
// CHECK removals, taxonomy backfill, user_version.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, readdirSync, readFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import Database from 'better-sqlite3';
import { openDb } from '../src/core/db.js';

const MIGRATIONS_DIR = path.join(import.meta.dirname, '..', 'migrations');

function migrationFiles() {
  return readdirSync(MIGRATIONS_DIR)
    .filter((f) => /^\d+_.*\.sql$/.test(f))
    .sort((a, b) => parseInt(a, 10) - parseInt(b, 10));
}

/** Build a scratch DB at user_version 20 with NL-shaped seed data. */
function buildDbAt020() {
  const dir = mkdtempSync(path.join(tmpdir(), 'bukio-mig021-'));
  const file = path.join(dir, 'test.db');
  const db = new Database(file);
  for (const f of migrationFiles()) {
    const v = parseInt(f.split('_')[0], 10);
    if (v > 20) continue;
    db.exec(readFileSync(path.join(MIGRATIONS_DIR, f), 'utf8'));
  }
  db.pragma('user_version = 20');
  db.prepare(
    "INSERT INTO company (name, kvk, legal_form, btw_id) VALUES ('Test BV', '12345678', 'bv', 'NL123456789B01')",
  ).run();
  db.prepare(
    "INSERT INTO accounts (code, name, type, normal_balance, rgs_code) VALUES ('8000', 'Omzet', 'income', 'credit', 'WOMZ.80')",
  ).run();
  // a codeless account: the taxonomy backfill must cover it too (uniform
  // with createAccount's unconditional 'rgs' insert)
  db.prepare(
    "INSERT INTO accounts (code, name, type, normal_balance) VALUES ('1990', 'Ongenummerd', 'asset', 'debit')",
  ).run();
  db.prepare("INSERT INTO vat_returns (type, period, status) VALUES ('OB', '2026-Q2', 'draft')").run();
  db.prepare("INSERT INTO filings (type, period, filed_at) VALUES ('JAARREKENING', '2025', '2026-01-01')").run();
  db.close();
  return file;
}

function columnNames(db, table) {
  return db.prepare(`PRAGMA table_info(${table})`).all().map((c) => c.name);
}

test('migrations 021-025 upgrade a 020 DB: new columns, CHECK removals, renames, backfill', () => {
  const file = buildDbAt020();
  const db = openDb(file); // runs the real migration runner (001 → 025)
  try {
    assert.equal(db.pragma('user_version', { simple: true }), 26); // 021-026 chain (026 adds cost_centers)

    // company: renamed identifier columns + jurisdiction columns + CHECK gone
    const cols = columnNames(db, 'company');
    assert.ok(cols.includes('registration_id'));
    assert.ok(cols.includes('tax_id'));
    assert.ok(cols.includes('country'));
    assert.ok(cols.includes('base_currency'));
    assert.ok(cols.includes('locale'));
    assert.ok(cols.includes('profile_version'));
    assert.ok(!cols.includes('kvk'));
    assert.ok(!cols.includes('btw_id'));

    const c = db.prepare('SELECT * FROM company WHERE id = 1').get();
    assert.equal(c.registration_id, '12345678');
    assert.equal(c.tax_id, 'NL123456789B01');
    assert.equal(c.legal_form, 'bv');
    assert.equal(c.country, 'NL');
    assert.equal(c.base_currency, 'EUR');
    assert.equal(c.locale, 'nl');
    assert.equal(c.profile_version, 1);

    // legal_form CHECK is gone: a non-NL value is accepted
    db.prepare("UPDATE company SET legal_form = 'ltd' WHERE id = 1").run();
    assert.equal(db.prepare('SELECT legal_form FROM company WHERE id = 1').get().legal_form, 'ltd');

    // accounts: rgs_code -> taxonomy_code rename + taxonomy backfill
    const aCols = columnNames(db, 'accounts');
    assert.ok(aCols.includes('taxonomy_code'));
    assert.ok(aCols.includes('taxonomy'));
    assert.ok(!aCols.includes('rgs_code'));
    const acc = db.prepare("SELECT taxonomy_code, taxonomy FROM accounts WHERE code = '8000'").get();
    assert.equal(acc.taxonomy_code, 'WOMZ.80');
    assert.equal(acc.taxonomy, 'rgs');
    // uniform discriminator: codeless rows also get 'rgs' (same as createAccount)
    const uncoded = db.prepare("SELECT taxonomy FROM accounts WHERE taxonomy_code IS NULL LIMIT 1").get();
    assert.equal(uncoded.taxonomy, 'rgs');

    // vat_returns: type CHECK widened
    db.prepare("INSERT INTO vat_returns (type, period, status) VALUES ('VAT', '2026-Q2', 'draft')").run();
    assert.equal(
      db.prepare("SELECT COUNT(*) n FROM vat_returns WHERE type = 'VAT'").get().n,
      1,
    );
    assert.equal(db.prepare("SELECT period FROM vat_returns WHERE type = 'OB'").get().period, '2026-Q2');

    // filings: type CHECK widened
    db.prepare("INSERT INTO filings (type, period, filed_at) VALUES ('VAT', '2026-Q2', '2026-08-14')").run();
    assert.equal(db.prepare("SELECT COUNT(*) n FROM filings WHERE type = 'VAT'").get().n, 1);

    // postings untouched (decision §9.1.2: vat_code_id stays)
    const pCols = columnNames(db, 'postings');
    assert.ok(pCols.includes('vat_code_id'));
    assert.ok(pCols.includes('vat_amount_cents'));
  } finally {
    db.close();
  }
});

test('migration 021 keeps company data lossless across the rebuild', () => {
  const file = buildDbAt020();
  const db = openDb(file);
  try {
    const c = db.prepare('SELECT * FROM company WHERE id = 1').get();
    assert.equal(c.name, 'Test BV');
    assert.equal(c.iban, null);
    assert.equal(c.vat_module, 0);
    assert.equal(c.kor_flag, 0);
    assert.equal(c.fiscal_year_end, '12-31');
    assert.equal(c.logo, null);
    assert.ok(c.created_at);
    assert.ok(c.updated_at);
    // no duplicate rows (PK id=1 CHECK preserved)
    assert.equal(db.prepare('SELECT COUNT(*) n FROM company').get().n, 1);
  } finally {
    db.close();
  }
});
