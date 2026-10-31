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
