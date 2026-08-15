/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// UBL 2.1 / Peppol BIS 3.0 (EN 16931) invoice XML export.
// Hand-rolled, deterministic, no dependencies. Covers the core BIS 3.0
// structure: seller/buyer parties (with EndpointID electronic addresses,
// BT-34/BT-49), VAT breakdown per rate, monetary totals with allowances,
// lines (with per-line allowances for discounts).
// Full Peppol validation (Schematron) is out of scope — verify via a
// validation service before production use.
import { computeInvoiceTotals, formatQty, lineDiscountCents } from './index.js';
import { resolveProfile } from '../jurisdictions/index.js';

function esc(s) {
  // XML 1.0 valid chars: strip control chars (0x00-0x08, 0x0B, 0x0C,
  // 0x0E-0x1F) that would make the document invalid — Peppol validation
  // rejects them; then escape the five special characters.
  return String(s ?? '')
    .replace(/[\x00-\x08\x0B\x0C\x0E-\x1F]/g, '')
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

function addressBlock(partyName, p, taxId = null, schemeId = '9944', country = 'NL') {
  // the supplier row is snake_case (postal_code); contacts are camelCase —
  // read both so the postal code is never silently dropped
  const postal = p.postalCode ?? p.postal_code ?? '';
  // Peppol BIS 3.0 BT-34: Seller electronic address (cbc:EndpointID, 1..1).
  // For Dutch companies this is the KVK number under scheme 9944 (the Peppol
  // registry code for the Dutch Chamber of Commerce). Emitted when present —
  // the seller's registration id is always set (finalize requires it).
  const endpoint = p.registration_id
    ? `\n        <cbc:EndpointID schemeID="${schemeId}">${esc(p.registration_id)}</cbc:EndpointID>`
    : '';
  return `
        <cac:Party>${endpoint}
          <cac:PartyName><cbc:Name>${esc(partyName)}</cbc:Name></cac:PartyName>
          <cac:PostalAddress>
            <cbc:StreetName>${esc(p.address ?? '')}</cbc:StreetName>
            <cbc:CityName>${esc(p.city ?? '')}</cbc:CityName>
            <cbc:PostalZone>${esc(postal)}</cbc:PostalZone>
            <cac:Country><cbc:IdentificationCode>${esc(p.country ?? country)}</cbc:IdentificationCode></cac:Country>
          </cac:PostalAddress>
          ${taxId ? `<cac:PartyTaxScheme><cbc:CompanyID schemeID="VAT">${esc(taxId)}</cbc:CompanyID><cac:TaxScheme><cbc:ID>VAT</cbc:ID></cac:TaxScheme></cac:PartyTaxScheme>` : ''}
          <cac:PartyLegalEntity>
            <cbc:RegistrationName>${esc(partyName)}</cbc:RegistrationName>
            ${p.registration_id ? `<cbc:CompanyID>${esc(p.registration_id)}</cbc:CompanyID>` : ''}
          </cac:PartyLegalEntity>
        </cac:Party>`;
}

/**
 * Build a Peppol BIS 3.0 UBL Invoice (380) or CreditNote (381) XML document.
 * Discounts are expressed as UBL allowances: per line (cac:AllowanceCharge on
 * the InvoiceLine) and on the total (AllowanceTotalAmount). VAT breakdown
 * bases are the discounted amounts, so the XML reconciles with the books.
 */
// e-invoicing builders keyed by profile.documents.eInvoicing. NL is the
// only format in Phase A ('peppol-bis-3.0'); future markets register theirs.
const EINVOICING_BUILDERS = {
  'peppol-bis-3.0': buildPeppolBis30,
};

export function invoiceToUbl(db, invoice) {
  const profile = resolveProfile(db);
  const { documents } = profile;
  const builder = EINVOICING_BUILDERS[documents.eInvoicing];
  if (!builder) {
    throw Object.assign(new Error(`e-invoicing format '${documents.eInvoicing}' has no builder (registered: ${Object.keys(EINVOICING_BUILDERS).join(', ')})`), { code: 'FORMAT_NOT_SUPPORTED' });
  }
  return builder(db, invoice, profile);
}

function buildPeppolBis30(db, invoice, profile) {
  const company = db.prepare('SELECT * FROM company WHERE id = 1').get();
  // BT-25 (preceding invoice): the credit note's BillingReference must carry
  // the ORIGINAL invoice number, not the buyer reference. Look it up via
  // credit_for_invoice_id — the invoice's own reference field is the buyer
  // reference (klantkenmerk) and may differ.
  const creditBillingRef = invoice.invoice_type === 'credit' && invoice.credit_for_invoice_id
    ? (db.prepare('SELECT invoice_number FROM invoices WHERE id = ?').get(invoice.credit_for_invoice_id)?.invoice_number ?? null)
    : null;
  const contact = invoice.contact;
  const isCredit = invoice.invoice_type === 'credit';
  const typeCode = isCredit ? '381' : '380';
  // the invoices table has no currency column — always EUR (the ledger is EUR)
  const currency = invoice.currency ?? 'EUR';

  // VAT breakdown per rate, on the DISCOUNTED bases (one source of truth).
  // EN 16931 (Peppol BIS 3.0): TaxSubtotal is 1..n MANDATORY and must cover
  // EVERY VAT category used — including zero-VAT categories (AE reverse
  // charge, Z 0%, E exempt) whose TaxAmount is 0.00. Using only the
  // vat>0 `breakdown` produced an empty breakdown (schema violation) for
  // verlegd-only, 0%-only and VAT-less invoices.
  const { groups, net_cents, vat_cents, gross_cents, discount_cents, net_before_cents } =
    computeInvoiceTotals(invoice.lines, invoice.discount_type, invoice.discount_value);

  const categoryOf = (code) => (code === 'R' || code === 'RE') ? 'AE'
    : code === '0' ? 'Z'
      : (code === 'V' || code === 'M') ? 'E'
        : (code ? 'S' : 'E');

  // one TaxSubtotal per (category, rate) — merge groups that share both
  const subtotalMap = new Map();
  for (const g of groups) {
    if (g.discountedNet === 0) continue;
    const cat = categoryOf(g.code);
    const key = `${cat}|${g.rateBp}`;
    if (!subtotalMap.has(key)) subtotalMap.set(key, { cat, rateBp: g.rateBp, baseCents: 0, vatCents: 0 });
    const s = subtotalMap.get(key);
    s.baseCents += g.discountedNet;
    s.vatCents += g.vat;
  }
  const taxSubtotals = [...subtotalMap.values()].map((s) => {
    // AE (reverse charge): emit the code's configured rate when one exists
    // (e.g. 9% verlegd constructiewerk); the default R/RE codes carry 0%
    // (reverse charge has no VAT) and fall back to the PROFILE standard rate
    const percent = s.cat === 'AE' ? (s.rateBp > 0 ? (s.rateBp / 100).toFixed(2) : (profile.tax.standardRateBp / 100).toFixed(2)) : (s.rateBp / 100).toFixed(2);
    return `
      <cac:TaxSubtotal>
        <cbc:TaxableAmount currencyID="${currency}">${moneyAmount(s.baseCents)}</cbc:TaxableAmount>
        <cbc:TaxAmount currencyID="${currency}">${moneyAmount(s.vatCents)}</cbc:TaxAmount>
        <cac:TaxCategory>
          <cbc:ID>${s.cat}</cbc:ID>
          <cbc:Percent>${percent}</cbc:Percent>
          <cac:TaxScheme><cbc:ID>VAT</cbc:ID></cac:TaxScheme>
        </cac:TaxCategory>
      </cac:TaxSubtotal>`;
  }).join('');

  // EN 16931 BT-131: the line net amount is AFTER per-line allowances, and
  // BR-26 checks LineExtensionAmount == qty × PriceAmount − allowance. The
  // gross line amount minus the line discount satisfies it; the document
  // allowance total (BT-107) covers ONLY document-level discounts — line
  // discounts are already folded into BT-106 (sum of line net amounts), so
  // BT-106 − BT-107 == TaxExclusiveAmount still reconciles exactly.
  // per-line discount percentage (R040: Amount = BaseAmount × pct / 100).
  // pct discount: the stored basis points ARE the percentage (1000 bp = 10).
  // amount discount: derived — rounded to 4 decimals (R040 allows ±0.02 slack)
  const linePct = (l) => {
    const disc = lineDiscountCents(l);
    if (disc <= 0) return null;
    if (l.discount_type === 'pct') return l.discount_value / 100;
    if (l.discount_type === 'amount') return (disc / l.amount_cents) * 100;
    return null;
  };
  const lineAllowance = (l) => {
    const disc = lineDiscountCents(l);
    const pct = linePct(l);
    return disc > 0 ? `
      <cac:AllowanceCharge>
        <cbc:ChargeIndicator>false</cbc:ChargeIndicator>
        <cbc:AllowanceChargeReasonCode>95</cbc:AllowanceChargeReasonCode>
        <cbc:Amount currencyID="${currency}">${moneyAmount(disc)}</cbc:Amount>
        <cbc:BaseAmount currencyID="${currency}">${moneyAmount(l.amount_cents)}</cbc:BaseAmount>
        <cbc:MultiplierFactorNumeric>${pct.toFixed(4).replace(/0+$/, '').replace(/\.$/, '')}</cbc:MultiplierFactorNumeric>
      </cac:AllowanceCharge>` : '';
  };

  // line net amount = gross − line discount (BT-131)
  const lineNet = (l) => l.amount_cents - lineDiscountCents(l);

  // document-level allowance total (BT-107): doc discount ONLY
  const allowanceTotal = discount_cents;

  const linesXml = invoice.lines.map((l, i) => {
    // EN 16931 category: AE reverse charge, S standard, Z zero-rated (@0),
    // E exempt (@V vrijgesteld / @M margin / no code)
    const category = (l.vat_code === 'R' || l.vat_code === 'RE') ? 'AE'
      : l.vat_code === '0' ? 'Z'
        : (l.vat_code === 'V' || l.vat_code === 'M') ? 'E'
          : (l.vat_code ? 'S' : 'E');
    // line-level percent must match the TaxSubtotal: AE falls back to the
    // PROFILE standard rate (a hardcoded 21.00 broke reverse-charge lines in
    // every non-NL market — EN 16931 BR-S-09-type inconsistency)
    const percent = (l.vat_code === 'R' || l.vat_code === 'RE') ? (l.vat_rate_bp > 0 ? (l.vat_rate_bp / 100).toFixed(2) : (profile.tax.standardRateBp / 100).toFixed(2)) : (l.vat_rate_bp / 100).toFixed(2);
    const unitCode = UNIT_CODE_MAP[l.unit] ?? 'C62';
    // Peppol BIS 3.0: credit notes use cac:CreditNoteLine +
    // cbc:CreditNoteLineQuantity, invoices use cac:InvoiceLine +
    // cbc:InvoicedQuantity
    const lineTag = isCredit ? 'CreditNoteLine' : 'InvoiceLine';
    const qtyTag = isCredit ? 'CreditNoteLineQuantity' : 'InvoicedQuantity';
    return `
    <cac:${lineTag}>
      <cbc:ID>${i + 1}</cbc:ID>
      <cbc:${qtyTag} unitCode="${unitCode}">${formatQty(l.quantity)}</cbc:${qtyTag}>
      <cbc:LineExtensionAmount currencyID="${currency}">${moneyAmount(lineNet(l))}</cbc:LineExtensionAmount>${lineAllowance(l)}
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
    </cac:${lineTag}>`;
  }).join('');

  const buyerTax = contact.vat_id
    ? `
        <cac:PartyTaxScheme><cbc:CompanyID schemeID="VAT">${esc(contact.vat_id)}</cbc:CompanyID><cac:TaxScheme><cbc:ID>VAT</cbc:ID></cac:TaxScheme></cac:PartyTaxScheme>`
    : '';

  const lineExtensionTotal = invoice.lines.reduce((s, l) => s + lineNet(l), 0);
  const allowanceTotalXml = allowanceTotal > 0
    ? `
    <cbc:AllowanceTotalAmount currencyID="${currency}">${moneyAmount(allowanceTotal)}</cbc:AllowanceTotalAmount>`
    : '';

  const xml = `<?xml version="1.0" encoding="UTF-8"?>
<${isCredit ? 'CreditNote' : 'Invoice'} xmlns="${isCredit ? 'urn:oasis:names:specification:ubl:schema:xsd:CreditNote-2' : 'urn:oasis:names:specification:ubl:schema:xsd:Invoice-2'}"
         xmlns:cac="urn:oasis:names:specification:ubl:schema:xsd:CommonAggregateComponents-2"
         xmlns:cbc="urn:oasis:names:specification:ubl:schema:xsd:CommonBasicComponents-2">
  <cbc:CustomizationID>urn:cen.eu:en16931:2017#compliant#urn:fdc:peppol.eu:2017:poacc:billing:3.0</cbc:CustomizationID>
  <cbc:ProfileID>urn:fdc:peppol.eu:2017:poacc:billing:01:1.0</cbc:ProfileID>
  <cbc:ID>${esc(invoice.invoice_number)}</cbc:ID>
  <cbc:IssueDate>${invoice.date}</cbc:IssueDate>
  ${invoice.due_date ? `<cbc:DueDate>${invoice.due_date}</cbc:DueDate>` : ''}
  <cbc:${isCredit ? 'CreditNoteTypeCode' : 'InvoiceTypeCode'}>${typeCode}</cbc:${isCredit ? 'CreditNoteTypeCode' : 'InvoiceTypeCode'}>
  ${invoice.notes ? `<cbc:Note>${esc(invoice.notes)}</cbc:Note>` : ''}
  <cbc:DocumentCurrencyCode>${currency}</cbc:DocumentCurrencyCode>
  ${invoice.reference ? `<cbc:BuyerReference>${esc(invoice.reference)}</cbc:BuyerReference>` : ''}
  ${isCredit && creditBillingRef ? `
  <cac:BillingReference>
    <cac:InvoiceDocumentReference>
      <cbc:ID>${esc(creditBillingRef)}</cbc:ID>
    </cac:InvoiceDocumentReference>
  </cac:BillingReference>` : ''}
  <cac:AccountingSupplierParty>${addressBlock(company.name, company, company.tax_id, profile.identifiers.peppolSchemeId, profile.meta.country)}</cac:AccountingSupplierParty>
  <cac:AccountingCustomerParty>
    <cac:Party>
      ${contact.kvk ? `<cbc:EndpointID schemeID="${profile.identifiers.peppolSchemeId}">${esc(contact.kvk)}</cbc:EndpointID>` : ''}
      <cac:PartyName><cbc:Name>${esc(contact.name)}</cbc:Name></cac:PartyName>
      <cac:PostalAddress>
        <cbc:StreetName>${esc(contact.address ?? '')}</cbc:StreetName>
        <cbc:CityName>${esc(contact.city ?? '')}</cbc:CityName>
        <cbc:PostalZone>${esc(contact.postal_code ?? '')}</cbc:PostalZone>
        <cac:Country><cbc:IdentificationCode>${esc(contact.country ?? 'NL')}</cbc:IdentificationCode></cac:Country>
      </cac:PostalAddress>${buyerTax}
      <cac:PartyLegalEntity>
        <cbc:RegistrationName>${esc(contact.name)}</cbc:RegistrationName>
        ${contact.kvk ? `<cbc:CompanyID>${esc(contact.kvk)}</cbc:CompanyID>` : ''}
      </cac:PartyLegalEntity>
    </cac:Party>
  </cac:AccountingCustomerParty>
  ${company.iban ? `
  <cac:PaymentMeans>
    <cbc:PaymentMeansCode>30</cbc:PaymentMeansCode>
    <cac:PayeeFinancialAccount><cbc:ID>${esc(company.iban)}</cbc:ID></cac:PayeeFinancialAccount>
  </cac:PaymentMeans>` : ''}
  ${invoice.due_date ? `<cac:PaymentTerms><cbc:PaymentDueDate>${invoice.due_date}</cbc:PaymentDueDate></cac:PaymentTerms>` : ''}
  ${discount_cents > 0 ? `
  <cac:AllowanceCharge>
    <cbc:ChargeIndicator>false</cbc:ChargeIndicator>
    <cbc:AllowanceChargeReasonCode>95</cbc:AllowanceChargeReasonCode>
    <cbc:Amount currencyID="${currency}">${moneyAmount(discount_cents)}</cbc:Amount>
    <cbc:BaseAmount currencyID="${currency}">${moneyAmount(net_before_cents)}</cbc:BaseAmount>
    <cbc:MultiplierFactorNumeric>${(discount_cents / net_before_cents * 100).toFixed(4).replace(/0+$/, '').replace(/\.$/, '')}</cbc:MultiplierFactorNumeric>
  </cac:AllowanceCharge>` : ''}
  <cac:TaxTotal>
    <cbc:TaxAmount currencyID="${currency}">${moneyAmount(vat_cents)}</cbc:TaxAmount>${taxSubtotals}
  </cac:TaxTotal>
  <cac:LegalMonetaryTotal>
    <cbc:LineExtensionAmount currencyID="${currency}">${moneyAmount(lineExtensionTotal)}</cbc:LineExtensionAmount>
    <cbc:TaxExclusiveAmount currencyID="${currency}">${moneyAmount(net_cents)}</cbc:TaxExclusiveAmount>
    <cbc:TaxInclusiveAmount currencyID="${currency}">${moneyAmount(gross_cents)}</cbc:TaxInclusiveAmount>${allowanceTotalXml}
    <cbc:PayableAmount currencyID="${currency}">${moneyAmount(gross_cents)}</cbc:PayableAmount>
  </cac:LegalMonetaryTotal>${linesXml}
</${isCredit ? 'CreditNote' : 'Invoice'}>`;
  return xml;
}
