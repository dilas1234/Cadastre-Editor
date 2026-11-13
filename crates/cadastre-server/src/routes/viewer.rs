//! Browser viewer (Phase 7, FR-VIEW): a self-contained MapLibre page plus a GeoJSON endpoint.
//!
//! The parcels are stored in EPSG:32632; web maps want WGS84, so the endpoint reprojects with
//! `ST_Transform(geom, 4326)`. This is the lightweight interim viewer (served by the server,
//! no build step); the Leptos/WASM client is the richer eventual replacement.

use axum::extract::{Query, State};
use axum::response::Html;
use serde::Deserialize;
use sqlx::Row;

use crate::error::ApiResult;
use crate::state::AppState;

/// Query params for the GeoJSON endpoint.
#[derive(Debug, Deserialize)]
pub struct GeoJsonQuery {
    /// Optional WGS84 bbox `minlon,minlat,maxlon,maxlat` to limit to the current view.
    pub bbox: Option<String>,
    /// Max features (default 3000).
    pub limit: Option<i64>,
    /// Status filter: `posted` (default), `draft`, `submitted`, `historic`, `archived`, `retired` (both), `superseded` (replaced by a correction, same UPIN), or `all`.
    pub status: Option<String>,
    /// Restrict to one job's parcels (uuid).
    pub job: Option<String>,
}

