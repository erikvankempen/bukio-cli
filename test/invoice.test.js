import { test, beforeEach } from 'node:test';
import assert from 'node:assert/strict';
import { XMLParser } from 'fast-xml-parser';
import { openDb } from '../src/core/db.js';
import { seedDefaultChart } from '../src/core/accounts.js';
import { getEntry } from '../src/core/entries.js';
import { enableVatModule } from '../src/vat/index.js';
import {
  buildInvoicePostings, createContact, createInvoice, creditInvoice,
  finalizeInvoice, getInvoice, markPaid, nextInvoiceNumber, parseLineSpec,
  validateCompliance,
} from '../src/invoice/index.js';
import { invoiceToUbl } from '../src/invoice/ubl.js';
import { importTransactions, autoMatch } from '../src/bank/index.js';
import { parseCamt053 } from '../src/bank/camt.js';

let db;

function setupCompany({ vat = true, complete = true } = {}) {
  db = openDb(':memory:');
  seedDefaultChart(db);
  db.prepare(`
    INSERT INTO company (name, kvk, legal_form, btw_id, iban, address, postal_code, city, vat_module)
    VALUES ('Demo BV', '12345678', 'bv', 'NL123456789B01', 'NL91ABNA0417164300', ?, '2712 CD', 'Zoetermeer', ?)
  `).run(complete ? 'Industrieweg 12' : null, vat ? 1 : 0);
  if (vat) enableVatModule(db);
}

beforeEach(() => {
  setupCompany();
});

function addContact(vatId = null) {
  return createContact(db, {
    name: 'ACME B.V.', address: 'Straat 1', postalCode: '1000 AA', city: 'Amsterdam',
    vatId, actor: 'agent:test',
  });
}

function mkInvoice(overrides = {}) {
  return createInvoice(db, {
    contactId: 1, date: '2026-07-10', lines: ['2x Consultancy @ 150.00 @21'],
    ...overrides,
  });
}

test('parseLineSpec: qty, description, price, vat', () => {
  assert.deepEqual(parseLineSpec('2x Consultancy @ 150.00 @21'), {
    qty: 2, description: 'Consultancy', priceCents: 15000, vatCode: '21',
  });
  assert.deepEqual(parseLineSpec('Kantoorartikelen @ 45,50'), {
    qty: 1, description: 'Kantoorartikelen', priceCents: 4550, vatCode: null,
  });
  assert.throws(() => parseLineSpec('garbage'), { code: 'INVALID_LINE' });
});

test('createInvoice: draft with line math (2x 150 @21 = 300 net, 63 vat)', () => {
  const c = addContact();
  const inv = createInvoice(db, {
    contactId: c.id, date: '2026-07-10', lines: ['2x Consultancy @ 150.00 @21'],
  });
  assert.equal(inv.status, 'draft');
  assert.equal(inv.invoice_number, null);
  assert.equal(inv.lines.length, 1);
  assert.equal(inv.lines[0].amount_cents, 30000);
  assert.equal(inv.lines[0].vat_amount_cents, 6300);
  assert.equal(inv.net_cents, 30000);
  assert.equal(inv.vat_cents, 6300);
  assert.equal(inv.gross_cents, 36300);
  assert.equal(inv.due_date, '2026-08-09');
});

test('createInvoice: guards', () => {
  assert.throws(() => createInvoice(db, { contactId: 99, date: '2026-07-10', lines: ['x @ 1 @21'] }), { code: 'CONTACT_NOT_FOUND' });
  addContact();
  assert.throws(() => createInvoice(db, { contactId: 1, date: '2026-07-10', lines: [] }), { code: 'NO_LINES' });
  assert.throws(() => createInvoice(db, { contactId: 1, date: 'bad', lines: ['x @ 1'] }), { code: 'INVALID_DATE' });
  // vat code without module
  setupCompany({ vat: false });
  addContact();
  assert.throws(() => mkInvoice(), { code: 'VAT_MODULE_OFF' });
  // unknown vat code (module on again)
  setupCompany({ vat: true });
  addContact();
  assert.throws(() => createInvoice(db, { contactId: 1, date: '2026-07-10', lines: ['x @ 1 @99'] }), { code: 'VAT_CODE_NOT_FOUND' });
});

