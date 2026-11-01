//! ASCII DXF import: survey points, lines and boundary polylines (SRS CAD import).
//!
//! Only the `ENTITIES` section is interpreted. Coordinates are read in drawing units and are
//! **not** transformed: the caller must state the drawing's CRS/units or georeference it. Block
//! references (`INSERT`) are reported, not exploded, because exploding inside a server request
//! would make the result depend on block-table ordering; the caller can flatten the drawing in
//! CAD first.
//!
//! Supported entities: `POINT`, `LINE`, `LWPOLYLINE`, `POLYLINE`/`VERTEX`/`SEQEND`, `CIRCLE`,
//! `ARC` and the insertion point of `TEXT`/`MTEXT`/`INSERT`. Binary DXF is rejected with a clear
//! message rather than misparsed.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use cadastre_core::types::Point;
use serde::{Deserialize, Serialize};

/// A parsed DXF point entity.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DxfPoint {
    /// Layer name.
    pub layer: String,
    /// Entity handle, when present.
    pub handle: Option<String>,
    /// Coordinate.
    pub point: Point,
    /// Elevation.
    pub z: f64,
}

/// A parsed DXF line entity.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DxfLine {
    /// Layer name.
    pub layer: String,
    /// Start.
    pub from: Point,
    /// End.
    pub to: Point,
}

/// A parsed DXF polyline (lightweight or classic).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DxfPolyline {
    /// Layer name.
    pub layer: String,
    /// Vertices in order.
    pub vertices: Vec<Point>,
    /// Whether the polyline closes back on its first vertex.
    pub closed: bool,
    /// Per-vertex bulges (0 for a straight segment), when present.
    pub bulges: Vec<f64>,
}

/// A parsed DXF circle.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DxfCircle {
    /// Layer name.
    pub layer: String,
    /// Centre.
    pub centre: Point,
    /// Radius in drawing units.
    pub radius: f64,
}

/// A parsed DXF arc.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DxfArc {
    /// Layer name.
    pub layer: String,
    /// Centre.
    pub centre: Point,
    /// Radius in drawing units.
    pub radius: f64,
    /// Start angle (decimal degrees, anticlockwise from east).
    pub start_deg: f64,
    /// End angle (decimal degrees, anticlockwise from east).
    pub end_deg: f64,
}

/// A text label and its insertion point.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DxfText {
    /// Layer name.
    pub layer: String,
    /// Insertion point.
    pub point: Point,
    /// Text content.
    pub text: String,
}

/// Everything extracted from one DXF file.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DxfDrawing {
    /// Point entities.
    pub points: Vec<DxfPoint>,
    /// Line entities.
    pub lines: Vec<DxfLine>,
    /// Polyline entities.
    pub polylines: Vec<DxfPolyline>,
    /// Circles.
    pub circles: Vec<DxfCircle>,
    /// Arcs.
    pub arcs: Vec<DxfArc>,
    /// Text labels.
    pub texts: Vec<DxfText>,
    /// Layer names declared or used.
    pub layers: Vec<String>,
    /// Entity counts by type.
    pub counts: BTreeMap<String, usize>,
    /// Non-fatal findings.
    pub warnings: Vec<String>,
}

impl DxfDrawing {
    /// Total number of interpreted entities.
    pub fn entity_count(&self) -> usize {
        self.points.len()
            + self.lines.len()
            + self.polylines.len()
            + self.circles.len()
            + self.arcs.len()
            + self.texts.len()
    }

    /// Closed polylines that can be used as parcel boundaries.
    pub fn closed_boundaries(&self) -> Vec<&DxfPolyline> {
        self.polylines
            .iter()
            .filter(|p| p.closed && p.vertices.len() >= 3)
            .collect()
    }
}

