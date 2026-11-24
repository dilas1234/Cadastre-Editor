//! Writing a checked boundary and its drawing as DXF, KML or EMF (Checking and Data Verification
//! unit). Coordinates in are UTM 32N metres. DWG is this DXF through the ODA File Converter.

use cadastre_core::crs::WorkingCrs;
use cadastre_core::types::Point;
use serde::{Deserialize, Serialize};

use crate::emf::GEOREF_TAG;

/// A line or ring.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Shape {
    /// Layer (KML folder).
    pub layer: String,
    /// Name (KML placemark).
    #[serde(default)]
    pub name: String,
    /// `[easting, northing]`, not repeated at the end when `closed`.
    pub points: Vec<[f64; 2]>,
    /// Whether it closes back to its start.
    #[serde(default)]
    pub closed: bool,
}

/// A corner or label.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Label {
    /// Layer (KML folder).
    pub layer: String,
    /// Text: beacon number or label.
    #[serde(default)]
    pub text: String,
    /// `[easting, northing]`.
    pub point: [f64; 2],
}

/// What is written.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Drawing {
    /// Title (KML document name, EMF description).
    #[serde(default)]
    pub title: String,
    /// Lines and rings.
    #[serde(default)]
    pub shapes: Vec<Shape>,
    /// Corners and labels.
    #[serde(default)]
    pub labels: Vec<Label>,
}

impl Drawing {
    /// Why this cannot be written, if so.
    pub fn problem(&self) -> Option<&'static str> {
        let n: usize = self.shapes.iter().map(|s| s.points.len()).sum::<usize>() + self.labels.len();
        let mut all = self.coordinates();
        if n == 0 {
            Some("there is nothing to export")
        } else if n > 200_000 {
            Some("the drawing is too large to export (over 200 000 points)")
        } else if !all.all(|p| p.iter().all(|v| v.is_finite() && v.abs() < 1e8)) {
            Some("the drawing has coordinates that are not numbers")
        } else {
            None
        }
    }

    fn coordinates(&self) -> impl Iterator<Item = [f64; 2]> + '_ {
        self.shapes.iter().flat_map(|s| s.points.iter().copied()).chain(self.labels.iter().map(|l| l.point))
    }

    fn layers(&self) -> Vec<&str> {
        let mut v: Vec<&str> = Vec::new();
        for l in self.shapes.iter().map(|s| s.layer.as_str()).chain(self.labels.iter().map(|l| l.layer.as_str())) {
            if !v.contains(&l) {
                v.push(l);
            }
        }
        v
    }
}

/// A DXF layer name: no characters AutoCAD refuses.
fn dxf_layer(s: &str) -> String {
    let t: String = s.trim().chars().take(60).map(|c| if c.is_ascii_alphanumeric() || "_-$".contains(c) { c } else { '_' }).collect();
    if t.is_empty() { "0".into() } else { t }
}

/// DXF text on one line; characters beyond ASCII in AutoCAD's `\U+XXXX` form.
fn dxf_text(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars().take(250) {
        if c.is_control() {
            out.push(' ');
        } else if c.is_ascii() {
            out.push(c);
        } else {
            for u in c.encode_utf16(&mut [0; 2]) {
                out.push_str(&format!("\\U+{u:04X}"));
            }
        }
    }
    out
}

/// AutoCAD colour numbers for successive layers.
const ACI: [i32; 7] = [1, 5, 3, 6, 4, 30, 7];
const RGB: [u32; 7] = [0x0000FF, 0xFF0000, 0x007F00, 0xC000C0, 0x909000, 0x0060C0, 0x000000]; // 0x00BBGGRR

