/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Payment batches — SEPA credit transfer initiation (pain.001) files that the
// user uploads to the bank's portal. Bank-agnostic: one format, two schema
// versions (pain.001.001.03 legacy / pain.001.001.09 SCT2), accepted by all
// Dutch business portals (Rabobank, ING, ABN AMRO).
//
// Flow: register payables (purchase invoices; 'transfer' = batch-eligible,
// 'direct_debit' = incasso, excluded) -> create a batch from payables and/or
// explicit lines -> export pain.001 (unique MsgId, one export per batch —
// re-uploading the same file would double-pay) -> upload in the bank portal.
// The ledger is untouched: money moves when the bank processes; the bank
// statement import (CAMT.053) books it.
import { createHash } from 'node:crypto';
import { isValidIban, normalizeIban } from '../core/iban.js';
import { parseImportAmount } from '../import/index.js';
import { getContact, listContacts } from '../invoice/index.js';
import { record } from '../audit/index.js';

export function paymentsError(code, message) {
  return Object.assign(new Error(message), { code });
}

const CSV_HEADER_ALIASES = {
  contact: ['contact', 'naam', 'leverancier', 'name'],
  amount: ['amount', 'bedrag'],
  reference: ['reference', 'omschrijving', 'ref', 'description'],
};

function getCompany(db) {
  return db.prepare('SELECT * FROM company WHERE id = 1').get() ?? null;
}

const DATE_RE = /^\d{4}-\d{2}-\d{2}$/;

function validDate(s) {
  if (!DATE_RE.test(s)) return false;
  const d = new Date(`${s}T00:00:00Z`);
  return !Number.isNaN(d.getTime()) && d.toISOString().slice(0, 10) === s;
}

/** Resolve a contact by id (number) or case-insensitive name. */
export function resolveContact(db, ref) {
  const id = Number(ref);
  if (Number.isInteger(id)) {
    const c = getContact(db, id);
    if (c) return c;
  }
  const name = String(ref).trim().toLowerCase();
  return listContacts(db).find((c) => c.name.trim().toLowerCase() === name) ?? null;
}

function serializeBatch(db, row) {
  const lines = db.prepare('SELECT * FROM payment_batch_lines WHERE batch_id = ? ORDER BY id').all(row.id);
  return {
    ...row,
    total: (row.total_cents / 100).toFixed(2),
    lines: lines.map((l) => ({ ...l, amount: (l.amount_cents / 100).toFixed(2) })),
  };
}

export function getPaymentBatch(db, id) {
  const row = db.prepare('SELECT * FROM payment_batches WHERE id = ?').get(id);
  return row ? serializeBatch(db, row) : null;
}

// --- SEPA direct-debit mandates --------------------------------------------

export function addMandate(db, {
  contactId, mandateRef, mandateDate = null, scheme = 'core', actor = 'human', dryRun = false,
}) {
  if (!Number.isInteger(contactId) || contactId <= 0) throw paymentsError('CONTACT_NOT_FOUND', 'a contact id is required');
  const contact = getContact(db, contactId);
  if (!contact) throw paymentsError('CONTACT_NOT_FOUND', `contact ${contactId} does not exist`);
  const ref = String(mandateRef ?? '').trim();
  if (!ref) throw paymentsError('INVALID_MANDATE_REF', 'a mandate reference is required (max 35 chars)');
  if (ref.length > 35) throw paymentsError('INVALID_MANDATE_REF', 'mandate reference max 35 characters');
  const date = mandateDate ?? new Date().toISOString().slice(0, 10);
  if (!/^\d{4}-\d{2}-\d{2}$/.test(date)) throw paymentsError('INVALID_DATE', `mandate date '${date}' must be YYYY-MM-DD`);
  const d = new Date(`${date}T00:00:00Z`);
  if (Number.isNaN(d.getTime()) || d.toISOString().slice(0, 10) !== date) {
    throw paymentsError('INVALID_DATE', `mandate date '${date}' is not a valid calendar date`);
  }
  const sch = String(scheme ?? 'core').toLowerCase();
  if (!['core', 'b2b'].includes(sch)) throw paymentsError('INVALID_SCHEME', "mandate scheme must be 'core' or 'b2b'");
  const dup = db.prepare('SELECT id FROM sepa_mandates WHERE contact_id = ? AND mandate_ref = ?').get(contactId, ref);
  if (dup) throw paymentsError('MANDATE_DUPLICATE', `contact ${contact.name} already has mandate '${ref}' (id ${dup.id})`);
  const plan = { action: 'payments.mandate.add', contact_id: contactId, contact_name: contact.name, mandate_ref: ref, mandate_date: date, scheme: sch, dryRun };
  if (dryRun) return plan;
  const info = db.prepare(
    'INSERT INTO sepa_mandates (contact_id, mandate_ref, mandate_date, scheme, created_by) VALUES (?, ?, ?, ?, ?)',
  ).run(contactId, ref, date, sch, actor);
  record(db, { actor, action: 'payments.mandate.add', command: 'payments mandate add', args: { mandate_id: Number(info.lastInsertRowid), contact_id: contactId, mandate_ref: ref }, outcome: 'ok' });
  return { id: Number(info.lastInsertRowid), contact_id: contactId, contact_name: contact.name, mandate_ref: ref, mandate_date: date, scheme: sch };
}

