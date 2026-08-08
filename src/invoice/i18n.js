/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Invoice presentation strings (labels + quantity units), Dutch default,
// English optional. Line descriptions are never auto-translated — only
// structural labels and unit codes localize.

export const UNIT_CODES = ['h', 'day', 'month', 'unit', 'session', 'km', 'kg', 'project'];

export const UNITS = {
  h:       { nl: 'uur',    en: 'h' },
  day:     { nl: 'dag',    en: 'day' },
  month:   { nl: 'maand',  en: 'month' },
  unit:    { nl: 'stuks',  en: 'unit' },
  session: { nl: 'sessie', en: 'session' },
  km:      { nl: 'km',     en: 'km' },
  kg:      { nl: 'kg',     en: 'kg' },
  project: { nl: 'project', en: 'project' },
};

export function unitLabel(code, language = 'nl') {
  return UNITS[code]?.[language] ?? UNITS[code]?.nl ?? code ?? '';
}

export const LABELS = {
  nl: {
    invoice: 'FACTUUR', credit: 'CREDITFACTUUR',
    billedTo: 'Factuur aan', date: 'Datum', dueDate: 'Vervaldatum',
    reference: 'Referentie', kvk: 'KvK', btw: 'BTW',
    description: 'Omschrijving', qty: 'Aantal', unit: 'Eenheid', price: 'Prijs',
    vat: 'Btw', discount: 'Korting', amount: 'Bedrag',
    subtotal: 'Subtotaal excl. btw', vatOn: 'Btw over', vatTotal: 'Totaal btw',
    total: 'Totaal', inclVat: 'incl. btw',
    footerPay: 'Gelieve het bedrag binnen {term} over te maken op IBAN {iban} t.n.v. {name}.',
    dueDateTerm: '{date}', defaultTerm: 'de gestelde termijn',
  },
  en: {
    invoice: 'INVOICE', credit: 'CREDIT NOTE',
    billedTo: 'Billed to', date: 'Date', dueDate: 'Due date',
    reference: 'Reference', kvk: 'CoC', btw: 'VAT',
    description: 'Description', qty: 'Qty', unit: 'Unit', price: 'Price',
    vat: 'VAT', discount: 'Discount', amount: 'Amount',
    subtotal: 'Subtotal excl. VAT', vatOn: 'VAT on', vatTotal: 'Total VAT',
    total: 'Total', inclVat: 'incl. VAT',
    footerPay: 'Please transfer the amount within {term} to IBAN {iban} for the account of {name}.',
    dueDateTerm: '{date}', defaultTerm: 'the agreed term',
  },
};

export function label(key, language = 'nl') {
  return LABELS[language]?.[key] ?? LABELS.nl[key] ?? key;
}
