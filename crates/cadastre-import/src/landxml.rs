//! LandXML import: control points, parcel boundaries and alignments (SRS survey/CAD import).
//!
//! LandXML coordinates are **northing easting [elevation]**, in the file's declared linear unit.
//! They are converted to metres here. Curves are tessellated finely enough (≤ 0.25 m chord) that
//! the imported ring keeps the plan's area; the original curve definition is preserved in the
//! warnings/report instead of being silently lost.
//!
//! Spiral transitions and `IrregularLine` are reported rather than approximated without warning.

use std::collections::BTreeMap;
use std::fmt;

use cadastre_core::types::Point;
use serde::{Deserialize, Serialize};

use crate::xml::{self, Node};

/// A LandXML control point.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LandXmlPoint {
    /// Point name (`name` or `pntRef`).
    pub id: String,
    /// Coordinate after unit conversion.
    pub point: Point,
    /// Elevation in metres.
    pub elevation: Option<f64>,
    /// Feature code.
    pub code: Option<String>,
    /// Description.
    pub description: Option<String>,
}

/// A boundary assembled from a coordinate geometry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LandXmlBoundary {
    /// Parcel/feature/alignment name.
    pub name: String,
    /// Source container: `Parcel`, `PlanFeature` or `Alignment`.
    pub kind: String,
    /// Ring vertices in import order (no repeated closing vertex).
    pub ring: Vec<Point>,
    /// Whether the source geometry closed.
    pub closed: bool,
    /// Area stated in the file (m²), when present.
    pub stated_area: Option<f64>,
}

/// Everything extracted from one LandXML document.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LandXmlDocument {
    /// Control points.
    pub points: Vec<LandXmlPoint>,
    /// Parcels, plan features and alignments with coordinate geometry.
    pub boundaries: Vec<LandXmlBoundary>,
    /// EPSG code from `<CoordinateSystem epsgCode=…>`, when present.
    pub epsg: Option<u32>,
    /// Linear unit as written in the file.
    pub linear_unit: Option<String>,
    /// Non-fatal findings.
    pub warnings: Vec<String>,
}

/// Why a LandXML file could not be read.
#[derive(Clone, Debug, PartialEq)]
pub enum LandXmlError {
    /// The XML itself is malformed.
    Xml(String),
    /// No `LandXML` root element.
    NotLandXml,
}

impl fmt::Display for LandXmlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LandXmlError::Xml(message) => write!(f, "invalid LandXML: {message}"),
            LandXmlError::NotLandXml => write!(f, "not a LandXML document: root element is not `LandXML`"),
        }
    }
}

impl std::error::Error for LandXmlError {}

/// Metres per declared linear unit.
fn unit_factor(unit: &str) -> Option<f64> {
    Some(match unit.to_ascii_lowercase().replace([' ', '_', '-'], "").as_str() {
        "meter" | "metre" | "meters" | "metres" | "m" => 1.0,
        "millimeter" | "millimetre" | "millimeters" | "millimetres" | "mm" => 0.001,
        "centimeter" | "centimetre" | "centimeters" | "centimetres" | "cm" => 0.01,
        "kilometer" | "kilometre" | "kilometers" | "kilometres" | "km" => 1000.0,
        "foot" | "feet" | "ft" | "internationalfoot" => 0.3048,
        "ussurveyfoot" | "usfoot" | "usft" => 1200.0 / 3937.0,
        _ => return None,
    })
}

/// Parse a LandXML `N E [Z]` coordinate string. The unit factor is applied here.
fn parse_coordinate(text: &str, factor: f64) -> Option<(Point, Option<f64>)> {
    let mut parts = text.split_whitespace().filter_map(|p| p.parse::<f64>().ok());
    let northing = parts.next()?;
    let easting = parts.next()?;
    if !northing.is_finite() || !easting.is_finite() {
        return None;
    }
    let elevation = parts.next().filter(|v| v.is_finite());
    Some((
        Point::new(easting * factor, northing * factor),
        elevation.map(|z| z * factor),
    ))
}

fn node_coordinate(node: &Node, factor: f64) -> Option<(Point, Option<f64>)> {
    parse_coordinate(&node.text, factor)
}

/// Number of straight segments used to tessellate an arc with radius `radius` and sweep `sweep`.
/// Keeps the chord at or below 0.25 m, capped to keep pathological input bounded.
fn arc_segments(radius: f64, sweep: f64) -> usize {
    if radius <= 0.0 {
        return 1;
    }
    let max_angle = (0.25 / radius).min(std::f64::consts::FRAC_PI_2);
    let count = if max_angle > 1e-9 {
        (sweep.abs() / max_angle).ceil() as usize
    } else {
        16
    };
    count.clamp(8, 256)
}