export function listMandates(db, { contactId = null } = {}) {
  const rows = contactId
    ? db.prepare('SELECT m.*, c.name AS contact_name FROM sepa_mandates m JOIN contacts c ON c.id = m.contact_id WHERE m.contact_id = ? ORDER BY m.id').all(contactId)
    : db.prepare('SELECT m.*, c.name AS contact_name FROM sepa_mandates m JOIN contacts c ON c.id = m.contact_id ORDER BY m.id').all();
  return rows.map((r) => ({
    id: r.id, contact_id: r.contact_id, contact_name: r.contact_name,
    mandate_ref: r.mandate_ref, mandate_date: r.mandate_date, scheme: r.scheme,
  }));
}

export function removeMandate(db, { id, actor = 'human', dryRun = false }) {
  const mandate = db.prepare('SELECT * FROM sepa_mandates WHERE id = ?').get(id);
  if (!mandate) throw paymentsError('MANDATE_NOT_FOUND', `mandate ${id} does not exist`);
  const contact = getContact(db, mandate.contact_id);
  const plan = { action: 'payments.mandate.remove', mandate_id: id, contact_id: mandate.contact_id, contact_name: contact?.name ?? null, mandate_ref: mandate.mandate_ref, dryRun };
  if (dryRun) return plan;
  db.prepare('DELETE FROM sepa_mandates WHERE id = ?').run(id);
  record(db, { actor, action: 'payments.mandate.remove', command: 'payments mandate remove', args: { mandate_id: id, mandate_ref: mandate.mandate_ref }, outcome: 'ok' });
  return { mandate_id: id, status: 'deleted' };
}

/** Latest mandate for a contact (newest id wins), or null. */
function latestMandate(db, contactId) {
  return db.prepare('SELECT * FROM sepa_mandates WHERE contact_id = ? ORDER BY id DESC LIMIT 1').get(contactId);
}

/** FRST on a mandate's first direct-debit batch, RCUR afterwards. SEPA:
 *  FRST is per-mandate — a NEW mandate (after revocation) starts at FRST
 *  again, even for a contact that has older direct-debit history. */
function mandateSeqFor(db, contactId, mandateRef) {
  const used = db.prepare(
    "SELECT COUNT(*) c FROM payment_batch_lines l JOIN payment_batches b ON b.id = l.batch_id WHERE l.contact_id = ? AND l.mandate_ref = ? AND b.batch_kind = 'direct_debit'",
  ).get(contactId, mandateRef);
  return used.c > 0 ? 'RCUR' : 'FRST';
}

export function listPaymentBatches(db, { status = null } = {}) {
  const rows = status
    ? db.prepare('SELECT * FROM payment_batches WHERE status = ? ORDER BY id DESC').all(status)
    : db.prepare('SELECT * FROM payment_batches ORDER BY id DESC').all();
  return rows.map((r) => serializeBatch(db, r));
}

// --- payables (purchase invoices) --------------------------------------------

