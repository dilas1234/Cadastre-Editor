//! Minimal shapefile reader: `.shp` geometry -> GeoJSON (in the file's native CRS), plus a
//! best-effort EPSG from the sibling `.prj`. Dependency-free (the format is simple) and read-only.
//! The browser reprojects to WGS84 for display. Z/M shapes are read as their 2-D X/Y.

use std::fs;
use std::io::{Error, ErrorKind, Result};
use std::path::Path;

use serde::Serialize;
use serde_json::{json, Value};

/// A shapefile's geometry as GeoJSON, ready for the editor to reproject and draw.
#[derive(Serialize)]
pub struct ShapeData {
    /// Best-effort EPSG of the coordinates (from the `.prj`); `null` if undetermined.
    pub epsg: Option<u32>,
    /// `[xmin, ymin, xmax, ymax]` in the file's native CRS (from the header).
    pub bbox: [f64; 4],
    /// `point` | `multipoint` | `polyline` | `polygon` (from the header shape type).
    pub geometry_type: String,
    /// Number of features returned.
    pub count: usize,
    /// True if the vertex cap was hit and the result is partial.
    pub truncated: bool,
    /// A GeoJSON `FeatureCollection` in native coordinates.
    pub geojson: Value,
}

fn be_i32(b: &[u8], o: usize) -> i32 {
    i32::from_be_bytes(b[o..o + 4].try_into().unwrap())
}
fn le_i32(b: &[u8], o: usize) -> i32 {
    i32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn le_f64(b: &[u8], o: usize) -> f64 {
    f64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}

fn base_type_name(t: i32) -> &'static str {
    match t {
        1 | 11 | 21 => "point",
        8 | 18 | 28 => "multipoint",
        3 | 13 | 23 => "polyline",
        5 | 15 | 25 => "polygon",
        _ => "null",
    }
}

/// Signed area (shoelace); shapefile outer rings are clockwise (negative here), holes CCW.
fn signed_area(ring: &[[f64; 2]]) -> f64 {
    let mut a = 0.0;
    for w in ring.windows(2) {
        a += w[0][0] * w[1][1] - w[1][0] * w[0][1];
    }
    a / 2.0
}

/// Read a shapefile into GeoJSON. `max_vertices` caps the result (large layers are truncated).
pub fn read_shapefile(shp: &Path, max_vertices: usize) -> Result<ShapeData> {
    let b = fs::read(shp)?;
    if b.len() < 100 || be_i32(&b, 0) != 9994 {
        return Err(Error::new(ErrorKind::InvalidData, "not a shapefile (bad header)"));
    }
    let shape_type = le_i32(&b, 32);
    let bbox = [le_f64(&b, 36), le_f64(&b, 44), le_f64(&b, 52), le_f64(&b, 60)];
    let epsg = read_prj_epsg(shp);

    let mut features: Vec<Value> = Vec::new();
    let mut verts = 0usize;
    let mut truncated = false;
    let mut pos = 100;
    while pos + 8 <= b.len() {
        let content_len = be_i32(&b, pos + 4) as usize * 2; // 16-bit words -> bytes
        let start = pos + 8;
        let end = start + content_len;
        if end > b.len() {
            break;
        }
        let rec = &b[start..end];
        if rec.len() >= 4 {
            if let Some((geom, nv)) = parse_geom(le_i32(rec, 0), rec) {
                verts += nv;
                let fid = features.len();
                features.push(json!({"type":"Feature","properties":{"FID": fid},"geometry": geom}));
                if verts >= max_vertices {
                    truncated = true;
                    break;
                }
            }
        }
        pos = end;
    }

    Ok(ShapeData {
        epsg,
        bbox,
        geometry_type: base_type_name(shape_type).to_string(),
        count: features.len(),
        truncated,
        geojson: json!({"type":"FeatureCollection","features": features}),
    })
}

/// Parse one record's geometry into (GeoJSON, vertex count). `None` for null/unsupported shapes.
fn parse_geom(st: i32, r: &[u8]) -> Option<(Value, usize)> {
    let pt = |o: usize| -> Option<[f64; 2]> {
        (o + 16 <= r.len()).then(|| [le_f64(r, o), le_f64(r, o + 8)])
    };
    match st {
        1 | 11 | 21 => {
            let p = pt(4)?;
            Some((json!({"type":"Point","coordinates": p}), 1))
        }
        8 | 18 | 28 => {
            let n = le_i32(r, 36) as usize;
            let mut pts = Vec::with_capacity(n);
            for i in 0..n {
                pts.push(pt(40 + i * 16)?);
            }
            Some((json!({"type":"MultiPoint","coordinates": pts}), n))
        }
        3 | 13 | 23 | 5 | 15 | 25 => {
            let nparts = le_i32(r, 36) as usize;
            let npoints = le_i32(r, 40) as usize;
            let parts_off = 44;
            let points_off = parts_off + nparts * 4;
            if points_off + npoints * 16 > r.len() {
                return None;
            }
            let mut bounds: Vec<usize> = (0..nparts).map(|i| le_i32(r, parts_off + i * 4) as usize).collect();
            bounds.push(npoints);
            let rings: Vec<Vec<[f64; 2]>> = bounds
                .windows(2)
                .map(|w| (w[0]..w[1]).map(|i| [le_f64(r, points_off + i * 16), le_f64(r, points_off + i * 16 + 8)]).collect())
                .collect();

            if matches!(st, 5 | 15 | 25) {
                // Group rings into polygons: a clockwise (negative-area) ring starts a new polygon;
                // CCW rings are holes of the current one.
                let mut polys: Vec<Vec<Vec<[f64; 2]>>> = Vec::new();
                for ring in rings {
                    if polys.is_empty() || signed_area(&ring) < 0.0 {
                        polys.push(vec![ring]);
                    } else {
                        polys.last_mut().unwrap().push(ring);
                    }
                }
                let geom = if polys.len() == 1 {
                    json!({"type":"Polygon","coordinates": polys.remove(0)})
                } else {
                    json!({"type":"MultiPolygon","coordinates": polys})
                };
                Some((geom, npoints))
            } else if rings.len() == 1 {
                Some((json!({"type":"LineString","coordinates": rings.into_iter().next().unwrap()}), npoints))
            } else {
                Some((json!({"type":"MultiLineString","coordinates": rings}), npoints))
            }
        }
        _ => None,
    }
}

/// Best-effort EPSG from the sibling `.prj` WKT (the shapefiles here are mostly WGS84 / UTM 32N).
fn read_prj_epsg(shp: &Path) -> Option<u32> {
    let t = fs::read_to_string(shp.with_extension("prj")).ok()?.to_uppercase();
    let has = |s: &str| t.contains(s);
    if has("WGS_1984") && (has("UTM_ZONE_32") || (has("CENTRAL_MERIDIAN\",9") && has("TRANSVERSE_MERCATOR"))) {
        return Some(32632);
    }
    if has("WGS_1984") && has("UTM_ZONE_31") {
        return Some(32631);
    }
    if has("MINNA") {
        if has("MID_BELT") {
            return Some(26392);
        } else if has("WEST_BELT") {
            return Some(26391);
        } else if has("EAST_BELT") {
            return Some(26393);
        } else if has("ZONE_31") {
            return Some(26331);
        }
        return Some(26332); // Minna / UTM zone 32N
    }
    if has("GEOGCS") && !has("PROJCS") && has("WGS_1984") {
        return Some(4326);
    }
    None
}
