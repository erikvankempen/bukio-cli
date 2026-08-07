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
import { execFileSync, spawn } from 'node:child_process';
import { mkdtempSync, existsSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import ExcelJS from 'exceljs';
import { openDb } from '../src/core/db.js';
import { seedDefaultChart, deactivateAccount, reactivateAccount, getAccountByCode } from '../src/core/accounts.js';
import { createEntry, postEntry, reverseEntry, getEntry } from '../src/core/entries.js';
import { parseAmount } from '../src/core/money.js';
import { trialBalance } from '../src/report/trial-balance.js';
import {
  enableVatModule, bookVatEntry, obReadout, parsePeriod,
  parseVatPostingSpecs, expandVatPostings,
} from '../src/vat/index.js';
import { addAsset, disposeAsset, runDue, register, createScheme } from '../src/assets/index.js';
import {
  createContact, updateContact, getContact, getInvoice,
  createInvoice, finalizeInvoice, markPaid, listInvoices, paymentFromBank, invoiceReminders,
} from '../src/invoice/index.js';
import { invoiceToUbl } from '../src/invoice/ubl.js';
import { parseBankCsv } from '../src/bank/csv.js';
import {
  importTransactions, getOrCreateBankAccount, linkTransaction, postFromTransaction, autoMatch,
  setTransactionState,
} from '../src/bank/index.js';
import { parseCamt053 } from '../src/bank/camt.js';
import { toEurPostings, setFxRate } from '../src/fx/index.js';
import { fetchEcbRate } from '../src/fx/ecb.js';
import {
  addPayable, createPaymentBatch, createPaymentBatchFromCsv,
  deletePaymentBatch, exportPaymentBatch, parseBatchCsv,
} from '../src/payments/index.js';
import { buildDepreciationTemplate, createTemplate, getTemplate, setTemplateStatus } from '../src/recurring/index.js';
import { markFiled } from '../src/compliance/index.js';
import { toCsv, writeXlsx } from '../src/report/export.js';

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

/** Raw (non-JSON) CLI output for csv/human renders. */
function runRaw(dbPath, args) {
  const env = { ...process.env, BUKIO_DB: dbPath, BUKIO_ACTOR: 'agent:test' };
  try {
    return { code: 0, raw: execFileSync(process.execPath, [BIN, ...args], { env, encoding: 'utf8' }) };
  } catch (err) {
    return { code: err.status, raw: `${err.stdout ?? ''}${err.stderr ?? ''}` };
  }
}

/** MCP stdio session against a real child process (harness like phase5.test.js). */
function mcpSession(dbPath) {
  const child = spawn(process.execPath, ['bin/bukio.js', 'mcp', '--db', dbPath], {
    cwd: path.join(path.dirname(fileURLToPath(import.meta.url)), '..'),
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

// --- follow-up pass (2026-08-07): dry-run uniformity + bank/payments edges ---

test('dry-run: contact add/update/markPaid write nothing', () => {
  const plan = createContact(db, { name: 'Nieuwe BV', iban: 'NL91ABNA0417164300', actor: 'agent:test', dryRun: true });
  assert.equal(plan.dryRun, true);
  assert.equal(db.prepare("SELECT COUNT(*) c FROM contacts WHERE name = 'Nieuwe BV'").get().c, 0);

  const c = addContact();
  const upd = updateContact(db, { id: c.id, name: 'Gewijzigd BV', dryRun: true, actor: 'agent:test' });
  assert.equal(upd.dryRun, true);
  assert.equal(upd.changes.name, 'Gewijzigd BV');
  assert.equal(getContact(db, c.id).name, 'ACME BV');

  // invoice pay dry-run: validated but not recorded
  const inv = createInvoice(db, { contactId: c.id, lines: ['1x Werk @ 100.00 @21'], date: '2099-01-10', actor: 'agent:test' });
  finalizeInvoice(db, { id: inv.id, actor: 'agent:test' });
  const payPlan = markPaid(db, { id: inv.id, date: '2099-02-01', amountCents: 5000, actor: 'agent:test', dryRun: true });
  assert.equal(payPlan.dryRun, true);
  assert.equal(payPlan.remaining_cents, 12100);
  assert.equal(db.prepare('SELECT COUNT(*) c FROM invoice_payments').get().c, 0);
  assert.equal(getInvoice(db, inv.id).status, 'sent');
  // overpay is still rejected in dry-run — validation always runs
  assert.throws(
    () => markPaid(db, { id: inv.id, date: '2099-02-01', amountCents: 999999, actor: 'agent:test', dryRun: true }),
    { code: 'OVERPAYMENT' },
  );
});

test('dry-run: compliance mark / fx set / recurring pause / account reactivate write nothing', () => {
  // compliance
  const filed = markFiled(db, { type: 'ICP', period: '2026-Q1', actor: 'agent:test', dryRun: true });
  assert.equal(filed.dryRun, true);
  assert.equal(db.prepare("SELECT COUNT(*) c FROM filings WHERE type = 'ICP'").get().c, 0);
  // the validation still runs
  assert.throws(() => markFiled(db, { type: 'ICP', period: '2026-Q13', actor: 'agent:test', dryRun: true }), { code: 'INVALID_PERIOD' });

  // fx
  const rate = setFxRate(db, { currency: 'USD', date: '2026-01-10', rate: '1.0875', actor: 'agent:test', dryRun: true });
  assert.equal(rate.dryRun, true);
  assert.equal(rate.rate_x10000, 10875);
  assert.equal(db.prepare("SELECT COUNT(*) c FROM fx_rates WHERE currency = 'USD'").get().c, 0);

  // recurring pause/resume
  const tpl = createTemplate(db, {
    name: 'Huur', frequency: 'monthly', startDate: '2026-01-01',
    postings: [{ code: '4300', amountCents: 100000 }, { code: '1100', amountCents: -100000 }],
    actor: 'agent:test',
  });
  const pause = setTemplateStatus(db, { id: tpl.id, status: 'paused', actor: 'agent:test', dryRun: true });
  assert.equal(pause.dryRun, true);
  assert.equal(getTemplate(db, tpl.id).status, 'active');

  // account reactivate
  const acc = db.prepare("SELECT code FROM accounts WHERE active = 0 LIMIT 1").get();
  if (!acc) {
    deactivateAccount(db, '1200');
  }
  const inactiveCode = db.prepare("SELECT code FROM accounts WHERE active = 0 LIMIT 1").get().code;
  const react = reactivateAccount(db, inactiveCode, { dryRun: true });
  assert.equal(react.dryRun, true);
  assert.equal(db.prepare('SELECT active FROM accounts WHERE code = ?').get(inactiveCode).active, 0);
});

test('dry-run: bank add + link write nothing', () => {
  const plan = getOrCreateBankAccount(db, { iban: 'NL91ABNA0417164300', accountCode: '1100', dryRun: true });
  assert.equal(plan.dryRun, true);
  assert.equal(plan.would_create, true);
  assert.equal(db.prepare("SELECT COUNT(*) c FROM bank_accounts").get().c, 0);

  // link dry-run: tx stays unmatched
  const e = post('2026-01-10', 'inkoop', [
    { code: '1100', amountCents: -5000 },
    { code: '4300', amountCents: 5000 },
  ]);
  importTransactions(db, {
    iban: 'NL91ABNA0417164300', name: 'Zakelijk', accountCode: '1100',
    transactions: [{ date: '2026-01-10', amount_cents: -5000, counterparty: 'ACME', description: 'factuur' }],
    actor: 'agent:test',
  });
  const tx = db.prepare("SELECT * FROM bank_transactions WHERE state = 'unmatched'").get();
  const linkPlan = linkTransaction(db, { txId: tx.id, entryId: e.id, actor: 'agent:test', dryRun: true });
  assert.equal(linkPlan.dryRun, true);
  assert.equal(db.prepare('SELECT COUNT(*) c FROM reconciliations').get().c, 0);
  assert.equal(db.prepare('SELECT state FROM bank_transactions WHERE id = ?').get(tx.id).state, 'unmatched');
});

test('CLI: backup --dry-run writes no file', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Demo BV', '--kvk', '12345678', '--legal-form', 'bv']);
  const outPath = path.join(path.dirname(dbPath), 'never.db');
  const { code, out } = cli(dbPath, ['backup', '--out', outPath, '--dry-run']);
  assert.equal(code, 0);
  assert.equal(out.data.dryRun, true);
  assert.equal(existsSync(outPath), false);
});

test('batch delete cascades lines and releases payables', () => {
  setup({ vat: false });
  const c = addContact('ACME BV');
  db.prepare("UPDATE company SET iban = 'NL91ABNA0417164300' WHERE id = 1").run();
  const payable = addPayable(db, {
    contact: c.id, invoiceRef: 'F1', date: '2026-01-01', amountCents: 5000,
    method: 'transfer', actor: 'agent:test',
  });
  const batch = createPaymentBatch(db, { payableIds: [payable.id], actor: 'agent:test' });
  assert.equal(db.prepare('SELECT COUNT(*) c FROM payment_batch_lines WHERE batch_id = ?').get(batch.id).c, 1);
  deletePaymentBatch(db, { id: batch.id, actor: 'agent:test' });
  // lines gone with the batch (ON DELETE CASCADE), payable back to unpaid
  assert.equal(db.prepare('SELECT COUNT(*) c FROM payment_batch_lines WHERE batch_id = ?').get(batch.id).c, 0);
  assert.equal(db.prepare('SELECT status FROM payables WHERE id = ?').get(payable.id).status, 'unpaid');
});

test('autoMatch never crosses bank accounts that share a ledger code', () => {
  setup({ vat: false });
  // two bank accounts, both mapped to ledger 1100
  importTransactions(db, {
    iban: 'NL91ABNA0417164300', name: 'Rabo A', accountCode: '1100',
    transactions: [{ date: '2026-01-10', amount_cents: -5000, counterparty: 'ACME', description: 'factuur A' }],
    actor: 'agent:test',
  });
  importTransactions(db, {
    iban: 'NL86INGB0002445588', name: 'ING B', accountCode: '1100',
    transactions: [{ date: '2026-01-10', amount_cents: -5000, counterparty: 'ACME', description: 'factuur B' }],
    actor: 'agent:test',
  });
  // book the expense for transaction A via the bank-post flow
  const txA = db.prepare("SELECT * FROM bank_transactions WHERE iban_counter IS NULL AND description = 'factuur A'").get();
  const posted = postFromTransaction(db, { txId: txA.id, accountCode: '4300', actor: 'agent:test' });
  assert.ok(posted.entry.id);

  // transaction B (same day, same amount, same ledger code) must NOT match A's entry
  const txB = db.prepare("SELECT * FROM bank_transactions WHERE description = 'factuur B'").get();
  const result = autoMatch(db, { windowDays: 5, actor: 'agent:test', dryRun: true });
  const bMatch = result.matched.find((m) => m.tx_id === txB.id);
  assert.equal(bMatch, undefined, 'tx B must not match an entry booked from account A');
});

test('SEPA MsgId stays within the 35-char limit even for huge batch ids', () => {
  setup({ vat: false });
  const c = addContact('ACME BV');
  db.prepare("UPDATE company SET iban = 'NL91ABNA0417164300' WHERE id = 1").run();
  // explicit id beyond any realistic AUTOINCREMENT range
  db.prepare(`
    INSERT INTO payment_batches (id, batch_date, debit_iban, debit_name, total_cents, created_by)
    VALUES (?, '2026-01-10', 'NL91ABNA0417164300', 'Demo BV', 10000, 'agent:test')
  `).run(99999999999999999);
  db.prepare(`
    INSERT INTO payment_batch_lines (batch_id, contact_id, name, iban, amount_cents, reference)
    VALUES (?, ?, 'ACME BV', 'NL91ABNA0417164300', 10000, 'F1')
  `).run(99999999999999999, c.id);
  const r = exportPaymentBatch(db, { id: 99999999999999999, actor: 'agent:test' });
  assert.ok(r.msg_id.length <= 35, `MsgId '${r.msg_id}' is ${r.msg_id.length} chars`);
  // the 17-digit id exceeds 2^53 so JS rounds it to 100000000000000000; the
  // slice(-16) still caps the MsgId at 35 chars — the exact digits don't matter
  assert.match(r.msg_id, /^BUKIO\d{30}$/);
});

// --- third pass (2026-08-07): UBL, export injection, derived status, MCP ---

test('UBL uses EUR currencyID and carries the supplier postal code', () => {
  const c = addContact('Klant BV');
  db.prepare("UPDATE company SET postal_code = '2712 CD' WHERE id = 1").run();
  const inv = createInvoice(db, { contactId: c.id, lines: ['1x Werk @ 100.00 @21'], date: '2099-01-10', actor: 'agent:test' });
  const { invoice } = finalizeInvoice(db, { id: inv.id, actor: 'agent:test' });
  const xml = invoiceToUbl(db, invoice);
  assert.ok(!xml.includes('currencyID="undefined"'), 'no undefined currency may leak into the UBL');
  assert.ok(xml.includes('currencyID="EUR"'), 'currencyID must be EUR');
  // the supplier PostalZone was always empty (snake_case row vs camelCase destructure)
  assert.ok(xml.includes('<cbc:PostalZone>2712 CD</cbc:PostalZone>'), 'supplier postal code must be present');
});

test('CSV and XLSX exports neuter formula injection', async () => {
  const rows = [
    { name: 'Normaal', amount: '-12.34' },
    { name: '=HYPERLINK("https://evil","x")', amount: '10.00' },
    { name: '+1+1', amount: '1.00' },
    { name: '@SUM(A1:A2)', amount: '2.00' },
  ];
  const csv = toCsv(rows, [{ key: 'name', label: 'naam' }, { key: 'amount', label: 'bedrag' }]);
  const lines = csv.trim().split('\n');
  assert.equal(lines[0], 'naam,bedrag');
  assert.equal(lines[1], 'Normaal,-12.34'); // negative amounts stay untouched
  // the guarded value contains quotes -> standard CSV quoting doubles them
  assert.equal(lines[2], `"'=HYPERLINK(""https://evil"",""x"")",10.00`);
  assert.equal(lines[3], "'+1+1,1.00");
  assert.equal(lines[4], "'@SUM(A1:A2),2.00");

  // xlsx round-trip: the guarded string must be stored as text, not a formula
  const file = path.join(mkdtempSync(path.join(os.tmpdir(), 'bukio-inj-')), 'out.xlsx');
  await writeXlsx(file, [{ name: 'S', columns: [{ header: 'naam', key: 'name' }], rows }]);
  const wb = new ExcelJS.Workbook();
  await wb.xlsx.readFile(file);
  const cell = wb.getWorksheet('S').getCell('A3');
  assert.equal(cell.value, "'=HYPERLINK(\"https://evil\",\"x\")");
});

test('invoice list --status overdue filters the derived status', () => {
  const c = addContact();
  // 2099: future invoice, never overdue
  const future = createInvoice(db, { contactId: c.id, lines: ['1x T @ 100.00'], date: '2099-01-10', actor: 'agent:test' });
  finalizeInvoice(db, { id: future.id, actor: 'agent:test' });
  // 2026-01-01: due 2026-01-31, long past — derived status is 'overdue'
  const past = createInvoice(db, { contactId: c.id, lines: ['1x T @ 100.00'], date: '2026-01-01', actor: 'agent:test' });
  finalizeInvoice(db, { id: past.id, actor: 'agent:test' });
  const overdue = listInvoices(db, { status: 'overdue' });
  assert.equal(overdue.length, 1);
  assert.equal(overdue[0].id, past.id);
  // both invoices are STORED 'sent' (overdue is derived) — the SQL filter
  // returns both; getInvoice derives the overdue one
  const sent = listInvoices(db, { status: 'sent' });
  assert.equal(sent.length, 2);
  assert.deepEqual(sent.map((i) => i.status).sort(), ['overdue', 'sent']);
});

test('MCP: vat_book execute leaves a draft unless post=true; invoice_pay defaults to outstanding', async () => {
  const dbPath = tmpDb();
  const db0 = openDb(dbPath);
  seedDefaultChart(db0);
  db0.prepare(`
    INSERT INTO company (name, kvk, legal_form, btw_id, iban, address, postal_code, city, vat_module)
    VALUES ('Demo BV', '12345678', 'bv', 'NL123456789B01', 'NL91ABNA0417164300', 'Industrieweg 12', '2712 CD', 'Zoetermeer', 1)
  `).run();
  enableVatModule(db0);
  const contact = createContact(db0, { name: 'Klant BV', address: 'A', city: 'B', actor: 'agent:test' });
  const inv = createInvoice(db0, { contactId: contact.id, lines: ['1x Werk @ 100.00 @21'], date: '2099-01-10', actor: 'agent:test' });
  finalizeInvoice(db0, { id: inv.id, actor: 'agent:test' });
  db0.close();

  const mcp = mcpSession(dbPath);
  try {
    await mcp.call('initialize', { protocolVersion: '2024-11-05', capabilities: {}, clientInfo: { name: 'test', version: '1' } });

    // vat_book without post: draft (the old code posted by default)
    const booked = await mcp.call('tools/call', {
      name: 'vat_book', arguments: { date: '2099-02-01', description: 'verkoop', postings: ['8000:-50.00@21', '1100:60.50'], mode: 'execute', actor: 'agent:mcp-test' },
    });
    const bookedRes = booked.result.content[0].text;
    assert.ok(bookedRes.includes('"state": "draft"'), `vat_book must leave a draft, got: ${bookedRes.slice(0, 200)}`);

    // invoice_pay without amount: pays the full outstanding (121.00)
    const paid = await mcp.call('tools/call', {
      name: 'invoice_pay', arguments: { id: 1, date: '2099-02-10', mode: 'execute', actor: 'agent:mcp-test' },
    });
    const paidRes = paid.result.content[0].text;
    assert.ok(paidRes.includes('"status": "paid"'), `invoice_pay should mark paid, got: ${paidRes.slice(0, 200)}`);

    // asset money parsing: '12,34' must book as 12.34 EUR (1234 cents) — the
    // old parseFloat silently booked 12.00; garbage must be rejected
    const dutch = await mcp.call('tools/call', {
      name: 'asset_add', arguments: {
        name: 'Laptop', purchase_date: '2026-01-01', purchase_price: '12,34',
        depreciation_start: '2026-01-01', recognition_date: '2026-01-01', mode: 'execute', actor: 'agent:mcp-test',
      },
    });
    assert.ok(dutch.result.content[0].text.includes('"action": "asset.add"'), 'Dutch comma amount must book');
    const bad = await mcp.call('tools/call', {
      name: 'asset_add', arguments: {
        name: 'Laptop2', purchase_date: '2026-01-01', purchase_price: 'abc',
        depreciation_start: '2026-01-01', recognition_date: '2026-01-01', mode: 'execute', actor: 'agent:mcp-test',
      },
    });
    assert.ok(bad.result.content[0].text.includes('INVALID_AMOUNT'), 'asset_add must reject garbage amounts');
  } finally {
    await mcp.close();
  }
  // the Dutch comma amount landed as the full 1234 cents, not 1200
  const check = openDb(dbPath);
  const asset = check.prepare("SELECT purchase_price_cents FROM assets WHERE name = 'Laptop'").get();
  check.close();
  assert.equal(asset.purchase_price_cents, 1234);
});

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

// --- fourth pass (2026-08-07): CLI crash paths, CSV shapes, dry-run validity ---

test('CLI: import xaf failure prints cleanly (no renderErrors crash)', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Demo BV', '--kvk', '12345678', '--legal-form', 'bv']);
  const badFile = path.join(path.dirname(dbPath), 'bad.xaf');
  writeFileSync(badFile, '<?xml version="1.0"?><Xaf><XafHeader><Version>4.0</Version></XafHeader><Mutaties><Mutatie><Boekstuknummer>1</Boekstuknummer><Datum>2026-01-01</Datum></Mutatie></Mutaties></Xaf>');
  const { code, raw } = runRaw(dbPath, ['import', 'xaf', '--file', badFile]);
  assert.equal(code, 1);
  assert.ok(raw.includes('IMPORT_VALIDATION_FAILED'), `expected validation error, got: ${raw.slice(0, 300)}`);
  assert.ok(!raw.includes('ReferenceError'), 'the CLI must not crash with a ReferenceError');
  assert.ok(!raw.includes('renderErrors'), 'the dead renderErrors call must be gone');
});

test('CLI: assets register --format csv has a header row and totals', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Demo BV', '--kvk', '12345678', '--legal-form', 'bv']);
  cli(dbPath, ['assets', 'add', '--name', 'Laptop', '--purchase-date', '2026-01-01', '--purchase-price', '1200.00', '--depreciation-start', '2026-01-01', '--recognition-date', '2026-01-01']);
  const csv = runRaw(dbPath, ['assets', 'register', '--format', 'csv']).raw;
  const lines = csv.trim().split('\n');
  assert.ok(lines[0].startsWith('id,naam,categorie'), `header row expected, got: ${lines[0]}`);
  assert.ok(csv.includes('Laptop'), 'asset row must be present');
  assert.ok(csv.includes('TOTAAL'), 'totals row must be present');
});

test('CLI: recurring run --dry-run renders plans, not undefined ids', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Demo BV', '--kvk', '12345678', '--legal-form', 'bv']);
  cli(dbPath, ['recurring', 'add', '--name', 'Huur', '--postings', '4300:1000.00,1100:-1000.00', '--frequency', 'monthly', '--start', '2026-01-10']);
  const out = runRaw(dbPath, ['recurring', 'run', '--dry-run']).raw;
  assert.ok(!out.includes('#undefined'), `dry-run must not render undefined ids, got: ${out.slice(0, 300)}`);
  assert.ok(out.includes('(plan)'), 'dry-run runs should render as plans');
});

