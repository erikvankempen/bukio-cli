/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// v0.13.0 features: items catalog, per-invoice item overrides, fractional
// quantities, line + total discounts with per-rate VAT allocation, VAT
// breakdown, invoice language, company logo.
import { test, beforeEach } from 'node:test';
import assert from 'node:assert/strict';
import { openDb } from '../src/core/db.js';
import { seedDefaultChart } from '../src/core/accounts.js';
import { enableVatModule } from '../src/vat/index.js';
import { formatAmount } from '../src/core/money.js';
import {
  allocateLargestRemainder, buildInvoicePostings, computeInvoiceTotals, createContact,
  createInvoice, creditInvoice, finalizeInvoice, formatQty, getInvoice, parseItemSpec,
  parseLineSpec,
} from '../src/invoice/index.js';
import { createItem, getItem, listItems, updateItem } from '../src/items/index.js';
import { createTemplate, runDue } from '../src/recurring/index.js';
import { invoiceToUbl } from '../src/invoice/ubl.js';
import { invoiceHtml, invoiceToPdf } from '../src/invoice/pdf.js';
import { unitLabel } from '../src/invoice/i18n.js';
import { autoMatch, getOrCreateBankAccount, importTransactions } from '../src/bank/index.js';
import { trialBalance } from '../src/report/trial-balance.js';

let db;

function setupCompany({ vat = true } = {}) {
  db = openDb(':memory:');
  seedDefaultChart(db);
  db.prepare(`
    INSERT INTO company (name, kvk, legal_form, btw_id, iban, address, postal_code, city, vat_module)
    VALUES ('Demo BV', '12345678', 'bv', 'NL123456789B01', 'NL91ABNA0417164300', 'Industrieweg 12', '2712 CD', 'Zoetermeer', ?)
  `).run(vat ? 1 : 0);
  if (vat) enableVatModule(db);
}

beforeEach(() => setupCompany());

function addContact(vatId = 'NL999999999B01') {
  return createContact(db, {
    name: 'ACME B.V.', address: 'Straat 1', postalCode: '1000 AA', city: 'Amsterdam',
    vatId, actor: 'agent:test',
  });
}

function addItem(over = {}) {
  return createItem(db, {
    name: 'Consultancy', unit: 'h', unitPriceCents: 15000, vatCode: '21',
    actor: 'agent:test', ...over,
  });
}

// --- parser + helpers -----------------------------------------------------

test('fractional quantities parse to milli-units', () => {
  assert.equal(parseLineSpec('1.5x Coaching @ 100.00 @9').qtyMilli, 1500);
  assert.equal(parseLineSpec('0.5x Coaching @ 100.00 @9').qtyMilli, 500);
  assert.equal(parseLineSpec('2x Coaching @ 100.00').qtyMilli, 2000);
  assert.equal(formatQty(2000), '2');
  assert.equal(formatQty(1500), '1.5');
  assert.equal(formatQty(1250), '1.25');
  assert.throws(() => parseLineSpec('0x Ding @ 5.00'), { code: 'INVALID_LINE' });
});

test('line discounts parse (pct and amount)', () => {
  const parsed = parseLineSpec('2x Ding @ 10.00 @21 @-10%');
  assert.equal(parsed.qtyMilli, 2000);
  assert.equal(parsed.priceCents, 1000);
  assert.equal(parsed.vatCode, '21');
  assert.equal(parsed.discountType, 'pct');
  assert.equal(parsed.discountValue, 1000);
  const amount = parseLineSpec('Ding @ 10.00 @-2.50');
  assert.equal(amount.discountType, 'amount');
  assert.equal(amount.discountValue, 250);
  // pct > 100 parses but is rejected at creation
  assert.equal(parseLineSpec('Ding @ 10.00 @-101%').discountValue, 10100);
  addContact();
  assert.throws(
    () => createInvoice(db, { contactId: 1, lines: ['Ding @ 10.00 @-101%'], date: '2026-08-10', actor: 'agent:test' }),
    { code: 'INVALID_LINE_DISCOUNT' },
  );
});

test('item specs parse (id, qty, overrides, discount)', () => {
  assert.deepEqual(parseItemSpec('1:2'), { itemId: 1, qtyMilli: 2000, priceCents: null, vatCode: null, discountType: null, discountValue: null });
  const over = parseItemSpec('1:1.5@140.00@21@-10%');
  assert.equal(over.qtyMilli, 1500);
  assert.equal(over.priceCents, 14000);
  assert.equal(over.vatCode, '21');
  assert.equal(over.discountType, 'pct');
  assert.equal(over.discountValue, 1000);
  assert.throws(() => parseItemSpec('x:2'), { code: 'INVALID_ITEM_SPEC' });
});

test('allocateLargestRemainder sums exactly and is deterministic', () => {
  const a = allocateLargestRemainder(100, [700, 200, 100]);
  assert.equal(a.reduce((s, x) => s + x, 0), 100);
  assert.deepEqual(a, allocateLargestRemainder(100, [700, 200, 100]));
  assert.deepEqual(allocateLargestRemainder(0, [1, 2]), [0, 0]);
  const b = allocateLargestRemainder(1, [100, 0]);
  assert.deepEqual(b, [1, 0]); // zero-weight share never gets a cent
});

// --- items catalog --------------------------------------------------------

