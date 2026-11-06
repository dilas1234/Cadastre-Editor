//! A beacon number another beacon already has: on an adjoining property the corner takes its
//! position; elsewhere the digitizer sees the property and needs a reason. And correcting a
//! committed number, which leaves the parcel alone.

use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{header, Request, StatusCode};
use axum::Router;
use serde_json::{json, Value};
use tower::ServiceExt;

use cadastre_server::config::{AuthMode, Config};
use cadastre_server::routes::wfbeacons::{apply, WfBeaconNumber, WfCommitRequest};
use cadastre_server::state::AppState;
use cadastre_server::{db, routes};

const TYPE: i32 = 9_300;

async fn send(router: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let b = Request::builder().method(method).uri(uri);
    let req = match body {
        Some(v) => b.header(header::CONTENT_TYPE, "application/json").body(Body::from(v.to_string())).unwrap(),
        None => b.body(Body::empty()).unwrap(),
    };
    let r = router.clone().oneshot(req).await.unwrap();
    let st = r.status();
    let bytes = to_bytes(r.into_body(), usize::MAX).await.unwrap();
    (st, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn job(app: &Router, file: &str, x: f64, y: f64) -> (i64, String) {
    let (st, v) = send(app, "POST", "/api/v1/wf/jobs", Some(json!({"job_type_id": TYPE, "name": format!("Job-{file}")}))).await;
    assert!(st.is_success(), "{st} {v}");
    let id = v.get("job").unwrap_or(&v)["id"].as_i64().unwrap();
    let (st, e) = send(app, "POST", &format!("/api/v1/wf/jobs/{id}/editing"), Some(json!({}))).await;
    assert!(st.is_success(), "{st} {e}");
    let area = json!({"geometry": {"type":"Polygon","coordinates":[[[x-5.0,y-5.0],[x+60.0,y-5.0],[x+60.0,y+25.0],[x-5.0,y+25.0],[x-5.0,y-5.0]]]}});
    let (st, a) = send(app, "PUT", &format!("/api/v1/wf/jobs/{id}/area"), Some(area)).await;
    assert!(st.is_success(), "{st} {a}");
    (id, e["cadastre_job_id"].as_str().unwrap().to_string())
}

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Value {
    json!({"geometry": {"type":"Polygon","coordinates":[[[x0,y0],[x1,y0],[x1,y1],[x0,y1],[x0,y0]]]}})
}

#[tokio::test]
async fn beacon_numbers_on_other_properties() {
    let Ok(url) = std::env::var("CADASTRE_TEST_DATABASE_URL") else {
        eprintln!("skipped: CADASTRE_TEST_DATABASE_URL not set");
        return;
    };
    std::env::remove_var("CADASTRE_REGISTRY_URL");
    std::env::remove_var("CADASTRE_AUMENTUM_JOBSERVICE_URL");
    let pool = db::connect_and_migrate(&url).await.expect("migrate");
    let config = Config {
        database_url: url.clone(), bind_addr: "127.0.0.1:0".into(), auth_mode: AuthMode::Disabled,
        keycloak_issuer: None, keycloak_audience: None, keycloak_jwks_json: None, keycloak_jwks_path: None,
        catalog_folders: Vec::new(), catalog_databases: Vec::new(),
    };
    let app = routes::router(AppState { pool: pool.clone(), config, jwks: None::<Arc<_>> });
    sqlx::query("INSERT INTO cadastre.wf_task (id, name) VALUES ($1, 'Beacon test task') ON CONFLICT (id) DO NOTHING").bind(TYPE).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO cadastre.wf_job_type (id, name, start_task_id, end_task_id) VALUES ($1, 'Beacon test', $1, $1) ON CONFLICT (id) DO NOTHING").bind(TYPE).execute(&pool).await.unwrap();
    let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() % 1_000_000;
    let (x, y) = (400_000.0 + (n % 5_000) as f64 * 100.0, 9_100_000.0);
    let (near, far, file) = (format!("NB{n}"), format!("FB{n}"), format!("PLB{n}"));

    // The neighbour: committed, with a beacon at its north-east corner and a file number.
    let (_, cid) = job(&app, "NEIGHBOUR", x, y).await;
    let (st, p) = send(&app, "POST", &format!("/api/v1/jobs/{cid}/parcels"), Some(rect(x, y, x + 20.0, y + 10.0)["geometry"].clone()).map(|g| json!({"geometry": g}))).await;
    assert_eq!(st, StatusCode::CREATED, "{p}");
    for action in ["submit", "start_review", "approve"] {
        let (st, j) = send(&app, "POST", &format!("/api/v1/jobs/{cid}/transition"), Some(json!({"action": action}))).await;
        assert_eq!(st, StatusCode::OK, "{action}: {j}");
    }
    let (st, r) = send(&app, "POST", &format!("/api/v1/jobs/{cid}/post"), None).await;
    assert_eq!(st, StatusCode::OK, "{r}");
    let upin = r["parcel_numbers"][0].as_str().unwrap().to_string();
    sqlx::query("INSERT INTO cadastre.parcel_attribute (object_id, upin, property_number, status) VALUES ($1, $2, $3, '0')")
        .bind(800_000 + (n % 90_000) as i32).bind(&upin).bind(&file).execute(&pool).await.unwrap();
    let add = |number: String, e: f64, nn: f64| { let pool = pool.clone(); async move {
        sqlx::query("INSERT INTO cadastre.beacon (number, geom, accuracy, is_control) VALUES ($1, ST_SetSRID(ST_MakePoint($2, $3), 32632), 0.05, false)")
            .bind(number).bind(e).bind(nn).execute(&pool).await.unwrap() } };
    add(near.clone(), x + 20.0, y + 10.0).await;
    add(far.clone(), x + 20.0, y).await; // on the neighbour too, but 40 m from where it will be typed

    // The new parcel, 30 cm off the neighbour's east side.
    let (id, cid) = job(&app, "NEW", x, y).await;
    let (ax, by) = (x + 20.3, y + 10.0);
    let (st, p) = send(&app, "POST", &format!("/api/v1/jobs/{cid}/parcels"), Some(json!({"geometry": rect(ax, y, x + 50.0, by)["geometry"]}))).await;
    assert_eq!(st, StatusCode::CREATED, "{p}");
    let job_id: uuid::Uuid = cid.parse().unwrap();
    let area_before: f64 = sqlx::query_scalar("SELECT ST_Area(geom) FROM cadastre.parcel WHERE job_id = $1").bind(job_id).fetch_one(&pool).await.unwrap();
    let number = |e: f64, nn: f64, s: &str, reason: Option<&str>| WfBeaconNumber { easting: e, northing: nn, number: s.to_string(), reason: reason.map(str::to_string) };
    let beacons = |reason: Option<&str>| vec![
        number(ax, by, &near, None),
        number(x + 50.0, by, &far, reason),
        number(x + 50.0, y, &format!("A{n}"), None),
        number(ax, y, &format!("B{n}"), None),
    ];

    // The check shows both, with the property.
    let body = json!({"beacons": beacons(None).iter().map(|b| json!({"easting": b.easting, "northing": b.northing, "number": b.number})).collect::<Vec<_>>()});
    let (st, c) = send(&app, "POST", &format!("/api/v1/wf/jobs/{id}/editing/beacon-check"), Some(body)).await;
    assert_eq!(st, StatusCode::OK, "{c}");
    let found = c.as_array().unwrap();
    assert_eq!(found.len(), 2, "{c}");
    let of = |num: &str| found.iter().find(|x| x["number"] == json!(num)).unwrap();
    assert_eq!(of(&near)["kind"], json!("adjoining"));
    assert!((of(&near)["distance_m"].as_f64().unwrap() - 0.3).abs() < 0.001);
    assert_eq!(of(&near)["properties"][0]["file_number"], json!(file));
    assert_eq!(of(&near)["properties"][0]["upin"], json!(upin));
    assert_eq!(of(&far)["kind"], json!("elsewhere"), "too far to be the same beacon, though the parcels adjoin");
    assert_eq!(of(&far)["properties"][0]["file_number"], json!(file));

    // No reason: refused, naming the file. Nothing is kept.
    let mut tx = pool.begin().await.unwrap();
    let e = apply(&mut tx, job_id, &WfCommitRequest { beacons: beacons(None), staff_name: None, staff_sid: None }).await.unwrap_err().to_string();
    assert!(e.contains(&format!("file {file}")) && e.contains("give a reason"), "{e}");
    tx.rollback().await.unwrap();

    // With a reason: the adjoining corner moves onto the beacon, the other number repeats.
    let mut tx = pool.begin().await.unwrap();
    let done = apply(&mut tx, job_id, &WfCommitRequest { beacons: beacons(Some("Plan shows this number twice")), staff_name: None, staff_sid: None }).await.unwrap();
    tx.commit().await.unwrap();
    assert_eq!(done.notes.len(), 2, "{:?}", done.notes);
    assert!(!done.added.contains(&near), "the adjoining beacon is not added again");
    let same: i64 = sqlx::query_scalar("SELECT count(*) FROM cadastre.beacon WHERE number = $1").bind(&near).fetch_one(&pool).await.unwrap();
    assert_eq!(same, 1, "one beacon, shared");
    let repeats: Vec<i32> = sqlx::query_scalar("SELECT duplicate_seq FROM cadastre.beacon WHERE number = $1 ORDER BY 1").bind(&far).fetch_all(&pool).await.unwrap();
    assert_eq!(repeats, vec![0, 1], "the number repeats");
    let on_it: bool = sqlx::query_scalar("SELECT ST_DWithin(p.geom, ST_SetSRID(ST_MakePoint($2, $3), ST_SRID(p.geom)), 0.0001) FROM cadastre.parcel p WHERE p.job_id = $1")
        .bind(job_id).bind(x + 20.0).bind(by).fetch_one(&pool).await.unwrap();
    assert!(on_it, "the corner is on the beacon");
    let (area, valid, stored): (f64, bool, Option<f64>) = sqlx::query_as("SELECT ST_Area(geom), ST_IsValid(geom), computed_area FROM cadastre.parcel WHERE job_id = $1").bind(job_id).fetch_one(&pool).await.unwrap();
    assert!(valid && area > area_before, "{area} {area_before}");
    assert!((stored.unwrap() - area).abs() < 1e-6, "the recorded area follows the shape");

    // A committed number corrected: a reason, then the holder is named, then it is confirmed.
    let change = |to: &str, reason: &str, confirm: bool| json!({"upin": upin, "easting": x + 20.0, "northing": y + 10.0, "number": to, "reason": reason, "confirm_repeat": confirm});
    let (st, _) = send(&app, "POST", "/api/v1/parcels/beacon-number", Some(change("Z1", " ", false))).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let geom_before: String = sqlx::query_scalar("SELECT ST_AsText(geom) FROM cadastre.parcel WHERE parcel_number = $1").bind(&upin).fetch_one(&pool).await.unwrap();
    let fixed = format!("FX{n}");
    let (st, r) = send(&app, "POST", "/api/v1/parcels/beacon-number", Some(change(&fixed, "Typed wrong", false))).await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert_eq!(r["from"], json!(near));
    assert_eq!(sqlx::query_scalar::<_, String>("SELECT ST_AsText(geom) FROM cadastre.parcel WHERE parcel_number = $1").bind(&upin).fetch_one(&pool).await.unwrap(), geom_before, "the parcel does not change");
    let (st, r) = send(&app, "POST", "/api/v1/parcels/beacon-number", Some(change(&far, "Typed wrong", false))).await;
    assert_eq!(st, StatusCode::CONFLICT, "{r}");
    assert!(r["message"].as_str().unwrap().contains(&format!("file {file}")), "{r}");
    let (st, r) = send(&app, "POST", "/api/v1/parcels/beacon-number", Some(change(&far, "Plan says so", true))).await;
    assert_eq!(st, StatusCode::OK, "{r}");
    let (st, r) = send(&app, "POST", "/api/v1/parcels/beacon-number", Some(json!({"upin": upin, "easting": x + 7.0, "northing": y + 3.0, "number": "Q", "reason": "x"}))).await;
    assert_eq!(st, StatusCode::NOT_FOUND, "not a corner: {r}");

    // Digitizing stage only. A beacon shared with a linked neighbour changes nothing...
    let new_upin = format!("PLN{n}");
    sqlx::query("UPDATE cadastre.parcel SET status = 'posted', parcel_number = $2 WHERE job_id = $1").bind(job_id).bind(&new_upin).execute(&pool).await.unwrap();
    sqlx::query("UPDATE cadastre.parcel_attribute SET linkage_status = 1, lrs_id = 'PLT1' WHERE upin = $1").bind(&upin).execute(&pool).await.unwrap();
    let shared = json!({"upin": new_upin, "easting": x + 20.0, "northing": y + 10.0, "number": "S1", "reason": "x"});
    let (st, r) = send(&app, "POST", "/api/v1/parcels/beacon-number", Some(shared)).await;
    assert_eq!(st, StatusCode::CONFLICT, "{r}");
    assert!(r["message"].as_str().unwrap().contains(&upin) && r["message"].as_str().unwrap().contains("unlink it there first"), "{r}");
    // ...and neither does the linked parcel itself; the Point List says so.
    let (st, r) = send(&app, "POST", "/api/v1/parcels/beacon-number", Some(change("L1", "x", true))).await;
    assert_eq!(st, StatusCode::CONFLICT, "even when confirmed: {r}");
    assert!(r["message"].as_str().unwrap().contains("unlink it there first"), "{r}");
    let (_, pl) = send(&app, "GET", &format!("/api/v1/parcels/points?upin={upin}"), None).await;
    assert_eq!(pl["linked"], json!(true));
    // A parcel the Registry alone holds counts as linked as well.
    sqlx::query("UPDATE cadastre.parcel_attribute SET linkage_status = 0, lrs_id = NULL WHERE upin = $1").bind(&upin).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO cadastre.registry_property (su_id, upin) VALUES ($1, $2) ON CONFLICT (su_id) DO UPDATE SET upin = EXCLUDED.upin").bind(format!("SU{n}")).bind(&upin).execute(&pool).await.unwrap();
    let (st, r) = send(&app, "POST", "/api/v1/parcels/beacon-number", Some(change("L2", "x", true))).await;
    assert_eq!(st, StatusCode::CONFLICT, "{r}");

    let (_, v) = send(&app, "GET", "/api/v1/audit/verify", None).await;
    assert_eq!(v["ok"], json!(true), "{v}");
}
