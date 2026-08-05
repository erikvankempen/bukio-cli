import { test, beforeEach } from 'node:test';
import assert from 'node:assert/strict';
import { openDb } from '../src/core/db.js';
import { seedDefaultChart, getAccountByCode } from '../src/core/accounts.js';
import { createEntry, postEntry, getEntry } from '../src/core/entries.js';
import { enableVatModule, bookVatEntry, parseVatPostingSpecs, obReadout } from '../src/vat/index.js';
import { yearEndClose, yearEndStatus } from '../src/year-end/index.js';
import { jaarrekening } from '../src/report/jaarrekening.js';
import { jaarrekeningToPdf, jaarrekeningHtml } from '../src/report/jaarrekening-pdf.js';
import { icpReadout } from '../src/icp/index.js';
import {
  createContact, createInvoice, finalizeInvoice, getInvoice,
} from '../src/invoice/index.js';

let db;

function setup({ vat = true, complete = true } = {}) {
  db = openDb(':memory:');
  seedDefaultChart(db);
  db.prepare(`
    INSERT INTO company (name, kvk, legal_form, btw_id, iban, address, postal_code, city, vat_module)
    VALUES ('Demo BV', '12345678', 'bv', 'NL123456789B01', 'NL91ABNA0417164300', ?, '2712 CD', 'Zoetermeer', ?)
  `).run(complete ? 'Industrieweg 12' : null, vat ? 1 : 0);
  if (vat) enableVatModule(db);
}

function entry(date, desc, postings, opts = {}) {
  const e = createEntry(db, { date, description: desc, postings, source: opts.source ?? 'manual', actor: 'agent:test' });
  return postEntry(db, { id: e.id, actor: 'agent:test' });
}

beforeEach(() => {
  setup();
});

test('year-end close: posts closing + appropriation, balanced, source closing', () => {
  entry('2026-03-01', 'Omzet A', [{ code: '1100', amountCents: 12100 }, { code: '8000', amountCents: -10000 }, { code: '2500', amountCents: -2100 }]);
  entry('2026-04-01', 'Software', [{ code: '4300', amountCents: 3000 }, { code: '1100', amountCents: -3000 }]);

  const result = yearEndClose(db, { year: 2026, actor: 'agent:test' });
  assert.equal(result.closed, true);
  assert.equal(result.result_cents, 7000); // 10000 - 3000

  // 9900 created on demand (equity)
  const r99 = getAccountByCode(db, '9900');
  assert.ok(r99);
  assert.equal(r99.type, 'equity');

  // two closing entries, posted, tagged
  const closing = db.prepare("SELECT * FROM journal_entries WHERE source='closing' ORDER BY id").all();
  assert.equal(closing.length, 2);
  assert.ok(closing.every((e) => e.state === 'posted' && e.source_ref === 'fy:2026'));
  assert.equal(closing[0].description, 'Afsluiting boekjaar 2026');
  assert.equal(closing[1].description, 'Resultaatbestemming 2026');

  // ledger net: income/expense closed to zero; equity up by result
  const omzet = getEntry(db, 1).postings.find((p) => p.account_code === '8000').amount_cents; // original -10000
  const closeOmzet = closing[0].id ? null : null;
  const sumOmzet = db.prepare(`
    SELECT COALESCE(SUM(p.amount_cents),0) s FROM postings p
    JOIN journal_entries e ON e.id = p.entry_id
    WHERE e.state='posted' AND p.account_id = (SELECT id FROM accounts WHERE code='8000')
  `).get().s;
  assert.equal(sumOmzet, 0); // closed out
  const sum3000 = db.prepare(`
    SELECT COALESCE(SUM(p.amount_cents),0) s FROM postings p
    JOIN journal_entries e ON e.id = p.entry_id
    WHERE e.state='posted' AND p.account_id = (SELECT id FROM accounts WHERE code='3000')
  `).get().s;
  assert.equal(sum3000, -7000); // equity credited with the result
  const totals = db.prepare('SELECT COALESCE(SUM(amount_cents),0) s FROM postings').get().s;
  assert.equal(totals, 0);
  assert.ok(closeOmzet === null || closeOmzet === null); // placeholder no-op

  // second close rejected
  assert.throws(() => yearEndClose(db, { year: 2026 }), { code: 'ALREADY_CLOSED' });
});

