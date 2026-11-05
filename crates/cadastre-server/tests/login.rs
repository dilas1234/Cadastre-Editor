//! Sign-in (`CADASTRE_AUTH=local`) through the real gate: who reaches what, the first-time
//! password change, the lockout, sign-out, and the Registry's services staying public.

use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{header, Request, StatusCode};
use axum::Router;
use serde_json::{json, Value};
use tower::ServiceExt;

use cadastre_server::config::{AuthMode, Config};
use cadastre_server::state::AppState;
use cadastre_server::{db, routes};

async fn state(url: &str, auth_mode: AuthMode) -> AppState {
    let pool = db::connect_and_migrate(url).await.expect("connect + migrate test db");
    let config = Config {
        database_url: url.to_string(), bind_addr: "127.0.0.1:0".to_string(), auth_mode,
        keycloak_issuer: None, keycloak_audience: None, keycloak_jwks_json: None, keycloak_jwks_path: None,
        catalog_folders: Vec::new(), catalog_databases: Vec::new(),
    };
    AppState { pool, config, jwks: None::<Arc<_>> }
}

struct Reply { status: StatusCode, json: Value, location: String, cookie: Option<String> }

async fn call(app: &Router, method: &str, uri: &str, cookie: Option<&str>, body: Option<Value>) -> Reply {
    let mut b = Request::builder().method(method).uri(uri);
    if let Some(c) = cookie { b = b.header(header::COOKIE, c); }
    let req = match body {
        Some(v) => b.header(header::CONTENT_TYPE, "application/json").body(Body::from(v.to_string())).unwrap(),
        None => b.body(Body::empty()).unwrap(),
    };
    let r = app.clone().oneshot(req).await.unwrap();
    let status = r.status();
    let location = r.headers().get(header::LOCATION).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let cookie = r.headers().get(header::SET_COOKIE).and_then(|v| v.to_str().ok()).map(|c| c.split(';').next().unwrap().to_string());
    let bytes = to_bytes(r.into_body(), usize::MAX).await.unwrap();
    Reply { status, json: serde_json::from_slice(&bytes).unwrap_or(Value::Null), location, cookie }
}

async fn sign_in(app: &Router, user: &str, password: &str, view: &str) -> Reply {
    call(app, "POST", "/api/v1/auth/login", None, Some(json!({"username": user, "password": password, "view": view}))).await
}

