//! Parcel area by the coordinate (shoelace) method, and comparison to a stated area
//! (SRS FR-COG-03).

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};
use crate::types::Point;

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
