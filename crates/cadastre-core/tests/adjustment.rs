//! Least-squares network adjustment: convergence, control held fixed, and blunder detection
//! (SRS FR-ADJ-01, FR-ADJ-03).

use cadastre_core::adjust::{least_squares, AdjustmentInput, Observation};
use cadastre_core::cogo::inverse;
use cadastre_core::types::Point;

/// Build an observation between two true points, optionally with a distance blunder added.
fn obs(truth: &[Point], from: usize, to: usize, distance_blunder: f64) -> Observation {
    let (bearing, distance) = inverse(truth[from], truth[to]);
    Observation::with_sigmas(from, to, distance + distance_blunder, bearing, 0.01, 5.0)
}

#[test]
fn square_network_converges_and_flags_the_blunder() {
    // True geometry: a 100 m square. Corners 0 and 1 are control (fixed).
    let truth = vec![
        Point::new(0.0, 0.0),     // 0 control
        Point::new(100.0, 0.0),   // 1 control
        Point::new(100.0, 100.0), // 2 free
        Point::new(0.0, 100.0),   // 3 free
    ];

    // Start the free beacons off their true positions.
    let coords = vec![
        truth[0],
        truth[1],
        Point::new(102.0, 97.5),
        Point::new(-2.0, 103.0),
    ];
    let fixed = vec![true, true, false, false];

    // Edges + diagonals. Seed a 0.40 m blunder into observation index 1 (edge 1→2).
    let observations = vec![
        obs(&truth, 0, 1, 0.0),
        obs(&truth, 1, 2, 0.40), // blunder
        obs(&truth, 2, 3, 0.0),
        obs(&truth, 3, 0, 0.0),
        obs(&truth, 0, 2, 0.0),
        obs(&truth, 1, 3, 0.0),
    ];

    let input = AdjustmentInput {
        coords,
        fixed,
        observations,
    };
    let result = least_squares(&input, 50, 1e-9).expect("adjustment converges");

    // Control is untouched.
    assert_eq!(result.coords[0], truth[0]);
    assert_eq!(result.coords[1], truth[1]);

    // Free beacons land close to truth despite the blunder being spread by least squares.
    assert!(
        result.coords[2].distance_to(truth[2]) < 0.5,
        "p2 {:?}",
        result.coords[2]
    );
    assert!(
        result.coords[3].distance_to(truth[3]) < 0.5,
        "p3 {:?}",
        result.coords[3]
    );

    // The blundered line shows the largest residual and is flagged.
    let flagged = result.blunders(0.05, 30.0);
    assert!(
        flagged.contains(&1),
        "expected obs 1 flagged, got {flagged:?}"
    );
}

#[test]
fn adjustment_without_control_is_rejected() {
    let coords = vec![Point::new(0.0, 0.0), Point::new(10.0, 0.0)];
    let fixed = vec![false, false];
    let observations = vec![Observation::with_sigmas(
        0,
        1,
        10.0,
        inverse(coords[0], coords[1]).0,
        0.01,
        5.0,
    )];
    let input = AdjustmentInput {
        coords,
        fixed,
        observations,
    };
    assert!(least_squares(&input, 10, 1e-9).is_err());
}
