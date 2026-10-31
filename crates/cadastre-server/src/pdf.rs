//! A minimal, dependency-free PDF writer for job reports and site plans (FR-OUT-*).
//!
//! Enough to place Helvetica text and stroke polylines on a single A4 page — not a general PDF
//! library. (The SRS names typst for richer output; this keeps the server self-contained and the
//! win7 target lean until Phase 9, and produces valid PDFs now.)

use std::fmt::Write;

/// A4 page size in PostScript points.
pub const A4_W: f64 = 595.0;
/// A4 height in points.
pub const A4_H: f64 = 842.0;

/// A page content-stream builder. User space origin is bottom-left.
#[derive(Default)]
pub struct Page {
    ops: String,
}

impl Page {
    /// Draw a line of Helvetica text at `(x, y)` with the given font size.
    pub fn text(&mut self, x: f64, y: f64, size: f64, s: &str) {
        let _ = writeln!(
            self.ops,
            "BT /F1 {size} Tf {x:.2} {y:.2} Td ({}) Tj ET",
            escape(s)
        );
    }

    /// Stroke a polyline through `pts`; `close` adds the closing segment.
    pub fn polyline(&mut self, pts: &[(f64, f64)], close: bool) {
        let Some(first) = pts.first() else { return };
        let _ = write!(self.ops, "{:.2} {:.2} m ", first.0, first.1);
        for p in &pts[1..] {
            let _ = write!(self.ops, "{:.2} {:.2} l ", p.0, p.1);
        }
        self.ops.push_str(if close { "h S\n" } else { "S\n" });
    }

    /// Finish the page and assemble the full PDF document bytes.
    pub fn into_pdf(self) -> Vec<u8> {
        build(&self.ops)
    }
}

/// Escape the characters that are special inside a PDF literal string.
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '(' | ')' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            // Drop control/non-ASCII to keep the stream well-formed (reports are ASCII).
            c if (c as u32) < 0x20 || (c as u32) > 0x7e => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

/// Assemble a one-page A4 PDF around a content stream, with a byte-accurate xref table.
fn build(content: &str) -> Vec<u8> {
    let objects = [
        "<</Type/Catalog/Pages 2 0 R>>".to_string(),
        "<</Type/Pages/Kids[3 0 R]/Count 1>>".to_string(),
        format!(
            "<</Type/Page/Parent 2 0 R/MediaBox[0 0 {A4_W:.0} {A4_H:.0}]\
             /Resources<</Font<</F1 4 0 R>>>>/Contents 5 0 R>>"
        ),
        "<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>".to_string(),
        format!("<</Length {}>>\nstream\n{content}\nendstream", content.len()),
    ];

    let mut body = String::from("%PDF-1.4\n");
    let mut offsets = Vec::with_capacity(objects.len());
    for (i, obj) in objects.iter().enumerate() {
        offsets.push(body.len());
        let _ = write!(body, "{} 0 obj\n{obj}\nendobj\n", i + 1);
    }

    let xref_start = body.len();
    let size = objects.len() + 1;
    let _ = write!(body, "xref\n0 {size}\n0000000000 65535 f \n");
    for off in &offsets {
        let _ = writeln!(body, "{off:010} 00000 n ");
    }
    let _ = write!(
        body,
        "trailer\n<</Size {size}/Root 1 0 R>>\nstartxref\n{xref_start}\n%%EOF"
    );
    body.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn produces_a_valid_pdf_envelope() {
        let mut page = Page::default();
        page.text(72.0, 800.0, 14.0, "Hello (PLAGIS)");
        page.polyline(&[(100.0, 100.0), (200.0, 100.0), (200.0, 200.0)], true);
        let pdf = page.into_pdf();
        assert!(pdf.starts_with(b"%PDF-1.4"));
        assert!(pdf.ends_with(b"%%EOF"));
        // The special chars in the text were escaped, not dropped.
        let s = String::from_utf8_lossy(&pdf);
        assert!(s.contains(r"Hello \(PLAGIS\)"));
        assert!(s.contains("startxref"));
    }
}
