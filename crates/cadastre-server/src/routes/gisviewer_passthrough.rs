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
//! Searches (`gis.gate/0/0/query` with a `where`) Aumentum answers with no feature are answered
//! from this Cadastre's posted parcels, in Aumentum's own JSON (its `fields`), so a parcel
//! committed here is found and outlined on the Web Access map. Aumentum's drawn map image still
//! shows only Aumentum's parcels.
//!
//! Each forwarded request is logged (method, path, status, size): that is the capture the
//! full replacement of the Web Access viewer is built from.

use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::extract::{Request, State};
use axum::http::{HeaderName, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use serde_json::{json, Value};
use sqlx::Row;

use crate::state::AppState;

/// The Web Access parcel layer's search path (`gis.gate/<service 0>/<layer 0>/query`).
const PARCEL_QUERY: &str = "/GISViewer/gis.gate/0/0/query";

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

/// A GET to Aumentum's GISViewer (e.g. a `gis.gate/0/0/query` search) for this server's own
/// checks; `None` when no upstream is configured. Read-only.
pub async fn aumentum_get(path_q: &str) -> Option<Result<Vec<u8>, String>> {
    let up = upstream()?;
    let (auth, host, port) = authority(&up)?;
    let head = format!("GET {path_q} HTTP/1.0\r\nHost: {auth}\r\nConnection: close\r\nAccept: application/json\r\n\r\n");
    let exchange = async {
        let mut s = TcpStream::connect((host.as_str(), port)).await?;
        s.write_all(head.as_bytes()).await?;
        let mut raw = Vec::new();
        (&mut s).take(MAX_BODY as u64).read_to_end(&mut raw).await?;
        Ok::<_, std::io::Error>(raw)
    };
    Some(match tokio::time::timeout(Duration::from_secs(30), exchange).await {
        Ok(Ok(raw)) => match parse_response(&raw) {
            Some(p) if p.status == 200 => Ok(p.body),
            Some(p) => Err(format!("Aumentum answered HTTP {}", p.status)),
            None => Err("malformed answer from Aumentum".into()),
        },
        Ok(Err(e)) => Err(format!("Aumentum ({auth}) could not be reached: {e}")),
        Err(_) => Err(format!("Aumentum ({auth}) did not answer in 30 s")),
    })
}

/// Percent-encode a query value.
pub fn pct_encode(v: &str) -> String {
    v.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

/// Fallback handler for `/GISViewer/{*rest}`.
pub async fn forward(State(state): State<AppState>, req: Request) -> Response {
    let Some(up) = upstream() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some((auth, host, port)) = authority(&up) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let method = req.method().clone();
    let is_parcel_query = req.uri().path().eq_ignore_ascii_case(PARCEL_QUERY);
    let query_params = req.uri().query().map(parse_query).unwrap_or_default();
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
    let mut p = p;
    if is_parcel_query && p.status == 200 {
        if let Some(body) = add_own_parcels(&state.pool, &query_params, &p.body).await {
            p.body = body;
        }
    }

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

fn parse_query(q: &str) -> Vec<(String, String)> {
    q.split('&')
        .filter(|kv| !kv.is_empty())
        .map(|kv| {
            let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
            (pct_decode(k), pct_decode(v))
        })
        .collect()
}

fn pct_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => match std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                Some(v) => {
                    out.push(v);
                    i += 2;
                }
                None => out.push(b'%'),
            },
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A simple search `FIELD = 'value'` or `FIELD LIKE 'value'` (upper()/spaces tolerated), as the
/// Web Access viewer sends it. Returns (field, value, is_like).
pub fn parse_where(w: &str) -> Option<(String, String, bool)> {
    let w = w.trim();
    let upper = w.to_ascii_uppercase();
    let (lhs, rhs, like) = if let Some(i) = upper.find(" LIKE ") {
        (&w[..i], &w[i + 6..], true)
    } else {
        let i = w.find('=')?;
        (&w[..i], &w[i + 1..], false)
    };
    let mut field = lhs.trim().to_string();
    for f in ["UPPER(", "upper(", "Upper("] {
        if let Some(x) = field.strip_prefix(f) {
            field = x.trim_end_matches(')').trim().to_string();
        }
    }
    let rhs = rhs.trim();
    let rhs = rhs.strip_prefix("UPPER(").or_else(|| rhs.strip_prefix("upper(")).map(|x| x.trim_end_matches(')').trim()).unwrap_or(rhs);
    let value = rhs.strip_prefix('\'')?.strip_suffix('\'')?.replace("''", "'");
    // The value is bound as a parameter and the field must map to a known column (own_column).
    if field.is_empty() || field.contains(' ') {
        return None;
    }
    Some((field, value, like))
}

/// Our column for a Web Access parcel-layer field.
fn own_column(field: &str) -> Option<&'static str> {
    Some(match field.to_ascii_lowercase().as_str() {
        "upin" => "p.parcel_number",
        "propertyno" => "a.property_number",
        "plotno" => "a.plot",
        "blockno" => "a.block",
        "planno" => "a.plan_number",
        "parcelname" => "a.parcel_name",
        _ => return None,
    })
}

/// When Aumentum found nothing for a simple search, add this Cadastre's posted parcels.
async fn add_own_parcels(pool: &sqlx::PgPool, params: &[(String, String)], body: &[u8]) -> Option<Vec<u8>> {
    let get = |k: &str| params.iter().find(|(n, _)| n.eq_ignore_ascii_case(k)).map(|(_, v)| v.as_str());
    let (field, value, like) = parse_where(get("where")?)?;
    let col = own_column(&field)?;
    let mut doc: Value = serde_json::from_slice(body).ok()?;
    if doc.get("features").and_then(|f| f.as_array()).is_none_or(|f| !f.is_empty()) {
        return None;
    }
    let with_geom = get("returnGeometry").is_none_or(|v| !v.eq_ignore_ascii_case("false"));
    let op = if like { "ILIKE" } else { "=" };
    let sql = format!(
        "SELECT a.object_id, a.parcel_id::text AS parcel_id, a.parcel_name, p.parcel_number AS upin, a.parent, \
                (EXTRACT(EPOCH FROM COALESCE(p.posted_at, a.creation_date AT TIME ZONE 'Africa/Lagos', p.created_at)) * 1000)::bigint AS created, \
                (EXTRACT(EPOCH FROM GREATEST(p.posted_at, a.last_edit_date AT TIME ZONE 'Africa/Lagos', p.updated_at)) * 1000)::bigint AS edited, a.mutation_type::text AS mutation_type, \
                a.plan_number, a.plot, a.block, a.property_number, ST_Area(p.geom) AS area, ST_Perimeter(p.geom) AS len, \
                ST_AsGeoJSON(ST_Transform(p.geom, 32632))::text AS gj \
         FROM cadastre.parcel p \
         LEFT JOIN LATERAL (SELECT * FROM cadastre.parcel_attribute x WHERE x.upin = p.parcel_number \
              ORDER BY (x.status = '0') DESC NULLS LAST, x.object_id DESC LIMIT 1) a ON true \
         WHERE p.status = 'posted' AND p.geom IS NOT NULL AND {col} {op} $1 LIMIT 50"
    );
    let rows = match sqlx::query(&sql).bind(&value).fetch_all(pool).await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("Web Access map search {field}={value}: {e}");
            return None;
        }
    };
    if rows.is_empty() {
        return None;
    }
    let mut features = Vec::new();
    for r in &rows {
        let mt: Option<String> = r.get("mutation_type");
        let mut f = json!({ "attributes": {
            "OBJECTID": r.get::<Option<i32>, _>("object_id"),
            "ParcelID": r.get::<Option<String>, _>("parcel_id").map(|g| format!("{{{}}}", g.to_uppercase())),
            "TempID": Value::Null,
            "ParcelName": r.get::<Option<String>, _>("parcel_name"),
            "UPIN": r.get::<Option<String>, _>("upin"),
            "ParcelLabel": Value::Null,
            "Status": 2,
            "JobID": Value::Null,
            "Parent": r.get::<Option<String>, _>("parent"),
            "CreateDate": r.get::<Option<i64>, _>("created"),
            "LastEditDate": r.get::<Option<i64>, _>("edited"),
            "ModificationType": mt.and_then(|m| m.trim().parse::<i64>().ok()).unwrap_or(0),
            "PlanNo": r.get::<Option<String>, _>("plan_number"),
            "CadZone": Value::Null, "Layout": Value::Null,
            "PlotNo": r.get::<Option<String>, _>("plot"),
            "BlockNo": r.get::<Option<String>, _>("block"),
            "propertyNo": r.get::<Option<String>, _>("property_number"),
            "SHAPE.STArea()": r.get::<Option<f64>, _>("area"),
            "SHAPE.STLength()": r.get::<Option<f64>, _>("len"),
        }});
        if with_geom {
            if let Some(g) = r.get::<Option<String>, _>("gj").and_then(|g| esri_polygon(&g)) {
                f["geometry"] = g;
            }
        }
        features.push(f);
    }
    tracing::info!("Web Access map search {field}={value}: Aumentum had none, {} from this Cadastre", features.len());
    doc["features"] = Value::Array(features);
    serde_json::to_vec(&doc).ok()
}

