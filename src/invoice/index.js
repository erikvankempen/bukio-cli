// Invoicing module (Phase 3, FR3) — outgoing invoices compliant with the
// 12 verplichte factuurvereisten, lifecycle draft -> sent -> paid, credit
// notes, booking integration, UBL/PDF export hooks.
import { getAccountByCode } from '../core/accounts.js';
import { createEntry, postEntry } from '../core/entries.js';
import { record } from '../audit/index.js';
import { isVatEnabled, listVatCodes } from '../vat/index.js';
import { parseBankAmount } from '../bank/csv.js';

export function invoiceError(code, message) {
  const e = new Error(message);
  e.code = code;
  return e;
}

// --- line spec parsing ----------------------------------------------------

/**
 * Parse a line spec: "[QTYx] DESCRIPTION @ PRICE [@ VATCODE]"
 * e.g. "2x Consultancy @ 150.00 @21" -> { qty: 2, description: 'Consultancy',
 * priceCents: 15000, vatCode: '21' }
 * Splits from the right so descriptions may contain '@' (e.g. "Email @ adres").
 */
export function parseLineSpec(spec) {
  const s = String(spec).trim();
  const qtyMatch = s.match(/^(\d+)\s*x\s+(.+)$/);
  const qty = qtyMatch ? parseInt(qtyMatch[1], 10) : 1;
  const rest = qtyMatch ? qtyMatch[2] : s;
  const parts = rest.split('@').map((p) => p.trim());
  let vatCode = null;
  let pricePart = parts[parts.length - 1];
  if (/^[A-Z0-9]+$/.test(pricePart)) {
    vatCode = pricePart;
    pricePart = parts[parts.length - 2];
  }
  const description = parts.slice(0, parts.length - (vatCode ? 2 : 1)).join('@').trim();
  const priceCents = parseBankAmount(pricePart);
  if (!description || !priceCents || priceCents <= 0) {
    throw invoiceError('INVALID_LINE', `line '${spec}' must be "[QTYx] DESCRIPTION @ PRICE [@ VATCODE]"`);
  }
  return { qty, description, priceCents, vatCode };
}

/** Split a (possibly comma-separated) line-spec list into individual specs. */
export function splitLineSpecs(lines) {
  return lines.flatMap((spec) => (typeof spec === 'string'
    ? String(spec).split(',').map((s) => s.trim()).filter(Boolean)
    : [spec]));
}

/**
 * Validate a line-spec list without inserting anything (used by recurring
 * invoice templates at creation). Returns the parsed lines with vat_rate_bp.
 */
export function validateInvoiceLines(db, lines) {
  const vatOn = isVatEnabled(db);
  const vatCodes = new Map(listVatCodes(db).map((c) => [c.code, c]));
  return splitLineSpecs(lines).map((spec) => {
    const p = typeof spec === 'string' ? parseLineSpec(spec) : spec;
    if (!p.description || !p.priceCents) throw invoiceError('INVALID_LINE', `line '${JSON.stringify(spec)}' is not parseable`);
    if (p.vatCode) {
      if (!vatOn) throw invoiceError('VAT_MODULE_OFF', 'line has a VAT code but the VAT module is off for this company');
      if (!vatCodes.has(p.vatCode)) throw invoiceError('VAT_CODE_NOT_FOUND', `vat code '${p.vatCode}' does not exist`);
    }
    return { ...p, vat_rate_bp: p.vatCode ? vatCodes.get(p.vatCode).rate_bp : 0 };
  });
}

// --- contacts -------------------------------------------------------------

export function createContact(db, {
  name, address = null, postalCode = null, city = null, country = 'NL',
  email = null, vatId = null, kvk = null, actor = 'human',
}) {
  if (!name || typeof name !== 'string') throw invoiceError('INVALID_NAME', 'contact needs a name');
  const info = db.prepare(`
    INSERT INTO contacts (name, address, postal_code, city, country, email, vat_id, kvk, created_by)
    VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
  `).run(name, address, postalCode, city, country, email, vatId, kvk, actor);
  record(db, { actor, action: 'contact.create', command: 'contact add', args: { name }, outcome: 'ok' });
  return getContact(db, info.lastInsertRowid);
}

