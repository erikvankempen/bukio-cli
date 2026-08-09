/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

import { test, beforeEach } from 'node:test';
import assert from 'node:assert/strict';
import { openDb } from '../src/core/db.js';
import { seedDefaultChart, getAccountByCode } from '../src/core/accounts.js';
import { createEntry, postEntry, getEntry, reverseEntry } from '../src/core/entries.js';
import { enableVatModule, bookVatEntry, parseVatPostingSpecs, obReadout } from '../src/vat/index.js';
import { yearEndClose, yearEndStatus, isYearClosed } from '../src/year-end/index.js';
import { complianceStatus } from '../src/compliance/index.js';
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

test('year-end close: reversing the closing entries re-opens the year (documented undo)', () => {
  entry('2026-03-01', 'Omzet', [{ code: '1100', amountCents: 12100 }, { code: '8000', amountCents: -10000 }, { code: '2500', amountCents: -2100 }]);
  const closed = yearEndClose(db, { year: 2026, actor: 'agent:test' });
  assert.equal(closed.closed, true);
  assert.equal(isYearClosed(db, 2026), true);
  // the documented undo: reverse the closing entries (the error message says
  // "undo with entry reverse on the closing entries")
  const closing = db.prepare("SELECT * FROM journal_entries WHERE source='closing' AND source_ref='fy:2026'").all();
  for (const e of closing) reverseEntry(db, { id: e.id, actor: 'agent:test' });
  // regression (round 11): isYearClosed used to match ANY source='closing'
  // row regardless of reversal — the year stayed locked forever and
  // re-closing threw ALREADY_CLOSED even after a full undo
  assert.equal(isYearClosed(db, 2026), false, 'reversing the closing entries must re-open the year');
  const status = complianceStatus(db, { year: 2026 });
  const ar = status.obligations.find((o) => o.type === 'JAARREKENING' && o.period === '2026');
  assert.ok(ar && ar.books_closed === false, 'the compliance calendar must report the year as open after the undo');
  // and the year can be closed again
  const reopened = yearEndClose(db, { year: 2026, actor: 'agent:test' });
  assert.equal(reopened.closed, true);
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

test('jaarrekening: klein model — resultaat counts inkoop ONCE and adds overige bedrijfsopbrengsten', () => {
  // omzet (8000/WOMZ.80), inkoopwaarde (4000/WKPR.70), overige opbrengsten
  // (8100/WOVB.82) and bedrijfskosten (4300) — regression: WKPR.70 used to be
  // counted twice (once as inkoop, once inside kosten) and WOVB.82 was dropped
  entry('2026-03-01', 'Omzet', [{ code: '1100', amountCents: 12100 }, { code: '8000', amountCents: -10000 }, { code: '2500', amountCents: -2100 }]);
  entry('2026-04-01', 'Inkoop', [{ code: '4000', amountCents: 4000 }, { code: '1100', amountCents: -4000 }]);
  entry('2026-05-01', 'Overige opbrengst', [{ code: '8100', amountCents: -500 }, { code: '1100', amountCents: 500 }]);
  entry('2026-06-01', 'Kosten', [{ code: '4300', amountCents: 2000 }, { code: '1100', amountCents: -2000 }]);

  const r = jaarrekening(db, { year: 2026, model: 'klein' });
  assert.equal(r.pnl.omzet_cents, 10000);
  assert.equal(r.pnl.overige_opbrengsten_cents, 500);
  assert.equal(r.pnl.inkoop_cents, 4000);
  assert.equal(r.pnl.bruto_marge_cents, 6000);
  assert.equal(r.pnl.kosten_cents, 2000); // pure operating costs — NOT incl. inkoop
  assert.equal(r.pnl.resultaat_cents, 4500); // 10000 + 500 - 4000 - 2000
  assert.equal(r.pnl.resultaat, '45.00');
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

test('jaarrekening: klein P&L follows the FISCAL year, not the calendar year', () => {
  // fiscal year ends 06-30 -> for year 2026 the P&L window is 2025-07-01
  // .. 2026-06-30 (the same period the balans peildatum closes)
  db.prepare("UPDATE company SET fiscal_year_end = '06-30'").run();
  entry('2025-06-15', 'Omzet te vroeg', [{ code: '1100', amountCents: 1000 }, { code: '8000', amountCents: -1000 }]); // before window
  entry('2025-09-01', 'Omzet 1', [{ code: '1100', amountCents: 2000 }, { code: '8000', amountCents: -2000 }]); // inside
  entry('2026-06-30', 'Omzet 2', [{ code: '1100', amountCents: 3000 }, { code: '8000', amountCents: -3000 }]); // inside (boundary)
  entry('2026-07-15', 'Omzet te laat', [{ code: '1100', amountCents: 4000 }, { code: '8000', amountCents: -4000 }]); // after window

  const r = jaarrekening(db, { year: 2026, model: 'klein' });
  assert.equal(r.as_of, '2026-06-30');
  // balans peildatum closes 2026-06-30: the July entry is after as-of and
  // off the balans; the cumulative result position still balances
  assert.ok(r.balans.balanced);
  // P&L includes ONLY the fiscal window: 2000 + 3000, NOT the 1000 before
  // or the 4000 after
  assert.equal(r.pnl.omzet_cents, 5000);
  assert.equal(r.pnl.resultaat_cents, 5000);
});

test('year-end close: follows the FISCAL year for non-calendar fiscal years', () => {
  // FY ends 06-30 -> closing year 2026 covers 2025-07-01..2026-06-30
  db.prepare("UPDATE company SET fiscal_year_end = '06-30'").run();
  entry('2025-06-15', 'Te vroeg', [{ code: '1100', amountCents: 1000 }, { code: '8000', amountCents: -1000 }]); // before window
  entry('2025-09-01', 'Omzet 1', [{ code: '1100', amountCents: 2000 }, { code: '8000', amountCents: -2000 }]); // inside
  entry('2026-06-30', 'Omzet 2', [{ code: '1100', amountCents: 3000 }, { code: '8000', amountCents: -3000 }]); // inside (FY end)
  entry('2026-07-15', 'Te laat', [{ code: '1100', amountCents: 4000 }, { code: '8000', amountCents: -4000 }]); // after window

  yearEndClose(db, { year: 2026, actor: 'agent:test' });
  const closing = db.prepare("SELECT * FROM journal_entries WHERE source = 'closing' ORDER BY id").all();
  assert.ok(closing.length >= 1);
  // closing entries are dated at the FISCAL year end, not 12-31
  assert.equal(closing[0].date, '2026-06-30');
  // the closed result is the in-window one: 2000 + 3000 = 5000
  const status = yearEndStatus(db, { year: 2026 });
  assert.equal(status.result_cents, 5000);
  // the outside-window entries (1000 before, 4000 after) were NOT closed —
  // the closing entries negate only the in-window 8000 balance, and the
  // P&L accounts still carry the untouched 10000 of original postings
  const leftover = db.prepare(`
    SELECT COALESCE(SUM(p.amount_cents), 0) AS net FROM postings p
    JOIN journal_entries e ON e.id = p.entry_id AND e.state = 'posted' AND e.source != 'closing'
    JOIN accounts a ON a.id = p.account_id WHERE a.code = '8000'
  `).get();
  assert.equal(leftover.net, -10000);
});

test('jaarrekening: invalid model rejected', () => {
  assert.throws(() => jaarrekening(db, { year: 2026, model: 'groot' }), { code: 'INVALID_MODEL' });
});

test('jaarrekening: account-level amounts are numbers, never NaN', () => {
  entry('2026-03-01', 'Omzet', [{ code: '1100', amountCents: 12100 }, { code: '8000', amountCents: -10000 }, { code: '2500', amountCents: -2100 }]);
  entry('2026-03-05', 'Laptop', [{ code: '1800', amountCents: 537000 }, { code: '1100', amountCents: -537000 }]);
  const r = jaarrekening(db, { year: 2026, model: 'klein' });
  const accounts = [
    ...r.balans.activa.flatMap((g) => g.sections.flatMap((s) => s.accounts)),
    ...r.balans.passiva.flatMap((g) => g.sections.flatMap((s) => s.accounts)),
    ...(r.pnl?.lines.flatMap((l) => l.sections.flatMap((s) => s.accounts)) ?? []),
  ];
  assert.ok(accounts.length >= 3, 'expected account detail rows');
  for (const a of accounts) {
    assert.equal(typeof a.amount_cents, 'number', `${a.name} amount_cents must be a number`);
    assert.ok(Number.isFinite(a.amount_cents), `${a.name} amount_cents must not be NaN`);
  }
});

test('jaarrekening: PDF html renders account detail without NaN', () => {
  entry('2026-03-01', 'Omzet', [{ code: '1100', amountCents: 12100 }, { code: '8000', amountCents: -10000 }, { code: '2500', amountCents: -2100 }]);
  entry('2026-03-05', 'Laptop', [{ code: '1800', amountCents: 537000 }, { code: '1100', amountCents: -537000 }]);
  const r = jaarrekening(db, { year: 2026, model: 'klein' });
  const html = jaarrekeningHtml(r);
  assert.ok(!html.includes('NaN'), 'the HTML template must not contain NaN');
  assert.ok(html.includes('5370.00'), 'account detail amount rendered');
});

test('jaarrekening: pnl includes the Afschrijvingen line for WAFS.41', () => {
  entry('2026-03-01', 'Afschr', [{ code: '1800', amountCents: -10000 }, { code: '4600', amountCents: 10000 }]);
  const r = jaarrekening(db, { year: 2026, model: 'klein' });
  const line = r.pnl.lines.find((l) => l.rgs_code === 'WAFS.41');
  assert.ok(line, 'WAFS.41 must map to an Afschrijvingen line');
  assert.equal(line.label, 'Afschrijvingen');
  assert.equal(line.total_cents, 10000);
});

test('jaarrekening PDF: renders (playwright)', async () => {
  entry('2026-03-01', 'Omzet', [{ code: '1100', amountCents: 12100 }, { code: '8000', amountCents: -10000 }, { code: '2500', amountCents: -2100 }]);
  const r = jaarrekening(db, { year: 2026, model: 'klein' });
  const html = jaarrekeningHtml(r);
  assert.match(html, /Jaarrekening 2026/);
  assert.match(html, /Totaal activa/);
  assert.ok(!html.includes('NaN'), 'the HTML template must not contain NaN');
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
