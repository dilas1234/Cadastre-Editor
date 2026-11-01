//! Circular and transition curve geometry for COGO (SRS FR-COG-01, FR-SUR-05).
//!
//! A simple circular curve is defined by its point of curve (PC), radius, deflection angle and
//! direction of curvature. All bearings are whole-circle, clockwise from grid north. The arc
//! is traversed from PC to PT; a right-hand curve increases the tangent bearing.

use serde::{Deserialize, Serialize};

use crate::cogo;
use crate::error::{CoreError, Result};
use crate::types::{Point, Turn};
use crate::units::Bearing;

/// The standard components of a simple circular curve.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CurveComponents {
    /// Radius (m).
    pub radius: f64,
    /// Deflection angle between the two tangents (radians, positive magnitude).
    pub delta: f64,
    /// Tangent distance from PC/PI and PI/PT (m).
    pub tangent: f64,
    /// Chord length PC→PT (m).
    pub chord: f64,
    /// External distance PI→mid-arc (m).
    pub external: f64,
    /// Mid-ordinate from chord midpoint to arc (m).
    pub mid_ordinate: f64,
    /// Arc length PC→PT (m).
    pub arc_length: f64,
    /// Degree of curve using the 100 m arc definition (degrees).
    pub degree_of_curve: f64,
}

/// A fully resolved simple circular curve.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CircularCurve {
    /// Point of curve (start of arc).
    pub pc: Point,
    /// Point of intersection of the two tangents.
    pub pi: Point,
    /// Point of tangent (end of arc).
    pub pt: Point,
    /// Arc centre.
    pub centre: Point,
    /// Radius (m).
    pub radius: f64,
    /// Deflection angle magnitude (radians).
    pub delta: f64,
    /// Direction of curvature.
    pub turn: Turn,
    /// Tangent distance (m).
    pub tangent: f64,
    /// Chord length (m).
    pub chord: f64,
    /// External distance (m).
    pub external: f64,
    /// Mid-ordinate (m).
    pub mid_ordinate: f64,
    /// Arc length (m).
    pub arc_length: f64,
    /// Whole-circle bearing of the tangent at PC.
    pub initial_bearing: Bearing,
    /// Whole-circle bearing of the tangent at PT.
    pub final_bearing: Bearing,
    /// Whole-circle bearing of the chord PC→PT.
    pub chord_bearing: Bearing,
}

/// A point on a circular curve, with the tangent direction there.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CurvePoint {
    /// Coordinates of the point.
    pub point: Point,
    /// Whole-circle tangent bearing at the point.
    pub tangent_bearing: Bearing,
    /// Arc distance from PC (m).
    pub arc_distance: f64,
    /// Signed perpendicular offset applied (positive right of travel).
    pub offset: f64,
}

/// The standard tangent/chord/external components for a radius and deflection angle.
pub fn components(radius: f64, delta: f64) -> Result<CurveComponents> {
    if !radius.is_finite() || radius <= 0.0 {
        return Err(CoreError::InvalidInput(
            "curve radius must be positive".to_string(),
        ));
    }
    if !delta.is_finite() || delta <= 0.0 || delta >= std::f64::consts::TAU {
        return Err(CoreError::InvalidInput(
            "deflection angle must be in (0, 360°) ".to_string(),
        ));
    }
    let half = delta / 2.0;
    Ok(CurveComponents {
        radius,
        delta,
        tangent: radius * half.tan(),
        chord: 2.0 * radius * half.sin(),
        external: radius * (1.0 / half.cos() - 1.0),
        mid_ordinate: radius * (1.0 - half.cos()),
        arc_length: radius * delta,
        degree_of_curve: degree_of_curve(radius, 100.0),
    })
}

/// The degree of curve for a radius, using an arc basis of `arc_basis` metres.
///
/// The conventional 100 m definition gives `D = 180·100 / (π·R)` degrees.
pub fn degree_of_curve(radius: f64, arc_basis: f64) -> f64 {
    if radius <= 0.0 || arc_basis <= 0.0 {
        return 0.0;
    }
    (arc_basis / radius).to_degrees()
}

/// The radius for a degree of curve on an arc basis of `arc_basis` metres.
pub fn radius_from_degree(degree: f64, arc_basis: f64) -> Result<f64> {
    if degree <= 0.0 || arc_basis <= 0.0 {
        return Err(CoreError::InvalidInput(
            "degree of curve and arc basis must be positive".to_string(),
        ));
    }
    Ok(arc_basis / degree.to_radians())
}

