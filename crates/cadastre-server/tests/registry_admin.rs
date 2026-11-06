//! The Sysadmin Registry dashboard: writes obey `CADASTRE_REGISTRY_WRITE`, and a password set there
//! puts its owner through the forced change at sign-in. The SQL Server side needs a real Registry,
//! so a stand-in future takes the place of the procedure call.
#![cfg(feature = "registry-admin")]

use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use serde_json::json;
use tower::ServiceExt;

use cadastre_server::config::{AuthMode, Config, RegistrySqlConfig};
use cadastre_server::error::AppError;
use cadastre_server::routes::regadmin::with_forced_change;
use cadastre_server::state::AppState;
use cadastre_server::{db, routes};

async fn listed(pool: &sqlx::PgPool, login: &str) -> bool {
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM cadastre.registry_password_reset WHERE login = $1)")
        .bind(login).fetch_one(pool).await.unwrap()
}

#[tokio::test]
async fn a_set_password_forces_a_change_only_if_the_write_worked() {
    let Ok(url) = std::env::var("CADASTRE_TEST_DATABASE_URL") else {
        eprintln!("skipped: CADASTRE_TEST_DATABASE_URL not set");
        return;
    };
    let pool = db::connect_and_migrate(&url).await.expect("migrate");
    for login in ["regadm_ok", "regadm_no"] {
        sqlx::query("DELETE FROM cadastre.registry_password_reset WHERE login = $1").bind(login).execute(&pool).await.unwrap();
    }

    // The written name is listed as sign-in looks it up: trimmed and in lower case.
    with_forced_change(&pool, " RegAdm_OK ", "tester", async { Ok(()) }).await.unwrap();
    assert!(listed(&pool, "regadm_ok").await);
    // Reset again: still one row, and no error.
    with_forced_change(&pool, "regadm_ok", "tester2", async { Ok(()) }).await.unwrap();
    let by: String = sqlx::query_scalar("SELECT set_by FROM cadastre.registry_password_reset WHERE login = 'regadm_ok'").fetch_one(&pool).await.unwrap();
    assert_eq!(by, "tester2");

    // A refused write leaves nothing behind, and the refusal reaches the caller.
    let refused = with_forced_change(&pool, "regadm_no", "tester", async { Err(AppError::BadRequest("Username already exists.".into())) }).await;
    assert!(matches!(refused, Err(AppError::BadRequest(_))));
    assert!(!listed(&pool, "regadm_no").await, "no row for a write that failed");

    sqlx::query("DELETE FROM cadastre.registry_password_reset WHERE login = 'regadm_ok'").execute(&pool).await.unwrap();
}

#[tokio::test]
async fn writes_are_refused_while_registry_writing_is_off() {
    let Ok(url) = std::env::var("CADASTRE_TEST_DATABASE_URL") else {
        eprintln!("skipped: CADASTRE_TEST_DATABASE_URL not set");
        return;
    };
    std::env::set_var("CADASTRE_REGISTRY_URL", "http://127.0.0.1:9/lrs/api/");
    std::env::set_var("CADASTRE_REGISTRY_USER", "svc");
    std::env::set_var("CADASTRE_REGISTRY_PASSWORD", "svcpw");
    std::env::set_var("CADASTRE_REGISTRY_WRITE", "0");
    let pool = db::connect_and_migrate(&url).await.expect("migrate");
    // A SQL Server nothing listens on: reaching it would answer 422, not the 409 of the switch.
    let registry_sql = Some(RegistrySqlConfig { host: "127.0.0.1".into(), port: 9, database: "LRS43".into(), username: "x".into(), password: "x".into(), encrypt: true, direct: false });
    let config = Config {
        database_url: url, bind_addr: "127.0.0.1:0".into(), auth_mode: AuthMode::Disabled,
        keycloak_issuer: None, keycloak_audience: None, keycloak_jwks_json: None, keycloak_jwks_path: None,
        catalog_folders: Vec::new(), catalog_databases: Vec::new(), registry_sql,
    };
    let state = AppState { pool: pool.clone(), config, jwks: None::<Arc<_>> };
    let app = routes::gated(routes::router(state.clone()), state);

    let writes = [
        ("POST", "/api/v1/admin/registry/users", json!({"user_name": "regadm_off", "first_name": "A", "last_name": "B", "password": "secret1", "ref_user": "c"})),
        ("POST", "/api/v1/admin/registry/groups", json!({"display_name": "Nope"})),
        ("PUT", "/api/v1/admin/registry/users/regadm_off", json!({"first_name": "A"})),
        ("POST", "/api/v1/admin/registry/users/regadm_off/groups", json!({"group": "GROUP_X"})),
        ("POST", "/api/v1/admin/registry/users/regadm_off/password", json!({"password": "secret1"})),
        ("POST", "/api/v1/admin/registry/users/regadm_off/enabled", json!({"enabled": false})),
    ];
    for (method, uri, body) in writes {
        let req = Request::builder().method(method).uri(uri).header(header::CONTENT_TYPE, "application/json").body(Body::from(body.to_string())).unwrap();
        assert_eq!(app.clone().oneshot(req).await.unwrap().status(), StatusCode::CONFLICT, "{method} {uri}");
    }
    let req = Request::builder().method("DELETE").uri("/api/v1/admin/registry/users/regadm_off/groups/GROUP_X").body(Body::empty()).unwrap();
    assert_eq!(app.clone().oneshot(req).await.unwrap().status(), StatusCode::CONFLICT, "DELETE from a group");
    assert!(!listed(&pool, "regadm_off").await);

    // Reading is not a write: it gets past the switch and fails only at the absent SQL Server.
    let req = Request::builder().uri("/api/v1/admin/registry/groups").body(Body::empty()).unwrap();
    assert_eq!(app.oneshot(req).await.unwrap().status(), StatusCode::UNPROCESSABLE_ENTITY);
}
