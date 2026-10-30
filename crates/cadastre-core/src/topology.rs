//! Topology checks before submission (SRS FR-ADJ-04) and spatial join proposals (FR-FAB-02).
//!
//! Uses `geo` for robust polygon boolean operations and `rstar` for nearest-neighbour queries.

use geo::{Area, BooleanOps, Coord, LineString, Polygon};
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
}

impl TopologyReport {
    /// Whether the geometry is clean (no issues of any kind).
    pub fn is_clean(&self) -> bool {
        self.invalid_rings.is_empty()
            && self.slivers.is_empty()
            && self.overlaps.is_empty()
            && self.duplicate_beacons.is_empty()
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

/// Run every topology check before submission (FR-ADJ-04).
pub fn check_topology(parcels: &[&Ring], beacons: &[Point], tol: &Tolerances) -> TopologyReport {
    TopologyReport {
        invalid_rings: invalid_rings(parcels),
        slivers: find_slivers(parcels, tol.sliver_area),
        overlaps: find_overlaps(parcels, tol.sliver_area),
        duplicate_beacons: duplicate_points(beacons, tol.duplicate_beacon),
    }
}
