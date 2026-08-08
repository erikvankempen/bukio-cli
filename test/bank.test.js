/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

import { test, beforeEach } from 'node:test';
import assert from 'node:assert/strict';
import { openDb } from '../src/core/db.js';
import { seedDefaultChart } from '../src/core/accounts.js';
import { createEntry, postEntry } from '../src/core/entries.js';
import { parseCamt053 } from '../src/bank/camt.js';
import { parseBankCsv, parseBankAmount } from '../src/bank/csv.js';
import {
  autoMatch, getOrCreateBankAccount, importTransactions, linkTransaction,
  listBankAccounts, listTransactions, postFromTransaction, previewImport,
  setTransactionState, suggestUnmatched,
} from '../src/bank/index.js';

const IBAN = 'NL91ABNA0417164300';

const CAMT = `<?xml version="1.0" encoding="UTF-8"?>
<Document xmlns="urn:iso:std:iso:20022:tech:xsd:camt.053.001.02">
  <BkToCstmrStmt>
    <Stmt>
      <Acct><Id><IBAN>${IBAN}</IBAN></Id></Acct>
      <Ntry>
        <Amt>100.00</Amt>
        <CdtDbtInd>CRDT</CdtDbtInd>
        <BookgDt><Dt>2026-06-01</Dt></BookgDt>
        <NtryDtls><TxDtls>
          <RltdPties><Dbtr><Nm>ACME B.V.</Nm></Dbtr></RltdPties>
          <RmtInf><Ustrd>Factuur 2026-001</Ustrd></RmtInf>
        </TxDtls></NtryDtls>
      </Ntry>
      <Ntry>
        <Amt>25.50</Amt>
        <CdtDbtInd>DBIT</CdtDbtInd>
        <BookgDt><Dt>2026-06-02</Dt></BookgDt>
        <NtryDtls><TxDtls>
          <RltdPties><Cdtr><Nm>Kantoorwinkel BV</Nm></Cdtr></RltdPties>
          <RmtInf><Ustrd>Kantoorartikelen</Ustrd></RmtInf>
        </TxDtls></NtryDtls>
      </Ntry>
    </Stmt>
  </BkToCstmrStmt>
</Document>`;

const RABO_CSV = [
  'Datum;Naam / Omschrijving;Rekening;Tegenrekening;Code;Af Bij;Bedrag (EUR);MutatieSoort;Mededelingen',
  `2026-06-01;ACME B.V.;${IBAN};NL00RABO0123456789;GT;Bij;100,00;Overschrijving;Factuur 2026-001`,
  `2026-06-02;Kantoorwinkel BV;${IBAN};NL00RABO9876543210;GT;Af;25,50;Overschrijving;Kantoorartikelen`,
].join('\n');

let db;

beforeEach(() => {
  db = openDb(':memory:');
  seedDefaultChart(db);
});

test('parseCamt053: CRDT positive, DBIT negative, counterparty + description', () => {
  const txs = parseCamt053(CAMT);
  assert.equal(txs.length, 2);
  assert.equal(txs[0].amount_cents, 10000);
  assert.equal(txs[0].date, '2026-06-01');
  assert.equal(txs[0].counterparty, 'ACME B.V.');
  assert.equal(txs[0].description, 'Factuur 2026-001');
  assert.equal(txs[1].amount_cents, -2550);
  assert.equal(txs[1].counterparty, 'Kantoorwinkel BV');
});

test('parseCamt053: rejects non-CAMT input', () => {
  assert.throws(() => parseCamt053('<foo><bar/></foo>'), { code: 'INVALID_CAMT' });
  assert.throws(() => parseCamt053('not xml at all'), { code: 'INVALID_CAMT' });
});

