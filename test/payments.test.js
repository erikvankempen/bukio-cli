import { test, beforeEach } from 'node:test';
import assert from 'node:assert/strict';
import { openDb } from '../src/core/db.js';
import { seedDefaultChart } from '../src/core/accounts.js';
import { isValidIban } from '../src/core/iban.js';
import { createContact, updateContact, listContacts } from '../src/invoice/index.js';
import {
  addPayable, listPayables, markPayablePaid,
  createPaymentBatch, createPaymentBatchFromCsv, exportPaymentBatch,
  deletePaymentBatch, getPaymentBatch, buildPain001,
} from '../src/payments/index.js';
import { listEntries } from '../src/core/entries.js';

let db;

const IBAN = 'NL91ABNA0417164300'; // valid test IBAN (ABN AMRO)

beforeEach(() => {
  db = openDb(':memory:');
  seedDefaultChart(db);
  db.prepare("INSERT INTO company (name, kvk, legal_form, iban, vat_module) VALUES ('Demo BV', '12345678', 'bv', ?, 0)").run(IBAN);
});

const vendor = () => createContact(db, { name: 'Vimexx', iban: 'NL02ABNA0123456789', actor: 'agent:test' });

// --- IBAN validator ----------------------------------------------------------

test('isValidIban: mod-97 check with normalization', () => {
  assert.equal(isValidIban('NL91ABNA0417164300'), true);
  assert.equal(isValidIban('nl91 abna 0417 1643 00'), true); // spaces + lowercase normalize
  assert.equal(isValidIban('NL91ABNA0417164301'), false); // checksum off by one
  assert.equal(isValidIban('NL91ABNA04171643'), false); // too short
  assert.equal(isValidIban(''), false);
});

// --- payables -----------------------------------------------------------------

test('payables: add transfer + direct-debit, audit, list filters', () => {
  const v = vendor();
  const p1 = addPayable(db, { contact: 'Vimexx', invoiceRef: '2026-118', date: '2026-07-01', dueDate: '2026-08-01', amountCents: 12100, actor: 'agent:test' });
  assert.equal(p1.contact_id, v.id);
  const p2 = addPayable(db, { contact: v.id, invoiceRef: 'DD-1', date: '2026-07-02', amountCents: 9999, method: 'direct_debit' });
  assert.equal(p2.payment_method, 'direct_debit');
  assert.equal(listPayables(db, { status: 'unpaid' }).length, 2);
  assert.equal(listPayables(db, { method: 'transfer' }).length, 1);
  assert.equal(listPayables(db, { method: 'direct_debit' })[0].invoice_ref, 'DD-1');
  const audit = db.prepare("SELECT * FROM audit_log WHERE action = 'payables.add'").all();
  assert.equal(audit.length, 2);
});

test('payables: unknown contact, bad amount, missing ref rejected', () => {
  vendor();
  assert.throws(() => addPayable(db, { contact: 'Nobody', invoiceRef: 'X', date: '2026-07-01', amountCents: 100 }), (e) => e.code === 'CONTACT_NOT_FOUND');
  assert.throws(() => addPayable(db, { contact: 'Vimexx', invoiceRef: 'X', date: '2026-07-01', amountCents: 0 }), (e) => e.code === 'INVALID_AMOUNT');
  assert.throws(() => addPayable(db, { contact: 'Vimexx', invoiceRef: ' ', date: '2026-07-01', amountCents: 100 }), (e) => e.code === 'INVOICE_REF_REQUIRED');
  assert.throws(() => addPayable(db, { contact: 'Vimexx', invoiceRef: 'X', date: '2026-07-01', amountCents: 100, method: 'card' }), (e) => e.code === 'INVALID_METHOD');
});

test('payables: mark paid (dry-run writes nothing, real is audited)', () => {
  vendor();
  const p = addPayable(db, { contact: 'Vimexx', invoiceRef: 'R1', date: '2026-07-01', amountCents: 5000 });
  const dry = markPayablePaid(db, { id: p.id, dryRun: true });
  assert.equal(dry.dryRun, true);
  assert.equal(listPayables(db, { status: 'unpaid' }).length, 1);
  const r = markPayablePaid(db, { id: p.id, actor: 'agent:test' });
  assert.equal(r.status, 'paid');
  assert.throws(() => markPayablePaid(db, { id: p.id }), (e) => e.code === 'ALREADY_PAID');
  assert.ok(db.prepare("SELECT 1 FROM audit_log WHERE action = 'payables.pay'").get());
});

// --- contact IBAN -------------------------------------------------------------

test('contacts: iban on create (validated) and update (audited)', () => {
  assert.throws(() => createContact(db, { name: 'Bad', iban: 'NL00INVALID' }), (e) => e.code === 'INVALID_IBAN');
  const c = updateContact(db, { id: vendor().id, iban: ' NL86 INGB 0002 4455 88 ', actor: 'agent:test' });
  assert.equal(c.iban, 'NL86INGB0002445588'); // normalized
  assert.ok(db.prepare("SELECT 1 FROM audit_log WHERE action = 'contact.update'").get());
});

