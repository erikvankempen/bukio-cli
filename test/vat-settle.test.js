/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// vat file / vat settle — af te dragen omzetbelasting flow:
// filing reclassifies the net VAT position to 2510; the bank payment cancels
// that balance; the whole-euro filing rounding lands in the P&L difference
// account at settlement (per the Belastingdienst 'round in your favour' rule).
import { test, beforeEach } from 'node:test';
import assert from 'node:assert/strict';
import { openDb } from '../src/core/db.js';
import { seedDefaultChart } from '../src/core/accounts.js';
import {
  enableVatModule, bookVatEntry, parseVatPostingSpecs, obReadout,
  vatFile, vatSettle, vatNetPosition,
  VAT_FILE_ACCOUNT_DEFAULT, VAT_DIFFERENCE_ACCOUNT_DEFAULT,
} from '../src/vat/index.js';
import { importTransactions } from '../src/bank/index.js';
import { parseCamt053 } from '../src/bank/camt.js';

let db;
const IBAN = 'NL91ABNA0417164300';

/** OB payment CAMT statement: one entry of `amount` (CRDT incoming / DBIT outgoing). */
function camtPayment(amount, direction = 'DBIT', date = '2026-07-25') {
  return `<?xml version="1.0"?>
<Document xmlns="urn:iso:std:iso:20022:tech:xsd:camt.053.001.02">
  <BkToCstmrStmt><Stmt><Acct><Id><IBAN>${IBAN}</IBAN></Id></Acct>
    <Ntry><Amt>${amount}</Amt><CdtDbtInd>${direction}</CdtDbtInd><BookgDt><Dt>${date}</Dt></BookgDt>
      <NtryDtls><TxDtls><RltdPties><Dbtr><Nm>Belastingdienst</Nm></Dbtr></RltdPties>
      <RmtInf><Ustrd>OB aangifte</Ustrd></RmtInf></TxDtls></NtryDtls></Ntry>
  </Stmt></BkToCstmrStmt>
</Document>`;
}

function setup() {
  db = openDb(':memory:');
  seedDefaultChart(db);
  db.prepare(
    "INSERT INTO company (name, kvk, legal_form, btw_id, iban, vat_module) VALUES ('Demo BV', '12345678', 'bv', 'NL123456789B01', ?, 1)",
  ).run(IBAN);
  enableVatModule(db);
}

beforeEach(setup);

function balance(code) {
  return db.prepare(`
    SELECT COALESCE(SUM(p.amount_cents), 0) AS bal
    FROM postings p
    JOIN journal_entries e ON e.id = p.entry_id AND e.state = 'posted'
    JOIN accounts a ON a.id = p.account_id
    WHERE a.code = ?
  `).get(code).bal;
}

function lastTx() {
  return db.prepare(`
    SELECT bt.*, ba.account_code
    FROM bank_transactions bt JOIN bank_accounts ba ON ba.id = bt.bank_account_id
    ORDER BY bt.id DESC LIMIT 1
  `).get();
}

/** assert.throws variant that checks the error CODE (vatError puts it on err.code). */
function throwsCode(fn, code) {
  assert.throws(fn, (err) => {
    assert.equal(err?.code, code, `expected error code ${code}, got ${err?.code}`);
    return true;
  });
}

/** Book a 121.00 sale (21 VAT) + a 60.50 purchase (10.50 input) -> owe €10.50. */
function bookQuarter() {
  bookVatEntry(db, {
    date: '2026-07-01', description: 'Omzet Q3',
    postings: parseVatPostingSpecs(['1100:121.00,8000:-100.00@21']), post: true,
  });
  bookVatEntry(db, {
    date: '2026-07-05', description: 'Inkoop Q3',
    postings: parseVatPostingSpecs(['1100:-60.50,4300:50.00@21']), post: true,
  });
}

test('vat file: owe — 2500 cleared, 2510 holds the exact-cents liability, audited', () => {
  bookQuarter();
  assert.equal(vatNetPosition(db), 1050); // 2100 output - 1050 input
  const r = vatFile(db, { period: '2026-Q3', actor: 'agent:test' });
  assert.equal(r.owe, true);
  assert.equal(r.liability_cents, 1050);
  assert.equal(balance('2500'), 0); // clearing account empty again
  assert.equal(balance('2510'), -1050); // af te dragen (credit)
  const rows = db.prepare("SELECT * FROM audit_log WHERE action = 'vat.file'").all();
  assert.equal(rows.length, 1);
  assert.equal(JSON.parse(rows[0].args_json).period, '2026-Q3');
  assert.equal(JSON.parse(rows[0].args_json).liability_cents, 1050);
});

