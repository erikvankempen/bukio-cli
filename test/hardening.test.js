// Hardening pass (2026-08-07): regression tests for the review-pass bug fixes
// plus edge cases that trip the behavior but should not.
//
// Fixed in this pass:
//   F1  entry reverse now carries VAT fields (negated) so the OB readout
//       cancels the reversed entry instead of double-counting it.
//   F2  parsePeriod rejects invalid months (2026-13, 2026-00) instead of
//       silently producing a nonsense range.
//   F3  asset disposal exactly at book value (result == 0) no longer crashes
//       on a zero-amount posting; fully-depreciated assets with no proceeds
//       dispose cleanly.
//   F4  invoice create validates the calendar date (2026-02-30 rejected at
//       create, not at finalize).
//   F5  CAMT dedup hash includes the bank's AcctSvcrRef, so two genuinely
//       identical same-day payments both import (previously the second was
//       silently dropped as a duplicate).
//   F6  bank CSV import reports skipped rows (line + reason) instead of
//       silently dropping money.
//   F7  payments batch CSV without a header parses positionally
//       (contact,amount[,reference]) — the "optional header" doc is now true.
//   F8  buildDepreciationTemplate rejects a life so long the final run would
//       be non-positive (previously a negative "depreciation" booked).
//   F9  vat book with a 0-rate code (@V vrijgesteld, @0 nultarief) skips the
//       zero VAT leg instead of crashing with INVALID_AMOUNT_CENTS.
//   F10 FX+VAT entries absorb per-leg rounding drift in the largest untagged
//       leg — a USD invoice whose converted legs would otherwise sum to ±1-2
//       cents now books balanced.
//   F11 `vat book --json` output carries the real vat_code (the formatter
//       inverted the check and dropped it).
//   F12 `invoice pay --amount` parses money as integer cents (parseAmount):
//       '12,34' and '1e3' are rejected instead of silently mis-booking.
import { test, beforeEach } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { openDb } from '../src/core/db.js';
import { seedDefaultChart } from '../src/core/accounts.js';
import { createEntry, postEntry, reverseEntry, getEntry } from '../src/core/entries.js';
import { parseAmount } from '../src/core/money.js';
import {
  enableVatModule, bookVatEntry, obReadout, parsePeriod,
  parseVatPostingSpecs, expandVatPostings,
} from '../src/vat/index.js';
import { addAsset, disposeAsset, runDue, register, createScheme } from '../src/assets/index.js';
import { createContact, createInvoice, finalizeInvoice } from '../src/invoice/index.js';
import { parseBankCsv } from '../src/bank/csv.js';
import { importTransactions } from '../src/bank/index.js';
import { parseCamt053 } from '../src/bank/camt.js';
import { toEurPostings } from '../src/fx/index.js';
import { createPaymentBatchFromCsv, parseBatchCsv } from '../src/payments/index.js';
import { buildDepreciationTemplate } from '../src/recurring/index.js';

const BIN = path.join(path.dirname(fileURLToPath(import.meta.url)), '..', 'bin', 'bukio.js');

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

function post(date, desc, postings, opts = {}) {
  const e = createEntry(db, { date, description: desc, postings, source: opts.source ?? 'manual', actor: 'agent:test' });
  return postEntry(db, { id: e.id, actor: 'agent:test' });
}

function addContact(name = 'ACME BV') {
  return createContact(db, {
    name, address: 'Straat 1', postalCode: '1000 AA', city: 'Amsterdam',
    iban: 'NL91ABNA0417164300', actor: 'agent:test',
  });
}

/** CLI runner against a temp DB file (spawned process — needs a real file). */
function cli(dbPath, args) {
  const env = { ...process.env, BUKIO_DB: dbPath, BUKIO_ACTOR: 'agent:test' };
  try {
    const stdout = execFileSync(process.execPath, [BIN, '--json', ...args], { env, encoding: 'utf8' });
    return { code: 0, out: JSON.parse(stdout) };
  } catch (err) {
    const combined = `${err.stdout ?? ''}${err.stderr ?? ''}`;
    return { code: err.status, out: combined ? JSON.parse(err.stdout) : null, raw: combined };
  }
}

function tmpDb() {
  return path.join(mkdtempSync(path.join(os.tmpdir(), 'bukio-hardening-')), 'test.db');
}

beforeEach(() => {
  setup();
});

// --- F1: reversal carries VAT fields ----------------------------------------

