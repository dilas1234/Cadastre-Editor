"use strict";
// PLAGIS CADASTRE Editor — browser client over the REST API, laid out like the desktop GIS
// editor PLAGIS staff use today (title/menu/toolbars, Table Of Contents, Catalog, status bar).
// Geometry is stored in EPSG:32632 (WGS84 / UTM 32N); the map is WGS84, so proj4 converts.

/* =====================================================================================
 * Core helpers
 * ===================================================================================== */
proj4.defs("EPSG:32632", "+proj=utm +zone=32 +datum=WGS84 +units=m +no_defs");
const toUTM = (lng, lat) => proj4("EPSG:4326", "EPSG:32632", [lng, lat]);
const toLL = (e, n) => proj4("EPSG:32632", "EPSG:4326", [e, n]);
const $ = (id) => document.getElementById(id);
const esc = (s) => String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
const empty = () => ({ type: "FeatureCollection", features: [] });
function h(tag, attrs = {}, ...kids) {
  const e = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs || {})) {
    if (v == null || v === false) continue;
    if (k === "class") e.className = v;
    else if (k === "html") e.innerHTML = v;
    else if (k.startsWith("on")) e.addEventListener(k.slice(2), v);
    else e.setAttribute(k, v === true ? "" : v);
  }
  for (const kid of kids.flat()) if (kid != null) e.append(kid.nodeType ? kid : document.createTextNode(kid));
  return e;
}
// Normalise a GeoJSON FeatureCollection (defensive: never let `features` be null).
const asFC = (fc) => ({ type: "FeatureCollection", features: (fc && Array.isArray(fc.features)) ? fc.features : [] });
// --- Sign-in (Keycloak, OIDC authorization code + PKCE) ------------------------------------------
// When the server requires sign-in, the editor sends the user to Keycloak and keeps the tokens
// for this browser tab only (sessionStorage). Every API call waits for sign-in and carries the
// access token; it is refreshed before it expires. With sign-in off, nothing changes.
const auth = { cfg: null, token: null, refresh: null, exp: 0, timer: null };
const b64url = (buf) => btoa(String.fromCharCode(...new Uint8Array(buf))).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
const authRedirectUri = () => location.origin + location.pathname;
// SHA-256 for the PKCE challenge. Browsers only offer crypto.subtle on HTTPS or localhost; the editor
// is served over plain HTTP on the LAN (http://10.10.10.9:8080), so a small implementation is used there.
function sha256(bytes) {
  const K = [0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3,
    0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
    0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
    0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2];
  const H = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
  const len = bytes.length, padded = new Uint8Array(((len + 9 + 63) >> 6) << 6);
  padded.set(bytes); padded[len] = 0x80;
  const dv = new DataView(padded.buffer); dv.setUint32(padded.length - 4, len * 8); dv.setUint32(padded.length - 8, Math.floor(len / 2 ** 29));
  const w = new Uint32Array(64), rotr = (x, n) => (x >>> n) | (x << (32 - n));
  for (let o = 0; o < padded.length; o += 64) {
    for (let i = 0; i < 16; i++) w[i] = dv.getUint32(o + i * 4);
    for (let i = 16; i < 64; i++) {
      const s0 = rotr(w[i - 15], 7) ^ rotr(w[i - 15], 18) ^ (w[i - 15] >>> 3), s1 = rotr(w[i - 2], 17) ^ rotr(w[i - 2], 19) ^ (w[i - 2] >>> 10);
      w[i] = (w[i - 16] + s0 + w[i - 7] + s1) | 0;
    }
    let [a, b, c, d, e, f, g, hh] = H;
    for (let i = 0; i < 64; i++) {
      const t1 = (hh + (rotr(e, 6) ^ rotr(e, 11) ^ rotr(e, 25)) + ((e & f) ^ (~e & g)) + K[i] + w[i]) | 0;
      const t2 = ((rotr(a, 2) ^ rotr(a, 13) ^ rotr(a, 22)) + ((a & b) ^ (a & c) ^ (b & c))) | 0;
      hh = g; g = f; f = e; e = (d + t1) | 0; d = c; c = b; b = a; a = (t1 + t2) | 0;
    }
    [a, b, c, d, e, f, g, hh].forEach((x, i) => (H[i] = (H[i] + x) | 0));
  }
  const out = new DataView(new ArrayBuffer(32)); H.forEach((x, i) => out.setUint32(i * 4, x)); return out.buffer;
}
const sha256Async = (bytes) => (window.crypto?.subtle ? crypto.subtle.digest("SHA-256", bytes) : Promise.resolve(sha256(bytes)));
function authStore(t) {
  auth.token = t.access_token; auth.refresh = t.refresh_token || auth.refresh; auth.exp = Date.now() + (t.expires_in || 300) * 1000;
  try { sessionStorage.setItem("cad.tokens", JSON.stringify({ token: auth.token, refresh: auth.refresh, exp: auth.exp })); } catch { /* private mode */ }
  clearTimeout(auth.timer); auth.timer = setTimeout(() => authRefresh().catch(authLogin), Math.max(10_000, auth.exp - Date.now() - 60_000));
}
async function authToken(body) {
  const r = await fetch(`${auth.cfg.issuer}/protocol/openid-connect/token`, { method: "POST", headers: { "content-type": "application/x-www-form-urlencoded" }, body: new URLSearchParams({ client_id: auth.cfg.client_id, ...body }) });
  if (!r.ok) throw new Error("sign-in failed: " + r.status);
  authStore(await r.json());
}
const authRefresh = () => (auth.refresh ? authToken({ grant_type: "refresh_token", refresh_token: auth.refresh }) : Promise.reject(new Error("no session")));
async function authLogin() {
  const verifier = b64url(crypto.getRandomValues(new Uint8Array(32))), st = b64url(crypto.getRandomValues(new Uint8Array(16)));
  const challenge = b64url(await sha256Async(new TextEncoder().encode(verifier)));
  sessionStorage.setItem("cad.pkce", JSON.stringify({ verifier, state: st, back: location.search }));
  location.assign(`${auth.cfg.issuer}/protocol/openid-connect/auth?` + new URLSearchParams({ client_id: auth.cfg.client_id, redirect_uri: authRedirectUri(),
    response_type: "code", scope: "openid", state: st, code_challenge: challenge, code_challenge_method: "S256" }));
  return new Promise(() => {}); // the page is leaving
}
function authLogout() {
  const id = auth.cfg?.enabled ? auth.cfg : null;
  try { sessionStorage.removeItem("cad.tokens"); } catch { /* ignore */ }
  if (id) location.assign(`${id.issuer}/protocol/openid-connect/logout?` + new URLSearchParams({ client_id: id.client_id, post_logout_redirect_uri: authRedirectUri() }));
}
async function initAuth() {
  auth.cfg = await fetch("/api/v1/auth/config").then((r) => r.json()).catch(() => ({ enabled: false }));
  if (!auth.cfg.enabled) return;
  const q = new URLSearchParams(location.search);
  if (q.get("code") && q.get("state")) {
    const pk = JSON.parse(sessionStorage.getItem("cad.pkce") || "{}");
    if (pk.state !== q.get("state")) return authLogin();
    await authToken({ grant_type: "authorization_code", code: q.get("code"), redirect_uri: authRedirectUri(), code_verifier: pk.verifier }).catch(authLogin);
    sessionStorage.removeItem("cad.pkce");
    history.replaceState(null, "", authRedirectUri() + (pk.back || ""));
    return;
  }
  try { const t = JSON.parse(sessionStorage.getItem("cad.tokens") || "null"); if (t) { auth.token = t.token; auth.refresh = t.refresh; auth.exp = t.exp; } } catch { /* ignore */ }
  if (auth.token && auth.exp > Date.now() + 30_000) { authStore({ access_token: auth.token, refresh_token: auth.refresh, expires_in: (auth.exp - Date.now()) / 1000 }); return; }
  if (auth.refresh) { try { await authRefresh(); return; } catch { /* sign in again */ } }
  return authLogin();
}
const authReady = initAuth().catch((e) => {
  const msg = "Sign-in could not start: " + e.message;
  setTimeout(() => { try { status(msg); } catch { /* status bar not built yet */ } alert(msg); }, 0);
  throw e;
});
const authHeaders = () => (auth.token ? { authorization: `Bearer ${auth.token}` } : {});
async function api(path, opts = {}) {
  await authReady;
  const init = { ...opts, headers: { ...(opts.body ? { "content-type": "application/json" } : {}), ...authHeaders(), ...(opts.headers || {}) } };
  if (opts.body && typeof opts.body !== "string") init.body = JSON.stringify(opts.body);
  let r = await fetch(path, init);
  if (r.status === 401 && auth.cfg?.enabled) {
    try { await authRefresh(); } catch { return authLogin(); }
    r = await fetch(path, { ...init, headers: { ...init.headers, ...authHeaders() } });
  }
  const text = await r.text();
  let data = null; try { data = text ? JSON.parse(text) : null; } catch { data = text; }
  if (!r.ok) throw new Error((data && data.message) || `${r.status} ${r.statusText}`);
  return data;
}
const fmt3 = (v) => Number(v).toFixed(3);
const status = (msg) => { $("statusMsg").textContent = msg || ""; };
// A message that cannot be missed (the status bar is easy to overlook): shown over the map for a
// few seconds, and also written to the status bar.
function notify(msg, kind = "ok") {
  status(msg);
  document.querySelectorAll(".toast").forEach((t) => t.remove());
  const t = h("div", { class: `toast ${kind}` }, msg);
  document.body.append(t);
  setTimeout(() => t.classList.add("fade"), 5500); setTimeout(() => t.remove(), 6500);
}
// Keep a sketch built from typed coordinates in view: fit the map to it when it is off-screen.
function showSketchExtent(force = false) {
  if (!edit.verts.length) return;
  const b = map.getBounds();
  const lls = edit.verts.map(([e, n]) => toLL(e, n));
  if (!force && lls.every(([x, y]) => b.contains([x, y]))) return;
  if (lls.length === 1) map.easeTo({ center: lls[0], zoom: Math.max(map.getZoom(), 18) });
  else zoomToFC({ type: "FeatureCollection", features: lls.map((c) => ({ type: "Feature", geometry: { type: "Point", coordinates: c }, properties: {} })) }, { padding: 120, maxZoom: 19 });
}
// After saving: zoom to the saved polygon (UTM ring) so the result is seen.
function zoomToUTMRing(utm) { zoomToFC({ type: "FeatureCollection", features: [{ type: "Feature", geometry: { type: "Polygon", coordinates: [[...utm, utm[0]].map(([e, n]) => toLL(e, n))] }, properties: {} }] }, { padding: 120, maxZoom: 19 }); }

const REGION = "Plateau_State";
// Plateau State extent (WGS84). "Full extent" is the region's data, ignoring anything outside it.
const REGION_BBOX = [8.2, 8.3, 10.7, 10.5];
const inRegion = ([x, y]) => x >= REGION_BBOX[0] && x <= REGION_BBOX[2] && y >= REGION_BBOX[1] && y <= REGION_BBOX[3];
const APP = "PLAGIS CADASTRE Editor";
const PARCEL_STROKE = "#b22222"; // Aumentum's dark-red parcel outline
const IMAGERY = "https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/{z}/{y}/{x}";
const POSTABLE_STAGE = "approved"; // server only posts approved jobs

const state = {
  me: null, jobs: [], workflow: { stages: [], transitions: [] }, job: null,
  tool: "pan", selection: new Set(), focusParcel: null,
  history: [], histIdx: -1, navigating: false,
  parcels: empty(), drafts: empty(), beacons: empty(), surveyFC: empty(),
  graphics: JSON.parse(localStorage.getItem("cad.graphics") || "[]"),
  bookmarks: JSON.parse(localStorage.getItem("cad.bookmarks") || "[]"),
  tocView: "drawing",
  dynLayers: [], // rasters/datasets dragged from the Catalog onto the TOC
};
let dynSeq = 1;
// Coordinate systems Catalog data in Plateau can arrive in. The fabric is UTM 32N; older survey data
// is on the Minna datum (UTM 31N/32N or the Nigeria West/Mid/East Belt transverse Mercators). The
// Minna -> WGS84 shift is EPSG's (-92, -93, 122). The server reads these codes from .prj files.
const MINNA = "+ellps=clrk80 +towgs84=-92,-93,122,0,0,0,0 +units=m +no_defs";
const EXTRA_CRS = {
  4263: "+proj=longlat +ellps=clrk80 +towgs84=-92,-93,122,0,0,0,0 +no_defs",
  26331: `+proj=utm +zone=31 ${MINNA}`,
  26332: `+proj=utm +zone=32 ${MINNA}`,
  26391: `+proj=tmerc +lat_0=4 +lon_0=4.5 +k=0.99975 +x_0=230738.26 +y_0=0 ${MINNA}`,
  26392: `+proj=tmerc +lat_0=4 +lon_0=8.5 +k=0.99975 +x_0=670553.98 +y_0=0 ${MINNA}`,
  26393: `+proj=tmerc +lat_0=4 +lon_0=12.5 +k=0.99975 +x_0=1110369.7 +y_0=0 ${MINNA}`,
  32631: "+proj=utm +zone=31 +datum=WGS84 +units=m +no_defs",
  32633: "+proj=utm +zone=33 +datum=WGS84 +units=m +no_defs",
};
// Resolve an EPSG code to a proj4 name. `known` is false when the code is missing or unsupported
// and UTM 32N (the fabric's CRS) is assumed, so callers can tell the user instead of silently
// drawing data in the wrong place.
function resolveCrs(epsg) {
  if (epsg === 4326) return { code: "EPSG:4326", known: true };
  const code = "EPSG:" + epsg;
  if (epsg && !proj4.defs(code) && EXTRA_CRS[epsg]) proj4.defs(code, EXTRA_CRS[epsg]);
  if (epsg && proj4.defs(code)) return { code, known: true };
  return { code: "EPSG:32632", known: false };
}
function ensureProj(epsg) { return resolveCrs(epsg).code; }

// Digitizing / edit session (ArcMap Editor). Vertices are kept in EPSG:32632 metres.
const edit = { active: false, template: "parcel", geomType: "polygon", construction: "polygon", verts: [], sel: -1, snapping: true, editingId: null };
// Beacon numbers that came with typed / pasted points ("CA6782 492951.814 1088827.672"), keyed by
// coordinates to the mm; the Beacon Numbers window at commit offers them for those corners.
const pointIds = new Map();
const ptKey = (e, n) => `${Number(e).toFixed(3)} ${Number(n).toFixed(3)}`;
try { for (const [k, v] of JSON.parse(localStorage.getItem("cad.pointIds") || "[]")) pointIds.set(k, v); } catch { /* storage unavailable */ }
function rememberPointId(e, n, id) {
  if (!id) return;
  pointIds.set(ptKey(e, n), String(id).trim());
  try { localStorage.setItem("cad.pointIds", JSON.stringify([...pointIds].slice(-5000))); } catch { /* storage unavailable */ }
}
// Feature templates in the Create Features window; each maps to a layer and a geometry type.
const FEATURE_TEMPLATES = [
  { id: "parcel", name: `${REGION}_Parcels`, geom: "polygon", icon: "polygonFc", tools: ["polygon", "autocomplete", "rightangle"] },
  { id: "line", name: `${REGION}_Lines`, geom: "line", icon: "lineFc", tools: ["line"] },
  { id: "point", name: `${REGION}_CtrlPnts`, geom: "point", icon: "pointFc", tools: ["point"] },
  // The current Aumentum job's job area (only while working on a job from the Workflow tab).
  { id: "jobArea", name: `${REGION}_JobsArea`, geom: "polygon", icon: "polygonFc", tools: ["polygon", "rightangle"], wfOnly: true },
];
const CTOOL_NAME = { polygon: "Polygon", autocomplete: "Auto Complete Polygon", rightangle: "Right Angle", line: "Line", point: "Point" };
const CTOOL_ICON = { polygon: "ctPolygon", autocomplete: "ctAutoComplete", rightangle: "ctRightAngle", line: "ctLine", point: "ctPoint" };

/* =====================================================================================
 * Map
 * ===================================================================================== */
const map = new maplibregl.Map({
  container: "map",
  style: {
    version: 8,
    // Label glyphs are served by the server (vendored Open Sans), so labels work offline.
    glyphs: "/editor/vendor/fonts/{fontstack}/{range}.pbf",
    // maxzoom: beyond Esri's last real level here, MapLibre over-zooms instead of fetching placeholder tiles.
    sources: { imagery: { type: "raster", tiles: [IMAGERY], tileSize: 256, maxzoom: 18, attribution: "Imagery © Esri" } },
    layers: [{ id: "imagery", type: "raster", source: "imagery" }],
  },
  center: [8.89, 9.9], zoom: 13, attributionControl: { compact: true },
  boxZoom: false, doubleClickZoom: false,
});

function crossImage(color, size = 15) {
  const c = document.createElement("canvas"); c.width = c.height = size;
  const g = c.getContext("2d"); g.strokeStyle = color; g.lineWidth = 2;
  g.beginPath(); g.moveTo(size / 2, 1); g.lineTo(size / 2, size - 1); g.moveTo(1, size / 2); g.lineTo(size - 1, size / 2); g.stroke();
  return g.getImageData(0, 0, size, size);
}

// Wire layers and data as soon as the style is parsed — NOT on "load", which waits for the first
// imagery tiles: a slow or unreachable tile server must never hold up jobs, parcels or tools.
let initialDeepLink = false;
map.once("style.load", async () => {
  map.addImage("cross", crossImage("#000"));
  map.addSource("parcels", { type: "geojson", data: empty(), promoteId: "id" });
  map.addLayer({ id: "parcels-fill", type: "fill", source: "parcels", paint: { "fill-color": "#fff", "fill-opacity": 0.01 } });
  // Aumentum's Plateau_State_Parcels symbology: thin dark-red outlines; the parcel in focus in
  // red with its vertices dotted.
  map.addLayer({ id: "parcels-line", type: "line", source: "parcels", paint: { "line-color": PARCEL_STROKE, "line-width": 1 } });
  // Parcel history (Phase 4): shapes replaced by a mutation (historic) or retired (archived).
  map.addSource("history", { type: "geojson", data: empty(), promoteId: "id" });
  map.addLayer({ id: "history-fill", type: "fill", source: "history", layout: { visibility: "none" }, paint: { "fill-color": "#888", "fill-opacity": 0.12 } }, "parcels-line");
  map.addLayer({ id: "history-line", type: "line", source: "history", layout: { visibility: "none" }, paint: { "line-color": "#666", "line-width": 1.2, "line-dasharray": [3, 2] } }, "parcels-line");
  map.addLayer({ id: "parcels-focus", type: "line", source: "parcels", filter: ["==", ["get", "id"], ""],
    paint: { "line-color": "#ff0000", "line-width": 1.6 } });
  map.addSource("focus-vertices", { type: "geojson", data: empty() });
  map.addLayer({ id: "parcels-focus-vtx", type: "circle", source: "focus-vertices",
    paint: { "circle-radius": 2.6, "circle-color": "#ff0000", "circle-stroke-color": "#7a0000", "circle-stroke-width": 0.6 } });
  // Aumentum's parcel label: the property (document) number, and the area in sq.m. beneath it.
  map.addLayer({ id: "parcels-label", type: "symbol", source: "parcels", minzoom: 16,
    layout: {
      "text-field": ["format",
        ["coalesce", ["get", "property_number"], ["get", "parcel_number"], ""], {},
        "\n", {},
        ["concat", ["number-format", ["coalesce", ["get", "parcel_area"], ["get", "area"]], { "min-fraction-digits": 2, "max-fraction-digits": 2 }], " sq.m."], { "font-scale": 0.92 }],
      "text-font": ["OpenSans-Regular"], "text-size": ["interpolate", ["linear"], ["zoom"], 16, 10, 19, 13],
      "text-padding": 2, "text-max-width": 12,
    },
    paint: { "text-color": "#000", "text-halo-color": "#fff", "text-halo-width": 1.4 } });
  map.addLayer({ id: "parcels-sel", type: "line", source: "parcels", filter: ["in", ["get", "id"], ["literal", []]],
    paint: { "line-color": "#00ffff", "line-width": 2.5 } });

  // The current job's job area (Aumentum's Plateau_State_JobsArea), dashed so parcels read through it.
  map.addSource("job-area", { type: "geojson", data: empty() });
  map.addLayer({ id: "job-area-fill", type: "fill", source: "job-area", paint: { "fill-color": "#ff3030", "fill-opacity": 0.06 } });
  map.addLayer({ id: "job-area-line", type: "line", source: "job-area", paint: { "line-color": "#e00000", "line-width": 2, "line-dasharray": [3, 2] } });
  map.addSource("drafts", { type: "geojson", data: empty(), promoteId: "id" });
  map.addLayer({ id: "drafts-fill", type: "fill", source: "drafts", paint: { "fill-color": "#fff", "fill-opacity": 0.08 } });
  map.addLayer({ id: "drafts-line", type: "line", source: "drafts", paint: { "line-color": "#111", "line-width": 1.6 } });

  map.addSource("survey", { type: "geojson", data: empty() });
  map.addLayer({ id: "survey-line", type: "line", source: "survey", filter: ["==", "$type", "LineString"],
    paint: { "line-color": "#e60000", "line-width": 2 } });
  map.addLayer({ id: "survey-pt", type: "symbol", source: "survey", filter: ["==", "$type", "Point"],
    layout: { "icon-image": "cross", "icon-allow-overlap": true } });

  map.addSource("beacons", { type: "geojson", data: empty(), promoteId: "id" });
  map.addLayer({ id: "beacons-pt", type: "circle", source: "beacons", filter: ["==", ["get", "is_control"], true],
    paint: { "circle-radius": 4.5, "circle-color": "#55ff00", "circle-stroke-color": "#1a3d00", "circle-stroke-width": 1 } });

  // Beacons found with Find: ringed in red and labelled.
  map.addSource("found", { type: "geojson", data: empty() });
  map.addLayer({ id: "found-ring", type: "circle", source: "found",
    paint: { "circle-radius": 9, "circle-color": "rgba(0,0,0,0)", "circle-stroke-color": "#ff0000", "circle-stroke-width": 2.5 } });
  map.addLayer({ id: "found-label", type: "symbol", source: "found",
    layout: { "text-field": ["get", "number"], "text-font": ["OpenSans-Regular"], "text-size": 12, "text-offset": [0.9, -0.9], "text-anchor": "bottom-left", "text-allow-overlap": true },
    paint: { "text-color": "#ff0000", "text-halo-color": "#ffffff", "text-halo-width": 1.5 } });

  // Corner points of the parcels being committed, numbered for the Beacon Numbers window.
  map.addSource("corners", { type: "geojson", data: empty() });
  map.addLayer({ id: "corners-pt", type: "circle", source: "corners",
    paint: { "circle-radius": ["case", ["boolean", ["get", "active"], false], 8, 5], "circle-color": ["case", ["boolean", ["get", "done"], false], "#00a000", "#ff8c00"],
      "circle-stroke-color": "#000", "circle-stroke-width": 1 } });
  map.addLayer({ id: "corners-label", type: "symbol", source: "corners",
    layout: { "text-field": ["get", "label"], "text-font": ["OpenSans-Regular"], "text-size": 12, "text-offset": [0.8, -0.8], "text-anchor": "bottom-left", "text-allow-overlap": true },
    paint: { "text-color": "#000", "text-halo-color": "#fff", "text-halo-width": 1.5 } });

  map.addSource("graphics", { type: "geojson", data: empty() });
  map.addLayer({ id: "graphics-fill", type: "fill", source: "graphics", filter: ["==", "$type", "Polygon"],
    paint: { "fill-color": ["get", "fill"], "fill-opacity": ["get", "fillOpacity"] } });
  map.addLayer({ id: "graphics-line", type: "line", source: "graphics",
    paint: { "line-color": ["get", "stroke"], "line-width": ["case", ["boolean", ["get", "selected"], false], 3, 1.5] } });

  map.addSource("measure", { type: "geojson", data: empty() });
  map.addLayer({ id: "measure-line", type: "line", source: "measure",
    paint: { "line-color": "#ff00ff", "line-width": 2, "line-dasharray": [2, 1] } });

  // Digitizing sketch (fill + segments + vertices). Last/selected vertex is drawn red.
  map.addSource("sketch", { type: "geojson", data: empty() });
  map.addLayer({ id: "sketch-fill", type: "fill", source: "sketch", filter: ["==", "$type", "Polygon"],
    paint: { "fill-color": "#00e5ff", "fill-opacity": 0.12 } });
  map.addLayer({ id: "sketch-line", type: "line", source: "sketch", filter: ["==", "$type", "LineString"],
    paint: { "line-color": "#00b7d4", "line-width": 2 } });
  map.addLayer({ id: "sketch-vtx", type: "circle", source: "sketch", filter: ["==", "$type", "Point"],
    paint: { "circle-radius": 4, "circle-color": ["case", ["boolean", ["get", "sel"], false], "#ff2a2a", "#1a8a2a"],
      "circle-stroke-color": "#063", "circle-stroke-width": 1.4 } });

  await Promise.all([loadParcels(true), loadBeacons(), loadJobs()]);
  renderGraphics();
  buildTOC(); buildCatalog();
  pushHistory();
});

async function loadParcels(fit) {
  try { state.parcels = asFC(await api("/api/v1/parcels.geojson?limit=20000")); }
  catch (e) { status("Could not load parcels: " + e.message); state.parcels = empty(); }
  map.getSource("parcels").setData(state.parcels);
  if (fit && !initialDeepLink) zoomToFC(state.parcels, { animate: false }, true) || map.fitBounds([REGION_BBOX.slice(0, 2), REGION_BBOX.slice(2)], { animate: false });
}
// The map shows the control points only (as before); survey beacons are not drawn, but Find, the
// Beacon box and the commit window search all of them on the server.
async function loadBeacons() {
  let rows = [];
  try { rows = (await api("/api/v1/beacons.compact?control=true")).rows; } catch { rows = []; }
  state.beacons = { type: "FeatureCollection", features: rows.map(([number, easting, northing, is_control]) => ({
    type: "Feature", geometry: { type: "Point", coordinates: toLL(easting, northing) },
    properties: { id: number, number, is_control, easting, northing } })) };
  map.getSource("beacons")?.setData(state.beacons);
}
let historyTimer = null;
map.on("moveend", () => { clearTimeout(historyTimer); historyTimer = setTimeout(loadHistory, 250); });
// The history layer, for the area in view (only while it is switched on).
let historySeq = 0;
async function loadHistory() {
  if (!map.getLayer("history-line") || map.getLayoutProperty("history-line", "visibility") === "none") return;
  const seq = ++historySeq, b = map.getBounds();
  let fc = empty();
  if (map.getZoom() >= 13) { try { fc = asFC(await api(`/api/v1/parcels.geojson?status=retired&limit=5000&bbox=${[b.getWest(), b.getSouth(), b.getEast(), b.getNorth()].join(",")}`)); } catch { fc = empty(); } }
  if (seq === historySeq) map.getSource("history")?.setData(fc);
}
async function loadDrafts() {
  state.drafts = state.job ? asFC(await api(`/api/v1/parcels.geojson?status=draft&limit=5000&job=${state.job.id}`).catch(() => empty())) : empty();
  map.getSource("drafts") && map.getSource("drafts").setData(state.drafts);
}
function fcBounds(fc, regionOnly = false) {
  const b = new maplibregl.LngLatBounds(); let n = 0;
  const add = (c) => { if (!regionOnly || inRegion(c)) { b.extend(c); n++; } };
  for (const f of fc.features || []) {
    const g = f.geometry; if (!g) continue;
    if (g.type === "Point") add(g.coordinates);
    else if (g.type === "LineString") g.coordinates.forEach(add);
    else if (g.type === "Polygon") g.coordinates.flat().forEach(add);
    else if (g.type === "MultiPolygon") g.coordinates.flat(2).forEach(add);
  }
  return n ? b : null;
}
function zoomToFC(fc, opts = {}, regionOnly = false) {
  const b = fcBounds(fc, regionOnly);
  if (b) map.fitBounds(b, { padding: 40, maxZoom: 18, ...opts });
  return !!b;
}

/* =====================================================================================
 * Phase 1 — window chrome: title, menus, toolbars, status bar
 * ===================================================================================== */
// No in-page title bar: the browser window/tab title carries it, as the native window title did.
function setTitle() {
  const who = state.me ? ` - Connected as ${state.me.username}` : "";
  document.title = `${REGION} - ${APP}${who}`;
}
api("/api/v1/me").then((me) => { state.me = me; setTitle(); }).catch(() => setTitle());
setTitle();

// --- status bar coordinates (UTM 32N metres, like the desktop editor) ---
map.on("mousemove", (e) => {
  const [E, N] = toUTM(e.lngLat.lng, e.lngLat.lat);
  $("coordXY").textContent = `${fmt3(E)}  ${fmt3(N)} Meters`;
});
map.getContainer().addEventListener("mouseleave", () => { $("coordXY").innerHTML = "&nbsp;"; });