/// GeoJSON (Multi)Polygon to an Esri polygon (`rings`).
pub fn esri_polygon(geojson: &str) -> Option<Value> {
    let g: Value = serde_json::from_str(geojson).ok()?;
    let coords = g.get("coordinates")?;
    let rings: Vec<Value> = match g.get("type")?.as_str()? {
        "Polygon" => coords.as_array()?.clone(),
        "MultiPolygon" => coords.as_array()?.iter().filter_map(|p| p.as_array()).flatten().cloned().collect(),
        _ => return None,
    };
    Some(json!({ "rings": rings, "spatialReference": { "wkid": 32632 } }))
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
    fn reads_the_viewer_search() {
        let q = parse_query("returnGeometry=true&where=UPIN%3d'PL000000023899'&outSR=32632");
        assert_eq!(q[1], ("where".into(), "UPIN='PL000000023899'".into()));
        assert_eq!(parse_where("UPIN='PL000000023899'"), Some(("UPIN".into(), "PL000000023899".into(), false)));
        assert_eq!(parse_where("upper(propertyNo) LIKE '%PL67363%'"), Some(("propertyNo".into(), "%PL67363%".into(), true)));
        assert_eq!(parse_where("1=1"), None);
        assert_eq!(parse_where("UPIN='a' OR 1=1"), None);
    }

    #[test]
    fn polygon_to_esri() {
        let g = esri_polygon(r#"{"type":"MultiPolygon","coordinates":[[[[0,0],[1,0],[1,1],[0,0]]]]}"#).unwrap();
        assert_eq!(g["rings"][0][1], json!([1, 0]));
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