export function getContact(db, id) {
  return db.prepare('SELECT * FROM contacts WHERE id = ?').get(id) ?? null;
}

export function listContacts(db) {
  return db.prepare('SELECT * FROM contacts ORDER BY name').all();
}

// --- invoice helpers ------------------------------------------------------

function vatCodeRate(db, code) {
  if (!code) return 0;
  const row = db.prepare('SELECT rate_bp FROM vat_codes WHERE code = ?').get(code);
  return row ? row.rate_bp : 0;
}

export function getInvoice(db, id) {
  const inv = db.prepare('SELECT * FROM invoices WHERE id = ?').get(id);
  if (!inv) return null;
  inv.lines = db.prepare('SELECT * FROM invoice_lines WHERE invoice_id = ? ORDER BY line_no').all(id);
  inv.payments = db.prepare('SELECT * FROM invoice_payments WHERE invoice_id = ? ORDER BY date, id').all(id);
  inv.contact = getContact(db, inv.contact_id);
  inv.net_cents = inv.lines.reduce((s, l) => s + l.amount_cents, 0);
  inv.vat_cents = inv.lines.reduce((s, l) => s + l.vat_amount_cents, 0);
  inv.gross_cents = inv.net_cents + inv.vat_cents;
  inv.paid_cents = inv.payments.reduce((s, p) => s + p.amount_cents, 0);
  // derived status
  inv.status = inv.status === 'sent' && inv.due_date && inv.due_date < new Date().toISOString().slice(0, 10)
    ? 'overdue' : inv.status;
  return inv;
}

export function listInvoices(db, { status = null, type = null } = {}) {
  const clauses = [];
  const params = [];
  if (status) { clauses.push('status = ?'); params.push(status); }
  if (type) { clauses.push('invoice_type = ?'); params.push(type); }
  const where = clauses.length ? `WHERE ${clauses.join(' AND ')}` : '';
  const rows = db.prepare(`SELECT * FROM invoices ${where} ORDER BY id DESC`).all(...params);
  return rows.map((r) => getInvoice(db, r.id));
}

/**
 * Create a draft invoice. Lines carry a VAT rate snapshot (vat_rate_bp) so
 * exports stay correct even if vat_codes change later. VAT codes are only
 * allowed when the VAT module is on.
 */
export function createInvoice(db, {
  contactId, lines, date, dueDays = 30, deliveryDate = null,
  description = null, reference = null, notes = null, actor = 'human',
}) {
  const contact = getContact(db, contactId);
  if (!contact) throw invoiceError('CONTACT_NOT_FOUND', `contact ${contactId} does not exist`);
  if (!/^\d{4}-\d{2}-\d{2}$/.test(date)) throw invoiceError('INVALID_DATE', `date '${date}' must be YYYY-MM-DD`);
  if (!Array.isArray(lines) || lines.length === 0) throw invoiceError('NO_LINES', 'an invoice needs at least one line');

  const vatOn = isVatEnabled(db);
  const vatCodes = new Map(listVatCodes(db).map((c) => [c.code, c]));
  const parsedLines = splitLineSpecs(lines).map((spec) => {
    const p = typeof spec === 'string' ? parseLineSpec(spec) : spec;
    if (!p.description || !p.priceCents) throw invoiceError('INVALID_LINE', `line '${JSON.stringify(spec)}' is not parseable`);
    if (p.vatCode) {
      if (!vatOn) throw invoiceError('VAT_MODULE_OFF', 'line has a VAT code but the VAT module is off for this company');
      if (!vatCodes.has(p.vatCode)) throw invoiceError('VAT_CODE_NOT_FOUND', `vat code '${p.vatCode}' does not exist`);
    }
    const rateBp = p.vatCode ? vatCodes.get(p.vatCode).rate_bp : 0;
    const amount = p.qty * p.priceCents;
    const vat = Math.round(Math.abs(amount * rateBp / 10000));
    return {
      line_no: 0, description: p.description, quantity: p.qty,
      unit_price_cents: p.priceCents, vat_code: p.vatCode, vat_rate_bp: rateBp,
      amount_cents: amount, vat_amount_cents: vat,
    };
  });

  const dueDate = dueDays != null
    ? new Date(Date.UTC(Number(date.slice(0, 4)), Number(date.slice(5, 7)) - 1, Number(date.slice(8, 10)) + dueDays)).toISOString().slice(0, 10)
    : null;

  const tx = db.transaction(() => {
    const info = db.prepare(`
      INSERT INTO invoices (contact_id, date, due_date, delivery_date, description, reference, notes, created_by)
      VALUES (?, ?, ?, ?, ?, ?, ?, ?)
    `).run(contactId, date, dueDate, deliveryDate, description, reference, notes, actor);
    const invoiceId = info.lastInsertRowid;
    const insertLine = db.prepare(`
      INSERT INTO invoice_lines
        (invoice_id, line_no, description, quantity, unit_price_cents, vat_code, vat_rate_bp, amount_cents, vat_amount_cents)
      VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
    `);
    parsedLines.forEach((l, i) => {
      insertLine.run(invoiceId, i + 1, l.description, l.quantity, l.unit_price_cents,
        l.vat_code, l.vat_rate_bp, l.amount_cents, l.vat_amount_cents);
    });
    record(db, {
      actor, action: 'invoice.create', command: 'invoice create',
      args: { contactId, date, lines: parsedLines.length, net: parsedLines.reduce((s, l) => s + l.amount_cents, 0) },
      outcome: 'ok', invoiceId,
    });
    return invoiceId;
  });
  return getInvoice(db, tx());
}