test('reversal of a VAT entry cancels the OB readout and keeps vat fields', () => {
  const { entry } = bookVatEntry(db, {
    date: '2026-01-15', description: 'verkoop',
    postings: parseVatPostingSpecs('8000:-100.00@21,1100:121.00'),
    actor: 'agent:test', post: true,
  });
  const before = obReadout(db, { period: '2026-Q1' });
  assert.equal(before.fields['1a'], 10000);
  assert.equal(before.fields['5a'], 2100);

  const reversal = reverseEntry(db, { id: entry.id, actor: 'agent:test' });
  const after = obReadout(db, { period: '2026-Q1' });
  assert.equal(after.fields['1a'], 0, '1a must cancel after reversal');
  assert.equal(after.fields['5a'], 0, '5a must cancel after reversal');

  // the reversal posting carries the VAT code + negated VAT amount
  const rev = getEntry(db, reversal.id);
  const tagged = rev.postings.find((p) => p.account_code === '8000');
  assert.equal(tagged.vat_code_id, 1);
  assert.equal(tagged.vat_amount_cents, 2100); // original was -2100
});

// --- F2: parsePeriod month bounds -------------------------------------------

test('parsePeriod rejects out-of-range months', () => {
  for (const bad of ['2026-13', '2026-00', '2026-99', '2026-1']) {
    assert.throws(() => parsePeriod(bad), { code: 'INVALID_PERIOD' }, `'${bad}' must be rejected`);
  }
  const q = parsePeriod('2026-Q4');
  assert.equal(q.from, '2026-10-01');
  assert.equal(q.to, '2026-12-31');
  const m = parsePeriod('2026-12');
  assert.equal(m.from, '2026-12-01');
  assert.equal(m.to, '2026-12-31');
  const leap = parsePeriod('2024-02');
  assert.equal(leap.to, '2024-02-29');
});

// --- F3: disposal at book value / fully depreciated -------------------------

test('dispose at exactly book value (result 0) books a balanced entry', () => {
  const scheme = createScheme(db, { name: 'S', lifeMonths: 24, actor: 'agent:test' });
  const a = addAsset(db, {
    name: 'Laptop', schemeId: scheme.id, purchaseDate: '2025-01-01',
    purchasePriceCents: 240000, depreciationStartDate: '2025-01-01',
    recognitionDate: '2025-01-01', assetAccount: '1800', cumDepAccount: null,
    expenseAccount: '4600', actor: 'agent:test',
  });
  runDue(db, { asOf: '2025-06-01', actor: 'agent:test' });
  const reg = register(db, { asOf: '2025-06-01' });
  const asset = reg.assets.find((r) => r.id === a.asset.id);
  const r = disposeAsset(db, {
    id: a.asset.id, date: '2025-07-15',
    proceedsCents: asset.book_value_cents, bankAccount: '1100', actor: 'agent:test',
  });
  assert.equal(r.result_cents, 0);
  assert.equal(r.entry.state, 'posted');
});

test('dispose a fully-depreciated asset with no proceeds', () => {
  const scheme = createScheme(db, { name: 'S2', lifeMonths: 12, actor: 'agent:test' });
  const a = addAsset(db, {
    name: 'Bureaulamp', schemeId: scheme.id, purchaseDate: '2025-01-01',
    purchasePriceCents: 120000, depreciationStartDate: '2025-01-01',
    recognitionDate: '2025-01-01', assetAccount: '1800', cumDepAccount: null,
    expenseAccount: '4600', actor: 'agent:test',
  });
  runDue(db, { asOf: '2026-01-01', actor: 'agent:test' });
  const r = disposeAsset(db, { id: a.asset.id, date: '2026-02-01', proceedsCents: 0, actor: 'agent:test' });
  assert.equal(r.result_cents, 0);
  const entry = getEntry(db, r.entry.id);
  assert.equal(entry.postings.reduce((s, p) => s + p.amount_cents, 0), 0);
});

// --- F4: invoice create validates the calendar date -------------------------

test('invoice create rejects impossible calendar dates', () => {
  const c = addContact();
  for (const bad of ['2026-02-30', '2026-13-01', '2026-00-10']) {
    assert.throws(
      () => createInvoice(db, { contactId: c.id, lines: ['1x Test @ 100.00'], date: bad, actor: 'agent:test' }),
      { code: 'INVALID_DATE' },
      `'${bad}' must be rejected at create`,
    );
  }
  // leap day + month ends still work
  const a = createInvoice(db, { contactId: c.id, lines: ['1x Test @ 100.00'], date: '2024-02-29', actor: 'agent:test' });
  assert.ok(a.id);
  const b = createInvoice(db, { contactId: c.id, lines: ['1x Test @ 100.00'], date: '2026-04-30', actor: 'agent:test' });
  assert.ok(b.id);
});