/// A curve from PC, initial tangent bearing, radius, deflection angle and handedness.
pub fn from_radius_delta(
    pc: Point,
    initial_bearing: Bearing,
    radius: f64,
    delta: f64,
    turn: Turn,
) -> Result<CircularCurve> {
    let c = components(radius, delta)?;
    let sign = turn.sign();
    let final_bearing = Bearing::from_angle(crate::units::Angle::from_radians(
        initial_bearing.radians() + sign * delta,
    ));
    let centre = cogo::forward(
        pc,
        Bearing::from_angle(crate::units::Angle::from_radians(
            initial_bearing.radians() + sign * std::f64::consts::FRAC_PI_2,
        )),
        radius,
    );
    // Compute PT from the centre so semicircles (parallel tangents) are exact rather than
    // relying on the tangent distance, which diverges at Δ = 180°.
    let radius_bearing_pc = cogo::inverse(centre, pc).0;
    let pt = cogo::forward(
        centre,
        Bearing::from_angle(crate::units::Angle::from_radians(
            radius_bearing_pc.radians() + sign * delta,
        )),
        radius,
    );
    let chord_bearing = cogo::inverse(pc, pt).0;
    // At a reversal the tangents are parallel and PI is at infinity; report the chord midpoint
    // as a finite placeholder (no coordinate computation depends on PI).
    let pi = if delta < std::f64::consts::PI - 1e-9 {
        cogo::forward(pc, initial_bearing, c.tangent)
    } else {
        cogo::midpoint(pc, pt, 0.5)
    };

    Ok(CircularCurve {
        pc,
        pi,
        pt,
        centre,
        radius,
        delta,
        turn,
        tangent: c.tangent,
        chord: c.chord,
        external: c.external,
        mid_ordinate: c.mid_ordinate,
        arc_length: c.arc_length,
        initial_bearing,
        final_bearing,
        chord_bearing,
    })
}

/// A curve fitted between two tangents meeting at `pi`.
///
/// `incoming` is the bearing along the first tangent towards PI; `outgoing` is the bearing
/// along the second tangent away from PI. The shorter (non-reflex) deflection is used.
pub fn from_tangents(
    pi: Point,
    incoming: Bearing,
    outgoing: Bearing,
    radius: f64,
) -> Result<CircularCurve> {
    if radius <= 0.0 {
        return Err(CoreError::InvalidInput(
            "curve radius must be positive".to_string(),
        ));
    }
    let delta = wrap_to_pi(outgoing.radians() - incoming.radians());
    if delta.abs() < 1e-12 {
        return Err(CoreError::NoSolution(
            "the two tangents are parallel".to_string(),
        ));
    }
    if delta.abs() >= std::f64::consts::PI - 1e-9 {
        return Err(CoreError::NoSolution(
            "the deflection is a reversal (180°); use a three-point curve".to_string(),
        ));
    }
    let turn = if delta > 0.0 { Turn::Right } else { Turn::Left };
    let magnitude = delta.abs();
    let tangent = radius * (magnitude / 2.0).tan();
    let pc = cogo::forward(pi, incoming.back(), tangent);
    from_radius_delta(pc, incoming, radius, magnitude, turn)
}

/// A circular curve passing through three points, in order.
///
/// The arc is the one from `a` to `c` that passes through `b`. At least one of the points must
/// not be collinear with the others.
pub fn through_three_points(a: Point, b: Point, c: Point) -> Result<CircularCurve> {
    let d = 2.0
        * (a.x * (b.y - c.y) + b.x * (c.y - a.y) + c.x * (a.y - b.y));
    if d.abs() < 1e-9 {
        return Err(CoreError::NoSolution(
            "the three points are collinear".to_string(),
        ));
    }
    let a2 = a.x * a.x + a.y * a.y;
    let b2 = b.x * b.x + b.y * b.y;
    let c2 = c.x * c.x + c.y * c.y;
    let centre = Point::new(
        (a2 * (b.y - c.y) + b2 * (c.y - a.y) + c2 * (a.y - b.y)) / d,
        (a2 * (c.x - b.x) + b2 * (a.x - c.x) + c2 * (b.x - a.x)) / d,
    );
    let radius = centre.distance_to(a);
    if radius < 1e-9 {
        return Err(CoreError::NoSolution("the three points coincide".to_string()));
    }

    let angle_a = (a.y - centre.y).atan2(a.x - centre.x);
    let angle_b = (b.y - centre.y).atan2(b.x - centre.x);
    let angle_c = (c.y - centre.y).atan2(c.x - centre.x);

    // Try both sweep directions; exactly one passes through `b`.
    let ccw_sweep = positive_sweep(angle_a, angle_c);
    let ccw_mid = positive_sweep(angle_a, angle_b);
    let (sweep, turn) = if ccw_mid <= ccw_sweep + 1e-9 {
        (ccw_sweep, Turn::Left)
    } else {
        (positive_sweep(angle_c, angle_a), Turn::Right)
    };
    if sweep <= 1e-9 {
        return Err(CoreError::NoSolution("the three points are coincident".to_string()));
    }
    if sweep >= std::f64::consts::PI - 1e-9 {
        return Err(CoreError::NoSolution(
            "the three-point arc is a half-circle or larger; use radius + delta".to_string(),
        ));
    }

    // Tangent direction in mathematical angle, converted to a survey bearing (0 = north,
    // increasing clockwise). A right turn rotates the tangent clockwise (negative in maths).
    let tangent_math = angle_a - turn.sign() * std::f64::consts::FRAC_PI_2;
    let initial = Bearing::from_angle(crate::units::Angle::from_radians(
        std::f64::consts::FRAC_PI_2 - tangent_math,
    ));
    let curve = from_radius_delta(a, initial, radius, sweep, turn)?;
    // The construction should land exactly on `c`; reject numerically bad input rather than
    // silently returning a curve that misses the third point.
    if curve.pt.distance_to(c) > 1e-6 {
        return Err(CoreError::NoSolution(
            "the three points do not form a usable simple curve".to_string(),
        ));
    }
    Ok(curve)
}

