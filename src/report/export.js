// Export helpers — CSV (hand-rolled, zero deps) and XLSX (exceljs).
import ExcelJS from 'exceljs';

/**
 * Formula-injection guard: a cell beginning with '=', '+' or '@' (after
 * leading whitespace) is treated as a FORMULA when the file is opened in a
 * spreadsheet (e.g. '=HYPERLINK(...)' in a description or contact name).
 * Prefix those cells with a single quote — Excel then shows them as text.
 * Negative amounts ('-12.34') are left untouched: they are valid numbers.
 */
function guardFormula(s) {
  return /^\s*[=+@]/.test(s) ? `'${s}` : s;
}

function csvCell(value) {
  const s = guardFormula(value == null ? '' : String(value));
  return /[",\n\r]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s;
}

/** Serialise rows to CSV. columns: [{ key, label }] */
export function toCsv(rows, columns) {
  const header = columns.map((c) => csvCell(c.label)).join(',');
  const body = rows.map((r) => columns.map((c) => csvCell(r[c.key])).join(','));
  return [header, ...body].join('\n') + '\n';
}

/** Write an XLSX workbook. sheets: [{ name, columns: [{ header, key }], rows }] */
export async function writeXlsx(filePath, sheets) {
  const wb = new ExcelJS.Workbook();
  for (const sheet of sheets) {
    const ws = wb.addWorksheet(sheet.name);
    ws.addRow(sheet.columns.map((c) => c.header));
    ws.getRow(1).font = { bold: true };
    for (const row of sheet.rows) {
      // exceljs auto-converts strings starting with '=' into formulas —
      // guard them the same way as the CSV writer
      ws.addRow(sheet.columns.map((c) => {
        const v = row[c.key];
        return typeof v === 'string' ? guardFormula(v) : v;
      }));
    }
    ws.columns.forEach((col, i) => {
      const max = Math.max(
        col.values.reduce((m, v) => Math.max(m, String(v ?? '').length), 0),
        1,
      );
      col.width = Math.min(max + 2, 60);
      void i;
    });
  }
  await wb.xlsx.writeFile(filePath);
  return filePath;
}