export function addPayable(db, {
  contact, invoiceRef, date, dueDate = null, amountCents,
  method = 'transfer', entryId = null, actor = 'human', dryRun = false,
}) {
  const c = resolveContact(db, contact);
  if (!c) throw paymentsError('CONTACT_NOT_FOUND', `contact '${contact}' does not exist`);
  if (!Number.isInteger(amountCents) || amountCents <= 0) {
    throw paymentsError('INVALID_AMOUNT', 'amount must be a positive amount in cents');
  }
  if (!invoiceRef || !String(invoiceRef).trim()) throw paymentsError('INVOICE_REF_REQUIRED', 'invoice reference is required');
  if (!['transfer', 'direct_debit'].includes(method)) throw paymentsError('INVALID_METHOD', "payment method must be 'transfer' or 'direct_debit'");
  if (!validDate(date)) throw paymentsError('INVALID_DATE', `date '${date}' must be yyyy-mm-dd`);
  if (dueDate != null && !validDate(dueDate)) throw paymentsError('INVALID_DATE', `due date '${dueDate}' must be yyyy-mm-dd`);
  const payable = { contact_id: c.id, invoice_ref: String(invoiceRef).trim(), date, due_date: dueDate, amount_cents: amountCents, payment_method: method, entry_id: entryId };
  if (dryRun) return { action: 'payables.add', ...payable, contact_name: c.name, dryRun: true };
  const info = db.prepare(
    'INSERT INTO payables (contact_id, invoice_ref, date, due_date, amount_cents, payment_method, entry_id, created_by) VALUES (?, ?, ?, ?, ?, ?, ?, ?)',
  ).run(c.id, payable.invoice_ref, date, dueDate, amountCents, method, entryId, actor);
  record(db, { actor, action: 'payables.add', command: 'payments payables add', args: { payable_id: info.lastInsertRowid, contact_id: c.id, invoice_ref: payable.invoice_ref, amount_cents: amountCents, method }, outcome: 'ok' });
  return { id: Number(info.lastInsertRowid), ...payable, contact_name: c.name };
}

export function listPayables(db, { status = null, method = null, contactId = null } = {}) {
  const clauses = [];
  const params = [];
  if (status) { clauses.push('p.status = ?'); params.push(status); }
  if (method) { clauses.push('p.payment_method = ?'); params.push(method); }
  if (contactId) { clauses.push('p.contact_id = ?'); params.push(contactId); }
  const where = clauses.length ? `WHERE ${clauses.join(' AND ')}` : '';
  return db.prepare(`
    SELECT p.*, c.name AS contact_name FROM payables p
    JOIN contacts c ON c.id = p.contact_id
    ${where} ORDER BY p.due_date IS NULL, p.due_date, p.id
  `).all(...params);
}

export function markPayablePaid(db, { id, actor = 'human', dryRun = false }) {
  const p = db.prepare('SELECT * FROM payables WHERE id = ?').get(id);
  if (!p) throw paymentsError('PAYABLE_NOT_FOUND', `payable ${id} does not exist`);
  if (p.status === 'paid') throw paymentsError('ALREADY_PAID', `payable ${id} is already paid`);
  if (dryRun) return { action: 'payables.pay', payable_id: id, status: p.status, dryRun: true };
  db.prepare("UPDATE payables SET status = 'paid' WHERE id = ?").run(id);
  record(db, { actor, action: 'payables.pay', command: 'payments payables pay', args: { payable_id: id }, outcome: 'ok' });
  return { payable_id: id, status: 'paid' };
}

// --- batch creation ----------------------------------------------------------

/**
 * Create a payment batch. lines: explicit [{contact?, name?, iban?, amountCents,
 * reference?}] and/or payableIds: payables to include (unpaid, 'transfer'
 * method only). Whole-set validation: every error is collected and thrown as
 * BATCH_VALIDATION_FAILED with per-line details.
 */
