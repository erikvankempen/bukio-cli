import { test, beforeEach } from 'node:test';
import assert from 'node:assert/strict';
import { openDb } from '../src/core/db.js';
import { seedDefaultChart, getAccountByCode } from '../src/core/accounts.js';
import { createEntry, postEntry } from '../src/core/entries.js';
import {
  bookVatEntry, enableVatModule, expandVatPostings, isVatEnabled, listVatCodes,
  markFiled, obReadout, parsePeriod, parseVatPostingSpecs,
} from '../src/vat/index.js';

let db;

beforeEach(() => {
  db = openDb(':memory:');
  seedDefaultChart(db);
  db.prepare("INSERT INTO company (name, legal_form) VALUES ('Test BV', 'bv')").run();
});

function enableVat() {
  enableVatModule(db);
}

test('enableVatModule: flag, accounts 1500/2500, 8 codes; idempotent', () => {
  enableVat();
  assert.equal(isVatEnabled(db), true);
  assert.equal(getAccountByCode(db, '1500').name, 'Te vorderen omzetbelasting');
  assert.equal(getAccountByCode(db, '2500').name, 'Te betalen omzetbelasting');
  const codes = listVatCodes(db);
  assert.equal(codes.length, 8);
  assert.ok(codes.some((c) => c.code === '21' && c.rate_bp === 2100 && c.type === 'standard'));
  assert.ok(codes.some((c) => c.code === 'RE' && c.eu_reverse === 1));

  // idempotent
  enableVat();
  assert.equal(listVatCodes(db).length, 8);
});

test('enableVatModule: refuses on KOR company', () => {
  db.prepare('UPDATE company SET kor_flag = 1, vat_module = 0').run();
  assert.throws(() => enableVatModule(db), { code: 'KOR_ACTIVE' });
});

test('parseVatPostingSpecs: CODE:AMOUNT[@VATCODE]', () => {
  assert.deepEqual(parseVatPostingSpecs(['1100:121.00,8000:-100.00@21']), [
    { code: '1100', amountCents: 12100, vatCode: null },
    { code: '8000', amountCents: -10000, vatCode: '21' },
  ]);
  assert.throws(() => parseVatPostingSpecs(['nonsense']), { code: 'INVALID_POSTING' });
});

test('expandVatPostings: adds VAT leg, computes vat amount', () => {
  enableVat();
  const expanded = expandVatPostings(db, parseVatPostingSpecs(['1100:121.00,8000:-100.00@21']));
  assert.deepEqual(expanded, [
    { code: '1100', amountCents: 12100, vatCode: null, vatAmountCents: null, fxCurrency: null, fxAmountCents: null },
    { code: '8000', amountCents: -10000, vatCode: '21', vatAmountCents: -2100, fxCurrency: null, fxAmountCents: null },
    { code: '2500', amountCents: -2100 }, // te betalen btw (credit)
  ]);
  const sum = expanded.reduce((s, p) => s + p.amountCents, 0);
  assert.equal(sum, 0); // 12100 - 10000 - 2100 = 0
});

test('expandVatPostings: input side goes to 1500 te vorderen', () => {
  enableVat();
  const expanded = expandVatPostings(db, parseVatPostingSpecs(['4340:100.00@21,1100:-121.00']));
  const vatLeg = expanded.find((p) => p.code === '1500');
  assert.equal(vatLeg.amountCents, 2100);
  assert.equal(expanded.reduce((s, p) => s + p.amountCents, 0), 0);
});

test('bookVatEntry: posts a 3-leg entry with vat fields persisted', () => {
  enableVat();
  const { entry } = bookVatEntry(db, {
    date: '2026-06-01', description: 'Factuur 2026-001',
    postings: parseVatPostingSpecs(['1100:121.00,8000:-100.00@21']),
    post: true,
  });
  assert.equal(entry.state, 'posted');
  assert.equal(entry.postings.length, 3);
  const omzet = entry.postings.find((p) => p.account_code === '8000');
  assert.equal(omzet.vat_amount_cents, -2100);
  assert.ok(omzet.vat_code_id);
});

test('bookVatEntry: guards — module off, unknown code', () => {
  assert.throws(() => bookVatEntry(db, {
    date: '2026-06-01', description: 'x',
    postings: parseVatPostingSpecs(['1100:121.00,8000:-100.00@21']),
  }), { code: 'VAT_MODULE_OFF' });

  enableVat();
  assert.throws(() => bookVatEntry(db, {
    date: '2026-06-01', description: 'x',
    postings: parseVatPostingSpecs(['1100:121.00,8000:-100.00@25']),
  }), { code: 'VAT_CODE_NOT_FOUND' });
});

