/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// UBL 2.1 / Peppol BIS 3.0 (EN 16931) invoice XML export.
// Hand-rolled, deterministic, no dependencies. Covers the core BIS 3.0
// structure: seller/buyer parties, VAT breakdown per rate, monetary totals
// with allowances, lines (with per-line allowances for discounts).
// Full Peppol validation (Schematron) is out of scope — verify via a
// validation service before production use.
import { computeInvoiceTotals, formatQty } from './index.js';
import { unitLabel } from './i18n.js';

function esc(s) {
  return String(s ?? '')
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}

function moneyAmount(cents, currency = 'EUR') {
  return (cents / 100).toFixed(2);
}

// UN/ECE Rec 20 unit codes for the quantity units we support (C62 = unit/one)
const UNIT_CODE_MAP = {
  h: 'HUR', day: 'DAY', month: 'MON', unit: 'C62',
  session: 'C62', km: 'KMT', kg: 'KGM', project: 'C62',
};

function addressBlock(partyName, p) {
  // the supplier row is snake_case (postal_code); contacts are camelCase —
  // read both so the postal code is never silently dropped
  const postal = p.postalCode ?? p.postal_code ?? '';
  return `
        <cac:Party>
          <cac:PartyName><cbc:Name>${esc(partyName)}</cbc:Name></cac:PartyName>
          <cac:PostalAddress>
            <cbc:StreetName>${esc(p.address ?? '')}</cbc:StreetName>
            <cbc:CityName>${esc(p.city ?? '')}</cbc:CityName>
            <cbc:PostalZone>${esc(postal)}</cbc:PostalZone>
            <cac:Country><cbc:IdentificationCode>${esc(p.country ?? 'NL')}</cbc:IdentificationCode></cac:Country>
          </cac:PostalAddress>
        </cac:Party>`;
}

/**
 * Build a Peppol BIS 3.0 UBL Invoice (380) or CreditNote (381) XML document.
 * Discounts are expressed as UBL allowances: per line (cac:AllowanceCharge on
 * the InvoiceLine) and on the total (AllowanceTotalAmount). VAT breakdown
 * bases are the discounted amounts, so the XML reconciles with the books.
 */