test('parseBankAmount: Dutch and international formats', () => {
  assert.equal(parseBankAmount('100,00'), 10000);
  assert.equal(parseBankAmount('1.234,56'), 123456);
  assert.equal(parseBankAmount('1234.56'), 123456);
  assert.equal(parseBankAmount('1.234'), 123400);
  assert.equal(parseBankAmount('-12,50'), -1250);
  assert.equal(parseBankAmount('€ 12,50'), 1250);
  assert.equal(parseBankAmount('0,50'), 50);
  assert.equal(parseBankAmount('.50'), 50); // leading-dot amounts
  assert.equal(parseBankAmount('-5.00'), -500);
  assert.equal(parseBankAmount('abc'), null);
});

test('parseBankCsv: Rabo-style export with Af/Bij sign', () => {
  const txs = parseBankCsv(RABO_CSV, { defaultIban: IBAN });
  assert.equal(txs.length, 2);
  assert.equal(txs[0].amount_cents, 10000); // Bij
  assert.equal(txs[1].amount_cents, -2550); // Af
  assert.equal(txs[0].counterparty, 'ACME B.V.');
  assert.equal(txs[0].iban, IBAN);
});

test('parseBankCsv: missing required columns rejected', () => {
  assert.throws(() => parseBankCsv('foo;bar\n1;2\n'), { code: 'INVALID_CSV_HEADER' });
});

test('importTransactions: idempotent via hash (duplicates skipped)', () => {
  const txs = parseCamt053(CAMT);
  const first = importTransactions(db, { iban: IBAN, transactions: txs });
  assert.equal(first.imported, 2);
  const second = importTransactions(db, { iban: IBAN, transactions: txs });
  assert.equal(second.imported, 0);
  assert.equal(second.duplicates, 2);
  assert.equal(listTransactions(db).length, 2);
});

test('previewImport: dry-run counts without writing', () => {
  const txs = parseCamt053(CAMT);
  importTransactions(db, { iban: IBAN, transactions: txs });
  const preview = previewImport(db, { iban: IBAN, transactions: txs });
  assert.equal(preview.imported, 0);
  assert.equal(preview.duplicates, 2);
});

test('getOrCreateBankAccount: validates IBAN and links to ledger account', () => {
  const account = getOrCreateBankAccount(db, { iban: IBAN, name: 'Betaalrekening', accountCode: '1100' });
  assert.equal(account.iban, IBAN);
  assert.equal(account.account_code, '1100');
  assert.throws(() => getOrCreateBankAccount(db, { iban: 'not-an-iban' }), { code: 'INVALID_IBAN' });
  assert.throws(() => getOrCreateBankAccount(db, { iban: IBAN, accountCode: '9999' }), { code: 'ACCOUNT_NOT_FOUND' });
});

test('listBankAccounts: balance and counts', () => {
  importTransactions(db, { iban: IBAN, transactions: parseCamt053(CAMT) });
  const accounts = listBankAccounts(db);
  assert.equal(accounts.length, 1);
  assert.equal(accounts[0].balance_cents, 7450); // 10000 - 2550
  assert.equal(accounts[0].unmatched_count, 2);
});

test('postFromTransaction: posts bank + counter leg and reconciles', () => {
  importTransactions(db, { iban: IBAN, transactions: parseCamt053(CAMT) });
  const txs = listTransactions(db, { state: 'unmatched' });
  const income = txs.find((t) => t.amount_cents > 0);
  const expense = txs.find((t) => t.amount_cents < 0);

  const { entry, transaction } = postFromTransaction(db, { txId: income.id, accountCode: '8000', actor: 'agent:test' });
  assert.equal(entry.state, 'posted');
  assert.equal(entry.source, 'bank');
  assert.equal(entry.source_ref, `tx:${income.id}`);
  assert.equal(transaction.state, 'matched');
  assert.equal(entry.postings.length, 2);

  const expensePost = postFromTransaction(db, { txId: expense.id, accountCode: '4300' });
  assert.equal(expensePost.entry.postings[1].account_code, '4300');
});

test('postFromTransaction: refuses already-matched transactions', () => {
  importTransactions(db, { iban: IBAN, transactions: parseCamt053(CAMT) });
  const tx = listTransactions(db, { state: 'unmatched' })[0];
  postFromTransaction(db, { txId: tx.id, accountCode: '8000' });
  assert.throws(() => postFromTransaction(db, { txId: tx.id, accountCode: '8000' }), { code: 'ALREADY_MATCHED' });
});

