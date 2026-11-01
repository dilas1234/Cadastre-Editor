//! Traverse closure: misclosure vector, bearing and precision ratio (SRS FR-COG-02).

use serde::{Deserialize, Serialize};

use crate::cogo::{self, traverse_points};
use crate::error::Result;
use crate::types::Traverse;
use crate::units::{Angle, Bearing};

/// The result of closing a traverse back onto its start.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Closure {
    /// Easting component of the misclosure (metres): computed end minus start.
    pub misclosure_east: f64,
    /// Northing component of the misclosure (metres).
    pub misclosure_north: f64,
    /// Linear misclosure: the length of the misclosure vector (metres).
    pub linear_misclosure: f64,
    /// Bearing of the misclosure vector (direction of the gap).
    pub misclosure_bearing: Bearing,
    /// Total traverse length (perimeter) used for the precision ratio (metres).
    pub perimeter: f64,
    /// Precision ratio denominator: `perimeter / linear_misclosure`. For a 1:10,000 result
    /// this is `10000.0`. Infinite when the traverse closes exactly.
    pub precision_ratio: f64,
}

impl Closure {
    /// Whether the closure meets a minimum precision (e.g. `10_000.0` for 1:10,000).
    pub fn meets_precision(&self, minimum_ratio: f64) -> bool {
        self.precision_ratio >= minimum_ratio
    }
}

/// Compute the closure of a traverse treated as a closed loop returning to its start.
///
/// FR-COG-02. The misclosure vector is the sum of all leg vectors — i.e. how far the
/// computed end falls from the start.
pub fn closure(traverse: &Traverse) -> Result<Closure> {
    cogo::require_min_courses(traverse, 1)?;

    let points = traverse_points(traverse);
    let start = points[0];
    let end = *points
        .last()
        .expect("traverse_points always returns the start");

    let misclosure_east = end.x - start.x;
    let misclosure_north = end.y - start.y;
    let linear_misclosure = misclosure_east.hypot(misclosure_north);
    // Precision is measured against the length actually surveyed, so curved legs contribute
    // their arc length rather than their shorter chord.
    let perimeter = cogo::traverse_arc_length(traverse);

    let misclosure_bearing =
        Bearing::from_angle(Angle::from_radians(misclosure_east.atan2(misclosure_north)));

    let precision_ratio = if linear_misclosure > 0.0 {
        perimeter / linear_misclosure
    } else {
        f64::INFINITY
    };

    Ok(Closure {
        misclosure_east,
        misclosure_north,
        linear_misclosure,
        misclosure_bearing,
        perimeter,
        precision_ratio,
    })
}
