//! DB-backed integration tests for Phase 3 (jobs, workflow, sessions/locks, auth).
//!
//! These drive the real `routes::router` in-process against a live PostGIS database.
//! They are skipped (not failed) when `CADASTRE_TEST_DATABASE_URL` is unset, so the
//! default `cargo test` stays hermetic; CI/dev sets it to a throwaway database.

use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{header, Request, StatusCode};
use axum::Router;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

use cadastre_server::config::{AuthMode, Config};
use cadastre_server::jwks::Jwks;
use cadastre_server::state::AppState;
use cadastre_server::{db, routes};

/// Returns the test database URL, or `None` to signal the test should be skipped.
fn test_db_url() -> Option<String> {
    std::env::var("CADASTRE_TEST_DATABASE_URL").ok().filter(|s| !s.is_empty())
}

async fn state_with(auth_mode: AuthMode, jwks: Option<Jwks>, db_url: &str) -> AppState {
    let pool = db::connect_and_migrate(db_url)
        .await
        .expect("connect + migrate test db");
    let config = Config {
        database_url: db_url.to_string(),
        bind_addr: "127.0.0.1:0".to_string(),
        auth_mode,
        keycloak_issuer: None,
        keycloak_audience: None,
        keycloak_jwks_json: None,
        keycloak_jwks_path: None,
        catalog_folders: Vec::new(),
        catalog_databases: Vec::new(),
    };
    AppState {
        pool,
        config,
        jwks: jwks.map(Arc::new),
    }
}

/// Send a request to the router and return the status + parsed JSON body (Null if empty).
async fn send(
    router: &Router,
    method: &str,
    uri: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(t) = token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {t}"));
    }
    let req = match body {
        Some(b) => builder
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(b.to_string()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    };
    let resp = router.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, value)
}

