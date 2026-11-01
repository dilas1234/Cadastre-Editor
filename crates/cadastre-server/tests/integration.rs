//! DB-backed integration tests for Phase 3 (jobs, workflow, sessions/locks, auth).
//!
//! These drive the real `routes::router` in-process against a live PostGIS database.
//! They are skipped (not failed) when `CADASTRE_TEST_DATABASE_URL` is unset, so the
//! default `cargo test` stays hermetic; CI/dev sets it to a throwaway database.

use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{header, Request, StatusCode};
use axum::Router;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
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

    let (st, c) = send(&app, "POST", &format!("/api/v1/wf/jobs/{id}/commit"), None, None).await;
    assert_eq!(st, StatusCode::OK, "{c}");
    assert_eq!(c["parcel_numbers"].as_array().unwrap().len(), 2);
    assert_eq!(c["editing"]["drafts"], 0);

    // A closed area takes no edits; discard; delete the area once it is empty.
    let (_, e) = send(&app, "POST", &format!("/api/v1/wf/jobs/{id}/area/close"), None, None).await;
    assert_eq!(e["area_status"], "closed");
    let (st, _) = draft(x0 + 100.0, 20.0).await;
    assert_eq!(st, StatusCode::CONFLICT);
    let (_, _) = send(&app, "POST", &format!("/api/v1/wf/jobs/{id}/area/reopen"), None, None).await;
    let (st, _) = draft(x0 + 100.0, 20.0).await;
    assert_eq!(st, StatusCode::CREATED);
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
