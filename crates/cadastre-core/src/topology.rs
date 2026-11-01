//! Topology checks before submission (SRS FR-ADJ-04) and spatial join proposals (FR-FAB-02).
//!
//! Uses `geo` for robust polygon boolean operations and `rstar` for nearest-neighbour queries.

use geo::{Area, BooleanOps, Coord, LineString, MultiPolygon, Polygon};
use rstar::{primitives::GeomWithData, RTree};
use serde::{Deserialize, Serialize};

use crate::area::signed_area;
use crate::tolerances::Tolerances;
use crate::types::Point;

/// A ring of boundary points for one parcel (open or closed).
pub type Ring = [Point];

/// An overlap between two parcels.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Overlap {
    /// Index of the first parcel.
    pub a: usize,
    /// Index of the second parcel.
    pub b: usize,
    /// Area of the overlapping region (m²).
    pub area: f64,
}

/// A proposed join between a new beacon and an existing one within the search distance.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct JoinMatch {
    /// Index into the new-beacon list.
    pub new_index: usize,
    /// Index into the existing-beacon list.
    pub existing_index: usize,
    /// Separation (metres).
    pub distance: f64,
}

/// An enclosed gap (hole) left between parcels.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Gap {
    /// Boundary of the gap.
    pub ring: Vec<Point>,
    /// Area of the gap (m²).
    pub area: f64,
}

/// Two vertices on different parcels that are close but not coincident — the usual cause of
/// an open sliver between parcels.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct NearMiss {
    /// First parcel index.
    pub a_parcel: usize,
    /// Vertex index within the first parcel.
    pub a_vertex: usize,
    /// Second parcel index.
    pub b_parcel: usize,
    /// Vertex index within the second parcel.
    pub b_vertex: usize,
    /// Separation (metres).
    pub distance: f64,
}

/// The full topology report for a job's geometry.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TopologyReport {
    /// Parcels whose ring is degenerate (fewer than 3 distinct vertices).
    pub invalid_rings: Vec<usize>,
    /// Parcels smaller than the sliver-area tolerance.
    pub slivers: Vec<usize>,
    /// Overlaps between current parcels.
    pub overlaps: Vec<Overlap>,
    /// Pairs of beacons closer than the duplicate tolerance.
    pub duplicate_beacons: Vec<(usize, usize)>,
    /// Enclosed gaps (holes) in the union of parcels.
    #[serde(default)]
    pub gaps: Vec<Gap>,
    /// Rings that cross themselves.
    #[serde(default)]
    pub self_intersections: Vec<usize>,
    /// Near-coincident vertices on different parcels.
    #[serde(default)]
    pub near_misses: Vec<NearMiss>,
}

impl TopologyReport {
    /// Whether the geometry is clean (no issues of any kind).
    pub fn is_clean(&self) -> bool {
        self.invalid_rings.is_empty()
            && self.slivers.is_empty()
            && self.overlaps.is_empty()
            && self.duplicate_beacons.is_empty()
            && self.gaps.is_empty()
            && self.self_intersections.is_empty()
            && self.near_misses.is_empty()
    }
}

/// Build a `geo::Polygon` from a ring of points (`Polygon::new` closes the exterior).
fn to_polygon(ring: &Ring) -> Polygon<f64> {
    let coords: Vec<Coord<f64>> = ring.iter().map(|p| Coord { x: p.x, y: p.y }).collect();
    Polygon::new(LineString::new(coords), vec![])
}

/// Count distinct vertices in a ring, ignoring an explicitly repeated closing vertex.
fn distinct_vertices(ring: &Ring) -> usize {
    const EPS: f64 = 1e-9;
    let mut count = 0;
    for (i, p) in ring.iter().enumerate() {
        if !ring[..i]
            .iter()
            .any(|q| (q.x - p.x).abs() < EPS && (q.y - p.y).abs() < EPS)
        {
            count += 1;
        }
    }
    count
}

/// Parcels with fewer than three distinct vertices.
pub fn invalid_rings(parcels: &[&Ring]) -> Vec<usize> {
    parcels
        .iter()
        .enumerate()
        .filter(|(_, r)| distinct_vertices(r) < 3)
        .map(|(i, _)| i)
        .collect()
}