test('parsePeriod: quarters and months', () => {
  assert.deepEqual(parsePeriod('2026-Q2'), { from: '2026-04-01', to: '2026-06-30', label: '2026-Q2' });
  assert.deepEqual(parsePeriod('2026-Q4'), { from: '2026-10-01', to: '2026-12-31', label: '2026-Q4' });
  assert.deepEqual(parsePeriod('2026-07'), { from: '2026-07-01', to: '2026-07-31', label: '2026-07' });
  assert.throws(() => parsePeriod('2026-Q5'), { code: 'INVALID_PERIOD' });
  assert.throws(() => parsePeriod('2026'), { code: 'INVALID_PERIOD' });
});

function seedQ2Scenario() {
  enableVat();
  // sale: 121.00 incl 21% (omzet 100, vat 21)
  bookVatEntry(db, {
    date: '2026-04-10', description: 'Factuur 2026-001',
    postings: parseVatPostingSpecs(['1100:121.00,8000:-100.00@21']), post: true,
  });
  // purchase: 60.50 incl 21% (kosten 50, vat 10.50)
  bookVatEntry(db, {
    date: '2026-05-15', description: 'Kantoorartikelen',
    postings: parseVatPostingSpecs(['4300:50.00@21,1100:-60.50']), post: true,
  });
  // sale at 9%: 109.00 incl (omzet 100, vat 9)
  bookVatEntry(db, {
    date: '2026-06-01', description: 'Factuur 2026-002',
    postings: parseVatPostingSpecs(['1100:109.00,8000:-100.00@9']), post: true,
  });
}

test('obReadout: full scenario fields 1a-5d', () => {
  seedQ2Scenario();
  const r = obReadout(db, { period: '2026-Q2' });
  assert.equal(r.fields['1a'], 10000); // omzet 21%
  assert.equal(r.fields['1b'], 10000); // omzet 9%
  assert.equal(r.fields['1c'], 0);
  assert.equal(r.fields['3a'], 5000); // inkopen 21%
  assert.equal(r.fields['5a'], 3000); // 2100 + 900
  assert.equal(r.fields['5b'], 1050); // voorbelasting
  assert.equal(r.fields['5d'], 1950); // 3000 - 1050
  assert.equal(r.to_pay, '19.50');
});

test('obReadout: period isolation and drafts excluded', () => {
  seedQ2Scenario();
  const q1 = obReadout(db, { period: '2026-Q1' });
  assert.equal(q1.fields['5d'], 0);

  // draft in Q2 must not leak (balanced draft with VAT fields)
  createEntry(db, {
    date: '2026-04-20', description: 'draft sale',
    postings: [
      { code: '1100', amountCents: 12100 },
      { code: '8000', amountCents: -10000, vatCode: '21', vatAmountCents: -2100 },
      { code: '2500', amountCents: -2100, vatCode: '21', vatAmountCents: -2100 },
    ],
  });
  const again = obReadout(db, { period: '2026-Q2' });
  assert.equal(again.fields['5a'], 3000);
});

test('obReadout: reverse charge fields 3a/4a (nets out via 5b)', () => {
  enableVat();
  bookVatEntry(db, {
    date: '2026-06-01', description: 'Inkoop verlegd',
    // 100 net, VAT due 21 (auto @R), paid 100, claim 21 back on 1500
    postings: parseVatPostingSpecs(['4300:100.00@R,1100:-100.00,1500:-21.00']), post: true,
  });
  const r = obReadout(db, { period: '2026-Q2' });
  assert.equal(r.fields['3a'], 10000); // binnenlandse verlegde inkoop -> 3a
  assert.equal(r.fields['4a'], 2100); // 21% of 100
  assert.equal(r.fields['5b'], 2100); // claimed back — nets out
  assert.equal(r.fields['5d'], 0);
});

test('obReadout: guards — module off, invalid period', () => {
  assert.throws(() => obReadout(db, { period: '2026-Q2' }), { code: 'VAT_MODULE_OFF' });
  enableVat();
  assert.throws(() => obReadout(db, { period: '2026' }), { code: 'INVALID_PERIOD' });
});

test('markFiled: records the filing and its fields', () => {
  seedQ2Scenario();
  const result = markFiled(db, { period: '2026-Q2', actor: 'agent:test' });
  assert.equal(result.status, 'filed');
  const row = db.prepare("SELECT * FROM vat_returns WHERE type='OB' AND period='2026-Q2'").get();
  assert.equal(row.status, 'filed');
  const fields = JSON.parse(row.fields_json);
  assert.equal(fields['5d'], 1950);
  // idempotent upsert
  markFiled(db, { period: '2026-Q2' });
  assert.equal(db.prepare("SELECT COUNT(*) c FROM vat_returns WHERE period='2026-Q2'").get().c, 1);
});