test('validateCompliance: 12 vereisten — supplier and customer data required', () => {
  addContact();
  const inv = mkInvoice();
  // missing supplier address
  setupCompany({ complete: false });
  addContact();
  const inv2 = createInvoice(db, { contactId: 1, date: '2026-07-10', lines: ['x @ 1 @21'] });
  assert.throws(() => validateCompliance(db, inv2), { code: 'SUPPLIER_INCOMPLETE' });
  // missing customer address
  setupCompany();
  createContact(db, { name: 'Zonder Adres', actor: 'agent:test' });
  const inv3 = createInvoice(db, { contactId: 1, date: '2026-07-10', lines: ['x @ 1 @21'] });
  assert.throws(() => validateCompliance(db, inv3), { code: 'CUSTOMER_INCOMPLETE' });
  // verlegd -> customer vat id required
  setupCompany();
  addContact(null);
  const inv4 = createInvoice(db, { contactId: 1, date: '2026-07-10', lines: ['x @ 1 @R'] });
  assert.throws(() => validateCompliance(db, inv4), { code: 'CUSTOMER_VAT_REQUIRED' });
  // complete passes
  setupCompany();
  addContact();
  assert.equal(validateCompliance(db, mkInvoice()).ok, true);
});

test('finalize: assigns sequential number and books Debiteuren/Omzet/btw', () => {
  addContact();
  const inv = mkInvoice();
  const result = finalizeInvoice(db, { id: inv.id, actor: 'agent:test' });
  assert.equal(result.invoice.invoice_number, '2026-0001');
  assert.equal(result.invoice.status, 'sent');
  assert.equal(result.invoice.entry_id, result.entry.id);

  const entry = getEntry(db, result.entry.id);
  assert.equal(entry.source, 'invoice');
  assert.equal(entry.state, 'posted');
  const byCode = Object.fromEntries(entry.postings.map((p) => [p.account_code, p]));
  assert.equal(byCode['1200'].amount_cents, 36300); // debiteuren
  assert.equal(byCode['8000'].amount_cents, -30000);
  assert.equal(byCode['8000'].vat_amount_cents, -6300);
  assert.equal(byCode['2500'].amount_cents, -6300);

  // second invoice continues the sequence
  const inv2 = mkInvoice();
  finalizeInvoice(db, { id: inv2.id });
  assert.equal(getInvoice(db, inv2.id).invoice_number, '2026-0002');
});

test('finalize: multiple VAT rates -> per-rate postings, exact vat', () => {
  addContact();
  const inv = createInvoice(db, {
    contactId: 1, date: '2026-07-10',
    lines: ['1x Dienstverlening @ 100.00 @21', '2x Maaltijd @ 25.00 @9'],
  });
  const result = finalizeInvoice(db, { id: inv.id });
  const entry = getEntry(db, result.entry.id);
  const omzet = entry.postings.filter((p) => p.account_code === '8000');
  assert.equal(omzet.length, 2);
  const vat21 = omzet.find((p) => p.vat_amount_cents === -2100);
  const vat9 = omzet.find((p) => p.vat_amount_cents === -450); // 2x25=50 @9% = 4.50
  assert.ok(vat21 && vat9);
  // gross = 100 + 21 + 50 + 4.50 = 175.50
  assert.equal(entry.postings.find((p) => p.account_code === '1200').amount_cents, 17550);
});

test('finalize: VAT module off -> net-only booking, no vat postings', () => {
  setupCompany({ vat: false });
  addContact();
  const inv = createInvoice(db, { contactId: 1, date: '2026-07-10', lines: ['1x Dienst @ 100.00'] });
  const result = finalizeInvoice(db, { id: inv.id });
  const entry = getEntry(db, result.entry.id);
  assert.equal(entry.postings.length, 2);
  assert.equal(entry.postings.find((p) => p.account_code === '1200').amount_cents, 10000);
  assert.equal(entry.postings.find((p) => p.account_code === '8000').amount_cents, -10000);
});