test('item add/list/show/update/deactivate with audit', () => {
  addContact();
  const item = addItem();
  assert.equal(item.name, 'Consultancy');
  assert.equal(item.unit_price_cents, 15000);

  const list = listItems(db);
  assert.equal(list.length, 1);
  assert.equal(getItem(db, item.id).name, 'Consultancy');

  const updated = updateItem(db, { id: item.id, unitPriceCents: 16000, actor: 'agent:test' });
  assert.equal(updated.unit_price_cents, 16000);
  assert.equal(getItem(db, item.id).active, 1);

  updateItem(db, { id: item.id, deactivate: true, actor: 'agent:test' });
  assert.equal(getItem(db, item.id).active, 0);
  assert.equal(listItems(db).length, 0); // activeOnly by default
  assert.equal(listItems(db, { activeOnly: false }).length, 1);

  const audit = db.prepare("SELECT action FROM audit_log WHERE action LIKE 'item.%' ORDER BY id").all();
  assert.deepEqual(audit.map((r) => r.action), ['item.create', 'item.update', 'item.update']);
});

test('item guards: name/unit/price/vat/account', () => {
  assert.throws(() => createItem(db, { name: '', unit: 'h', unitPriceCents: 100, actor: 'agent:test' }), { code: 'INVALID_NAME' });
  assert.throws(() => createItem(db, { name: 'X', unit: 'weeks', unitPriceCents: 100, actor: 'agent:test' }), { code: 'INVALID_UNIT' });
  assert.throws(() => createItem(db, { name: 'X', unit: 'h', unitPriceCents: 0, actor: 'agent:test' }), { code: 'INVALID_PRICE' });
  assert.throws(() => createItem(db, { name: 'X', unit: 'h', unitPriceCents: 100, vatCode: '999', actor: 'agent:test' }), { code: 'VAT_CODE_NOT_FOUND' });
  assert.throws(() => createItem(db, { name: 'X', unit: 'h', unitPriceCents: 100, glAccount: '9999', actor: 'agent:test' }), { code: 'ACCOUNT_NOT_FOUND' });
  assert.throws(() => updateItem(db, { id: 999, actor: 'agent:test' }), { code: 'ITEM_NOT_FOUND' });
  // dry-run writes nothing
  const plan = createItem(db, { name: 'Dry', unit: 'h', unitPriceCents: 100, actor: 'agent:test', dryRun: true });
  assert.equal(plan.dryRun, true);
  assert.equal(listItems(db).length, 0);
});

test('item without a VAT code is allowed when the VAT module is off', () => {
  setupCompany({ vat: false });
  const item = createItem(db, { name: 'Coaching', unit: 'session', unitPriceCents: 7500, actor: 'agent:test' });
  assert.equal(item.vat_code, null);
  assert.throws(() => createItem(db, { name: 'X', unit: 'h', unitPriceCents: 100, vatCode: '21', actor: 'agent:test' }), { code: 'VAT_MODULE_OFF' });
});

test('unit labels localize', () => {
  assert.equal(unitLabel('h', 'nl'), 'uur');
  assert.equal(unitLabel('h', 'en'), 'h');
  assert.equal(unitLabel('month', 'en'), 'month');
  assert.equal(unitLabel('unit', 'nl'), 'stuks');
});

// --- invoices from items --------------------------------------------------

test('invoice create --items snapshots catalog values', () => {
  const c = addContact();
  const item = addItem({ name: 'Advisory', description: 'Ad-hoc advisory', unitPriceCents: 20000 });
  const inv = createInvoice(db, { contactId: c.id, items: [`${item.id}:2`], date: '2026-08-10', actor: 'agent:test' });
  assert.equal(inv.lines.length, 1);
  const l = inv.lines[0];
  assert.equal(l.description, 'Ad-hoc advisory');
  assert.equal(l.quantity, 2000);
  assert.equal(l.unit, 'h');
  assert.equal(l.item_id, item.id);
  assert.equal(l.unit_price_cents, 20000);
  assert.equal(l.amount_cents, 40000);
  assert.equal(inv.net_cents, 40000);
  assert.equal(inv.vat_cents, 8400); // 400.00 @21
  assert.equal(inv.gross_cents, 48400);

  // price edits after creation never rewrite the invoice
  updateItem(db, { id: item.id, unitPriceCents: 99900, actor: 'agent:test' });
  assert.equal(getInvoice(db, inv.id).lines[0].unit_price_cents, 20000);
});

test('invoice create --items per-invoice overrides (price, VAT, discount)', () => {
  const c = addContact();
  const item = addItem({ unitPriceCents: 20000 }); // catalog 200.00
  const inv = createInvoice(db, {
    contactId: c.id, items: [`${item.id}:1.5@180.00@9@-10%`], date: '2026-08-10', actor: 'agent:test',
  });
  const l = inv.lines[0];
  assert.equal(l.unit_price_cents, 18000); // override, not catalog
  assert.equal(l.vat_code, '9');
  assert.equal(l.quantity, 1500);
  assert.equal(l.discount_type, 'pct');
  assert.equal(l.discount_value, 1000);
  // 1.5 × 180.00 = 270.00, −10% = 243.00 net, 9% vat = 21.87
  assert.equal(inv.net_cents, 24300);
  assert.equal(inv.vat_cents, 2187);
  assert.equal(inv.gross_cents, 26487);
  // catalog untouched
  assert.equal(getItem(db, item.id).unit_price_cents, 20000);
});

test('item guards on invoices: unknown, inactive, bad override, conflicting sources', () => {
  const c = addContact();
  const item = addItem();
  assert.throws(() => createInvoice(db, { contactId: c.id, items: ['999:1'], date: '2026-08-10', actor: 'agent:test' }), { code: 'ITEM_NOT_FOUND' });
  assert.throws(() => createInvoice(db, { contactId: c.id, items: [`${item.id}:1@0.00`], date: '2026-08-10', actor: 'agent:test' }), { code: 'INVALID_ITEM_OVERRIDE' });
  assert.throws(() => createInvoice(db, { contactId: c.id, items: [`${item.id}:1`], lines: ['X @ 5.00'], date: '2026-08-10', actor: 'agent:test' }), { code: 'CONFLICTING_LINES' });
  updateItem(db, { id: item.id, deactivate: true, actor: 'agent:test' });
  assert.throws(() => createInvoice(db, { contactId: c.id, items: [`${item.id}:1`], date: '2026-08-10', actor: 'agent:test' }), { code: 'ITEM_INACTIVE' });
});

