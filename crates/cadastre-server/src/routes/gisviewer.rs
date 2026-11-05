//! The Registry's map, served by this Cadastre in place of Aumentum's GISViewer
//! (`http://10.10.10.6/GISViewer/MapWorker.aspx`, Registry settings `GISServer` / `LWAMapURL`).
//!
//! The Registry admin links a Registry property to a parcel in the Registry desktop
//! (`ThomsonReuters.Aumentum.Registry.PLAGIS.RegistryDesktop`, Properties page). Its
//! `MapWorkerControl` (decompiled, `ThomsonReuters.Aumentum.Registry.Services.MapWorker`) hosts an
//! embedded Internet Explorer and:
//! * POSTs to `{GISServer}?ct=indexer&justmap=1&map=property-select&lang=…` the form
//!   `auth=1&lrsuser=…&lrsticket=…&initparam_inputobjects=…&initparam_extent=…`;
//! * waits for an element `silverlightControl` (and `indexerEvents`, whose value becomes
//!   `Map Ready` with a click);
//! * calls the page's `doAction(object, method, a, b, c)`: `SelectionSet.GetSelectedObjects`
//!   (the parcels the admin selected), `Map.GetExtentAsString`, `Editor.IsEditInProgress`;
//! * copies the selected parcel's UPIN into the Registry property itself (GISRetrievedFields) and
//!   writes the Registry's values onto the parcel through
//!   `{GISServer}/../WebServices/CadastralWebService.asmx` → `UpdateCadastralData`
//!   (GISUpdatedFields: LRSID, LinkageStatus, LinkageDate, …), signed with the user and ticket.
//!
//! Objects travel as JSON (.NET DataContractJsonSerializer), base64, wrapped `[BEGIN]…[END]`.
//! Here the map is a Leaflet page in ES5 (the Registry's browser is IE11); an entity's
//! `ObjectID` is the parcel's attribute record OBJECTID (Aumentum's own, preserved), on the map
//! and in `UpdateCadastralData` alike.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::body::Bytes;
use axum::extract::{Query, RawQuery, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use base64::Engine;
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;

use crate::audit;
use crate::lrs::{self, escape, LrsConfig};
use crate::state::AppState;

pub const PAGE: &str = "/GISViewer/MapWorker.aspx";
pub const SERVICE: &str = "/GISViewer/WebServices/CadastralWebService.asmx";
const NS: &str = "http://www.landsystems.com/multicadastre/in4/service";

/// Map tokens (page → its data requests), 8 hours, as a Registry user's map session.
fn tokens() -> &'static Mutex<HashMap<String, (String, Instant)>> {
    static T: OnceLock<Mutex<HashMap<String, (String, Instant)>>> = OnceLock::new();
    T.get_or_init(|| Mutex::new(HashMap::new()))
}
fn token_user(t: &str) -> Option<String> {
    let mut m = tokens().lock().unwrap();
    m.retain(|_, (_, at)| at.elapsed() < Duration::from_secs(8 * 3600));
    m.get(t).map(|(u, _)| u.clone())
}

/// `application/x-www-form-urlencoded` → pairs.
pub fn parse_form(s: &str) -> HashMap<String, String> {
    fn dec(v: &str) -> String {
        let b = v.replace('+', " ");
        let bytes = b.as_bytes();
        let mut out = Vec::with_capacity(bytes.len());
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'%' && i + 2 < bytes.len() {
                if let Ok(x) = u8::from_str_radix(&b[i + 1..i + 3], 16) {
                    out.push(x);
                    i += 3;
                    continue;
                }
            }
            out.push(bytes[i]);
            i += 1;
        }
        String::from_utf8_lossy(&out).to_string()
    }
    s.split('&').filter(|p| !p.is_empty()).map(|p| match p.split_once('=') { Some((k, v)) => (dec(k), dec(v)), None => (dec(p), String::new()) }).collect()
}

/// The Registry's encoding (`SerializerUtils`): `<base64Binary …>` or `[BEGIN]`, base64, `[END]`.
pub fn decode_aumentum(s: &str) -> Option<Value> {
    let s = s.trim();
    let body = if let Some(i) = s.find("[BEGIN]") { &s[i + 7..] } else if s.starts_with("<base64Binary") { &s[s.find('>')? + 1..] } else { s };
    let body = body.trim_end().trim_end_matches("[END]").trim_end_matches("</base64Binary>").trim();
    let bytes = base64::engine::general_purpose::STANDARD.decode(body.replace(['\r', '\n', ' '], "")).ok()?;
    let text = String::from_utf8(bytes).ok()?;
    serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()
}
pub fn encode_aumentum(v: &Value) -> String {
    format!("[BEGIN]{}[END]", base64::engine::general_purpose::STANDARD.encode(v.to_string()))
}

fn registry_cfg() -> Option<LrsConfig> {
    LrsConfig::from_env()
}

/// Development switch: open the map without a Registry ticket (never in production).
fn open_without_ticket() -> bool {
    std::env::var("CADASTRE_GISVIEWER_OPEN").map(|v| v.trim() == "1").unwrap_or(false)
}

