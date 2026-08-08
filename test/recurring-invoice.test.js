/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

import { test, beforeEach } from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { openDb } from '../src/core/db.js';
import { seedDefaultChart } from '../src/core/accounts.js';
import { enableVatModule } from '../src/vat/index.js';
import { createTemplate, runDue, getTemplate } from '../src/recurring/index.js';
import { createContact, createInvoice, finalizeInvoice, getInvoice } from '../src/invoice/index.js';
import { sendPeppolInvoice } from '../src/invoice/peppol.js';

let db;

beforeEach(() => {
  db = openDb(':memory:');
  seedDefaultChart(db);
  db.prepare(`
    INSERT INTO company (name, kvk, legal_form, btw_id, iban, address, postal_code, city, vat_module)
    VALUES ('Demo BV', '12345678', 'bv', 'NL123456789B01', 'NL91ABNA0417164300',
            'Industrieweg 12', '2712 CD', 'Zoetermeer', 1)
  `).run();
  enableVatModule(db);
});

function addContact() {
  return createContact(db, {
    name: 'ACME B.V.', address: 'Straat 1', postalCode: '1000 AA', city: 'Amsterdam',
    vatId: 'NL999999999B01', kvk: '98765432', actor: 'agent:test',
  });
}

test('invoice template: generates draft invoices on schedule (never auto-finalizes)', () => {
  const c = addContact();
  const tpl = createTemplate(db, {
    name: 'SaaS abonnement', kind: 'invoice', contactId: c.id,
    invoiceLines: ['2x Premium @ 99.00 @21'],
    frequency: 'monthly', startDate: '2026-08-01', dueDays: 14, actor: 'agent:test',
  });
  assert.equal(tpl.kind, 'invoice');
  assert.equal(tpl.vat_aware, 1);

  const result = runDue(db, { asOf: '2026-09-30', actor: 'agent:test' });
  assert.equal(result.templates.length, 1);
  assert.equal(result.templates[0].runs.length, 2); // Aug 1 + Sep 1 (backfill)

  const invoices = db.prepare('SELECT * FROM invoices ORDER BY id').all();
  assert.equal(invoices.length, 2);
  assert.equal(invoices[0].date, '2026-08-01');
  assert.equal(invoices[1].date, '2026-09-01');
  // drafts only — nothing booked, no numbers
  assert.equal(invoices.every((i) => i.status === 'draft'), true);
  assert.equal(invoices.every((i) => i.invoice_number === null), true);
  assert.equal(db.prepare("SELECT COUNT(*) c FROM journal_entries WHERE source='invoice'").get().c, 0);

  // lines + due date carried over
  const inv = getInvoice(db, invoices[0].id);
  assert.equal(inv.lines[0].description, 'Premium');
  assert.equal(inv.lines[0].quantity, 2000); // milli-units: 2.000 = 2
  assert.equal(inv.lines[0].vat_amount_cents, 4158); // 198.00 @21%
  assert.equal(inv.due_date, '2026-08-15');
  assert.equal(inv.contact.name, 'ACME B.V.');
});

test('invoice template: generated drafts finalize normally (compliance + number)', () => {
  const c = addContact();
  createTemplate(db, {
    name: 'SaaS abonnement', kind: 'invoice', contactId: c.id,
    invoiceLines: ['1x Premium @ 99.00 @21'],
    frequency: 'monthly', startDate: '2026-08-01', actor: 'agent:test',
  });
  runDue(db, { asOf: '2026-08-31' });
  const draft = db.prepare('SELECT * FROM invoices').get();
  const result = finalizeInvoice(db, { id: draft.id, actor: 'agent:test' });
  assert.equal(result.invoice.invoice_number, '2026-0001');
  assert.equal(result.invoice.status, 'sent');
  assert.equal(result.entry.state, 'posted');
});

test('invoice template: guards', () => {
  addContact();
  // reverse-previous is entry-only
  assert.throws(() => createTemplate(db, {
    name: 'x', kind: 'invoice', contactId: 1, invoiceLines: ['1x A @ 10.00 @21'],
    frequency: 'monthly', startDate: '2026-08-01', reversePrevious: true,
  }), { code: 'INVALID_REVERSE' });
  // contact required + must exist
  assert.throws(() => createTemplate(db, {
    name: 'x', kind: 'invoice', invoiceLines: ['1x A @ 10.00 @21'],
    frequency: 'monthly', startDate: '2026-08-01',
  }), { code: 'INVALID_KIND' });
  assert.throws(() => createTemplate(db, {
    name: 'x', kind: 'invoice', contactId: 99, invoiceLines: ['1x A @ 10.00 @21'],
    frequency: 'monthly', startDate: '2026-08-01',
  }), { code: 'CONTACT_NOT_FOUND' });
  // unknown vat code at creation
  assert.throws(() => createTemplate(db, {
    name: 'x', kind: 'invoice', contactId: 1, invoiceLines: ['1x A @ 10.00 @77'],
    frequency: 'monthly', startDate: '2026-08-01',
  }), { code: 'VAT_CODE_NOT_FOUND' });
  // bad kind
  assert.throws(() => createTemplate(db, {
    name: 'x', kind: 'ledger', postings: ['1100:1,3000:-1'],
    frequency: 'monthly', startDate: '2026-08-01',
  }), { code: 'INVALID_KIND' });
});

