/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

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
//   F13 numeric CLI inputs validate: `invoice reminders --within-days 0` stays
//       0 (was masked to 7 by `Number(x) || 7`); garbage within-days errors
//       INVALID_WINDOW; `--limit` validates at the module boundary
//       (INVALID_LIMIT instead of a raw SQLITE_MISMATCH from NaN LIMIT
//       binding); `--limit 0` returns 0 rows instead of the default; the MCP
//       journal tool actually applies its limit and flags truncation, and the
//       MCP year/limit params are validated (INVALID_YEAR/INVALID_LIMIT).
import { test, beforeEach } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync, spawn } from 'node:child_process';
import { mkdtempSync, existsSync, writeFileSync } from 'node:fs';
import { readdirSync, readFileSync, statSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import ExcelJS from 'exceljs';
import { openDb } from '../src/core/db.js';
import { seedDefaultChart, deactivateAccount, reactivateAccount, getAccountByCode } from '../src/core/accounts.js';
import { createEntry, postEntry, reverseEntry, getEntry, listEntries } from '../src/core/entries.js';
import { parseAmount } from '../src/core/money.js';
import { trialBalance } from '../src/report/trial-balance.js';
import { balans } from '../src/report/balans.js';
import {
  enableVatModule, bookVatEntry, obReadout, parsePeriod,
  parseVatPostingSpecs, expandVatPostings,
} from '../src/vat/index.js';
import { addAsset, disposeAsset, runDue, register, createScheme } from '../src/assets/index.js';
import {
  createContact, updateContact, getContact, getInvoice,
  createInvoice, finalizeInvoice, creditInvoice, markPaid, listInvoices, paymentFromBank, invoiceReminders,
} from '../src/invoice/index.js';
import { invoiceToUbl } from '../src/invoice/ubl.js';
import { parseBankCsv } from '../src/bank/csv.js';
import { importXaf } from '../src/import/index.js';
import {
  importTransactions, getOrCreateBankAccount, linkTransaction, postFromTransaction, autoMatch,
  setTransactionState, listTransactions,
} from '../src/bank/index.js';
import { parseCamt053 } from '../src/bank/camt.js';
import { toEurPostings, setFxRate, listFxRates } from '../src/fx/index.js';
import { fetchEcbRate } from '../src/fx/ecb.js';
import { exportXaf } from '../src/export/index.js';
import { jaarrekening } from '../src/report/jaarrekening.js';
import { journal } from '../src/report/journal.js';
import { list as listAudit } from '../src/audit/index.js';
import {
  addPayable, createPaymentBatch, createPaymentBatchFromCsv,
  deletePaymentBatch, exportPaymentBatch, parseBatchCsv,
} from '../src/payments/index.js';
import { buildDepreciationTemplate, createTemplate, getTemplate, setTemplateStatus, runDue as recurringRunDue, previewDue } from '../src/recurring/index.js';
import { yearEndStatus, yearEndClose } from '../src/year-end/index.js';
import { markFiled } from '../src/compliance/index.js';
import { toCsv, writeXlsx } from '../src/report/export.js';
import { renderJaarrekeningXlsx } from '../src/report/jaarrekening-xlsx.js';

const BIN = path.join(path.dirname(fileURLToPath(import.meta.url)), '..', 'bin', 'bukio.js');

let db;

function setup({ vat = true } = {}) {
  db = openDb(':memory:');
  seedDefaultChart(db);
  db.prepare(`
    INSERT INTO company (name, registration_id, legal_form, tax_id, iban, address, postal_code, city, vat_module)
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

/** MCP stdio session against a real child process (harness like agent-layer.test.js). */
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

test('vat book with @R (verlegd) books NO VAT leg — self-assessed, nets to zero', () => {
  // verlegd (R/RE): the VAT is self-assessed (verschuldigd + aftrekbaar net to
  // zero), so no 2500/1500 leg is auto-booked — the bank leg stays at the net
  // amount and the OB readout derives 4a/5b from the tagged posting.
  const { entry } = bookVatEntry(db, {
    date: '2026-01-10', description: 'verlegd',
    postings: parseVatPostingSpecs('8000:-100.00@R,1100:100.00'),
    actor: 'agent:test', post: true,
  });
  assert.equal(entry.postings.length, 2); // omzet + bank, no VAT leg
  assert.ok(!entry.postings.some((p) => p.account_code === '2500' || p.account_code === '1500'));
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

test('jaarrekening XLSX guards formula injection in account and company names', async () => {
  // hostile company name + hostile account name must be stored as TEXT cells
  db.prepare("UPDATE company SET name = '=HYPERLINK(\"https://evil\",\"x\")' WHERE id = 1").run();
  db.prepare("INSERT INTO accounts (code, name, type, normal_balance, taxonomy_code) VALUES ('9999', '=SUM(A1:A2)', 'expense', 'debit', 'WBED.42')").run();
  const e = createEntry(db, {
    date: '2026-06-01', description: 'kost',
    postings: [{ code: '1100', amountCents: 1000 }, { code: '9999', amountCents: -1000 }],
  });
  postEntry(db, { id: e.id });
  const report = jaarrekening(db, { year: 2026, model: 'klein' });
  const file = path.join(mkdtempSync(path.join(os.tmpdir(), 'bukio-jrk-inj-')), 'jrk.xlsx');
  await renderJaarrekeningXlsx(report, { outPath: file });
  const wb = new ExcelJS.Workbook();
  await wb.xlsx.readFile(file);
  // company name (Balans!A3 — A1 is the column header row, A2 the title) must be guarded text, not a formula
  const nameCell = wb.getWorksheet('Balans').getCell('A3');
  assert.ok(String(nameCell.value).startsWith("'=HYPERLINK"), `company name must be guarded, got: ${nameCell.value}`);
  // hostile account name in the W&V sheet must be guarded too
  // the statutory jaarrekening XLSX keeps its Dutch sheet names (statutory document)
  const wv = wb.getWorksheet('Winst en verlies');
  let guarded = false;
  wv.eachRow((row) => {
    const v = String(row.getCell(1).value ?? '');
    if (v.includes('=SUM(A1:A2)')) guarded = v.startsWith("'");
  });
  assert.ok(guarded, 'account name must be guarded in the W&V sheet');
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
    INSERT INTO company (name, registration_id, legal_form, tax_id, iban, address, postal_code, city, vat_module)
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
    assert.ok(dutch.result.content[0].text.includes('"action": "assets.add"'), 'Dutch comma amount must book');
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

test('CLI: assets register --format json emits JSON without the global --json flag (round 11)', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Demo BV', '--kvk', '12345678', '--legal-form', 'bv']);
  const raw = runRaw(dbPath, ['assets', 'register', '--format', 'json']).raw;
  const parsed = JSON.parse(raw);
  assert.equal(parsed.ok, true);
  assert.ok(Array.isArray(parsed.data.assets));
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
  assert.equal(fx.taxonomy_code, 'WFBE.84');
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
    db.prepare("INSERT INTO company (name, registration_id, legal_form) VALUES ('Test BV', '12345678', 'bv')").run();
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

test('jaarrekening and exportXaf reject a non-YYYY year instead of building nonsense documents', () => {
  // company is already seeded by setup()
  assert.throws(
    () => jaarrekening(db, { year: 'abc', model: 'klein' }),
    (err) => err.code === 'INVALID_YEAR',
  );
  assert.throws(
    () => jaarrekening(db, { year: 20261, model: 'klein' }),
    (err) => err.code === 'INVALID_YEAR',
  );
  assert.throws(
    () => exportXaf(db, { year: 'abc', out: '/tmp/never.xaf' }),
    (err) => err.code === 'INVALID_YEAR',
  );
});



// --- F13: numeric CLI inputs validate — no silent defaults, no raw SQL errors ---

test('invoice reminders --within-days 0 stays 0 and garbage is rejected (no silent 7)', () => {
  const dbPath = tmpDb();
  const db0 = openDb(dbPath);
  seedDefaultChart(db0);
  db0.prepare("INSERT INTO company (name, registration_id, legal_form) VALUES ('Test BV', '12345678', 'bv')").run();
  db0.close();

  // 0 must stay 0 — the old `Number(x) || 7` masked it to 7
  const zero = cli(dbPath, ['invoice', 'reminders', '--within-days', '0']);
  assert.equal(zero.code, 0);
  assert.equal(zero.out.data.within_days, 0, '--within-days 0 must not become 7');

  // garbage must error, not silently default to 7
  const garbage = cli(dbPath, ['invoice', 'reminders', '--within-days', 'abc']);
  assert.equal(garbage.code, 1);
  assert.equal(garbage.out.error.code, 'INVALID_WINDOW');

  // negatives stay rejected
  const neg = cli(dbPath, ['invoice', 'reminders', '--within-days', '-1']);
  assert.equal(neg.code, 1);
  assert.equal(neg.out.error.code, 'INVALID_WINDOW');
});

test('list limits validate at the module boundary (INVALID_LIMIT, not SQLITE_MISMATCH)', () => {
  assert.throws(() => listEntries(db, { limit: 'abc' }), (e) => e.code === 'INVALID_LIMIT');
  assert.throws(() => listEntries(db, { limit: -1 }), (e) => e.code === 'INVALID_LIMIT');
  assert.throws(() => listAudit(db, { limit: 'abc' }), (e) => e.code === 'INVALID_LIMIT');
  assert.throws(() => listTransactions(db, { limit: NaN }), (e) => e.code === 'INVALID_LIMIT');
  assert.throws(() => listFxRates(db, { limit: 'abc' }), (e) => e.code === 'INVALID_LIMIT');
  assert.throws(() => listFxRates(db, { limit: -5 }), (e) => e.code === 'INVALID_LIMIT');

  // zero is legal and returns zero rows (SQLite LIMIT 0 semantics)
  assert.equal(listEntries(db, { limit: 0 }).length, 0);
  assert.equal(listFxRates(db, { limit: 0 }).length, 0);
});

test('CLI --limit 0 returns 0 rows; garbage --limit errors (no parseInt || default masking)', () => {
  const dbPath = tmpDb();
  const db0 = openDb(dbPath);
  seedDefaultChart(db0);
  db0.prepare("INSERT INTO company (name, registration_id, legal_form) VALUES ('Test BV', '12345678', 'bv')").run();
  for (let i = 0; i < 2; i += 1) {
    const e = createEntry(db0, {
      date: '2026-01-01', description: `e${i}`,
      postings: [{ code: '1100', amountCents: 1000 }, { code: '8000', amountCents: -1000 }],
      actor: 'agent:test',
    });
    postEntry(db0, { id: e.id, actor: 'agent:test' });
  }
  db0.close();

  // --limit 0 stays 0 (the old parseInt(x) || 100 returned the default)
  const zero = cli(dbPath, ['entry', 'list', '--limit', '0']);
  assert.equal(zero.code, 0);
  assert.equal(zero.out.data.entries.length, 0, '--limit 0 must not become the default');

  // --limit 1 caps
  const one = cli(dbPath, ['entry', 'list', '--limit', '1']);
  assert.equal(one.code, 0);
  assert.equal(one.out.data.entries.length, 1);

  // garbage errors with the proper code, not SQLITE_MISMATCH
  const garbage = cli(dbPath, ['entry', 'list', '--limit', 'abc']);
  assert.equal(garbage.code, 1);
  assert.equal(garbage.out.error.code, 'INVALID_LIMIT');

  // audit --limit 0
  const auditZero = cli(dbPath, ['audit', '--limit', '0']);
  assert.equal(auditZero.code, 0);
  assert.equal(auditZero.out.data.entries.length, 0);

  // fx list --limit abc (the original SQLITE_MISMATCH path)
  const fxGarbage = cli(dbPath, ['fx', 'list', '--limit', 'abc']);
  assert.equal(fxGarbage.code, 1);
  assert.equal(fxGarbage.out.error.code, 'INVALID_LIMIT');
  const fxZero = cli(dbPath, ['fx', 'list', '--limit', '0']);
  assert.equal(fxZero.code, 0);
  assert.equal(fxZero.out.data.rates.length, 0);
});

test('MCP: journal honors limit with a truncation flag; year and limit are validated', async () => {
  const dbPath = tmpDb();
  const db0 = openDb(dbPath);
  seedDefaultChart(db0);
  db0.prepare("INSERT INTO company (name, registration_id, legal_form) VALUES ('Test BV', '12345678', 'bv')").run();
  for (let i = 0; i < 3; i += 1) {
    const e = createEntry(db0, {
      date: '2026-01-01', description: `e${i}`,
      postings: [{ code: '1100', amountCents: 1000 }, { code: '8000', amountCents: -1000 }],
      actor: 'agent:test',
    });
    postEntry(db0, { id: e.id, actor: 'agent:test' });
  }
  db0.close();

  const mcp = mcpSession(dbPath);
  try {
    await mcp.call('initialize', { protocolVersion: '2024-11-05', capabilities: {}, clientInfo: { name: 'test', version: '1' } });

    // limit actually bounds the response and flags truncation
    const capped = await mcp.call('tools/call', { name: 'journal', arguments: { year: '2026', limit: 2 } });
    const cappedRes = JSON.parse(capped.result.content[0].text);
    assert.equal(cappedRes.rows.length, 2, 'journal limit must actually cap the rows');
    assert.equal(cappedRes.truncated, true, 'truncation must be flagged');

    // no limit -> complete journal, not truncated
    const full = await mcp.call('tools/call', { name: 'journal', arguments: { year: '2026' } });
    const fullRes = JSON.parse(full.result.content[0].text);
    assert.equal(fullRes.truncated, false);
    assert.ok(fullRes.rows.length > 2, 'unlimited journal must be complete');

    // garbage year -> INVALID_YEAR, not a silent empty result
    const badYear = await mcp.call('tools/call', { name: 'journal', arguments: { year: 'abcd' } });
    assert.ok(badYear.result.content[0].text.includes('INVALID_YEAR'), badYear.result.content[0].text.slice(0, 200));

    // garbage limit -> INVALID_LIMIT, not SQLITE_MISMATCH
    const badLimit = await mcp.call('tools/call', { name: 'journal', arguments: { year: '2026', limit: 'abc' } });
    assert.ok(badLimit.result.content[0].text.includes('INVALID_LIMIT'), badLimit.result.content[0].text.slice(0, 200));

    // invoices tool validates its limit too
    const badInvLimit = await mcp.call('tools/call', { name: 'invoices', arguments: { limit: 'abc' } });
    assert.ok(badInvLimit.result.content[0].text.includes('INVALID_LIMIT'), badInvLimit.result.content[0].text.slice(0, 200));

    // pnl rejects a garbage year instead of silently querying '2026-13-01'
    const badPnl = await mcp.call('tools/call', { name: 'pnl', arguments: { year: '2026-13' } });
    assert.ok(badPnl.result.content[0].text.includes('INVALID_YEAR'), badPnl.result.content[0].text.slice(0, 200));
  } finally {
    await mcp.close();
  }
});

// --- F14: payable + asset-run dates validate (no silent over-booking) --------

test('addPayable rejects garbage or impossible dates (they would land in the payables register)', () => {
  const contact = addContact();
  assert.throws(
    () => addPayable(db, { contact: contact.name, invoiceRef: 'F1', date: 'garbage', amountCents: 1000, actor: 'agent:test' }),
    (e) => e.code === 'INVALID_DATE',
  );
  assert.throws(
    () => addPayable(db, { contact: contact.name, invoiceRef: 'F1', date: '2026-02-30', amountCents: 1000, actor: 'agent:test' }),
    (e) => e.code === 'INVALID_DATE',
  );
  assert.throws(
    () => addPayable(db, { contact: contact.name, invoiceRef: 'F1', date: '2026-01-05', dueDate: 'nonsense', amountCents: 1000, actor: 'agent:test' }),
    (e) => e.code === 'INVALID_DATE',
  );
  // valid still works
  const ok = addPayable(db, { contact: contact.name, invoiceRef: 'F1', date: '2026-01-05', dueDate: '2026-02-05', amountCents: 1000, actor: 'agent:test' });
  assert.equal(ok.date, '2026-01-05');
  assert.equal(ok.due_date, '2026-02-05');
});

test('assets run/register reject garbage periods and as-of dates (no silent over-booking)', () => {
  createScheme(db, { name: '3y', method: 'lineair', lifeMonths: 36, actor: 'agent:test' });
  addAsset(db, {
    name: 'Laptop', schemeId: 1, purchaseDate: '2026-01-01', purchasePriceCents: 360000,
    depreciationStartDate: '2026-01-01', recognitionDate: '2026-01-01',
    assetAccount: '1200', expenseAccount: '4600', actor: 'agent:test',
  });
  // a 13th month previously booked the WHOLE year; garbage as-of booked the
  // whole remaining life (25 months) — both must now fail before any write
  assert.throws(() => runDue(db, { period: '2026-13', actor: 'agent:test' }), (e) => e.code === 'INVALID_PERIOD');
  assert.throws(() => runDue(db, { period: '2026-00', actor: 'agent:test' }), (e) => e.code === 'INVALID_PERIOD');
  assert.throws(() => runDue(db, { asOf: 'garbage', actor: 'agent:test' }), (e) => e.code === 'INVALID_DATE');
  assert.throws(() => runDue(db, { asOf: '2026-02-30', actor: 'agent:test' }), (e) => e.code === 'INVALID_DATE');
  assert.throws(() => register(db, { asOf: 'garbage', actor: 'agent:test' }), (e) => e.code === 'INVALID_DATE');
  // valid period still books exactly the due runs up to it (catch-up model)
  const r = runDue(db, { period: '2026-02', actor: 'agent:test' });
  assert.deepEqual(r.booked.map((b) => b.period), ['2026-01', '2026-02']);
  const entries = listEntries(db);
  assert.equal(entries.filter((e) => e.state === 'posted').length, 2);
});

// --- F15: XAF import dedupes duplicate boekstuknummer within one file --------

test('import xaf skips a duplicate Boekstuknummer within the same file (parity with AuditFile layout)', () => {
  const xaf = `<?xml version="1.0" encoding="UTF-8"?>
<Xaf xmlns="http://www.auditfiles.nl/XAF/4.0">
  <XafHeader><Version>4.0</Version><CompanyName>Demo BV</CompanyName><CompanyID>12345678</CompanyID><FiscalYear>2026</FiscalYear></XafHeader>
  <Rekeningen>
    <Rekening><RekeningCode>1100</RekeningCode><RekeningOmschrijving>Bank</RekeningOmschrijving><RekeningSoort>Balans</RekeningSoort></Rekening>
    <Rekening><RekeningCode>8000</RekeningCode><RekeningOmschrijving>Omzet</RekeningOmschrijving><RekeningSoort>Winst en Verlies</RekeningSoort></Rekening>
  </Rekeningen>
  <Mutaties>
    <Mutatie>
      <Boekstuknummer>DUP-1</Boekstuknummer><Datum>2026-01-10</Datum>
      <Boekingen>
        <Boeking><RekeningCode>1100</RekeningCode><TegenrekeningCode>8000</TegenrekeningCode><Bedrag>100.00</Bedrag></Boeking>
      </Boekingen>
    </Mutatie>
    <Mutatie>
      <Boekstuknummer>DUP-1</Boekstuknummer><Datum>2026-01-11</Datum>
      <Boekingen>
        <Boeking><RekeningCode>1100</RekeningCode><TegenrekeningCode>8000</TegenrekeningCode><Bedrag>200.00</Bedrag></Boeking>
      </Boekingen>
    </Mutatie>
  </Mutaties>
</Xaf>`;
  const r = importXaf(db, { xmlText: xaf, actor: 'agent:test' });
  // the second mutatie shares the boekstuknummer -> skipped as a duplicate
  assert.equal(r.imported, 1, 'only the first mutatie imports');
  assert.equal(r.duplicates, 1, 'the duplicate boekstuknummer is reported');
  const entries = db.prepare("SELECT * FROM journal_entries WHERE source = 'xaf'").all();
  assert.equal(entries.length, 1);
  assert.equal(entries[0].source_ref, 'DUP-1');
  // re-importing the same file stays idempotent (both mutaties now dupes)
  const again = importXaf(db, { xmlText: xaf, actor: 'agent:test' });
  assert.equal(again.imported, 0);
  assert.equal(again.duplicates, 2);
});

// --- F16: recurring run + year-end status validate as-of/year ----------------

test('recurring run rejects a garbage as-of (it generated 120 draft runs before)', () => {
  createTemplate(db, {
    name: 'Huur', kind: 'entry', frequency: 'monthly', dayOfPeriod: 1,
    startDate: '2026-01-01', postings: [{ code: '4600', amountCents: 10000 }, { code: '1100', amountCents: -10000 }],
    actor: 'agent:test',
  });
  assert.throws(() => recurringRunDue(db, { asOf: 'garbage', actor: 'agent:test' }), (e) => e.code === 'INVALID_DATE');
  assert.throws(() => recurringRunDue(db, { asOf: '2026-02-30', actor: 'agent:test' }), (e) => e.code === 'INVALID_DATE');
  assert.throws(() => previewDue(db, { asOf: 'garbage' }), (e) => e.code === 'INVALID_DATE');
  // a valid as-of still generates exactly the due runs
  const r = recurringRunDue(db, { asOf: '2026-03-01', actor: 'agent:test' });
  assert.equal(r.templates[0].runs.length, 3, 'Jan, Feb, Mar');
});

test('year-end status rejects a non-YYYY year', () => {
  assert.throws(() => yearEndStatus(db, { year: 'abc' }), (e) => e.code === 'INVALID_YEAR');
  assert.throws(() => yearEndStatus(db, { year: 20261 }), (e) => e.code === 'INVALID_YEAR');
  // a valid year still works
  const s = yearEndStatus(db, { year: '2026' });
  assert.equal(s.closed, false);
});

// --- F17: autoMatch window validation + FX tolerance parity ------------------

test('bank match auto validates --window-days (garbage errors, 0 stays 0)', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Test BV', '--kvk', '12345678']);
  const garbage = cli(dbPath, ['bank', 'match', 'auto', '--window-days', 'abc']);
  assert.equal(garbage.code, 1);
  assert.equal(garbage.out.error.code, 'INVALID_WINDOW');
  // module boundary too
  getOrCreateBankAccount(db, { iban: 'NL91ABNA0417164300', name: 'Zakelijk', accountCode: '1100' });
  assert.throws(() => autoMatch(db, { windowDays: NaN, actor: 'agent:test' }), (e) => e.code === 'INVALID_WINDOW');
  // 0 stays 0 (the old parseInt(x) || 5 masked it) — empty result, not an error
  const zero = cli(dbPath, ['bank', 'match', 'auto', '--window-days', '0']);
  assert.equal(zero.code, 0);
});

test('autoMatch FX tolerance matches the posting tolerance exactly (SQL integer-division drift)', () => {
  getOrCreateBankAccount(db, { iban: 'NL91ABNA0417164300', name: 'Zakelijk', accountCode: '1100' });
  const contact = addContact();
  const inv = createInvoice(db, { contactId: contact.id, lines: ['1x Werk @ 123.45 @21'], date: '2026-01-10', actor: 'agent:test' });
  finalizeInvoice(db, { id: inv.id, actor: 'agent:test' });
  const outstanding = getInvoice(db, inv.id).gross_cents; // 12345 + 21% = 14937
  const tol = Math.round((outstanding * 200) / 10000);    // 299 (JS posting tolerance)
  const payAmount = outstanding - tol;                    // exactly at the boundary
  const r = importTransactions(db, {
    iban: 'NL91ABNA0417164300',
    transactions: [{ date: '2026-01-15', amount_cents: payAmount, counterparty: contact.name, description: `Betaling ${inv.invoice_number}` }],
    actor: 'agent:test',
  });
  assert.equal(r.imported, 1);
  // the SQL tolerance used integer division (298) and never proposed this
  // payment; with the fix it matches the JS posting tolerance (299)
  const dry = autoMatch(db, { dryRun: true, actor: 'agent:test' });
  assert.ok(
    dry.matched.some((x) => x.kind === 'invoice' && x.invoice_id === inv.id),
    `payment at the ${tol}-cent boundary must be proposed (matches: ${JSON.stringify(dry.matched.map((m) => ({ kind: m.kind, invoice_id: m.invoice_id })))})`,
  );
  // and the real run settles the invoice + books the FX difference balanced
  const real = autoMatch(db, { actor: 'agent:test' });
  assert.equal(real.matched.length, 1);
  const check = getInvoice(db, inv.id);
  assert.equal(check.status, 'paid');
  assert.equal(trialBalance(db, {}).balanced, true);
});

// --- F18: year-end close on a zero-result year -------------------------------

test('year-end close handles a zero-result year (income == expense) without zero-amount legs', () => {
  // income 100 + expense 100 -> net result 0
  const e1 = createEntry(db, { date: '2026-03-01', description: 'omzet', postings: [{ code: '1100', amountCents: 10000 }, { code: '8000', amountCents: -10000 }], actor: 'agent:test' });
  postEntry(db, { id: e1.id, actor: 'agent:test' });
  const e2 = createEntry(db, { date: '2026-04-01', description: 'kosten', postings: [{ code: '4600', amountCents: 10000 }, { code: '1100', amountCents: -10000 }], actor: 'agent:test' });
  postEntry(db, { id: e2.id, actor: 'agent:test' });

  // dry-run first: one closing entry, no appropriation
  const plan = yearEndClose(db, { year: '2026', dryRun: true, actor: 'agent:test' });
  assert.equal(plan.result_cents, 0);
  assert.equal(plan.entries.length, 1, 'zero-result year needs only the closing reversal');
  assert.equal(plan.create_9900, false, '9900 is not needed when there is nothing to appropriate');

  // real close: one posted closing entry, no 9900 account created
  const r = yearEndClose(db, { year: '2026', actor: 'agent:test' });
  assert.equal(r.closed, true);
  assert.equal(r.result_cents, 0);
  assert.equal(r.entries.length, 1);
  assert.equal(r.entries[0].postings.length, 2, 'only the two P&L reversals, no zero legs');
  assert.equal(getAccountByCode(db, '9900'), null, '9900 must not be created for a zero result');
  assert.equal(trialBalance(db, {}).balanced, true);
  // a normal (non-zero) year still closes with both entries
  const e3 = createEntry(db, { date: '2025-05-01', description: 'omzet2', postings: [{ code: '1100', amountCents: 5000 }, { code: '8000', amountCents: -5000 }], actor: 'agent:test' });
  postEntry(db, { id: e3.id, actor: 'agent:test' });
  const r2 = yearEndClose(db, { year: '2025', actor: 'agent:test' });
  assert.equal(r2.result_cents, 5000);
  assert.equal(r2.entries.length, 2, 'non-zero result still books closing + appropriation');
});

// --- F19: recurring template numeric inputs pass through unmasked -----------
// (pre-release pass 5): `recurring add` still masked --due-days 0 -> 30 and
// --day 0/abc -> 1 with `Number(x) || default`, and the dry-run simulator
// previewed due_date null for a 0-day template the real run dates same-day.

test('recurring add --due-days 0 stays 0 (the old Number(x) || 30 masked it)', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Test BV', '--kvk', '12345678']);
  cli(dbPath, ['contact', 'add', '--name', 'ACME BV']);
  const zero = cli(dbPath, ['recurring', 'add', '--kind', 'invoice', '--contact', '1',
    '--lines', '1x Coaching @ 100.00', '--frequency', 'monthly', '--start', '2026-01-15',
    '--due-days', '0', '--name', 't0']);
  assert.equal(zero.code, 0, zero.raw ?? '');
  const check = openDb(dbPath);
  const row = check.prepare("SELECT due_days FROM recurring_templates WHERE name = 't0'").get();
  assert.equal(row.due_days, 0, 'due-days 0 must survive, not become 30');
  check.close();
});

test('recurring add rejects garbage --due-days (INVALID_DUE_DAYS) instead of silently defaulting to 30', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Test BV', '--kvk', '12345678']);
  cli(dbPath, ['contact', 'add', '--name', 'ACME BV']);
  const due = cli(dbPath, ['recurring', 'add', '--kind', 'invoice', '--contact', '1',
    '--lines', '1x Coaching @ 100.00', '--frequency', 'monthly', '--start', '2026-01-15',
    '--due-days', 'abc', '--name', 't2']);
  assert.equal(due.code, 1);
  assert.equal(due.out.error.code, 'INVALID_DUE_DAYS');
});

test('recurring add --day 0 and --day abc are rejected (INVALID_DATE) instead of silently becoming day 1', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Test BV', '--kvk', '12345678']);
  const day0 = cli(dbPath, ['recurring', 'add', '--postings', '4300:100.00,1100:-100.00',
    '--frequency', 'monthly', '--start', '2026-01-15', '--day', '0', '--name', 't1']);
  assert.equal(day0.code, 1);
  assert.equal(day0.out.error.code, 'INVALID_DATE');
  const abc = cli(dbPath, ['recurring', 'add', '--postings', '4300:100.00,1100:-100.00',
    '--frequency', 'monthly', '--start', '2026-01-15', '--day', 'abc', '--name', 't3']);
  assert.equal(abc.code, 1);
  assert.equal(abc.out.error.code, 'INVALID_DATE');
});

test('recurring run dry-run previews due_date = invoice date when due_days is 0 (parity with the real run)', () => {
  setup();
  const contact = addContact();
  const tpl = createTemplate(db, {
    name: 'abonnement', frequency: 'monthly', dayOfPeriod: 15, startDate: '2026-01-15',
    kind: 'invoice', contactId: contact.id,
    invoiceLines: ['1x Coaching @ 100.00'], dueDays: 0, actor: 'agent:test',
  });
  assert.equal(tpl.due_days, 0);
  const plan = recurringRunDue(db, { asOf: '2026-01-20', dryRun: true, actor: 'agent:test' });
  assert.equal(plan.templates.length, 1);
  assert.equal(plan.templates[0].runs.length, 1);
  assert.equal(plan.templates[0].runs[0].invoice.due_date, '2026-01-15', 'due_days 0 → due on the invoice date, not null');
  // the real run books a draft invoice due the same day
  const real = recurringRunDue(db, { asOf: '2026-01-20', actor: 'agent:test' });
  assert.equal(real.templates.length, 1);
  assert.equal(real.templates[0].runs.length, 1);
  const inv = getInvoice(db, real.templates[0].runs[0].generated[0].invoice.id);
  assert.equal(inv.due_date, '2026-01-15', 'real run dates a 0-day template on the invoice date');
});

test('recurring add --dry-run validates like the real run (garbage rejected, nothing written)', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Test BV', '--kvk', '12345678']);
  // the old hand-built dry-run plan echoed garbage as ok:true — now it validates
  const badDay = cli(dbPath, ['recurring', 'add', '--postings', '4300:100.00,1100:-100.00',
    '--frequency', 'monthly', '--start', '2026-01-15', '--day', 'abc', '--name', 't1', '--dry-run']);
  assert.equal(badDay.code, 1);
  assert.equal(badDay.out.error.code, 'INVALID_DATE');
  const badPostings = cli(dbPath, ['recurring', 'add', '--postings', 'BOGUS',
    '--frequency', 'monthly', '--start', '2026-01-15', '--name', 't2', '--dry-run']);
  assert.equal(badPostings.code, 1);
  assert.equal(badPostings.out.error.code, 'INVALID_POSTING');
  // a valid dry-run returns the plan and writes nothing
  const ok = cli(dbPath, ['recurring', 'add', '--postings', '4300:100.00,1100:-100.00',
    '--frequency', 'monthly', '--start', '2026-01-15', '--name', 't3', '--dry-run']);
  assert.equal(ok.code, 0, ok.raw ?? '');
  assert.equal(ok.out.data.dryRun, true);
  const check = openDb(dbPath);
  assert.equal(check.prepare('SELECT COUNT(*) c FROM recurring_templates').get().c, 0, 'dry-run must not write');
  check.close();
});

test('invoice create --dry-run validates like the real run (garbage date/contact rejected, nothing written)', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Test BV', '--kvk', '12345678']);
  cli(dbPath, ['contact', 'add', '--name', 'ACME BV']);
  // the old branch only parsed lines — a garbage date came back ok:true
  const badDate = cli(dbPath, ['invoice', 'create', '--contact', '1', '--lines', '1x Coaching @ 100.00', '--date', 'abc', '--dry-run']);
  assert.equal(badDate.code, 1);
  assert.equal(badDate.out.error.code, 'INVALID_DATE');
  const badContact = cli(dbPath, ['invoice', 'create', '--contact', '99', '--lines', '1x Coaching @ 100.00', '--date', '2026-01-15', '--dry-run']);
  assert.equal(badContact.code, 1);
  assert.equal(badContact.out.error.code, 'CONTACT_NOT_FOUND');
  // valid dry-run: plan with totals, nothing written
  const ok = cli(dbPath, ['invoice', 'create', '--contact', '1', '--lines', '1x Coaching @ 100.00', '--date', '2026-01-15', '--dry-run']);
  assert.equal(ok.code, 0, ok.raw ?? '');
  assert.equal(ok.out.data.dryRun, true);
  assert.equal(ok.out.data.gross_cents, 10000);
  const check = openDb(dbPath);
  assert.equal(check.prepare('SELECT COUNT(*) c FROM invoices').get().c, 0, 'dry-run must not write');
  check.close();
});

test('creditInvoice dry-run validates like the real run (no plan for nonexistent/unfinalized invoices)', () => {
  setup();
  // nonexistent invoice
  assert.throws(() => creditInvoice(db, { id: 999, dryRun: true, actor: 'agent:test' }), (e) => e.code === 'NOT_FOUND');
  // draft (unfinalized) invoice
  const contact = addContact();
  const inv = createInvoice(db, {
    contactId: contact.id, lines: ['1x Coaching @ 100.00'], date: '2026-01-15', actor: 'agent:test',
  });
  assert.throws(() => creditInvoice(db, { id: inv.id, dryRun: true, actor: 'agent:test' }), (e) => e.code === 'NOT_FINALIZED');
  // finalized invoice → plan, nothing written
  finalizeInvoice(db, { id: inv.id, actor: 'agent:test' });
  const plan = creditInvoice(db, { id: inv.id, dryRun: true, actor: 'agent:test' });
  assert.equal(plan.dryRun, true);
  assert.equal(plan.for_invoice, inv.id);
  assert.equal(db.prepare("SELECT COUNT(*) c FROM invoices WHERE invoice_type = 'credit'").get().c, 0, 'dry-run must not write a credit note');
});

test('entry add --dry-run validates like the real run (garbage date/desc/unbalanced rejected, nothing written)', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Test BV', '--kvk', '12345678']);
  // the old branch echoed garbage as ok:true
  const badDate = cli(dbPath, ['entry', 'add', '--date', 'abc', '--desc', 'x', '--postings', '1100:100.00,3000:-100.00', '--dry-run']);
  assert.equal(badDate.code, 1);
  assert.equal(badDate.out.error.code, 'INVALID_DATE');
  const unbalanced = cli(dbPath, ['entry', 'add', '--date', '2026-01-15', '--desc', 'x', '--postings', '1100:5.00,3000:-4.00', '--dry-run']);
  assert.equal(unbalanced.code, 1);
  assert.equal(unbalanced.out.error.code, 'UNBALANCED');
  // valid dry-run still plans and writes nothing
  const ok = cli(dbPath, ['entry', 'add', '--date', '2026-01-15', '--desc', 'x', '--postings', '1100:100.00,3000:-100.00', '--dry-run']);
  assert.equal(ok.code, 0, ok.raw ?? '');
  assert.equal(ok.out.data.dryRun, true);
  const check = openDb(dbPath);
  assert.equal(check.prepare('SELECT COUNT(*) c FROM journal_entries').get().c, 0, 'dry-run must not write');
  check.close();
});

// --- F22: day-overflow calendar dates rejected at EVERY money boundary ------
// (pre-release pass 8): validateDate/validDate only checked Number.isNaN, but
// JS rolls 2026-02-30 -> Mar 2 with a valid getTime() — impossible dates were
// POSTED into the ledger (verified: entry add --date 2026-02-30 --post and
// import opening-balances --date 2026-02-30 both wrote 2026-02-30 rows).

test('entry add rejects day-overflow dates (2026-02-30 was posted before)', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Test BV', '--kvk', '12345678']);
  const bad = cli(dbPath, ['entry', 'add', '--date', '2026-02-30', '--desc', 'x', '--postings', '1100:100.00,8000:-100.00']);
  assert.equal(bad.code, 1);
  assert.equal(bad.out.error.code, 'INVALID_DATE');
  // module boundary too
  setup();
  assert.throws(
    () => createEntry(db, { date: '2026-04-31', description: 'x', postings: [{ code: '1100', amountCents: 100 }, { code: '8000', amountCents: -100 }], actor: 'agent:test' }),
    (e) => e.code === 'INVALID_DATE',
  );
  // valid dates (incl. leap day) still pass
  const ok = createEntry(db, { date: '2024-02-29', description: 'leap', postings: [{ code: '1100', amountCents: 100 }, { code: '8000', amountCents: -100 }], actor: 'agent:test' });
  assert.ok(ok.id);
});

test('import opening-balances rejects a day-overflow --date', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Test BV', '--kvk', '12345678']);
  const dir = mkdtempSync(path.join(os.tmpdir(), 'bukio-ob-'));
  const csvPath = path.join(dir, 'ob.csv');
  writeFileSync(csvPath, '1100,10000.00\n3000,-10000.00\n');
  const bad = cli(dbPath, ['import', 'opening-balances', '--file', csvPath, '--date', '2026-02-30']);
  assert.equal(bad.code, 1);
  assert.equal(bad.out.error.code, 'INVALID_DATE');
  // nothing was written
  const check = openDb(dbPath);
  assert.equal(check.prepare('SELECT COUNT(*) c FROM journal_entries').get().c, 0, 'a rejected opening-balances import writes nothing');
  check.close();
});

test('fx set rejects a day-overflow date (it used to store 2026-02-30 in fx_rates)', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Test BV', '--kvk', '12345678']);
  const bad = cli(dbPath, ['fx', 'set', '--currency', 'USD', '--date', '2026-02-30', '--rate', '1.0875']);
  assert.equal(bad.code, 1);
  assert.equal(bad.out.error.code, 'INVALID_DATE');
  // and the module boundary
  setup();
  assert.throws(() => setFxRate(db, { currency: 'USD', date: '2026-02-30', rate: '1.0875', actor: 'agent:test' }), (e) => e.code === 'INVALID_DATE');
});

test('report balance-sheet rejects a garbage as-of (it silently read as "forever" before)', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Test BV', '--kvk', '12345678']);
  const bad = cli(dbPath, ['report', 'balance-sheet', '--as-of', 'garbage']);
  assert.equal(bad.code, 1);
  assert.equal(bad.out.error.code, 'INVALID_DATE');
  // module boundary too
  setup();
  assert.throws(() => balans(db, { asOf: 'garbage' }), (e) => e.code === 'INVALID_DATE');
  assert.throws(() => balans(db, { asOf: '2026-02-30' }), (e) => e.code === 'INVALID_DATE');
});

test('report pnl / journal / trial-balance reject a garbage year (no abc-01-01 ranges)', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Test BV', '--kvk', '12345678']);
  const pnlBad = cli(dbPath, ['report', 'pnl', '--year', 'abc']);
  assert.equal(pnlBad.code, 1);
  assert.equal(pnlBad.out.error.code, 'INVALID_DATE');
  const journalBad = cli(dbPath, ['report', 'journal', '--year', 'abc']);
  assert.equal(journalBad.code, 1);
  assert.equal(journalBad.out.error.code, 'INVALID_DATE');
  const tbBad = cli(dbPath, ['report', 'trial-balance', '--year', 'abc']);
  assert.equal(tbBad.code, 1);
  assert.equal(tbBad.out.error.code, 'INVALID_YEAR');
  // valid years still work
  const ok = cli(dbPath, ['report', 'pnl', '--year', '2026']);
  assert.equal(ok.code, 0);
});

test('entry list rejects garbage date bounds (--date-to garbage returned ALL entries before)', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Test BV', '--kvk', '12345678']);
  cli(dbPath, ['entry', 'add', '--date', '2026-01-15', '--desc', 'x', '--postings', '1100:100.00,8000:-100.00', '--post']);
  const badTo = cli(dbPath, ['entry', 'list', '--date-to', 'garbage']);
  assert.equal(badTo.code, 1);
  assert.equal(badTo.out.error.code, 'INVALID_DATE');
  const badFrom = cli(dbPath, ['entry', 'list', '--date-from', '2026-02-30']);
  assert.equal(badFrom.code, 1);
  assert.equal(badFrom.out.error.code, 'INVALID_DATE');
  // valid bounds still filter correctly
  const ok = cli(dbPath, ['entry', 'list', '--date-from', '2026-01-01', '--date-to', '2026-01-31']);
  assert.equal(ok.code, 0);
  assert.equal(ok.out.data.entries.length, 1);
});

test('import opening-balances accepts the documented optional header row (2- and 3-column)', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Test BV', '--kvk', '12345678']);
  const dir = mkdtempSync(path.join(os.tmpdir(), 'bukio-ob-'));
  // 2-column header layout — failed with INVALID_CODE before (doc promised it)
  const csv2 = path.join(dir, 'ob2.csv');
  writeFileSync(csv2, 'code,amount\n1100,10000.00\n3000,-10000.00\n');
  const ok2 = cli(dbPath, ['import', 'opening-balances', '--file', csv2]);
  assert.equal(ok2.code, 0, ok2.raw ?? '');
  assert.equal(ok2.out.data.accounts, 2);
  // 3-column header layout on a second database
  const dbPath3 = tmpDb();
  cli(dbPath3, ['init', '--name', 'Test BV', '--kvk', '12345678']);
  const csv3 = path.join(dir, 'ob3.csv');
  writeFileSync(csv3, 'code,debet,credit\n1100,10000.00,\n3000,,10000.00\n');
  const ok3 = cli(dbPath3, ['import', 'opening-balances', '--file', csv3]);
  assert.equal(ok3.code, 0, ok3.raw ?? '');
  assert.equal(ok3.out.data.accounts, 2);
  // a data-only file (no header) still works — first cell is a code
  const dbPath4 = tmpDb();
  cli(dbPath4, ['init', '--name', 'Test BV', '--kvk', '12345678']);
  const csv4 = path.join(dir, 'ob4.csv');
  writeFileSync(csv4, '1100,10000.00\n3000,-10000.00\n');
  const ok4 = cli(dbPath4, ['import', 'opening-balances', '--file', csv4]);
  assert.equal(ok4.code, 0, ok4.raw ?? '');
});

test('MCP entry_add dry-run validates like execute (garbage date/unbalanced/single-posting rejected, no isError:false plan)', async () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Test BV', '--kvk', '12345678']);
  const mcp = mcpSession(dbPath);
  try {
    // garbage date → INVALID_DATE even in the default dry-run mode
    const badDate = await mcp.call('tools/call', { name: 'entry_add', arguments: { date: 'abc', description: 'x', postings: ['1100:100.00', '3000:-100.00'] } });
    assert.equal(badDate.result.isError, true);
    assert.equal(JSON.parse(badDate.result.content[0].text).error.code, 'INVALID_DATE');
    // unbalanced → UNBALANCED in dry-run
    const unbalanced = await mcp.call('tools/call', { name: 'entry_add', arguments: { date: '2026-01-15', description: 'x', postings: ['1100:5.00', '3000:-4.00'] } });
    assert.equal(unbalanced.result.isError, true);
    assert.equal(JSON.parse(unbalanced.result.content[0].text).error.code, 'UNBALANCED');
    // single posting → TOO_FEW_POSTINGS in dry-run
    const single = await mcp.call('tools/call', { name: 'entry_add', arguments: { date: '2026-01-15', description: 'x', postings: ['1100:100.00'] } });
    assert.equal(single.result.isError, true);
    assert.equal(JSON.parse(single.result.content[0].text).error.code, 'TOO_FEW_POSTINGS');
    // a valid plan still returns a green dry-run with balanced:true
    const ok = await mcp.call('tools/call', { name: 'entry_add', arguments: { date: '2026-01-15', description: 'x', postings: ['1100:100.00', '3000:-100.00'] } });
    assert.equal(ok.result.isError, false);
    assert.equal(JSON.parse(ok.result.content[0].text).balanced, true);
  } finally {
    await mcp.close();
  }
});

test('MCP entry_reverse / invoice_credit / invoice_pay dry-runs validate like execute', async () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Test BV', '--kvk', '12345678']);
  cli(dbPath, ['contact', 'add', '--name', 'ACME BV', '--address', 'Klantstraat 1', '--city', 'Amsterdam']);
  const mcp = mcpSession(dbPath);
  try {
    // entry_reverse on a nonexistent entry → NOT_FOUND in dry-run
    const rev = await mcp.call('tools/call', { name: 'entry_reverse', arguments: { id: 999 } });
    assert.equal(rev.result.isError, true);
    assert.equal(JSON.parse(rev.result.content[0].text).error.code, 'NOT_FOUND');
    // entry_post on a nonexistent entry → NOT_FOUND in dry-run
    const post = await mcp.call('tools/call', { name: 'entry_post', arguments: { id: 999 } });
    assert.equal(post.result.isError, true);
    assert.equal(JSON.parse(post.result.content[0].text).error.code, 'NOT_FOUND');
    // invoice_credit on a nonexistent invoice → NOT_FOUND in dry-run
    const credit = await mcp.call('tools/call', { name: 'invoice_credit', arguments: { id: 999 } });
    assert.equal(credit.result.isError, true);
    assert.equal(JSON.parse(credit.result.content[0].text).error.code, 'NOT_FOUND');
    // invoice_pay on a nonexistent invoice → NOT_FOUND in dry-run
    const payMissing = await mcp.call('tools/call', { name: 'invoice_pay', arguments: { id: 999, date: '2026-01-15' } });
    assert.equal(payMissing.result.isError, true);
    assert.equal(JSON.parse(payMissing.result.content[0].text).error.code, 'NOT_FOUND');
    // invoice_pay with a garbage date on a real finalized invoice → INVALID_DATE in dry-run
    cli(dbPath, ['company', 'update', '--address', 'Teststraat 1', '--postal-code', '1000 AA', '--city', 'Amsterdam']);
    cli(dbPath, ['invoice', 'create', '--contact', '1', '--lines', '1x Coaching @ 100.00', '--date', '2026-01-15']);
    cli(dbPath, ['invoice', 'finalize', '--id', '1']);
    const pay = await mcp.call('tools/call', { name: 'invoice_pay', arguments: { id: 1, date: 'abc' } });
    assert.equal(pay.result.isError, true);
    assert.equal(JSON.parse(pay.result.content[0].text).error.code, 'INVALID_DATE');
    // contact_add without a name → INVALID_NAME in dry-run
    const contact = await mcp.call('tools/call', { name: 'contact_add', arguments: { name: '  ' } });
    assert.equal(contact.result.isError, true);
    assert.equal(JSON.parse(contact.result.content[0].text).error.code, 'INVALID_NAME');
    // invoice_finalize on a nonexistent invoice → NOT_FOUND in dry-run
    const finalize = await mcp.call('tools/call', { name: 'invoice_finalize', arguments: { id: 999 } });
    assert.equal(finalize.result.isError, true);
    assert.equal(JSON.parse(finalize.result.content[0].text).error.code, 'NOT_FOUND');
    // fx_set with an impossible date → INVALID_DATE in dry-run
    const fx = await mcp.call('tools/call', { name: 'fx_set', arguments: { currency: 'USD', date: '2026-02-30', rate: '1.0875' } });
    assert.equal(fx.result.isError, true);
    assert.equal(JSON.parse(fx.result.content[0].text).error.code, 'INVALID_DATE');
  } finally {
    await mcp.close();
  }
});

test('init validates iban, vat choice and fiscal-year-end (garbage was stored silently)', () => {
  const dir = mkdtempSync(path.join(os.tmpdir(), 'bukio-init-'));
  const dbPath = path.join(dir, 't.db');
  // garbage IBAN → INVALID_IBAN (company update already rejected it; init stored it)
  const badIban = cli(dbPath, ['init', '--name', 'Test BV', '--iban', 'NL00BOGUS']);
  assert.equal(badIban.code, 1);
  assert.equal(badIban.out.error.code, 'INVALID_IBAN');
  // garbage vat choice → INVALID_VAT_CHOICE (was silently treated as off)
  const badVat = cli(dbPath, ['init', '--name', 'Test BV', '--vat', 'banana']);
  assert.equal(badVat.code, 1);
  assert.equal(badVat.out.error.code, 'INVALID_VAT_CHOICE');
  // impossible fiscal year end → INVALID_FISCAL_YEAR_END (99-99 passed the regex and
  // made the jaarrekening as-of '2026-99-99' → silently empty annual accounts)
  const badFye = cli(dbPath, ['init', '--name', 'Test BV', '--fiscal-year-end', '99-99']);
  assert.equal(badFye.code, 1);
  assert.equal(badFye.out.error.code, 'INVALID_FISCAL_YEAR_END');
  const badFye2 = cli(dbPath, ['init', '--name', 'Test BV', '--fiscal-year-end', '02-30']);
  assert.equal(badFye2.code, 1);
  assert.equal(badFye2.out.error.code, 'INVALID_FISCAL_YEAR_END');
  // a valid init still works
  const ok = cli(dbPath, ['init', '--name', 'Test BV', '--iban', 'NL91ABNA0417164300', '--fiscal-year-end', '12-31', '--vat', 'on']);
  assert.equal(ok.code, 0, ok.raw ?? '');
  assert.equal(ok.out.data.company.vat_module, 1);
});

test('account add/deactivate/reactivate/import are audited (they mutated silently before)', () => {
  const dbPath = tmpDb();
  cli(dbPath, ['init', '--name', 'Test BV', '--kvk', '12345678']);
  cli(dbPath, ['account', 'add', '--code', '9999', '--name', 'Test account', '--type', 'asset', '--normal-balance', 'debit']);
  cli(dbPath, ['account', 'deactivate', '--code', '9999']);
  cli(dbPath, ['account', 'reactivate', '--code', '9999']);
  const dir = mkdtempSync(path.join(os.tmpdir(), 'bukio-chart-'));
  const csvPath = path.join(dir, 'chart.csv');
  writeFileSync(csvPath, 'code,name,type,normal_balance,taxonomy_code\n8888,Nieuwe rekening,expense,debit,WKPR.70\n');
  cli(dbPath, ['account', 'import', '--file', csvPath]);
  const audit = cli(dbPath, ['audit']);
  const actions = audit.out.data.entries.map((e) => e.action);
  for (const expected of ['company.init', 'account.add', 'account.deactivate', 'account.reactivate', 'account.import']) {
    assert.ok(actions.includes(expected), `audit log must contain ${expected} (got: ${actions.join(', ')})`);
  }
  // dry-runs must NOT record
  cli(dbPath, ['account', 'deactivate', '--code', '8888', '--dry-run']);
  const audit2 = cli(dbPath, ['audit']);
  assert.equal(audit2.out.data.entries.filter((e) => e.action === 'account.deactivate').length, 1, 'dry-run must not write an audit row');
});

test('every emitted error code in src/ is documented in AGENTS.md §7', () => {
  const agents = readFileSync(path.join(process.cwd(), 'AGENTS.md'), 'utf8');
  const codes = new Set();
  const walk = (dir) => {
    for (const f of readdirSync(dir)) {
      const p = path.join(dir, f);
      if (statSync(p).isDirectory()) walk(p);
      else if (f.endsWith('.js')) {
        const src = readFileSync(p, 'utf8');
        for (const m of src.matchAll(/(?:Error|throw Object\.assign\(new Error)\(\s*'([A-Z][A-Z0-9_]{2,40})'|code:\s*'([A-Z][A-Z0-9_]{2,40})'|e\.code = '([A-Z][A-Z0-9_]{2,40})'/g)) {
          codes.add(m[1] ?? m[2] ?? m[3]);
        }
      }
    }
  };
  walk(path.join(process.cwd(), 'src'));
  const missing = [...codes].filter((c) => !agents.includes(c)).sort();
  assert.deepEqual(missing, [], `error codes emitted by src/ but missing from AGENTS.md: ${missing.join(', ')}`);
});

test('MCP on a missing database errors NO_DATABASE instead of silently creating an empty company', () => {
  const dir = mkdtempSync(path.join(os.tmpdir(), 'bukio-mcp-'));
  const missingPath = path.join(dir, 'missing.db');
  const child = spawn(process.execPath, ['bin/bukio.js', 'mcp', '--db', missingPath], {
    cwd: process.cwd(),
    env: { ...process.env, BUKIO_ACTOR: 'agent:test' },
  });
  child.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'tools/call', params: { name: 'trial_balance', arguments: {} } })}\n`);
  child.stdin.end();
  const out = [];
  child.stdout.on('data', (d) => out.push(d.toString()));
  child.stderr.on('data', (d) => out.push(d.toString()));
  return new Promise((resolve) => {
    child.on('exit', (code) => {
      try {
        assert.notEqual(code, 0, 'MCP must exit non-zero on a missing database');
        assert.match(out.join(''), /no database at .*run 'bukio init' first/);
        assert.equal(existsSync(missingPath), false, 'must not create the database file');
        resolve();
      } catch (err) {
        resolve(err);
      }
    });
  }).then((err) => {
    if (err) throw err;
  });
});