test('vat file: refund position — 1500 cleared, 2510 debit (te ontvangen)', () => {
  // only input VAT: 121.00 purchase -> 21.00 voorbelasting, no sales
  bookVatEntry(db, {
    date: '2026-07-01', description: 'Inkoop',
    postings: parseVatPostingSpecs(['1100:-121.00,4300:100.00@21']), post: true,
  });
  assert.equal(vatNetPosition(db), -2100);
  const r = vatFile(db, { period: '2026-Q3', actor: 'agent:test' });
  assert.equal(r.owe, false);
  assert.equal(r.liability_cents, 2100);
  assert.equal(balance('1500'), 0);
  assert.equal(balance('2510'), 2100); // debit = terug te ontvangen
});

test('vat file: nothing to file when the position is zero', () => {
  throwsCode(() => vatFile(db, { actor: 'agent:test' }), 'VAT_NOTHING_TO_FILE');
});

test('vat file: dry-run writes nothing and does not create the account', () => {
  bookQuarter();
  const r = vatFile(db, { period: '2026-Q3', actor: 'agent:test', dryRun: true });
  assert.equal(r.dryRun, true);
  assert.equal(r.liability_cents, 1050);
  assert.equal(balance('2500'), -2100); // untouched
  assert.equal(balance('2510'), 0);
  assert.ok(!db.prepare("SELECT 1 FROM accounts WHERE code = '2510'").get());
  assert.equal(db.prepare("SELECT COUNT(*) c FROM audit_log WHERE action = 'vat.file'").get().c, 0);
});

test('vat settle: rounding in your favour (paid less than booked) books the gain to 4700', () => {
  bookQuarter();
  vatFile(db, { period: '2026-Q3', actor: 'agent:test' });
  // OB form: 5d 10.50 -> filed €10 (rounded down in favour) — bank shows the payment
  importTransactions(db, { iban: IBAN, transactions: parseCamt053(camtPayment('10.00')) });
  const tx = lastTx();
  assert.equal(tx.state, 'unmatched');
  const r = vatSettle(db, {
    txAmountCents: tx.amount_cents, txDate: tx.date, bankAccountCode: tx.account_code,
    period: '2026-Q3', actor: 'agent:test',
  });
  assert.equal(r.difference_cents, -50); // paid 50 cents less -> gain (credit)
  assert.equal(r.difference_account, VAT_DIFFERENCE_ACCOUNT_DEFAULT);
  assert.equal(balance('2510'), 0); // af te dragen cancelled
  assert.equal(balance('4700'), -50); // P&L gain
  assert.equal(balance('1100'), 5050); // 121.00 - 60.50 = 60.50, minus the 10.00 payment
  const rows = db.prepare("SELECT * FROM audit_log WHERE action = 'vat.settle'").all();
  assert.equal(rows.length, 1);
  assert.equal(JSON.parse(rows[0].args_json).difference_cents, -50);
});

test('vat settle: refund received in your favour (more than booked) books a gain', () => {
  bookVatEntry(db, {
    date: '2026-07-01', description: 'Inkoop',
    postings: parseVatPostingSpecs(['1100:-121.00,8000:100.00@21']), post: true,
  });
  vatFile(db, { period: '2026-Q3', actor: 'agent:test' });
  // filed teruggave €22 (21.00 rounded UP in favour) -> incoming 22.00
  importTransactions(db, { iban: IBAN, transactions: parseCamt053(camtPayment('22.00', 'CRDT')) });
  const tx = lastTx();
  const r = vatSettle(db, {
    txAmountCents: tx.amount_cents, txDate: tx.date, bankAccountCode: tx.account_code,
    period: '2026-Q3', actor: 'agent:test',
  });
  assert.equal(r.difference_cents, -100); // received 1.00 more -> gain
  assert.equal(balance('2510'), 0);
  assert.equal(balance('4700'), -100);
  assert.equal(balance('1100'), -9900); // -121.00 purchase + 22.00 refund
});

test('vat settle: paying MORE than booked books a loss to the difference account', () => {
  bookQuarter();
  vatFile(db, { period: '2026-Q3', actor: 'agent:test' });
  // filed €11 (rounded up — not in favour) -> paid 11.00 vs 10.50 booked
  importTransactions(db, { iban: IBAN, transactions: parseCamt053(camtPayment('11.00')) });
  const tx = lastTx();
  const r = vatSettle(db, {
    txAmountCents: tx.amount_cents, txDate: tx.date, bankAccountCode: tx.account_code,
    period: '2026-Q3', actor: 'agent:test',
  });
  assert.equal(r.difference_cents, 50); // loss (debit)
  assert.equal(balance('4700'), 50);
});

