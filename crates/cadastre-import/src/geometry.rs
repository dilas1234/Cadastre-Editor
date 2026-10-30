//! A small WKT reader for the geometry kinds the Aumentum cadastre feature classes use
//! (`POINT`, `POLYGON`, `MULTIPOLYGON`, and their `EMPTY` forms), read from `SHAPE.STAsText()`.
//!
//! Scoped on purpose: rather than pull a general WKT dependency, this parses exactly the shapes
//! we import, into `cadastre_core::Point` rings the geometry engine already understands.

use cadastre_core::Point;

use crate::ImportError;

/// A parsed geometry. Rings are vectors of points; polygons are `[exterior, holes…]`.
#[derive(Clone, Debug, PartialEq)]
pub enum ImportedGeometry {
    /// An empty geometry (`… EMPTY`) — e.g. placeholder parcel rows.
    Empty,
    /// A single point.
    Point(Point),
    /// A polygon as one or more rings (exterior first).
    Polygon(Vec<Vec<Point>>),
    /// A multipolygon: several polygons, each a list of rings.
    MultiPolygon(Vec<Vec<Vec<Point>>>),
}

impl ImportedGeometry {
    /// Parse a WKT string. Case-insensitive; tolerant of extra whitespace.
    pub fn from_wkt(wkt: &str) -> Result<Self, ImportError> {
        let body = wkt.trim().to_ascii_uppercase();
        let err = || ImportError::InvalidWkt(wkt.to_string());

        if body.ends_with("EMPTY") {
            return Ok(Self::Empty);
        }
        if let Some(rest) = body.strip_prefix("MULTIPOLYGON") {
            let outer = top_level_groups(rest);
            let inner = outer.first().ok_or_else(err)?;
            let polys = top_level_groups(inner)
                .into_iter()
                .map(|p| {
                    top_level_groups(p)
                        .into_iter()
                        .map(parse_ring)
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(|_| err())
                })
                .collect::<Result<Vec<_>, _>>()?;
            if polys.is_empty() {
                return Err(err());
            }
            Ok(Self::MultiPolygon(polys))
        } else if let Some(rest) = body.strip_prefix("POLYGON") {
            let outer = top_level_groups(rest);
            let inner = outer.first().ok_or_else(err)?;
            let rings = top_level_groups(inner)
                .into_iter()
                .map(parse_ring)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| err())?;
            if rings.is_empty() {
                return Err(err());
            }
            Ok(Self::Polygon(rings))
        } else if let Some(rest) = body.strip_prefix("POINT") {
            let g = top_level_groups(rest);
            let coords = parse_ring(g.first().ok_or_else(err)?).map_err(|_| err())?;
            let p = coords.first().ok_or_else(err)?;
            Ok(Self::Point(*p))
        } else {
            Err(err())
        }
    }

    /// Whether this geometry carries no usable coordinates.
    pub fn is_empty(&self) -> bool {
        match self {
            Self::Empty => true,
            Self::Point(_) => false,
            Self::Polygon(rings) => rings.iter().all(|r| r.is_empty()),
            Self::MultiPolygon(polys) => polys.iter().flatten().all(|r| r.is_empty()),
        }
    }

    /// The exterior ring of the first polygon, if this is a (multi)polygon with one.
    pub fn exterior_ring(&self) -> Option<&[Point]> {
        match self {
            Self::Polygon(rings) => rings.first().map(|r| r.as_slice()),
            Self::MultiPolygon(polys) => polys.first().and_then(|p| p.first()).map(|r| r.as_slice()),
            _ => None,
        }
    }
}

/// Return the inner text of each outermost `(...)` group in `s`, honouring nesting.
/// E.g. `"((a),(b))"` → `["(a),(b)"]`; `"(a),(b)"` → `["a","b"]`.
fn top_level_groups(s: &str) -> Vec<&str> {
    let bytes = s.as_bytes();
    let mut groups = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'(' => {
                if depth == 0 {
                    start = i + 1;
                }
                depth += 1;
            }
            b')' => {
                depth -= 1;
                if depth == 0 {
                    groups.push(&s[start..i]);
                }
            }
            _ => {}
        }
    }
    groups
}