test('CLI: export xaf --dry-run writes nothing; scheme/depreciation dry-runs validate', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Demo BV', '--kvk', '12345678', '--legal-form', 'bv']);
  cli(dbPath, ['entry', 'add', '--date', '2026-01-10', '--desc', 'Start', '--postings', '1100:1000.00,3000:-1000.00', '--post']);
  const outPath = path.join(path.dirname(dbPath), 'never.xaf');
  const { code, out } = cli(dbPath, ['export', 'xaf', '--year', '2026', '--out', outPath, '--dry-run']);
  assert.equal(code, 0);
  assert.equal(out.data.dryRun, true);
  assert.equal(existsSync(outPath), false);

  // scheme dry-run validates bounds instead of printing a NaN plan
  const badScheme = runRaw(dbPath, ['assets', 'scheme', 'add', '--name', 'X', '--life-months', 'abc', '--dry-run']);
  assert.equal(badScheme.code, 1);
  assert.ok(badScheme.raw.includes('INVALID_LIFE'));

  // depreciation dry-run validates the non-positive-final guard
  const badDep = runRaw(dbPath, ['depreciation', 'add', '--name', 'D', '--cost', '1.00', '--life-months', '150', '--start', '2026-01-01', '--dry-run']);
  assert.equal(badDep.code, 1);
  assert.ok(badDep.raw.includes('INVALID_LIFE'));
});