// --- fractional quantities + line discounts -------------------------------

test('fractional quantity line math (1.5h @ 100 = 150.00)', () => {
  const c = addContact();
  const inv = createInvoice(db, { contactId: c.id, lines: ['1.5x Coaching @ 100.00 @9'], date: '2026-08-10', actor: 'agent:test' });
  assert.equal(inv.lines[0].quantity, 1500);
  assert.equal(inv.lines[0].amount_cents, 15000);
  assert.equal(inv.net_cents, 15000);
  assert.equal(inv.vat_cents, 1350); // 9%
});

test('line discount pct and amount reduce net and VAT', () => {
  const c = addContact();
  const pct = createInvoice(db, { contactId: c.id, lines: ['2x Ding @ 100.00 @21 @-10%'], date: '2026-08-10', actor: 'agent:test' });
  assert.equal(pct.lines[0].discount_type, 'pct');
  assert.equal(pct.lines[0].discount_value, 1000);
  assert.equal(pct.net_cents, 18000); // 200 − 20
  assert.equal(pct.vat_cents, 3780);  // 21% of 180
  assert.equal(pct.discount_cents, 0); // line discounts are per-line; invoice-level total is 0

  const amt = createInvoice(db, { contactId: c.id, lines: ['2x Ding @ 100.00 @21 @-25.00'], date: '2026-08-10', actor: 'agent:test' });
  assert.equal(amt.net_cents, 17500);
  assert.equal(amt.vat_cents, 3675);
  assert.equal(amt.discount_cents, 0); // per-line only; invoice-level total is 0

  // discount ≥ line amount is rejected
  assert.throws(
    () => createInvoice(db, { contactId: c.id, lines: ['2x Ding @ 100.00 @-250.00'], date: '2026-08-10', actor: 'agent:test' }),
    { code: 'INVALID_LINE_DISCOUNT' },
  );
});

// --- total discount + per-rate VAT allocation -----------------------------

test('total discount: single rate, pct and amount', () => {
  const c = addContact();
  const pct = createInvoice(db, {
    contactId: c.id, lines: ['2x Ding @ 100.00 @21'], date: '2026-08-10',
    discountType: 'pct', discountValue: 500, actor: 'agent:test', // 5%
  });
  assert.equal(pct.discount_cents, 1000); // 5% of 200.00
  assert.equal(pct.net_cents, 19000);
  assert.equal(pct.vat_cents, 3990); // 21% of 190.00
  assert.equal(pct.gross_cents, 22990);

  const amt = createInvoice(db, {
    contactId: c.id, lines: ['2x Ding @ 100.00 @21'], date: '2026-08-10',
    discountType: 'amount', discountValue: 1000, actor: 'agent:test',
  });
  assert.equal(amt.net_cents, 19000);
  assert.equal(amt.vat_cents, 3990);
  assert.equal(amt.discount_cents, 1000);
});

test('total discount across mixed VAT rates allocates to the cent', () => {
  const c = addContact();
  // 21% line: 300.00 | 9% line: 200.00 | 0% line: 100.00 — total 600.00
  const inv = createInvoice(db, {
    contactId: c.id,
    lines: ['3x Hoog @ 100.00 @21', '2x Laag @ 100.00 @9', '1x Nul @ 100.00 @0'],
    date: '2026-08-10',
    discountType: 'amount', discountValue: 6000, actor: 'agent:test', // 60.00 off total
  });
  assert.equal(inv.discount_cents, 6000);
  assert.equal(inv.net_cents, 54000); // 600 − 60
  // allocation: 60 × (300/600)=30, (200/600)=20, (100/600)=10 → exact thirds
  assert.equal(inv.vat_cents, 7290); // 21% of 270 + 9% of 180 = 56.70 + 16.20
  assert.equal(inv.gross_cents, 61290);
  // per-line VAT sums exactly to the invoice VAT
  const lineVat = inv.lines.reduce((s, l) => s + l.vat_amount_cents, 0);
  assert.equal(lineVat, inv.vat_cents);
  // breakdown reconciles: the VAT block covers only rates that charge VAT
  // (the 0% line's 90.00 base is outside the breakdown, but inside net)
  const sumBase = inv.vat_breakdown.reduce((s, b) => s + b.base_cents, 0);
  const sumVat = inv.vat_breakdown.reduce((s, b) => s + b.vat_cents, 0);
  assert.equal(sumBase, 45000); // 270 + 180 (21% + 9% bases only)
  assert.equal(sumVat, inv.vat_cents);
});

test('total discount with awkward split still balances (largest remainder)', () => {
  const c = addContact();
  // 21% line 101.00 + 9% line 1.00 → discount 1.00 must split 0.99/0.01
  const inv = createInvoice(db, {
    contactId: c.id, lines: ['1x A @ 101.00 @21', '1x B @ 1.00 @9'],
    date: '2026-08-10', discountType: 'amount', discountValue: 100, actor: 'agent:test',
  });
  assert.equal(inv.discount_cents, 100);
  assert.equal(inv.net_cents, 10100);
  assert.equal(inv.vat_cents, Math.round(10000 * 0.21) + Math.round(100 * 0.09)); // 2100 + 9
  assert.equal(inv.vat_cents, 2109);
  const lineVat = inv.lines.reduce((s, l) => s + l.vat_amount_cents, 0);
  assert.equal(lineVat, 2109);
});

