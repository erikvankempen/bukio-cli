/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across thirty-one jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Jaarrekening PDF — the KVK deposit package in the statutory Dutch layout,
// rendered via headless Chromium (same engine as the invoice PDF).
// chromium is lazy-loaded inside jaarrekeningToPdf (playwright-core costs
// ~1.2s to import; cli/index.js pulls this module in eagerly via cli/year-end.js).
import { formatAmount } from '../core/money.js';
import { pdfError } from '../invoice/pdf.js';

function esc(s) {
  // quotes too (like every other esc() in the codebase): user data may end
  // up inside an HTML attribute — a raw " would break out of it
  return String(s ?? '').replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
}

function sideLines(lines) {
  const out = [];
  for (const g of lines) {
    if (!g.sections?.length) continue;
    out.push(`<tr class="group"><td>${esc(g.label)}</td><td class="num">${formatAmount(g.total_cents)}</td></tr>`);
    for (const s of g.sections) {
      for (const a of s.accounts ?? []) {
        out.push(`<tr class="detail"><td class="indent">${esc(a.name)}</td><td class="num">${formatAmount(a.amount_cents)}</td></tr>`);
      }
    }
  }
  return out.join('');
}

function pnlLines(report) {
  if (!report.pnl) return '';
  return report.pnl.lines.map((l) => `
      <tr class="group"><td>${esc(l.label)}</td><td class="num">${formatAmount(l.total_cents)}</td></tr>
      ${l.sections.flatMap((s) => s.accounts.map((a) => `<tr class="detail"><td class="indent">${esc(a.name)}</td><td class="num">${formatAmount(a.amount_cents)}</td></tr>`)).join('')}
    `).join('');
}

/**
 * Year result for the micro model (no P&L in the report object): the
 * 'Onverdeeld resultaat' line the balans engine folds into Eigen vermogen.
 * The BEIV.05 total is capital + ALL prior results — using it as the year
 * result mislabels e.g. €18k capital + €12k result as a €30k 'resultaat'.
 */
function microResultCents(report) {
  for (const s of report.balans?.passiva ?? []) {
    if (s.taxonomy_code === null && s.label === 'Onverdeeld resultaat') return s.total_cents;
  }
  return 0;
}

export function jaarrekeningHtml(report) {
  const c = report.company;
  return `<!DOCTYPE html>
<html lang="nl">
<head>
<meta charset="utf-8">
<style>
  body { font-family: 'DejaVu Sans', sans-serif; font-size: 10.5px; color: #1a1a1a; margin: 0; }
  h1 { font-size: 17px; margin: 0 0 2px 0; }
  .meta { color: #444; margin-bottom: 18px; }
  .meta p { margin: 1px 0; }
  h2 { font-size: 12px; text-transform: uppercase; color: #555; border-bottom: 1px solid #999; padding-bottom: 3px; margin: 22px 0 8px 0; }
  table { width: 100%; border-collapse: collapse; }
  td { padding: 2px 6px; }
  .num { text-align: right; }
  tr.group td { font-weight: bold; border-top: 1px solid #ccc; padding-top: 5px; }
  tr.detail td { color: #333; }
  td.indent { padding-left: 18px; }
  tr.total td { font-weight: bold; border-top: 2px solid #333; }
  .footer { margin-top: 36px; font-size: 9.5px; color: #666; }
  .footer .sign { margin-top: 26px; }
</style>
</head>
<body>
  <h1>Jaarrekening ${report.year}</h1>
  <div class="meta">
    <p><strong>${esc(c.name)}</strong></p>
    <p>${esc(c.address ?? '')} ${esc(c.postal_code ?? '')} ${esc(c.city ?? '')}</p>
    <p>KvK ${esc(c.kvk ?? '')} · BTW ${esc(c.btw_id ?? '')}</p>
    <p>Model: ${report.model === 'micro' ? 'micro (art. 2:395a BW)' : 'klein (art. 2:396 BW)'} · peildatum ${report.as_of}</p>
  </div>

  <h2>Balans per ${report.as_of}</h2>
  <table>
    <tr><td style="width:50%"><table><tr class="group"><td>Activa</td><td class="num"></td></tr>${sideLines(report.balans.activa)}<tr class="total"><td>Totaal activa</td><td class="num">${formatAmount(report.balans.total_activa_cents)}</td></tr></table></td>
        <td><table><tr class="group"><td>Passiva</td><td class="num"></td></tr>${sideLines(report.balans.passiva)}<tr class="total"><td>Totaal passiva</td><td class="num">${formatAmount(report.balans.total_passiva_cents)}</td></tr></table></td></tr>
  </table>

  ${report.pnl ? `<h2>Winst- en verliesrekening ${report.year}</h2>
  <table style="width:60%">
    ${pnlLines(report)}
    <tr class="total"><td>Resultaat na belastingen</td><td class="num">${report.pnl.resultaat}</td></tr>
  </table>` : `<h2>Resultaat ${report.year}</h2>
  <table style="width:60%"><tr class="total"><td>Resultaat na belastingen</td><td class="num">${formatAmount(microResultCents(report))}</td></tr></table>`}

  <div class="footer">
    <p>Opgesteld op basis van de administratie. Jaarrekeningmodel ${report.model === 'micro' ? 'micro' : 'klein'} conform Titel 9 Boek 2 BW.</p>
    <p class="sign">Vastgesteld door het bestuur te ${esc(c.city ?? '')} op \u005f\u005f\u005f\u005f\u005f\u005f\u005f\u005f\u005f\u005f</p>
  </div>
</body>
</html>`;
}

export async function jaarrekeningToPdf(report, { outPath = null } = {}) {
  const html = jaarrekeningHtml(report);
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
    throw pdfError('PDF_UNAVAILABLE', `could not render the jaarrekening PDF (Playwright/Chromium): ${err.message}`);
  } finally {
    if (browser) await browser.close();
  }
}