test('bank ignore dry-run leaves the transaction untouched', () => {
  setup({ vat: false });
  importTransactions(db, {
    iban: 'NL91ABNA0417164300', name: 'Zakelijk', accountCode: '1100',
    transactions: [{ date: '2026-01-10', amount_cents: -5000, counterparty: 'ACME', description: 'factuur' }],
    actor: 'agent:test',
  });
  const tx = db.prepare("SELECT * FROM bank_transactions WHERE state = 'unmatched'").get();
  const plan = setTransactionState(db, { id: tx.id, state: 'ignored', actor: 'agent:test', dryRun: true });
  assert.equal(plan.dryRun, true);
  assert.equal(db.prepare('SELECT state FROM bank_transactions WHERE id = ?').get(tx.id).state, 'unmatched');
});

test('assets pause dry-run leaves the status unchanged', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Demo BV', '--kvk', '12345678', '--legal-form', 'bv']);
  cli(dbPath, ['assets', 'add', '--name', 'Laptop', '--purchase-date', '2026-01-01', '--purchase-price', '1200.00', '--depreciation-start', '2026-01-01', '--recognition-date', '2026-01-01']);
  const paused = cli(dbPath, ['assets', 'pause', '--id', '1', '--dry-run']);
  assert.equal(paused.code, 0);
  const show = cli(dbPath, ['assets', 'list']);
  assert.equal(show.out.data.assets[0].status, 'active');
});