/// Parse a coordinate list `"x y, x y, …"` (ignores any Z/M beyond the first two ordinates).
fn parse_ring(s: &str) -> Result<Vec<Point>, ()> {
    s.split(',')
        .map(|pair| {
            let mut it = pair.split_whitespace();
            let x = it.next().and_then(|v| v.parse::<f64>().ok()).ok_or(())?;
            let y = it.next().and_then(|v| v.parse::<f64>().ok()).ok_or(())?;
            Ok(Point::new(x, y))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_point_and_empty() {
        assert_eq!(
            ImportedGeometry::from_wkt("POINT (500000 1099097.5)").unwrap(),
            ImportedGeometry::Point(Point::new(500000.0, 1099097.5))
        );
        assert_eq!(ImportedGeometry::from_wkt("POINT EMPTY").unwrap(), ImportedGeometry::Empty);
        assert_eq!(ImportedGeometry::from_wkt("POLYGON EMPTY").unwrap(), ImportedGeometry::Empty);
    }

    #[test]
    fn parses_a_simple_polygon() {
        // A 100 m square in UTM-like coordinates.
        let wkt = "POLYGON ((0 0, 100 0, 100 100, 0 100, 0 0))";
        let g = ImportedGeometry::from_wkt(wkt).unwrap();
        match &g {
            ImportedGeometry::Polygon(rings) => {
                assert_eq!(rings.len(), 1);
                assert_eq!(rings[0].len(), 5);
                assert_eq!(rings[0][2], Point::new(100.0, 100.0));
            }
            other => panic!("expected polygon, got {other:?}"),
        }
        assert_eq!(g.exterior_ring().unwrap().len(), 5);
        assert!(!g.is_empty());
    }

    #[test]
    fn parses_polygon_with_a_hole() {
        let wkt = "POLYGON ((0 0, 10 0, 10 10, 0 10, 0 0), (2 2, 4 2, 4 4, 2 4, 2 2))";
        match ImportedGeometry::from_wkt(wkt).unwrap() {
            ImportedGeometry::Polygon(rings) => {
                assert_eq!(rings.len(), 2);
                assert_eq!(rings[1].len(), 5);
            }
            other => panic!("expected polygon, got {other:?}"),
        }
    }

    #[test]
    fn parses_multipolygon() {
        let wkt = "MULTIPOLYGON (((0 0, 1 0, 1 1, 0 0)), ((5 5, 6 5, 6 6, 5 5)))";
        match ImportedGeometry::from_wkt(wkt).unwrap() {
            ImportedGeometry::MultiPolygon(polys) => {
                assert_eq!(polys.len(), 2);
                assert_eq!(polys[0][0].len(), 4);
                assert_eq!(polys[1][0][1], Point::new(6.0, 5.0));
            }
            other => panic!("expected multipolygon, got {other:?}"),
        }
    }

    #[test]
    fn parses_real_production_wkt_prefix() {
        // Shape of a real PLATEAU_STATE_PARCELS row (EPSG:32632 metres).
        let wkt = "POLYGON ((488757.521 1095143.118, 488749.824 1095155.290, 488760.0 1095160.0, 488757.521 1095143.118))";
        let g = ImportedGeometry::from_wkt(wkt).unwrap();
        assert_eq!(g.exterior_ring().unwrap().len(), 4);
        assert!((g.exterior_ring().unwrap()[0].x - 488757.521).abs() < 1e-6);
    }

    #[test]
    fn rejects_garbage_and_unsupported() {
        assert!(ImportedGeometry::from_wkt("LINESTRING (0 0, 1 1)").is_err());
        assert!(ImportedGeometry::from_wkt("not wkt at all").is_err());
    }
}