// --- batch creation -----------------------------------------------------------

test('batch: explicit lines resolve contacts by name or id', () => {
  vendor();
  const b = createPaymentBatch(db, {
    lines: [
      { contact: 'Vimexx', amountCents: 12100, reference: 'Factuur 2026-118' },
      { contact: 1, amountCents: 4550 },
    ],
    actor: 'agent:test',
  });
  assert.equal(b.lines.length, 2);
  assert.equal(b.total_cents, 16650);
  assert.equal(b.debit_iban, IBAN);
  assert.equal(b.lines[0].iban, 'NL02ABNA0123456789');
  assert.equal(listEntries(db).length, 0); // ledger untouched
});

test('batch: company without IBAN fails with a hint', () => {
  db.prepare('UPDATE company SET iban = NULL').run();
  vendor();
  assert.throws(
    () => createPaymentBatch(db, { lines: [{ contact: 'Vimexx', amountCents: 100 }] }),
    (e) => e.code === 'COMPANY_INCOMPLETE' && e.message.includes('company update --iban'),
  );
});

test('batch: contact without IBAN fails with a hint and details', () => {
  createContact(db, { name: 'NoIbanCo' });
  assert.throws(
    () => createPaymentBatch(db, { lines: [{ contact: 'NoIbanCo', amountCents: 100 }] }),
    (e) => e.code === 'BATCH_VALIDATION_FAILED' && e.details.some((d) => d.error.includes('CONTACT_IBAN_MISSING')),
  );
});

test('batch: invalid iban, zero amount, missing name, long reference all collected', () => {
  const c = createContact(db, { name: 'BadCo' });
  assert.throws(
    () => createPaymentBatch(db, {
      lines: [
        { contact: 'BadCo', iban: 'NL00BAD0000000000', amountCents: 100 }, // invalid iban
        { name: 'X', iban: IBAN, amountCents: 0 }, // zero amount
        { name: '', iban: IBAN, amountCents: 100 }, // missing name
        { name: 'Y', iban: IBAN, amountCents: 100, reference: 'r'.repeat(141) }, // too long
      ],
    }),
    (e) => e.code === 'BATCH_VALIDATION_FAILED' && e.details.length === 4,
  );
  assert.equal(c.id > 0, true);
});

test('batch: from payables excludes direct-debit and marks payables in_batch', () => {
  vendor();
  addPayable(db, { contact: 'Vimexx', invoiceRef: 'A1', date: '2026-07-01', amountCents: 12100 });
  addPayable(db, { contact: 'Vimexx', invoiceRef: 'A2', date: '2026-07-01', amountCents: 4550 });
  addPayable(db, { contact: 'Vimexx', invoiceRef: 'DD', date: '2026-07-01', amountCents: 9999, method: 'direct_debit' });
  const b = createPaymentBatch(db, { payableIds: listPayables(db, { status: 'unpaid', method: 'transfer' }).map((p) => p.id), actor: 'agent:test' });
  assert.equal(b.lines.length, 2); // DD excluded
  assert.equal(b.total_cents, 16650);
  assert.deepEqual(listPayables(db, { status: 'in_batch' }).map((p) => p.invoice_ref).sort(), ['A1', 'A2']);
  assert.deepEqual(listPayables(db, { status: 'unpaid' }).map((p) => p.invoice_ref), ['DD']);
  assert.equal(listEntries(db).length, 0);
});

test('batch: dry-run writes nothing; empty batch rejected', () => {
  vendor();
  const plan = createPaymentBatch(db, { lines: [{ contact: 'Vimexx', amountCents: 100 }], dryRun: true });
  assert.equal(plan.dryRun, true);
  assert.equal(plan.lines.length, 1);
  assert.equal(listPaymentBatchesCount(db), 0);
  assert.throws(() => createPaymentBatch(db, { lines: [] }), (e) => e.code === 'EMPTY_BATCH');
});

function listPaymentBatchesCount(d) {
  return d.prepare('SELECT COUNT(*) AS n FROM payment_batches').get().n;
}

// --- CSV ----------------------------------------------------------------------

test('batch CSV: comma and semicolon delimiters, Dutch amounts', () => {
  vendor();
  // comma-delimited: amount uses dot or plain integer (comma is the delimiter)
  const csv1 = 'contact,amount,reference\nVimexx,121.00,Factuur A\nVimexx,45,Factuur B';
  const b1 = createPaymentBatchFromCsv(db, { csvText: csv1, actor: 'agent:test' });
  assert.equal(b1.lines.length, 2);
  assert.equal(b1.total_cents, 16600); // 12100 + 4500
  // semicolon-delimited: Dutch comma decimals keep their meaning
  const csv2 = 'contact;amount;reference\nVimexx;121,00;Factuur A\nVimexx;45,50;Factuur B';
  const b2 = createPaymentBatchFromCsv(db, { csvText: csv2, actor: 'agent:test' });
  assert.equal(b2.lines.length, 2);
  assert.equal(b2.total_cents, 16650);
});