// --- FX-difference booking on invoice payments (user request 2026-08-07) ---

function fxInvoice(db, { grossCents, date = '2099-01-10' }) {
  const c = addContact();
  const inv = createInvoice(db, { contactId: c.id, lines: [`1x Werk @ ${(grossCents / 100).toFixed(2)}`], date, actor: 'agent:test' });
  return finalizeInvoice(db, { id: inv.id, actor: 'agent:test' }).invoice;
}

test('autoMatch books a small FX difference on an invoice payment to 4840', () => {
  // EUR invoice of 1000.00; the bank payment arrives as 997.50 (FX move at
  // payment date) — the 2.50 loss books to 4840 Koersverschillen
  const inv = fxInvoice(db, { grossCents: 100000 });
  const ba = getOrCreateBankAccount(db, { iban: 'NL91ABNA0417164300', name: 'Zakelijk', accountCode: '1100' });
  importTransactions(db, {
    iban: 'NL91ABNA0417164300', name: 'Zakelijk', accountCode: '1100',
    transactions: [{ date: '2099-01-20', amount_cents: 99750, counterparty: 'Klant BV', description: 'betaling' }],
    actor: 'agent:test',
  });
  const res = autoMatch(db, { actor: 'agent:test' });
  const m = res.matched.find((x) => x.kind === 'invoice');
  assert.ok(m, 'payment within the FX bound must match');
  assert.equal(m.fx_delta_cents, -250);

  // invoice settled, entry balanced with the FX leg
  assert.equal(getInvoice(db, inv.id).status, 'paid');
  const tb = trialBalance(db, {});
  assert.equal(tb.balanced, true);
  const fxNet = tb.accounts.find((a) => a.code === '4840').net_cents;
  assert.equal(fxNet, 250, 'loss of 2.50 must sit on 4840 (debit)');
  const bankNet = tb.accounts.find((a) => a.code === '1100').net_cents;
  assert.equal(bankNet, 99750);
  const deb = tb.accounts.find((a) => a.code === '1200').net_cents;
  assert.equal(deb, 0, 'Debiteuren must be fully released');
});

