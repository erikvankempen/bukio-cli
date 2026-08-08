/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync, spawn } from 'node:child_process';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { openDb } from '../src/core/db.js';
import { createContact } from '../src/invoice/index.js';
import { importUblInvoice } from '../src/import/ubl-invoice.js';

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

/** Minimal-but-valid EN 16931 UBL invoice fixture. */
function ublInvoice({
  id = 'F2026-123', typeCode = '380', issueDate = '2026-08-01', dueDate = '2026-08-31',
  supplierName = 'Acme BV', vatId = 'NL123456789B01', payable = '121.00',
  taxExclusive = '100.00', taxAmount = '21.00', percent = '21', currency = 'EUR',
} = {}) {
  return `<?xml version="1.0" encoding="UTF-8"?>
<Invoice xmlns="urn:oasis:names:specification:ubl:schema:xsd:Invoice-2"
         xmlns:cac="urn:oasis:names:specification:ubl:schema:xsd:CommonAggregateComponents-2"
         xmlns:cbc="urn:oasis:names:specification:ubl:schema:xsd:CommonBasicComponents-2">
  <cbc:CustomizationID>urn:cen.eu:en16931:2017</cbc:CustomizationID>
  <cbc:ID>${id}</cbc:ID>
  <cbc:IssueDate>${issueDate}</cbc:IssueDate>
  ${dueDate ? `<cbc:DueDate>${dueDate}</cbc:DueDate>` : ''}
  <cbc:InvoiceTypeCode>${typeCode}</cbc:InvoiceTypeCode>
  <cbc:DocumentCurrencyCode>${currency}</cbc:DocumentCurrencyCode>
  <cac:AccountingSupplierParty>
    <cac:Party>
      <cac:PartyName><cbc:Name>${supplierName}</cbc:Name></cac:PartyName>
      <cac:PostalAddress>
        <cbc:StreetName>Leverstraat 3</cbc:StreetName>
        <cbc:CityName>Rotterdam</cbc:CityName>
        <cbc:PostalZone>3000 AA</cbc:PostalZone>
      </cac:PostalAddress>
      <cac:PartyTaxScheme>
        <cbc:CompanyID schemeID="VAT">${vatId}</cbc:CompanyID>
        <cac:TaxScheme><cbc:ID>VAT</cbc:ID></cac:TaxScheme>
      </cac:PartyTaxScheme>
      <cac:Contact><cbc:ElectronicMail>billing@acme.example</cbc:ElectronicMail></cac:Contact>
    </cac:Party>
  </cac:AccountingSupplierParty>
  <cac:AccountingCustomerParty>
    <cac:Party><cac:PartyName><cbc:Name>Test Coaching</cbc:Name></cac:PartyName></cac:Party>
  </cac:AccountingCustomerParty>
  <cac:TaxTotal>
    <cbc:TaxAmount currencyID="EUR">${taxAmount}</cbc:TaxAmount>
    <cac:TaxSubtotal>
      <cbc:TaxableAmount currencyID="EUR">${taxExclusive}</cbc:TaxableAmount>
      <cbc:TaxAmount currencyID="EUR">${taxAmount}</cbc:TaxAmount>
      <cac:TaxCategory><cbc:ID>S</cbc:ID><cbc:Percent>${percent}</cbc:Percent></cac:TaxCategory>
    </cac:TaxSubtotal>
  </cac:TaxTotal>
  <cac:LegalMonetaryTotal>
    <cbc:LineExtensionAmount currencyID="EUR">${taxExclusive}</cbc:LineExtensionAmount>
    <cbc:TaxExclusiveAmount currencyID="EUR">${taxExclusive}</cbc:TaxExclusiveAmount>
    <cbc:TaxInclusiveAmount currencyID="EUR">${payable}</cbc:TaxInclusiveAmount>
    <cbc:PayableAmount currencyID="EUR">${payable}</cbc:PayableAmount>
  </cac:LegalMonetaryTotal>
  <cac:InvoiceLine>
    <cbc:ID>1</cbc:ID>
    <cbc:InvoicedQuantity unitCode="HUR">2.0</cbc:InvoicedQuantity>
    <cbc:LineExtensionAmount currencyID="EUR">${taxExclusive}</cbc:LineExtensionAmount>
    <cac:Item><cbc:Name>Consultancy</cbc:Name></cac:Item>
    <cac:Price><cbc:PriceAmount currencyID="EUR">50.00</cbc:PriceAmount></cac:Price>
  </cac:InvoiceLine>
</Invoice>`;
}

