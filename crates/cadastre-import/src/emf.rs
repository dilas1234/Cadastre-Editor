//! Enhanced Metafile (EMF) import: the lines and labels of a drawing saved as a Windows picture.
//!
//! An EMF holds drawing commands in page units, not ground coordinates, so what is read here must
//! be placed on the ground by known points before it means anything, and is only as accurate as
//! the picture's units. The exception is an EMF this editor wrote: it carries its placement in a
//! comment record ([`GEOREF_TAG`]) and comes back in UTM.
//!
//! Units returned: the metafile's device units, with **y upwards** (as on a map).

use std::collections::{BTreeMap, HashMap};

use cadastre_core::types::Point;

use crate::conversion::{ImportError, ImportFormat, ImportedBoundary, ImportedPoint, ImportedSurvey};

/// Comment record identifying this editor's placement: six little-endian f64 `a b c d e f` follow,
/// `E = a·x + b·y + c`, `N = d·x + e·y + f` for device units with y **down**.
pub const GEOREF_TAG: &[u8; 8] = b"PLAGISG1";

const MAX_VERTICES: usize = 2_000_000;
/// Flattened segments per Bézier curve.
const BEZIER_STEPS: usize = 8;

/// What was read from a metafile.
pub struct EmfDrawing {
    /// Lines and labels; UTM metres when `placed`, else device units (y up).
    pub survey: ImportedSurvey,
    /// Whether the file said where it lies.
    pub placed: bool,
}

type M = [f64; 6]; // x' = m0 x + m2 y + m4 ; y' = m1 x + m3 y + m5
const IDENTITY: M = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
/// `a` applied first, then `b`.
fn mul(a: M, b: M) -> M {
    [
        a[0] * b[0] + a[1] * b[2],
        a[0] * b[1] + a[1] * b[3],
        a[2] * b[0] + a[3] * b[2],
        a[2] * b[1] + a[3] * b[3],
        a[4] * b[0] + a[5] * b[2] + b[4],
        a[4] * b[1] + a[5] * b[3] + b[5],
    ]
}

#[derive(Clone)]
struct Dc {
    world: M,
    mode: u32,
    worg: (f64, f64),
    wext: (f64, f64),
    vorg: (f64, f64),
    vext: (f64, f64),
    pen: String,
    cur: (f64, f64),
}

impl Dc {
    fn device(&self, (x, y): (f64, f64)) -> (f64, f64) {
        let w = self.world;
        let (px, py) = (w[0] * x + w[2] * y + w[4], w[1] * x + w[3] * y + w[5]);
        let (sx, sy) = match self.mode {
            7 | 8 => (self.vext.0 / self.wext.0, self.vext.1 / self.wext.1),
            2..=6 => (1.0, -1.0), // metric / English modes: y upwards; placing settles the scale
            _ => (1.0, 1.0),
        };
        ((px - self.worg.0) * sx + self.vorg.0, (py - self.worg.1) * sy + self.vorg.1)
    }
}

/// Bounds-checked little-endian reads within one record.
struct Rec<'a>(&'a [u8]);
impl Rec<'_> {
    fn u32(&self, at: usize) -> Option<u32> {
        Some(u32::from_le_bytes(self.0.get(at..at.checked_add(4)?)?.try_into().ok()?))
    }
    fn i32(&self, at: usize) -> Option<f64> {
        self.u32(at).map(|v| f64::from(v as i32))
    }
    fn i16(&self, at: usize) -> Option<f64> {
        Some(f64::from(i16::from_le_bytes(self.0.get(at..at.checked_add(2)?)?.try_into().ok()?)))
    }
    fn f32(&self, at: usize) -> Option<f64> {
        self.u32(at).map(|v| f64::from(f32::from_bits(v)))
    }
    fn pair(&self, at: usize) -> Option<(f64, f64)> {
        Some((self.i32(at)?, self.i32(at + 4)?))
    }
    fn matrix(&self, at: usize) -> Option<M> {
        let mut m = IDENTITY;
        for (k, v) in m.iter_mut().enumerate() {
            *v = self.f32(at + k * 4)?;
        }
        Some(m)
    }
    /// `count` points from `at`, 16- or 32-bit. `None` when the record is too short for them.
    fn points(&self, at: usize, count: usize, short: bool) -> Option<Vec<(f64, f64)>> {
        let size = if short { 4 } else { 8 };
        if at.checked_add(count.checked_mul(size)?)? > self.0.len() {
            return None;
        }
        (0..count)
            .map(|i| {
                let p = at + i * size;
                if short { Some((self.i16(p)?, self.i16(p + 2)?)) } else { self.pair(p) }
            })
            .collect()
    }
}