test('computeInvoiceTotals is deterministic across recomputes (getInvoice consistency)', () => {
  const c = addContact();
  const inv = createInvoice(db, {
    contactId: c.id, lines: ['3x A @ 33.33 @21', '2x B @ 12.50 @9', '1x C @ 7.77 @21'],
    date: '2026-08-10', discountType: 'pct', discountValue: 750, actor: 'agent:test',
  });
  const fresh = getInvoice(db, inv.id);
  assert.equal(fresh.net_cents, inv.net_cents);
  assert.equal(fresh.vat_cents, inv.vat_cents);
  assert.deepEqual(fresh.vat_breakdown, inv.vat_breakdown);
});

test('booking with discounts: omzet uses discounted nets, VAT per rate', () => {
  const c = addContact();
  const inv = createInvoice(db, {
    contactId: c.id, lines: ['3x Hoog @ 100.00 @21', '2x Laag @ 100.00 @9'],
    date: '2026-08-10', discountType: 'amount', discountValue: 5000, actor: 'agent:test',
  });
  const postings = buildInvoicePostings(db, inv);
  // omzet legs: 21% group 300 → 270 (alloc 30 of the 50 discount), 9% group 200 → 180
  const omzet = postings.filter((p) => p.code === '8000');
  assert.equal(omzet.length, 2);
  assert.equal(omzet.reduce((s, p) => s + p.amountCents, 0), -45000); // 270 + 180
  const vatLeg = postings.find((p) => p.code === '2500');
  assert.equal(vatLeg.amountCents, -inv.vat_cents);
  assert.equal(vatLeg.amountCents, -7290); // 21% of 270 + 9% of 180
});

test('finalize with discounts books a balanced entry', () => {
  const c = addContact();
  const inv = createInvoice(db, {
    contactId: c.id, lines: ['2x Ding @ 100.00 @21'], date: '2026-08-10',
    discountType: 'pct', discountValue: 1000, actor: 'agent:test',
  });
  const result = finalizeInvoice(db, { id: inv.id, actor: 'agent:test' });
  assert.equal(result.invoice.invoice_number, '2026-0001');
  assert.equal(result.invoice.gross_cents, 21780); // (200−20) + 21% of 180
  const entry = db.prepare('SELECT * FROM journal_entries WHERE id = ?').get(result.entry.id);
  assert.equal(entry.state, 'posted');
  const tb = db.prepare(`SELECT SUM(amount_cents) s FROM postings WHERE entry_id = ?`).get(result.entry.id);
  assert.equal(tb.s, 0); // balanced
});

// --- language -------------------------------------------------------------

test('invoice language: nl default, en allowed, invalid rejected', () => {
  const c = addContact();
  const inv = createInvoice(db, { contactId: c.id, lines: ['Ding @ 10.00'], date: '2026-08-10', actor: 'agent:test' });
  assert.equal(inv.language, 'nl');
  const en = createInvoice(db, { contactId: c.id, lines: ['Ding @ 10.00'], date: '2026-08-11', language: 'en', actor: 'agent:test' });
  assert.equal(en.language, 'en');
  assert.throws(
    () => createInvoice(db, { contactId: c.id, lines: ['Ding @ 10.00'], date: '2026-08-11', language: 'de', actor: 'agent:test' }),
    { code: 'INVALID_LANGUAGE' },
  );
});

test('CLI: --discount-pct and --discount-amount together are rejected', () => {
  const dir = mkdtempSync(path.join(tmpdir(), 'bukio-disc-'));
  const dbPath = path.join(dir, 'test.db');
  runCli(['init', '--name', 'Demo BV', '--kvk', '12345678', '--vat', 'on'], dbPath);
  runCli(['contact', 'add', '--name', 'ACME B.V.', '--address', 'Straat 1', '--city', 'Amsterdam'], dbPath);
  assertCliError(
    ['invoice', 'create', '--contact', '1', '--lines', '1x Ding @ 10.00', '--date', '2026-08-10',
      '--discount-pct', '5', '--discount-amount', '5.00'],
    dbPath, /INVALID_DISCOUNT/,
  );
});

test('credit note inherits language, total discount and line discounts', () => {
  const c = addContact();
  const inv = createInvoice(db, {
    contactId: c.id, lines: ['2x Ding @ 100.00 @21 @-10%'], date: '2026-08-10',
    discountType: 'pct', discountValue: 500, language: 'en', actor: 'agent:test',
  });
  finalizeInvoice(db, { id: inv.id, actor: 'agent:test' });
  const credit = creditInvoice(db, { id: inv.id, actor: 'agent:test' });
  assert.equal(credit.language, 'en');
  assert.equal(credit.discount_type, 'pct');
  assert.equal(credit.discount_value, 500);
  assert.equal(credit.lines[0].discount_type, 'pct');
  assert.equal(credit.net_cents, inv.net_cents);
  assert.equal(credit.vat_cents, inv.vat_cents);
});

// --- UBL with the new fields ----------------------------------------------

