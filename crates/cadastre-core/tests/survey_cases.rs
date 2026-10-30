//! Reference survey cases, hand-checkable, asserting the SRS accuracy target: COGO and area
//! results match to 1 mm and 0.01 m² (NFR-01).

use cadastre_core::area::{area_check, polygon_area};
use cadastre_core::closure::closure;
use cadastre_core::cogo::{forward, inverse, traverse_points};
use cadastre_core::types::{Course, Point, Traverse};
use cadastre_core::units::Bearing;

/// 1 mm linear tolerance.
const MM: f64 = 1e-3;
/// 0.01 m² area tolerance.
const AREA_TOL: f64 = 0.01;

/// A simple 100 m east leg lands exactly 100 m east, 0 m north.
#[test]
fn forward_due_east() {
    let p = forward(
        Point::new(1000.0, 2000.0),
        Bearing::from_degrees(90.0),
        100.0,
    );
    assert!((p.x - 1100.0).abs() < MM, "easting {}", p.x);
    assert!((p.y - 2000.0).abs() < MM, "northing {}", p.y);
}

/// Forward then inverse recovers the original bearing and distance.
#[test]
fn forward_inverse_round_trip() {
    let from = Point::new(500_000.0, 1_000_000.0);
    let bearing = Bearing::from_dms(37, 12, 45.0);
    let distance = 234.567;
    let to = forward(from, bearing, distance);
    let (b2, d2) = inverse(from, to);
    assert!((d2 - distance).abs() < MM, "distance {d2}");
    assert!(
        (b2.degrees() - bearing.degrees()).abs() < 1e-7,
        "bearing {}",
        b2.degrees()
    );
}

/// A perfect 100 m × 50 m rectangle: closure is exact, area is 5000 m².
///
/// Courses: E 100, N 50, W 100, S 50 — back to the start.
#[test]
fn rectangle_closes_and_area_is_exact() {
    let start = Point::new(0.0, 0.0);
    let courses = vec![
        Course::new(Bearing::from_degrees(90.0), 100.0), // east
        Course::new(Bearing::from_degrees(0.0), 50.0),   // north
        Course::new(Bearing::from_degrees(270.0), 100.0), // west
        Course::new(Bearing::from_degrees(180.0), 50.0), // south
    ];
    let traverse = Traverse::from_courses(start, courses);

    let c = closure(&traverse).unwrap();
    assert!(
        c.linear_misclosure < MM,
        "misclosure {}",
        c.linear_misclosure
    );
    // A rectangle closes to within floating-point rounding (~1e-13 m), so the precision ratio
    // is astronomically large rather than literally infinite.
    assert!(c.precision_ratio > 1e9, "ratio {}", c.precision_ratio);

    // Area from the four corners (drop the duplicated closing point).
    let mut ring = traverse_points(&traverse);
    ring.pop();
    let area = polygon_area(&ring).unwrap();
    assert!((area - 5000.0).abs() < AREA_TOL, "area {area}");

    let check = area_check(area, 5000.0);
    assert!(check.within(0.01));
}

/// A closed polygon traverse with a small deliberate misclosure gives a sensible precision ratio.
#[test]
fn precision_ratio_of_closed_traverse() {
    // Four ~200 m legs that don't quite close: the last leg is 0.05 m short.
    let start = Point::new(100_000.0, 200_000.0);
    let courses = vec![
        Course::new(Bearing::from_degrees(90.0), 200.0),
        Course::new(Bearing::from_degrees(0.0), 200.0),
        Course::new(Bearing::from_degrees(270.0), 200.0),
        Course::new(Bearing::from_degrees(180.0), 199.95), // 0.05 m short
    ];
    let traverse = Traverse::from_courses(start, courses);
    let c = closure(&traverse).unwrap();

    assert!(
        (c.linear_misclosure - 0.05).abs() < MM,
        "misclosure {}",
        c.linear_misclosure
    );
    // Perimeter ~799.95 m over 0.05 m ≈ 1:15999.
    assert!(
        c.precision_ratio > 15_000.0 && c.precision_ratio < 17_000.0,
        "ratio {}",
        c.precision_ratio
    );
}

/// A right-angled triangle 3-4-5 has area 6 m².
#[test]
fn triangle_area() {
    let ring = [
        Point::new(0.0, 0.0),
        Point::new(4.0, 0.0),
        Point::new(0.0, 3.0),
    ];
    let area = polygon_area(&ring).unwrap();
    assert!((area - 6.0).abs() < AREA_TOL, "area {area}");
}