/// Why a DXF file could not be parsed.
#[derive(Clone, Debug, PartialEq)]
pub enum DxfError {
    /// The file is binary DXF, not ASCII.
    Binary,
    /// A group-code line was not an integer.
    BadGroupCode {
        /// One-based line number.
        line: usize,
        /// The offending text.
        text: String,
    },
    /// The file ended in the middle of a group pair.
    Truncated,
}

impl fmt::Display for DxfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DxfError::Binary => write!(
                f,
                "binary DXF is not supported; save the drawing as ASCII DXF"
            ),
            DxfError::BadGroupCode { line, text } => {
                write!(f, "DXF line {line} is not a group code: `{text}`")
            }
            DxfError::Truncated => write!(f, "DXF ended in the middle of a group pair"),
        }
    }
}

impl std::error::Error for DxfError {}

/// One DXF group-code/value pair.
#[derive(Clone, Debug, PartialEq)]
struct Pair {
    code: i32,
    value: String,
}

impl Pair {
    fn text(&self) -> &str {
        self.value.trim()
    }

    fn number(&self) -> Option<f64> {
        self.text().parse::<f64>().ok().filter(|v| v.is_finite())
    }

    fn int(&self) -> Option<i32> {
        self.text().parse::<i32>().ok()
    }
}

fn parse_pairs(contents: &str) -> Result<Vec<Pair>, DxfError> {
    if contents.starts_with("AutoCAD Binary DXF") {
        return Err(DxfError::Binary);
    }
    let mut lines = contents.lines().enumerate();
    let mut pairs = Vec::new();
    loop {
        let Some((line_no, code_line)) = lines.next() else {
            break;
        };
        if code_line.trim().is_empty() {
            continue;
        }
        let code = code_line
            .trim()
            .parse::<i32>()
            .map_err(|_| DxfError::BadGroupCode {
                line: line_no + 1,
                text: code_line.to_string(),
            })?;
        let Some((_, value)) = lines.next() else {
            return Err(DxfError::Truncated);
        };
        pairs.push(Pair {
            code,
            value: value.trim_end_matches('\r').to_string(),
        });
    }
    Ok(pairs)
}

/// An entity: its type name and its group pairs.
struct Entity<'a> {
    kind: String,
    pairs: &'a [Pair],
}

impl Entity<'_> {
    fn value(&self, code: i32) -> Option<&str> {
        self.pairs
            .iter()
            .rev()
            .find(|p| p.code == code)
            .map(Pair::text)
    }

    fn number(&self, code: i32) -> Option<f64> {
        self.pairs
            .iter()
            .rev()
            .find(|p| p.code == code)
            .and_then(Pair::number)
    }

    fn int(&self, code: i32) -> Option<i32> {
        self.pairs
            .iter()
            .rev()
            .find(|p| p.code == code)
            .and_then(Pair::int)
    }

    fn layer(&self) -> String {
        self.value(8).unwrap_or("0").to_string()
    }

    fn handle(&self) -> Option<String> {
        self.value(5).map(str::to_string)
    }

    fn xy(&self, x_code: i32, y_code: i32) -> Option<Point> {
        Some(Point::new(self.number(x_code)?, self.number(y_code)?))
    }
}

fn split_entities(pairs: &[Pair]) -> Vec<Entity<'_>> {
    let mut entities = Vec::new();
    let mut current: Option<(String, usize)> = None;
    for (i, pair) in pairs.iter().enumerate() {
        if pair.code == 0 {
            if let Some((kind, start)) = current.take() {
                entities.push(Entity {
                    kind,
                    pairs: &pairs[start..i],
                });
            }
            current = Some((pair.text().to_string(), i + 1));
        }
    }
    if let Some((kind, start)) = current {
        entities.push(Entity {
            kind,
            pairs: &pairs[start..],
        });
    }
    entities
}