test('paymentFromBank with an FX gain books a credit on 4840', () => {
  const inv = fxInvoice(db, { grossCents: 50000 }); // 500.00
  importTransactions(db, {
    iban: 'NL91ABNA0417164300', name: 'Zakelijk', accountCode: '1100',
    transactions: [{ date: '2099-01-20', amount_cents: 50150, counterparty: 'Klant BV', description: 'betaling' }],
    actor: 'agent:test',
  });
  const tx = db.prepare("SELECT * FROM bank_transactions WHERE state = 'unmatched'").get();
  paymentFromBank(db, { invoiceId: inv.id, bankTxId: tx.id, actor: 'agent:test' });
  const tb = trialBalance(db, {});
  assert.equal(tb.balanced, true);
  assert.equal(tb.accounts.find((a) => a.code === '4840').net_cents, -150, 'gain of 1.50 must be a credit');
  assert.equal(getInvoice(db, inv.id).status, 'paid');
});

test('a difference beyond the sanity bound is not an FX move — rejected', () => {
  const inv = fxInvoice(db, { grossCents: 100000 }); // 1000.00
  // 950.00 = 5% off — beyond the 2% bound: wrong amount, not FX
  importTransactions(db, {
    iban: 'NL91ABNA0417164300', name: 'Zakelijk', accountCode: '1100',
    transactions: [{ date: '2099-01-20', amount_cents: 95000, counterparty: 'Klant BV', description: 'betaling' }],
    actor: 'agent:test',
  });
  const res = autoMatch(db, { actor: 'agent:test' });
  assert.equal(res.matched.filter((m) => m.kind === 'invoice').length, 0);
  assert.equal(res.unmatched_remaining, 1, 'the transaction must stay unmatched');
  assert.equal(getInvoice(db, inv.id).status, 'sent');

  const tx = db.prepare("SELECT * FROM bank_transactions WHERE state = 'unmatched'").get();
  assert.throws(
    () => paymentFromBank(db, { invoiceId: inv.id, bankTxId: tx.id, actor: 'agent:test' }),
    (err) => err.code === 'FX_DIFFERENCE_TOO_LARGE',
  );
});

