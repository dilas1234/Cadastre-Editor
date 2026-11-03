//! Registry Web Access map: pass Aumentum's GISViewer through to Aumentum (trial bridge).
//!
//! Registry Web Access (`lrswa`, `map.do`) builds its map tab from the Registry setting
//! `GISServer`: it probes `{GISViewer}/Assets/s.gif` ("No connection to Cadastre Server" when
//! that fails) and embeds Aumentum's Silverlight viewer `{GISViewer}/Client/Client.xap`, which
//! loads `Client/Conf/web-access.xml` and draws through `GISViewer/gis.gate/*`. That viewer is
//! Aumentum's own program and reads Aumentum's database, so until it is replaced this server
//! forwards every `/GISViewer/...` request it does not answer itself (MapWorker.aspx, the
//! CadastralWebService and `/GISViewer/data/*` stay ours) to `CADASTRE_GISVIEWER_UPSTREAM`
//! (e.g. `http://10.10.10.6`). Unset: such requests get 404, as before.
//!
//! Each forwarded request is logged (method, path, status, size): that is the capture the
//! full replacement of the Web Access viewer is built from.

use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::extract::Request;
use axum::http::{HeaderName, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// Request headers passed on to Aumentum.
const FORWARD_REQUEST: &[&str] = &[
    "accept", "accept-language", "content-type", "cookie", "soapaction", "user-agent",
    "if-modified-since", "if-none-match", "referer",
];

/// Response headers not copied back (hop-by-hop, or recomputed here).
const DROP_RESPONSE: &[&str] = &["connection", "transfer-encoding", "content-length", "keep-alive"];

/// Upper bound on a forwarded request or response body.
const MAX_BODY: usize = 64 * 1024 * 1024;

pub fn upstream() -> Option<String> {
    std::env::var("CADASTRE_GISVIEWER_UPSTREAM")
        .ok()
        .map(|v| v.trim().trim_end_matches('/').to_string())
        .filter(|v| v.starts_with("http://"))
}

/// Split `http://host[:port]` into (authority, host, port).
pub fn authority(upstream: &str) -> Option<(String, String, u16)> {
    let a = upstream.strip_prefix("http://")?.split('/').next()?.to_string();
    let (h, p) = match a.rsplit_once(':') {
        Some((h, p)) => (h.to_string(), p.parse().ok()?),
        None => (a.clone(), 80),
    };
    Some((a, h, p))
}

pub struct Parsed {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// Parse an HTTP/1.x response read to EOF; de-chunks a chunked body.
pub fn parse_response(raw: &[u8]) -> Option<Parsed> {
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = String::from_utf8_lossy(&raw[..split]);
    let mut lines = head.split("\r\n");
    let status = lines.next()?.split_whitespace().nth(1)?.parse().ok()?;
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':').map(|(k, v)| (k.trim().to_string(), v.trim().to_string())))
        .collect();
    let mut body = raw[split + 4..].to_vec();
    let chunked = headers.iter().any(|(k, v)| k.eq_ignore_ascii_case("transfer-encoding") && v.to_ascii_lowercase().contains("chunked"));
    if chunked {
        body = dechunk(&body)?;
    }
    Some(Parsed { status, headers, body })
}

fn dechunk(mut b: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let eol = b.windows(2).position(|w| w == b"\r\n")?;
        let size_txt = String::from_utf8_lossy(&b[..eol]);
        let size = usize::from_str_radix(size_txt.split(';').next()?.trim(), 16).ok()?;
        b = &b[eol + 2..];
        if size == 0 {
            return Some(out);
        }
        if b.len() < size {
            return None;
        }
        out.extend_from_slice(&b[..size]);
        b = b.get(size + 2..).unwrap_or(&[]);
    }
}

/// Rewrite an absolute `Location` on Aumentum to this server's own address.
pub fn rewrite_location(v: &str, upstream: &str, own_host: Option<&str>) -> String {
    match (v.strip_prefix(upstream), own_host) {
        (Some(rest), Some(h)) => format!("http://{h}{rest}"),
        _ => v.to_string(),
    }
}