/// Read the `ENTITIES` section, falling back to every entity when the section markers are absent.
fn entity_pairs(pairs: &[Pair]) -> &[Pair] {
    let mut start = None;
    let mut end = pairs.len();
    let mut i = 0;
    while i + 1 < pairs.len() {
        if pairs[i].code == 0 && pairs[i].text() == "SECTION" {
            let is_entities = pairs[i + 1].code == 2 && pairs[i + 1].text() == "ENTITIES";
            if is_entities {
                start = Some(i + 2);
                let mut j = i + 2;
                while j + 1 < pairs.len() {
                    if pairs[j].code == 0 && pairs[j].text() == "ENDSEC" {
                        end = j;
                        break;
                    }
                    j += 1;
                }
                break;
            }
        }
        i += 1;
    }
    match start {
        Some(s) => &pairs[s..end],
        None => pairs,
    }
}

/// Parse an ASCII DXF document.
pub fn parse(contents: &str) -> Result<DxfDrawing, DxfError> {
    let pairs = parse_pairs(contents)?;
    let entities = split_entities(entity_pairs(&pairs));
    let mut drawing = DxfDrawing::default();
    let mut layers = BTreeSet::new();
    let mut unsupported: BTreeMap<String, usize> = BTreeMap::new();

    let mut i = 0;
    while i < entities.len() {
        let entity = &entities[i];
        *drawing.counts.entry(entity.kind.clone()).or_insert(0) += 1;
        let layer = entity.layer();
        if !layer.is_empty() {
            layers.insert(layer.clone());
        }

        match entity.kind.as_str() {
            "POINT" => {
                if let Some(point) = entity.xy(10, 20) {
                    drawing.points.push(DxfPoint {
                        layer,
                        handle: entity.handle(),
                        point,
                        z: entity.number(30).unwrap_or(0.0),
                    });
                } else {
                    drawing
                        .warnings
                        .push("a POINT entity has no 10/20 coordinates".to_string());
                }
            }
            "LINE" => match (entity.xy(10, 20), entity.xy(11, 21)) {
                (Some(from), Some(to)) => drawing.lines.push(DxfLine { layer, from, to }),
                _ => drawing
                    .warnings
                    .push("a LINE entity is missing an endpoint".to_string()),
            },
            "LWPOLYLINE" => {
                let mut vertices = Vec::new();
                let mut bulges = Vec::new();
                let mut pending: Option<(f64, f64)> = None;
                for pair in entity.pairs {
                    match pair.code {
                        10 => {
                            if let Some(v) = pending.take() {
                                vertices.push(Point::new(v.0, v.1));
                            }
                            if let Some(x) = pair.number() {
                                pending = Some((x, 0.0));
                            }
                        }
                        20 => {
                            if let (Some((x, _)), Some(y)) = (pending, pair.number()) {
                                pending = Some((x, y));
                            }
                        }
                        42 => {
                            if let Some(v) = pending.take() {
                                vertices.push(Point::new(v.0, v.1));
                            }
                            bulges.push(pair.number().unwrap_or(0.0));
                        }
                        _ => {}
                    }
                }
                if let Some(v) = pending.take() {
                    vertices.push(Point::new(v.0, v.1));
                }
                bulges.resize(vertices.len(), 0.0);
                let closed = entity.int(70).unwrap_or(0) & 1 == 1;
                drawing.polylines.push(DxfPolyline {
                    layer,
                    vertices,
                    closed,
                    bulges,
                });
            }
            "POLYLINE" => {
                // Collect the following VERTEX entities until SEQEND.
                let flags = entity.int(70).unwrap_or(0);
                let closed = flags & 1 == 1;
                let mut vertices = Vec::new();
                let mut bulges = Vec::new();
                let mut j = i + 1;
                while j < entities.len() && entities[j].kind == "VERTEX" {
                    if let Some(p) = entities[j].xy(10, 20) {
                        vertices.push(p);
                        bulges.push(entities[j].number(42).unwrap_or(0.0));
                    }
                    j += 1;
                }
                if j < entities.len() && entities[j].kind == "SEQEND" {
                    j += 1;
                }
                drawing.polylines.push(DxfPolyline {
                    layer,
                    vertices,
                    closed,
                    bulges,
                });
                i = j;
                continue;
            }
            "VERTEX" | "SEQEND" => {}
            "CIRCLE" => match (entity.xy(10, 20), entity.number(40)) {
                (Some(centre), Some(radius)) => drawing.circles.push(DxfCircle {
                    layer,
                    centre,
                    radius,
                }),
                _ => drawing
                    .warnings
                    .push("a CIRCLE entity is missing its centre or radius".to_string()),
            },
            "ARC" => match (entity.xy(10, 20), entity.number(40)) {
                (Some(centre), Some(radius)) => drawing.arcs.push(DxfArc {
                    layer,
                    centre,
                    radius,
                    start_deg: entity.number(50).unwrap_or(0.0),
                    end_deg: entity.number(51).unwrap_or(360.0),
                }),
                _ => drawing
                    .warnings
                    .push("an ARC entity is missing its centre or radius".to_string()),
            },
            "TEXT" | "MTEXT" | "ATTRIB" => {
                if let Some(point) = entity.xy(10, 20) {
                    drawing.texts.push(DxfText {
                        layer,
                        point,
                        text: entity.value(1).unwrap_or("").to_string(),
                    });
                }
            }
            "INSERT" => {
                let block = entity.value(2).unwrap_or("?").to_string();
                drawing.warnings.push(format!(
                    "block reference `{block}` on layer `{layer}` was not exploded; flatten it in CAD before import"
                ));
            }
            "HATCH" => drawing
                .warnings
                .push(format!("hatch on layer `{layer}` ignored")),
            "DIMENSION" => drawing
                .warnings
                .push(format!("dimension on layer `{layer}` ignored")),
            other => {
                *unsupported.entry(other.to_string()).or_insert(0) += 1;
            }
        }
        i += 1;
    }

    for (kind, count) in unsupported {
        drawing
            .warnings
            .push(format!("{count} unsupported `{kind}` entities ignored"));
    }
    drawing.layers = layers.into_iter().collect();
    Ok(drawing)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair(code: i32, value: &str) -> String {
        format!("{code}\n{value}\n")
    }

    fn doc(entities: &str) -> String {
        let mut s = String::new();
        s.push_str(&pair(0, "SECTION"));
        s.push_str(&pair(2, "HEADER"));
        s.push_str(&pair(0, "ENDSEC"));
        s.push_str(&pair(0, "SECTION"));
        s.push_str(&pair(2, "ENTITIES"));
        s.push_str(entities);
        s.push_str(&pair(0, "ENDSEC"));
        s.push_str(&pair(0, "EOF"));
        s
    }

    #[test]
    fn parses_points_lines_and_layers() {
        let mut e = String::new();
        e.push_str(&pair(0, "POINT"));
        e.push_str(&pair(8, "BEACONS"));
        e.push_str(&pair(5, "A1"));
        e.push_str(&pair(10, "352440.677"));
        e.push_str(&pair(20, "1233094.048"));
        e.push_str(&pair(30, "512.0"));
        e.push_str(&pair(0, "LINE"));
        e.push_str(&pair(8, "BOUNDARY"));
        e.push_str(&pair(10, "0"));
        e.push_str(&pair(20, "0"));
        e.push_str(&pair(11, "100"));
        e.push_str(&pair(21, "0"));
        let d = parse(&doc(&e)).unwrap();
        assert_eq!(d.points.len(), 1);
        assert_eq!(d.points[0].layer, "BEACONS");
        assert_eq!(d.points[0].handle.as_deref(), Some("A1"));
        assert_eq!(d.points[0].z, 512.0);
        assert_eq!(d.lines.len(), 1);
        assert_eq!(d.layers, vec!["BEACONS", "BOUNDARY"]);
        assert_eq!(d.counts.get("POINT"), Some(&1));
    }

    #[test]
    fn parses_closed_lwpolyline_with_bulges() {
        let mut e = String::new();
        e.push_str(&pair(0, "LWPOLYLINE"));
        e.push_str(&pair(8, "PARCEL"));
        e.push_str(&pair(90, "4"));
        e.push_str(&pair(70, "1"));
        for (x, y) in [(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)] {
            e.push_str(&pair(10, &x.to_string()));
            e.push_str(&pair(20, &y.to_string()));
        }
        let d = parse(&doc(&e)).unwrap();
        assert_eq!(d.polylines.len(), 1);
        let p = &d.polylines[0];
        assert!(p.closed);
        assert_eq!(p.vertices.len(), 4);
        assert_eq!(p.vertices[2], Point::new(100.0, 100.0));
        assert_eq!(d.closed_boundaries().len(), 1);
    }

    #[test]
    fn parses_classic_polyline_vertices() {
        let mut e = String::new();
        e.push_str(&pair(0, "POLYLINE"));
        e.push_str(&pair(8, "PARCEL"));
        e.push_str(&pair(70, "1"));
        for (x, y) in [(0.0, 0.0), (50.0, 0.0), (50.0, 50.0)] {
            e.push_str(&pair(0, "VERTEX"));
            e.push_str(&pair(10, &x.to_string()));
            e.push_str(&pair(20, &y.to_string()));
        }
        e.push_str(&pair(0, "SEQEND"));
        let d = parse(&doc(&e)).unwrap();
        assert_eq!(d.polylines.len(), 1);
        assert_eq!(d.polylines[0].vertices.len(), 3);
        assert!(d.polylines[0].closed);
    }

    #[test]
    fn parses_arcs_circles_and_text() {
        let mut e = String::new();
        e.push_str(&pair(0, "ARC"));
        e.push_str(&pair(10, "0"));
        e.push_str(&pair(20, "0"));
        e.push_str(&pair(40, "25"));
        e.push_str(&pair(50, "0"));
        e.push_str(&pair(51, "90"));
        e.push_str(&pair(0, "CIRCLE"));
        e.push_str(&pair(10, "50"));
        e.push_str(&pair(20, "50"));
        e.push_str(&pair(40, "5"));
        e.push_str(&pair(0, "TEXT"));
        e.push_str(&pair(10, "10"));
        e.push_str(&pair(20, "20"));
        e.push_str(&pair(1, "1001"));
        let d = parse(&doc(&e)).unwrap();
        assert_eq!(d.arcs[0].radius, 25.0);
        assert_eq!(d.arcs[0].end_deg, 90.0);
        assert_eq!(d.circles[0].centre, Point::new(50.0, 50.0));
        assert_eq!(d.texts[0].text, "1001");
    }

    #[test]
    fn warns_about_inserts_and_ignores_unknown_entities() {
        let mut e = String::new();
        e.push_str(&pair(0, "INSERT"));
        e.push_str(&pair(8, "BLOCKS"));
        e.push_str(&pair(2, "BEACON"));
        e.push_str(&pair(0, "SPLINE"));
        let d = parse(&doc(&e)).unwrap();
        assert!(d.warnings.iter().any(|w| w.contains("BEACON")));
        assert!(d.warnings.iter().any(|w| w.contains("SPLINE")));
        assert_eq!(d.entity_count(), 0);
    }

    #[test]
    fn rejects_binary_and_truncated_dxf() {
        assert_eq!(
            parse("AutoCAD Binary DXF\u{1a}").unwrap_err(),
            DxfError::Binary
        );
        assert!(matches!(parse("0\n").unwrap_err(), DxfError::Truncated));
    }

    #[test]
    fn tolerates_crlf_and_blank_lines() {
        let mut entity = pair(0, "POINT");
        entity.push_str(&pair(10, "1"));
        entity.push_str(&pair(20, "2"));
        let text = doc(&entity).replace('\n', "\r\n");
        let d = parse(&text).unwrap();
        assert_eq!(d.points.len(), 1);
    }
}
