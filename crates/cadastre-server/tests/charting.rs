//! The Survey Report: saved with versions, the signed part locked, and signed by two officers.

use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{header, Request, StatusCode};
use axum::Router;
use serde_json::{json, Value};
use tower::ServiceExt;

use cadastre_server::config::{AuthMode, Config};
use cadastre_server::state::AppState;
use cadastre_server::{db, routes};

async fn call(app: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let b = Request::builder().method(method).uri(uri);
    let req = match body {
        Some(v) => b.header(header::CONTENT_TYPE, "application/json").body(Body::from(v.to_string())).unwrap(),
        None => b.body(Body::empty()).unwrap(),
    };
    let r = app.clone().oneshot(req).await.unwrap();
    let st = r.status();
    let bytes = to_bytes(r.into_body(), usize::MAX).await.unwrap();
    (st, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

#[tokio::test]
async fn a_report_is_saved_locked_and_signed_by_two_officers() {
    let Ok(url) = std::env::var("CADASTRE_TEST_DATABASE_URL") else {
        eprintln!("skipped: CADASTRE_TEST_DATABASE_URL not set");
        return;
    };
    let pool = db::connect_and_migrate(&url).await.expect("migrate");
    let id: Option<i32> = sqlx::query_scalar("SELECT j.id FROM cadastre.wf_job_instance j WHERE j.status = 3 AND j.id NOT IN (SELECT wf_job_instance_id FROM cadastre.charting_report) \
         AND EXISTS (SELECT 1 FROM cadastre.parcel p WHERE p.job_id = j.cadastre_job_id AND p.status = 'posted' AND p.parcel_number IS NOT NULL) LIMIT 1")
        .fetch_optional(&pool).await.unwrap();
    let id = match id {
        Some(i) => i,
        None => { eprintln!("skipped: no completed job to chart"); return; }
    };
    let config = Config {
        database_url: url, bind_addr: "127.0.0.1:0".into(), auth_mode: AuthMode::Disabled,
        keycloak_issuer: None, keycloak_audience: None, keycloak_jwks_json: None, keycloak_jwks_path: None,
        catalog_folders: Vec::new(), catalog_databases: Vec::new(), registry_sql: None,
    };
    let state = AppState { pool: pool.clone(), config, jwks: None::<Arc<_>> };
    let app = routes::gated(routes::router(state.clone()), state);
    let report = format!("/api/v1/charting/files/{id}/report");
    let sign = format!("/api/v1/charting/files/{id}/sign");

    assert_eq!(call(&app, "POST", &sign, Some(json!({"part": "charting"}))).await.0, StatusCode::CONFLICT, "nothing saved yet");
    let (st, v) = call(&app, "PUT", &report, Some(json!({"answers": {"charting": {"identified": "Yes"}}}))).await;
    assert_eq!((st, &v["version"]), (StatusCode::OK, &json!(1)), "{v}");
    let (st, v) = call(&app, "PUT", &report, Some(json!({"answers": {"charting": {"identified": "No"}}, "version": 1}))).await;
    assert_eq!((st, &v["version"]), (StatusCode::OK, &json!(2)), "{v}");
    assert_eq!(call(&app, "PUT", &report, Some(json!({"answers": {"charting": {}}, "version": 1}))).await.0, StatusCode::CONFLICT, "an old copy cannot overwrite");

    assert_eq!(call(&app, "POST", &sign, Some(json!({"part": "final"}))).await.0, StatusCode::CONFLICT, "the charting part comes first");
    let (st, v) = call(&app, "POST", &sign, Some(json!({"part": "charting"}))).await;
    assert_eq!(st, StatusCode::CONFLICT, "a blank report is not signed");
    assert!(v["message"].as_str().unwrap().contains("Question 2"), "{v}");

    let charting = json!({ "officer_name": "A Officer", "rank": "C.T.O. Carto", "ref_no": "PL1", "property_no": "PL1", "applicant": "B", "area": "237.419", "location": "Jos",
        "identified": "No", "ground_open": "Yes", "beaconed": "No", "town_plan": "No", "under_title": "No", "rail": "No", "major_road": "No", "settlement": "No" });
    assert_eq!(call(&app, "PUT", &report, Some(json!({"answers": {"charting": charting}}))).await.0, StatusCode::OK);
    let (_, list) = call(&app, "GET", "/api/v1/charting/files?stage=to_report", None).await;
    assert!(list["files"].as_array().unwrap().iter().any(|f| f["id"] == json!(id)), "a started report is to report");
    assert_eq!(call(&app, "POST", &sign, Some(json!({"part": "charting"}))).await.0, StatusCode::OK);
    let (st, v) = call(&app, "POST", &sign, Some(json!({"part": "final"}))).await;
    assert_eq!(st, StatusCode::CONFLICT);
    assert!(v["message"].as_str().unwrap().contains("second officer"), "told before filling the part in: {v}");

    // The signed part stays; the final part can be filled.
    let last = json!({ "officer_name": "S Officer", "rank": "C.T.O. Carto", "ground_opened": "Yes", "satisfactory": "Yes", "description": "Plot", "check_survey": "No", "recommended": "No" });
    let (st, _) = call(&app, "PUT", &report, Some(json!({"answers": {"charting": {"identified": "Yes"}, "final": last}}))).await;
    assert_eq!(st, StatusCode::OK);
    let saved: Value = sqlx::query_scalar("SELECT answers FROM cadastre.charting_report WHERE wf_job_instance_id = $1").bind(id).fetch_one(&pool).await.unwrap();
    assert_eq!(saved["charting"]["identified"], json!("No"));
    assert_eq!(saved["final"]["ground_opened"], json!("Yes"));
    assert_eq!(call(&app, "POST", &sign, Some(json!({"part": "final"}))).await.0, StatusCode::CONFLICT, "the same officer cannot sign both");

    sqlx::query("UPDATE cadastre.charting_report SET charted_by = 'first.officer' WHERE wf_job_instance_id = $1").bind(id).execute(&pool).await.unwrap();
    assert_eq!(call(&app, "POST", &sign, Some(json!({"part": "final"}))).await.0, StatusCode::OK, "a second officer signs");
    assert_eq!(call(&app, "PUT", &report, Some(json!({"answers": {"final": {}}}))).await.0, StatusCode::CONFLICT, "final is final");

    let versions: i64 = sqlx::query_scalar("SELECT count(*) FROM cadastre.charting_report_version WHERE wf_job_instance_id = $1").bind(id).fetch_one(&pool).await.unwrap();
    assert_eq!(versions, 6, "every save and signature is kept");
    let (_, list) = call(&app, "GET", "/api/v1/charting/files?stage=done", None).await;
    assert!(list["files"].as_array().unwrap().iter().any(|f| f["id"] == json!(id)));
    assert_eq!(call(&app, "GET", "/api/v1/charting/files/2147483647", None).await.0, StatusCode::NOT_FOUND);
    let (st, history) = call(&app, "GET", &format!("/api/v1/registry/history?job={id}"), None).await;
    assert_eq!(st, StatusCode::OK);
    let text = history.to_string();
    assert!(text.contains("charting_report_saved") && text.contains("charting_report_signed"), "the file's history shows the report: {text}");

    sqlx::query("DELETE FROM cadastre.charting_report_version WHERE wf_job_instance_id = $1").bind(id).execute(&pool).await.unwrap();
    sqlx::query("DELETE FROM cadastre.charting_report WHERE wf_job_instance_id = $1").bind(id).execute(&pool).await.unwrap();
}