export function invoiceToUbl(db, invoice) {
  const company = db.prepare('SELECT * FROM company WHERE id = 1').get();
  const contact = invoice.contact;
  const isCredit = invoice.invoice_type === 'credit';
  const typeCode = isCredit ? '381' : '380';
  // the invoices table has no currency column — always EUR (the ledger is EUR)
  const currency = invoice.currency ?? 'EUR';
  const language = invoice.language ?? 'nl';

  // VAT breakdown per rate, on the DISCOUNTED bases (one source of truth)
  const { breakdown, net_cents, vat_cents, gross_cents, discount_cents } =
    computeInvoiceTotals(invoice.lines, invoice.discount_type, invoice.discount_value);

  const taxSubtotals = breakdown.map((g) => {
    const code = g.rate_bp === 2100 ? 'S' : 'S'; // standard rate category; R/RE never carry VAT
    return `
      <cac:TaxSubtotal>
        <cbc:TaxableAmount currencyID="${currency}">${moneyAmount(g.base_cents)}</cbc:TaxableAmount>
        <cbc:TaxAmount currencyID="${currency}">${moneyAmount(g.vat_cents)}</cbc:TaxAmount>
        <cac:TaxCategory>
          <cbc:ID>${code}</cbc:ID>
          <cbc:Percent>${(g.rate_bp / 100).toFixed(2)}</cbc:Percent>
          <cac:TaxScheme><cbc:ID>VAT</cbc:ID></cac:TaxScheme>
        </cac:TaxCategory>
      </cac:TaxSubtotal>`;
  }).join('');

  const lineAllowance = (l) => {
    const disc = l.discount_type === 'pct'
      ? Math.round(l.amount_cents * l.discount_value / 10000)
      : l.discount_type === 'amount' ? Math.min(l.discount_value, l.amount_cents) : 0;
    return disc > 0 ? `
      <cac:AllowanceCharge>
        <cbc:ChargeIndicator>false</cbc:ChargeIndicator>
        <cbc:AllowanceChargeReasonCode>95</cbc:AllowanceChargeReasonCode>
        <cbc:Amount currencyID="${currency}">${moneyAmount(disc)}</cbc:Amount>
        <cbc:BaseAmount currencyID="${currency}">${moneyAmount(l.amount_cents)}</cbc:BaseAmount>
      </cac:AllowanceCharge>` : '';
  };

  const linesXml = invoice.lines.map((l, i) => {
    const category = (l.vat_code === 'R' || l.vat_code === 'RE') ? 'AE'
      : (l.vat_code ? 'S' : (l.vat_rate_bp === 0 && l.vat_code ? 'Z' : 'E'));
    const percent = (l.vat_code === 'R' || l.vat_code === 'RE') ? '21.00' : (l.vat_rate_bp / 100).toFixed(2);
    const unitCode = UNIT_CODE_MAP[l.unit] ?? 'C62';
    return `
    <cac:InvoiceLine>
      <cbc:ID>${i + 1}</cbc:ID>
      <cbc:InvoicedQuantity unitCode="${unitCode}">${formatQty(l.quantity)}</cbc:InvoicedQuantity>
      <cbc:LineExtensionAmount currencyID="${currency}">${moneyAmount(l.amount_cents)}</cbc:LineExtensionAmount>${lineAllowance(l)}
      <cac:Item>
        <cbc:Name>${esc(l.description)}</cbc:Name>
        <cac:ClassifiedTaxCategory>
          <cbc:ID>${category}</cbc:ID>
          <cbc:Percent>${percent}</cbc:Percent>
          <cac:TaxScheme><cbc:ID>VAT</cbc:ID></cac:TaxScheme>
        </cac:ClassifiedTaxCategory>
      </cac:Item>
      <cac:Price>
        <cbc:PriceAmount currencyID="${currency}">${moneyAmount(l.unit_price_cents)}</cbc:PriceAmount>
      </cac:Price>
    </cac:InvoiceLine>`;
  }).join('');

  const buyerTax = contact.vat_id
    ? `
        <cac:PartyTaxScheme><cbc:CompanyID schemeID="VAT">${esc(contact.vat_id)}</cbc:CompanyID><cac:TaxScheme><cbc:ID>VAT</cbc:ID></cac:TaxScheme></cac:PartyTaxScheme>`
    : '';

  const lineExtensionTotal = invoice.lines.reduce((s, l) => s + l.amount_cents, 0);
  const allowanceTotal = discount_cents > 0
    ? `
    <cbc:AllowanceTotalAmount currencyID="${currency}">${moneyAmount(discount_cents)}</cbc:AllowanceTotalAmount>`
    : '';

  const xml = `<?xml version="1.0" encoding="UTF-8"?>
<Invoice xmlns="urn:oasis:names:specification:ubl:schema:xsd:Invoice-2"
         xmlns:cac="urn:oasis:names:specification:ubl:schema:xsd:CommonAggregateComponents-2"
         xmlns:cbc="urn:oasis:names:specification:ubl:schema:xsd:CommonBasicComponents-2">
  <cbc:CustomizationID>urn:cen.eu:en16931:2017#compliant#urn:fdc:peppol.eu:2017:poacc:billing:3.0</cbc:CustomizationID>
  <cbc:ProfileID>urn:fdc:peppol.eu:2017:poacc:billing:01:1.0</cbc:ProfileID>
  <cbc:ID>${esc(invoice.invoice_number)}</cbc:ID>
  <cbc:IssueDate>${invoice.date}</cbc:IssueDate>
  ${invoice.due_date ? `<cbc:DueDate>${invoice.due_date}</cbc:DueDate>` : ''}
  <cbc:InvoiceTypeCode>${typeCode}</cbc:InvoiceTypeCode>
  <cbc:LanguageID>${language === 'en' ? 'en' : 'nl-NL'}</cbc:LanguageID>
  ${invoice.notes ? `<cbc:Note>${esc(invoice.notes)}</cbc:Note>` : ''}
  <cac:AccountingSupplierParty>${addressBlock(company.name, company)}</cac:AccountingSupplierParty>
  <cac:AccountingCustomerParty>
    <cac:Party>
      <cac:PartyName><cbc:Name>${esc(contact.name)}</cbc:Name></cac:PartyName>
      <cac:PostalAddress>
        <cbc:StreetName>${esc(contact.address ?? '')}</cbc:StreetName>
        <cbc:CityName>${esc(contact.city ?? '')}</cbc:CityName>
        <cbc:PostalZone>${esc(contact.postal_code ?? '')}</cbc:PostalZone>
        <cac:Country><cbc:IdentificationCode>${esc(contact.country ?? 'NL')}</cbc:IdentificationCode></cac:Country>
      </cac:PostalAddress>${buyerTax}
    </cac:Party>
  </cac:AccountingCustomerParty>
  <cac:PaymentMeans>
    <cbc:PaymentMeansCode>30</cbc:PaymentMeansCode>
    <cac:PayeeFinancialAccount><cbc:ID>${esc(company.iban ?? '')}</cbc:ID></cac:PayeeFinancialAccount>
  </cac:PaymentMeans>
  ${invoice.due_date ? `<cac:PaymentTerms><cbc:PaymentDueDate>${invoice.due_date}</cbc:PaymentDueDate></cac:PaymentTerms>` : ''}
  <cac:TaxTotal>
    <cbc:TaxAmount currencyID="${currency}">${moneyAmount(vat_cents)}</cbc:TaxAmount>${taxSubtotals}
  </cac:TaxTotal>
  <cac:LegalMonetaryTotal>
    <cbc:LineExtensionAmount currencyID="${currency}">${moneyAmount(lineExtensionTotal)}</cbc:LineExtensionAmount>${allowanceTotal}
    <cbc:TaxExclusiveAmount currencyID="${currency}">${moneyAmount(net_cents)}</cbc:TaxExclusiveAmount>
    <cbc:TaxInclusiveAmount currencyID="${currency}">${moneyAmount(gross_cents)}</cbc:TaxInclusiveAmount>
    <cbc:PayableAmount currencyID="${currency}">${moneyAmount(gross_cents)}</cbc:PayableAmount>
  </cac:LegalMonetaryTotal>${linesXml}
</Invoice>`;
  return xml;
}