let t;
let db;
let file;

test.beforeEach(() => {
  const dir = mkdtempSync(path.join(os.tmpdir(), 'bukio-ubl-test-'));
  file = path.join(dir, 'test.db');
  cli(file, ['init', '--name', 'Test Coaching', '--kvk', '12345678', '--legal-form', 'eenmanszaak', '--vat', 'off']);
  db = openDb(file);
  t = dir;
});
test.afterEach(() => {
  db.close();
  rmSync(t, { recursive: true, force: true });
});

function payables() {
  return db.prepare('SELECT * FROM payables ORDER BY id').all();
}

test('importUblInvoice: registers a payable, matches contact by vat-id, parses VAT', () => {
  const existing = createContact(db, {
    name: 'Acme BV', address: 'Leverstraat 3', city: 'Rotterdam',
    vatId: 'NL123456789B01', actor: 'agent:test',
  });
  const r = importUblInvoice(db, { xmlText: ublInvoice(), actor: 'agent:test' });
  assert.equal(r.imported, 1);
  assert.equal(r.duplicates, 0);
  assert.equal(r.amount_cents, 12100);
  assert.equal(r.due_date, '2026-08-31');
  assert.equal(r.contact.id, existing.id);
  assert.deepEqual(r.vat_by_rate, { 21: 2100 });

  const rows = payables();
  assert.equal(rows.length, 1);
  assert.equal(rows[0].invoice_ref, 'F2026-123');
  assert.equal(rows[0].amount_cents, 12100);
  assert.equal(rows[0].contact_id, existing.id);
  assert.equal(rows[0].source, 'ubl');
  assert.equal(rows[0].source_ref, 'nl123456789b01:F2026-123');
  assert.equal(rows[0].payment_method, 'transfer');

  const audit = db.prepare("SELECT * FROM audit_log WHERE action = 'import.invoice'").all();
  assert.equal(audit.length, 1);
  assert.equal(audit[0].actor, 'agent:test');
  assert.equal(JSON.parse(audit[0].args_json).payable_id, rows[0].id);
});

test('importUblInvoice: idempotent re-import → duplicate skipped', () => {
  createContact(db, { name: 'Acme BV', vatId: 'NL123456789B01', actor: 'agent:test' });
  importUblInvoice(db, { xmlText: ublInvoice(), actor: 'agent:test' });
  const r2 = importUblInvoice(db, { xmlText: ublInvoice(), actor: 'agent:test' });
  assert.equal(r2.imported, 0);
  assert.equal(r2.duplicates, 1);
  assert.equal(payables().length, 1);
});

test('importUblInvoice: --create-missing creates the supplier contact with address + vat-id', () => {
  const r = importUblInvoice(db, { xmlText: ublInvoice(), createMissing: true, actor: 'agent:test' });
  assert.equal(r.imported, 1);
  assert.equal(r.contacts_created, 1);
  const contact = db.prepare('SELECT * FROM contacts WHERE id = ?').get(r.contact.id);
  assert.equal(contact.name, 'Acme BV');
  assert.equal(contact.vat_id, 'NL123456789B01');
  assert.equal(contact.city, 'Rotterdam');
  assert.equal(contact.email, 'billing@acme.example');
  // idempotent on re-import via the NEW contact's key
  const r2 = importUblInvoice(db, { xmlText: ublInvoice(), createMissing: true, actor: 'agent:test' });
  assert.equal(r2.duplicates, 1);
  assert.equal(r2.contacts_created, 0);
});

