//! PostGIS staging-load tests (feature `sink`).
//!
//! Run with: `cargo test -p cadastre-import --features sink` and `CADASTRE_TEST_DATABASE_URL` set
//! to a local PostGIS. Without the variable the test skips; without the feature it is not built.
#![cfg(feature = "sink")]

use cadastre_import::exceptions::stage_parcels;
use cadastre_import::model::{RawCoordinate, RawParcel};
use cadastre_import::sink::Sink;

fn test_db_url() -> Option<String> {
    std::env::var("CADASTRE_TEST_DATABASE_URL").ok().filter(|s| !s.is_empty())
}

fn raw(object_id: i32, upin: Option<&str>, status: i16, wkt: &str) -> RawParcel {
    RawParcel {
        object_id,
        upin: upin.map(str::to_string),
        status_code: status,
        job_id: Some("job-guid".to_string()),
        plan_no: None,
        cad_zone: Some("A".to_string()),
        plot_no: None,
        block_no: None,
        wkt: Some(wkt.to_string()),
        srid: 32632,
    }
}

#[tokio::test]
async fn staging_load_writes_parcels_beacons_and_exceptions() {
    let Some(url) = test_db_url() else {
        eprintln!("skipping: CADASTRE_TEST_DATABASE_URL not set");
        return;
    };

    // A 100 m square in UTM 32N coordinates (area 10,000 m²).
    const SQ: &str = "POLYGON((1000 1000,1100 1000,1100 1100,1000 1100,1000 1000))";
    let raws = vec![
        raw(1, Some("PL000000000001"), 2, SQ), // active, ok
        raw(2, Some("PL000000000002"), 3, SQ), // historic, ok
        raw(3, None, 2, SQ),                   // missing UPIN -> exception
        raw(4, Some("PLBAD"), 9, SQ),          // bad status -> exception
    ];
    let report = stage_parcels(&raws);
    assert_eq!(report.staged_count(), 2);
    assert_eq!(report.exception_count(), 2);

    let beacons = vec![RawCoordinate {
        alias: Some("N1".to_string()),
        x: 488_249.908,
        y: 1_099_097.516,
    }
    .validate()
    .unwrap()];

    let sink = Sink::connect(&url).await.unwrap();
    sink.ensure_schema().await.unwrap();
    let summary = sink.load(&report, &beacons).await.unwrap();
    assert_eq!(summary.parcels, 2);
    assert_eq!(summary.beacons, 1);
    assert_eq!(summary.exceptions, 2);

    let pool = sink.pool();
    let rid = summary.run_id;

    let parcels: i64 = sqlx::query_scalar("SELECT count(*) FROM import.parcel WHERE run_id = $1")
        .bind(rid).fetch_one(pool).await.unwrap();
    assert_eq!(parcels, 2);

    // Geometry round-trips losslessly: area and SRID are preserved through WKT.
    let area: f64 = sqlx::query_scalar(
        "SELECT ST_Area(geom) FROM import.parcel WHERE run_id = $1 ORDER BY id LIMIT 1",
    ).bind(rid).fetch_one(pool).await.unwrap();
    assert!((area - 10_000.0).abs() < 0.01, "area {area}");

    let srid: i32 = sqlx::query_scalar(
        "SELECT ST_SRID(geom) FROM import.parcel WHERE run_id = $1 LIMIT 1",
    ).bind(rid).fetch_one(pool).await.unwrap();
    assert_eq!(srid, 32632);

    let statuses: Vec<String> = sqlx::query_scalar(
        "SELECT status FROM import.parcel WHERE run_id = $1 ORDER BY status",
    ).bind(rid).fetch_all(pool).await.unwrap();
    assert_eq!(statuses, vec!["active".to_string(), "historic".to_string()]);

    let beacons_n: i64 = sqlx::query_scalar("SELECT count(*) FROM import.beacon WHERE run_id = $1")
        .bind(rid).fetch_one(pool).await.unwrap();
    assert_eq!(beacons_n, 1);

    let exceptions_n: i64 =
        sqlx::query_scalar("SELECT count(*) FROM import.exception WHERE run_id = $1")
            .bind(rid).fetch_one(pool).await.unwrap();
    assert_eq!(exceptions_n, 2);

    // The run row records the same counts.
    let recorded: (i32, i32, i32) = sqlx::query_as(
        "SELECT parcels, beacons, exceptions FROM import.run WHERE id = $1",
    ).bind(rid).fetch_one(pool).await.unwrap();
    assert_eq!(recorded, (2, 1, 2));
}