// --- F5: CAMT dedup with AcctSvcrRef ----------------------------------------

test('two identical same-day CAMT entries both import (distinct AcctSvcrRef)', () => {
  const xml = `<?xml version="1.0"?>
<Document><BkToCstmrStmt><Stmt>
<Acct><Id><IBAN>NL91ABNA0417164300</IBAN></Id></Acct>
<Ntry><Amt>10.00</Amt><CdtDbtInd>DBIT</CdtDbtInd><BookgDt><Dt>2026-01-05</Dt></BookgDt><AcctSvcrRef>REF-1</AcctSvcrRef><NtryDtls><TxDtls><RltdPties><Cdtr><Nm>Spotify</Nm></Cdtr></RltdPties><RmtInf><Ustrd>Abonnement</Ustrd></RmtInf></TxDtls></NtryDtls></Ntry>
<Ntry><Amt>10.00</Amt><CdtDbtInd>DBIT</CdtDbtInd><BookgDt><Dt>2026-01-05</Dt></BookgDt><AcctSvcrRef>REF-2</AcctSvcrRef><NtryDtls><TxDtls><RltdPties><Cdtr><Nm>Spotify</Nm></Cdtr></RltdPties><RmtInf><Ustrd>Abonnement</Ustrd></RmtInf></TxDtls></NtryDtls></Ntry>
</Stmt></BkToCstmrStmt></Document>`;
  const txs = parseCamt053(xml);
  assert.equal(txs.length, 2);
  assert.equal(txs[0].bank_ref, 'REF-1');
  const first = importTransactions(db, { iban: txs[0].iban, transactions: txs, actor: 'agent:test' });
  assert.equal(first.imported, 2, 'both identical payments must import');
  const second = importTransactions(db, { iban: txs[0].iban, transactions: txs, actor: 'agent:test' });
  assert.equal(second.imported, 0, 're-import must stay idempotent');
  assert.equal(second.duplicates, 2);
});

// --- F6: bank CSV skipped rows ----------------------------------------------

test('bank CSV surfaces skipped rows instead of dropping them silently', () => {
  const csv = 'Datum;Naam;Bedrag\n2026-01-01;ACME;12,34\n2026-01-02;BAD;notanumber\n2026-01-03;GOOD;5.00\n';
  const txs = parseBankCsv(csv);
  assert.equal(txs.length, 2);
  assert.equal(txs.skipped.length, 1);
  assert.equal(txs.skipped[0].line, 3);
  assert.match(txs.skipped[0].reason, /unparseable amount/);
});

// --- F7: headerless payments batch CSV --------------------------------------

test('payments batch CSV without a header parses positionally', () => {
  setup({ vat: false });
  addContact('ACME BV');
  db.prepare("UPDATE company SET iban = 'NL91ABNA0417164300' WHERE id = 1").run();
  const { lines, errors } = parseBatchCsv('ACME BV;100.00;factuur 1\n');
  assert.equal(errors.length, 0);
  assert.equal(lines.length, 1);
  assert.equal(lines[0].contact, 'ACME BV');
  assert.equal(lines[0].amountCents, 10000);
  const batch = createPaymentBatchFromCsv(db, { csvText: 'ACME BV;100.00;factuur 1\n', actor: 'agent:test' });
  assert.equal(batch.lines.length, 1);
});

// --- F8: depreciation template final run ------------------------------------

test('buildDepreciationTemplate rejects a non-positive final run', () => {
  assert.throws(
    () => buildDepreciationTemplate(db, {
      name: 'D', costCents: 100, residualCents: 0, lifeMonths: 150, startDate: '2026-01-01', actor: 'agent:test',
    }),
    { code: 'INVALID_LIFE' },
  );
  const ok = buildDepreciationTemplate(db, {
    name: 'OK', costCents: 12000, residualCents: 0, lifeMonths: 120, startDate: '2026-01-01', actor: 'agent:test',
  });
  assert.equal(ok.monthly_cents, 100);
  assert.equal(ok.final_cents, 100);
  assert.equal(ok.total_cents, 12000);
});

// --- F9: vat book with 0-rate codes -----------------------------------------

