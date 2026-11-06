//! This Cadastre's own job and task numbers (migration 0019): jobs and task instances created
//! here are numbered above 1,000,000, and Aumentum's ids (loaded below that) never move the
//! counters, so a reload of Aumentum's workflow cannot meet a number used here.

use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{header, Request, StatusCode};
use axum::Router;
use serde_json::{json, Value};
use tower::ServiceExt;

use cadastre_server::config::{AuthMode, Config};
use cadastre_server::state::AppState;
use cadastre_server::{db, routes};

const BASE: i64 = 1_000_000;
const TYPE: i32 = 9_100; // a job type of this test only
const TASK: i32 = 9_100;

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
        catalog_databases: Vec::new(), registry_sql: None,
    };
    (routes::router(AppState { pool: pool.clone(), config, jwks: None::<Arc<_>> }), pool)
}

async fn send(router: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let b = Request::builder().method(method).uri(uri);
    let req = match body {
        Some(v) => b.header(header::CONTENT_TYPE, "application/json").body(Body::from(v.to_string())).unwrap(),
        None => b.body(Body::empty()).unwrap(),
    };
    let resp = router.clone().oneshot(req).await.unwrap();
    let st = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (st, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

/// Create a job of the test type; returns (job id, its first task instance id).
async fn new_job(router: &Router, name: &str) -> (i64, i64) {
    let (st, v) = send(router, "POST", "/api/v1/wf/jobs", Some(json!({"job_type_id": TYPE, "name": name}))).await;
    assert!(st.is_success(), "create job: {st} {v}");
    let job = v.get("job").unwrap_or(&v);
    let id = job["id"].as_i64().expect("job id");
    let (st, d) = send(router, "GET", &format!("/api/v1/wf/jobs/{id}"), None).await;
    assert_eq!(st, StatusCode::OK, "{d}");
    let task = d["history"][0]["id"].as_i64().expect("first task instance");
    (id, task)
}

#[tokio::test]
async fn jobs_created_here_use_their_own_range_and_aumentum_ids_never_move_it() {
    let Some(url) = test_db_url() else {
        eprintln!("skipped: CADASTRE_TEST_DATABASE_URL not set");
        return;
    };
    let (router, pool) = app(&url).await;
    sqlx::query("INSERT INTO cadastre.wf_task (id, name) VALUES ($1, 'Numbering test task') ON CONFLICT (id) DO NOTHING")
        .bind(TASK).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO cadastre.wf_job_type (id, name, start_task_id, end_task_id) VALUES ($1, 'Numbering test', $2, $2) ON CONFLICT (id) DO NOTHING")
        .bind(TYPE).bind(TASK).execute(&pool).await.unwrap();

    let (job1, task1) = new_job(&router, "Job-NUMTEST-1").await;
    assert!(job1 > BASE, "a job created here is numbered above {BASE}, got {job1}");
    assert!(task1 > BASE, "its task instance too, got {task1}");

    // Aumentum's jobs arrive below the base (a reload during the parallel run) and the loader
    // re-syncs the counters: they must stay in this Cadastre's range and keep counting up.
    let aum: i32 = 990_000 - (job1 % 1000) as i32;
    sqlx::query("INSERT INTO cadastre.wf_job_instance (id, job_type_id, current_task_id, status, name, source) VALUES ($1, $2, $3, 2, 'Job-AUMENTUM', 'aumentum') ON CONFLICT (id) DO NOTHING")
        .bind(aum).bind(TYPE).bind(TASK).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO cadastre.wf_task_instance (id, job_instance_id, job_type_id, task_id) VALUES ($1, $1, $2, $3) ON CONFLICT (id) DO NOTHING")
        .bind(aum).bind(TYPE).bind(TASK).execute(&pool).await.unwrap();
    sqlx::query("SELECT cadastre.wf_sync_sequences()").execute(&pool).await.unwrap();

    let (job2, task2) = new_job(&router, "Job-NUMTEST-2").await;
    assert!(job2 > job1, "the job counter keeps counting up in its range: {job1} then {job2}");
    assert!(task2 > task1, "the task counter too: {task1} then {task2}");

    // The Aumentum rows are untouched.
    let name: String = sqlx::query_scalar("SELECT name FROM cadastre.wf_job_instance WHERE id = $1").bind(aum).fetch_one(&pool).await.unwrap();
    assert_eq!(name, "Job-AUMENTUM");

    sqlx::query("DELETE FROM cadastre.wf_job_instance WHERE id = $1").bind(aum).execute(&pool).await.unwrap();
}