test('linkTransaction: links a posted entry and guards', () => {
  importTransactions(db, { iban: IBAN, transactions: parseCamt053(CAMT) });
  const tx = listTransactions(db, { state: 'unmatched' })[0];
  const entry = createEntry(db, {
    date: '2026-06-01', description: 'Factuur 2026-001',
    postings: [{ code: '1100', amountCents: tx.amount_cents }, { code: '8000', amountCents: -tx.amount_cents }],
  });
  postEntry(db, { id: entry.id });

  const linked = linkTransaction(db, { txId: tx.id, entryId: entry.id, method: 'exact', confidence: 0.99 });
  assert.equal(linked.state, 'matched');
  assert.throws(() => linkTransaction(db, { txId: tx.id, entryId: entry.id }), { code: 'ALREADY_MATCHED' });

  const draft = createEntry(db, {
    date: '2026-06-01', description: 'draft',
    postings: [{ code: '1100', amountCents: 100 }, { code: '3000', amountCents: -100 }],
  });
  const tx2 = listTransactions(db, { state: 'unmatched' })[0];
  assert.throws(() => linkTransaction(db, { txId: tx2.id, entryId: draft.id }), { code: 'NOT_POSTED' });
});

test('autoMatch: exact and fuzzy matching, dry-run writes nothing', () => {
  importTransactions(db, { iban: IBAN, transactions: parseCamt053(CAMT) });
  // post an entry that matches tx #1 exactly (same amount, same date)
  const entry = createEntry(db, {
    date: '2026-06-01', description: 'Factuur 2026-001',
    postings: [{ code: '1100', amountCents: 10000 }, { code: '8000', amountCents: -10000 }],
  });
  postEntry(db, { id: entry.id });

  const dry = autoMatch(db, { dryRun: true });
  assert.equal(dry.matched.length, 1);
  assert.equal(dry.matched[0].method, 'exact');
  assert.equal(dry.matched[0].entry_id, entry.id);
  assert.equal(listTransactions(db, { state: 'matched' }).length, 0); // nothing written

  const real = autoMatch(db, { actor: 'agent:test' });
  assert.equal(real.matched.length, 1);
  assert.equal(listTransactions(db, { state: 'matched' }).length, 1);

  // fuzzy: entry 5 days away
  const tx2 = listTransactions(db, { state: 'unmatched' })[0];
  const lateEntry = createEntry(db, {
    date: '2026-06-07', description: 'late entry',
    postings: [{ code: '1100', amountCents: tx2.amount_cents }, { code: '4300', amountCents: -tx2.amount_cents }],
  });
  postEntry(db, { id: lateEntry.id });
  const fuzzy = autoMatch(db);
  assert.equal(fuzzy.matched.length, 1);
  assert.equal(fuzzy.matched[0].method, 'fuzzy');
});

test('autoMatch: two same-amount transactions never claim the same entry in one run', () => {
  // two incoming €100 transfers within the window, one €100 entry — the
  // first transaction claims it; the second must stay unmatched instead of
  // reconciling the same entry twice (books €100, bank €200)
  importTransactions(db, {
    iban: IBAN,
    transactions: [
      { date: '2026-06-01', amount_cents: 10000, counterparty: 'A', description: 'p1', iban_counter: null },
      { date: '2026-06-02', amount_cents: 10000, counterparty: 'B', description: 'p2', iban_counter: null },
    ],
  });
  const entry = createEntry(db, {
    date: '2026-06-01', description: 'Factuur',
    postings: [{ code: '1100', amountCents: 10000 }, { code: '8000', amountCents: -10000 }],
  });
  postEntry(db, { id: entry.id });

  const r = autoMatch(db, { actor: 'agent:test' });
  assert.equal(r.matched.length, 1);
  assert.equal(r.matched[0].kind, 'entry');
  assert.equal(r.matched[0].entry_id, entry.id);
  assert.equal(r.unmatched_remaining, 1);
  // the entry has exactly ONE reconciliation, one transaction stays unmatched
  const recs = db.prepare("SELECT * FROM reconciliations WHERE target_type = 'entry' AND target_id = ?").all(entry.id);
  assert.equal(recs.length, 1);
  assert.equal(listTransactions(db, { state: 'matched' }).length, 1);
  assert.equal(listTransactions(db, { state: 'unmatched' }).length, 1);
});

