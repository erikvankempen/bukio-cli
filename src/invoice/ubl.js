// UBL 2.1 / Peppol BIS 3.0 (EN 16931) invoice XML export.
// Hand-rolled, deterministic, no dependencies. Covers the core BIS 3.0
// structure: seller/buyer parties, VAT breakdown, monetary totals, lines.
// Full Peppol validation (Schematron) is out of scope — verify via a
// validation service before production use.

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
 */
export function invoiceToUbl(db, invoice) {
  const company = db.prepare('SELECT * FROM company WHERE id = 1').get();
  const contact = invoice.contact;
  const isCredit = invoice.invoice_type === 'credit';
  const typeCode = isCredit ? '381' : '380';
  // the invoices table has no currency column — always EUR (the ledger is EUR)
  const currency = invoice.currency ?? 'EUR';

  // VAT breakdown per rate (exact per-line sums)
  const byRate = new Map();
  for (const l of invoice.lines) {
    const key = `${l.vat_code ?? 'none'}|${l.vat_rate_bp}`;
    if (!byRate.has(key)) byRate.set(key, { code: l.vat_code, rateBp: l.vat_rate_bp, taxable: 0, tax: 0 });
    byRate.get(key).taxable += l.amount_cents;
    byRate.get(key).tax += l.vat_amount_cents;
  }

  const taxSubtotals = [...byRate.values()]
    .filter((g) => g.tax !== 0)
    .map((g) => {
      const category = (g.code === 'R' || g.code === 'RE') ? 'AE' : 'S';
      const percent = g.code === 'R' || g.code === 'RE' ? '21.00' : (g.rateBp / 100).toFixed(2);
      return `
      <cac:TaxSubtotal>
        <cbc:TaxableAmount currencyID="${currency}">${moneyAmount(g.taxable)}</cbc:TaxableAmount>
        <cbc:TaxAmount currencyID="${currency}">${moneyAmount(g.tax)}</cbc:TaxAmount>
        <cac:TaxCategory>
          <cbc:ID>${category}</cbc:ID>
          <cbc:Percent>${percent}</cbc:Percent>
          <cac:TaxScheme><cbc:ID>VAT</cbc:ID></cac:TaxScheme>
        </cac:TaxCategory>
      </cac:TaxSubtotal>`;
    }).join('');

  const linesXml = invoice.lines.map((l, i) => {
    const category = (l.vat_code === 'R' || l.vat_code === 'RE') ? 'AE' : (l.vat_code ? 'S' : 'E');
    const percent = (l.vat_code === 'R' || l.vat_code === 'RE') ? '21.00' : (l.vat_rate_bp / 100).toFixed(2);
    return `
    <cac:InvoiceLine>
      <cbc:ID>${i + 1}</cbc:ID>
      <cbc:InvoicedQuantity unitCode="C62">${l.quantity}</cbc:InvoicedQuantity>
      <cbc:LineExtensionAmount currencyID="${currency}">${moneyAmount(l.amount_cents)}</cbc:LineExtensionAmount>
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
    <cbc:TaxAmount currencyID="${currency}">${moneyAmount(invoice.vat_cents)}</cbc:TaxAmount>${taxSubtotals}
  </cac:TaxTotal>
  <cac:LegalMonetaryTotal>
    <cbc:LineExtensionAmount currencyID="${currency}">${moneyAmount(invoice.net_cents)}</cbc:LineExtensionAmount>
    <cbc:TaxExclusiveAmount currencyID="${currency}">${moneyAmount(invoice.net_cents)}</cbc:TaxExclusiveAmount>
    <cbc:TaxInclusiveAmount currencyID="${currency}">${moneyAmount(invoice.gross_cents)}</cbc:TaxInclusiveAmount>
    <cbc:PayableAmount currencyID="${currency}">${moneyAmount(invoice.gross_cents)}</cbc:PayableAmount>
  </cac:LegalMonetaryTotal>${linesXml}
</Invoice>`;
  return xml;
}