#[tokio::test]
async fn sign_in_follows_the_groups() {
    let Ok(url) = std::env::var("CADASTRE_TEST_DATABASE_URL") else {
        eprintln!("skipped: CADASTRE_TEST_DATABASE_URL not set");
        return;
    };
    let open = state(&url, AuthMode::Disabled).await;
    let pool = open.pool.clone();
    let open = routes::gated(routes::router(open.clone()), open);
    let local = state(&url, AuthMode::Local).await;
    let app = routes::gated(routes::router(local.clone()), local);

    let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos() % 100_000_000;
    let sid0 = 500_000_000 + (n as i32 % 100_000_000);
    for (sid, code) in [(499_000_001, "admins"), (499_000_002, "users")] {
        sqlx::query("INSERT INTO cadastre.principal (sid, code, name, is_user) SELECT $1, $2, $2, false WHERE NOT EXISTS (SELECT 1 FROM cadastre.principal WHERE lower(code) = $2)")
            .bind(sid).bind(code).execute(&pool).await.unwrap();
    }
    let group = |code: &'static str| { let pool = pool.clone(); async move {
        sqlx::query_scalar::<_, i32>("SELECT sid FROM cadastre.principal WHERE lower(code) = $1").bind(code).fetch_one(&pool).await.unwrap() } };
    let (admins, users) = (group("admins").await, group("users").await);
    let (admin, staff, other, victim) = (format!("adm{n}"), format!("stf{n}"), format!("oth{n}"), format!("vic{n}"));
    let mut temp = std::collections::HashMap::new();
    for (i, (code, g)) in [(&admin, Some(admins)), (&staff, Some(users)), (&other, None), (&victim, Some(users))].into_iter().enumerate() {
        let sid = sid0 + i as i32;
        sqlx::query("INSERT INTO cadastre.principal (sid, code, name, is_user) VALUES ($1, $2, $2, true)").bind(sid).bind(code).execute(&pool).await.unwrap();
        if let Some(g) = g {
            sqlx::query("INSERT INTO cadastre.principal_member (group_sid, member_sid) VALUES ($1, $2)").bind(g).bind(sid).execute(&pool).await.unwrap();
        }
        // Accounts are prepared while sign-in is still off.
        let r = call(&open, "POST", &format!("/api/v1/admin/users/{sid}/password"), None, None).await;
        assert_eq!(r.status, StatusCode::OK, "{}", r.json);
        temp.insert(code.clone(), r.json["temporary_password"].as_str().unwrap().to_string());
    }

    // Signed out: pages go to the sign-in page, the API refuses, public addresses answer.
    let r = call(&app, "GET", "/digitising", None, None).await;
    assert!(r.status.is_redirection() && r.location == "/login", "{} {}", r.status, r.location);
    assert_eq!(call(&app, "GET", "/editor/app.js", None, None).await.location, "/login");
    assert_eq!(call(&app, "GET", "/api/v1/wf/principals", None, None).await.status, StatusCode::UNAUTHORIZED);
    assert_eq!(call(&app, "GET", "/api/v1/me", Some("cad_session=forged"), None).await.status, StatusCode::UNAUTHORIZED);
    assert_eq!(call(&app, "GET", "/login", None, None).await.status, StatusCode::OK);
    assert_eq!(call(&app, "GET", "/editor/login/login.css", None, None).await.status, StatusCode::OK);
    assert_eq!(call(&app, "GET", "/api/v1/auth/config", None, None).await.json["local"], json!(true));
    let r = call(&app, "GET", "/GISViewer/MapWorker.aspx", None, None).await;
    assert!(r.status != StatusCode::UNAUTHORIZED && r.location != "/login", "the Registry's map stays public");
    let r = call(&app, "GET", cadastre_server::routes::mcservice::PATH, None, None).await;
    assert!(r.status != StatusCode::UNAUTHORIZED && r.location != "/login", "the Registry's job service stays public");

    // Wrong password and unknown user answer the same.
    let bad = sign_in(&app, &staff, "wrong", "digitising").await;
    let unknown = sign_in(&app, "nobody-here", "wrong", "digitising").await;
    assert_eq!(bad.status, StatusCode::UNAUTHORIZED);
    assert_eq!((unknown.status, &unknown.json["message"]), (bad.status, &bad.json["message"]));

    // Staff: not Sysadmin; a temporary password blocks everything until it is changed.
    assert_eq!(sign_in(&app, &staff, &temp[&staff], "sysadmin").await.status, StatusCode::FORBIDDEN);
    let r = sign_in(&app, &staff, &temp[&staff], "digitising").await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.json);
    assert_eq!((r.json["must_change"].clone(), r.json["redirect"].clone()), (json!(true), json!("/digitising")));
    let c = r.cookie.unwrap();
    assert_eq!(call(&app, "GET", "/digitising", Some(&c), None).await.location, "/login?change=1");
    assert_eq!(call(&app, "GET", "/api/v1/wf/principals", Some(&c), None).await.status, StatusCode::FORBIDDEN);
    let change = |current: &str, new: &str| json!({"current_password": current, "new_password": new});
    assert_eq!(call(&app, "POST", "/api/v1/auth/change-password", Some(&c), Some(change(&temp[&staff], "short"))).await.status, StatusCode::BAD_REQUEST);
    assert_eq!(call(&app, "POST", "/api/v1/auth/change-password", Some(&c), Some(change("wrong", "Plateau2026"))).await.status, StatusCode::UNAUTHORIZED);
    assert_eq!(call(&app, "POST", "/api/v1/auth/change-password", Some(&c), Some(change(&temp[&staff], "Plateau2026"))).await.status, StatusCode::OK);
    assert_eq!(call(&app, "GET", "/digitising", Some(&c), None).await.status, StatusCode::OK);
    assert_eq!(call(&app, "GET", "/title-deed", Some(&c), None).await.status, StatusCode::OK);
    assert_eq!(call(&app, "GET", "/sys-editor", Some(&c), None).await.location, "/login?denied=1");
    assert_eq!(call(&app, "GET", "/api/v1/admin/security", Some(&c), None).await.status, StatusCode::FORBIDDEN);
    assert_eq!(call(&app, "GET", "/api/v1/audit", Some(&c), None).await.status, StatusCode::FORBIDDEN);
    let me = call(&app, "GET", "/api/v1/me", Some(&c), None).await;
    assert_eq!(me.json["username"], json!(staff), "the audit sees the real person");
    assert!(!me.json["roles"].as_array().unwrap().contains(&json!("admin")));
    assert_eq!(sign_in(&app, &staff, &temp[&staff], "digitising").await.status, StatusCode::UNAUTHORIZED, "the temporary password is gone");

    // Sign-out ends the session on the server.
    assert_eq!(call(&app, "POST", "/api/v1/auth/logout", Some(&c), None).await.status, StatusCode::OK);
    assert_eq!(call(&app, "GET", "/api/v1/me", Some(&c), None).await.status, StatusCode::UNAUTHORIZED);

    // Not in Users: Charting and Reporting only, read-only.
    assert_eq!(sign_in(&app, &other, &temp[&other], "titledeed").await.status, StatusCode::FORBIDDEN);
    let c = sign_in(&app, &other, &temp[&other], "reporting").await.cookie.unwrap();
    assert_eq!(call(&app, "POST", "/api/v1/auth/change-password", Some(&c), Some(change(&temp[&other], "Plateau2026"))).await.status, StatusCode::OK);
    assert_eq!(call(&app, "GET", "/reporting", Some(&c), None).await.status, StatusCode::OK);
    assert_eq!(call(&app, "GET", "/digitising", Some(&c), None).await.location, "/login?denied=1");
    assert_eq!(call(&app, "POST", "/api/v1/jobs", Some(&c), Some(json!({}))).await.status, StatusCode::FORBIDDEN);

    // Administrators reach everything.
    let c = sign_in(&app, &admin, &temp[&admin], "sysadmin").await.cookie.unwrap();
    assert_eq!(call(&app, "POST", "/api/v1/auth/change-password", Some(&c), Some(change(&temp[&admin], "Plateau2026"))).await.status, StatusCode::OK);
    for page in ["/sys-editor", "/digitising", "/title-deed", "/reporting"] {
        assert_eq!(call(&app, "GET", page, Some(&c), None).await.status, StatusCode::OK, "{page}");
    }
    assert_eq!(call(&app, "GET", "/api/v1/admin/security", Some(&c), None).await.status, StatusCode::OK);

    // Five wrong passwords lock the account, even for the right one, until an administrator resets it.
    for _ in 0..5 {
        assert_eq!(sign_in(&app, &victim, "wrong", "digitising").await.status, StatusCode::UNAUTHORIZED);
    }
    assert_eq!(sign_in(&app, &victim, &temp[&victim], "digitising").await.status, StatusCode::FORBIDDEN);
    let r = call(&app, "POST", &format!("/api/v1/admin/users/{}/password", sid0 + 3), Some(&c), None).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.json);
    assert_eq!(sign_in(&app, &victim, r.json["temporary_password"].as_str().unwrap(), "digitising").await.status, StatusCode::OK);

    // A locked account's session stops working at once.
    let c = sign_in(&app, &staff, "Plateau2026", "digitising").await.cookie.unwrap();
    sqlx::query("UPDATE cadastre.principal SET is_locked = true WHERE lower(code) = $1").bind(&staff).execute(&pool).await.unwrap();
    assert_eq!(call(&app, "GET", "/api/v1/me", Some(&c), None).await.status, StatusCode::UNAUTHORIZED);
}