test('finalize: already finalized is rejected; dry-run writes nothing', () => {
  addContact();
  const inv = mkInvoice();
  finalizeInvoice(db, { id: inv.id });
  assert.throws(() => finalizeInvoice(db, { id: inv.id }), { code: 'ALREADY_FINALIZED' });

  const inv2 = mkInvoice();
  const plan = finalizeInvoice(db, { id: inv2.id, dryRun: true });
  assert.equal(plan.dryRun, true);
  assert.equal(plan.invoice_number, '2026-0002');
  assert.equal(getInvoice(db, inv2.id).invoice_number, null); // nothing written
  assert.equal(db.prepare("SELECT COUNT(*) c FROM journal_entries WHERE source='invoice'").get().c, 1);
});

test('credit note: reversed booking, sequence continues', () => {
  addContact();
  const inv = mkInvoice();
  finalizeInvoice(db, { id: inv.id });

  const credit = creditInvoice(db, { id: inv.id, reason: 'verkeerde tarief' });
  assert.equal(credit.invoice_type, 'credit');
  assert.equal(credit.credit_for_invoice_id, inv.id);
  assert.equal(credit.reference, '2026-0001');
  assert.equal(credit.lines.length, 1);

  const result = finalizeInvoice(db, { id: credit.id });
  assert.equal(result.invoice.invoice_number, '2026-0002');
  const entry = getEntry(db, result.entry.id);
  const byCode = Object.fromEntries(entry.postings.map((p) => [p.account_code, p]));
  assert.equal(byCode['1200'].amount_cents, -36300); // debiteuren credit
  assert.equal(byCode['8000'].amount_cents, 30000); // omzet debit
  assert.equal(byCode['2500'].amount_cents, 6300); // vat debit
});

test('payments: partial then full -> paid; overpayment rejected', () => {
  addContact();
  const inv = mkInvoice();
  finalizeInvoice(db, { id: inv.id });

  const partial = markPaid(db, { id: inv.id, date: '2026-07-20', amountCents: 20000 });
  assert.equal(partial.status, 'sent');
  assert.equal(partial.paid_cents, 20000);

  const paid = markPaid(db, { id: inv.id, date: '2026-07-25', amountCents: 16300 });
  assert.equal(paid.status, 'paid');

  assert.throws(() => markPaid(db, { id: inv.id, date: '2026-07-26', amountCents: 100 }), { code: 'NOT_PAYABLE' });
  // overpayment
  const inv2 = mkInvoice();
  finalizeInvoice(db, { id: inv2.id });
  assert.throws(() => markPaid(db, { id: inv2.id, date: '2026-07-20', amountCents: 40000 }), { code: 'OVERPAYMENT' });
});

test('nextInvoiceNumber: year-scoped sequence', () => {
  addContact();
  assert.equal(nextInvoiceNumber(db, 2026), '2026-0001');
  const inv = mkInvoice();
  finalizeInvoice(db, { id: inv.id });
  assert.equal(nextInvoiceNumber(db, 2026), '2026-0002');
  assert.equal(nextInvoiceNumber(db, 2027), '2027-0001');
});

test('UBL: Peppol BIS 3.0 structure', () => {
  addContact('NL999999999B01');
  const inv = mkInvoice();
  finalizeInvoice(db, { id: inv.id });
  const xml = invoiceToUbl(db, getInvoice(db, inv.id));
  const parser = new XMLParser({ ignoreAttributes: false, removeNSPrefix: true, parseTagValue: false });
  const doc = parser.parse(xml);
  const root = doc['Invoice'];
  assert.equal(root['CustomizationID'], 'urn:cen.eu:en16931:2017#compliant#urn:fdc:peppol.eu:2017:poacc:billing:3.0');
  assert.equal(root['ID'], '2026-0001');
  assert.equal(root['InvoiceTypeCode'], '380');
  assert.equal(root['IssueDate'], '2026-07-10');
  assert.equal(root['AccountingSupplierParty']['Party']['PartyName']['Name'], 'Demo BV');
  assert.equal(root['AccountingCustomerParty']['Party']['PartyName']['Name'], 'ACME B.V.');
  // CompanyID carries a schemeID attribute -> object form in fast-xml-parser
  const buyerVat = root['AccountingCustomerParty']['Party']['PartyTaxScheme']['CompanyID'];
  assert.equal(buyerVat['#text'], 'NL999999999B01');
  assert.equal(buyerVat['@_schemeID'], 'VAT');
  assert.equal(root['TaxTotal']['TaxAmount']['#text'], '63.00');
  assert.equal(root['LegalMonetaryTotal']['PayableAmount']['#text'], '363.00');
  const line = root['InvoiceLine'];
  assert.equal(line['InvoicedQuantity']['#text'], '2');
  assert.equal(line['InvoicedQuantity']['@_unitCode'], 'C62');
  assert.equal(line['Item']['ClassifiedTaxCategory']['ID'], 'S');
  assert.equal(line['Item']['ClassifiedTaxCategory']['Percent'], '21.00');
});

