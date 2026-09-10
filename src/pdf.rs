//! Jaarrekening HTML/PDF — the KVK deposit package in the statutory Dutch
//! layout, mirroring src/report/jaarrekening-pdf.js.
//!
//! NATIVE: the PDF is written by the small writer below (PDF 1.4, the 14
//! standard fonts, WinAnsiEncoding). No browser, no dependency — the JS uses
//! playwright/Chromium for this, which is why the port could not do it at all.
//!
//! ponytail: text-only layout (no images, no embedded fonts, no table borders
//! beyond rules). If the deposit package ever needs the HTML's styling verbatim,
//! swap the writer for a full PDF crate rather than growing this one.
use crate::money::{format_amount, BukioError, Result};
use serde_json::Value;

// ── minimal PDF writer ──────────────────────────────────────────────────────

/// A4 in points, with the HTML's 1.8cm/2cm margins (1cm = 28.35pt).
const PAGE_W: f64 = 595.28;
const PAGE_H: f64 = 841.89;
const MARGIN_X: f64 = 51.0;
const MARGIN_TOP: f64 = 56.7;
const MARGIN_BOTTOM: f64 = 56.7;

struct Pdf {
    /// content streams, one per page, as raw bytes: the stream is
    /// WinAnsi/Latin-1, so a char like ë must be ONE byte (0xEB) — pushing it
    /// into a String wrote the two UTF-8 bytes and the PDF showed "MateriÃ«le"
    pages: Vec<Vec<u8>>,
    cur: Vec<u8>,
    y: f64,
}

impl Pdf {
    fn new() -> Self {
        Pdf {
            pages: Vec::new(),
            cur: Vec::new(),
            y: PAGE_H - MARGIN_TOP,
        }
    }

    fn escape(text: &str) -> Vec<u8> {
        let mut out: Vec<u8> = Vec::new();
        for ch in text.chars() {
            // WinAnsi == Latin-1 for the range we use; anything else degrades to
            // '?' rather than corrupting the byte stream
            let byte = if (ch as u32) <= 0xFF { ch as u8 } else { b'?' };
            match byte {
                b'(' => out.extend_from_slice(b"\\("),
                b')' => out.extend_from_slice(b"\\)"),
                b'\\' => out.extend_from_slice(b"\\\\"),
                b'\n' => out.push(b' '),
                _ => out.push(byte),
            }
        }
        out
    }

    /// right-aligned text ending at `x_right`
    fn text_right(&mut self, x_right: f64, y: f64, size: f64, bold: bool, text: &str) {
        let w = text.chars().count() as f64 * size * 0.5;
        self.text_at(x_right - w, y, size, bold, text);
    }

    /// a row of columns: (x_from_left, width, right_align, text)
    fn cols(&mut self, size: f64, bold: bool, cols: &[(f64, f64, bool, String)]) {
        self.need(size + 4.0);
        self.y -= size + 3.0;
        let y = self.y;
        for (x, w, right, text) in cols {
            if text.is_empty() {
                continue;
            }
            if *right {
                self.text_right(MARGIN_X + x + w, y, size, bold, text);
            } else {
                self.text_at(MARGIN_X + x, y, size, bold, text);
            }
        }
    }

    fn text_at(&mut self, x: f64, y: f64, size: f64, bold: bool, text: &str) {
        let font = if bold { "/F2" } else { "/F1" };
        self.cur
            .extend_from_slice(format!("BT {font} {size} Tf {x:.2} {y:.2} Td (").as_bytes());
        self.cur.extend_from_slice(&Self::escape(text));
        self.cur.extend_from_slice(b") Tj ET\n");
    }

    fn line_at(&mut self, x1: f64, y: f64, x2: f64, width: f64) {
        self.cur.extend_from_slice(
            format!("{width:.2} w {x1:.2} {y:.2} m {x2:.2} {y:.2} l S\n").as_bytes(),
        );
    }

    fn need(&mut self, space: f64) {
        if self.y - space < MARGIN_BOTTOM {
            self.new_page();
        }
    }

    fn new_page(&mut self) {
        if !self.cur.is_empty() {
            self.pages.push(std::mem::take(&mut self.cur));
        }
        self.y = PAGE_H - MARGIN_TOP;
    }

    fn text(&mut self, size: f64, bold: bool, text: &str) {
        self.need(size + 4.0);
        self.y -= size + 2.0;
        self.text_at(MARGIN_X, self.y, size, bold, text);
    }

    /// label left, amount right-aligned in the amount column
    fn row(&mut self, indent: f64, bold: bool, size: f64, label: &str, amount: Option<&str>) {
        self.need(size + 4.0);
        self.y -= size + 3.0;
        self.text_at(MARGIN_X + indent, self.y, size, bold, label);
        if let Some(a) = amount {
            // right-align: Helvetica averages ~0.5em per char (close enough for
            // a statement; exact metrics would need the AFM tables)
            let w = a.chars().count() as f64 * size * 0.5;
            self.text_at(PAGE_W - MARGIN_X - w, self.y, size, bold, a);
        }
    }

    fn rule(&mut self, width: f64) {
        self.need(6.0);
        self.y -= 4.0;
        let y = self.y;
        self.line_at(MARGIN_X, y, PAGE_W - MARGIN_X, width);
        self.y -= 2.0;
    }

    fn space(&mut self, h: f64) {
        self.need(h);
        self.y -= h;
    }