test('autoMatch: two same-amount transactions match TWO distinct entries (param order regression)', () => {
  // regression for the usedEntryIds parameter-order bug: with the exclusion
  // active, .all() passed (bank_account_id, ...used) against SQL placeholders
  // (...used, bank_account_id), so the NOT IN got the bank account id and the
  // bank filter got a used entry id — the second same-amount transaction then
  // double-matched the first entry. Only visible when entry ids differ from
  // the bank account id (the older test had both == 1 and masked it).
  importTransactions(db, {
    iban: IBAN,
    transactions: [
      { date: '2026-07-01', amount_cents: -50000, counterparty: 'Leverancier', description: 'F1', bank_ref: 'REF-A', iban_counter: null },
      { date: '2026-07-02', amount_cents: -50000, counterparty: 'Leverancier', description: 'F2', bank_ref: 'REF-B', iban_counter: null },
    ],
  });
  for (const [date, desc] of [['2026-06-25', 'Betaling A'], ['2026-06-26', 'Betaling B']]) {
    const e = createEntry(db, {
      date, description: desc,
      postings: [{ code: '1100', amountCents: -50000 }, { code: '4300', amountCents: 50000 }],
    });
    postEntry(db, { id: e.id });
  }
  const r = autoMatch(db, { actor: 'agent:test', windowDays: 10 });
  assert.equal(r.matched.length, 2);
  const targets = r.matched.map((m) => m.entry_id);
  assert.equal(new Set(targets).size, 2, `each transaction must match a DISTINCT entry, got ${targets}`);
  // each entry reconciled exactly once
  const recs = db.prepare("SELECT target_id, COUNT(*) c FROM reconciliations WHERE target_type='entry' GROUP BY target_id").all();
  assert.ok(recs.every((x) => x.c === 1), `no entry may carry two bank legs: ${JSON.stringify(recs)}`);
});

test('autoMatch: outside the window stays unmatched', () => {
  importTransactions(db, { iban: IBAN, transactions: parseCamt053(CAMT) });
  createEntry(db, {
    date: '2026-01-01', description: 'old entry',
    postings: [{ code: '1100', amountCents: 10000 }, { code: '8000', amountCents: -10000 }],
  });
  const e = createEntry(db, {
    date: '2026-01-01', description: 'old entry posted',
    postings: [{ code: '1100', amountCents: 10000 }, { code: '8000', amountCents: -10000 }],
  });
  postEntry(db, { id: e.id });
  const result = autoMatch(db, { windowDays: 5 });
  assert.equal(result.matched.length, 0);
  assert.equal(result.unmatched_remaining, 2);
});

test('setTransactionState: ignore and re-open', () => {
  importTransactions(db, { iban: IBAN, transactions: parseCamt053(CAMT) });
  const tx = listTransactions(db)[0];
  setTransactionState(db, { id: tx.id, state: 'ignored' });
  assert.equal(listTransactions(db, { state: 'ignored' }).length, 1);
  setTransactionState(db, { id: tx.id, state: 'unmatched' });
  assert.equal(listTransactions(db, { state: 'ignored' }).length, 0);
});

test('suggestUnmatched: proposes expense/income accounts', () => {
  importTransactions(db, { iban: IBAN, transactions: parseCamt053(CAMT) });
  const suggestions = suggestUnmatched(db);
  assert.equal(suggestions.length, 2);
  assert.equal(suggestions.find((s) => s.amount_cents > 0).suggested_account, '8000');
  assert.equal(suggestions.find((s) => s.amount_cents < 0).suggested_account, '4300');
});