// --- menus ---
const isChecked = (fn) => (typeof fn === "function" ? fn() : !!fn);
const MENUS = () => ({
  File: [
    { label: "New Job…", icon: "jobOpen", action: newJobDialog },
    { label: "Open Job…", action: () => $("jobBox").focus() },
    "-",
    { label: "Site Plan (PDF)", icon: "sitePlan", action: () => openPdf("siteplan"), disabled: () => !state.job },
    { label: "Job Report (PDF)", icon: "report", action: () => openPdf("report"), disabled: () => !state.job },
    "-",
    { label: "Import Survey / CAD / Attributes…", icon: "addData", action: openImportDialog },
    "-",
    { label: "Print…", kbd: "Ctrl+P", action: () => window.print() },
    "-",
    { label: "Sign Out", action: () => authLogout(), disabled: () => !auth.cfg?.enabled },
    { label: "Exit", disabled: true },
  ],
  Edit: [
    { label: "Undo", kbd: "Ctrl+Z", disabled: true }, { label: "Redo", kbd: "Ctrl+Y", disabled: true },
    "-",
    { label: "Copy Map Coordinates", action: copyCenterXY },
    { label: "Delete Selected Elements", kbd: "Delete", action: deleteSelectedGraphics, disabled: () => !state.graphics.some((g) => g.selected) },
    "-",
    { label: "Select All Elements", action: () => { state.graphics.forEach((g) => (g.selected = true)); renderGraphics(); } },
    { label: "Unselect All Elements", action: () => { state.graphics.forEach((g) => (g.selected = false)); renderGraphics(); } },
  ],
  View: [
    { label: "Data View", icon: "dataView", checked: true },
    { label: "Layout View", icon: "layout", disabled: true },
    "-",
    { label: "Zoom Data", sub: [
      { label: "Full Extent", icon: "fullExtent", action: fullExtent },
      { label: "Fixed Zoom In", icon: "fixedZoomIn", action: () => map.zoomIn() },
      { label: "Fixed Zoom Out", icon: "fixedZoomOut", action: () => map.zoomOut() },
      { label: "Go Back To Previous Extent", icon: "back", action: goBack },
      { label: "Go To Next Extent", icon: "forward", action: goForward },
    ] },
    { label: "Bookmarks", action: manageBookmarks },
    "-",
    { label: "Status Bar", checked: () => !$("statusbar").classList.contains("hidden"), action: () => $("statusbar").classList.toggle("hidden") },
    { label: "Refresh", kbd: "F5", icon: "refresh", action: refreshAll },
  ],
  Bookmarks: [
    { label: "Create Bookmark…", action: createBookmark },
    { label: "Manage Bookmarks…", action: manageBookmarks },
    ...(state.bookmarks.length ? ["-"] : []),
    ...state.bookmarks.map((b) => ({ label: b.name, action: () => map.fitBounds(b.bounds) })),
  ],
  Insert: [
    { label: "Data Frame", disabled: true }, { label: "Title", disabled: true }, { label: "Text", disabled: true },
    { label: "North Arrow…", disabled: true }, { label: "Scale Bar…", disabled: true }, { label: "Legend…", disabled: true },
  ],
  Selection: [
    { label: "Select By Attributes…", action: findDialog },
    { label: "Select By Location…", disabled: true },
    "-",
    { label: "Zoom To Selected Features", action: zoomToSelected, disabled: () => !state.selection.size },
    { label: "Statistics…", action: selectionStats, disabled: () => !state.selection.size },
    "-",
    { label: "Interactive Selection Method", disabled: true },
    { label: "Clear Selected Features", icon: "clearSelection", action: clearSelection, disabled: () => !state.selection.size },
  ],
  Geoprocessing: [
    { label: "Validate Fabric (selected / job)…", icon: "validate", action: validateFabric },
    { label: "Survey Closure Check…", icon: "survey", action: () => openTask("survey") },
    { label: "Computation Tools (35)…", icon: "arcToolbox", action: openComputeTools },
    "-",
    { label: "ArcToolbox", icon: "arcToolbox", disabled: true },
    { label: "Python", icon: "python", disabled: true },
    { label: "ModelBuilder", icon: "modelBuilder", disabled: true },
  ],
  Customize: [
    { label: "Toolbars", sub: TOOLBAR_DEFS.map((t) => ({ label: t.name, checked: () => !toolbarHidden(t.id), action: () => toggleToolbar(t.id) })) },
    { label: "Extensions…", disabled: true },
    { label: "Customize Mode…", disabled: true },
  ],
  Windows: [
    { label: "Table Of Contents", checked: () => !$("tocPane").classList.contains("hidden"), action: () => togglePane("tocPane") },
    { label: "Catalog", icon: "catalog", checked: () => !$("catalogPane").classList.contains("hidden"), action: () => togglePane("catalogPane") },
    { label: "Tasks", icon: "tasks", action: () => openTask(taskSel()) },
    { label: "Workflow (Job Instances)", icon: "jobOpen", action: () => openWorkflow() },
    { label: "Productivity Report…", action: () => productivityWindow() },
    { label: "Notifications…", action: () => notificationsWindow() },
    { label: "Administration…", action: () => adminWindow() },
    { label: "Create Features", icon: "createFeatures", action: () => (edit.active ? openCreateFeatures() : startEditing()) },
    { label: "Editor Toolbar", checked: () => !toolbarHidden("tb-editor"), action: () => toggleToolbar("tb-editor") },
    { label: "Identify", icon: "identify", action: () => setTool("identify") },
  ],
  Help: [
    { label: "API Documentation", action: () => window.open("/swagger-ui", "_blank") },
    "-",
    { label: `About ${APP}…`, action: aboutDialog },
  ],
});
function buildMenus() {
  const bar = $("menubar"); bar.innerHTML = "";
  for (const name of Object.keys(MENUS())) {
    const m = h("div", { class: "menu" }, name);
    m.addEventListener("mousedown", (ev) => {
      if (ev.target.closest(".menuList")) return;
      const wasOpen = m.classList.contains("open"); closeMenus();
      if (!wasOpen) openMenu(m, name);
      ev.stopPropagation();
    });
    m.addEventListener("mouseenter", () => { if (bar.querySelector(".menu.open") && !m.classList.contains("open")) { closeMenus(); openMenu(m, name); } });
    bar.append(m);
  }
}
function menuList(items) {
  const list = h("div", { class: "menuList" });
  for (const it of items) {
    if (it === "-") { list.append(h("div", { class: "msep" })); continue; }
    const dis = isChecked(it.disabled);
    const row = h("div", { class: `mi${dis ? " disabled" : ""}${it.checked && isChecked(it.checked) ? " checked" : ""}` },
      h("span", { class: "icSlot", html: it.icon && !(it.checked && isChecked(it.checked)) ? icon(it.icon) : "" }),
      h("span", {}, it.label), it.kbd ? h("span", { class: "kbd" }, it.kbd) : it.sub ? h("span", { class: "kbd" }, "▸") : null);
    if (it.sub) {
      const sub = menuList(it.sub); sub.style.left = "100%"; sub.style.top = "-3px";
      row.style.position = "relative"; row.append(sub);
      row.addEventListener("mouseenter", () => (sub.style.display = "block"));
      row.addEventListener("mouseleave", () => (sub.style.display = "none"));
    } else if (!dis && it.action) {
      row.addEventListener("mouseup", (ev) => { ev.stopPropagation(); closeMenus(); it.action(); });
    }
    list.append(row);
  }
  return list;
}
function openMenu(m, name) { m.querySelector(".menuList")?.remove(); m.append(menuList(MENUS()[name])); m.classList.add("open"); }
function closeMenus() { document.querySelectorAll(".menu.open, .ctx").forEach((m) => (m.classList.contains("ctx") ? m.remove() : m.classList.remove("open"))); }
document.addEventListener("mousedown", (ev) => { if (!ev.target.closest(".menu, .ctx")) closeMenus(); });
document.addEventListener("keydown", (ev) => {
  const inField = /INPUT|SELECT|TEXTAREA/.test(document.activeElement.tagName);
  if (ev.key === "Escape") { closeMenus(); cancelInteraction(); }
  if (ev.key === "F5") { ev.preventDefault(); refreshAll(); }
  if (ev.key === "F2" && state.tool === "sketch") { ev.preventDefault(); finishSketch(); }
  if (ev.key === "Backspace" && !inField && edit.active && edit.verts.length) { ev.preventDefault(); edit.verts.pop(); edit.sel = edit.verts.length - 1; drawSketch(); refreshSketchProps(); updateEditorButtons(); }
  if (ev.key === "Delete" && !inField) deleteSelectedGraphics();
});

// --- toolbars ---
// Item kinds: tool (sticky map tool), btn (one-shot), sep, label, select, drop (menu button), custom.
const TOOLBAR_DEFS = [
  { id: "tb-tools", name: "Tools", row: 1, items: [
    { kind: "tool", tool: "zoomIn", icon: "zoomIn", tip: "Zoom In: click or drag a box" },
    { kind: "tool", tool: "zoomOut", icon: "zoomOut", tip: "Zoom Out: click or drag a box" },
    { kind: "tool", tool: "pan", icon: "pan", tip: "Pan" },
    { kind: "btn", icon: "fullExtent", tip: "Full Extent", action: () => fullExtent() },
    { kind: "sep" },
    { kind: "btn", icon: "fixedZoomIn", tip: "Fixed Zoom In", action: () => map.zoomIn() },
    { kind: "btn", icon: "fixedZoomOut", tip: "Fixed Zoom Out", action: () => map.zoomOut() },
    { kind: "btn", id: "btnBack", icon: "back", tip: "Go Back To Previous Extent", action: () => goBack() },
    { kind: "btn", id: "btnFwd", icon: "forward", tip: "Go To Next Extent", action: () => goForward() },
    { kind: "sep" },
    { kind: "tool", tool: "select", icon: "selectFeatures", tip: "Select Features: click or drag a box (Shift adds)" },
    { kind: "btn", icon: "clearSelection", tip: "Clear Selected Features", action: () => clearSelection() },
    { kind: "tool", tool: "selectEl", icon: "selectElements", tip: "Select Elements" },
    { kind: "sep" },
    { kind: "tool", tool: "identify", icon: "identify", tip: "Identify" },
    { kind: "btn", icon: "htmlPopup", tip: "HTML Popup", disabled: true },
    { kind: "btn", icon: "find", tip: "Find", action: () => findDialog() },
    { kind: "btn", icon: "findRoute", tip: "Find Route", disabled: true },
    { kind: "tool", tool: "measure", icon: "measure", tip: "Measure: click to add points, double-click to finish" },
    { kind: "btn", icon: "goToXY", tip: "Go To XY", action: () => goToXYDialog() },
    { kind: "btn", icon: "timeSlider", tip: "Time Slider", disabled: true },
    { kind: "btn", icon: "viewer", tip: "Create Viewer Window", action: () => window.open("/viewer", "_blank") },
  ] },
  { id: "tb-standard", name: "Standard", row: 1, items: [
    { kind: "btn", icon: "addData", tip: "Add Data", disabled: true },
    { kind: "custom", render: () => h("input", { class: "tinput scaleBox", id: "scaleBox", title: "Map Scale",
        onkeydown: (e) => { if (e.key === "Enter") zoomToScale(e.target.value); } }) },
    { kind: "sep" },
    { kind: "btn", icon: "editor", tip: "Editor: survey entry task", action: () => openTask("survey") },
    { kind: "btn", icon: "listDrawing", tip: "Table Of Contents Window", action: () => togglePane("tocPane") },
    { kind: "btn", icon: "catalog", tip: "Catalog Window", action: () => togglePane("catalogPane") },
    { kind: "btn", icon: "search", tip: "Search (find parcel)", action: () => findDialog() },
    { kind: "btn", icon: "arcToolbox", tip: "ArcToolbox", disabled: true },
    { kind: "btn", icon: "python", tip: "Python", disabled: true },
    { kind: "btn", icon: "modelBuilder", tip: "ModelBuilder", disabled: true },
  ] },
  { id: "tb-drawing", name: "Drawing", row: 1, right: true, items: [
    { kind: "drop", label: "Drawing", menu: () => [
      { label: "New Rectangle", icon: "rectangle", action: () => setTool("rect") },
      { label: "New Text", icon: "text", action: () => setTool("text") },
      "-",
      { label: "Delete Selected Elements", action: deleteSelectedGraphics, disabled: () => !state.graphics.some((g) => g.selected) },
      { label: "Delete All Elements", action: () => { state.graphics = []; renderGraphics(); } },
    ] },
    { kind: "tool", tool: "selectEl", icon: "selectElements", tip: "Select Elements" },
    { kind: "btn", icon: "rotate", tip: "Rotate", disabled: true },
    { kind: "btn", icon: "zoomToElements", tip: "Zoom to Selected Elements", action: () => zoomToGraphics() },
    { kind: "sep" },
    { kind: "tool", tool: "rect", icon: "rectangle", tip: "Rectangle", dropArrow: true },
    { kind: "tool", tool: "text", icon: "text", tip: "Text", dropArrow: true },
    { kind: "btn", icon: "editVertices", tip: "Edit Vertices", disabled: true },
    { kind: "custom", render: () => h("select", { class: "fontBox", id: "fontBox", title: "Font" },
        ...["Arial", "Tahoma", "Times New Roman", "Courier New", "Verdana"].map((f) => h("option", {}, f))) },
    { kind: "custom", render: () => h("select", { class: "sizeBox", id: "sizeBox", title: "Size" },
        ...[8, 9, 10, 11, 12, 14, 16, 18, 20, 24, 28, 36].map((s) => h("option", { selected: s === 10 }, String(s)))) },
    { kind: "toggle", id: "fmtB", icon: "bold", tip: "Bold" },
    { kind: "toggle", id: "fmtI", icon: "italic", tip: "Italic" },
    { kind: "toggle", id: "fmtU", icon: "underline", tip: "Underline" },
    { kind: "color", id: "colFont", icon: "fontColor", tip: "Font Color", value: "#d10000" },
    { kind: "color", id: "colFill", icon: "fillColor", tip: "Fill Color", value: "#ffd400" },
    { kind: "color", id: "colLine", icon: "lineColor", tip: "Line Color", value: "#7a2a9a" },
    { kind: "color", id: "colMarker", icon: "markerColor", tip: "Marker Color", value: "#1aa31a" },
  ] },
  { id: "tb-editor", name: "Editor", row: 2, indent: true, items: [
    { kind: "drop", label: "Editor", menu: () => editorMenu() },
    { kind: "drop", label: "Computation", menu: () => computationMenu() },
    { kind: "tool", tool: "sketch", icon: "sketchTool", tip: "Sketch tool: click to add vertices, double-click to finish" },
    { kind: "tool", tool: "editVertex", icon: "editTool", tip: "Edit tool: click a draft parcel to edit its corners; drag vertices to move them" },
    { kind: "btn", icon: "trace", tip: "Trace", disabled: true },
    { kind: "sep" },
    { kind: "btn", id: "edCreate", icon: "createFeatures", tip: "Create Features window", action: () => openCreateFeatures() },
    { kind: "btn", id: "edProps", icon: "sketchProps", tip: "Edit Sketch Properties", action: () => openSketchProps() },
    { kind: "sep" },
    { kind: "btn", id: "edFinish", icon: "finishSketch", tip: "Finish Sketch (F2)", action: () => finishSketch() },
    { kind: "btn", id: "edDelete", icon: "deleteSketch", tip: "Delete Sketch", action: () => deleteSketch() },
    { kind: "btn", id: "edSnap", icon: "snapping", tip: "Snapping on/off", action: () => toggleSnapping() },
    { kind: "btn", icon: "attributes", tip: "Attributes", disabled: true },
  ] },
  { id: "tb-cadastre", name: "Cadastre", row: 2, items: [
    { kind: "label", text: "Region:" },
    { kind: "custom", render: () => h("select", { class: "regionBox", id: "regionBox", title: "Region" }, h("option", {}, REGION)) },
    { kind: "btn", icon: "refresh", tip: "Refresh region data", action: () => refreshAll() },
    { kind: "btn", icon: "find", tip: "Find parcel or beacon in region", action: () => findDialog() },
    { kind: "label", text: "Job Area:" },
    { kind: "custom", render: () => h("select", { class: "jobBox", id: "jobBox", title: "Job Area", onchange: (e) => selectJob(e.target.value) },
        h("option", { value: "" }, "")) },
    { kind: "btn", id: "jbOpen", icon: "jobOpen", tip: "New job…", action: () => newJobDialog() },
    { kind: "btn", id: "jbSurvey", icon: "survey", tip: "New parcel from survey (task)", action: () => openTask("survey") },
    { kind: "btn", id: "jbSubmit", icon: "jobSubmit", tip: "Submit job", action: () => doTransition("submit") },
    { kind: "btn", id: "jbReview", icon: "jobReview", tip: "Start review", action: () => doTransition("start_review") },
    { kind: "btn", id: "jbApprove", icon: "jobApprove", tip: "Approve job", action: () => doTransition("approve") },
    { kind: "btn", id: "jbPost", icon: "jobPost", tip: "Post job (allocate UPINs)", action: () => postJob() },
    { kind: "btn", id: "jbPlan", icon: "sitePlan", tip: "Site plan (PDF)", action: () => openPdf("siteplan") },
    { kind: "btn", id: "jbReport", icon: "report", tip: "Job report (PDF)", action: () => openPdf("report") },
    { kind: "btn", id: "jbValidate", icon: "validate", tip: "Validate fabric", action: () => validateFabric() },
  ] },
];
const toolButtons = new Map(); // tool -> [elements]
function buildToolbars() {
  const hidden = JSON.parse(localStorage.getItem("cad.hiddenToolbars") || "[]");
  for (const row of [1, 2]) $(`tbrow${row}`).innerHTML = "";
  for (const def of TOOLBAR_DEFS) {
    const tb = h("div", { class: "toolbar", id: def.id, title: "" }, h("span", { class: "grip" }));
    if (def.right) tb.style.marginLeft = "auto";
    if (def.indent) tb.style.marginLeft = "clamp(0px, calc(24vw - 10px), 372px)";
    if (hidden.includes(def.id)) tb.classList.add("hidden");
    for (const it of def.items) tb.append(toolbarItem(it));
    tb.append(h("span", { class: "chev" }, "▾"));
    $(`tbrow${def.row}`).append(tb);
  }
  setTool(state.tool);
}
function toolbarItem(it) {
  switch (it.kind) {
    case "sep": return h("span", { class: "tsep" });
    case "label": return h("span", { class: "tlabel" }, it.text);
    case "custom": return it.render();
    case "drop": {
      const d = h("span", { class: "tdrop" }, it.label);
      d.addEventListener("mousedown", (ev) => { ev.stopPropagation(); closeMenus(); contextMenu(it.menu(), d.getBoundingClientRect().left, d.getBoundingClientRect().bottom); });
      return d;
    }
    case "color": {
      const input = h("input", { type: "color", id: it.id, value: it.value, class: "hidden" });
      const b = h("span", { class: "tb drop", title: it.tip, html: iconWithSwatch(it.icon, it.value) }, input);
      b.addEventListener("click", () => input.click());
      input.addEventListener("input", () => { b.innerHTML = iconWithSwatch(it.icon, input.value); b.append(input); applyStyleToSelected(); });
      return b;
    }
    case "toggle": {
      const b = h("span", { class: "tb", id: it.id, title: it.tip, html: icon(it.icon) });
      b.addEventListener("click", () => { b.classList.toggle("active"); applyStyleToSelected(); });
      return b;
    }
    default: {
      const b = h("span", { class: `tb${it.disabled ? " disabled" : ""}${it.dropArrow ? " drop" : ""}`, id: it.id, title: it.tip, html: icon(it.icon) });
      b.addEventListener("mouseenter", () => status(it.tip));
      b.addEventListener("mouseleave", () => status(""));
      if (!it.disabled) b.addEventListener("click", () => {
        if (b.classList.contains("disabled")) return;
        if (it.kind === "tool") setTool(it.tool); else it.action();
      });
      if (it.kind === "tool") { if (!toolButtons.has(it.tool)) toolButtons.set(it.tool, []); toolButtons.get(it.tool).push(b); }
      return b;
    }
  }
}
function iconWithSwatch(name, color) {
  // Recolour the swatch bar (last rect) of the colour icons to the chosen colour.
  return icon(name).replace(/(<rect[^>]*?fill=")#[0-9a-fA-F]{6}("[^>]*\/>)(?![\s\S]*<rect)/, `$1${color}$2`);
}
const toolbarHidden = (id) => $(id)?.classList.contains("hidden");
function toggleToolbar(id) {
  $(id).classList.toggle("hidden");
  const hidden = TOOLBAR_DEFS.map((t) => t.id).filter(toolbarHidden);
  localStorage.setItem("cad.hiddenToolbars", JSON.stringify(hidden));
}
function togglePane(id) { $(id).classList.toggle("hidden"); map.resize(); }
// Panes resize by dragging their inner edge (widths remembered per browser).
document.querySelectorAll(".paneResize").forEach((bar) => {
  const pane = $(bar.dataset.resize); const leftSide = bar.dataset.resize === "tocPane";
  const saved = Number(localStorage.getItem("cad.w." + pane.id)); if (saved > 120) pane.style.width = saved + "px";
  bar.addEventListener("mousedown", (e) => {
    e.preventDefault(); bar.classList.add("dragging");
    const x0 = e.clientX, w0 = pane.getBoundingClientRect().width;
    const move = (ev) => { const w = Math.max(150, Math.min(560, w0 + (leftSide ? ev.clientX - x0 : x0 - ev.clientX))); pane.style.width = w + "px"; map.resize(); };
    const up = () => { bar.classList.remove("dragging"); localStorage.setItem("cad.w." + pane.id, parseInt(pane.style.width)); window.removeEventListener("mousemove", move); window.removeEventListener("mouseup", up); };
    window.addEventListener("mousemove", move); window.addEventListener("mouseup", up);
  });
});
// Small screens: the Catalog starts closed (it floats over the map there); reopen from Windows ▸ Catalog.
if (window.matchMedia("(max-width: 1024px)").matches) $("catalogPane").classList.add("hidden");
if (window.matchMedia("(max-width: 640px)").matches) $("tocPane").classList.add("hidden");
new ResizeObserver(() => map.resize()).observe($("mapFrame"));
document.querySelectorAll("[data-pane-close]").forEach((b) => { b.innerHTML = icon("close"); b.onclick = () => togglePane(b.dataset.paneClose); });
document.querySelectorAll("[data-pane-pin]").forEach((b) => { b.innerHTML = icon("pin"); });

// --- map strip (data/layout view, refresh, pause drawing) ---
$("stripButtons").append(
  h("span", { class: "tb active", title: "Data View", html: icon("dataView") }),
  h("span", { class: "tb disabled", title: "Layout View", html: icon("layout") }),
  h("span", { class: "tb", title: "Refresh", html: icon("refresh"), onclick: () => refreshAll() }),
  h("span", { class: "tb", id: "pauseBtn", title: "Pause Drawing", html: icon("pause"), onclick: (e) => {
    const b = e.currentTarget; b.classList.toggle("active");
    map.getCanvas().style.visibility = b.classList.contains("active") ? "hidden" : "";
  } }),
);

// --- scale box ---
function currentScale() {
  const lat = map.getCenter().lat * Math.PI / 180;
  const mpp = 156543.03392 * Math.cos(lat) / 2 ** map.getZoom() / 2; // 512px tiles in MapLibre
  return mpp * 96 / 0.0254;
}
function updateScaleBox() {
  const s = currentScale();
  const box = $("scaleBox"); if (box && document.activeElement !== box) box.value = "1:" + Math.round(s).toLocaleString("en-US");
}
function zoomToScale(text) {
  const s = parseFloat(String(text).replace(/^1\s*:\s*/, "").replace(/,/g, ""));
  if (!(s > 0)) return;
  map.zoomTo(map.getZoom() + Math.log2(currentScale() / s));
}
map.on("moveend", updateScaleBox);

/* =====================================================================================
 * Phase 2 — Table Of Contents
 * ===================================================================================== */
// Layer model (drawing order, top first). Each leaf maps to MapLibre layer ids + legend entries.
const LAYERS = [
  { id: "cadastral", name: "PLAGIS CADASTRE - Cadastral Fabric", group: true, visible: true, open: true, children: [
    { id: "history", name: `${REGION}_Parcels (history)`, kind: "polygon", visible: false, open: false,
      map: ["history-fill", "history-line"], source: "cadastre.parcel (historic / archived; in view when zoomed in)",
      legend: () => [{ sym: "fill", fill: "#ddd", stroke: "#666", label: "Replaced or retired parcel" }] },
    { id: "parcels", name: `${REGION}_Parcels`, kind: "polygon", visible: true, open: true,
      map: ["parcels-fill", "parcels-line", "parcels-focus", "parcels-focus-vtx", "parcels-sel", "parcels-label"], source: "cadastre.parcel",
      legend: () => [{ sym: "fill", fill: "#fff", stroke: PARCEL_STROKE, label: "<all other values>" },
                     ...(state.focusParcel ? [{ sym: "fill", fill: "#fff", stroke: "#ff0000", label: state.focusParcel }] : [])] },
    { id: "jobsArea", name: `${REGION}_JobsArea`, kind: "polygon", visible: true, open: true,
      map: ["job-area-fill", "job-area-line", "drafts-fill", "drafts-line"], source: "cadastre.job_area + cadastre.parcel (job drafts)",
      legend: () => [{ sym: "fill", fill: "#fff", stroke: "#e00000", label: wf.editing?.area ? `Job area: ${wf.detail?.job?.name || "job " + wf.editing.job_instance_id}${wf.editing.area_status === "closed" ? " (closed)" : ""}` : "" },
                     { sym: "fill", fill: "#fff", stroke: "#111", label: state.job ? `Draft parcels: ${state.job.reference}` : "" }] },
    { id: "lines", name: `${REGION}_Lines`, kind: "line", visible: true, open: true,
      map: ["survey-line"], source: "survey traverse (live)", legend: () => [{ sym: "line", stroke: "#e60000", label: "" }] },
    { id: "points", name: `${REGION}_Points`, kind: "point", visible: true, open: true,
      map: ["survey-pt"], source: "survey traverse (live)", legend: () => [{ sym: "cross", label: "" }] },
    { id: "ctrl", name: `${REGION}_CtrlPnts`, kind: "point", visible: true, open: true,
      map: ["beacons-pt"], source: "cadastre.beacon", legend: () => [{ sym: "dot", fill: "#55ff00", stroke: "#1a3d00", label: "" }] },
  ] },
  { id: "imagery", name: "World_Imagery", kind: "raster", visible: true, open: true, map: ["imagery"], source: "Esri World Imagery (tiles)",
    legend: () => [{ text: "RGB" }, { sym: "swatch", fill: "#ff0000", label: "Red:    Band_1" },
                   { sym: "swatch", fill: "#00ff00", label: "Green: Band_2" }, { sym: "swatch", fill: "#0000ff", label: "Blue:   Band_3" }] },
];
let tocSelected = "dataframe";
const allLeaves = () => LAYERS.flatMap((l) => (l.group ? l.children : [l]));
const findLayer = (id) => LAYERS.find((l) => l.id === id) || allLeaves().find((l) => l.id === id);
function effectiveVisible(l) {
  const parent = LAYERS.find((g) => g.group && g.children.includes(l));
  return l.visible && (!parent || parent.visible);
}
function applyVisibility() {
  setTimeout(loadHistory, 0);
  for (const l of allLeaves()) for (const id of l.map) if (map.getLayer(id)) map.setLayoutProperty(id, "visibility", effectiveVisible(l) ? "visible" : "none");
  applyDynVisibility();
}
function applyDynVisibility() {
  for (const l of state.dynLayers) for (const id of l.map) if (map.getLayer(id)) map.setLayoutProperty(id, "visibility", l.visible ? "visible" : "none");
}
function legendSym(e) {
  if (e.text) return null;
  const svg = (inner) => `<svg class="sym" viewBox="0 0 16 11">${inner}</svg>`;
  switch (e.sym) {
    case "fill": return svg(`<rect x="1.5" y="1" width="13" height="9" fill="${e.fill}" stroke="${e.stroke}" stroke-width="1.2"/>`);
    case "line": return svg(`<path d="M1 5.5h14" stroke="${e.stroke}" stroke-width="1.6"/>`);
    case "cross": return svg(`<path d="M8 1v9M3.5 5.5h9" stroke="#000" stroke-width="1.2"/>`);
    case "dot": return svg(`<circle cx="8" cy="5.5" r="3.5" fill="${e.fill}" stroke="${e.stroke}"/>`);
    case "swatch": return svg(`<rect x="1" y="1" width="14" height="9" fill="${e.fill}"/>`);
    default: return svg("");
  }
}
function twisty(open, onClick) {
  const t = h("span", { class: "tw" }, open ? "−" : "+");
  t.addEventListener("click", (ev) => { ev.stopPropagation(); onClick(); });
  return t;
}
function layerNode(l) {
  const li = h("li");
  const hasKids = l.group ? l.children.length : l.legend().length;
  const lbl = h("span", { class: `lbl${tocSelected === l.id ? " sel" : ""}` }, l.name);
  const cb = h("input", { type: "checkbox", checked: l.visible, title: "Visible" });
  cb.addEventListener("click", (ev) => { ev.stopPropagation(); l.visible = cb.checked; applyVisibility(); });
  const row = h("div", { class: "node" }, hasKids ? twisty(l.open, () => { l.open = !l.open; buildTOC(); }) : h("span", { class: "tw none" }), cb, lbl);
  row.addEventListener("click", () => { tocSelected = l.id; buildTOC(); });
  row.addEventListener("dblclick", () => zoomToLayer(l));
  row.addEventListener("contextmenu", (ev) => { ev.preventDefault(); tocSelected = l.id; buildTOC(); layerContextMenu(l, ev.clientX, ev.clientY); });
  li.append(row);
  if (l.open && hasKids) {
    const ul = h("ul");
    if (l.group) l.children.forEach((c) => ul.append(layerNode(c)));
    else for (const e of l.legend()) {
      ul.append(h("li", {}, h("div", { class: "leaf" }, e.text ? h("span", { class: "symLabel", style: "margin-left:22px" }, e.text)
        : [h("span", { html: legendSym(e) }), h("span", { class: "symLabel" }, e.label)])));
    }
    li.append(ul);
  }
  return li;
}
function buildTOC() {
  const body = $("tocBody"); body.innerHTML = "";
  const root = h("ul", { class: "tree" });
  const df = h("li");
  const dfRow = h("div", { class: "node" }, twisty(true, () => {}), h("span", { html: icon("layers") }),
    h("span", { class: `lbl${tocSelected === "dataframe" ? " sel" : ""}` }, "Layers"));
  dfRow.addEventListener("click", () => { tocSelected = "dataframe"; buildTOC(); });
  df.append(dfRow);
  const ul = h("ul");
  if (state.tocView === "drawing") { state.dynLayers.forEach((l) => ul.append(dynLayerNode(l))); LAYERS.forEach((l) => ul.append(layerNode(l))); }
  else if (state.tocView === "source") {
    const bySrc = new Map();
    for (const l of allLeaves()) { const k = l.source.split(" ")[0]; if (!bySrc.has(k)) bySrc.set(k, []); bySrc.get(k).push(l); }
    for (const [src, ls] of bySrc) {
      const g = h("li", {}, h("div", { class: "node" }, h("span", { class: "tw none" }), h("span", { html: icon(src.startsWith("cadastre") ? "gdb" : "folder") }), h("span", { class: "lbl" }, src)));
      const gu = h("ul"); ls.forEach((l) => gu.append(layerNode(l))); g.append(gu); ul.append(g);
    }
  } else if (state.tocView === "visibility") {
    for (const [title, test] of [["Visible", (l) => effectiveVisible(l)], ["Not Visible", (l) => !effectiveVisible(l)]]) {
      const g = h("li", {}, h("div", { class: "node" }, h("span", { class: "tw none" }), h("span", { class: "lbl" }, title)));
      const gu = h("ul"); allLeaves().filter(test).forEach((l) => gu.append(layerNode(l))); g.append(gu); ul.append(g);
    }
  } else if (state.tocView === "selection") {
    const n = state.selection.size;
    ul.append(h("li", {}, h("div", { class: "node" }, h("span", { class: "tw none" }), h("span", { html: icon("polygonFc") }),
      h("span", { class: "lbl" }, `${REGION}_Parcels`), h("span", { class: "muted" }, ` (${n} selected)`))));
  }
  df.append(ul); root.append(df); body.append(root);
  applyVisibility();
}
function zoomToLayer(l) {
  const regionExtent = () => map.fitBounds([REGION_BBOX.slice(0, 2), REGION_BBOX.slice(2)], { padding: 20 });
  if (l.kind === "raster") { regionExtent(); return; } // the basemap: its working extent is the region
  const both = { type: "FeatureCollection", features: [...state.parcels.features, ...state.beacons.features] };
  const fc = { parcels: state.parcels, jobsArea: state.drafts, ctrl: state.beacons, cadastral: both,
    lines: state.surveyFC, points: state.surveyFC }[l.id];
  // The layer's data inside the region first (a stray outlier must not zoom out to half the
  // country); if it has none there, all of it.
  if (!fc || !(zoomToFC(fc, { padding: 40 }, true) || zoomToFC(fc, { padding: 40 }))) status(`${l.name}: nothing to zoom to`);
}
// --- dropped Catalog layers (rasters) ---
function dynBounds(l) {
  const xs = l.bbox.map((c) => c[0]), ys = l.bbox.map((c) => c[1]);
  return [[Math.min(...xs), Math.min(...ys)], [Math.max(...xs), Math.max(...ys)]];
}
// Zoom to a layer added from the Catalog; capped at street level so a single small feature
// does not zoom in past readable detail.
function zoomToDynLayer(l) { map.fitBounds(dynBounds(l), { padding: 40, maxZoom: 18 }); }
function dynLayerNode(l) {
  const li = h("li");
  const cb = h("input", { type: "checkbox", checked: l.visible, title: "Visible" });
  cb.addEventListener("click", (ev) => { ev.stopPropagation(); l.visible = cb.checked; applyDynVisibility(); });
  const icName = l.icon || (l.kind === "raster" ? "raster" : "lineFc");
  const row = h("div", { class: "node", title: l.unc || l.name }, h("span", { class: "tw none" }), cb, h("span", { html: icon(icName) }),
    h("span", { class: `lbl${tocSelected === l.id ? " sel" : ""}` }, l.name));
  row.addEventListener("click", () => { tocSelected = l.id; buildTOC(); });
  row.addEventListener("dblclick", () => zoomToDynLayer(l));
  row.addEventListener("contextmenu", (ev) => {
    ev.preventDefault(); tocSelected = l.id; buildTOC();
    contextMenu([
      { label: "Zoom To Layer", icon: "fullExtent", action: () => zoomToDynLayer(l) },
      { label: l.visible ? "Hide Layer" : "Show Layer", action: () => { l.visible = !l.visible; applyDynVisibility(); buildTOC(); } },
      { label: "Transparency 50%", action: () => { l.map.forEach((id) => map.getLayer(id) && map.setPaintProperty(id, "raster-opacity", 0.5)); }, disabled: l.kind !== "raster" },
      { label: "Full opacity", action: () => { l.map.forEach((id) => map.getLayer(id) && map.setPaintProperty(id, "raster-opacity", 1)); }, disabled: l.kind !== "raster" },
      // ArcMap's "Display Background Value": show an ortho's black (0,0,0) or white (255,255,255)
      // collar as transparent.
      { label: "Display Background As Transparent", disabled: l.kind !== "raster", sub: [
        { label: (l.background === 0 ? "\u2713 " : "") + "Black (0, 0, 0)", action: () => setRasterBackground(l, 0) },
        { label: (l.background === 255 ? "\u2713 " : "") + "White (255, 255, 255)", action: () => setRasterBackground(l, 255) },
        { label: (l.background == null ? "\u2713 " : "") + "None (show background)", action: () => setRasterBackground(l, null) },
      ] },
      "-",
      { label: "Remove", action: () => removeDynLayer(l) },
    ], ev.clientX, ev.clientY);
  });
  li.append(row);
  return li;
}
function setRasterBackground(l, value) {
  l.background = value;
  const src = map.getSource(l.src); if (!src) return;
  src.updateImage({ url: l.ovUrl + (value == null ? "" : `&background=${value}`), coordinates: l.bbox });
  status(value == null ? `${l.name}: background shown.` : `${l.name}: ${value === 0 ? "black" : "white"} background shown as transparent.`);
}
function removeDynLayer(l) {
  for (const id of l.map) if (map.getLayer(id)) map.removeLayer(id);
  if (l.src && map.getSource(l.src)) map.removeSource(l.src);
  if (l.gfxId) { state.graphics = state.graphics.filter((g) => g.id !== l.gfxId); renderGraphics(); }
  state.dynLayers = state.dynLayers.filter((x) => x !== l);
  buildTOC();
}
// Put rasters just above the base imagery so cadastral vectors stay on top.
const firstVectorLayer = () => ["parcels-fill", "drafts-fill", "beacons-pt"].find((id) => map.getLayer(id));
async function addRasterLayer(node) {
  const { folder, path, name } = node;
  status(`Adding ${name}…`);
  let info;
  try { info = await api(`/api/v1/catalog/raster/info?folder=${folder}&path=${encodeURIComponent(path)}`); }
  catch (e) { status(`Could not read ${name}: ${e.message}`); alert(`Could not read ${name}:\n${e.message}`); return; }
  const crs = resolveCrs(info.epsg);
  const code = crs.code;
  const toLLc = (x, y) => (code === "EPSG:4326" ? [x, y] : proj4(code, "EPSG:4326", [x, y]));
  const [minx, miny, maxx, maxy] = info.bbox;
  const tl = toLLc(minx, maxy), tr = toLLc(maxx, maxy), br = toLLc(maxx, miny), bl = toLLc(minx, miny);
  const id = "r" + dynSeq++;
  const srcId = "rsrc-" + id, lyrId = "rlyr-" + id;
  if (info.decodable) {
    const url = `/api/v1/catalog/raster/overview?folder=${folder}&path=${encodeURIComponent(path)}&max=2048`;
    map.addSource(srcId, { type: "image", url, coordinates: [tl, tr, br, bl] });
    map.addLayer({ id: lyrId, type: "raster", source: srcId, paint: { "raster-opacity": 1 } }, firstVectorLayer());
    state.dynLayers.unshift({ id, name, kind: "raster", map: [lyrId], src: srcId, bbox: [tl, tr, br, bl], visible: true, unc: catUNC(node), icon: "raster", ovUrl: url, background: null });
    status(crs.known ? `${name} added to the map.` : `${name} added. Its coordinate system is not stated in the file, so WGS 1984 UTM Zone 32N (the fabric's) is assumed.`);
  } else {
    const gfxId = addGraphic({ type: "line", pts: [tl, tr, br, bl, tl] });
    state.dynLayers.unshift({ id, name: name + " (extent)", kind: "rasterExtent", map: [], bbox: [tl, tr, br, bl], visible: true, unc: catUNC(node), gfxId });
    status(`${name}: too large to preview without tiling — added its extent. Full pixels need the ortho tiling pipeline.`);
  }
  buildTOC();
  map.fitBounds(dynBounds(state.dynLayers[0]), { padding: 40 });
}