// --- compliance (12 factuurvereisten) -------------------------------------

/**
 * Validate the 12 verplichte factuurvereisten (art. 35c/35d Wet OB + KVK).
 * Returns the list of missing vereisten; throws SUPPLIER_INCOMPLETE /
 * CUSTOMER_INCOMPLETE / CUSTOMER_VAT_REQUIRED when they matter.
 */
export function validateCompliance(db, invoice) {
  const company = db.prepare('SELECT * FROM company WHERE id = 1').get();
  const contact = invoice.contact;

  const missingSupplier = [];
  if (!company.name) missingSupplier.push('bedrijfsnaam');
  if (!company.btw_id) missingSupplier.push('btw-id');
  if (!company.kvk) missingSupplier.push('kvk-nummer');
  if (!company.address) missingSupplier.push('adres');
  if (!company.postal_code) missingSupplier.push('postcode');
  if (!company.city) missingSupplier.push('plaats');
  if (missingSupplier.length) {
    throw invoiceError('SUPPLIER_INCOMPLETE', `supplier gegevens ontbreken (vereiste 1-3): ${missingSupplier.join(', ')} — set them with init/company update`);
  }

  if (!contact.name || !contact.address || !contact.city) {
    throw invoiceError('CUSTOMER_INCOMPLETE', 'klantgegevens ontbreken (vereiste 6): naam, adres en plaats zijn verplicht');
  }

  const hasReverse = invoice.lines.some((l) => l.vat_code === 'R' || l.vat_code === 'RE');
  if (hasReverse && !contact.vat_id) {
    throw invoiceError('CUSTOMER_VAT_REQUIRED', 'btw verlegd op de factuur: het btw-id van de klant is verplicht (vereiste 7)');
  }

  return { ok: true, vereisten: 12 };
}

// --- numbering + finalize -------------------------------------------------

export function nextInvoiceNumber(db, year) {
  const row = db.prepare(
    "SELECT MAX(CAST(SUBSTR(invoice_number, 6) AS INTEGER)) AS m FROM invoices WHERE invoice_number LIKE ?",
  ).get(`${year}-%`);
  return `${year}-${String((row?.m ?? 0) + 1).padStart(4, '0')}`;
}

/**
 * Build the booking postings for an invoice: Debiteuren vs Omzet + Te betalen
 * btw (VAT module on) or Debiteuren vs Omzet (module off). Credit notes use
 * the reversed signs. Line-exact VAT rounding is preserved per rate group.
 */