/// The point at `arc_distance` along a resolved curve, optionally offset (positive right).
pub fn point_on_curve(
    curve: &CircularCurve,
    arc_distance: f64,
    offset: f64,
) -> Result<CurvePoint> {
    if arc_distance < -1e-9 || arc_distance > curve.arc_length + 1e-9 {
        return Err(CoreError::InvalidInput(
            "arc distance is outside the curve".to_string(),
        ));
    }
    let theta = (arc_distance / curve.radius) * curve.turn.sign();
    let radius_bearing_pc = cogo::inverse(curve.centre, curve.pc).0;
    let radius_bearing = Bearing::from_angle(crate::units::Angle::from_radians(
        radius_bearing_pc.radians() + theta,
    ));
    let on_arc = cogo::forward(curve.centre, radius_bearing, curve.radius);
    let tangent_bearing = Bearing::from_angle(crate::units::Angle::from_radians(
        curve.initial_bearing.radians() + theta,
    ));
    let point = if offset == 0.0 {
        on_arc
    } else {
        cogo::offset(on_arc, tangent_bearing, offset)
    };
    Ok(CurvePoint {
        point,
        tangent_bearing,
        arc_distance,
        offset,
    })
}

/// A clothoid (Euler spiral) transition's standard components.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Spiral {
    /// Radius at the end of the spiral (m).
    pub radius: f64,
    /// Spiral length (m).
    pub length: f64,
    /// Spiral angle `θs = L / (2R)` (radians).
    pub theta: f64,
    /// Shift of the circular curve `p` (m).
    pub shift: f64,
    /// Offset of the spiral from the tangent `k` (m).
    pub offset_k: f64,
    /// Local x of the spiral end (m).
    pub local_x: f64,
    /// Local y of the spiral end (m).
    pub local_y: f64,
    /// Long tangent (m).
    pub long_tangent: f64,
    /// Short tangent (m).
    pub short_tangent: f64,
}

/// Compute the standard components of a clothoid transition of length `length` ending at
/// radius `radius`. Series forms are used (accurate for the usual `L ≪ R` range).
pub fn spiral(radius: f64, length: f64) -> Result<Spiral> {
    if radius <= 0.0 || length <= 0.0 {
        return Err(CoreError::InvalidInput(
            "spiral radius and length must be positive".to_string(),
        ));
    }
    let theta = length / (2.0 * radius);
    let r2 = radius * radius;
    let r3 = r2 * radius;
    let r4 = r2 * r2;
    let r5 = r4 * radius;
    let p = length * length / (24.0 * radius) - length.powi(4) / (2688.0 * r3);
    let k = length / 2.0 - length.powi(3) / (240.0 * r2);
    let x = length - length.powi(3) / (40.0 * r2) + length.powi(5) / (3456.0 * r4);
    let y = length * length / (6.0 * radius) - length.powi(4) / (336.0 * r3)
        + length.powi(6) / (42240.0 * r5);
    let long_tangent = x - y / theta.tan();
    let short_tangent = y / theta.sin();
    Ok(Spiral {
        radius,
        length,
        theta,
        shift: p,
        offset_k: k,
        local_x: x,
        local_y: y,
        long_tangent,
        short_tangent,
    })
}

/// Wrap an angle into `(-π, π]`.
fn wrap_to_pi(mut r: f64) -> f64 {
    use std::f64::consts::{PI, TAU};
    r = (r + PI) % TAU;
    if r <= 0.0 {
        r += TAU;
    }
    r - PI
}

