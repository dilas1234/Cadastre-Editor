//! Correct only touches a file's current UPIN. Aumentum archived a wrong parcel and digitised it
//! again under a new UPIN, so a file can hold several; an old one never comes back while the file
//! has a current one, and duplicates on the same land wait for the one the Registry holds.

use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{header, Request, StatusCode};
use axum::Router;
use serde_json::{json, Value};
use tower::ServiceExt;

use cadastre_server::config::{AuthMode, Config};
use cadastre_server::state::AppState;
use cadastre_server::{db, routes};

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

#[tokio::test]
async fn only_the_files_current_upin_is_corrected() {
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
        catalog_folders: Vec::new(), catalog_databases: Vec::new(), registry_sql: None,
    };
    let app = routes::router(AppState { pool: pool.clone(), config, jwks: None::<Arc<_>> });
    let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() % 1_000_000;
    let (x0, y0) = (300_000.0 + (n % 5_000) as f64 * 200.0, 9_300_000.0);
    let mut object_id = 700_000_000 + (n as i32 % 1_000_000) * 10;

    // A parcel (a 20 m square at x) and its Aumentum attribute record for `file`, made on `day`.
    let parcel = |upin: String, file: String, x: f64, status: &'static str, day: &'static str, oid: i32| {
        let pool = pool.clone();
        async move {
            sqlx::query("INSERT INTO cadastre.parcel (parcel_number, geom, status) VALUES ($1, ST_SetSRID(ST_MakeEnvelope($2, $3, $2 + 20, $3 + 20), 32632), $4)")
                .bind(&upin).bind(x).bind(9_300_000.0).bind(status).execute(&pool).await.unwrap();
            sqlx::query("INSERT INTO cadastre.parcel_attribute (object_id, upin, property_number, status, creation_date) VALUES ($1, $2, $3, '0', $4::timestamp)")
                .bind(oid).bind(&upin).bind(&file).bind(day).execute(&pool).await.unwrap();
        }
    };
    let held = |upin: String, su: String| {
        let pool = pool.clone();
        async move { sqlx::query("INSERT INTO cadastre.registry_property (su_id, upin, refnum) VALUES ($1, $2, 'x')").bind(su).bind(upin).execute(&pool).await.unwrap(); }
    };
    let why = |upin: String| {
        let app = app.clone();
        async move {
            let (st, d) = send(&app, "GET", &format!("/api/v1/parcels/points?upin={upin}"), None).await;
            assert_eq!(st, StatusCode::OK, "{d}");
            (d["can_correct"].as_bool().unwrap(), d["why_not"].as_str().unwrap_or("").to_string())
        }
    };
    let u = |tag: &str| format!("PLT{tag}{n:06}");
    let _ = y0;

    // 1. An old archived UPIN beside the live one (PL54737): only the live one.
    let (old, cur, file) = (u("A1"), u("A2"), format!("PLCA{n}"));
    object_id += 1; parcel(old.clone(), file.clone(), x0, "archived", "2021-12-07", object_id).await;
    object_id += 1; parcel(cur.clone(), file.clone(), x0, "posted", "2022-09-14", object_id).await;
    let (ok, w) = why(old.clone()).await;
    assert!(!ok && w.contains("old UPIN") && w.contains(&cur), "the old UPIN is refused and the current one named: {w}");
    assert_eq!(why(cur.clone()).await, (true, String::new()), "the live UPIN is the current one");
    let (st, r) = send(&app, "POST", "/api/v1/wf/correct", Some(json!({"upin": old, "reason": "bring back"}))).await;
    assert_eq!(st, StatusCode::CONFLICT, "{r}");
    assert!(r["message"].as_str().unwrap().contains(&cur), "{r}");
    let after: i64 = sqlx::query_scalar("SELECT count(*) FROM cadastre.parcel WHERE parcel_number = $1 AND status = 'draft'").bind(&old).fetch_one(&pool).await.unwrap();
    assert_eq!(after, 0, "nothing was opened for the old UPIN");

    // 2. Nothing live, the Registry holds none: the most recent UPIN.
    let (b1, b2, file) = (u("B1"), u("B2"), format!("PLCB{n}"));
    object_id += 1; parcel(b1.clone(), file.clone(), x0 + 40.0, "archived", "2020-01-01", object_id).await;
    object_id += 1; parcel(b2.clone(), file.clone(), x0 + 40.0, "archived", "2023-01-01", object_id).await;
    assert!(why(b1.clone()).await.1.contains(&format!("most recent UPIN is {b2}")), "the older archived UPIN is refused");
    assert_eq!(why(b2.clone()).await, (true, String::new()), "the most recent archived UPIN can come back");
    // … and when the Registry holds one of them, that one.
    held(b1.clone(), format!("SUB{n}")).await;
    assert_eq!(why(b1.clone()).await, (true, String::new()), "the UPIN the Registry holds is current");
    assert!(why(b2.clone()).await.1.contains(&format!("the Registry holds {b1}")), "the other is refused");

    // 3. Two live UPINs on the same land: none until the duplicate is retired; then the Registry's.
    let (c1, c2, file) = (u("C1"), u("C2"), format!("PLCC{n}"));
    object_id += 1; parcel(c1.clone(), file.clone(), x0 + 80.0, "posted", "2022-01-01", object_id).await;
    object_id += 1; parcel(c2.clone(), file.clone(), x0 + 81.0, "posted", "2023-01-01", object_id).await;
    assert!(why(c1.clone()).await.1.contains("retire the duplicate first"), "duplicates, none held: refused");
    assert!(why(c2.clone()).await.1.contains("retire the duplicate first"));
    held(c2.clone(), format!("SUC{n}")).await;
    assert_eq!(why(c2.clone()).await, (true, String::new()), "the duplicate the Registry holds is current");
    assert!(why(c1.clone()).await.1.contains(&format!("correct {c2} instead")), "the other duplicate is refused");

    // 4. Two live UPINs on different land: a file of two plots, both current.
    let (d1, d2, file) = (u("D1"), u("D2"), format!("PLCD{n}"));
    object_id += 1; parcel(d1.clone(), file.clone(), x0 + 120.0, "posted", "2022-01-01", object_id).await;
    object_id += 1; parcel(d2.clone(), file.clone(), x0 + 160.0, "posted", "2023-01-01", object_id).await;
    assert_eq!(why(d1.clone()).await, (true, String::new()));
    assert_eq!(why(d2.clone()).await, (true, String::new()));

    // 5. A file with one UPIN, archived (PL63757): it comes back.
    let (e1, file) = (u("E1"), format!("PLCE{n}"));
    object_id += 1; parcel(e1.clone(), file.clone(), x0 + 200.0, "archived", "2024-12-05", object_id).await;
    assert_eq!(why(e1.clone()).await, (true, String::new()));
}