export function buildInvoicePostings(db, invoice) {
  const vatOn = isVatEnabled(db);
  const isCredit = invoice.invoice_type === 'credit';
  const sign = isCredit ? 1 : -1; // sales: omzet is credit (-), credit note: debit (+)
  const gross = invoice.gross_cents;
  const postings = [];

  if (vatOn) {
    // group nets by vat code (vat totals are the exact per-line sums)
    const groups = new Map();
    for (const l of invoice.lines) {
      const key = l.vat_code ?? 'none';
      if (!groups.has(key)) groups.set(key, { net: 0, vat: 0, code: l.vat_code });
      groups.get(key).net += l.amount_cents;
      groups.get(key).vat += l.vat_amount_cents;
    }
    for (const g of groups.values()) {
      if (g.net === 0) continue;
      if (g.code && g.vat > 0) {
        // btw-plichtige omzet: tagged posting so the OB readout picks it up
        postings.push({
          code: '8000', amountCents: sign * g.net,
          vatCode: g.code, vatAmountCents: sign * g.vat,
        });
      } else {
        // 0% / vrijgesteld / verlegd: no VAT on the books
        postings.push({ code: '8000', amountCents: sign * g.net });
      }
    }
    if (invoice.vat_cents > 0) {
      postings.push({ code: '2500', amountCents: sign * invoice.vat_cents });
    }
  } else {
    postings.push({ code: '8000', amountCents: sign * invoice.net_cents });
  }
  // debiteuren leg: sales -> debit (+gross), credit note -> credit (-gross)
  postings.push({ code: '1200', amountCents: isCredit ? -gross : gross });
  return postings;
}

/** Finalize a draft invoice: assign the sequential number and book the entry. */
export function finalizeInvoice(db, { id, actor = 'human', dryRun = false }) {
  const invoice = getInvoice(db, id);
  if (!invoice) throw invoiceError('NOT_FOUND', `invoice ${id} does not exist`);
  if (invoice.status !== 'draft') throw invoiceError('ALREADY_FINALIZED', `invoice ${id} is already ${invoice.status}`);

  validateCompliance(db, invoice);
  const year = invoice.date.slice(0, 4);
  const number = nextInvoiceNumber(db, year);
  const postings = buildInvoicePostings(db, invoice);

  if (dryRun) {
    return {
      invoice_number: number, postings,
      net: invoice.net_cents, vat: invoice.vat_cents, gross: invoice.gross_cents,
      dryRun: true,
    };
  }

  const tx = db.transaction(() => {
    const entry = createEntry(db, {
      date: invoice.date,
      description: `Factuur ${number}${invoice.contact ? ` - ${invoice.contact.name}` : ''}`,
      postings,
      source: 'invoice',
      sourceRef: `inv:${id}`,
      actor,
    });
    const posted = postEntry(db, { id: entry.id, actor });
    db.prepare('UPDATE invoices SET invoice_number = ?, status = ?, entry_id = ? WHERE id = ?')
      .run(number, 'sent', posted.id, id);
    record(db, {
      actor, action: 'invoice.finalize', command: 'invoice finalize',
      args: { id, invoice_number: number, gross: invoice.gross_cents }, outcome: 'ok', entryIds: [posted.id],
    });
    return posted;
  });
  const posted = tx();
  return { invoice: getInvoice(db, id), entry: posted, dryRun: false };
}

/** Create a credit note (draft) from a finalized sales invoice. */
export function creditInvoice(db, { id, date = null, reason = null, actor = 'human' }) {
  const original = getInvoice(db, id);
  if (!original) throw invoiceError('NOT_FOUND', `invoice ${id} does not exist`);
  if (original.invoice_type !== 'sales') throw invoiceError('NOT_SALES_INVOICE', 'only sales invoices can be credited');
  if (!['sent', 'paid', 'overdue'].includes(original.status)) throw invoiceError('NOT_FINALIZED', 'the invoice must be finalized before crediting');

  const creditId = createInvoice(db, {
    contactId: original.contact_id,
    lines: original.lines.map((l) => ({
      qty: l.quantity, description: l.description,
      priceCents: l.unit_price_cents, vatCode: l.vat_code,
    })),
    date: date ?? new Date().toISOString().slice(0, 10),
    dueDays: null,
    description: reason ?? `Creditfactuur voor ${original.invoice_number}`,
    reference: original.invoice_number,
    actor,
  }).id;
  db.prepare("UPDATE invoices SET invoice_type = 'credit', credit_for_invoice_id = ? WHERE id = ?")
    .run(id, creditId);
  record(db, { actor, action: 'invoice.credit', command: 'invoice credit', args: { id, creditId }, outcome: 'ok' });
  return getInvoice(db, creditId);
}