// Distinct outline colours for successive shapefile layers (readable on imagery).
const SHP_COLORS = ["#ff7f00", "#00e5ff", "#ff3df5", "#7cff3d", "#ffd400", "#ff4040", "#9a7dff"];
// Reproject every position in a GeoJSON geometry (any type) with `fwd`.
function reprojectCoords(c, fwd) {
  return typeof c[0] === "number" ? fwd(c) : c.map((x) => reprojectCoords(x, fwd));
}
async function addShapefileLayer(node) {
  const { folder, path, name } = node;
  status(`Adding ${name}…`);
  let d;
  try { d = await api(`/api/v1/catalog/shapefile?folder=${folder}&path=${encodeURIComponent(path)}`); }
  catch (e) { status(`Could not read ${name}: ${e.message}`); alert(`Could not read ${name}:\n${e.message}`); return; }
  const feats = (d.geojson && d.geojson.features) || [];
  if (!feats.length) { status(`${name} has no features to draw.`); return; }
  const crs = resolveCrs(d.epsg);
  const fwd = crs.code === "EPSG:4326" ? (p) => [p[0], p[1]] : ((t) => (p) => t.forward([p[0], p[1]]))(proj4(crs.code, "EPSG:4326"));
  const fc = { type: "FeatureCollection", features: feats.filter((f) => f.geometry).map((f) => ({ ...f, geometry: { ...f.geometry, coordinates: reprojectCoords(f.geometry.coordinates, fwd) } })) };
  const [minx, miny, maxx, maxy] = d.bbox;
  const corners = [fwd([minx, maxy]), fwd([maxx, maxy]), fwd([maxx, miny]), fwd([minx, miny])];
  const id = "s" + dynSeq++, srcId = "ssrc-" + id;
  const color = SHP_COLORS[(dynSeq - 2) % SHP_COLORS.length];
  map.addSource(srcId, { type: "geojson", data: fc });
  // New vector layers go on top of the map, as ArcMap adds them.
  const ids = [];
  if (/polygon/.test(d.geometry_type)) {
    map.addLayer({ id: id + "-fill", type: "fill", source: srcId, paint: { "fill-color": color, "fill-opacity": 0.12 } }); ids.push(id + "-fill");
    map.addLayer({ id: id + "-line", type: "line", source: srcId, paint: { "line-color": color, "line-width": 1.6 } }); ids.push(id + "-line");
  } else if (/line/.test(d.geometry_type)) {
    map.addLayer({ id: id + "-line", type: "line", source: srcId, paint: { "line-color": color, "line-width": 1.8 } }); ids.push(id + "-line");
  } else {
    map.addLayer({ id: id + "-pt", type: "circle", source: srcId, paint: { "circle-radius": 4, "circle-color": color, "circle-stroke-color": "#000", "circle-stroke-width": 1 } }); ids.push(id + "-pt");
  }
  state.dynLayers.unshift({ id, name, kind: "shapefile", map: ids, src: srcId, bbox: corners, visible: true, unc: catUNC(node),
    icon: node.icon || "polygonFc", features: fc.features.length, epsg: crs.known ? d.epsg : null });
  buildTOC();
  map.fitBounds(dynBounds(state.dynLayers[0]), { padding: 40, maxZoom: 18 });
  const notes = [];
  if (!crs.known) notes.push("its coordinate system was not recognised, so WGS 1984 UTM Zone 32N is assumed");
  if (d.truncated) notes.push("it is very large, so only part of its geometry is shown");
  status(`${name} added: ${fc.features.length.toLocaleString()} feature(s)` + (notes.length ? `; ${notes.join("; ")}.` : "."));
}
// Add a Catalog item to the map, by its type (drag-and-drop, or "Add To Map").
function addCatalogLayer(n) {
  if (!n) return;
  if (n.kind === "raster") return addRasterLayer(n);
  if (n.kind === "shapefile") return addShapefileLayer(n);
  status(`${n.name} cannot be added to the map.`);
}

function layerContextMenu(l, x, y) {
  contextMenu([
    { label: "Zoom To Layer", icon: "fullExtent", action: () => zoomToLayer(l), disabled: l.kind === "graphics" },
    { label: l.visible ? "Hide Layer" : "Show Layer", action: () => { l.visible = !l.visible; buildTOC(); } },
    "-",
    { label: "Selection", sub: [
      { label: "Zoom To Selected Features", action: zoomToSelected, disabled: () => !state.selection.size },
      { label: "Clear Selected Features", action: clearSelection, disabled: () => !state.selection.size },
    ] },
    { label: "Open Attribute Table", icon: "table", action: () => attributeTable(l), disabled: !["parcels", "jobsArea", "ctrl"].includes(l.id) },
    "-",
    { label: "Properties…", action: () => layerProperties(l) },
  ], x, y);
}
function contextMenu(items, x, y) {
  closeMenus();
  const m = menuList(items); m.classList.add("ctx"); m.style.display = "block"; m.style.position = "fixed";
  m.style.left = x + "px"; m.style.top = y + "px"; document.body.append(m);
}
function buildTocTools() {
  const t = $("tocTools"); t.innerHTML = "";
  const views = [["drawing", "listDrawing", "List By Drawing Order"], ["source", "listSource", "List By Source"],
    ["visibility", "listVisibility", "List By Visibility"], ["selection", "listSelection", "List By Selection"]];
  for (const [v, ic, tip] of views) {
    const b = h("span", { class: `tb${state.tocView === v && !tasksOpen() && !wfOpen() ? " active" : ""}`, title: tip, html: icon(ic) });
    b.onclick = () => { state.tocView = v; showWorkflow(false); showTasks(false); buildTocTools(); buildTOC(); };
    t.append(b);
  }
  t.append(h("span", { class: "tsep" }));
  const tasksBtn = h("span", { class: `tb${tasksOpen() && !wfOpen() ? " active" : ""}`, title: "Tasks", html: icon("tasks") });
  tasksBtn.onclick = () => { showWorkflow(false); showTasks(true); buildTocTools(); };
  t.append(tasksBtn);
  const wfBtn = h("span", { class: `tb${wfOpen() ? " active" : ""}`, title: "Workflow (job instances)", html: icon("jobOpen") });
  wfBtn.onclick = () => openWorkflow();
  t.append(wfBtn);
  t.append(h("span", { class: "tb", title: "Options…", html: icon("options"), onclick: () => tocOptions() }));
}
const tasksOpen = () => !$("tasksBody").classList.contains("hidden");
function showTasks(on) {
  $("tasksBody").classList.toggle("hidden", !on); $("tocBody").classList.toggle("hidden", on);
  document.querySelector("#tocPane .hscroll").classList.toggle("hidden", on);
}
function tocOptions() {
  floatWindow("Table Of Contents Options", h("div", {},
    h("p", {}, "Layers are listed top-down in drawing order. Double-click a layer to zoom to it; right-click for more."),
    h("div", { class: "frow" }, h("button", { class: "btn", onclick: (e) => { LAYERS.forEach((l) => { l.open = true; (l.children || []).forEach((c) => (c.open = true)); }); buildTOC(); } }, "Expand all"),
      h("button", { class: "btn", onclick: () => { LAYERS.forEach((l) => { l.open = false; (l.children || []).forEach((c) => (c.open = false)); }); buildTOC(); } }, "Collapse all"))), { width: 320 });
}

/* =====================================================================================
 * Phase 3 — map tools
 * ===================================================================================== */
// MapLibre paints the cursor on its own <canvas>, which overrides any cursor set via a CSS class
// on the map container — so the active tool's cursor must be set on the canvas itself. "" lets
// MapLibre show its native grab/grabbing for Pan.
const TOOL_CURSOR = {
  pan: "", zoomIn: "zoom-in", zoomOut: "zoom-out", select: "crosshair", selectEl: "default",
  identify: "crosshair", measure: "crosshair", survey: "crosshair", rect: "crosshair", text: "text",
  sketch: "crosshair", editVertex: "pointer",
};
function setTool(tool) {
  // The sketch/edit tools require an active edit session, like ArcMap's "Start Editing".
  if ((tool === "sketch" || tool === "editVertex") && !edit.active) { startEditing(tool); return; }
  state.tool = tool;
  for (const [t, btns] of toolButtons) btns.forEach((b) => b.classList.toggle("active", t === tool));
  const c = map.getContainer(); c.className = c.className.replace(/\btool-\S+/g, "").trim(); c.classList.add("tool-" + tool);
  const cv = map.getCanvas(); if (cv) cv.style.cursor = tool in TOOL_CURSOR ? TOOL_CURSOR[tool] : "crosshair";
  // Pan is the only tool that drags the map; the others own the mouse.
  if (tool === "pan") map.dragPan.enable(); else map.dragPan.disable();
  if (tool !== "measure") clearMeasure();
}
function cancelInteraction() {
  if (box) { box.el.remove(); box = null; }
  if (state.tool === "measure") clearMeasure();
  if (edit.active && edit.verts.length) { clearSketch(); refreshSketchProps(); updateEditorButtons(); status("Sketch cancelled."); }
}

// extent history
function pushHistory() {
  if (state.navigating) { state.navigating = false; return; }
  const b = map.getBounds().toArray();
  state.history = state.history.slice(0, state.histIdx + 1); state.history.push(b); state.histIdx = state.history.length - 1;
  updateNavButtons();
}
function goBack() { if (state.histIdx > 0) { state.histIdx--; state.navigating = true; map.fitBounds(state.history[state.histIdx], { animate: false }); updateNavButtons(); } }
function goForward() { if (state.histIdx < state.history.length - 1) { state.histIdx++; state.navigating = true; map.fitBounds(state.history[state.histIdx], { animate: false }); updateNavButtons(); } }
function updateNavButtons() {
  $("btnBack")?.classList.toggle("disabled", state.histIdx <= 0);
  $("btnFwd")?.classList.toggle("disabled", state.histIdx >= state.history.length - 1);
}
map.on("moveend", () => pushHistory());
function fullExtent() {
  if (!zoomToFC(state.parcels, {}, true) && !zoomToFC(state.beacons, {}, true)) map.fitBounds([REGION_BBOX.slice(0, 2), REGION_BBOX.slice(2)]);
}
// Deep link: /editor?center=E,N&scale=2500 (UTM 32N metres) opens straight to that view.
function initialViewFromURL() {
  const q = new URLSearchParams(location.search); const c = (q.get("center") || "").split(",").map(Number);
  if (c.length !== 2 || !c.every(isFinite)) return false;
  map.jumpTo({ center: toLL(c[0], c[1]), zoom: 16 });
  const sc = Number(q.get("scale")); if (sc > 0) map.jumpTo({ zoom: map.getZoom() + Math.log2(currentScale() / sc) });
  return true;
}

// drag-box for zoom/select/rectangle
let box = null;
const canvas = map.getCanvasContainer();
canvas.addEventListener("mousedown", (ev) => {
  if (ev.button !== 0 || !["zoomIn", "zoomOut", "select", "rect"].includes(state.tool)) return;
  const r = canvas.getBoundingClientRect();
  box = { x0: ev.clientX - r.left, y0: ev.clientY - r.top, el: h("div", { style: "position:absolute;border:1px dashed #000;background:rgba(0,120,255,.08);pointer-events:none;z-index:5" }), moved: false, shift: ev.shiftKey };
  canvas.append(box.el);
});
window.addEventListener("mousemove", (ev) => {
  if (!box) return;
  const r = canvas.getBoundingClientRect();
  const x = ev.clientX - r.left, y = ev.clientY - r.top;
  if (Math.abs(x - box.x0) + Math.abs(y - box.y0) > 4) box.moved = true;
  Object.assign(box.el.style, { left: Math.min(x, box.x0) + "px", top: Math.min(y, box.y0) + "px", width: Math.abs(x - box.x0) + "px", height: Math.abs(y - box.y0) + "px" });
  box.x1 = x; box.y1 = y;
});
window.addEventListener("mouseup", () => {
  if (!box) return;
  const b = box; box = null; b.el.remove();
  if (!b.moved) return; // treated as a click by the map handler
  const p0 = [Math.min(b.x0, b.x1), Math.min(b.y0, b.y1)], p1 = [Math.max(b.x0, b.x1), Math.max(b.y0, b.y1)];
  const sw = map.unproject([p0[0], p1[1]]), ne = map.unproject([p1[0], p0[1]]);
  suppressClickUntil = Date.now() + 250;
  if (state.tool === "zoomIn") map.fitBounds([sw, ne], { padding: 0 });
  else if (state.tool === "zoomOut") {
    const c = map.getCanvas(); const ratio = Math.min((p1[0] - p0[0]) / c.clientWidth, (p1[1] - p0[1]) / c.clientHeight);
    map.easeTo({ center: map.unproject([(p0[0] + p1[0]) / 2, (p0[1] + p1[1]) / 2]), zoom: map.getZoom() + Math.log2(Math.max(ratio, 0.05)) });
  } else if (state.tool === "select") {
    const feats = map.queryRenderedFeatures([p0, p1], { layers: ["parcels-fill"] });
    selectFeatures(feats.map((f) => f.properties.id), b.shift);
  } else if (state.tool === "rect") {
    addGraphic({ type: "rect", sw: [sw.lng, sw.lat], ne: [ne.lng, ne.lat] });
  }
});
let suppressClickUntil = 0; // a drag-box ends with mouseup; ignore any click the map derives from it

map.on("click", (e) => {
  if (Date.now() < suppressClickUntil) return;
  const pt = e.point;
  switch (state.tool) {
    case "zoomIn": map.easeTo({ center: e.lngLat, zoom: map.getZoom() + 1 }); break;
    case "zoomOut": map.easeTo({ center: e.lngLat, zoom: map.getZoom() - 1 }); break;
    case "select": {
      const f = map.queryRenderedFeatures(pt, { layers: ["parcels-fill"] })[0];
      selectFeatures(f ? [f.properties.id] : [], e.originalEvent.shiftKey); break;
    }
    case "identify": identifyAt(e); break;
    case "measure": measureClick(e.lngLat); break;
    case "survey": setSurveyStart(e.lngLat); break;
    case "text": addTextGraphic(e.lngLat); break;
    case "selectEl": selectGraphicAt(e); break;
    case "sketch": sketchClick(e); break;
    case "editVertex": if (!edit.verts.length) editDraftAt(pt); break;
    case "pickDraft": pickDraftAt(pt); break;
  }
});
// Edit tool on a saved draft parcel of the current job: its corners become the sketch, to move,
// retype (Edit Sketch Properties), add or delete; Finish Sketch saves the changed boundary.
function editDraftAt(pt) {
  const f = map.queryRenderedFeatures([[pt.x - 3, pt.y - 3], [pt.x + 3, pt.y + 3]], { layers: ["drafts-fill"].filter((l) => map.getLayer(l)) })[0];
  if (!f) { status("Edit tool: click one of the job's draft parcels to edit its corners."); return; }
  const d = state.drafts.features.find((x) => String(x.properties.id) === String(f.properties.id));
  if (!d) return;
  const ring = d.geometry.type === "MultiPolygon" ? d.geometry.coordinates[0][0] : d.geometry.coordinates[0];
  selectTemplate("parcel");
  edit.verts = ring.slice(0, -1).map(([x, y]) => toUTM(x, y).map((v) => Math.round(v * 1000) / 1000));
  edit.editingId = d.properties.id; edit.sel = -1;
  drawSketch(); openSketchProps(); updateEditorButtons();
  status(`Editing draft parcel corners (${edit.verts.length}). Drag them, or retype them in Edit Sketch Properties; Finish Sketch saves.`);
}
map.on("dblclick", (e) => {
  if (state.tool === "measure") { e.preventDefault(); finishMeasure(); }
  else if (state.tool === "sketch" && edit.verts.length) { e.preventDefault(); finishSketch(); }
});

// selection
function selectFeatures(ids, add) {
  if (!add) state.selection.clear();
  ids.forEach((id) => (add && state.selection.has(id) ? state.selection.delete(id) : state.selection.add(id)));
  map.setFilter("parcels-sel", ["in", ["get", "id"], ["literal", [...state.selection]]]);
  status(`${state.selection.size} feature(s) selected`);
  if (state.tocView === "selection") buildTOC();
}
function clearSelection() { selectFeatures([], false); }
function selectedFC() { return { type: "FeatureCollection", features: state.parcels.features.filter((f) => state.selection.has(f.properties.id)) }; }
function zoomToSelected() { zoomToFC(selectedFC()); }
function selectionStats() {
  const fs = selectedFC().features; const areas = fs.map((f) => Number(f.properties.area) || 0);
  const sum = areas.reduce((a, b) => a + b, 0);
  floatWindow("Selection Statistics", h("table", {},
    ...[["Count", fs.length], ["Total area (m²)", sum.toFixed(1)], ["Mean area (m²)", fs.length ? (sum / fs.length).toFixed(1) : "—"],
      ["Min area (m²)", fs.length ? Math.min(...areas).toFixed(1) : "—"], ["Max area (m²)", fs.length ? Math.max(...areas).toFixed(1) : "—"]]
      .map(([k, v]) => h("tr", {}, h("th", {}, k), h("td", {}, String(v))))), { width: 300 });
}
function focusParcel(f) {
  state.focusParcel = f ? f.properties.parcel_number || "(unposted)" : null;
  map.setFilter("parcels-focus", ["==", ["get", "id"], f ? f.properties.id : ""]);
  // Dot the focused parcel's vertices (closing vertex not repeated), as Aumentum does.
  // Use the full source geometry: a rendered feature is clipped at tile edges.
  const g = f && ((state.parcels.features.find((x) => x.properties.id === f.properties.id) || f).geometry);
  const rings = !g ? [] : g.type === "Polygon" ? g.coordinates : g.type === "MultiPolygon" ? g.coordinates.flat() : [];
  map.getSource("focus-vertices")?.setData({ type: "FeatureCollection",
    features: rings.flatMap((r) => r.slice(0, -1)).map((c) => ({ type: "Feature", geometry: { type: "Point", coordinates: c }, properties: {} })) });
  buildTOC();
}

// identify
function identifyAt(e) {
  const feats = map.queryRenderedFeatures([[e.point.x - 3, e.point.y - 3], [e.point.x + 3, e.point.y + 3]],
    { layers: ["beacons-pt", "drafts-fill", "parcels-fill", "history-fill"].filter((l) => map.getLayer(l) && map.getLayoutProperty(l, "visibility") !== "none") });
  const [E, N] = toUTM(e.lngLat.lng, e.lngLat.lat);
  const names = { "beacons-pt": `${REGION}_CtrlPnts`, "drafts-fill": `${REGION}_JobsArea`, "parcels-fill": `${REGION}_Parcels`, "history-fill": `${REGION}_Parcels (history)` };
  const body = h("div", {}, h("div", { class: "frow" }, h("label", {}, "Location:"), h("span", {}, `${fmt3(E)}  ${fmt3(N)} Meters`)));
  if (!feats.length) body.append(h("p", { class: "muted" }, "No identified features."));
  for (const f of feats) {
    const p = f.properties;
    const rows = Object.entries(p).filter(([k]) => k !== "id").map(([k, v]) => h("tr", {}, h("th", {}, k), h("td", {}, v == null || v === "null" ? "<Null>" : String(v))));
    body.append(h("h4", { style: "margin:8px 0 3px" }, names[f.layer.id]), h("table", {}, ...rows));
    if ((f.layer.id === "parcels-fill" || f.layer.id === "history-fill") && p.parcel_number)
      body.append(h("div", { class: "frow" }, h("button", { class: "btn", onclick: () => lineageWindow(p.parcel_number) }, "Lineage…")));
  }
  const top = feats.find((f) => f.layer.id === "parcels-fill");
  if (top) focusParcel(top);
  floatWindow("Identify", body, { id: "identifyWin", width: 330 });
}

// measure
let measurePts = [];
function measureClick(ll) {
  // A double-click also delivers two clicks at the same spot; don't add zero-length segments.
  const last = measurePts[measurePts.length - 1];
  if (last) { const a = map.project(last), b = map.project(ll); if (Math.hypot(a.x - b.x, a.y - b.y) < 3) return; }
  measurePts.push([ll.lng, ll.lat]); drawMeasure();
}
function drawMeasure(done) {
  map.getSource("measure").setData({ type: "FeatureCollection", features: measurePts.length > 1 ? [{ type: "Feature", geometry: { type: "LineString", coordinates: measurePts } }] : [] });
  const utm = measurePts.map(([x, y]) => toUTM(x, y));
  let total = 0, seg = 0;
  for (let i = 1; i < utm.length; i++) { seg = Math.hypot(utm[i][0] - utm[i - 1][0], utm[i][1] - utm[i - 1][1]); total += seg; }
  floatWindow("Measure", h("div", {},
    h("div", { class: "frow" }, h("label", {}, "Segment:"), h("b", {}, `${seg.toFixed(3)} Meters`)),
    h("div", { class: "frow" }, h("label", {}, "Length:"), h("b", {}, `${total.toFixed(3)} Meters`)),
    h("p", { class: "muted" }, done ? "Measurement finished. Click to start a new one." : "Click to add points; double-click to finish.")), { id: "measureWin", width: 260 });
  if (done) measurePts = [];
}
function finishMeasure() { drawMeasure(true); }
function clearMeasure() { measurePts = []; map.getSource("measure")?.setData(empty()); $("measureWin")?.remove(); }

// find / select by attributes
// Search results carry their own geometry (EPSG:32632), so zooming never depends on the parcel
// already being loaded in the map layer.
function resultFeature(p) {
  const toLLRing = (ring) => ring.map(([e, n]) => toLL(e, n));
  const g = p.geometry || {};
  const coordinates = g.type === "MultiPolygon" ? g.coordinates.map((poly) => poly.map(toLLRing)) : (g.coordinates || []).map(toLLRing);
  return { type: "Feature", properties: { id: p.id, parcel_number: p.parcel_number, status: p.status }, geometry: { type: g.type || "Polygon", coordinates } };
}
function goToResult(p) {
  const f = resultFeature(p);
  zoomToFC({ type: "FeatureCollection", features: [f] });
  focusParcel(f); selectFeatures([p.id]);
  status(`${p.parcel_number || "(unposted)"} — ${p.status}`);
}
// Find: parcels (UPIN / plot number) and beacons / control points (point IDs). Several IDs can be
// pasted at once, e.g. a surveyor's point list "CA6782, RX1271, GB7772, CA6783, SCS01": every
// one found is ringed on the map and the view fits them all; the missing ones are listed.
// PLAGIS wrote the surveyor's beacon CA6782 into Aumentum as C6782A (and back); see the server's
// plagis_alternate.
function plagisAlternate(t) {
  const s = t.trim(); let m;
  if ((m = s.match(/^([A-Za-z])([A-Za-z])(\d+)$/))) return m[1] + m[3] + m[2];
  if ((m = s.match(/^([A-Za-z])(\d+)([A-Za-z])$/))) return m[1] + m[3] + m[2];
  return null;
}
function splitFindTerms(q) {
  let terms = q.split(/[,;\n\t]+/).map((t) => t.trim()).filter(Boolean);
  if (terms.length === 1) {
    const words = terms[0].split(/\s+/);
    if (words.length > 1 && words.every((w) => /^[A-Za-z]+[0-9]+[A-Za-z0-9]*$/.test(w))) terms = words;
  }
  return terms;
}
function showFoundBeacons(list, zoom = true) {
  const fc = { type: "FeatureCollection", features: list.map((b) => ({ type: "Feature", geometry: { type: "Point", coordinates: toLL(b.easting, b.northing) },
    properties: { number: b.number, is_control: b.is_control, easting: b.easting, northing: b.northing } })) };
  map.getSource("found")?.setData(fc);
  if (!zoom || !list.length) return;
  if (list.length === 1) map.flyTo({ center: fc.features[0].geometry.coordinates, zoom: 18 });
  else zoomToFC(fc, { padding: 80, maxZoom: 18 });
}
function findDialog() {
  const input = h("input", { style: "flex:1", placeholder: "UPIN, plot no., or beacon IDs, e.g. CA6782, SCS01", value: "" });
  const results = h("div", { style: "margin-top:6px" });
  const run = async () => {
    const q = input.value.trim(); if (!q) return;
    const terms = splitFindTerms(q);
    results.textContent = "Searching…";
    try {
      const [beaconHits, parcels] = await Promise.all([
        Promise.all(terms.map(async (t) => {
          const l = await api(`/api/v1/beacons/search?limit=20&q=${encodeURIComponent(t)}`);
          const alt = plagisAlternate(t);
          const exact = l.filter((b) => b.number.toUpperCase() === t.toUpperCase() || (alt && b.number.toUpperCase() === alt.toUpperCase()));
          return { term: t, list: exact.length ? exact : l };
        })),
        terms.length === 1 ? api("/api/v1/parcels/search?number=" + encodeURIComponent(q)) : Promise.resolve([]),
      ]);
      results.innerHTML = "";
      const beacons = [], seen = new Set(), missing = [];
      for (const r of beaconHits) {
        if (!r.list.length) { missing.push(r.term); continue; }
        for (const b of r.list) if (!seen.has(b.id)) { seen.add(b.id); beacons.push(b); }
      }
      if (beacons.length) {
        const t = h("table", {}, h("tr", {}, h("th", {}, "Point ID"), h("th", {}, "Easting"), h("th", {}, "Northing"), h("th", {}, "Type")));
        for (const b of beacons) {
          const tr = h("tr", { style: "cursor:pointer", title: "Zoom to point" }, h("td", {}, b.number), h("td", {}, fmt3(b.easting)), h("td", {}, fmt3(b.northing)), h("td", {}, b.is_control ? "Control" : "Beacon"));
          tr.onclick = () => showFoundBeacons([b]);
          t.append(tr);
        }
        results.append(h("div", { class: "muted" }, `${beacons.length} point(s) — ringed in red; click a row to zoom`), t);
        showFoundBeacons(beacons);
      } else map.getSource("found")?.setData(empty());
      if (missing.length && terms.length > 1) results.append(h("div", { style: "margin-top:6px;color:#b00" }, `Not found: ${missing.join(", ")}`));
      if (parcels.length) {
        const t = h("table", {}, h("tr", {}, h("th", {}, "Parcel"), h("th", {}, "Status"), h("th", {}, "Area (m²)")));
        for (const p of parcels.slice(0, 200)) {
          const area = p.computed_area ?? p.stated_area;
          const tr = h("tr", { style: "cursor:pointer", title: "Zoom to parcel" }, h("td", {}, p.parcel_number || "(unposted)"), h("td", {}, p.status), h("td", {}, area != null ? Number(area).toFixed(1) : "—"));
          tr.onclick = () => goToResult(p);
          t.append(tr);
        }
        results.append(h("div", { class: "muted", style: "margin-top:6px" }, `${parcels.length} parcel(s)${parcels.length > 200 ? " (first 200 shown)" : ""} — click a row to zoom`), t);
        if (!beacons.length) { if (parcels.length === 1) goToResult(parcels[0]); else selectFeatures(parcels.map((p) => p.id)); }
      }
      if (!beacons.length && !parcels.length) {
        results.append(h("div", {}, `Nothing matches "${q}".`), h("div", { class: "muted" }, "Parcels: try the plot number alone (e.g. 5621) or part of the UPIN. Points: the beacon or control point ID."));
      }
    } catch (e) { results.textContent = "Search failed: " + e.message; }
  };
  input.addEventListener("keydown", (e) => e.key === "Enter" && run());
  floatWindow("Find", h("div", {}, h("div", { class: "frow" }, h("label", {}, "Find:"), input, h("button", { class: "btn", onclick: run }, "Find")),
    h("div", { class: "frow" }, h("label", {}, "In layers:"), h("span", {}, `${REGION}_Parcels, ${REGION}_Points, ${REGION}_CtrlPnts`)), results), { id: "findWin", width: 420 });
  setTimeout(() => input.focus(), 0);
}

// go to XY (UTM 32N metres)
function goToXYDialog() {
  const x = h("input", { type: "number", step: "0.001", placeholder: "Easting" }), y = h("input", { type: "number", step: "0.001", placeholder: "Northing" });
  const c = map.getCenter(); const [E, N] = toUTM(c.lng, c.lat); x.value = E.toFixed(3); y.value = N.toFixed(3);
  const go = (flash) => {
    const e = parseFloat(x.value), n = parseFloat(y.value); if (!isFinite(e) || !isFinite(n)) return;
    const ll = toLL(e, n); map.flyTo({ center: ll, zoom: Math.max(map.getZoom(), 17) });
    if (flash) { const m = new maplibregl.Marker({ color: "#e00" }).setLngLat(ll).addTo(map); setTimeout(() => m.remove(), 2500); }
  };
  floatWindow("Go To XY", h("div", {},
    h("div", { class: "frow" }, h("label", {}, "X (Easting):"), x), h("div", { class: "frow" }, h("label", {}, "Y (Northing):"), y),
    h("div", { class: "frow muted" }, "Units: Meters (WGS 84 / UTM zone 32N)"),
    h("div", { class: "frow" }, h("button", { class: "btn", onclick: () => go(false) }, "Pan To"), h("button", { class: "btn", onclick: () => go(true) }, "Flash"))), { id: "gotoWin", width: 300 });
}
function copyCenterXY() {
  const c = map.getCenter(); const [E, N] = toUTM(c.lng, c.lat);
  navigator.clipboard?.writeText(`${fmt3(E)} ${fmt3(N)}`); status(`Copied ${fmt3(E)} ${fmt3(N)}`);
}

