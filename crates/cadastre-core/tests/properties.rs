//! Property tests for the geometry engine (SRS NFR-14: property tests for COGO and adjustment).

use cadastre_core::adjust::bowditch;
use cadastre_core::area::signed_area;
use cadastre_core::cogo::{forward, inverse};
use cadastre_core::types::{Course, Point, Traverse};
use cadastre_core::units::Bearing;
use proptest::prelude::*;

/// A reasonable coordinate in the PLAGIS grid (metres).
fn coord() -> impl Strategy<Value = f64> {
    -500_000.0f64..2_000_000.0
}

proptest! {
    /// forward then inverse recovers the bearing and distance.
    #[test]
    fn forward_inverse_round_trip(
        x in coord(), y in coord(),
        bearing_deg in 0.0f64..360.0,
        distance in 0.1f64..5_000.0,
    ) {
        let from = Point::new(x, y);
        let bearing = Bearing::from_degrees(bearing_deg);
        let to = forward(from, bearing, distance);
        let (b2, d2) = inverse(from, to);

        prop_assert!((d2 - distance).abs() < 1e-6, "distance {d2} vs {distance}");
        // Compare bearings on the unit circle to avoid 0/360 wrap artefacts.
        let (s1, c1) = bearing.radians().sin_cos();
        let (s2, c2) = b2.radians().sin_cos();
        prop_assert!((s1 - s2).abs() < 1e-9 && (c1 - c2).abs() < 1e-9);
    }

    /// Signed area is invariant under cyclic rotation of the vertex list and under translation.
    #[test]
    fn area_invariant_under_rotation_and_translation(
        pts in prop::collection::vec((coord(), coord()), 3..8),
        shift in 1usize..7,
        tx in -1000.0f64..1000.0, ty in -1000.0f64..1000.0,
    ) {
        let ring: Vec<Point> = pts.iter().map(|&(x, y)| Point::new(x, y)).collect();
        let base = signed_area(&ring);

        // Cyclic rotation.
        let k = shift % ring.len();
        let mut rotated = ring.clone();
        rotated.rotate_left(k);
        prop_assert!((signed_area(&rotated) - base).abs() < 1e-3 * (1.0 + base.abs()));

        // Translation.
        let moved: Vec<Point> = ring.iter().map(|p| Point::new(p.x + tx, p.y + ty)).collect();
        prop_assert!((signed_area(&moved) - base).abs() < 1e-3 * (1.0 + base.abs()));
    }

    /// After a Bowditch adjustment the loop closes: the last adjusted vertex equals the start.
    #[test]
    fn bowditch_closes_the_loop(
        x in coord(), y in coord(),
        b1 in 0.0f64..360.0, d1 in 10.0f64..1_000.0,
        b2 in 0.0f64..360.0, d2 in 10.0f64..1_000.0,
        b3 in 0.0f64..360.0, d3 in 10.0f64..1_000.0,
    ) {
        let start = Point::new(x, y);
        let courses = vec![
            Course::new(Bearing::from_degrees(b1), d1),
            Course::new(Bearing::from_degrees(b2), d2),
            Course::new(Bearing::from_degrees(b3), d3),
        ];
        let traverse = Traverse::from_courses(start, courses);
        let adjusted = bowditch(&traverse).unwrap();
        let last = *adjusted.last().unwrap();
        prop_assert!(last.distance_to(start) < 1e-6, "closes to {}", last.distance_to(start));
    }
}