/// Parcels whose area is below `min_area` (slivers).
pub fn find_slivers(parcels: &[&Ring], min_area: f64) -> Vec<usize> {
    parcels
        .iter()
        .enumerate()
        .filter(|(_, r)| distinct_vertices(r) >= 3 && signed_area(r).abs() < min_area)
        .map(|(i, _)| i)
        .collect()
}

/// Overlaps between parcels whose shared area exceeds `min_area` (ignores sliver-sized slivers
/// of overlap caused by coordinate noise).
pub fn find_overlaps(parcels: &[&Ring], min_area: f64) -> Vec<Overlap> {
    let polys: Vec<Polygon<f64>> = parcels.iter().map(|r| to_polygon(r)).collect();

    let mut overlaps = Vec::new();
    for a in 0..polys.len() {
        if distinct_vertices(parcels[a]) < 3 {
            continue;
        }
        for b in (a + 1)..polys.len() {
            if distinct_vertices(parcels[b]) < 3 {
                continue;
            }
            let area = polys[a].intersection(&polys[b]).unsigned_area();
            if area > min_area {
                overlaps.push(Overlap { a, b, area });
            }
        }
    }
    overlaps
}

/// Pairs of beacons closer together than `tolerance` (duplicates). Each pair is reported once
/// with the lower index first.
pub fn duplicate_points(points: &[Point], tolerance: f64) -> Vec<(usize, usize)> {
    if points.is_empty() {
        return Vec::new();
    }
    let indexed: Vec<GeomWithData<[f64; 2], usize>> = points
        .iter()
        .enumerate()
        .map(|(i, p)| GeomWithData::new([p.x, p.y], i))
        .collect();
    let tree = RTree::bulk_load(indexed);
    let tol_sq = tolerance * tolerance;

    let mut pairs = Vec::new();
    for (i, p) in points.iter().enumerate() {
        for hit in tree.locate_within_distance([p.x, p.y], tol_sq) {
            let j = hit.data;
            if i < j {
                pairs.push((i, j));
            }
        }
    }
    pairs
}

/// Propose joins from new beacons to existing beacons within `max_distance` (FR-FAB-02).
///
/// For each new beacon, the nearest existing beacon within range is proposed (if any).
pub fn propose_joins(new: &[Point], existing: &[Point], max_distance: f64) -> Vec<JoinMatch> {
    if existing.is_empty() {
        return Vec::new();
    }
    let indexed: Vec<GeomWithData<[f64; 2], usize>> = existing
        .iter()
        .enumerate()
        .map(|(i, p)| GeomWithData::new([p.x, p.y], i))
        .collect();
    let tree = RTree::bulk_load(indexed);

    let mut matches = Vec::new();
    for (new_index, p) in new.iter().enumerate() {
        if let Some(hit) = tree.nearest_neighbor(&[p.x, p.y]) {
            let distance = (hit.geom()[0] - p.x).hypot(hit.geom()[1] - p.y);
            if distance <= max_distance {
                matches.push(JoinMatch {
                    new_index,
                    existing_index: hit.data,
                    distance,
                });
            }
        }
    }
    matches
}

/// Enclosed gaps between parcels: holes in the union of all valid parcel polygons.
///
/// A gap that is open to the outside is not a hole, so it is reported indirectly through
/// [`near_misses`] (boundary vertices that fail to meet) and through the overlap/sliver checks.
pub fn find_gaps(parcels: &[&Ring], min_area: f64) -> Vec<Gap> {
    let valid: Vec<Polygon<f64>> = parcels
        .iter()
        .filter(|r| distinct_vertices(r) >= 3)
        .map(|r| to_polygon(r))
        .collect();
    if valid.len() < 2 {
        return Vec::new();
    }

    let mut union = MultiPolygon::new(vec![valid[0].clone()]);
    for p in &valid[1..] {
        union = union.union(&MultiPolygon::new(vec![p.clone()]));
    }

    let mut gaps = Vec::new();
    for polygon in &union.0 {
        for interior in polygon.interiors() {
            let ring: Vec<Point> = interior
                .coords()
                .map(|c| Point::new(c.x, c.y))
                .collect();
            let area = signed_area(&ring).abs();
            if area >= min_area {
                gaps.push(Gap { ring, area });
            }
        }
    }
    gaps.sort_by(|a, b| b.area.total_cmp(&a.area));
    gaps
}

