//! UPIN allocation: no duplicates when officers commit at the same moment, and (parallel run)
//! never a number Aumentum has already issued. A separate test binary, so its environment
//! (the fake Aumentum job service) affects no other test.

use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{header, Request, StatusCode};
use axum::Router;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower::ServiceExt;

use cadastre_server::config::{AuthMode, Config};
use cadastre_server::state::AppState;
use cadastre_server::{db, routes};

static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn test_db_url() -> Option<String> {
    std::env::var("CADASTRE_TEST_DATABASE_URL").ok().filter(|s| !s.is_empty())
}

async fn app(db_url: &str) -> Router {
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
    routes::router(AppState { pool, config, jwks: None::<Arc<_>> })
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

fn square(x: f64, y: f64, s: f64) -> Value {
    json!({"type":"Polygon","coordinates":[[[x,y],[x+s,y],[x+s,y+s],[x,y+s],[x,y]]]})
}

fn seed() -> u64 {
    (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos() as u64) % 1_000_000
}

/// One job with one draft parcel, taken to posting; returns the posted UPIN (or the error).
async fn post_one(app: Router, x: f64, y: f64) -> Result<String, String> {
    let (_, job) = send(&app, "POST", "/api/v1/jobs", Some(json!({"reference": format!("JOB-{}", uuid::Uuid::new_v4()), "job_type": "capture"}))).await;
    let id = job["id"].as_str().unwrap().to_string();
    let (st, p) = send(&app, "POST", &format!("/api/v1/jobs/{id}/parcels"), Some(json!({"geometry": square(x, y, 20.0)}))).await;
    assert_eq!(st, StatusCode::CREATED, "{p}");
    for action in ["submit", "start_review", "approve"] {
        let (st, j) = send(&app, "POST", &format!("/api/v1/jobs/{id}/transition"), Some(json!({ "action": action }))).await;
        assert_eq!(st, StatusCode::OK, "{action}: {j}");
    }
    let (st, r) = send(&app, "POST", &format!("/api/v1/jobs/{id}/post"), None).await;
    if st == StatusCode::OK { Ok(r["parcel_numbers"][0].as_str().unwrap().to_string()) } else { Err(format!("{st}: {r}")) }
}

#[tokio::test]
async fn concurrent_commits_never_share_a_upin() {
    let Some(db_url) = test_db_url() else { return };
    let _g = SERIAL.lock().await;
    std::env::remove_var("CADASTRE_AUMENTUM_JOBSERVICE_URL");
    let app = app(&db_url).await;
    let base = 300_000.0 + (seed() % 5_000) as f64 * 100.0;
    // Twelve officers commit at the same moment.
    let tasks: Vec<_> = (0..12).map(|i| tokio::spawn(post_one(app.clone(), base + i as f64 * 50.0, 8_500_000.0))).collect();
    let mut upins = Vec::new();
    for t in tasks {
        upins.push(t.await.unwrap().expect("every commit succeeds"));
    }
    let mut unique = upins.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), 12, "duplicate UPINs: {upins:?}");
    let pool = sqlx::PgPool::connect(&db_url).await.unwrap();
    let posted: i64 = sqlx::query_scalar("SELECT count(*) FROM cadastre.parcel WHERE parcel_number = ANY($1) AND status = 'posted'").bind(&upins).fetch_one(&pool).await.unwrap();
    assert_eq!(posted, 12);
}