export function createPaymentBatch(db, {
  date = null, debitIban = null, lines = [], payableIds = [], kind = 'transfer', actor = 'human', dryRun = false,
}) {
  if (!['transfer', 'direct_debit'].includes(kind)) {
    throw paymentsError('INVALID_KIND', "batch kind must be 'transfer' (SEPA credit) or 'direct_debit' (incasso)");
  }
  const company = getCompany(db);
  if (!company) throw paymentsError('COMPANY_REQUIRED', 'company is not initialised');
  const debit = debitIban ? normalizeIban(debitIban) : normalizeIban(company.iban ?? '');
  if (!isValidIban(debit)) {
    throw paymentsError('COMPANY_INCOMPLETE', 'no valid company IBAN — set one with: bukio company update --iban <IBAN>');
  }
  const batchDate = date ?? new Date().toISOString().slice(0, 10);
  if (!/^\d{4}-\d{2}-\d{2}$/.test(batchDate)) {
    throw paymentsError('INVALID_DATE', `batch date '${batchDate}' must be YYYY-MM-DD`);
  }
  const bd = new Date(`${batchDate}T00:00:00Z`);
  if (Number.isNaN(bd.getTime()) || bd.toISOString().slice(0, 10) !== batchDate) {
    throw paymentsError('INVALID_DATE', `batch date '${batchDate}' is not a valid calendar date`);
  }

  const items = [];
  const errors = [];
  const fail = (line, code, message) => errors.push({ line, error: `${code}: ${message}` });

  // explicit lines
  lines.forEach((l, i) => {
    const lineNo = `line ${i + 1}`;
    let name = l.name; let iban = l.iban ? normalizeIban(l.iban) : null; let amountCents = l.amountCents; let reference = l.reference ?? null;
    if (l.contact != null) {
      const c = resolveContact(db, l.contact);
      if (!c) { fail(lineNo, 'CONTACT_NOT_FOUND', `contact '${l.contact}' does not exist`); return; }
      name = name ?? c.name; iban = iban ?? normalizeIban(c.iban ?? '');
      if (!iban) { fail(lineNo, 'CONTACT_IBAN_MISSING', `contact ${c.name} has no IBAN — set one with: bukio contact update --id ${c.id} --iban <IBAN>`); return; }
      l.contact_id = c.id;
    }
    if (!Number.isInteger(amountCents) || amountCents <= 0) { fail(lineNo, 'INVALID_AMOUNT', 'amount must be a positive amount in cents'); return; }
    if (!name) { fail(lineNo, 'NAME_REQUIRED', 'beneficiary name is required'); return; }
    if (!isValidIban(iban)) { fail(lineNo, 'INVALID_IBAN', `'${iban}' is not a valid IBAN`); return; }
    if (reference && reference.length > 140) { fail(lineNo, 'REFERENCE_TOO_LONG', 'reference max 140 characters'); return; }
    items.push({ contact_id: l.contact_id ?? null, name, iban, amount_cents: amountCents, reference });
  });

  // payables
  for (const id of payableIds) {
    const p = db.prepare('SELECT * FROM payables WHERE id = ?').get(id);
    const lineNo = `payable ${id}`;
    if (!p) { fail(lineNo, 'PAYABLE_NOT_FOUND', `payable ${id} does not exist`); continue; }
    if (p.status !== 'unpaid') { fail(lineNo, 'PAYABLE_NOT_UNPAID', `payable ${id} is ${p.status}`); continue; }
    if (kind === 'transfer') {
      if (p.payment_method !== 'transfer') { fail(lineNo, 'PAYABLE_DIRECT_DEBIT', `payable ${id} is paid by direct debit (incasso) — excluded from transfer batches`); continue; }
    } else if (p.payment_method !== 'direct_debit') {
      fail(lineNo, 'PAYABLE_NOT_DIRECT_DEBIT', `payable ${id} is a transfer (betaalbaar) — not an incasso; use a transfer batch`); continue;
    }
    const c = getContact(db, p.contact_id);
    const iban = normalizeIban(c?.iban ?? '');
    if (!iban) { fail(lineNo, 'CONTACT_IBAN_MISSING', `contact ${c?.name ?? p.contact_id} has no IBAN — set one with: bukio contact update --id ${p.contact_id} --iban <IBAN>`); continue; }
    if (!isValidIban(iban)) { fail(lineNo, 'INVALID_IBAN', `'${iban}' is not a valid IBAN`); continue; }
    if (kind === 'direct_debit') {
      const mandate = latestMandate(db, p.contact_id);
      if (!mandate) {
        fail(lineNo, 'MANDATE_REQUIRED', `contact ${c?.name ?? p.contact_id} has no SEPA mandate — add one with: bukio payments mandate add --contact ${p.contact_id} --ref <REF> [--type b2b]`);
        continue;
      }
      items.push({
        payable_id: p.id, contact_id: p.contact_id, name: c.name, iban, amount_cents: p.amount_cents,
        reference: `Factuur ${p.invoice_ref}`,
        mandate_ref: mandate.mandate_ref, mandate_date: mandate.mandate_date,
        mandate_seq: mandateSeqFor(db, p.contact_id, mandate.mandate_ref), scheme: mandate.scheme,
      });
      continue;
    }
    items.push({ payable_id: p.id, contact_id: p.contact_id, name: c.name, iban, amount_cents: p.amount_cents, reference: `Factuur ${p.invoice_ref}` });
  }

  if (items.length === 0 && errors.length === 0) throw paymentsError('EMPTY_BATCH', 'no payments to batch — pass --lines, --csv or --from-invoices');
  if (errors.length) {
    const err = paymentsError('BATCH_VALIDATION_FAILED', `${errors.length} payment line${errors.length === 1 ? '' : 's'} failed validation`);
    err.details = errors;
    throw err;
  }

  const totalCents = items.reduce((s, l) => s + l.amount_cents, 0);
  const plan = {
    action: 'payments.batch.create',
    batch_date: batchDate, debit_iban: debit, debit_name: company.name, batch_kind: kind,
    total_cents: totalCents, lines: items, dryRun,
  };
  if (dryRun) return plan;

  const created = db.transaction(() => {
    const info = db.prepare(
      'INSERT INTO payment_batches (batch_date, debit_iban, debit_name, total_cents, batch_kind, created_by) VALUES (?, ?, ?, ?, ?, ?)',
    ).run(batchDate, debit, company.name, totalCents, kind, actor);
    const batchId = Number(info.lastInsertRowid);
    const insertLine = db.prepare(
      'INSERT INTO payment_batch_lines (batch_id, contact_id, name, iban, amount_cents, reference, mandate_ref, mandate_seq, mandate_date, scheme) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)',
    );
    for (const l of items) {
      const li = insertLine.run(
        batchId, l.contact_id, l.name, l.iban, l.amount_cents, l.reference,
        l.mandate_ref ?? null, l.mandate_seq ?? null, l.mandate_date ?? null, l.scheme ?? null,
      );
      if (l.payable_id) {
        db.prepare("UPDATE payables SET status = 'in_batch', batch_line_id = ? WHERE id = ?").run(Number(li.lastInsertRowid), l.payable_id);
      }
    }
    return batchId;
  })();
  record(db, { actor, action: 'payments.batch.create', command: 'payments batch create', args: { batch_id: created, kind, lines: items.length, total_cents: totalCents }, outcome: 'ok' });
  return getPaymentBatch(db, created);
}