// bookmarks
function saveBookmarks() { localStorage.setItem("cad.bookmarks", JSON.stringify(state.bookmarks)); }
function createBookmark() {
  const name = prompt("Bookmark Name:", `Bookmark ${state.bookmarks.length + 1}`); if (!name) return;
  state.bookmarks.push({ name, bounds: map.getBounds().toArray() }); saveBookmarks(); status(`Bookmark "${name}" created`);
}
function manageBookmarks() {
  const list = h("table");
  const render = () => {
    list.innerHTML = ""; list.append(h("tr", {}, h("th", {}, "Name"), h("th", {}, "")));
    state.bookmarks.forEach((b, i) => list.append(h("tr", {}, h("td", { style: "cursor:pointer", onclick: () => map.fitBounds(b.bounds) }, b.name),
      h("td", {}, h("button", { class: "btn", style: "min-width:0", onclick: () => { state.bookmarks.splice(i, 1); saveBookmarks(); render(); } }, "Remove")))));
    if (!state.bookmarks.length) list.append(h("tr", {}, h("td", { colspan: 2, class: "muted" }, "No bookmarks yet.")));
  };
  render();
  floatWindow("Bookmarks Manager", h("div", {}, list, h("div", { class: "frow" }, h("button", { class: "btn", onclick: () => { createBookmark(); render(); } }, "New…"))), { width: 320 });
}

// floating windows
function floatWindow(title, body, { id, width = 300 } = {}) {
  if (id) $(id)?.remove();
  const w = h("div", { class: "float", id: id || null, style: `width:${width}px` });
  const head = h("div", { class: "fhead" }, h("span", {}, title), h("span", { class: "fx", onclick: () => w.remove() }, "✕"));
  w.append(head, h("div", { class: "fbody" }, body));
  const r = $("mapFrame").getBoundingClientRect();
  const n = document.querySelectorAll(".float").length;
  w.style.left = Math.round(r.left + 40 + n * 24) + "px"; w.style.top = Math.round(r.top + 30 + n * 24) + "px";
  let drag = null;
  head.addEventListener("mousedown", (e) => { if (e.target.classList.contains("fx")) return; drag = [e.clientX - w.offsetLeft, e.clientY - w.offsetTop]; });
  window.addEventListener("mousemove", (e) => { if (drag) { w.style.left = e.clientX - drag[0] + "px"; w.style.top = e.clientY - drag[1] + "px"; } });
  window.addEventListener("mouseup", () => (drag = null));
  $("floats").append(w);
  return w;
}
function aboutDialog() {
  floatWindow(`About ${APP}`, h("div", {},
    h("p", {}, h("b", {}, APP)), h("p", {}, "Parcel capture, survey computation, job workflow and posting for PLAGIS."),
    h("p", {}, "This software is developed by ", h("b", {}, "Che Dilas Yusuph"), " and ", h("b", {}, "Catherine Wuyep"), "."),
    h("p", { class: "muted" }, `Coordinate system: WGS 1984 UTM Zone 32N (EPSG:32632). Connected as ${state.me ? state.me.username : "—"}.`)), { width: 340 });
}
/* =====================================================================================
 * Survey computation tools (the 35-tool catalogue) and survey/CAD/attribute import.
 * Both talk to the same `cadastre-core` engine the desktop editor and the server use, so a
 * bearing, closure or area computed here matches a headless report exactly.
 * ===================================================================================== */
const TOOL_CATEGORY_LABELS = {
  basic: "Basic COGO", intersections: "Intersections", curves: "Curves",
  area: "Area & adjustment", transformations: "Transformations",
  geodetic: "Geodetic", "line-utilities": "Line utilities",
};
const FIELD_HINTS = {
  legs: 'JSON legs, e.g. [{"type":"line","bearing_deg":90,"distance":100},{"type":"curve","radius":50,"arc_length":78.54,"chord_bearing_deg":135,"turn":"right"}]',
  ring: "One point per line: easting,northing (or a JSON array of points).",
};
let toolCatalogue = null;
async function loadToolCatalogue() {
  if (!toolCatalogue) toolCatalogue = await api("/api/v1/survey/tools");
  return toolCatalogue;
}
function parsePointValue(text) {
  const t = String(text).trim();
  if (!t) return undefined;
  if (t.startsWith("[") || t.startsWith("{")) {
    const v = JSON.parse(t);
    const p = Array.isArray(v) ? { x: Number(v[0]), y: Number(v[1]) } : { x: Number(v.x), y: Number(v.y) };
    if (!isFinite(p.x) || !isFinite(p.y)) throw new Error("point is not numeric");
    return p;
  }
  throw new Error("enter a point as easting,northing or {\"x\":…,\"y\":…}");
}
function parsePointsValue(text) {
  const t = String(text).trim();
  if (!t) return [];
  if (t.startsWith("[")) {
    const rows = JSON.parse(t);
    if (!Array.isArray(rows)) throw new Error("expected a JSON array of points");
    return rows.map((row) => {
      const p = Array.isArray(row) ? { x: Number(row[0]), y: Number(row[1]) } : { x: Number(row.x), y: Number(row.y) };
      if (!isFinite(p.x) || !isFinite(p.y)) throw new Error("a point in the array is not numeric");
      return p;
    });
  }
  return t.split(/\n|;/).map((s) => s.trim()).filter(Boolean).map((line) => {
    const [x, y] = line.split(/[,\s]+/).filter(Boolean).map(Number);
    if (!isFinite(x) || !isFinite(y)) throw new Error(`bad point line: ${line}`);
    return { x, y };
  });
}
function toolField(field) {
  const label = h("label", { title: field.description }, field.name);
  if (field.kind === "number") {
    const input = h("input", { type: "number", step: "any", style: "flex:1;min-width:0" });
    return {
      name: field.name,
      el: h("div", { class: "frow" }, label, input),
      value: () => (input.value.trim() === "" ? undefined : Number(input.value)),
    };
  }
  if (field.kind === "string") {
    const input = h("input", { type: "text", placeholder: field.description, style: "flex:1;min-width:0" });
    return {
      name: field.name,
      el: h("div", { class: "frow" }, label, input),
      value: () => (input.value.trim() === "" ? undefined : input.value.trim()),
    };
  }
  if (field.kind === "point") {
    const x = h("input", { type: "number", step: "any", placeholder: "E", style: "width:110px" });
    const y = h("input", { type: "number", step: "any", placeholder: "N", style: "width:110px" });
    const pick = h("button", {
      class: "btn", style: "min-width:0", title: "Use the current map centre",
      onclick: () => { const [E, N] = toUTM(map.getCenter().lng, map.getCenter().lat); x.value = E.toFixed(3); y.value = N.toFixed(3); },
    }, "Map centre");
    return {
      name: field.name,
      el: h("div", { class: "frow", style: "flex-wrap:wrap" }, label, h("span", { class: "muted" }, "E"), x, h("span", { class: "muted" }, "N"), y, pick),
      value: () => {
        if (x.value.trim() === "" && y.value.trim() === "") return undefined;
        const p = { x: Number(x.value), y: Number(y.value) };
        if (!isFinite(p.x) || !isFinite(p.y)) throw new Error(`${field.name}: enter both easting and northing`);
        return p;
      },
    };
  }
  // `points` and `array` both take JSON. Points also accept a simple "easting,northing" line list.
  const area = h("textarea", {
    rows: field.kind === "points" ? 4 : 5,
    placeholder: FIELD_HINTS[field.name] || (field.kind === "points" ? "easting,northing per line" : "JSON array"),
    style: "width:100%;box-sizing:border-box;font-family:ui-monospace,monospace;font-size:11px",
  });
  return {
    name: field.name,
    el: h("div", { style: "margin:6px 0" }, h("div", { style: "font-weight:600;margin-bottom:2px" }, field.name),
      h("div", { class: "muted", style: "font-size:11px;margin-bottom:2px" }, field.description), area),
    value: () => {
      const t = area.value.trim();
      if (!t) return undefined;
      if (field.kind === "points") return parsePointsValue(t);
      const v = JSON.parse(t);
      if (!Array.isArray(v)) throw new Error(`${field.name}: expected a JSON array`);
      return v;
    },
  };
}
async function openComputeTools() {
  const body = h("div", {}, h("p", { class: "muted" }, "Loading tool catalogue…"));
  floatWindow("Survey Computation Tools", body, { id: "computeWin", width: 660 });
  let catalogue;
  try { catalogue = await loadToolCatalogue(); }
  catch (e) { body.textContent = "Could not load the tool catalogue: " + e.message; return; }
  body.innerHTML = "";

  const list = h("div", { style: "width:190px;flex:0 0 190px;max-height:64vh;overflow:auto;border-right:1px solid #ddd;padding-right:6px" });
  const right = h("div", { style: "flex:1;min-width:0" });
  body.append(h("div", { class: "muted", style: "margin-bottom:5px" },
    `${catalogue.count} tools. Inputs use EPSG:32632 metres; output is JSON from the shared survey engine.`),
    h("div", { style: "display:flex;gap:8px;align-items:flex-start" }, list, right));

  const buttons = [];
  const groups = new Map();
  for (const tool of catalogue.tools) {
    if (!groups.has(tool.category)) groups.set(tool.category, []);
    groups.get(tool.category).push(tool);
  }
  for (const [category, tools] of groups) {
    list.append(h("div", { style: "font-weight:600;margin:7px 0 3px;color:#444" }, TOOL_CATEGORY_LABELS[category] || category));
    for (const tool of tools) {
      const b = h("button", { class: "btn", style: "display:block;width:100%;text-align:left;margin:1px 0;min-width:0", title: tool.description, onclick: () => select(tool) }, tool.name);
      buttons.push([b, tool.id]);
      list.append(b);
    }
  }

  function select(tool) {
    for (const [b, id] of buttons) b.classList.toggle("active", id === tool.id);
    const fields = tool.inputs.map(toolField);
    const result = h("pre", { style: "white-space:pre-wrap;max-height:300px;overflow:auto;margin:8px 0 0;background:#fafafa;border:1px solid #ddd;padding:5px" }, "Run the tool to see its result.");
    const run = h("button", {
      class: "btn",
      onclick: async () => {
        const input = {};
        try {
          for (const f of fields) { const v = f.value(); if (v !== undefined) input[f.name] = v; }
        } catch (e) { result.textContent = "Input error: " + e.message; return; }
        run.disabled = true; result.textContent = "Computing…";
        try {
          const r = await api(`/api/v1/survey/tools/${tool.id}`, { method: "POST", body: input });
          result.textContent = JSON.stringify(r.result ?? r, null, 2);
          status(`${tool.name} computed.`);
        } catch (e) { result.textContent = "Error: " + e.message; }
        finally { run.disabled = false; }
      },
    }, "Run");
    right.innerHTML = "";
    right.append(h("h4", { style: "margin:0 0 2px" }, tool.name),
      h("div", { class: "muted", style: "margin-bottom:6px" }, tool.description),
      ...fields.map((f) => f.el),
      h("div", { class: "muted", style: "font-size:11px;margin-top:6px" }, `Returns: ${tool.output}`),
      h("div", { class: "frow", style: "margin-top:6px" }, run), result);
  }
  select(catalogue.tools[0]);
}

const readTextFile = (input) => new Promise((resolve, reject) => {
  const file = input.files && input.files[0];
  if (!file) { resolve(null); return; }
  const reader = new FileReader();
  reader.onload = () => resolve({ name: file.name, text: String(reader.result) });
  reader.onerror = () => reject(reader.error || new Error(`could not read ${file.name}`));
  reader.readAsText(file);
});

function importSummary(result, filename) {
  if (result.kind === "raster") {
    const w = result.world, size = w ? `${w.a} × ${w.e}` : "—";
    return h("div", {},
      h("p", {}, h("b", {}, `${filename}: raster georeference`)),
      h("table", {},
        h("tr", {}, h("th", {}, "CRS"), h("td", {}, result.epsg ? `EPSG:${result.epsg}` : "unknown (no .prj)")),
        h("tr", {}, h("th", {}, "Pixel size"), h("td", {}, size)),
        h("tr", {}, h("th", {}, "Top-left centre"), h("td", {}, w ? `${fmt3(w.c)}  ${fmt3(w.f)}` : "—")),
        h("tr", {}, h("th", {}, "Rotation"), h("td", {}, w && (w.b || w.d) ? "rotated/sheared" : "north-up"))),
      h("p", { class: "muted" }, "Raster files are referenced through the Catalog; a world file only supplies the pixel-to-map transform."));
  }
  const box = h("div", {},
    h("p", {}, h("b", {}, `${filename}: ${result.format || "survey"}`)),
    h("p", {}, `${result.points.length} point(s), ${result.boundaries.length} boundary(ies), `
      + `CRS ${result.crs || (result.epsg ? "EPSG:" + result.epsg : "not stated")}.`));
  if (result.points.length) {
    const t = h("table", {}, h("tr", {}, h("th", {}, "Point"), h("th", {}, "Easting"), h("th", {}, "Northing"), h("th", {}, "Code")));
    for (const p of result.points.slice(0, 25)) t.append(h("tr", {},
      h("td", {}, p.id), h("td", {}, fmt3(p.point.x)), h("td", {}, fmt3(p.point.y)), h("td", {}, p.code || "")));
    box.append(t);
    if (result.points.length > 25) box.append(h("div", { class: "muted" }, `… ${result.points.length - 25} more`));
  }
  if (result.boundaries.length) {
    box.append(h("div", { style: "margin-top:6px;font-weight:600" }, "Boundaries"),
      h("ul", { style: "margin:2px 0 0 16px;padding:0" }, ...result.boundaries.slice(0, 25).map((b) =>
        h("li", {}, `${b.id} (${b.kind}) — ${b.ring.length} vertices, ${b.closed ? "closed" : "open"}`))));
  }
  for (const [label, list] of [["Warning", result.warnings || []], ["Row issue", result.issues || []]]) {
    if (list.length) box.append(h("div", { style: "margin-top:6px" }, h("b", {}, `${label}s`),
      h("ul", { style: "margin:2px 0 0 16px;padding:0" }, ...list.slice(0, 30).map((w) =>
        h("li", {}, typeof w === "string" ? w : (w.line ? `line ${w.line}: ${w.message}` : JSON.stringify(w)))))));
  }
  return box;
}

function openImportDialog() {
  const body = h("div");
  floatWindow("Import Survey / CAD / Attributes", body, { id: "importWin", width: 580 });

  const file = h("input", { type: "file", accept: ".csv,.tsv,.txt,.psv,.dxf,.xml,.landxml,.tfw,.tifw,.jgw,.jpgw,.jpegw,.pgw,.gfw,.bpw,.j2w,.wld" });
  const crs = h("input", { type: "text", value: "EPSG:32632", style: "width:120px" });
  const prj = h("input", { type: "file", accept: ".prj" });
  const join = h("input", { type: "checkbox" });
  const attrFile = h("input", { type: "file", accept: ".csv,.tsv,.txt,.psv", disabled: true });
  const keyColumn = h("input", { type: "text", placeholder: "auto (e.g. Point)", style: "width:130px", disabled: true });
  const out = h("div", { class: "muted" }, "Choose a file to preview. DXF must be saved as ASCII; drawing units and CRS are not reprojected.");
  const apply = h("button", { class: "btn", disabled: true }, "Import into job");
  let current = null;

  join.onchange = () => {
    attrFile.disabled = !join.checked; keyColumn.disabled = !join.checked;
  };

  const preview = h("button", {
    class: "btn",
    onclick: async () => {
      out.className = ""; out.textContent = "Reading…"; apply.disabled = true; current = null;
      try {
        const f = await readTextFile(file);
        if (!f) { out.className = "muted"; out.textContent = "Choose a file first."; return; }
        const prjFile = await readTextFile(prj);
        if (join.checked) {
          const a = await readTextFile(attrFile);
          if (!a) { out.textContent = "Choose the attribute table, or clear the join box."; return; }
          const result = await api("/api/v1/import/attributes", { method: "POST", body: {
            coordinates: f.text, attributes: a.text,
            key_column: keyColumn.value.trim() || null, crs: crs.value.trim() || null,
          } });
          current = { name: f.name, contents: f.text, crs: crs.value.trim() || null, attributes: a.text, key_column: keyColumn.value.trim() || null };
          out.innerHTML = ""; out.append(importSummary({ kind: "survey", ...result }, f.name));
        } else {
          const result = await api("/api/v1/import/preview", { method: "POST", body: {
            filename: f.name, contents: f.text, crs: crs.value.trim() || null, prj: prjFile ? prjFile.text : null,
          } });
          current = { name: f.name, contents: f.text, crs: crs.value.trim() || null, attributes: null, key_column: null };
          out.innerHTML = ""; out.append(importSummary(result, f.name));
          if (result.kind === "raster") current = null;
        }
        apply.disabled = !(current && state.job);
        if (current && !state.job) out.append(h("p", { class: "muted" }, "Pick a Job Area to import these features as drafts."));
      } catch (e) { out.textContent = "Import preview failed: " + e.message; }
    },
  }, "Preview");

  apply.onclick = async () => {
    if (!current || !state.job) return;
    apply.disabled = true; out.textContent = "Importing into " + state.job.reference + "…";
    try {
      const r = await api(`/api/v1/jobs/${state.job.id}/import`, { method: "POST", body: {
        filename: current.name, contents: current.contents, crs: current.crs,
        attributes: current.attributes, key_column: current.key_column,
      } });
      await loadDrafts(); buildCatalog();
      if (state.drafts.features.length) zoomToFC(state.drafts);
      out.className = "";
      out.innerHTML = "";
      out.append(h("p", {}, h("b", {}, `Imported into ${state.job.reference}`)),
        h("p", {}, `${r.created_parcels} draft parcel(s), ${r.created_beacons} beacon(s).`));
      // Under a workflow job the file's point IDs are kept for the Beacon Numbers window at commit
      // (Point Creation Report), not made into beacons now.
      if (r.workflow_job && r.points_for_commit?.length) {
        for (const q of r.points_for_commit) rememberPointId(q.easting, q.northing, q.id);
        const t = h("table", { class: "legs" }, h("tr", {}, h("th", {}, "Point"), h("th", {}, "Easting"), h("th", {}, "Northing")));
        for (const q of r.points_for_commit.slice(0, 200)) t.append(h("tr", {}, h("td", {}, q.id), h("td", {}, fmt3(q.easting)), h("td", {}, fmt3(q.northing))));
        out.append(h("p", { style: "margin:6px 0 2px" }, h("b", {}, "Point Creation Report"), ` — ${r.points_for_commit.length} point(s); their numbers fill the Beacon Numbers window at commit.`), h("div", { style: "max-height:180px;overflow:auto" }, t));
      }
      if (r.skipped_boundaries?.length) out.append(h("div", { class: "muted" }, `${r.skipped_boundaries.length} boundary(ies) skipped`));
      if (r.skipped_beacons?.length) out.append(h("div", { class: "muted" }, `${r.skipped_beacons.length} beacon(s) skipped (already present or unnamed)`));
      for (const w of r.warnings || []) out.append(h("div", { class: "muted" }, w));
      status(`Imported ${r.created_parcels} parcel(s) and ${r.created_beacons} beacon(s) into ${state.job.reference}.`);
    } catch (e) { out.textContent = "Import failed: " + e.message; }
    finally { apply.disabled = !(current && state.job); }
  };

  body.append(
    h("p", { class: "muted", style: "margin-top:0" }, "Coordinate lists (CSV/TSV), ASCII DXF, LandXML and raster world files are detected from the file name and contents."),
    h("div", { class: "frow" }, h("label", {}, "File:"), file),
    h("div", { class: "frow" }, h("label", {}, "CRS:"), crs, h("span", { class: "muted" }, "assumed only if the file does not state one")),
    h("div", { class: "frow" }, h("label", {}, "World .prj:"), prj, h("span", { class: "muted" }, "optional, for rasters")),
    h("div", { class: "frow" }, h("label", {}, "Join table:"), join, attrFile),
    h("div", { class: "frow" }, h("label", {}, "Key column:"), keyColumn),
    h("div", { class: "frow", style: "margin-top:8px" }, preview, apply),
    h("div", { style: "margin-top:8px;background:#fafafa;border:1px solid #ddd;padding:5px;max-height:48vh;overflow:auto" }, out));
}

async function attributeTable(l) {
  const fc = { parcels: state.parcels, jobsArea: state.drafts, ctrl: state.beacons }[l.id] || empty();
  const fs = fc.features.slice(0, 500);
  const cols = fs.length ? Object.keys(fs[0].properties).filter((k) => k !== "id") : [];
  const t = h("table", {}, h("tr", {}, ...cols.map((c) => h("th", {}, c))));
  for (const f of fs) {
    const tr = h("tr", { style: "cursor:pointer" }, ...cols.map((c) => h("td", {}, f.properties[c] == null ? "<Null>" : String(f.properties[c]))));
    tr.onclick = () => { zoomToFC({ type: "FeatureCollection", features: [f] }); if (l.id === "parcels") { focusParcel(f); selectFeatures([f.properties.id]); } };
    t.append(tr);
  }
  floatWindow(`Table: ${l.name}`, h("div", {}, h("div", { class: "muted" }, `${fc.features.length} record(s)${fc.features.length > 500 ? " (first 500 shown)" : ""}`), t), { id: "tableWin", width: 520 });
}
function layerProperties(l) {
  floatWindow(`Layer Properties: ${l.name}`, h("table", {},
    ...[["Name", l.name], ["Type", l.group ? "Group layer" : l.kind], ["Data source", l.source || "—"], ["Visible", effectiveVisible(l) ? "Yes" : "No"],
      ["Spatial reference", l.kind === "raster" ? "WGS 1984 Web Mercator (tiles)" : "WGS 1984 UTM Zone 32N"]]
      .map(([k, v]) => h("tr", {}, h("th", {}, k), h("td", {}, v)))), { width: 360 });
}

/* =====================================================================================
 * Phase 4 — Cadastre toolbar (Region / Job Area) and Tasks wizard
 * ===================================================================================== */
async function loadJobs() {
  try {
    [state.workflow, state.jobs] = await Promise.all([api("/api/v1/workflow"), api("/api/v1/jobs")]);
  } catch (e) { status("Could not load jobs: " + e.message); }
  renderJobBox();
}
function renderJobBox() {
  const sel = $("jobBox"); if (!sel) return;
  const cur = state.job?.id || "";
  sel.innerHTML = ""; sel.append(h("option", { value: "" }, ""));
  for (const j of state.jobs) sel.append(h("option", { value: j.id, selected: j.id === cur }, `${j.reference} (${j.status})`));
  updateJobButtons();
}
async function selectJob(id) {
  state.job = state.jobs.find((j) => j.id === id) || null;
  await loadDrafts();
  if (state.job && state.drafts.features.length) zoomToFC(state.drafts);
  updateJobButtons(); buildTOC(); buildCatalog(); renderTask();
  status(state.job ? `Job ${state.job.reference}: ${state.job.status}` : "");
}
const canDo = (action) => !!state.job && state.workflow.transitions.some((t) => t.from_stage === state.job.status && t.action === action);
function updateJobButtons() {
  const set = (id, on) => $(id)?.classList.toggle("disabled", !on);
  set("jbSurvey", !!state.job); set("jbSubmit", canDo("submit")); set("jbReview", canDo("start_review"));
  set("jbApprove", canDo("approve")); set("jbPost", state.job?.status === POSTABLE_STAGE);
  set("jbPlan", !!state.job); set("jbReport", !!state.job); set("jbValidate", !!state.job || state.selection.size > 0);
}
async function doTransition(action) {
  if (!canDo(action)) return;
  try {
    const j = await api(`/api/v1/jobs/${state.job.id}/transition`, { method: "POST", body: { action } });
    Object.assign(state.job, j); const i = state.jobs.findIndex((x) => x.id === j.id); if (i >= 0) state.jobs[i] = j;
    renderJobBox(); renderTask(); buildCatalog(); status(`Job ${j.reference} is now ${j.status}`);
  } catch (e) { alert(`${action} failed: ${e.message}`); }
}
async function postJob() {
  if (!state.job) return;
  if (state.job.status !== POSTABLE_STAGE) { alert(`Only ${POSTABLE_STAGE} jobs can be posted (this job is ${state.job.status}).`); return; }
  try {
    const res = await api(`/api/v1/jobs/${state.job.id}/post`, { method: "POST" });
    status(`Posted ${res.posted} parcel(s): ${res.parcel_numbers.join(", ")}`);
    await Promise.all([loadDrafts(), loadParcels(false)]); renderTask();
    floatWindow("Post Job", h("div", {}, h("p", {}, `Posted ${res.posted} parcel(s).`), h("div", {}, res.parcel_numbers.join(", ") || "—")), { width: 300 });
  } catch (e) { alert("Post failed: " + e.message); }
}
function openPdf(kind) { if (state.job) window.open(`/api/v1/jobs/${state.job.id}/${kind === "siteplan" ? "siteplan" : "report"}.pdf`, "_blank"); }
function newJobDialog() {
  const ref = h("input", { value: `JOB-${new Date().toISOString().slice(0, 10)}-${Math.floor(Math.random() * 900 + 100)}` });
  const type = h("select", {}, ...["capture", "subdivision", "consolidation", "resurvey"].map((t) => h("option", {}, t)));
  const title = h("input", { placeholder: "e.g. Plot 5, Rayfield" });
  const w = floatWindow("New Job", h("div", {},
    h("div", { class: "frow" }, h("label", {}, "Reference:"), ref), h("div", { class: "frow" }, h("label", {}, "Type:"), type),
    h("div", { class: "frow" }, h("label", {}, "Title:"), title),
    h("div", { class: "frow", style: "justify-content:flex-end" }, h("button", { class: "btn", onclick: async () => {
      try {
        const j = await api("/api/v1/jobs", { method: "POST", body: { reference: ref.value.trim(), job_type: type.value, title: title.value.trim() || null } });
        state.jobs.unshift(j); renderJobBox(); $("jobBox").value = j.id; await selectJob(j.id); w.remove();
      } catch (e) { alert("Create job failed: " + e.message); }
    } }, "OK"), h("button", { class: "btn", onclick: () => w.remove() }, "Cancel"))), { id: "newJobWin", width: 330 });
}
async function validateFabric() {
  const fs = (state.selection.size ? selectedFC() : state.drafts).features;
  if (!fs.length) { alert("Select parcels, or choose a Job Area with draft parcels, to validate."); return; }
  const rings = fs.map((f) => {
    const g = f.geometry; const ring = (g.type === "MultiPolygon" ? g.coordinates[0][0] : g.coordinates[0]);
    return ring.slice(0, -1).map(([x, y]) => { const [E, N] = toUTM(x, y); return { x: E, y: N }; });
  });
  try {
    const r = await api("/api/v1/fabric/validate", { method: "POST", body: { parcels: rings } });
    floatWindow("Validate Fabric", h("pre", { style: "white-space:pre-wrap;margin:0" }, JSON.stringify(r, null, 2)), { width: 420 });
  } catch (e) { alert("Validation failed: " + e.message); }
}

// ---- Tasks wizard ----
const TASKS = {
  survey: { name: "New parcel from survey (COGO)", steps: ["Job", "Start point", "Traverse", "Review & save"] },
  workflow: { name: "Job workflow", steps: ["Job status", "History"] },
};
const task = { kind: "survey", step: 0, legs: [{ b: "", d: "" }], start: null, stated: "", check: null };
const taskSel = () => task.kind;
function openTask(kind) {
  if ($("tocPane").classList.contains("hidden")) togglePane("tocPane");
  task.kind = kind; task.step = 0; showTasks(true); buildTocTools(); renderTask();
  if (kind === "survey") setTool("survey");
}
function renderTask() {
  const body = $("tasksBody"); body.innerHTML = "";
  const local = h("select", { onchange: (e) => openTask(e.target.value) }, ...Object.entries(TASKS).map(([k, v]) => h("option", { value: k, selected: k === task.kind }, v.name)));
  body.append(h("div", { class: "tasksHead" }, h("div", {}, "Tasks"), h("div", { class: "frow" }, h("label", {}, "Local:"), local)));
  if (task.kind === "survey") { renderSurveyForm(body); return; }
  // Workflow task keeps a short stepped page.
  const t = TASKS.workflow;
  body.append(h("div", { class: "taskSteps" }, h("span", { html: icon("tasks") }),
    h("div", { class: "taskStepBar", title: `Step ${task.step + 1} of ${t.steps.length}` }, h("span", { style: `width:${((task.step + 1) / t.steps.length) * 100}%` }))));
  const page = h("div", { class: "taskPage" }, h("h4", {}, `${task.step + 1}. ${t.steps[task.step]}`));
  workflowPage(page); body.append(page);
  const last = task.step === t.steps.length - 1;
  body.append(h("div", { class: "taskNav" },
    h("button", { class: "btn", disabled: task.step === 0, onclick: () => { task.step--; renderTask(); } }, "Back"),
    h("button", { class: "btn", disabled: last, onclick: () => { task.step++; renderTask(); } }, "Next"),
    h("button", { class: "btn", onclick: cancelTask }, "Close")));
}

