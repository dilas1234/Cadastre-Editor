//! KML import (Google Earth and similar): placemark polygons, lines and points.
//!
//! KML coordinates are always WGS 84 longitude/latitude, so they are projected to UTM 32N metres
//! (the fabric's CRS) here. A KMZ is a zip archive and is not read: save as KML first.

use std::collections::BTreeMap;

use cadastre_core::crs::WorkingCrs;
use cadastre_core::types::Point;

use crate::conversion::{ImportError, ImportFormat, ImportedBoundary, ImportedPoint, ImportedSurvey};
use crate::xml::{self, Node};

/// Placemarks may be nested this deep in folders.
const MAX_DEPTH: usize = 64;

struct Reader {
    crs: WorkingCrs,
    out: ImportedSurvey,
    bad: usize,
    far: usize,
    holes: usize,
}

fn name_of(n: &Node) -> Option<String> {
    n.child("name").map(|c| c.text.trim().to_string()).filter(|t| !t.is_empty())
}

impl Reader {
    /// The `<coordinates>` text under `n`: `lon,lat[,alt]` tuples separated by white space.
    fn coordinates(&mut self, n: &Node) -> Vec<(Point, Option<f64>)> {
        let mut pts = Vec::new();
        for tuple in n.child("coordinates").map(|c| c.text.as_str()).unwrap_or("").split_whitespace() {
            let v: Vec<f64> = tuple.split(',').filter_map(|t| t.parse().ok()).collect();
            match v[..] {
                [lon, lat, ..] if lon.is_finite() && lat.is_finite() => {
                    // Outside this the zone's projection is meaningless.
                    if (lon - 9.0).abs() > 10.0 || lat.abs() > 80.0 {
                        self.far += 1;
                    } else {
                        pts.push((self.crs.project_deg(lat, lon), v.get(2).copied().filter(|z| z.is_finite() && *z != 0.0)));
                    }
                }
                _ => self.bad += 1,
            }
        }
        pts
    }

    fn boundary(&mut self, name: &str, layer: &str, pts: Vec<(Point, Option<f64>)>, polygon: bool) {
        let mut ring: Vec<Point> = pts.into_iter().map(|(p, _)| p).collect();
        let returns = ring.len() > 3 && matches!((ring.first(), ring.last()), (Some(a), Some(b)) if (a.x - b.x).abs() < 1e-4 && (a.y - b.y).abs() < 1e-4);
        if returns {
            ring.pop();
        }
        if ring.len() < if polygon { 3 } else { 2 } {
            return;
        }
        let n = self.out.boundaries.len() + 1;
        let id = if name.is_empty() { format!("KML{n}") } else { name.to_string() };
        self.out.boundaries.push(ImportedBoundary { id, kind: layer.to_string(), ring, closed: polygon || returns, stated_area: None });
    }

    fn geometry(&mut self, n: &Node, name: &str, layer: &str, depth: usize) {
        match n.name.as_str() {
            "Polygon" => {
                if let Some(ring) = n.child("outerBoundaryIs").and_then(|o| o.child("LinearRing")) {
                    let pts = self.coordinates(ring);
                    self.boundary(name, layer, pts, true);
                }
                self.holes += n.children_named("innerBoundaryIs").count();
            }
            "LineString" | "LinearRing" => {
                let pts = self.coordinates(n);
                self.boundary(name, layer, pts, n.name == "LinearRing");
            }
            "Point" => {
                for (point, elevation) in self.coordinates(n) {
                    let id = if name.is_empty() { format!("P{}", self.out.points.len() + 1) } else { name.to_string() };
                    self.out.points.push(ImportedPoint { id, point, elevation, code: Some(layer.to_string()), attributes: BTreeMap::new() });
                }
            }
            "MultiGeometry" if depth < MAX_DEPTH => {
                for c in &n.children {
                    self.geometry(c, name, layer, depth + 1);
                }
            }
            _ => {}
        }
    }

    fn walk(&mut self, n: &Node, layer: &str, depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        for c in &n.children {
            match c.name.as_str() {
                "Placemark" => {
                    let name = name_of(c).unwrap_or_default();
                    for g in &c.children {
                        self.geometry(g, &name, layer, depth);
                    }
                }
                "Folder" => {
                    let inner = name_of(c).unwrap_or_else(|| layer.to_string());
                    self.walk(c, &inner, depth + 1);
                }
                "Document" | "kml" => self.walk(c, layer, depth + 1),
                _ => {}
            }
        }
    }
}

/// Read a KML document. Folder names become layers.
pub fn parse(contents: &str) -> Result<ImportedSurvey, ImportError> {
    let root = xml::parse(contents).map_err(|e| ImportError::Invalid(format!("not a readable KML file: {e}")))?;
    if root.name != "kml" {
        return Err(ImportError::Invalid("not a KML file (no <kml> element)".into()));
    }
    let mut r = Reader { crs: WorkingCrs::utm_zone_32n(), out: ImportedSurvey { format: Some(ImportFormat::Kml), ..Default::default() }, bad: 0, far: 0, holes: 0 };
    r.walk(&root, "KML", 0);
    for (n, what) in [
        (r.bad, "coordinate(s) could not be read"),
        (r.far, "coordinate(s) are too far from UTM zone 32N and were left out"),
        (r.holes, "hole(s) inside polygons were left out: only outer boundaries are read"),
    ] {
        if n > 0 {
            r.out.warnings.push(format!("{n} {what}"));
        }
    }
    r.out.crs = Some("EPSG:32632".into());
    r.out.epsg = Some(32632);
    Ok(r.out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polygons_lines_and_points_become_utm() {
        let s = parse(r#"<?xml version="1.0"?><kml xmlns="http://www.opengis.net/kml/2.2"><Document><name>d</name>
            <Folder><name>PLOTS</name><Placemark><name>Plot 7</name><MultiGeometry><Polygon><outerBoundaryIs><LinearRing><coordinates>
              9,9.8,0 9.001,9.8,0 9.001,9.801,0 9,9.8,0 </coordinates></LinearRing></outerBoundaryIs>
              <innerBoundaryIs><LinearRing><coordinates>9,9 9,9 9,9</coordinates></LinearRing></innerBoundaryIs></Polygon></MultiGeometry></Placemark></Folder>
            <Placemark><name>PB 1</name><Point><coordinates>9,9.8,1200.5</coordinates></Point></Placemark>
            <Placemark><LineString><coordinates>9,9.8 9.001,9.8 x,y 120,9</coordinates></LineString></Placemark></Document></kml>"#).unwrap();
        let b = &s.boundaries[0];
        assert_eq!((b.id.as_str(), b.kind.as_str(), b.closed, b.ring.len()), ("Plot 7", "PLOTS", true, 3));
        // Longitude 9° is the central meridian: easting 500 000 exactly. Northing per PROJ.
        assert!((b.ring[0].x - 500_000.0).abs() < 1e-6, "{:?}", b.ring[0]);
        assert!((b.ring[0].y - 1_083_277.0).abs() < 60.0, "{:?}", b.ring[0]);
        assert_eq!((s.points[0].id.as_str(), s.points[0].elevation), ("PB 1", Some(1200.5)));
        assert!(!s.boundaries[1].closed);
        assert_eq!(s.warnings.len(), 3, "{:?}", s.warnings);
    }

    #[test]
    fn other_files_are_refused() {
        assert!(parse("<gpx></gpx>").is_err());
        assert!(parse("PK\u{3}\u{4}").is_err());
        assert!(parse(&"<a>".repeat(100_000)).is_err(), "deep nesting is refused, not a crash");
    }
}