    fn build(mut self) -> Vec<u8> {
        self.new_page();
        if self.pages.is_empty() {
            self.pages.push(Vec::new());
        }
        let n_pages = self.pages.len();
        // objects: 1 catalog, 2 pages, 3..=2+n page objects, then the content
        // streams, then the two fonts
        let first_content = 3 + n_pages;
        let font_regular = first_content + n_pages;
        let font_bold = font_regular + 1;

        let mut out: Vec<u8> = Vec::new();
        let mut offsets: Vec<usize> = vec![0]; // object 0 is the free head
        let mut push = |out: &mut Vec<u8>, offsets: &mut Vec<usize>, body: String| {
            offsets.push(out.len());
            out.extend_from_slice(body.as_bytes());
        };

        out.extend_from_slice(b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n");
        push(
            &mut out,
            &mut offsets,
            "1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n".to_string(),
        );
        let kids: Vec<String> = (0..n_pages).map(|i| format!("{} 0 R", 3 + i)).collect();
        push(
            &mut out,
            &mut offsets,
            format!(
                "2 0 obj\n<< /Type /Pages /Count {n_pages} /Kids [{}] >>\nendobj\n",
                kids.join(" ")
            ),
        );
        for i in 0..n_pages {
            push(
                &mut out,
                &mut offsets,
                format!(
                    "{} 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {PAGE_W:.2} {PAGE_H:.2}] \
                     /Resources << /Font << /F1 {font_regular} 0 R /F2 {font_bold} 0 R >> >> \
                     /Contents {} 0 R >>\nendobj\n",
                    3 + i,
                    first_content + i
                ),
            );
        }
        for (i, content) in self.pages.iter().enumerate() {
            let bytes = content.as_slice();
            push(
                &mut out,
                &mut offsets,
                format!(
                    "{} 0 obj\n<< /Length {} >>\nstream\n",
                    first_content + i,
                    bytes.len()
                ),
            );
            out.extend_from_slice(bytes);
            out.extend_from_slice(b"endstream\nendobj\n");
        }
        push(
            &mut out,
            &mut offsets,
            format!(
                "{font_regular} 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>\nendobj\n"
            ),
        );
        push(
            &mut out,
            &mut offsets,
            format!(
                "{font_bold} 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >>\nendobj\n"
            ),
        );

        let xref_at = out.len();
        let count = offsets.len(); // includes object 0
        out.extend_from_slice(format!("xref\n0 {count}\n0000000000 65535 f \n").as_bytes());
        for off in offsets.iter().skip(1) {
            out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!("trailer\n<< /Size {count} /Root 1 0 R >>\nstartxref\n{xref_at}\n%%EOF\n")
                .as_bytes(),
        );
        out
    }
}

// ── HTML (unchanged shape: mirrors the JS template) ─────────────────────────

/// Like every other esc() in the codebase: quotes too, because user data can end
/// up inside an HTML attribute and a raw " would break out of it.
fn esc(s: Option<&Value>) -> String {
    let raw = match s {
        Some(Value::String(t)) => t.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    };
    raw.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn s_of<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    v.get(key)
}

fn side_lines(lines: &[Value]) -> String {
    let mut out = String::new();
    for g in lines {
        let sections = g["sections"].as_array().cloned().unwrap_or_default();
        if sections.is_empty() {
            continue;
        }
        out.push_str(&format!(
            "<tr class=\"group\"><td>{}</td><td class=\"num\">{}</td></tr>",
            esc(Some(&g["label"])),
            format_amount(g["total_cents"].as_i64().unwrap_or(0))
        ));
        for s in sections {
            for a in s["accounts"].as_array().cloned().unwrap_or_default() {
                out.push_str(&format!(
                    "<tr class=\"detail\"><td class=\"indent\">{}</td><td class=\"num\">{}</td></tr>",
                    esc(Some(&a["name"])),
                    format_amount(a["amount_cents"].as_i64().unwrap_or(0))
                ));
            }
        }
    }
    out
}

fn pnl_lines(report: &Value) -> String {
    let Some(pnl) = report.get("pnl").filter(|p| !p.is_null()) else {
        return String::new();
    };
    let mut out = String::new();
    for l in pnl["lines"].as_array().cloned().unwrap_or_default() {
        out.push_str(&format!(
            "\n      <tr class=\"group\"><td>{}</td><td class=\"num\">{}</td></tr>\n      ",
            esc(Some(&l["label"])),
            format_amount(l["total_cents"].as_i64().unwrap_or(0))
        ));
        for s in l["sections"].as_array().cloned().unwrap_or_default() {
            for a in s["accounts"].as_array().cloned().unwrap_or_default() {
                out.push_str(&format!(
                    "<tr class=\"detail\"><td class=\"indent\">{}</td><td class=\"num\">{}</td></tr>",
                    esc(Some(&a["name"])),
                    format_amount(a["amount_cents"].as_i64().unwrap_or(0))
                ));
            }
        }
        out.push_str("\n    ");
    }
    out
}

/// Year result for the micro model (no P&L in the report object): the
/// 'Onverdeeld resultaat' line the balans folds into Eigen vermogen. Using the
/// BEIV.05 total would mislabel capital + prior results as the year result.
fn micro_result_cents(report: &Value) -> i64 {
    for s in report["balans"]["passiva"]
        .as_array()
        .cloned()
        .unwrap_or_default()
    {
        if s["taxonomy_code"].is_null() && s["label"] == serde_json::json!("Onverdeeld resultaat") {
            return s["total_cents"].as_i64().unwrap_or(0);
        }
    }
    0
}

pub fn jaarrekening_html(report: &Value) -> String {
    let c = &report["company"];
    let pnl = report.get("pnl").filter(|p| !p.is_null());
    let pnl_section = match pnl {
        Some(p) => format!(
            "<h2>Winst- en verliesrekening {year}</h2>\n  <table style=\"width:60%\">\n    {lines}\n    <tr class=\"total\"><td>Resultaat na belastingen</td><td class=\"num\">{resultaat}</td></tr>\n  </table>",
            year = report["year"].as_str().unwrap_or(""),
            lines = pnl_lines(report),
            resultaat = p["resultaat"].as_str().unwrap_or("")
        ),
        None => format!(
            "<h2>Resultaat {year}</h2>\n  <table style=\"width:60%\"><tr class=\"total\"><td>Resultaat na belastingen</td><td class=\"num\">{amount}</td></tr></table>",
            year = report["year"].as_str().unwrap_or(""),
            amount = format_amount(micro_result_cents(report))
        ),
    };
    let model_line = if report["model"] == serde_json::json!("micro") {
        "micro (art. 2:395a BW)"
    } else {
        "klein (art. 2:396 BW)"
    };
    let model_footer = if report["model"] == serde_json::json!("micro") {
        "micro"
    } else {
        "klein"
    };

    format!(
        r#"<!DOCTYPE html>
<html lang="nl">
<head>
<meta charset="utf-8">
<style>
  body {{ font-family: 'DejaVu Sans', sans-serif; font-size: 10.5px; color: #1a1a1a; margin: 0; }}
  h1 {{ font-size: 17px; margin: 0 0 2px 0; }}
  .meta {{ color: #444; margin-bottom: 18px; }}
  .meta p {{ margin: 1px 0; }}
  h2 {{ font-size: 12px; text-transform: uppercase; color: #555; border-bottom: 1px solid #999; padding-bottom: 3px; margin: 22px 0 8px 0; }}
  table {{ width: 100%; border-collapse: collapse; }}
  td {{ padding: 2px 6px; }}
  .num {{ text-align: right; }}
  tr.group td {{ font-weight: bold; border-top: 1px solid #ccc; padding-top: 5px; }}
  tr.detail td {{ color: #333; }}
  td.indent {{ padding-left: 18px; }}
  tr.total td {{ font-weight: bold; border-top: 2px solid #333; }}
  .footer {{ margin-top: 36px; font-size: 9.5px; color: #666; }}
  .footer .sign {{ margin-top: 26px; }}
</style>
</head>
<body>
  <h1>Jaarrekening {year}</h1>
  <div class="meta">
    <p><strong>{cname}</strong></p>
    <p>{caddr} {cpost} {ccity}</p>
    <p>KvK {ckvk} · BTW {cbtw}</p>
    <p>Model: {model_line} · peildatum {as_of}</p>
  </div>

  <h2>Balans per {as_of}</h2>
  <table>
    <tr><td style="width:50%"><table><tr class="group"><td>Activa</td><td class="num"></td></tr>{activa}<tr class="total"><td>Totaal activa</td><td class="num">{total_activa}</td></tr></table></td>
        <td><table><tr class="group"><td>Passiva</td><td class="num"></td></tr>{passiva}<tr class="total"><td>Totaal passiva</td><td class="num">{total_passiva}</td></tr></table></td></tr>
  </table>

  {pnl_section}

  <div class="footer">
    <p>Opgesteld op basis van de administratie. Jaarrekeningmodel {model_footer} conform Titel 9 Boek 2 BW.</p>
    <p class="sign">Vastgesteld door het bestuur te {ccity} op __________</p>
  </div>
</body>
</html>"#,
        year = report["year"].as_str().unwrap_or(""),
        cname = esc(s_of(c, "name")),
        caddr = esc(s_of(c, "address")),
        cpost = esc(s_of(c, "postal_code")),
        ccity = esc(s_of(c, "city")),
        ckvk = esc(s_of(c, "kvk")),
        cbtw = esc(s_of(c, "btw_id")),
        model_line = model_line,
        as_of = report["as_of"].as_str().unwrap_or(""),
        activa = side_lines(
            &report["balans"]["activa"]
                .as_array()
                .cloned()
                .unwrap_or_default()
        ),
        passiva = side_lines(
            &report["balans"]["passiva"]
                .as_array()
                .cloned()
                .unwrap_or_default()
        ),
        total_activa = format_amount(report["balans"]["total_activa_cents"].as_i64().unwrap_or(0)),
        total_passiva = format_amount(
            report["balans"]["total_passiva_cents"]
                .as_i64()
                .unwrap_or(0)
        ),
        pnl_section = pnl_section,
        model_footer = model_footer,
    )
}

// ── native PDF ──────────────────────────────────────────────────────────────

/// The same document the HTML carries, laid out for A4 with the standard fonts.
pub fn jaarrekening_pdf(report: &Value) -> Vec<u8> {
    let c = &report["company"];
    let plain = |v: Option<&Value>| match v {
        Some(Value::String(t)) => t.clone(),
        Some(Value::Null) | None => String::new(),
        Some(o) => o.to_string(),
    };
    let mut p = Pdf::new();

    p.text(
        16.0,
        true,
        &format!("Jaarrekening {}", report["year"].as_str().unwrap_or("")),
    );
    p.space(2.0);
    p.text(10.5, true, &plain(s_of(c, "name")));
    p.text(
        9.5,
        false,
        &format!(
            "{} {} {}",
            plain(s_of(c, "address")),
            plain(s_of(c, "postal_code")),
            plain(s_of(c, "city"))
        )
        .trim()
        .to_string(),
    );
    p.text(
        9.5,
        false,
        &format!(
            "KvK {} - BTW {}",
            plain(s_of(c, "kvk")),
            plain(s_of(c, "btw_id"))
        ),
    );
    let model_line = if report["model"] == serde_json::json!("micro") {
        "micro (art. 2:395a BW)"
    } else {
        "klein (art. 2:396 BW)"
    };
    p.text(
        9.5,
        false,
        &format!(
            "Model: {model_line} - peildatum {}",
            report["as_of"].as_str().unwrap_or("")
        ),
    );
    p.space(10.0);

    let as_of = report["as_of"].as_str().unwrap_or("");
    p.text(12.0, true, &format!("BALANS PER {as_of}"));
    p.rule(0.7);
    let mut render_side = |p: &mut Pdf, header: &str, groups: &[Value], total: i64| {
        p.row(0.0, true, 10.5, header, None);
        for g in groups {
            let sections = g["sections"].as_array().cloned().unwrap_or_default();
            if sections.is_empty() {
                continue;
            }
            p.rule(0.4);
            p.row(
                0.0,
                true,
                10.5,
                g["label"].as_str().unwrap_or(""),
                Some(&format_amount(g["total_cents"].as_i64().unwrap_or(0))),
            );
            for s in sections {
                for a in s["accounts"].as_array().cloned().unwrap_or_default() {
                    p.row(
                        14.0,
                        false,
                        10.0,
                        a["name"].as_str().unwrap_or(""),
                        Some(&format_amount(a["amount_cents"].as_i64().unwrap_or(0))),
                    );
                }
            }
        }
        p.rule(1.0);
        p.row(
            0.0,
            true,
            10.5,
            &format!("Totaal {}", header.to_lowercase()),
            Some(&format_amount(total)),
        );
        p.space(8.0);
    };
    render_side(
        &mut p,
        "Activa",
        &report["balans"]["activa"]
            .as_array()
            .cloned()
            .unwrap_or_default(),
        report["balans"]["total_activa_cents"].as_i64().unwrap_or(0),
    );
    render_side(
        &mut p,
        "Passiva",
        &report["balans"]["passiva"]
            .as_array()
            .cloned()
            .unwrap_or_default(),
        report["balans"]["total_passiva_cents"]
            .as_i64()
            .unwrap_or(0),
    );

    match report.get("pnl").filter(|p| !p.is_null()) {
        Some(pnl) => {
            p.text(
                12.0,
                true,
                &format!(
                    "WINST- EN VERLIESREKENING {}",
                    report["year"].as_str().unwrap_or("")
                ),
            );
            p.rule(0.7);
            for l in pnl["lines"].as_array().cloned().unwrap_or_default() {
                p.row(
                    0.0,
                    true,
                    10.5,
                    l["label"].as_str().unwrap_or(""),
                    Some(&format_amount(l["total_cents"].as_i64().unwrap_or(0))),
                );
                for s in l["sections"].as_array().cloned().unwrap_or_default() {
                    for a in s["accounts"].as_array().cloned().unwrap_or_default() {
                        p.row(
                            14.0,
                            false,
                            10.0,
                            a["name"].as_str().unwrap_or(""),
                            Some(&format_amount(a["amount_cents"].as_i64().unwrap_or(0))),
                        );
                    }
                }
            }
            p.rule(1.0);
            p.row(
                0.0,
                true,
                10.5,
                "Resultaat na belastingen",
                Some(pnl["resultaat"].as_str().unwrap_or("")),
            );
        }
        None => {
            p.text(
                12.0,
                true,
                &format!("RESULTAAT {}", report["year"].as_str().unwrap_or("")),
            );
            p.rule(0.7);
            p.row(
                0.0,
                true,
                10.5,
                "Resultaat na belastingen",
                Some(&format_amount(micro_result_cents(report))),
            );
        }
    }

    p.space(24.0);
    p.text(
        9.0,
        false,
        "Opgesteld op basis van de administratie. Jaarrekeningmodel conform Titel 9 Boek 2 BW.",
    );
    p.space(20.0);
    p.text(
        9.0,
        false,
        &format!(
            "Vastgesteld door het bestuur te {} op __________",
            plain(s_of(c, "city"))
        ),
    );
    p.build()
}

pub fn jaarrekening_to_pdf(report: &Value, out_path: Option<&str>) -> Result<Value> {
    let data = jaarrekening_pdf(report);
    if data.is_empty() {
        return Err(BukioError::new("PDF_UNAVAILABLE", "empty PDF"));
    }
    match out_path {
        Some(path) => {
            std::fs::write(path, &data).map_err(|e| {
                BukioError::new("PDF_UNAVAILABLE", format!("could not write {path}: {e}"))
            })?;
            Ok(serde_json::json!({ "path": path, "bytes": data.len() }))
        }
        None => Ok(serde_json::json!({
            "bytes": data.len(),
            "data": base64(&data),
        })),
    }
}

fn base64(data: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

// ── invoice document (mirrors src/invoice/pdf.js) ───────────────────────────

use rusqlite::Connection;

/// The label list the invoice template uses (all `pdf.*` keys, like the JS).
fn invoice_labels(lang: &str) -> std::collections::HashMap<&'static str, String> {
    let mut m = std::collections::HashMap::new();
    for k in [
        "invoice",
        "credit",
        "kvk",
        "btw",
        "billedTo",
        "description",
        "qty",
        "unit",
        "price",
        "vat",
        "amount",
        "subtotal",
        "discount",
        "vatTotal",
        "total",
        "inclVat",
        "footerPay",
        "dueDateTerm",
        "defaultTerm",
        "dueDate",
        "reference",
        "date",
        "reverseCharge",
    ] {
        m.insert(k, crate::i18n::label(k, lang));
    }
    m
}

fn fill(template: &str, pairs: &[(&str, &str)]) -> String {
    let mut out = template.to_string();
    for (k, v) in pairs {
        out = out.replace(&format!("{{{k}}}"), v);
    }
    out
}

fn company_row(db: &Connection) -> Value {
    db.query_row(
        "SELECT name, address, postal_code, city, registration_id, tax_id, iban FROM company WHERE id = 1",
        [],
        |r| {
            Ok(serde_json::json!({
                "name": r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                "address": r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                "postal_code": r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                "city": r.get::<_, Option<String>>(3)?.unwrap_or_default(),
                "registration_id": r.get::<_, Option<String>>(4)?.unwrap_or_default(),
                "tax_id": r.get::<_, Option<String>>(5)?.unwrap_or_default(),
                "iban": r.get::<_, Option<String>>(6)?.unwrap_or_default(),
            }))
        },
    )
    .unwrap_or_else(|_| serde_json::json!({}))
}

pub fn invoice_html(db: &Connection, invoice: &Value) -> String {
    let company = company_row(db);
    let contact = &invoice["contact"];
    let is_credit = invoice["invoice_type"] == serde_json::json!("credit");
    let lang = invoice["language"].as_str().unwrap_or("en");
    let l = invoice_labels(lang);
    let lab = |k: &str| l.get(k).cloned().unwrap_or_default();

    let lines_arr = invoice["lines"].as_array().cloned().unwrap_or_default();
    let totals = crate::invoice::compute_invoice_totals(
        &lines_arr,
        invoice["discount_type"].as_str(),
        invoice["discount_value"].as_i64(),
    );

    let rows: String = lines_arr
        .iter()
        .enumerate()
        .map(|(i, ln)| {
            let disc = crate::invoice::line_discount_cents(ln);
            let rate_bp = ln["vat_rate_bp"].as_i64().unwrap_or(0);
            let vat_code = ln["vat_code"].as_str().unwrap_or("");
            let vat_txt = if rate_bp != 0 {
                format!("{:.1}%", rate_bp as f64 / 100.0)
            } else if vat_code == "R" || vat_code == "RE" {
                lab("reverseCharge")
            } else {
                "-".to_string()
            };
            let disc_html = if disc > 0 {
                format!(
                    "<div class=\"disc\">{}: \u{2212}{}</div>",
                    lab("discount"),
                    format_amount(disc)
                )
            } else {
                String::new()
            };
            format!(
                "\n      <tr>\n        <td>{n}</td>\n        <td>{desc}{disc_html}</td>\n        <td class=\"num\">{qty}</td>\n        <td>{unit}</td>\n        <td class=\"num\">{price}</td>\n        <td class=\"num\">{vat}</td>\n        <td class=\"num\">{amount}</td>\n      </tr>",
                n = i + 1,
                desc = esc(Some(&ln["description"])),
                qty = crate::invoice::format_qty(ln["quantity"].as_i64().unwrap_or(0)),
                unit = esc(Some(&serde_json::json!(crate::i18n::unit_label(
                    ln["unit"].as_str().unwrap_or(""),
                    lang
                )))),
                price = format_amount(ln["unit_price_cents"].as_i64().unwrap_or(0)),
                vat = vat_txt,
                amount = format_amount(ln["amount_cents"].as_i64().unwrap_or(0)),
            )
        })
        .collect();

    let vat_rows: String = totals["breakdown"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|b| {
            format!(
                "\n      <tr><td>{} {:.0}%</td><td class=\"num\">{}</td><td class=\"num\">{}</td></tr>",
                lab("vatOn"),
                b["rate_bp"].as_i64().unwrap_or(0) as f64 / 100.0,
                format_amount(b["base_cents"].as_i64().unwrap_or(0)),
                format_amount(b["vat_cents"].as_i64().unwrap_or(0))
            )
        })
        .collect();

    let due_date = invoice["due_date"].as_str().unwrap_or("");
    let footer_term = if !due_date.is_empty() {
        fill(&lab("dueDateTerm"), &[("date", due_date)])
    } else {
        lab("defaultTerm")
    };
    let invoice_iban = company["iban"].as_str().unwrap_or("").to_string();
    let invoice_name = company["name"].as_str().unwrap_or("").to_string();
    let footer_pay = fill(
        &lab("footerPay"),
        &[
            ("term", &footer_term),
            ("iban", &invoice_iban),
            ("name", &invoice_name),
        ],
    );
    let number = match invoice["invoice_number"].as_str() {
        Some(n) => n.to_string(),
        None => crate::i18n::t("status.draft", &[], lang),
    };
    let contact_block = {
        let mut out = format!(
            "<p><strong>{}</strong></p>\n      <p>{}</p>\n      <p>{} {}</p>",
            esc(Some(&contact["name"])),
            esc(contact.get("address")),
            esc(contact.get("postal_code")),
            esc(contact.get("city"))
        );
        if let Some(vat) = contact["vat_id"].as_str().filter(|v| !v.is_empty()) {
            out.push_str(&format!(
                "\n      <p>{} {}</p>",
                lab("btw"),
                esc(Some(&serde_json::json!(vat)))
            ));
        }
        out
    };

    format!(
        r#"<!DOCTYPE html>
<html lang="{lang}">
<head>
<meta charset="utf-8">
<style>
  body {{ font-family: 'DejaVu Sans', sans-serif; font-size: 11px; color: #1a1a1a; margin: 0; }}
  .header {{ display: flex; justify-content: space-between; margin-bottom: 28px; }}
  .supplier {{ display: flex; align-items: flex-start; gap: 12px; }}
  .supplier img.logo {{ max-height: 60px; max-width: 160px; object-fit: contain; }}
  .supplier h1 {{ font-size: 18px; margin: 0 0 4px 0; }}
  .supplier p {{ margin: 1px 0; color: #444; }}
  .title {{ text-align: right; }}
  .title h2 {{ font-size: 22px; margin: 0 0 8px 0; }}
  .title p {{ margin: 2px 0; }}
  .parties {{ display: flex; justify-content: space-between; margin-bottom: 24px; }}
  .parties h3 {{ font-size: 11px; text-transform: uppercase; color: #666; margin: 0 0 6px 0; }}
  .parties p {{ margin: 1px 0; }}
  table {{ width: 100%; border-collapse: collapse; margin-bottom: 20px; }}
  th {{ text-align: left; border-bottom: 2px solid #333; padding: 4px 6px; font-size: 10px; text-transform: uppercase; color: #555; }}
  td {{ border-bottom: 1px solid #ddd; padding: 6px; }}
  .disc {{ font-size: 10px; color: #777; }}
  .num {{ text-align: right; }}
  .totals {{ width: 300px; margin-left: auto; }}
  .totals td {{ border-bottom: none; padding: 3px 6px; }}
  .totals .grand td {{ border-top: 2px solid #333; font-weight: bold; font-size: 13px; }}
  .footer {{ margin-top: 40px; font-size: 10px; color: #666; }}
  .footer p {{ margin: 2px 0; }}
</style>
</head>
<body>
  <div class="header">
    <div class="supplier">
      <div>
        <h1>{cname}</h1>
        <p>{caddr}</p>
        <p>{cpost} {ccity}</p>
        <p>{kvk} {creg} · {btw} {ctax}</p>
      </div>
    </div>
    <div class="title">
      <h2>{title}</h2>
      <p><strong>{number}</strong></p>
      <p>{date_l}: {date}</p>
      {due_p}
      {ref_p}
    </div>
  </div>
  <div class="parties">
    <div>
      <h3>{billed_to}</h3>
      {contact_block}
    </div>
  </div>
  <table>
    <thead><tr>
      <th>#</th><th>{description}</th>
      <th class="num">{qty}</th><th>{unit}</th>
      <th class="num">{price}</th><th class="num">{vat}</th><th class="num">{amount}</th>
    </tr></thead>
    <tbody>{rows}</tbody>
  </table>
  <table class="totals">
    <tr><td>{subtotal}</td><td></td><td class="num">{net_before}</td></tr>
    {disc_row}
    {vat_rows}
    {vat_total_row}
    <tr class="grand"><td>{total}{incl}</td><td></td><td class="num">{gross}</td></tr>
  </table>
  <div class="footer">
    <p>{footer_pay}</p>
    {notes}
  </div>
</body>
</html>"#,
        lang = lang,
        cname = esc(company.get("name")),
        caddr = esc(company.get("address")),
        cpost = esc(company.get("postal_code")),
        ccity = esc(company.get("city")),
        kvk = lab("kvk"),
        creg = esc(company.get("registration_id")),
        btw = lab("btw"),
        ctax = esc(company.get("tax_id")),
        title = if is_credit {
            lab("credit")
        } else {
            lab("invoice")
        },
        number = esc(Some(&serde_json::json!(number))),
        date_l = lab("date"),
        date = esc(Some(&invoice["date"])),
        due_p = if due_date.is_empty() {
            String::new()
        } else {
            format!(
                "<p>{}: {}</p>",
                lab("dueDate"),
                esc(Some(&serde_json::json!(due_date)))
            )
        },
        ref_p = match invoice["reference"].as_str().filter(|r| !r.is_empty()) {
            Some(r) => format!(
                "<p>{}: {}</p>",
                lab("reference"),
                esc(Some(&serde_json::json!(r)))
            ),
            None => String::new(),
        },
        billed_to = lab("billedTo"),
        contact_block = contact_block,
        description = lab("description"),
        qty = lab("qty"),
        unit = lab("unit"),
        price = lab("price"),
        vat = lab("vat"),
        amount = lab("amount"),
        rows = rows,
        subtotal = lab("subtotal"),
        net_before = format_amount(totals["net_before_cents"].as_i64().unwrap_or(0)),
        disc_row = if totals["discount_cents"].as_i64().unwrap_or(0) > 0 {
            format!(
                "<tr><td>{}</td><td></td><td class=\"num\">\u{2212}{}</td></tr>",
                lab("discount"),
                format_amount(totals["discount_cents"].as_i64().unwrap_or(0))
            )
        } else {
            String::new()
        },
        vat_rows = vat_rows,
        vat_total_row = if totals["vat_cents"].as_i64().unwrap_or(0) > 0 {
            format!(
                "<tr><td>{}</td><td></td><td class=\"num\">{}</td></tr>",
                lab("vatTotal"),
                format_amount(totals["vat_cents"].as_i64().unwrap_or(0))
            )
        } else {
            String::new()
        },
        total = lab("total"),
        incl = if totals["vat_cents"].as_i64().unwrap_or(0) > 0 {
            format!(" ({})", lab("inclVat"))
        } else {
            String::new()
        },
        gross = format_amount(invoice["gross_cents"].as_i64().unwrap_or(0)),
        footer_pay = esc(Some(&serde_json::json!(footer_pay))),
        notes = match invoice["notes"].as_str().filter(|n| !n.is_empty()) {
            Some(n) => format!("<p>{}</p>", esc(Some(&serde_json::json!(n)))),
            None => String::new(),
        },
    )
}

/// The same invoice on A4 with the standard fonts. Column layout follows the
/// HTML table: #, description, qty, unit, price, vat, amount.
pub fn invoice_pdf(db: &Connection, invoice: &Value) -> Vec<u8> {
    let company = company_row(db);
    let contact = &invoice["contact"];
    let is_credit = invoice["invoice_type"] == serde_json::json!("credit");
    let lang = invoice["language"].as_str().unwrap_or("en");
    let l = invoice_labels(lang);
    let lab = |k: &str| l.get(k).cloned().unwrap_or_default();
    let lines_arr = invoice["lines"].as_array().cloned().unwrap_or_default();
    let totals = crate::invoice::compute_invoice_totals(
        &lines_arr,
        invoice["discount_type"].as_str(),
        invoice["discount_value"].as_i64(),
    );
    let number = match invoice["invoice_number"].as_str() {
        Some(n) => n.to_string(),
        None => crate::i18n::t("status.draft", &[], lang),
    };
    // columns: x offset from the left margin, width, right-aligned
    const W: f64 = PAGE_W - 2.0 * MARGIN_X;
    let cols: [(f64, f64, bool); 7] = [
        (0.0, 0.05 * W, false),
        (0.06 * W, 0.44 * W, false),
        (0.52 * W, 0.08 * W, true),
        (0.61 * W, 0.06 * W, false),
        (0.68 * W, 0.10 * W, true),
        (0.79 * W, 0.08 * W, true),
        (0.88 * W, 0.12 * W, true),
    ];

    let mut p = Pdf::new();
    let right_edge = PAGE_W - MARGIN_X;
    p.text_at(
        MARGIN_X,
        PAGE_H - MARGIN_TOP,
        16.0,
        true,
        company["name"].as_str().unwrap_or(""),
    );
    let mut y = PAGE_H - MARGIN_TOP;
    for (i, line) in [
        company["address"].as_str().unwrap_or(""),
        &format!(
            "{} {}",
            company["postal_code"].as_str().unwrap_or(""),
            company["city"].as_str().unwrap_or("")
        ),
    ]
    .iter()
    .enumerate()
    {
        y -= 12.0;
        p.text_at(MARGIN_X, y, 9.5, false, line);
        let _ = i;
    }
    y -= 12.0;
    p.text_at(
        MARGIN_X,
        y,
        9.5,
        false,
        &format!(
            "{} {} - {} {}",
            lab("kvk"),
            company["registration_id"].as_str().unwrap_or(""),
            lab("btw"),
            company["tax_id"].as_str().unwrap_or("")
        ),
    );
    let head_y = PAGE_H - MARGIN_TOP;
    let title = if is_credit {
        lab("credit")
    } else {
        lab("invoice")
    };
    p.text_right(right_edge, head_y, 20.0, true, &title);
    p.text_right(right_edge, head_y - 18.0, 11.0, true, &number);
    p.text_right(
        right_edge,
        head_y - 32.0,
        9.5,
        false,
        &format!(
            "{}: {}",
            lab("date"),
            invoice["date"].as_str().unwrap_or("")
        ),
    );
    let mut hy = head_y - 46.0;
    if let Some(d) = invoice["due_date"].as_str().filter(|d| !d.is_empty()) {
        p.text_right(
            right_edge,
            hy,
            9.5,
            false,
            &format!("{}: {d}", lab("dueDate")),
        );
        hy -= 14.0;
    }
    if let Some(r) = invoice["reference"].as_str().filter(|r| !r.is_empty()) {
        p.text_right(
            right_edge,
            hy,
            9.5,
            false,
            &format!("{}: {r}", lab("reference")),
        );
    }

    p.y = y - 26.0;
    p.text(9.0, true, &lab("billedTo").to_uppercase());
    p.text(10.0, true, contact["name"].as_str().unwrap_or(""));
    p.text(9.5, false, contact["address"].as_str().unwrap_or(""));
    p.text(
        9.5,
        false,
        &format!(
            "{} {}",
            contact["postal_code"].as_str().unwrap_or(""),
            contact["city"].as_str().unwrap_or("")
        )
        .trim()
        .to_string(),
    );
    if let Some(v) = contact["vat_id"].as_str().filter(|v| !v.is_empty()) {
        p.text(9.5, false, &format!("{} {v}", lab("btw")));
    }
    p.space(14.0);

    p.cols(
        9.0,
        true,
        &[
            (cols[0].0, cols[0].1, false, "#".into()),
            (cols[1].0, cols[1].1, false, lab("description")),
            (cols[2].0, cols[2].1, true, lab("qty")),
            (cols[3].0, cols[3].1, false, lab("unit")),
            (cols[4].0, cols[4].1, true, lab("price")),
            (cols[5].0, cols[5].1, true, lab("vat")),
            (cols[6].0, cols[6].1, true, lab("amount")),
        ],
    );
    p.rule(0.8);
    for (i, ln) in lines_arr.iter().enumerate() {
        let rate_bp = ln["vat_rate_bp"].as_i64().unwrap_or(0);
        let vat_code = ln["vat_code"].as_str().unwrap_or("");
        let vat_txt = if rate_bp != 0 {
            format!("{:.1}%", rate_bp as f64 / 100.0)
        } else if vat_code == "R" || vat_code == "RE" {
            lab("reverseCharge")
        } else {
            "-".to_string()
        };
        p.cols(
            9.5,
            false,
            &[
                (cols[0].0, cols[0].1, false, format!("{}", i + 1)),
                (
                    cols[1].0,
                    cols[1].1,
                    false,
                    ln["description"].as_str().unwrap_or("").to_string(),
                ),
                (
                    cols[2].0,
                    cols[2].1,
                    true,
                    crate::invoice::format_qty(ln["quantity"].as_i64().unwrap_or(0)),
                ),
                (
                    cols[3].0,
                    cols[3].1,
                    false,
                    crate::i18n::unit_label(ln["unit"].as_str().unwrap_or(""), lang),
                ),
                (
                    cols[4].0,
                    cols[4].1,
                    true,
                    format_amount(ln["unit_price_cents"].as_i64().unwrap_or(0)),
                ),
                (cols[5].0, cols[5].1, true, vat_txt),
                (
                    cols[6].0,
                    cols[6].1,
                    true,
                    format_amount(ln["amount_cents"].as_i64().unwrap_or(0)),
                ),
            ],
        );
        let disc = crate::invoice::line_discount_cents(ln);
        if disc > 0 {
            p.cols(
                8.5,
                false,
                &[(
                    cols[1].0,
                    cols[1].1,
                    false,
                    format!("{}: -{}", lab("discount"), format_amount(disc)),
                )],
            );
        }
    }
    p.rule(0.8);
    p.space(6.0);

    let mut amount_row = |p: &mut Pdf, label: &str, amount: String, bold: bool| {
        p.cols(
            10.0,
            bold,
            &[
                (0.60 * W, 0.28 * W, false, label.to_string()),
                (0.88 * W, 0.12 * W, true, amount),
            ],
        );
    };
    amount_row(
        &mut p,
        &lab("subtotal"),
        format_amount(totals["net_before_cents"].as_i64().unwrap_or(0)),
        false,
    );
    if totals["discount_cents"].as_i64().unwrap_or(0) > 0 {
        amount_row(
            &mut p,
            &lab("discount"),
            format!(
                "\u{2212}{}",
                format_amount(totals["discount_cents"].as_i64().unwrap_or(0))
            ),
            false,
        );
    }
    for b in totals["breakdown"].as_array().cloned().unwrap_or_default() {
        amount_row(
            &mut p,
            &format!(
                "{} {:.0}%",
                lab("vatOn"),
                b["rate_bp"].as_i64().unwrap_or(0) as f64 / 100.0
            ),
            format_amount(b["vat_cents"].as_i64().unwrap_or(0)),
            false,
        );
    }
    if totals["vat_cents"].as_i64().unwrap_or(0) > 0 {
        amount_row(
            &mut p,
            &lab("vatTotal"),
            format_amount(totals["vat_cents"].as_i64().unwrap_or(0)),
            false,
        );
    }
    p.rule(0.8);
    let total_label = if totals["vat_cents"].as_i64().unwrap_or(0) > 0 {
        format!("{} ({})", lab("total"), lab("inclVat"))
    } else {
        lab("total")
    };
    amount_row(
        &mut p,
        &total_label,
        format_amount(invoice["gross_cents"].as_i64().unwrap_or(0)),
        true,
    );

    p.space(22.0);
    let due_date = invoice["due_date"].as_str().unwrap_or("");
    let footer_term = if !due_date.is_empty() {
        fill(&lab("dueDateTerm"), &[("date", due_date)])
    } else {
        lab("defaultTerm")
    };
    let footer = fill(
        &lab("footerPay"),
        &[
            ("term", &footer_term),
            ("iban", company["iban"].as_str().unwrap_or("")),
            ("name", company["name"].as_str().unwrap_or("")),
        ],
    );
    p.text(9.0, false, &footer);
    if let Some(n) = invoice["notes"].as_str().filter(|n| !n.is_empty()) {
        p.space(6.0);
        p.text(9.0, false, n);
    }
    p.build()
}

pub fn invoice_to_pdf(db: &Connection, invoice: &Value, out_path: Option<&str>) -> Result<Value> {
    let data = invoice_pdf(db, invoice);
    match out_path {
        Some(path) => {
            std::fs::write(path, &data).map_err(|e| {
                BukioError::new("PDF_UNAVAILABLE", format!("could not write {path}: {e}"))
            })?;
            Ok(serde_json::json!({ "path": path, "bytes": data.len() }))
        }
        None => Ok(serde_json::json!({ "bytes": data.len(), "data": base64(&data) })),
    }
}