test('UBL: formatted quantity, unit code, language, discounted tax bases', () => {
  const c = addContact();
  const inv = createInvoice(db, {
    contactId: c.id, items: null,
    lines: ['1.5x Consultancy @ 100.00 @21 @-10%'],
    date: '2026-08-10', discountType: 'pct', discountValue: 1000, language: 'en', actor: 'agent:test',
  });
  finalizeInvoice(db, { id: inv.id, actor: 'agent:test' });
  const xml = invoiceToUbl(db, getInvoice(db, inv.id));
  assert.match(xml, /<cbc:InvoicedQuantity unitCode="C62">1.5<\/cbc:InvoicedQuantity>/);
  assert.match(xml, /<cbc:LanguageID>en<\/cbc:LanguageID>/);
  // line allowance for the 10% line discount: 15.00
  assert.match(xml, /<cac:AllowanceCharge>[\s\S]*?<cbc:ChargeIndicator>false<\/cbc:ChargeIndicator>[\s\S]*?<cbc:Amount currencyID="EUR">15.00<\/cbc:Amount>/);
  // EN 16931 BT-131: LineExtensionAmount is net of the line allowance (BR-26)
  assert.match(xml, /<cbc:LineExtensionAmount currencyID="EUR">135.00<\/cbc:LineExtensionAmount>/);
  // BT-107 covers ONLY the document-level allowance (13.50) — line discounts
  // are folded into the line net amounts
  assert.match(xml, /<cbc:AllowanceTotalAmount currencyID="EUR">13.50<\/cbc:AllowanceTotalAmount>/);
  // TaxExclusiveAmount = discounted net 121.50
  assert.match(xml, /<cbc:TaxExclusiveAmount currencyID="EUR">121.50<\/cbc:TaxExclusiveAmount>/);
  // taxable base in the TaxSubtotal is the discounted base
  assert.match(xml, /<cbc:TaxableAmount currencyID="EUR">121.50<\/cbc:TaxableAmount>/);
});

test('UBL: line-only discounts net LineExtensionAmount (BR-26); no doc allowance emitted; @V maps to E, @0 to Z', () => {
  const c = addContact();
  // line discount 10% (15.00) but NO total discount
  const inv = createInvoice(db, {
    contactId: c.id, lines: ['1.5x Consultancy @ 100.00 @21 @-10%'], date: '2026-08-10',
    actor: 'agent:test',
  });
  finalizeInvoice(db, { id: inv.id, actor: 'agent:test' });
  const xml = invoiceToUbl(db, getInvoice(db, inv.id));
  // line discount folds into the line net amount: 150 − 15 = 135
  assert.match(xml, /<cbc:LineExtensionAmount currencyID="EUR">135.00<\/cbc:LineExtensionAmount>/);
  // no document-level allowance -> no AllowanceTotalAmount element
  assert.doesNotMatch(xml, /<cbc:AllowanceTotalAmount/);
  assert.match(xml, /<cbc:TaxExclusiveAmount currencyID="EUR">135.00<\/cbc:TaxExclusiveAmount>/);

  // category mapping: @0 zero-rated -> Z, @V vrijgesteld -> E, @21 -> S
  const c2 = addContact('NL999999999B01');
  const inv2 = createInvoice(db, {
    contactId: c2.id,
    lines: ['1x Nul @ 10.00 @0', '1x Vrij @ 10.00 @V', '1x Normaal @ 10.00 @21'],
    date: '2026-08-10', actor: 'agent:test',
  });
  finalizeInvoice(db, { id: inv2.id, actor: 'agent:test' });
  const xml2 = invoiceToUbl(db, getInvoice(db, inv2.id));
  assert.match(xml2, /<cbc:ID>Z<\/cbc:ID>/);
  assert.match(xml2, /<cbc:ID>E<\/cbc:ID>/);
  assert.match(xml2, /<cbc:ID>S<\/cbc:ID>/);
});

test('UBL: zero-VAT categories (RE/V) still emit TaxSubtotal — EN 16931 1..n', () => {
  const c = addContact();
  const inv = createInvoice(db, {
    contactId: c.id,
    lines: ['1x Dienst @ 500.00 @RE', '1x Vrij @ 100.00 @V'],
    date: '2026-08-10', actor: 'agent:test',
  });
  finalizeInvoice(db, { id: inv.id, actor: 'agent:test' });
  const xml = invoiceToUbl(db, getInvoice(db, inv.id));
  // AE subtotal: verlegd base 500.00 with zero VAT
  assert.match(xml, /<cac:TaxSubtotal>[\s\S]*?<cbc:TaxableAmount currencyID="EUR">500\.00<\/cbc:TaxableAmount>[\s\S]*?<cbc:TaxAmount currencyID="EUR">0\.00<\/cbc:TaxAmount>[\s\S]*?<cbc:ID>AE<\/cbc:ID>/);
  // E subtotal: vrijgesteld base 100.00 with zero VAT
  assert.match(xml, /<cac:TaxSubtotal>[\s\S]*?<cbc:TaxableAmount currencyID="EUR">100\.00<\/cbc:TaxableAmount>[\s\S]*?<cbc:TaxAmount currencyID="EUR">0\.00<\/cbc:TaxAmount>[\s\S]*?<cbc:ID>E<\/cbc:ID>/);
  // totals unchanged and reconcile: 500 + 100, no VAT
  assert.match(xml, /<cbc:TaxExclusiveAmount currencyID="EUR">600\.00<\/cbc:TaxExclusiveAmount>/);
  assert.match(xml, /<cbc:TaxInclusiveAmount currencyID="EUR">600\.00<\/cbc:TaxInclusiveAmount>/);
});

test('UBL: hour unit maps to HUR', () => {
  const c = addContact();
  const inv = createInvoice(db, {
    contactId: c.id, items: null, lines: ['2x Coaching @ 50.00 @21'],
    date: '2026-08-10', actor: 'agent:test',
  });
  // force a unit on the stored line (normally set via --items)
  db.prepare('UPDATE invoice_lines SET unit = ? WHERE invoice_id = ?').run('h', inv.id);
  const xml = invoiceToUbl(db, getInvoice(db, inv.id));
  assert.match(xml, /unitCode="HUR">2<\/cbc:InvoicedQuantity>/);
});

// --- PDF layout (i18n, units, VAT breakdown, logo) -------------------------

