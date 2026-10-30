//! Coordinate geometry: forward/inverse computation, traverse running, intersection, offset
//! (SRS FR-COG-01, FR-COG-05).
//!
//! Convention: bearings are whole-circle, clockwise from grid north. For a bearing `β`,
//! a step of distance `d` moves `d·sin β` in easting and `d·cos β` in northing.

use crate::error::{CoreError, Result};
use crate::types::{Point, Traverse};
use crate::units::{Angle, Bearing};

/// Compute a new point from a start, a bearing and a distance (forward / COGO).
///
/// FR-COG-01: the primitive from which beacon coordinates are built.
pub fn forward(from: Point, bearing: Bearing, distance: f64) -> Point {
    let a = bearing.radians();
    Point::new(from.x + distance * a.sin(), from.y + distance * a.cos())
}

/// The bearing and distance from one point to another (inverse).
///
/// FR-COG-05. Returns a normalised whole-circle bearing and a non-negative distance.
pub fn inverse(from: Point, to: Point) -> (Bearing, f64) {
    let de = to.x - from.x;
    let dn = to.y - from.y;
    let bearing = Bearing::from_angle(Angle::from_radians(de.atan2(dn)));
    (bearing, de.hypot(dn))
}

/// Run a traverse, returning every vertex starting with the traverse's start point.
///
/// The result has `segments.len() + 1` points. Curves are reduced to their chords.
pub fn traverse_points(traverse: &Traverse) -> Vec<Point> {
    let mut points = Vec::with_capacity(traverse.segments.len() + 1);
    let mut current = traverse.start;
    points.push(current);
    for segment in &traverse.segments {
        let course = segment.as_course();
        current = forward(current, course.bearing, course.distance);
        points.push(current);
    }
    points
}

/// The intersection of two lines, each given by a point and a bearing.
///
/// FR-COG-05. Returns `None` when the lines are parallel (or anti-parallel).
pub fn intersection(p1: Point, b1: Bearing, p2: Point, b2: Bearing) -> Option<Point> {
    // Direction vectors (east, north) for each bearing.
    let (s1, c1) = (b1.radians().sin(), b1.radians().cos());
    let (s2, c2) = (b2.radians().sin(), b2.radians().cos());

    // Solve p1 + t·d1 = p2 + u·d2 for t. Denominator is the 2D cross product of directions.
    let denom = s1 * c2 - c1 * s2;
    if denom.abs() < 1e-12 {
        return None;
    }
    let dx = p2.x - p1.x;
    let dy = p2.y - p1.y;
    let t = (dx * c2 - dy * s2) / denom;
    Some(Point::new(p1.x + t * s1, p1.y + t * c1))
}

/// Offset a point perpendicular to a travel bearing by a signed distance.
///
/// FR-COG-05. Positive `offset` is to the **right** of the travel direction
/// (bearing + 90°), negative to the left.
pub fn offset(point: Point, travel: Bearing, offset: f64) -> Point {
    let perp = Bearing::from_angle(Angle::from_radians(
        travel.radians() + std::f64::consts::FRAC_PI_2,
    ));
    forward(point, perp, offset)
}

/// The total plane length of a traverse (sum of leg/chord lengths).
pub fn traverse_length(traverse: &Traverse) -> f64 {
    traverse
        .segments
        .iter()
        .map(|s| s.as_course().distance)
        .sum()
}

/// Validate that a traverse has at least `min` legs, returning a descriptive error otherwise.
pub(crate) fn require_min_courses(traverse: &Traverse, min: usize) -> Result<()> {
    if traverse.len() < min {
        return Err(CoreError::TooFewCourses {
            min,
            got: traverse.len(),
        });
    }
    Ok(())
}