/// An ASCII DXF (R12, which every CAD program reads).
pub fn dxf(d: &Drawing) -> String {
    let mut o = String::new();
    let mut g = |c: i32, v: &str| {
        o.push_str(&format!("{c}\n{v}\n"));
    };
    g(0, "SECTION"); g(2, "HEADER"); g(9, "$ACADVER"); g(1, "AC1009"); g(0, "ENDSEC");
    let mut names: Vec<String> = Vec::new();
    for l in d.layers() {
        let n = dxf_layer(l);
        if n != "0" && !names.contains(&n) {
            names.push(n);
        }
    }
    g(0, "SECTION"); g(2, "TABLES");
    g(0, "TABLE"); g(2, "LTYPE"); g(70, "1");
    g(0, "LTYPE"); g(2, "CONTINUOUS"); g(70, "0"); g(3, "Solid line"); g(72, "65"); g(73, "0"); g(40, "0.0");
    g(0, "ENDTAB");
    g(0, "TABLE"); g(2, "LAYER"); g(70, &(names.len() + 1).to_string());
    g(0, "LAYER"); g(2, "0"); g(70, "0"); g(62, "7"); g(6, "CONTINUOUS");
    for (i, n) in names.iter().enumerate() {
        g(0, "LAYER"); g(2, n); g(70, "0"); g(62, &ACI[i % ACI.len()].to_string()); g(6, "CONTINUOUS");
    }
    g(0, "ENDTAB"); g(0, "ENDSEC");
    g(0, "SECTION"); g(2, "ENTITIES");
    for s in d.shapes.iter().filter(|s| s.points.len() >= 2) {
        let layer = dxf_layer(&s.layer);
        g(0, "POLYLINE"); g(8, &layer); g(66, "1"); g(70, if s.closed { "1" } else { "0" });
        g(10, "0.0"); g(20, "0.0"); g(30, "0.0");
        for p in &s.points {
            g(0, "VERTEX"); g(8, &layer); g(10, &format!("{:.4}", p[0])); g(20, &format!("{:.4}", p[1])); g(30, "0.0");
        }
        g(0, "SEQEND"); g(8, &layer);
    }
    for l in &d.labels {
        let layer = dxf_layer(&l.layer);
        let (x, y) = (format!("{:.4}", l.point[0]), format!("{:.4}", l.point[1]));
        g(0, "POINT"); g(8, &layer); g(10, &x); g(20, &y); g(30, "0.0");
        if !l.text.trim().is_empty() {
            g(0, "TEXT"); g(8, &layer); g(10, &x); g(20, &y); g(30, "0.0"); g(40, "0.5"); g(1, &dxf_text(&l.text));
        }
    }
    g(0, "ENDSEC"); g(0, "EOF");
    o
}

fn esc(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).collect::<String>().replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// A KML document (WGS 84 longitude/latitude), one folder a layer.
pub fn kml(d: &Drawing) -> String {
    let crs = WorkingCrs::utm_zone_32n();
    let ll = |p: &[f64; 2]| {
        let (lat, lon) = crs.unproject_deg(Point::new(p[0], p[1]));
        format!("{lon:.10},{lat:.10},0")
    };
    let mut o = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<kml xmlns=\"http://www.opengis.net/kml/2.2\">\n<Document>\n<name>{}</name>\n\
        <Style id=\"b\"><LineStyle><color>ff0000ff</color><width>2</width></LineStyle><PolyStyle><color>220000ff</color></PolyStyle></Style>\n", esc(&d.title));
    for layer in d.layers() {
        o.push_str(&format!("<Folder>\n<name>{}</name>\n", esc(layer)));
        for s in d.shapes.iter().filter(|s| s.layer == layer && s.points.len() >= 2) {
            let mut c: Vec<String> = s.points.iter().map(ll).collect();
            let ring = s.closed && c.len() >= 3;
            if ring {
                c.push(c[0].clone());
            }
            let c = c.join(" ");
            let geom = if ring {
                format!("<Polygon><outerBoundaryIs><LinearRing><coordinates>{c}</coordinates></LinearRing></outerBoundaryIs></Polygon>")
            } else {
                format!("<LineString><coordinates>{c}</coordinates></LineString>")
            };
            o.push_str(&format!("<Placemark><name>{}</name><styleUrl>#b</styleUrl>{geom}</Placemark>\n", esc(&s.name)));
        }
        for l in d.labels.iter().filter(|l| l.layer == layer) {
            o.push_str(&format!("<Placemark><name>{}</name><Point><coordinates>{}</coordinates></Point></Placemark>\n", esc(&l.text), ll(&l.point)));
        }
        o.push_str("</Folder>\n");
    }
    o.push_str("</Document>\n</kml>\n");
    o
}