test('year-end close: guards — drafts block, empty year reports', () => {
  createEntry(db, { date: '2026-05-01', description: 'draft', postings: [{ code: '1100', amountCents: 100 }, { code: '3000', amountCents: -100 }], actor: 'agent:test' });
  assert.throws(() => yearEndClose(db, { year: 2026 }), { code: 'INCOMPLETE_YEAR' });

  const empty = yearEndClose(db, { year: 2025 });
  assert.equal(empty.closed, false);
  assert.equal(empty.reason, 'EMPTY_YEAR');
  assert.throws(() => yearEndClose(db, { year: 'bad' }), { code: 'INVALID_YEAR' });
});

test('year-end close: dry-run writes nothing', () => {
  entry('2026-03-01', 'Omzet', [{ code: '1100', amountCents: 12100 }, { code: '8000', amountCents: -10000 }, { code: '2500', amountCents: -2100 }]);
  const plan = yearEndClose(db, { year: 2026, dryRun: true });
  assert.equal(plan.dryRun, true);
  assert.equal(plan.result_cents, 10000);
  assert.equal(plan.create_9900, true);
  assert.equal(plan.entries.length, 2);
  assert.equal(db.prepare("SELECT COUNT(*) c FROM journal_entries WHERE source='closing'").get().c, 0);
  assert.equal(getAccountByCode(db, '9900'), null);
});

test('P&L still shows the year result after closing', async () => {
  entry('2026-03-01', 'Omzet', [{ code: '1100', amountCents: 12100 }, { code: '8000', amountCents: -10000 }, { code: '2500', amountCents: -2100 }]);
  entry('2026-04-01', 'Kosten', [{ code: '4300', amountCents: 3000 }, { code: '1100', amountCents: -3000 }]);
  yearEndClose(db, { year: 2026 });
  const { pnl } = await import('../src/report/pnl.js');
  const r = pnl(db, { from: '2026-01-01', to: '2026-12-31' });
  assert.equal(r.revenue_cents, 10000);
  assert.equal(r.costs_cents, 3000);
  assert.equal(r.result_cents, 7000);
});

test('jaarrekening: klein model — statutory balans + W&V, balanced', () => {
  entry('2026-03-01', 'Omzet', [{ code: '1100', amountCents: 12100 }, { code: '8000', amountCents: -10000 }, { code: '2500', amountCents: -2100 }]);
  entry('2026-04-01', 'Kosten', [{ code: '4300', amountCents: 3000 }, { code: '1100', amountCents: -3000 }]);

  const r = jaarrekening(db, { year: 2026, model: 'klein' });
  assert.equal(r.model, 'klein');
  assert.ok(r.balans.balanced, 'balans must balance');
  assert.equal(r.balans.total_activa_cents, r.balans.total_passiva_cents);
  // activa lines present
  const labels = r.balans.activa.map((g) => g.label);
  assert.ok(labels.includes('Liquide middelen')); // 1100
  // eigen vermogen includes the onverdeeld resultaat (pre-close)
  const ev = r.balans.passiva.find((s) => s.rgs_code === 'BEIV.05');
  assert.ok(ev);
  assert.equal(ev.total_cents, 7000);
  // W&V klein
  assert.ok(r.pnl);
  assert.equal(r.pnl.omzet_cents, 10000);
  assert.equal(r.pnl.inkoop_cents, 0);
  assert.equal(r.pnl.resultaat_cents, 7000);
});

test('jaarrekening: after closing, result sits in equity (no onverdeeld)', () => {
  entry('2026-03-01', 'Omzet', [{ code: '1100', amountCents: 12100 }, { code: '8000', amountCents: -10000 }, { code: '2500', amountCents: -2100 }]);
  yearEndClose(db, { year: 2026 });
  const r = jaarrekening(db, { year: 2026, model: 'micro' });
  const ev = r.balans.passiva.find((s) => s.rgs_code === 'BEIV.05');
  assert.equal(ev.total_cents, 10000); // result closed into equity
  assert.ok(!ev.sections.some((s) => s.label === 'Onverdeeld resultaat'));
  assert.equal(r.pnl, undefined); // micro has no W&V
});

test('jaarrekening: invalid model rejected', () => {
  assert.throws(() => jaarrekening(db, { year: 2026, model: 'groot' }), { code: 'INVALID_MODEL' });
});

test('jaarrekening PDF: renders (playwright)', async () => {
  entry('2026-03-01', 'Omzet', [{ code: '1100', amountCents: 12100 }, { code: '8000', amountCents: -10000 }, { code: '2500', amountCents: -2100 }]);
  const r = jaarrekening(db, { year: 2026, model: 'klein' });
  const html = jaarrekeningHtml(r);
  assert.match(html, /Jaarrekening 2026/);
  assert.match(html, /Totaal activa/);
  const pdf = await jaarrekeningToPdf(r, { outPath: '/tmp/test-jaarrekening.pdf' });
  assert.ok(pdf.bytes > 5000);
});

