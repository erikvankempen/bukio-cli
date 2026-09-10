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