/// An EMF picture on a 250 × 180 mm page, north up, to scale, carrying its own placement so this
/// editor reads it back in UTM.
pub fn emf(d: &Drawing) -> Vec<u8> {
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for [x, y] in d.coordinates() {
        (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
    }
    let pad = ((x1 - x0).max(y1 - y0) * 0.08).max(1.0);
    (x0, y0, x1, y1) = (x0 - pad, y0 - pad, x1 + pad, y1 + pad);
    let (gw, gh) = (x1 - x0, y1 - y0);
    // Device units are 0.01 mm of page; the drawing (window) is in ground millimetres, coarser only
    // when the extent would pass 32-bit metafile limits (over 100 km).
    let unit = 0.001f64.max(gw.max(gh) / 1e8);
    let dev = (25_000.0 / gw).min(18_000.0 / gh); // device units a ground metre
    let (dw, dh) = ((gw * dev).round().max(1.0), (gh * dev).round().max(1.0));
    let (ww, wh) = ((gw / unit).round().max(1.0), (gh / unit).round().max(1.0));
    let at = |p: &[f64; 2]| [((p[0] - x0) / unit).round() as i32, ((y1 - p[1]) / unit).round() as i32];

    let mut recs: Vec<Vec<u8>> = Vec::new();
    let mut add = |kind: u32, body: Vec<u8>| {
        let mut v = kind.to_le_bytes().to_vec();
        let padded = body.len().div_ceil(4) * 4;
        v.extend((8 + padded as u32).to_le_bytes());
        v.extend(&body);
        v.resize(8 + padded, 0);
        recs.push(v);
    };
    let ints = |v: &[i32]| -> Vec<u8> { v.iter().flat_map(|x| x.to_le_bytes()).collect() };
    let mut note = Vec::new();
    let body: Vec<u8> = GEOREF_TAG.iter().copied()
        .chain([ww * unit / dw, 0.0, x0, 0.0, -wh * unit / dh, y1].iter().flat_map(|v| v.to_le_bytes())).collect();
    note.extend((body.len() as u32).to_le_bytes());
    note.extend(body);
    add(70, note);
    add(17, ints(&[8])); // MM_ANISOTROPIC
    add(9, ints(&[ww as i32, wh as i32]));
    add(10, ints(&[0, 0]));
    add(11, ints(&[dw as i32, dh as i32]));
    add(12, ints(&[0, 0]));
    add(18, ints(&[1])); // transparent background
    // Objects: font 1, then one pen a layer.
    let mm = (ww / dw * 100.0).max(1.0); // a page millimetre in window units
    let mut font = ints(&[1, -((mm * 3.0).min(1e9) as i32), 0, 0, 0, 400, 0, 0]); // LOGFONTW up to its flag bytes
    let face: Vec<u16> = "Arial".encode_utf16().collect();
    font.resize(4 + 28, 0);
    font.extend(face.iter().flat_map(|c| c.to_le_bytes()));
    font.resize(4 + 92, 0);
    font.resize(4 + 320, 0); // as EXTLOGFONTW, the fixed form readers expect
    add(82, font);
    add(37, ints(&[1]));
    let layers = d.layers();
    for (i, _) in layers.iter().enumerate() {
        let w = (mm * if i == 0 { 0.35 } else { 0.18 }) as i32;
        add(38, ints(&[i as i32 + 2, 0, w, 0, RGB[i % RGB.len()] as i32]));
    }
    for (i, layer) in layers.iter().enumerate() {
        add(37, ints(&[i as i32 + 2]));
        for s in d.shapes.iter().filter(|s| s.layer == *layer && s.points.len() >= 2) {
            let pts: Vec<[i32; 2]> = s.points.iter().map(at).collect();
            let ring = s.closed && pts.len() >= 3;
            let mut b = ints(&[0, 0, dw as i32, dh as i32, pts.len() as i32 + i32::from(ring)]);
            for p in pts.iter().chain(ring.then_some(&pts[0])) {
                b.extend(ints(p));
            }
            add(4, b); // POLYLINE, closed by repeating its start, so it is never filled
        }
        add(24, ints(&[RGB[i % RGB.len()] as i32]));
        for l in d.labels.iter().filter(|l| l.layer == *layer) {
            let (p, r) = (at(&l.point), (mm * 0.5) as i32);
            // A small cross, so the label's point is seen.
            for (a, b) in [([p[0] - r, p[1]], [p[0] + r, p[1]]), ([p[0], p[1] - r], [p[0], p[1] + r])] {
                add(27, ints(&a));
                add(54, ints(&b));
            }
            let text: Vec<u16> = l.text.chars().filter(|c| !c.is_control()).take(250).collect::<String>().encode_utf16().collect();
            if text.is_empty() {
                continue;
            }
            let n = text.len();
            let off_dx = 76 + (n * 2).div_ceil(4) * 4;
            let mut b = ints(&[0, 0, dw as i32, dh as i32, 1]);
            b.extend(1.0f32.to_le_bytes());
            b.extend(1.0f32.to_le_bytes());
            b.extend(ints(&[p[0], p[1], n as i32, 76, 0, 0, 0, -1, -1, off_dx as i32]));
            b.extend(text.iter().flat_map(|c| c.to_le_bytes()));
            b.resize(off_dx - 8, 0);
            b.extend(ints(&vec![(mm * 1.7) as i32; n]));
            add(84, b);
        }
    }
    add(14, ints(&[0, 16, 20]));

    let desc: Vec<u16> = format!("PLAGIS Cadastre\0{}\0\0", d.title.chars().filter(|c| !c.is_control()).take(120).collect::<String>()).encode_utf16().collect();
    let head_len = 88 + (desc.len() * 2).div_ceil(4) * 4;
    let total = head_len + recs.iter().map(Vec::len).sum::<usize>();
    let mut f = ints(&[1, head_len as i32, 0, 0, dw as i32, dh as i32, 0, 0, dw as i32, dh as i32]);
    f.extend(b" EMF");
    f.extend(ints(&[0x1_0000, total as i32, recs.len() as i32 + 1]));
    f.extend((layers.len() as u16 + 2).to_le_bytes()); // handles: 0, the font, the pens
    f.extend([0, 0]);
    f.extend(ints(&[desc.len() as i32, 88, 0, 32_000, 20_000, 320, 200])); // 100 device units a mm
    f.extend(desc.iter().flat_map(|c| c.to_le_bytes()));
    f.resize(head_len, 0);
    for r in recs {
        f.extend(r);
    }
    f
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{dxf as dxf_read, emf as emf_read, kml as kml_read};

    fn sample() -> Drawing {
        Drawing {
            title: "VER <1> & co".into(),
            shapes: vec![
                Shape { layer: "BOUNDARY".into(), name: "Plot 7".into(), closed: true, points: vec![[493_000.123, 1_083_100.456], [493_030.0, 1_083_100.0], [493_030.5, 1_083_131.25]] },
                Shape { layer: "ROAD EDGE".into(), name: String::new(), closed: false, points: vec![[492_990.0, 1_083_090.0], [493_050.0, 1_083_090.0]] },
            ],
            labels: vec![Label { layer: "CORNERS".into(), text: "PB 1234 Ọ".into(), point: [493_000.123, 1_083_100.456] }],
        }
    }
    fn near(a: &Point, b: [f64; 2], tol: f64) {
        assert!((a.x - b[0]).abs() < tol && (a.y - b[1]).abs() < tol, "{a:?} {b:?}");
    }

    #[test]
    fn dxf_reads_back() {
        let r = dxf_read::parse(&dxf(&sample())).unwrap();
        assert_eq!(r.polylines.len(), 2);
        assert_eq!((r.polylines[0].layer.as_str(), r.polylines[0].closed, r.polylines[1].layer.as_str(), r.polylines[1].closed), ("BOUNDARY", true, "ROAD_EDGE", false));
        for (p, q) in r.polylines[0].vertices.iter().zip(&sample().shapes[0].points) {
            near(p, *q, 1e-4);
        }
        assert_eq!(r.texts[0].text, "PB 1234 \\U+1ECC");
    }

    #[test]
    fn kml_reads_back() {
        let r = kml_read::parse(&kml(&sample())).unwrap();
        assert_eq!((r.boundaries[0].id.as_str(), r.boundaries[0].kind.as_str(), r.boundaries[0].closed, r.boundaries[0].ring.len()), ("Plot 7", "BOUNDARY", true, 3));
        for (p, q) in r.boundaries[0].ring.iter().zip(&sample().shapes[0].points) {
            near(p, *q, 1e-3);
        }
        assert!(!r.boundaries[1].closed);
        assert_eq!(r.points[0].id, "PB 1234 Ọ");
    }

    #[test]
    fn emf_reads_back_where_it_lies() {
        let f = emf(&sample());
        assert_eq!(f.len() as u32, u32::from_le_bytes(f[48..52].try_into().unwrap()), "nBytes");
        let r = emf_read::parse(&f).unwrap();
        assert!(r.placed);
        let b = &r.survey.boundaries[0];
        assert_eq!((b.closed, b.ring.len()), (true, 3));
        for (p, q) in b.ring.iter().zip(&sample().shapes[0].points) {
            near(p, *q, 0.002);
        }
        assert_eq!(r.survey.points[0].id, "PB 1234 Ọ");
        near(&r.survey.points[0].point, sample().labels[0].point, 0.002);
        assert!(r.survey.warnings.is_empty(), "{:?}", r.survey.warnings);
    }

    #[test]
    fn what_cannot_be_written_is_named() {
        assert!(Drawing::default().problem().is_some());
        let mut d = sample();
        assert!(d.problem().is_none());
        d.shapes[0].points[0][0] = f64::NAN;
        assert!(d.problem().unwrap().contains("not numbers"));
    }
}
