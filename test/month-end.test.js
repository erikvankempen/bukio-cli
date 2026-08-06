import { test, beforeEach } from 'node:test';
import assert from 'node:assert/strict';
import { openDb } from '../src/core/db.js';
import { seedDefaultChart } from '../src/core/accounts.js';
import { createEntry, postEntry } from '../src/core/entries.js';
import { enableVatModule, bookVatEntry, parseVatPostingSpecs } from '../src/vat/index.js';
import { getOrCreateBankAccount, importTransactions } from '../src/bank/index.js';
import { createContact, createInvoice, finalizeInvoice } from '../src/invoice/index.js';
import { monthEnd } from '../src/month-end/index.js';

let db;

function setup() {
  db = openDb(':memory:');
  seedDefaultChart(db);
  db.prepare(`
    INSERT INTO company (name, kvk, legal_form, btw_id, iban, address, postal_code, city, vat_module)
    VALUES ('Demo BV', '12345678', 'bv', 'NL123456789B01', 'NL91ABNA0417164300', 'Industrieweg 12', '2712 CD', 'Zoetermeer', 1)
  `).run();
  enableVatModule(db);
}

beforeEach(() => {
  setup();
});

function post(date, description, postings) {
  const entry = createEntry(db, { date, description, postings, actor: 'agent:test' });
  return postEntry(db, { id: entry.id, actor: 'agent:test' });
}

test('month-end: clean month -> all clear, zero totals', () => {
  const r = monthEnd(db, { period: '2026-01' });
  assert.equal(r.period, '2026-01');
  assert.equal(r.from, '2026-01-01');
  assert.equal(r.to, '2026-01-31');
  assert.equal(r.entries.draft, 0);
  assert.equal(r.bank.unmatched, 0);
  assert.equal(r.invoices.overdue, 0);
  assert.equal(r.recurring.due, 0);
  assert.equal(r.totals.balanced, true);
  assert.equal(r.totals.profit_cents, 0);
  assert.deepEqual(r.warnings, ['all clear — the month can be closed']);
});

test('month-end: drafts and unmatched bank transactions are flagged', () => {
  createEntry(db, { date: '2026-01-10', description: 'Concept', postings: [{ code: '1100', amountCents: 100 }, { code: '3000', amountCents: -100 }], actor: 'agent:test' });
  importTransactions(db, {
    iban: 'NL91ABNA0417164300',
    transactions: [{
      date: '2026-01-15', amount_cents: 5000, counterparty: 'ACME', description: 'Betaling', iban_counter: 'NL02ABNA0123456789',
    }],
  });
  const r = monthEnd(db, { period: '2026-01' });
  assert.equal(r.entries.draft, 1);
  assert.equal(r.bank.unmatched, 1);
  assert.ok(r.warnings.some((w) => w.includes('1 draft entry')));
  assert.ok(r.warnings.some((w) => w.includes('1 unmatched bank transaction')));
});

test('month-end: VAT quarter readout when module on', () => {
  bookVatEntry(db, {
    date: '2026-01-20', description: 'Verkoop',
    postings: parseVatPostingSpecs(['1100:121.00,8000:-100.00@21']),
    actor: 'agent:test', post: true,
  });
  const r = monthEnd(db, { period: '2026-01' });
  assert.ok(r.vat, 'vat readout expected');
  assert.equal(r.vat.quarter, '2026-Q1');
  assert.equal(r.vat.to_pay_cents, 2100);
  assert.equal(r.vat.fields['1a'], 10000);
});

test('month-end: profit = income - expense for the period', () => {
  post('2026-01-05', 'Verkoop', [{ code: '1100', amountCents: 100000 }, { code: '8000', amountCents: -100000 }]);
  post('2026-01-06', 'Inkoop', [{ code: '4000', amountCents: 30000 }, { code: '1100', amountCents: -30000 }]);
  const r = monthEnd(db, { period: '2026-01' });
  assert.equal(r.totals.profit_cents, 70000);
  assert.equal(r.totals.profit, '700.00');
  assert.equal(r.totals.balanced, true);
  assert.equal(r.totals.debit_cents, 100000);
  assert.equal(r.totals.credit_cents, 100000);
});

test('month-end: overdue invoice warning with outstanding total', () => {
  createContact(db, {
    name: 'ACME B.V.', address: 'Straat 1', postalCode: '1000 AA', city: 'Amsterdam',
    actor: 'agent:test',
  });
  const inv = createInvoice(db, {
    contactId: 1, date: '2020-01-01', lines: ['1x Oud @ 100.00 @21'], dueDays: 0,
  });
  finalizeInvoice(db, { id: inv.id, actor: 'agent:test' });
  const r = monthEnd(db, { period: '2026-01' });
  assert.equal(r.invoices.overdue, 1);
  assert.equal(r.invoices.overdue_total_cents, 12100);
  assert.ok(r.warnings.some((w) => w.includes('1 overdue invoice')));
});

test('month-end: invalid period rejected', () => {
  assert.throws(() => monthEnd(db, { period: '2026-13' }), { code: 'INVALID_PERIOD' });
});