test('vat book with @V (vrijgesteld) and @0 (nultarief) books without a zero leg', () => {
  for (const code of ['V', '0']) {
    const { entry } = bookVatEntry(db, {
      date: '2026-01-10', description: `code ${code}`,
      postings: parseVatPostingSpecs(`8000:-100.00@${code},1100:100.00`),
      actor: 'agent:test', post: true,
    });
    assert.equal(entry.state, 'posted');
    const tagged = entry.postings.find((p) => p.account_code === '8000');
    assert.equal(tagged.vat_code, code);
    assert.equal(tagged.vat_amount_cents, 0);
    // no zero-amount leg survived
    assert.ok(entry.postings.every((p) => p.amount_cents !== 0));
  }
  // the readout still reports the base (1c)
  const readout = obReadout(db, { period: '2026-Q1' });
  assert.equal(readout.fields['1c'], 20000);
});

test('vat book with @R (verlegd) still books the 21% due leg', () => {
  const { entry } = bookVatEntry(db, {
    date: '2026-01-10', description: 'verlegd',
    postings: parseVatPostingSpecs('8000:-100.00@R,1100:121.00'),
    actor: 'agent:test', post: true,
  });
  assert.equal(entry.postings.find((p) => p.account_code === '2500').amount_cents, -2100);
});

// --- F10: FX+VAT rounding drift ---------------------------------------------

test('FX+VAT booking absorbs rounding drift (rate 1.0001, 41.33 USD @21)', () => {
  // brute-force-verified failing case: naive per-leg rounding sums to -1 cent
  const specs = parseVatPostingSpecs('8000:-41.33@21,1100:50.01');
  const converted = toEurPostings(specs, { currency: 'USD', rateX10000: 10001 });
  const expanded = expandVatPostings(db, converted);
  assert.equal(expanded.reduce((s, p) => s + p.amountCents, 0), 0);
  const { entry } = bookVatEntry(db, {
    date: '2026-01-10', description: 'USD inkoop',
    postings: converted, actor: 'agent:test', post: true,
  });
  assert.equal(entry.state, 'posted');
  assert.equal(entry.postings.reduce((s, p) => s + p.amount_cents, 0), 0);
});

test('FX+VAT: a range of amounts never trips UNBALANCED', () => {
  // sweep a band of net amounts at a hostile rate — every one must balance
  for (let fx = 4000; fx <= 4300; fx += 7) {
    const gross = Math.round(fx * 1.21);
    const specs = parseVatPostingSpecs(`8000:-${(fx / 100).toFixed(2)}@21,1100:${(gross / 100).toFixed(2)}`);
    const converted = toEurPostings(specs, { currency: 'USD', rateX10000: 10001 });
    const expanded = expandVatPostings(db, converted);
    const sum = expanded.reduce((s, p) => s + p.amountCents, 0);
    assert.equal(sum, 0, `fx=${fx} unbalanced by ${sum}`);
  }
});

// --- F11: vat book --json carries vat_code ----------------------------------

test('CLI: vat book --json reports the vat_code on tagged postings', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Demo BV', '--kvk', '12345678', '--legal-form', 'bv', '--vat', 'on']);
  const { code, out } = cli(dbPath, ['vat', 'book', '--date', '2026-01-10', '--desc', 'verkoop', '--postings', '8000:-100.00@21,1100:121.00', '--post']);
  assert.equal(code, 0);
  const tagged = out.data.entry.postings.find((p) => p.account_code === '8000');
  assert.equal(tagged.vat_code, '21');
  assert.equal(tagged.vat_amount_cents, -2100);
});

// --- F12: invoice pay amount parsing ----------------------------------------

test('CLI: invoice pay rejects non-international amounts', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Demo BV', '--kvk', '12345678', '--legal-form', 'bv', '--vat', 'on']);
  cli(dbPath, ['company', 'update', '--btw-id', 'NL123456789B01', '--address', 'Industrieweg 12', '--postal-code', '2712 CD', '--city', 'Zoetermeer', '--iban', 'NL91ABNA0417164300']);
  cli(dbPath, ['contact', 'add', '--name', 'ACME BV', '--address', 'Straat 1', '--city', 'Amsterdam']);
  cli(dbPath, ['invoice', 'create', '--contact', '1', '--lines', '1x Test @ 100.00 @21', '--date', '2026-01-10']);
  cli(dbPath, ['invoice', 'finalize', '--id', '1']);

  // Dutch decimal comma must NOT silently parse as 12.00
  const badComma = cli(dbPath, ['invoice', 'pay', '--id', '1', '--date', '2026-01-20', '--amount', '12,34']);
  assert.equal(badComma.code, 1);
  assert.equal(badComma.out.error.code, 'INVALID_AMOUNT');

  // scientific notation must NOT parse
  const badSci = cli(dbPath, ['invoice', 'pay', '--id', '1', '--date', '2026-01-20', '--amount', '1e3']);
  assert.equal(badSci.code, 1);
  assert.equal(badSci.out.error.code, 'INVALID_AMOUNT');

  // the clean international form books exact cents
  const ok = cli(dbPath, ['invoice', 'pay', '--id', '1', '--date', '2026-01-20', '--amount', '12.34']);
  assert.equal(ok.code, 0);
  const show = cli(dbPath, ['invoice', 'show', '--id', '1']);
  assert.equal(show.out.data.invoice.paid, '12.34');
});

