/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across eleven jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync, spawn } from 'node:child_process';
import { mkdtempSync, rmSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { openDb } from '../src/core/db.js';
import { createContact } from '../src/invoice/index.js';
import { addPayable, addMandate, listMandates, removeMandate, createPaymentBatch, exportPaymentBatch, buildPain008 } from '../src/payments/index.js';

const BIN = path.join(path.dirname(fileURLToPath(import.meta.url)), '..', 'bin', 'bukio.js');

function cli(dbPath, args, { expectFail = false } = {}) {
  const env = { ...process.env, BUKIO_DB: dbPath, BUKIO_ACTOR: 'agent:test' };
  try {
    const stdout = execFileSync(process.execPath, [BIN, '--json', ...args], { env, encoding: 'utf8' });
    return { code: 0, out: JSON.parse(stdout) };
  } catch (err) {
    if (expectFail) return { code: err.status, out: JSON.parse(err.stdout), err: err.stderr };
    throw err;
  }
}

function mcpSession(dbPath) {
  const child = spawn(process.execPath, [BIN, 'mcp', '--db', dbPath], {
    cwd: path.join(path.dirname(fileURLToPath(import.meta.url)), '..'),
    env: { ...process.env, BUKIO_ACTOR: 'agent:test' },
    stdio: ['pipe', 'pipe', 'pipe'],
  });
  let buf = '';
  let nextId = 1;
  const pending = new Map();
  child.stdout.on('data', (chunk) => {
    buf += chunk;
    let idx;
    while ((idx = buf.indexOf('\n')) >= 0) {
      const line = buf.slice(0, idx);
      buf = buf.slice(idx + 1);
      if (!line.trim()) continue;
      let msg;
      try { msg = JSON.parse(line); } catch { continue; }
      if (msg.id && pending.has(msg.id)) { pending.get(msg.id)(msg); pending.delete(msg.id); }
    }
  });
  return {
    call(method, params) {
      const id = nextId++;
      return new Promise((resolve, reject) => {
        pending.set(id, resolve);
        child.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', id, method, params })}\n`);
        setTimeout(() => {
          if (pending.has(id)) { pending.delete(id); reject(new Error(`MCP timeout for ${method}`)); }
        }, 10000);
      });
    },
    close() { child.kill(); },
  };
}

let t;
let db;
let file;

test.beforeEach(() => {
  const dir = mkdtempSync(path.join(os.tmpdir(), 'bukio-dd-test-'));
  file = path.join(dir, 'test.db');
  cli(file, ['init', '--name', 'Test Coaching', '--kvk', '12345678', '--legal-form', 'eenmanszaak', '--vat', 'off']);
  cli(file, ['company', 'update', '--address', 'Teststraat 1', '--postal-code', '1000 AA', '--city', 'Amsterdam', '--iban', 'NL91ABNA0417164300']);
  db = openDb(file);
  t = dir;
});
test.afterEach(() => {
  db.close();
  rmSync(t, { recursive: true, force: true });
});

function seedContact({ name = 'Debiteur BV', iban = 'NL91ABNA0417164300' } = {}) {
  return createContact(db, { name, address: 'Klantstraat 1', city: 'Amsterdam', iban, actor: 'agent:test' });
}

function seedPayable(contactId, { ref = 'F2026-10', method = 'direct_debit', amount = 12100 } = {}) {
  return addPayable(db, { contact: contactId, invoiceRef: ref, date: '2026-08-01', dueDate: '2026-08-31', amountCents: amount, method, actor: 'agent:test' });
}

test('mandates: add/list/remove with audit; guards', () => {
  const c = seedContact();
  const m = addMandate(db, { contactId: c.id, mandateRef: 'NL01ZZZ123456789012', mandateDate: '2026-07-01', scheme: 'b2b', actor: 'agent:test' });
  assert.equal(m.scheme, 'b2b');
  assert.equal(m.contact_name, 'Debiteur BV');

  const rows = listMandates(db);
  assert.equal(rows.length, 1);
  assert.equal(rows[0].mandate_ref, 'NL01ZZZ123456789012');
  assert.equal(listMandates(db, { contactId: c.id }).length, 1);
  assert.equal(listMandates(db, { contactId: 999999 }).length, 0);

  // duplicate ref for same contact
  assert.throws(
    () => addMandate(db, { contactId: c.id, mandateRef: 'NL01ZZZ123456789012', actor: 'agent:test' }),
    (e) => e.code === 'MANDATE_DUPLICATE',
  );
  // same ref for ANOTHER contact is fine
  const c2 = seedContact({ name: 'Tweede BV', iban: 'NL91ABNA0417164300' });
  addMandate(db, { contactId: c2.id, mandateRef: 'NL01ZZZ123456789012', actor: 'agent:test' });
  assert.equal(listMandates(db).length, 2);

  // guards
  assert.throws(() => addMandate(db, { contactId: 999999, mandateRef: 'R1' }), (e) => e.code === 'CONTACT_NOT_FOUND');
  assert.throws(() => addMandate(db, { contactId: c.id, mandateRef: '' }), (e) => e.code === 'INVALID_MANDATE_REF');
  assert.throws(() => addMandate(db, { contactId: c.id, mandateRef: 'x'.repeat(36) }), (e) => e.code === 'INVALID_MANDATE_REF');
  assert.throws(() => addMandate(db, { contactId: c.id, mandateRef: 'R2', scheme: 'sct' }), (e) => e.code === 'INVALID_SCHEME');
  assert.throws(() => addMandate(db, { contactId: c.id, mandateRef: 'R3', mandateDate: '2026-02-30' }), (e) => e.code === 'INVALID_DATE');

  // dry-run writes nothing
  const plan = addMandate(db, { contactId: c.id, mandateRef: 'R4', actor: 'agent:test', dryRun: true });
  assert.equal(plan.dryRun, true);
  assert.equal(listMandates(db).length, 2);

  // remove
  const r = removeMandate(db, { id: m.id, actor: 'agent:test' });
  assert.equal(r.status, 'deleted');
  assert.equal(listMandates(db).length, 1);
  assert.throws(() => removeMandate(db, { id: 999999 }), (e) => e.code === 'MANDATE_NOT_FOUND');

  const audit = db.prepare("SELECT action FROM audit_log WHERE action IN ('payments.mandate.add','payments.mandate.remove') ORDER BY id").all();
  assert.equal(audit.length, 3); // 1 add + 1 remove (dry-runs don't audit)
});

test('direct-debit batch: FRST then RCUR, mandate snapshot on the line', () => {
  const c = seedContact();
  addMandate(db, { contactId: c.id, mandateRef: 'NL01ZZZ999', mandateDate: '2026-07-01', actor: 'agent:test' });
  const p1 = seedPayable(c.id, { ref: 'INV-1' });

  const batch = createPaymentBatch(db, { date: '2026-08-10', payableIds: [p1.id], kind: 'direct_debit', actor: 'agent:test' });
  assert.equal(batch.batch_kind, 'direct_debit');
  assert.equal(batch.lines.length, 1);
  assert.equal(batch.lines[0].mandate_ref, 'NL01ZZZ999');
  assert.equal(batch.lines[0].mandate_seq, 'FRST');
  assert.equal(batch.lines[0].scheme, 'core');

  // second batch for the same contact → RCUR
  const p2 = seedPayable(c.id, { ref: 'INV-2' });
  const batch2 = createPaymentBatch(db, { date: '2026-09-10', payableIds: [p2.id], kind: 'direct_debit', actor: 'agent:test' });
  assert.equal(batch2.lines[0].mandate_seq, 'RCUR');

  // NEW mandate (after revocation) starts at FRST again — SEPA per-mandate rule
  addMandate(db, { contactId: c.id, mandateRef: 'NL01ZZZ888', mandateDate: '2026-10-01', actor: 'agent:test' });
  const p3 = seedPayable(c.id, { ref: 'INV-3' });
  const batch3 = createPaymentBatch(db, { date: '2026-11-10', payableIds: [p3.id], kind: 'direct_debit', actor: 'agent:test' });
  assert.equal(batch3.lines[0].mandate_ref, 'NL01ZZZ888');
  assert.equal(batch3.lines[0].mandate_seq, 'FRST', 'a brand-new mandate must start at FRST, not RCUR');

  // mandate REMOVED and RE-ADDED with the same ref = a NEW mandate (SEPA) —
  // its first batch must be FRST, not RCUR. Regression: counting by the ref
  // snapshot alone used to see the old lines and emit RCUR here.
  const m1 = listMandates(db, { contactId: c.id }).find((m) => m.mandate_ref === 'NL01ZZZ999');
  removeMandate(db, { id: m1.id, actor: 'agent:test' });
  addMandate(db, { contactId: c.id, mandateRef: 'NL01ZZZ999', mandateDate: '2026-12-01', actor: 'agent:test' });
  const p4 = seedPayable(c.id, { ref: 'INV-4' });
  const batch4 = createPaymentBatch(db, { date: '2026-12-10', payableIds: [p4.id], kind: 'direct_debit', actor: 'agent:test' });
  assert.equal(batch4.lines[0].mandate_ref, 'NL01ZZZ999');
  assert.equal(batch4.lines[0].mandate_seq, 'FRST', 'a re-created mandate with the same ref must start at FRST (SEPA per-mandate rule)');
});

test('direct-debit batch without a mandate → MANDATE_REQUIRED', () => {
  const c = seedContact();
  const p = seedPayable(c.id);
  assert.throws(
    () => createPaymentBatch(db, { date: '2026-08-10', payableIds: [p.id], kind: 'direct_debit', actor: 'agent:test' }),
    (e) => e.code === 'BATCH_VALIDATION_FAILED' && e.details.some((d) => d.error.startsWith('MANDATE_REQUIRED')),
  );
});

test('payment-term isolation: transfer batch rejects direct-debit payables and vice versa', () => {
  const c = seedContact();
  addMandate(db, { contactId: c.id, mandateRef: 'M1', actor: 'agent:test' });
  const ddPayable = seedPayable(c.id, { ref: 'DD-1', method: 'direct_debit' });
  const trPayable = seedPayable(c.id, { ref: 'TR-1', method: 'transfer' });

  assert.throws(
    () => createPaymentBatch(db, { payableIds: [ddPayable.id], kind: 'transfer', actor: 'agent:test' }),
    (e) => e.code === 'BATCH_VALIDATION_FAILED' && e.details.some((d) => d.error.startsWith('PAYABLE_DIRECT_DEBIT')),
  );
  assert.throws(
    () => createPaymentBatch(db, { payableIds: [trPayable.id], kind: 'direct_debit', actor: 'agent:test' }),
    (e) => e.code === 'BATCH_VALIDATION_FAILED' && e.details.some((d) => d.error.startsWith('PAYABLE_NOT_DIRECT_DEBIT')),
  );
});

test('buildPain008: structure, mandate data, NOTPROVIDED agents, CORE/B2B split', () => {
  const xml = buildPain008({
    msgId: 'BUKIO20260810123456', createdIso: '2026-08-10T10:00:00Z',
    debitName: 'Test Coaching', debitIban: 'NL91ABNA0417164300', batchDate: '2026-08-12',
    lines: [
      { name: 'Debiteur BV', iban: 'NL91ABNA0417164300', amount_cents: 12100, reference: 'Factuur INV-1', mandate_ref: 'NL01ZZZ999', mandate_date: '2026-07-01', mandate_seq: 'FRST', scheme: 'core' },
      { name: 'Grootbedrijf NV', iban: 'NL91ABNA0417164300', amount_cents: 25000, reference: 'Factuur B2B-2', mandate_ref: 'B2BMAND1', mandate_date: '2026-06-15', mandate_seq: 'RCUR', scheme: 'b2b' },
    ],
  });
  assert.match(xml, /xmlns="urn:iso:std:iso:20022:tech:xsd:pain\.008\.001\.02"/);
  assert.match(xml, /<CstmrDrctDbtInitn>/);
  assert.match(xml, /<PmtMtd>DD<\/PmtMtd>/);
  assert.match(xml, /<CtrlSum>371\.00<\/CtrlSum>/);
  assert.match(xml, /<ReqdColltnDt>2026-08-12<\/ReqdColltnDt>/);
  // two PmtInf blocks, one per scheme
  assert.match(xml, /<LclInstrm><Cd>CORE<\/Cd><\/LclInstrm>/);
  assert.match(xml, /<LclInstrm><Cd>B2B<\/Cd><\/LclInstrm>/);
  assert.equal((xml.match(/<PmtInf>/g) ?? []).length, 2);
  // mandate snapshot per transaction
  assert.match(xml, /<MndtId>NL01ZZZ999<\/MndtId>/);
  assert.match(xml, /<DtOfSgntr>2026-07-01<\/DtOfSgntr>/);
  assert.match(xml, /<DbtrAgt><FinInstnId><Othr><Id>NOTPROVIDED<\/Id><\/Othr><\/FinInstnId><\/DbtrAgt>/);
  assert.match(xml, /<Dbtr><Nm>Grootbedrijf NV<\/Nm><\/Dbtr>/);
  assert.equal((xml.match(/<DrctDbtTxInf>/g) ?? []).length, 2);
});

test('export: DD batch → pain.008.001.02, one export per batch; transfer regression', () => {
  const c = seedContact();
  addMandate(db, { contactId: c.id, mandateRef: 'M1', actor: 'agent:test' });
  const p = seedPayable(c.id);
  const batch = createPaymentBatch(db, { date: '2026-08-10', payableIds: [p.id], kind: 'direct_debit', actor: 'agent:test' });

  const plan = exportPaymentBatch(db, { id: batch.id, dryRun: true });
  assert.equal(plan.dryRun, true);
  assert.equal(plan.schema, 'pain.008.001.02');
  assert.match(plan.xml, /pain\.008\.001\.02/);

  const r = exportPaymentBatch(db, { id: batch.id, actor: 'agent:test' });
  assert.equal(r.status, 'exported');
  assert.equal(r.schema, 'pain.008.001.02');
  assert.ok(r.msg_id.length <= 35);

  const stored = db.prepare('SELECT * FROM payment_batches WHERE id = ?').get(batch.id);
  assert.equal(stored.schema, 'pain.008.001.02');
  assert.ok(stored.msg_id);

  // re-export blocked
  assert.throws(() => exportPaymentBatch(db, { id: batch.id, actor: 'agent:test' }), (e) => e.code === 'BATCH_ALREADY_EXPORTED');
  // wrong schema for DD
  const c2 = seedContact({ name: 'Tweede BV' });
  addMandate(db, { contactId: c2.id, mandateRef: 'M2', actor: 'agent:test' });
  const p2 = seedPayable(c2.id, { ref: 'INV-2' });
  const b2 = createPaymentBatch(db, { date: '2026-08-10', payableIds: [p2.id], kind: 'direct_debit', actor: 'agent:test' });
  assert.throws(() => exportPaymentBatch(db, { id: b2.id, schema: '001.03' }), (e) => e.code === 'INVALID_SCHEMA');

  // transfer batch still exports pain.001
  const tr = seedPayable(c2.id, { ref: 'TR-1', method: 'transfer' });
  const tb = createPaymentBatch(db, { date: '2026-08-10', payableIds: [tr.id], kind: 'transfer', actor: 'agent:test' });
  assert.equal(tb.batch_kind, 'transfer');
  const trExp = exportPaymentBatch(db, { id: tb.id, actor: 'agent:test' });
  assert.equal(trExp.schema, 'pain.001.001.03');
  assert.match(trExp.xml, /pain\.001\.001\.03/);
});

test('cli: mandate add/list + direct-debit batch create/export e2e', () => {
  cli(file, ['contact', 'add', '--name', 'Debiteur BV', '--address', 'Klantstraat 1', '--city', 'Amsterdam', '--iban', 'NL91ABNA0417164300']);
  const m = cli(file, ['payments', 'mandate', 'add', '--contact', '1', '--ref', 'NL01ZZZ999', '--date', '2026-07-01', '--type', 'b2b']);
  assert.equal(m.code, 0);
  assert.equal(m.out.data.scheme, 'b2b');

  const list = cli(file, ['payments', 'mandate', 'list']);
  assert.equal(list.out.data.mandates.length, 1);

  cli(file, ['payments', 'payables', 'add', '--contact', '1', '--ref', 'INV-1', '--date', '2026-08-01', '--due', '2026-08-31', '--amount', '121.00', '--method', 'direct-debit']);

  const plan = cli(file, ['payments', 'batch', 'create', '--type', 'direct-debit', '--from-invoices', '--date', '2026-08-10', '--dry-run']);
  assert.equal(plan.out.data.dryRun, true);
  assert.equal(plan.out.data.batch_kind, 'direct_debit');
  assert.equal(plan.out.data.lines[0].mandate_ref, 'NL01ZZZ999');

  const batch = cli(file, ['payments', 'batch', 'create', '--type', 'direct-debit', '--from-invoices', '--date', '2026-08-10']);
  assert.equal(batch.out.data.batch_kind, 'direct_debit');
  assert.equal(batch.out.data.lines[0].mandate_seq, 'FRST');

  const exp = cli(file, ['payments', 'batch', 'export', '--id', String(batch.out.data.id)]);
  assert.equal(exp.out.data.schema, 'pain.008.001.02');

  // the DD payable is now in_batch — a transfer batch must refuse to take it
  const bad = cli(file, ['payments', 'batch', 'create', '--type', 'transfer', '--payable', '1', '--date', '2026-08-10'], { expectFail: true });
  assert.equal(bad.out.error.code, 'PAYABLE_NOT_ELIGIBLE');
});

test('mcp: mandate add/list + batch create/export (dry-run parity + execute)', async () => {
  cli(file, ['contact', 'add', '--name', 'Debiteur BV', '--address', 'Klantstraat 1', '--city', 'Amsterdam', '--iban', 'NL91ABNA0417164300']);
  cli(file, ['payments', 'payables', 'add', '--contact', '1', '--ref', 'INV-1', '--date', '2026-08-01', '--due', '2026-08-31', '--amount', '121.00', '--method', 'direct-debit']);
  const mcp = mcpSession(file);
  try {
    await mcp.call('initialize', { protocolVersion: '2024-11-05', capabilities: {}, clientInfo: { name: 't', version: '1' } });

    const mandAdd = await mcp.call('tools/call', { name: 'payments_mandate_add', arguments: { contact_id: 1, mandate_ref: 'NL01ZZZ999', scheme: 'b2b', mode: 'execute' } });
    assert.equal(mandAdd.result.isError, false);
    const mandData = JSON.parse(mandAdd.result.content[0].text);
    assert.equal(mandData.mode, 'execute');
    assert.equal(mandData.scheme, 'b2b');

    const mandList = await mcp.call('tools/call', { name: 'payments_mandate_list', arguments: {} });
    const mandListData = JSON.parse(mandList.result.content[0].text);
    assert.equal(mandListData.mandates.length, 1);

    const plan = await mcp.call('tools/call', { name: 'payments_batch_create', arguments: { type: 'direct_debit', batch_date: '2026-08-10', payable_ids: [1] } });
    const planData = JSON.parse(plan.result.content[0].text);
    assert.equal(planData.mode, 'dry-run');
    assert.equal(planData.batch_kind, 'direct_debit');

    const exec = await mcp.call('tools/call', { name: 'payments_batch_create', arguments: { type: 'direct_debit', batch_date: '2026-08-10', payable_ids: [1], mode: 'execute' } });
    const execData = JSON.parse(exec.result.content[0].text);
    assert.equal(execData.mode, 'execute');
    assert.equal(execData.batch_kind, 'direct_debit');

    const exp = await mcp.call('tools/call', { name: 'payments_batch_export', arguments: { batch_id: execData.batch_id, mode: 'execute' } });
    const expData = JSON.parse(exp.result.content[0].text);
    assert.equal(expData.schema, 'pain.008.001.02');
    assert.match(expData.xml, /pain\.008\.001\.02/);
  } finally {
    mcp.close();
  }
});
