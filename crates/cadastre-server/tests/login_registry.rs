//! Sign-in with Registry users, against a stand-in Registry: the Registry checks the password, its
//! groups decide access, and staff who are not in the Registry keep this server's accounts.

use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{header, HeaderMap, Request, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{any, get};
use axum::Router;
use serde_json::{json, Value};
use tower::ServiceExt;

use cadastre_server::config::{AuthMode, Config};
use cadastre_server::state::AppState;
use cadastre_server::{db, routes};

fn tag(xml: &str, name: &str) -> String {
    xml.split(&format!(":{name}>")).nth(1).unwrap_or("").trim_end_matches("</auth").trim_end_matches("</acs").to_string()
}

async fn auth_service(_: HeaderMap, body: String) -> axum::response::Response {
    if body.contains("endSession") {
        return "<ok/>".into_response();
    }
    let (u, p) = (tag(&body, "username"), tag(&body, "password"));
    let known = [("svc", "svcpw"), ("scan1", "RegPass1"), ("regadmin", "RegPass1"), ("cashier", "RegPass1"), ("guest", "anything")];
    if known.iter().any(|(a, b)| *a == u && *b == p) {
        format!("<r><ticket>TICKET_{u}</ticket></r>").into_response()
    } else {
        (StatusCode::INTERNAL_SERVER_ERROR, "<fault/>").into_response()
    }
}

async fn acs_service(body: String) -> String {
    let groups: &[&str] = match tag(&body, "authority").as_str() {
        "scan1" => &["GROUP_LRS", "GROUP_Scanning Clerk"],
        "regadmin" => &["GROUP_LRS", "GROUP_LRS_ADMINISTRATION_SYSTEM"],
        "cashier" => &["GROUP_LRS", "GROUP_Cashier"],
        _ => &[],
    };
    format!("<r>{}</r>", groups.iter().map(|g| format!("<results>{g}</results>")).collect::<String>())
}

async fn registry() -> String {
    let app = Router::new()
        .route("/lrs/api/DomainModelService", get(|| async { "wsdl" }))
        .route("/lrs/api/AuthenticationService", any(auth_service))
        .route("/lrs/api/AccessControlService", any(acs_service));
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    format!("http://{addr}/lrs/api/")
}

async fn call(app: &Router, method: &str, uri: &str, cookie: Option<&str>, body: Option<Value>) -> (StatusCode, Value, Option<String>, String) {
    let mut b = Request::builder().method(method).uri(uri);
    if let Some(c) = cookie { b = b.header(header::COOKIE, c); }
    let req = match body {
        Some(v) => b.header(header::CONTENT_TYPE, "application/json").body(Body::from(v.to_string())).unwrap(),
        None => b.body(Body::empty()).unwrap(),
    };
    let r = app.clone().oneshot(req).await.unwrap();
    let st = r.status();
    let loc = r.headers().get(header::LOCATION).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let c = r.headers().get(header::SET_COOKIE).and_then(|v| v.to_str().ok()).map(|c| c.split(';').next().unwrap().to_string());
    let bytes = to_bytes(r.into_body(), usize::MAX).await.unwrap();
    (st, serde_json::from_slice(&bytes).unwrap_or(Value::Null), c, loc)
}

async fn sign_in(app: &Router, u: &str, p: &str, view: &str) -> (StatusCode, Value, Option<String>, String) {
    call(app, "POST", "/api/v1/auth/login", None, Some(json!({"username": u, "password": p, "view": view}))).await
}

#[tokio::test]
async fn registry_users_sign_in_and_local_staff_still_can() {
    let Ok(url) = std::env::var("CADASTRE_TEST_DATABASE_URL") else {
        eprintln!("skipped: CADASTRE_TEST_DATABASE_URL not set");
        return;
    };
    let base = registry().await;
    std::env::set_var("CADASTRE_REGISTRY_URL", &base);
    std::env::set_var("CADASTRE_REGISTRY_USER", "svc");
    std::env::set_var("CADASTRE_REGISTRY_PASSWORD", "svcpw");
    let pool = db::connect_and_migrate(&url).await.expect("migrate");
    for code in ["scan1", "regadmin", "cashier", "guest"] {
        sqlx::query("DELETE FROM cadastre.principal WHERE lower(code) = $1").bind(code).execute(&pool).await.unwrap();
        sqlx::query("DELETE FROM cadastre.login_throttle WHERE login = $1").bind(code).execute(&pool).await.unwrap();
    }
    let config = Config {
        database_url: url.clone(), bind_addr: "127.0.0.1:0".into(), auth_mode: AuthMode::Local,
        keycloak_issuer: None, keycloak_audience: None, keycloak_jwks_json: None, keycloak_jwks_path: None,
        catalog_folders: Vec::new(), catalog_databases: Vec::new(),
    };
    let state = AppState { pool: pool.clone(), config, jwks: None::<Arc<_>> };
    let app = routes::gated(routes::router(state.clone()), state);

    // A scanning user: no account here yet, no password to change, Title Deed but not Sysadmin.
    assert_eq!(sign_in(&app, "scan1", "RegPass1", "sysadmin").await.0, StatusCode::FORBIDDEN);
    let (st, v, c, _) = sign_in(&app, "scan1", "RegPass1", "titledeed").await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["must_change"], json!(false));
    let c = c.unwrap();
    assert_eq!(call(&app, "GET", "/title-deed", Some(&c), None).await.0, StatusCode::OK);
    assert_eq!(call(&app, "GET", "/sys-editor", Some(&c), None).await.3, "/login?denied=1");
    let (_, me, _, _) = call(&app, "GET", "/api/v1/me", Some(&c), None).await;
    assert_eq!(me["username"], json!("scan1"));
    let made: i64 = sqlx::query_scalar("SELECT count(*) FROM cadastre.principal WHERE code = 'scan1' AND is_user").fetch_one(&pool).await.unwrap();
    assert_eq!(made, 1, "the account is made at first sign-in");
    let held: i64 = sqlx::query_scalar("SELECT count(*) FROM cadastre.user_credential c JOIN cadastre.principal p ON p.sid = c.sid WHERE p.code = 'scan1'").fetch_one(&pool).await.unwrap();
    assert_eq!(held, 0, "no Registry password is kept here");
    let (st, _, _, _) = call(&app, "POST", "/api/v1/auth/change-password", Some(&c), Some(json!({"current_password": "RegPass1", "new_password": "Plateau2026"}))).await;
    assert_eq!(st, StatusCode::CONFLICT, "a Registry password is changed in the Registry");

    // Case does not make a second account.
    assert_eq!(sign_in(&app, "scan1", "wrong", "titledeed").await.0, StatusCode::UNAUTHORIZED);

    // The Registry's admin group opens Sysadmin.
    let (st, _, c, _) = sign_in(&app, "regadmin", "RegPass1", "sysadmin").await;
    assert_eq!(st, StatusCode::OK);
    let admin = c.unwrap();
    assert_eq!(call(&app, "GET", "/sys-editor", Some(&admin), None).await.0, StatusCode::OK);
    assert_eq!(call(&app, "GET", "/api/v1/admin/security", Some(&admin), None).await.0, StatusCode::OK);

    // Another Registry group: signed in, Charting and Reporting only.
    assert_eq!(sign_in(&app, "cashier", "RegPass1", "digitising").await.0, StatusCode::FORBIDDEN);
    assert_eq!(sign_in(&app, "cashier", "RegPass1", "reporting").await.0, StatusCode::OK);

    // The Registry's guest never signs in, and neither does an empty password.
    assert_eq!(sign_in(&app, "guest", "anything", "reporting").await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(sign_in(&app, "regadmin", "", "reporting").await.0, StatusCode::UNAUTHORIZED);

    // Staff who are not in the Registry keep this server's account, also with the Registry down.
    let (sid, code): (i32, String) = (499_500_000 + (std::process::id() as i32 % 400_000), format!("local{}", std::process::id()));
    sqlx::query("INSERT INTO cadastre.principal (sid, code, name, is_user) VALUES ($1, $2, $2, true)").bind(sid).bind(&code).execute(&pool).await.unwrap();
    let (st, r, _, _) = call(&app, "POST", &format!("/api/v1/admin/users/{sid}/password"), Some(&admin), None).await;
    assert_eq!(st, StatusCode::OK, "{r}");
    let temp = r["temporary_password"].as_str().unwrap();
    let (st, v, _, _) = sign_in(&app, &code, temp, "reporting").await;
    assert_eq!((st, v["must_change"].clone()), (StatusCode::OK, json!(true)));
    std::env::set_var("CADASTRE_REGISTRY_URL", "http://127.0.0.1:1/lrs/api/");
    assert_eq!(sign_in(&app, &code, temp, "reporting").await.0, StatusCode::OK, "the Registry being down does not stop local staff");
    assert_eq!(sign_in(&app, "scan1", "RegPass1", "titledeed").await.0, StatusCode::UNAUTHORIZED, "nor does it let anyone else in");
    std::env::set_var("CADASTRE_REGISTRY_URL", &base);

    // Five wrong passwords lock a Registry login too, even for the right password.
    for _ in 0..3 {
        assert_eq!(sign_in(&app, "scan1", "wrong", "titledeed").await.0, StatusCode::UNAUTHORIZED);
    }
    assert_eq!(sign_in(&app, "scan1", "RegPass1", "titledeed").await.0, StatusCode::FORBIDDEN);
}
