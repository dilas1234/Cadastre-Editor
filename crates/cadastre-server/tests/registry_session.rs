//! Registry sessions never leak, against a stand-in Registry that counts open tickets. One test: the
//! shared session is per process.

use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
use std::sync::Mutex;
use std::time::Duration;

use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{any, get};
use axum::Router;
use cadastre_server::lrs::{self, LrsConfig, Session};

static OPEN: Mutex<Option<HashSet<String>>> = Mutex::new(None);
static STARTS: AtomicUsize = AtomicUsize::new(0);
static FAIL_ENDS: AtomicUsize = AtomicUsize::new(0);
static FULL: AtomicUsize = AtomicUsize::new(0);

fn open() -> usize { OPEN.lock().unwrap().as_ref().map_or(0, HashSet::len) }
fn ticket(body: &str) -> String { body.split("auth:ticket>").nth(1).unwrap_or("").trim_end_matches("</").to_string() }

async fn auth(body: String) -> axum::response::Response {
    let fault = |m: &str| (StatusCode::INTERNAL_SERVER_ERROR, format!("<s:Fault><faultstring></faultstring><detail><ns:message>{m}</ns:message></detail></s:Fault>")).into_response();
    if body.contains("isTicketValid") {
        let ok = OPEN.lock().unwrap().as_ref().is_some_and(|o| o.contains(&ticket(&body)));
        return format!("<r><result>{ok}</result></r>").into_response();
    }
    if body.contains("endSession") {
        if FAIL_ENDS.swap(0, SeqCst) == 1 { return fault("busy"); }
        if let Some(o) = OPEN.lock().unwrap().as_mut() { o.remove(&ticket(&body)); }
        return "<ok/>".into_response();
    }
    if FULL.load(SeqCst) == 1 { return fault("The number of authenticated users has exceeded the number of supported by license."); }
    let t = format!("T{}", STARTS.fetch_add(1, SeqCst));
    OPEN.lock().unwrap().get_or_insert_with(HashSet::new).insert(t.clone());
    tokio::time::sleep(Duration::from_millis(200)).await; // the ticket exists before the answer arrives
    format!("<r><ticket>{t}</ticket></r>").into_response()
}

async fn none_open() {
    for _ in 0..60 {
        if open() == 0 { return; }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("{} session(s) left open", open());
}

#[tokio::test]
async fn sessions_are_always_signed_out() {
    let app = Router::new()
        .route("/lrs/api/DomainModelService", get(|| async { "wsdl" }).post(|| async { (StatusCode::INTERNAL_SERVER_ERROR, "<f/>") }))
        .route("/lrs/api/AuthenticationService", any(auth));
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let cfg = LrsConfig { base_url: format!("http://{}/lrs/api/", l.local_addr().unwrap()), user: "svc".into(), password: "pw".into(), write_enabled: false };
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });

    // The caller goes away while signing in, after the Registry made the ticket.
    assert!(tokio::time::timeout(Duration::from_millis(100), Session::sign_in(&cfg)).await.is_err());
    assert_eq!(open(), 1);
    none_open().await;
    // The same through the shared session.
    assert!(tokio::time::timeout(Duration::from_millis(100), Session::start(&cfg)).await.is_err());
    assert_eq!(open(), 1);
    none_open().await;
    // Dropped without end(); ended while the Registry refuses the first sign-out.
    drop(Session::sign_in(&cfg).await.unwrap());
    none_open().await;
    FAIL_ENDS.store(1, SeqCst);
    Session::sign_in(&cfg).await.unwrap().end().await;
    none_open().await;

    // Shared: many uses, one sign-in, and a check after a refused call does not sign in again.
    let before = STARTS.load(SeqCst);
    for _ in 0..10 {
        let s = Session::start(&cfg).await.unwrap();
        assert!(s.query("x", 1).await.is_err());
        s.end().await;
    }
    assert_eq!((STARTS.load(SeqCst) - before, open()), (1, 1));
    // Validating someone else's ticket does not sign it out.
    let own = Session::sign_in(&cfg).await.unwrap();
    assert!(lrs::ticket_valid(&cfg, "svc", &format!("T{}", STARTS.load(SeqCst) - 1)).await.unwrap());
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(open(), 2);
    own.end().await;
    // Deleted in User Sessions: the next use after a refusal signs in again.
    OPEN.lock().unwrap().as_mut().unwrap().clear();
    let s = Session::start(&cfg).await.unwrap();
    assert!(s.query("x", 1).await.is_err());
    s.end().await;
    Session::start(&cfg).await.unwrap().end().await;
    assert_eq!((STARTS.load(SeqCst) - before, open()), (3, 1));
    // The server stops.
    lrs::shutdown().await;
    assert_eq!(open(), 0);

    // The licence message is shown.
    FULL.store(1, SeqCst);
    let e = Session::start(&cfg).await.err().unwrap().to_string();
    assert!(e.contains("exceeded the number of supported by license"), "{e}");
}
