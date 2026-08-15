/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Central i18n mechanism (S1, owner decision 15 Aug 2026):
//   - t(key, params, locale) resolves a key against the locale table with
//     fallbacks: requested locale -> 'en' (default) -> the key itself.
//   - resolveLocale(ctx, db) picks the active locale: explicit --locale flag
//     (or BUKIO_LOCALE env) -> the company's stored locale (set at init from
//     the country profile) -> 'en'. English is the default whenever
//     localization is unset or a locale has no table yet.
//   - Statutory artifacts (XAF/FAIA, jaarrekening models, OB readout),
//     JSON output keys, error codes and MCP descriptions never localize
//     (per the Aug 14 decision: documents localize, UI/JSON stay English).
// Line descriptions and account names are data, never auto-translated.

export const UNIT_CODES = ['h', 'day', 'month', 'unit', 'session', 'km', 'kg', 'project'];

export const TABLES = {
  en: {
    // --- invoice PDF / document labels (structural labels only) ---
    'pdf.invoice': 'INVOICE', 'pdf.credit': 'CREDIT NOTE',
    'pdf.billedTo': 'Billed to', 'pdf.date': 'Date', 'pdf.dueDate': 'Due date',
    'pdf.reference': 'Reference', 'pdf.kvk': 'CoC', 'pdf.btw': 'VAT',
    'pdf.description': 'Description', 'pdf.qty': 'Qty', 'pdf.unit': 'Unit', 'pdf.price': 'Price',
    'pdf.vat': 'VAT', 'pdf.discount': 'Discount', 'pdf.amount': 'Amount',
    'pdf.subtotal': 'Subtotal excl. VAT', 'pdf.vatOn': 'VAT on', 'pdf.vatTotal': 'Total VAT',
    'pdf.total': 'Total', 'pdf.inclVat': 'incl. VAT',
    'pdf.footerPay': 'Please transfer the amount within {term} to IBAN {iban} for the account of {name}.',
    'pdf.dueDateTerm': '{date}', 'pdf.defaultTerm': 'the agreed term',
    // --- units ---
    'unit.h': 'h', 'unit.day': 'day', 'unit.month': 'month', 'unit.unit': 'unit',
    'unit.session': 'session', 'unit.km': 'km', 'unit.kg': 'kg', 'unit.project': 'project',
    // --- statuses & directions ---
    'status.draft': 'draft', 'status.posted': 'posted', 'status.open': 'open',
    'status.closed': 'closed', 'status.overdue': 'overdue', 'status.paid': 'paid',
    'dir.payable': 'payable', 'dir.receivable': 'receivable', 'dir.debit': 'debit', 'dir.credit': 'credit',
    // --- report labels (generic reports) ---
    'report.revenue': 'revenue', 'report.costs': 'costs', 'report.result': 'result',
    'report.undistributedResult': 'undistributed result', 'report.totalLiabilities': 'total liabilities',
    'report.netResult': 'net result', 'report.profitAndLoss': 'PROFIT AND LOSS', 'report.pnlSheet': 'Profit and loss',
    // --- vat file / settle descriptions ---
    'vat.file.description': 'VAT return{period} — transfer to {account} ({direction})',
    'vat.settle.description': 'VAT return payment{period} — {account} (rounding difference {amount})',
    // --- emails ---
    'email.invoiceSubject': 'Invoice {number} — {company}',
    'email.invoiceBody': 'Dear client,\n\nPlease find attached invoice {number} for a total of {gross} (incl. VAT).\n\nKind regards,\n{company}',
    'email.reminderSubject': 'Payment reminder invoice {number}',
    'email.reminderBody': 'Dear {name},\n\nInvoice {number} still has {outstanding} outstanding (due {dueDate}).\n{transfer}\n\nKind regards,\n{company}',
    'email.reminderTransferIban': 'Please transfer the amount to IBAN {iban} with reference {number}.',
    'email.reminderTransferPlain': 'Please transfer the outstanding amount with reference {number}.',
    // --- invoice list / reminders tables ---
    'invlist.number': 'number', 'invlist.type': 'type', 'invlist.date': 'date',
    'invlist.customer': 'customer', 'invlist.total': 'total', 'invlist.status': 'status',
    'reminder.none': 'no reminders as of {date}',
    'invlist.days': 'days', 'invlist.reminder': 'reminder',
    'entry.paid': 'paid {date}: {amount} ({method})',
    'entry.outstanding': 'outstanding: {amount}',
    // --- month-end / year-end renders ---
    'monthend.totals': 'totals:   debit {debit} / credit {credit} {state}',
    'yearend.plan': 'plan: close {year} — result {amount}{extra}',
    'yearend.closed': '{year} closed — result {amount} (entries #{entries}, posted)',
    'yearend.status': '{year}: {state} — result {amount}',
  },
  nl: {
    // --- invoice PDF / document labels (structural labels only) ---
    'pdf.invoice': 'FACTUUR', 'pdf.credit': 'CREDITFACTUUR',
    'pdf.billedTo': 'Factuur aan', 'pdf.date': 'Datum', 'pdf.dueDate': 'Vervaldatum',
    'pdf.reference': 'Referentie', 'pdf.kvk': 'KvK', 'pdf.btw': 'BTW',
    'pdf.description': 'Omschrijving', 'pdf.qty': 'Aantal', 'pdf.unit': 'Eenheid', 'pdf.price': 'Prijs',
    'pdf.vat': 'Btw', 'pdf.discount': 'Korting', 'pdf.amount': 'Bedrag',
    'pdf.subtotal': 'Subtotaal excl. btw', 'pdf.vatOn': 'Btw over', 'pdf.vatTotal': 'Totaal btw',
    'pdf.total': 'Totaal', 'pdf.inclVat': 'incl. btw',
    'pdf.footerPay': 'Gelieve het bedrag binnen {term} over te maken op IBAN {iban} t.n.v. {name}.',
    'pdf.dueDateTerm': '{date}', 'pdf.defaultTerm': 'de gestelde termijn',
    // --- units ---
    'unit.h': 'uur', 'unit.day': 'dag', 'unit.month': 'maand', 'unit.unit': 'stuks',
    'unit.session': 'sessie', 'unit.km': 'km', 'unit.kg': 'kg', 'unit.project': 'project',
    // --- statuses & directions ---
    'status.draft': 'concept', 'status.posted': 'geboekt', 'status.open': 'open',
    'status.closed': 'gesloten', 'status.overdue': 'vervallen', 'status.paid': 'betaald',
    'dir.payable': 'te betalen', 'dir.receivable': 'te ontvangen', 'dir.debit': 'debet', 'dir.credit': 'credit',
    // --- report labels (generic reports) ---
    'report.revenue': 'opbrengsten', 'report.costs': 'kosten', 'report.result': 'resultaat',
    'report.undistributedResult': 'Nog te verdelen resultaat', 'report.totalLiabilities': 'totaal passiva',
    'report.netResult': 'Netto resultaat', 'report.profitAndLoss': 'WINST- EN VERLIESREKENING', 'report.pnlSheet': 'Winst en verlies',
    // --- vat file / settle descriptions ---
    'vat.file.description': 'OB-aangifte{period} verlegging naar {account} ({direction})',
    'vat.settle.description': 'Betaling OB-aangifte{period} — {account} (afrondingsverschil {amount})',
    // --- emails ---
    'email.invoiceSubject': 'Factuur {number} — {company}',
    'email.invoiceBody': 'Geachte,\n\nHierbij ontvangt u factuur {number} voor een totaalbedrag van {gross} (incl. btw).\n\nMet vriendelijke groet,\n{company}',
    'email.reminderSubject': 'Betalingsherinnering factuur {number}',
    'email.reminderBody': 'Beste {name},\n\nVoor factuur {number} staat nog {outstanding} open (vervaldatum {dueDate}).\n{transfer}\n\nMet vriendelijke groet,\n{company}',
    'email.reminderTransferIban': 'Wilt u dit bedrag overmaken naar IBAN {iban} o.v.v. {number}?',
    'email.reminderTransferPlain': 'Wilt u het openstaande bedrag overmaken o.v.v. {number}?',
    // --- invoice list / reminders tables ---
    'invlist.number': 'nummer', 'invlist.type': 'type', 'invlist.date': 'datum',
    'invlist.customer': 'klant', 'invlist.total': 'totaal', 'invlist.status': 'status',
    'reminder.none': 'geen herinneringen per {date}',
    'invlist.days': 'dagen', 'invlist.reminder': 'herinnering',
    'entry.paid': 'betaald {date}: {amount} ({method})',
    'entry.outstanding': 'openstaand: {amount}',
    // --- month-end / year-end renders ---
    'monthend.totals': 'totalen:  debet {debit} / credit {credit} {state}',
    'yearend.plan': 'plan: sluit {year} — resultaat {amount}{extra}',
    'yearend.closed': '{year} gesloten — resultaat {amount} (boekingen #{entries}, geboekt)',
    'yearend.status': '{year}: {state} — resultaat {amount}',
  },
};

