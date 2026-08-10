/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Edge cases across the whole surface — guards, rounding, boundaries,
// lifecycle violations, idempotency. Each test asserts real behaviour only.
import { test, beforeEach } from 'node:test';
import assert from 'node:assert/strict';
import { existsSync, mkdtempSync, rmSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { openDb } from '../src/core/db.js';
import { ensureDb } from '../src/cli/util.js';
import { seedDefaultChart } from '../src/core/accounts.js';
import { createEntry, postEntry, getEntry, reverseEntry } from '../src/core/entries.js';
import { enableVatModule, bookVatEntry, parseVatPostingSpecs, obReadout } from '../src/vat/index.js';
import { createTemplate, runDue, buildDepreciationTemplate } from '../src/recurring/index.js';
import { importTransactions, autoMatch } from '../src/bank/index.js';
import { parseCamt053 } from '../src/bank/camt.js';
import { parseBankCsv } from '../src/bank/csv.js';
import { yearEndClose } from '../src/year-end/index.js';
import { jaarrekening } from '../src/report/jaarrekening.js';
import { balans } from '../src/report/balans.js';
import { pnl } from '../src/report/pnl.js';
import { icpReadout } from '../src/icp/index.js';
import { setFxRate, parseRate, convertFx } from '../src/fx/index.js';
import { invoiceToUbl } from '../src/invoice/ubl.js';
import {
  createContact, createInvoice, finalizeInvoice, creditInvoice,
  getInvoice, markPaid, parseLineSpec, validateCompliance,
} from '../src/invoice/index.js';

let db;

function setup({ vat = true } = {}) {
  db = openDb(':memory:');
  seedDefaultChart(db);
  db.prepare(`
    INSERT INTO company (name, kvk, legal_form, btw_id, iban, address, postal_code, city, vat_module)
    VALUES ('Demo BV', '12345678', 'bv', 'NL123456789B01', 'NL91ABNA0417164300',
            'Industrieweg 12', '2712 CD', 'Zoetermeer', ?)
  `).run(vat ? 1 : 0);
  if (vat) enableVatModule(db);
}

function entry(date, desc, postings, opts = {}) {
  const e = createEntry(db, { date, description: desc, postings, source: opts.source ?? 'manual', actor: 'agent:test' });
  return postEntry(db, { id: e.id, actor: 'agent:test' });
}

function addContact(vatId = null) {
  return createContact(db, {
    name: 'ACME B.V.', address: 'Straat 1', postalCode: '1000 AA', city: 'Amsterdam',
    vatId, actor: 'agent:test',
  });
}

// Relative date helper — invoice status derives 'overdue' at read time
// (sent + past due), so fixtures must stay relative to the wall clock.
function daysFromNow(days) {
  return new Date(Date.now() + days * 86400000).toISOString().slice(0, 10);
}

beforeEach(() => {
  setup();
});

// --- ledger guards ---------------------------------------------------------

test('ledger: unbalanced, zero-amount, too-few postings rejected', () => {
  assert.throws(() => createEntry(db, { date: '2026-01-01', description: 'x', postings: [{ code: '1100', amountCents: 100 }, { code: '3000', amountCents: -99 }], actor: 'a' }), { code: 'UNBALANCED' });
  assert.throws(() => createEntry(db, { date: '2026-01-01', description: 'x', postings: [{ code: '1100', amountCents: 0 }, { code: '3000', amountCents: 0 }], actor: 'a' }), { code: 'INVALID_AMOUNT_CENTS' });
  assert.throws(() => createEntry(db, { date: '2026-01-01', description: 'x', postings: [{ code: '1100', amountCents: 100 }], actor: 'a' }), { code: 'TOO_FEW_POSTINGS' });
  assert.throws(() => createEntry(db, { date: '2026-01-01', description: 'x', postings: [{ code: '9999', amountCents: 100 }, { code: '3000', amountCents: -100 }], actor: 'a' }), { code: 'ACCOUNT_NOT_FOUND' });
  assert.throws(() => createEntry(db, { date: '2026-01-01', description: 'x', postings: [{ code: '1100', amountCents: 100 }, { code: '3000', amountCents: -100 }], source: 'bogus', actor: 'a' }), { code: 'INVALID_SOURCE' });
});

test('ledger: same account on both sides is legal', () => {
  const e = entry('2026-01-01', 'Kas naar bank', [{ code: '1100', amountCents: 10000 }, { code: '1100', amountCents: -10000 }]);
  assert.equal(e.state, 'posted');
});

test('ledger: reversal guards — draft and double-reversal rejected', () => {
  const draft = createEntry(db, { date: '2026-01-01', description: 'd', postings: [{ code: '1100', amountCents: 100 }, { code: '3000', amountCents: -100 }], actor: 'a' });
  assert.throws(() => reverseEntry(db, { id: draft.id, actor: 'a' }), { code: 'NOT_POSTED' });
  const posted = entry('2026-01-01', 'p', [{ code: '1100', amountCents: 100 }, { code: '3000', amountCents: -100 }]);
  reverseEntry(db, { id: posted.id, actor: 'a' });
  assert.throws(() => reverseEntry(db, { id: posted.id, actor: 'a' }), { code: 'ALREADY_REVERSED' });
  // reversal nets the account to zero
  const net = db.prepare("SELECT COALESCE(SUM(p.amount_cents),0) s FROM postings p JOIN journal_entries e ON e.id=p.entry_id WHERE e.state='posted' AND p.account_id=(SELECT id FROM accounts WHERE code='1100')").get().s;
  assert.equal(net, 0);
});

test('ledger: posted entries are immutable — direct UPDATE blocked by trigger', () => {
  entry('2026-01-01', 'p', [{ code: '1100', amountCents: 100 }, { code: '3000', amountCents: -100 }]);
  assert.throws(() => db.prepare("UPDATE journal_entries SET description='hack' WHERE id=1").run(), { code: 'SQLITE_CONSTRAINT_TRIGGER' });
  assert.throws(() => db.prepare('DELETE FROM postings WHERE entry_id=1').run(), { code: 'SQLITE_CONSTRAINT_TRIGGER' });
});

test('ledger: drafts excluded from balans and P&L', () => {
  createEntry(db, { date: '2026-01-01', description: 'draft', postings: [{ code: '1100', amountCents: 500 }, { code: '3000', amountCents: -500 }], actor: 'a' });
  entry('2026-01-01', 'posted', [{ code: '1100', amountCents: 10000 }, { code: '8000', amountCents: -10000 }]);
  const b = balans(db, { asOf: '2026-12-31' });
  assert.equal(b.assets.total_cents, 10000); // draft 500 not counted
  const p = pnl(db, { from: '2026-01-01', to: '2026-12-31' });
  assert.equal(p.revenue_cents, 10000);
});

// --- invoice edge cases ----------------------------------------------------

test('invoice: quantity and price guards', () => {
  addContact();
  assert.throws(() => createInvoice(db, { contactId: 1, date: '2026-07-10', lines: ['0x Ding @ 10.00'] }), { code: 'INVALID_LINE' });
  assert.throws(() => createInvoice(db, { contactId: 1, date: '2026-07-10', lines: ['Ding @ 0.00'] }), { code: 'INVALID_LINE' });
  assert.throws(() => createInvoice(db, { contactId: 1, date: '2026-07-10', lines: ['Ding @ -5.00'] }), { code: 'INVALID_LINE' });
});

test('invoice: line parser — Dutch comma price, @ inside description', () => {
  assert.deepEqual(parseLineSpec('Papier @ 45,50 @21'), { qtyMilli: 1000, qty: 1, description: 'Papier', priceCents: 4550, vatCode: '21', discountType: null, discountValue: null });
  // descriptions with @ are kept (space-normalised around the @)
  assert.deepEqual(parseLineSpec('2x Email @ adres @ 5.00'), { qtyMilli: 2000, qty: 2, description: 'Email@adres', priceCents: 500, vatCode: null, discountType: null, discountValue: null });
  assert.deepEqual(parseLineSpec('1000x Zeer lange omschrijving met @ tekens erin @ 0.99 @9'), { qtyMilli: 1000000, qty: 1000, description: 'Zeer lange omschrijving met@tekens erin', priceCents: 99, vatCode: '9', discountType: null, discountValue: null });
});

test('invoice: line parser — price-only integer price, lowercase vat codes, negative qty (regression)', () => {
  // "DESC @ 100" used to fail: "100" was misread as a VAT code
  assert.deepEqual(parseLineSpec('Consultancy @ 100'), { qtyMilli: 1000, qty: 1, description: 'Consultancy', priceCents: 10000, vatCode: null, discountType: null, discountValue: null });
  // lowercase vat codes are normalised (and recognised as codes, not prices)
  assert.deepEqual(parseLineSpec('Uren @ 75.00 @re'), { qtyMilli: 1000, qty: 1, description: 'Uren', priceCents: 7500, vatCode: 'RE', discountType: null, discountValue: null });
  // a non-vat-code word stays a price (fails cleanly, not silently as a code)
  assert.throws(() => parseLineSpec('Uren @ 75.00 @nope'), { code: 'INVALID_LINE' });
  // negative quantity is rejected, not silently booked as qty 1
  assert.throws(() => parseLineSpec('-2x Uren @ 75.00'), { code: 'INVALID_LINE' });
});

test('invoice: per-line rounding edge — 3x 0.01 @21 has 1 cent VAT (line-total rounding)', () => {
  addContact();
  const inv = createInvoice(db, { contactId: 1, date: '2026-07-10', lines: ['3x Pennen @ 0.01 @21'] });
  // vat is computed on the line total: round(0.03 * 21%) = round(0.63¢) = 1¢
  assert.equal(inv.lines[0].vat_amount_cents, 1);
  assert.equal(inv.net_cents, 3);
  assert.equal(inv.gross_cents, 4);
});

test('invoice: 0% and exempt (V) lines book without VAT', () => {
  addContact();
  const inv = createInvoice(db, { contactId: 1, date: '2026-07-10', lines: ['1x Export @ 500.00 @0', '1x Vrijstelling @ 100.00 @V'] });
  finalizeInvoice(db, { id: inv.id });
  const e = getEntry(db, 1);
  // 0%/V lines are TAGGED (vat_code surfaced since the 2026-08-07 getEntry
  // join) with a zero vat amount — that is what makes the OB readout report
  // the base in 1c without any vat due in 5a
  const tagged = e.postings.filter((p) => p.vat_code);
  assert.equal(tagged.length, 2);
  assert.ok(tagged.every((p) => p.vat_amount_cents === 0));
  assert.equal(e.postings.find((p) => p.account_code === '1200').amount_cents, 60000);
  const omzet = e.postings.filter((p) => p.account_code === '8000').reduce((s, p) => s + p.amount_cents, 0);
  assert.equal(omzet, -60000);
  const r = obReadout(db, { period: '2026-Q3' });
  assert.equal(r.fields['1c'], 60000); // 0%/vrijgesteld omzet
  assert.equal(r.fields['5a'], 0);
});

test('invoice: credit note of a paid invoice; credit of a credit rejected', () => {
  addContact();
  const inv = createInvoice(db, { contactId: 1, date: '2026-07-10', lines: ['1x Werk @ 100.00 @21'] });
  finalizeInvoice(db, { id: inv.id });
  markPaid(db, { id: inv.id, date: '2026-07-20', amountCents: 12100 });
  const credit = creditInvoice(db, { id: inv.id });
  finalizeInvoice(db, { id: credit.id });
  assert.throws(() => creditInvoice(db, { id: credit.id }), { code: 'NOT_SALES_INVOICE' });
});

test('invoice: lifecycle — pay draft rejected, overpayment rejected, overdue derived', () => {
  addContact();
  const draft = createInvoice(db, { contactId: 1, date: '2026-01-01', lines: ['1x X @ 100.00 @21'] });
  assert.throws(() => markPaid(db, { id: draft.id, date: '2026-01-05', amountCents: 100 }), { code: 'NOT_PAYABLE' });
  finalizeInvoice(db, { id: draft.id });
  assert.throws(() => markPaid(db, { id: draft.id, date: '2026-01-05', amountCents: 999999 }), { code: 'OVERPAYMENT' });
  // overdue is derived: due_date 2026-01-31 < today (2026-08-04)
  const inv = getInvoice(db, draft.id);
  assert.equal(inv.status, 'overdue');
});

test('invoice: UBL escaping and verlegd category', () => {
  addContact('DE123456789');
  const inv = createInvoice(db, {
    contactId: 1, date: '2026-07-10',
    lines: ['1x IT & Support <urgent> @ 500.00 @RE', '1x Normaal @ 100.00 @21'],
  });
  finalizeInvoice(db, { id: inv.id });
  const xml = invoiceToUbl(db, getInvoice(db, inv.id));
  assert.match(xml, /IT &amp; Support &lt;urgent&gt;/); // escaped
  assert.match(xml, /<cbc:ID>AE<\/cbc:ID>/); // verlegd line -> AE category
  assert.match(xml, /<cbc:ID>S<\/cbc:ID>/); // standard line
  assert.match(xml, /<cbc:TaxAmount currencyID="EUR">21\.00<\/cbc:TaxAmount>/); // only the 21% line
});

test('invoice: due date crosses the year boundary', () => {
  addContact();
  const inv = createInvoice(db, { contactId: 1, date: '2026-12-20', lines: ['1x X @ 10.00'], dueDays: 30 });
  assert.equal(inv.due_date, '2027-01-19');
});

// --- recurring edge cases --------------------------------------------------

test('recurring: day 28 keeps the 28th every month (no drift)', () => {
  const tpl = createTemplate(db, {
    name: 'Huur', postings: ['4300:1000.00,1100:-1000.00'],
    frequency: 'monthly', dayOfPeriod: 28, startDate: '2026-01-28', actor: 'a',
  });
  assert.equal(tpl.next_run_date, '2026-01-28');
  runDue(db, { asOf: '2026-12-31' });
  const dates = db.prepare("SELECT DISTINCT date FROM journal_entries WHERE source='recurring' ORDER BY date").all().map((r) => r.date);
  assert.equal(dates.length, 12);
  assert.ok(dates.every((d) => d.endsWith('-28')));
});

test('recurring: quarterly and yearly frequencies', () => {
  createTemplate(db, { name: 'Q', postings: ['4300:10.00,1100:-10.00'], frequency: 'quarterly', startDate: '2026-01-01', actor: 'a' });
  createTemplate(db, { name: 'Y', postings: ['4300:10.00,1100:-10.00'], frequency: 'yearly', startDate: '2026-01-01', actor: 'a' });
  runDue(db, { asOf: '2027-06-30' });
  const dates = db.prepare("SELECT date FROM journal_entries WHERE source='recurring' ORDER BY date").all().map((r) => r.date);
  // quarterly: 2026 Q1..Q4 + 2027 Q1..Q2 = 6; yearly: 2026 + 2027 = 2
  assert.equal(dates.filter((d) => d.startsWith('2026-')).length, 5);
  assert.equal(dates.length, 8);
});

test('recurring: end_date stops the schedule', () => {
  createTemplate(db, {
    name: 'Tijdelijk', postings: ['4300:10.00,1100:-10.00'],
    frequency: 'monthly', startDate: '2026-01-01', endDate: '2026-03-15', actor: 'a',
  });
  runDue(db, { asOf: '2026-12-31' });
  assert.equal(db.prepare("SELECT COUNT(*) c FROM journal_entries WHERE source='recurring'").get().c, 3);
});

test('recurring: templateId run only runs that template even when due later', () => {
  createTemplate(db, { name: 'A', postings: ['4300:10.00,1100:-10.00'], frequency: 'monthly', startDate: '2026-01-01', actor: 'a' });
  createTemplate(db, { name: 'B', postings: ['4300:20.00,1100:-20.00'], frequency: 'monthly', startDate: '2026-03-01', actor: 'a' });
  const r = runDue(db, { asOf: '2026-01-31', templateId: 2 });
  assert.equal(r.templates.length, 0); // B not due yet
  const r2 = runDue(db, { asOf: '2026-03-31', templateId: 2 });
  assert.equal(r2.templates[0].runs.length, 1);
  assert.equal(r2.templates[0].runs[0].generated[0].entry.description, 'B 2026-03-01');
});

test('recurring: depreciation with residual value — final run absorbs the remainder', () => {
  const t = buildDepreciationTemplate(db, {
    name: 'Auto', costCents: 1200000, residualCents: 100000, lifeMonths: 12, startDate: '2026-01-01', actor: 'a',
  });
  // (12000 - 1000) / 12 = 916.67/mo; final = 11000 - 916.67*11 = 916.63
  assert.equal(t.template.postings.find((p) => p.code === '4600').amountCents, 91667);
  assert.equal(t.template.final_postings.find((p) => p.code === '4600').amountCents, 91663);
  assert.equal(t.total_cents, 1100000); // full depreciation to residual
  assert.throws(() => buildDepreciationTemplate(db, { name: 'x', costCents: 1000, residualCents: 1000, lifeMonths: 36, startDate: '2026-01-01' }), { code: 'INVALID_RESIDUAL' });
});

// --- bank edge cases -------------------------------------------------------

const IBAN = 'NL91ABNA0417164300';
const CAMT_DUP = `<?xml version="1.0"?>
<Document xmlns="urn:iso:std:iso:20022:tech:xsd:camt.053.001.02">
  <BkToCstmrStmt><Stmt><Acct><Id><IBAN>${IBAN}</IBAN></Id></Acct>
    <Ntry><Amt>100.00</Amt><CdtDbtInd>CRDT</CdtDbtInd><BookgDt><Dt>2026-06-01</Dt></BookgDt>
      <NtryDtls><TxDtls><RltdPties><Dbtr><Nm>ACME B.V.</Nm></Dbtr></RltdPties>
      <RmtInf><Ustrd>Factuur 1</Ustrd></RmtInf></TxDtls></NtryDtls></Ntry>
  </Stmt></BkToCstmrStmt>
</Document>`;

test('bank: import is idempotent — same statement twice = 0 duplicates on re-import', () => {
  importTransactions(db, { iban: IBAN, transactions: parseCamt053(CAMT_DUP) });
  const again = importTransactions(db, { iban: IBAN, transactions: parseCamt053(CAMT_DUP) });
  assert.equal(again.imported, 0);
  assert.equal(again.duplicates, 1);
  assert.equal(again.total, 1);
});

test('bank: Rabo CSV with Af/Bij and Dutch decimals parses correctly', () => {
  const csv = [
    'Datum;Naam / Omschrijving;Rekening;Tegenrekening;Code;Af Bij;Bedrag (EUR);MutatieSoort;Mededelingen',
    `2026-06-01;ACME B.V.;${IBAN};NL00RABO0123456789;GT;Bij;100,00;Overschrijving;Factuur 1`,
    `2026-06-02;Kantoorwinkel BV;${IBAN};NL00RABO9876543210;GT;Af;25,50;Overschrijving;Kantoorartikelen`,
  ].join('\n');
  const txs = parseBankCsv(csv, { defaultIban: IBAN });
  assert.equal(txs.length, 2);
  assert.equal(txs[0].amount_cents, 10000);
  assert.equal(txs[1].amount_cents, -2550);
});

test('bank: auto-match prefers an exact entry over an invoice for the same amount', () => {
  addContact();
  const inv = createInvoice(db, { contactId: 1, date: daysFromNow(-2), lines: ['1x Werk @ 100.00 @21'] });
  finalizeInvoice(db, { id: inv.id }); // gross 121.00 on 1200, not on 1100
  // manually book the receipt on the bank account (e.g. cashflow entry)
  entry(daysFromNow(-1), 'Ontvangst', [{ code: '1100', amountCents: 12100 }, { code: '1200', amountCents: -12100 }]);
  importTransactions(db, {
    iban: IBAN,
    transactions: parseCamt053(CAMT_DUP
      .replace('100.00', '121.00')
      .replace('2026-06-01', daysFromNow(-1))
      .replace('Factuur 1', 'Factuur 2026-0001')),
  });
  const r = autoMatch(db, { actor: 'agent:test' });
  assert.equal(r.matched[0].kind, 'entry'); // entry candidate wins over invoice
  assert.equal(getInvoice(db, inv.id).status, 'sent'); // invoice untouched
});

test('bank: partial payment does not auto-match the invoice', () => {
  addContact();
  const inv = createInvoice(db, { contactId: 1, date: '2026-07-10', lines: ['1x Werk @ 100.00 @21'] });
  finalizeInvoice(db, { id: inv.id }); // 121.00 outstanding
  const camt50 = CAMT_DUP.replace('100.00', '50.00').replace('2026-06-01', '2026-07-20');
  importTransactions(db, { iban: IBAN, transactions: parseCamt053(camt50) });
  const r = autoMatch(db, { actor: 'agent:test' });
  assert.equal(r.matched.length, 0); // 50.00 != 121.00 outstanding — no match
  assert.equal(r.unmatched_remaining, 1);
});

// --- VAT edge cases --------------------------------------------------------

test('vat: mixed rates in one entry, monthly period readout', () => {
  bookVatEntry(db, {
    date: '2026-07-01', description: 'Gemengd',
    postings: parseVatPostingSpecs(['1100:131.90,8000:-100.00@21,8000:-10.00@9']), post: true,
  });
  const r = obReadout(db, { period: '2026-07' });
  assert.equal(r.fields['1a'], 10000);
  assert.equal(r.fields['1b'], 1000);
  assert.equal(r.fields['5a'], 2190); // 21.00 + 0.90
  assert.equal(r.fields['5d'], 2190);
});

test('vat: private use (P) -> 1d/5a at the standard rate (21%)', () => {
  bookVatEntry(db, {
    date: '2026-07-01', description: 'Privégebruik',
    postings: parseVatPostingSpecs(['1100:60.50,8000:-50.00@P']), post: true,
  });
  const r = obReadout(db, { period: '2026-07' });
  assert.equal(r.fields['1d'], 5000);
  assert.equal(r.fields['5a'], 1050); // privégebruik is taxed at 21%
  assert.equal(r.fields['5d'], 1050);
});

test('vat: private use (P) VAT is ALWAYS owed (credit 2500) regardless of the posting sign', () => {
  // regression (round 11): the VAT leg used to follow the posting's sign —
  // a DEBIT-signed private-use booking (expense taken privately) produced a
  // 2500 DEBIT that reduced te-betalen and negative 1d/5a. Privégebruik is a
  // deemed supply: you owe (credit 2500), the readout base is always positive.
  //
  // The old masked booking "1100:-121.00,4700:100.00@P" booked a WRONG-SIGN
  // 2500 leg (+21 debit, reducing VAT payable). With the fix the 2500 leg is
  // a -21 credit (you owe) and the readout reports positive 1d/5a; the bank
  // sweep leg absorbs the difference (the engine's documented balancing).
  bookVatEntry(db, {
    date: '2026-07-01', description: 'Privégebruik (old masked form)',
    postings: parseVatPostingSpecs(['1100:-121.00,4700:100.00@P']), post: true,
  });
  const legs = db.prepare(`
    SELECT a.code, p.amount_cents FROM postings p
    JOIN accounts a ON a.id = p.account_id
    WHERE p.entry_id = (SELECT MAX(id) FROM journal_entries)
    ORDER BY a.code
  `).all();
  const vatLeg = legs.find((l) => l.code === '2500');
  assert.ok(vatLeg, 'private use must book a 2500 VAT leg');
  assert.equal(vatLeg.amount_cents, -2100, 'the 2500 leg must be a CREDIT (you owe 21%), never a debit');
  const r = obReadout(db, { period: '2026-07' });
  assert.equal(r.fields['1d'], 10000); // positive base even for debit-signed bookings
  assert.equal(r.fields['5a'], 2100);
  assert.equal(r.fields['5d'], 2100);
});

test('vat: R income (verlegd binnenland sale) reports the base in 1c, no VAT due', () => {
  bookVatEntry(db, {
    date: '2026-07-01', description: 'Verlegd binnenland uitgaand',
    postings: parseVatPostingSpecs(['1100:121.00,8000:-100.00@R']), post: true,
  });
  const r = obReadout(db, { period: '2026-07' });
  // the supply remains taxable in NL — the base goes to 1c ('andere
  // tarieven' incl. 0%), the VAT itself is reversed to the customer
  assert.equal(r.fields['1c'], 10000);
  assert.equal(r.fields['1a'], 0);
  assert.equal(r.fields['2a'], 0);
  assert.equal(r.fields['5a'], 0);
});

// --- year-end / jaarrekening edge cases ------------------------------------

test('year-end: loss year closes with negative result into equity', () => {
  entry('2026-03-01', 'Kosten', [{ code: '4300', amountCents: 5000 }, { code: '1100', amountCents: -5000 }]);
  const r = yearEndClose(db, { year: 2026 });
  assert.equal(r.result_cents, -5000);
  const eq = db.prepare("SELECT COALESCE(SUM(p.amount_cents),0) s FROM postings p JOIN journal_entries e ON e.id=p.entry_id WHERE e.state='posted' AND p.account_id=(SELECT id FROM accounts WHERE code='3000')").get().s;
  assert.equal(eq, 5000); // verlies deblokkeert EV
});

test('year-end: fiscal year end 06-30 drives the jaarrekening as-of date', () => {
  db.prepare("UPDATE company SET fiscal_year_end = '06-30'").run();
  entry('2026-03-01', 'Omzet', [{ code: '1100', amountCents: 12100 }, { code: '8000', amountCents: -10000 }, { code: '2500', amountCents: -2100 }]);
  const r = jaarrekening(db, { year: 2026, model: 'micro' });
  assert.equal(r.as_of, '2026-06-30');
  assert.equal(r.balans.total_activa_cents, 12100);
});

test('jaarrekening: custom account lands in Overig, totals still balance', async () => {
  const { createAccount } = await import('../src/core/accounts.js');
  createAccount(db, { code: '1999', name: 'Crypto', type: 'asset', normalBalance: 'debit', rgsCode: null });
  entry('2026-03-01', 'Omzet', [{ code: '1999', amountCents: 10000 }, { code: '8000', amountCents: -10000 }]);
  const r = jaarrekening(db, { year: 2026, model: 'klein' });
  const overig = r.balans.activa.find((g) => g.label === 'Overig');
  assert.ok(overig);
  assert.equal(overig.total_cents, 10000);
  assert.equal(r.balans.total_activa_cents, r.balans.total_passiva_cents);
});

test('jaarrekening: micro with no activity — zero balans, balanced', () => {
  const r = jaarrekening(db, { year: 2026, model: 'micro' });
  assert.equal(r.balans.total_activa_cents, 0);
  assert.equal(r.balans.total_passiva_cents, 0);
  assert.equal(r.balans.balanced, true);
});

test('year-end: closing two different years works independently', () => {
  entry('2025-03-01', 'Omzet 2025', [{ code: '1100', amountCents: 5000 }, { code: '8000', amountCents: -5000 }]);
  entry('2026-03-01', 'Omzet 2026', [{ code: '1100', amountCents: 8000 }, { code: '8000', amountCents: -8000 }]);
  yearEndClose(db, { year: 2025 });
  yearEndClose(db, { year: 2026 });
  const p26 = pnl(db, { from: '2026-01-01', to: '2026-12-31' });
  assert.equal(p26.revenue_cents, 8000); // 2025 closing does not touch 2026
});

// --- ICP edge cases --------------------------------------------------------

test('icp: credit note reduces the customer total; period boundary respected', () => {
  const de = createContact(db, { name: 'GmbH Berlin', address: 'H 1', city: 'Berlin', country: 'DE', vatId: 'DE123456789', actor: 'a' });
  const inv = createInvoice(db, { contactId: de.id, date: '2026-07-10', lines: ['1x Advies @ 2000.00 @RE'] });
  finalizeInvoice(db, { id: inv.id });
  const credit = creditInvoice(db, { id: inv.id });
  finalizeInvoice(db, { id: credit.id });
  // invoice in Q3, credit note also in Q3 (today 2026-08-04)
  const r = icpReadout(db, { period: '2026-Q3' });
  assert.equal(r.customers[0].amount_cents, 0); // 2000 - 2000
  assert.equal(r.total_cents, 0);
  // Q2 has nothing
  const q2 = icpReadout(db, { period: '2026-Q2' });
  assert.equal(q2.customers.length, 0);
});

test('icp: RE base uses the DISCOUNTED amount (agrees with the OB 2a base)', () => {
  const de = createContact(db, { name: 'GmbH Hamburg', address: 'H 1', city: 'Hamburg', country: 'DE', vatId: 'DE987654321', actor: 'a' });
  // 1000.00 RE with 10% total discount -> discounted base 900.00
  const inv = createInvoice(db, {
    contactId: de.id, date: '2026-07-10', lines: ['1x Levering @ 1000.00 @RE'],
    discountType: 'pct', discountValue: 1000,
  });
  finalizeInvoice(db, { id: inv.id });
  const r = icpReadout(db, { period: '2026-Q3' });
  assert.equal(r.customers[0].amount_cents, 90000); // 900.00, not 1000.00
  // credit note of the discounted invoice subtracts the same discounted base
  const credit = creditInvoice(db, { id: inv.id });
  finalizeInvoice(db, { id: credit.id });
  const r2 = icpReadout(db, { period: '2026-Q3' });
  assert.equal(r2.customers[0].amount_cents, 0);
});

test('fx: setFxRate with a raw float rate parses as 1.0875, not 1.0875 x10000', () => {
  const stored = setFxRate(db, { currency: 'USD', date: '2026-08-01', rate: 1.0875, actor: 'agent:test' });
  assert.equal(stored.rate_x10000, 10875);
  assert.equal(stored.rate, '1.0875');
  assert.equal(convertFx(10000, stored.rate_x10000), 9195); // $100.00 -> EUR 91.95
  // scaled integers still pass through untouched (ECB/MCP path)
  const scaled = setFxRate(db, { currency: 'USD', date: '2026-08-02', rate: 10875, actor: 'agent:test' });
  assert.equal(scaled.rate_x10000, 10875);
});

// --- dry-run hygiene -------------------------------------------------------

test('all mutating paths leave no trace in dry-run', () => {
  addContact();
  const inv = createInvoice(db, { contactId: 1, date: '2026-07-10', lines: ['1x X @ 100.00 @21'] });
  finalizeInvoice(db, { id: inv.id, dryRun: true });
  assert.equal(getInvoice(db, inv.id).invoice_number, null);
  createTemplate(db, { name: 'T', postings: ['4300:10.00,1100:-10.00'], frequency: 'monthly', startDate: '2026-01-01', actor: 'a' });
  runDue(db, { asOf: '2026-12-31', dryRun: true });
  assert.equal(db.prepare("SELECT COUNT(*) c FROM journal_entries WHERE source='recurring'").get().c, 0);
  yearEndClose(db, { year: 2026, dryRun: true });
  assert.equal(db.prepare("SELECT COUNT(*) c FROM journal_entries WHERE source='closing'").get().c, 0);
});

test('ensureDb(mustExist:false) returns null and never creates the database file', () => {
  const dir = mkdtempSync(path.join(os.tmpdir(), 'bukio-nodb-'));
  const missing = path.join(dir, 'absent.db');
  const ctx = { dbPath: missing, json: true, actor: 'human:test', dryRun: true };
  const db = ensureDb(ctx, { mustExist: false });
  assert.equal(db, null, 'a missing DB must yield null, not a handle');
  assert.equal(existsSync(missing), false, 'the file must not be created');
  // the default (mustExist:true) still throws NO_DATABASE
  assert.throws(() => ensureDb(ctx), (e) => e.code === 'NO_DATABASE');
  rmSync(dir, { recursive: true, force: true });
});