// --- CSV ---------------------------------------------------------------------

/** Parse a batch CSV (',' or ';' delimited, optional header). Whole-file
 *  validation: every parse problem is returned, nothing is written. */
export function parseBatchCsv(csvText) {
  const lines = String(csvText ?? '').split(/\r?\n/).filter((l) => l.trim() !== '');
  const errors = [];
  if (!lines.length) throw paymentsError('EMPTY_CSV', 'batch CSV is empty');
  const parseRow = (line) => {
    const a = line.split(';').length > 1 ? line.split(';') : line.split(',');
    return a.map((c) => c.trim());
  };
  const header = parseRow(lines[0]).map((h) => h.toLowerCase());
  const hasHeader = header.some((h) => ['contact', 'naam', 'leverancier', 'amount', 'bedrag'].includes(h));
  const idx = hasHeader
    ? Object.fromEntries(header.map((h, i) => [h, i]))
    : null;
  const col = (row, key) => {
    if (!idx) {
      // headerless CSV: fixed positional layout contact,amount[,reference]
      const pos = { contact: 0, amount: 1, reference: 2 };
      return row[pos[key]] ?? null;
    }
    for (const alias of CSV_HEADER_ALIASES[key]) {
      if (idx[alias] != null) return row[idx[alias]];
    }
    return null;
  };
  const out = [];
  const start = hasHeader ? 1 : 0;
  for (let i = start; i < lines.length; i += 1) {
    const row = parseRow(lines[i]);
    if (row.length === 1 && row[0] === '') continue;
    const contact = col(row, 'contact');
    const amountStr = col(row, 'amount');
    const reference = col(row, 'reference') || null;
    if (!contact) { errors.push({ line: i + 1, error: 'CONTACT_REQUIRED: every batch line needs a contact' }); continue; }
    let amountCents = null;
    try { amountCents = parseImportAmount(amountStr); } catch { /* reported below */ }
    if (amountCents == null || amountCents <= 0) { errors.push({ line: i + 1, error: `INVALID_AMOUNT: '${amountStr ?? ''}' is not a positive amount` }); continue; }
    out.push({ contact, amountCents, reference });
  }
  if (!out.length && !errors.length) throw paymentsError('EMPTY_CSV', 'batch CSV contains no payment lines');
  return { lines: out, errors, hasHeader };
}

