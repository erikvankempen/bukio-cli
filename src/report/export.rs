//! Export helpers — CSV (hand-rolled, zero deps) and XLSX (rust_xlsxwriter).
//! Port of the Node `src/report/export.js`.

use crate::error::{AppError, Result};
use rust_xlsxwriter::{Format, Workbook};

fn csv_cell(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

/// Serialise rows to CSV. Headers + rows, RFC-4180 quoting, trailing newline.
pub fn to_csv(rows: &[Vec<String>], headers: &[String]) -> String {
    let header = headers.iter().map(|h| csv_cell(h)).collect::<Vec<_>>().join(",");
    let body = rows
        .iter()
        .map(|r| r.iter().map(|c| csv_cell(c)).collect::<Vec<_>>().join(","))
        .collect::<Vec<_>>()
        .join("\n");
    format!("{header}\n{body}\n")
}

/// One worksheet spec: headers + already-stringified rows.
pub struct SheetSpec {
    pub name: String,
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

/// Write an XLSX workbook (bold header row, content-sized columns, max 60).
pub fn write_xlsx(file_path: &str, sheets: &[SheetSpec]) -> Result<()> {
    let mut workbook = Workbook::new();
    let bold = Format::new().set_bold();
    for sheet in sheets {
        let ws = workbook.add_worksheet();
        ws.set_name(&sheet.name)
            .map_err(|e| AppError::new("XLSX_ERROR", format!("sheet name '{}': {}", sheet.name, e)))?;
        for (i, h) in sheet.headers.iter().enumerate() {
            ws.write_string_with_format(0, i as u16, h, &bold)
                .map_err(|e| AppError::new("XLSX_ERROR", e.to_string()))?;
        }
        for (r, row) in sheet.rows.iter().enumerate() {
            for (c, cell) in row.iter().enumerate() {
                ws.write_string((r + 1) as u32, c as u16, cell)
                    .map_err(|e| AppError::new("XLSX_ERROR", e.to_string()))?;
            }
        }
        for (i, h) in sheet.headers.iter().enumerate() {
            let mut max_len = h.len();
            for row in &sheet.rows {
                if let Some(cell) = row.get(i) {
                    max_len = max_len.max(cell.len());
                }
            }
            ws.set_column_width(i as u16, (max_len + 2).min(60) as f64)
                .map_err(|e| AppError::new("XLSX_ERROR", e.to_string()))?;
        }
    }
    workbook
        .save(file_path)
        .map_err(|e| AppError::new("XLSX_ERROR", format!("cannot write {}: {}", file_path, e)))?;
    Ok(())
}