/// Fallback handler for `/GISViewer/{*rest}`.
pub async fn forward(req: Request) -> Response {
    let Some(up) = upstream() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some((auth, host, port)) = authority(&up) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let method = req.method().clone();
    let path_q = req.uri().path_and_query().map(|p| p.as_str().to_string()).unwrap_or_else(|| "/".into());
    let own_host = req.headers().get("host").and_then(|v| v.to_str().ok()).map(str::to_string);
    let mut head = format!("{method} {path_q} HTTP/1.0\r\nHost: {auth}\r\nConnection: close\r\n");
    for name in FORWARD_REQUEST {
        if let Some(v) = req.headers().get(*name).and_then(|v| v.to_str().ok()) {
            head.push_str(&format!("{name}: {v}\r\n"));
        }
    }
    let body: Bytes = match axum::body::to_bytes(req.into_body(), MAX_BODY).await {
        Ok(b) => b,
        Err(_) => return StatusCode::PAYLOAD_TOO_LARGE.into_response(),
    };
    if !body.is_empty() || method == Method::POST || method == Method::PUT {
        head.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    head.push_str("\r\n");

    let exchange = async {
        let mut s = TcpStream::connect((host.as_str(), port)).await?;
        s.write_all(head.as_bytes()).await?;
        s.write_all(&body).await?;
        let mut raw = Vec::new();
        (&mut s).take(MAX_BODY as u64 + 65536).read_to_end(&mut raw).await?;
        Ok::<_, std::io::Error>(raw)
    };
    let raw = match tokio::time::timeout(Duration::from_secs(120), exchange).await {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => {
            tracing::warn!("Web Access map pass-through: {method} {path_q} -> {auth}: {e}");
            return (StatusCode::BAD_GATEWAY, "Aumentum GISViewer could not be reached").into_response();
        }
        Err(_) => {
            tracing::warn!("Web Access map pass-through: {method} {path_q} -> {auth}: timed out");
            return (StatusCode::GATEWAY_TIMEOUT, "Aumentum GISViewer timed out").into_response();
        }
    };
    let Some(p) = parse_response(&raw) else {
        tracing::warn!("Web Access map pass-through: {method} {path_q}: malformed answer from {auth}");
        return StatusCode::BAD_GATEWAY.into_response();
    };
    tracing::info!("Web Access map pass-through: {method} {path_q} -> {} ({} bytes)", p.status, p.body.len());

    let mut resp = Response::new(Body::from(p.body));
    *resp.status_mut() = StatusCode::from_u16(p.status).unwrap_or(StatusCode::BAD_GATEWAY);
    let h = resp.headers_mut();
    for (k, v) in p.headers {
        let lk = k.to_ascii_lowercase();
        if DROP_RESPONSE.contains(&lk.as_str()) {
            continue;
        }
        let v = if lk == "location" { rewrite_location(&v, &up, own_host.as_deref()) } else { v };
        if let (Ok(n), Ok(val)) = (HeaderName::from_bytes(lk.as_bytes()), HeaderValue::from_str(&v)) {
            h.append(n, val);
        }
    }
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_and_chunked() {
        let r = parse_response(b"HTTP/1.1 200 OK\r\nContent-Type: image/gif\r\n\r\nGIF89a").unwrap();
        assert_eq!((r.status, r.body.as_slice()), (200, &b"GIF89a"[..]));
        let r = parse_response(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n2\r\nde\r\n0\r\n\r\n").unwrap();
        assert_eq!(r.body, b"abcde");
    }

    #[test]
    fn authority_and_location() {
        assert_eq!(authority("http://10.10.10.6").unwrap(), ("10.10.10.6".into(), "10.10.10.6".into(), 80));
        assert_eq!(authority("http://h:81").unwrap().2, 81);
        assert_eq!(
            rewrite_location("http://10.10.10.6/GISViewer/x", "http://10.10.10.6", Some("10.10.10.9:8080")),
            "http://10.10.10.9:8080/GISViewer/x"
        );
    }
}