#[derive(Default)]
struct Out {
    lines: Vec<(String, Vec<(f64, f64)>, bool)>,
    texts: Vec<(String, (f64, f64), String)>,
    vertices: usize,
    figure: Vec<(f64, f64)>, // the open move/line figure, in device units
    figure_pen: String,
}

impl Out {
    fn add(&mut self, pen: &str, mut pts: Vec<(f64, f64)>, mut closed: bool) {
        pts.dedup();
        if pts.len() > 3 && pts.first() == pts.last() {
            pts.pop();
            closed = true;
        }
        if pts.len() < 2 {
            return;
        }
        self.vertices += pts.len();
        let closed = closed && pts.len() > 2;
        self.lines.push((pen.to_string(), pts, closed));
    }
    fn flush(&mut self, closed: bool) {
        let pts = std::mem::take(&mut self.figure);
        let pen = std::mem::take(&mut self.figure_pen);
        self.add(&pen, pts, closed);
    }
    /// Continue the open figure from the current position.
    fn line_to(&mut self, dc: &mut Dc, pts: &[(f64, f64)]) {
        if self.figure.is_empty() || self.figure_pen != dc.pen {
            self.flush(false);
            self.figure.push(dc.device(dc.cur));
            self.figure_pen = dc.pen.clone();
        }
        for p in pts {
            self.figure.push(dc.device(*p));
            dc.cur = *p;
        }
    }
}

/// Points along cubic Béziers: `start`, then triples (two controls and an end).
fn bezier(start: (f64, f64), triples: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let mut out = Vec::new();
    let mut p0 = start;
    for c in triples.chunks_exact(3) {
        for s in 1..=BEZIER_STEPS {
            let t = s as f64 / BEZIER_STEPS as f64;
            let u = 1.0 - t;
            let f = |a: f64, b: f64, cc: f64, d: f64| u * u * u * a + 3.0 * u * u * t * b + 3.0 * u * t * t * cc + t * t * t * d;
            out.push(if s == BEZIER_STEPS { c[2] } else { (f(p0.0, c[0].0, c[1].0, c[2].0), f(p0.1, c[0].1, c[1].1, c[2].1)) });
        }
        p0 = c[2];
    }
    out
}

fn colour(c: u32) -> String {
    format!("pen #{:02X}{:02X}{:02X}", c & 0xFF, (c >> 8) & 0xFF, (c >> 16) & 0xFF)
}

/// Whether the bytes start as an EMF does.
pub fn is_emf(bytes: &[u8]) -> bool {
    bytes.len() >= 88 && bytes[..4] == [1, 0, 0, 0] && &bytes[40..44] == b" EMF"
}