export function createPaymentBatchFromCsv(db, { csvText, date = null, debitIban = null, actor = 'human', dryRun = false }) {
  const { lines, errors } = parseBatchCsv(csvText);
  if (errors.length) {
    const err = paymentsError('IMPORT_VALIDATION_FAILED', `${errors.length} CSV line${errors.length === 1 ? '' : 's'} failed validation`);
    err.details = errors;
    throw err;
  }
  return createPaymentBatch(db, { date, debitIban, lines, actor, dryRun });
}

// --- export ------------------------------------------------------------------

function esc(s) {
  return String(s ?? '').replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;').replace(/'/g, '&apos;');
}

/** Build a pain.001 credit-transfer-initiation document. */
export function buildPain001({ msgId, createdIso, debitName, debitIban, batchDate, lines, schema = '001.03' }) {
  const ns = schema === '001.09' ? 'pain.001.001.09' : 'pain.001.001.03';
  const total = lines.reduce((s, l) => s + l.amount_cents, 0);
  const ctrlSum = (total / 100).toFixed(2);
  const txs = lines.map((l, i) => {
    const endToEnd = (l.reference ? String(l.reference).slice(0, 35) : `BUKIO${i + 1}`);
    return `      <CdtTrfTxInf>
        <PmtId><EndToEndId>${esc(endToEnd)}</EndToEndId></PmtId>
        <Amt><InstdAmt Ccy="EUR">${(l.amount_cents / 100).toFixed(2)}</InstdAmt></Amt>
        <Cdtr><Nm>${esc(l.name)}</Nm></Cdtr>
        <CdtrAcct><Id><IBAN>${esc(l.iban)}</IBAN></Id></CdtrAcct>
        ${l.reference ? `<RmtInf><Ustrd>${esc(l.reference)}</Ustrd></RmtInf>` : ''}
      </CdtTrfTxInf>`;
  }).join('\n');
  return `<?xml version="1.0" encoding="UTF-8"?>
<Document xmlns="urn:iso:std:iso:20022:tech:xsd:${ns}">
  <CstmrCdtTrfInitn>
    <GrpHdr>
      <MsgId>${esc(msgId)}</MsgId>
      <CreDtTm>${esc(createdIso)}</CreDtTm>
      <NbOfTxs>${lines.length}</NbOfTxs>
      <CtrlSum>${ctrlSum}</CtrlSum>
      <InitgPty><Nm>${esc(debitName)}</Nm></InitgPty>
    </GrpHdr>
    <PmtInf>
      <PmtInfId>${esc(msgId)}</PmtInfId>
      <PmtMtd>TRF</PmtMtd>
      <BtchBookg>true</BtchBookg>
      <NbOfTxs>${lines.length}</NbOfTxs>
      <CtrlSum>${ctrlSum}</CtrlSum>
      <PmtTpInf><SvcLvl><Cd>SEPA</Cd></SvcLvl></PmtTpInf>
      <ReqdExctnDt>${esc(batchDate)}</ReqdExctnDt>
      <Dbtr><Nm>${esc(debitName)}</Nm></Dbtr>
      <DbtrAcct><Id><IBAN>${esc(debitIban)}</IBAN></Id></DbtrAcct>
      <ChrgBr>SLEV</ChrgBr>
${txs}
    </PmtInf>
  </CstmrCdtTrfInitn>
</Document>
`;
}

/**
 * Build a pain.008.001.02 direct-debit-initiation document. One PmtInf per
 * mandate scheme (CORE / B2B). Lines carry the debtor's mandate snapshot:
 *   { name, iban, amount_cents, reference, mandate_ref, mandate_date, mandate_seq, scheme }
 */