/// Parcels whose boundary crosses itself.
pub fn find_self_intersections(parcels: &[&Ring]) -> Vec<usize> {
    parcels
        .iter()
        .enumerate()
        .filter(|(_, r)| ring_self_intersects(r))
        .map(|(i, _)| i)
        .collect()
}

/// Vertices on different parcels that are within `tolerance` but not coincident.
///
/// This is the review signal for a boundary that almost joins: the parcels are topologically
/// separate even though they look closed at drawing scale.
pub fn near_misses(parcels: &[&Ring], tolerance: f64) -> Vec<NearMiss> {
    if parcels.len() < 2 {
        return Vec::new();
    }
    let indexed: Vec<GeomWithData<[f64; 2], (usize, usize)>> = parcels
        .iter()
        .enumerate()
        .flat_map(|(pi, ring)| {
            ring.iter()
                .enumerate()
                .map(move |(vi, p)| GeomWithData::new([p.x, p.y], (pi, vi)))
        })
        .collect();
    if indexed.is_empty() {
        return Vec::new();
    }
    let tree = RTree::bulk_load(indexed);
    let mut found = Vec::new();
    let mut seen = std::collections::BTreeSet::new();

    for (pi, ring) in parcels.iter().enumerate() {
        for (vi, p) in ring.iter().enumerate() {
            for hit in tree.locate_within_distance([p.x, p.y], tolerance * tolerance) {
                let (pj, vj) = hit.data;
                if pj <= pi {
                    continue;
                }
                let q = Point::new(hit.geom()[0], hit.geom()[1]);
                let distance = p.distance_to(q);
                if distance <= 1e-9 || distance > tolerance {
                    continue;
                }
                let key = (pi, vi, pj, vj);
                if seen.insert(key) {
                    found.push(NearMiss {
                        a_parcel: pi,
                        a_vertex: vi,
                        b_parcel: pj,
                        b_vertex: vj,
                        distance,
                    });
                }
            }
        }
    }
    found.sort_by(|a, b| a.distance.total_cmp(&b.distance));
    found.truncate(500);
    found
}

/// Whether a ring has two non-adjacent edges that cross.
fn ring_self_intersects(ring: &Ring) -> bool {
    let n = ring.len();
    if n < 4 {
        return false;
    }
    let closed = ring[0].distance_to(ring[n - 1]) < 1e-9;
    let edges = if closed { n - 1 } else { n };
    if edges < 3 {
        return false;
    }
    for i in 0..edges {
        let a = ring[i];
        let b = ring[(i + 1) % n];
        for j in (i + 1)..edges {
            if j == i || j == i + 1 || (i == 0 && j == edges - 1) {
                continue;
            }
            let c = ring[j];
            let d = ring[(j + 1) % n];
            if segments_cross(a, b, c, d) {
                return true;
            }
        }
    }
    false
}

fn orientation(a: Point, b: Point, c: Point) -> f64 {
    (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
}

fn segments_cross(a: Point, b: Point, c: Point, d: Point) -> bool {
    const EPS: f64 = 1e-12;
    let o1 = orientation(a, b, c);
    let o2 = orientation(a, b, d);
    let o3 = orientation(c, d, a);
    let o4 = orientation(c, d, b);
    ((o1 > EPS && o2 < -EPS) || (o1 < -EPS && o2 > EPS))
        && ((o3 > EPS && o4 < -EPS) || (o3 < -EPS && o4 > EPS))
}

/// Run every topology check before submission (FR-ADJ-04).
pub fn check_topology(parcels: &[&Ring], beacons: &[Point], tol: &Tolerances) -> TopologyReport {
    TopologyReport {
        invalid_rings: invalid_rings(parcels),
        slivers: find_slivers(parcels, tol.sliver_area),
        overlaps: find_overlaps(parcels, tol.sliver_area),
        duplicate_beacons: duplicate_points(beacons, tol.duplicate_beacon),
        gaps: find_gaps(parcels, tol.sliver_area),
        self_intersections: find_self_intersections(parcels),
        near_misses: near_misses(parcels, tol.join_distance),
    }
}