// --- extra edge cases (trip but should not) ---------------------------------

test('entry with the same account on both sides books the net', () => {
  const e = post('2026-01-10', 'partial same-code', [
    { code: '1200', amountCents: 10000 },
    { code: '1200', amountCents: -2000 },
    { code: '8000', amountCents: -8000 },
  ]);
  const full = getEntry(db, e.id);
  const byCode = new Map();
  for (const p of full.postings) byCode.set(p.account_code, (byCode.get(p.account_code) ?? 0) + p.amount_cents);
  assert.equal(byCode.get('1200'), 8000);
  assert.equal(byCode.get('8000'), -8000);
});

test('reversal of an FX entry negates the fx amounts', () => {
  post('2026-01-10', 'fx purchase', [
    { code: '4340', amountCents: 875, fxCurrency: 'USD', fxAmountCents: 1000 },
    { code: '3000', amountCents: -875, fxCurrency: 'USD', fxAmountCents: -1000 },
  ]);
  const original = db.prepare("SELECT id FROM journal_entries WHERE description = 'fx purchase'").get();
  const reversal = reverseEntry(db, { id: original.id, actor: 'agent:test' });
  const rev = getEntry(db, reversal.id);
  const fx = rev.postings.find((p) => p.account_code === '4340');
  assert.equal(fx.fx_amount_cents, -1000);
  assert.equal(fx.fx_currency, 'USD');
});

test('invoice finalize with a 0% line books a tagged zero-vat posting', () => {
  const c = addContact();
  const inv = createInvoice(db, {
    // 2099: far future, so the derived status stays 'sent' regardless of when
    // the suite runs (a past due date would flip it to 'overdue')
    contactId: c.id, lines: ['1x Diensten @ 100.00 @V'], date: '2099-01-10', actor: 'agent:test',
  });
  const { invoice } = finalizeInvoice(db, { id: inv.id, actor: 'agent:test' });
  assert.equal(invoice.status, 'sent');
  const entry = getEntry(db, invoice.entry_id);
  const omzet = entry.postings.find((p) => p.account_code === '8000');
  assert.equal(omzet.vat_code, 'V');
  assert.equal(omzet.vat_amount_cents, 0);
  // readout reports the base in 1c, no vat in 5a
  const readout = obReadout(db, { period: '2099-Q1' });
  assert.equal(readout.fields['1c'], 10000);
  assert.equal(readout.fields['5a'], 0);
});

test('parseAmount boundaries: 1 decimal, zero, negatives, large values', () => {
  assert.equal(parseAmount('1234.5'), 123450);
  assert.equal(parseAmount('-0.01'), -1);
  assert.equal(parseAmount('0'), 0);
  assert.equal(parseAmount('-0'), 0);
  assert.equal(parseAmount('999999999999.99'), 99999999999999);
  assert.equal(parseAmount(' 12.34 '), 1234); // input is trimmed by design
  for (const bad of ['', '.5', '1.', '1.234', '+12.34', '12,34', '1e3', '12.345']) {
    assert.throws(() => parseAmount(bad), { code: 'INVALID_AMOUNT' }, `'${bad}' must be rejected`);
  }
});

test('obReadout period with a year boundary stays within the period', () => {
  bookVatEntry(db, {
    date: '2026-12-31', description: 'q4 sale',
    postings: parseVatPostingSpecs('8000:-100.00@21,1100:121.00'),
    actor: 'agent:test', post: true,
  });
  bookVatEntry(db, {
    date: '2027-01-01', description: 'q1 sale',
    postings: parseVatPostingSpecs('8000:-50.00@21,1100:60.50'),
    actor: 'agent:test', post: true,
  });
  const q4 = obReadout(db, { period: '2026-Q4' });
  assert.equal(q4.fields['1a'], 10000);
  const q1 = obReadout(db, { period: '2027-Q1' });
  assert.equal(q1.fields['1a'], 5000);
});