test('PDF: Dutch labels, unit column, VAT breakdown, discount row', () => {
  const c = addContact();
  const inv = createInvoice(db, {
    contactId: c.id,
    lines: ['2x Consultancy @ 100.00 @21 @-10%', '1x Maand @ 50.00 @9'],
    date: '2026-08-10', discountType: 'pct', discountValue: 500, actor: 'agent:test',
  });
  // unit codes for the layout test
  db.prepare('UPDATE invoice_lines SET unit = ? WHERE invoice_id = ? AND line_no = 1').run('h', inv.id);
  const html = invoiceHtml(db, getInvoice(db, inv.id));
  assert.match(html, /FACTUUR/);
  assert.match(html, /Factuur aan/);
  assert.match(html, /Omschrijving/);
  assert.match(html, /Aantal/);
  assert.match(html, /Eenheid/); // unit column header
  assert.match(html, /Btw over 21%/);
  assert.match(html, /Btw over 9%/);
  assert.match(html, /Totaal btw/);
  assert.match(html, /Korting/);
  assert.match(html, /Totaal \(incl\. btw\)/);
  assert.match(html, />2</); // formatted quantity, not 2000
  assert.match(html, />uur</); // localized unit
  assert.doesNotMatch(html, /2000<\/td>/); // milli never leaks to the PDF
  // money reconciliation in the rendered totals
  const totals = computeInvoiceTotals(inv.lines, inv.discount_type, inv.discount_value);
  assert.match(html, new RegExp(formatAmount(totals.net_before_cents)));
  assert.match(html, new RegExp(formatAmount(totals.gross_cents)));
});

test('PDF: English labels + reverse-charge wording', () => {
  const c = addContact('NL999999999B01');
  const inv = createInvoice(db, {
    contactId: c.id, lines: ['1x Ding @ 100.00 @21'], date: '2026-08-10',
    language: 'en', actor: 'agent:test',
  });
  const html = invoiceHtml(db, inv);
  assert.match(html, /INVOICE/);
  assert.match(html, /Billed to/);
  assert.match(html, /Description/);
  assert.match(html, /Qty/);
  assert.match(html, /Unit/);
  assert.match(html, /Subtotal excl\. VAT/);
  assert.match(html, /Total \(incl\. VAT\)/);
});