/// The positive angular sweep from `from` to `to`, in `[0, 2π)`.
fn positive_sweep(from: f64, to: f64) -> f64 {
    use std::f64::consts::TAU;
    let mut sweep = (to - from) % TAU;
    if sweep < 0.0 {
        sweep += TAU;
    }
    sweep
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Turn;
    use crate::units::Angle;

    #[test]
    fn quarter_circle_components_are_standard() {
        let c = components(100.0, std::f64::consts::FRAC_PI_2).unwrap();
        assert!((c.tangent - 100.0).abs() < 1e-9);
        assert!((c.chord - 141.421_356_237_309_5).abs() < 1e-9);
        assert!((c.arc_length - 157.079_632_679_586_47).abs() < 1e-9);
        assert!((c.external - 41.421_356_237_309_48).abs() < 1e-9);
        assert!((c.mid_ordinate - 29.289_321_880_660_6).abs() < 1e-9);
    }

    #[test]
    fn radius_delta_curve_lands_on_the_tangent_point() {
        let pc = Point::new(1000.0, 2000.0);
        let initial = Bearing::from_degrees(0.0);
        let curve = from_radius_delta(
            pc,
            initial,
            100.0,
            std::f64::consts::FRAC_PI_2,
            Turn::Right,
        )
        .unwrap();
        // A 90° right curve from north ends 100 m east and 100 m north of PC.
        assert!(curve.pt.distance_to(Point::new(1100.0, 2100.0)) < 1e-6);
        assert!(curve.centre.distance_to(Point::new(1100.0, 2000.0)) < 1e-6);
        assert!((curve.chord_bearing.degrees() - 45.0).abs() < 1e-9);
    }

    #[test]
    fn curve_from_tangents_matches_radius_delta() {
        let pi = Point::new(0.0, 0.0);
        let incoming = Bearing::from_degrees(0.0);
        let outgoing = Bearing::from_degrees(90.0);
        let c = from_tangents(pi, incoming, outgoing, 100.0).unwrap();
        assert!((c.tangent - 100.0).abs() < 1e-6);
        assert!(c.pc.distance_to(Point::new(0.0, -100.0)) < 1e-6);
        assert!(c.pt.distance_to(Point::new(100.0, 0.0)) < 1e-6);
    }

    #[test]
    fn three_point_curve_passes_through_all_three() {
        // Shallow arc through (0,0), (50,30), (100,0); centre (50, -80/3), R = 170/3.
        let c = through_three_points(
            Point::new(0.0, 0.0),
            Point::new(50.0, 30.0),
            Point::new(100.0, 0.0),
        )
        .unwrap();
        assert!((c.radius - 170.0 / 3.0).abs() < 1e-6);
        assert!(c.centre.distance_to(Point::new(50.0, -80.0 / 3.0)) < 1e-6);
        assert!(c.pc.distance_to(Point::new(0.0, 0.0)) < 1e-6);
        assert!(c.pt.distance_to(Point::new(100.0, 0.0)) < 1e-6);
        assert!(c.chord_bearing.degrees() > 89.0 && c.chord_bearing.degrees() < 91.0);
    }

    #[test]
    fn point_on_curve_tracks_arc_length() {
        let curve = from_radius_delta(
            Point::new(0.0, 0.0),
            Bearing::from_degrees(0.0),
            100.0,
            std::f64::consts::FRAC_PI_2,
            Turn::Right,
        )
        .unwrap();
        let p = point_on_curve(&curve, curve.arc_length, 0.0).unwrap();
        assert!(p.point.distance_to(curve.pt) < 1e-6);
        assert!((p.tangent_bearing.degrees() - 90.0).abs() < 1e-9);
    }

    #[test]
    fn degree_of_curve_round_trips() {
        let d = degree_of_curve(100.0, 100.0);
        let r = radius_from_degree(d, 100.0).unwrap();
        assert!((r - 100.0).abs() < 1e-9);
        assert!((d - 57.295_779_513_082_31).abs() < 1e-9);
    }

    #[test]
    fn spiral_components_are_positive_and_ordered() {
        let s = spiral(300.0, 60.0).unwrap();
        assert!(s.theta > 0.0);
        assert!(s.local_x > s.local_y);
        assert!(s.shift > 0.0);
        assert!(s.long_tangent > s.short_tangent);
    }

    #[test]
    fn degenerate_curves_are_rejected() {
        assert!(components(0.0, Angle::from_degrees(90.0).radians()).is_err());
        assert!(through_three_points(
            Point::new(0.0, 0.0),
            Point::new(1.0, 1.0),
            Point::new(2.0, 2.0)
        )
        .is_err());
        assert!(from_tangents(
            Point::new(0.0, 0.0),
            Bearing::from_degrees(0.0),
            Bearing::from_degrees(0.0),
            100.0
        )
        .is_err());
    }
}