test('vat settle: difference beyond €5 is rejected as the wrong amount', () => {
  bookQuarter();
  vatFile(db, { period: '2026-Q3', actor: 'agent:test' });
  importTransactions(db, { iban: IBAN, transactions: parseCamt053(camtPayment('20.00')) });
  const tx = lastTx();
  throwsCode(
    () => vatSettle(db, {
      txAmountCents: tx.amount_cents, txDate: tx.date, bankAccountCode: tx.account_code, actor: 'agent:test',
    }),
    'VAT_SETTLE_DIFFERENCE_TOO_LARGE',
  );
});

test('vat settle: nothing to settle without a filed balance', () => {
  bookQuarter();
  importTransactions(db, { iban: IBAN, transactions: parseCamt053(camtPayment('10.50')) });
  const tx = lastTx();
  throwsCode(
    () => vatSettle(db, {
      txAmountCents: tx.amount_cents, txDate: tx.date, bankAccountCode: tx.account_code, actor: 'agent:test',
    }),
    'VAT_SETTLE_NOTHING',
  );
});

test('vat settle: direction guard — incoming tx cannot pay a te-betalen balance', () => {
  bookQuarter();
  vatFile(db, { period: '2026-Q3', actor: 'agent:test' });
  importTransactions(db, { iban: IBAN, transactions: parseCamt053(camtPayment('10.50', 'CRDT')) });
  const tx = lastTx();
  throwsCode(
    () => vatSettle(db, {
      txAmountCents: tx.amount_cents, txDate: tx.date, bankAccountCode: tx.account_code, actor: 'agent:test',
    }),
    'VAT_SETTLE_DIRECTION',
  );
});

test('vat settle: invalid difference account is rejected', () => {
  bookQuarter();
  vatFile(db, { period: '2026-Q3', actor: 'agent:test' });
  importTransactions(db, { iban: IBAN, transactions: parseCamt053(camtPayment('10.50')) });
  const tx = lastTx();
  throwsCode(
    () => vatSettle(db, {
      txAmountCents: tx.amount_cents, txDate: tx.date, bankAccountCode: tx.account_code,
      differenceAccount: '9999', actor: 'agent:test',
    }),
    'INVALID_DIFFERENCE_ACCOUNT',
  );
});

test('vat settle: dry-run books nothing and leaves the tx unmatched', () => {
  bookQuarter();
  vatFile(db, { period: '2026-Q3', actor: 'agent:test' });
  importTransactions(db, { iban: IBAN, transactions: parseCamt053(camtPayment('10.00')) });
  const tx = lastTx();
  const r = vatSettle(db, {
    txAmountCents: tx.amount_cents, txDate: tx.date, bankAccountCode: tx.account_code,
    period: '2026-Q3', actor: 'agent:test', dryRun: true,
  });
  assert.equal(r.dryRun, true);
  assert.equal(r.difference_cents, -50);
  assert.equal(balance('2510'), -1050); // untouched
  assert.equal(db.prepare("SELECT COUNT(*) c FROM journal_entries WHERE description LIKE '%Betaling OB%'").get().c, 0);
  assert.equal(lastTx().state, 'unmatched');
});

test('vat settle: custom difference account (e.g. dedicated Afrondingsverschillen)', () => {
  db.prepare("INSERT INTO accounts (code, name, type, normal_balance, active) VALUES ('4850', 'Afrondingsverschillen', 'expense', 'debit', 1)").run();
  bookQuarter();
  vatFile(db, { period: '2026-Q3', actor: 'agent:test' });
  importTransactions(db, { iban: IBAN, transactions: parseCamt053(camtPayment('10.00')) });
  const tx = lastTx();
  const r = vatSettle(db, {
    txAmountCents: tx.amount_cents, txDate: tx.date, bankAccountCode: tx.account_code,
    differenceAccount: '4850', actor: 'agent:test',
  });
  assert.equal(r.difference_account, '4850');
  assert.equal(balance('4850'), -50);
  assert.equal(balance('4700'), 0);
});

test('vat file + settle round-trip: readout 5d agrees with the booked net position', () => {
  bookQuarter();
  const readout = obReadout(db, { period: '2026-07' });
  assert.equal(readout.to_pay_cents, 1050);
  assert.equal(vatNetPosition(db), readout.to_pay_cents);
});
