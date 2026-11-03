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

/// The pixel size of a baseline or progressive JPEG (its SOF marker), or `None` if `jpeg` is not
/// a JPEG.
pub fn jpeg_size(jpeg: &[u8]) -> Option<(u32, u32, u8)> {
    if jpeg.len() < 4 || jpeg[0] != 0xFF || jpeg[1] != 0xD8 {
        return None;
    }
    let mut i = 2;
    while i + 9 < jpeg.len() {
        if jpeg[i] != 0xFF {
            return None;
        }
        let marker = jpeg[i + 1];
        if marker == 0xFF {
            i += 1;
            continue;
        }
        let len = u16::from_be_bytes([jpeg[i + 2], jpeg[i + 3]]) as usize;
        // SOF0..SOF15, except DHT (C4), JPG (C8) and DAC (CC).
        if (0xC0..=0xCF).contains(&marker) && ![0xC4, 0xC8, 0xCC].contains(&marker) {
            let h = u16::from_be_bytes([jpeg[i + 5], jpeg[i + 6]]) as u32;
            let w = u16::from_be_bytes([jpeg[i + 7], jpeg[i + 8]]) as u32;
            let components = jpeg[i + 9];
            return (w > 0 && h > 0).then_some((w, h, components));
        }
        i += 2 + len;
    }
    None
}

/// A one-page A4 PDF showing a JPEG (the title deed sheet) fitted to the page, centred, aspect
/// kept. The JPEG is embedded as it is (DCTDecode), so the PDF is exactly the sheet officers
/// preview. Errors if `jpeg` is not a JPEG.
pub fn jpeg_a4_pdf(jpeg: &[u8]) -> Result<Vec<u8>, String> {
    let (w, h, comps) = jpeg_size(jpeg).ok_or("not a JPEG image")?;
    let colour = match comps {
        1 => "/DeviceGray",
        4 => "/DeviceCMYK",
        _ => "/DeviceRGB",
    };
    let k = (A4_W / w as f64).min(A4_H / h as f64);
    let (dw, dh) = (w as f64 * k, h as f64 * k);
    let content = format!("q {dw:.3} 0 0 {dh:.3} {:.3} {:.3} cm /Im1 Do Q", (A4_W - dw) / 2.0, (A4_H - dh) / 2.0);
    let mut objects: Vec<Vec<u8>> = vec![
        b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
        b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
        format!("<</Type/Page/Parent 2 0 R/MediaBox[0 0 {A4_W:.0} {A4_H:.0}]/Resources<</XObject<</Im1 4 0 R>>>>/Contents 5 0 R>>").into_bytes(),
    ];
    let mut img = format!("<</Type/XObject/Subtype/Image/Width {w}/Height {h}/ColorSpace{colour}/BitsPerComponent 8/Filter/DCTDecode/Length {}>>\nstream\n", jpeg.len()).into_bytes();
    img.extend_from_slice(jpeg);
    img.extend_from_slice(b"\nendstream");
    objects.push(img);
    objects.push(format!("<</Length {}>>\nstream\n{content}\nendstream", content.len()).into_bytes());

    let mut body: Vec<u8> = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = Vec::with_capacity(objects.len());
    for (i, obj) in objects.iter().enumerate() {
        offsets.push(body.len());
        body.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        body.extend_from_slice(obj);
        body.extend_from_slice(b"\nendobj\n");
    }
    let xref_start = body.len();
    let size = objects.len() + 1;
    let mut tail = format!("xref\n0 {size}\n0000000000 65535 f \n");
    for off in &offsets {
        let _ = writeln!(tail, "{off:010} 00000 n ");
    }
    let _ = write!(tail, "trailer\n<</Size {size}/Root 1 0 R>>\nstartxref\n{xref_start}\n%%EOF");
    body.extend_from_slice(tail.as_bytes());
    Ok(body)
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

    /// A 2x3 px greyscale JPEG header (SOI, APP0, SOF0) and EOI: enough for the size parser.
    fn tiny_jpeg() -> Vec<u8> {
        let mut j = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
        j.extend_from_slice(b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0");
        j.extend_from_slice(&[0xFF, 0xC0, 0x00, 0x0B, 0x08, 0x00, 0x03, 0x00, 0x02, 0x01, 0x01, 0x11, 0x00]);
        j.extend_from_slice(&[0xFF, 0xD9]);
        j
    }

    #[test]
    fn reads_jpeg_size_and_rejects_other_files() {
        assert_eq!(jpeg_size(&tiny_jpeg()), Some((2, 3, 1)));
        assert_eq!(jpeg_size(b"%PDF-1.4"), None);
        assert_eq!(jpeg_size(b"\x89PNG\r\n"), None);
        assert!(jpeg_a4_pdf(b"\x89PNG\r\n").is_err());
    }

    #[test]
    fn jpeg_becomes_a_one_page_a4_pdf_with_a_valid_xref() {
        let j = tiny_jpeg();
        let pdf = jpeg_a4_pdf(&j).unwrap();
        assert!(pdf.starts_with(b"%PDF-1.4") && pdf.ends_with(b"%%EOF"));
        // The JPEG is embedded byte for byte.
        assert!(pdf.windows(j.len()).any(|w| w == j.as_slice()));
        let s = String::from_utf8_lossy(&pdf);
        assert!(s.contains("/Filter/DCTDecode") && s.contains("/Width 2/Height 3") && s.contains("/DeviceGray") && s.contains("/MediaBox[0 0 595 842]"));
        // Every xref offset points at its "N 0 obj".
        let start: usize = s.rsplit("startxref\n").next().unwrap().split('\n').next().unwrap().parse().unwrap();
        let xref = &pdf[start..];
        let lines: Vec<&str> = std::str::from_utf8(xref).unwrap().lines().collect();
        for (n, line) in lines[3..8].iter().enumerate() {
            let off: usize = line[..10].parse().unwrap();
            assert!(pdf[off..].starts_with(format!("{} 0 obj", n + 1).as_bytes()), "object {}", n + 1);
        }
    }
}