test('PDF: company logo renders as a data URI in the header', async (t) => {
  const c = addContact();
  const inv = createInvoice(db, { contactId: c.id, lines: ['1x Ding @ 10.00'], date: '2026-08-10', actor: 'agent:test' });
  db.prepare('UPDATE company SET logo = ?, logo_mime = ? WHERE id = 1').run(pngBytes(120, 60), 'image/png');
  const html = invoiceHtml(db, inv);
  assert.match(html, /<img class="logo" src="data:image\/png;base64,/);
  // and without a logo there is no <img>
  db.prepare('UPDATE company SET logo = NULL, logo_mime = NULL WHERE id = 1').run();
  assert.doesNotMatch(invoiceHtml(db, inv), /<img/);
});

test('PDF: renders through Chromium (skipped when no browser installed)', async (t) => {
  const c = addContact();
  const inv = createInvoice(db, {
    contactId: c.id, lines: ['1.5x Consultancy @ 100.00 @21 @-10%'], date: '2026-08-10',
    discountType: 'pct', discountValue: 1000, language: 'en', actor: 'agent:test',
  });
  const dir = mkdtempSync(path.join(tmpdir(), 'bukio-pdf-'));
  try {
    const result = await invoiceToPdf(db, inv, { outPath: path.join(dir, 'inv.pdf') });
    assert.ok(result.bytes > 1000);
    assert.ok(readFileSync(path.join(dir, 'inv.pdf')).length > 1000);
  } catch (err) {
    if (err.code === 'PDF_UNAVAILABLE') {
      t.skip(`Chromium not available: ${err.message}`);
      return;
    }
    throw err;
  }
});

// --- recurring items + MCP -------------------------------------------------

test('recurring invoice template with items snapshots catalog prices per run', () => {
  const c = addContact();
  const item = addItem({ name: 'SaaS', unitPriceCents: 9900, vatCode: '21' });
  const tpl = createTemplate(db, {
    name: 'SaaS abonnement', kind: 'invoice', contactId: c.id,
    invoiceItems: [`${item.id}:1`], frequency: 'monthly', startDate: '2026-08-01',
    dueDays: 14, actor: 'agent:test',
  });
  assert.equal(tpl.invoice_items.length, 1);
  assert.equal(tpl.vat_aware, 1);

  runDue(db, { asOf: '2026-08-31', actor: 'agent:test' });
  let inv = getInvoice(db, db.prepare('SELECT id FROM invoices ORDER BY id LIMIT 1').get().id);
  assert.equal(inv.lines[0].item_id, item.id);
  assert.equal(inv.lines[0].unit_price_cents, 9900);
  assert.equal(inv.lines[0].quantity, 1000);

  // a price change applies from the NEXT run (snapshot semantics)
  updateItem(db, { id: item.id, unitPriceCents: 11900, actor: 'agent:test' });
  runDue(db, { asOf: '2026-09-30', actor: 'agent:test' });
  const sep = getInvoice(db, db.prepare("SELECT id FROM invoices WHERE date = '2026-09-01'").get().id);
  assert.equal(sep.lines[0].unit_price_cents, 11900);
  const aug = getInvoice(db, db.prepare("SELECT id FROM invoices WHERE date = '2026-08-01'").get().id);
  assert.equal(aug.lines[0].unit_price_cents, 9900); // past drafts untouched
});

// --- MCP tools (items + extended invoice_create) --------------------------

import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const REPO_ROOT = path.resolve(import.meta.dirname, '..');

function mcpSession(dbPath) {
  const child = spawn(process.execPath, ['bin/bukio.js', 'mcp', '--db', dbPath], {
    cwd: REPO_ROOT,
    env: { ...process.env, BUKIO_ACTOR: 'agent:test' },
  });
  let buf = '';
  const pending = [];
  const waiters = [];
  child.stdout.on('data', (d) => {
    buf += d.toString();
    let nl;
    while ((nl = buf.indexOf('\n')) >= 0) {
      const line = buf.slice(0, nl);
      buf = buf.slice(nl + 1);
      const msg = JSON.parse(line);
      if (waiters.length) waiters.shift()(msg);
      else pending.push(msg);
    }
  });
  const next = () => (pending.length ? Promise.resolve(pending.shift()) : new Promise((res) => waiters.push(res)));
  return {
    child,
    call(method, params = {}, id = Math.floor(Math.random() * 1e9)) {
      child.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', id, method, params })}\n`);
      return next();
    },
    close() {
      child.stdin.end();
      return new Promise((res) => child.on('exit', res));
    },
  };
}

test('MCP: item_add/item_list/item_update + invoice_create with items/discount/language', async () => {
  const dir = mkdtempSync(path.join(tmpdir(), 'bukio-mcp-features-'));
  const dbPath = path.join(dir, 'test.db');
  runCli(['init', '--name', 'Demo BV', '--kvk', '12345678', '--vat', 'on'], dbPath);
  runCli(['contact', 'add', '--name', 'ACME B.V.', '--address', 'Straat 1', '--city', 'Amsterdam'], dbPath);
  const mcp = mcpSession(dbPath);
  try {
    const added = await mcp.call('tools/call', {
      name: 'item_add', arguments: {
        name: 'Consultancy', unit: 'h', unit_price: '150.00', vat_code: '21',
        mode: 'execute', actor: 'agent:mcp-test',
      },
    });
    assert.ok(added.result.content[0].text.includes('"action": "item.create"'), 'item_add must execute');

    const list = await mcp.call('tools/call', { name: 'item_list', arguments: {} });
    assert.ok(list.result.content[0].text.includes('Consultancy'), 'item_list must show the item');

    const inv = await mcp.call('tools/call', {
      name: 'invoice_create', arguments: {
        contact_id: 1, items: ['1:2@140.00'], date: '2026-08-10', discount_pct: 5,
        language: 'en', mode: 'execute', actor: 'agent:mcp-test',
      },
    });
    const parsed = JSON.parse(inv.result.content[0].text);
    assert.equal(parsed.invoice_id, 1);
    assert.equal(parsed.totals.discount, 1400); // 5% of 280.00
    assert.equal(parsed.totals.net, 26600);
    assert.equal(parsed.totals.vat, 5586); // 21% of 266.00

    const deact = await mcp.call('tools/call', {
      name: 'item_update', arguments: { id: 1, deactivate: true, mode: 'execute', actor: 'agent:mcp-test' },
    });
    assert.ok(deact.result.content[0].text.includes('"active": false'), 'item_update must deactivate');

    // item_add dry-run writes nothing
    const plan = await mcp.call('tools/call', {
      name: 'item_add', arguments: { name: 'X', unit_price: '10.00', mode: 'dry-run' },
    });
    assert.ok(plan.result.content[0].text.includes('"dryRun": true'));
  } finally {
    await mcp.close();
  }
  const check = openDb(dbPath);
  assert.equal(check.prepare('SELECT COUNT(*) c FROM items').get().c, 1); // dry-run wrote nothing
  check.close();
});

test('bank autoMatch: incoming payment matches a DISCOUNTED invoice at its discounted gross', () => {
  const c = addContact();
  // 2x 100 @21, 10% total discount -> gross 217.80 (line sums would say 242.00)
  const inv = createInvoice(db, {
    contactId: c.id, lines: ['2x Dienst @ 100.00 @21'], date: '2026-08-01',
    discountType: 'pct', discountValue: 1000, actor: 'agent:test',
  });
  finalizeInvoice(db, { id: inv.id, actor: 'agent:test' });
  assert.equal(getInvoice(db, inv.id).gross_cents, 21780);
  getOrCreateBankAccount(db, { iban: 'NL91ABNA0417164300' });
  importTransactions(db, {
    iban: 'NL91ABNA0417164300',
    transactions: [{ date: '2026-08-05', amount_cents: 21780, counterparty: 'ACME B.V.', description: 'betaling factuur' }],
    actor: 'agent:test',
  });
  const dry = autoMatch(db, { actor: 'agent:test', dryRun: true });
  assert.equal(dry.matched.length, 1);
  assert.equal(dry.matched[0].kind, 'invoice');
  assert.equal(dry.matched[0].fx_delta_cents, 0);
  // execute: marks paid, posts Bank/Debiteuren, balances
  autoMatch(db, { actor: 'agent:test' });
  const paid = getInvoice(db, inv.id);
  assert.equal(paid.status, 'paid');
  assert.equal(paid.paid_cents, 21780);
  assert.equal(trialBalance(db).balanced, true);
});

test('bank autoMatch: discounted invoice does NOT match a partial/off payment', () => {
  const c = addContact();
  const inv = createInvoice(db, {
    contactId: c.id, lines: ['2x Dienst @ 100.00 @21'], date: '2026-08-01',
    discountType: 'pct', discountValue: 1000, actor: 'agent:test',
  });
  finalizeInvoice(db, { id: inv.id, actor: 'agent:test' });
  getOrCreateBankAccount(db, { iban: 'NL91ABNA0417164300' });
  importTransactions(db, {
    iban: 'NL91ABNA0417164300',
    transactions: [{ date: '2026-08-05', amount_cents: 24200, counterparty: 'ACME B.V.', description: 'pre-discount amount' }],
    actor: 'agent:test',
  });
  const dry = autoMatch(db, { actor: 'agent:test', dryRun: true });
  assert.equal(dry.matched.length, 0); // 242.00 != 217.80 and outside tolerance
});

// --- company logo ---------------------------------------------------------

function pngBytes(width, height) {
  // minimal PNG: signature + IHDR (dims at byte 16/20) + IEND
  const buf = Buffer.alloc(33);
  Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]).copy(buf, 0);
  buf.writeUInt32BE(13, 8);
  buf.write('IHDR', 12);
  buf.writeUInt32BE(width, 16);
  buf.writeUInt32BE(height, 20);
  buf.writeUInt32BE(0, 24); // CRC placeholder
  return buf;
}

