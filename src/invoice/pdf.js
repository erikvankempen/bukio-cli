// Invoice PDF via Playwright (headless Chromium) from an HTML template.
// Uses the locally installed Playwright browsers (see /root/.cache/ms-playwright).
// chromium is lazy-loaded inside invoiceToPdf: playwright-core costs ~1.2s to
// import, and keeping it out of the static graph keeps every CLI invocation
// fast (cli/index.js pulls this module in eagerly via cli/invoice.js).
import { formatAmount } from '../core/money.js';

export function pdfError(code, message) {
  const e = new Error(message);
  e.code = code;
  return e;
}

function esc(s) {
  return String(s ?? '').replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

function invoiceHtml(db, invoice) {
  const company = db.prepare('SELECT * FROM company WHERE id = 1').get();
  const contact = invoice.contact;
  const isCredit = invoice.invoice_type === 'credit';
  const vatOn = invoice.vat_cents > 0;

  const rows = invoice.lines.map((l, i) => `
      <tr>
        <td>${i + 1}</td>
        <td>${esc(l.description)}</td>
        <td class="num">${l.quantity}</td>
        <td class="num">${formatAmount(l.unit_price_cents)}</td>
        <td class="num">${l.vat_rate_bp ? (l.vat_rate_bp / 100).toFixed(1) + '%' : (l.vat_code === 'R' || l.vat_code === 'RE' ? 'verlegd' : '-')}</td>
        <td class="num">${formatAmount(l.amount_cents)}</td>
      </tr>`).join('');

  return `<!DOCTYPE html>
<html lang="nl">
<head>
<meta charset="utf-8">
<style>
  body { font-family: 'DejaVu Sans', sans-serif; font-size: 11px; color: #1a1a1a; margin: 0; }
  .header { display: flex; justify-content: space-between; margin-bottom: 28px; }
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
  .num { text-align: right; }
  .totals { width: 260px; margin-left: auto; }
  .totals td { border-bottom: none; padding: 3px 6px; }
  .totals .grand td { border-top: 2px solid #333; font-weight: bold; font-size: 13px; }
  .footer { margin-top: 40px; font-size: 10px; color: #666; }
  .footer p { margin: 2px 0; }
</style>
</head>
<body>
  <div class="header">
    <div class="supplier">
      <h1>${esc(company.name)}</h1>
      <p>${esc(company.address)}</p>
      <p>${esc(company.postal_code)} ${esc(company.city)}</p>
      <p>KvK ${esc(company.kvk)} · BTW ${esc(company.btw_id)}</p>
    </div>
    <div class="title">
      <h2>${isCredit ? 'CREDITFACTUUR' : 'FACTUUR'}</h2>
      <p><strong>${esc(invoice.invoice_number ?? 'concept')}</strong></p>
      <p>Datum: ${invoice.date}</p>
      ${invoice.due_date ? `<p>Vervaldatum: ${invoice.due_date}</p>` : ''}
      ${invoice.reference ? `<p>Referentie: ${esc(invoice.reference)}</p>` : ''}
    </div>
  </div>
  <div class="parties">
    <div>
      <h3>Factuur aan</h3>
      <p><strong>${esc(contact.name)}</strong></p>
      <p>${esc(contact.address ?? '')}</p>
      <p>${esc(contact.postal_code ?? '')} ${esc(contact.city ?? '')}</p>
      ${contact.vat_id ? `<p>BTW ${esc(contact.vat_id)}</p>` : ''}
    </div>
  </div>
  <table>
    <thead><tr><th>#</th><th>Omschrijving</th><th class="num">Aantal</th><th class="num">Prijs</th><th class="num">Btw</th><th class="num">Bedrag</th></tr></thead>
    <tbody>${rows}</tbody>
  </table>
  <table class="totals">
    <tr><td>Subtotaal excl. btw</td><td class="num">${formatAmount(invoice.net_cents)}</td></tr>
    ${vatOn ? `<tr><td>Btw</td><td class="num">${formatAmount(invoice.vat_cents)}</td></tr>` : ''}
    <tr class="grand"><td>Totaal</td><td class="num">${formatAmount(invoice.gross_cents)}</td></tr>
  </table>
  <div class="footer">
    <p>Gelieve het bedrag binnen ${invoice.due_date ? `${invoice.due_date}` : 'de gestelde termijn'} over te maken op IBAN ${esc(company.iban ?? '')} t.n.v. ${esc(company.name)}.</p>
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