/// `GET|POST /GISViewer/MapWorker.aspx`: the Registry's map page.
pub async fn page(RawQuery(q): RawQuery, headers: HeaderMap, body: Bytes) -> Response {
    let query = parse_form(q.as_deref().unwrap_or(""));
    let is_form = headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).is_some_and(|v| v.starts_with("application/x-www-form-urlencoded"));
    let form = if is_form || !body.is_empty() { parse_form(&String::from_utf8_lossy(&body)) } else { HashMap::new() };
    let get = |k: &str| form.get(k).or_else(|| query.get(k)).cloned().unwrap_or_default();
    let (user, ticket) = (get("lrsuser"), get("lrsticket"));
    let authorised = if open_without_ticket() {
        Ok(if user.is_empty() { "dev".to_string() } else { user.clone() })
    } else if user.is_empty() || ticket.is_empty() {
        Err("The map is opened from the Registry (no Registry user / ticket was given).".to_string())
    } else {
        match registry_cfg() {
            None => Err("The Registry connection is not configured on this Cadastre.".to_string()),
            Some(cfg) => match lrs::ticket_valid(&cfg, &user, &ticket).await {
                Ok(true) => Ok(user.clone()),
                Ok(false) => Err(format!("The Registry session of {user} is not valid.")),
                Err(e) => Err(format!("The Registry could not be reached: {e}")),
            },
        }
    };
    match authorised {
        Err(msg) => {
            tracing::warn!("Registry map refused: {msg}");
            Html(format!("<!DOCTYPE html><html><head><meta http-equiv=\"X-UA-Compatible\" content=\"IE=edge\"><title>Cadastre map</title></head>\
                <body style=\"font-family:Segoe UI,Arial;padding:20px\"><h3>Cadastre map</h3><p>{}</p></body></html>", escape(&msg))).into_response()
        }
        Ok(u) => {
            tracing::info!("Registry map opened for {u} (map={}, {} input byte(s))", get("map"), get("initparam_inputobjects").len());
            let token = uuid::Uuid::new_v4().simple().to_string();
            tokens().lock().unwrap().insert(token.clone(), (u.clone(), Instant::now()));
            let input = decode_aumentum(&get("initparam_inputobjects")).unwrap_or(json!([]));
            let extent = decode_aumentum(&get("initparam_extent")).unwrap_or(Value::Null);
            let init = json!({
                "token": token, "user": u, "map": get("map"), "lang": get("lang"),                 "input": input, "extent": extent,
            });
            let init = init.to_string().replace("</", "<\\/");
            ([("X-UA-Compatible", "IE=edge"), (header::CACHE_CONTROL.as_str(), "no-store")], Html(PAGE_HTML.replace("__INIT__", &init))).into_response()
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct DataQuery {
    pub token: String,
    /// minLon,minLat,maxLon,maxLat
    pub bbox: Option<String>,
    /// Search: a GIS field (UPIN, LRSID, PropertyNumber, Plot, Block, ParcelID, OBJECTID) and its value.
    pub field: Option<String>,
    pub value: Option<String>,
}

fn search_column(field: &str) -> Option<&'static str> {
    Some(match field.to_ascii_lowercase().as_str() {
        "upin" => "a.upin",
        "lrsid" | "suid" | "uid" => "a.lrs_id",
        "propertynumber" | "refnum" => "a.property_number",
        "plot" | "lotnumber" => "a.plot",
        "block" | "blocknumber" => "a.block",
        "parcelid" => "a.parcel_id::text",
        "objectid" => "a.object_id::text",
        _ => return None,
    })
}

/// `GET /GISViewer/data/parcels`: parcels in view or matching a search, for the Registry's map.
pub async fn data(State(state): State<AppState>, Query(q): Query<DataQuery>) -> Response {
    if token_user(&q.token).is_none() {
        return (StatusCode::UNAUTHORIZED, "map session expired; reopen the map from the Registry").into_response();
    }
    let base = "SELECT a.object_id, p.parcel_number AS upin, a.lrs_id, a.linkage_status, a.property_number, a.plot, a.block, a.parcel_id::text AS parcel_id, \
                round(ST_Area(p.geom)::numeric, 2)::float8 AS area, ST_AsGeoJSON(ST_Transform(p.geom, 4326), 7) AS g \
                FROM cadastre.parcel p JOIN LATERAL (SELECT * FROM cadastre.parcel_attribute x WHERE x.upin = p.parcel_number \
                  ORDER BY (x.status = '0') DESC NULLS LAST, x.object_id DESC LIMIT 1) a ON true WHERE p.status = 'posted'";
    let rows = if let (Some(f), Some(v)) = (q.field.as_deref(), q.value.as_deref().map(str::trim).filter(|v| !v.is_empty())) {
        let Some(col) = search_column(f) else { return (StatusCode::BAD_REQUEST, "unknown field").into_response() };
        sqlx::query(&format!("{base} AND upper({col}) = upper($1) LIMIT 50")).bind(v).fetch_all(&state.pool).await
    } else if let Some(b) = q.bbox.as_deref() {
        let n: Vec<f64> = b.split(',').filter_map(|x| x.trim().parse().ok()).collect();
        if n.len() != 4 {
            return (StatusCode::BAD_REQUEST, "bbox").into_response();
        }
        sqlx::query(&format!("{base} AND p.geom && ST_Transform(ST_MakeEnvelope($1, $2, $3, $4, 4326), 32632) LIMIT 4000"))
            .bind(n[0]).bind(n[1]).bind(n[2]).bind(n[3]).fetch_all(&state.pool).await
    } else {
        return (StatusCode::BAD_REQUEST, "bbox or field/value").into_response();
    };
    let rows = match rows {
        Ok(r) => r,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    let features: Vec<Value> = rows.iter().map(|r| json!({
        "type": "Feature",
        "geometry": serde_json::from_str::<Value>(&r.get::<String, _>("g")).unwrap_or(Value::Null),
        "properties": {
            "OBJECTID": r.get::<i32, _>("object_id"), "UPIN": r.get::<Option<String>, _>("upin"), "LRSID": r.get::<Option<String>, _>("lrs_id"),
            "LinkageStatus": r.get::<Option<i16>, _>("linkage_status"), "PropertyNumber": r.get::<Option<String>, _>("property_number"),
            "Plot": r.get::<Option<String>, _>("plot"), "Block": r.get::<Option<String>, _>("block"), "ParcelID": r.get::<Option<String>, _>("parcel_id"),
            "Area": r.get::<Option<f64>, _>("area"),
        }
    })).collect();
    ([(header::CONTENT_TYPE, "application/json"), (header::CACHE_CONTROL, "no-store")], json!({ "type": "FeatureCollection", "features": features }).to_string()).into_response()
}

// ---------------------------------------------------------------------------------------------
// CadastralWebService.asmx
// ---------------------------------------------------------------------------------------------

/// One `UpdateRecord` of `UpdateCadastralData`.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct UpdateRecord {
    /// `UpdateRecordByOID` / `UpdateRecordByFields`.
    pub kind: String,
    pub object_id: Option<i32>,
    pub values: Vec<(String, Option<String>)>,
    pub primary: Vec<(String, Option<String>)>,
}
#[derive(Debug, Default, Clone, PartialEq)]
pub struct UpdateDataList {
    pub layer: String,
    pub sde_alias: Option<String>,
    pub records: Vec<UpdateRecord>,
    pub exclude: Vec<i32>,
}

/// Parse a CadastralWebService request: the operation, the SecurityHeader (user, ticket) and,
/// for UpdateCadastralData, its lists.
pub fn parse_service_request(xml: &str) -> Option<(String, Option<(String, String)>, Vec<UpdateDataList>)> {
    use quick_xml::events::Event;
    let mut r = quick_xml::Reader::from_str(xml);
    let mut path: Vec<String> = Vec::new();
    let mut op: Option<String> = None;
    let (mut user, mut pass) = (None::<String>, None::<String>);
    let mut lists: Vec<UpdateDataList> = Vec::new();
    let mut rec: Option<UpdateRecord> = None;
    let mut val: Option<(Option<String>, Option<String>, bool)> = None; // field, value, nil
    let mut text = String::new();
    loop {
        let ev = r.read_event();
        match ev {
            Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e)) => {
                let empty = matches!(ev, Ok(Event::Empty(_)));
                let name = String::from_utf8_lossy(e.local_name().as_ref()).to_string();
                let xsi_type = e.attributes().flatten().find(|a| a.key.local_name().as_ref() == b"type").map(|a| String::from_utf8_lossy(&a.value).to_string());
                let nil = e.attributes().flatten().any(|a| a.key.local_name().as_ref() == b"nil" && a.value.as_ref() == b"true");
                if path.last().map(String::as_str) == Some("Body") && op.is_none() {
                    op = Some(name.clone());
                }
                match name.as_str() {
                    "UpdateDataList" => lists.push(UpdateDataList::default()),
                    "UpdateRecord" | "UpdateRecordByOID" | "UpdateRecordByFields" => {
                        let kind = xsi_type.map(|t| t.rsplit(':').next().unwrap_or("").to_string()).unwrap_or_else(|| name.clone());
                        rec = Some(UpdateRecord { kind, ..Default::default() });
                    }
                    "UpdateValue" => val = Some((None, None, false)),
                    "Value" if val.is_some() && nil => { if let Some(v) = val.as_mut() { v.2 = true; } }
                    _ => {}
                }
                text.clear();
                if !empty { path.push(name); } else if name == "UpdateValue" {
                    val = None;
                }
            }
            Ok(Event::Text(t)) => text.push_str(&t.unescape().ok()?),
            Ok(Event::End(e)) => {
                let name = String::from_utf8_lossy(e.local_name().as_ref()).to_string();
                let parent = path.len().checked_sub(2).and_then(|i| path.get(i)).cloned().unwrap_or_default();
                let t = text.trim().to_string();
                match name.as_str() {
                    "User" if parent == "SecurityHeader" => user = Some(t.clone()),
                    "Password" if parent == "SecurityHeader" => pass = Some(t.clone()),
                    "LayerNameOrAlias" => { if let Some(l) = lists.last_mut() { l.layer = t.clone(); } }
                    "SdeAlias" => { if let Some(l) = lists.last_mut() { l.sde_alias = Some(t.clone()).filter(|s| !s.is_empty()); } }
                    "int" if parent == "ExculdeFeatures" => { if let (Some(l), Ok(n)) = (lists.last_mut(), t.parse()) { l.exclude.push(n); } }
                    "ObjectID" if rec.is_some() => { if let Some(rc) = rec.as_mut() { rc.object_id = t.parse().ok(); } }
                    "Field" if val.is_some() => { if let Some(v) = val.as_mut() { v.0 = Some(t.clone()); } }
                    "Value" if val.is_some() => { if let Some(v) = val.as_mut() { if !v.2 { v.1 = Some(t.clone()); } } }
                    "UpdateValue" => {
                        if let (Some((Some(f), v, nil)), Some(rc)) = (val.take(), rec.as_mut()) {
                            let v = if nil { None } else { v };
                            if path.iter().any(|p| p == "PrimaryFields") { rc.primary.push((f, v)); } else { rc.values.push((f, v)); }
                        }
                    }
                    "UpdateRecord" | "UpdateRecordByOID" | "UpdateRecordByFields" => {
                        if let (Some(rc), Some(l)) = (rec.take(), lists.last_mut()) { l.records.push(rc); }
                    }
                    _ => {}
                }
                text.clear();
                path.pop();
            }
            Ok(Event::Eof) => break,
            Err(_) => return None,
            _ => {}
        }
    }
    let header = match (user, pass) { (Some(u), Some(p)) => Some((u, p)), _ => None };
    op.map(|o| (o, header, lists))
}

