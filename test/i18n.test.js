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
  assert.equal(t('dir.payable', {}, 'xx'), 'payable'); // unknown locale -> en
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

test('company show + balance-sheet labels localize (round-10 review keys)', () => {
  assert.equal(t('company.name', {}, 'nl'), 'naam');
  assert.equal(t('company.language', {}, 'nl'), 'taal');
  assert.equal(t('company.regId', {}, 'de'), 'Reg.-Nr.');
  assert.equal(t('report.totalAssets', {}, 'en'), 'total assets:');
  assert.equal(t('report.totalAssets', {}, 'nl'), 'totaal activa:');
  assert.equal(t('report.assets', {}, 'de'), 'AKTIVA');
  assert.equal(t('report.liabilities', {}, 'fr'), 'PASSIF');
});

test('reminders table labels localize fully (nl)', () => {
  assert.equal(t('invlist.dueDate', {}, 'nl'), 'vervaldatum');
  assert.equal(t('invlist.outstanding', {}, 'nl'), 'openstaand');
  assert.equal(t('invlist.dueDate', {}, 'de'), 'Fälligkeitsdatum');
});

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

test('locale normalization: de-DE -> de, en-GB -> en, nl-BE -> nl-be, fr-LU -> fr-lu', () => {
  assert.equal(t('dir.payable', {}, 'de-DE'), 'zu zahlen');
  assert.equal(t('dir.payable', {}, 'DE-DE'), 'zu zahlen'); // case-insensitive
  assert.equal(t('dir.payable', {}, 'en-GB'), 'payable'); // en-gb -> en
  assert.equal(t('dir.payable', {}, 'en-US'), 'payable');
  assert.equal(t('pdf.kvk', {}, 'nl-BE'), 'KBO'); // regional override first
  assert.equal(t('dir.payable', {}, 'nl-BE'), 'te betalen'); // ...then base language
  assert.equal(t('pdf.kvk', {}, 'fr-LU'), 'RCS'); // lu override
  assert.equal(t('pdf.kvk', {}, 'fr'), 'SIREN'); // fr base
});

test('per-locale spot checks: every market table resolves its own language', () => {
  const spots = {
    de: [['pdf.invoice', 'RECHNUNG'], ['status.overdue', 'überfällig'], ['report.revenue', 'Erlöse']],
    fr: [['pdf.invoice', 'FACTURE'], ['dir.receivable', 'à recevoir'], ['email.reminderSubject', 'Rappel de paiement — facture 2026-001']],
    da: [['pdf.invoice', 'FAKTURA'], ['status.paid', 'betalt'], ['vat.file.description', 'Momsangivelse 2026-Q3 — overførsel til 2510 (skyldig)']],
    fi: [['pdf.invoice', 'LASKU'], ['status.draft', 'luonnos'], ['dir.debit', 'debet']],
    nb: [['pdf.invoice', 'FAKTURA'], ['status.overdue', 'forfalt'], ['vat.settle.description', 'Betaling av mva-melding 2026-Q3 — 2510 (avrundingsdifferanse 0.01)']],
    sv: [['pdf.invoice', 'FAKTURA'], ['status.paid', 'betald'], ['report.netResult', 'nettoresultat']],
  };
  for (const [loc, checks] of Object.entries(spots)) {
    for (const [key, expected] of checks) {
      const actual = t(key, {
        period: ' 2026-Q3', account: '2510', direction: 'skyldig',
        number: '2026-001', amount: '0.01',
      }, loc);
      assert.equal(actual, expected, `${loc}.${key}`);
    }
  }
});

test('vat file description: --desc override still wins over localization', () => {
  bookVatEntry(db, {
    date: '2026-07-01', description: 'Verkoop',
    postings: parseVatPostingSpecs(['1100:121.00,8000:-100.00@21']), post: true,
  });
  const r = vatFile(db, { period: '2026-Q3', actor: 'agent:test', locale: 'nl', desc: 'custom' });
  assert.equal(db.prepare('SELECT description FROM journal_entries WHERE id = ?').get(r.entry_id).description, 'custom');
});