/// A fake Aumentum job service: Authenticate (with a session cookie), and
/// GetParcelAttributesByUPIN answering "issued" for the UPINs given.
async fn fake_aumentum(issued: Vec<String>, up: bool) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else { break };
            let issued = issued.clone();
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut tmp = [0u8; 4096];
                loop {
                    let n = sock.read(&mut tmp).await.unwrap_or(0);
                    if n == 0 { break; }
                    buf.extend_from_slice(&tmp[..n]);
                    let t = String::from_utf8_lossy(&buf);
                    if let Some(i) = t.find("\r\n\r\n") {
                        let len = t[..i].lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0))).unwrap_or(0);
                        if buf.len() >= i + 4 + len { break; }
                    }
                }
                let req = String::from_utf8_lossy(&buf).to_string();
                let (status, cookie, body) = if !up {
                    ("500 Internal Server Error", "", "<fault/>".to_string())
                } else if req.contains("<Authenticate") {
                    ("200 OK", "Set-Cookie: ASP.NET_SessionId=fake; path=/\r\n", "<AuthenticateResult>true</AuthenticateResult>".to_string())
                } else if req.contains("<GetParcelAttributesByUPIN") {
                    let hit = issued.iter().any(|u| req.contains(&format!("<upin>{u}</upin>")));
                    ("200 OK", "", if hit { "<DataField><FieldName>UPIN</FieldName><FieldValue>x</FieldValue></DataField>".to_string() } else { String::new() })
                } else {
                    ("200 OK", "", String::new())
                };
                let resp = format!("HTTP/1.1 {status}\r\nContent-Type: text/xml\r\n{cookie}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                let _ = sock.write_all(resp.as_bytes()).await;
            });
        }
    });
    format!("http://{addr}/ApplicationServerAdmin/WebServices/LRSWebServices.asmx")
}

#[tokio::test]
async fn never_a_upin_aumentum_already_issued() {
    let Some(db_url) = test_db_url() else { return };
    let _g = SERIAL.lock().await;
    let app = app(&db_url).await;
    let pool = sqlx::PgPool::connect(&db_url).await.unwrap();
    std::env::set_var("CADASTRE_MC_SERVICE_USER", "administrator");
    std::env::set_var("CADASTRE_MC_SERVICE_PASSWORD", "test");
    // The next two numbers were issued by Aumentum after the data was copied.
    let next: i64 = sqlx::query_scalar("SELECT cadastre.sync_upin_sequence()").fetch_one(&pool).await.unwrap();
    let taken: Vec<String> = (0..2).map(|k| format!("PL{:012}", next + k)).collect();
    std::env::set_var("CADASTRE_AUMENTUM_JOBSERVICE_URL", fake_aumentum(taken.clone(), true).await);
    let x = 400_000.0 + (seed() % 5_000) as f64 * 100.0;
    let upin = post_one(app.clone(), x, 8_600_000.0).await.expect("commit");
    assert!(!taken.contains(&upin), "{upin} was Aumentum's");
    assert!(upin[2..].parse::<i64>().unwrap() >= next + 2, "{upin}");
    let recorded: Vec<String> = sqlx::query_scalar("SELECT source FROM cadastre.issued_upin WHERE upin = ANY($1) ORDER BY upin").bind(&taken).fetch_all(&pool).await.unwrap();
    assert_eq!(recorded, vec!["aumentum:live", "aumentum:live"], "Aumentum's numbers are kept in the ledger");

    // Aumentum cannot be asked: the commit is refused, nothing is posted, no number is used up.
    std::env::set_var("CADASTRE_AUMENTUM_JOBSERVICE_URL", fake_aumentum(vec![], false).await);
    let before: i64 = sqlx::query_scalar("SELECT count(*) FROM cadastre.issued_upin").fetch_one(&pool).await.unwrap();
    let err = post_one(app.clone(), x + 100.0, 8_600_000.0).await.expect_err("refused");
    assert!(err.starts_with("409") && err.contains("nothing was committed"), "{err}");
    let after: i64 = sqlx::query_scalar("SELECT count(*) FROM cadastre.issued_upin").fetch_one(&pool).await.unwrap();
    assert_eq!(before, after);
    std::env::remove_var("CADASTRE_AUMENTUM_JOBSERVICE_URL");
}