test('importUblInvoice: TaxScheme/cbc:ID is the literal scheme id, not the VAT number', () => {
  // Real-world UBL: the number lives in PartyTaxScheme/cbc:CompanyID;
  // TaxScheme/cbc:ID is always the string 'VAT'. A supplier block with
  // only the scheme id (no CompanyID) must NOT store 'VAT' as vat_id —
  // that would collapse the idempotency key and vat-id contact matching
  // across every vendor that carries a PartyTaxScheme.
  const noNumber = ublInvoice().replace(
    /<cac:PartyTaxScheme>[\s\S]*?<\/cac:PartyTaxScheme>/,
    '<cac:PartyTaxScheme><cac:TaxScheme><cbc:ID>VAT</cbc:ID></cac:TaxScheme></cac:PartyTaxScheme>',
  );
  const r = importUblInvoice(db, { xmlText: noNumber, createMissing: true, actor: 'agent:test' });
  assert.equal(r.imported, 1);
  const contact = db.prepare('SELECT * FROM contacts WHERE id = ?').get(r.contact.id);
  assert.equal(contact.name, 'Acme BV');
  assert.equal(contact.vat_id, null);
  // idempotency key falls back to the normalized name — not 'vat:...'
  assert.equal(payables()[0].source_ref, 'acmebv:F2026-123');
});

test('importUblInvoice: explicit --contact wins; no match and no flag → CONTACT_NOT_FOUND', () => {
  const other = createContact(db, { name: 'Iets Anders BV', actor: 'agent:test' });
  const r = importUblInvoice(db, { xmlText: ublInvoice(), contact: other.id, actor: 'agent:test' });
  assert.equal(r.contact.id, other.id);
  assert.equal(r.contact.name, 'Iets Anders BV');

  assert.throws(
    () => importUblInvoice(db, { xmlText: ublInvoice({ id: 'F2026-124' }), actor: 'agent:test' }),
    (e) => e.code === 'CONTACT_NOT_FOUND',
  );
  assert.throws(
    () => importUblInvoice(db, { xmlText: ublInvoice(), contact: 999999, actor: 'agent:test' }),
    (e) => e.code === 'CONTACT_NOT_FOUND',
  );
});

test('importUblInvoice: validation failures write nothing', () => {
  const cases = [
    ['not xml', 'hello world', 'INVALID_UBL_INVOICE'],
    ['wrong root', '<AuditFile><Xaf/></AuditFile>', 'INVALID_UBL_INVOICE'],
    ['missing amount', ublInvoice().replace('<cbc:PayableAmount currencyID="EUR">121.00</cbc:PayableAmount>', ''), 'IMPORT_VALIDATION_FAILED'],
    ['bad date', ublInvoice({ issueDate: '2026-02-30' }), 'IMPORT_VALIDATION_FAILED'],
    ['negative amount', ublInvoice({ payable: '-5.00' }), 'IMPORT_VALIDATION_FAILED'],
    ['credit note', ublInvoice({ typeCode: '381' }), 'UNSUPPORTED_UBL_DOCUMENT'],
    ['non-EUR currency', ublInvoice({ currency: 'USD' }), 'IMPORT_VALIDATION_FAILED'],
  ];
  for (const [label, xml, code] of cases) {
    assert.throws(() => importUblInvoice(db, { xmlText: xml, createMissing: true, actor: 'agent:test' }), (e) => {
      assert.equal(e.code, code, label);
      return true;
    }, label);
  }
  assert.equal(payables().length, 0);
  assert.equal(db.prepare('SELECT COUNT(*) c FROM contacts').get().c, 0);
});

test('importUblInvoice: due date defaults to issue + 30 days', () => {
  createContact(db, { name: 'Acme BV', vatId: 'NL123456789B01', actor: 'agent:test' });
  const r = importUblInvoice(db, { xmlText: ublInvoice({ dueDate: null }), actor: 'agent:test' });
  assert.equal(r.due_date, '2026-08-31');
});

test('importUblInvoice: dry-run validates like execute but writes nothing', () => {
  const r = importUblInvoice(db, { xmlText: ublInvoice(), createMissing: true, actor: 'agent:test', dryRun: true });
  assert.equal(r.dryRun, true);
  assert.equal(r.action, 'import.invoice');
  assert.equal(r.contact.created, true);
  assert.equal(payables().length, 0);
  assert.equal(db.prepare('SELECT COUNT(*) c FROM contacts').get().c, 0);
  assert.equal(db.prepare("SELECT COUNT(*) c FROM audit_log WHERE action = 'import.invoice'").get().c, 0);

  // garbage still fails in dry-run
  assert.throws(() => importUblInvoice(db, { xmlText: 'garbage', createMissing: true, dryRun: true }), (e) => e.code === 'INVALID_UBL_INVOICE');
});

