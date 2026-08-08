/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync, spawn } from 'node:child_process';
import { mkdtempSync, rmSync, readFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { openDb } from '../src/core/db.js';
import {
  createContact, createInvoice, finalizeInvoice, markPaid, contactStatement, creditInvoice,
} from '../src/invoice/index.js';
import { addPayable } from '../src/payments/index.js';
import { createItem } from '../src/items/index.js';
import { aging } from '../src/report/aging.js';
import { sales } from '../src/report/sales.js';

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
      if (msg.id && pending.has(msg.id)) {
        pending.get(msg.id)(msg);
        pending.delete(msg.id);
      }
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

function seed() {
  const dir = mkdtempSync(path.join(os.tmpdir(), 'bukio-reports-v014-'));
  const file = path.join(dir, 'test.db');
  cli(file, ['init', '--name', 'Test Coaching', '--kvk', '12345678', '--legal-form', 'eenmanszaak', '--vat', 'off']);
  cli(file, ['company', 'update', '--address', 'Teststraat 1', '--postal-code', '1000 AA', '--city', 'Amsterdam']);
  const d = openDb(file);
  const contacts = {
    acme: createContact(d, { name: 'Acme BV', address: 'Klantstraat 1', city: 'Amsterdam', actor: 'agent:test' }),
    beta: createContact(d, { name: 'Beta BV', address: 'Klantstraat 2', city: 'Utrecht', actor: 'agent:test' }),
    supplier: createContact(d, { name: 'Lever BV', address: 'Leverstraat 3', city: 'Rotterdam', actor: 'agent:test' }),
  };
  return { dir, file, d, contacts };
}

test.beforeEach(() => { t = seed(); db = t.d; });
test.afterEach(() => {
  db.close();
  rmSync(t.dir, { recursive: true, force: true });
});

function makeFinalized(contactId, { date, dueDays = 30, lines = ['Ding @ 100.00'], actor = 'agent:test' }) {
  const inv = createInvoice(db, { contactId, lines, date, dueDays, actor });
  return finalizeInvoice(db, { id: inv.id, actor });
}

test('aging debtors: buckets, totals, paid excluded, contacts sorted by total', () => {
  // Acme: invoice due 2026-06-01 → 68 days past as-of → d90 (61-90 band)
  makeFinalized(t.contacts.acme.id, { date: '2026-05-01', dueDays: 31 });
  // Acme: invoice due 2026-07-20 → 19 days past → d30
  const inv2 = makeFinalized(t.contacts.acme.id, { date: '2026-06-20', dueDays: 30 });
  // Acme: fully paid invoice — must NOT appear
  const inv3 = makeFinalized(t.contacts.acme.id, { date: '2026-07-01', dueDays: 30 });
  markPaid(db, { id: inv3.invoice.id, date: '2026-07-10', amountCents: inv3.invoice.gross_cents, actor: 'agent:test' });
  // Beta: due after as-of → current
  makeFinalized(t.contacts.beta.id, { date: '2026-08-01', dueDays: 30 });

  const r = aging(db, { asOf: '2026-08-08', kind: 'debtors' });
  assert.equal(r.kind, 'debtors');
  assert.equal(r.debtors.contacts.length, 2);
  const acme = r.debtors.contacts.find((c) => c.contact_id === t.contacts.acme.id);
  assert.equal(acme.buckets.d90, 10000); // €100 invoice, 68 days past due
  assert.equal(acme.buckets.d30, 10000);
  assert.equal(acme.buckets.current, 0);
  assert.equal(acme.buckets.d60, 0);
  assert.equal(acme.total_cents, 20000);
  assert.equal(acme.items.length, 2);
  assert.ok(acme.items.every((i) => i.outstanding_cents === 10000));
  const beta = r.debtors.contacts.find((c) => c.contact_id === t.contacts.beta.id);
  assert.equal(beta.buckets.current, 10000);
  assert.equal(r.debtors.totals.total_cents, 30000);
  assert.equal(r.debtors.totals.d90, 10000);
  assert.equal(r.debtors.totals.d30, 10000);
  assert.equal(r.debtors.totals.current, 10000);
});

test('aging creditors: buckets + in_batch shown separately', () => {
  addPayable(db, {
    contact: t.contacts.supplier.id, invoiceRef: 'F-1', date: '2026-05-01', dueDate: '2026-06-01',
    amountCents: 5000, actor: 'agent:test',
  });
  addPayable(db, {
    contact: t.contacts.supplier.id, invoiceRef: 'F-2', date: '2026-08-01', dueDate: '2026-08-20',
    amountCents: 2500, actor: 'agent:test',
  });
  // move one payable into a batch (simulating batch create)
  db.prepare("UPDATE payables SET status = 'in_batch' WHERE invoice_ref = 'F-1'").run();

  const r = aging(db, { asOf: '2026-08-08', kind: 'creditors' });
  assert.equal(r.creditors.contacts.length, 1);
  const sup = r.creditors.contacts[0];
  assert.equal(sup.in_batch_cents, 5000);
  assert.equal(sup.buckets.current, 2500); // due 2026-08-20 → not yet past
  assert.equal(sup.total_cents, 7500);
  assert.equal(r.creditors.totals.total_cents, 7500);
  // items carry their status
  assert.equal(sup.items.find((i) => i.ref === 'F-1').status, 'in_batch');
});

test('aging validation: bad as-of and kind rejected', () => {
  assert.throws(() => aging(db, { asOf: 'garbage', kind: 'debtors' }), (e) => e.code === 'INVALID_DATE');
  assert.throws(() => aging(db, { asOf: '2026-02-30', kind: 'debtors' }), (e) => e.code === 'INVALID_DATE');
  assert.throws(() => aging(db, { asOf: '2026-08-08', kind: 'bogus' }), (e) => e.code === 'INVALID_KIND');
});

test('contact statement: running balance ends at outstanding; supplier side negative', () => {
  const inv = makeFinalized(t.contacts.acme.id, { date: '2026-07-01', dueDays: 30 });
  markPaid(db, { id: inv.invoice.id, date: '2026-07-20', amountCents: 4000, actor: 'agent:test' }); // partial

  const r = contactStatement(db, { contactId: t.contacts.acme.id, asOf: '2026-08-08' });
  assert.equal(r.contact.name, 'Acme BV');
  assert.equal(r.rows.length, 2); // invoice + payment
  assert.equal(r.rows[0].kind, 'invoice');
  assert.equal(r.rows[0].debit_cents, inv.invoice.gross_cents);
  assert.equal(r.rows[1].kind, 'payment');
  assert.equal(r.rows[1].credit_cents, 4000);
  assert.equal(r.balance_cents, inv.invoice.gross_cents - 4000);
  assert.equal(r.rows[r.rows.length - 1].balance_cents, r.balance_cents);
  assert.throws(() => contactStatement(db, { contactId: 999999 }), (e) => e.code === 'CONTACT_NOT_FOUND');
  assert.throws(() => contactStatement(db, { contactId: t.contacts.acme.id, asOf: 'abc' }), (e) => e.code === 'INVALID_DATE');

  // supplier: payable makes the balance negative (we owe them)
  addPayable(db, {
    contact: t.contacts.supplier.id, invoiceRef: 'F-9', date: '2026-07-05',
    amountCents: 12345, actor: 'agent:test',
  });
  const s = contactStatement(db, { contactId: t.contacts.supplier.id, asOf: '2026-08-08' });
  assert.equal(s.rows[0].kind, 'payable');
  assert.equal(s.balance_cents, -12345);
});

test('contact statement: credit notes reduce the balance (regression)', () => {
  const inv = makeFinalized(t.contacts.acme.id, { date: '2026-07-01', lines: ['Ding @ 100.00'] });
  let st = contactStatement(db, { contactId: t.contacts.acme.id, asOf: '2026-08-08' });
  assert.equal(st.balance_cents, 10000); // they owe us 100.00

  const credit = creditInvoice(db, { id: inv.invoice.id, date: '2026-07-15', actor: 'agent:test' });
  finalizeInvoice(db, { id: credit.id, actor: 'agent:test' });

  st = contactStatement(db, { contactId: t.contacts.acme.id, asOf: '2026-08-08' });
  const creditRow = st.rows.find((r) => r.kind === 'credit');
  assert.ok(creditRow, 'credit note must appear on the opgave');
  assert.equal(creditRow.credit_cents, 10000);
  assert.equal(st.balance_cents, 0); // 100.00 invoice − 100.00 creditnote
});

test('sales by contact: net/vat/gross from the totals engine; credit notes excluded', () => {
  makeFinalized(t.contacts.acme.id, { date: '2026-01-10', lines: ['Ding @ 100.00'] }); // 0% vat-off → net 100.00
  makeFinalized(t.contacts.acme.id, { date: '2026-03-15', lines: ['Ding @ 50.00'], dueDays: 30 });
  // a draft and a void invoice must not count
  const draft = createInvoice(db, { contactId: t.contacts.acme.id, lines: ['Ding @ 999.00'], date: '2026-02-01', actor: 'agent:test' });
  assert.equal(draft.status, 'draft');
  // outside the year
  makeFinalized(t.contacts.beta.id, { date: '2025-12-31', dueDays: 30 });

  const r = sales(db, { year: '2026', by: 'contact' });
  assert.equal(r.groups.length, 1);
  const g = r.groups[0];
  assert.equal(g.contact_id, t.contacts.acme.id);
  assert.equal(g.invoice_count, 2);
  assert.equal(g.net_cents, 15000);
  assert.equal(g.gross_cents, 15000);
  assert.equal(r.totals.net_cents, 15000);

  assert.throws(() => sales(db, { year: 'abc', by: 'contact' }), (e) => e.code === 'INVALID_YEAR');
  assert.throws(() => sales(db, { year: '2026', by: 'bogus' }), (e) => e.code === 'INVALID_KIND');
});

test('sales by item: catalog items group by item_id, ad-hoc lines by description', () => {
  const item = createItem(db, { name: 'Coaching uur', unit: 'h', unitPriceCents: 8000, vatCode: null, actor: 'agent:test' });
  makeFinalized(t.contacts.acme.id, { date: '2026-02-01', lines: [`1x Coaching uur @ 80.00`, 'Materiaal @ 20.00'] });
  makeFinalized(t.contacts.beta.id, { date: '2026-02-02', lines: [`1x Coaching uur @ 80.00`] });

  const r = sales(db, { year: '2026', by: 'item' });
  const groups = new Map(r.groups.map((g) => [g.name, g]));
  assert.ok(groups.has('Coaching uur'));
  assert.equal(groups.get('Coaching uur').line_count, 2);
  assert.equal(groups.get('Coaching uur').net_cents, 16000);
  assert.ok(groups.has('Materiaal'));
  assert.equal(groups.get('Materiaal').net_cents, 2000);
  assert.equal(r.totals.line_count, 3);
  assert.equal(r.totals.net_cents, 18000);
});

test('cli: report aging + sales + contact statement e2e with csv export', () => {
  const inv = makeFinalized(t.contacts.acme.id, { date: '2026-07-01', dueDays: 30 });
  assert.ok(inv.invoice.invoice_number);

  const agingCli = cli(t.file, ['report', 'aging', '--as-of', '2026-08-08', '--kind', 'debtors']);
  assert.equal(agingCli.out.data.debtors.totals.total_cents, 10000);

  const salesCli = cli(t.file, ['report', 'sales', '--year', '2026']);
  assert.equal(salesCli.out.data.totals.invoice_count, 1);

  const stmtCli = cli(t.file, ['contact', 'statement', '--id', String(t.contacts.acme.id)]);
  assert.equal(stmtCli.out.data.rows.length, 1);
  assert.equal(stmtCli.out.data.balance_cents, 10000);

  const csvPath = path.join(t.dir, 'aging.csv');
  // csv --out prints a human line (not JSON) — same pattern as the journal csv tests
  execFileSync(process.execPath, [BIN, '--json', 'report', 'aging', '--format', 'csv', '--out', csvPath], {
    env: { ...process.env, BUKIO_DB: t.file, BUKIO_ACTOR: 'agent:test' }, encoding: 'utf8',
  });
  const csv = readFileSync(csvPath, 'utf8');
  assert.match(csv, /debtors/);
  assert.ok(!csv.includes('=HYPERLINK'), 'formula-injection guard');
});

test('mcp: report_aging and report_sales expose the same shapes', async () => {
  makeFinalized(t.contacts.acme.id, { date: '2026-07-01', dueDays: 30 });
  const mcp = mcpSession(t.file);
  try {
    await mcp.call('initialize', { protocolVersion: '2024-11-05', capabilities: {}, clientInfo: { name: 't', version: '1' } });
    const agingRes = await mcp.call('tools/call', { name: 'report_aging', arguments: { as_of: '2026-08-08', kind: 'debtors' } });
    const agingData = JSON.parse(agingRes.result.content[0].text);
    assert.equal(agingData.debtors.totals.total_cents, 10000);

    const salesRes = await mcp.call('tools/call', { name: 'report_sales', arguments: { year: '2026' } });
    const salesData = JSON.parse(salesRes.result.content[0].text);
    assert.equal(salesData.totals.invoice_count, 1);

    const badYear = await mcp.call('tools/call', { name: 'report_sales', arguments: { year: 'abc' } });
    assert.equal(badYear.result.isError, true);
  } finally {
    mcp.close();
  }
});