// The survey task is one continuous form — no Back/Next. Pick a job (inline), drop a start, enter
// legs with a live closure check, Save. After saving it stays armed for the next parcel.
const subHead = (t) => h("div", { style: "font-weight:600;margin:9px 0 3px;color:#2a2a2a" }, t);
// Type-to-search beacon box: searches every control point and survey beacon on the server (not
// just those loaded on the map), nearest the map centre first, sorted by number otherwise.
function beaconPicker(onPick) {
  const id = "beaconList" + Math.random().toString(36).slice(2, 8);
  const input = h("input", { list: id, placeholder: "— beacon or control point —", style: "flex:1;min-width:0", autocomplete: "off" });
  const list = h("datalist", { id });
  let found = [], timer = null, seq = 0;
  const label = (b) => `${b.number}${b.is_control ? " (control)" : ""}`;
  const search = async () => {
    const my = ++seq, c = map.getCenter(), [e, n] = toUTM(c.lng, c.lat);
    const q = input.value.replace(/ \(control\)$/, "").trim();
    try { found = await api(`/api/v1/beacons/search?limit=100&q=${encodeURIComponent(q)}&e=${e.toFixed(1)}&n=${n.toFixed(1)}`); } catch { found = []; }
    if (my !== seq) return;
    list.replaceChildren(...found.map((b) => h("option", { value: label(b) }, `${fmt3(b.easting)}  ${fmt3(b.northing)}`)));
  };
  const pick = () => { const b = found.find((x) => label(x) === input.value || x.number === input.value.trim()); if (b) onPick(b); };
  input.addEventListener("input", () => { clearTimeout(timer); timer = setTimeout(search, 200); });
  input.addEventListener("focus", search);
  input.addEventListener("change", pick);
  input.addEventListener("keydown", (ev) => { if (ev.key === "Enter") pick(); });
  return h("span", { style: "display:flex;flex:1;min-width:0" }, input, list);
}
function renderSurveyForm(body) {
  const page = h("div", { class: "taskPage" });

  const jobSel = h("select", { style: "flex:1;min-width:0", onchange: async (e) => { $("jobBox").value = e.target.value; await selectJob(e.target.value); } },
    h("option", { value: "" }, "— choose a job —"),
    ...state.jobs.map((j) => h("option", { value: j.id, selected: state.job?.id === j.id }, `${j.reference} (${j.status})`)));
  page.append(subHead("Job"), h("div", { class: "frow" }, h("label", {}, "Job Area:"), jobSel, h("button", { class: "btn", style: "min-width:0", onclick: newJobDialog }, "New…")),
    state.job ? null : h("p", { class: "muted" }, "Pick or create a job to save under (you can set the start and legs first)."));

  const E = h("input", { type: "number", step: "0.001", value: task.start ? task.start[0].toFixed(3) : "" });
  const N = h("input", { type: "number", step: "0.001", value: task.start ? task.start[1].toFixed(3) : "" });
  const upd = () => { const e = parseFloat(E.value), n = parseFloat(N.value); task.start = isFinite(e) && isFinite(n) ? [e, n] : null; drawTraverse(); scheduleCheck(); };
  E.oninput = upd; N.oninput = upd;
  const beacon = beaconPicker((b) => { task.start = [b.easting, b.northing]; drawTraverse(); renderTask(); });
  page.append(subHead("Start point"), h("p", { class: "muted" }, "Click the map (crosshair), pick a control point, or type UTM 32N coordinates."),
    h("div", { class: "frow" }, h("label", {}, "Easting:"), E), h("div", { class: "frow" }, h("label", {}, "Northing:"), N),
    h("div", { class: "frow" }, h("label", {}, "Beacon:"), beacon),
    h("p", { class: "muted", style: "margin:2px 0 0" }, "Type part of a beacon or control point number; the nearest matches are listed first."));

  const tbl = h("table", { class: "legs" }, h("tr", {}, h("th", {}, "#"), h("th", {}, "Bearing"), h("th", {}, "Dist (m)"), h("th", {}, "")));
  task.legs.forEach((l, i) => {
    const b = h("input", { value: l.b, placeholder: "ddd mm ss", title: "Decimal degrees, or d m s" });
    const d = h("input", { value: l.d, type: "number", step: "0.001" });
    b.oninput = () => { l.b = b.value; drawTraverse(); scheduleCheck(); };
    d.oninput = () => { l.d = d.value; drawTraverse(); scheduleCheck(); };
    // Enter in the last distance box adds the next leg and jumps to its bearing — fast entry.
    d.onkeydown = (ev) => { if (ev.key === "Enter" && i === task.legs.length - 1 && l.b !== "" && l.d !== "") { task.legs.push({ b: "", d: "" }); renderTask(); focusLastBearing(); } };
    tbl.append(h("tr", {}, h("td", { class: "n" }, String(i + 1)), h("td", {}, b), h("td", {}, d),
      h("td", {}, h("button", { title: "Remove leg", onclick: () => { task.legs.splice(i, 1); if (!task.legs.length) task.legs.push({ b: "", d: "" }); renderTask(); drawTraverse(); scheduleCheck(); } }, "✕"))));
  });
  page.append(subHead("Traverse (bearing + distance)"), tbl,
    h("button", { class: "btn", onclick: () => { task.legs.push({ b: "", d: "" }); renderTask(); focusLastBearing(); } }, "Add leg"),
    h("div", { class: "checkout", id: "checkOut" }, "Enter a start point and at least three legs to check closure."));

  const stated = h("input", { type: "number", step: "0.01", value: task.stated, placeholder: "m² (optional)" });
  stated.oninput = () => { task.stated = stated.value; scheduleCheck(); };
  const save = h("button", { class: "btn", id: "surveySave", onclick: finishTask }, "Save draft parcel");
  page.append(subHead("Save"), h("div", { class: "frow" }, h("label", {}, "Stated area:"), stated),
    h("div", { class: "taskNav" }, save, h("button", { class: "btn", onclick: cancelTask }, "Clear")));

  body.append(page);
  scheduleCheck(true); updateTaskButtons();
}
function focusLastBearing() { const rows = $("tasksBody").querySelectorAll("table.legs tr"); rows[rows.length - 1]?.querySelectorAll("input")[0]?.focus(); }
function updateTaskButtons() {
  if (task.kind !== "survey") return;
  const save = $("surveySave"); if (save) save.disabled = !(state.job && task.start && surveyLegs().length >= 3);
}
function resetTraverse() { task.step = 0; task.legs = [{ b: "", d: "" }]; task.start = null; task.stated = ""; task.check = null; map.getSource("survey")?.setData(empty()); }
function cancelTask() { resetTraverse(); setTool("pan"); renderTask(); }
async function finishTask() {
  if (task.kind !== "survey") return;
  const utm = traverseUTM(); if (utm.length < 4 || !state.job) return;
  const stated = parseFloat(task.stated);
  try {
    await api(`/api/v1/jobs/${state.job.id}/parcels`, { method: "POST", body: { geometry: { type: "Polygon", coordinates: [[...utm, utm[0]]] }, stated_area: isFinite(stated) ? stated : null } });
    status(`Saved draft parcel to ${state.job.reference} — ready for the next. ${draftNextStep()}`); wfRefreshEditing();
    await loadDrafts(); buildCatalog();
    resetTraverse(); renderTask(); // stay in survey mode for the next parcel
  } catch (e) { alert("Save failed: " + e.message); }
}
function jobPicker() {
  const sel = h("select", { style: "flex:1;min-width:0", onchange: async (e) => { $("jobBox").value = e.target.value; await selectJob(e.target.value); } },
    h("option", { value: "" }, "— choose a job —"), ...state.jobs.map((j) => h("option", { value: j.id, selected: state.job?.id === j.id }, `${j.reference} (${j.status})`)));
  return h("div", {}, h("div", { class: "frow" }, h("label", {}, "Job Area:"), sel),
    h("div", { class: "frow" }, h("button", { class: "btn", onclick: newJobDialog }, "New job…")));
}
function workflowPage(page) {
  if (task.step === 0) {
    page.append(jobPicker());
    if (state.job) {
      const acts = state.workflow.transitions.filter((t) => t.from_stage === state.job.status);
      page.append(h("table", {}, ...[["Reference", state.job.reference], ["Type", state.job.job_type], ["Title", state.job.title || "—"], ["Stage", state.job.status], ["Draft parcels", String(state.drafts.features.length)]]
        .map(([k, v]) => h("tr", {}, h("th", {}, k), h("td", {}, v)))),
        h("div", { class: "frow", style: "flex-wrap:wrap;margin-top:6px" }, ...acts.map((t) => h("button", { class: "btn", onclick: () => doTransition(t.action) }, t.action.replace("_", " "))),
          state.job.status === POSTABLE_STAGE ? h("button", { class: "btn", onclick: postJob }, "Post job") : null));
    }
  } else {
    const box = h("div", {}, "Loading…"); page.append(box);
    if (state.job) api(`/api/v1/jobs/${state.job.id}/events`).then((ev) => {
      box.innerHTML = ""; box.append(h("table", {}, h("tr", {}, h("th", {}, "When"), h("th", {}, "Action"), h("th", {}, "By")),
        ...ev.map((e) => h("tr", {}, h("td", {}, new Date(e.created_at || e.at || Date.now()).toLocaleString()), h("td", {}, `${e.action}: ${e.from_stage || ""} → ${e.to_stage}`), h("td", {}, e.actor)))));
    }).catch((e) => (box.textContent = e.message));
  }
}
// bearings: decimal degrees, or "d m s" / d°m's"
function parseBearing(s) {
  s = String(s).trim(); if (!s) return NaN;
  const parts = s.split(/[^\d.]+/).filter(Boolean).map(Number);
  if (parts.length === 1) return parts[0];
  return parts[0] + (parts[1] || 0) / 60 + (parts[2] || 0) / 3600;
}
function surveyLegs() { return task.legs.map((l) => ({ bearing: parseBearing(l.b), distance: parseFloat(l.d) })).filter((l) => isFinite(l.bearing) && isFinite(l.distance) && l.distance > 0); }
function traverseUTM() {
  if (!task.start) return [];
  let [E, N] = task.start; const pts = [[E, N]];
  for (const l of surveyLegs()) { const r = l.bearing * Math.PI / 180; E += l.distance * Math.sin(r); N += l.distance * Math.cos(r); pts.push([E, N]); }
  return pts;
}
function drawTraverse() {
  const ll = traverseUTM().map(([e, n]) => toLL(e, n));
  const feats = ll.map((c) => ({ type: "Feature", geometry: { type: "Point", coordinates: c } }));
  if (ll.length > 1) feats.push({ type: "Feature", geometry: { type: "LineString", coordinates: ll } });
  state.surveyFC = { type: "FeatureCollection", features: feats };
  map.getSource("survey")?.setData(state.surveyFC);
}
function setSurveyStart(ll) {
  task.start = toUTM(ll.lng, ll.lat);
  drawTraverse(); renderTask(); scheduleCheck();
}
let checkTimer = null;
function scheduleCheck(now) { clearTimeout(checkTimer); checkTimer = setTimeout(runCheck, now ? 0 : 200); updateTaskButtons(); }
async function runCheck() {
  const out = $("checkOut"); if (!out) return;
  const legs = surveyLegs();
  if (!task.start || legs.length < 3) { out.textContent = "Enter a start point and at least three legs to check closure."; return; }
  const stated = parseFloat(task.stated);
  try {
    const s = await api("/api/v1/survey/check", { method: "POST", body: { start: { x: task.start[0], y: task.start[1] },
      legs: legs.map((l) => ({ type: "line", bearing: l.bearing, distance: l.distance })), stated_area: isFinite(stated) ? stated : null } });
    const flag = (ok, label) => ok == null ? `<span class="muted">${label}: n/a</span>` : `<span class="flag-${ok ? "ok" : "bad"}">${ok ? "✓" : "✗"} ${label}</span>`;
    const prec = s.closure && s.closure.precision_ratio ? "1:" + Math.round(s.closure.precision_ratio) : "exact";
    out.innerHTML = `Legs: ${s.leg_count}   Perimeter: ${s.perimeter.toFixed(3)} m\nMisclosure: ${s.closure ? s.closure.linear_misclosure.toFixed(3) : "—"} m   Precision: ${prec}\n` +
      `Area: ${s.area != null ? s.area.toFixed(2) + " m²" : "—"}\n${flag(s.checks.closes_on_start, "closes")}  ${flag(s.checks.meets_precision, "precision")}  ${flag(s.checks.area_within_tolerance, "area")}`;
  } catch (e) { out.textContent = "Check failed: " + e.message; }
}

/* =====================================================================================
 * Phase 4b — Editor: sketch digitizing (ArcMap Editor toolbar + Create Features /
 * Construction Tools / Edit Sketch Properties). Click vertices on the imagery to draw a
 * parcel polygon (or a control point / boundary line); finished parcels post as job drafts
 * via the same endpoint as COGO survey entry. Vertices are kept in EPSG:32632 metres.
 * ===================================================================================== */
function editorMenu() {
  return [
    { label: "Start Editing", action: () => startEditing(), disabled: () => edit.active },
    { label: "Stop Editing", action: stopEditing, disabled: () => !edit.active },
    { label: "Save Edits", action: saveEdits, disabled: () => !edit.active },
    "-",
    { label: "Create Features Window", icon: "createFeatures", action: openCreateFeatures },
    { label: "Edit Sketch Properties…", icon: "sketchProps", action: openSketchProps },
    "-",
    { label: "Snapping", checked: () => edit.snapping, action: toggleSnapping },
    "-",
    { label: "Finish Sketch", kbd: "F2", action: finishSketch, disabled: () => !canFinishSketch() },
    { label: "Delete Sketch", action: deleteSketch, disabled: () => !edit.verts.length },
  ];
}
function startEditing(tool = "sketch") {
  edit.active = true;
  selectTemplate(edit.template);
  openCreateFeatures();
  setTool(tool);
  openSketchProps();
  updateEditorButtons();
  status("Editing started — pick a template in Create Features, then click the map to add vertices (double-click to finish).");
}
function stopEditing() {
  if (edit.verts.length && !confirm("You have an unfinished sketch. Discard it and stop editing?")) return;
  edit.active = false; clearSketch();
  $("createFeaturesWin")?.remove(); $("sketchPropsWin")?.remove();
  setTool("pan"); updateEditorButtons(); status("Editing stopped.");
}
function saveEdits() { status("Edits are saved as each feature is finished — drafts are written to the server immediately."); }

function selectTemplate(id) {
  const t = FEATURE_TEMPLATES.find((x) => x.id === id) || FEATURE_TEMPLATES[0];
  edit.template = t.id; edit.geomType = t.geom;
  if (!t.tools.includes(edit.construction)) edit.construction = t.tools[0];
  drawSketch();
}

// --- sketching ---
function sketchClick(e) {
  const pt = e.point;
  // A double-click also delivers two clicks at the same spot; don't add a duplicate vertex.
  if (edit.verts.length) { const last = map.project(toLL(...edit.verts[edit.verts.length - 1])); if (Math.hypot(last.x - pt.x, last.y - pt.y) < 4) return; }
  let [E, N] = toUTM(e.lngLat.lng, e.lngLat.lat);
  if (edit.snapping) { const s = snapUTM(pt); if (s) [E, N] = s; }
  if (edit.geomType === "point") { edit.verts = [[E, N]]; edit.sel = 0; drawSketch(); finishSketch(); return; }
  edit.verts.push([E, N]); edit.sel = edit.verts.length - 1;
  drawSketch(); refreshSketchProps(); updateEditorButtons();
}
// Snap to the nearest beacon / parcel / draft vertex within ~14px of the cursor.
function snapUTM(pt) {
  const layers = ["beacons-pt", "drafts-line", "parcels-line"].filter((l) => map.getLayer(l));
  if (!layers.length) return null;
  const feats = map.queryRenderedFeatures([[pt.x - 14, pt.y - 14], [pt.x + 14, pt.y + 14]], { layers });
  let best = null, bestD = 14;
  const consider = (ll) => { const p = map.project(ll); const d = Math.hypot(p.x - pt.x, p.y - pt.y); if (d < bestD) { bestD = d; best = ll; } };
  for (const f of feats) {
    const g = f.geometry; if (!g) continue;
    if (g.type === "Point") consider(g.coordinates);
    else if (g.type === "LineString") g.coordinates.forEach(consider);
    else if (g.type === "MultiLineString") g.coordinates.flat().forEach(consider);
    else if (g.type === "Polygon") g.coordinates.flat().forEach(consider);
    else if (g.type === "MultiPolygon") g.coordinates.flat(2).forEach(consider);
  }
  return best ? toUTM(best[0], best[1]) : null;
}
function drawSketch() {
  const ll = edit.verts.map(([e, n]) => toLL(e, n));
  const feats = [];
  if (edit.geomType === "polygon" && ll.length >= 3) feats.push({ type: "Feature", geometry: { type: "Polygon", coordinates: [[...ll, ll[0]]] } });
  if (edit.geomType !== "point" && ll.length >= 2) feats.push({ type: "Feature", geometry: { type: "LineString", coordinates: ll } });
  ll.forEach((c, i) => feats.push({ type: "Feature", properties: { idx: i, sel: i === edit.sel || (edit.sel < 0 && i === ll.length - 1) }, geometry: { type: "Point", coordinates: c } }));
  map.getSource("sketch")?.setData({ type: "FeatureCollection", features: feats });
}
function clearSketch() { edit.verts = []; edit.sel = -1; edit.editingId = null; edit.splitMode = false; map.getSource("sketch")?.setData(empty()); }
function canFinishSketch() { const n = edit.verts.length; return edit.splitMode ? n >= 2 : edit.geomType === "polygon" ? n >= 3 : edit.geomType === "line" ? n >= 2 : n >= 1; }
async function finishSketch() {
  if (!canFinishSketch()) { status(`Add ${edit.geomType === "polygon" ? "at least 3" : edit.geomType === "line" ? "at least 2" : "a"} vertex(es) before finishing.`); return; }
  if (edit.splitMode) { await mutationSplit(); return; }
  if (edit.geomType === "polygon" && edit.template === "jobArea") await saveSketchJobArea();
  else if (edit.geomType === "polygon") await saveSketchParcel();
  else if (edit.geomType === "point") await saveSketchPoint();
  else saveSketchLine();
}
async function saveSketchJobArea() {
  if (!wf.editing) { alert("Open a job in the Workflow tab and choose Work On This Job before drawing its job area."); return; }
  const utm = edit.verts.slice();
  try {
    wf.editing = await api(`/api/v1/wf/jobs/${wf.editing.job_instance_id}/area`, { method: "PUT", body: { geometry: { type: "Polygon", coordinates: [[...utm, utm[0]]] } } });
    showJobArea(); clearSketch(); refreshSketchProps(); updateEditorButtons(); zoomToUTMRing(utm);
    selectTemplate("parcel"); notify(`Job area saved (${Math.round(wf.editing.area_m2 || 0).toLocaleString()} m²). Parcels for this job must be drawn inside it.`);
    renderWorkflow();
  } catch (e) { alert("Saving the job area failed: " + e.message); }
}
async function saveSketchParcel() {
  if (!state.job) { alert("Choose a Job Area first — a digitized parcel is saved as a draft under a job."); return; }
  const utm = edit.verts.slice();
  try {
    const geometry = { type: "Polygon", coordinates: [[...utm, utm[0]]] };
    if (edit.editingId) {
      await api(`/api/v1/jobs/${state.job.id}/parcels/${edit.editingId}`, { method: "PUT", body: { geometry, stated_area: null } });
      notify(`Draft parcel updated in ${state.job.reference}. ${draftNextStep()}`);
    } else {
      await api(`/api/v1/jobs/${state.job.id}/parcels`, { method: "POST", body: { geometry, stated_area: null } });
      if (state.job.reference.startsWith("WF-")) notify(`Parcel saved as a draft to ${state.job.reference}. ${draftNextStep()}`);
      else notify(`Parcel saved to ${state.job.reference}, which is not a workflow job. To work as in Aumentum, open the job in the Workflow panel and choose Work On This Job.`, "warn");
    }
    zoomToUTMRing(utm);
    wfRefreshEditing();
    await loadDrafts(); buildCatalog(); clearSketch(); refreshSketchProps(); updateEditorButtons();
  } catch (e) { alert("Save failed: " + e.message); }
}
async function saveSketchPoint() {
  const [E, N] = edit.verts[0];
  const number = prompt("Control point number:", "");
  if (number == null || !number.trim()) return;
  try {
    await api("/api/v1/beacons", { method: "POST", body: { number: number.trim(), easting: E, northing: N, is_control: true } });
    status(`Control point ${number.trim()} created at ${E.toFixed(3)} ${N.toFixed(3)}.`);
    await loadBeacons(); clearSketch(); refreshSketchProps(); updateEditorButtons();
  } catch (e) { alert("Create point failed: " + e.message); }
}
function saveSketchLine() {
  const ll = edit.verts.map(([e, n]) => toLL(e, n));
  addGraphic({ type: "line", pts: ll });
  status(`Boundary line added to the drawing (${edit.verts.length} vertices).`);
  clearSketch(); refreshSketchProps(); updateEditorButtons();
}
function deleteSketch() { clearSketch(); refreshSketchProps(); updateEditorButtons(); status("Sketch deleted."); }
function toggleSnapping() { edit.snapping = !edit.snapping; $("edSnap")?.classList.toggle("active", edit.snapping); status(`Snapping ${edit.snapping ? "on" : "off"}.`); }
function updateEditorButtons() {
  const set = (id, on) => $(id)?.classList.toggle("disabled", !on);
  const has = edit.verts.length > 0;
  set("edFinish", canFinishSketch()); set("edDelete", has); set("edProps", true);
  $("edSnap")?.classList.toggle("active", edit.snapping);
}

// Dragging an existing sketch vertex (Edit tool, or the Sketch tool if you grab a vertex).
let vtxDrag = null;
canvas.addEventListener("mousedown", (ev) => {
  if (ev.button !== 0 || !(state.tool === "sketch" || state.tool === "editVertex") || !edit.verts.length) return;
  const r = canvas.getBoundingClientRect(); const px = ev.clientX - r.left, py = ev.clientY - r.top;
  let hit = -1, hd = 9;
  edit.verts.forEach(([e, n], i) => { const p = map.project(toLL(e, n)); const d = Math.hypot(p.x - px, p.y - py); if (d < hd) { hd = d; hit = i; } });
  if (hit >= 0) { ev.preventDefault(); ev.stopPropagation(); vtxDrag = hit; edit.sel = hit; drawSketch(); }
});
window.addEventListener("mousemove", (ev) => {
  if (vtxDrag == null) return;
  const r = canvas.getBoundingClientRect(); const pt = { x: ev.clientX - r.left, y: ev.clientY - r.top };
  const ll = map.unproject([pt.x, pt.y]); let [E, N] = toUTM(ll.lng, ll.lat);
  if (edit.snapping) { const s = snapUTM(pt); if (s) [E, N] = s; }
  edit.verts[vtxDrag] = [E, N]; drawSketch(); refreshSketchProps();
});
window.addEventListener("mouseup", () => { if (vtxDrag != null) { vtxDrag = null; suppressClickUntil = Date.now() + 250; updateEditorButtons(); } });

// --- Create Features window (templates + construction tools) ---
function openCreateFeatures() {
  const search = h("input", { class: "tinput", placeholder: "Search", style: "width:100%;margin:3px 0" });
  const tlist = h("div", { class: "cfList" });
  const ctools = h("div", { class: "cfTools" });
  const jobLine = h("p", { class: "muted", style: "margin:6px 0 0" });
  const renderTools = () => {
    ctools.innerHTML = ""; const tmpl = FEATURE_TEMPLATES.find((t) => t.id === edit.template);
    ctools.append(h("div", { class: "cfHdr" }, "Construction Tools"));
    for (const ct of tmpl.tools) {
      ctools.append(h("div", { class: `cfRow${edit.construction === ct ? " sel" : ""}`, onclick: () => { edit.construction = ct; renderTools(); } },
        h("span", { html: icon(CTOOL_ICON[ct]) }), CTOOL_NAME[ct]));
    }
  };
  const renderTmpl = () => {
    tlist.innerHTML = ""; const q = search.value.trim().toLowerCase();
    const shown = FEATURE_TEMPLATES.filter((t) => (!t.wfOnly || wf.editing) && (!q || t.name.toLowerCase().includes(q)));
    if (!shown.length) tlist.append(h("p", { class: "muted", style: "padding:4px" }, "There are no templates to show."));
    for (const t of shown) {
      tlist.append(h("div", { class: `cfRow${edit.template === t.id ? " sel" : ""}`, onclick: () => { selectTemplate(t.id); renderTmpl(); renderTools(); if (edit.active) setTool("sketch"); } },
        h("span", { html: icon(t.icon) }), t.name));
    }
    jobLine.textContent = state.job ? `Parcels save to job ${state.job.reference}.` : "Pick a Job Area to save parcels into.";
  };
  search.oninput = renderTmpl; renderTmpl(); renderTools();
  floatWindow("Create Features", h("div", {}, h("div", { class: "cfHdr" }, "Create Features"), search, tlist, ctools, jobLine), { id: "createFeaturesWin", width: 240 });
}

// --- Edit Sketch Properties window (live X/Y vertex table) ---
function openSketchProps() {
  const tbl = h("table", { class: "legs" }, h("tr", {}, h("th", {}, "#"), h("th", {}, "X (Easting)"), h("th", {}, "Y (Northing)"), h("th", {}, "")));
  edit.verts.forEach(([E, N], i) => {
    const x = h("input", { type: "number", step: "0.001", value: E.toFixed(3), style: "width:92px" });
    const y = h("input", { type: "number", step: "0.001", value: N.toFixed(3), style: "width:92px" });
    x.oninput = () => { const v = parseFloat(x.value); if (isFinite(v)) { edit.verts[i][0] = v; drawSketch(); } };
    y.oninput = () => { const v = parseFloat(y.value); if (isFinite(v)) { edit.verts[i][1] = v; drawSketch(); } };
    const tr = h("tr", { class: i === edit.sel ? "selrow" : "" }, h("td", { class: "n" }, String(i + 1)), h("td", {}, x), h("td", {}, y),
      h("td", {}, h("button", { title: "Delete vertex", onclick: () => { edit.verts.splice(i, 1); edit.sel = Math.min(edit.sel, edit.verts.length - 1); drawSketch(); openSketchProps(); updateEditorButtons(); } }, "✕")));
    tr.addEventListener("click", (ev) => { if (!/INPUT|BUTTON/.test(ev.target.tagName)) { edit.sel = i; drawSketch(); openSketchProps(); } });
    tbl.append(tr);
  });
  if (!edit.verts.length) tbl.append(h("tr", {}, h("td", { colspan: 4, class: "muted" }, "No points yet: click the map, or type / paste coordinates below.")));
  // Typing points: Easting/Northing, a beacon number (its coordinates), or a pasted point list.
  const addPts = (pts) => {
    if (!edit.active) startEditing("sketch"); // typing can start the edit session (Che)
    for (const [E, N, id] of pts) {
      if (edit.geomType === "point") edit.verts = [[E, N]]; else edit.verts.push([E, N]);
      if (id) rememberPointId(E, N, id);
    }
    edit.sel = edit.verts.length - 1; drawSketch(); openSketchProps(); updateEditorButtons(); showSketchExtent();
  };
  const xIn = h("input", { type: "number", step: "0.001", placeholder: "Easting", style: "width:92px" });
  const yIn = h("input", { type: "number", step: "0.001", placeholder: "Northing", style: "width:92px" });
  // Seeded with the last vertex, or the map centre, so a nearby point only needs its last digits changed.
  const last = edit.verts[edit.verts.length - 1];
  if (last) { xIn.value = last[0].toFixed(3); yIn.value = last[1].toFixed(3); }
  else { const c = map.getCenter(); const [cE, cN] = toUTM(c.lng, c.lat); xIn.value = cE.toFixed(3); yIn.value = cN.toFixed(3); }
  const addXY = () => { const E = parseFloat(xIn.value), N = parseFloat(yIn.value); if (!isFinite(E) || !isFinite(N)) { status("Type the Easting and Northing (UTM 32N metres)."); return; } addPts([[E, N]]); setTimeout(() => $("sketchPropsWin")?.querySelector("input[placeholder=Easting]")?.focus(), 0); };
  yIn.addEventListener("keydown", (ev) => ev.key === "Enter" && addXY());
  xIn.addEventListener("keydown", (ev) => { if (ev.key === "Enter") yIn.focus(); });
  const bIn = h("input", { placeholder: "Beacon no., e.g. CA6782", style: "width:150px" });
  const addBeacon = async () => {
    const q = bIn.value.trim(); if (!q) return;
    const alt = plagisAlternate(q);
    const l = await api(`/api/v1/beacons/search?limit=5&q=${encodeURIComponent(q)}`).catch(() => []);
    const b = l.find((x) => x.number.toUpperCase() === q.toUpperCase() || (alt && x.number.toUpperCase() === alt.toUpperCase()));
    if (!b) { status(`Beacon ${q} is not in the fabric; type its coordinates instead.`); return; }
    addPts([[b.easting, b.northing, b.number]]); status(`Added beacon ${b.number} (${fmt3(b.easting)} ${fmt3(b.northing)}).`);
  };
  bIn.addEventListener("keydown", (ev) => ev.key === "Enter" && addBeacon());
  const paste = h("textarea", { rows: 3, style: "width:100%;box-sizing:border-box;font-family:monospace", placeholder: "Paste points, one per line: Easting Northing, or ID Easting Northing (as in the surveyor's point list), in boundary order" });
  const addPasted = () => {
    const pts = [];
    for (const line of paste.value.split(/\n/)) {
      const t = line.trim().split(/[\s,;|]+/).filter(Boolean);
      const hasId = t.length >= 3 && !isFinite(Number(t[0])); // "CA6782 492951.814 1088827.672"
      const [E, N] = (hasId ? t.slice(1, 3) : t.slice(0, 2)).map(Number);
      if (isFinite(E) && isFinite(N) && E > 100000 && N > 100000) pts.push([E, N, hasId ? t[0] : null]);
    }
    if (!pts.length) { status("No coordinates found in the pasted text."); return; }
    addPts(pts); status(`Added ${pts.length} point(s).`);
  };
  const body = h("div", {},
    h("div", { class: "muted" }, `${edit.editingId ? "Editing draft parcel — " : ""}${edit.template === "jobArea" ? "job area" : edit.geomType} sketch — ${edit.verts.length} vertex(es), WGS 84 / UTM 32N metres`),
    h("div", { style: "max-height:260px;overflow:auto" }, tbl),
    h("div", { class: "cfHdr", style: "margin-top:6px" }, "Add points"),
    h("div", { class: "frow" }, xIn, yIn, h("button", { class: "btn", style: "min-width:0", onclick: addXY }, "Add")),
    h("div", { class: "frow" }, bIn, h("button", { class: "btn", style: "min-width:0", onclick: addBeacon }, "Add beacon")),
    paste, h("div", { class: "frow" }, h("button", { class: "btn", onclick: addPasted }, "Add pasted points")),
    h("div", { class: "taskNav" },
      h("button", { class: "btn", disabled: !canFinishSketch(), onclick: finishSketch }, edit.editingId ? "Save Changes" : "Finish Sketch"),
      h("button", { class: "btn", disabled: !edit.verts.length, onclick: deleteSketch }, edit.editingId ? "Cancel Edit" : "Delete Sketch"),
      edit.editingId ? h("button", { class: "btn", onclick: deleteDraftBeingEdited }, "Delete Parcel") : null));
  floatWindow("Edit Sketch Properties", body, { id: "sketchPropsWin", width: 340 });
}
function refreshSketchProps() { if ($("sketchPropsWin")) openSketchProps(); }
async function deleteDraftBeingEdited() {
  if (!edit.editingId || !state.job || !confirm("Delete this draft parcel from the job?")) return;
  try {
    await api(`/api/v1/jobs/${state.job.id}/parcels/${edit.editingId}`, { method: "DELETE" });
    clearSketch(); await loadDrafts(); wfRefreshEditing(); refreshSketchProps(); updateEditorButtons(); status("Draft parcel deleted.");
  } catch (e) { alert("Delete failed: " + e.message); }
}

/* =====================================================================================
 * Phase 3 — Aumentum's Computation toolbar (ComputationToolbar.*): point and line tools that work
 * on the sketch, Donut Parcel and Parcel Separation on draft parcels, and Create Parcel by FCI.
 * Calculations are plane UTM 32N metres; bearings are whole-circle from grid north.
 * ===================================================================================== */
// A workflow step's own tools, as a small window of buttons.
function stepTools(title, items) {
  const w = floatWindow(title, h("div", { style: "display:flex;flex-direction:column;gap:4px" },
    ...items.map(([label, fn]) => h("button", { class: "btn", style: "text-align:left", onclick: () => { w.remove(); fn(); } }, label))), { id: "stepToolsWin", width: 330 });
}
function computationMenu() {
  const sel = edit.sel >= 0 && edit.sel < edit.verts.length;
  return [
    { label: "List Control Points…", action: listControlPoints },
    { label: "Link Control To Point…", action: linkControlToPoint, disabled: () => !sel },
    { label: "Move Point…", action: movePointDialog, disabled: () => !sel },
    { label: "Point Attributes…", action: pointAttributesDialog, disabled: () => !sel },
    "-",
    { label: "Create Multi Point Line…", action: multiPointLineDialog },
    { label: "Create Connection Line…", action: connectionLineDialog },
    "-",
    { label: "Create Donut Parcel", action: () => startPickDraft("donut"), disabled: () => !(edit.geomType === "polygon" && edit.verts.length >= 3 && !edit.editingId) },
    { label: "Parcel Separation", action: () => startPickDraft("separate"), disabled: () => !(edit.verts.length >= 2 && !edit.editingId) },
    "-",
    { label: "Split Parcel (sketch a line)", action: () => startSplitLine() },
    { label: "Merge Selected Parcels", action: mutationMerge },
    { label: "Re-survey (Adjust) Selected Parcel", action: mutationResurvey },
    { label: "Free Mutation of Selected Parcel", action: mutationFree },
    { label: "Separate Selected Parcel (sketch a line)", action: () => startSplitLine("separate") },
    { label: "Retire Selected Parcel…", action: mutationRetire },
    { label: "Parcel Lineage…", action: () => lineageOfSelection() },
    "-",
    { label: "Create Parcel by FCI (Feature Class Import)…", action: fciDialog },
    { label: "Import Points (Import Wizard)…", action: openImportDialog },
    { label: "Computation Tools (35)…", action: openComputeTools },
  ];
}
// Whole-circle bearing as d°m's" and back; inverse and forward in the plane.
function fmtDMS(deg) {
  let d = ((deg % 360) + 360) % 360, D = Math.floor(d), M = Math.floor((d - D) * 60), S = Math.round(((d - D) * 60 - M) * 60);
  if (S === 60) { S = 0; M++; } if (M === 60) { M = 0; D = (D + 1) % 360; }
  return `${D}°${String(M).padStart(2, "0")}'${String(S).padStart(2, "0")}"`;
}
const inverseUTM = ([e1, n1], [e2, n2]) => ({ bearing: ((Math.atan2(e2 - e1, n2 - n1) * 180 / Math.PI) + 360) % 360, distance: Math.hypot(e2 - e1, n2 - n1) });
const forwardUTM = ([e, n], bearing, distance) => { const r = bearing * Math.PI / 180; return [e + distance * Math.sin(r), n + distance * Math.cos(r)]; };
// A point typed as "E N", or a beacon / control point number (either PLAGIS form).
async function resolvePoint(text) {
  const t = String(text || "").trim(); if (!t) return null;
  const nums = t.split(/[\s,;]+/).map(Number);
  if (nums.length >= 2 && nums.slice(0, 2).every(isFinite)) return { e: nums[0], n: nums[1], id: null };
  const alt = plagisAlternate(t);
  const l = await api(`/api/v1/beacons/search?limit=5&q=${encodeURIComponent(t)}`).catch(() => []);
  const b = l.find((x) => x.number.toUpperCase() === t.toUpperCase() || (alt && x.number.toUpperCase() === alt.toUpperCase()));
  return b ? { e: b.easting, n: b.northing, id: b.number } : null;
}
const needVertex = () => { if (edit.sel >= 0 && edit.sel < edit.verts.length) return true; status("Select a sketch point first (click its row in Edit Sketch Properties, or click it on the map)."); return false; };
function afterVertexChange(msg) { drawSketch(); refreshSketchProps(); updateEditorButtons(); showSketchExtent(); if (msg) status(msg); }