fn soap_ok(op: &str, inner: &str) -> Response {
    let body = format!("<?xml version=\"1.0\" encoding=\"utf-8\"?><soap:Envelope xmlns:soap=\"http://schemas.xmlsoap.org/soap/envelope/\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xmlns:xsd=\"http://www.w3.org/2001/XMLSchema\"><soap:Body><{op}Response xmlns=\"{NS}\">{inner}</{op}Response></soap:Body></soap:Envelope>");
    ([(header::CONTENT_TYPE, "text/xml; charset=utf-8")], body).into_response()
}

/// Apply UpdateCadastralData: by OBJECTID (the attribute record) or by matching primary fields,
/// the excluded features left out. Returns (records changed, UPINs touched).
pub async fn apply_updates(pool: &sqlx::PgPool, lists: &[UpdateDataList], actor: &str) -> Result<(u64, Vec<String>), String> {
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    let mut changed = 0;
    let mut upins: Vec<String> = Vec::new();
    for l in lists {
        for r in &l.records {
            let targets: Vec<(i32, Option<String>)> = if let Some(oid) = r.object_id.filter(|_| r.kind.ends_with("ByOID") || r.primary.is_empty()) {
                sqlx::query_as("SELECT object_id, upin FROM cadastre.parcel_attribute WHERE object_id = $1").bind(oid).fetch_all(&mut *tx).await.map_err(|e| e.to_string())?
            } else {
                let mut qb = sqlx::QueryBuilder::<sqlx::Postgres>::new("SELECT object_id, upin FROM cadastre.parcel_attribute WHERE (status = '0' OR status IS NULL)");
                let mut any = false;
                for (f, v) in &r.primary {
                    let col = if f.eq_ignore_ascii_case("UPIN") { Some("upin") } else { crate::registry_sync::attribute_column(f).map(|c| c.0) };
                    let Some(col) = col else { continue };
                    any = true;
                    match v { Some(v) => { qb.push(format!(" AND {col}::text = ")).push_bind(v.clone()); } None => { qb.push(format!(" AND {col} IS NULL")); } }
                }
                if !any { continue; }
                if !l.exclude.is_empty() { qb.push(" AND NOT (object_id = ANY(").push_bind(l.exclude.clone()).push("))"); }
                qb.build_query_as().fetch_all(&mut *tx).await.map_err(|e| e.to_string())?
            };
            for (oid, upin) in targets {
                let cols: Vec<(&'static str, bool, Option<String>)> = r.values.iter().filter_map(|(f, v)| crate::registry_sync::attribute_column(f).map(|(c, s)| (c, s, v.clone()))).collect();
                if cols.is_empty() { continue; }
                let mut qb = sqlx::QueryBuilder::<sqlx::Postgres>::new("UPDATE cadastre.parcel_attribute SET last_edit_date = now()::timestamp, updated_at = now()");
                for (c, small, v) in &cols {
                    qb.push(format!(", {c} = "));
                    if *small { qb.push_bind(v.as_ref().and_then(|s| s.trim().parse::<i16>().ok())); } else { qb.push_bind(v.clone()); }
                }
                qb.push(" WHERE object_id = ").push_bind(oid);
                changed += qb.build().execute(&mut *tx).await.map_err(|e| e.to_string())?.rows_affected();
                if let Some(u) = upin { if !upins.contains(&u) { upins.push(u); } }
            }
        }
    }
    for u in &upins {
        audit::append(&mut tx, actor, "registry_map_update", "registry_link", Some(u), json!({ "lists": lists.iter().map(|l| json!({
            "layer": l.layer, "records": l.records.iter().map(|r| json!({ "kind": r.kind, "object_id": r.object_id, "values": r.values, "primary": r.primary })).collect::<Vec<_>>(),
            "exclude": l.exclude })).collect::<Vec<_>>() })).await.map_err(|e| e.to_string())?;
    }
    tx.commit().await.map_err(|e| e.to_string())?;
    Ok((changed, upins))
}

/// The one link an OK on the map makes: a record by OBJECTID with LinkageStatus 1 and an LRSID. The
/// desktop's list that clears a UPIN from other parcels has empty values, so it never counts.
#[derive(Debug, PartialEq)]
pub struct MapOkLink {
    pub object_id: i32,
    pub su_id: String,
    pub property_number: Option<String>,
}

/// `Ok(None)`: no link in this call. `Err`: more than one, which is left to the Registry's own save.
pub fn map_ok_link(lists: &[UpdateDataList]) -> Result<Option<MapOkLink>, String> {
    fn value<'a>(r: &'a UpdateRecord, field: &str) -> Option<&'a str> {
        r.values.iter().find(|(f, _)| f.eq_ignore_ascii_case(field)).and_then(|(_, v)| v.as_deref()).map(str::trim).filter(|v| !v.is_empty())
    }
    let mut links: Vec<MapOkLink> = lists.iter().flat_map(|l| &l.records)
        .filter(|r| r.kind.ends_with("ByOID") && value(r, "LinkageStatus") == Some("1"))
        .filter_map(|r| Some(MapOkLink { object_id: r.object_id?, su_id: value(r, "LRSID")?.to_string(), property_number: value(r, "PropertyNumber").map(str::to_string) }))
        .collect();
    match links.len() {
        0 => Ok(None),
        1 => Ok(links.pop()),
        n => Err(format!("OK linked {n} parcels at once: save the file in the Registry")),
    }
}

