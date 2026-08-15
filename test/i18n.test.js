/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// i18n mechanism (S1): resolveLocale precedence, t() fallbacks and
// interpolation, legacy label/unitLabel shims, and the localized vat file
// description (English default, Dutch when BUKIO_LOCALE=nl).
import { test, beforeEach } from 'node:test';
import assert from 'node:assert/strict';
import { openDb } from '../src/core/db.js';
import { seedDefaultChart } from '../src/core/accounts.js';
import {
  enableVatModule, bookVatEntry, parseVatPostingSpecs, vatFile,
} from '../src/vat/index.js';
import {
  t, resolveLocale, label, unitLabel, LABELS, UNITS, TABLES,
} from '../src/i18n/index.js';

let db;

function setup() {
  db = openDb(':memory:');
  seedDefaultChart(db);
  db.prepare(
    "INSERT INTO company (name, registration_id, legal_form, tax_id, iban, vat_module) VALUES ('Demo BV', '12345678', 'bv', 'NL123456789B01', 'NL91ABNA0417164300', 1)",
  ).run();
  enableVatModule(db);
}

beforeEach(setup);

test('t: missing locale falls back to English, missing key falls back to the key', () => {
  assert.equal(t('dir.payable', {}, 'fr'), 'payable'); // unknown locale -> en
  assert.equal(t('no.such.key', {}, 'nl'), 'no.such.key'); // unknown key -> key
  assert.equal(t('dir.payable', {}, 'nl'), 'te betalen'); // known nl
  assert.equal(t('dir.payable'), 'payable'); // default en
});

test('t: {param} interpolation', () => {
  assert.equal(
    t('vat.file.description', { period: ' 2026-Q3', account: '2510', direction: 'payable' }),
    'VAT return 2026-Q3 — transfer to 2510 (payable)',
  );
  assert.equal(
    t('vat.file.description', { period: ' 2026-Q3', account: '2510', direction: 'te betalen' }, 'nl'),
    'OB-aangifte 2026-Q3 verlegging naar 2510 (te betalen)',
  );
  assert.equal(
    t('email.reminderSubject', { number: '2026-001' }, 'nl'),
    'Betalingsherinnering factuur 2026-001',
  );
});

test('resolveLocale: flag > env > en (UI stays English unless opted in)', () => {
  assert.equal(resolveLocale({ locale: 'fr' }, db), 'fr'); // --locale flag wins
  assert.equal(resolveLocale({}, null), 'en'); // nothing set
  assert.equal(resolveLocale(null, db), 'en'); // null-safe, no opts
  db.prepare('UPDATE company SET locale = ? WHERE id = 1').run('nl');
  assert.equal(resolveLocale({}, db), 'en'); // company locale does NOT flip the UI
  process.env.BUKIO_LOCALE = 'de';
  try {
    assert.equal(resolveLocale({}, db), 'de'); // env opts in
    assert.equal(resolveLocale({ locale: 'fr' }, db), 'fr'); // flag beats env
  } finally {
    delete process.env.BUKIO_LOCALE;
  }
});

test('legacy shims: label/unitLabel/LABELS/UNITS keep the old API and values', () => {
  assert.equal(label('invoice', 'nl'), 'FACTUUR');
  assert.equal(label('invoice', 'en'), 'INVOICE');
  assert.equal(label('billedTo', 'nl'), 'Factuur aan');
  assert.equal(unitLabel('h', 'nl'), 'uur');
  assert.equal(unitLabel('day', 'en'), 'day');
  assert.equal(LABELS.nl.invoice, 'FACTUUR');
  assert.equal(UNITS.h.nl, 'uur');
  assert.equal(TABLES.en['pdf.invoice'], 'INVOICE');
});

function bookSale() {
  // 121.00 sale @ 21% -> 21.00 output VAT payable
  bookVatEntry(db, {
    date: '2026-07-01', description: 'Verkoop',
    postings: parseVatPostingSpecs(['1100:121.00,8000:-100.00@21']), post: true,
  });
}
const entryDesc = (id) => db.prepare('SELECT description FROM journal_entries WHERE id = ?').get(id).description;

test('vat file description: English by default', () => {
  bookSale();
  const r = vatFile(db, { period: '2026-Q3', actor: 'agent:test' });
  assert.equal(entryDesc(r.entry_id), 'VAT return 2026-Q3 — transfer to 2510 (payable)');
});

test('vat file description: Dutch when localized (locale: nl)', () => {
  bookSale();
  const r = vatFile(db, { period: '2026-Q3', actor: 'agent:test', locale: 'nl' });
  assert.equal(entryDesc(r.entry_id), 'OB-aangifte 2026-Q3 verlegging naar 2510 (te betalen)');
});

test('vat file description: --desc override still wins over localization', () => {
  bookVatEntry(db, {
    date: '2026-07-01', description: 'Verkoop',
    postings: parseVatPostingSpecs(['1100:121.00,8000:-100.00@21']), post: true,
  });
  const r = vatFile(db, { period: '2026-Q3', actor: 'agent:test', locale: 'nl', desc: 'custom' });
  assert.equal(db.prepare('SELECT description FROM journal_entries WHERE id = ?').get(r.entry_id).description, 'custom');
});
