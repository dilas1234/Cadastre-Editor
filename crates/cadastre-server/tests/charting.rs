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

/// The automatic checks (C3) on a plot built for the test, far from real parcels: a titled
/// neighbour overlapping it, a neighbour sharing its boundary (a 2 cm sliver), a retired parcel
/// under it, and a beacon on every corner.
#[tokio::test]
async fn the_checks_suggest_answers_with_their_evidence() {
    let Ok(url) = std::env::var("CADASTRE_TEST_DATABASE_URL") else {
        eprintln!("skipped: CADASTRE_TEST_DATABASE_URL not set");
        return;
    };
    let pool = db::connect_and_migrate(&url).await.expect("migrate");
    let n: u32 = rand::random::<u32>() % 900_000 + 100_000;
    let (pn, upin, titled, shared, retired) = (format!("PLZ{n}"), format!("PLZ{n}A"), format!("PLZ{n}B"), format!("PLZ{n}C"), format!("PLZ{n}D"));
    // A square of 20 m, somewhere no real parcel is (UTM 32N, far east of Plateau).
    let (x, y) = (900_000.0 + (n % 1000) as f64 * 100.0, 1_200_000.0);
    let sq = |x0: f64, y0: f64, x1: f64, y1: f64| format!("SRID=32632;POLYGON(({x0} {y0},{x1} {y0},{x1} {y1},{x0} {y1},{x0} {y0}))");
    let job_type: i32 = sqlx::query_scalar("SELECT id FROM cadastre.wf_job_type ORDER BY id LIMIT 1").fetch_one(&pool).await.unwrap();
    let id: i32 = sqlx::query_scalar("INSERT INTO cadastre.wf_job_instance (job_type_id, status, name) VALUES ($1, 3, $2) RETURNING id")
        .bind(job_type).bind(format!("Job-{pn}")).fetch_one(&pool).await.unwrap();
    for (u, status, wkt) in [
        (&upin, "posted", sq(x, y, x + 20.0, y + 20.0)),
        (&titled, "posted", sq(x + 15.0, y, x + 35.0, y + 20.0)),        // 5 m into the plot
        (&shared, "posted", sq(x - 20.0, y, x + 0.02, y + 20.0)),        // the shared west boundary, 2 cm off
        (&retired, "archived", sq(x + 2.0, y + 2.0, x + 8.0, y + 8.0)),  // retired, under the plot
    ] {
        sqlx::query("INSERT INTO cadastre.parcel (parcel_number, geom, status, posted_at) VALUES ($1, ST_GeomFromEWKT($2), $3, now())")
            .bind(u).bind(wkt).bind(status).execute(&pool).await.unwrap();
    }
    let obj: i32 = sqlx::query_scalar("SELECT COALESCE(max(object_id), 0) + 1 FROM cadastre.parcel_attribute").fetch_one(&pool).await.unwrap();
    sqlx::query("INSERT INTO cadastre.parcel_attribute (object_id, upin, property_number, plan_number, layout, status) VALUES ($1, $2, $3, 'PLA/Z1', 'Test Layout', '0'), ($1 + 1, $4, $5, NULL, NULL, '0')")
        .bind(obj).bind(&upin).bind(&pn).bind(&titled).bind(format!("PLZ{n}X")).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO cadastre.registry_property (su_id, upin, refnum) VALUES ($1, $2, $3)")
        .bind(format!("SUZ{n}")).bind(&titled).bind(format!("PLZ{n}X")).execute(&pool).await.unwrap();
    for (i, (bx, by)) in [(x, y), (x + 20.0, y), (x + 20.0, y + 20.0), (x, y + 20.0)].into_iter().enumerate() {
        sqlx::query("INSERT INTO cadastre.beacon (number, geom) VALUES ($1, ST_SetSRID(ST_MakePoint($2, $3), 32632))")
            .bind(format!("ZQ{n}{i}")).bind(bx + 0.01).bind(by).execute(&pool).await.unwrap();
    }

    let config = Config {
        database_url: url, bind_addr: "127.0.0.1:0".into(), auth_mode: AuthMode::Disabled,
        keycloak_issuer: None, keycloak_audience: None, keycloak_jwks_json: None, keycloak_jwks_path: None,
        catalog_folders: Vec::new(), catalog_databases: Vec::new(), registry_sql: None,
    };
    let state = AppState { pool: pool.clone(), config, jwks: None::<Arc<_>> };
    let app = routes::gated(routes::router(state.clone()), state);
    let (st, v) = call(&app, "GET", &format!("/api/v1/charting/files/{id}/checks"), None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let q = |field: &str| v["checks"].as_array().unwrap().iter().find(|c| c["field"] == field).cloned().unwrap_or_else(|| panic!("no {field}: {v}"));

    let one = q("identified");
    assert_eq!(one["value"], "Yes", "{one}");
    assert!(one["evidence"].as_str().unwrap().contains(&format!("filed under {pn}")), "{one}");

    let two = q("ground_open");
    assert_eq!(two["value"], "No", "{two}");
    let detail = two["follow"]["ground_detail"].as_str().unwrap();
    assert!(detail.contains(&titled) && detail.contains("100.00 m²") && !detail.contains(&shared), "only the real overlap: {detail}");
    let ev = two["evidence"].as_str().unwrap();
    assert!(ev.contains("Shared boundaries") && ev.contains(&shared), "the 2 cm sliver is named, not counted: {ev}");
    assert!(ev.contains(&format!("{retired} (retired")), "the retired parcel is named: {ev}");

    let three = q("beaconed");
    assert_eq!((three["value"].as_str(), three["follow"]["tracing_no"].as_str(), three["follow"]["deposited_plan_no"].as_str()), (Some("Yes"), Some("ZQ"), Some("PLA/Z1")), "{three}");

    let five = q("under_title");
    assert_eq!((five["value"].as_str(), five["follow"]["title_no"].as_str()), (Some("Yes"), Some(format!("PLZ{n}X").as_str())), "{five}");

    for f in ["town_plan", "rail", "major_road", "settlement"] {
        assert_eq!(q(f)["result"], "no_layer", "{f}");
    }
    assert!(q("town_plan")["evidence"].as_str().unwrap().contains("Test Layout"), "the parcel's layout is named");

    // A corner without a beacon: not completely beaconed.
    sqlx::query("DELETE FROM cadastre.beacon WHERE number = $1").bind(format!("ZQ{n}2")).execute(&pool).await.unwrap();
    let (_, v) = call(&app, "GET", &format!("/api/v1/charting/files/{id}/checks"), None).await;
    let three = v["checks"].as_array().unwrap().iter().find(|c| c["field"] == "beaconed").unwrap();
    assert_eq!(three["value"], "No", "{three}");
    assert!(three["evidence"].as_str().unwrap().contains("1 of 4 corners"), "{three}");

    // Clean up.
    sqlx::query("DELETE FROM cadastre.beacon WHERE number LIKE $1").bind(format!("ZQ{n}%")).execute(&pool).await.unwrap();
    sqlx::query("DELETE FROM cadastre.registry_property WHERE su_id = $1").bind(format!("SUZ{n}")).execute(&pool).await.unwrap();
    sqlx::query("DELETE FROM cadastre.parcel_attribute WHERE object_id IN ($1, $1 + 1)").bind(obj).execute(&pool).await.unwrap();
    sqlx::query("DELETE FROM cadastre.parcel WHERE parcel_number LIKE $1").bind(format!("PLZ{n}%")).execute(&pool).await.unwrap();
    sqlx::query("DELETE FROM cadastre.wf_job_instance WHERE id = $1").bind(id).execute(&pool).await.unwrap();
}
