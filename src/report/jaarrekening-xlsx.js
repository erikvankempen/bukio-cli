/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across thirty-one jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Jaarrekening XLSX export (exceljs) — balans + W&V sheets for the accountant.
import ExcelJS from 'exceljs';
import { statSync } from 'node:fs';
import { guardFormula } from './export.js';

export async function renderJaarrekeningXlsx(report, { outPath }) {
  const wb = new ExcelJS.Workbook();
  wb.creator = 'bukio-cli';

  const balans = wb.addWorksheet('Balans');
  balans.columns = [
    { header: 'Activa', key: 'activa', width: 34 },
    { header: 'Bedrag', key: 'a', width: 14, style: { numFmt: '#,##0.00' } },
    { header: 'Passiva', key: 'passiva', width: 34 },
    { header: 'Bedrag', key: 'p', width: 14, style: { numFmt: '#,##0.00' } },
  ];
  balans.addRow([guardFormula(`Jaarrekening ${report.year} (${report.model})`), '', '', '']);
  balans.addRow([guardFormula(`${report.company.name} — ${report.as_of}`), '', '', '']);
  balans.addRow([]);
  const rows = Math.max(report.balans.activa.length, report.balans.passiva.length);
  for (let i = 0; i < rows; i += 1) {
    const a = report.balans.activa[i];
    const p = report.balans.passiva[i];
    balans.addRow([guardFormula(a?.label ?? ''), a ? a.total_cents / 100 : '', guardFormula(p?.label ?? ''), p ? p.total_cents / 100 : '']);
  }
  balans.addRow(['Totaal activa', report.balans.total_activa_cents / 100, 'Totaal passiva', report.balans.total_passiva_cents / 100]);
  balans.getRow(balans.rowCount).font = { bold: true };

  if (report.pnl) {
    const pnl = wb.addWorksheet('Winst en verlies');
    pnl.columns = [
      { header: 'Post', key: 'post', width: 40 },
      { header: 'Bedrag', key: 'bedrag', width: 14, style: { numFmt: '#,##0.00' } },
    ];
    pnl.addRow([guardFormula(`Winst- en verliesrekening ${report.year}`), '']);
    pnl.addRow([]);
    for (const l of report.pnl.lines) {
      pnl.addRow([guardFormula(l.label), l.total_cents / 100]);
      for (const s of l.sections ?? []) {
        for (const a of s.accounts ?? []) pnl.addRow([guardFormula(`  ${a.name}`), a.amount_cents / 100]);
      }
    }
    pnl.addRow(['Resultaat na belastingen', report.pnl.resultaat_cents / 100]);
    pnl.getRow(pnl.rowCount).font = { bold: true };
  }

  return wb.xlsx.writeFile(outPath).then(() => ({ path: outPath, bytes: statSync(outPath).size }));
}