test('4840 Koersverschillen is created on demand for pre-2026-08-07 databases', () => {
  db.prepare("DELETE FROM accounts WHERE code = '4840'").run();
  assert.equal(getAccountByCode(db, '4840'), null);
  const inv = fxInvoice(db, { grossCents: 10000 }); // 100.00
  importTransactions(db, {
    iban: 'NL91ABNA0417164300', name: 'Zakelijk', accountCode: '1100',
    transactions: [{ date: '2099-01-20', amount_cents: 9980, counterparty: 'Klant BV', description: 'betaling' }],
    actor: 'agent:test',
  });
  const tx = db.prepare("SELECT * FROM bank_transactions WHERE state = 'unmatched'").get();
  paymentFromBank(db, { invoiceId: inv.id, bankTxId: tx.id, actor: 'agent:test' });
  const fx = getAccountByCode(db, '4840');
  assert.ok(fx, '4840 must be created on demand');
  assert.equal(fx.rgs_code, 'WFBE.84');
  assert.equal(trialBalance(db, {}).balanced, true);
});

// --- fifth pass (2026-08-07): paymentFromBank atomicity, FX floor, audit ---

test('paymentFromBank is atomic: a failing entry leaves no payment behind', () => {
  const inv = fxInvoice(db, { grossCents: 50000 }); // 500.00
  importTransactions(db, {
    iban: 'NL91ABNA0417164300', name: 'Zakelijk', accountCode: '1100',
    transactions: [{ date: '2099-01-20', amount_cents: 50150, counterparty: 'Klant BV', description: 'betaling' }],
    actor: 'agent:test',
  });
  const tx = db.prepare("SELECT * FROM bank_transactions WHERE state = 'unmatched'").get();

  // break the ledger: deactivate the bank account's ledger code so
  // createEntry's resolvePostings fails (ACCOUNT_INACTIVE)
  deactivateAccount(db, '1100');
  assert.throws(
    () => paymentFromBank(db, { invoiceId: inv.id, bankTxId: tx.id, actor: 'agent:test' }),
    (err) => err.code === 'ACCOUNT_INACTIVE',
  );

  // nothing may be half-written: no payment row, invoice still sent, tx unmatched
  const payments = db.prepare('SELECT COUNT(*) AS n FROM invoice_payments WHERE invoice_id = ?').get(inv.id);
  assert.equal(payments.n, 0, 'no payment row may exist after a failed booking');
  assert.equal(getInvoice(db, inv.id).status, 'sent', 'invoice must not be marked paid');
  const state = db.prepare('SELECT state FROM bank_transactions WHERE id = ?').get(tx.id).state;
  assert.equal(state, 'unmatched', 'transaction must stay unmatched for re-match');
  assert.equal(db.prepare('SELECT COUNT(*) AS n FROM reconciliations').get().n, 0, 'no dangling reconciliation');
});