#[tokio::test]
async fn jobs_workflow_sessions_end_to_end() {
    let Some(db_url) = test_db_url() else {
        eprintln!("skipping: CADASTRE_TEST_DATABASE_URL not set");
        return;
    };
    let state = state_with(AuthMode::Disabled, None, &db_url).await;
    let app = routes::router(state);

    // Seeded workflow has five stages.
    let (st, wf) = send(&app, "GET", "/api/v1/workflow", None, None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(wf["stages"].as_array().unwrap().len(), 5);

    // Create a job; it starts in `draft`.
    let reference = format!("JOB-{}", Uuid::new_v4());
    let (st, job) = send(
        &app,
        "POST",
        "/api/v1/jobs",
        None,
        Some(json!({"reference": reference, "job_type": "capture", "title": "Plot 5"})),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{job}");
    assert_eq!(job["status"], "draft");
    let job_id = job["id"].as_str().unwrap().to_string();

    // Walk the happy path: draft -> submitted -> under_review -> approved.
    for (action, expected) in [
        ("submit", "submitted"),
        ("start_review", "under_review"),
        ("approve", "approved"),
    ] {
        let (st, j) = send(
            &app,
            "POST",
            &format!("/api/v1/jobs/{job_id}/transition"),
            None,
            Some(json!({ "action": action })),
        )
        .await;
        assert_eq!(st, StatusCode::OK, "{action}: {j}");
        assert_eq!(j["status"], expected);
    }

    // An invalid transition from a terminal stage is a 400, not a 500.
    let (st, _) = send(
        &app,
        "POST",
        &format!("/api/v1/jobs/{job_id}/transition"),
        None,
        Some(json!({ "action": "submit" })),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);

    // History records the three applied transitions.
    let (st, events) = send(&app, "GET", &format!("/api/v1/jobs/{job_id}/events"), None, None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(events.as_array().unwrap().len(), 3);
    assert_eq!(events[0]["action"], "submit");

    // --- Sessions and locks -------------------------------------------------
    let (st, s1) = send(&app, "POST", "/api/v1/sessions", None, Some(json!({"job_id": job_id}))).await;
    assert_eq!(st, StatusCode::CREATED);
    let s1_id = s1["id"].as_str().unwrap().to_string();

    let (st, s2) = send(&app, "POST", "/api/v1/sessions", None, Some(json!({}))).await;
    assert_eq!(st, StatusCode::CREATED);
    let s2_id = s2["id"].as_str().unwrap().to_string();

    let resource = Uuid::new_v4().to_string();

    // s1 locks the parcel.
    let (st, lock) = send(
        &app,
        "POST",
        &format!("/api/v1/sessions/{s1_id}/locks"),
        None,
        Some(json!({"resource_type": "parcel", "resource_id": resource})),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{lock}");
    let lock_id = lock["id"].as_str().unwrap().to_string();

    // s2 cannot lock the same parcel — active holder, so 409.
    let (st, _) = send(
        &app,
        "POST",
        &format!("/api/v1/sessions/{s2_id}/locks"),
        None,
        Some(json!({"resource_type": "parcel", "resource_id": resource})),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT);

    // Invalid resource type is a 400.
    let (st, _) = send(
        &app,
        "POST",
        &format!("/api/v1/sessions/{s1_id}/locks"),
        None,
        Some(json!({"resource_type": "building", "resource_id": resource})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);

    // Release the lock; now s2 can take it.
    let (st, _) = send(
        &app,
        "DELETE",
        &format!("/api/v1/sessions/{s1_id}/locks/{lock_id}"),
        None,
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);

    let (st, _) = send(
        &app,
        "POST",
        &format!("/api/v1/sessions/{s2_id}/locks"),
        None,
        Some(json!({"resource_type": "parcel", "resource_id": resource})),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);

    // Releasing a session drops its locks.
    let (st, released) = send(&app, "POST", &format!("/api/v1/sessions/{s2_id}/release"), None, None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(released["status"], "released");
    let (st, locks) = send(&app, "GET", &format!("/api/v1/sessions/{s2_id}/locks"), None, None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(locks.as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn auth_enabled_enforces_roles_over_http() {
    let Some(db_url) = test_db_url() else {
        eprintln!("skipping: CADASTRE_TEST_DATABASE_URL not set");
        return;
    };
    let idp = TestIdp::new("it-key");
    let jwks = Jwks::from_json(&idp.jwks_json).unwrap();
    let state = state_with(AuthMode::Enabled, Some(jwks), &db_url).await;
    let app = routes::router(state);

    let viewer = idp.mint(r#"{"sub":"v","realm_access":{"roles":["viewer"]}}"#);
    let editor = idp.mint(r#"{"sub":"e","realm_access":{"roles":["editor"]}}"#);
    let new_job = json!({"reference": format!("JOB-{}", Uuid::new_v4()), "job_type": "capture"});

    // No token.
    let (st, _) = send(&app, "POST", "/api/v1/jobs", None, Some(new_job.clone())).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);

    // Wrong role.
    let (st, _) = send(&app, "POST", "/api/v1/jobs", Some(&viewer), Some(new_job.clone())).await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    // Correct role creates the job.
    let (st, _) = send(&app, "POST", "/api/v1/jobs", Some(&editor), Some(new_job)).await;
    assert_eq!(st, StatusCode::CREATED);

    // A garbage token is rejected.
    let (st, _) = send(
        &app,
        "POST",
        "/api/v1/jobs",
        Some("not.a.jwt"),
        Some(json!({"reference": "x", "job_type": "capture"})),
    )
    .await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn stale_session_lock_is_reclaimed() {
    let Some(db_url) = test_db_url() else {
        eprintln!("skipping: CADASTRE_TEST_DATABASE_URL not set");
        return;
    };
    let state = state_with(AuthMode::Disabled, None, &db_url).await;
    let app = routes::router(state);
    // A side connection to backdate the heartbeat (simulating a crashed/idle client).
    let pool = sqlx::PgPool::connect(&db_url).await.unwrap();

    // s1 opens and locks a resource.
    let (_, s1) = send(&app, "POST", "/api/v1/sessions", None, Some(json!({}))).await;
    let s1_id = s1["id"].as_str().unwrap().to_string();
    let resource = Uuid::new_v4().to_string();
    let (st, _) = send(
        &app,
        "POST",
        &format!("/api/v1/sessions/{s1_id}/locks"),
        None,
        Some(json!({"resource_type": "beacon", "resource_id": resource})),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);

    // Backdate s1's heartbeat well past the 5-minute TTL.
    sqlx::query(
        "UPDATE cadastre.edit_session SET last_heartbeat_at = now() - interval '10 minutes' WHERE id = $1::uuid",
    )
    .bind(&s1_id)
    .execute(&pool)
    .await
    .unwrap();

    // s2 acquires the same resource: the stale holder is reclaimed, so this succeeds.
    let (_, s2) = send(&app, "POST", "/api/v1/sessions", None, Some(json!({}))).await;
    let s2_id = s2["id"].as_str().unwrap().to_string();
    let (st, lock) = send(
        &app,
        "POST",
        &format!("/api/v1/sessions/{s2_id}/locks"),
        None,
        Some(json!({"resource_type": "beacon", "resource_id": resource})),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{lock}");

    // s1 was expired by the reclaim: its lock is gone and it can no longer heartbeat.
    let (st, locks) = send(&app, "GET", &format!("/api/v1/sessions/{s1_id}/locks"), None, None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(locks.as_array().unwrap().len(), 0);
    let (st, _) = send(&app, "POST", &format!("/api/v1/sessions/{s1_id}/heartbeat"), None, None).await;
    assert_eq!(st, StatusCode::CONFLICT);
}

#[tokio::test]
async fn survey_check_validates_against_active_tolerances() {
    let Some(db_url) = test_db_url() else {
        eprintln!("skipping: CADASTRE_TEST_DATABASE_URL not set");
        return;
    };
    let state = state_with(AuthMode::Disabled, None, &db_url).await;
    let app = routes::router(state);

    // A closed 100 m square (one hectare) that lands back on its start.
    let square = json!({
        "start": {"x": 1000.0, "y": 1000.0},
        "start_beacon": "B1",
        "stated_area": 10000.0,
        "legs": [
            {"type": "line", "bearing": 0.0,   "distance": 100.0},
            {"type": "line", "bearing": 90.0,  "distance": 100.0},
            {"type": "line", "bearing": 180.0, "distance": 100.0},
            {"type": "line", "bearing": 270.0, "distance": 100.0}
        ]
    });
    let (st, body) = send(&app, "POST", "/api/v1/survey/check", None, Some(square)).await;
    assert_eq!(st, StatusCode::OK, "{body}");
    assert_eq!(body["leg_count"], 4);
    assert_eq!(body["vertex_count"], 5);
    assert!((body["area"].as_f64().unwrap() - 10_000.0).abs() < 0.01);
    assert_eq!(body["checks"]["closes_on_start"], true);
    assert_eq!(body["checks"]["meets_precision"], true);
    assert_eq!(body["checks"]["area_within_tolerance"], true);
    // Exact closure is reported as a null precision ratio.
    assert!(body["closure"]["precision_ratio"].is_null());

    // An open, two-leg figure does not close and overstates its area.
    let open = json!({
        "start": {"x": 0.0, "y": 0.0},
        "stated_area": 10000.0,
        "legs": [
            {"type": "line", "bearing": 0.0,  "distance": 100.0},
            {"type": "line", "bearing": 90.0, "distance": 100.0}
        ]
    });
    let (st, body) = send(&app, "POST", "/api/v1/survey/check", None, Some(open)).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(body["checks"]["closes_on_start"], false);
    assert_eq!(body["checks"]["meets_precision"], false);
    assert_eq!(body["checks"]["area_within_tolerance"], false);
    assert!(body["closure"]["linear_misclosure"].as_f64().unwrap() > 100.0);
}

#[tokio::test]
async fn promote_import_run_into_live_fabric() {
    let Some(db_url) = test_db_url() else {
        eprintln!("skipping: CADASTRE_TEST_DATABASE_URL not set");
        return;
    };
    let app = routes::router(state_with(AuthMode::Disabled, None, &db_url).await);
    let pool = sqlx::PgPool::connect(&db_url).await.unwrap();

    // Seed a staged import run: one active parcel + one historic (not promoted) + one beacon.
    let upin = format!("PL{:012}", rand_12());
    let bnum = format!("IMP-{}", Uuid::new_v4());
    let run_id: i64 = sqlx::query_scalar(
        "INSERT INTO import.run (parcels, beacons, exceptions) VALUES (2, 1, 0) RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    for (num, status) in [(&upin, "active"), (&format!("{upin}H"), "historic")] {
        sqlx::query(
            "INSERT INTO import.parcel (run_id, source_object_id, parcel_number, status, srid, computed_area, geom) \
             VALUES ($1, 1, $2, $3, 32632, 10000, ST_GeomFromText('POLYGON((1000 1000,1100 1000,1100 1100,1000 1100,1000 1000))', 32632))",
        )
        .bind(run_id).bind(num).bind(status).execute(&pool).await.unwrap();
    }
    sqlx::query(
        "INSERT INTO import.beacon (run_id, number, geom) \
         VALUES ($1, $2, ST_SetSRID(ST_MakePoint(500000, 1100000), 32632))",
    )
    .bind(run_id).bind(&bnum).execute(&pool).await.unwrap();

    // An attribute record carrying a Registry link, and a UPIN known only from Aumentum history.
    let object_id = (rand_12() % 2_000_000_000) as i32;
    let history_upin = format!("PL{:012}", rand_12());
    sqlx::query(
        "INSERT INTO import.parcel_attribute (run_id, object_id, upin, plot, block, plan_number, status, \
           parcel_area, area_unit, creation_date, lrs_id, linkage_status, linkage_date) \
         VALUES ($1, $2, $3, '15', 'B3', 'PSG 12', 'Active', '3095.34', '2', '2017-03-01T10:00:00', 'LRS-000981', '1', '20170302')",
    )
    .bind(run_id).bind(object_id).bind(&upin).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO import.issued_upin (run_id, upin, first_source, occurrences) VALUES ($1, $2, 'parcels-history', 1)")
        .bind(run_id).bind(&history_upin).execute(&pool).await.unwrap();

    // Promote: only the active parcel + the beacon go live.
    let (st, res) = send(&app, "POST", &format!("/api/v1/import/promote/{run_id}"), None, None).await;
    assert_eq!(st, StatusCode::OK, "{res}");
    assert_eq!(res["parcels_promoted"], 1);
    assert_eq!(res["beacons_promoted"], 1);
    assert_eq!(res["attributes_promoted"], 1);

    // The Registry linkage survives promotion, typed; imported beacons are control points; a UPIN
    // seen only in Aumentum history is reserved.
    let (lrs, linked, area): (Option<String>, Option<i16>, Option<f64>) = sqlx::query_as(
        "SELECT lrs_id, linkage_status, parcel_area FROM cadastre.parcel_attribute WHERE object_id = $1",
    )
    .bind(object_id).fetch_one(&pool).await.unwrap();
    assert_eq!((lrs.as_deref(), linked), (Some("LRS-000981"), Some(1)));
    assert!((area.unwrap() - 3095.34).abs() < 1e-6);
    let control: bool = sqlx::query_scalar("SELECT is_control FROM cadastre.beacon WHERE number = $1")
        .bind(&bnum).fetch_one(&pool).await.unwrap();
    assert!(control, "imported Aumentum coordinates are control points");
    let reserved: i64 = sqlx::query_scalar("SELECT count(*) FROM cadastre.issued_upin WHERE upin = $1")
        .bind(&history_upin).fetch_one(&pool).await.unwrap();
    assert_eq!(reserved, 1);

    // The active parcel is now in the live fabric, posted; the historic one is not.
    let (_, found) = send(&app, "GET", &format!("/api/v1/parcels/search?number={upin}"), None, None).await;
    let arr = found.as_array().unwrap();
    assert!(arr.iter().any(|p| p["parcel_number"] == upin && p["status"] == "posted"));
    assert!(!arr.iter().any(|p| p["parcel_number"] == format!("{upin}H")));

    // Promotion is idempotent.
    let (st, res2) = send(&app, "POST", &format!("/api/v1/import/promote/{run_id}"), None, None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(res2["parcels_promoted"], 1);

    // Audit chain still verifies after the promote entries.
    let (_, v) = send(&app, "GET", "/api/v1/audit/verify", None, None).await;
    assert_eq!(v["ok"], true, "{v}");

    // Unknown run → 404.
    let (st, _) = send(&app, "POST", "/api/v1/import/promote/999999999", None, None).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
}

/// A UPIN is the Registry's link to a parcel, so one issued anywhere (an imported historic parcel,
/// an import exception, or a ledger entry) must never be allocated again by posting
/// (regression: the allocator started at 1 and ignored imported Aumentum UPINs).
#[tokio::test]
async fn posting_never_reuses_an_issued_upin() {
    let Some(db_url) = test_db_url() else {
        eprintln!("skipping: CADASTRE_TEST_DATABASE_URL not set");
        return;
    };
    let app = routes::router(state_with(AuthMode::Disabled, None, &db_url).await);
    let pool = sqlx::PgPool::connect(&db_url).await.unwrap();

    // Work above everything already issued in this database (other tests run concurrently).
    let base: i64 = sqlx::query_scalar(
        "SELECT GREATEST(COALESCE(max(substring(upin FROM 3)::bigint), 0), \
                (SELECT last_value FROM cadastre.parcel_number_seq)) \
         FROM cadastre.issued_upin WHERE upin ~ '^PL[0-9]{12}$'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let active = format!("PL{:012}", base + 100);
    let historic = format!("PL{:012}", base + 1_000); // not promoted, but issued
    let excepted = format!("PL{:012}", base + 2_000); // a duplicate-UPIN exception, still issued

    let run_id: i64 = sqlx::query_scalar(
        "INSERT INTO import.run (parcels, beacons, exceptions) VALUES (2, 0, 1) RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    for (num, status) in [(&active, "active"), (&historic, "historic")] {
        sqlx::query(
            "INSERT INTO import.parcel (run_id, source_object_id, parcel_number, status, srid, computed_area, geom) \
             VALUES ($1, 1, $2, $3, 32632, 10000, ST_GeomFromText('POLYGON((3000 3000,3100 3000,3100 3100,3000 3100,3000 3000))', 32632))",
        )
        .bind(run_id).bind(num).bind(status).execute(&pool).await.unwrap();
    }
    sqlx::query("INSERT INTO import.exception (run_id, source_object_id, parcel_number, kind) VALUES ($1, 7, $2, 'DuplicateUpin')")
        .bind(run_id).bind(&excepted).execute(&pool).await.unwrap();

    let (st, res) = send(&app, "POST", &format!("/api/v1/import/promote/{run_id}"), None, None).await;
    assert_eq!(st, StatusCode::OK, "{res}");

    // Every UPIN in the run is in the ledger, whatever its status.
    for upin in [&active, &historic, &excepted] {
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM cadastre.issued_upin WHERE upin = $1")
            .bind(upin).fetch_one(&pool).await.unwrap();
        assert_eq!(n, 1, "{upin} missing from the ledger");
    }

    // A job posted now gets a UPIN above every issued one, never a reused one.
    let reference = format!("JOB-{}", Uuid::new_v4());
    let (_, job) = send(&app, "POST", "/api/v1/jobs", None,
        Some(json!({"reference": reference, "job_type": "capture"}))).await;
    let job_id = job["id"].as_str().unwrap().to_string();
    let (st, p) = send(&app, "POST", &format!("/api/v1/jobs/{job_id}/parcels"), None,
        Some(json!({"geometry": square(3500.0, 3500.0, 40.0)}))).await;
    assert_eq!(st, StatusCode::CREATED, "{p}");
    for action in ["submit", "start_review", "approve"] {
        let (st, j) = send(&app, "POST", &format!("/api/v1/jobs/{job_id}/transition"), None,
            Some(json!({ "action": action }))).await;
        assert_eq!(st, StatusCode::OK, "{action}: {j}");
    }
    let (st, result) = send(&app, "POST", &format!("/api/v1/jobs/{job_id}/post"), None, None).await;
    assert_eq!(st, StatusCode::OK, "{result}");
    let posted = result["parcel_numbers"][0].as_str().unwrap().to_string();
    let posted_n: i64 = posted[2..].parse().unwrap();
    assert!(posted_n > base + 2_000, "posted {posted} is not above the issued {excepted}");
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM cadastre.issued_upin WHERE upin = $1 AND source = 'posted'")
        .bind(&posted).fetch_one(&pool).await.unwrap();
    assert_eq!(n, 1, "the posted UPIN is recorded in the ledger");

    // The ledger is append-only: an issued UPIN cannot be deleted or rewritten.
    assert!(sqlx::query("DELETE FROM cadastre.issued_upin WHERE upin = $1").bind(&historic).execute(&pool).await.is_err());
    assert!(sqlx::query("UPDATE cadastre.issued_upin SET upin = 'X' WHERE upin = $1").bind(&historic).execute(&pool).await.is_err());
}

/// A pseudo-random number for a unique UPIN per test run (formatted to 12 digits). Kept below
/// 10^11 so that tests allocating above every issued UPIN still fit the 12-digit format.
fn rand_12() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    (SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos() as u64) % 100_000_000_000
}

/// Send a request and return (status, content-type, raw body bytes) — for non-JSON responses.
async fn send_bytes(router: &Router, method: &str, uri: &str) -> (StatusCode, String, Vec<u8>) {
    let req = Request::builder().method(method).uri(uri).body(Body::empty()).unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let ct = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap().to_vec();
    (status, ct, bytes)
}

fn square(x: f64, y: f64, s: f64) -> Value {
    json!({"type":"Polygon","coordinates":[[[x,y],[x+s,y],[x+s,y+s],[x,y+s],[x,y]]]})
}

#[tokio::test]
async fn fabric_validate_flags_overlaps_and_passes_clean() {
    let Some(db_url) = test_db_url() else {
        eprintln!("skipping: CADASTRE_TEST_DATABASE_URL not set");
        return;
    };
    let app = routes::router(state_with(AuthMode::Disabled, None, &db_url).await);

    // Two separate 100 m squares: clean.
    let clean = json!({"parcels": [
        [[0.0,0.0],[100.0,0.0],[100.0,100.0],[0.0,100.0]],
        [[500.0,0.0],[600.0,0.0],[600.0,100.0],[500.0,100.0]]
    ], "beacons": []});
    let (st, body) = send(&app, "POST", "/api/v1/fabric/validate", None, Some(clean)).await;
    assert_eq!(st, StatusCode::OK, "{body}");
    assert_eq!(body["is_clean"], true);

    // Two overlapping squares + duplicate beacons.
    let dirty = json!({"parcels": [
        [[0.0,0.0],[100.0,0.0],[100.0,100.0],[0.0,100.0]],
        [[50.0,0.0],[150.0,0.0],[150.0,100.0],[50.0,100.0]]
    ], "beacons": [{"x":10.0,"y":10.0},{"x":10.001,"y":10.0}]});
    let (st, body) = send(&app, "POST", "/api/v1/fabric/validate", None, Some(dirty)).await;
    assert_eq!(st, StatusCode::OK, "{body}");
    assert_eq!(body["is_clean"], false);
    assert_eq!(body["overlaps"].as_array().unwrap().len(), 1);
    assert_eq!(body["duplicate_beacons"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn fabric_adjust_converges_on_a_fixed_network() {
    let Some(db_url) = test_db_url() else {
        eprintln!("skipping: CADASTRE_TEST_DATABASE_URL not set");
        return;
    };
    let app = routes::router(state_with(AuthMode::Disabled, None, &db_url).await);

    // A free point C observed from two fixed anchors A, B; initial C is 2 m off.
    let d = 2.0_f64.sqrt() * 50.0; // 70.7107
    let req = json!({
        "coords": [{"x":0.0,"y":0.0},{"x":100.0,"y":0.0},{"x":50.0,"y":48.0}],
        "fixed": [true, true, false],
        "observations": [
            {"from":0,"to":2,"distance":d,"bearing":45.0,"sigma_distance_m":0.01,"sigma_bearing_sec":5.0},
            {"from":1,"to":2,"distance":d,"bearing":315.0,"sigma_distance_m":0.01,"sigma_bearing_sec":5.0}
        ]
    });
    let (st, body) = send(&app, "POST", "/api/v1/fabric/adjust", None, Some(req)).await;
    assert_eq!(st, StatusCode::OK, "{body}");
    assert!(body["iterations"].as_u64().unwrap() >= 1);
    let c = &body["coords"][2];
    assert!((c["x"].as_f64().unwrap() - 50.0).abs() < 0.1, "{c}");
    assert!((c["y"].as_f64().unwrap() - 50.0).abs() < 0.1, "{c}");
    assert_eq!(body["blunders"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn posting_numbers_parcels_and_audit_chain_verifies() {
    let Some(db_url) = test_db_url() else {
        eprintln!("skipping: CADASTRE_TEST_DATABASE_URL not set");
        return;
    };
    let app = routes::router(state_with(AuthMode::Disabled, None, &db_url).await);

    // Create a job and capture two draft parcels under it.
    let reference = format!("JOB-{}", Uuid::new_v4());
    let (_, job) = send(&app, "POST", "/api/v1/jobs", None,
        Some(json!({"reference": reference, "job_type": "capture"}))).await;
    let job_id = job["id"].as_str().unwrap().to_string();

    for i in 0..2 {
        let (st, p) = send(&app, "POST", &format!("/api/v1/jobs/{job_id}/parcels"), None,
            Some(json!({"geometry": square(1000.0 + i as f64 * 200.0, 1000.0, 50.0)}))).await;
        assert_eq!(st, StatusCode::CREATED, "{p}");
        assert!(p["parcel_number"].is_null()); // unposted drafts have no number
        assert_eq!(p["status"], "draft");
    }

    // Posting requires approval: walk the job through the workflow first.
    for action in ["submit", "start_review", "approve"] {
        let (st, j) = send(&app, "POST", &format!("/api/v1/jobs/{job_id}/transition"), None,
            Some(json!({ "action": action }))).await;
        assert_eq!(st, StatusCode::OK, "{action}: {j}");
    }

    // Post the job: both parcels get numbers and go active.
    let (st, result) = send(&app, "POST", &format!("/api/v1/jobs/{job_id}/post"), None, None).await;
    assert_eq!(st, StatusCode::OK, "{result}");
    assert_eq!(result["posted"], 2);
    let numbers = result["parcel_numbers"].as_array().unwrap();
    assert_eq!(numbers.len(), 2);
    assert!(numbers[0].as_str().unwrap().starts_with("PL"));

    // The audit chain (incl. this post) verifies intact.
    let (st, v) = send(&app, "GET", "/api/v1/audit/verify", None, None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["ok"], true, "{v}");
    assert!(v["checked"].as_u64().unwrap() >= 1);

    // Search finds a posted parcel by number prefix.
    let prefix = &numbers[0].as_str().unwrap()[..6];
    let (st, found) = send(&app, "GET", &format!("/api/v1/parcels/search?number={prefix}"), None, None).await;
    assert_eq!(st, StatusCode::OK);
    assert!(found.as_array().unwrap().iter().any(|p| p["status"] == "posted"));

    // PDF outputs are valid PDFs.
    let (st, ct, bytes) = send_bytes(&app, "GET", &format!("/api/v1/jobs/{job_id}/report.pdf")).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(ct, "application/pdf");
    assert!(bytes.starts_with(b"%PDF-1.4") && bytes.ends_with(b"%%EOF"));

    let (st, _, bytes) = send_bytes(&app, "GET", &format!("/api/v1/jobs/{job_id}/siteplan.pdf")).await;
    assert_eq!(st, StatusCode::OK);
    assert!(bytes.starts_with(b"%PDF-1.4"));
}

/// A throwaway RSA identity provider used to mint signed tokens for the auth test.
struct TestIdp {
    kid: String,
    signing_key: rsa::pkcs1v15::SigningKey<sha2::Sha256>,
    jwks_json: String,
}

impl TestIdp {
    fn new(kid: &str) -> Self {
        use rsa::traits::PublicKeyParts;
        let mut rng = rand::thread_rng();
        let priv_key = rsa::RsaPrivateKey::new(&mut rng, 2048).unwrap();
        let pub_key = priv_key.to_public_key();
        let n = URL_SAFE_NO_PAD.encode(pub_key.n().to_bytes_be());
        let e = URL_SAFE_NO_PAD.encode(pub_key.e().to_bytes_be());
        let jwks_json = format!(
            r#"{{"keys":[{{"kty":"RSA","kid":"{kid}","alg":"RS256","n":"{n}","e":"{e}"}}]}}"#
        );
        Self {
            kid: kid.to_string(),
            signing_key: rsa::pkcs1v15::SigningKey::<sha2::Sha256>::new(priv_key),
            jwks_json,
        }
    }

    fn mint(&self, claims_json: &str) -> String {
        use rsa::signature::{SignatureEncoding, Signer};
        let header = format!(r#"{{"alg":"RS256","typ":"JWT","kid":"{}"}}"#, self.kid);
        let h = URL_SAFE_NO_PAD.encode(header.as_bytes());
        let p = URL_SAFE_NO_PAD.encode(claims_json.as_bytes());
        let sig = self.signing_key.sign(format!("{h}.{p}").as_bytes());
        let s = URL_SAFE_NO_PAD.encode(sig.to_bytes());
        format!("{h}.{p}.{s}")
    }
}

/// Posting allocates official parcel numbers, so it must be refused until the job is approved
/// (regression: a draft job used to post and consume a UPIN).
#[tokio::test]
async fn posting_requires_an_approved_job() {
    let Some(db_url) = test_db_url() else {
        eprintln!("skipping: CADASTRE_TEST_DATABASE_URL not set");
        return;
    };
    let app = routes::router(state_with(AuthMode::Disabled, None, &db_url).await);

    let (st, job) = send(
        &app,
        "POST",
        "/api/v1/jobs",
        None,
        Some(json!({"reference": format!("JOB-{}", Uuid::new_v4()), "job_type": "capture"})),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{job}");
    let job_id = job["id"].as_str().unwrap().to_string();

    let square = json!({"type": "Polygon", "coordinates": [[[0.0, 0.0], [50.0, 0.0], [50.0, 50.0], [0.0, 50.0], [0.0, 0.0]]]});
    let (st, p) = send(&app, "POST", &format!("/api/v1/jobs/{job_id}/parcels"), None,
        Some(json!({"geometry": square, "stated_area": 2500.0}))).await;
    assert_eq!(st, StatusCode::CREATED, "{p}");

    // Draft, submitted and under review: all refused with 409, and nothing is numbered.
    for next in [None, Some("submit"), Some("start_review")] {
        if let Some(action) = next {
            let (st, j) = send(&app, "POST", &format!("/api/v1/jobs/{job_id}/transition"), None,
                Some(json!({ "action": action }))).await;
            assert_eq!(st, StatusCode::OK, "{action}: {j}");
        }
        let (st, body) = send(&app, "POST", &format!("/api/v1/jobs/{job_id}/post"), None, None).await;
        assert_eq!(st, StatusCode::CONFLICT, "post before approval must be refused: {body}");
    }

    // Approve, then posting numbers the parcel exactly once.
    let (st, _) = send(&app, "POST", &format!("/api/v1/jobs/{job_id}/transition"), None,
        Some(json!({ "action": "approve" }))).await;
    assert_eq!(st, StatusCode::OK);
    let (st, posted) = send(&app, "POST", &format!("/api/v1/jobs/{job_id}/post"), None, None).await;
    assert_eq!(st, StatusCode::OK, "{posted}");
    assert_eq!(posted["posted"], 1);
    let upin = posted["parcel_numbers"][0].as_str().unwrap();
    assert!(upin.starts_with("PL") && upin.len() == 14, "UPIN format: {upin}");
}

/// An empty result must still be valid GeoJSON (`"features": []`, not `null`) — the editor
/// relies on it whenever a job has no draft parcels (regression).
#[tokio::test]
async fn empty_geojson_has_an_empty_features_array() {
    let Some(db_url) = test_db_url() else {
        eprintln!("skipping: CADASTRE_TEST_DATABASE_URL not set");
        return;
    };
    let app = routes::router(state_with(AuthMode::Disabled, None, &db_url).await);
    let (st, fc) = send(&app, "GET", &format!("/api/v1/parcels.geojson?status=draft&job={}", Uuid::new_v4()), None, None).await;
    assert_eq!(st, StatusCode::OK, "{fc}");
    assert_eq!(fc["type"], "FeatureCollection");
    assert_eq!(fc["features"], json!([]), "{fc}");
}

/// Surveyors type plot numbers in short form; search must find the zero-padded UPIN, and rank
/// the exact parcel first (regression: only an exact-prefix match worked).
#[tokio::test]
async fn search_finds_a_parcel_by_its_short_plot_number() {
    let Some(db_url) = test_db_url() else {
        eprintln!("skipping: CADASTRE_TEST_DATABASE_URL not set");
        return;
    };
    let app = routes::router(state_with(AuthMode::Disabled, None, &db_url).await);
    let (_, job) = send(&app, "POST", "/api/v1/jobs", None,
        Some(json!({"reference": format!("JOB-{}", Uuid::new_v4()), "job_type": "capture"}))).await;
    let job_id = job["id"].as_str().unwrap().to_string();
    let (st, _) = send(&app, "POST", &format!("/api/v1/jobs/{job_id}/parcels"), None,
        Some(json!({"geometry": square(5000.0, 5000.0, 20.0)}))).await;
    assert_eq!(st, StatusCode::CREATED);
    for action in ["submit", "start_review", "approve"] {
        send(&app, "POST", &format!("/api/v1/jobs/{job_id}/transition"), None, Some(json!({ "action": action }))).await;
    }
    let (_, posted) = send(&app, "POST", &format!("/api/v1/jobs/{job_id}/post"), None, None).await;
    let upin = posted["parcel_numbers"][0].as_str().unwrap().to_string();
    let short = upin.trim_start_matches("PL").trim_start_matches('0').to_string(); // e.g. "57"

    for typed in [short.clone(), format!("PL{short}"), format!(" pl {short} "), upin.to_lowercase()] {
        let (st, found) = send(&app, "GET", &format!("/api/v1/parcels/search?number={}", typed.replace(' ', "%20")), None, None).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(found[0]["parcel_number"], upin, "typed {typed:?}: exact UPIN must rank first: {found}");
    }

    // The Registry property number on the map label finds the parcel too, spaces and case aside.
    let pool = sqlx::PgPool::connect(&db_url).await.unwrap();
    let prop = format!("PL{}", rand_12() % 1_000_000 + 9_000_000);
    sqlx::query("INSERT INTO cadastre.parcel_attribute (object_id, upin, property_number, status) VALUES ($1, $2, $3, '0')")
        .bind(-((rand_12() % 1_000_000_000) as i32) - 1).bind(&upin).bind(&prop).execute(&pool).await.unwrap();
    for typed in [prop.clone(), prop.to_lowercase(), format!("PL {}", &prop[2..])] {
        let (st, found) = send(&app, "GET", &format!("/api/v1/parcels/search?number={}", typed.replace(' ', "%20")), None, None).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(found[0]["parcel_number"], upin, "typed property number {typed:?}: {found}");
    }
}

// ---- Survey computation tools and survey/CAD/attribute import ----
//
// The editor, the headless reports and these endpoints must all run the same engine. These
// tests drive the HTTP surface and check it returns exactly what `cadastre-core` computes.

#[tokio::test]
async fn survey_tool_catalogue_and_execution() {
    let Some(db_url) = test_db_url() else {
        eprintln!("skipping: CADASTRE_TEST_DATABASE_URL not set");
        return;
    };
    let app = routes::router(state_with(AuthMode::Disabled, None, &db_url).await);

    let (st, catalogue) = send(&app, "GET", "/api/v1/survey/tools", None, None).await;
    assert_eq!(st, StatusCode::OK, "{catalogue}");
    assert_eq!(catalogue["count"], 35);
    let tools = catalogue["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 35);
    assert!(tools.iter().any(|t| t["id"] == "inverse"));

    let (st, spec) = send(&app, "GET", "/api/v1/survey/tools/inverse", None, None).await;
    assert_eq!(st, StatusCode::OK, "{spec}");
    assert_eq!(spec["category"], "basic");

    let (st, run) = send(
        &app,
        "POST",
        "/api/v1/survey/tools/inverse",
        None,
        Some(json!({"from": {"x": 0.0, "y": 0.0}, "to": {"x": 100.0, "y": 100.0}})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{run}");
    assert_eq!(run["tool"], "inverse");
    let distance = run["result"]["distance"].as_f64().unwrap();
    assert!((distance - 20_000.0f64.sqrt()).abs() < 1e-9, "{distance}");
    let bearing = run["result"]["bearing_deg"].as_f64().unwrap();
    assert!((bearing - 45.0).abs() < 1e-9, "{bearing}");

    let (st, _) = send(&app, "GET", "/api/v1/survey/tools/no-such-tool", None, None).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    let (st, _) = send(
        &app,
        "POST",
        "/api/v1/survey/tools/inverse",
        None,
        Some(json!({"from": {"x": 0.0, "y": 0.0}})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "incomplete input must be rejected");
}

#[tokio::test]
async fn import_preview_agrees_across_coordinate_cad_and_landxml() {
    let Some(db_url) = test_db_url() else {
        eprintln!("skipping: CADASTRE_TEST_DATABASE_URL not set");
        return;
    };
    let app = routes::router(state_with(AuthMode::Disabled, None, &db_url).await);

    let csv = "Point,Easting,Northing,Code\n\
               1,352440.677,1233094.048,BC\n\
               2,352440.677,1233144.048,BC\n\
               3,352540.677,1233144.048,BC\n\
               4,352540.677,1233094.048,BC\n";
    let (st, preview) = send(
        &app,
        "POST",
        "/api/v1/import/preview",
        None,
        Some(json!({"filename": "parcel.csv", "contents": csv, "crs": "EPSG:32632"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{preview}");
    assert_eq!(preview["kind"], "survey");
    assert_eq!(preview["format"], "coordinates");
    assert_eq!(preview["points"].as_array().unwrap().len(), 4);
    assert_eq!(preview["points"][0]["point"]["x"], 352440.677);

    let landxml = r#"<?xml version="1.0"?>
<LandXML xmlns="http://www.landxml.org/schema/LandXML-1.2" version="1.2">
  <CoordinateSystem name="WGS 84 / UTM zone 32N" epsgCode="32632"/>
  <CgPoints>
    <CgPoint name="1" code="BC">1233094.048 352440.677</CgPoint>
    <CgPoint name="2" code="BC">1233144.048 352440.677</CgPoint>
  </CgPoints>
  <Parcels>
    <Parcel name="PL-1" area="5000">
      <CoordGeom>
        <Line><Start>1233094.048 352440.677</Start><End>1233144.048 352440.677</End></Line>
        <Line><Start>1233144.048 352440.677</Start><End>1233144.048 352540.677</End></Line>
        <Line><Start>1233144.048 352540.677</Start><End>1233094.048 352540.677</End></Line>
        <Line><Start>1233094.048 352540.677</Start><End>1233094.048 352440.677</End></Line>
      </CoordGeom>
    </Parcel>
  </Parcels>
</LandXML>"#;
    let (st, preview) = send(
        &app,
        "POST",
        "/api/v1/import/preview",
        None,
        Some(json!({"filename": "parcel.xml", "contents": landxml})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{preview}");
    assert_eq!(preview["epsg"], 32632);
    assert_eq!(preview["boundaries"][0]["closed"], true);
    let ring = preview["boundaries"][0]["ring"].as_array().unwrap();
    assert_eq!(ring.len(), 4);
    // Northing/easting in the file, easting/northing on the wire.
    assert_eq!(ring[0]["x"], 352440.677);
    assert_eq!(ring[0]["y"], 1233094.048);

    let (st, raster) = send(
        &app,
        "POST",
        "/api/v1/import/preview",
        None,
        Some(json!({
            "filename": "ortho.tfw",
            "contents": "0.5\n0\n0\n-0.5\n352440.25\n1233094.75\n",
            "prj": "PROJCS[\"WGS 84 / UTM zone 32N\",AUTHORITY[\"EPSG\",\"32632\"]]"
        })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{raster}");
    assert_eq!(raster["kind"], "raster");
    assert_eq!(raster["epsg"], 32632);
    assert_eq!(raster["world"]["e"], -0.5);

    let (st, _) = send(
        &app,
        "POST",
        "/api/v1/import/preview",
        None,
        Some(json!({"filename": "mystery.xyz", "contents": "not a survey"})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn attribute_join_and_job_import_create_drafts() {
    let Some(db_url) = test_db_url() else {
        eprintln!("skipping: CADASTRE_TEST_DATABASE_URL not set");
        return;
    };
    let app = routes::router(state_with(AuthMode::Disabled, None, &db_url).await);

    // Unique beacon numbers and coordinates so repeated runs never collide with earlier data.
    let tag = Uuid::new_v4().simple().to_string();
    let ids: Vec<String> = (0..4).map(|i| format!("T{tag}{i}")).collect();
    let mut easting = 600_000.0 + (Uuid::new_v4().as_u128() % 1000) as f64;
    let northing = 4_000_000.0;
    easting = easting.round();
    let corners = [
        (easting, northing),
        (easting, northing + 50.0),
        (easting + 100.0, northing + 50.0),
        (easting + 100.0, northing),
    ];
    let mut csv = String::from("Point,Easting,Northing,Code\n");
    for (id, (x, y)) in ids.iter().zip(corners.iter()) {
        csv.push_str(&format!("{id},{x},{y},BC\n"));
    }
    let mut attrs = String::from("Point,Owner\n");
    for id in &ids {
        attrs.push_str(&format!("{id},Owner {id}\n"));
    }

    let (st, joined) = send(
        &app,
        "POST",
        "/api/v1/import/attributes",
        None,
        Some(json!({
            "coordinates": csv,
            "attributes": attrs,
            "key_column": "Point",
            "crs": "EPSG:32632"
        })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{joined}");
    assert_eq!(joined["points"].as_array().unwrap().len(), 4);
    assert_eq!(joined["points"][0]["attributes"]["Owner"], format!("Owner {}", ids[0]));
    assert_eq!(joined["points"][0]["attributes"]["Code"], "BC");
    assert!(joined["warnings"].as_array().unwrap().is_empty(), "{joined}");

    let reference = format!("IMPORT-{}", Uuid::new_v4());
    let (st, job) = send(
        &app,
        "POST",
        "/api/v1/jobs",
        None,
        Some(json!({"reference": reference, "job_type": "capture"})),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{job}");
    let job_id = job["id"].as_str().unwrap().to_string();

    // A DXF boundary becomes one draft parcel.
    let dxf = format!(
        "0\nSECTION\n2\nENTITIES\n0\nLWPOLYLINE\n8\nPARCEL\n90\n4\n70\n1\n\
         10\n{easting}\n20\n{northing}\n\
         10\n{easting}\n20\n{}\n\
         10\n{}\n20\n{}\n\
         10\n{}\n20\n{northing}\n\
         0\nENDSEC\n0\nEOF\n",
        northing + 50.0,
        easting + 100.0,
        northing + 50.0,
        easting + 100.0
    );
    let (st, imported) = send(
        &app,
        "POST",
        &format!("/api/v1/jobs/{job_id}/import"),
        None,
        Some(json!({"filename": "parcel.dxf", "contents": dxf})),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{imported}");
    assert_eq!(imported["created_parcels"], 1);

    // The coordinate list (with attribute join) becomes four beacons.
    let (st, imported) = send(
        &app,
        "POST",
        &format!("/api/v1/jobs/{job_id}/import"),
        None,
        Some(json!({
            "filename": "beacons.csv",
            "contents": csv,
            "attributes": attrs,
            "key_column": "Point",
            "crs": "EPSG:32632"
        })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{imported}");
    assert_eq!(imported["created_beacons"], 4);
    assert_eq!(imported["created_parcels"], 0);
    assert!(imported["skipped_beacons"].as_array().unwrap().is_empty(), "{imported}");

    // The draft parcel is visible in the job's draft fabric with its computed area.
    let (st, drafts) = send(
        &app,
        "GET",
        &format!("/api/v1/parcels.geojson?status=draft&limit=50&job={job_id}"),
        None,
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{drafts}");
    let features = drafts["features"].as_array().unwrap();
    assert_eq!(features.len(), 1);
    let computed = features[0]["properties"]["computed_area"]
        .as_f64()
        .or_else(|| features[0]["properties"]["area"].as_f64());
    assert!(
        computed.map(|a| (a - 5000.0).abs() < 0.01).unwrap_or(false),
        "draft parcel area: {computed:?} in {drafts}"
    );

    // A raster world file cannot become job geometry.
    let (st, _) = send(
        &app,
        "POST",
        &format!("/api/v1/jobs/{job_id}/import"),
        None,
        Some(json!({"filename": "ortho.tfw", "contents": "0.5\n0\n0\n-0.5\n0\n0\n"})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
}

/// The Aumentum workflow engine over HTTP: a job type shaped like Aumentum's "Deed Plan
/// Creationv2" (branch at the start, a review that can send work back, an end task).
#[tokio::test]
async fn aumentum_workflow_follows_the_task_graph() {
    let Some(db_url) = test_db_url() else {
        eprintln!("skipping: CADASTRE_TEST_DATABASE_URL not set");
        return;
    };
    let app = routes::router(state_with(AuthMode::Disabled, None, &db_url).await);
    let pool = sqlx::PgPool::connect(&db_url).await.unwrap();

    // Unique ids per run: tasks b..b+4 (search, plot, review, commit, plan), job type b.
    let b = 100_000 + (rand_12() % 800_000) as i32 * 10;
    let (search, plot, review, commit, plan) = (b, b + 1, b + 2, b + 3, b + 4);
    for (id, name) in [(search, "Search"), (plot, "Plot Parcel"), (review, "Review Plotted Parcel"), (commit, "Commit Parcel"), (plan, "Generate Title Deed Plan")] {
        sqlx::query("INSERT INTO cadastre.wf_task (id, name) VALUES ($1, $2)").bind(id).bind(name).execute(&pool).await.unwrap();
    }
    sqlx::query("INSERT INTO cadastre.wf_step (id, name, commands) VALUES ($1, 'Generate Title Deed Plan', ARRAY['{0FF462C4-AB8B-4735-B153-11284B5B2E6E}'])")
        .bind(b).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO cadastre.wf_task_step (task_id, step_id, ord) VALUES ($1, $2, 1)").bind(plan).bind(b).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO cadastre.wf_job_type (id, name, start_task_id, end_task_id) VALUES ($1, 'Deed Plan (test)', $2, $3)")
        .bind(b).bind(search).bind(plan).execute(&pool).await.unwrap();
    for (from, to) in [(search, plot), (search, plan), (plot, review), (review, commit), (review, plot), (commit, plan)] {
        sqlx::query("INSERT INTO cadastre.wf_task_flow (job_type_id, task_id, next_task_id) VALUES ($1, $2, $3)")
            .bind(b).bind(from).bind(to).execute(&pool).await.unwrap();
    }
    let max_before: i32 = sqlx::query_scalar("SELECT COALESCE(max(id), 0) FROM cadastre.wf_job_instance").fetch_one(&pool).await.unwrap();

    let (st, job) = send(&app, "POST", "/api/v1/wf/jobs", None,
        Some(json!({"job_type_id": b, "name": "Job-PL00001", "lrs_transaction_number": "PLS000000001"}))).await;
    assert_eq!(st, StatusCode::CREATED, "{job}");
    let id = job["job"]["id"].as_i64().unwrap();
    assert!(id as i32 > max_before, "new job ids continue above existing (imported) ones");
    assert_eq!(job["job"]["current_task_id"], search);
    assert_eq!(job["job"]["status_label"], "In progress");

    let adv_url = format!("/api/v1/wf/jobs/{id}/advance");
    let adv = |body: Value| send(&app, "POST", &adv_url, None, Some(body));
    // A branch needs a choice.
    let (st, _) = adv(json!({})).await;
    assert_eq!(st, StatusCode::CONFLICT);
    let (st, d) = adv(json!({"to_task_id": plot})).await;
    assert_eq!(st, StatusCode::OK, "{d}");
    let (_, d) = adv(json!({})).await; // the only path
    assert_eq!(d["job"]["current_task_id"], review);
    let (_, d) = adv(json!({"to_task_id": plot, "comment": "replot"})).await; // rework
    assert_eq!(d["job"]["current_task_id"], plot);
    let (_, _) = adv(json!({})).await;
    let (st, _) = adv(json!({"to_task_id": plan})).await; // not an edge from review
    assert_eq!(st, StatusCode::CONFLICT);
    let (_, _) = adv(json!({"to_task_id": commit})).await;
    let (_, d) = adv(json!({})).await;
    assert_eq!(d["job"]["current_task_id"], plan);
    assert_eq!(d["can_complete"], true);
    assert_eq!(d["steps"][0]["commands"][0], "{0FF462C4-AB8B-4735-B153-11284B5B2E6E}");
    let (st, d) = adv(json!({"comment": "plan issued"})).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(d["job"]["status_label"], "Completed");
    assert!(d["job"]["ended"].is_string());
    let (st, _) = adv(json!({})).await;
    assert_eq!(st, StatusCode::CONFLICT, "a completed job does not move");

    let tasks: Vec<i64> = d["history"].as_array().unwrap().iter().map(|h| h["task_id"].as_i64().unwrap()).collect();
    assert_eq!(tasks, [search, plot, review, plot, review, commit, plan].map(i64::from).to_vec());
    assert!(d["history"].as_array().unwrap().iter().all(|h| h["ended"].is_string()));
    // End Job: from any task, as cancelled (status 9); a closed job cannot be ended again.
    let (_, other) = send(&app, "POST", "/api/v1/wf/jobs", None, Some(json!({"job_type_id": b, "name": "Job-PL00002"}))).await;
    let oid = other["job"]["id"].as_i64().unwrap();
    let (st, e) = send(&app, "POST", &format!("/api/v1/wf/jobs/{oid}/end"), None, Some(json!({"outcome": "cancelled", "comment": "duplicate application"}))).await;
    assert_eq!(st, StatusCode::OK, "{e}");
    assert_eq!(e["job"]["status"], 9);
    assert_eq!(e["job"]["status_label"], "Cancelled");
    assert!(e["history"].as_array().unwrap().iter().all(|h| h["ended"].is_string()));
    let (st, _) = send(&app, "POST", &format!("/api/v1/wf/jobs/{oid}/end"), None, Some(json!({"outcome": "completed"}))).await;
    assert_eq!(st, StatusCode::CONFLICT);

    let (_, v) = send(&app, "GET", "/api/v1/audit/verify", None, None).await;
    assert_eq!(v["ok"], true, "{v}");
}

/// Editing under an Aumentum job: job area rules, commit task, AllowOverlap = 0, discard.
#[tokio::test]
async fn job_editing_follows_job_area_and_commit_rules() {
    let Some(db_url) = test_db_url() else {
        eprintln!("skipping: CADASTRE_TEST_DATABASE_URL not set");
        return;
    };
    let app = routes::router(state_with(AuthMode::Disabled, None, &db_url).await);
    let pool = sqlx::PgPool::connect(&db_url).await.unwrap();
    let b = 200_000 + (rand_12() % 700_000) as i32 * 10;
    let (construct, commit_t, plan) = (b, b + 1, b + 2);
    for (id, name) in [(construct, "Construct Parcel"), (commit_t, "Commit Parcel"), (plan, "Generate Title Deed Plan")] {
        sqlx::query("INSERT INTO cadastre.wf_task (id, name) VALUES ($1, $2)").bind(id).bind(name).execute(&pool).await.unwrap();
    }
    sqlx::query("INSERT INTO cadastre.wf_step (id, name, commands) VALUES ($1, 'Commit Parcel', ARRAY[$2])")
        .bind(b).bind(cadastre_server::routes::wf::COMMIT_TOOL).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO cadastre.wf_task_step (task_id, step_id) VALUES ($1, $2)").bind(commit_t).bind(b).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO cadastre.wf_job_type (id, name, start_task_id, end_task_id) VALUES ($1, 'Editing (test)', $2, $3)")
        .bind(b).bind(construct).bind(plan).execute(&pool).await.unwrap();
    for (f, t) in [(construct, commit_t), (commit_t, plan)] {
        sqlx::query("INSERT INTO cadastre.wf_task_flow (job_type_id, task_id, next_task_id) VALUES ($1, $2, $3)").bind(b).bind(f).bind(t).execute(&pool).await.unwrap();
    }
    // Somewhere unique per run (no other test's parcels there), inside UTM zone 32N.
    let x0 = 150_000.0 + ((b / 10) % 60_000) as f64 * 10.0;
    let y0 = 9_000_000.0; // far north of the other tests' parcels

    let (_, d) = send(&app, "POST", "/api/v1/wf/jobs", None, Some(json!({"job_type_id": b, "name": "Job-PL1"}))).await;
    let id = d["job"]["id"].as_i64().unwrap();
    let (st, e) = send(&app, "POST", &format!("/api/v1/wf/jobs/{id}/editing"), None, None).await;
    assert_eq!(st, StatusCode::OK, "{e}");
    let cjob = e["cadastre_job_id"].as_str().unwrap().to_string();
    let draft_url = format!("/api/v1/jobs/{cjob}/parcels");
    let draft = |x: f64, s: f64| send(&app, "POST", &draft_url, None, Some(json!({"geometry": square(x, y0 + 10.0, s)})));

    let (st, _) = draft(x0 + 10.0, 20.0).await;
    assert_eq!(st, StatusCode::CONFLICT, "no job area yet");
    let (st, _) = send(&app, "PUT", &format!("/api/v1/wf/jobs/{id}/area"), None, Some(json!({"geometry": square(5_000_000.0, 990.0, 200.0)}))).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "a job area outside UTM 32N is refused");
    let (st, e) = send(&app, "PUT", &format!("/api/v1/wf/jobs/{id}/area"), None, Some(json!({"geometry": square(x0, y0, 200.0)}))).await;
    assert_eq!(st, StatusCode::OK, "{e}");
    assert_eq!(e["area_status"], "open");
    let (st, r) = draft(x0 + 190.0, 50.0).await; // crosses the area's edge
    assert_eq!(st, StatusCode::CONFLICT);
    assert_eq!(r["message"], cadastre_server::routes::wf::OUTSIDE_JOB_AREA);
    let (st, _) = draft(x0 + 10.0, 20.0).await;
    assert_eq!(st, StatusCode::CREATED);
    let (st, _) = draft(x0 + 50.0, 20.0).await;
    assert_eq!(st, StatusCode::CREATED);

    // Edit Vertices on a saved draft: moved inside the area is saved, outside is refused with
    // Aumentum's message; a draft can be deleted; posted parcels are not drafts.
    let (st, extra) = draft(x0 + 100.0, 20.0).await;
    assert_eq!(st, StatusCode::CREATED);
    let extra_url = format!("{draft_url}/{}", extra["id"].as_str().unwrap());
    let (st, moved) = send(&app, "PUT", &extra_url, None, Some(json!({"geometry": square(x0 + 120.0, y0 + 30.0, 25.0)}))).await;
    assert_eq!(st, StatusCode::OK, "{moved}");
    let (st, r) = send(&app, "PUT", &extra_url, None, Some(json!({"geometry": square(x0 + 190.0, y0 + 30.0, 25.0)}))).await;
    assert_eq!(st, StatusCode::CONFLICT, "{r}");
    assert_eq!(r["message"], cadastre_server::routes::wf::OUTSIDE_JOB_AREA);
    let (st, _) = send(&app, "DELETE", &extra_url, None, None).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let (st, _) = send(&app, "DELETE", &extra_url, None, None).await;
    assert_eq!(st, StatusCode::NOT_FOUND);

    let (st, _) = send(&app, "POST", &format!("/api/v1/wf/jobs/{id}/commit"), None, None).await;
    assert_eq!(st, StatusCode::CONFLICT, "commit only at the commit task");
    let (_, _) = send(&app, "POST", &format!("/api/v1/wf/jobs/{id}/advance"), None, Some(json!({}))).await;
    let (st, _) = send(&app, "POST", &format!("/api/v1/wf/jobs/{id}/advance"), None, Some(json!({}))).await;
    assert_eq!(st, StatusCode::CONFLICT, "cannot leave the commit task with drafts");

    // A posted parcel under one draft: AllowOverlap = 0 refuses the commit, naming it.
    let blocker = format!("PL{:012}", 900_000_000_000u64 + b as u64);
    sqlx::query("INSERT INTO cadastre.parcel (parcel_number, geom, status) VALUES ($1, ST_GeomFromText($2, 32632), 'posted')")
        .bind(&blocker)
        .bind(format!("POLYGON(({a} {b0},{c} {b0},{c} {b1},{a} {b1},{a} {b0}))", a = x0 + 15.0, c = x0 + 25.0, b0 = y0 + 15.0, b1 = y0 + 25.0))
        .execute(&pool).await.unwrap();
    let (st, r) = send(&app, "POST", &format!("/api/v1/wf/jobs/{id}/commit"), None, None).await;
    assert_eq!(st, StatusCode::CONFLICT, "{r}");
    assert!(r["message"].as_str().unwrap().contains(&blocker));
    sqlx::query("DELETE FROM cadastre.parcel WHERE parcel_number = $1").bind(&blocker).execute(&pool).await.unwrap();

    // Beacon numbers are typed by the officer at commit, never generated. One corner lies on a
    // fabric beacon already (a shared pillar): it keeps that beacon's number.
    let tag = format!("B{}", rand_12() % 100_000_000);
    let (st, _) = send(&app, "POST", "/api/v1/beacons", None,
        Some(json!({"number": format!("{tag}X"), "easting": x0 + 10.0, "northing": y0 + 10.0}))).await;
    assert_eq!(st, StatusCode::CREATED);
    let (st, pts) = send(&app, "GET", &format!("/api/v1/wf/jobs/{id}/editing/corners"), None, None).await;
    assert_eq!(st, StatusCode::OK, "{pts}");
    let pts = pts.as_array().unwrap().clone();
    assert_eq!(pts.len(), 8, "two separate squares: 8 corner points");
    assert_eq!(pts.iter().filter(|p| p["beacon_number"] == format!("{tag}X")).count(), 1, "the shared pillar keeps its number");
    let new_pts: Vec<&Value> = pts.iter().filter(|p| p["beacon_id"].is_null()).collect();
    let typed = |numbers: Vec<String>| json!({"beacons": new_pts.iter().zip(numbers).map(|(p, n)| json!({"easting": p["easting"], "northing": p["northing"], "number": n})).collect::<Vec<_>>()});

    let (st, r) = send(&app, "POST", &format!("/api/v1/wf/jobs/{id}/commit"), None, None).await;
    assert_eq!(st, StatusCode::CONFLICT, "no beacon numbers typed: {r}");
    assert!(r["message"].as_str().unwrap().contains("beacon number of every corner"), "{r}");
    let mut numbers: Vec<String> = (1..=7).map(|i| format!("{tag}-{i}")).collect();
    numbers[6] = numbers[5].clone();
    let (st, r) = send(&app, "POST", &format!("/api/v1/wf/jobs/{id}/commit"), None, Some(typed(numbers.clone()))).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "one number for two corners: {r}");
    numbers[6] = format!("{tag}x").to_lowercase().to_uppercase(); // the pillar's number, used elsewhere
    let (st, r) = send(&app, "POST", &format!("/api/v1/wf/jobs/{id}/commit"), None, Some(typed(numbers.clone()))).await;
    assert_eq!(st, StatusCode::CONFLICT, "a number already in the fabric elsewhere: {r}");
    assert!(r["message"].as_str().unwrap().contains("already exists"), "{r}");
    numbers[6] = format!("{tag}-7");

    let (st, c) = send(&app, "POST", &format!("/api/v1/wf/jobs/{id}/commit"), None, Some(typed(numbers.clone()))).await;
    assert_eq!(st, StatusCode::OK, "{c}");
    assert_eq!(c["parcel_numbers"].as_array().unwrap().len(), 2);
    assert_eq!(c["beacons_added"].as_array().unwrap().len(), 7, "{c}");
    assert_eq!(c["editing"]["drafts"], 0);
    let linked: i64 = sqlx::query_scalar("SELECT count(*) FROM cadastre.parcel_corner pc JOIN cadastre.parcel p ON p.id = pc.parcel_id WHERE p.parcel_number = ANY($1)")
        .bind(c["parcel_numbers"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect::<Vec<_>>())
        .fetch_one(&pool).await.unwrap();
    assert_eq!(linked, 8, "every corner of both parcels is linked to its beacon");
    let (_, found) = send(&app, "GET", &format!("/api/v1/beacons/search?q={tag}-3"), None, None).await;
    assert_eq!(found[0]["number"], format!("{tag}-3"), "a committed beacon is found by its number");

    // A closed area takes no edits; discard; delete the area once it is empty.
    let (_, e) = send(&app, "POST", &format!("/api/v1/wf/jobs/{id}/area/close"), None, None).await;
    assert_eq!(e["area_status"], "closed");
    let (st, _) = draft(x0 + 100.0, 20.0).await;
    assert_eq!(st, StatusCode::CONFLICT);
    let (_, _) = send(&app, "POST", &format!("/api/v1/wf/jobs/{id}/area/reopen"), None, None).await;
    let (st, _) = draft(x0 + 100.0, 20.0).await;
    assert_eq!(st, StatusCode::CREATED);
    // Phase 3 under a workflow job. Create Parcel by FCI: an uploaded polygon shapefile becomes
    // drafts (no .prj: taken as UTM 32N); one polygon outside the job area refuses the whole file.
    let drafts_now = |app: Router| { let u = format!("/api/v1/parcels.geojson?status=draft&job={cjob}"); async move { send(&app, "GET", &u, None, None).await.1["features"].as_array().unwrap().clone() } };
    let before = drafts_now(app.clone()).await.len();
    let fci = format!("/api/v1/jobs/{cjob}/import/feature-class");
    let sq = |x: f64, y: f64, s: f64| vec![(x, y), (x, y + s), (x + s, y + s), (x + s, y), (x, y)];
    let shp = shapefile_polygons(&[sq(x0 + 20.0, y0 + 120.0, 20.0), sq(x0 + 60.0, y0 + 120.0, 20.0)]);
    let (st, r) = send(&app, "POST", &fci, None, Some(json!({"shp_base64": STANDARD.encode(&shp), "name": "plots.shp"}))).await;
    assert_eq!(st, StatusCode::CREATED, "{r}");
    assert_eq!(r["created_parcels"], 2);
    assert!(r["warnings"][0].as_str().unwrap().contains("32632"), "{r}");
    let outside = shapefile_polygons(&[sq(x0 + 100.0, y0 + 160.0, 20.0), sq(x0 + 190.0, y0 + 160.0, 30.0)]);
    let (st, r) = send(&app, "POST", &fci, None, Some(json!({"shp_base64": STANDARD.encode(&outside)}))).await;
    assert_eq!(st, StatusCode::CONFLICT, "{r}");
    assert!(r["message"].as_str().unwrap().contains("FID 1"), "{r}");
    assert_eq!(drafts_now(app.clone()).await.len(), before + 2, "nothing imported from a refused file");
    // The same polygon in WGS 84 (with its .prj) is converted to UTM 32N.
    let ll: String = sqlx::query_scalar("SELECT ST_AsText(ST_Transform(ST_SetSRID(ST_MakePoint($1, $2), 32632), 4326))").bind(x0 + 130.0).bind(y0 + 60.0).fetch_one(&pool).await.unwrap();
    let nums: Vec<f64> = ll.trim_start_matches("POINT(").trim_end_matches(')').split(' ').map(|v| v.parse().unwrap()).collect();
    let d = 0.00015; // ~16 m
    let wgs = shapefile_polygons(&[vec![(nums[0], nums[1]), (nums[0], nums[1] + d), (nums[0] + d, nums[1] + d), (nums[0] + d, nums[1]), (nums[0], nums[1])]]);
    let prj = r#"GEOGCS["GCS_WGS_1984",DATUM["D_WGS_1984",SPHEROID["WGS_1984",6378137.0,298.257223563]],PRIMEM["Greenwich",0.0],UNIT["Degree",0.0174532925199433]]"#;
    let (st, r) = send(&app, "POST", &fci, None, Some(json!({"shp_base64": STANDARD.encode(&wgs), "prj": prj}))).await;
    assert_eq!(st, StatusCode::CREATED, "{r}");
    assert_eq!(r["source_epsg"], 4326);

    // Parcel Separation: a line across the first FCI parcel makes two drafts of 200 m² each; a
    // line that does not cross is refused.
    let draft_at = |x: f64, y: f64| {
        let pool = pool.clone(); let cjob = cjob.clone();
        async move {
            sqlx::query_scalar::<_, Uuid>("SELECT id FROM cadastre.parcel WHERE job_id = $1::uuid AND status = 'draft' AND ST_Contains(geom, ST_SetSRID(ST_MakePoint($2, $3), 32632))")
                .bind(cjob).bind(x).bind(y).fetch_one(&pool).await.unwrap()
        }
    };
    let sep = format!("{draft_url}/{}/separate", draft_at(x0 + 30.0, y0 + 130.0).await);
    let (st, r) = send(&app, "POST", &sep, None, Some(json!({"line": {"type": "LineString", "coordinates": [[x0 + 5.0, y0 + 5.0], [x0 + 6.0, y0 + 6.0]]}}))).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{r}");
    let (st, parts) = send(&app, "POST", &sep, None, Some(json!({"line": {"type": "LineString", "coordinates": [[x0 + 30.0, y0 + 110.0], [x0 + 30.0, y0 + 150.0]]}}))).await;
    assert_eq!(st, StatusCode::CREATED, "{parts}");
    let areas: Vec<f64> = parts.as_array().unwrap().iter().map(|p| p["computed_area"].as_f64().unwrap().round()).collect();
    assert_eq!(areas, vec![200.0, 200.0], "{parts}");
    // Donut Parcel: a hole inside is accepted; a "hole" outside the parcel is not a valid shape.
    let put = format!("{draft_url}/{}", draft_at(x0 + 70.0, y0 + 130.0).await);
    let outer = json!([[x0 + 60.0, y0 + 120.0], [x0 + 80.0, y0 + 120.0], [x0 + 80.0, y0 + 140.0], [x0 + 60.0, y0 + 140.0], [x0 + 60.0, y0 + 120.0]]);
    let hole = json!([[x0 + 65.0, y0 + 125.0], [x0 + 65.0, y0 + 130.0], [x0 + 70.0, y0 + 130.0], [x0 + 70.0, y0 + 125.0], [x0 + 65.0, y0 + 125.0]]);
    let (st, r) = send(&app, "PUT", &put, None, Some(json!({"geometry": {"type": "Polygon", "coordinates": [outer, hole]}}))).await;
    assert_eq!(st, StatusCode::OK, "{r}");
    let bad = json!([[x0 + 90.0, y0 + 125.0], [x0 + 90.0, y0 + 130.0], [x0 + 95.0, y0 + 130.0], [x0 + 95.0, y0 + 125.0], [x0 + 90.0, y0 + 125.0]]);
    let (st, r) = send(&app, "PUT", &put, None, Some(json!({"geometry": {"type": "Polygon", "coordinates": [outer, bad]}}))).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{r}");

    // Survey import under a workflow job: point IDs come back for the commit, no beacons are made.
    let csv = format!("id,easting,northing\nZQ{b}A,{},{}\nZQ{b}B,{},{}\n", x0 + 150.0, y0 + 20.0, x0 + 160.0, y0 + 20.0);
    let (st, r) = send(&app, "POST", &format!("/api/v1/jobs/{cjob}/import"), None, Some(json!({"filename": "points.csv", "contents": csv}))).await;
    assert_eq!(st, StatusCode::CREATED, "{r}");
    assert_eq!(r["workflow_job"], true);
    assert_eq!(r["created_beacons"], 0, "beacons are entered at commit, not at import");
    assert_eq!(r["points_for_commit"].as_array().unwrap().len(), 2, "{r}");

    let (st, _) = send(&app, "DELETE", &format!("/api/v1/wf/jobs/{id}/area"), None, None).await;
    assert_eq!(st, StatusCode::CONFLICT, "drafts block deleting the area");
    let (_, e) = send(&app, "POST", &format!("/api/v1/wf/jobs/{id}/discard"), None, None).await;
    assert_eq!(e["drafts"], 0);
    let (st, e) = send(&app, "DELETE", &format!("/api/v1/wf/jobs/{id}/area"), None, None).await;
    assert_eq!(st, StatusCode::OK);
    assert!(e["area"].is_null());
    let (st, d) = send(&app, "POST", &format!("/api/v1/wf/jobs/{id}/advance"), None, Some(json!({}))).await;
    assert_eq!(st, StatusCode::OK, "{d}");
    assert_eq!(d["job"]["current_task_id"], plan);
}

/// The Beacon box searches every beacon on the server: exact number first, then numbers starting
/// with the text, nearest first; and the map loads survey beacons only for the area in view.
#[tokio::test]
async fn beacon_search_ranks_exact_then_prefix_then_nearest() {
    let Some(db_url) = test_db_url() else {
        eprintln!("skipping: CADASTRE_TEST_DATABASE_URL not set");
        return;
    };
    let app = routes::router(state_with(AuthMode::Disabled, None, &db_url).await);
    let tag = format!("T{}", rand_12() % 1_000_000);
    let (x0, y0) = (300_000.0 + (rand_12() % 100_000) as f64, 1_000_000.0 + (rand_12() % 100_000) as f64);
    for (name, dx, control) in [(tag.clone(), 900.0, true), (format!("{tag}1"), 500.0, false), (format!("{tag}2"), 10.0, false), (format!("X{tag}"), 1.0, false)] {
        let (st, b) = send(&app, "POST", "/api/v1/beacons", None,
            Some(json!({"number": name, "easting": x0 + dx, "northing": y0, "is_control": control}))).await;
        assert_eq!(st, StatusCode::CREATED, "{b}");
    }
    let (st, found) = send(&app, "GET", &format!("/api/v1/beacons/search?q={}&e={x0}&n={y0}", tag.to_lowercase()), None, None).await;
    assert_eq!(st, StatusCode::OK, "{found}");
    let names: Vec<&str> = found.as_array().unwrap().iter().map(|b| b["number"].as_str().unwrap()).collect();
    assert_eq!(names, [tag.clone(), format!("{tag}2"), format!("{tag}1"), format!("X{tag}")], "exact, then prefix by distance, then contains");

    // A wildcard typed by the user is taken literally.
    let (_, none) = send(&app, "GET", &format!("/api/v1/beacons/search?q={tag}_"), None, None).await;
    assert_eq!(none, json!([]), "{none}");

    // GeoJSON: survey beacons only, in a bbox around the test points (in WGS84).
    let (_, all) = send(&app, "GET", "/api/v1/beacons.geojson?control=false&limit=20000", None, None).await;
    let pick = all["features"].as_array().unwrap().iter().find(|f| f["properties"]["number"] == format!("{tag}2")).expect("survey beacon listed").clone();
    let (lon, lat) = (pick["geometry"]["coordinates"][0].as_f64().unwrap(), pick["geometry"]["coordinates"][1].as_f64().unwrap());
    let bbox = format!("{},{},{},{}", lon - 0.0003, lat - 0.0003, lon + 0.0003, lat + 0.0003);
    let (_, near) = send(&app, "GET", &format!("/api/v1/beacons.geojson?control=false&bbox={bbox}"), None, None).await;
    let near: Vec<String> = near["features"].as_array().unwrap().iter().map(|f| f["properties"]["number"].as_str().unwrap().to_string()).collect();
    assert!(near.contains(&format!("{tag}2")) && near.contains(&format!("X{tag}")) && !near.contains(&format!("{tag}1")), "{near:?}");
    let (_, ctrl) = send(&app, "GET", &format!("/api/v1/beacons.geojson?control=true&bbox={},{},{},{}", lon - 0.01, lat - 0.01, lon + 0.01, lat + 0.01), None, None).await;
    assert!(ctrl["features"].as_array().unwrap().iter().all(|f| f["properties"]["is_control"] == true), "{ctrl}");
}

/// A minimal polygon shapefile (.shp) holding one ring per polygon, for the FCI import tests.
fn shapefile_polygons(polys: &[Vec<(f64, f64)>]) -> Vec<u8> {
    let all: Vec<(f64, f64)> = polys.iter().flatten().copied().collect();
    let bbox = |pts: &[(f64, f64)]| {
        let (mut a, mut b, mut c, mut d) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for &(x, y) in pts { a = a.min(x); b = b.min(y); c = c.max(x); d = d.max(y); }
        [a, b, c, d]
    };
    let mut recs = Vec::new();
    for (i, ring) in polys.iter().enumerate() {
        let mut c = Vec::new();
        c.extend(5i32.to_le_bytes());
        for v in bbox(ring) { c.extend(v.to_le_bytes()); }
        c.extend(1i32.to_le_bytes());
        c.extend((ring.len() as i32).to_le_bytes());
        c.extend(0i32.to_le_bytes());
        for &(x, y) in ring { c.extend(x.to_le_bytes()); c.extend(y.to_le_bytes()); }
        recs.extend(((i + 1) as i32).to_be_bytes());
        recs.extend(((c.len() / 2) as i32).to_be_bytes());
        recs.extend(c);
    }
    let mut out = vec![0u8; 100];
    out[0..4].copy_from_slice(&9994i32.to_be_bytes());
    out[24..28].copy_from_slice(&(((100 + recs.len()) / 2) as i32).to_be_bytes());
    out[28..32].copy_from_slice(&1000i32.to_le_bytes());
    out[32..36].copy_from_slice(&5i32.to_le_bytes());
    for (k, v) in bbox(&all).iter().enumerate() { out[36 + k * 8..44 + k * 8].copy_from_slice(&v.to_le_bytes()); }
    out.extend(recs);
    out
}

/// Phase 4: mutations recorded in a workflow job and applied at commit, as Aumentum does (from
/// its parcelLineage history): split (1 -> many), merge (many -> 1), retire (archived with a
/// reason), cancel; parents become historic/archived, children get new UPINs, lineage is kept.
#[tokio::test]
async fn mutations_split_merge_retire_with_lineage() {
    let Some(db_url) = test_db_url() else {
        eprintln!("skipping: CADASTRE_TEST_DATABASE_URL not set");
        return;
    };
    let app = routes::router(state_with(AuthMode::Disabled, None, &db_url).await);
    let pool = sqlx::PgPool::connect(&db_url).await.unwrap();
    let b = 300_000 + (rand_12() % 600_000) as i32 * 10;
    let (construct, commit_t, plan) = (b, b + 1, b + 2);
    for (id, name) in [(construct, "Split Parcel"), (commit_t, "Commit Parcel"), (plan, "Generate Title Deed Plan")] {
        sqlx::query("INSERT INTO cadastre.wf_task (id, name) VALUES ($1, $2)").bind(id).bind(name).execute(&pool).await.unwrap();
    }
    sqlx::query("INSERT INTO cadastre.wf_step (id, name, commands) VALUES ($1, 'Commit Parcel', ARRAY[$2])")
        .bind(b).bind(cadastre_server::routes::wf::COMMIT_TOOL).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO cadastre.wf_task_step (task_id, step_id) VALUES ($1, $2)").bind(commit_t).bind(b).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO cadastre.wf_job_type (id, name, start_task_id, end_task_id) VALUES ($1, 'Mutation (test)', $2, $3)")
        .bind(b).bind(construct).bind(plan).execute(&pool).await.unwrap();
    for (f, t) in [(construct, commit_t), (commit_t, plan)] {
        sqlx::query("INSERT INTO cadastre.wf_task_flow (job_type_id, task_id, next_task_id) VALUES ($1, $2, $3)").bind(b).bind(f).bind(t).execute(&pool).await.unwrap();
    }
    let x0 = 150_000.0 + ((b / 10) % 60_000) as f64 * 10.0;
    let y0 = 8_800_000.0;
    // Four posted parcels: A alone; B and C sharing an edge; D alone.
    let tag = format!("PLM{}", rand_12() % 1_000_000_000);
    let upin = |k: &str| format!("{tag}{k}");
    for (k, x, y) in [("A", 10.0, 10.0), ("B", 60.0, 10.0), ("C", 80.0, 10.0), ("D", 130.0, 10.0)] {
        sqlx::query("INSERT INTO cadastre.parcel (parcel_number, geom, computed_area, status) VALUES ($1, ST_MakeEnvelope($2, $3, $2 + 20, $3 + 20, 32632), 400, 'posted')")
            .bind(upin(k)).bind(x0 + x).bind(y0 + y).execute(&pool).await.unwrap();
    }
    let (_, d) = send(&app, "POST", "/api/v1/wf/jobs", None, Some(json!({"job_type_id": b, "name": "Job-MUT"}))).await;
    let id = d["job"]["id"].as_i64().unwrap();
    let murl = format!("/api/v1/wf/jobs/{id}/mutations");
    let (st, r) = send(&app, "POST", &murl, None, Some(json!({"kind": "split", "upins": [upin("A")]}))).await;
    assert_eq!(st, StatusCode::CONFLICT, "editing not started: {r}");
    send(&app, "POST", &format!("/api/v1/wf/jobs/{id}/editing"), None, None).await;
    let (st, _) = send(&app, "PUT", &format!("/api/v1/wf/jobs/{id}/area"), None, Some(json!({"geometry": square(x0, y0, 200.0)}))).await;
    assert_eq!(st, StatusCode::OK);

    // Split A along x = x0 + 25.
    let line = json!({"type": "LineString", "coordinates": [[x0 + 25.0, y0 + 5.0], [x0 + 25.0, y0 + 35.0]]});
    let (st, split) = send(&app, "POST", &murl, None, Some(json!({"kind": "split", "upins": [upin("A")], "line": line}))).await;
    assert_eq!(st, StatusCode::CREATED, "{split}");
    assert_eq!(split["drafts"].as_array().unwrap().len(), 2);
    let (st, r) = send(&app, "POST", &murl, None, Some(json!({"kind": "resurvey", "upins": [upin("A")]}))).await;
    assert_eq!(st, StatusCode::CONFLICT, "A is already being mutated: {r}");
    // Merge B + C (adjacent); B + D do not touch.
    let (st, r) = send(&app, "POST", &murl, None, Some(json!({"kind": "merge", "upins": [upin("B"), upin("D")]}))).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{r}");
    let (st, merge) = send(&app, "POST", &murl, None, Some(json!({"kind": "merge", "upins": [upin("B"), upin("C")]}))).await;
    assert_eq!(st, StatusCode::CREATED, "{merge}");
    // Retire D: a reason is required.
    let (st, _) = send(&app, "POST", &murl, None, Some(json!({"kind": "retire", "upins": [upin("D")]}))).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let (st, _) = send(&app, "POST", &murl, None, Some(json!({"kind": "retire", "upins": [upin("D")], "reason": "overlapping"}))).await;
    assert_eq!(st, StatusCode::CREATED);
    // A re-survey recorded then cancelled leaves nothing behind.
    sqlx::query("INSERT INTO cadastre.parcel (parcel_number, geom, computed_area, status) VALUES ($1, ST_MakeEnvelope($2, $3, $2 + 20, $3 + 20, 32632), 400, 'posted')")
        .bind(upin("E")).bind(x0 + 10.0).bind(y0 + 100.0).execute(&pool).await.unwrap();
    let (_, rs) = send(&app, "POST", &murl, None, Some(json!({"kind": "resurvey", "upins": [upin("E")]}))).await;
    let (st, _) = send(&app, "DELETE", &format!("{murl}/{}", rs["group"].as_str().unwrap()), None, None).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let (_, pending) = send(&app, "GET", &murl, None, None).await;
    assert_eq!(pending.as_array().unwrap().len(), 3, "split, merge, retire pending: {pending}");

    // The parents' Aumentum attribute records (B before C, so B is a merge's source).
    let oid = 800_000_000 + (rand_12() % 100_000_000) as i32;
    for (k, o, block, pn) in [("A", oid, "B7", "PL999"), ("B", oid + 1, "B9", "PL111"), ("C", oid + 2, "C1", "PL222")] {
        sqlx::query("INSERT INTO cadastre.parcel_attribute (object_id, upin, dataset_name, region_name, parcel_name, block, lga, location, property_type, area_unit, property_number, status, lrs_id, linkage_status, plan_number) \
                     VALUES ($1, $2, 'DS', 'Plateau_State', 'PCL1', $3, '6', '1', '1', 2, $4, '0', $5, 1, 'PLAN-X')")
            .bind(o).bind(upin(k)).bind(block).bind(pn).bind(format!("LRS-{k}")).execute(&pool).await.unwrap();
    }

    // Commit at the commit task (beacon numbers typed for every new corner).
    send(&app, "POST", &format!("/api/v1/wf/jobs/{id}/advance"), None, Some(json!({}))).await;
    let (_, pts) = send(&app, "GET", &format!("/api/v1/wf/jobs/{id}/editing/corners"), None, None).await;
    let beacons: Vec<Value> = pts.as_array().unwrap().iter().filter(|p| p["beacon_id"].is_null()).enumerate()
        .map(|(i, p)| json!({"easting": p["easting"], "northing": p["northing"], "number": format!("{tag}-{i}")})).collect();
    let (st, c) = send(&app, "POST", &format!("/api/v1/wf/jobs/{id}/commit"), None, Some(json!({"beacons": beacons}))).await;
    assert_eq!(st, StatusCode::OK, "children overlap their parents, which are being replaced: {c}");
    let new_upins: Vec<String> = c["parcel_numbers"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
    assert_eq!(new_upins.len(), 3, "two split parts and one merged parcel: {c}");
    let mut retired: Vec<String> = c["retired"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
    retired.sort();
    assert_eq!(retired, vec![upin("A"), upin("B"), upin("C"), upin("D")]);
    let status_of = |u: String| { let pool = pool.clone(); async move { sqlx::query_scalar::<_, String>("SELECT string_agg(status, ',' ORDER BY status) FROM cadastre.parcel WHERE parcel_number = $1").bind(u).fetch_one(&pool).await.unwrap() } };
    assert_eq!(status_of(upin("A")).await, "historic");
    assert_eq!(status_of(upin("D")).await, "archived");
    assert_eq!(status_of(upin("E")).await, "posted", "the cancelled re-survey changed nothing");
    for u in &new_upins { assert_eq!(status_of(u.clone()).await, "posted"); }

    // Attribute inheritance (Aumentum IsMutationInherited): new records for the new parcels.
    let attrs = sqlx::query_as::<_, (String, Option<String>, Option<String>, Option<String>, Option<String>, Option<String>, Option<f64>, Option<i16>, Option<String>, Option<String>, Option<String>)>(
        "SELECT upin, parcel_name, block, lga, location, property_number, parcel_area, mutation_type, lrs_id, plan_number, dataset_name FROM cadastre.parcel_attribute WHERE upin = ANY($1) ORDER BY upin")
        .bind(&new_upins).fetch_all(&pool).await.unwrap();
    assert_eq!(attrs.len(), 3, "one attribute record per new parcel: {attrs:?}");
    let split_rows: Vec<_> = attrs.iter().filter(|a| a.7 == Some(1)).collect();
    let merge_row = attrs.iter().find(|a| a.7 == Some(2)).expect("merged parcel's record");
    assert_eq!(split_rows.len(), 2);
    let mut names: Vec<_> = split_rows.iter().map(|a| a.1.clone().unwrap()).collect();
    names.sort();
    assert_eq!(names, vec!["PCL1", "PCL2"], "children are named in order");
    for a in &split_rows {
        assert_eq!((a.2.as_deref(), a.3.as_deref(), a.4.as_deref(), a.8.as_deref(), a.10.as_deref()), (Some("B7"), Some("6"), Some("1"), Some("LRS-A"), Some("DS")), "inherited from A: {a:?}");
        assert_eq!((a.5.as_deref(), a.9.as_deref()), (None, None), "Registry property number and plan number are not inherited: {a:?}");
    }
    let mut areas: Vec<f64> = split_rows.iter().map(|a| a.6.unwrap().round()).collect();
    areas.sort_by(f64::total_cmp);
    assert_eq!(areas, vec![100.0, 300.0], "each child's area from its own shape (the line cuts A 5 m / 15 m)");
    assert_eq!((merge_row.1.as_deref(), merge_row.2.as_deref(), merge_row.8.as_deref()), (Some("PCL1"), Some("B9"), Some("LRS-B")), "a merge inherits from its first parent: {merge_row:?}");
    assert!((merge_row.6.unwrap() - 800.0).abs() < 0.01, "{merge_row:?}");

    // Lineage, as Aumentum records it.
    let (_, la) = send(&app, "GET", &format!("/api/v1/parcels/lineage?upin={}", upin("A")), None, None).await;
    let kids: Vec<&str> = la["edges"].as_array().unwrap().iter().filter(|e| e["parent_upin"] == upin("A")).map(|e| e["child_upin"].as_str().unwrap()).collect();
    assert_eq!(kids.len(), 2, "{la}");
    assert!(la["edges"].as_array().unwrap().iter().all(|e| e["mutation"] == "Split"), "{la}");
    let merged = new_upins.iter().find(|u| !kids.contains(&u.as_str())).unwrap();
    let (_, lm) = send(&app, "GET", &format!("/api/v1/parcels/lineage?upin={merged}"), None, None).await;
    let mut parents: Vec<&str> = lm["edges"].as_array().unwrap().iter().map(|e| e["parent_upin"].as_str().unwrap()).collect();
    parents.sort();
    assert_eq!(parents, vec![upin("B").as_str(), upin("C").as_str()], "{lm}");
    let (_, ld) = send(&app, "GET", &format!("/api/v1/parcels/lineage?upin={}", upin("D")), None, None).await;
    assert_eq!(ld["retirements"][0]["reason"], "overlapping", "{ld}");
    let reason: Option<String> = sqlx::query_scalar("SELECT archive_reason FROM cadastre.parcel_attribute WHERE upin = $1").bind(upin("D")).fetch_optional(&pool).await.unwrap().flatten();
    assert!(reason.is_none(), "no attribute row exists for a test parcel; nothing invented");
    // The retired shapes are on the history layer; a retired parcel cannot be mutated again.
    let (_, hist) = send(&app, "GET", "/api/v1/parcels.geojson?status=retired&limit=20000", None, None).await;
    assert!(hist["features"].as_array().unwrap().iter().any(|f| f["properties"]["parcel_number"] == upin("A")));
    let (_, e2) = send(&app, "GET", &format!("/api/v1/wf/jobs/{id}/editing"), None, None).await;
    assert_eq!(e2["drafts"], 0);
    let (st, _) = send(&app, "POST", &murl, None, Some(json!({"kind": "resurvey", "upins": [upin("A")]}))).await;
    assert_eq!(st, StatusCode::CONFLICT);
}

/// Phase 4: Aumentum's other two mutation types, as its parcelLineage history has them: a
/// Separation (6, one parcel cut into parts along a line) and a Free Mutation (4, one parcel to
/// one with a new UPIN). Lineage and attribute records carry the type.
#[tokio::test]
async fn mutations_separate_and_free_mutation() {
    let Some(db_url) = test_db_url() else {
        eprintln!("skipping: CADASTRE_TEST_DATABASE_URL not set");
        return;
    };
    let app = routes::router(state_with(AuthMode::Disabled, None, &db_url).await);
    let pool = sqlx::PgPool::connect(&db_url).await.unwrap();
    let b = 300_001 + (rand_12() % 600_000) as i32 * 10;
    let (construct, commit_t, plan) = (b, b + 1, b + 2);
    for (id, name) in [(construct, "Split Parcel"), (commit_t, "Commit Parcel"), (plan, "Generate Title Deed Plan")] {
        sqlx::query("INSERT INTO cadastre.wf_task (id, name) VALUES ($1, $2)").bind(id).bind(name).execute(&pool).await.unwrap();
    }
    sqlx::query("INSERT INTO cadastre.wf_step (id, name, commands) VALUES ($1, 'Commit Parcel', ARRAY[$2])")
        .bind(b).bind(cadastre_server::routes::wf::COMMIT_TOOL).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO cadastre.wf_task_step (task_id, step_id) VALUES ($1, $2)").bind(commit_t).bind(b).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO cadastre.wf_job_type (id, name, start_task_id, end_task_id) VALUES ($1, 'Mutation 4/6 (test)', $2, $3)")
        .bind(b).bind(construct).bind(plan).execute(&pool).await.unwrap();
    for (f, t) in [(construct, commit_t), (commit_t, plan)] {
        sqlx::query("INSERT INTO cadastre.wf_task_flow (job_type_id, task_id, next_task_id) VALUES ($1, $2, $3)").bind(b).bind(f).bind(t).execute(&pool).await.unwrap();
    }
    let x0 = 150_000.0 + ((b / 10) % 60_000) as f64 * 10.0;
    let y0 = 8_900_000.0;
    let tag = format!("PLN{}", rand_12() % 1_000_000_000);
    let upin = |k: &str| format!("{tag}{k}");
    for (k, x) in [("F", 10.0), ("G", 60.0)] {
        sqlx::query("INSERT INTO cadastre.parcel (parcel_number, geom, computed_area, status) VALUES ($1, ST_MakeEnvelope($2, $3, $2 + 20, $3 + 20, 32632), 400, 'posted')")
            .bind(upin(k)).bind(x0 + x).bind(y0 + 10.0).execute(&pool).await.unwrap();
    }
    let oid = 700_000_000 + (rand_12() % 100_000_000) as i32;
    for (k, o) in [("F", oid), ("G", oid + 1)] {
        sqlx::query("INSERT INTO cadastre.parcel_attribute (object_id, upin, dataset_name, region_name, parcel_name, block, status) VALUES ($1, $2, 'DS', 'Plateau_State', 'PCL1', 'B1', '0')")
            .bind(o).bind(upin(k)).execute(&pool).await.unwrap();
    }
    let (_, d) = send(&app, "POST", "/api/v1/wf/jobs", None, Some(json!({"job_type_id": b, "name": "Job-MUT46"}))).await;
    let id = d["job"]["id"].as_i64().unwrap();
    let murl = format!("/api/v1/wf/jobs/{id}/mutations");
    send(&app, "POST", &format!("/api/v1/wf/jobs/{id}/editing"), None, None).await;
    send(&app, "PUT", &format!("/api/v1/wf/jobs/{id}/area"), None, Some(json!({"geometry": square(x0, y0, 200.0)}))).await;

    // A separation needs its line and one parcel; a free mutation one parcel.
    let (st, r) = send(&app, "POST", &murl, None, Some(json!({"kind": "separate", "upins": [upin("F")]}))).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{r}");
    let (st, r) = send(&app, "POST", &murl, None, Some(json!({"kind": "free", "upins": [upin("F"), upin("G")]}))).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{r}");
    let line = json!({"type": "LineString", "coordinates": [[x0 + 20.0, y0 + 5.0], [x0 + 20.0, y0 + 35.0]]});
    let (st, sep) = send(&app, "POST", &murl, None, Some(json!({"kind": "separate", "upins": [upin("F")], "line": line}))).await;
    assert_eq!(st, StatusCode::CREATED, "{sep}");
    assert_eq!((sep["kind"].as_str(), sep["drafts"].as_array().unwrap().len()), (Some("separate"), 2));
    let (st, free) = send(&app, "POST", &murl, None, Some(json!({"kind": "free", "upins": [upin("G")]}))).await;
    assert_eq!(st, StatusCode::CREATED, "{free}");
    assert_eq!((free["kind"].as_str(), free["drafts"].as_array().unwrap().len()), (Some("free"), 1));

    send(&app, "POST", &format!("/api/v1/wf/jobs/{id}/advance"), None, Some(json!({}))).await;
    let (_, pts) = send(&app, "GET", &format!("/api/v1/wf/jobs/{id}/editing/corners"), None, None).await;
    let beacons: Vec<Value> = pts.as_array().unwrap().iter().filter(|p| p["beacon_id"].is_null()).enumerate()
        .map(|(i, p)| json!({"easting": p["easting"], "northing": p["northing"], "number": format!("{tag}-{i}")})).collect();
    let (st, c) = send(&app, "POST", &format!("/api/v1/wf/jobs/{id}/commit"), None, Some(json!({"beacons": beacons}))).await;
    assert_eq!(st, StatusCode::OK, "{c}");
    let new_upins: Vec<String> = c["parcel_numbers"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
    assert_eq!(new_upins.len(), 3, "two separated parts and one free-mutation parcel: {c}");

    // Lineage and attribute records carry Aumentum's type codes and names.
    let (_, lf) = send(&app, "GET", &format!("/api/v1/parcels/lineage?upin={}", upin("F")), None, None).await;
    let f_edges: Vec<&Value> = lf["edges"].as_array().unwrap().iter().filter(|e| e["parent_upin"] == upin("F")).collect();
    assert_eq!(f_edges.len(), 2, "{lf}");
    assert!(f_edges.iter().all(|e| e["mutation"] == "Separated"), "{lf}");
    let (_, lg) = send(&app, "GET", &format!("/api/v1/parcels/lineage?upin={}", upin("G")), None, None).await;
    let g_edges: Vec<&Value> = lg["edges"].as_array().unwrap().iter().filter(|e| e["parent_upin"] == upin("G")).collect();
    assert_eq!(g_edges.len(), 1, "{lg}");
    assert_eq!(g_edges[0]["mutation"], "Free Mutation", "{lg}");
    let mut types: Vec<i16> = sqlx::query_scalar("SELECT mutation_type FROM cadastre.parcel_attribute WHERE upin = ANY($1)")
        .bind(&new_upins).fetch_all(&pool).await.unwrap();
    types.sort();
    assert_eq!(types, vec![4, 6, 6], "attribute records: Free Mutation once, Separated twice");
    for k in ["F", "G"] {
        let st: String = sqlx::query_scalar("SELECT status FROM cadastre.parcel WHERE parcel_number = $1").bind(upin(k)).fetch_one(&pool).await.unwrap();
        assert_eq!(st, "historic", "{k} was replaced by its mutation");
    }
}