// --- payments -------------------------------------------------------------

/**
 * Record a payment (tracking only — the posting comes from the bank flow).
 * When fully paid, the invoice status becomes 'paid'.
 */
export function markPaid(db, { id, date, amountCents, method = 'bank', bankTxId = null, actor = 'human' }) {
  const invoice = getInvoice(db, id);
  if (!invoice) throw invoiceError('NOT_FOUND', `invoice ${id} does not exist`);
  if (invoice.invoice_type === 'credit') throw invoiceError('CREDIT_NOT_PAYABLE', 'credit notes are not payable');
  if (!['sent', 'overdue'].includes(invoice.status)) throw invoiceError('NOT_PAYABLE', `invoice ${id} is ${invoice.status}`);
  if (!Number.isInteger(amountCents) || amountCents <= 0) throw invoiceError('INVALID_AMOUNT', 'payment amount must be positive cents');

  const remaining = invoice.gross_cents - invoice.paid_cents;
  if (amountCents > remaining) throw invoiceError('OVERPAYMENT', `payment ${amountCents} exceeds the outstanding ${remaining}`);

  db.prepare('INSERT INTO invoice_payments (invoice_id, date, amount_cents, method, bank_tx_id, created_by) VALUES (?, ?, ?, ?, ?, ?)')
    .run(id, date, amountCents, method, bankTxId, actor);
  const paidNow = invoice.paid_cents + amountCents;
  if (paidNow >= invoice.gross_cents) {
    db.prepare("UPDATE invoices SET status = 'paid' WHERE id = ?").run(id);
  }
  record(db, { actor, action: 'invoice.pay', command: 'invoice pay', args: { id, amountCents }, outcome: 'ok' });
  return getInvoice(db, id);
}

/**
 * Apply a bank payment to an invoice: record the payment, post the bank entry
 * (Bank / Debiteuren) and reconcile the transaction. Used by the bank
 * auto-match engine and by `invoice pay --from-bank`.
 */
export function paymentFromBank(db, { invoiceId, bankTxId, actor = 'human' }) {
  const invoice = getInvoice(db, invoiceId);
  if (!invoice) throw invoiceError('NOT_FOUND', `invoice ${invoiceId} does not exist`);
  const txRow = db.prepare('SELECT * FROM bank_transactions WHERE id = ?').get(bankTxId);
  if (!txRow) throw invoiceError('NOT_FOUND', `bank transaction ${bankTxId} does not exist`);
  if (txRow.state !== 'unmatched') throw invoiceError('ALREADY_MATCHED', `bank transaction ${bankTxId} is already ${txRow.state}`);
  const bankAccount = db.prepare('SELECT * FROM bank_accounts WHERE id = ?').get(txRow.bank_account_id);

  const paid = markPaid(db, { id: invoiceId, date: txRow.date, amountCents: txRow.amount_cents, method: 'bank', bankTxId, actor });
  const entry = createEntry(db, {
    date: txRow.date,
    description: `Betaling ${invoice.invoice_number ?? invoiceId}${invoice.contact ? ` - ${invoice.contact.name}` : ''}`,
    postings: [
      { code: bankAccount.account_code, amountCents: txRow.amount_cents },
      { code: '1200', amountCents: -txRow.amount_cents },
    ],
    source: 'bank',
    sourceRef: `tx:${bankTxId}`,
    actor,
  });
  const posted = postEntry(db, { id: entry.id, actor });
  db.prepare(`
    INSERT INTO reconciliations (bank_tx_id, target_type, target_id, method, confidence, created_by)
    VALUES (?, 'invoice', ?, ?, 1.0, ?)
  `).run(bankTxId, invoiceId, actor.startsWith('agent') ? 'agent' : 'manual', actor);
  db.prepare("UPDATE bank_transactions SET state = 'matched' WHERE id = ?").run(bankTxId);
  record(db, {
    actor, action: 'invoice.payment_bank', command: 'bank match',
    args: { invoiceId, bankTxId }, outcome: 'ok', entryIds: [posted.id],
  });
  return { invoice: paid, entry: posted };
}