test('the FX sanity floor is 25 cents — a 10% short payment on a €10 invoice is rejected', () => {
  // 200bp of €10 = 20 cents; floor 25 cents caps the tolerance — a €1 (10%)
  // shortfall is a wrong amount, not an FX move, and must NOT auto-settle.
  const inv = fxInvoice(db, { grossCents: 1000 }); // 10.00
  importTransactions(db, {
    iban: 'NL91ABNA0417164300', name: 'Zakelijk', accountCode: '1100',
    transactions: [{ date: '2099-01-20', amount_cents: 900, counterparty: 'Klant BV', description: 'betaling' }],
    actor: 'agent:test',
  });
  const res = autoMatch(db, { actor: 'agent:test' });
  assert.equal(res.matched.filter((m) => m.kind === 'invoice').length, 0, '10% off must not match');
  assert.equal(getInvoice(db, inv.id).status, 'sent');
});

test('4840 creation on demand is audited', () => {
  db.prepare("DELETE FROM accounts WHERE code = '4840'").run();
  const inv = fxInvoice(db, { grossCents: 10000 });
  importTransactions(db, {
    iban: 'NL91ABNA0417164300', name: 'Zakelijk', accountCode: '1100',
    transactions: [{ date: '2099-01-20', amount_cents: 9980, counterparty: 'Klant BV', description: 'betaling' }],
    actor: 'agent:test',
  });
  const tx = db.prepare("SELECT * FROM bank_transactions WHERE state = 'unmatched'").get();
  paymentFromBank(db, { invoiceId: inv.id, bankTxId: tx.id, actor: 'agent:test' });
  const rows = db.prepare("SELECT * FROM audit_log WHERE action = 'account.create'").all();
  assert.equal(rows.length, 1, 'the on-demand account creation must leave an audit trail');
  assert.equal(rows[0].actor, 'agent:test');
  assert.equal(JSON.parse(rows[0].args_json).code, '4840');
});

test('postFromTransaction is atomic: a failing post leaves no draft or reconciliation', () => {
  const ba = getOrCreateBankAccount(db, { iban: 'NL91ABNA0417164300', name: 'Zakelijk', accountCode: '1100' });
  importTransactions(db, {
    iban: 'NL91ABNA0417164300', name: 'Zakelijk', accountCode: '1100',
    transactions: [{ date: '2099-01-20', amount_cents: -5000, counterparty: 'ACME', description: 'kosten' }],
    actor: 'agent:test',
  });
  const tx = db.prepare("SELECT * FROM bank_transactions WHERE state = 'unmatched'").get();
  const before = db.prepare('SELECT COUNT(*) AS n FROM journal_entries').get().n;

  // counter leg on a deactivated account -> postEntry fails inside the transaction
  deactivateAccount(db, '4300');
  assert.throws(
    () => postFromTransaction(db, { txId: tx.id, accountCode: '4300', actor: 'agent:test' }),
    (err) => err.code === 'ACCOUNT_INACTIVE',
  );

  const after = db.prepare('SELECT COUNT(*) AS n FROM journal_entries').get().n;
  assert.equal(after, before, 'no stray draft entry may survive a failed post');
  assert.equal(db.prepare('SELECT state FROM bank_transactions WHERE id = ?').get(tx.id).state, 'unmatched');
  assert.equal(db.prepare('SELECT COUNT(*) AS n FROM reconciliations').get().n, 0);
});