/// The Registry's half of an OK (`CADASTRE_REGISTRY_LINK_ON_MAP_OK`), after the Cadastre's half stood.
async fn write_registry_half(state: &AppState, lists: &[UpdateDataList], actor: &str, upins: &[String]) {
    let link = match map_ok_link(lists) {
        Ok(Some(l)) => l,
        Ok(None) => return,
        Err(why) => {
            for u in upins { crate::routes::registry::map_ok_not_written(state, actor, u, None, &why).await; }
            return;
        }
    };
    let upin: Option<String> = sqlx::query_scalar("SELECT upin FROM cadastre.parcel_attribute WHERE object_id = $1 AND lrs_id = $2 AND linkage_status = 1")
        .bind(link.object_id).bind(&link.su_id).fetch_optional(&state.pool).await.ok().flatten();
    match upin {
        Some(u) => crate::routes::registry::link_after_map_ok(state, actor, &link.su_id, link.property_number.as_deref(), &u).await,
        None => tracing::warn!("Registry map: no linked parcel record {} for {}: nothing written to the Registry", link.object_id, link.su_id),
    }
}

/// `POST /GISViewer/WebServices/CadastralWebService.asmx`.
pub async fn service(State(state): State<AppState>, body: Bytes) -> Response {
    let text = String::from_utf8_lossy(&body);
    let Some((op, header_auth, lists)) = parse_service_request(&text) else {
        tracing::warn!("Registry link service: not a SOAP request ({} bytes)", body.len());
        return (StatusCode::BAD_REQUEST, "not a SOAP request").into_response();
    };
    tracing::info!("Registry link service: {op} called ({} update list(s))", lists.len());
    // Aumentum's CadastralWebService checks the Registry user's ticket (AbstractSecurityWebService).
    let user = match (&header_auth, open_without_ticket()) {
        (_, true) => header_auth.as_ref().map(|h| h.0.clone()).unwrap_or_else(|| "dev".into()),
        (Some((u, t)), false) => match registry_cfg() {
            Some(cfg) => match lrs::ticket_valid(&cfg, u, t).await {
                Ok(true) => u.clone(),
                other => {
                    tracing::warn!("Registry link service: {op} refused, the Registry session of {u} is not valid ({other:?})");
                    return soap_ok(&op, &format!("<{op}Result><Error>The Registry session of {} is not valid.</Error></{op}Result>", escape(u)));
                }
            },
            None => return soap_ok(&op, &format!("<{op}Result><Error>The Registry connection is not configured on this Cadastre.</Error></{op}Result>")),
        },
        (None, false) => {
            tracing::warn!("Registry link service: {op} refused, no SecurityHeader");
            return soap_ok(&op, &format!("<{op}Result><Error>No SecurityHeader.</Error></{op}Result>"));
        }
    };
    match op.as_str() {
        "UpdateCadastralData" => match apply_updates(&state.pool, &lists, &format!("registry:{user}")).await {
            Ok((n, upins)) => {
                tracing::info!("Registry map: {n} parcel record(s) updated by {user} ({})", upins.join(", "));
                if crate::routes::registry::link_on_map_ok() {
                    let (state, lists, actor) = (state.clone(), lists.clone(), format!("registry:{user}"));
                    tokio::spawn(async move { write_registry_half(&state, &lists, &actor, &upins).await });
                }
                soap_ok(&op, "<UpdateCadastralDataResult />")
            }
            Err(e) => {
                tracing::warn!("Registry link service: UpdateCadastralData by {user} failed: {e}");
                soap_ok(&op, &format!("<UpdateCadastralDataResult><Error>{}</Error></UpdateCadastralDataResult>", escape(&e)))
            }
        },
        "GetVariables" => soap_ok(&op, "<GetVariablesResult />"),
        "DetectNeighbours" => soap_ok(&op, "<DetectNeighboursResult />"),
        _ => soap_ok(&op, &format!("<{op}Result><Error>Unknown operation {}</Error></{op}Result>", escape(&op))),
    }
}

