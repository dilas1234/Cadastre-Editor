//! A second copy of the server, whose port is taken, stops before it touches the database or the
//! Registry (a service retrying such a copy filled the Registry's licensed places).

use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn a_second_copy_stops_before_the_database_and_the_registry() {
    let port = TcpListener::bind("127.0.0.1:0").unwrap();
    let database = TcpListener::bind("127.0.0.1:0").unwrap();
    let registry = TcpListener::bind("127.0.0.1:0").unwrap();
    for l in [&database, &registry] { l.set_nonblocking(true).unwrap(); }
    let mut child = Command::new(env!("CARGO_BIN_EXE_cadastre-server"))
        .current_dir(std::env::temp_dir())
        .env_clear()
        .env("CADASTRE_BIND", port.local_addr().unwrap().to_string())
        .env("CADASTRE_AUTH", "disabled")
        .env("CADASTRE_DATABASE_URL", format!("postgres://t:t@{}/t", database.local_addr().unwrap()))
        .env("CADASTRE_REGISTRY_URL", format!("http://{}/lrs/api/", registry.local_addr().unwrap()))
        .env("CADASTRE_REGISTRY_USER", "svc")
        .env("CADASTRE_REGISTRY_PASSWORD", "pw")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let started = Instant::now();
    while child.try_wait().unwrap().is_none() {
        if started.elapsed() > Duration::from_secs(8) {
            child.kill().unwrap();
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let out = child.wait_with_output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    assert!(database.accept().is_err(), "the database was contacted: {text}");
    assert!(registry.accept().is_err(), "the Registry was contacted: {text}");
    assert!(!out.status.success());
    assert!(text.contains("cannot listen on"), "{text}");
}