test('batch CSV: whole-file validation reports every bad line', () => {
  vendor();
  const csv = 'contact;amount;reference\nVimexx;abc;x\nNobody;10.00;y\nVimexx;-5;z\n';
  assert.throws(
    () => createPaymentBatchFromCsv(db, { csvText: csv }),
    (e) => e.code === 'IMPORT_VALIDATION_FAILED' && e.details.length === 2, // parse errors fail fast
  );
  assert.equal(listPaymentBatchesCount(db), 0);
  // a parse-clean CSV still hits contact resolution inside batch creation
  assert.throws(
    () => createPaymentBatchFromCsv(db, { csvText: 'contact;amount\nNobody;10.00' }),
    (e) => e.code === 'BATCH_VALIDATION_FAILED' && e.details.some((d) => d.error.includes('CONTACT_NOT_FOUND')),
  );
});

// --- export -------------------------------------------------------------------

test('export: pain.001.001.03 XML with totals, SEPA level, escaping; re-export blocked', () => {
  vendor();
  const b = createPaymentBatch(db, { lines: [{ contact: 'Vimexx', amountCents: 12100, reference: 'Factuur 2026-118' }], actor: 'agent:test' });
  const r = exportPaymentBatch(db, { id: b.id, actor: 'agent:test' });
  assert.equal(r.status, 'exported');
  assert.match(r.msg_id, /^BUKIO\d{14}1$/);
  assert.ok(r.file_hash.length === 64);
  assert.ok(r.xml.includes('urn:iso:std:iso:20022:tech:xsd:pain.001.001.03'));
  assert.ok(r.xml.includes('<MsgId>'));
  assert.ok(r.xml.includes('<NbOfTxs>1</NbOfTxs>'));
  assert.ok(r.xml.includes('<CtrlSum>121.00</CtrlSum>'));
  assert.ok(r.xml.includes('<SvcLvl><Cd>SEPA</Cd></SvcLvl>'));
  assert.ok(r.xml.includes('<ChrgBr>SLEV</ChrgBr>'));
  assert.ok(r.xml.includes(`<IBAN>${IBAN}</IBAN>`));
  assert.ok(r.xml.includes('Factuur 2026-118'));
  // one export per batch — re-exporting could double-pay
  assert.throws(() => exportPaymentBatch(db, { id: b.id }), (e) => e.code === 'BATCH_ALREADY_EXPORTED');
  assert.ok(db.prepare("SELECT 1 FROM audit_log WHERE action = 'payments.batch.export'").get());
  assert.equal(listEntries(db).length, 0);
});

test('export: escaping and .09 schema', () => {
  createContact(db, { name: 'Amp & Sons', iban: 'NL86INGB0002445588' });
  const b = createPaymentBatch(db, { lines: [{ contact: 'Amp & Sons', amountCents: 1234, reference: 'a < b & c > d' }] });
  const r = exportPaymentBatch(db, { id: b.id, schema: '001.09' });
  assert.ok(r.xml.includes('pain.001.001.09'));
  assert.ok(r.xml.includes('Amp &amp; Sons'));
  assert.ok(r.xml.includes('a &lt; b &amp; c &gt; d'));
});

test('buildPain001: batch date lands in ReqdExctnDt', () => {
  const xml = buildPain001({ msgId: 'M1', createdIso: '2026-08-06T10:00:00Z', debitName: 'Demo BV', debitIban: IBAN, batchDate: '2026-08-10', lines: [{ name: 'X', iban: IBAN, amount_cents: 100 }] });
  assert.ok(xml.includes('<ReqdExctnDt>2026-08-10</ReqdExctnDt>'));
});

// --- delete -------------------------------------------------------------------

test('delete: only drafts; payables released back to unpaid', () => {
  vendor();
  addPayable(db, { contact: 'Vimexx', invoiceRef: 'K1', date: '2026-07-01', amountCents: 1000 });
  const b = createPaymentBatch(db, { payableIds: [1], actor: 'agent:test' });
  assert.deepEqual(listPayables(db, { status: 'in_batch' }).map((p) => p.invoice_ref), ['K1']);
  const r = deletePaymentBatch(db, { id: b.id, actor: 'agent:test' });
  assert.equal(r.status, 'deleted');
  assert.equal(listPaymentBatchesCount(db), 0);
  assert.deepEqual(listPayables(db, { status: 'unpaid' }).map((p) => p.invoice_ref), ['K1']);
  // exported batches cannot be deleted
  const b2 = createPaymentBatch(db, { lines: [{ contact: 'Vimexx', amountCents: 100 }] });
  exportPaymentBatch(db, { id: b2.id });
  assert.throws(() => deletePaymentBatch(db, { id: b2.id }), (e) => e.code === 'BATCH_ALREADY_EXPORTED');
});

test('getPaymentBatch: serializes total + lines', () => {
  vendor();
  const b = createPaymentBatch(db, { lines: [{ contact: 'Vimexx', amountCents: 12100 }] });
  const fetched = getPaymentBatch(db, b.id);
  assert.equal(fetched.total, '121.00');
  assert.equal(fetched.lines.length, 1);
  assert.equal(fetched.lines[0].amount, '121.00');
  assert.equal(listContacts(db).length, 1); // sanity
});
