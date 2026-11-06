//! The title deed plan of a job: data and prefill, the saved form, the printable sheet, and the
//! refusals (no committed parcel; a UPIN that is not the job's). The Registry is not configured
//! here, so the prefill comes from the job and the parcel record only.

use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{header, Request, StatusCode};
use axum::Router;
use serde_json::{json, Value};
use tower::ServiceExt;

use cadastre_server::config::{AuthMode, Config};
use cadastre_server::state::AppState;
use cadastre_server::{db, routes};

const TYPE: i32 = 9_200; // a job type of this test only
const TASK: i32 = 9_200;

fn test_db_url() -> Option<String> {
    std::env::var("CADASTRE_TEST_DATABASE_URL").ok().filter(|s| !s.is_empty())
}

async fn app(db_url: &str) -> (Router, sqlx::PgPool) {
    let pool = db::connect_and_migrate(db_url).await.expect("connect + migrate test db");
    let config = Config {
        database_url: db_url.to_string(),
        bind_addr: "127.0.0.1:0".to_string(),
        auth_mode: AuthMode::Disabled,
        keycloak_issuer: None,
        keycloak_audience: None,
        keycloak_jwks_json: None,
        keycloak_jwks_path: None,
        catalog_folders: Vec::new(),
        catalog_databases: Vec::new(),
    };
    (routes::router(AppState { pool: pool.clone(), config, jwks: None::<Arc<_>> }), pool)
}

async fn send(router: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value, String) {
    let b = Request::builder().method(method).uri(uri);
    let req = match body {
        Some(v) => b.header(header::CONTENT_TYPE, "application/json").body(Body::from(v.to_string())).unwrap(),
        None => b.body(Body::empty()).unwrap(),
    };
    let resp = router.clone().oneshot(req).await.unwrap();
    let st = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8_lossy(&bytes).to_string();
    (st, serde_json::from_slice(&bytes).unwrap_or(Value::Null), text)
}

/// A workflow job named Job-<file> with its editing job; returns (job id, editing job id).
async fn wf_job(router: &Router, file: &str) -> (i64, String) {
    let (st, v, _) = send(router, "POST", "/api/v1/wf/jobs", Some(json!({"job_type_id": TYPE, "name": format!("Job-{file}")}))).await;
    assert!(st.is_success(), "{st} {v}");
    let id = v.get("job").unwrap_or(&v)["id"].as_i64().unwrap();
    let (st, e, _) = send(router, "POST", &format!("/api/v1/wf/jobs/{id}/editing"), Some(json!({}))).await;
    assert!(st.is_success(), "{st} {e}");
    (id, e["cadastre_job_id"].as_str().unwrap().to_string())
}

/// Commit one rectangle (stored anticlockwise from its south-east corner) under the editing job.
async fn commit_rect(router: &Router, job: i64, cid: &str, x: f64, y: f64) -> String {
    let area = json!({"geometry": {"type":"Polygon","coordinates":[[[x-5.0,y-5.0],[x+25.0,y-5.0],[x+25.0,y+15.0],[x-5.0,y+15.0],[x-5.0,y-5.0]]]}});
    let (st, a, _) = send(router, "PUT", &format!("/api/v1/wf/jobs/{job}/area"), Some(area)).await;
    assert!(st.is_success(), "job area: {st} {a}");
    let ring = json!({"type":"Polygon","coordinates":[[[x+20.0,y],[x+20.0,y+10.0],[x,y+10.0],[x,y],[x+20.0,y]]]});
    let (st, p, _) = send(router, "POST", &format!("/api/v1/jobs/{cid}/parcels"), Some(json!({"geometry": ring}))).await;
    assert_eq!(st, StatusCode::CREATED, "{p}");
    for action in ["submit", "start_review", "approve"] {
        let (st, j, _) = send(router, "POST", &format!("/api/v1/jobs/{cid}/transition"), Some(json!({ "action": action }))).await;
        assert_eq!(st, StatusCode::OK, "{action}: {j}");
    }
    let (st, r, _) = send(router, "POST", &format!("/api/v1/jobs/{cid}/post"), None).await;
    assert_eq!(st, StatusCode::OK, "{r}");
    r["parcel_numbers"][0].as_str().unwrap().to_string()
}