test('createInvoice rejects non-integer due-days and malformed delivery dates cleanly', () => {
  const c = addContact();
  assert.throws(
    () => createInvoice(db, { contactId: c.id, lines: ['1x A @ 10.00'], date: '2099-01-10', dueDays: Number('abc'), actor: 'agent:test' }),
    (err) => err.code === 'INVALID_DUE_DAYS',
  );
  assert.throws(
    () => createInvoice(db, { contactId: c.id, lines: ['1x A @ 10.00'], date: '2099-01-10', dueDays: -5, actor: 'agent:test' }),
    (err) => err.code === 'INVALID_DUE_DAYS',
  );
  assert.throws(
    () => createInvoice(db, { contactId: c.id, lines: ['1x A @ 10.00'], date: '2099-01-10', deliveryDate: '10-01-2099', actor: 'agent:test' }),
    (err) => err.code === 'INVALID_DATE',
  );
  // valid values still work
  const inv = createInvoice(db, { contactId: c.id, lines: ['1x A @ 10.00'], date: '2099-01-10', dueDays: 14, actor: 'agent:test' });
  assert.equal(inv.due_date, '2099-01-24');

  // invoiceReminders rejects a NaN window instead of throwing Invalid time value
  assert.throws(
    () => invoiceReminders(db, { withinDays: Number('abc') }),
    (err) => err.code === 'INVALID_WINDOW',
  );
});

test('createTemplate rejects non-integer due-days for invoice templates', () => {
  const c = addContact();
  const base = { name: 'Sub', kind: 'invoice', contactId: c.id, invoiceLines: ['1x A @ 10.00'], frequency: 'monthly', dayOfPeriod: 1, startDate: '2099-01-01', actor: 'agent:test' };
  assert.throws(
    () => createTemplate(db, { ...base, dueDays: Number('abc') }),
    (err) => err.code === 'INVALID_DUE_DAYS',
  );
  assert.throws(
    () => createTemplate(db, { ...base, dueDays: -1 }),
    (err) => err.code === 'INVALID_DUE_DAYS',
  );
  // valid still works
  const tpl = createTemplate(db, { ...base, dueDays: 14 });
  assert.equal(tpl.due_days, 14);
});

test('fetchEcbRate rejects a malformed date instead of throwing Invalid time value', async () => {
  const fetcher = async () => ({ ok: false, status: 404 });
  await assert.rejects(
    () => fetchEcbRate({ currency: 'USD', date: 'not-a-date', fetcher }),
    (err) => err.code === 'INVALID_DATE',
  );
  await assert.rejects(
    () => fetchEcbRate({ currency: 'USD', date: '2026-02-30', fetcher }),
    (err) => err.code === 'INVALID_DATE',
  );
  // valid date still flows to the fetcher
  let called = false;
  const okFetcher = async () => { called = true; return { ok: true, status: 200, text: async () => '<message:GenericData xmlns:message="http://www.sdmx.org/resources/sdmxml/schemas/v2_1/message"><message:DataSet><generic:Obs xmlns:generic="http://www.sdmx.org/resources/sdmxml/schemas/v2_1/data/generic"><generic:ObsDimension value="2026-08-03"/><generic:ObsValue value="1.0834"/></generic:Obs></message:DataSet></message:GenericData>' }; };
  await fetchEcbRate({ currency: 'USD', date: '2026-08-03', fetcher: okFetcher });
  assert.equal(called, true);
});

test('importTransactions rejects garbage or impossible transaction dates', () => {
  getOrCreateBankAccount(db, { iban: 'NL91ABNA0417164300', name: 'Zakelijk', accountCode: '1100' });
  const base = { iban: 'NL91ABNA0417164300', transactions: [{ date: '2026-01-10', amount_cents: -5000, counterparty: 'ACME', description: 'x' }], actor: 'agent:test' };
  assert.throws(
    () => importTransactions(db, { ...base, transactions: [{ ...base.transactions[0], date: 'garbage' }] }),
    (err) => err.code === 'INVALID_DATE',
  );
  assert.throws(
    () => importTransactions(db, { ...base, transactions: [{ ...base.transactions[0], date: '2026-02-30' }] }),
    (err) => err.code === 'INVALID_DATE',
  );
  assert.throws(
    () => importTransactions(db, { ...base, transactions: [{ ...base.transactions[0], date: '10-01-2026' }] }),
    (err) => err.code === 'INVALID_DATE',
  );
  // valid still imports
  const r = importTransactions(db, base);
  assert.equal(r.imported, 1);
});

test('createPaymentBatch rejects a garbage batch date (it would land in pain.001)', () => {
  const company = db.prepare('SELECT * FROM company').get();
  if (!company) {
    db.prepare("INSERT INTO company (name, kvk, legal_form) VALUES ('Test BV', '12345678', 'bv')").run();
  }
  db.prepare("UPDATE company SET iban = 'NL91ABNA0417164300' WHERE id = 1").run();
  assert.throws(
    () => createPaymentBatch(db, { date: 'garbage', lines: [{ name: 'ACME', iban: 'NL86INGB0002445588', amountCents: 1000 }], actor: 'agent:test' }),
    (err) => err.code === 'INVALID_DATE',
  );
  assert.throws(
    () => createPaymentBatch(db, { date: '2026-02-30', lines: [{ name: 'ACME', iban: 'NL86INGB0002445588', amountCents: 1000 }], actor: 'agent:test' }),
    (err) => err.code === 'INVALID_DATE',
  );
  // valid still works
  const b = createPaymentBatch(db, { date: '2026-03-05', lines: [{ name: 'ACME', iban: 'NL86INGB0002445588', amountCents: 1000 }], actor: 'agent:test' });
  assert.equal(b.batch_date, '2026-03-05');
});