/// Parcels as a WGS84 GeoJSON FeatureCollection (reprojected from 32632).
pub async fn parcels_geojson(
    State(state): State<AppState>,
    Query(q): Query<GeoJsonQuery>,
) -> ApiResult<([(axum::http::HeaderName, &'static str); 1], String)> {
    let limit = q.limit.unwrap_or(3000).clamp(1, 20000);

    // Status is validated against an allow-list before being inlined.
    let status_clause = match q.status.as_deref() {
        None | Some("posted") => " AND p.status = 'posted'".to_string(),
        Some("all") => String::new(),
        Some(s @ ("draft" | "submitted" | "historic" | "archived" | "superseded")) => format!(" AND p.status = '{s}'"),
        // Retired shapes (Phase 4 history): replaced by a mutation, or archived.
        Some("retired") => " AND p.status IN ('historic', 'archived')".to_string(),
        Some(_) => return Err(crate::error::AppError::BadRequest("invalid status".into())),
    };
    // Job is parsed as a uuid (so the inlined value is safe).
    let job_clause = match q.job.as_deref().filter(|s| !s.is_empty()) {
        Some(j) => {
            let id: uuid::Uuid = j.parse().map_err(|_| crate::error::AppError::BadRequest("invalid job id".into()))?;
            format!(" AND p.job_id = '{id}'")
        }
        None => String::new(),
    };
    let bbox_clause = match q.bbox.as_deref().and_then(parse_bbox) {
        Some((a, b, c, d)) => format!(
            " AND p.geom && ST_Transform(ST_MakeEnvelope({a},{b},{c},{d},4326),32632)"
        ),
        None => String::new(),
    };

    let sql = format!(
        "SELECT COALESCE(json_build_object('type','FeatureCollection','features', COALESCE(json_agg(f), '[]'::json))::text, \
                '{{\"type\":\"FeatureCollection\",\"features\":[]}}') \
         FROM (SELECT json_build_object('type','Feature', \
                 'geometry', ST_AsGeoJSON(ST_Transform(p.geom,4326))::json, \
                 'properties', json_build_object('id', p.id, 'parcel_number', p.parcel_number, \
                     'status', p.status, 'job_id', p.job_id, \
                     'area', trunc(COALESCE(p.computed_area, ST_Area(p.geom))::numeric, 2), \
                     'computed_area', ST_Area(p.geom), \
                     'property_number', a.property_number, 'parcel_area', a.parcel_area)) AS f \
               FROM cadastre.parcel p \
               LEFT JOIN LATERAL (SELECT NULLIF(btrim(pa.property_number), '') AS property_number, pa.parcel_area \
                    FROM cadastre.parcel_attribute pa WHERE pa.upin = p.parcel_number \
                    ORDER BY (pa.status = '0') DESC NULLS LAST, pa.last_edit_date DESC NULLS LAST, pa.object_id DESC LIMIT 1) a ON true \
               WHERE p.geom IS NOT NULL{status_clause}{job_clause}{bbox_clause} \
               LIMIT {limit}) s"
    );
    let json: String = sqlx::query(&sql).fetch_one(&state.pool).await?.get(0);
    Ok((
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        json,
    ))
}

/// Query params for the beacons GeoJSON endpoint.
#[derive(Debug, Deserialize)]
pub struct BeaconGeoJsonQuery {
    /// Optional WGS84 bbox `minlon,minlat,maxlon,maxlat` to limit to the current view.
    pub bbox: Option<String>,
    /// `true`: control points only; `false`: survey beacons only; absent: both.
    pub control: Option<bool>,
    /// Max features (default and maximum 20000).
    pub limit: Option<i64>,
}

/// Beacons as a WGS84 GeoJSON FeatureCollection (reprojected from 32632), ordered by number.
pub async fn beacons_geojson(
    State(state): State<AppState>,
    Query(q): Query<BeaconGeoJsonQuery>,
) -> ApiResult<([(axum::http::HeaderName, &'static str); 1], String)> {
    let limit = q.limit.unwrap_or(20000).clamp(1, 20000);
    let control_clause = match q.control {
        Some(true) => " AND is_control",
        Some(false) => " AND NOT is_control",
        None => "",
    };
    let bbox_clause = match q.bbox.as_deref().and_then(parse_bbox) {
        Some((a, b, c, d)) => format!(" AND geom && ST_Transform(ST_MakeEnvelope({a},{b},{c},{d},4326),32632)"),
        None => String::new(),
    };
    let sql = format!(
        "SELECT COALESCE(json_build_object('type','FeatureCollection','features', COALESCE(json_agg(f ORDER BY n), '[]'::json))::text, \
                '{{\"type\":\"FeatureCollection\",\"features\":[]}}') \
         FROM (SELECT number AS n, json_build_object('type','Feature', \
                 'geometry', ST_AsGeoJSON(ST_Transform(geom,4326))::json, \
                 'properties', json_build_object('id', id, 'number', number, 'is_control', is_control, \
                     'easting', round(ST_X(geom)::numeric,3), 'northing', round(ST_Y(geom)::numeric,3))) AS f \
               FROM cadastre.beacon WHERE geom IS NOT NULL{control_clause}{bbox_clause} \
               ORDER BY is_control DESC, number LIMIT {limit}) s"
    );
    let json: String = sqlx::query(&sql).fetch_one(&state.pool).await?.get(0);
    Ok((
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        json,
    ))
}

/// Query params for the compact beacon list.
#[derive(Debug, Deserialize)]
pub struct BeaconCompactQuery {
    /// `true`: control points only; `false`: survey beacons only; absent: both.
    pub control: Option<bool>,
}

/// Every beacon as a compact list `{"fields":["number","easting","northing","is_control"],
/// "rows":[["C6782A",492951.814,1088827.672,false],...]}` (EPSG:32632), so the map can show all
/// of them at every scale; the browser projects them. Far smaller than GeoJSON (and gzipped).
pub async fn beacons_compact(
    State(state): State<AppState>,
    Query(q): Query<BeaconCompactQuery>,
) -> ApiResult<([(axum::http::HeaderName, &'static str); 1], String)> {
    let sql = "SELECT json_build_object('fields', json_build_array('number','easting','northing','is_control'), \
                 'rows', COALESCE(json_agg(json_build_array(number, round(ST_X(geom)::numeric, 3), round(ST_Y(geom)::numeric, 3), is_control) ORDER BY number), '[]'::json))::text \
               FROM cadastre.beacon WHERE geom IS NOT NULL AND ($1::boolean IS NULL OR is_control = $1)";
    let json: String = sqlx::query(sql).bind(q.control).fetch_one(&state.pool).await?.get(0);
    Ok(([(axum::http::header::CONTENT_TYPE, "application/json")], json))
}

fn parse_bbox(s: &str) -> Option<(f64, f64, f64, f64)> {
    let n: Vec<f64> = s.split(',').filter_map(|v| v.trim().parse().ok()).collect();
    match n.as_slice() {
        [a, b, c, d] => Some((*a, *b, *c, *d)),
        _ => None,
    }
}

/// The viewer page.
pub async fn viewer() -> Html<&'static str> {
    Html(VIEWER_HTML)
}

const VIEWER_HTML: &str = r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8"/>
<meta name="viewport" content="width=device-width, initial-scale=1"/>
<title>PLAGIS Cadastre Viewer</title>
<link href="https://unpkg.com/maplibre-gl@4.7.1/dist/maplibre-gl.css" rel="stylesheet"/>
<script src="https://unpkg.com/maplibre-gl@4.7.1/dist/maplibre-gl.js"></script>
<style>
  html,body{margin:0;height:100%;font-family:system-ui,sans-serif}
  #map{position:absolute;inset:0}
  #panel{position:absolute;z-index:1;top:10px;left:10px;background:#fff;padding:10px 12px;
         border-radius:8px;box-shadow:0 1px 6px rgba(0,0,0,.3);max-width:280px}
  #panel h1{font-size:14px;margin:0 0 6px}
  #search{width:100%;box-sizing:border-box;padding:6px;margin-bottom:6px}
  #info{font-size:12px;color:#333;white-space:pre-line}
</style>
</head>
<body>
<div id="map"></div>
<div id="panel">
  <h1>PLAGIS Cadastre</h1>
  <input id="search" placeholder="Search parcel number (UPIN)…"/>
  <div id="info">loading…</div>
</div>
<script>
const info = document.getElementById('info');
const map = new maplibregl.Map({
  container: 'map',
  style: { version: 8, sources: {}, layers: [
    { id: 'bg', type: 'background', paint: { 'background-color': '#0b1021' } } ] },
  center: [8.9, 9.9], zoom: 11
});
map.addControl(new maplibregl.NavigationControl());

async function fetchParcels() {
  const b = map.getBounds();
  const bbox = [b.getWest(), b.getSouth(), b.getEast(), b.getNorth()].join(',');
  const r = await fetch('/api/v1/parcels.geojson?limit=4000&bbox=' + encodeURIComponent(bbox));
  const fc = await r.json();
  const src = map.getSource('parcels');
  if (src) src.setData(fc);
  info.textContent = fc.features.length + ' parcels in view';
  return fc;
}

map.on('load', async () => {
  map.addSource('parcels', { type: 'geojson', data: { type:'FeatureCollection', features: [] } });
  map.addLayer({ id: 'fill', type: 'fill', source: 'parcels',
    paint: { 'fill-color': '#39b', 'fill-opacity': 0.25 } });
  map.addLayer({ id: 'line', type: 'line', source: 'parcels',
    paint: { 'line-color': '#5cf', 'line-width': 0.8 } });
  // Initial unbounded load to discover extent, then fit.
  const r = await fetch('/api/v1/parcels.geojson?limit=4000');
  const fc = await r.json();
  map.getSource('parcels').setData(fc);
  info.textContent = fc.features.length + ' parcels loaded';
  if (fc.features.length) {
    const bb = new maplibregl.LngLatBounds();
    for (const f of fc.features) {
      const g = f.geometry; if (!g) continue;
      const rings = g.type === 'MultiPolygon' ? g.coordinates.flat() : g.coordinates;
      for (const ring of rings) for (const c of ring) bb.extend(c);
    }
    map.fitBounds(bb, { padding: 40, maxZoom: 16 });
  }
  map.on('moveend', fetchParcels);
});

map.on('click', 'fill', (e) => {
  const p = e.features[0].properties;
  new maplibregl.Popup().setLngLat(e.lngLat)
    .setHTML('<b>' + (p.parcel_number || '(unposted)') + '</b><br/>status: ' + p.status +
             '<br/>area: ' + p.area + ' m²').addTo(map);
});
map.on('mouseenter', 'fill', () => map.getCanvas().style.cursor = 'pointer');
map.on('mouseleave', 'fill', () => map.getCanvas().style.cursor = '');

document.getElementById('search').addEventListener('keydown', async (ev) => {
  if (ev.key !== 'Enter') return;
  const q = ev.target.value.trim(); if (!q) return;
  const r = await fetch('/api/v1/parcels/search?number=' + encodeURIComponent(q));
  const list = await r.json();
  info.textContent = list.length + ' match(es) for "' + q + '"';
});
</script>
</body>
</html>
"##;
