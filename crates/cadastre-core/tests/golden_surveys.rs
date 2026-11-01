//! Golden-file regression tests for the survey engine.
//!
//! Every `tests/fixtures/golden/*.json` file describes a survey the way it appears on a plan
//! (a start beacon and a tabulation of legs) together with independently computed expected
//! results. The engine must reproduce those results exactly within millimetre / 0.01 m²
//! tolerances, whichever front end runs it.
//!
//! The fixtures currently in the tree are synthetic regression fixtures (clearly labelled in
//! their `source` field). Replace them with real plan tabulations when those are supplied; the
//! test itself does not change.

use std::fs;
use std::path::{Path, PathBuf};

use cadastre_core::area;
use cadastre_core::closure;
use cadastre_core::survey::SurveySession;
use cadastre_core::tolerances::Tolerances;
use cadastre_core::types::{Course, Curve, Point, Turn};
use cadastre_core::units::Bearing;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Fixture {
    name: String,
    source: String,
    crs: String,
    start: Xy,
    #[serde(default)]
    stated_area: Option<f64>,
    legs: Vec<Leg>,
    expected: Expected,
}

#[derive(Debug, Deserialize)]
struct Xy {
    x: f64,
    y: f64,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Leg {
    Line {
        bearing_deg: f64,
        distance: f64,
    },
    Curve {
        radius: f64,
        arc_length: f64,
        chord_bearing_deg: f64,
        #[serde(default)]
        turn: Turn,
    },
}

#[derive(Debug, Deserialize)]
struct Expected {
    vertices: Vec<Xy>,
    area: f64,
    perimeter: f64,
    closure: ExpectedClosure,
}

#[derive(Debug, Deserialize)]
struct ExpectedClosure {
    misclosure_east: f64,
    misclosure_north: f64,
    linear_misclosure: f64,
}

/// Every golden fixture, loaded and sorted by file name for stable output.
fn fixtures() -> Vec<(PathBuf, Fixture)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/golden");
    let mut paths: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            (path.extension().and_then(|e| e.to_str()) == Some("json")).then_some(path)
        })
        .collect();
    paths.sort();
    assert!(
        !paths.is_empty(),
        "no golden fixtures in {}; the exit criterion needs real plan tabulations here",
        dir.display()
    );
    paths
        .into_iter()
        .map(|path| {
            let text = fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
            let fixture: Fixture = serde_json::from_str(&text)
                .unwrap_or_else(|e| panic!("{} is not a valid fixture: {e}", path.display()));
            assert!(
                !fixture.name.is_empty() && !fixture.source.is_empty() && !fixture.crs.is_empty(),
                "{} must name the survey, its source and its CRS",
                path.display()
            );
            (path, fixture)
        })
        .collect()
}

fn session_for(fixture: &Fixture) -> SurveySession {
    let mut session = SurveySession::new(Point::new(fixture.start.x, fixture.start.y));
    session.set_stated_area(fixture.stated_area);
    for leg in &fixture.legs {
        match *leg {
            Leg::Line {
                bearing_deg,
                distance,
            } => session.push_course(Course::new(
                Bearing::from_degrees(bearing_deg),
                distance,
            )),
            Leg::Curve {
                radius,
                arc_length,
                chord_bearing_deg,
                turn,
            } => session.push_curve(Curve::with_turn(
                radius,
                arc_length,
                Bearing::from_degrees(chord_bearing_deg),
                turn,
            )),
        }
    }
    session
}

fn assert_close(actual: f64, expected: f64, tolerance: f64, what: &str) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "{what}: got {actual}, expected {expected} (tolerance {tolerance})"
    );
}

#[test]
fn golden_surveys_reproduce_coordinates_closure_and_area() {
    let tolerances = Tolerances::default();
    for (path, fixture) in fixtures() {
        let label = path.display();
        let session = session_for(&fixture);
        assert_eq!(
            session.len(),
            fixture.legs.len(),
            "{label}: leg count"
        );

        let vertices = session.vertices();
        assert_eq!(
            vertices.len(),
            fixture.expected.vertices.len(),
            "{label}: vertex count"
        );
        for (index, (actual, expected)) in vertices.iter().zip(&fixture.expected.vertices).enumerate() {
            assert_close(
                actual.x,
                expected.x,
                1e-3,
                &format!("{label}: vertex {index} easting"),
            );
            assert_close(
                actual.y,
                expected.y,
                1e-3,
                &format!("{label}: vertex {index} northing"),
            );
        }

        // The summary is the exact structure the editor and the server both consume.
        let summary = session.summary(&tolerances);

        let computed_area = summary.area.expect("a closed figure has an area");
        assert_close(computed_area, fixture.expected.area, 0.01, &format!("{label}: area"));

        let computed_perimeter = summary.perimeter;
        assert_close(
            computed_perimeter,
            fixture.expected.perimeter,
            1e-3,
            &format!("{label}: perimeter"),
        );

        let computed_closure = summary.closure.expect("a figure with legs closes");
        assert_close(
            computed_closure.misclosure_east,
            fixture.expected.closure.misclosure_east,
            1e-3,
            &format!("{label}: closure east"),
        );
        assert_close(
            computed_closure.misclosure_north,
            fixture.expected.closure.misclosure_north,
            1e-3,
            &format!("{label}: closure north"),
        );
        assert_close(
            computed_closure.linear_misclosure,
            fixture.expected.closure.linear_misclosure,
            1e-3,
            &format!("{label}: linear misclosure"),
        );

        // The same values must come out of the standalone area/closure entry points, because
        // headless reports call those directly rather than going through the session.
        let traverse = session.traverse();
        assert_close(
            area::traverse_area(&traverse).expect("traverse area"),
            fixture.expected.area,
            0.01,
            &format!("{label}: traverse_area"),
        );
        assert_close(
            area::traverse_perimeter(&traverse),
            fixture.expected.perimeter,
            1e-3,
            &format!("{label}: traverse_perimeter"),
        );
        let direct = closure::closure(&traverse).expect("traverse closure");
        assert_close(
            direct.linear_misclosure,
            fixture.expected.closure.linear_misclosure,
            1e-3,
            &format!("{label}: direct closure"),
        );

        if let Some(stated) = fixture.stated_area {
            let check = summary
                .area_check
                .expect("a stated area gives an area check");
            assert_close(check.stated, stated, 1e-9, &format!("{label}: stated area"));
            assert!(
                check.fraction < 1e-3,
                "{label}: computed area {computed_area} differs from stated {stated} by more than 0.1 %"
            );
        }
    }
}
