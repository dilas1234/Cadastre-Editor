//! Loading the Keycloak JWKS at startup.
//!
//! Resolution order when auth is enabled:
//!   1. `CADASTRE_KEYCLOAK_JWKS_JSON` (inline) — used by tests and air-gapped deploys;
//!   2. `CADASTRE_KEYCLOAK_JWKS_PATH` (file);
//!   3. fetch from `{issuer}/protocol/openid-connect/certs`.
//!
//! The live fetch supports **plaintext `http://` only** (the dev Keycloak on localhost).
//! `https://` returns an error pointing at Phase 9, where the server's TLS stack and crypto
//! provider are chosen — consistent with sqlx being built without a TLS backend (NFR-02).

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::config::Config;
use crate::jwks::Jwks;

/// Load the verification key set according to the resolution order above.
pub async fn load_jwks(config: &Config) -> Result<Jwks, String> {
    if let Some(json) = &config.keycloak_jwks_json {
        return Jwks::from_json(json).map_err(|e| format!("CADASTRE_KEYCLOAK_JWKS_JSON: {e}"));
    }
    if let Some(path) = &config.keycloak_jwks_path {
        let json = tokio::fs::read_to_string(path)
            .await
            .map_err(|e| format!("reading {path}: {e}"))?;
        return Jwks::from_json(&json).map_err(|e| format!("{path}: {e}"));
    }
    if let Some(issuer) = &config.keycloak_issuer {
        let url = format!("{}/protocol/openid-connect/certs", issuer.trim_end_matches('/'));
        let json = http_get(&url).await?;
        return Jwks::from_json(&json).map_err(|e| format!("JWKS from {url}: {e}"));
    }
    Err("auth enabled but no JWKS source (set CADASTRE_KEYCLOAK_JWKS_JSON, \
         CADASTRE_KEYCLOAK_JWKS_PATH, or CADASTRE_KEYCLOAK_ISSUER)"
        .to_string())
}

/// Minimal HTTP/1.1 GET for `http://` URLs. Sends `Connection: close` and reads the
/// body to EOF, so there is no chunked/keep-alive parsing to get wrong.
async fn http_get(url: &str) -> Result<String, String> {
    let rest = url
        .strip_prefix("http://")
        .ok_or_else(|| match url.strip_prefix("https://") {
            Some(_) => format!(
                "JWKS fetch over https is not supported yet (Phase 9 TLS); \
                 use CADASTRE_KEYCLOAK_JWKS_JSON/_PATH for now: {url}"
            ),
            None => format!("unsupported JWKS url scheme: {url}"),
        })?;

    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => (h, p.parse::<u16>().map_err(|_| "bad port".to_string())?),
        None => (authority, 80),
    };

    let mut stream = TcpStream::connect((host, port))
        .await
        .map_err(|e| format!("connect {host}:{port}: {e}"))?;
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {authority}\r\nAccept: application/json\r\n\
         Connection: close\r\n\r\n"
    );
    stream
        .write_all(request.as_bytes())
        .await
        .map_err(|e| format!("write: {e}"))?;

    let mut raw = Vec::new();
    stream
        .read_to_end(&mut raw)
        .await
        .map_err(|e| format!("read: {e}"))?;

    let text = String::from_utf8_lossy(&raw);
    let (headers, body) = text
        .split_once("\r\n\r\n")
        .ok_or_else(|| "malformed HTTP response (no header/body split)".to_string())?;
    let status_line = headers.lines().next().unwrap_or_default();
    if !status_line.contains(" 200") {
        return Err(format!("JWKS endpoint returned: {status_line}"));
    }
    Ok(body.to_string())
}
