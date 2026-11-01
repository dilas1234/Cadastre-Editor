//! Parcel area by the coordinate (shoelace) method, and comparison to a stated area
//! (SRS FR-COG-03).

use serde::{Deserialize, Serialize};

use crate::cogo;
use crate::error::{CoreError, Result};
use crate::types::{Point, Segment, Traverse};

/// Signed area of a ring by the shoelace formula (positive for anticlockwise ordering).
///
/// The ring may be given open or closed (a repeated final vertex is handled either way).
pub fn signed_area(ring: &[Point]) -> f64 {
    let n = ring.len();
    if n < 3 {
        return 0.0;
    }
    let mut sum = 0.0;
    for i in 0..n {
        let a = ring[i];
        let b = ring[(i + 1) % n];
        sum += a.x * b.y - b.x * a.y;
    }
    sum / 2.0
}

/// Unsigned polygon area in square metres.
///
/// FR-COG-03. Returns an error if the ring has fewer than three distinct vertices.
pub fn polygon_area(ring: &[Point]) -> Result<f64> {
    let distinct = distinct_vertex_count(ring);
    if distinct < 3 {
        return Err(CoreError::DegeneratePolygon(distinct));
    }
    Ok(signed_area(ring).abs())
}

/// How a computed parcel area compares with the area stated on the plan.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct AreaCheck {
    /// Area computed from the fabric geometry (m²).
    pub computed: f64,
    /// Area stated on the survey plan (m²).
    pub stated: f64,
    /// Signed difference `computed - stated` (m²).
    pub difference: f64,
    /// Absolute fractional difference `|difference| / stated`. `INFINITY` if `stated` is zero.
    pub fraction: f64,
}

impl AreaCheck {
    /// Whether the area agrees with the plan within a fractional tolerance (e.g. `0.01` for 1%).
    pub fn within(&self, max_fraction: f64) -> bool {
        self.fraction <= max_fraction
    }
}

/// Compare a computed area with the plan's stated area.
///
/// FR-COG-03.
pub fn area_check(computed: f64, stated: f64) -> AreaCheck {
    let difference = computed - stated;
    let fraction = if stated != 0.0 {
        difference.abs() / stated.abs()
    } else {
        f64::INFINITY
    };
    AreaCheck {
        computed,
        stated,
        difference,
        fraction,
    }
}

/// Count distinct vertices, ignoring an explicitly closed final vertex and near-duplicates.
fn distinct_vertex_count(ring: &[Point]) -> usize {
    const EPS: f64 = 1e-9;
    let mut count = 0;
    for (i, p) in ring.iter().enumerate() {
        let is_dup = ring[..i]
            .iter()
            .any(|q| (q.x - p.x).abs() < EPS && (q.y - p.y).abs() < EPS);
        if !is_dup {
            count += 1;
        }
    }
    count
}

/// Planar perimeter of a ring (metres). A repeated closing vertex is handled naturally.
pub fn perimeter(ring: &[Point]) -> f64 {
    let n = ring.len();
    if n < 2 {
        return 0.0;
    }
    (0..n)
        .map(|i| ring[i].distance_to(ring[(i + 1) % n]))
        .sum()
}

/// Area centroid of a ring, or the arithmetic mean for a degenerate (zero-area) ring.
pub fn centroid(ring: &[Point]) -> Point {
    let n = ring.len();
    if n == 0 {
        return Point::new(0.0, 0.0);
    }
    let area = signed_area(ring);
    if area.abs() < 1e-12 {
        let (sx, sy) = ring
            .iter()
            .fold((0.0, 0.0), |(sx, sy), p| (sx + p.x, sy + p.y));
        return Point::new(sx / n as f64, sy / n as f64);
    }
    let mut cx = 0.0;
    let mut cy = 0.0;
    for i in 0..n {
        let a = ring[i];
        let b = ring[(i + 1) % n];
        let cross = a.x * b.y - b.x * a.y;
        cx += (a.x + b.x) * cross;
        cy += (a.y + b.y) * cross;
    }
    Point::new(cx / (6.0 * area), cy / (6.0 * area))
}

/// Axis-aligned bounding box `(min, max)` of a point set, or `None` when empty.
pub fn bounding_box(points: &[Point]) -> Option<(Point, Point)> {
    let mut iter = points.iter();
    let first = *iter.next()?;
    let mut min = first;
    let mut max = first;
    for p in iter {
        min.x = min.x.min(p.x);
        min.y = min.y.min(p.y);
        max.x = max.x.max(p.x);
        max.y = max.y.max(p.y);
    }
    Some((min, max))
}

/// Area enclosed by a traverse, including the circular segments of any curved legs (m²).
///
/// FR-COG-03. The chord polygon gives the base shoelace area; each curve adds the exact
/// signed area between its chord and its arc. This keeps a curved parcel's area faithful to
/// the plan instead of under-measuring it by the segment area.
pub fn traverse_area(traverse: &Traverse) -> Result<f64> {
    let vertices = cogo::traverse_points(traverse);
    let distinct = distinct_vertex_count(&vertices);
    if distinct < 3 {
        return Err(CoreError::DegeneratePolygon(distinct));
    }

    let mut signed = signed_area(&vertices);
    for (i, segment) in traverse.segments.iter().enumerate() {
        if let Segment::Curve(curve) = segment {
            signed += curve.arc_area_correction(vertices[i]);
        }
    }
    Ok(signed.abs())
}

/// Total measured length around a traverse, using arc length for curved legs (metres).
pub fn traverse_perimeter(traverse: &Traverse) -> f64 {
    traverse
        .segments
        .iter()
        .map(|segment| match segment {
            Segment::Line(course) => course.distance,
            Segment::Curve(curve) => curve.arc_length,
        })
        .sum()
}