test('UBL: credit note uses type 381', () => {
  addContact();
  const inv = mkInvoice();
  finalizeInvoice(db, { id: inv.id });
  const credit = creditInvoice(db, { id: inv.id });
  finalizeInvoice(db, { id: credit.id });
  const xml = invoiceToUbl(db, getInvoice(db, credit.id));
  const parser = new XMLParser({ ignoreAttributes: false, removeNSPrefix: true, parseTagValue: false });
  const doc = parser.parse(xml);
  assert.equal(doc['Invoice']['InvoiceTypeCode'], '381');
  assert.equal(doc['Invoice']['ID'], '2026-0002');
});

test('bank auto-match: incoming payment pays the invoice and posts Bank/Debiteuren', () => {
  addContact();
  const inv = mkInvoice();
  finalizeInvoice(db, { id: inv.id }); // gross 363.00

  const camt = `<?xml version="1.0"?>
<Document xmlns="urn:iso:std:iso:20022:tech:xsd:camt.053.001.02">
  <BkToCstmrStmt><Stmt><Acct><Id><IBAN>NL91ABNA0417164300</IBAN></Id></Acct>
    <Ntry><Amt>363.00</Amt><CdtDbtInd>CRDT</CdtDbtInd><BookgDt><Dt>2026-07-18</Dt></BookgDt>
      <NtryDtls><TxDtls><RltdPties><Dbtr><Nm>ACME B.V.</Nm></Dbtr></RltdPties>
      <RmtInf><Ustrd>2026-0001</Ustrd></RmtInf></TxDtls></NtryDtls></Ntry>
  </Stmt></BkToCstmrStmt>
</Document>`;
  importTransactions(db, { iban: 'NL91ABNA0417164300', transactions: parseCamt053(camt) });

  const dry = autoMatch(db, { dryRun: true });
  assert.equal(dry.matched.length, 1);
  assert.equal(dry.matched[0].kind, 'invoice');
  assert.equal(dry.matched[0].invoice_number, '2026-0001');
  assert.equal(getInvoice(db, inv.id).status, 'sent'); // dry run wrote nothing

  const real = autoMatch(db, { actor: 'agent:test' });
  assert.equal(real.matched.length, 1);
  assert.equal(real.matched[0].kind, 'invoice');
  const after = getInvoice(db, inv.id);
  assert.equal(after.status, 'paid');
  assert.equal(after.paid_cents, 36300);
  // payment entry posted
  const paymentEntry = db.prepare("SELECT * FROM journal_entries WHERE source='bank' ORDER BY id DESC").get();
  assert.equal(paymentEntry.state, 'posted');
  const entry = getEntry(db, paymentEntry.id);
  const byCode = Object.fromEntries(entry.postings.map((p) => [p.account_code, p]));
  assert.equal(byCode['1100'].amount_cents, 36300);
  assert.equal(byCode['1200'].amount_cents, -36300);
  // reconciled
  const rec = db.prepare("SELECT * FROM reconciliations WHERE target_type='invoice'").get();
  assert.equal(rec.target_id, inv.id);
  // trial balance still balanced
  const totals = db.prepare('SELECT COALESCE(SUM(amount_cents),0) s FROM postings').get().s;
  assert.equal(totals, 0);
});

test('buildInvoicePostings: sales vs credit sign flip', () => {
  addContact();
  const inv = mkInvoice();
  const sales = buildInvoicePostings(db, inv);
  assert.equal(sales.find((p) => p.code === '1200').amountCents, 36300);
  const credit = { ...inv, invoice_type: 'credit' };
  const reversed = buildInvoicePostings(db, credit);
  assert.equal(reversed.find((p) => p.code === '1200').amountCents, -36300);
});