// List Control Points: control points and beacons nearest the map centre (or matching a number);
// a row zooms there, or adds the point to the sketch.
function listControlPoints() {
  const q = h("input", { placeholder: "Number (blank = nearest the map centre)", style: "flex:1" });
  const only = h("input", { type: "checkbox", checked: true });
  const list = h("div", { style: "max-height:330px;overflow:auto;margin-top:6px" });
  const run = async () => {
    const c = map.getCenter(), [e, n] = toUTM(c.lng, c.lat);
    const l = await api(`/api/v1/beacons/search?limit=200&q=${encodeURIComponent(q.value.trim())}&e=${e.toFixed(1)}&n=${n.toFixed(1)}${only.checked ? "&control=true" : ""}`).catch(() => []);
    const t = h("table", { class: "legs" }, h("tr", {}, h("th", {}, "Point"), h("th", {}, "Easting"), h("th", {}, "Northing"), h("th", {}, "Dist (m)"), h("th", {}, "")));
    for (const b of l.slice(0, 100)) {
      const tr = h("tr", { style: "cursor:pointer", title: "Zoom to point" }, h("td", {}, b.number + (b.is_control ? " (control)" : "")), h("td", {}, fmt3(b.easting)), h("td", {}, fmt3(b.northing)),
        h("td", {}, Math.hypot(b.easting - e, b.northing - n).toFixed(1)),
        h("td", {}, h("button", { class: "btn", style: "min-width:0", title: "Add to the sketch", onclick: (ev) => { ev.stopPropagation(); if (!edit.active) startEditing("sketch"); edit.verts.push([b.easting, b.northing]); rememberPointId(b.easting, b.northing, b.number); edit.sel = edit.verts.length - 1; afterVertexChange(`Added ${b.number} to the sketch.`); } }, "Add")));
      tr.onclick = () => showFoundBeacons([b]);
      t.append(tr);
    }
    list.replaceChildren(l.length ? t : h("p", { class: "muted" }, "No points found."));
  };
  q.addEventListener("keydown", (ev) => ev.key === "Enter" && run());
  only.onchange = run;
  floatWindow("List Control Points", h("div", {}, h("div", { class: "frow" }, q, h("button", { class: "btn", style: "min-width:0", onclick: run }, "List")),
    h("label", { style: "display:flex;gap:6px;align-items:center" }, only, "Control points only"), list), { id: "listCtrlWin", width: 470 });
  run();
}
// Link Control To Point: the selected sketch point takes a control point's / beacon's exact coordinates.
function linkControlToPoint() {
  if (!needVertex()) return;
  const i = edit.sel, inp = h("input", { placeholder: "e.g. XSK101 or C6782A", style: "flex:1" }), msg = h("div", { class: "muted" });
  const go = async () => {
    const p = await resolvePoint(inp.value);
    if (!p || !p.id) { msg.textContent = `${inp.value.trim()} is not a control point or beacon in the fabric.`; return; }
    const [e0, n0] = edit.verts[i]; edit.verts[i] = [p.e, p.n]; rememberPointId(p.e, p.n, p.id);
    w.remove(); afterVertexChange(`Point ${i + 1} linked to ${p.id} (moved ${Math.hypot(p.e - e0, p.n - n0).toFixed(3)} m).`);
  };
  inp.addEventListener("keydown", (ev) => ev.key === "Enter" && go());
  const w = floatWindow("Link Control To Point", h("div", {}, h("p", { class: "muted", style: "margin-top:0" }, `Sketch point ${i + 1}: ${fmt3(edit.verts[i][0])} ${fmt3(edit.verts[i][1])}`),
    h("div", { class: "frow" }, h("label", {}, "Control point:"), inp, h("button", { class: "btn", style: "min-width:0", onclick: go }, "Link")), msg), { id: "linkCtrlWin", width: 380 });
  setTimeout(() => inp.focus(), 0);
}
// Move Point: to typed coordinates, by a shift, or by a bearing and distance.
function movePointDialog() {
  if (!needVertex()) return;
  const i = edit.sel, [e0, n0] = edit.verts[i];
  const E = h("input", { type: "number", step: "0.001", value: e0.toFixed(3), style: "width:110px" }), N = h("input", { type: "number", step: "0.001", value: n0.toFixed(3), style: "width:110px" });
  const dE = h("input", { type: "number", step: "0.001", value: "0", style: "width:80px" }), dN = h("input", { type: "number", step: "0.001", value: "0", style: "width:80px" });
  const B = h("input", { placeholder: "ddd mm ss", style: "width:100px" }), D = h("input", { type: "number", step: "0.001", style: "width:80px" });
  const apply = (p, how) => { edit.verts[i] = p; w.remove(); afterVertexChange(`Point ${i + 1} moved ${how} to ${fmt3(p[0])} ${fmt3(p[1])}.`); };
  const w = floatWindow("Move Point", h("div", {},
    h("p", { class: "muted", style: "margin-top:0" }, `Sketch point ${i + 1}: ${fmt3(e0)} ${fmt3(n0)}`),
    h("div", { class: "frow" }, h("label", {}, "To E / N:"), E, N, h("button", { class: "btn", style: "min-width:0", onclick: () => { const p = [parseFloat(E.value), parseFloat(N.value)]; if (p.every(isFinite)) apply(p, "absolutely"); } }, "Move")),
    h("div", { class: "frow" }, h("label", {}, "By ΔE / ΔN:"), dE, dN, h("button", { class: "btn", style: "min-width:0", onclick: () => { const a = parseFloat(dE.value), b = parseFloat(dN.value); if (isFinite(a) && isFinite(b)) apply([e0 + a, n0 + b], "by a shift"); } }, "Move")),
    h("div", { class: "frow" }, h("label", {}, "Bearing / dist:"), B, D, h("button", { class: "btn", style: "min-width:0", onclick: () => { const b = parseBearing(B.value), d = parseFloat(D.value); if (isFinite(b) && isFinite(d)) apply(forwardUTM([e0, n0], b, d), `${d} m on ${fmtDMS(b)}`); } }, "Move"))),
    { id: "movePtWin", width: 420 });
}
// Point Attributes: the selected sketch point's beacon number (kept for the commit) and position.
function pointAttributesDialog() {
  if (!needVertex()) return;
  const i = edit.sel, [e, n] = edit.verts[i];
  const num = h("input", { value: pointIds.get(ptKey(e, n)) || "", placeholder: "e.g. CA6782", style: "width:140px" });
  const prev = edit.verts[(i - 1 + edit.verts.length) % edit.verts.length], next = edit.verts[(i + 1) % edit.verts.length];
  const leg = (a, b) => { const r = inverseUTM(a, b); return `${fmtDMS(r.bearing)}  ${r.distance.toFixed(3)} m`; };
  const save = () => { rememberPointId(e, n, num.value); w.remove(); refreshSketchProps(); status(num.value.trim() ? `Point ${i + 1} is beacon ${num.value.trim()} (used at commit).` : "Point attributes saved."); };
  num.addEventListener("keydown", (ev) => ev.key === "Enter" && save());
  const w = floatWindow("Point Attributes", h("div", {},
    h("table", { class: "legs" },
      h("tr", {}, h("th", {}, "Point"), h("td", {}, String(i + 1))),
      h("tr", {}, h("th", {}, "Easting"), h("td", {}, fmt3(e))), h("tr", {}, h("th", {}, "Northing"), h("td", {}, fmt3(n))),
      edit.verts.length > 1 ? h("tr", {}, h("th", {}, "From previous"), h("td", {}, leg(prev, [e, n]))) : null,
      edit.verts.length > 1 ? h("tr", {}, h("th", {}, "To next"), h("td", {}, leg([e, n], next))) : null),
    h("div", { class: "frow" }, h("label", {}, "Beacon number:"), num, h("button", { class: "btn", style: "min-width:0", onclick: save }, "OK"))), { id: "ptAttrWin", width: 360 });
  setTimeout(() => num.focus(), 0);
}
// Create Multi Point Line: points by bearing and distance from a start point (a beacon, E N, or
// the last sketch point); added to the sketch, or drawn as a line.
function multiPointLineDialog() {
  const last = edit.verts[edit.verts.length - 1];
  const start = h("input", { value: last ? `${last[0].toFixed(3)} ${last[1].toFixed(3)}` : "", placeholder: "Beacon number, or E N", style: "flex:1" });
  const legs = h("textarea", { rows: 6, style: "width:100%;box-sizing:border-box;font-family:monospace", placeholder: "One leg per line: bearing distance\ne.g. 45 30 00  25.430" });
  const out = h("div", { class: "muted", style: "margin:4px 0" });
  const compute = async () => {
    const s0 = await resolvePoint(start.value); if (!s0) { out.textContent = "Give the start point (a beacon number, or E N)."; return null; }
    const pts = [[s0.e, s0.n, s0.id]]; let bad = 0;
    for (const line of legs.value.split(/\n/)) {
      const t = line.trim().split(/\s+/).filter(Boolean); if (!t.length) continue;
      const d = parseFloat(t[t.length - 1]), b = parseBearing(t.slice(0, -1).join(" "));
      if (!isFinite(b) || !isFinite(d) || d <= 0) { bad++; continue; }
      const [e, n] = forwardUTM(pts[pts.length - 1], b, d); pts.push([e, n, null]);
    }
    const end = pts[pts.length - 1];
    out.textContent = `${pts.length - 1} leg(s)${bad ? `, ${bad} unreadable line(s) skipped` : ""}; end point ${fmt3(end[0])} ${fmt3(end[1])}.`;
    return pts;
  };
  const toSketch = async () => {
    const pts = await compute(); if (!pts || pts.length < 2) return;
    if (!edit.active) startEditing("sketch");
    const startsAtLast = last && Math.hypot(pts[0][0] - last[0], pts[0][1] - last[1]) < 0.0005;
    for (const [e, n, id] of startsAtLast ? pts.slice(1) : pts) { edit.verts.push([e, n]); rememberPointId(e, n, id); }
    edit.sel = edit.verts.length - 1; afterVertexChange(`Added ${startsAtLast ? pts.length - 1 : pts.length} point(s) to the sketch.`);
  };
  const asLine = async () => { const pts = await compute(); if (!pts || pts.length < 2) return; addGraphic({ type: "line", pts: pts.map(([e, n]) => toLL(e, n)) }); status("Multi point line drawn."); };
  floatWindow("Create Multi Point Line", h("div", {}, h("div", { class: "frow" }, h("label", {}, "Start:"), start), legs, out,
    h("div", { class: "frow", style: "justify-content:flex-end" }, h("button", { class: "btn", onclick: compute }, "Compute"), h("button", { class: "btn", onclick: toSketch }, "Add to Sketch"), h("button", { class: "btn", onclick: asLine }, "Draw Line"))),
    { id: "multiLineWin", width: 420 });
}
// Create Connection Line: the bearing and distance between two points (e.g. a control point and a
// parcel corner), drawn on the map and labelled.
function connectionLineDialog() {
  const a = h("input", { placeholder: "From: beacon number, or E N", style: "flex:1" }), b = h("input", { placeholder: "To: beacon number, or E N", style: "flex:1" });
  if (edit.sel >= 0 && edit.verts[edit.sel]) b.value = `${edit.verts[edit.sel][0].toFixed(3)} ${edit.verts[edit.sel][1].toFixed(3)}`;
  const out = h("div", { style: "margin:6px 0;font-weight:600" });
  const go = async () => {
    const [p, q] = await Promise.all([resolvePoint(a.value), resolvePoint(b.value)]);
    if (!p || !q) { out.textContent = `${!p ? "From" : "To"} point not found.`; return; }
    const r = inverseUTM([p.e, p.n], [q.e, q.n]);
    out.textContent = `${p.id || "From"} → ${q.id || "To"}: ${fmtDMS(r.bearing)}  ${r.distance.toFixed(3)} m`;
    const A = toLL(p.e, p.n), B = toLL(q.e, q.n);
    addGraphic({ type: "line", pts: [A, B] });
    addGraphic({ type: "text", at: [(A[0] + B[0]) / 2, (A[1] + B[1]) / 2], text: `${fmtDMS(r.bearing)} ${r.distance.toFixed(3)}m` });
    zoomToFC({ type: "FeatureCollection", features: [{ type: "Feature", geometry: { type: "LineString", coordinates: [A, B] }, properties: {} }] }, { padding: 100, maxZoom: 19 });
  };
  floatWindow("Create Connection Line", h("div", {}, h("div", { class: "frow" }, a), h("div", { class: "frow" }, b), out,
    h("div", { class: "frow", style: "justify-content:flex-end" }, h("button", { class: "btn", onclick: go }, "Create"))), { id: "connLineWin", width: 400 });
}
// Donut Parcel / Parcel Separation: the current sketch is applied to a draft parcel clicked next.
let pickDraftMode = null;
function startPickDraft(mode) {
  if (!state.job) { alert("Choose the job first (Workflow → Work On This Job)."); return; }
  pickDraftMode = mode; setTool("pickDraft");
  status(mode === "donut" ? "Create Donut Parcel: click the draft parcel the sketch is a hole in." : "Parcel Separation: click the draft parcel the sketched line separates.");
}
async function pickDraftAt(pt) {
  const f = map.queryRenderedFeatures([[pt.x - 3, pt.y - 3], [pt.x + 3, pt.y + 3]], { layers: ["drafts-fill"].filter((l) => map.getLayer(l)) })[0];
  if (!f) { status("Click one of the job's draft parcels."); return; }
  const d = state.drafts.features.find((x) => String(x.properties.id) === String(f.properties.id)); if (!d) return;
  const mode = pickDraftMode; pickDraftMode = null; setTool("sketch");
  try {
    if (mode === "donut") {
      const outer = (d.geometry.type === "MultiPolygon" ? d.geometry.coordinates[0] : d.geometry.coordinates).map((ring) => ring.map(([x, y]) => toUTM(x, y).map((v) => Math.round(v * 1000) / 1000)));
      const hole = [...edit.verts, edit.verts[0]];
      await api(`/api/v1/jobs/${state.job.id}/parcels/${d.properties.id}`, { method: "PUT", body: { geometry: { type: "Polygon", coordinates: [...outer, hole] }, stated_area: null } });
      clearSketch(); await loadDrafts(); wfRefreshEditing(); refreshSketchProps(); updateEditorButtons(); notify("Donut parcel created: the sketch is now a hole in the draft parcel.");
    } else {
      const parts = await api(`/api/v1/jobs/${state.job.id}/parcels/${d.properties.id}/separate`, { method: "POST", body: { line: { type: "LineString", coordinates: edit.verts.map((v) => [...v]) } } });
      clearSketch(); await loadDrafts(); wfRefreshEditing(); refreshSketchProps(); updateEditorButtons(); notify(`Parcel separated into ${parts.length} draft parcels.`);
    }
  } catch (e) { notify((mode === "donut" ? "Donut parcel" : "Parcel separation") + " failed: " + e.message, "err"); }
}
// Create Parcel by FCI: a polygon feature class (shapefile) becomes the job's draft parcels.
function fciDialog() {
  if (!state.job) { alert("Choose the job first (Workflow → Work On This Job)."); return; }
  const node = cat.nodes.get(cat.selected);
  const catShp = node && node.folder != null && /\.shp$/i.test(node.path || "") ? node : null;
  const src = h("select", {}, catShp ? h("option", { value: "catalog" }, `Selected in the Catalog: ${catShp.name}`) : null, h("option", { value: "pc" }, "A shapefile on this PC"));
  const files = h("input", { type: "file", multiple: true, accept: ".shp,.prj,.shx,.dbf" });
  const epsg = h("input", { type: "number", placeholder: "only if no .prj", style: "width:120px" });
  const out = h("div", { style: "margin-top:6px" });
  const pcRow = h("div", { class: "frow" }, h("label", {}, "Files:"), files);
  const sync = () => { pcRow.style.display = src.value === "pc" ? "" : "none"; }; src.onchange = sync;
  const b64 = (buf) => { const u = new Uint8Array(buf); let s = ""; for (let i = 0; i < u.length; i += 0x8000) s += String.fromCharCode(...u.subarray(i, i + 0x8000)); return btoa(s); };
  const go = async () => {
    let body;
    if (src.value === "catalog") body = { folder: catShp.folder, path: catShp.path, name: catShp.name };
    else {
      const fs = [...files.files], shp = fs.find((f) => /\.shp$/i.test(f.name)), prj = fs.find((f) => /\.prj$/i.test(f.name));
      if (!shp) { out.textContent = "Choose the .shp (and its .prj) on this PC."; return; }
      body = { shp_base64: b64(await shp.arrayBuffer()), prj: prj ? await prj.text() : null, name: shp.name };
    }
    if (epsg.value) body.epsg = parseInt(epsg.value, 10);
    out.textContent = "Importing…";
    try {
      const r = await api(`/api/v1/jobs/${state.job.id}/import/feature-class`, { method: "POST", body });
      await loadDrafts(); wfRefreshEditing(); if (state.drafts.features.length) zoomToFC(state.drafts, { padding: 60 });
      out.replaceChildren(h("p", {}, h("b", {}, `${r.created_parcels} draft parcel(s) created`), ` from ${r.features} feature(s) of ${r.name || "the feature class"}.`),
        ...(r.skipped || []).map((x) => h("div", { class: "muted" }, `FID ${x.fid}: ${x.reason}`)), ...(r.warnings || []).map((x) => h("div", { class: "muted" }, x)));
      notify(`Feature class imported: ${r.created_parcels} draft parcel(s) in ${state.job.reference}.`);
    } catch (e) { out.textContent = "Import failed: " + e.message; }
  };
  floatWindow("Create Parcel by FCI", h("div", {},
    h("p", { class: "muted", style: "margin-top:0" }, `Polygons of a feature class become draft parcels of ${state.job.reference}. Under a workflow job they must all lie inside the job area. Beacon numbers are entered at commit.`),
    h("div", { class: "frow" }, h("label", {}, "Source:"), src), pcRow,
    h("div", { class: "frow" }, h("label", {}, "EPSG:"), epsg),
    h("div", { class: "frow", style: "justify-content:flex-end" }, h("button", { class: "btn", onclick: go }, "Import")), out), { id: "fciWin", width: 460 });
  sync();
}

/* =====================================================================================
 * Phase 4 — mutations and lineage (Aumentum: Split Parcel / Merge Parcel steps; re-survey and
 * retirement from its lineage history). Recorded in the workflow job, applied at commit.
 * ===================================================================================== */
const selectedUpins = () => selectedFC().features.map((f) => f.properties.parcel_number).filter(Boolean);
function needWorkflowJob() {
  if (wf.editing) return wf.editing.job_instance_id;
  alert("Open the job in the Workflow panel and choose Work On This Job first."); return null;
}
async function afterMutation(msg) {
  clearSketch(); refreshSketchProps(); updateEditorButtons();
  await loadDrafts(); await wfRefreshEditing(); if (wf.selected) await wfSelect(wf.selected);
  notify(msg);
}
async function recordMutation(body, msg) {
  const id = needWorkflowJob(); if (!id) return;
  try {
    const r = await api(`/api/v1/wf/jobs/${id}/mutations`, { method: "POST", body });
    clearSelection(); await afterMutation(typeof msg === "function" ? msg(r) : msg);
    if (r.drafts.length) zoomToFC(state.drafts, { padding: 80 });
  } catch (e) { notify(e.message, "err"); }
}
// Aumentum's mutation types (d_mutation_type), by the kind names the server takes.
const MUTATION_LABEL = { split: "Split", merge: "Merge", resurvey: "Adjust (re-survey)", free: "Free Mutation", separate: "Separation", retire: "Retire" };
// A split (type 1) or a separation (type 6): both cut one posted parcel along a sketched line.
function startSplitLine(kind = "split") {
  const verb = kind === "separate" ? "separate" : "split";
  const u = selectedUpins();
  if (u.length !== 1) { alert(`Select the one parcel to ${verb} first (Select tool).`); return; }
  if (!needWorkflowJob()) return;
  if (!edit.active) startEditing("sketch");
  clearSketch(); selectTemplate("line"); edit.splitMode = kind; setTool("sketch"); openSketchProps();
  status(`${verb[0].toUpperCase() + verb.slice(1)} ${u[0]}: click the line's points across the parcel (or type them), then Finish Sketch.`);
}
async function mutationSplit() {
  const kind = edit.splitMode === "separate" ? "separate" : "split";
  const u = selectedUpins();
  if (u.length !== 1) { alert(`Select the one parcel to ${kind} (Select tool).`); return; }
  if (edit.verts.length < 2) { alert("Sketch the line across the parcel first."); return; }
  const line = { type: "LineString", coordinates: edit.verts.map((v) => [...v]) };
  edit.splitMode = false;
  await recordMutation({ kind, upins: u, line }, (r) => `${MUTATION_LABEL[kind]} of ${u[0]} recorded: ${r.drafts.length} new parcels (drafts). They replace it when the job's parcels are committed.`);
}
async function mutationMerge() {
  const u = selectedUpins();
  if (u.length < 2) { alert("Select two or more neighbouring parcels to merge (Select tool, Shift-click to add)."); return; }
  if (!confirm(`Merge ${u.join(" + ")} into one parcel?`)) return;
  await recordMutation({ kind: "merge", upins: u }, `Merge of ${u.join(" + ")} recorded (a draft). It replaces them when the job's parcels are committed.`);
}
// Aumentum: an adjustment takes n adjacent parcels to n (each new parcel's parents: all n); a
// free mutation X parcels to Y (reshape, separate or delete the copies). New UPINs for all.
async function mutationResurvey() {
  const u = selectedUpins();
  if (!u.length) { alert("Select the parcel(s) to adjust (Select tool; Shift-click to add adjacent parcels)."); return; }
  await recordMutation({ kind: "resurvey", upins: u }, `Adjustment of ${u.join(" + ")} recorded: edit the ${u.length > 1 ? "copies" : "copy"} with the Edit tool; ${u.length > 1 ? "they replace them" : `it replaces ${u[0]}`} (new UPINs) at commit.`);
}
async function mutationFree() {
  const u = selectedUpins();
  if (!u.length) { alert("Select the parcel(s) for the free mutation (Select tool; Shift-click to add adjacent parcels)."); return; }
  await recordMutation({ kind: "free", upins: u }, `Free mutation of ${u.join(" + ")} recorded: reshape, separate or delete the copies; they replace ${u.join(" + ")} (new UPINs) at commit.`);
}
// Aumentum's Separation Configuration: which part keeps the parent's UPIN and attributes.
async function mutationKeepUpin(jobId, m, draft) {
  try { await api(`/api/v1/wf/jobs/${jobId}/mutations/${m.group}/keep-upin`, { method: "PUT", body: { draft: draft || null } }); await afterMutation(draft ? `The chosen part keeps ${m.parents[0]}.` : "Every part gets a new UPIN."); }
  catch (e) { notify(e.message, "err"); }
}
function mutationRetire() {
  const u = selectedUpins();
  if (!u.length) { alert("Select the parcel(s) to retire (Select tool)."); return; }
  if (!needWorkflowJob()) return;
  const reason = h("input", { placeholder: "e.g. RE-SURVEYED, overlapping, WRONGLY DONE", style: "flex:1" });
  const go = async () => { if (!reason.value.trim()) { reason.focus(); return; } w.remove(); await recordMutation({ kind: "retire", upins: u, reason: reason.value.trim() }, `Retirement of ${u.join(", ")} recorded; applied when the job is committed.`); };
  reason.addEventListener("keydown", (ev) => ev.key === "Enter" && go());
  const w = floatWindow("Retire Parcel", h("div", {}, h("p", { style: "margin-top:0" }, `Retire ${u.join(", ")}: archived with this reason (Aumentum's Archive Reason). The UPIN is never reused.`),
    h("div", { class: "frow" }, h("label", {}, "Reason:"), reason), h("div", { class: "frow", style: "justify-content:flex-end" }, h("button", { class: "btn", onclick: go }, "Retire"))), { id: "retireWin", width: 400 });
  setTimeout(() => reason.focus(), 0);
}
function lineageOfSelection() {
  const u = selectedUpins();
  if (u.length) { lineageWindow(u[0]); return; }
  const v = prompt("Parcel UPIN:", ""); if (v && v.trim()) lineageWindow(v.trim());
}
// Lineage: where a parcel came from and what it became (Aumentum's history and mutations made here).
async function lineageWindow(upin) {
  let d;
  try { d = await api(`/api/v1/parcels/lineage?upin=${encodeURIComponent(upin)}`); } catch (e) { alert(e.message); return; }
  const body = h("div", {});
  const link = (u) => u ? h("a", { href: "#", onclick: (ev) => { ev.preventDefault(); lineageWindow(u); } }, u) : "?";
  const groups = new Map();
  for (const e of d.edges) { const k = `${e.mutation_id || ""}|${e.mutation}|${(e.date || "").slice(0, 10)}`; if (!groups.has(k)) groups.set(k, []); groups.get(k).push(e); }
  if (!groups.size && !d.retirements.length) body.append(h("p", { class: "muted" }, `No recorded mutations for ${d.upin}.`));
  for (const [k, es] of groups) {
    const [, name, date] = k.split("|");
    const parents = [...new Set(es.map((e) => e.parent_upin))], kids = [...new Set(es.map((e) => e.child_upin))];
    const row = h("div", { style: `margin:4px 0;padding:4px 6px;border-left:3px solid ${parents.includes(d.upin) || kids.includes(d.upin) ? "#b22222" : "#999"}` },
      h("b", {}, `${date || "—"}  ${name}`), h("span", { class: "muted" }, es[0].source === "aumentum" ? "  (Aumentum)" : ""), h("br"));
    parents.forEach((u, i) => { if (i) row.append(" + "); row.append(link(u)); });
    row.append(" → "); kids.forEach((u, i) => { if (i) row.append(", "); row.append(link(u)); });
    body.append(row);
  }
  for (const r of d.retirements) body.append(h("div", { style: "margin:4px 0;padding:4px 6px;border-left:3px solid #666" }, h("b", {}, `${(r.date || "").slice(0, 10)}  Retired `), link(r.upin), `: ${r.reason || ""}`));
  if (d.shapes.length) {
    const t = h("table", { class: "legs", style: "margin-top:6px" }, h("tr", {}, h("th", {}, "UPIN"), h("th", {}, "Status"), h("th", {}, "Area (m²)"), h("th", {}, "")));
    for (const s of d.shapes) t.append(h("tr", {}, h("td", {}, s.upin), h("td", {}, s.status === "posted" ? "active" : s.status), h("td", {}, s.area.toFixed(2)),
      h("td", {}, h("button", { class: "btn", style: "min-width:0", onclick: async () => {
        const l = await api(`/api/v1/parcels/search?number=${encodeURIComponent(s.upin)}`).catch(() => []);
        const p = l.find((x) => x.parcel_number === s.upin && x.status === s.status);
        if (!p) return;
        if (s.status !== "posted") { const lyr = allLeaves().find((x) => x.id === "history"); if (lyr && !lyr.visible) { lyr.visible = true; applyVisibility(); buildTOC(); } }
        zoomToFC({ type: "FeatureCollection", features: [resultFeature(p)] });
      } }, "Zoom"))));
    body.append(h("div", { class: "wfSub", style: "margin-top:6px" }, "Shapes in the fabric"), t);
  }
  floatWindow(`Lineage — ${d.upin}`, body, { id: "lineageWin", width: 440 });
}

/* =====================================================================================
 * Phase 5 — Catalog: live folder connections (e.g. \\PRODUCTIONMC\Image), browsed through the
 * server's read-only /api/v1/catalog endpoints — nothing here is hard-coded.
 * ===================================================================================== */
const CAT_ICON = { folder: "folder", file_geodatabase: "gdb", toolbox: "toolbox", raster: "raster", raster_band: "band",
  map_document: "mxd", layer_file: "lyr", kml: "kml", table: "table", workbook: "workbook", database_connection: "dbConn" };
const CAT_TYPE = { folder: "Folder", file_geodatabase: "File Geodatabase", toolbox: "Toolbox", raster: "Raster Dataset", raster_band: "Raster Band",
  map_document: "ArcMap Document", layer_file: "Layer", kml: "KML", table: "Table", workbook: "Excel Workbook", database_connection: "Database Connection" };