test('invoice template: entry templates keep working alongside', () => {
  const c = addContact();
  createTemplate(db, {
    name: 'SaaS abonnement', kind: 'invoice', contactId: c.id,
    invoiceLines: ['1x Premium @ 99.00 @21'],
    frequency: 'monthly', startDate: '2026-08-01', actor: 'agent:test',
  });
  createTemplate(db, {
    name: 'Huur kantoor', postings: ['4300:1000.00,1100:-1000.00'],
    frequency: 'monthly', startDate: '2026-08-01', actor: 'agent:test',
  });
  const result = runDue(db, { asOf: '2026-08-31' });
  assert.equal(result.templates.length, 2);
  const invoiceTpl = result.templates.find((t) => t.name === 'SaaS abonnement');
  const entryTpl = result.templates.find((t) => t.name === 'Huur kantoor');
  assert.equal(invoiceTpl.runs[0].generated[0].kind, 'invoice');
  assert.equal(entryTpl.runs[0].generated[0].kind, 'entry');
  assert.equal(db.prepare('SELECT COUNT(*) c FROM invoices').get().c, 1);
  assert.equal(db.prepare("SELECT COUNT(*) c FROM journal_entries WHERE source='recurring'").get().c, 1);
});

test('invoice template: dry-run shows the invoice plan, writes nothing', () => {
  const c = addContact();
  createTemplate(db, {
    name: 'SaaS abonnement', kind: 'invoice', contactId: c.id,
    invoiceLines: ['1x Premium @ 99.00 @21'],
    frequency: 'monthly', startDate: '2026-08-01', dueDays: 14, actor: 'agent:test',
  });
  const result = runDue(db, { asOf: '2026-08-31', dryRun: true });
  const run = result.templates[0].runs[0];
  assert.equal(run.kind, 'invoice');
  assert.equal(run.invoice.date, '2026-08-01');
  assert.equal(run.invoice.due_date, '2026-08-15');
  assert.equal(run.invoice.contact_name, 'ACME B.V.');
  assert.equal(db.prepare('SELECT COUNT(*) c FROM invoices').get().c, 0);
});

test('invoice template: runs limit completes the template', () => {
  addContact();
  createTemplate(db, {
    name: 'Abonnement Q', kind: 'invoice', contactId: 1,
    invoiceLines: ['1x Premium @ 99.00 @21'],
    frequency: 'quarterly', startDate: '2026-07-01', runs: 2, actor: 'agent:test',
  });
  runDue(db, { asOf: '2026-12-31' });
  assert.equal(db.prepare('SELECT COUNT(*) c FROM invoices').get().c, 2);
  const tpl = getTemplate(db, 1);
  assert.equal(tpl.status, 'completed');
  assert.equal(tpl.runs_done, 2);
});

test('peppol send: posts the UBL to the provider (mock server)', async () => {
  const c = addContact();
  const inv = createInvoice(db, {
    contactId: c.id, date: '2026-08-01', lines: ['1x Premium @ 99.00 @21'],
  });
  finalizeInvoice(db, { id: inv.id });

  let captured = null;
  const server = createServer((req, res) => {
    let body = '';
    req.on('data', (d) => { body += d; });
    req.on('end', () => {
      captured = { method: req.method, auth: req.headers.authorization, contentType: req.headers['content-type'], body };
      res.writeHead(202, { 'Content-Type': 'application/json' });
      res.end(JSON.stringify({ ok: true }));
    });
  });
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  const { port } = server.address();

  const oldEndpoint = process.env.BUKIO_PEPPOL_ENDPOINT;
  const oldToken = process.env.BUKIO_PEPPOL_TOKEN;
  process.env.BUKIO_PEPPOL_ENDPOINT = `http://127.0.0.1:${port}/invoices`;
  process.env.BUKIO_PEPPOL_TOKEN = 'test-token-123';
  try {
    const result = await sendPeppolInvoice(db, getInvoice(db, inv.id));
    assert.equal(result.status, 202);
    assert.equal(result.invoice_number, '2026-0001');
    assert.ok(captured);
    assert.equal(captured.method, 'POST');
    assert.equal(captured.auth, 'Bearer test-token-123');
    assert.match(captured.contentType, /xml/);
    assert.match(captured.body, /<cbc:ID>2026-0001<\/cbc:ID>/);
  } finally {
    if (oldEndpoint === undefined) delete process.env.BUKIO_PEPPOL_ENDPOINT; else process.env.BUKIO_PEPPOL_ENDPOINT = oldEndpoint;
    if (oldToken === undefined) delete process.env.BUKIO_PEPPOL_TOKEN; else process.env.BUKIO_PEPPOL_TOKEN = oldToken;
    server.close();
  }
});