test('cli: import invoice end-to-end → payable in the register, dry-run plan', () => {
  const ublPath = path.join(t, 'invoice.xml');
  writeFileSync(ublPath, ublInvoice());

  const plan = cli(file, ['import', 'invoice', '--file', ublPath, '--create-missing', '--dry-run']);
  assert.equal(plan.out.data.dryRun, true);
  assert.equal(plan.out.data.contact.created, true);
  assert.equal(payables().length, 0);

  const r = cli(file, ['import', 'invoice', '--file', ublPath, '--create-missing']);
  assert.equal(r.code, 0);
  assert.equal(r.out.data.imported, 1);

  const listed = cli(file, ['payments', 'payables', 'list']);
  assert.equal(listed.out.data.payables.length, 1);
  assert.equal(listed.out.data.payables[0].invoice_ref, 'F2026-123');

  // re-import → duplicate
  const r2 = cli(file, ['import', 'invoice', '--file', ublPath, '--create-missing']);
  assert.equal(r2.out.data.duplicates, 1);

  // missing file
  const bad = cli(file, ['import', 'invoice', '--file', path.join(t, 'nope.xml')], { expectFail: true });
  assert.equal(bad.out.error.code, 'FILE_NOT_FOUND');
});

test('mcp: invoice_import dry-run parity + execute', async () => {
  const ublPath = path.join(t, 'invoice.xml');
  writeFileSync(ublPath, ublInvoice());
  const mcp = mcpSession(file);
  try {
    await mcp.call('initialize', { protocolVersion: '2024-11-05', capabilities: {}, clientInfo: { name: 't', version: '1' } });

    const missing = await mcp.call('tools/call', { name: 'invoice_import', arguments: { file_path: '/nope/nope.xml' } });
    assert.equal(missing.result.isError, true);

    const plan = await mcp.call('tools/call', { name: 'invoice_import', arguments: { file_path: ublPath, create_missing: true } });
    assert.equal(plan.result.isError, false);
    const planData = JSON.parse(plan.result.content[0].text);
    assert.equal(planData.mode, 'dry-run');
    assert.equal(payables().length, 0);

    const exec = await mcp.call('tools/call', { name: 'invoice_import', arguments: { file_path: ublPath, create_missing: true, mode: 'execute' } });
    assert.equal(exec.result.isError, false);
    const execData = JSON.parse(exec.result.content[0].text);
    assert.equal(execData.mode, 'execute');
    assert.equal(execData.imported, 1);
    assert.equal(payables().length, 1);
  } finally {
    mcp.close();
  }
});

test('importUblInvoice: multiple PartyTaxScheme entries — VAT number still extracted', () => {
  // A supplier with BOTH a local tax number and a USt-IdNr (two
  // cac:PartyTaxScheme siblings, per UBL 0..n) — fast-xml-parser yields an
  // ARRAY; the vat-id extraction must pick the 'VAT' scheme and not drop it.
  const existing = createContact(db, {
    name: 'Acme BV', address: 'Leverstraat 3', city: 'Rotterdam',
    vatId: 'NL123456789B01', actor: 'agent:test',
  });
  const xml = ublInvoice().replace(
    '</cac:PartyTaxScheme>',
    `</cac:PartyTaxScheme>
      <cac:PartyTaxScheme>
        <cbc:CompanyID schemeID="TIN">DE123456789</cbc:CompanyID>
        <cac:TaxScheme><cbc:ID>TIN</cbc:ID></cac:TaxScheme>
      </cac:PartyTaxScheme>`,
  );
  const r = importUblInvoice(db, { xmlText: xml, actor: 'agent:test' });
  assert.equal(r.imported, 1);
  assert.equal(r.contact.id, existing.id); // matched by vat-id, not name fallback
  const rows = payables();
  assert.equal(rows[0].source_ref, 'nl123456789b01:F2026-123'); // vat-id key preserved
  assert.equal(rows[0].contact_id, existing.id);
});