#[tokio::test]
async fn a_jobs_title_deed_plan_prefills_saves_and_prints() {
    let Some(url) = test_db_url() else {
        eprintln!("skipped: CADASTRE_TEST_DATABASE_URL not set");
        return;
    };
    std::env::remove_var("CADASTRE_REGISTRY_URL");
    std::env::remove_var("CADASTRE_AUMENTUM_JOBSERVICE_URL");
    let (router, pool) = app(&url).await;
    sqlx::query("INSERT INTO cadastre.wf_task (id, name) VALUES ($1, 'Title deed test task') ON CONFLICT (id) DO NOTHING").bind(TASK).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO cadastre.wf_job_type (id, name, start_task_id, end_task_id) VALUES ($1, 'Title deed test', $2, $2) ON CONFLICT (id) DO NOTHING")
        .bind(TYPE).bind(TASK).execute(&pool).await.unwrap();
    let seed = (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos() % 5_000) as f64;
    let (x, y) = (200_000.0 + seed * 100.0, 9_050_000.0); // inside UTM 32N, apart from the other tests

    // No committed parcel yet: refused, with the reason.
    let (job, cid) = wf_job(&router, "PLTD1").await;
    let (st, v, _) = send(&router, "GET", &format!("/api/v1/wf/jobs/{job}/titledeed"), None).await;
    assert_eq!(st, StatusCode::CONFLICT, "{v}");

    let upin = commit_rect(&router, job, &cid, x, y).await;
    let (st, d, _) = send(&router, "GET", &format!("/api/v1/wf/jobs/{job}/titledeed"), None).await;
    assert_eq!(st, StatusCode::OK, "{d}");
    assert_eq!(d["registry"], json!("off"));
    assert_eq!(d["plan"]["upin"], json!(upin));
    assert!((d["plan"]["area_m2"].as_f64().unwrap() - 200.0).abs() < 1e-6);
    assert_eq!(d["plan"]["fields"]["title_no"], json!("PLTD1"), "the file no. from the job");
    let corners = d["plan"]["corners"].as_array().unwrap();
    assert_eq!(corners.len(), 4);
    // Clockwise from the westernmost (south-west on a tie): SW, NW, NE, SE.
    let at = |i: usize| (corners[i]["e"].as_f64().unwrap() - x, corners[i]["n"].as_f64().unwrap() - y);
    assert_eq!([at(0), at(1), at(2), at(3)], [(0.0, 0.0), (0.0, 10.0), (20.0, 10.0), (20.0, 0.0)]);

    // The officer's form is saved with the job and wins over the prefill.
    let fields = json!({"holder": "Test Holder", "surveyed_by": "Test Surveys Ltd", "corner_names": ["TD1", "TD2", "TD3", "TD4"], "road_side": 3, "scale": 500});
    let (st, s, _) = send(&router, "PUT", &format!("/api/v1/wf/jobs/{job}/titledeed"), Some(json!({"upin": upin, "fields": fields}))).await;
    assert_eq!(st, StatusCode::OK, "{s}");
    assert_eq!(s["plan"]["fields"]["holder"], json!("Test Holder"));
    assert_eq!(s["plan"]["fields"]["title_no"], json!("PLTD1"), "blank saved fields keep the prefill");

    let (st, _, svg) = send(&router, "GET", &format!("/api/v1/wf/jobs/{job}/titledeed.svg"), None).await;
    assert_eq!(st, StatusCode::OK);
    for want in ["TITLE NO. PLTD1", "HOLDER: TEST HOLDER", "SURVEYED BY: Test Surveys Ltd", "Scale = 1:500", &format!("UPIN {upin}"),
                 "AREA 200.000 SQUARE METERS", ">TD1<", ">TD4<", "WGS 1984 Co-ordinates of TD1", "Access Road", ">0°<", ">90°<", ">20.00m<", ">10.00m<"] {
        assert!(svg.contains(want), "the sheet lacks {want}");
    }

    // A UPIN that is not this job's is refused.
    let (other, ocid) = wf_job(&router, "PLTD2").await;
    let other_upin = commit_rect(&router, other, &ocid, x + 100.0, y).await;
    let (st, v, _) = send(&router, "PUT", &format!("/api/v1/wf/jobs/{job}/titledeed"), Some(json!({"upin": other_upin, "fields": {}}))).await;
    assert_eq!(st, StatusCode::CONFLICT, "{v}");
    let _ = other;
}