test('OB readout: R purchase -> 3a/4a, RE purchase -> 3b/4b, RE sale -> 2a', () => {
  // binnenlandse verlegde inkoop
  bookVatEntry(db, {
    date: '2026-04-01', description: 'Inkoop verlegd binnenland',
    postings: parseVatPostingSpecs(['4300:100.00@R,1100:-100.00,2500:-21.00']), post: true,
  });
  let r = obReadout(db, { period: '2026-Q2' });
  assert.equal(r.fields['3a'], 10000);
  assert.equal(r.fields['4a'], 2100);
  assert.equal(r.fields['5b'], 2100);
  assert.equal(r.fields['5d'], 0);

  // EU verlegde inkoop
  bookVatEntry(db, {
    date: '2026-05-01', description: 'Inkoop verlegd EU',
    postings: parseVatPostingSpecs(['4300:500.00@RE,1100:-500.00,2500:-105.00']), post: true,
  });
  r = obReadout(db, { period: '2026-Q2' });
  assert.equal(r.fields['3b'], 50000);
  assert.equal(r.fields['4b'], 10500);
  assert.equal(r.fields['5b'], 12600);
  assert.equal(r.fields['5d'], 0);
});

test('OB readout: verlegde EU sale (RE invoice) reports 2a', () => {
  const c = createContact(db, {
    name: 'GmbH Berlin', address: 'Hauptstr 1', city: 'Berlin', country: 'DE',
    vatId: 'DE123456789', actor: 'agent:test',
  });
  const inv = createInvoice(db, {
    contactId: c.id, date: '2026-07-10', lines: ['1x Advies @ 2000.00 @RE'],
  });
  finalizeInvoice(db, { id: inv.id });
  const r = obReadout(db, { period: '2026-Q3' });
  assert.equal(r.fields['2a'], 200000);
  assert.equal(r.fields['1a'], 0);
});

test('ICP readout: EU customers with RE lines, totals per customer', () => {
  const de = createContact(db, {
    name: 'GmbH Berlin', address: 'Hauptstr 1', city: 'Berlin', country: 'DE',
    vatId: 'DE123456789', actor: 'agent:test',
  });
  const be = createContact(db, {
    name: 'NV Brussel', address: 'Rue 2', city: 'Brussel', country: 'BE',
    vatId: 'BE0123456789', actor: 'agent:test',
  });
  const inv1 = createInvoice(db, { contactId: de.id, date: '2026-07-10', lines: ['1x Advies @ 2000.00 @RE'] });
  const inv2 = createInvoice(db, { contactId: de.id, date: '2026-08-01', lines: ['1x Advies @ 500.00 @RE'] });
  const inv3 = createInvoice(db, { contactId: be.id, date: '2026-09-05', lines: ['1x Support @ 300.00 @RE'] });
  finalizeInvoice(db, { id: inv1.id });
  finalizeInvoice(db, { id: inv2.id });
  finalizeInvoice(db, { id: inv3.id });

  const r = icpReadout(db, { period: '2026-Q3' });
  assert.equal(r.customers.length, 2);
  const deRow = r.customers.find((c) => c.name === 'GmbH Berlin');
  assert.equal(deRow.amount_cents, 250000);
  assert.equal(deRow.vat_id, 'DE123456789');
  assert.equal(deRow.invoice_numbers.length, 2);
  const beRow = r.customers.find((c) => c.name === 'NV Brussel');
  assert.equal(beRow.amount_cents, 30000);
  assert.equal(r.total_cents, 280000);
});

test('ICP readout: missing customer vat-id fails loudly', () => {
  const c = createContact(db, {
    name: 'GmbH Ohne Vat', address: 'Hauptstr 1', city: 'Berlin', country: 'DE',
    vatId: 'DE123456789', actor: 'agent:test',
  });
  const inv = createInvoice(db, { contactId: c.id, date: '2026-07-10', lines: ['1x Advies @ 2000.00 @RE'] });
  finalizeInvoice(db, { id: inv.id });
  // compliance guarantees a vat-id at finalize — simulate it being lost later
  db.prepare('UPDATE contacts SET vat_id = NULL WHERE id = ?').run(c.id);
  assert.throws(() => icpReadout(db, { period: '2026-Q3' }), { code: 'ICP_VAT_ID_MISSING' });
});

test('ICP readout: no RE lines -> empty listing', () => {
  const r = icpReadout(db, { period: '2026-Q3' });
  assert.equal(r.customers.length, 0);
  assert.equal(r.total_cents, 0);
});