export function buildPain008({ msgId, createdIso, debitName, debitIban, batchDate, lines }) {
  const total = lines.reduce((s, l) => s + l.amount_cents, 0);
  const ctrlSum = (total / 100).toFixed(2);
  const byScheme = (scheme) => lines.filter((l) => (l.scheme ?? 'core') === scheme);
  const txInf = (l, i) => {
    const endToEnd = (l.reference ? String(l.reference).slice(0, 35) : `BUKIO${i + 1}`);
    return `      <DrctDbtTxInf>
        <PmtId><EndToEndId>${esc(endToEnd)}</EndToEndId></PmtId>
        <InstdAmt Ccy="EUR">${(l.amount_cents / 100).toFixed(2)}</InstdAmt>
        <DrctDbtTx><MndtRltdInf>
          <MndtId>${esc(l.mandate_ref)}</MndtId>
          <DtOfSgntr>${esc(l.mandate_date)}</DtOfSgntr>
        </MndtRltdInf></DrctDbtTx>
        <DbtrAgt><FinInstnId><Othr><Id>NOTPROVIDED</Id></Othr></FinInstnId></DbtrAgt>
        <Dbtr><Nm>${esc(l.name)}</Nm></Dbtr>
        <DbtrAcct><Id><IBAN>${esc(l.iban)}</IBAN></Id></DbtrAcct>
        ${l.reference ? `<RmtInf><Ustrd>${esc(l.reference)}</Ustrd></RmtInf>` : ''}
      </DrctDbtTxInf>`;
  };
  const pmtInf = (scheme, schemeLines, idx) => {
    const subTotal = schemeLines.reduce((s, l) => s + l.amount_cents, 0);
    const subCtrl = (subTotal / 100).toFixed(2);
    const txs = schemeLines.map((l, i) => txInf(l, i)).join('\n');
    return `    <PmtInf>
      <PmtInfId>${esc(msgId)}${idx}</PmtInfId>
      <PmtMtd>DD</PmtMtd>
      <BtchBookg>true</BtchBookg>
      <NbOfTxs>${schemeLines.length}</NbOfTxs>
      <CtrlSum>${subCtrl}</CtrlSum>
      <PmtTpInf><SvcLvl><Cd>SEPA</Cd></SvcLvl><LclInstrm><Cd>${scheme === 'b2b' ? 'B2B' : 'CORE'}</Cd></LclInstrm></PmtTpInf>
      <ReqdColltnDt>${esc(batchDate)}</ReqdColltnDt>
      <Cdtr><Nm>${esc(debitName)}</Nm></Cdtr>
      <CdtrAcct><Id><IBAN>${esc(debitIban)}</IBAN></Id></CdtrAcct>
      <CdtrAgt><FinInstnId><Othr><Id>NOTPROVIDED</Id></Othr></FinInstnId></CdtrAgt>
      <ChrgBr>SLEV</ChrgBr>
${txs}
    </PmtInf>`;
  };
  const schemes = ['core', 'b2b'].filter((s) => byScheme(s).length > 0);
  const pmtInfs = schemes.map((s, i) => pmtInf(s, byScheme(s), i + 1)).join('\n');
  return `<?xml version="1.0" encoding="UTF-8"?>
<Document xmlns="urn:iso:std:iso:20022:tech:xsd:pain.008.001.02">
  <CstmrDrctDbtInitn>
    <GrpHdr>
      <MsgId>${esc(msgId)}</MsgId>
      <CreDtTm>${esc(createdIso)}</CreDtTm>
      <NbOfTxs>${lines.length}</NbOfTxs>
      <CtrlSum>${ctrlSum}</CtrlSum>
      <InitgPty><Nm>${esc(debitName)}</Nm></InitgPty>
    </GrpHdr>
${pmtInfs}
  </CstmrDrctDbtInitn>
</Document>
`;
}