/// Read a metafile. The pen's colour is used as the layer, since an EMF has no layers.
pub fn parse(bytes: &[u8]) -> Result<EmfDrawing, ImportError> {
    if !is_emf(bytes) {
        let wmf = bytes.starts_with(&[0xD7, 0xCD, 0xC6, 0x9A]);
        return Err(ImportError::Invalid(if wmf { "this is an old WMF picture; save it as EMF".into() } else { "not an EMF picture".to_string() }));
    }
    let new_dc = Dc { world: IDENTITY, mode: 1, worg: (0.0, 0.0), wext: (1.0, 1.0), vorg: (0.0, 0.0), vext: (1.0, 1.0), pen: colour(0), cur: (0.0, 0.0) };
    let mut dc = new_dc;
    let mut saved: Vec<Dc> = Vec::new();
    let mut pens: HashMap<u32, String> = HashMap::new();
    let mut out = Out::default();
    let (mut pictures, mut arcs, mut plus, mut cut) = (0usize, 0usize, false, true);
    let mut georef: Option<M> = None;
    let mut at = 0usize;
    while let Some(head) = bytes.get(at..at + 8) {
        let kind = u32::from_le_bytes([head[0], head[1], head[2], head[3]]);
        let size = u32::from_le_bytes([head[4], head[5], head[6], head[7]]) as usize;
        let Some(end) = at.checked_add(size).filter(|e| size >= 8 && *e <= bytes.len()) else { break };
        let r = Rec(&bytes[at..end]);
        at = end;
        if kind == 14 {
            cut = false;
            break;
        }
        if out.vertices > MAX_VERTICES {
            return Err(ImportError::Invalid("the EMF has too many vertices".into()));
        }
        // A record too short for what it says is skipped.
        let _ = (|| -> Option<()> {
            match kind {
                9 => { let v = r.pair(8)?; if v.0 != 0.0 && v.1 != 0.0 { dc.wext = v; } }
                10 => dc.worg = r.pair(8)?,
                11 => { let v = r.pair(8)?; if v.0 != 0.0 && v.1 != 0.0 { dc.vext = v; } }
                12 => dc.vorg = r.pair(8)?,
                17 => dc.mode = r.u32(8)?,
                33 => if saved.len() < 1000 { saved.push(dc.clone()) },
                34 => {
                    let n = r.i32(8)?;
                    let keep = if n < 0.0 { saved.len() as f64 + n } else { n - 1.0 };
                    if keep >= 0.0 && (keep as usize) < saved.len() {
                        out.flush(false);
                        saved.truncate(keep as usize + 1);
                        dc = saved.pop()?;
                    }
                }
                35 => dc.world = r.matrix(8)?,
                36 => {
                    let m = r.matrix(8)?;
                    match r.u32(32)? {
                        1 => dc.world = IDENTITY,
                        2 => dc.world = mul(m, dc.world),
                        3 => dc.world = mul(dc.world, m),
                        _ => dc.world = m,
                    }
                }
                37 => {
                    let i = r.u32(8)?;
                    if let Some(p) = pens.get(&i) { dc.pen = p.clone(); }
                    match i { 0x8000_0006 => dc.pen = colour(0xFF_FFFF), 0x8000_0007 => dc.pen = colour(0), _ => {} }
                }
                38 => { pens.insert(r.u32(8)?, colour(r.u32(24)?)); }
                95 => { pens.insert(r.u32(8)?, colour(r.u32(40)?)); }
                40 => { pens.remove(&r.u32(8)?); }
                27 => { out.flush(false); dc.cur = r.pair(8)?; }
                54 => { let p = r.pair(8)?; out.line_to(&mut dc, &[p]); }
                59 | 60 | 68 => out.flush(false),
                61 => out.flush(true),
                // One figure: bounds (16 bytes), count, points.
                2..=6 | 85..=89 => {
                    let short = kind >= 85;
                    let what = if short { kind - 83 } else { kind }; // as the 32-bit record
                    let n = r.u32(24)? as usize;
                    let p = r.points(28, n, short)?;
                    let first = *p.first()?;
                    match what {
                        2 => { out.flush(false); let mut v = vec![first]; v.extend(bezier(first, &p[1..])); out.add(&dc.pen, v.iter().map(|q| dc.device(*q)).collect(), false); }
                        3 | 4 => { out.flush(false); out.add(&dc.pen, p.iter().map(|q| dc.device(*q)).collect(), what == 3); }
                        5 => { let v = bezier(dc.cur, &p); out.line_to(&mut dc, &v); }
                        _ => out.line_to(&mut dc, &p),
                    }
                }
                // Several figures: bounds, figure count, total, counts, points.
                7 | 8 | 90 | 91 => {
                    out.flush(false);
                    let short = kind >= 90;
                    let (figures, total) = (r.u32(24)? as usize, r.u32(28)? as usize);
                    let start = 32usize.checked_add(figures.checked_mul(4)?)?;
                    let p = r.points(start, total, short)?;
                    let mut from = 0usize;
                    for k in 0..figures {
                        let n = r.u32(32 + k * 4)? as usize;
                        let part = p.get(from..from.checked_add(n)?)?;
                        from += n;
                        out.add(&dc.pen, part.iter().map(|q| dc.device(*q)).collect(), kind == 8 || kind == 91);
                    }
                }
                42 => {
                    let ((x0, y0), (x1, y1)) = (r.pair(8)?, r.pair(16)?);
                    let (cx, cy, rx, ry) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0, (x1 - x0) / 2.0, (y1 - y0) / 2.0);
                    let ring = (0..64).map(|k| { let a = std::f64::consts::TAU * f64::from(k) / 64.0; dc.device((cx + rx * a.cos(), cy + ry * a.sin())) }).collect();
                    out.add(&dc.pen, ring, true);
                }
                43 => {
                    let ((x0, y0), (x1, y1)) = (r.pair(8)?, r.pair(16)?);
                    out.add(&dc.pen, [(x0, y0), (x1, y0), (x1, y1), (x0, y1)].iter().map(|q| dc.device(*q)).collect(), true);
                }
                41 | 44..=47 | 55 => arcs += 1,
                70 => {
                    let n = r.u32(8)? as usize;
                    let data = r.0.get(12..12usize.checked_add(n)?)?;
                    if data.starts_with(b"EMF+") { plus = true; }
                    if let Some(v) = data.strip_prefix(GEOREF_TAG.as_slice()).filter(|v| v.len() >= 48) {
                        let mut m = [0.0; 6];
                        for (k, q) in m.iter_mut().enumerate() {
                            *q = f64::from_le_bytes(v[k * 8..k * 8 + 8].try_into().ok()?);
                        }
                        if m.iter().all(|q| q.is_finite()) { georef = Some(m); }
                    }
                }
                76..=81 | 114 | 116 => pictures += 1,
                83 | 84 => {
                    let p = r.pair(36)?;
                    let (n, off) = (r.u32(44)? as usize, r.u32(48)? as usize);
                    let width = if kind == 84 { 2 } else { 1 };
                    let raw = r.0.get(off..off.checked_add(n.checked_mul(width)?)?)?;
                    let glyphs = r.u32(52)? & 0x10 != 0; // ETO_GLYPH_INDEX: not characters
                    let text = if kind == 84 {
                        String::from_utf16_lossy(&raw.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect::<Vec<_>>())
                    } else {
                        raw.iter().map(|b| char::from(*b)).collect()
                    };
                    let text = text.trim_matches(|c: char| c.is_whitespace() || c.is_control()).to_string();
                    if !text.is_empty() && !glyphs { out.texts.push((dc.pen.clone(), dc.device(p), text)); }
                }
                _ => {}
            }
            Some(())
        })();
    }
    out.flush(false);
    if out.lines.is_empty() && out.texts.is_empty() {
        return Err(ImportError::Invalid(if pictures > 0 {
            "the EMF holds only an image, no lines: save the image as TIFF or BMP and import that".into()
        } else if plus {
            "the EMF is in the EMF+ form only, which is not read: save it as plain EMF, DXF or DWG".into()
        } else {
            "the EMF has no lines or text".to_string()
        }));
    }
    let place = |(x, y): (f64, f64)| match georef {
        Some(m) => Point::new(m[0] * x + m[1] * y + m[2], m[3] * x + m[4] * y + m[5]),
        None => Point::new(x, -y),
    };
    let mut s = ImportedSurvey { format: Some(ImportFormat::Emf), ..Default::default() };
    for (i, (pen, pts, closed)) in out.lines.into_iter().enumerate() {
        let ring: Vec<Point> = pts.into_iter().map(place).filter(|p| p.x.is_finite() && p.y.is_finite()).collect();
        if ring.len() >= 2 {
            s.boundaries.push(ImportedBoundary { id: format!("LINE{}", i + 1), closed: closed && ring.len() > 2, kind: pen, ring, stated_area: None });
        }
    }
    for (pen, p, text) in out.texts {
        let point = place(p);
        if point.x.is_finite() && point.y.is_finite() {
            s.points.push(ImportedPoint { id: text, point, elevation: None, code: Some(format!("TEXT:{pen}")), attributes: BTreeMap::new() });
        }
    }
    for (n, what) in [(pictures, "image(s) inside the EMF were left out"), (arcs, "arc(s) inside the EMF were left out")] {
        if n > 0 {
            s.warnings.push(format!("{n} {what}"));
        }
    }
    if cut {
        s.warnings.push("the EMF ends early; what was readable is shown".into());
    }
    if georef.is_some() {
        s.crs = Some("EPSG:32632".into());
        s.epsg = Some(32632);
    }
    Ok(EmfDrawing { survey: s, placed: georef.is_some() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(kind: u32, body: &[u8]) -> Vec<u8> {
        let mut v = kind.to_le_bytes().to_vec();
        v.extend((8 + body.len() as u32).to_le_bytes());
        v.extend(body);
        v
    }
    fn ints(v: &[i32]) -> Vec<u8> {
        v.iter().flat_map(|x| x.to_le_bytes()).collect()
    }
    fn file(records: &[Vec<u8>]) -> Vec<u8> {
        let mut h = vec![0u8; 80];
        h[32..36].copy_from_slice(b" EMF");
        let mut f = rec(1, &h);
        for r in records {
            f.extend(r);
        }
        f.extend(rec(14, &[0; 12]));
        f
    }
    fn poly16(kind: u32, pts: &[(i16, i16)]) -> Vec<u8> {
        let mut b = vec![0u8; 16];
        b.extend((pts.len() as u32).to_le_bytes());
        for (x, y) in pts {
            b.extend(x.to_le_bytes());
            b.extend(y.to_le_bytes());
        }
        rec(kind, &b)
    }

    #[test]
    fn lines_figures_and_the_page_mapping() {
        let f = file(&[
            rec(17, &ints(&[8])), rec(9, &ints(&[100, 100])), rec(11, &ints(&[200, 200])), rec(10, &ints(&[10, 0])),
            rec(38, &ints(&[1, 0, 0, 0, 0x0000FF])), rec(37, &ints(&[1])),
            poly16(86, &[(10, 0), (20, 0), (20, 10)]),
            rec(33, &[]), rec(35, &[2.0f32, 0.0, 0.0, 2.0, 0.0, 0.0].iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<_>>()),
            rec(27, &ints(&[5, 5])), rec(54, &ints(&[15, 5])), rec(54, &ints(&[15, 15])), rec(54, &ints(&[5, 5])),
            rec(34, &ints(&[-1])),
            poly16(87, &[(10, 10), (30, 10)]),
        ]);
        let d = parse(&f).unwrap();
        assert!(!d.placed);
        let b = &d.survey.boundaries;
        assert_eq!(b.len(), 3, "{b:?}");
        assert_eq!((b[0].kind.as_str(), b[0].closed), ("pen #FF0000", true));
        assert_eq!(b[0].ring, vec![Point::new(0.0, 0.0), Point::new(20.0, 0.0), Point::new(20.0, -20.0)]);
        assert!(!b[1].closed || b[1].ring.len() == 3);
        assert_eq!(b[1].ring[0], Point::new(0.0, -20.0), "world transform, then the window");
        assert_eq!(b[2].ring, vec![Point::new(0.0, -20.0), Point::new(40.0, -20.0)], "restored");
        assert!(d.survey.warnings.is_empty(), "{:?}", d.survey.warnings);
    }

    #[test]
    fn damaged_or_empty_files_are_refused_not_a_crash() {
        assert!(parse(b"hello").is_err());
        assert!(parse(&file(&[])).err().unwrap().to_string().contains("no lines"));
        assert!(parse(&file(&[rec(81, &[0; 80])])).err().unwrap().to_string().contains("TIFF"));
        let whole = file(&[poly16(86, &[(0, 0), (5, 0), (5, 5)]), rec(7, &ints(&[0, 0, 0, 0, -1, -1])), rec(84, &ints(&[-1; 20])), rec(87, &[255; 24])]);
        for cut in 0..whole.len() {
            let _ = parse(&whole[..cut]);
        }
        assert_eq!(parse(&whole).unwrap().survey.boundaries.len(), 1);
    }
}
