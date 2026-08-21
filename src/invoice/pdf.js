/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across thirty-one jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Invoice PDF via Playwright (headless Chromium) from an HTML template.
// chromium is lazy-loaded inside invoiceToPdf: playwright-core costs ~1.2s to
// import, and keeping it out of the static graph keeps every CLI invocation
// fast (cli/index.js pulls this module in eagerly via cli/invoice.js).
import { formatAmount } from '../core/money.js';
import { computeInvoiceTotals, formatQty, lineDiscountCents } from './index.js';
import { label, unitLabel } from './i18n.js';
import { t } from '../i18n/index.js';

export function pdfError(code, message) {
  const e = new Error(message);
  e.code = code;
  return e;
}

function esc(s) {
  // quotes too: the logo MIME type is interpolated into a src="..." attribute
  return String(s ?? '').replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
}

/** Base64 data-URI of the stored company logo, or null. */
function logoDataUri(company) {
  if (!company?.logo || !company.logo_mime) return null;
  return `data:${esc(company.logo_mime)};base64,${Buffer.from(company.logo).toString('base64')}`;
}

/**
 * Build the invoice HTML. Exported for layout tests; rendered to PDF by
 * invoiceToPdf. Language (nl|en) localizes labels and unit names; the VAT
 * breakdown per rate is shown between subtotal and total; the company logo
 * (if stored) renders in the header.
 */