const shpIcon = (g) => (g === "polyline" ? "lineFc" : g === "point" || g === "multipoint" ? "pointFc" : "polygonFc");
const cat = {
  nodes: new Map(),           // key -> node
  open: new Set(["fc", "dc"]), selected: null, back: [], fwd: [], contentsVisible: true, view: "list",
};
// Node keys: "fc" (Folder Connections), "f:<folder>:<path>" (share items), "tb" / "tb:<tool>" (toolboxes).
function catNode(key, props) { const n = cat.nodes.get(key) || { key, children: null, loading: false, error: null }; Object.assign(n, props); cat.nodes.set(key, n); return n; }
function catItemKey(folder, path) { return `f:${folder}:${path}`; }
function catUNC(n) { return n.folder == null ? n.name : (cat.roots[n.folder].name + (n.path ? "\\" + n.path.replace(/\//g, "\\") : "")); }
cat.roots = [];
catNode("fc", { name: "Folder Connections", icon: "folderConns", expandable: true, children: [] });
catNode("dc", { name: "Database Connections", icon: "dbConn", expandable: true, children: [] });
catNode("tb", { name: "Toolboxes", icon: "toolbox", expandable: true, children: ["tb:validate", "tb:survey"] });
catNode("tb:validate", { name: "Validate Fabric", icon: "validate", expandable: false, run: () => validateFabric(), type: "Tool" });
catNode("tb:survey", { name: "Survey Closure Check", icon: "survey", expandable: false, run: () => openTask("survey"), type: "Tool" });

async function loadCatalogRoots() {
  try {
    cat.roots = await api("/api/v1/catalog");
    const keys = cat.roots.map((r) => {
      const k = catItemKey(r.id, ""); catNode(k, { name: r.name, icon: "folderConn", expandable: true, folder: r.id, path: "", kind: "folder", type: "Folder Connection" }); return k;
    });
    catNode("fc", { children: keys });
    if (keys.length) { cat.open.add(keys[0]); await catLoad(keys[0]); if (!cat.selected) cat.selected = keys[0]; }
  } catch (e) { catNode("fc", { error: "Could not load folder connections: " + e.message }); }
  buildCatalog();
}
async function loadCatalogDatabases() {
  try {
    const dbs = await api("/api/v1/catalog/databases");
    const keys = dbs.map((d) => {
      const k = `db:${d.id}`;
      catNode(k, { name: d.name, icon: "dbConn", expandable: true, db: d.id, kind: "database_connection", type: "Database Connection" });
      return k;
    });
    catNode("dc", { children: keys });
    if (keys.length) { cat.open.add(keys[0]); await catLoad(keys[0]); }
  } catch (e) { catNode("dc", { error: "Could not load database connections: " + e.message }); }
  buildCatalog();
}
async function catLoad(key) {
  const n = cat.nodes.get(key);
  if (!n || n.children || n.loading) return;
  if (n.folder == null && n.db == null) return; // only folder & database-connection nodes have children to fetch
  n.loading = true; buildCatalog();
  try {
    if (n.db != null) {
      // A geodatabase connection: its children are feature classes.
      const fcs = await api(`/api/v1/catalog/database/items?db=${n.db}`);
      n.children = fcs.map((fc) => {
        const k = `dbfc:${n.db}:${fc.name}`;
        catNode(k, { name: fc.name, geometry: fc.geometry, expandable: false,
          icon: shpIcon(fc.geometry), type: `Feature Class (${fc.geometry})` });
        return k;
      });
    } else {
      const items = await api(`/api/v1/catalog/items?folder=${n.folder}&path=${encodeURIComponent(n.path)}`);
      n.children = items.map((it) => {
        const path = n.path ? `${n.path}/${it.name}` : it.name;
        const k = catItemKey(n.folder, path);
        catNode(k, { name: it.name, kind: it.kind, geometry: it.geometry, expandable: it.expandable, size: it.size, modified: it.modified,
          folder: n.folder, path, icon: it.kind === "shapefile" ? shpIcon(it.geometry) : CAT_ICON[it.kind] || "folder",
          type: it.kind === "shapefile" ? `Shapefile (${it.geometry || "?"})` : CAT_TYPE[it.kind] || it.kind });
        return k;
      });
    }
    n.error = null;
  } catch (e) { n.error = e.message; n.children = []; }
  n.loading = false; buildCatalog();
}
async function catToggle(key) {
  if (cat.open.has(key)) cat.open.delete(key); else { cat.open.add(key); await catLoad(key); }
  buildCatalog();
}
async function catSelect(key, { record = true } = {}) {
  if (record && cat.selected && cat.selected !== key) { cat.back.push(cat.selected); cat.fwd = []; }
  cat.selected = key;
  const n = cat.nodes.get(key);
  if (n && n.expandable) await catLoad(key);
  buildCatalog();
}
function catParentKey(key) {
  for (const [k, n] of cat.nodes) if ((n.children || []).includes(key)) return k;
  return null;
}
function catAncestors(key) { const out = []; for (let k = key; k; k = catParentKey(k)) out.unshift(k); return out; }
async function catActivate(key) {
  const n = cat.nodes.get(key); if (!n) return;
  if (n.run) return n.run();
  if (n.expandable) { cat.open.add(key); for (const a of catAncestors(key)) cat.open.add(a); return catSelect(key); }
  catProperties(key);
}
function catProperties(key) {
  const n = cat.nodes.get(key); if (!n) return;
  const rows = [["Name", n.name], ["Type", n.type || "—"], ["Location", n.folder != null ? catUNC(cat.nodes.get(catParentKey(key)) || n) : "—"]];
  if (n.geometry) rows.push(["Geometry", n.geometry]);
  if (n.size != null) rows.push(["Size", n.size >= 1 << 30 ? (n.size / (1 << 30)).toFixed(2) + " GB" : n.size >= 1 << 20 ? (n.size / (1 << 20)).toFixed(1) + " MB" : n.size.toLocaleString() + " bytes"]);
  if (n.modified) rows.push(["Modified", new Date(n.modified).toLocaleString()]);
  if (n.kind === "raster" && n.children) rows.push(["Bands", String(n.children.length)]);
  floatWindow(`${n.name} Properties`, h("table", {}, ...rows.map(([k, v]) => h("tr", {}, h("th", {}, k), h("td", {}, v)))), { id: "catPropWin", width: 380 });
}
function catContextMenu(key, x, y) {
  const n = cat.nodes.get(key);
  contextMenu([
    { label: n.expandable ? (cat.open.has(key) ? "Collapse" : "Expand") : "Open", action: () => (n.expandable ? catToggle(key) : catActivate(key)), disabled: !n.expandable && !n.run },
    // Same as dragging the item onto the Table Of Contents.
    ...(n.kind === "raster" || n.kind === "shapefile" ? [{ label: "Add To Map", icon: "addData", action: () => addCatalogLayer(n) }] : []),
    { label: "Copy Location", action: () => { const loc = n.folder != null ? catUNC(n) : n.name; navigator.clipboard?.writeText(loc); status("Copied " + loc); }, disabled: n.folder == null },
    { label: "Refresh", icon: "refresh", action: () => catRefresh(key), disabled: n.folder == null },
    "-",
    { label: "Properties…", action: () => catProperties(key), disabled: n.folder == null && !n.run },
  ], x, y);
}
async function catRefresh(key) {
  const n = cat.nodes.get(key || cat.selected); if (!n) return;
  if (n.folder == null) { await loadCatalogRoots(); return; }
  const target = n.expandable ? n : cat.nodes.get(catParentKey(n.key));
  if (target) { target.children = null; await catLoad(target.key); }
}
function catRow(key, depth, inContents) {
  const n = cat.nodes.get(key);
  const isOpen = cat.open.has(key);
  const ic = n.icon === "folder" && isOpen && !inContents ? "folderOpen" : n.icon;
  const row = h("div", { class: "node", title: n.folder != null ? catUNC(n) : n.name });
  if (!inContents) row.append(n.expandable ? twisty(isOpen, () => catToggle(key)) : h("span", { class: "tw none" }));
  row.append(h("span", { html: icon(ic) }), h("span", { class: `lbl${cat.selected === key ? " sel" : ""}` }, n.name));
  if (n.kind === "raster" || n.kind === "shapefile") {
    row.setAttribute("draggable", "true");
    row.addEventListener("dragstart", (ev) => {
      ev.dataTransfer.setData("text/cad-item", key); ev.dataTransfer.effectAllowed = "copy";
      status(`Drag ${n.name} onto the map or Table Of Contents to add it as a layer.`);
    });
  }
  if (inContents && cat.view === "details") {
    row.append(h("span", { class: "catCol" }, n.type || ""), h("span", { class: "catCol num" }, n.size != null ? Math.round(n.size / 1024).toLocaleString() + " KB" : ""));
  }
  row.onclick = () => catSelect(key);
  row.ondblclick = () => catActivate(key);
  row.oncontextmenu = (ev) => { ev.preventDefault(); catSelect(key); catContextMenu(key, ev.clientX, ev.clientY); };
  return row;
}
function catSubtree(key) {
  const n = cat.nodes.get(key);
  const li = h("li", {}, catRow(key));
  if (cat.open.has(key) && (n.children || n.loading || n.error)) {
    const ul = h("ul");
    if (n.loading && !n.children) ul.append(h("li", {}, h("div", { class: "node muted" }, h("span", { class: "tw none" }), "Loading…")));
    if (n.error) ul.append(h("li", {}, h("div", { class: "node", style: "color:#c62020" }, h("span", { class: "tw none" }), n.error)));
    for (const c of n.children || []) ul.append(catSubtree(c));
    li.append(ul);
  }
  return li;
}
function buildCatalog() {
  if (!$("catTree")) return;
  const tree = $("catTree"); const scroll = tree.scrollTop; tree.innerHTML = "";
  tree.append(h("ul", { class: "tree ctree" }, catSubtree("fc"), catSubtree("dc"), catSubtree("tb")));
  tree.scrollTop = scroll;
  // Location: the selected location, with its ancestors to step back up.
  const loc = $("catLocation"); loc.innerHTML = "";
  const sel = cat.selected && cat.nodes.get(cat.selected);
  const where = sel && !sel.expandable ? catParentKey(sel.key) : cat.selected;
  for (const k of where ? catAncestors(where) : []) { const n = cat.nodes.get(k); loc.append(h("option", { value: k, selected: k === where }, n.name)); }
  // Contents: the selected container's items (a selected dataset shows itself).
  const contents = $("catContents"); contents.innerHTML = "";
  contents.classList.toggle("hidden", !cat.contentsVisible);
  if (!sel) return;
  const list = sel.expandable ? (sel.children || []) : [sel.key];
  const ul = h("ul", { class: `tree catList${cat.view === "details" ? " details" : ""}` });
  if (sel.expandable && sel.loading) ul.append(h("li", { class: "muted", style: "padding:3px" }, "Loading…"));
  for (const k of list) ul.append(h("li", {}, catRow(k, 0, true)));
  if (sel.expandable && sel.children && !sel.children.length) ul.append(h("li", { class: "muted", style: "padding:3px" }, "(no GIS items)"));
  contents.append(ul);
}
$("catLocation").addEventListener("change", (e) => catSelect(e.target.value));
(function buildCatTools() {
  const t = $("catTools");
  const add = (ic, tip, fn, opts = {}) => { const b = h("span", { class: `tb${opts.drop ? " drop" : ""}${opts.disabled ? " disabled" : ""}`, title: tip, html: icon(ic), id: opts.id || null }); if (!opts.disabled) b.onclick = (ev) => fn(ev, b); t.append(b); return b; };
  add("back", "Back", (ev) => {
    if (ev.offsetX > 22) return contextMenu(cat.back.slice(-10).reverse().map((k) => ({ label: cat.nodes.get(k)?.name || k, action: () => { cat.fwd.push(cat.selected); cat.back.splice(cat.back.lastIndexOf(k)); catSelect(k, { record: false }); } })), ev.clientX, ev.clientY);
    const k = cat.back.pop(); if (k) { cat.fwd.push(cat.selected); catSelect(k, { record: false }); }
  }, { drop: true });
  add("forward", "Forward", () => { const k = cat.fwd.pop(); if (k) { cat.back.push(cat.selected); catSelect(k, { record: false }); } });
  t.append(h("span", { class: "tsep" }));
  add("up", "Up One Level", () => { const p = catParentKey(cat.selected); if (p) catSelect(p); });
  add("home", "Default Home Folder", () => { const k = cat.nodes.get("fc").children?.[0]; if (k) catActivate(k); });
  add("connectFolder", "Connect To Folder (folder connections are configured on the server)", null, { disabled: true });
  t.append(h("span", { class: "tsep" }));
  add("views", "Views", (ev, b) => { const r = b.getBoundingClientRect(); contextMenu([
    { label: "List", checked: () => cat.view === "list", action: () => { cat.view = "list"; buildCatalog(); } },
    { label: "Details", checked: () => cat.view === "details", action: () => { cat.view = "details"; buildCatalog(); } }], r.left, r.bottom); }, { drop: true });
  t.append(h("span", { class: "tsep" }));
  add("refresh", "Refresh", () => catRefresh(cat.selected));
  add("toggleContents", "Toggle Contents Panel", () => { cat.contentsVisible = !cat.contentsVisible; buildCatalog(); });
})();

/* =====================================================================================
 * Phase 6 — Drawing toolbar (graphics: rectangles + text), persisted locally
 * ===================================================================================== */
function saveGraphics() { localStorage.setItem("cad.graphics", JSON.stringify(state.graphics.map(({ selected, ...g }) => g))); }
function drawStyle() {
  return { stroke: $("colLine")?.value || "#7a2a9a", fill: $("colFill")?.value || "#ffd400", font: $("colFont")?.value || "#d10000",
    family: $("fontBox")?.value || "Arial", size: Number($("sizeBox")?.value || 10),
    bold: $("fmtB")?.classList.contains("active"), italic: $("fmtI")?.classList.contains("active"), underline: $("fmtU")?.classList.contains("active") };
}
function addGraphic(g) { const id = crypto.randomUUID?.() || String(Date.now()); state.graphics.push({ id, ...drawStyle(), ...g }); renderGraphics(); return id; }
function addTextGraphic(ll) { const text = prompt("Text:", "Text"); if (text) addGraphic({ type: "text", at: [ll.lng, ll.lat], text }); }
const gfxMarkers = [];
function renderGraphics() {
  saveGraphics();
  const feats = state.graphics.filter((g) => g.type === "rect").map((g) => ({ type: "Feature", properties: { id: g.id, stroke: g.stroke, fill: g.fill, fillOpacity: 0.35, selected: !!g.selected },
    geometry: { type: "Polygon", coordinates: [[[g.sw[0], g.sw[1]], [g.ne[0], g.sw[1]], [g.ne[0], g.ne[1]], [g.sw[0], g.ne[1]], [g.sw[0], g.sw[1]]]] } }));
  for (const g of state.graphics.filter((x) => x.type === "line")) feats.push({ type: "Feature", properties: { id: g.id, stroke: g.stroke, fill: g.fill, fillOpacity: 0, selected: !!g.selected },
    geometry: { type: "LineString", coordinates: g.pts } });
  map.getSource("graphics")?.setData({ type: "FeatureCollection", features: feats });
  gfxMarkers.splice(0).forEach((m) => m.remove());
  for (const g of state.graphics.filter((x) => x.type === "text")) {
    const el = h("div", { class: "gfxText", style: `font-family:${g.family};font-size:${g.size * 1.33}px;color:${g.font};font-weight:${g.bold ? 700 : 400};` +
      `font-style:${g.italic ? "italic" : "normal"};text-decoration:${g.underline ? "underline" : "none"};white-space:nowrap;` +
      `outline:${g.selected ? "1px dashed #00f" : "none"};text-shadow:0 0 2px #fff;cursor:default` }, g.text);
    el.addEventListener("click", (ev) => { if (state.tool !== "selectEl") return; ev.stopPropagation(); state.graphics.forEach((x) => (x.selected = x === g ? !x.selected : ev.shiftKey && x.selected)); renderGraphics(); });
    gfxMarkers.push(new maplibregl.Marker({ element: el }).setLngLat(g.at).addTo(map));
  }
  applyVisibility();
}
function selectGraphicAt(e) {
  const f = map.queryRenderedFeatures(e.point, { layers: ["graphics-fill"] })[0];
  state.graphics.forEach((g) => (g.selected = f && g.id === f.properties.id ? !g.selected : e.originalEvent.shiftKey && g.selected));
  renderGraphics();
}
function deleteSelectedGraphics() { state.graphics = state.graphics.filter((g) => !g.selected); renderGraphics(); }
function applyStyleToSelected() { const s = drawStyle(); let n = 0; state.graphics.forEach((g) => { if (g.selected) { Object.assign(g, s); n++; } }); if (n) renderGraphics(); }
function zoomToGraphics() {
  const sel = state.graphics.filter((g) => g.selected); const gs = sel.length ? sel : state.graphics;
  const b = new maplibregl.LngLatBounds(); gs.forEach((g) => (g.type === "rect" ? (b.extend(g.sw), b.extend(g.ne)) : g.type === "line" ? g.pts.forEach((p) => b.extend(p)) : b.extend(g.at)));
  if (!b.isEmpty()) map.fitBounds(b, { padding: 60, maxZoom: 18 });
}

/* =====================================================================================
 * Phase 2 — Workflow (Aumentum job instances, tasks and steps)
 * Mirrors Aumentum's Workflow tab: job instance list, the current task and its steps, moving the
 * job along its task graph (branches and rework), comments, hold, reassign and history.
 * ===================================================================================== */
// Aumentum COM tools used by workflow steps (CLSID -> class), from the add-in's registration on
// PRODUCTIONMC. Step commands not listed here are ArcMap's own built-in tools.
const AUM_TOOL_GUIDS = {"{0595D611-38E1-44cb-B371-1AA836158D1C}":"Ils.MultiCadastre.Desktop.LineAge.LineAgeSelectTemplateCommand","{05F057AE-2439-41fd-9055-398E79A2AAE1}":"Ils.MultiCadastre.Desktop.CreateMap.SendCreateMapToLRSCommand","{0BC18962-91E8-446a-9413-F3104D47D9D6}":"Ils.MultiCadastre.Desktop.EditJobAreaCommand","{0FF462C4-AB8B-4735-B153-11284B5B2E60}":"Ils.MultiCadastre.Desktop.CreateMap.CreateSurveyPlanCommand","{27226abb-b63c-4dc6-ba79-85326d31923b}":"Ils.MultiCadastre.Desktop.COGOTools","{2997CA47-9651-478e-AC0B-38DD3D9374A1}":"Ils.MultiCadastre.Desktop.ComputationCreateConnectionLine","{3063E881-2310-4480-A35C-9370E70A1DD8}":"Ils.MultiCadastre.Desktop.MC_ImportFeatures.ImportFeatureClass","{38C747B4-0B79-4368-AF89-7CC8A6238ED1}":"Ils.MultiCadastre.Desktop.AttributeImportConfigCmd","{3C6C3C8A-2249-49de-8E04-777770A57C4A}":"Ils.MultiCadastre.Desktop.SaveChangesJobAreaCommand","{47E4889C-37A1-43b9-90BD-6A8778F6E7FD}":"Ils.MultiCadastre.Desktop.ComputationCreateMultiPointLine","{4F7520B2-C09A-4abd-843E-CDD9F241AE16}":"Ils.MultiCadastre.Desktop.AddUpinsCommand","{52D2FA3A-60EF-43bd-A7F9-61C247312080}":"Ils.MultiCadastre.Desktop.CreateJobAreaCommand","{5A08B750-BC9F-4918-A759-33596B3D3E5E}":"Ils.MultiCadastre.Desktop.clsConfigVisualization","{5F9BC173-CF00-4a3b-8766-E14FF7D462A8}":"Ils.MultiCadastre.Desktop.DeleteJobAreaCommand","{62313f05-7337-4e01-aced-dc6cc4f4dff5}":"Ils.MultiCadastre.Desktop.ViewRegionCommand","{74EB69D4-55BE-4305-8291-30557D603D0F}":"Ils.MultiCadastre.Desktop.ParcelSeparationCommand","{8004083A-6315-4f10-BBFA-79C0900C0230}":"Ils.MultiCadastre.Desktop.MCC_CreateMapPlotting.CreateSurveyPlanPlottingCommand","{93FE7956-0F01-464a-AE98-4ED11CED4F55}":"Ils.MultiCadastre.Desktop.LoadJobAreaCommand"};
const wf = { defs: null, filter: "open", jobType: "", q: "", jobs: [], total: 0, offset: 0, selected: null, detail: null, principals: [], busy: false,
  editing: null, // the job being worked on: { job_instance_id, cadastre_job_id, area, area_status, drafts, is_commit_task, ... }
};
const wfOpen = () => !$("wfBody").classList.contains("hidden");
function showWorkflow(on) {
  $("wfBody").classList.toggle("hidden", !on);
  if (on) { $("tasksBody").classList.add("hidden"); $("tocBody").classList.add("hidden"); document.querySelector("#tocPane .hscroll").classList.add("hidden"); }
  else if (!tasksOpen()) { $("tocBody").classList.remove("hidden"); document.querySelector("#tocPane .hscroll").classList.remove("hidden"); }
}
async function openWorkflow() {
  if ($("tocPane").classList.contains("hidden")) togglePane("tocPane");
  showWorkflow(true); buildTocTools();
  if (!wf.defs) {
    try { [wf.defs, wf.principals] = await Promise.all([api("/api/v1/wf/definitions"), api("/api/v1/wf/principals")]); }
    catch (e) { $("wfBody").innerHTML = ""; $("wfBody").append(h("p", { class: "muted", style: "padding:8px" }, "Could not load the workflow: " + e.message)); return; }
  }
  await wfLoadJobs(true);
}
// A step command as a readable tool name: "Ils.MultiCadastre.Desktop.CreateJobAreaCommand" -> "Create Job Area".
function wfToolName(cmd) {
  const cls = AUM_TOOL_GUIDS[cmd] || (cmd.startsWith("{") ? null : cmd);
  if (!cls) return "ArcMap tool " + cmd;
  const parts = cls.split(".");
  const last = parts[parts.length - 1].replace(/(Command|Tool|ToolControl|Control|Form)$/, "").replace(/^MC_?/, "");
  const words = last.replace(/([a-z0-9])([A-Z])/g, "$1 $2").replace(/([A-Z]+)([A-Z][a-z])/g, "$1 $2").replace(/_/g, " ").trim();
  const ns = cls.startsWith("esri") ? "ArcMap: " : "";
  return ns + words;
}
function wfMine() { const me = state.me && (state.me.username || "").toLowerCase(); return wf.principals.find((p) => p.is_user && me && (p.code.toLowerCase() === me || (p.os_account || "").toLowerCase().endsWith("\\" + me))); }
async function wfLoadJobs(reset) {
  if (reset) { wf.offset = 0; wf.jobs = []; }
  const qs = new URLSearchParams({ limit: "100", offset: String(wf.offset) });
  if (wf.filter === "all") qs.set("open", "false");
  if (wf.filter === "mine") { const me = wfMine(); if (me) qs.set("assigned", String(me.sid)); }
  if (wf.jobType) qs.set("job_type", wf.jobType);
  if (wf.q.trim()) { qs.set("q", wf.q.trim()); qs.set("open", "false"); }
  try {
    const r = await api("/api/v1/wf/jobs?" + qs);
    wf.total = r.total; wf.jobs = wf.jobs.concat(r.jobs); wf.offset = wf.jobs.length;
  } catch (e) { status("Could not load jobs: " + e.message); }
  renderWorkflow();
}
async function wfSelect(id) {
  wf.selected = id;
  try { wf.detail = await api("/api/v1/wf/jobs/" + id); } catch (e) { status("Could not open job " + id + ": " + e.message); wf.detail = null; }
  try { wf.selEditing = await api(`/api/v1/wf/jobs/${id}/editing`); } catch { wf.selEditing = null; }
  try { wf.files = await api(`/api/v1/wf/jobs/${id}/files`); } catch { wf.files = []; }
  // The job being worked on: its editing state (e.g. whether it is now at its commit task) follows the job.
  if (wf.editing && wf.editing.job_instance_id === id && wf.selEditing) { wf.editing = wf.selEditing; showJobArea(); }
  if (wf.editing && wf.editing.job_instance_id === id) { try { wf.mutations = await api(`/api/v1/wf/jobs/${id}/mutations`); } catch { wf.mutations = []; } }
  renderWorkflow();
}
const wfDate = (s) => (s ? s.replace("T", " ").slice(0, 16) : "—");
function wfStatusPill(j) {
  const cls = j.status === 3 ? "done" : j.status === 8 ? "hold" : j.is_open ? "open" : "ended";
  return h("span", { class: "wfPill " + cls, title: `Aumentum status code ${j.status}` }, j.status_label);
}
function renderWorkflow() {
  const body = $("wfBody"); if (!body) return;
  const scroll = body.querySelector(".wfList")?.scrollTop || 0;
  body.innerHTML = "";
  const types = (wf.defs?.job_types || []).filter((t) => t.active);
  const search = h("input", { type: "search", placeholder: "Job, name or Registry transaction (PLS…)", value: wf.q, style: "flex:1;min-width:0" });
  search.addEventListener("keydown", (e) => { if (e.key === "Enter") { wf.q = search.value; wfLoadJobs(true); } });
  body.append(h("div", { class: "tasksHead" },
    h("div", { class: "frow", style: "justify-content:space-between;flex-wrap:wrap" }, h("b", {}, "Workflow"),
      h("span", {}, h("button", { class: "btn", style: "min-width:0", onclick: wfNewJobDialog }, "New Job…"), " ",
        h("button", { class: "btn", style: "min-width:0", title: "Refresh", onclick: () => wfLoadJobs(true) }, "Refresh"))),
    h("div", { class: "frow" },
      h("select", { onchange: (e) => { wf.filter = e.target.value; wfLoadJobs(true); } },
        ...[["open", "Open jobs"], ["mine", "My jobs"], ["all", "All jobs"]].map(([v, l]) => h("option", { value: v, selected: wf.filter === v }, l))),
      h("select", { style: "flex:1;min-width:0", onchange: (e) => { wf.jobType = e.target.value; wfLoadJobs(true); } },
        h("option", { value: "" }, "All job types"), ...types.map((t) => h("option", { value: t.id, selected: String(t.id) === String(wf.jobType) }, t.name)))),
    h("div", { class: "frow" }, search, h("button", { class: "btn", style: "min-width:0", onclick: () => { wf.q = search.value; wfLoadJobs(true); } }, "Find"))));

  const list = h("div", { class: "wfList" });
  if (!wf.jobs.length) list.append(h("p", { class: "muted", style: "padding:6px" }, wf.defs ? "No jobs match." : "Loading…"));
  for (const j of wf.jobs) {
    const row = h("div", { class: "wfRow" + (j.id === wf.selected ? " sel" : ""), title: j.lrs_transaction_number ? "Registry transaction " + j.lrs_transaction_number : "" },
      h("div", { class: "wfRowTop" }, h("b", {}, j.name || "Job " + j.id), wfStatusPill(j)),
      h("div", { class: "muted" }, `#${j.id} · ${j.current_task || "—"} · ${j.assigned_to || "Unassigned"}`));
    row.onclick = () => wfSelect(j.id);
    list.append(row);
  }
  if (wf.jobs.length < wf.total) list.append(h("div", { class: "frow", style: "justify-content:center" },
    h("button", { class: "btn", onclick: () => wfLoadJobs(false) }, `Load more (${wf.jobs.length} of ${wf.total.toLocaleString()})`)));
  body.append(h("div", { class: "muted", style: "padding:0 6px" }, `${wf.total.toLocaleString()} job(s)`), list);
  list.scrollTop = scroll;
  if (wf.detail && wf.detail.job.id === wf.selected) body.append(wfDetailView(wf.detail));
}
function wfDetailView(d) {
  const j = d.job;
  const box = h("div", { class: "wfDetail" });
  const kv = (k, v) => h("div", { class: "wfKvRow" }, h("div", { class: "wfKvK" }, k), h("div", { class: "wfKvV" }, v ?? "—"));
  box.append(h("div", { class: "wfRowTop" }, h("b", {}, j.name || "Job " + j.id), wfStatusPill(j)),
    h("div", { class: "wfKv" },
      kv("Job", `#${j.id} · ${j.job_type}`), kv("Registry transaction", j.lrs_transaction_number), kv("Current task", j.current_task),
      kv("Assigned to", j.assigned_to || "Unassigned"), kv("Created", wfDate(j.created)), kv("Updated", wfDate(j.updated)),
      j.ended ? kv("Ended", wfDate(j.ended)) : null, j.comment ? kv("Comment", j.comment) : null));
  // The parcel the job is about: Aumentum names deed-plan jobs after the parcel's property number.
  const pn = (j.name || "").match(/PL\d+/i)?.[0]?.toUpperCase();
  const parcel = pn && state.parcels.features.find((f) => (f.properties.property_number || "").toUpperCase() === pn);
  if (parcel) box.append(h("div", { class: "frow" }, h("button", { class: "btn", onclick: () => { zoomToFC({ type: "FeatureCollection", features: [parcel] }); focusParcel(parcel); selectFeatures([parcel.properties.id]); } }, `Zoom To Parcel ${pn}`),
    h("span", { class: "muted" }, parcel.properties.parcel_number)));

  if (j.is_open && d.steps.length) {
    box.append(h("div", { class: "wfSub" }, `Steps of "${j.current_task}"`));
    const ol = h("ol", { class: "wfSteps" });
    for (const s of d.steps) {
      // Aumentum tools by name; ArcMap's own built-in tools (not in Aumentum's registry) as a count.
      const named = s.commands.filter((c) => AUM_TOOL_GUIDS[c] || !c.startsWith("{")).map(wfToolName).filter((v, i, a) => a.indexOf(v) === i);
      const builtIn = s.commands.filter((c) => c.startsWith("{") && !AUM_TOOL_GUIDS[c]).length;
      const tools = named.join(" · ") + (builtIn ? `${named.length ? " · " : ""}+ ${builtIn} ArcMap tool${builtIn > 1 ? "s" : ""}` : "");
      const act = wfStepAction(s);
      ol.append(h("li", {}, h("div", { class: "wfStepHead" }, h("b", {}, s.name),
          act.run ? h("button", { class: "btn wfStepBtn", title: act.tip, onclick: act.run }, act.label || "Open")
                  : h("span", { class: "muted wfStepNote", title: act.tip }, act.label)),
        h("div", { class: "muted" }, tools || "—")));
    }
    box.append(ol);
  }
  if (j.is_open) box.append(wfEditingView(j, wf.selEditing));
  if (j.is_open) {
    const comment = h("textarea", { rows: 2, placeholder: "Comment for this task (optional)", style: "width:100%;box-sizing:border-box" });
    const assign = h("select", { style: "flex:1;min-width:0" }, h("option", { value: "" }, "Default assignee"),
      ...wf.principals.filter((p) => p.is_user && !p.is_locked).map((p) => h("option", { value: p.sid }, p.name || p.code)));
    const choices = d.next_tasks.map((t) => h("label", { class: "wfChoice" }, h("input", { type: "radio", name: "wfNext", value: t.id, checked: d.next_tasks.length === 1 }), " ", t.name,
      d.history.some((x) => x.task_id === t.id) ? h("span", { class: "muted" }, " (back for rework)") : null));
    box.append(h("div", { class: "wfSub" }, "Finish this task"), comment);
    if (choices.length) box.append(h("div", { class: "muted" }, d.next_tasks.length > 1 ? "Send the job to:" : "Next task:"), ...choices,
      h("div", { class: "frow" }, h("label", {}, "Assign to:"), assign));
    const go = async (complete) => {
      const picked = box.querySelector('input[name="wfNext"]:checked');
      if (!complete && !picked) { status("Choose the task to send the job to."); return; }
      const bodyReq = { comment: comment.value || null };
      if (!complete) { bodyReq.to_task_id = Number(picked.value); if (assign.value) bodyReq.assign_sid = Number(assign.value); }
      await wfAction(`/api/v1/wf/jobs/${j.id}/advance`, bodyReq, complete ? "Job completed." : "Job moved on.");
    };
    box.append(h("div", { class: "frow", style: "flex-wrap:wrap" },
      choices.length ? h("button", { class: "btn", onclick: () => go(false) }, "Send To Task") : null,
      d.can_complete ? h("button", { class: "btn", onclick: () => go(true) }, "Complete Job") : null,
      h("button", { class: "btn", onclick: () => wfAction(`/api/v1/wf/jobs/${j.id}/hold`, { hold: j.status !== 8, comment: comment.value || null }, j.status === 8 ? "Job resumed." : "Job put on hold.") }, j.status === 8 ? "Resume" : "Hold"),
      h("button", { class: "btn", onclick: () => wfReassignDialog(j) }, "Reassign…"),
      h("button", { class: "btn", title: "End the job from its current task (administrators, or the Delete Job Instance role)", onclick: () => wfEndDialog(j) }, "End Job…")));
  }
  box.append(wfFilesView(j));
  box.append(h("div", { class: "wfSub" }, "History"));
  const t = h("table", { class: "wfHist" }, h("tr", {}, h("th", {}, "Task"), h("th", {}, "By"), h("th", {}, "Ended"), h("th", {}, "Comment")));
  for (const x of d.history) t.append(h("tr", { class: x.ended ? "" : "cur" }, h("td", {}, x.task), h("td", {}, x.assigned_to || "Unassigned"),
    h("td", {}, x.ended ? wfDate(x.ended) : "open"), h("td", {}, x.comment || "")));
  box.append(t);
  return box;
}
async function wfAction(url, body, done) {
  if (wf.busy) return; wf.busy = true;
  try {
    wf.detail = await api(url, { method: "POST", body });
    const i = wf.jobs.findIndex((x) => x.id === wf.detail.job.id); if (i >= 0) wf.jobs[i] = wf.detail.job;
    wf.selEditing = await api(`/api/v1/wf/jobs/${wf.detail.job.id}/editing`).catch(() => wf.selEditing);
    if (wf.editing && wf.editing.job_instance_id === wf.detail.job.id) wf.editing = wf.selEditing;
    status(done);
  } catch (e) { status(e.message); alert(e.message); }
  finally { wf.busy = false; renderWorkflow(); }
}
function wfEndDialog(j) {
  const outcome = h("select", { style: "width:100%" }, h("option", { value: "completed" }, "Completed"), h("option", { value: "cancelled" }, "Cancelled"));
  const comment = h("textarea", { rows: 2, style: "width:100%;box-sizing:border-box", placeholder: "Reason (recorded in the job's history)" });
  const w = floatWindow(`End ${j.name || "Job " + j.id}`, h("div", {}, h("p", {}, `Ends the job at "${j.current_task}". It cannot be reopened.`), outcome, comment,
    h("div", { class: "frow", style: "justify-content:flex-end" }, h("button", { class: "btn", onclick: async () => { w.remove(); await wfAction(`/api/v1/wf/jobs/${j.id}/end`, { outcome: outcome.value, comment: comment.value || null }, `Job ${outcome.value}.`); } }, "End Job"))), { id: "wfEndWin", width: 340 });
}
function wfReassignDialog(j) {
  const sel = h("select", { style: "width:100%" }, ...wf.principals.filter((p) => p.is_user && !p.is_locked).map((p) => h("option", { value: p.sid, selected: p.sid === j.assigned_sid }, p.name || p.code)));
  const w = floatWindow(`Reassign ${j.name || "Job " + j.id}`, h("div", {}, h("p", {}, `Current task: ${j.current_task}`), sel,
    h("div", { class: "frow", style: "justify-content:flex-end" }, h("button", { class: "btn", onclick: async () => { w.remove(); await wfAction(`/api/v1/wf/jobs/${j.id}/assign`, { sid: Number(sel.value) }, "Task reassigned."); } }, "Reassign"))), { id: "wfAssignWin", width: 320 });
}
function wfNewJobDialog() {
  const types = (wf.defs?.job_types || []).filter((t) => t.active && t.start_task_id);
  const type = h("select", { style: "flex:1" }, ...types.map((t) => h("option", { value: t.id, selected: t.id === 9 }, t.name)));
  const name = h("input", { style: "flex:1", placeholder: "e.g. Job-PL68955" });
  const lrs = h("input", { style: "flex:1", placeholder: "e.g. PLS000139391" });
  const comment = h("input", { style: "flex:1" });
  const w = floatWindow("New Job", h("div", {},
    h("div", { class: "frow" }, h("label", {}, "Job type:"), type), h("div", { class: "frow" }, h("label", {}, "Name:"), name),
    h("div", { class: "frow" }, h("label", {}, "Registry transaction:"), lrs), h("div", { class: "frow" }, h("label", {}, "Comment:"), comment),
    h("div", { class: "frow", style: "justify-content:flex-end" }, h("button", { class: "btn", onclick: async () => {
      try {
        const d = await api("/api/v1/wf/jobs", { method: "POST", body: { job_type_id: Number(type.value), name: name.value || null, lrs_transaction_number: lrs.value || null, comment: comment.value || null } });
        w.remove(); wf.filter = "open"; wf.q = ""; await wfLoadJobs(true); wf.selected = d.job.id; wf.detail = d; renderWorkflow(); status(`Job ${d.job.id} created on "${d.job.current_task}".`);
      } catch (e) { alert(e.message); }
    } }, "Create"))), { id: "wfNewWin", width: 380 });
}

// What turns a saved draft into a parcel with a UPIN, for the job being edited.
function draftNextStep() {
  return wf.editing && state.job && state.job.id === wf.editing.cadastre_job_id
    ? "Commit it at the job's Commit Parcel task to give it its UPIN."
    : "Post the job (once approved) to allocate a UPIN.";
}
// Refresh the worked-on job's editing state (draft count, area) after an edit.
async function wfRefreshEditing() {
  if (!wf.editing) return;
  try { wf.mutations = await api(`/api/v1/wf/jobs/${wf.editing.job_instance_id}/mutations`); } catch { wf.mutations = []; }
  try { wf.editing = await api(`/api/v1/wf/jobs/${wf.editing.job_instance_id}/editing`); if (wf.selected === wf.editing.job_instance_id) wf.selEditing = wf.editing; renderWorkflow(); } catch { /* keep the last known state */ }
}
// --- Editing under a job: job area, drafts, commit -------------------------------------------
function showJobArea() {
  const g = wf.editing?.area;
  map.getSource("job-area")?.setData(g ? { type: "FeatureCollection", features: [{ type: "Feature", geometry: g, properties: {} }] } : empty());
  buildTOC();
}
// Make the job the editor's current job: its draft parcels and job area load, and new drafts save
// under it (the server applies the job-area rules).
async function wfWorkOn(jobId) {
  try {
    wf.editing = await api(`/api/v1/wf/jobs/${jobId}/editing`, { method: "POST" });
    wf.selEditing = wf.editing;
    const job = await api(`/api/v1/jobs/${wf.editing.cadastre_job_id}`);
    if (!state.jobs.some((x) => x.id === job.id)) state.jobs.unshift(job);
    renderJobBox(); $("jobBox").value = job.id; await selectJob(job.id);
    showJobArea();
    if (wf.editing.area) zoomToFC({ type: "FeatureCollection", features: [{ type: "Feature", geometry: wf.editing.area, properties: {} }] });
    status(wf.editing.area ? `Working on ${wf.detail?.job?.name || "job " + jobId}: draw parcels inside its job area.` : `Working on ${wf.detail?.job?.name || "job " + jobId}: create its job area first (Create Job Area).`);
  } catch (e) { alert(e.message); }
  renderWorkflow();
}
async function wfEditAction(method, path, done, body) {
  const id = wf.editing?.job_instance_id; if (!id) return;
  try {
    const r = await api(`/api/v1/wf/jobs/${id}${path}`, { method, body });
    wf.editing = r.editing || r; wf.selEditing = wf.editing;
    if (r.parcel_numbers) { await loadParcels(false); status(`Committed ${r.parcel_numbers.length} parcel(s): ${r.parcel_numbers.join(", ")}.`); }
    else status(done);
    await loadDrafts(); showJobArea();
    wf.detail = await api("/api/v1/wf/jobs/" + id);
  } catch (e) { alert(e.message); }
  renderWorkflow();
}
// Commit Parcels: the officer types the beacon number of every corner, from the surveyor's point
// list (Aumentum: beacon numbers are entered at commit, never generated). Corners already on a
// fabric beacon keep its number. A point list (ID Easting Northing per line, as Trimble Business
// Center prints it) can be pasted to fill the numbers of the corners at those coordinates.
async function wfCommitDialog() {
  const id = wf.editing?.job_instance_id; if (!id) { status("Choose Work On This Job first."); return; }
  let pts;
  try { pts = await api(`/api/v1/wf/jobs/${id}/editing/corners`); } catch (e) { alert(e.message); return; }
  if (!pts.length) {
    const ret = (wf.mutations || []).filter((m) => m.kind === "retire");
    if (!ret.length) { alert("The job has no draft parcels to commit."); return; }
    if (!confirm(`Commit the retirement of ${ret.flatMap((m) => m.parents).join(", ")}?`)) return;
    try {
      const r = await api(`/api/v1/wf/jobs/${id}/commit`, { method: "POST", body: { beacons: [] } });
      wf.editing = r.editing; wf.selEditing = wf.editing; await Promise.all([loadParcels(false), loadDrafts()]); loadHistory();
      notify(`Retired: ${r.retired.join(", ")}.`); wf.detail = await api("/api/v1/wf/jobs/" + id); await wfRefreshEditing(); renderWorkflow();
    } catch (e) { notify("Commit failed: " + e.message, "err"); }
    return;
  }
  const src = () => map.getSource("corners");
  const inputs = new Map();
  let active = null, zoomOnFocus = false;
  const redraw = () => src()?.setData({ type: "FeatureCollection", features: pts.map((p) => {
    const v = p.beacon_number || inputs.get(p.point)?.value.trim() || "";
    return { type: "Feature", geometry: { type: "Point", coordinates: toLL(p.easting, p.northing) },
      properties: { label: v ? `${p.point}: ${v}` : String(p.point), done: !!v, active: p.point === active } };
  }) });
  const msg = h("div", { style: "margin:6px 0;min-height:1.2em" });
  const t = h("table", { class: "legs" }, h("tr", {}, h("th", {}, "Pt"), h("th", {}, "Easting"), h("th", {}, "Northing"), h("th", {}, "Beacon number")));
  for (const p of pts) {
    let cell;
    if (p.beacon_number) cell = h("span", { title: "This corner is on a beacon already in the fabric" }, `${p.beacon_number} (existing)`);
    else {
      cell = h("input", { placeholder: "e.g. CA6782", style: "width:110px", autocomplete: "off", value: pointIds.get(ptKey(p.easting, p.northing)) || "" });
      cell.addEventListener("input", redraw);
      cell.addEventListener("focus", () => { active = p.point; redraw(); if (zoomOnFocus) map.easeTo({ center: toLL(p.easting, p.northing), zoom: Math.max(map.getZoom(), 19) }); });
      cell.addEventListener("keydown", (ev) => { if (ev.key === "Enter") { const all = [...inputs.values()]; const i = all.indexOf(cell); (all[i + 1] || commitBtn).focus(); } });
      inputs.set(p.point, cell);
    }
    t.append(h("tr", {}, h("td", { class: "n" }, String(p.point)), h("td", {}, fmt3(p.easting)), h("td", {}, fmt3(p.northing)), h("td", {}, cell)));
  }
  const paste = h("textarea", { rows: 4, style: "width:100%;box-sizing:border-box;font-family:monospace", placeholder: "Optional: paste the point list (ID  Easting  Northing per line) to fill the numbers by coordinates" });
  const fill = h("button", { class: "btn", onclick: () => {
    let filled = 0; const unmatched = [];
    for (const line of paste.value.split(/\n/)) {
      const m = line.trim().match(/^([A-Za-z0-9][A-Za-z0-9/_.-]*)[\s,;|]+(-?\d+(?:\.\d+)?)[\s,;|]+(-?\d+(?:\.\d+)?)/);
      if (!m) continue;
      const [_, num, e, n] = m, E = parseFloat(e), N = parseFloat(n);
      const p = pts.find((q) => !q.beacon_number && Math.hypot(q.easting - E, q.northing - N) <= 0.05);
      if (p) { inputs.get(p.point).value = num; filled++; } else unmatched.push(num);
    }
    redraw();
    msg.style.color = unmatched.length ? "#b06000" : "#060";
    msg.textContent = `Filled ${filled} corner(s).` + (unmatched.length ? ` Not a corner of these parcels: ${unmatched.join(", ")}.` : "");
  } }, "Fill from point list");
  const commitBtn = h("button", { class: "btn", onclick: async () => {
    const beacons = pts.filter((p) => !p.beacon_number).map((p) => ({ easting: p.easting, northing: p.northing, number: inputs.get(p.point).value.trim() }));
    const blank = pts.filter((p) => !p.beacon_number && !inputs.get(p.point).value.trim()).map((p) => p.point);
    if (blank.length) { msg.style.color = "#b00"; msg.textContent = `Enter the beacon number of point(s) ${blank.join(", ")}.`; inputs.get(blank[0]).focus(); return; }
    commitBtn.disabled = true; msg.style.color = ""; msg.textContent = "Committing…";
    try {
      const r = await api(`/api/v1/wf/jobs/${id}/commit`, { method: "POST", body: { beacons } });
      w.remove(); src()?.setData(empty());
      wf.editing = r.editing; wf.selEditing = wf.editing;
      await Promise.all([loadParcels(false), loadBeacons(), loadDrafts()]); showJobArea();
      notify(`Committed ${r.parcel_numbers.length} parcel(s): ${r.parcel_numbers.join(", ")}; ${r.beacons_added.length} beacon(s) added.` + (r.retired?.length ? ` Replaced / retired: ${r.retired.join(", ")}.` : ""));
      loadHistory(); await wfRefreshEditing();
      wf.detail = await api("/api/v1/wf/jobs/" + id); renderWorkflow();
    } catch (e) { commitBtn.disabled = false; msg.style.color = "#b00"; msg.textContent = e.message; }
  } }, "Commit");
  const w = floatWindow("Commit Parcels — Beacon Numbers", h("div", {},
    h("p", { class: "muted", style: "margin:0 0 6px" }, `${pts.length} corner point(s), numbered on the map. Type each beacon number from the surveyor's point list.`),
    h("div", { style: "max-height:320px;overflow:auto" }, t), paste, h("div", { class: "frow" }, fill), msg,
    h("div", { class: "frow", style: "justify-content:flex-end" }, commitBtn, h("button", { class: "btn", onclick: () => { w.remove(); src()?.setData(empty()); } }, "Cancel"))),
    { id: "commitWin", width: 440 });
  redraw();
  zoomToFC({ type: "FeatureCollection", features: pts.map((p) => ({ type: "Feature", geometry: { type: "Point", coordinates: toLL(p.easting, p.northing) }, properties: {} })) }, { padding: 120, maxZoom: 19 });
  w.querySelector(".fx")?.addEventListener("click", () => src()?.setData(empty()));
  setTimeout(() => { inputs.values().next().value?.focus(); zoomOnFocus = true; }, 0);
}
function wfDrawJobArea() {
  if (!wf.editing) { status("Choose Work On This Job first."); return; }
  if (wf.editing.area_utm) { // start from the existing area so it can be adjusted
    edit.verts = wf.editing.area_utm.coordinates[0].slice(0, -1).map((c) => [c[0], c[1]]);
  }
  edit.template = "jobArea"; startEditing("sketch"); selectTemplate("jobArea"); openSketchProps();
  status("Job area: click its corners on the map, or type / paste their coordinates in Edit Sketch Properties; then Finish Sketch.");
}
function wfEditingView(j, e) {
  const sec = h("div", { class: "wfEditing" }, h("div", { class: "wfSub" }, "Editing"));
  const working = wf.editing && wf.editing.job_instance_id === j.id;
  if (!working) {
    sec.append(h("div", { class: "frow" }, h("button", { class: "btn", onclick: () => wfWorkOn(j.id) }, "Work On This Job"),
      h("span", { class: "muted" }, e?.drafts ? `${e.drafts} draft parcel(s)` : e?.area ? "Job area defined" : "")));
    return sec;
  }
  const ed = wf.editing;
  sec.append(h("div", { class: "muted" }, `Editing job ${ed.reference || ""} · ${ed.drafts} draft parcel(s) · job area: ${ed.area ? `${ed.area_status}, ${Math.round(ed.area_m2).toLocaleString()} m²` : "none"}`));
  sec.append(h("div", { class: "frow", style: "flex-wrap:wrap" },
    h("button", { class: "btn", onclick: wfDrawJobArea }, ed.area ? "Edit Job Area" : "Create Job Area"),
    ed.area ? h("button", { class: "btn", onclick: () => wfEditAction("POST", `/area/${ed.area_status === "closed" ? "reopen" : "close"}`, ed.area_status === "closed" ? "Job area reopened." : "Job area closed.") }, ed.area_status === "closed" ? "Reopen Job Area" : "Close Job Area") : null,
    ed.area ? h("button", { class: "btn", onclick: () => { if (confirm("Delete the job area?")) wfEditAction("DELETE", "/area", "Job area deleted."); } }, "Delete Job Area") : null,
    h("button", { class: "btn", disabled: !ed.drafts, onclick: () => { if (confirm(`Discard the job's ${ed.drafts} uncommitted parcel(s)?`)) wfEditAction("POST", "/discard", "Draft parcels discarded."); } }, "Discard Changes"),
    h("button", { class: "btn", onclick: async () => { await loadDrafts(); await wfSelect(j.id); status("Reloaded from the fabric."); } }, "Reload"),
    ed.is_commit_task ? h("button", { class: "btn", disabled: !ed.drafts && !(wf.mutations || []).length, title: "Give the job's draft parcels their UPINs and add them to the fabric", onclick: () => wfCommitDialog() }, "Commit Parcels…") : null));
  // Mutations recorded in the job (Phase 4), applied when the parcels are committed.
  if ((wf.mutations || []).length) {
    const ul = h("ul", { class: "wfMutations", style: "margin:4px 0 0;padding-left:18px" });
    for (const m of wf.mutations) {
      const what = m.kind === "retire" ? `Retire ${m.parents.join(", ")} (${m.reason || ""})` : `${MUTATION_LABEL[m.kind] || m.kind} ${m.parents.join(" + ")} → ${m.drafts.length} new parcel(s)`;
      const keep = m.kind === "separate" ? h("select", { title: "Separation Configuration: the part that keeps the parent's UPIN and attribute record", onchange: (e) => mutationKeepUpin(ed.job_instance_id, m, e.target.value) },
        h("option", { value: "" }, "every part gets a new UPIN"),
        ...m.drafts.map((d, i) => { const f = state.drafts.features.find((x) => String(x.properties.id) === String(d));
          return h("option", { value: d, selected: m.keeps_upin === d }, `part ${i + 1} keeps ${m.parents[0]}${f && f.properties.computed_area ? ` (${Number(f.properties.computed_area).toFixed(1)} m²)` : ""}`); })) : null;
      ul.append(h("li", {}, what, " ", keep, " ", h("button", { class: "btn", style: "min-width:0;padding:0 6px", title: "Cancel this mutation (its draft parcels are removed)", onclick: async () => {
        if (!confirm(`Cancel: ${what}?`)) return;
        try { await api(`/api/v1/wf/jobs/${ed.job_instance_id}/mutations/${m.group}`, { method: "DELETE" }); await afterMutation("Mutation cancelled."); } catch (e) { alert(e.message); }
      } }, "Cancel")));
    }
    sec.append(h("div", { class: "wfSub", style: "margin-top:6px" }, "Mutations (applied at commit)"), ul);
  }
  return sec;
}
// What a workflow step's "Open" does in this editor. Steps whose tools belong to later phases say so.
function wfStepAction(s) {
  const jobId = wf.detail?.job?.id;
  const needJob = (fn) => async () => { if (!wf.editing || wf.editing.job_instance_id !== jobId) await wfWorkOn(jobId); fn(); };
  const later = (phase, what) => ({ label: `${phase}`, tip: `${what}: arrives in ${phase} of the rollout.` });
  switch (s.id) {
    case 1: case 1004: return { run: () => { const a = wf.editing?.area; a ? zoomToFC({ type: "FeatureCollection", features: [{ type: "Feature", geometry: a, properties: {} }] }) : map.fitBounds([REGION_BBOX.slice(0, 2), REGION_BBOX.slice(2)]); status(`Region ${REGION} loaded.`); }, tip: "Zoom to the region (or the job area)" };
    case 2: case 1006: return { run: () => { if ($("catalogPane").classList.contains("hidden")) togglePane("catalogPane"); status("Drag the scanned survey plan from the Catalog onto the map to add it."); }, tip: "Add the survey plan image from the Catalog" };
    case 1005: return { run: needJob(wfDrawJobArea), label: "Draw", tip: "Create or edit the job area" };
    case 1007: return { run: needJob(() => openTask("survey")), tip: "Enter the survey (COGO traverse)" };
    // Aumentum's Construct Parcel / Create Parcel steps carry several tools: offer exactly those.
    case 1019: return { run: needJob(() => stepTools("Construct Parcel", [
      ["Digitize Parcel (Sketch)", () => { edit.template = "parcel"; startEditing("sketch"); selectTemplate("parcel"); }],
      ["Import Feature Class…", fciDialog], ["COGO Tools (survey entry)…", () => openTask("survey")], ["Computation Tools (35)…", openComputeTools]])), tip: "Construct the parcel: digitize, import a feature class, or COGO" };
    case 3: return { run: needJob(() => stepTools("Create Parcel", [
      ["Digitize Parcel (Sketch)", () => { edit.template = "parcel"; startEditing("sketch"); selectTemplate("parcel"); }],
      ["COGO Tools (survey entry)…", () => openTask("survey")], ["Create Connection Line…", connectionLineDialog], ["Create Multi Point Line…", multiPointLineDialog],
      ["Create Donut Parcel (sketch the hole, then click the parcel)", () => startPickDraft("donut")], ["Parcel Separation (sketch the line, then click the parcel)", () => startPickDraft("separate")],
      ["Move Point…", movePointDialog], ["Point Attributes…", pointAttributesDialog], ["List Control Points…", listControlPoints]])), tip: "Create the parcel with Aumentum's computation tools" };
    case 1008: return { run: () => validateFabric(), tip: "Validate the fabric (topology)" };
    case 1010: case 1011: case 1015: return { run: () => { setTool("identify"); status("Click a parcel to review its attributes."); }, tip: "Review parcel attributes" };
    case 1002: case 1012: return { run: needJob(() => wfCommitDialog()), label: "Commit", tip: "Commit the job's parcels to the fabric" };
    case 1014: return { run: () => findDialog(), tip: "Search the cadastre for parcels" };
    case 1003: case 1016: case 1017: case 1018: return { run: needJob(() => window.open(`/api/v1/jobs/${wf.editing.cadastre_job_id}/siteplan.pdf`, "_blank")), label: "Plan", tip: "Open the job's site plan (Title Deed Plan layout parity is Phase 6)" };
    case 1009: return { run: needJob(() => fciDialog()), label: "Import", tip: "Create parcels from a polygon feature class (shapefile)" };
    case 1013: return later("Phase 5", "Linking parcels to the Registry");
    case 1020: return { run: needJob(() => stepTools("Merge Parcel", [
      ["1. Select the parcels (Select tool; Shift-click to add)", () => { setTool("select"); status("Click the parcels to merge; Shift-click to add more."); }],
      ["2. Merge the selected parcels", mutationMerge], ["Lineage of the selected parcel…", () => lineageOfSelection()]])), label: "Open", tip: "Merge posted parcels into one (applied at commit)" };
    case 1021: return { run: needJob(() => stepTools("Split Parcel", [
      ["1. Select the parcel (Select tool)", () => { setTool("select"); status("Click the parcel to split."); }],
      ["2. Sketch the split line, then Finish Sketch", () => startSplitLine()],
      ["Separate the selected parcel along a line", () => startSplitLine("separate")],
      ["Re-survey (adjust) the selected parcel", mutationResurvey], ["Free mutation of the selected parcel", mutationFree], ["Retire the selected parcel…", mutationRetire], ["Lineage of the selected parcel…", () => lineageOfSelection()]])), label: "Open", tip: "Split a posted parcel along a line (applied at commit)" };
    default: return { label: "—", tip: "No editor action for this step" };
  }
}

// --- Job attachments ---------------------------------------------------------------------------
function wfFilesView(j) {
  const sec = h("div", {}, h("div", { class: "wfSub" }, `Attachments (${(wf.files || []).length})`));
  const kb = (n) => (n == null ? "" : n >= 1 << 20 ? (n / (1 << 20)).toFixed(1) + " MB" : Math.round(n / 1024) + " KB");
  const t = h("table", { class: "wfHist" });
  for (const f of wf.files || []) t.append(h("tr", {},
    h("td", {}, f.available ? h("a", { href: `/api/v1/wf/files/${f.id}`, target: "_blank" }, f.file_name)
                            : h("span", { title: "In Aumentum's document store, which this server cannot reach yet" }, f.file_name + " ⚠")),
    h("td", {}, kb(f.size_bytes)), h("td", {}, f.uploaded_by || ""), h("td", {}, wfDate(f.uploaded_at))));
  if ((wf.files || []).length) sec.append(t);
  const input = h("input", { type: "file", style: "max-width:100%" });
  input.onchange = async () => {
    const file = input.files[0]; if (!file) return;
    try {
      await authReady;
      const r = await fetch(`/api/v1/wf/jobs/${j.id}/files?name=${encodeURIComponent(file.name)}`, { method: "POST", body: file, headers: { "content-type": "application/octet-stream", ...authHeaders() } });
      if (!r.ok) throw new Error((await r.json().catch(() => ({}))).message || r.statusText);
      status(`${file.name} attached.`); await wfSelect(j.id);
    } catch (e) { alert("Attaching failed: " + e.message); }
  };
  sec.append(h("div", { class: "frow" }, h("label", {}, "Attach file:"), input));
  return sec;
}

// --- Productivity report and notifications ----------------------------------------------------
async function productivityWindow() {
  const to = new Date().toISOString().slice(0, 10), from = new Date(Date.now() - 30 * 864e5).toISOString().slice(0, 10);
  const f = h("input", { type: "date", value: from }), t = h("input", { type: "date", value: to });
  const out = h("div", { style: "max-height:420px;overflow:auto" });
  const run = async () => {
    out.innerHTML = "Loading…";
    try {
      const rows = await api(`/api/v1/wf/productivity?from=${f.value}&to=${t.value}`);
      const tb = h("table", { class: "wfHist" }, h("tr", {}, ...["User", "Tasks done", "Median h/task", "Open tasks", "Commits", "Parcels created", "Mutated", "Jobs created"].map((x) => h("th", {}, x))));
      for (const r of rows) tb.append(h("tr", {}, h("td", {}, r.user), h("td", {}, r.tasks_completed), h("td", {}, r.median_task_hours == null ? "—" : r.median_task_hours.toFixed(1)),
        h("td", {}, r.open_tasks), h("td", {}, r.commits), h("td", {}, r.parcels_created), h("td", {}, r.parcels_mutated), h("td", {}, r.jobs_created)));
      out.innerHTML = ""; out.append(rows.length ? tb : h("p", { class: "muted" }, "No activity in this period."));
    } catch (e) { out.textContent = e.message; }
  };
  floatWindow("Productivity Report", h("div", {}, h("div", { class: "frow" }, h("label", {}, "From:"), f, h("label", { style: "min-width:0" }, "To:"), t,
    h("button", { class: "btn", onclick: run }, "Run")), out), { id: "prodWin", width: 640 });
  run();
}
async function notificationsWindow() {
  const out = h("div", { style: "max-height:420px;overflow:auto" }, "Loading…");
  floatWindow("Task Notifications", out, { id: "notifWin", width: 620 });
  try {
    const rows = await api("/api/v1/wf/notifications");
    const tb = h("table", { class: "wfHist" }, h("tr", {}, ...["When", "To", "Subject", "Status"].map((x) => h("th", {}, x))));
    for (const r of rows) tb.append(h("tr", {}, h("td", {}, wfDate(r.created_at)), h("td", {}, `${r.to || ""}${r.to_email ? " <" + r.to_email + ">" : ""}`), h("td", {}, r.subject),
      h("td", { title: r.detail || "" }, r.status + (r.detail ? ` (${r.detail})` : ""))));
    out.innerHTML = ""; out.append(rows.length ? tb : h("p", { class: "muted" }, "No notifications yet."));
  } catch (e) { out.textContent = e.message; }
}

// --- Administration (Aumentum Web Administrator) ------------------------------------------------
const adm = { tab: "users", data: null, settings: [], sel: null };
async function adminWindow() {
  try { [adm.data, adm.settings] = await Promise.all([api("/api/v1/admin/security"), api("/api/v1/admin/settings")]); if (!wf.defs) wf.defs = await api("/api/v1/wf/definitions"); }
  catch (e) { alert(e.message); return; }
  floatWindow("Administration", h("div", { id: "admBody" }), { id: "admWin", width: 700 });
  renderAdmin();
}
async function admChange(body) {
  try { adm.data = await api("/api/v1/admin/change", { method: "POST", body }); renderAdmin(); status("Saved."); } catch (e) { alert(e.message); }
}
function renderAdmin() {
  const root = $("admBody"); if (!root) return; root.innerHTML = "";
  const d = adm.data, P = d.principals, name = (sid) => { const p = P.find((x) => x.sid === sid); return p ? p.name || p.code : "sid " + sid; };
  const tabs = [["users", "Users & Groups"], ["roles", "Roles"], ["tasks", "Task Permissions"], ["jobtypes", "Job Types"], ["settings", "Settings"]];
  root.append(h("div", { class: "frow", style: "flex-wrap:wrap" }, ...tabs.map(([k, l]) => h("button", { class: "btn" + (adm.tab === k ? " active" : ""), style: adm.tab === k ? "font-weight:bold" : "", onclick: () => { adm.tab = k; renderAdmin(); } }, l))));
  const body = h("div", { style: "max-height:460px;overflow:auto;margin-top:6px" }); root.append(body);
  const groups = P.filter((p) => !p.is_user);
  if (adm.tab === "users") {
    const tb = h("table", { class: "wfHist" }, h("tr", {}, h("th", {}, "User"), h("th", {}, "Windows account"), h("th", {}, "E-mail"), ...groups.map((g) => h("th", {}, g.name || g.code)), h("th", {}, "Locked")));
    for (const u of P.filter((p) => p.is_user)) {
      const email = h("input", { value: u.email || "", style: "width:150px", onchange: (e) => admChange({ kind: "principal", sid: u.sid, email: e.target.value }) });
      tb.append(h("tr", {}, h("td", {}, u.name || u.code), h("td", {}, u.os_account || ""), h("td", {}, email),
        ...groups.map((g) => { const on = d.memberships.some(([gs, ms]) => gs === g.sid && ms === u.sid);
          return h("td", {}, h("input", { type: "checkbox", checked: on, onchange: (e) => admChange({ kind: "member", group_sid: g.sid, member_sid: u.sid, add: e.target.checked }) })); }),
        h("td", {}, h("input", { type: "checkbox", checked: u.is_locked, onchange: (e) => admChange({ kind: "principal", sid: u.sid, is_locked: e.target.checked }) }))));
    }
    body.append(tb);
  } else if (adm.tab === "roles") {
    const sel = h("select", { onchange: (e) => { adm.sel = Number(e.target.value); renderAdmin(); } }, ...P.map((p) => h("option", { value: p.sid, selected: p.sid === adm.sel }, `${p.name || p.code}${p.is_user ? "" : " (group)"}`)));
    if (adm.sel == null) adm.sel = P[0]?.sid;
    body.append(h("div", { class: "frow" }, h("label", {}, "User or group:"), sel));
    const tb = h("table", { class: "wfHist" }, h("tr", {}, h("th", {}, "Role"), h("th", {}, "Description"), h("th", {}, "Granted")));
    for (const [code, desc] of d.roles) { const g = d.grants.find(([s, r]) => s === adm.sel && r === code);
      tb.append(h("tr", {}, h("td", {}, code), h("td", {}, desc || ""), h("td", {}, h("input", { type: "checkbox", checked: !!(g && g[2]), onchange: (e) => admChange({ kind: "grant", sid: adm.sel, role: code, add: e.target.checked, allow: true }) })))); }
    body.append(tb);
  } else if (adm.tab === "tasks") {
    const tb = h("table", { class: "wfHist" }, h("tr", {}, h("th", {}, "Task"), h("th", {}, "Permitted users and groups (★ default assignee)"), h("th", {}, "Add")));
    for (const t of wf.defs.tasks) {
      const perms = d.task_permissions.filter(([tid]) => tid === t.id);
      const add = h("select", {}, h("option", { value: "" }, "+ add…"), ...P.filter((p) => !perms.some(([, s]) => s === p.sid)).map((p) => h("option", { value: p.sid }, p.name || p.code)));
      add.onchange = () => add.value && admChange({ kind: "task_permission", task_id: t.id, sid: Number(add.value), add: true });
      tb.append(h("tr", {}, h("td", {}, t.name), h("td", {}, ...perms.map(([, sid, def]) => h("span", { class: "wfPill " + (def ? "done" : "ended"), style: "display:inline-block;margin:1px", title: "Click to remove; double-click to make default" ,
        onclick: () => confirm(`Remove ${name(sid)} from "${t.name}"?`) && admChange({ kind: "task_permission", task_id: t.id, sid, add: false }),
        ondblclick: (e) => { e.stopPropagation(); admChange({ kind: "task_permission", task_id: t.id, sid, add: true, is_default: !def }); } }, (def ? "★ " : "") + name(sid)))), h("td", {}, add)));
    }
    body.append(tb);
  } else if (adm.tab === "jobtypes") {
    const tb = h("table", { class: "wfHist" }, h("tr", {}, h("th", {}, "Job type"), h("th", {}, "Tasks"), h("th", {}, "Active")));
    const tn = (id) => wf.defs.tasks.find((t) => t.id === id)?.name || id;
    for (const jt of wf.defs.job_types) tb.append(h("tr", {}, h("td", {}, `${jt.name} (#${jt.id})`),
      h("td", {}, jt.flow.map((e) => `${tn(e.task_id)} → ${e.next_task_id ? tn(e.next_task_id) : "end"}`).join("; ") || "—"),
      h("td", {}, h("input", { type: "checkbox", checked: jt.active, onchange: async (e) => { await admChange({ kind: "job_type", id: jt.id, active: e.target.checked }); wf.defs = await api("/api/v1/wf/definitions"); renderAdmin(); } }))));
    body.append(tb);
  } else {
    const tb = h("table", { class: "wfHist" }, h("tr", {}, h("th", {}, "Setting"), h("th", {}, "Value"), h("th", {}, "Source")));
    for (const st of adm.settings) {
      const inp = h("input", { value: st.value ?? "", style: "width:260px", type: /pass|secret/i.test(st.key) ? "password" : "text" });
      inp.onchange = async () => { try { adm.settings = await api("/api/v1/admin/settings", { method: "PUT", body: { key: st.key, value: inp.value } }); status(`${st.key} saved.`); renderAdmin(); } catch (e) { alert(e.message); } };
      tb.append(h("tr", {}, h("td", {}, st.key), h("td", {}, inp), h("td", {}, st.source || "")));
    }
    const k = h("input", { placeholder: "e.g. mail.smtp_password", style: "width:220px" }), v = h("input", { style: "width:220px" });
    body.append(tb, h("div", { class: "frow" }, h("label", {}, "Add setting:"), k, v, h("button", { class: "btn", onclick: async () => { if (!k.value) return; adm.settings = await api("/api/v1/admin/settings", { method: "PUT", body: { key: k.value, value: v.value } }); renderAdmin(); } }, "Save")));
  }
}

/* =====================================================================================
 * Boot
 * ===================================================================================== */
async function refreshAll() { await Promise.all([loadParcels(false), loadBeacons(), loadJobs(), loadDrafts()]); buildTOC(); await catRefresh(cat.selected); status("Refreshed"); }
// Drag a raster (.tif) or a shapefile (.shp) from the Catalog onto the map or the Table Of Contents
// to add it as a layer. The Catalog row puts its node key on the drag as "text/cad-item".
const CAT_DRAG_TYPE = "text/cad-item";
["tocPane", "mapFrame"].forEach((pid) => {
  const el = $(pid); if (!el) return;
  el.addEventListener("dragover", (ev) => {
    if (!ev.dataTransfer.types.includes(CAT_DRAG_TYPE)) return;
    ev.preventDefault(); ev.dataTransfer.dropEffect = "copy"; el.classList.add("dropok");
  });
  el.addEventListener("dragleave", (ev) => { if (!el.contains(ev.relatedTarget)) el.classList.remove("dropok"); });
  el.addEventListener("drop", (ev) => {
    el.classList.remove("dropok");
    const key = ev.dataTransfer.getData(CAT_DRAG_TYPE);
    if (!key) return;
    ev.preventDefault();
    addCatalogLayer(cat.nodes.get(key));
  });
});

initialDeepLink = initialViewFromURL(); // apply /editor?center=E,N&scale=N before any data arrives
buildMenus(); buildToolbars(); buildTocTools(); buildTOC(); renderTask(); buildCatalog(); updateNavButtons(); updateScaleBox(); updateEditorButtons();
loadCatalogRoots();
loadCatalogDatabases();