#[tokio::test]
async fn the_commit_window_preview_is_the_upin_the_commit_gives() {
    let Some(db_url) = test_db_url() else { return };
    let _g = SERIAL.lock().await;
    let app = app(&db_url).await;
    let pool = sqlx::PgPool::connect(&db_url).await.unwrap();
    std::env::set_var("CADASTRE_MC_SERVICE_USER", "administrator");
    std::env::set_var("CADASTRE_MC_SERVICE_PASSWORD", "test");
    // Aumentum issued the next two numbers after the data was copied.
    let next: i64 = sqlx::query_scalar("SELECT cadastre.sync_upin_sequence()").fetch_one(&pool).await.unwrap();
    let taken: Vec<String> = (0..2).map(|k| format!("PL{:012}", next + k)).collect();
    std::env::set_var("CADASTRE_AUMENTUM_JOBSERVICE_URL", fake_aumentum(taken.clone(), true).await);

    let mut conn = pool.acquire().await.unwrap();
    let before: i64 = sqlx::query_scalar("SELECT count(*) FROM cadastre.issued_upin").fetch_one(&mut *conn).await.unwrap();
    let p = cadastre_server::routes::jobs::preview_upins(&mut conn, 1).await.expect("preview");
    let after: i64 = sqlx::query_scalar("SELECT count(*) FROM cadastre.issued_upin").fetch_one(&mut *conn).await.unwrap();
    drop(conn);
    assert_eq!(p.aumentum, "checked", "{p:?}");
    assert_eq!(p.aumentum_skipped, taken, "Aumentum's numbers are shown as skipped");
    assert_eq!(p.upins.len(), 1);
    assert_eq!(before, after, "the preview records nothing");

    let x = 500_000.0 + (seed() % 5_000) as f64 * 100.0;
    let upin = post_one(app.clone(), x, 8_700_000.0).await.expect("commit");
    assert_eq!(upin, p.upins[0], "the commit gives the UPIN the Commit window showed");

    // Not configured: the preview says so and still gives the next number.
    std::env::remove_var("CADASTRE_AUMENTUM_JOBSERVICE_URL");
    let mut conn = pool.acquire().await.unwrap();
    let p = cadastre_server::routes::jobs::preview_upins(&mut conn, 2).await.expect("preview");
    assert_eq!(p.aumentum, "not_configured");
    assert_eq!(p.upins.len(), 2);
}

#[tokio::test]
async fn the_watch_flags_a_upin_aumentum_issued_after_it_was_given_here() {
    let Some(db_url) = test_db_url() else { return };
    let _g = SERIAL.lock().await;
    let app = app(&db_url).await;
    let pool = sqlx::PgPool::connect(&db_url).await.unwrap();
    std::env::remove_var("CADASTRE_AUMENTUM_JOBSERVICE_URL");
    let x = 600_000.0 + (seed() % 5_000) as f64 * 100.0;
    let upin = post_one(app.clone(), x, 8_800_000.0).await.expect("commit");

    // Later, Aumentum issues the same number (it does not know this Cadastre's numbers).
    std::env::set_var("CADASTRE_MC_SERVICE_USER", "administrator");
    std::env::set_var("CADASTRE_MC_SERVICE_PASSWORD", "test");
    std::env::set_var("CADASTRE_AUMENTUM_JOBSERVICE_URL", fake_aumentum(vec![upin.clone()], true).await);
    let s = cadastre_server::upin_watch::check_once(&pool).await;
    assert_eq!(s["ok"], json!(true), "{s}");
    let dups: Vec<String> = s["duplicates"].as_array().unwrap().iter().map(|d| d["upin"].as_str().unwrap().to_string()).collect();
    assert!(dups.contains(&upin), "{upin} is flagged: {s}");

    // Audited once, however many passes see it.
    let _ = cadastre_server::upin_watch::check_once(&pool).await;
    let audits: i64 = sqlx::query_scalar("SELECT count(*) FROM cadastre.audit_log WHERE action = 'duplicate_upin_found' AND entity_id = $1")
        .bind(&upin).fetch_one(&pool).await.unwrap();
    assert_eq!(audits, 1);

    // Aumentum unreachable: the pass fails but keeps the duplicates already known.
    std::env::set_var("CADASTRE_AUMENTUM_JOBSERVICE_URL", fake_aumentum(vec![], false).await);
    let s = cadastre_server::upin_watch::check_once(&pool).await;
    assert_eq!(s["ok"], json!(false));
    assert!(s["duplicates"].as_array().unwrap().iter().any(|d| d["upin"] == json!(upin)), "{s}");
    std::env::remove_var("CADASTRE_AUMENTUM_JOBSERVICE_URL");
}
