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

/// The total measured length of a traverse, using arc length for curved legs.
///
/// Unlike [`traverse_length`] (which reduces curves to chords for coordinate running), this is
/// the distance a surveyor actually measured, so it is the correct perimeter for precision.
pub fn traverse_arc_length(traverse: &Traverse) -> f64 {
    traverse
        .segments
        .iter()
        .map(|segment| match segment {
            crate::types::Segment::Line(course) => course.distance,
            crate::types::Segment::Curve(curve) => curve.arc_length,
        })
        .sum()
}

/// The intersection of two lines through point pairs (`a1→a2` and `b1→b2`).
///
/// Returns `None` when either line is degenerate or the lines are parallel.
pub fn line_intersection(a1: Point, a2: Point, b1: Point, b2: Point) -> Option<Point> {
    if a1.distance_to(a2) < 1e-12 || b1.distance_to(b2) < 1e-12 {
        return None;
    }
    let ba = inverse(a1, a2).0;
    let bb = inverse(b1, b2).0;
    intersection(a1, ba, b1, bb)
}

/// The intersection of a ray (point + bearing) and a circle (centre + radius).
///
/// Returns zero, one or two points.
pub fn intersect_bearing_distance(
    from: Point,
    bearing: Bearing,
    centre: Point,
    radius: f64,
) -> Vec<Point> {
    if radius < 0.0 {
        return Vec::new();
    }
    let dir = Point::new(bearing.radians().sin(), bearing.radians().cos());
    let f = Point::new(from.x - centre.x, from.y - centre.y);
    let b = f.x * dir.x + f.y * dir.y;
    let c = f.x * f.x + f.y * f.y - radius * radius;
    let disc = b * b - c;
    if disc < -1e-12 {
        return Vec::new();
    }
    let root = disc.max(0.0).sqrt();
    let mut hits: Vec<Point> = [-b - root, -b + root]
        .into_iter()
        .map(|t| Point::new(from.x + t * dir.x, from.y + t * dir.y))
        .collect();
    if hits.len() == 2 && hits[0].distance_to(hits[1]) < 1e-9 {
        hits.truncate(1);
    }
    hits
}

/// The intersections of two circles (centres `c1`, `c2`; radii `r1`, `r2`).
///
/// Returns zero, one or two points.
pub fn intersect_distances(c1: Point, r1: f64, c2: Point, r2: f64) -> Vec<Point> {
    if r1 < 0.0 || r2 < 0.0 {
        return Vec::new();
    }
    let d = c1.distance_to(c2);
    if d < 1e-12 {
        return Vec::new();
    }
    if d > r1 + r2 + 1e-12 || d < (r1 - r2).abs() - 1e-12 {
        return Vec::new();
    }
    let a = (r1 * r1 - r2 * r2 + d * d) / (2.0 * d);
    let h2 = r1 * r1 - a * a;
    let h = h2.max(0.0).sqrt();
    let (ux, uy) = ((c2.x - c1.x) / d, (c2.y - c1.y) / d);
    let base = Point::new(c1.x + a * ux, c1.y + a * uy);
    let perp = Point::new(-uy, ux);
    let p1 = Point::new(base.x + h * perp.x, base.y + h * perp.y);
    let p2 = Point::new(base.x - h * perp.x, base.y - h * perp.y);
    if p1.distance_to(p2) < 1e-9 {
        vec![p1]
    } else {
        vec![p1, p2]
    }
}

/// A point at `distance` along the line `from→to`, offset perpendicular by `offset` metres.
///
/// Positive `offset` is to the right of the `from→to` direction. Returns `None` for a
/// degenerate line.
pub fn point_on_line(from: Point, to: Point, distance: f64, offset: f64) -> Option<Point> {
    let (bearing, length) = inverse(from, to);
    if length < 1e-12 {
        return None;
    }
    let along = forward(from, bearing, distance);
    Some(offset_point(along, bearing, offset))
}

/// The foot of the perpendicular from `p` onto the line `a→b`.
///
/// Returns `(foot, along, offset)` where `along` is the distance from `a` and `offset` is the
/// signed perpendicular distance (positive to the right of `a→b`).
pub fn perpendicular_foot(p: Point, a: Point, b: Point) -> Option<(Point, f64, f64)> {
    let (bearing, length) = inverse(a, b);
    if length < 1e-12 {
        return None;
    }
    let (sin, cos) = (bearing.radians().sin(), bearing.radians().cos());
    let dx = p.x - a.x;
    let dy = p.y - a.y;
    let along = dx * sin + dy * cos;
    let offset = dx * cos - dy * sin;
    let foot = forward(a, bearing, along);
    Some((foot, along, offset))
}

/// Divide the line `a→b` into `parts` equal segments, returning `parts + 1` points including
/// both endpoints. `parts == 0` returns just the endpoints.
pub fn divide_line(a: Point, b: Point, parts: usize) -> Vec<Point> {
    if parts == 0 {
        return vec![a, b];
    }
    (0..=parts)
        .map(|i| {
            let t = i as f64 / parts as f64;
            Point::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t)
        })
        .collect()
}

/// The midpoint of `a→b`, or the point at `ratio` (0 = a, 1 = b) when one is wanted.
pub fn midpoint(a: Point, b: Point, ratio: f64) -> Point {
    Point::new(
        a.x + (b.x - a.x) * ratio,
        a.y + (b.y - a.y) * ratio,
    )
}

/// Offset a point perpendicular to a travel bearing by a signed distance.
///
/// FR-COG-05. Positive `offset` is to the **right** of the travel direction
/// (bearing + 90°), negative to the left.
pub fn offset_point(point: Point, travel: Bearing, offset: f64) -> Point {
    crate::cogo::offset(point, travel, offset)
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