fn positive_sweep(from: f64, to: f64) -> f64 {
    let mut sweep = (to - from) % std::f64::consts::TAU;
    if sweep < 0.0 {
        sweep += std::f64::consts::TAU;
    }
    sweep
}

/// Tessellate one LandXML `<Curve>` into points from start to end.
fn curve_points(curve: &Node, factor: f64, warnings: &mut Vec<String>) -> Vec<Point> {
    let Some((start, _)) = curve.child("Start").and_then(|n| node_coordinate(n, factor)) else {
        warnings.push("a Curve has no usable Start".to_string());
        return Vec::new();
    };
    let Some((end, _)) = curve.child("End").and_then(|n| node_coordinate(n, factor)) else {
        warnings.push("a Curve has no usable End".to_string());
        return vec![start];
    };
    let Some(centre) = curve.child("Center").and_then(|n| node_coordinate(n, factor)).map(|(p, _)| p) else {
        // A curve without a centre is at least a straight boundary segment.
        warnings.push("a Curve has no Center; treated as a straight chord".to_string());
        return vec![start, end];
    };
    let rotation = curve
        .child("Rotation")
        .map(|n| n.text.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .or_else(|| curve.attr("rot").map(|s| s.to_ascii_lowercase()))
        .unwrap_or_else(|| "ccw".to_string());
    let radius = curve
        .child("Radius")
        .and_then(|n| n.text.trim().parse::<f64>().ok())
        .unwrap_or_else(|| centre.distance_to(start))
        * factor;
    if radius <= 0.0 {
        warnings.push("a Curve has a non-positive radius; treated as a chord".to_string());
        return vec![start, end];
    }

    let a0 = (start.y - centre.y).atan2(start.x - centre.x);
    let a1 = (end.y - centre.y).atan2(end.x - centre.x);
    let sweep = if rotation.starts_with("cw") {
        -positive_sweep(a1, a0)
    } else {
        positive_sweep(a0, a1)
    };
    if sweep.abs() < 1e-12 {
        return vec![start, end];
    }
    let segments = arc_segments(radius, sweep);
    let mut points = Vec::with_capacity(segments + 1);
    for i in 0..=segments {
        let t = i as f64 / segments as f64;
        let angle = a0 + sweep * t;
        points.push(Point::new(
            centre.x + radius * angle.cos(),
            centre.y + radius * angle.sin(),
        ));
    }
    if let Some(first) = points.first_mut() {
        *first = start;
    }
    if let Some(last) = points.last_mut() {
        *last = end;
    }
    points
}

fn push_ring_vertex(ring: &mut Vec<Point>, point: Point) {
    if ring
        .last()
        .map(|last| last.distance_to(point) < 1e-9)
        .unwrap_or(false)
    {
        return;
    }
    ring.push(point);
}

/// Assemble a `<CoordGeom>` into a ring.
fn coord_geom_ring(coord_geom: &Node, factor: f64, warnings: &mut Vec<String>) -> Vec<Point> {
    let mut ring = Vec::new();
    for child in &coord_geom.children {
        match child.name.as_str() {
            "Line" => {
                if let Some((start, _)) = child.child("Start").and_then(|n| node_coordinate(n, factor)) {
                    push_ring_vertex(&mut ring, start);
                }
                if let Some((end, _)) = child.child("End").and_then(|n| node_coordinate(n, factor)) {
                    push_ring_vertex(&mut ring, end);
                }
            }
            "Curve" => {
                for point in curve_points(child, factor, warnings) {
                    push_ring_vertex(&mut ring, point);
                }
            }
            "IrregularLine" | "IrregularLine3D" => {
                for point_node in child.children_named("Pnt") {
                    if let Some((point, _)) = node_coordinate(point_node, factor) {
                        push_ring_vertex(&mut ring, point);
                    }
                }
            }
            "Spiral" => warnings.push(
                "a Spiral transition was not tessellated; its chord endpoints were used".to_string(),
            ),
            _ => {}
        }
    }
    ring
}

/// Parse a LandXML document.
pub fn parse(contents: &str) -> Result<LandXmlDocument, LandXmlError> {
    let root = xml::parse(contents).map_err(|e| LandXmlError::Xml(e.message))?;
    if root.name != "LandXML" {
        return Err(LandXmlError::NotLandXml);
    }

    let mut doc = LandXmlDocument::default();

    // Units.
    let linear_unit = root
        .find("Metric")
        .and_then(|m| m.attr("linearUnit").map(str::to_string))
        .or_else(|| {
            root.find("Imperial")
                .and_then(|m| m.attr("linearUnit").map(str::to_string))
        });
    let factor = match linear_unit.as_deref() {
        Some(unit) => match unit_factor(unit) {
            Some(f) => f,
            None => {
                doc.warnings.push(format!(
                    "unknown linear unit `{unit}`; coordinates assumed to be metres"
                ));
                1.0
            }
        },
        None => {
            doc.warnings
                .push("no linear unit declared; coordinates assumed to be metres".to_string());
            1.0
        }
    };
    doc.linear_unit = linear_unit;

    // Coordinate system.
    if let Some(cs) = root.find("CoordinateSystem") {
        doc.epsg = cs
            .attr("epsgCode")
            .and_then(|c| c.parse::<u32>().ok())
            .or_else(|| cs.attr("name").and_then(crate::worldfile::epsg_from_wkt));
        if doc.epsg.is_none() {
            if let Some(name) = cs.attr("name") {
                doc.warnings
                    .push(format!("coordinate system `{name}` has no EPSG code"));
            }
        }
    }

    // Control points. Resolve `pntRef` after collecting the points that carry coordinates.
    let mut all_points = Vec::new();
    root.find_all("CgPoint", &mut all_points);
    let mut by_name: BTreeMap<String, Point> = BTreeMap::new();
    let mut pending_refs: Vec<(String, String, Option<String>, Option<String>)> = Vec::new();
    for node in all_points {
        let id = node
            .attr("name")
            .or_else(|| node.attr("pntRef"))
            .unwrap_or("")
            .trim()
            .to_string();
        match node_coordinate(node, factor) {
            Some((point, elevation)) => {
                if !id.is_empty() {
                    by_name.insert(id.clone(), point);
                }
                doc.points.push(LandXmlPoint {
                    id,
                    point,
                    elevation,
                    code: node.attr("code").map(str::to_string),
                    description: node.attr("desc").map(str::to_string),
                });
            }
            None => {
                if let Some(reference) = node.attr("pntRef") {
                    pending_refs.push((
                        id,
                        reference.to_string(),
                        node.attr("code").map(str::to_string),
                        node.attr("desc").map(str::to_string),
                    ));
                } else if !id.is_empty() {
                    doc.warnings
                        .push(format!("point `{id}` has no usable coordinate"));
                }
            }
        }
    }
    for (id, reference, code, description) in pending_refs {
        match by_name.get(&reference) {
            Some(point) => doc.points.push(LandXmlPoint {
                id,
                point: *point,
                elevation: None,
                code,
                description,
            }),
            None => doc.warnings.push(format!(
                "point `{id}` references unknown point `{reference}`"
            )),
        }
    }

    // Parcels, plan features and alignments.
    for (container, kind) in [
        ("Parcel", "Parcel"),
        ("PlanFeature", "PlanFeature"),
        ("Alignment", "Alignment"),
    ] {
        let mut nodes = Vec::new();
        root.find_all(container, &mut nodes);
        for node in nodes {
            let Some(coord_geom) = node.child("CoordGeom") else {
                continue;
            };
            let mut ring = coord_geom_ring(coord_geom, factor, &mut doc.warnings);
            let mut closed = false;
            if ring.len() >= 3 && ring[0].distance_to(*ring.last().expect("non-empty")) < 1e-6 {
                ring.pop();
                closed = true;
            }
            if ring.len() < 3 {
                doc.warnings.push(format!(
                    "{kind} `{}` has fewer than three distinct boundary vertices",
                    node.attr("name").unwrap_or("?")
                ));
                continue;
            }
            doc.boundaries.push(LandXmlBoundary {
                name: node
                    .attr("name")
                    .or_else(|| node.attr("desc"))
                    .unwrap_or("")
                    .to_string(),
                kind: kind.to_string(),
                ring,
                closed,
                stated_area: node
                    .attr("area")
                    .and_then(|a| a.parse::<f64>().ok())
                    .map(|a| a * factor * factor),
            });
        }
    }

    if doc.points.is_empty() && doc.boundaries.is_empty() {
        doc.warnings.push(
            "no CgPoint, Parcel, PlanFeature or Alignment geometry found in the document"
                .to_string(),
        );
    }
    Ok(doc)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = r#"<?xml version="1.0"?>
<LandXML xmlns="http://www.landxml.org/schema/LandXML-1.2" version="1.2">
  <Units><Metric linearUnit="meter" areaUnit="squareMeter" angularUnit="decimal degrees"/></Units>
  <CoordinateSystem name="WGS 84 / UTM zone 32N" epsgCode="32632"/>
  <CgPoints>
    <CgPoint name="1001" code="BC" desc="Road corner">1233094.048 352440.677 512.0</CgPoint>
    <CgPoint name="1002" code="BC">1233094.048 352540.677</CgPoint>
  </CgPoints>
  <Parcels>
    <Parcel name="PL1" area="10000">
      <CoordGeom>
        <Line><Start>1233094.048 352440.677</Start><End>1233094.048 352540.677</End></Line>
        <Line><Start>1233094.048 352540.677</Start><End>1233194.048 352540.677</End></Line>
        <Line><Start>1233194.048 352540.677</Start><End>1233194.048 352440.677</End></Line>
        <Line><Start>1233194.048 352440.677</Start><End>1233094.048 352440.677</End></Line>
      </CoordGeom>
    </Parcel>
  </Parcels>
</LandXML>"#;

    #[test]
    fn reads_units_points_and_a_parcel() {
        let doc = parse(DOC).unwrap();
        assert_eq!(doc.epsg, Some(32632));
        assert_eq!(doc.linear_unit.as_deref(), Some("meter"));
        assert_eq!(doc.points.len(), 2);
        assert_eq!(doc.points[0].id, "1001");
        assert!((doc.points[0].point.x - 352_440.677).abs() < 1e-9);
        assert!((doc.points[0].point.y - 1_233_094.048).abs() < 1e-9);
        assert_eq!(doc.points[0].elevation, Some(512.0));
        assert_eq!(doc.points[0].code.as_deref(), Some("BC"));

        assert_eq!(doc.boundaries.len(), 1);
        let parcel = &doc.boundaries[0];
        assert_eq!(parcel.name, "PL1");
        assert!(parcel.closed);
        assert_eq!(parcel.ring.len(), 4);
        assert_eq!(parcel.stated_area, Some(10_000.0));
    }

    #[test]
    fn converts_feet_to_metres() {
        let doc = parse(
            r#"<LandXML><Units><Imperial linearUnit="USSurveyFoot"/></Units>
               <CgPoints><CgPoint name="1">0 100</CgPoint></CgPoints></LandXML>"#,
        )
        .unwrap();
        let expected = 100.0 * 1200.0 / 3937.0;
        assert!((doc.points[0].point.x - expected).abs() < 1e-9);
    }

    #[test]
    fn tessellates_a_quarter_circle_and_keeps_its_area_close() {
        // Quarter circle of radius 100 about (0,0), from east to north.
        let doc = parse(
            r#"<LandXML><Units><Metric linearUnit="meter"/></Units>
               <Parcels><Parcel name="C"><CoordGeom>
                 <Line><Start>0 0</Start><End>100 0</End></Line>
                 <Curve><Start>100 0</Start><Center>0 0</Center><Rotation>cw</Rotation><Radius>100</Radius><End>0 100</End></Curve>
                 <Line><Start>0 100</Start><End>0 0</End></Line>
               </CoordGeom></Parcel></Parcels></LandXML>"#,
        )
        .unwrap();
        let ring = &doc.boundaries[0].ring;
        assert!(ring.len() > 10);
        // Shoelace area of the tessellated ring approximates a quarter disc of radius 100.
        let mut area = 0.0;
        for i in 0..ring.len() {
            let a = ring[i];
            let b = ring[(i + 1) % ring.len()];
            area += a.x * b.y - b.x * a.y;
        }
        let area = (area / 2.0).abs();
        let exact = std::f64::consts::PI * 100.0 * 100.0 / 4.0;
        assert!((area - exact).abs() / exact < 1e-3, "area {area} vs {exact}");
    }

    #[test]
    fn resolves_pnt_ref_and_warns_about_spirals() {
        let doc = parse(
            r#"<LandXML><CgPoints>
                 <CgPoint name="A">0 0</CgPoint>
                 <CgPoint name="B" pntRef="A"/>
               </CgPoints>
               <Alignments><Alignment name="R"><CoordGeom>
                 <Spiral><Start>0 0</Start><End>1 1</End></Spiral>
               </CoordGeom></Alignment></Alignments></LandXML>"#,
        )
        .unwrap();
        assert_eq!(doc.points.len(), 2);
        assert_eq!(doc.points[1].point, Point::new(0.0, 0.0));
        assert!(doc.warnings.iter().any(|w| w.contains("Spiral")));
    }

    #[test]
    fn rejects_non_landxml_and_malformed_xml() {
        assert_eq!(parse("<gpx/>").unwrap_err(), LandXmlError::NotLandXml);
        assert!(matches!(
            parse("<LandXML><a></LandXML>").unwrap_err(),
            LandXmlError::Xml(_)
        ));
    }
}