/** Resolve the active UI locale: --locale flag > BUKIO_LOCALE env > 'en'.
 *
 * UI text is English by default (Aug 14 decision: UI/JSON stay English;
 * localization is an explicit opt-in via --locale / BUKIO_LOCALE). The
 * company's stored locale drives DOCUMENTS (invoice.language), not the CLI
 * surface — a company locale of 'nl' (the migration default) must not flip
 * the whole UI back to Dutch. */
export function resolveLocale(ctx = {}, db = null) {
  ctx = ctx ?? {};
  if (ctx.locale) return ctx.locale;
  if (process.env.BUKIO_LOCALE) return process.env.BUKIO_LOCALE;
  return 'en';
}

/** Translate key with {param} interpolation; fallback: locale -> en -> key. */
export function t(key, params = {}, locale = 'en') {
  const table = TABLES[locale] ?? TABLES.en;
  let s = table[key] ?? TABLES.en[key] ?? key;
  for (const [k, v] of Object.entries(params)) s = s.replaceAll(`{${k}}`, String(v));
  return s;
}

/** Legacy invoice-label API (was src/invoice/i18n.js) — label(k, 'nl'|'en'). */
export function label(key, language = 'nl') {
  return t(`pdf.${key}`, {}, language);
}

/** Legacy unit-label API (was src/invoice/i18n.js). */
export function unitLabel(code, language = 'nl') {
  const s = t(`unit.${code}`, {}, language);
  return s === `unit.${code}` ? (code ?? '') : s;
}

// Backwards-compatible exports for importers of src/invoice/i18n.js.
export const LABELS = {
  nl: Object.fromEntries(Object.entries(TABLES.nl).filter(([k]) => k.startsWith('pdf.')).map(([k, v]) => [k.slice(4), v])),
  en: Object.fromEntries(Object.entries(TABLES.en).filter(([k]) => k.startsWith('pdf.')).map(([k, v]) => [k.slice(4), v])),
};
export const UNITS = {
  h: { nl: TABLES.nl['unit.h'], en: TABLES.en['unit.h'] },
  day: { nl: TABLES.nl['unit.day'], en: TABLES.en['unit.day'] },
  month: { nl: TABLES.nl['unit.month'], en: TABLES.en['unit.month'] },
  unit: { nl: TABLES.nl['unit.unit'], en: TABLES.en['unit.unit'] },
  session: { nl: TABLES.nl['unit.session'], en: TABLES.en['unit.session'] },
  km: { nl: TABLES.nl['unit.km'], en: TABLES.en['unit.km'] },
  kg: { nl: TABLES.nl['unit.kg'], en: TABLES.en['unit.kg'] },
  project: { nl: TABLES.nl['unit.project'], en: TABLES.en['unit.project'] },
};