/// A file back after its title deed: its job is closed and it has no link event, so the
/// transaction comes from the closed job, and no job is offered to work on. Correct plan needs a
/// reason and a Registry to check the link and stage against.
#[tokio::test]
async fn a_closed_jobs_file_keeps_its_transaction_for_plan_correction() {
    let Some(url) = test_db_url() else {
        eprintln!("skipped: CADASTRE_TEST_DATABASE_URL not set");
        return;
    };
    std::env::remove_var("CADASTRE_REGISTRY_URL");
    std::env::remove_var("CADASTRE_AUMENTUM_JOBSERVICE_URL");
    let (router, pool) = app(&url).await;
    sqlx::query("INSERT INTO cadastre.wf_task (id, name) VALUES ($1, 'Title deed test task') ON CONFLICT (id) DO NOTHING").bind(TASK).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO cadastre.wf_job_type (id, name, start_task_id, end_task_id) VALUES ($1, 'Title deed test', $2, $2) ON CONFLICT (id) DO NOTHING")
        .bind(TYPE).bind(TASK).execute(&pool).await.unwrap();
    let seed = (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos() % 5_000) as f64;
    let (x, y) = (300_000.0 + seed * 100.0, 9_050_000.0);
    let n = seed as u32;
    let (file, txn) = (format!("PLTDC{n}"), format!("PLS9{n}"));

    // Committed under another job, as an Aumentum parcel is: no job of the parcel to follow.
    let (work_job, cid) = wf_job(&router, &format!("PLTDW{n}")).await;
    let upin = commit_rect(&router, work_job, &cid, x, y).await;
    sqlx::query("UPDATE cadastre.parcel SET job_id = NULL WHERE parcel_number = $1").bind(&upin).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO cadastre.parcel_attribute (object_id, upin, property_number, lrs_id, linkage_status, status) VALUES ($4, $1, $2, $3, 1, '0')")
        .bind(&upin).bind(&file).bind(format!("PLT9{n}")).bind(900_000 + n as i32).execute(&pool).await.unwrap();

    let (job, _) = wf_job(&router, &file).await;
    sqlx::query("UPDATE cadastre.wf_job_instance SET lrs_transaction_number = $2, status = 3, ended = now()::timestamp WHERE id = $1")
        .bind(job as i32).bind(&txn).execute(&pool).await.unwrap();

    let (st, w, _) = send(&router, "GET", &format!("/api/v1/registry/parcels/{upin}/work"), None).await;
    assert_eq!(st, StatusCode::OK, "{w}");
    assert_eq!(w["linked"], json!(true));
    assert_eq!(w["transaction_number"], json!(txn), "the closed job gives the transaction");
    assert_eq!(w["job_instance_id"], Value::Null, "a closed job is not offered as the job to work on");

    // An open job of the file is still the one to work on.
    sqlx::query("UPDATE cadastre.wf_job_instance SET status = 2, ended = NULL WHERE id = $1").bind(job as i32).execute(&pool).await.unwrap();
    let (_, w, _) = send(&router, "GET", &format!("/api/v1/registry/parcels/{upin}/work"), None).await;
    assert_eq!(w["job_instance_id"], json!(job));

    // The reason is required; without a Registry the link and stage cannot be checked, so it is refused.
    let (st, v, _) = send(&router, "POST", "/api/v1/registry/correct-plan", Some(json!({"upin": upin, "reason": "  "}))).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");
    let (st, v, _) = send(&router, "POST", "/api/v1/registry/correct-plan", Some(json!({"upin": upin, "reason": "Wrong scale"}))).await;
    assert_eq!(st, StatusCode::CONFLICT, "{v}");

    // Nothing to warn about while the Aumentum check is off.
    let (st, h, _) = send(&router, "GET", "/api/v1/aumentum/status", None).await;
    assert_eq!((st, h["configured"].clone(), h["ok"].clone()), (StatusCode::OK, json!(false), json!(true)));

    // A correction waits for its plan: it is open until a plan is sent after it.
    let (_, c, _) = send(&router, "GET", &format!("/api/v1/wf/jobs/{job}/registry/plan-correction"), None).await;
    assert_eq!(c["correction"], Value::Null);
    let audit = |action: &'static str, payload: Value| {
        let (pool, job) = (pool.clone(), job);
        async move {
            let mut tx = pool.begin().await.unwrap();
            cadastre_server::audit::append(&mut tx, "test", action, "wf_job_instance", Some(&job.to_string()), payload).await.unwrap();
            tx.commit().await.unwrap();
        }
    };
    audit("title_deed_plan_correction", json!({"reason": "Wrong scale", "created_job": true})).await;
    let (_, c, _) = send(&router, "GET", &format!("/api/v1/wf/jobs/{job}/registry/plan-correction"), None).await;
    assert_eq!(c["correction"]["reason"], json!("Wrong scale"));
    assert_eq!(c["correction"]["created_job"], json!(true));
    audit("registry_plan_sent", json!({"transaction_number": txn})).await;
    let (_, c, _) = send(&router, "GET", &format!("/api/v1/wf/jobs/{job}/registry/plan-correction"), None).await;
    assert_eq!(c["correction"], Value::Null, "the plan sent ends the correction");
}
