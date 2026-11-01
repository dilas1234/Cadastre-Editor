//! Golden-file tests for survey/CAD import.
//!
//! The same parcel is tabulated three ways — a CSV coordinate list, an ASCII DXF drawing and a
//! LandXML document — with an independently computed `expected.json`. Importing any of them must
//! give the same coordinates and the same boundary area, to the millimetre and 0.01 m².
//!
//! As with the core fixtures, the files in the tree are synthetic regression fixtures; drop the
//! real plan exports in the same directory to test them without touching this file.

use std::fs;
use std::path::{Path, PathBuf};

use cadastre_core::area::polygon_area;
use cadastre_core::types::Point;
use cadastre_import::conversion::{self, ImportResult, ImportedSurvey};
use serde::Deserialize;

const GOLDEN_DIR: &str = "tests/fixtures/golden";

#[derive(Debug, Deserialize)]
struct ExpectedPoint {
    id: String,
    easting: f64,
    northing: f64,
}

#[derive(Debug, Deserialize)]
struct Expected {
    points: Vec<ExpectedPoint>,
    boundary_area: f64,
    #[serde(default)]
    epsg: Option<u32>,
}

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(GOLDEN_DIR)
}

fn read(name: &str) -> String {
    let path = golden_dir().join(name);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

fn expected() -> Expected {
    let path = golden_dir().join("expected.json");
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("{} is not a valid expectation file: {e}", path.display()))
}

fn import(name: &str) -> ImportedSurvey {
    let contents = read(name);
    match conversion::import_text(name, &contents, None) {
        Ok(ImportResult::Survey(survey)) => survey,
        Ok(other) => panic!("{name}: expected a survey, got {other:?}"),
        Err(e) => panic!("{name}: import failed: {e}"),
    }
}

fn assert_close(actual: f64, expected: f64, tolerance: f64, what: &str) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "{what}: got {actual}, expected {expected} (tolerance {tolerance})"
    );
}

fn boundary_area(survey: &ImportedSurvey) -> f64 {
    assert_eq!(
        survey.boundaries.len(),
        1,
        "expected exactly one boundary, got {:?}",
        survey
            .boundaries
            .iter()
            .map(|b| b.id.as_str())
            .collect::<Vec<_>>()
    );
    let boundary = &survey.boundaries[0];
    assert!(boundary.closed, "boundary `{}` is not closed", boundary.id);
    polygon_area(&boundary.ring).expect("boundary ring has an area")
}

fn assert_points_match(survey: &ImportedSurvey, expected: &Expected, source: &str) {
    assert_eq!(
        survey.points.len(),
        expected.points.len(),
        "{source}: point count (warnings: {:?}, issues: {:?})",
        survey.warnings,
        survey.issues
    );
    for want in &expected.points {
        let point = survey
            .points
            .iter()
            .find(|p| p.id == want.id)
            .unwrap_or_else(|| {
                panic!(
                    "{source}: point `{}` missing; imported ids are {:?}",
                    want.id,
                    survey.points.iter().map(|p| p.id.as_str()).collect::<Vec<_>>()
                )
            });
        assert_close(
            point.point.x,
            want.easting,
            1e-3,
            &format!("{source}: point {} easting", want.id),
        );
        assert_close(
            point.point.y,
            want.northing,
            1e-3,
            &format!("{source}: point {} northing", want.id),
        );
    }
}

#[test]
fn csv_dxf_and_landxml_reproduce_the_same_parcel() {
    let expected = expected();
    assert!(
        !expected.points.is_empty(),
        "expected.json has no points to compare"
    );

    let csv = import("parcel.csv");
    assert_points_match(&csv, &expected, "parcel.csv");

    let dxf = import("parcel.dxf");
    assert_points_match(&dxf, &expected, "parcel.dxf");
    let dxf_area = boundary_area(&dxf);
    assert_close(
        dxf_area,
        expected.boundary_area,
        0.01,
        "parcel.dxf: boundary area",
    );

    let landxml = import("parcel.xml");
    assert_points_match(&landxml, &expected, "parcel.xml");
    let xml_area = boundary_area(&landxml);
    assert_close(
        xml_area,
        expected.boundary_area,
        0.01,
        "parcel.xml: boundary area",
    );

    if let Some(epsg) = expected.epsg {
        assert_eq!(landxml.epsg, Some(epsg), "parcel.xml: EPSG code");
    }

    // The vector formats must agree with each other, not just with the expectation file.
    assert_close(dxf_area, xml_area, 0.01, "DXF and LandXML area agree");
    for want in &expected.points {
        let a = dxf.points.iter().find(|p| p.id == want.id).expect("DXF point");
        let b = landxml
            .points
            .iter()
            .find(|p| p.id == want.id)
            .expect("LandXML point");
        assert_close(a.point.x, b.point.x, 1e-3, "DXF/LandXML easting agree");
        assert_close(a.point.y, b.point.y, 1e-3, "DXF/LandXML northing agree");
    }
}

#[test]
fn import_is_silent_about_a_clean_coordinate_list() {
    let survey = import("parcel.csv");
    assert_eq!(survey.format, Some(conversion::ImportFormat::Coordinates));
    assert!(
        survey.issues.is_empty(),
        "clean CSV reported issues: {:?}",
        survey.issues
    );
    assert!(
        survey.warnings.is_empty(),
        "clean CSV reported warnings: {:?}",
        survey.warnings
    );
    // A bare coordinate list has no boundary geometry of its own.
    assert!(survey.boundaries.is_empty());
}

#[test]
fn world_file_sidecar_is_imported_as_raster_georeferencing() {
    // A 0.5 m orthophoto whose top-left pixel centre is at (352440.25, 1233094.75).
    let contents = "0.5\n0.0\n0.0\n-0.5\n352440.25\n1233094.75\n";
    let result = conversion::import_text("ortho.tfw", contents, None).expect("world file");
    let ImportResult::Raster(raster) = result else {
        panic!("expected a raster georeference");
    };
    assert_close(raster.world.a, 0.5, 1e-9, "world file x pixel size");
    assert_close(raster.world.e, -0.5, 1e-9, "world file y pixel size");
    assert!(!raster.world.is_rotated());
    let centre = raster.world.pixel_to_map(0.0, 0.0);
    assert_eq!(centre, Point::new(352_440.25, 1_233_094.75));
}