pub async fn service_get() -> Response {
    ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], "<html><body><h3>CadastralWebService</h3><p>The Cadastre's map service for the Aumentum Registry.</p></body></html>").into_response()
}

const PAGE_HTML: &str = r##"<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8">
<meta http-equiv="X-UA-Compatible" content="IE=edge">
<title>Cadastre map</title>
<link rel="stylesheet" href="/editor/vendor/leaflet/leaflet.css">
<style>
html, body { margin: 0; height: 100%; font-family: "Segoe UI", Tahoma, Arial, sans-serif; font-size: 12px; color: #1e1e1e; }
#side { position: absolute; left: 0; top: 0; bottom: 0; width: 290px; overflow-y: auto; background: #f4f6f9; border-right: 1px solid #c5ccd6; }
#silverlightControl { position: absolute; left: 291px; right: 0; top: 0; bottom: 0; }
.hd { background: #e9eef5; padding: 8px 10px; border-bottom: 1px solid #c5ccd6; }
.hd b { font-size: 13px; }
.sec { padding: 8px 10px; border-bottom: 1px solid #e1e4e8; }
.sec h4 { margin: 0 0 6px; font-size: 11px; text-transform: uppercase; color: #666; letter-spacing: .04em; }
.row { display: block; padding: 5px 7px; margin: 3px 0; background: #fff; border: 1px solid #dde2e8; border-left: 3px solid #d08000; cursor: pointer; }
.row.sel { border-left-color: #1f9ad6; background: #e8f5fc; }
.row.lnk { border-left-color: #3a86d6; }
.row .m { color: #777; }
input[type=text] { width: 168px; padding: 3px; border: 1px solid #b8b8b8; }
select { padding: 2px; }
button { padding: 3px 8px; cursor: pointer; }
.muted { color: #888; }
.tag { display: inline-block; padding: 0 6px; border-radius: 8px; font-size: 10.5px; border: 1px solid #8cc79a; color: #1a6b2a; background: #ecf8ef; }
.tag.no { border-color: #e0b04a; color: #8a5a00; background: #fff6df; }
</style>
</head>
<body>
<div id="side">
  <div class="hd"><b>Cadastre map</b><div class="muted" id="who"></div></div>
  <div class="sec" id="inputSec"><h4>Registry properties</h4><div id="inputs"></div></div>
  <div class="sec"><h4>Find a parcel</h4>
    <select id="sf"><option value="UPIN">UPIN</option><option value="PropertyNumber">Property No.</option><option value="LRSID">LRS ID</option><option value="Plot">Plot</option><option value="Block">Block</option></select>
    <input type="text" id="sv" placeholder="e.g. PL000000023901"> <button id="sb">Find</button>
    <div id="found"></div>
  </div>
  <div class="sec"><h4>Selected <span id="selCount"></span></h4><div id="selected" class="muted">Click parcels on the map to select them (click again to deselect).</div>
    <div id="okHint" class="muted" style="margin-top:6px">Press OK below to link, then Suspend in the Registry to save the file.</div></div>
</div>
<div id="silverlightControl"></div>
<input type="hidden" id="indexerEvents" value="">
<input type="hidden" id="mapErrors" value="">
<script>
// Script errors, kept for the Registry simulator (and support) to read.
window.onerror = function (msg, src, line) { try { var e = document.getElementById("mapErrors"); e.value += msg + " (" + String(src || "").split("/").pop() + ":" + line + "); "; } catch (x) {} };
// IE11 lacks Number.isFinite, which proj4 uses.
if (!Number.isFinite) { Number.isFinite = function (v) { return typeof v === "number" && isFinite(v); }; }
</script>
<script src="/editor/vendor/leaflet/leaflet.js"></script>
<script src="/editor/vendor/proj4.js"></script>
<script>
// ES5 only: the Registry's map window is Internet Explorer 11.
var INIT = __INIT__;
var UTM = "+proj=utm +zone=32 +datum=WGS84 +units=m +no_defs";
var sel = {};          // OBJECTID -> { f: feature, uuid: InputUUID, group: layer }
var current = { group: 0, uuid: -1 };
var groups = (INIT.input && INIT.input.length) ? INIT.input : [];
var layerName = groups.length ? (groups[0].LayerName || "Plateau_State_Parcels") : "Plateau_State_Parcels";
if (groups.length && groups[0].Entities && groups[0].Entities.length) current.uuid = groups[0].Entities[0].UUID === undefined ? 0 : groups[0].Entities[0].UUID;
var readOnly = groups.length > 0 && (function () { for (var i = 0; i < groups.length; i++) if (!groups[i].IsReadOnly) return false; return true; })();
function $(id) { return document.getElementById(id); }
function encode(obj) { return "[BEGIN]" + window.btoa(unescape(encodeURIComponent(JSON.stringify(obj)))) + "[END]"; }
function get(url, cb) {
  var x = new XMLHttpRequest(); x.open("GET", url, true);
  x.onreadystatechange = function () { if (x.readyState === 4) { if (x.status === 200) { try { cb(JSON.parse(x.responseText)); } catch (e) { cb(null); } } else { cb(null, x.status); } } };
  x.send();
}
function text(s) { return String(s === null || s === undefined ? "" : s).replace(/[&<>"]/g, function (c) { return { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]; }); }
$("who").innerHTML = text("Registry user: " + INIT.user + (INIT.map ? " · map: " + INIT.map : ""));

var map = L.map("silverlightControl", { zoomControl: true }).setView([9.9, 8.9], 12);
// The background imagery comes from the internet. It is added once the page has loaded, so a PC
// without internet (or a slow link) never holds up the page the Registry waits for.
function addImagery() { L.tileLayer("https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/{z}/{y}/{x}", { maxZoom: 21, maxNativeZoom: 17, attribution: "Imagery &copy; Esri" }).addTo(map); }
if (document.readyState === "complete") addImagery();
else if (window.addEventListener) window.addEventListener("load", addImagery, false);
else window.attachEvent("onload", addImagery);
function style(f) {
  var p = f.properties, s = sel[p.OBJECTID];
  return { color: s ? "#00d0ff" : (p.LinkageStatus === 1 ? "#3a86d6" : "#ff9f1a"), weight: s ? 3 : 1.5, fillOpacity: s ? 0.35 : 0.08 };
}
var layer = L.geoJSON(null, { style: style, onEachFeature: function (f, l) {
  var p = f.properties;
  l.bindTooltip(text(p.UPIN) + (p.PropertyNumber ? " · " + text(p.PropertyNumber) : "") + (p.LinkageStatus === 1 ? " (linked " + text(p.LRSID) + ")" : ""), { sticky: true });
  l.on("click", function () { toggle(f); });
} }).addTo(map);
var loaded = {};
function loadView() {
  if (map.getZoom() < 15) return;
  var b = map.getBounds();
  get("/GISViewer/data/parcels?token=" + INIT.token + "&bbox=" + [b.getWest(), b.getSouth(), b.getEast(), b.getNorth()].join(","), function (fc) {
    if (!fc) return;
    for (var i = 0; i < fc.features.length; i++) { var f = fc.features[i]; if (!loaded[f.properties.OBJECTID]) { loaded[f.properties.OBJECTID] = true; layer.addData(f); } }
    layer.setStyle(style);
  });
}
map.on("moveend", loadView);
function addFeatures(fc) { for (var i = 0; i < fc.features.length; i++) { var f = fc.features[i]; if (!loaded[f.properties.OBJECTID]) { loaded[f.properties.OBJECTID] = true; layer.addData(f); } } layer.setStyle(style); }
function zoomTo(fc) { try { var g = L.geoJSON(fc); map.fitBounds(g.getBounds(), { maxZoom: 20, padding: [40, 40] }); } catch (e) {} }

function toggle(f) {
  if (readOnly) return;
  var id = f.properties.OBJECTID;
  if (sel[id]) delete sel[id]; else sel[id] = { f: f, uuid: current.uuid, group: (groups[current.group] && groups[current.group].LayerName) || layerName };
  layer.setStyle(style); renderSelected();
}
function renderSelected() {
  var h = "", n = 0;
  for (var k in sel) if (sel.hasOwnProperty(k)) {
    var p = sel[k].f.properties; n++;
    h += '<div class="row sel">' + text(p.UPIN) + ' <span class="m">' + text(p.PropertyNumber || "") + (sel[k].uuid >= 0 ? " · for property " + (sel[k].uuid + 1) : "") + '</span></div>';
  }
  $("selected").innerHTML = n ? h : "Click parcels on the map to select them (click again to deselect).";
  $("selected").className = n ? "" : "muted";
  $("selCount").innerHTML = n ? "(" + n + ")" : "";
}

// The Registry's input objects: the properties being linked, with their search values.
function attr(entity, name) { var a = entity.Attributes || []; for (var i = 0; i < a.length; i++) if (a[i].Name === name) return a[i].Value; return null; }
function renderInputs() {
  if (!groups.length) { $("inputSec").style.display = "none"; return; }
  var h = "";
  for (var g = 0; g < groups.length; g++) {
    var grp = groups[g], fields = grp.Fields || [], ents = grp.Entities || [];
    for (var e = 0; e < ents.length; e++) {
      var en = ents[e], parts = [];
      for (var i = 0; i < fields.length; i++) { var v = attr(en, fields[i].Name); if (v !== null && v !== "") parts.push(text(fields[i].Alias || fields[i].Name) + ": " + text(v)); }
      var uuid = en.UUID === undefined ? e : en.UUID;
      h += '<label class="row' + (current.group === g && current.uuid === uuid ? " sel" : "") + '"><input type="radio" name="cur" ' + (current.group === g && current.uuid === uuid ? "checked" : "") +
           ' onclick="pick(' + g + ',' + uuid + ')"> Property ' + (e + 1) + ' <span class="m">' + (parts.join(" · ") || "no search values") + "</span></label>";
    }
  }
  $("inputs").innerHTML = h;
}
window.pick = function (g, uuid) { current = { group: g, uuid: uuid }; renderInputs(); };
function searchInputs() {
  var pending = 0, all = { type: "FeatureCollection", features: [] };
  for (var g = 0; g < groups.length; g++) {
    var grp = groups[g], fields = grp.Fields || [], ents = grp.Entities || [];
    for (var e = 0; e < ents.length; e++) {
      for (var i = 0; i < fields.length; i++) {
        if (!fields[i].IsLayerField) continue;
        var v = attr(ents[e], fields[i].Name);
        if (v === null || v === "") continue;
        pending++;
        (function (g, en, uuid, field, value, preselect) {
          get("/GISViewer/data/parcels?token=" + INIT.token + "&field=" + encodeURIComponent(field) + "&value=" + encodeURIComponent(value), function (fc) {
            if (fc && fc.features.length) {
              addFeatures(fc);
              for (var j = 0; j < fc.features.length; j++) {
                all.features.push(fc.features[j]);
                if (preselect && !readOnly) sel[fc.features[j].properties.OBJECTID] = { f: fc.features[j], uuid: uuid, group: groups[g].LayerName || layerName };
              }
            }
            if (--pending === 0) { layer.setStyle(style); renderSelected(); if (all.features.length) zoomTo(all); ready(); }
          });
        })(g, ents[e], ents[e].UUID === undefined ? e : ents[e].UUID, fields[i].Name, v, grp.IsSelectionSet !== false && !ents[e].IsOnlySearchCriteria);
      }
    }
  }
  if (pending === 0) ready();
}
$("sb").onclick = function () {
  get("/GISViewer/data/parcels?token=" + INIT.token + "&field=" + encodeURIComponent($("sf").value) + "&value=" + encodeURIComponent($("sv").value), function (fc) {
    if (!fc || !fc.features.length) { $("found").innerHTML = '<div class="muted">Not found.</div>'; return; }
    addFeatures(fc); zoomTo(fc);
    var h = "";
    for (var i = 0; i < fc.features.length; i++) { var p = fc.features[i].properties; h += '<div class="row' + (p.LinkageStatus === 1 ? " lnk" : "") + '" onclick="zoomOid(' + p.OBJECTID + ')">' + text(p.UPIN) + ' <span class="tag' + (p.LinkageStatus === 1 ? "" : " no") + '">' + (p.LinkageStatus === 1 ? "linked " + text(p.LRSID) : "not linked") + "</span></div>"; }
    $("found").innerHTML = h;
  });
};
$("sv").onkeydown = function (e) { if ((e || window.event).keyCode === 13) $("sb").onclick(); };
window.zoomOid = function (oid) { layer.eachLayer(function (l) { if (l.feature.properties.OBJECTID === oid) { map.fitBounds(l.getBounds(), { maxZoom: 20, padding: [40, 40] }); } }); };

// The Registry's MapWorkerControl calls doAction(object, method, a, b, c).
function selectedEntities() {
  var byGroup = {}, out = [];
  for (var k in sel) if (sel.hasOwnProperty(k)) {
    var s = sel[k], p = s.f.properties, attrs = [];
    var names = ["UPIN", "LRSID", "PropertyNumber", "Plot", "Block", "ParcelID", "LinkageStatus"];
    for (var i = 0; i < names.length; i++) if (p[names[i]] !== null && p[names[i]] !== undefined) attrs.push({ Name: names[i], Value: String(p[names[i]]) });
    if (!byGroup[s.group]) { byGroup[s.group] = { Entities: [], EntitiesIDs: null, EntitiesField: null, LayerName: s.group }; out.push(byGroup[s.group]); }
    byGroup[s.group].Entities.push({ Envelope: null, ObjectID: p.OBJECTID, InputUUID: s.uuid, Attributes: attrs });
  }
  return out;
}
function extentObject() {
  var b = map.getBounds(), sw = proj4(UTM, [b.getWest(), b.getSouth()]), ne = proj4(UTM, [b.getEast(), b.getNorth()]);
  return { MinX: sw[0], MinY: sw[1], MaxX: ne[0], MaxY: ne[1], WKT: null, WKID: 32632 };
}
window.doAction = function (obj, method, a, b, c) {
  try {
    if (obj === "SelectionSet" && method === "GetSelectedObjects") return encode(selectedEntities());
    if (obj === "Map" && method === "GetExtentAsString") return encode(extentObject());
    if (obj === "Editor" && method === "IsEditInProgress") return false;
    if (obj === "SelectionSet" && method === "Clear") { sel = {}; layer.setStyle(style); renderSelected(); return true; }
  } catch (e) { return null; }
  return null;
};
var isReady = false;
function ready() {
  if (isReady) return; isReady = true;
  var ev = $("indexerEvents"); ev.value = "Map Ready";
  try { ev.click(); } catch (e) {}
}
if (INIT.extent && INIT.extent.MinX) {
  try { var sw = proj4(UTM, "WGS84", [INIT.extent.MinX, INIT.extent.MinY]), ne = proj4(UTM, "WGS84", [INIT.extent.MaxX, INIT.extent.MaxY]); map.fitBounds([[sw[1], sw[0]], [ne[1], ne[0]]]); } catch (e) {}
}
if (readOnly) $("okHint").style.display = "none";
renderInputs();
searchInputs();
</script>
</body>
</html>
"##;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_and_encodes_the_registry_format() {
        let v = json!([{ "LayerName": "Plateau_State_Parcels", "Entities": [{ "UUID": 0, "Attributes": [{ "Name": "UPIN", "Value": "PL000000022581" }] }] }]);
        let enc = encode_aumentum(&v);
        assert!(enc.starts_with("[BEGIN]") && enc.ends_with("[END]"));
        assert_eq!(decode_aumentum(&enc), Some(v.clone()));
        // As the Registry's SerializerUtils writes it: the XmlSerializer tag with namespaces, then [END].
        let b64 = base64::engine::general_purpose::STANDARD.encode(v.to_string());
        let reg = format!("<base64Binary xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xmlns:xsd=\"http://www.w3.org/2001/XMLSchema\">{b64}[END]");
        assert_eq!(decode_aumentum(&reg), Some(v));
        assert_eq!(parse_form("a=1&lrsuser=ragoro&initparam_inputobjects=%5BBEGIN%5Dab%2B%2Fc%3D%5BEND%5D&x=a+b")["initparam_inputobjects"], "[BEGIN]ab+/c=[END]");
    }

    fn record(kind: &str, oid: Option<i32>, values: &[(&str, Option<&str>)]) -> UpdateRecord {
        UpdateRecord { kind: kind.into(), object_id: oid, primary: vec![],
            values: values.iter().map(|(f, v)| (f.to_string(), v.map(str::to_string))).collect() }
    }

    #[test]
    fn only_a_real_link_counts_as_an_ok() {
        let link = record("UpdateRecordByOID", Some(526260), &[("LRSID", Some("PLT00031551")), ("LinkageStatus", Some("1")),
            ("LinkageDate", Some("2026-11-05")), ("PropertyNumber", Some("PL63576")), ("Plot", Some("plotNumber"))]);
        // As the desktop sent it on 5 Nov: the link, then the list clearing the UPIN from other parcels.
        let clear = record("UpdateRecordByFields", None, &[("LRSID", None), ("LinkageStatus", None), ("PropertyNumber", None)]);
        let lists = vec![UpdateDataList { records: vec![link.clone()], ..Default::default() }, UpdateDataList { records: vec![clear.clone()], exclude: vec![526260], ..Default::default() }];
        assert_eq!(map_ok_link(&lists), Ok(Some(MapOkLink { object_id: 526260, su_id: "PLT00031551".into(), property_number: Some("PL63576".into()) })));
        // The clearing list alone, an unlink, or no LRSID: no link, so nothing goes to the Registry.
        assert_eq!(map_ok_link(&[UpdateDataList { records: vec![clear], ..Default::default() }]), Ok(None));
        let unlink = record("UpdateRecordByOID", Some(1), &[("LRSID", Some("PLT1")), ("LinkageStatus", Some("0"))]);
        assert_eq!(map_ok_link(&[UpdateDataList { records: vec![unlink], ..Default::default() }]), Ok(None));
        let no_su = record("UpdateRecordByOID", Some(1), &[("LRSID", Some("  ")), ("LinkageStatus", Some("1"))]);
        assert_eq!(map_ok_link(&[UpdateDataList { records: vec![no_su], ..Default::default() }]), Ok(None));
        // Two parcels in one OK are not guessed at.
        assert!(map_ok_link(&[UpdateDataList { records: vec![link.clone(), link], ..Default::default() }]).is_err());
    }

    #[test]
    fn parses_update_cadastral_data() {
        let x = r#"<?xml version="1.0" encoding="utf-8"?><soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:xsd="http://www.w3.org/2001/XMLSchema">
<soap:Header><SecurityHeader xmlns="http://www.landsystems.com/multicadastre/in4/service"><User>ragoro</User><Password>TICKET_1</Password></SecurityHeader></soap:Header>
<soap:Body><UpdateCadastralData xmlns="http://www.landsystems.com/multicadastre/in4/service"><updateList>
<UpdateDataList><LayerNameOrAlias>sde.SDE.Plateau_State_Parcels</LayerNameOrAlias><Records>
<UpdateRecord xsi:type="UpdateRecordByOID"><Values><UpdateValue><Field>LRSID</Field><Value xsi:type="xsd:string">PLT1</Value></UpdateValue><UpdateValue><Field>LinkageStatus</Field><Value xsi:type="xsd:int">1</Value></UpdateValue></Values><ObjectID>42</ObjectID></UpdateRecord>
</Records></UpdateDataList>
<UpdateDataList><LayerNameOrAlias>sde.SDE.Plateau_State_Parcels</LayerNameOrAlias><Records>
<UpdateRecord xsi:type="UpdateRecordByFields"><Values><UpdateValue><Field>LRSID</Field><Value xsi:nil="true" /></UpdateValue></Values><PrimaryFields><UpdateValue><Field>UPIN</Field><Value xsi:type="xsd:string">PL9</Value></UpdateValue></PrimaryFields></UpdateRecord>
</Records><ExculdeFeatures><int>42</int></ExculdeFeatures></UpdateDataList>
</updateList></UpdateCadastralData></soap:Body></soap:Envelope>"#;
        let (op, h, lists) = parse_service_request(x).unwrap();
        assert_eq!(op, "UpdateCadastralData");
        assert_eq!(h, Some(("ragoro".into(), "TICKET_1".into())));
        assert_eq!(lists.len(), 2);
        assert_eq!(lists[0].records[0], UpdateRecord { kind: "UpdateRecordByOID".into(), object_id: Some(42),
            values: vec![("LRSID".into(), Some("PLT1".into())), ("LinkageStatus".into(), Some("1".into()))], primary: vec![] });
        assert_eq!(lists[1].records[0].kind, "UpdateRecordByFields");
        assert_eq!(lists[1].records[0].values, vec![("LRSID".into(), None)]);
        assert_eq!(lists[1].records[0].primary, vec![("UPIN".into(), Some("PL9".into()))]);
        assert_eq!(lists[1].exclude, vec![42]);
    }
}
