//! CRS round-trips for EPSG:26392 (Minna / Nigeria Mid Belt) and the Minna↔WGS84 datum shift.
//!
//! Note: the datum-shift parameters are provisional (SRS §9). These tests assert internal
//! self-consistency (round-trips) and projection sanity, not agreement with PLAGIS's exact
//! transformation — that is validated once the real parameters are supplied (NFR-02).

use cadastre_core::crs::{TransverseMercator, WorkingCrs};
use cadastre_core::types::Point;

/// On the central meridian, easting equals the false easting (defining property of TM).
#[test]
fn central_meridian_maps_to_false_easting() {
    let tm = TransverseMercator::nigeria_mid_belt();
    let p = tm.forward(9.0_f64.to_radians(), 8.5_f64.to_radians());
    assert!((p.x - tm.false_easting).abs() < 1e-3, "easting {}", p.x);
}

/// project → unproject recovers latitude/longitude to sub-micro-degree.
#[test]
fn project_unproject_round_trip() {
    let crs = WorkingCrs::default();
    // A spread of points across the Plateau area.
    for &(lat, lon) in &[(9.9, 8.9), (8.5, 8.5), (10.5, 9.3), (7.2, 7.9)] {
        let grid = crs.project_deg(lat, lon);
        let (lat2, lon2) = crs.unproject_deg(grid);
        assert!((lat2 - lat).abs() < 1e-8, "lat {lat} -> {lat2}");
        assert!((lon2 - lon).abs() < 1e-8, "lon {lon} -> {lon2}");
    }
}

/// grid → WGS84 → grid recovers the grid coordinate to within 1 mm.
#[test]
fn grid_wgs84_round_trip_within_mm() {
    let crs = WorkingCrs::default();
    let grid = Point::new(700_000.0, 1_050_000.0);
    let (lat, lon) = crs.grid_to_wgs84_deg(grid);
    let back = crs.wgs84_to_grid_deg(lat, lon);
    assert!(
        (back.x - grid.x).abs() < 1e-3,
        "easting {} -> {}",
        grid.x,
        back.x
    );
    assert!(
        (back.y - grid.y).abs() < 1e-3,
        "northing {} -> {}",
        grid.y,
        back.y
    );
}

/// The datum shift moves the position by roughly its magnitude (~180 m), i.e. it is applied.
#[test]
fn datum_shift_is_nonzero_and_bounded() {
    let crs = WorkingCrs::default();
    let grid = Point::new(670_553.98, 600_000.0); // on the central meridian
    let (wlat, wlon) = crs.grid_to_wgs84_deg(grid);
    let (nlat, nlon) = crs.unproject_deg(grid); // Minna lat/lon (no datum shift)
                                                // A few hundred metres is a few thousandths of a degree — present but small.
    let dlat = (wlat - nlat).abs();
    let dlon = (wlon - nlon).abs();
    assert!(dlat > 1e-5 && dlat < 1e-2, "dlat {dlat}");
    assert!(dlon < 1e-2, "dlon {dlon}");
}