test('peppol send: not configured / provider error / dry-run', async () => {
  const c = addContact();
  const inv = createInvoice(db, { contactId: c.id, date: '2026-08-01', lines: ['1x A @ 10.00 @21'] });
  finalizeInvoice(db, { id: inv.id });

  const oldEndpoint = process.env.BUKIO_PEPPOL_ENDPOINT;
  const oldToken = process.env.BUKIO_PEPPOL_TOKEN;
  delete process.env.BUKIO_PEPPOL_ENDPOINT;
  delete process.env.BUKIO_PEPPOL_TOKEN;
  try {
    await assert.rejects(() => sendPeppolInvoice(db, getInvoice(db, inv.id)), { code: 'PEPPOL_NOT_CONFIGURED' });
  } finally {
    if (oldEndpoint === undefined) delete process.env.BUKIO_PEPPOL_ENDPOINT; else process.env.BUKIO_PEPPOL_ENDPOINT = oldEndpoint;
    if (oldToken === undefined) delete process.env.BUKIO_PEPPOL_TOKEN; else process.env.BUKIO_PEPPOL_TOKEN = oldToken;
  }

  // provider returns 500
  const server = createServer((req, res) => {
    req.resume();
    req.on('end', () => {
      res.writeHead(500, { 'Content-Type': 'text/plain' });
      res.end('provider exploded');
    });
  });
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  const { port } = server.address();
  process.env.BUKIO_PEPPOL_ENDPOINT = `http://127.0.0.1:${port}`;
  process.env.BUKIO_PEPPOL_TOKEN = 'tok';
  try {
    await assert.rejects(() => sendPeppolInvoice(db, getInvoice(db, inv.id)), { code: 'PEPPOL_SEND_FAILED' });
    // dry-run: no request, reports config
    const plan = await sendPeppolInvoice(db, getInvoice(db, inv.id), { dryRun: true });
    assert.equal(plan.dryRun, true);
    assert.equal(plan.configured, true);
    assert.ok(plan.bytes > 0);
  } finally {
    delete process.env.BUKIO_PEPPOL_ENDPOINT;
    delete process.env.BUKIO_PEPPOL_TOKEN;
    server.close();
  }
});

test('peppol send: buyer without a KVK number is rejected up front (BT-49)', async () => {
  const c = createContact(db, {
    name: 'Geen KVK B.V.', address: 'Straat 1', postalCode: '1000 AA', city: 'Amsterdam',
    vatId: 'NL888888888B01', actor: 'agent:test', // no kvk
  });
  const inv = createInvoice(db, { contactId: c.id, date: '2026-08-01', lines: ['1x A @ 10.00 @21'] });
  finalizeInvoice(db, { id: inv.id });
  const oldEndpoint = process.env.BUKIO_PEPPOL_ENDPOINT;
  const oldToken = process.env.BUKIO_PEPPOL_TOKEN;
  process.env.BUKIO_PEPPOL_ENDPOINT = 'http://127.0.0.1:9';
  process.env.BUKIO_PEPPOL_TOKEN = 'tok';
  try {
    // fails BEFORE any network call — even dry-run validates like execute
    await assert.rejects(() => sendPeppolInvoice(db, getInvoice(db, inv.id)), { code: 'PEPPOL_BUYER_MISSING_ID' });
    await assert.rejects(() => sendPeppolInvoice(db, getInvoice(db, inv.id), { dryRun: true }), { code: 'PEPPOL_BUYER_MISSING_ID' });
  } finally {
    if (oldEndpoint === undefined) delete process.env.BUKIO_PEPPOL_ENDPOINT; else process.env.BUKIO_PEPPOL_ENDPOINT = oldEndpoint;
    if (oldToken === undefined) delete process.env.BUKIO_PEPPOL_TOKEN; else process.env.BUKIO_PEPPOL_TOKEN = oldToken;
  }
});