export function exportPaymentBatch(db, { id, schema = null, actor = 'human', dryRun = false }) {
  const batch = db.prepare('SELECT * FROM payment_batches WHERE id = ?').get(id);
  if (!batch) throw paymentsError('BATCH_NOT_FOUND', `batch ${id} does not exist`);
  if (batch.status !== 'draft') throw paymentsError('BATCH_ALREADY_EXPORTED', `batch ${id} is already ${batch.status} — exporting again could double-pay; create a new batch instead`);
  const isDD = batch.batch_kind === 'direct_debit';
  const effSchema = schema ?? (isDD ? '008.02' : '001.03');
  if (isDD) {
    if (!['008.02'].includes(effSchema)) throw paymentsError('INVALID_SCHEMA', 'direct-debit batches export pain.008.001.02 only');
  } else if (!['001.03', '001.09'].includes(effSchema)) {
    throw paymentsError('INVALID_SCHEMA', "schema must be '001.03' or '001.09'");
  }
  const lines = db.prepare('SELECT * FROM payment_batch_lines WHERE batch_id = ? ORDER BY id').all(id);
  // SEPA MsgId max 35 chars: BUKIO + 14-char timestamp + batch id (last 16
  // digits — ids beyond 10^16 are not realistic, but the slice keeps the
  // contract unconditional; the guard below would catch any future change)
  const msgId = `BUKIO${new Date().toISOString().replace(/[-:TZ.]/g, '').slice(0, 14)}${String(id).slice(-16)}`;
  if (msgId.length > 35) throw paymentsError('MSGID_TOO_LONG', `generated MsgId '${msgId}' exceeds the SEPA 35-character limit`);
  const xml = isDD
    ? buildPain008({
        msgId, createdIso: new Date().toISOString().replace(/\.\d{3}Z$/, 'Z'),
        debitName: batch.debit_name, debitIban: batch.debit_iban, batchDate: batch.batch_date,
        lines,
      })
    : buildPain001({
        msgId, createdIso: new Date().toISOString().replace(/\.\d{3}Z$/, 'Z'),
        debitName: batch.debit_name, debitIban: batch.debit_iban, batchDate: batch.batch_date,
        lines, schema: effSchema,
      });
  const storedSchema = isDD ? 'pain.008.001.02' : (effSchema === '001.03' ? 'pain.001.001.03' : 'pain.001.001.09');
  const fileHash = createHash('sha256').update(xml).digest('hex');
  if (dryRun) {
    return { action: 'payments.batch.export', batch_id: id, batch_kind: batch.batch_kind, schema: storedSchema, msg_id: msgId, lines: lines.length, total_cents: batch.total_cents, file_hash: fileHash, xml, dryRun: true };
  }
  db.prepare("UPDATE payment_batches SET status = 'exported', msg_id = ?, file_hash = ?, schema = ?, exported_at = ? WHERE id = ?")
    .run(msgId, fileHash, storedSchema, new Date().toISOString(), id);
  record(db, { actor, action: 'payments.batch.export', command: 'payments batch export', args: { batch_id: id, kind: batch.batch_kind, msg_id: msgId, lines: lines.length, total_cents: batch.total_cents, file_hash: fileHash.slice(0, 12), schema: storedSchema }, outcome: 'ok' });
  return { batch_id: id, status: 'exported', msg_id: msgId, file_hash: fileHash, schema: storedSchema, xml, lines: lines.length, total_cents: batch.total_cents };
}

export function deletePaymentBatch(db, { id, actor = 'human', dryRun = false }) {
  const batch = db.prepare('SELECT * FROM payment_batches WHERE id = ?').get(id);
  if (!batch) throw paymentsError('BATCH_NOT_FOUND', `batch ${id} does not exist`);
  if (batch.status !== 'draft') throw paymentsError('BATCH_ALREADY_EXPORTED', `batch ${id} is already ${batch.status} — delete is only allowed on drafts`);
  if (dryRun) return { action: 'payments.batch.delete', batch_id: id, dryRun: true };
  db.transaction(() => {
    // release payables back to unpaid
    db.prepare("UPDATE payables SET status = 'unpaid', batch_line_id = NULL WHERE batch_line_id IN (SELECT id FROM payment_batch_lines WHERE batch_id = ?)").run(id);
    db.prepare('DELETE FROM payment_batches WHERE id = ?').run(id);
  })();
  record(db, { actor, action: 'payments.batch.delete', command: 'payments batch delete', args: { batch_id: id }, outcome: 'ok' });
  return { batch_id: id, status: 'deleted' };
}