export function invoiceHtml(db, invoice) {
  const company = db.prepare('SELECT * FROM company WHERE id = 1').get();
  const contact = invoice.contact;
  const isCredit = invoice.invoice_type === 'credit';
  const lang = invoice.language ?? 'en';
  const L = (k) => label(k, lang);

  const totals = computeInvoiceTotals(invoice.lines, invoice.discount_type, invoice.discount_value);
  const logo = logoDataUri(company);

  const rows = invoice.lines.map((l, i) => {
    const disc = lineDiscountCents(l);
    const vatTxt = l.vat_rate_bp ? `${(l.vat_rate_bp / 100).toFixed(1)}%`
      : (l.vat_code === 'R' || l.vat_code === 'RE') ? L('reverseCharge') : '-';
    return `
      <tr>
        <td>${i + 1}</td>
        <td>${esc(l.description)}${disc > 0 ? `<div class="disc">${L('discount')}: −${formatAmount(disc)}</div>` : ''}</td>
        <td class="num">${formatQty(l.quantity)}</td>
        <td>${esc(unitLabel(l.unit, lang) || '')}</td>
        <td class="num">${formatAmount(l.unit_price_cents)}</td>
        <td class="num">${vatTxt}</td>
        <td class="num">${formatAmount(l.amount_cents)}</td>
      </tr>`;
  }).join('');

  const vatRows = totals.breakdown.map((b) => `
      <tr><td>${L('vatOn')} ${(b.rate_bp / 100).toFixed(0)}%</td><td class="num">${formatAmount(b.base_cents)}</td><td class="num">${formatAmount(b.vat_cents)}</td></tr>`).join('');

  const footerTerm = invoice.due_date
    ? L('dueDateTerm').replace('{date}', invoice.due_date)
    : L('defaultTerm');
  const footerPay = L('footerPay')
    .replace('{term}', footerTerm)
    .replace('{iban}', esc(company.iban ?? ''))
    .replace('{name}', esc(company.name));

  return `<!DOCTYPE html>
<html lang="${lang}">
<head>
<meta charset="utf-8">
<style>
  body { font-family: 'DejaVu Sans', sans-serif; font-size: 11px; color: #1a1a1a; margin: 0; }
  .header { display: flex; justify-content: space-between; margin-bottom: 28px; }
  .supplier { display: flex; align-items: flex-start; gap: 12px; }
  .supplier img.logo { max-height: 60px; max-width: 160px; object-fit: contain; }
  .supplier h1 { font-size: 18px; margin: 0 0 4px 0; }
  .supplier p { margin: 1px 0; color: #444; }
  .title { text-align: right; }
  .title h2 { font-size: 22px; margin: 0 0 8px 0; }
  .title p { margin: 2px 0; }
  .parties { display: flex; justify-content: space-between; margin-bottom: 24px; }
  .parties h3 { font-size: 11px; text-transform: uppercase; color: #666; margin: 0 0 6px 0; }
  .parties p { margin: 1px 0; }
  table { width: 100%; border-collapse: collapse; margin-bottom: 20px; }
  th { text-align: left; border-bottom: 2px solid #333; padding: 4px 6px; font-size: 10px; text-transform: uppercase; color: #555; }
  td { border-bottom: 1px solid #ddd; padding: 6px; }
  .disc { font-size: 10px; color: #777; }
  .num { text-align: right; }
  .totals { width: 300px; margin-left: auto; }
  .totals td { border-bottom: none; padding: 3px 6px; }
  .totals .grand td { border-top: 2px solid #333; font-weight: bold; font-size: 13px; }
  .footer { margin-top: 40px; font-size: 10px; color: #666; }
  .footer p { margin: 2px 0; }
</style>
</head>
<body>
  <div class="header">
    <div class="supplier">
      ${logo ? `<img class="logo" src="${logo}" alt="logo">` : ''}
      <div>
        <h1>${esc(company.name)}</h1>
        <p>${esc(company.address)}</p>
        <p>${esc(company.postal_code)} ${esc(company.city)}</p>
        <p>${L('kvk')} ${esc(company.registration_id)} · ${L('btw')} ${esc(company.tax_id)}</p>
      </div>
    </div>
    <div class="title">
      <h2>${isCredit ? L('credit') : L('invoice')}</h2>
      <p><strong>${esc(invoice.invoice_number ?? t('status.draft', {}, lang))}</strong></p>
      <p>${L('date')}: ${invoice.date}</p>
      ${invoice.due_date ? `<p>${L('dueDate')}: ${invoice.due_date}</p>` : ''}
      ${invoice.reference ? `<p>${L('reference')}: ${esc(invoice.reference)}</p>` : ''}
    </div>
  </div>
  <div class="parties">
    <div>
      <h3>${L('billedTo')}</h3>
      <p><strong>${esc(contact.name)}</strong></p>
      <p>${esc(contact.address ?? '')}</p>
      <p>${esc(contact.postal_code ?? '')} ${esc(contact.city ?? '')}</p>
      ${contact.vat_id ? `<p>${L('btw')} ${esc(contact.vat_id)}</p>` : ''}
    </div>
  </div>
  <table>
    <thead><tr>
      <th>#</th><th>${L('description')}</th>
      <th class="num">${L('qty')}</th><th>${L('unit')}</th>
      <th class="num">${L('price')}</th><th class="num">${L('vat')}</th><th class="num">${L('amount')}</th>
    </tr></thead>
    <tbody>${rows}</tbody>
  </table>
  <table class="totals">
    <tr><td>${L('subtotal')}</td><td></td><td class="num">${formatAmount(totals.net_before_cents)}</td></tr>
    ${totals.discount_cents > 0 ? `<tr><td>${L('discount')}</td><td></td><td class="num">−${formatAmount(totals.discount_cents)}</td></tr>` : ''}
    ${vatRows}
    ${totals.vat_cents > 0 ? `<tr><td>${L('vatTotal')}</td><td></td><td class="num">${formatAmount(totals.vat_cents)}</td></tr>` : ''}
    <tr class="grand"><td>${L('total')}${totals.vat_cents > 0 ? ` (${L('inclVat')})` : ''}</td><td></td><td class="num">${formatAmount(invoice.gross_cents)}</td></tr>
  </table>
  <div class="footer">
    <p>${footerPay}</p>
    ${invoice.notes ? `<p>${esc(invoice.notes)}</p>` : ''}
  </div>
</body>
</html>`;
}

/**
 * Render an invoice to PDF bytes via headless Chromium.
 * Throws PDF_UNAVAILABLE if Playwright cannot launch (no browser installed).
 */
export async function invoiceToPdf(db, invoice, { outPath = null } = {}) {
  const html = invoiceHtml(db, invoice);
  let browser;
  try {
    const { chromium } = await import('playwright-core');
    browser = await chromium.launch({ headless: true });
    const page = await browser.newPage();
    await page.setContent(html, { waitUntil: 'load' });
    const pdf = await page.pdf({ format: 'A4', printBackground: true, margin: { top: '2cm', bottom: '2cm', left: '1.8cm', right: '1.8cm' } });
    if (outPath) {
      const { writeFileSync } = await import('node:fs');
      writeFileSync(outPath, pdf);
      return { bytes: pdf.length, path: outPath };
    }
    return { bytes: pdf.length, data: pdf.toString('base64') };
  } catch (err) {
    throw pdfError('PDF_UNAVAILABLE', `could not render the invoice PDF (Playwright/Chromium): ${err.message}`);
  } finally {
    if (browser) await browser.close();
  }
}