function jpegBytes(width, height) {
  // minimal JPEG: SOI + APP0 + SOF0 with dims
  const buf = Buffer.alloc(41);
  buf.writeUInt16BE(0xffd8, 0);
  buf.writeUInt16BE(0xffe0, 2);
  buf.writeUInt16BE(16, 4);
  buf.write('JFIF\0', 6);
  buf.writeUInt16BE(0xffc0, 20);
  buf.writeUInt16BE(17, 22);
  buf[24] = 8; // precision
  buf.writeUInt16BE(height, 25);
  buf.writeUInt16BE(width, 27);
  return buf;
}

function svgBytes(body) {
  return Buffer.from(`<svg xmlns="http://www.w3.org/2000/svg" ${body}></svg>`);
}

import { readFileSync, writeFileSync, mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { execFileSync } from 'node:child_process';

function runCli(args, dbPath) {
  const out = execFileSync('node', ['bin/bukio.js', '--db', dbPath, '--actor', 'agent:test', '--json', ...args], {
    encoding: 'utf8', cwd: path.resolve(import.meta.dirname, '..'),
  });
  return JSON.parse(out);
}

/** Assert a CLI call fails with a code (regex over the JSON stdout). */
function assertCliError(args, dbPath, codeRe) {
  try {
    runCli(args, dbPath);
    assert.fail(`expected ${args.join(' ')} to fail with ${codeRe}`);
  } catch (err) {
    assert.match(err.stdout ?? '', codeRe);
  }
}

test('company logo: set (PNG), extract round-trip, remove; audits', () => {
  const dir = mkdtempSync(path.join(tmpdir(), 'bukio-logo-'));
  const logo = path.join(dir, 'logo.png');
  writeFileSync(logo, pngBytes(200, 80));
  const dbPath = path.join(dir, 'test.db');

  runCli(['init', '--name', 'Demo BV', '--kvk', '12345678', '--vat', 'on'], dbPath);
  const set = runCli(['company', 'update', '--logo', logo], dbPath);
  assert.equal(set.data.company.logo_mime, 'image/png');
  assert.equal(set.data.company.logo_bytes, 33);

  const extract = path.join(dir, 'out.png');
  runCli(['company', 'logo', '--out', extract], dbPath);
  assert.deepEqual(readFileSync(extract), pngBytes(200, 80)); // byte-identical round-trip

  runCli(['company', 'update', '--remove-logo'], dbPath);
  const show = runCli(['company', 'show'], dbPath);
  assert.equal(show.data.company.logo_mime, null);
  assert.equal(show.data.company.logo_bytes, null);
});

test('company logo: format, size and dimension guards', () => {
  const dir = mkdtempSync(path.join(tmpdir(), 'bukio-logo-'));
  const dbPath = path.join(dir, 'test.db');
  runCli(['init', '--name', 'Demo BV', '--kvk', '12345678', '--vat', 'on'], dbPath);

  const bad = path.join(dir, 'logo.txt');
  writeFileSync(bad, 'not an image at all');
  assertCliError(['company', 'update', '--logo', bad], dbPath, /LOGO_UNSUPPORTED_FORMAT/);

  const big = path.join(dir, 'big.png');
  writeFileSync(big, pngBytes(4096, 100));
  assertCliError(['company', 'update', '--logo', big], dbPath, /LOGO_DIMENSIONS_TOO_LARGE/);

  const huge = path.join(dir, 'huge.png');
  writeFileSync(huge, Buffer.concat([pngBytes(100, 100), Buffer.alloc(1_100_000)]));
  assertCliError(['company', 'update', '--logo', huge], dbPath, /LOGO_TOO_LARGE/);

  assertCliError(['company', 'update', '--logo', path.join(dir, 'nope.png')], dbPath, /LOGO_FILE_NOT_FOUND/);

  // JPEG + SVG accepted; SVG dims parsed from width/height
  const jpg = path.join(dir, 'logo.jpg');
  writeFileSync(jpg, jpegBytes(120, 60));
  const setJpg = runCli(['company', 'update', '--logo', jpg], dbPath);
  assert.equal(setJpg.data.company.logo_mime, 'image/jpeg');

  const svg = path.join(dir, 'logo.svg');
  writeFileSync(svg, svgBytes('width="100" height="50"'));
  const setSvg = runCli(['company', 'update', '--logo', svg], dbPath);
  assert.equal(setSvg.data.company.logo_mime, 'image/svg+xml');

  // SVG with XML declaration + comment before <svg (real-world logo files)
  const commented = path.join(dir, 'commented.svg');
  writeFileSync(commented,
    '<?xml version="1.0" encoding="UTF-8"?>\n<!-- a long comment block that pushes <svg far past the first 200 bytes of the file -->\n<svg xmlns="http://www.w3.org/2000/svg" width="100" height="50"></svg>');
  const setC = runCli(['company', 'update', '--logo', commented], dbPath);
  assert.equal(setC.data.company.logo_mime, 'image/svg+xml');

  const got = runCli(['company', 'logo', '--out', path.join(dir, 'x.png')], dbPath);
  assert.equal(got.data.mime, 'image/svg+xml');
});
