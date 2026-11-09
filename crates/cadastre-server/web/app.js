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
let tdDraw = null; // a title deed plan feature being drawn on the map (see tdStartDraw)
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
async function authLogout() {
  if (auth.cfg?.local) {
    try { sessionStorage.removeItem("cad.session"); sessionStorage.removeItem("cad.adminSwitch"); } catch { /* ignore */ }
    await fetch("/api/v1/auth/logout", { method: "POST" }).catch(() => {});
    location.assign("/login");
    return;
  }
  const id = auth.cfg?.enabled ? auth.cfg : null;
  try { sessionStorage.removeItem("cad.tokens"); sessionStorage.removeItem("cad.session"); } catch { /* ignore */ }
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
  setTimeout(() => { try { status(msg); } catch { /* status bar not built yet */ } uiAlert(msg); }, 0);
  throw e;
});
const authHeaders = () => (auth.token ? { authorization: `Bearer ${auth.token}` } : {});
async function api(path, opts = {}) {
  await authReady;
  const init = { ...opts, headers: { ...(opts.body ? { "content-type": "application/json" } : {}), ...authHeaders(), ...(opts.headers || {}) } };
  if (opts.body && typeof opts.body !== "string") init.body = JSON.stringify(opts.body);
  let r = await fetch(path, init);
  if (r.status === 401 && auth.cfg?.local) { location.assign("/login"); return new Promise(() => {}); }
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
// Date and time as PLAGIS reads them: 03/11/2026 16:47 (day first, 24-hour, this PC's time zone).
function fmtDT(iso) {
  if (!iso) return "";
  const d = new Date(String(iso).includes("T") || /[zZ+]/.test(iso) ? iso : String(iso).replace(" ", "T"));
  if (isNaN(d)) return String(iso);
  const p = (n) => String(n).padStart(2, "0");
  return `${p(d.getDate())}/${p(d.getMonth() + 1)}/${d.getFullYear()} ${p(d.getHours())}:${p(d.getMinutes())}`;
}
// A date-only value (yyyy-mm-dd, as the Registry's link date is kept) shown day first.
const fmtD = (s) => (/^\d{4}-\d{2}-\d{2}$/.test(String(s || "")) ? String(s).split("-").reverse().join("/") : fmtDT(s));
// Parcel metadata (committed when / by whom / in which job; linked when / to what), by UPIN.
async function parcelMeta({ upins = [], jobs = [] } = {}) {
  const q = new URLSearchParams(); if (upins.length) q.set("upins", upins.join(",")); if (jobs.length) q.set("jobs", jobs.join(","));
  if (!upins.length && !jobs.length) return [];
  try { return await api("/api/v1/parcels/metadata?" + q); } catch { return []; }
}
const metaCommitted = (m) => !m || !m.committed_at ? "—"
  : m.committed_in === "aumentum" ? `${fmtDT(m.committed_at)} (Aumentum)`
  : `${fmtDT(m.committed_at)}${m.committed_by ? " by " + m.committed_by : ""}`;
const metaLinked = (m) => !m ? "—" : m.linked ? `${m.linked_at ? fmtD(m.linked_at) : "yes"}${m.lrs_id ? " → " + m.lrs_id : ""}` : "Not linked";
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

/* =====================================================================================
 * Views per unit (no access control yet: a view only hides what a unit does not need; the
 * server's rules apply in every view). The view is remembered on this PC; ?view=… in the address
 * sets it too. Only the Sysadmin view shows the switch; a tab switched from Sysadmin keeps the
 * switch so the administrator can come back.
 * ===================================================================================== */
const VIEWS = {
  sysadmin: { label: "Sysadmin", path: "/sys-editor", all: true, panel: null },
  digitising: { label: "Digitising", path: "/digitising", features: ["digitise", "edit", "workflow", "registry", "tasks", "completed"], panel: "workflow" },
  titledeed: { label: "Title Deed", path: "/title-deed", features: ["titledeed", "workflow", "registry", "completed"], panel: "queue" },
  reporting: { label: "Reporting & Chat", path: "/reporting", features: ["completed", "reports"], panel: "completed" },
};
const VIEW_FOR = { digitise: "Digitising", edit: "Digitising", titledeed: "Title Deed", admin: "Sysadmin", reports: "Reporting & Chat" };
// The view comes from the address each unit is given: /digitising, /title-deed, /reporting, and
// /sys-editor for the system administrator.
const currentView = (() => {
  const path = location.pathname.replace(/\/+$/, "");
  return Object.keys(VIEWS).find((k) => VIEWS[k].path === path) || "digitising";
})();
const viewDef = VIEWS[currentView];
const can = (feature) => !!viewDef.all || (viewDef.features || []).includes(feature);
let adminTab = false; try { adminTab = currentView === "sysadmin" || sessionStorage.getItem("cad.adminSwitch") === "1"; } catch { /* storage */ }
function switchView(v) {
  if (!VIEWS[v]) return;
  try { sessionStorage.setItem("cad.adminSwitch", "1"); } catch { /* storage */ }
  try { saveSession(); } catch { /* not ready */ }
  location.href = VIEWS[v].path + location.search;
}
document.body.classList.add("view-" + currentView);
document.title = `PLAGIS CADASTRE Editor — ${viewDef.label}`;

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
const edit = { active: false, template: "parcel", geomType: "polygon", construction: "polygon", verts: [], sel: -1, snapping: true, editingId: null,
  cut: [], tracing: false, trace: null };
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
        ["concat", ["number-format", ["/", ["floor", ["+", ["*", ["to-number", ["coalesce", ["get", "parcel_area"], ["get", "area"]]], 100], 0.0000001]], 100], { "min-fraction-digits": 2, "max-fraction-digits": 2 }], " sq.m."], { "font-scale": 0.92 }],
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
  // The features drawn for the open title deed plan.
  map.addSource("tdfeat", { type: "geojson", data: empty() });
  map.addLayer({ id: "tdfeat-line", type: "line", source: "tdfeat", filter: ["==", ["geometry-type"], "LineString"],
    paint: { "line-color": "#00e5ff", "line-width": 2.5 } });
  map.addLayer({ id: "tdfeat-point", type: "circle", source: "tdfeat", filter: ["==", ["geometry-type"], "Point"],
    paint: { "circle-color": "#00e5ff", "circle-radius": 5, "circle-stroke-color": "#000", "circle-stroke-width": 1 } });

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
  await restoreSession();
  // A unit's view opens on its own panel (unless a refresh put another one back).
  const restored = savedSession?.panel && savedSession.panel !== "toc";
  if (!restored && viewDef.panel) {
    try {
      if (viewDef.panel === "workflow") await openWorkflow();
      else if (viewDef.panel === "queue") { done.source = "queue"; await openCompleted(); }
      else if (viewDef.panel === "completed") await openCompleted();
    } catch (e) { console.warn("view panel", e); }
  }
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
  document.title = `${viewDef.label} - ${APP}${who}`;
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
    { label: "Sign Out", action: () => authLogout(), disabled: () => !auth.cfg?.enabled && !auth.cfg?.local },
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
    can("admin") ? { label: "Administration…", action: () => adminWindow() } : null,
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
  const menus = MENUS();
  for (const name of Object.keys(menus)) {
    if (name === "Geoprocessing" && !can("edit")) continue;
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
  // The current view, and the switch (Sysadmin, or a tab switched from it).
  const vb = h("div", { class: "viewBadge" + (adminTab ? " canSwitch" : ""), title: adminTab ? "Switch to another view" : `This unit's view (${location.origin}${viewDef.path})` }, `View: ${viewDef.label}`, adminTab ? " ▾" : "");
  if (adminTab) vb.addEventListener("mousedown", (ev) => {
    ev.stopPropagation(); closeMenus();
    const r = vb.getBoundingClientRect();
    contextMenu([...Object.entries(VIEWS).map(([k, v]) => ({ label: `${k === currentView ? "✓ " : ""}${v.label}   ${v.path}`, action: () => switchView(k) })), "-",
      { label: "Copy the units' addresses", action: async () => {
        const t = Object.values(VIEWS).map((v) => `${v.label}: ${location.origin}${v.path}`).join("\n");
        try { await navigator.clipboard.writeText(t); notify("Addresses copied."); } catch { uiAlert(t, { title: "Addresses" }); }
      } }], r.left, r.bottom);
  });
  bar.append(vb);
  if (currentView === "sysadmin" && can("admin")) bar.append(h("div", { class: "viewBadge canSwitch", style: "margin-left:0", title: "Add a Registry user", onclick: settingsAddUser }, "Settings"));
}
function menuList(items) {
  const list = h("div", { class: "menuList" });
  for (const it of items) {
    if (it == null) continue;
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
    { kind: "btn", id: "edTrace", icon: "trace", tip: "Trace: follow existing boundaries while sketching", action: () => toggleTrace() },
    { kind: "tool", tool: "cut", icon: "cutPolygons", tip: "Cut Polygons: draw a line across a draft parcel to split it" },
    { kind: "sep" },
    { kind: "btn", id: "edCreate", icon: "createFeatures", tip: "Create Features window", action: () => openCreateFeatures() },
    { kind: "btn", id: "edProps", icon: "sketchProps", tip: "Edit Sketch Properties", action: () => openSketchProps() },
    { kind: "sep" },
    { kind: "btn", id: "edFinish", icon: "finishSketch", tip: "Finish Sketch (F2)", action: () => finishSketch() },
    { kind: "btn", id: "edDelete", icon: "deleteSketch", tip: "Delete Sketch", action: () => deleteSketch() },
    { kind: "btn", id: "edSnap", icon: "snapping", tip: "Snapping on/off", action: () => toggleSnapping() },
    { kind: "tool", tool: "attr", icon: "attributes", tip: "Attributes: click a draft parcel to edit its attributes" },
    { kind: "sep" },
    { kind: "btn", id: "edPost", icon: "jobPost", tip: "Post Parcel & Finish Job: commit the parcel, allocate its UPIN, complete the workflow", action: () => wfFinishCurrent() },
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
    if ((def.id === "tb-editor" || def.id === "tb-drawing") && !can("edit")) tb.classList.add("hidden");
    if (def.id === "tb-cadastre" && !can("digitise") && !can("titledeed")) tb.classList.add("hidden");
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
  catch (e) { status(`Could not read ${name}: ${e.message}`); uiAlert(`Could not read ${name}:\n${e.message}`); return; }
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
  catch (e) { status(`Could not read ${name}: ${e.message}`); uiAlert(`Could not read ${name}:\n${e.message}`); return; }
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
    const b = h("span", { class: `tb${state.tocView === v && !tasksOpen() && !wfOpen() && !regOpen() && !doneOpen() ? " active" : ""}`, title: tip, html: icon(ic) });
    b.onclick = () => { state.tocView = v; showCompleted(false); showRegistry(false); showWorkflow(false); showTasks(false); buildTocTools(); buildTOC(); };
    t.append(b);
  }
  t.append(h("span", { class: "tsep" }));
  if (can("tasks")) {
  const tasksBtn = h("span", { class: `tb${tasksOpen() && !wfOpen() && !regOpen() && !doneOpen() ? " active" : ""}`, title: "Tasks", html: icon("tasks") });
  tasksBtn.onclick = () => { showCompleted(false); showRegistry(false); showWorkflow(false); showTasks(true); buildTocTools(); };
  t.append(tasksBtn);
  }
  const wfBtn = h("span", { class: `tb${wfOpen() ? " active" : ""}`, title: "Workflow (job instances)", html: icon("jobOpen") });
  wfBtn.onclick = () => openWorkflow();
  if (can("workflow")) t.append(wfBtn);
  const doneBtn = h("span", { class: `tb${doneOpen() ? " active" : ""}`, title: "Completed jobs (digitised; plan, Registry, Correct)", html: icon("jobApprove") });
  doneBtn.onclick = () => openCompleted();
  if (can("completed")) t.append(doneBtn);
  // Registry links: their own tab, apart from the workflow.
  const waiting = registryWaiting();
  const regBtn = h("span", { class: `tb${regOpen() ? " active" : ""}`, title: "Registry — links from the Aumentum Registry" + (waiting ? ` (${waiting} waiting for a job)` : ""), html: icon("registry"), style: "position:relative" });
  if (waiting) regBtn.append(h("span", { class: "regBadge" }, String(waiting)));
  regBtn.onclick = () => openRegistry();
  if (can("registry")) t.append(regBtn);
  t.append(h("span", { class: "tb", title: "Options…", html: icon("options"), onclick: () => tocOptions() }));
}
const tasksOpen = () => !$("tasksBody").classList.contains("hidden");
function showTasks(on) {
  if (on) $("doneBody")?.classList.add("hidden");
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
  sketch: "crosshair", editVertex: "pointer", cut: "crosshair", attr: "pointer",
};
function setTool(tool) {
  if (tool !== "tdFeature" && tdDraw) tdEnd(null);
  // The sketch/edit tools require an active edit session, like ArcMap's "Start Editing".
  if ((tool === "sketch" || tool === "editVertex") && !edit.active) { startEditing(tool); return; }
  if (tool === "cut" && !edit.active) { startEditing("cut"); return; }
  if (state.tool === "cut" && tool !== "cut") clearCut();
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
  if (tdDraw) { tdEnd(null); return; }
  if (state.tool === "measure") clearMeasure();
  if (state.tool === "cut" && edit.cut.length) { clearCut(); status("Cut cancelled."); return; }
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
    case "tdFeature": tdClick(e.lngLat); break;
    case "survey": setSurveyStart(e.lngLat); break;
    case "text": addTextGraphic(e.lngLat); break;
    case "selectEl": selectGraphicAt(e); break;
    case "sketch": sketchClick(e); break;
    case "editVertex": if (!edit.verts.length) editDraftAt(pt); break;
    case "pickDraft": pickDraftAt(pt); break;
    case "cut": cutClick(e); break;
    case "attr": attrClick(e); break;
  }
});
// Edit tool on a saved draft parcel of the current job: its corners become the sketch, to move,
// retype (Edit Sketch Properties), add or delete; Finish Sketch saves the changed boundary.
function editDraftAt(pt) {
  const f = map.queryRenderedFeatures([[pt.x - 3, pt.y - 3], [pt.x + 3, pt.y + 3]], { layers: ["drafts-fill"].filter((l) => map.getLayer(l)) })[0];
  if (!f) { status("Edit tool: click one of the job's draft parcels to edit its corners."); return; }
  const d = state.drafts.features.find((x) => String(x.properties.id) === String(f.properties.id));
  if (!d) return;
  editDraftFeature(d);
}
function editDraftFeature(d) {
  if (!edit.active) startEditing("editVertex");
  const ring = d.geometry.type === "MultiPolygon" ? d.geometry.coordinates[0][0] : d.geometry.coordinates[0];
  selectTemplate("parcel");
  edit.verts = ring.slice(0, -1).map(([x, y]) => toUTM(x, y).map((v) => Math.round(v * 1000) / 1000));
  edit.editingId = d.properties.id; edit.sel = -1;
  drawSketch(); openSketchProps(); updateEditorButtons();
  status(`Editing draft parcel corners (${edit.verts.length}). Drag them, or retype them in Edit Sketch Properties; Finish Sketch saves.`);
}
map.on("dblclick", (e) => {
  if (state.tool === "tdFeature") { e.preventDefault(); tdFinish(); }
  else if (state.tool === "measure") { e.preventDefault(); finishMeasure(); }
  else if (state.tool === "sketch" && edit.verts.length) { e.preventDefault(); finishSketch(); }
  else if (state.tool === "cut" && edit.cut.length) { e.preventDefault(); finishCut(); }
  else if (parcelAt(e.point)) { e.preventDefault(); parcelMenu(e); }
});
// Absolute X, Y: right-click while sketching to key the next vertex by coordinate (Aumentum parity).
map.on("contextmenu", (e) => {
  if (edit.active && state.tool === "sketch") { e.preventDefault(); absoluteXYDialog(e.lngLat); }
});
map.on("contextmenu", (e) => {
  if (state.tool === "sketch") return;
  if (parcelAt(e.point)) { e.preventDefault(); parcelMenu(e); return; }
  const d = draftAt(e.point); if (d) { e.preventDefault(); draftMenu(d, e); }
});
function draftAt(point) {
  const layers = ["drafts-fill"].filter((l) => map.getLayer(l)); if (!layers.length) return null;
  const f = map.queryRenderedFeatures([[point.x - 3, point.y - 3], [point.x + 3, point.y + 3]], { layers })[0];
  return f ? state.drafts.features.find((x) => String(x.properties.id) === String(f.properties.id)) || null : null;
}
function parcelAt(point) {
  const layers = ["parcels-fill"].filter((l) => map.getLayer(l));
  const f = layers.length ? map.queryRenderedFeatures(point, { layers })[0] : null;
  return f && f.properties.parcel_number ? f : null;
}
// The parcel's menu: Start edit opens the title deed work on the parcel's job (Phase 5 flow:
// the Registry admin has linked the file; the title deed unit works on it and sends it back).
function parcelMenu(e) {
  const f = parcelAt(e.point); if (!f) return;
  const upin = f.properties.parcel_number;
  const ev = e.originalEvent;
  selectFeatures([f.properties.id], false);
  contextMenu([
    can("titledeed") ? { label: `Start edit — title deed (${upin})`, icon: "editor", action: () => startEditParcel(upin) } : null,
    can("titledeed") ? { label: "Correct Plan…", icon: "editor", action: () => correctPlan(upin) } : null,
    { label: "Zoom To Parcel", icon: "fullExtent", action: () => registryZoom(upin) },
    { label: "Registry File…", icon: "registry", action: () => startEditParcel(upin, true) },
    "-",
    beaconLabels.upin === upin ? { label: "Hide Beacons", action: () => hideBeacons() } : { label: "Show Beacons", action: () => showParcelBeacons(upin) },
    { label: "Point List…", action: () => pointListWindow(upin) },
    can("digitise") ? { label: "Correct & Recommit…", icon: "editor", action: () => correctAndRecommit(upin) } : null,
    { label: "History…", action: () => historyWindow({ upin, title: upin }) },
    "-",
    { label: "Parcel Lineage…", action: () => lineageOfSelection() },
  ], ev.clientX, ev.clientY);
}
// A draft parcel of the job: its beacons, its point list, or edit its corners.
function draftMenu(d, e) {
  const ev = e.originalEvent; const key = "draft:" + d.properties.id;
  contextMenu([
    can("digitise") ? { label: "Edit Corners (point list)", icon: "editor", action: () => editDraftFeature(d) } : null,
    beaconLabels.upin === key ? { label: "Hide Beacons", action: () => hideBeacons() } : { label: "Show Beacons", action: () => showDraftBeacons(d) },
    { label: "Point List…", action: () => pointListWindow(null, d) },
  ], ev.clientX, ev.clientY);
}

/* =====================================================================================
 * Beacons and the point list of a parcel (right-click): see each corner's beacon on the map,
 * open the point list (beacon, Easting, Northing, bearing and distance to the next corner), and
 * Correct & Recommit a parcel committed here (reopened as a draft keeping its UPIN, straight into
 * the sketch with its point list and the Orthophoto; then Finish → Verify → Check → Attributes →
 * Commit as usual).
 * ===================================================================================== */
const beaconLabels = { upin: null };
function setBeaconLabels(key, pts) {
  // Drawn as on the survey plan: red boundary, + at each corner with its beacon number, and along
  // each side the distance on one side of the line and the bearing (degrees and minutes) on the other.
  if (!map.getSource("beacon-labels")) {
    if (!map.hasImage("plan-plus")) map.addImage("plan-plus", crossImage("#c62828", 13));
    map.addSource("beacon-labels", { type: "geojson", data: empty() });
    map.addLayer({ id: "beacon-legs-line", type: "line", source: "beacon-labels", filter: ["==", "$type", "LineString"], paint: { "line-color": "#e53935", "line-width": 2 } });
    map.addLayer({ id: "beacon-labels-pt", type: "symbol", source: "beacon-labels", filter: ["==", "$type", "Point"],
      layout: { "icon-image": "plan-plus", "icon-allow-overlap": true,
        "text-field": ["get", "label"], "text-font": ["OpenSans-Regular"], "text-size": 13, "text-offset": [0.7, -0.7], "text-anchor": "bottom-left", "text-allow-overlap": true },
      paint: { "text-color": "#111", "text-halo-color": "rgba(255,255,255,.92)", "text-halo-width": 2 } });
    const along = (field, dy) => ({ type: "symbol", source: "beacon-labels", filter: ["==", "$type", "LineString"],
      layout: { "text-field": ["get", field], "text-font": ["OpenSans-Regular"], "text-size": 12, "symbol-placement": "line-center", "text-keep-upright": true,
        "text-offset": [0, dy], "text-allow-overlap": true, "text-ignore-placement": true },
      paint: { "text-color": "#111", "text-halo-color": "rgba(255,255,255,.92)", "text-halo-width": 2 } });
    map.addLayer({ id: "beacon-legs-dist", ...along("distance", -0.85) });
    map.addLayer({ id: "beacon-legs-brg", ...along("bearing", 0.85) });
  }
  // Bearing as the plan writes it: whole degrees and minutes (128° 01').
  const planBearing = (b) => {
    const m = String(b || "").match(/(\d+)°\s*(\d+)'\s*(\d+(?:\.\d+)?)?/);
    if (!m) return String(b || "");
    let d = +m[1], mi = +m[2] + Math.round((+(m[3] || 0)) / 60);
    if (mi >= 60) { mi -= 60; d = (d + 1) % 360; }
    return `${d}° ${String(mi).padStart(2, "0")}'`;
  };
  const corners = pts.map((p) => ({ type: "Feature", properties: { label: p.beacon }, geometry: { type: "Point", coordinates: toLL(p.easting, p.northing) } }));
  const legs = pts.map((p, i) => {
    const q = pts[(i + 1) % pts.length];
    return { type: "Feature", properties: { distance: `${Number(p.distance).toFixed(2)}m`, bearing: planBearing(p.bearing) },
      geometry: { type: "LineString", coordinates: [toLL(p.easting, p.northing), toLL(q.easting, q.northing)] } };
  });
  map.getSource("beacon-labels").setData({ type: "FeatureCollection", features: [...legs, ...corners] });
  beaconLabels.upin = key;
}
function hideBeacons() { map.getSource("beacon-labels")?.setData(empty()); beaconLabels.upin = null; status("Beacons hidden."); }
window.addEventListener("keydown", (e) => { if (e.key === "Escape" && beaconLabels.upin && !document.querySelector(".uiModalBack")) hideBeacons(); });
async function showParcelBeacons(upin) {
  let d; try { d = await api("/api/v1/parcels/points?upin=" + encodeURIComponent(upin)); } catch (e) { uiAlert(e.message); return; }
  setBeaconLabels(upin, d.points); status(`${upin}: ${d.points.length} beacons with bearings and distances (right-click → Hide Beacons, or Esc).`);
}
// A draft's corners: its own ring, named as Verify Points names them.
async function draftPoints(d) {
  const ring = (d.geometry.type === "MultiPolygon" ? d.geometry.coordinates[0][0] : d.geometry.coordinates[0]).slice(0, -1).map(([x, y]) => toUTM(x, y));
  let ids = []; try { ids = await identifyVertices(ring); } catch { /* names unknown */ }
  let k = 0;
  return ring.map(([e, n], i) => {
    const id = ids[i]; const nx = ring[(i + 1) % ring.length];
    const typed = pointIds.get(ptKey(e, n));
    const beacon = id?.source === "beacon" ? id.beacon_number : typed || `P${++k}`;
    const dist = Math.hypot(nx[0] - e, nx[1] - n);
    let az = Math.atan2(nx[0] - e, nx[1] - n) * 180 / Math.PI; if (az < 0) az += 360;
    const dd = Math.floor(az), mm = Math.floor((az - dd) * 60), ss = Math.round(((az - dd) * 60 - mm) * 60);
    return { beacon, easting: e, northing: n, distance: dist, bearing: `${dd}°${String(mm).padStart(2, "0")}'${String(ss).padStart(2, "0")}"` };
  });
}
async function showDraftBeacons(d) { const pts = await draftPoints(d); setBeaconLabels("draft:" + d.properties.id, pts); status(`Draft parcel: ${pts.length} corners shown (Esc hides them).`); }
async function pointListWindow(upin, draft) {
  let data;
  if (upin) { try { data = await api("/api/v1/parcels/points?upin=" + encodeURIComponent(upin)); } catch (e) { uiAlert(e.message); return; } }
  else data = { upin: null, points: await draftPoints(draft), area_m2: featureAreaM2(draft), can_correct: false };
  const pts = data.points;
  const tbl = h("table", { class: "verifyTbl" }, h("tr", {}, h("th", {}, "#"), h("th", {}, "Beacon"), h("th", {}, "Easting"), h("th", {}, "Northing"), h("th", {}, "Bearing"), h("th", {}, "Distance (m)")));
  pts.forEach((p, i) => {
    const tr = h("tr", { title: "Zoom to this corner" }, h("td", { class: "n" }, String(i + 1)), h("td", {}, h("b", {}, p.beacon)), h("td", {}, fmt3(p.easting)), h("td", {}, fmt3(p.northing)),
      h("td", {}, p.bearing), h("td", { class: "n" }, Number(p.distance).toFixed(2)));
    tr.onclick = () => { map.easeTo({ center: toLL(p.easting, p.northing), zoom: Math.max(map.getZoom(), 19) }); tbl.querySelectorAll("tr.selrow").forEach((r) => r.classList.remove("selrow")); tr.classList.add("selrow"); };
    tbl.append(tr);
  });
  const listText = pts.map((p) => `${p.beacon} ${fmt3(p.easting)} ${fmt3(p.northing)}`).join("\n");
  const title = upin ? `Point List — ${upin}` : "Point List — draft parcel";
  floatWindow(title, h("div", {},
    h("p", { class: "muted", style: "margin:0 0 4px" }, `${pts.length} corners · area ${area2(data.area_m2)} m²${data.job_name ? " · " + data.job_name : ""}${data.committed_at ? " · committed " + fmtDT(data.committed_at) : ""}`),
    h("div", { class: "verifyWrap" }, tbl),
    h("div", { class: "frow", style: "margin-top:6px;flex-wrap:wrap;gap:4px" },
      h("button", { class: "btn", title: "Beacon Easting Northing, one per line (the surveyor's format)", onclick: async (ev) => {
        try { await navigator.clipboard.writeText(listText); ev.target.textContent = "Copied"; } catch { status("Select the table to copy it."); } } }, "Copy list"),
      h("button", { class: "btn", onclick: () => upin ? showParcelBeacons(upin) : showDraftBeacons(draft) }, "Show Beacons"),
      upin ? h("button", { class: "btn btnPrimary", title: data.can_correct ? "Reopen as a draft (same UPIN), fix the points, commit again" : data.why_not, disabled: !data.can_correct,
        onclick: () => correctAndRecommit(upin) }, "Correct & Recommit…") : null,
      upin && !data.can_correct ? h("span", { class: "muted", style: "font-size:11px" }, `Correct here: no, ${data.why_not}.`) : null)),
    { id: "pointListWin", width: 560 });
}
async function correctAndRecommit(upin) {
  let d; try { d = await api("/api/v1/parcels/points?upin=" + encodeURIComponent(upin)); } catch (e) { uiAlert(e.message); return; }
  if (!d.can_correct) { await uiAlert(`${upin} cannot be corrected here: ${d.why_not}.`, { title: "Correct & Recommit", kind: "warn" }); return; }
  const aum = d.committed_in === "aumentum";
  const reason = await uiPrompt(aum
    ? `Correct ${upin}?\n\nIt was committed in Aumentum. Its file's job is opened here (the open one, or a new Job-<file number> on its Active Registry transaction) with ${upin} back as a draft that keeps its UPIN, and its points open for correction. Then Finish Sketch, check, save the attributes and commit again. The correction is in the file's history. Reason:`
    : `Correct ${upin} (${d.job_name || "job " + d.job_instance_id})?\n\nThe job is reopened with ${upin} back as a draft that keeps its UPIN, and its points open for correction. Then Finish Sketch, check, save the attributes and commit again. Reason:`, "", { title: "Correct & Recommit", ok: "Reopen and correct" });
  if (!reason || !reason.trim()) return;
  let r;
  try { r = await api(aum ? "/api/v1/wf/correct-aumentum" : `/api/v1/wf/jobs/${d.job_instance_id}/correct`, { method: "POST", body: { upin, reason: reason.trim() } }); }
  catch (e) { uiAlert("Correct refused: " + e.message); return; }
  if (aum) { d.job_instance_id = r.job_instance_id; notify(`${upin}: ${r.created ? "new job" : "the file's open job"} ${r.job_name} (${r.transaction_number}).`); }
  for (const b of r.beacons || []) rememberPointId(b.easting, b.northing, b.number);
  // Keep the numbers of beacons shared with neighbours too (they stay in the fabric and are matched again).
  for (const p of d.points) if (!/^P\d+$/.test(p.beacon)) rememberPointId(p.easting, p.northing, p.beacon);
  $("pointListWin")?.remove(); hideBeacons(); done.loaded = false;
  await wfResume(d.job_instance_id);
  await loadDrafts(); await loadParcels(false);
  const draft = state.drafts.features.find((x) => String(x.properties.id) === String(r.parcel_id));
  if (!draft) { notify(`${upin} is a draft again; open the job and use the Edit tool on it.`, "warn"); return; }
  editDraftFeature(draft); setTool("editVertex");
  orthoSketchChanged();
  notify(`${upin} is a draft again (it keeps its UPIN). Correct the points (drag, or retype in Edit Sketch Properties), then Finish Sketch.`);
}
// Correct a title deed plan that came back: the parcel, its UPIN and its link stay; the plan is
// redone (scale, form) and resent. The server allows it for a linked parcel at Raise Title Deed Plan.
async function correctPlan(upin, txn) {
  const reason = await uiPrompt(`Correct the title deed plan of ${upin}?\n\nThe parcel, its UPIN and its Registry link stay as they are. The plan form opens: change it, Attach as PDF, then Registry → Send plan resends it. The earlier plan is marked as replaced; nothing is deleted in the Registry.\n\nReason:`, "", { title: "Correct Plan", ok: "Correct plan", multiline: true });
  if (!reason || !reason.trim()) return;
  let r;
  try { r = await api("/api/v1/registry/correct-plan", { method: "POST", body: { upin, transaction_number: txn || null, reason: reason.trim() } }); }
  catch (e) { uiAlert("Correct plan refused: " + e.message); return; }
  await openWorkflow(); await wfSelect(r.job_instance_id);
  titleDeedDialog(r.job_instance_id, upin);
  notify(r.already_open ? `The plan of ${upin} is already being corrected: ${r.reason}` : `Correcting the plan of ${upin}. Attach as PDF, then Registry → Send plan.`);
}
async function startEditParcel(upin, fileOnly) {
  let w;
  try { w = await api(`/api/v1/registry/parcels/${encodeURIComponent(upin)}/work`); } catch (e) { uiAlert(e.message); return; }
  if (fileOnly) { if (w.transaction_number) regFind(w.transaction_number); else uiAlert(`${upin} has no Registry file (transaction) yet.`); return; }
  // Digitised but not linked yet: Aumentum's flow, the Registry admin links it at Raise Title
  // Deed Plan; then it is in the title deed queue. It is not digitised again.
  if (!w.linked && !w.job_instance_id) {
    uiAlert(`${upin} is digitised${w.property_number ? ` (file ${w.property_number})` : ""}, but not linked in the Registry yet.\nThe Registry admin links it when the file reaches Raise Title Deed Plan; it then shows in the title deed queue (Completed tab).`);
    return;
  }
  let job = w.job_instance_id;
  if (!job) {
    if (!w.transaction_number) { uiAlert(`${upin} is linked (${w.su_id}), but its Registry transaction is not known here. Find the file under Registry → Find file.`); return; }
    if (!(await uiConfirm(`No open job for ${w.transaction_number}. Start the title deed job (Job-${w.property_number || upin})?`))) return;
    try { job = (await api("/api/v1/registry/start-digitising", { method: "POST", body: { transaction_number: w.transaction_number, property_number: w.property_number, title_deed: true } })).job_instance_id; }
    catch (e) { uiAlert(e.message); return; }
  }
  await openWorkflow(); await wfSelect(job);
  try { await wfWorkOn(job); } catch (e) { notify(e.message, "err"); }
  stepTools(`Title Deed Plan — ${upin}`, [
    ["Title Deed Plan… (form, print, PDF)", () => titleDeedDialog(job, upin)],
    ["Send the plan to the Registry…", () => openRegistry({ job, view: "send" })],
  ]);
  status(`Working on ${w.job_name || "job " + job} for ${upin}${w.current_task ? " — task: " + w.current_task : ""}.`);
}

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
    ...[["Count", fs.length], ["Total area (m²)", area2(sum)], ["Mean area (m²)", fs.length ? area2(sum / fs.length) : "—"],
      ["Min area (m²)", fs.length ? area2(Math.min(...areas)) : "—"], ["Max area (m²)", fs.length ? area2(Math.max(...areas)) : "—"]]
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
    const tbl = h("table", {}, ...rows);
    body.append(h("h4", { style: "margin:8px 0 3px" }, names[f.layer.id]), tbl);
    if ((f.layer.id === "parcels-fill" || f.layer.id === "history-fill") && p.parcel_number) {
      parcelMeta({ upins: [p.parcel_number] }).then(([m]) => {
        if (!m) return;
        tbl.append(h("tr", {}, h("th", {}, "Committed"), h("td", {}, metaCommitted(m) + (m.job_name ? ` (${m.job_name})` : ""))),
          h("tr", {}, h("th", {}, "Linked"), h("td", {}, metaLinked(m))));
      });
    }
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
  const input = h("input", { style: "flex:1", placeholder: "UPIN, plot, property no. or beacon IDs, e.g. 5621, PL68755, CA6782", value: "" });
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
        const t = h("table", {}, h("tr", {}, h("th", {}, "Parcel"), h("th", {}, "Status"), h("th", {}, "Area (m²)"), h("th", {}, "Committed"), h("th", {}, "Linked")));
        const metaCells = new Map();
        for (const p of parcels.slice(0, 200)) {
          const area = p.computed_area ?? p.stated_area;
          const cc = h("td", { class: "muted" }, "…"), lc = h("td", { class: "muted" }, "…");
          if (p.parcel_number) metaCells.set(p.parcel_number, [cc, lc]); else { cc.textContent = "—"; lc.textContent = "—"; }
          const tr = h("tr", { style: "cursor:pointer", title: "Zoom to parcel" }, h("td", {}, p.parcel_number || "(unposted)"), h("td", {}, p.status), h("td", {}, area2(area)), cc, lc);
          tr.onclick = () => goToResult(p);
          t.append(tr);
        }
        parcelMeta({ upins: [...metaCells.keys()] }).then((list) => {
          for (const m of list) { const c = metaCells.get(m.upin); if (!c) continue; c[0].textContent = metaCommitted(m); c[1].textContent = metaLinked(m); c[0].classList.remove("muted"); c[1].classList.remove("muted"); }
          for (const [, c] of metaCells) for (const td of c) if (td.textContent === "…") td.textContent = "—";
        });
        results.append(h("div", { class: "muted", style: "margin-top:6px" }, `${parcels.length} parcel(s)${parcels.length > 200 ? " (first 200 shown)" : ""} — click a row to zoom`), t);
        if (!beacons.length) { if (parcels.length === 1) goToResult(parcels[0]); else selectFeatures(parcels.map((p) => p.id)); }
      }
      if (!beacons.length && !parcels.length) {
        results.append(h("div", {}, `Nothing matches "${q}".`), h("div", { class: "muted" }, "Parcels: try the plot number alone (e.g. 5621), part of the UPIN, or the property number (e.g. PL68755). Points: the beacon or control point ID."));
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
async function createBookmark() {
  const name = await uiPrompt("Bookmark Name:", `Bookmark ${state.bookmarks.length + 1}`); if (!name) return;
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
  floatWindow("Bookmarks Manager", h("div", {}, list, h("div", { class: "frow" }, h("button", { class: "btn", onclick: async () => { await createBookmark(); render(); } }, "New…"))), { width: 320 });
}

// floating windows
/* =====================================================================================
 * In-page dialogs: one modal in the editor's own style instead of the browser's alert / confirm /
 * prompt (which look foreign, cannot be styled, and on some browsers offer "prevent this page
 * from creating more dialogs"). Each returns a promise:
 *   await uiAlert("Saved.")                    -> undefined
 *   if (!(await uiConfirm("Delete it?"))) ...  -> true / false
 *   const v = await uiPrompt("Name:", "x")     -> the text, or null when cancelled
 * Enter = the main button, Esc = cancel; focus stays inside the dialog; errors offer "Copy details".
 * ===================================================================================== */
const UI_ICON = { info: "i", warn: "!", error: "✕", question: "?" };
function uiDialog({ title = APP, message = "", kind = "info", buttons = [{ label: "OK", value: true, primary: true, cancel: true }], input = null, details = null, width = 420 } = {}) {
  return new Promise((resolve) => {
    const before = document.activeElement;
    const text = typeof message === "string" ? message : "";
    const msg = typeof message === "string"
      ? h("div", { class: "uiMsg" }, ...message.split("\n").map((line) => h("p", {}, line)))
      : h("div", { class: "uiMsg" }, message);
    const inp = input ? h(input.multiline ? "textarea" : "input", { class: "uiInput", value: input.value ?? "", placeholder: input.placeholder || null, rows: input.multiline ? 4 : null }) : null;
    if (inp && input.multiline) inp.value = input.value ?? "";
    const back = h("div", { class: "uiModalBack" });
    let done = false;
    const finish = (v) => {
      if (done) return; done = true;
      document.removeEventListener("keydown", onKey, true);
      back.remove();
      try { before?.focus?.(); } catch { /* element gone */ }
      resolve(v === uiDialog.INPUT ? inp.value : v);
    };
    const btns = buttons.map((b) => h("button", { class: `btn${b.primary ? " btnPrimary" : ""}`, onclick: () => finish(b.value) }, b.label));
    const copyText = [title, text, details || ""].filter(Boolean).join("\n");
    const copy = (kind === "error" || details)
      ? h("button", { class: "btn uiCopy", title: "Copy the message, to send to support", onclick: async (ev) => {
          try { await navigator.clipboard.writeText(copyText); ev.target.textContent = "Copied"; } catch { ev.target.textContent = "Select and copy the text"; }
        } }, "Copy details")
      : null;
    const box = h("div", { class: `uiModal ${kind}`, role: kind === "error" || kind === "warn" ? "alertdialog" : "dialog", "aria-modal": "true", "aria-label": title, style: `width:min(${width}px, calc(100vw - 32px))` },
      h("div", { class: "uiHead" }, h("span", {}, title), h("span", { class: "fx", title: "Close", onclick: () => finish(cancelValue()) }, "✕")),
      h("div", { class: "uiBody" }, h("span", { class: `uiIcon ${kind}`, "aria-hidden": "true" }, UI_ICON[kind] || "i"),
        h("div", { class: "uiMain" }, msg, details ? h("pre", { class: "uiDetails" }, details) : null, inp)),
      h("div", { class: "uiButtons" }, copy, ...btns));
    back.append(box);
    function cancelValue() { const c = buttons.find((b) => b.cancel); return c ? c.value : undefined; }
    function onKey(ev) {
      if (ev.key === "Escape") { ev.preventDefault(); ev.stopPropagation(); finish(cancelValue()); return; }
      if (ev.key === "Enter" && !(ev.target && ev.target.tagName === "TEXTAREA") && !(ev.target && ev.target.tagName === "BUTTON")) {
        ev.preventDefault(); ev.stopPropagation();
        const p = buttons.find((b) => b.primary) || buttons[0]; finish(p.value); return;
      }
      if (ev.key === "Tab") { // keep focus inside the dialog
        const f = [...box.querySelectorAll("input,textarea,button")].filter((x) => !x.disabled);
        if (!f.length) return;
        const i = f.indexOf(document.activeElement);
        if (ev.shiftKey && i <= 0) { ev.preventDefault(); f[f.length - 1].focus(); }
        else if (!ev.shiftKey && i === f.length - 1) { ev.preventDefault(); f[0].focus(); }
      }
      ev.stopPropagation(); // the map's shortcuts stay quiet while a dialog is open
    }
    document.addEventListener("keydown", onKey, true);
    document.body.append(back);
    setTimeout(() => { if (inp) { inp.focus(); inp.select?.(); } else (btns.find((_, i) => buttons[i].primary) || btns[0])?.focus(); }, 0);
  });
}
uiDialog.INPUT = Symbol("input");
// An alert's look follows its words: failures read as errors, the rest as information.
const uiKind = (m) => (/\b(fail|failed|error|could not|cannot|can't|not saved|refused|invalid)\b/i.test(String(m)) ? "error" : "info");
function uiAlert(message, opts = {}) {
  return uiDialog({ title: opts.title || APP, message: String(message ?? ""), kind: opts.kind || uiKind(message), details: opts.details || null,
    buttons: [{ label: "OK", value: undefined, primary: true, cancel: true }] });
}
function uiConfirm(message, opts = {}) {
  return uiDialog({ title: opts.title || APP, message: String(message ?? ""), kind: opts.kind || "question",
    buttons: [{ label: opts.ok || "OK", value: true, primary: true }, { label: opts.cancel || "Cancel", value: false, cancel: true }] });
}
function uiPrompt(message, value = "", opts = {}) {
  return uiDialog({ title: opts.title || APP, message: String(message ?? ""), kind: "question", input: { value, placeholder: opts.placeholder, multiline: opts.multiline },
    buttons: [{ label: opts.ok || "OK", value: uiDialog.INPUT, primary: true }, { label: "Cancel", value: null, cancel: true }] });
}
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
  } catch (e) { uiAlert(`${action} failed: ${e.message}`); }
}
async function postJob() {
  if (!state.job) return;
  if (state.job.status !== POSTABLE_STAGE) { uiAlert(`Only ${POSTABLE_STAGE} jobs can be posted (this job is ${state.job.status}).`); return; }
  try {
    const res = await api(`/api/v1/jobs/${state.job.id}/post`, { method: "POST" });
    status(`Posted ${res.posted} parcel(s): ${res.parcel_numbers.join(", ")}`);
    await Promise.all([loadDrafts(), loadParcels(false)]); renderTask();
    floatWindow("Post Job", h("div", {}, h("p", {}, `Posted ${res.posted} parcel(s).`), h("div", {}, res.parcel_numbers.join(", ") || "—")), { width: 300 });
  } catch (e) { uiAlert("Post failed: " + e.message); }
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
      } catch (e) { uiAlert("Create job failed: " + e.message); }
    } }, "OK"), h("button", { class: "btn", onclick: () => w.remove() }, "Cancel"))), { id: "newJobWin", width: 330 });
}
async function validateFabric() {
  const fs = (state.selection.size ? selectedFC() : state.drafts).features;
  if (!fs.length) { uiAlert("Select parcels, or choose a Job Area with draft parcels, to validate."); return; }
  const rings = fs.map((f) => {
    const g = f.geometry; const ring = (g.type === "MultiPolygon" ? g.coordinates[0][0] : g.coordinates[0]);
    return ring.slice(0, -1).map(([x, y]) => { const [E, N] = toUTM(x, y); return { x: E, y: N }; });
  });
  try {
    const r = await api("/api/v1/fabric/validate", { method: "POST", body: { parcels: rings } });
    floatWindow("Validate Fabric", h("pre", { style: "white-space:pre-wrap;margin:0" }, JSON.stringify(r, null, 2)), { width: 420 });
  } catch (e) { uiAlert("Validation failed: " + e.message); }
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
  } catch (e) { uiAlert("Save failed: " + e.message); }
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
    { label: "Cut Polygons Tool", icon: "cutPolygons", action: () => setTool("cut") },
    { label: "Attributes", icon: "attributes", action: () => setTool("attr") },
    { label: "Absolute X, Y…", icon: "goToXY", action: () => { if (!edit.active) startEditing("sketch"); absoluteXYDialog(map.getCenter()); } },
    { label: "Trace", icon: "trace", checked: () => edit.tracing, action: toggleTrace },
    { label: "Snapping", icon: "snapping", checked: () => edit.snapping, action: toggleSnapping },
    "-",
    { label: "Post Parcel & Finish Job", icon: "jobPost", action: () => wfFinishCurrent() },
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
async function stopEditing() {
  if (edit.verts.length && !(await uiConfirm("You have an unfinished sketch. Discard it and stop editing?", { ok: "Discard", kind: "warn" }))) return;
  edit.active = false; clearSketch(); orthoClose();
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
  // Trace: if the click lands on an existing boundary, follow it instead of free-digitizing.
  if (edit.tracing && edit.geomType !== "point" && traceClick(pt)) { refreshSketchProps(); updateEditorButtons(); return; }
  let [E, N] = toUTM(e.lngLat.lng, e.lngLat.lat);
  if (edit.snapping) { const s = snapUTM(pt); if (s) [E, N] = s; }
  if (edit.geomType === "point") { edit.verts = [[E, N]]; edit.sel = 0; drawSketch(); finishSketch(); return; }
  edit.trace = null; // a free click breaks the trace chain
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
  orthoSketchChanged();
}
function clearSketch() { edit.verts = []; edit.sel = -1; edit.editingId = null; edit.splitMode = false; edit.trace = null; map.getSource("sketch")?.setData(empty()); orthoSketchChanged(); }
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
  if (!wf.editing) { uiAlert("Open a job in the Workflow tab and choose Work On This Job before drawing its job area."); return; }
  const utm = edit.verts.slice();
  try {
    wf.editing = await api(`/api/v1/wf/jobs/${wf.editing.job_instance_id}/area`, { method: "PUT", body: { geometry: { type: "Polygon", coordinates: [[...utm, utm[0]]] } } });
    showJobArea(); clearSketch(); refreshSketchProps(); updateEditorButtons(); zoomToUTMRing(utm);
    selectTemplate("parcel"); notify(`Job area saved (${Math.round(wf.editing.area_m2 || 0).toLocaleString()} m²). Parcels for this job must be drawn inside it.`);
    renderWorkflow();
  } catch (e) { uiAlert("Saving the job area failed: " + e.message); }
}
// Digitize Parcel (Construct Parcel / Create Parcel steps). The points typed or pasted for the job
// area are the parcel's corners from the survey plan, so they are not asked for again: an empty
// sketch starts from the job area's points (beacon numbers that came with them are kept by
// coordinates), and the digitiser checks them and presses Finish Sketch. Not when the job already
// has a draft parcel, nor from the box the editor makes itself when no job area was drawn.
function isAutoJobArea(g) {
  const ring = g?.coordinates?.[0] || [];
  const pts = ring.slice(0, -1);
  if (pts.length !== 4) return false;
  return pts.every((p, i) => { const q = pts[(i + 1) % 4]; return Math.abs(p[0] - q[0]) < 0.001 || Math.abs(p[1] - q[1]) < 0.001; });
}
function digitizeParcel() {
  selectTemplate("parcel");
  if (!edit.active) startEditing("sketch"); else setTool("sketch");
  if (wf.editing) setTimeout(() => { if (!$("orthoWin")) orthoOpen(); else orthoSketchChanged(); }, 0);
  const area = wf.editing?.area_utm;
  if (edit.verts.length || !area || (wf.editing?.drafts || 0) > 0) {
    if ((wf.editing?.drafts || 0) > 0) status("The job already has a draft parcel: click it with the Edit tool to change it, or sketch another.");
    return;
  }
  if (isAutoJobArea(area)) { status("Click the parcel's corners on the map (the job area was made automatically, so there are no plan points to load)."); return; }
  const ring = (area.coordinates?.[0] || []).slice(0, -1).map((c) => [c[0], c[1]]);
  if (ring.length < 3) return;
  edit.verts = ring; edit.sel = ring.length - 1; edit.editingId = null;
  afterVertexChange(); showSketchExtent(true);
  const named = ring.filter(([e, n]) => pointIds.get(ptKey(e, n))).length;
  notify(`Loaded the ${ring.length} points you entered for the job area${named ? ` (${named} with beacon numbers)` : ""}. Check them, then Finish Sketch.`);
}
/* =====================================================================================
 * Orthophoto window: the actual view of the plot being digitised (as the old Cadastre's
 * "autophoto"). A close-up on the satellite imagery with the sketch (or job area) outline, a
 * crosshair whose position is shown in Lat/Long, UTM 32N and MGRS, neighbouring parcels, overlaps
 * shaded red, and a badge: plot free / overlaps / title exists (this Cadastre's fabric, at once;
 * the System Check adds Aumentum and the Registry). A viewer only: nothing is saved or attached.
 * ===================================================================================== */
// MGRS (UTM zone 32, Plateau State), 1 m: e.g. 32PMR 93285 90339.
function mgrs(lng, lat) {
  const [E, N] = toUTM(lng, lat);
  const zone = 32, set = zone % 6 || 6;
  const band = "CDEFGHJKLMNPQRSTUVWX"[Math.max(0, Math.min(19, Math.floor((lat + 80) / 8)))];
  const col = ["ABCDEFGH", "JKLMNPQR", "STUVWXYZ"][(set - 1) % 3][Math.floor(E / 100000) - 1] || "?";
  const rows = "ABCDEFGHJKLMNPQRSTUV";
  const row = rows[(Math.floor(N / 100000) + (set % 2 === 0 ? 5 : 0)) % 20];
  const p5 = (v) => String(Math.round(v % 100000) % 100000).padStart(5, "0");
  return `${zone}${band}${col}${row} ${p5(E)} ${p5(N)}`;
}
const ortho = { map: null, timer: null, seq: 0, fitted: "", status: null };
function orthoOpen() {
  ortho.map?.remove(); ortho.map = null; ortho.fitted = "";
  const box = h("div", { class: "orthoMap" });
  const cross = h("div", { class: "orthoCross" });
  const badge = h("div", { class: "orthoBadge muted" }, "Checking the land…");
  const coords = h("table", { class: "orthoCoords" });
  const finishBtn = h("button", { class: "btn btnPrimary", onclick: () => finishSketch() }, "Finish Sketch");
  const body = h("div", {}, h("div", { class: "orthoWrap" }, box, cross), badge, coords,
    h("div", { class: "frow", style: "flex-wrap:wrap;gap:4px" },
      h("button", { class: "btn", title: "Copy the crosshair's coordinates", onclick: async (ev) => {
        try { await navigator.clipboard.writeText(coords.innerText.replace(/\t/g, " ")); ev.target.textContent = "Copied"; setTimeout(() => (ev.target.textContent = "Copy"), 1500); }
        catch { status("Select the coordinates and copy them."); } } }, "Copy"),
      h("button", { class: "btn", title: "Google's satellite view of this exact spot (new tab)", onclick: () => {
        const c = ortho.map?.getCenter(); if (!c) return;
        window.open(`https://www.google.com/maps/@?api=1&map_action=map&center=${c.lat.toFixed(7)},${c.lng.toFixed(7)}&zoom=20&basemap=satellite`, "_blank", "noopener");
      } }, "Open in Google Maps"),
      h("button", { class: "btn", onclick: () => { ortho.fitted = ""; orthoUpdate(); } }, "Centre on plot"),
      finishBtn));
  floatWindow("Orthophoto — actual view of the plot", body, { id: "orthoWin", width: 460 });
  const w = $("orthoWin"); const r = $("mapFrame").getBoundingClientRect();
  w?.querySelector(".fhead .fx")?.addEventListener("click", () => { ortho.map?.remove(); ortho.map = null; ortho.dismissed = true; });
  if (w) { w.style.left = Math.max(r.left + 8, r.right - 470) + "px"; w.style.top = (r.top + 8) + "px"; }
  const m = new maplibregl.Map({
    container: box, attributionControl: { compact: true }, maxZoom: 21, boxZoom: false, doubleClickZoom: true,
    style: { version: 8, glyphs: "/editor/vendor/fonts/{fontstack}/{range}.pbf",
      sources: { imagery: { type: "raster", tiles: [IMAGERY], tileSize: 256, maxzoom: 18, attribution: "Imagery © Esri" } },
      layers: [{ id: "imagery", type: "raster", source: "imagery" }] },
    center: map.getCenter(), zoom: Math.max(map.getZoom(), 17),
  });
  ortho.map = m; ortho.coords = coords; ortho.badge = badge; ortho.finishBtn = finishBtn;
  m.on("load", () => {
    m.addSource("nb", { type: "geojson", data: state.parcels || empty() });
    m.addLayer({ id: "nb-line", type: "line", source: "nb", paint: { "line-color": "#ffd23f", "line-width": 1.4 } });
    m.addSource("area", { type: "geojson", data: empty() });
    m.addLayer({ id: "area-line", type: "line", source: "area", paint: { "line-color": "#ffffff", "line-width": 1.5, "line-dasharray": [3, 2] } });
    m.addSource("ov", { type: "geojson", data: empty() });
    m.addLayer({ id: "ov-fill", type: "fill", source: "ov", paint: { "fill-color": "#ff1f1f", "fill-opacity": 0.45 } });
    m.addSource("sk", { type: "geojson", data: empty() });
    m.addLayer({ id: "sk-line", type: "line", source: "sk", paint: { "line-color": "#00e5ff", "line-width": 2.5 } });
    m.addLayer({ id: "sk-pt", type: "circle", source: "sk", filter: ["==", "$type", "Point"], paint: { "circle-radius": 3.5, "circle-color": "#00e5ff", "circle-stroke-color": "#003", "circle-stroke-width": 1 } });
    orthoUpdate();
  });
  m.on("move", orthoCoords);
  orthoCoords();
}
function orthoCoords() {
  const m = ortho.map; if (!m || !ortho.coords) return;
  const c = m.getCenter(); const [E, N] = toUTM(c.lng, c.lat);
  ortho.coords.innerHTML = "";
  ortho.coords.append(
    h("tr", {}, h("th", {}, "Lat Long"), h("td", {}, `${c.lat.toFixed(7)}  ${c.lng.toFixed(7)}`)),
    h("tr", {}, h("th", {}, "UTM 32N"), h("td", {}, `${E.toFixed(3)}E  ${N.toFixed(3)}N`)),
    h("tr", {}, h("th", {}, "MGRS"), h("td", {}, mgrs(c.lng, c.lat))));
}
// The outline to show and check: the sketch, else the job area.
function orthoRing() {
  if (edit.verts.length >= 3 && edit.geomType === "polygon") return edit.verts.map((v) => v.slice());
  const a = wf.editing?.area_utm?.coordinates?.[0];
  return a && a.length >= 4 ? a.slice(0, -1).map((c) => [c[0], c[1]]) : null;
}
function orthoClose() { ortho.map?.remove(); ortho.map = null; $("orthoWin")?.remove(); ortho.dismissed = false; }
function orthoSketchChanged() {
  // It appears by itself while a parcel is being sketched (however the sketch was started), unless
  // the digitiser closed it for this sketch.
  if (!$("orthoWin") && edit.active && edit.template === "parcel" && edit.geomType === "polygon" && edit.verts.length && !ortho.dismissed) {
    setTimeout(() => { if (!$("orthoWin")) orthoOpen(); }, 0);
    return;
  }
  if (!$("orthoWin") || !ortho.map) return;
  clearTimeout(ortho.timer); ortho.timer = setTimeout(orthoUpdate, 400);
}
async function orthoUpdate() {
  const m = ortho.map; if (!m || !$("orthoWin") || !m.getSource("sk")) return;
  if (ortho.finishBtn) ortho.finishBtn.disabled = !(edit.active && canFinishSketch());
  const ll = edit.verts.map(([e, n]) => toLL(e, n));
  const feats = ll.map((c) => ({ type: "Feature", geometry: { type: "Point", coordinates: c }, properties: {} }));
  if (ll.length >= 2) feats.push({ type: "Feature", geometry: { type: "LineString", coordinates: edit.geomType === "polygon" && ll.length >= 3 ? [...ll, ll[0]] : ll }, properties: {} });
  m.getSource("sk").setData({ type: "FeatureCollection", features: feats });
  const area = wf.editing?.area_utm?.coordinates?.[0];
  m.getSource("area").setData(area ? { type: "FeatureCollection", features: [{ type: "Feature", properties: {}, geometry: { type: "LineString", coordinates: area.map(([e, n]) => toLL(e, n)) } }] } : empty());
  const ring = orthoRing();
  const key = ring ? vertsKey(ring) : "";
  if (ring && ortho.fitted !== key) {
    const lls = ring.map(([e, n]) => toLL(e, n));
    const b = lls.reduce((bb, c) => [[Math.min(bb[0][0], c[0]), Math.min(bb[0][1], c[1])], [Math.max(bb[1][0], c[0]), Math.max(bb[1][1], c[1])]], [[Infinity, Infinity], [-Infinity, -Infinity]]);
    m.fitBounds(b, { padding: 60, maxZoom: 20, animate: false }); ortho.fitted = key;
  }
  if (!ring) { ortho.badge.className = "orthoBadge muted"; ortho.badge.textContent = "Sketch the parcel (or load the job area's points) to check the land."; m.getSource("ov").setData(empty()); return; }
  const seq = ++ortho.seq;
  let r;
  try { r = await api("/api/v1/parcels/overlaps", { method: "POST", body: { vertices: ring, editing_id: edit.editingId || null } }); }
  catch (e) { if (seq === ortho.seq) { ortho.badge.className = "orthoBadge muted"; ortho.badge.textContent = "The land could not be checked: " + e.message; } return; }
  if (seq !== ortho.seq || !m.getSource("ov")) return;
  m.getSource("ov").setData({ type: "FeatureCollection", features: r.overlaps.filter((o) => o.geometry).map((o) => ({ type: "Feature", geometry: o.geometry, properties: { upin: o.upin } })) });
  if (r.titled.length) {
    ortho.badge.className = "orthoBadge bad";
    ortho.badge.textContent = "✕ Title exists on this land: " + r.titled.map((t) => `${t.upin}${t.su_id ? " → " + t.su_id : ""}${t.refnum ? " (" + t.refnum + ")" : ""}`).join("; ");
  } else if (r.overlaps.length || r.at_point.length) {
    ortho.badge.className = "orthoBadge bad";
    ortho.badge.textContent = "✕ " + (r.overlaps.length ? "Overlaps " + r.overlaps.map((o) => `${o.upin} (${area2(o.area_m2)} m²)`).join(", ") : "Already digitised: " + r.at_point.join(", "));
  } else {
    ortho.badge.className = "orthoBadge ok";
    ortho.badge.textContent = "✔ Plot free: no overlap and no title in this Cadastre (Aumentum and the Registry are checked at Finish).";
  }
}

// What happened with the Registry file right after the commit (the UPIN and link date written).
function linkSummary(links) {
  if (!links || !links.length) return "";
  const ok = links.filter((l) => l.linked), bad = links.filter((l) => !l.linked);
  return [ok.length ? `Linked to the Registry file: ${ok.map((l) => `${l.upin} → ${l.su_id}`).join(", ")}.` : "",
    bad.length ? `Not linked: ${bad.map((l) => `${l.upin} (${l.reason})`).join("; ")}. Use Link now on the Completed tab once fixed.` : ""].filter(Boolean).join(" ");
}
// Digitising done (the commit completed the job): a clean start for the next job. The edit session
// ends (nothing is left to save), the editing windows close, the job is no longer worked on, and
// the map shows the Table Of Contents, zoomed to the parcel just committed.
async function goHome(upins = []) {
  edit.active = false; clearSketch(); orthoClose();
  if (beaconLabels.upin) hideBeacons();
  ["createFeaturesWin", "sketchPropsWin", "commitWin", "pointListWin", "parcelAttrWin", "stepToolsWin"].forEach((id) => $(id)?.remove());
  setTool("pan"); updateEditorButtons();
  wf.editing = null; showJobArea();
  state.job = null; renderJobBox(); try { await loadDrafts(); } catch { /* the job is closed */ }
  showCompleted(false); showRegistry(false); showWorkflow(false); showTasks(false);
  if ($("tocPane").classList.contains("hidden")) togglePane("tocPane");
  buildTocTools(); buildTOC(); renderTask();
  if (upins[0]) { try { await registryZoom(upins[0]); } catch { /* zoom is a convenience */ } }
  saveSession();
  status(upins.length ? `Digitising complete: ${upins.join(", ")} committed. Ready for the next job.` : "Ready for the next job.");
}

// Straight on to the commit of the job being worked on (from the attribute form): the Commit window
// when the job is at its commit task, else Post & Finish (which walks it there). Same windows, same
// rules (beacon numbers, staff name, the System Check again on the server).
async function commitWorkedJob() {
  const id = wf.editing?.job_instance_id; if (!id) return;
  await wfRefreshEditing();
  if (wf.editing?.is_commit_task) { await wfCommitDialog(); return; }
  let j = wf.detail?.job && wf.detail.job.id === id ? wf.detail.job : null;
  if (!j) { try { j = (await api("/api/v1/wf/jobs/" + id)).job; } catch (e) { uiAlert(e.message); return; } }
  await wfFinishJob(j, { skipConfirm: true });
}
// The bounding box of a set of UTM vertices, expanded by `margin` metres, as a closed ring.
function bboxRingUTM(verts, margin) {
  let minE = Infinity, minN = Infinity, maxE = -Infinity, maxN = -Infinity;
  for (const [e, n] of verts) { if (e < minE) minE = e; if (e > maxE) maxE = e; if (n < minN) minN = n; if (n > maxN) maxN = n; }
  minE -= margin; minN -= margin; maxE += margin; maxN += margin;
  return [[minE, minN], [maxE, minN], [maxE, maxN], [minE, maxN], [minE, minN]];
}
// Ensure the job being worked on has an open job area, deriving one from the sketch if needed. A
// parcel under a workflow job must sit inside a job area; this keeps Finish Sketch from failing for
// a missing or closed area, so digitizing flows straight through.
async function ensureJobArea(utm) {
  if (!wf.editing || !wf.editing.job_instance_id) return true; // not a workflow job: no area rule
  const jid = wf.editing.job_instance_id;
  try {
    if (!wf.editing.area) {
      wf.editing = await api(`/api/v1/wf/jobs/${jid}/area`, { method: "PUT", body: { geometry: { type: "Polygon", coordinates: [bboxRingUTM(utm, 25)] } } });
      wf.selEditing = wf.editing; showJobArea();
      status("Job area created automatically from the parcel extent.");
    } else if (wf.editing.area_status === "closed") {
      wf.editing = await api(`/api/v1/wf/jobs/${jid}/area/reopen`, { method: "POST" });
      wf.selEditing = wf.editing; showJobArea();
      status("Job area reopened for editing.");
    }
    return true;
  } catch (e) { uiAlert("Could not prepare the job area: " + e.message); return false; }
}
async function saveSketchParcel() {
  if (!state.job) { uiAlert("Choose a Job Area first — a digitized parcel is saved as a draft under a job."); return; }
  const utm = edit.verts.slice();
  // Verify Points: the digitiser checks every corner (beacon, coordinates) before anything is saved.
  if (!(await verifyPoints(utm))) { status("Back to the sketch: adjust the corners, then Finish Sketch again."); return; }
  // System check before the next stage: nothing goes on to attributes and commit on taken land.
  if (!(await runPrecheck(utm))) { status("Back to the sketch: the system check did not pass."); return; }
  if (!(await ensureJobArea(utm))) return; // keep the sketch so nothing is lost
  // Finish Sketch saves the shape at once (nothing to confirm, nothing lost if the attribute form
  // is closed); the Update Parcel Attribute form then opens on the saved draft.
  const editingId = edit.editingId, sketchArea = sketchAreaM2();
  let saved;
  try {
    const geometry = { type: "Polygon", coordinates: [[...utm, utm[0]]] };
    saved = editingId
      ? await api(`/api/v1/jobs/${state.job.id}/parcels/${editingId}`, { method: "PUT", body: { geometry, stated_area: null } })
      : await api(`/api/v1/jobs/${state.job.id}/parcels`, { method: "POST", body: { geometry, stated_area: null } });
  } catch (e) { uiAlert("Save failed: " + e.message + "\nThe sketch is kept: correct it and Finish Sketch again."); return; }
  const id = saved && saved.id ? saved.id : editingId;
  if (!state.job.reference.startsWith("WF-")) notify(`Parcel saved to ${state.job.reference}, which is not a workflow job. To work as in Aumentum, open the job in the Workflow panel and choose Work On This Job.`, "warn");
  else notify(`${editingId ? "Draft parcel updated" : "Parcel saved as a draft"} in ${state.job.reference}. ${draftNextStep()}`);
  zoomToUTMRing(utm);
  clearSketch(); orthoClose(); refreshSketchProps(); updateEditorButtons();
  try { await loadDrafts(); buildCatalog(); wfRefreshEditing(); renderWorkflow(); } catch { /* the draft is saved; the view refreshes later */ }
  // Attributes: the values already captured on this draft, else the job's Registry defaults.
  let prefill = {};
  if (id) { try { prefill = await api(`/api/v1/parcels/${id}/attributes`); } catch { prefill = {}; } }
  const attrs = await parcelAttrDialog({ prefill, parcelId: id, areaM2: saved && saved.computed_area != null ? Number(saved.computed_area) : sketchArea,
    title: "Update Parcel Attribute", saveLabel: "Save attributes", commitLabel: wf.editing ? "Save & Commit…" : null });
  if (!attrs || !id) return; // closed: the shape stays saved
  const goCommit = !!attrs.__commit; delete attrs.__commit;
  if (Object.keys(attrs).length) {
    try { await api(`/api/v1/parcels/${id}/attributes`, { method: "PUT", body: attrs }); status("Parcel attributes saved."); }
    catch (e) { uiAlert("The parcel is saved, but its attributes were not: " + e.message + "\nUse Update Parcel Attribute to enter them again."); return; }
  }
  if (goCommit) await commitWorkedJob();
}
// Property-document capture shown on Finish Sketch (parcel). Resolves to the attributes object
// (possibly empty), or null if cancelled. Prefills the Registry link + property fields from the
// job's linkage (its LRS transaction's property), so the surveyor confirms rather than retypes.
// Plateau State LGAs (for the LGA dropdown), plus common lookup values. Editable in the field.
const PLATEAU_LGAS = ["Barkin Ladi", "Bassa", "Bokkos", "Jos East", "Jos North", "Jos South", "Kanam",
  "Kanke", "Langtang North", "Langtang South", "Mangu", "Mikang", "Pankshin", "Qua'an Pan", "Riyom", "Shendam", "Wase"];
const PROPERTY_TYPES = ["Land Parcel", "Building", "Strata", "Easement", "Road", "Reserve"];
const PARCEL_STATUSES = ["Active", "Pending", "Historic", "Archived"];
// Signed area of the current sketch (m², UTM 32N) via the shoelace formula.
// Areas are shown to 2 decimals, cut not rounded (PLAGIS: 903.1976 m² is 903.19 m²).
function area2(v) { const n = Number(v); return v == null || v === "" || !isFinite(n) ? "—" : (Math.floor(n * 100 + 1e-7) / 100).toFixed(2); }
function sketchAreaM2() {
  const v = edit.verts; if (v.length < 3) return null;
  let a = 0; for (let i = 0; i < v.length; i++) { const [x1, y1] = v[i], [x2, y2] = v[(i + 1) % v.length]; a += x1 * y2 - x2 * y1; }
  return Math.abs(a) / 2;
}
// The ArcMap/Aumentum "Update Parcel Attribute" form (General + Survey tabs), shown on Finish
// Sketch. Resolves to the attributes object (possibly empty), or null if cancelled. Prefills the
// Registry link + property fields from the job's linkage. The UPIN is assigned at commit.
// The Registry's place lists (Location, Layout, Area), kept by the server; loaded once per page.
let placeLists = null;
async function loadPlaceLists() {
  if (placeLists && !placeLists.error) return placeLists;
  try { placeLists = await api("/api/v1/registry/lookups"); }
  catch (e) { placeLists = { location: [], layout_type: [], property_area: [], error: e.message }; }
  return placeLists;
}
// The job's Registry property (its location / layout), when the job belongs to a transaction.
async function jobRegistryProperty(txn, propertyNumber) {
  if (!txn) return null;
  try {
    const t = await api("/api/v1/registry/transactions/" + encodeURIComponent(txn));
    const ps = t.properties || [];
    return ps.find((p) => propertyNumber && (p.refnum || "").toUpperCase() === propertyNumber.toUpperCase()) || ps[0] || null;
  } catch { return null; }
}
async function parcelAttrDialog(opts = {}) {
  let link = {};
  if (wf.editing && wf.editing.job_instance_id) {
    try { link = await api(`/api/v1/wf/jobs/${wf.editing.job_instance_id}/linkage`); } catch { link = {}; }
  }
  const [places, regProp] = await Promise.all([loadPlaceLists(), jobRegistryProperty(link.lrs_transaction_number, link.property_number)]);
  const jobNo = link.property_number || link.lrs_transaction_number || (state.job && state.job.reference) || "";
  const areaM2 = opts.areaM2 !== undefined ? opts.areaM2 : sketchAreaM2();
  // Values already captured on the draft (editing) take precedence over the job's linkage defaults.
  const pf = opts.prefill || {};
  const pv = (k, d) => (pf[k] != null && pf[k] !== "" ? pf[k] : d);
  // Parcel Name: the property number (PLAGIS), e.g. PL67363; with several parcels in the job,
  // PL67363-1, PL67363-2, … Without a property number, Aumentum's PCL1, PCL2, …
  const drafts = (state.drafts && state.drafts.features) || [];
  const at = opts.parcelId ? drafts.findIndex((x) => x.properties.id === opts.parcelId) : -1;
  const nth = at >= 0 ? at + 1 : drafts.length + 1;
  const many = drafts.length > 1 || (at < 0 && drafts.length >= 1);
  const propNo = (link.property_number || regProp?.refnum || "").trim();
  const pclDefault = propNo ? (many ? `${propNo}-${nth}` : propNo) : "PCL" + nth;
  return new Promise((resolve) => {
    const f = {};
    const row = (label, ctrl) => h("div", { class: "frow" }, h("label", {}, label), ctrl);
    const ro = (label, val) => row(label, h("span", { class: "roval" }, val));
    const txt = (key, val) => { const i = h("input", { value: val || "", style: "flex:1;min-width:0" }); f[key] = i; return i; };
    const sel = (key, opts, val) => { const s = h("select", { style: "flex:1;min-width:0" }, ...opts.map((o) => h("option", { selected: o === (val || "") }, o))); f[key] = s; return s; };
    // A Registry list as a dropdown. A value already on the parcel that is not in the list (the
    // codes 1 to 4 Aumentum stored) is kept and shown as it is; with no list, a text box.
    // Location (the Registry's 4 zones), Layout and Area: dropdowns of the Registry's lists that also
    // take a new name (type it). Pre-filled from the Registry property; Aumentum's codes 1 to 4 kept.
    const placeCombo = (key, listKey, val) => {
      const list = places[listKey] || [];
      const id = "places-" + key;
      const dl = h("datalist", { id }, ...list.map((x) => h("option", { value: x.label })));
      const i = h("input", { list: id, value: val || "", class: "comboIn", autocomplete: "off",
        placeholder: list.length ? `Choose or type (${list.length.toLocaleString()} in the Registry)` : "Type",
        title: places.stale ? `Registry list from ${String(places.fetched_at || "").slice(0, 10)} (the Registry could not be reached)` : "Choose from the Registry's list, or type a new name" });
      const open = h("button", { type: "button", class: "comboBtn", title: "Show the list", tabindex: "-1", onclick: () => {
        const keep = i.value; i.value = ""; i.focus(); try { i.showPicker?.(); } catch { /* not supported */ }
        i.addEventListener("blur", () => { if (!i.value) i.value = keep; }, { once: true });
      } }, "▾");
      f[key] = i;
      return h("span", { class: "combo" }, i, open, dl);
    };
    const placeSearch = (key, val) => {
      const lists = [["location", "Zone"], ["property_area", "Area"], ["layout_type", "Layout"]];
      const id = "places-" + key;
      const dl = h("datalist", { id }, ...lists.flatMap(([k, kind]) => (places[k] || []).map((x) => h("option", { value: x.label, label: kind }))));
      const n = lists.reduce((t, [k]) => t + (places[k] || []).length, 0);
      const i = h("input", { list: id, value: val || "", style: "flex:1;min-width:0", autocomplete: "off",
        placeholder: n ? `Type to search ${n.toLocaleString()} Registry places` : "Location",
        title: places.stale ? `Registry list from ${String(places.fetched_at || "").slice(0, 10)} (the Registry could not be reached)` : "From the Registry: zones, areas and layouts" });
      f[key] = i;
      return h("span", { style: "flex:1;min-width:0;display:flex" }, i, dl);
    };
    const placeSelect = (key, list, val) => {
      if (!list || !list.length) return txt(key, val);
      const labels = list.map((x) => x.label);
      const s = h("select", { style: "flex:1;min-width:0", title: places.stale ? `Registry list from ${String(places.fetched_at || "").slice(0, 10)} (the Registry could not be reached)` : "From the Registry" },
        h("option", { value: "" }, ""),
        val && !labels.includes(val) ? h("option", { value: val, selected: true }, /^\d+$/.test(val) ? `${val} (Aumentum code)` : val) : null,
        ...labels.map((l) => h("option", { value: l, selected: l === val }, l)));
      f[key] = s; return s;
    };

    const general = h("div", {},
      ro("UPIN:", "— assigned on commit —"),
      row("Parcel Name:", txt("parcel_name", pv("parcel_name", pclDefault))),
      row("Block:", txt("block", pv("block", ""))),
      ro("Parcel Area:", areaM2 != null ? `${area2(areaM2)} m²` : "—"),
      row("Area Unit:", sel("_area_unit", ["sq.m.", "Hectare"], "sq.m.")),
      row("Property Type:", sel("property_type", PROPERTY_TYPES, pv("property_type", link.property_type || "Land Parcel"))),
      row("Property Number:", txt("property_number", pv("property_number", link.property_number || jobNo))),
      row("Plot:", txt("plot", pv("plot", link.plot || ""))),
      row("Location:", placeCombo("location", "location", pv("location", regProp?.location || ""))),
      row("Layout:", placeCombo("layout", "layout_type", pv("layout", regProp?.layout || ""))),
      row("Area:", placeCombo("area_name", "property_area", pv("area_name", regProp?.area || ""))),
      row("Plan Number:", txt("plan_number", pv("plan_number", link.plan_number || ""))),
      row("LGA:", sel("lga", ["", ...PLATEAU_LGAS], pv("lga", link.lga || ""))),
      row("Status:", sel("status", PARCEL_STATUSES, pv("status", "Active"))),
      row("Registry ID (LRSID):", txt("lrs_id", pv("lrs_id", link.lrs_id || ""))),
      ro("Linkage Status:", pv("lrs_id", link.lrs_id) ? "Linked" : "Pending (set on commit)"));
    const survey = h("div", { class: "hidden" },
      row("Surveyor-General:", txt("surveyor_general", pv("surveyor_general", ""))),
      link.lrs_transaction_number ? h("div", { class: "frow muted" }, `LRS transaction: ${link.lrs_transaction_number}`) : null,
      h("p", { class: "muted" }, "Traverse / control-point detail comes from the Construct Parcel (survey entry) step."));

    const tabG = h("span", { class: "attrTab active" }, "General");
    const tabS = h("span", { class: "attrTab" }, "Survey");
    const showTab = (g) => { general.classList.toggle("hidden", !g); survey.classList.toggle("hidden", g); tabG.classList.toggle("active", g); tabS.classList.toggle("active", !g); };
    tabG.onclick = () => showTab(true); tabS.onclick = () => showTab(false);

    const collect = () => {
      const o = {};
      for (const k of ["parcel_name", "block", "property_type", "property_number", "plot", "location", "layout", "area_name", "plan_number", "lga", "status", "surveyor_general", "lrs_id"]) {
        const v = (f[k]?.value || "").trim(); if (v) o[k] = v;
      }
      return o;
    };
    const w = floatWindow(opts.title || "Update Parcel Attribute", h("div", {}, h("div", { class: "attrTabs" }, tabG, tabS), general, survey,
      h("div", { class: "taskNav" },
        opts.commitLabel ? h("button", { class: "btn btnPrimary", title: "Save the attributes, then commit the job's parcel(s)", onclick: () => { w.remove(); resolve({ ...collect(), __commit: true }); } }, opts.commitLabel) : null,
        h("button", { class: "btn", onclick: () => { w.remove(); resolve(collect()); } }, opts.saveLabel || "Save parcel"),
        h("button", { class: "btn", onclick: () => { w.remove(); resolve(null); } }, "Cancel"))),
      { id: "parcelAttrWin", width: 340 });
    setTimeout(() => w.querySelector("input")?.focus(), 0);
  });
}
async function saveSketchPoint() {
  const [E, N] = edit.verts[0];
  const number = await uiPrompt("Control point number:", "");
  if (number == null || !number.trim()) return;
  try {
    await api("/api/v1/beacons", { method: "POST", body: { number: number.trim(), easting: E, northing: N, is_control: true } });
    status(`Control point ${number.trim()} created at ${E.toFixed(3)} ${N.toFixed(3)}.`);
    await loadBeacons(); clearSketch(); refreshSketchProps(); updateEditorButtons();
  } catch (e) { uiAlert("Create point failed: " + e.message); }
}
function saveSketchLine() {
  const ll = edit.verts.map(([e, n]) => toLL(e, n));
  addGraphic({ type: "line", pts: ll });
  status(`Boundary line added to the drawing (${edit.verts.length} vertices).`);
  clearSketch(); refreshSketchProps(); updateEditorButtons();
}
function deleteSketch() { clearSketch(); refreshSketchProps(); updateEditorButtons(); status("Sketch deleted."); }
function toggleSnapping() { edit.snapping = !edit.snapping; $("edSnap")?.classList.toggle("active", edit.snapping); status(`Snapping ${edit.snapping ? "on" : "off"}.`); }

// --- Trace (follow existing boundaries while sketching) --------------------------------------
function toggleTrace() {
  edit.tracing = !edit.tracing; edit.trace = null;
  $("edTrace")?.classList.toggle("active", edit.tracing);
  if (edit.tracing) { if (!edit.active) startEditing("sketch"); else setTool("sketch"); }
  status(edit.tracing ? "Trace on — click along existing parcel boundaries." : "Trace off.");
}
// The rings (vertex arrays, in lng/lat) of a GeoJSON geometry.
function ringsOf(g) {
  if (!g) return [];
  if (g.type === "LineString") return [{ coords: g.coordinates, closed: false }];
  if (g.type === "MultiLineString") return g.coordinates.map((c) => ({ coords: c, closed: false }));
  if (g.type === "Polygon") return g.coordinates.map((c) => ({ coords: c, closed: true }));
  if (g.type === "MultiPolygon") return g.coordinates.flat().map((c) => ({ coords: c, closed: true }));
  return [];
}
// The nearest existing-boundary vertex within tolPx of the cursor, with its ring (UTM coords).
function nearestRingUTM(pt, tolPx) {
  const layers = ["parcels-line", "drafts-line", "survey-line"].filter((l) => map.getLayer(l));
  if (!layers.length) return null;
  const feats = map.queryRenderedFeatures([[pt.x - tolPx, pt.y - tolPx], [pt.x + tolPx, pt.y + tolPx]], { layers });
  let best = null, bestD = tolPx;
  feats.forEach((f) => ringsOf(f.geometry).forEach((ring, ri) => {
    let coords = ring.coords.map(([lng, lat]) => toUTM(lng, lat));
    // A polygon ring repeats its first point last; index the unique vertices only.
    if (ring.closed && coords.length > 1) coords = coords.slice(0, -1);
    const key = (f.id ?? f.properties?.id ?? "r") + ":" + ri;
    coords.forEach(([E, N], i) => {
      const p = map.project(toLL(E, N)); const d = Math.hypot(p.x - pt.x, p.y - pt.y);
      if (d < bestD) { bestD = d; best = { coords, idx: i, key, closed: ring.closed }; }
    });
  }));
  return best;
}
// Vertices along a ring between index `from` and `to`, exclusive (the shorter way on a closed ring).
function ringArc(coords, from, to, closed) {
  const n = coords.length, out = [];
  if (from === to || n < 2) return out;
  if (!closed) { const step = to > from ? 1 : -1; for (let i = from + step; i !== to; i += step) out.push(coords[i]); return out; }
  const fwd = []; for (let i = (from + 1) % n; i !== to && fwd.length <= n; i = (i + 1) % n) fwd.push(coords[i]);
  const bwd = []; for (let i = (from - 1 + n) % n; i !== to && bwd.length <= n; i = (i - 1 + n) % n) bwd.push(coords[i]);
  return fwd.length <= bwd.length ? fwd : bwd;
}
// A trace click: snap to the nearest boundary vertex; if the previous vertex was on the same ring,
// insert the chain of boundary vertices between them. Returns false when no boundary is near.
function traceClick(pt) {
  const ring = nearestRingUTM(pt, 16);
  if (!ring) return false;
  if (edit.verts.length && edit.trace && edit.trace.key === ring.key) {
    for (const v of ringArc(ring.coords, edit.trace.idx, ring.idx, ring.closed)) edit.verts.push([v[0], v[1]]);
  }
  const [E, N] = ring.coords[ring.idx];
  edit.verts.push([E, N]); edit.sel = edit.verts.length - 1;
  edit.trace = { key: ring.key, idx: ring.idx };
  drawSketch();
  return true;
}

// --- Absolute X, Y (key the next sketch vertex by coordinate) --------------------------------
function absoluteXYDialog(lngLat) {
  const [E, N] = toUTM(lngLat.lng, lngLat.lat);
  const xi = h("input", { type: "number", step: "0.001", value: E.toFixed(3), style: "width:110px" });
  const yi = h("input", { type: "number", step: "0.001", value: N.toFixed(3), style: "width:110px" });
  const add = () => {
    const x = parseFloat(xi.value), y = parseFloat(yi.value);
    if (!isFinite(x) || !isFinite(y)) { status("Enter a valid Easting and Northing."); return; }
    w.remove();
    if (edit.geomType === "point") { edit.verts = [[x, y]]; edit.sel = 0; drawSketch(); finishSketch(); return; }
    edit.trace = null; edit.verts.push([x, y]); edit.sel = edit.verts.length - 1;
    drawSketch(); refreshSketchProps(); updateEditorButtons();
  };
  xi.onkeydown = (ev) => { if (ev.key === "Enter") yi.focus(); };
  yi.onkeydown = (ev) => { if (ev.key === "Enter") add(); };
  const w = floatWindow("Absolute X, Y", h("div", {},
    h("div", { class: "frow" }, h("label", {}, "X (Easting):"), xi),
    h("div", { class: "frow" }, h("label", {}, "Y (Northing):"), yi),
    h("div", { class: "taskNav" }, h("button", { class: "btn", onclick: add }, "Add vertex"),
      h("button", { class: "btn", onclick: () => w.remove() }, "Cancel"))), { id: "absXYWin", width: 240 });
  setTimeout(() => { xi.focus(); xi.select(); }, 0);
}

// --- Cut Polygons (split a draft parcel along a drawn line) -----------------------------------
function cutClick(e) {
  const pt = e.point;
  if (edit.cut.length) { const last = map.project(toLL(...edit.cut[edit.cut.length - 1])); if (Math.hypot(last.x - pt.x, last.y - pt.y) < 4) return; }
  let [E, N] = toUTM(e.lngLat.lng, e.lngLat.lat);
  if (edit.snapping) { const s = snapUTM(pt); if (s) [E, N] = s; }
  edit.cut.push([E, N]); drawCut();
  status("Cut line: click to add points, double-click to split the parcel it crosses.");
}
function drawCut() {
  const ll = edit.cut.map(([e, n]) => toLL(e, n));
  const feats = [];
  if (ll.length >= 2) feats.push({ type: "Feature", geometry: { type: "LineString", coordinates: ll } });
  ll.forEach((c, i) => feats.push({ type: "Feature", properties: { idx: i, sel: i === ll.length - 1 }, geometry: { type: "Point", coordinates: c } }));
  map.getSource("sketch")?.setData({ type: "FeatureCollection", features: feats });
}
function clearCut() { edit.cut = []; if (edit.verts.length) drawSketch(); else map.getSource("sketch")?.setData(empty()); }
async function finishCut() {
  if (edit.cut.length < 2) { status("Draw a line across the parcel (two or more points)."); return; }
  if (!state.job) { uiAlert("Open a job (Work On This Job) before cutting its draft parcels."); return; }
  const target = draftForCut(edit.cut);
  if (!target) { uiAlert("The cut line must cross one of the job's draft parcels."); clearCut(); return; }
  const line = { type: "LineString", coordinates: edit.cut.slice() };
  try {
    const parts = await api(`/api/v1/jobs/${state.job.id}/parcels/${target}/cut`, { method: "POST", body: { line } });
    status(`Parcel cut into ${parts.length} parts.`);
    await loadDrafts(); buildCatalog(); wfRefreshEditing();
  } catch (e) { uiAlert("Cut failed: " + e.message); }
  clearCut();
}
// The job draft the cut line crosses: the first whose polygon contains a point sampled along the line.
function draftForCut(cutUTM) {
  const ll = cutUTM.map(([e, n]) => toLL(e, n)); // -> [lng, lat]
  const samples = [];
  for (let i = 0; i < ll.length; i++) { samples.push([ll[i].lng, ll[i].lat]); if (i + 1 < ll.length) samples.push([(ll[i].lng + ll[i + 1].lng) / 2, (ll[i].lat + ll[i + 1].lat) / 2]); }
  for (const f of state.drafts.features) if (samples.some((s) => pointInFeature(s, f))) return f.properties.id;
  return null;
}
function pointInFeature(pt, f) {
  const g = f.geometry; if (!g) return false;
  const polys = g.type === "Polygon" ? [g.coordinates] : g.type === "MultiPolygon" ? g.coordinates : [];
  return polys.some((rings) => pointInRing(pt, rings[0]) && !rings.slice(1).some((hole) => pointInRing(pt, hole)));
}
function pointInRing(pt, ring) {
  let inside = false;
  for (let i = 0, j = ring.length - 1; i < ring.length; j = i++) {
    const xi = ring[i][0], yi = ring[i][1], xj = ring[j][0], yj = ring[j][1];
    if (((yi > pt[1]) !== (yj > pt[1])) && (pt[0] < ((xj - xi) * (pt[1] - yi)) / (yj - yi) + xi)) inside = !inside;
  }
  return inside;
}

// --- Attributes (edit a draft parcel's captured attributes) ----------------------------------
// Add Parcel Attributes (Plot Parcel step 1010): the job's one draft opens at once; with several,
// click the one to edit (the Attributes tool).
async function wfAddAttributes() {
  try { await loadDrafts(); } catch { /* use what is loaded */ }
  const mine = (state.drafts && state.drafts.features) || [];
  if (!mine.length) { status("No draft parcel in this job yet: plot it first (Construct Parcel → Digitize Parcel)."); return; }
  if (mine.length === 1) { await editDraftAttributes(mine[0].properties.id); return; }
  setTool("attr");
  status(`${mine.length} draft parcels in this job: click the one whose attributes you want to enter.`);
}
async function attrClick(e) {
  const layers = ["drafts-fill"].filter((l) => map.getLayer(l));
  const f = layers.length ? map.queryRenderedFeatures(e.point, { layers })[0] : null;
  if (!f) { status("Click a draft parcel to edit its attributes."); return; }
  await editDraftAttributes(f.properties.id);
}
async function editDraftAttributes(id) {
  let prefill = {};
  try { prefill = await api(`/api/v1/parcels/${id}/attributes`); } catch { prefill = {}; }
  const draft = state.drafts.features.find((x) => x.properties.id === id);
  const areaM2 = draft ? featureAreaM2(draft) : null;
  const attrs = await parcelAttrDialog({ prefill, parcelId: id, areaM2, title: "Update Parcel Attribute", saveLabel: "Save attributes" });
  if (attrs === null) return;
  try { await api(`/api/v1/parcels/${id}/attributes`, { method: "PUT", body: attrs }); status("Parcel attributes updated."); }
  catch (e) { uiAlert("Update failed: " + e.message); }
}
// Area (m², UTM 32N) of a polygon feature (lng/lat), via the shoelace formula on its outer ring.
function featureAreaM2(f) {
  // The server's area of the stored shape (exact); the map's degrees are only a fallback.
  const ca = f.properties && Number(f.properties.computed_area); if (ca > 0) return ca;
  const g = f.geometry; if (!g) return null;
  const ring = g.type === "Polygon" ? g.coordinates[0] : g.type === "MultiPolygon" ? g.coordinates[0][0] : null;
  if (!ring || ring.length < 4) return null;
  const v = ring.map(([lng, lat]) => toUTM(lng, lat));
  let a = 0; for (let i = 0; i < v.length - 1; i++) a += v[i][0] * v[i + 1][1] - v[i + 1][0] * v[i][1];
  return Math.abs(a) / 2;
}

function updateEditorButtons() {
  const set = (id, on) => $(id)?.classList.toggle("disabled", !on);
  const has = edit.verts.length > 0;
  set("edFinish", canFinishSketch()); set("edDelete", has); set("edProps", true);
  set("edPost", !!(wf.editing && wf.editing.drafts > 0));
  $("edSnap")?.classList.toggle("active", edit.snapping);
  $("edTrace")?.classList.toggle("active", edit.tracing);
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

// --- Corner identification: what each sketched corner is (fabric beacon, shared corner, new) ---
// Asked of the server after each change (debounced), so the corner list shows beacon numbers as
// the digitiser clicks; Verify Points asks again before saving.
const sketchIdent = { key: "", list: [], timer: null };
const vertsKey = (verts) => verts.map(([e, n]) => `${e.toFixed(3)},${n.toFixed(3)}`).join(";");
async function identifyVertices(verts) {
  if (!verts.length) return [];
  return api("/api/v1/beacons/identify", { method: "POST", body: { vertices: verts.map(([e, n]) => [e, n]) } });
}
function scheduleIdentify() {
  const key = vertsKey(edit.verts);
  if (key === sketchIdent.key) return;
  clearTimeout(sketchIdent.timer);
  sketchIdent.timer = setTimeout(async () => {
    const verts = edit.verts.map((v) => v.slice());
    try {
      const list = await identifyVertices(verts);
      if (vertsKey(edit.verts) !== vertsKey(verts)) return; // the sketch moved on meanwhile
      sketchIdent.key = vertsKey(verts); sketchIdent.list = list; refreshSketchProps();
    } catch { /* the list stays without beacon names; Verify Points asks again */ }
  }, 350);
}
// How a corner is shown: its beacon number, the committed parcel it shares, or a number typed
// for it (kept by coordinates), else a temporary P-number.
function cornerLabel(id, newIndex) {
  if (!id) return "";
  if (id.source === "beacon") return id.beacon_number;
  const typed = pointIds.get(ptKey(id.easting, id.northing));
  if (typed) return typed;
  if (id.source === "shared") return `corner of ${id.shared_with[0]}`;
  return `P${newIndex}`;
}
const CORNER_SOURCE = { beacon: "Existing beacon", shared: "Shared corner", new: "New point" };
// Verify Points (before attributes): every corner, its beacon, coordinates, where it came from and
// the distance to the next corner. New points can be numbered here (from the surveyor's plan);
// those numbers fill the Beacon Numbers window at commit. Resolves true to go on, false to go back.
async function verifyPoints(utm) {
  let ids;
  try { ids = await identifyVertices(utm); }
  catch (e) { await uiAlert("The corners could not be checked: " + e.message + "\nThe sketch is kept; try Finish Sketch again."); return false; }
  let k = 0;
  const inputs = [];
  const tbl = h("table", { class: "verifyTbl" },
    h("tr", {}, h("th", {}, "#"), h("th", {}, "Beacon"), h("th", {}, "Easting"), h("th", {}, "Northing"), h("th", {}, "Source"), h("th", {}, "To next (m)")));
  ids.forEach((id, i) => {
    const isNew = id.source !== "beacon";
    const label = cornerLabel(id, isNew && !pointIds.get(ptKey(id.easting, id.northing)) ? ++k : 0);
    const cell = isNew
      ? h("input", { class: "uiInput", style: "margin:0;width:118px", value: pointIds.get(ptKey(id.easting, id.northing)) || "", placeholder: label })
      : h("b", {}, label);
    if (isNew) inputs.push([id, cell]);
    const src = id.source === "shared" ? `Shared corner (${id.shared_with.join(", ")})` : CORNER_SOURCE[id.source];
    const tr = h("tr", { class: `src-${id.source}`, title: "Show this corner on the map" },
      h("td", { class: "n" }, String(i + 1)), h("td", {}, cell), h("td", {}, fmt3(id.easting)), h("td", {}, fmt3(id.northing)),
      h("td", {}, src, id.offset_m ? h("span", { class: "muted" }, ` ${(id.offset_m * 100).toFixed(1)} cm`) : null), h("td", { class: "n" }, id.next_distance_m.toFixed(2)));
    tr.addEventListener("click", (ev) => {
      if (ev.target.tagName === "INPUT") return;
      edit.sel = i; drawSketch(); map.easeTo({ center: toLL(id.easting, id.northing), zoom: Math.max(map.getZoom(), 19) });
      tbl.querySelectorAll("tr.selrow").forEach((r) => r.classList.remove("selrow")); tr.classList.add("selrow");
    });
    tbl.append(tr);
  });
  const counts = ["beacon", "shared", "new"].map((s) => [s, ids.filter((x) => x.source === s).length]).filter(([, n]) => n)
    .map(([s, n]) => `${n} ${CORNER_SOURCE[s].toLowerCase()}${n > 1 ? "s" : ""}`).join(", ");
  const body = h("div", {},
    h("p", {}, h("b", {}, `${ids.length} corners`), `: ${counts}. Area ${area2(sketchAreaM2())} m².`),
    h("p", { class: "muted" }, "Check each corner against the survey plan. Click a row to see it on the map. Type the plan's beacon number for a new point if you have it (it can also be entered at commit)."),
    h("div", { class: "verifyWrap" }, tbl));
  const ok = await uiDialog({ title: "Verify Points", message: body, kind: "info", width: 720,
    buttons: [{ label: "Back to sketch", value: false, cancel: true }, { label: "Confirm points", value: true, primary: true }] });
  if (!ok) return false;
  for (const [id, inp] of inputs) { const v = inp.value.trim().toUpperCase(); if (v) rememberPointId(id.easting, id.northing, v); }
  return true;
}

// --- System check (before the next stage, and again at commit on the server) ---
// Overlaps here and in Aumentum, the drop point (is the plot free?), a title on the land, and the
// Registry transaction. A failure sends the digitiser back to the sketch; warnings need a reason,
// which is recorded. Resolves true to go on.
const CHECK_MARK = { pass: "✔", warn: "!", fail: "✕", skipped: "–" };
function precheckTable(report) {
  return h("table", { class: "verifyTbl checkTbl" },
    ...report.items.map((i) => h("tr", { class: `chk-${i.result}` },
      h("td", { class: "chkIcon" }, CHECK_MARK[i.result] || "?"), h("td", {}, h("b", {}, i.label)), h("td", { style: "white-space:normal" }, i.detail))));
}
async function runPrecheck(utm) {
  if (!wf.editing?.job_instance_id) return true; // not a workflow job: the commit rules still apply
  const id = wf.editing.job_instance_id;
  status("System check: overlaps here and in Aumentum, drop point, title on the land, Registry transaction…");
  const wait = h("div", { class: "toast" }, "Running the system check…"); document.body.append(wait);
  let report;
  try { report = await api(`/api/v1/wf/jobs/${id}/precheck`, { method: "POST", body: { vertices: utm, editing_id: edit.editingId || null } }); }
  catch (e) { wait.remove(); await uiAlert("The system check could not run: " + e.message + "\nThe sketch is kept; try Finish Sketch again."); return false; }
  wait.remove();
  if (report.ok && !report.warnings) { notify("System check passed: the plot is free, no overlap, no title on the land."); return true; }
  const fails = report.items.filter((i) => i.result === "fail").length;
  const body = h("div", {},
    h("p", {}, report.ok ? "The system check found warnings. Look at them before going on." : h("b", {}, `The system check failed (${fails} problem${fails > 1 ? "s" : ""}). Nothing can be committed on this land.`)),
    h("div", { class: "verifyWrap" }, precheckTable(report)));
  const buttons = report.ok
    ? [{ label: "Back to sketch", value: false, cancel: true }, { label: "Continue…", value: true, primary: true }]
    : [{ label: "Back to sketch", value: false, cancel: true, primary: true }];
  const go = await uiDialog({ title: "System Check", message: body, kind: report.ok ? "warn" : "error", width: 720, buttons });
  if (!go || !report.ok) return false;
  const reason = await uiPrompt("Why go on despite the warnings? (recorded with the job)", "", { title: "System Check" });
  if (!reason || !reason.trim()) return false;
  try { await api(`/api/v1/wf/jobs/${id}/precheck`, { method: "POST", body: { vertices: utm, editing_id: edit.editingId || null, acknowledge: reason.trim() } }); }
  catch (e) { await uiAlert("Going on was refused: " + e.message); return false; }
  return true;
}

// --- Confirmation from the Registry (10.10.10.5) after a commit ---
// Read back, read-only: the committed parcels readable by the Registry, the job the Registry finds
// for its transaction, the transaction, the property and the Registry's job record. Kept on the job;
// Re-check asks again (e.g. when the Registry was down, or after the admin linked).
async function registryConfirmDialog(jobId, opts = {}) {
  for (;;) {
    const wait = h("div", { class: "toast" }, "Asking the Registry (10.10.10.5)…"); document.body.append(wait);
    let c;
    try { c = await api(`/api/v1/wf/jobs/${jobId}/registry-confirmation`, { method: "POST" }); }
    catch (e) { wait.remove(); await uiAlert("The Registry confirmation could not run: " + e.message); return; }
    wait.remove();
    const when = fmtDT(c.checked_at);
    const body = h("div", {},
      opts.headline ? h("p", {}, h("b", {}, opts.headline)) : null,
      h("p", {}, c.ok ? h("b", { style: "color:#138a2a" }, "Confirmed by the Registry.") : h("b", { style: "color:#c62020" }, "Not confirmed: see the crosses below."),
        ` Checked ${when}${c.checked_by ? " by " + c.checked_by : ""}.`),
      h("div", { class: "verifyWrap" }, precheckTable(c)));
    const again = await uiDialog({ title: "Registry Confirmation", message: body, kind: c.ok ? "info" : "error", width: 720,
      buttons: [{ label: "Re-check", value: true }, { label: "Close", value: false, primary: true, cancel: true }] });
    if (!again) return c;
  }
}

// --- Edit Sketch Properties window (live X/Y vertex table) ---
function openSketchProps() {
  const tbl = h("table", { class: "legs" }, h("tr", {}, h("th", {}, "#"), h("th", {}, "Beacon"), h("th", {}, "X (Easting)"), h("th", {}, "Y (Northing)"), h("th", {}, "")));
  const known = sketchIdent.key === vertsKey(edit.verts) ? sketchIdent.list : null;
  let newNo = 0;
  edit.verts.forEach(([E, N], i) => {
    const id = known && known[i];
    const bcell = h("td", { class: id ? `src-${id.source}` : "muted", title: id ? (id.source === "shared" ? `Shared corner (${id.shared_with.join(", ")})` : CORNER_SOURCE[id.source]) : "Checking…" },
      id ? cornerLabel(id, id.source !== "beacon" && !pointIds.get(ptKey(id.easting, id.northing)) ? ++newNo : 0) : "…");
    const x = h("input", { type: "number", step: "0.001", value: E.toFixed(3), style: "width:92px" });
    const y = h("input", { type: "number", step: "0.001", value: N.toFixed(3), style: "width:92px" });
    x.oninput = () => { const v = parseFloat(x.value); if (isFinite(v)) { edit.verts[i][0] = v; drawSketch(); } };
    y.oninput = () => { const v = parseFloat(y.value); if (isFinite(v)) { edit.verts[i][1] = v; drawSketch(); } };
    const tr = h("tr", { class: i === edit.sel ? "selrow" : "" }, h("td", { class: "n" }, String(i + 1)), bcell, h("td", {}, x), h("td", {}, y),
      h("td", {}, h("button", { title: "Delete vertex", onclick: () => { edit.verts.splice(i, 1); edit.sel = Math.min(edit.sel, edit.verts.length - 1); drawSketch(); openSketchProps(); updateEditorButtons(); } }, "✕")));
    tr.addEventListener("click", (ev) => { if (!/INPUT|BUTTON/.test(ev.target.tagName)) { edit.sel = i; drawSketch(); openSketchProps(); } });
    tbl.append(tr);
  });
  if (!edit.verts.length) tbl.append(h("tr", {}, h("td", { colspan: 5, class: "muted" }, "No points yet: click the corners on the map. Beacons and shared corners are recognised as you click.")));
  scheduleIdentify();
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
  // Digitising needs no point list: corners come from the map clicks. Typing or pasting
  // coordinates (COGO, a surveyor's list) stays one click away; the choice is remembered.
  let showEntry = false; try { showEntry = localStorage.getItem("cad.sketchEntry") === "1"; } catch { /* storage unavailable */ }
  const entry = h("div", { class: showEntry ? "" : "hidden" },
    h("div", { class: "frow" }, xIn, yIn, h("button", { class: "btn", style: "min-width:0", onclick: addXY }, "Add")),
    h("div", { class: "frow" }, bIn, h("button", { class: "btn", style: "min-width:0", onclick: addBeacon }, "Add beacon")),
    paste, h("div", { class: "frow" }, h("button", { class: "btn", onclick: addPasted }, "Add pasted points")));
  const entryToggle = h("a", { href: "#", class: "linkish", style: "display:inline-block;margin:6px 0 2px", onclick: (ev) => {
    ev.preventDefault(); const on = entry.classList.toggle("hidden") === false;
    try { localStorage.setItem("cad.sketchEntry", on ? "1" : "0"); } catch { /* storage unavailable */ }
    ev.target.textContent = on ? "Hide coordinate entry" : "Type / paste coordinates…";
  } }, showEntry ? "Hide coordinate entry" : "Type / paste coordinates…");
  const body = h("div", {},
    h("div", { class: "muted" }, `${edit.editingId ? "Editing draft parcel — " : ""}${edit.template === "jobArea" ? "job area" : edit.geomType} sketch — ${edit.verts.length} vertex(es), WGS 84 / UTM 32N metres`),
    h("div", { style: "max-height:260px;overflow:auto" }, tbl),
    entryToggle, entry,
    h("div", { class: "taskNav" },
      h("button", { class: "btn", disabled: !canFinishSketch(), onclick: finishSketch }, edit.editingId ? "Save Changes" : "Finish Sketch"),
      h("button", { class: "btn", disabled: !edit.verts.length, onclick: deleteSketch }, edit.editingId ? "Cancel Edit" : "Delete Sketch"),
      edit.editingId ? h("button", { class: "btn", onclick: deleteDraftBeingEdited }, "Delete Parcel") : null));
  floatWindow("Edit Sketch Properties", body, { id: "sketchPropsWin", width: 430 });
}
function refreshSketchProps() { if ($("sketchPropsWin")) openSketchProps(); }
async function deleteDraftBeingEdited() {
  if (!edit.editingId || !state.job || !(await uiConfirm("Delete this draft parcel from the job?", { ok: "Delete", kind: "warn" }))) return;
  try {
    await api(`/api/v1/jobs/${state.job.id}/parcels/${edit.editingId}`, { method: "DELETE" });
    clearSketch(); await loadDrafts(); wfRefreshEditing(); refreshSketchProps(); updateEditorButtons(); status("Draft parcel deleted.");
  } catch (e) { uiAlert("Delete failed: " + e.message); }
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
  if (!state.job) { uiAlert("Choose the job first (Workflow → Work On This Job)."); return; }
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
  if (!state.job) { uiAlert("Choose the job first (Workflow → Work On This Job)."); return; }
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
  uiAlert("Open the job in the Workflow panel and choose Work On This Job first."); return null;
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
  if (u.length !== 1) { uiAlert(`Select the one parcel to ${verb} first (Select tool).`); return; }
  if (!needWorkflowJob()) return;
  if (!edit.active) startEditing("sketch");
  clearSketch(); selectTemplate("line"); edit.splitMode = kind; setTool("sketch"); openSketchProps();
  status(`${verb[0].toUpperCase() + verb.slice(1)} ${u[0]}: click the line's points across the parcel (or type them), then Finish Sketch.`);
}
async function mutationSplit() {
  const kind = edit.splitMode === "separate" ? "separate" : "split";
  const u = selectedUpins();
  if (u.length !== 1) { uiAlert(`Select the one parcel to ${kind} (Select tool).`); return; }
  if (edit.verts.length < 2) { uiAlert("Sketch the line across the parcel first."); return; }
  const line = { type: "LineString", coordinates: edit.verts.map((v) => [...v]) };
  edit.splitMode = false;
  await recordMutation({ kind, upins: u, line }, (r) => `${MUTATION_LABEL[kind]} of ${u[0]} recorded: ${r.drafts.length} new parcels (drafts). They replace it when the job's parcels are committed.`);
}
async function mutationMerge() {
  const u = selectedUpins();
  if (u.length < 2) { uiAlert("Select two or more neighbouring parcels to merge (Select tool, Shift-click to add)."); return; }
  if (!(await uiConfirm(`Merge ${u.join(" + ")} into one parcel?`))) return;
  await recordMutation({ kind: "merge", upins: u }, `Merge of ${u.join(" + ")} recorded (a draft). It replaces them when the job's parcels are committed.`);
}
// Aumentum: an adjustment takes n adjacent parcels to n (each new parcel's parents: all n); a
// free mutation X parcels to Y (reshape, separate or delete the copies). New UPINs for all.
async function mutationResurvey() {
  const u = selectedUpins();
  if (!u.length) { uiAlert("Select the parcel(s) to adjust (Select tool; Shift-click to add adjacent parcels)."); return; }
  await recordMutation({ kind: "resurvey", upins: u }, `Adjustment of ${u.join(" + ")} recorded: edit the ${u.length > 1 ? "copies" : "copy"} with the Edit tool; ${u.length > 1 ? "they replace them" : `it replaces ${u[0]}`} (new UPINs) at commit.`);
}
async function mutationFree() {
  const u = selectedUpins();
  if (!u.length) { uiAlert("Select the parcel(s) for the free mutation (Select tool; Shift-click to add adjacent parcels)."); return; }
  await recordMutation({ kind: "free", upins: u }, `Free mutation of ${u.join(" + ")} recorded: reshape, separate or delete the copies; they replace ${u.join(" + ")} (new UPINs) at commit.`);
}
// Aumentum's Separation Configuration: which part keeps the parent's UPIN and attributes.
async function mutationKeepUpin(jobId, m, draft) {
  try { await api(`/api/v1/wf/jobs/${jobId}/mutations/${m.group}/keep-upin`, { method: "PUT", body: { draft: draft || null } }); await afterMutation(draft ? `The chosen part keeps ${m.parents[0]}.` : "Every part gets a new UPIN."); }
  catch (e) { notify(e.message, "err"); }
}
function mutationRetire() {
  const u = selectedUpins();
  if (!u.length) { uiAlert("Select the parcel(s) to retire (Select tool)."); return; }
  if (!needWorkflowJob()) return;
  const reason = h("input", { placeholder: "e.g. RE-SURVEYED, overlapping, WRONGLY DONE", style: "flex:1" });
  const go = async () => { if (!reason.value.trim()) { reason.focus(); return; } w.remove(); await recordMutation({ kind: "retire", upins: u, reason: reason.value.trim() }, `Retirement of ${u.join(", ")} recorded; applied when the job is committed.`); };
  reason.addEventListener("keydown", (ev) => ev.key === "Enter" && go());
  const w = floatWindow("Retire Parcel", h("div", {}, h("p", { style: "margin-top:0" }, `Retire ${u.join(", ")}: archived with this reason (Aumentum's Archive Reason). The UPIN is never reused.`),
    h("div", { class: "frow" }, h("label", {}, "Reason:"), reason), h("div", { class: "frow", style: "justify-content:flex-end" }, h("button", { class: "btn", onclick: go }, "Retire"))), { id: "retireWin", width: 400 });
  setTimeout(() => reason.focus(), 0);
}
async function lineageOfSelection() {
  const u = selectedUpins();
  if (u.length) { lineageWindow(u[0]); return; }
  const v = await uiPrompt("Parcel UPIN:", ""); if (v && v.trim()) lineageWindow(v.trim());
}
// Lineage: where a parcel came from and what it became (Aumentum's history and mutations made here).
async function lineageWindow(upin) {
  let d;
  try { d = await api(`/api/v1/parcels/lineage?upin=${encodeURIComponent(upin)}`); } catch (e) { uiAlert(e.message); return; }
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
    for (const s of d.shapes) t.append(h("tr", {}, h("td", {}, s.upin), h("td", {}, s.status === "posted" ? "active" : s.status), h("td", {}, area2(s.area)),
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
async function addTextGraphic(ll) { const text = await uiPrompt("Text:", "Text"); if (text) addGraphic({ type: "text", at: [ll.lng, ll.lat], text }); }
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
  if (on) { showRegistry(false); showCompleted(false); }
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
/* =====================================================================================
 * Completed tab: every completed job (this editor's and Aumentum's), newest first, with its
 * metadata — Registry transaction and property, UPIN(s), committed / linked date and time, the
 * plan sent — and what the Registry (10.10.10.5) holds for it, read live for the page shown.
 * A committed job is completed (the end of digitising). From here: Title deed plan, Registry
 * confirmation, Withdraw plan (the parcel stays), and Correct (reopen with the parcel back as a
 * draft keeping its UPIN; commit again, then resend the corrected plan).
 * ===================================================================================== */
const done = { q: "", source: "", jobs: [], total: 0, offset: 0, busy: false, err: "", regErr: "", loaded: false };
const doneOpen = () => !!$("doneBody") && !$("doneBody").classList.contains("hidden");
function showCompleted(on) {
  const body = $("doneBody"); if (!body) return;
  const pane = $("tocPane");
  if (on && !done.widened && pane.getBoundingClientRect().width < 380) { done.widened = pane.style.width || "auto"; pane.style.width = "380px"; setTimeout(() => map.resize(), 0); }
  if (!on && done.widened) { pane.style.width = done.widened === "auto" ? "" : done.widened; done.widened = null; setTimeout(() => map.resize(), 0); }
  body.classList.toggle("hidden", !on);
  const title = document.querySelector("#tocPane .paneHead > span");
  if (on) {
    if (title) title.textContent = "Completed Jobs";
    ["wfBody", "tasksBody", "regBody", "tocBody"].forEach((id) => $(id).classList.add("hidden"));
    document.querySelector("#tocPane .hscroll").classList.add("hidden");
  } else if (!tasksOpen() && !wfOpen() && !regOpen()) {
    if (title) title.textContent = "Table Of Contents";
    $("tocBody").classList.remove("hidden"); document.querySelector("#tocPane .hscroll").classList.remove("hidden");
  }
}
async function openCompleted() {
  if ($("tocPane").classList.contains("hidden")) togglePane("tocPane");
  showWorkflow(false); showRegistry(false); showTasks(false);
  showCompleted(true); buildTocTools();
  if (!done.loaded) await doneLoad(true); else renderCompleted();
}
// --- File history: the Registry's stages and this Cadastre's own events, one timeline, read only ---
function historyText(e) {
  const d = e.detail || {};
  switch (e.kind) {
    case "wf_commit": return `Committed: ${(d.parcel_numbers || []).join(", ") || "—"}${d.retired && d.retired.length ? ` (replaced ${d.retired.join(", ")})` : ""}`;
    case "registry_confirmation": return `Registry confirmation: ${d.ok ? "confirmed" : "not confirmed"}`;
    case "registry_link_made": return `Linked to the Registry property ${d.su_id || ""}${d.how === "commit" ? " (on commit)" : ""}`;
    case "registry_link_reverted": return `Link removed (${d.su_id || ""})`;
    case "registry_link_synced": return `Linked in the Registry: property ${d.su_id || ""}`;
    case "registry_unlink_synced": return "Unlinked in the Registry";
    case "registry_link_failed": return `Could not be linked: ${d.reason || ""}`;
    case "registry_plan_sent": return `Title deed plan ${d.resend ? "resent (corrected)" : "sent"} to the Registry: ${d.file_name || ""}${d.reason ? ` (${d.reason})` : ""}`;
    case "title_deed_plan_correction": return `Title deed plan correction started: ${d.reason || ""}`;
    case "registry_plan_withdrawn": return `Title deed plan withdrawn: ${d.reason || ""}`;
    case "title_deed_plan_deleted": return "Title deed plan deleted";
    case "wf_reopened_for_correction": return `Reopened for correction (${d.upin || ""}): ${d.reason || ""}`;
    case "wf_commit_reverted": return `Commit undone (${d.upin || ""})`;
    case "wf_job_area_save": return "Job area saved";
    case "precheck_acknowledged": return `System check warnings accepted: ${d.reason || ""}`;
    case "duplicate_upin_found": return `Duplicate UPIN reported: ${d.upin || ""}`;
    default: return e.text || e.kind.replace(/_/g, " ");
  }
}
async function historyWindow(opts) {
  const q = new URLSearchParams(); for (const k of ["txn", "job", "upin"]) if (opts[k]) q.set(k, opts[k]);
  const body = h("div", {}, h("p", { class: "muted" }, "Reading the history…"));
  floatWindow(`History — ${opts.title || opts.txn || opts.upin || "job " + opts.job}`, body, { id: "historyWin", width: 720 });
  let d;
  try { d = await api("/api/v1/registry/history?" + q); } catch (e) { body.innerHTML = ""; body.append(h("p", { style: "color:#b00" }, e.message)); return; }
  const r = d.registry || {};
  const rows = [];
  for (const st of r.stages || []) rows.push({ at: st.started, src: "Registry", what: `${st.workflow} → ${st.stage_label}`, by: st.by,
    extra: st.finished ? `finished ${fmtDT(st.finished)}${st.next ? " → " + st.next : ""}` : "open now" });
  for (const j of r.cadastre_jobs || []) rows.push({ at: j.started, src: "Registry", what: `Cadastre job ${j.name} (#${j.job}) recorded`, by: "", extra: j.ended ? `completed ${fmtDT(j.ended)}` : "not completed" });
  for (const e of d.cadastre || []) rows.push({ at: e.at, src: "Cadastre", what: historyText(e), by: e.by || "", extra: e.end ? `finished ${fmtDT(e.end)}` : e.note || "" });
  rows.sort((a, b) => String(b.at || "").localeCompare(String(a.at || "")));
  const cur = (r.current || []).map((c) => `${c.workflow} → ${c.stage_label} (since ${fmtDT(c.started)})`).join("; ");
  const props = (r.properties || []).map((p) => p.upin ? `${p.property_number || p.su_id}: linked to ${p.upin}` : `${p.property_number || p.su_id}: not linked yet`).join("; ");
  const tbl = h("table", { class: "verifyTbl histTbl" }, h("tr", {}, h("th", {}, "When"), h("th", {}, "Where"), h("th", {}, "What"), h("th", {}, "By"), h("th", {}, "")));
  for (const x of rows) tbl.append(h("tr", { class: "src-" + x.src.toLowerCase() }, h("td", { style: "white-space:nowrap" }, fmtDT(x.at) || "—"),
    h("td", {}, h("span", { class: "histSrc " + x.src.toLowerCase() }, x.src)), h("td", { style: "white-space:normal" }, x.what), h("td", {}, x.by || ""), h("td", { class: "muted", style: "white-space:normal" }, x.extra || "")));
  const text = [`History of ${d.transaction_number || opts.upin || ""}`, cur ? `Now at: ${cur}` : "", props, ...rows.map((x) => `${fmtDT(x.at)}\t${x.src}\t${x.what}\t${x.by || ""}\t${x.extra || ""}`)].filter(Boolean).join("\n");
  body.innerHTML = "";
  body.append(
    h("div", { class: "histHead" },
      h("div", {}, h("b", {}, "File: "), d.transaction_number || "— (no Registry transaction)", d.upins && d.upins.length ? ` · UPIN ${d.upins.join(", ")}` : ""),
      !d.transaction_number ? null
        : !r.reachable ? h("div", { style: "color:#a35a00" }, r.error || "The Registry (10.10.10.5) can't be reached: its stages aren't shown.")
        : r.found === false ? h("div", { class: "muted" }, "The Registry has no such transaction.")
        : h("div", {}, h("b", {}, "Now at: "), cur || "no open stage (finished in the Registry)", h("br"), h("b", {}, "Link: "), props || "—")),
    h("div", { class: "verifyWrap" }, tbl),
    h("div", { class: "frow", style: "margin-top:6px;gap:4px" },
      h("button", { class: "btn", onclick: () => historyWindow(opts) }, "Refresh"),
      h("button", { class: "btn", onclick: async (ev) => { try { await navigator.clipboard.writeText(text); ev.target.textContent = "Copied"; } catch { status("Select the table to copy it."); } } }, "Copy")));
}

// The title deed queue: files the Registry has at Raise Title Deed Plan, linked by the Registry admin.
async function queueLoad() {
  done.busy = true; done.err = ""; renderCompleted();
  try { done.queue = await api("/api/v1/registry/title-deed-queue"); done.queueLoaded = true; } catch (e) { done.err = e.message; done.queue = null; }
  done.busy = false; renderCompleted();
}
function queueCard(f) {
  const stageAge = f.stage_since ? fmtDT(f.stage_since) : "—";
  const job = f.job_instance_id ? `#${f.job_instance_id} (${f.job_status})` : "none yet";
  const plan = f.plan ? `${f.plan.action === "registry_plan_withdrawn" ? "Withdrawn" : "Sent"} ${fmtDT(f.plan.at)}` : "Not sent from this editor";
  const actions = [];
  if (f.in_cadastre) actions.push(regLink("Zoom", "Zoom to the parcel", () => registryZoom(f.upin)));
  if (can("titledeed")) actions.push(regLink("Title deed plan", "Open (or start) the file's title deed job and its plan", async () => {
    if (f.job_instance_id && f.job_open) { await openWorkflow(); await wfSelect(f.job_instance_id); titleDeedDialog(f.job_instance_id, f.upin); }
    else startEditParcel(f.upin);
  }, !f.in_cadastre));
  if (can("titledeed")) actions.push(regLink("Correct plan", "The plan came back: give the reason, change it and resend it (the parcel stays)", () => correctPlan(f.upin, f.transaction_number), !f.in_cadastre));
  actions.push(regLink("Point List", "Beacons, bearings and distances", () => pointListWindow(f.upin), !f.in_cadastre));
  actions.push(regLink("History", "The file's Registry stages and this Cadastre's events", () => historyWindow({ txn: f.transaction_number, upin: f.upin, title: f.property_number })));
  return h("div", { class: "regCard " + (f.in_cadastre ? "job" : "wait") },
    h("div", { class: "regCardTop" }, h("span", { class: "regUpin" }, f.property_number || f.transaction_number), h("span", { class: "regPill ok" }, f.stage_label || f.stage)),
    regStatus(f.in_cadastre ? "job" : "wait", f.in_cadastre ? `Linked to ${f.upin}` : `Linked to ${f.upin}, which is not a parcel in this Cadastre`),
    h("div", { class: "regKv" },
      h("div", {}, h("i", {}, "Transaction"), f.transaction_number),
      h("div", {}, h("i", {}, "Property"), f.su_id),
      h("div", {}, h("i", {}, "Linked"), f.linked_at ? fmtDT(f.linked_at) : "— (no link date in the Registry)"),
      h("div", {}, h("i", {}, "At stage since"), stageAge),
      h("div", {}, h("i", {}, "Job here"), job),
      h("div", {}, h("i", {}, "Plan"), plan)),
    h("div", { class: "regCardFoot" }, h("span", { class: "regLinks" }, ...actions)));
}
async function doneLoad(reset) {
  if (done.source === "queue") { await queueLoad(); return; }
  if (reset) { done.offset = 0; done.jobs = []; }
  done.busy = true; done.err = ""; renderCompleted();
  try {
    const q = new URLSearchParams({ q: done.q, source: done.source, offset: String(done.offset), limit: "50", registry: "1" });
    const r = await api("/api/v1/wf/completed?" + q);
    done.jobs = done.jobs.concat(r.jobs); done.total = r.total; done.offset = done.jobs.length; done.regErr = r.registry_error || ""; done.loaded = true;
  } catch (e) { done.err = e.message; }
  done.busy = false; renderCompleted();
}
const doneParcels = (j) => (j.parcels || []).filter((p) => p && p.upin);
function doneCard(j) {
  const parcels = doneParcels(j);
  const reg = j.registry;
  const upins = parcels.map((p) => p.upin);
  const firstUpin = upins[0] || reg?.upin || null;
  const committedAt = parcels[0]?.committed_at || j.completed;
  const linked = parcels.filter((p) => p.linked);
  const plan = j.plan;
  const planText = !plan ? "Not sent from this editor"
    : plan.action === "registry_plan_withdrawn" ? `Withdrawn ${fmtDT(plan.at)}${plan.by ? " by " + plan.by : ""}${plan.reason ? " — " + plan.reason : ""}`
    : `${plan.resend ? "Corrected plan resent" : "Sent"} ${fmtDT(plan.at)}${plan.by ? " by " + plan.by : ""}${plan.file ? " (" + plan.file + ")" : ""}`;
  const regText = !reg ? (done.regErr ? "Registry not reached" : "—")
    : !reg.found ? "Not found in the Registry"
    : `${reg.status}; property ${reg.property || "—"}${reg.refnum ? " (" + reg.refnum + ")" : ""}${reg.upin ? `, holds ${reg.upin}` : ", no UPIN yet"}${reg.linked_at ? `, linked ${fmtDT(reg.linked_at)}` : ""}`;
  const mine = j.source === "cadastre";
  const actions = [];
  if (firstUpin) actions.push(regLink("Zoom", "Zoom to the parcel", () => registryZoom(firstUpin)));
  actions.push(regLink("Open job", "Open the job in the Workflow", async () => { await openWorkflow(); await wfSelect(j.id); }));
  actions.push(regLink("History", "The file's Registry stages and this Cadastre's events", () => historyWindow({ job: j.id, txn: j.transaction_number, title: j.property_number || j.name })));
  if (mine) actions.push(regLink("Registry check", "Ask the Registry whether the committed parcels are visible there", () => registryConfirmDialog(j.id)));
  if (mine && upins.length && can("titledeed")) actions.push(regLink("Title deed plan", "Open the title deed plan (form, sheet, PDF, send)", () => titleDeedDialog(j.id, upins[0])));
  if (mine && plan && plan.action === "registry_plan_sent" && can("titledeed")) actions.push(regLink("Withdraw plan", "Mark the plan sent to the Registry as withdrawn (the parcel and UPIN stay)", () => doneWithdraw(j)));
  if (mine && upins.length && parcels.some((p) => !p.linked) && j.transaction_number && can("admin")) actions.push(regLink("Link now", "Write the UPIN and link date onto the Registry file (as on commit)", () => doneLinkNow(j)));
  if (j.can_correct && upins.length && can("digitise")) actions.push(regLink("Correct", "Reopen the job to fix the parcel (same UPIN), then commit again and resend the plan", () => doneCorrect(j, parcels)));
  const conf = j.confirmation;
  return h("div", { class: "regCard " + (mine ? "job" : "gone"), title: "Double-click to zoom", ondblclick: () => firstUpin && registryZoom(firstUpin) },
    h("div", { class: "regCardTop" }, h("span", { class: "regUpin" }, j.property_number || j.name || "#" + j.id),
      h("span", { class: "regPill " + (mine ? "ok" : "gone") }, mine ? "Plagis editor" : "Aumentum")),
    regStatus(linked.length ? "job" : "wait", linked.length ? `Linked: ${linked.map((p) => p.upin).join(", ")}` : upins.length ? `Committed ${upins.join(", ")} · not linked yet` : "Completed (no parcel found)"),
    h("div", { class: "regKv" },
      h("div", {}, h("i", {}, "Job"), `${j.name || ""} (#${j.id}) · ${j.job_type}`),
      h("div", {}, h("i", {}, "Transaction"), j.transaction_number || "—"),
      h("div", {}, h("i", {}, "Committed"), committedAt ? `${fmtDT(committedAt)}${j.completed_by && mine ? " by " + j.completed_by : ""}` : "—"),
      linked.length ? h("div", {}, h("i", {}, "Linked"), linked.map((p) => `${p.upin} ${p.linked_at ? fmtD(p.linked_at) : ""}${p.lrs_id ? " → " + p.lrs_id : ""}`).join("; ")) : null,
      h("div", {}, h("i", {}, "Plan"), planText),
      h("div", {}, h("i", {}, "Registry"), regText),
      reg && reg.stages && reg.stages.length ? h("div", {}, h("i", {}, "Registry stage"), reg.stages.map((x) => `${x.workflow} → ${x.stage} (since ${fmtDT(x.since)})`).join("; ")) : null,
      conf ? h("div", {}, h("i", {}, "Confirmed"), `${conf.ok ? "✔" : "✕"} ${fmtDT(conf.at)}`) : null),
    h("div", { class: "regCardFoot" }, h("span", { class: "regLinks" }, ...actions)));
}
function renderCompleted() {
  const box = $("doneBody"); if (!box) return;
  box.innerHTML = "";
  const q = h("input", { class: "tinput", value: done.q, placeholder: "File, transaction or UPIN (e.g. PL68928)", style: "flex:1;min-width:0" });
  const run = () => { done.q = q.value.trim(); if (done.source === "queue" && done.queueLoaded) renderCompleted(); else doneLoad(true); };
  q.addEventListener("keydown", (e) => e.key === "Enter" && run());
  const src = h("select", { onchange: (e) => { done.source = e.target.value; doneLoad(true); } },
    ...[["queue", "Title deed queue (Registry)"], ["", "All completed"], ["cadastre", "Plagis editor"], ["aumentum", "Aumentum"]].map(([v, l]) => h("option", { value: v, selected: done.source === v }, l)));
  box.append(...[h("div", { class: "frow", style: "margin:6px 8px 2px" }, q, h("button", { class: "btn", style: "min-width:0", onclick: run }, "Find"), src),
    done.source === "queue" ? null : h("div", { class: "muted", style: "margin:0 8px 4px" }, done.busy && !done.jobs.length ? "Loading…" : `${done.total.toLocaleString()} completed job(s)${done.q ? ` matching “${done.q}”` : ""}, newest first`),
    done.regErr && done.source !== "queue" ? h("div", { class: "muted", style: "margin:0 8px 4px;color:#a35a00" }, `Registry details not shown: ${done.regErr}`) : null,
    done.err && done.source !== "queue" ? h("div", { style: "margin:0 8px 4px;color:#b00" }, done.err) : null].filter(Boolean));
  if (done.source === "queue") {
    const qv = normName(done.q);
    const all = done.queue?.files || [];
    const files = all.filter((f) => !qv || normName(`${f.property_number} ${f.transaction_number} ${f.upin} ${f.su_id} ${f.stage_label}`).includes(qv));
    box.append(h("div", { class: "muted", style: "margin:0 8px 4px" }, done.busy ? "Reading the Registry…" : done.queue
      ? `${files.length.toLocaleString()} file(s) at Raise Title Deed Plan, linked by the Registry admin${qv ? ` matching “${done.q}”` : ""} (of ${done.queue.at_stage.toLocaleString()} at that stage) · read ${fmtDT(done.queue.read_at)}`
      : ""));
    if (done.err) box.append(h("div", { style: "margin:0 8px 4px;color:#b00" }, done.err));
    for (const f of files.slice(0, 300)) box.append(queueCard(f));
    if (files.length > 300) box.append(h("div", { class: "muted", style: "margin:6px 8px" }, `Showing 300 of ${files.length}: search to narrow.`));
    return;
  }
  for (const j of done.jobs) box.append(doneCard(j));
  if (done.jobs.length < done.total) box.append(h("div", { class: "frow", style: "margin:6px 8px 10px" },
    h("button", { class: "btn", disabled: done.busy, onclick: () => doneLoad(false) }, done.busy ? "Loading…" : `More (${(done.total - done.jobs.length).toLocaleString()} left)`)));
}
async function doneLinkNow(j) {
  if (!(await uiConfirm(`Link ${j.property_number || j.name}'s committed parcel(s) to Registry file ${j.transaction_number}?\nThe Registry property gets the UPIN and today's link date, as Map → Connect does.`, { title: "Link now", ok: "Link" }))) return;
  try {
    const out = await api(`/api/v1/wf/jobs/${j.id}/registry/link`, { method: "POST", body: {} });
    const msg = linkSummary(out);
    notify(msg || "Nothing to link.", out.some((l) => !l.linked) ? "warn" : "ok");
    await doneLoad(true);
  } catch (e) { uiAlert("Link failed: " + e.message); }
}
async function doneWithdraw(j) {
  const reason = await uiPrompt(`Withdraw the title deed plan of ${j.property_number || j.name} from Registry transaction ${j.transaction_number}?\n\nThe plan documents in the transaction are marked WITHDRAWN (nothing is deleted in the Registry) and the saved title deed form is cleared. The parcel and its UPIN stay. Reason:`, "", { title: "Withdraw Plan", ok: "Withdraw" });
  if (!reason || !reason.trim()) return;
  try {
    const r = await api(`/api/v1/wf/jobs/${j.id}/registry/withdraw-plan`, { method: "POST", body: { reason: reason.trim() } });
    notify(`Plan withdrawn in ${r.transaction_number} (${r.withdrawn.length} document(s) marked).` + (r.failed.length ? ` Not marked: ${r.failed.join("; ")}.` : ""), r.failed.length ? "warn" : "ok");
    await doneLoad(true);
  } catch (e) { uiAlert("Withdraw failed: " + e.message); }
}
async function doneCorrect(j, parcels) {
  let upin = parcels[0].upin;
  if (parcels.length > 1) {
    const pick = await uiPrompt(`Which parcel of ${j.name} to correct? (${parcels.map((p) => p.upin).join(", ")})`, upin, { title: "Correct" });
    if (!pick) return; upin = pick.trim().toUpperCase();
  }
  if (j.source === "aumentum") { await correctAndRecommit(upin); done.loaded = false; return; }
  const reason = await uiPrompt(`Correct ${upin}?\n\nThe job is reopened at its commit step with ${upin} back as a draft (it keeps this UPIN). Fix the shape, beacons or attributes, commit again, then resend the corrected plan. Refused if the parcel is linked in the Registry (unlink it there first). Reason:`, "", { title: "Correct", ok: "Reopen to correct" });
  if (!reason || !reason.trim()) return;
  try {
    const r = await api(`/api/v1/wf/jobs/${j.id}/correct`, { method: "POST", body: { upin, reason: reason.trim() } });
    for (const b of r.beacons || []) rememberPointId(b.easting, b.northing, b.number);
    done.loaded = false;
    await openWorkflow(); await wfSelect(j.id); await wfWorkOn(j.id);
    notify(`${upin} is a draft of ${j.name} again (it keeps its UPIN). Fix it, then Commit; resend the corrected plan after.`);
  } catch (e) { uiAlert("Correct refused: " + e.message); }
}

/* =====================================================================================
 * Phase 5 — Registry tab (apart from the Workflow). The server reads the Aumentum Registry every
 * minute and records each property the Registry admin links to a parcel (the parcel's link
 * fields follow at once), and unlinks every 15 minutes. From here: start the job for a link,
 * look up a transaction, link / unlink from the Cadastre, and send the plan to the transaction —
 * Aumentum's Registry extension (LinkParcel, SendImageCommand), writing to the Registry only on
 * those actions.
 * ===================================================================================== */
const reg = { links: [], status: null, busy: false, loaded: false, view: "inbox", filter: "all", q: "", findQ: "", found: null, findErr: "", finding: false,
  txn: null, txnNo: "", txnErr: "", txnLoading: false, job: null, files: [], sent: [], pick: null };
const registryWaiting = () => reg.links.filter((l) => !l.job_instance_id && l.parcel_id && l.linked_now).length;
const regOpen = () => !$("regBody").classList.contains("hidden");
function showRegistry(on) {
  // The Registry needs room: widen the pane while it is open, give the width back after.
  const pane = $("tocPane");
  if (on && !reg.widened && pane.getBoundingClientRect().width < 360) { reg.widened = pane.style.width || "auto"; pane.style.width = "360px"; setTimeout(() => map.resize(), 0); }
  if (!on && reg.widened) { pane.style.width = reg.widened === "auto" ? "" : reg.widened; reg.widened = null; setTimeout(() => map.resize(), 0); }
  $("regBody").classList.toggle("hidden", !on);
  const title = document.querySelector("#tocPane .paneHead > span");
  if (title) title.textContent = on ? "Registry" : "Table Of Contents";
  if (on) { showCompleted(false); $("wfBody").classList.add("hidden"); $("tasksBody").classList.add("hidden"); $("tocBody").classList.add("hidden"); document.querySelector("#tocPane .hscroll").classList.add("hidden"); }
  else if (!tasksOpen() && !wfOpen() && !doneOpen()) { $("tocBody").classList.remove("hidden"); document.querySelector("#tocPane .hscroll").classList.remove("hidden"); }
}
async function openRegistry(opts = {}) {
  if ($("tocPane").classList.contains("hidden")) togglePane("tocPane");
  showRegistry(true); buildTocTools();
  if (!wf.defs) { try { [wf.defs, wf.principals] = await Promise.all([api("/api/v1/wf/definitions"), api("/api/v1/wf/principals")]); } catch { /* job types load later */ } }
  if (opts.view) reg.view = opts.view;
  if (opts.job) await regSetJob(opts.job);
  if (opts.txn) await regLookup(opts.txn);
  await registryLoad(); renderRegistry();
}
async function registryLoad() {
  try { [reg.links, reg.status] = await Promise.all([api("/api/v1/registry/links?days=14"), api("/api/v1/registry/status")]); reg.loaded = true; }
  catch (e) { reg.status = { enabled: false, error: e.message }; }
  buildTocTools();
}
setInterval(async () => { await registryLoad(); if (regOpen() && !reg.busy) renderRegistry(); }, 60000);
// Aumentum not answering: commits are refused, so the units that commit are told before they try.
// It clears itself when Aumentum is back.
async function aumentumWarning() {
  if (!can("digitise") && !can("titledeed")) return;
  let s; try { s = await api("/api/v1/aumentum/status"); } catch { return; }
  const down = s.configured && !s.ok;
  let bar = $("aumWarn");
  if (down && !bar) {
    bar = h("div", { id: "aumWarn", role: "alert", title: s.reason || "" },
      h("b", {}, "Aumentum server is not responding."), " New commits are paused until it is back. Please contact the ICT Admin. You can keep working: your drafts stay as they are.");
    $("toolbars").after(bar);
  } else if (bar && !down) bar.remove();
  else if (bar) bar.title = s.reason || "";
  try { map.resize(); } catch { /* the map is not ready */ }
}
aumentumWarning();
setInterval(aumentumWarning, 60000);
setTimeout(registryLoad, 1500);
const regTime = (s) => (s ? s.replace("T", " ").slice(0, 16) : "—");
async function regRun(label, fn) {
  reg.busy = true; renderRegistry();
  try { await fn(); } catch (e) { notify(`${label}: ${e.message}`, "err"); }
  reg.busy = false; await registryLoad(); renderRegistry();
}
async function registryZoom(upin) {
  try { const r = await api("/api/v1/parcels/search?number=" + encodeURIComponent(upin)); if (r.length) goToResult(r[0]); else status(`${upin} is not in the fabric.`); }
  catch (e) { status(e.message); }
}
// A job for the transaction, as the Registry created them in Aumentum: job type 9 "Deed Plan
// Creationv2", named Job-<property number>, carrying the transaction number.
async function registryOpenJob(id) { await openWorkflow(); await wfSelect(id); }
// --- transaction lookup, link / unlink (Aumentum LinkageManager.LinkParcel) ---
async function regLookup(no) {
  reg.txnNo = (no || "").trim().toUpperCase(); reg.txn = null; reg.txnErr = "";
  if (!reg.txnNo) return;
  reg.txnLoading = true; if (regOpen()) renderRegistry();
  try { reg.txn = await api("/api/v1/registry/transactions/" + encodeURIComponent(reg.txnNo)); } catch (e) { reg.txnErr = e.message; }
  reg.txnLoading = false;
}
// --- send the plan (Aumentum SendImageCommand: the job's image file → its transaction) ---
async function regSetJob(id) {
  reg.job = null; reg.files = []; reg.sent = []; reg.correction = null;
  if (!id) return;
  try {
    const d = await api("/api/v1/wf/jobs/" + id);
    reg.job = d.job;
    // The plan goes to the Registry as a PDF; a JPG (earlier plan images) is sent as a PDF too.
    reg.files = (await api(`/api/v1/wf/jobs/${id}/files`)).filter((f) => /\.(pdf|jpe?g)$/i.test(f.file_name));
    const pdfs = reg.files.filter((f) => /\.pdf$/i.test(f.file_name));
    if (!reg.files.some((f) => f.id === reg.pick)) reg.pick = (pdfs.length ? pdfs[pdfs.length - 1] : reg.files[reg.files.length - 1] || {}).id || null;
    reg.sent = await api(`/api/v1/wf/jobs/${id}/registry/sent`);
    reg.correction = (await api(`/api/v1/wf/jobs/${id}/registry/plan-correction`)).correction;
    if (reg.job.lrs_transaction_number && !reg.txnNo) await regLookup(reg.job.lrs_transaction_number);
  } catch (e) { notify(e.message, "err"); }
}
async function regSendPlan(fileId, fileName, resend) {
  const t = reg.job.lrs_transaction_number;
  const asPdf = /\.pdf$/i.test(fileName) ? fileName : fileName.replace(/\.[^.]+$/, "") + ".pdf";
  const ask = resend
    ? `Resend the corrected plan ${asPdf} to Registry transaction ${t}?\n\nIt is filed as "Survey Plan (corrected)", and the plan sent before is marked as replaced. Nothing is deleted in the Registry.`
    : `Send the plan ${asPdf} to Registry transaction ${t}?\nIt is filed in the transaction as a Title Deed Plan (PDF).`;
  if (!(await uiConfirm(ask, { ok: resend ? "Resend" : "Send" }))) return;
  await regRun(resend ? "Resend plan" : "Send plan", async () => {
    const r = await api(`/api/v1/wf/jobs/${reg.job.id}/registry/send-plan`, { method: "POST", body: { file_id: fileId, resend: !!resend } });
    const done = `${r.file_name} ${r.resend ? "resent (corrected)" : "sent"} to ${r.transaction_number}.`;
    if (r.replace_note) notify(`${done} Note: ${r.replace_note}.`, "warn");
    else notify(r.resend ? `${done} ${r.replaced.length} earlier plan(s) marked as replaced.` : done);
    if (r.job_completed) notify(`${done} ${reg.job.name || "The job"} is completed.`);
    await regSetJob(reg.job.id);
  });
}
// ---- view ---------------------------------------------------------------------------------
const regDay = (iso) => {
  if (!iso) return "Earlier";
  const d = iso.slice(0, 10), t = new Date(), y = new Date(Date.now() - 864e5), f = (x) => x.toISOString().slice(0, 10);
  return d === f(t) ? "Today" : d === f(y) ? "Yesterday" : new Date(d + "T12:00:00Z").toLocaleDateString(undefined, { weekday: "short", day: "numeric", month: "short" });
};
const regClock = (iso) => (iso ? iso.slice(11, 16) : "");
const regKind = (l) => (!l.linked_now ? "unlinked" : l.job_instance_id ? "job" : l.parcel_id ? "waiting" : "nofabric");
const REG_KIND = { waiting: ["Digitised, linked: no title deed job yet", "wait"], job: ["Job started", "job"], unlinked: ["Unlinked in the Registry", "gone"], nofabric: ["Parcel not in the fabric", "gone"] };
function regPill(kind, text) { return h("span", { class: "regPill " + REG_KIND[kind][1] }, text || REG_KIND[kind][0]); }
// A card's status line: a coloured dot and words (fits any pane width).
function regStatus(cls, text) { return h("div", { class: "regStatus " + cls }, h("span", { class: "regStatusDot" }), text); }
function regLink(label, title, onclick, disabled) { return h("a", { class: "regLinkBtn" + (disabled ? " off" : ""), title, href: "#", onclick: (e) => { e.preventDefault(); e.stopPropagation(); if (!disabled) onclick(); } }, label); }
function regIconBtn(label, title, onclick, opts = {}) { return h("button", { class: "regAct" + (opts.primary ? " primary" : ""), title, disabled: !!opts.disabled, onclick: (e) => { e.stopPropagation(); onclick(); } }, label); }
function regTxnPill(code, label) { return h("span", { class: "regPill " + (code === "transaction_lodged" ? "ok" : "gone") }, label || code); }

function renderRegistry() {
  const body = $("regBody"); if (!body) return;
  const keep = body.querySelector(".regScroll")?.scrollTop || 0;
  body.innerHTML = "";
  const st = reg.status, last = st && st.last;
  const up = st && st.enabled && last && last.ok;
  const writes = st && st.write_enabled;

  // Header: connection, one refresh.
  body.append(h("div", { class: "regHead" },
    h("div", { class: "regConn" },
      h("span", { class: "regDot " + (!st ? "" : up ? "on" : "off") }),
      h("div", {}, h("b", {}, "Aumentum Registry"),
        h("div", { class: "muted" }, !st ? "Connecting…" : !st.enabled ? "Not configured on this server" :
          up ? `Connected · checked ${regClock(last.at)} UTC · every minute` : `Not reachable${last ? " · " + last.error : ""}`))),
    h("span", { class: "regHeadBtns" },
      regIconBtn(reg.busy ? "…" : "⟳", "Check the Registry now: new links (every minute) and unlinks (every 15 min)", () => regRun("Registry", async () => {
        const r = await api("/api/v1/registry/sync", { method: "POST" });
        const u = await api("/api/v1/registry/reconcile", { method: "POST" }).catch(() => null);
        notify([r.new ? `${r.new} new link(s)` : "No new links", u && u.unlinked ? `${u.unlinked} unlink(s)` : null].filter(Boolean).join(" · ") + ".");
      }), { disabled: reg.busy || !(st && st.enabled) }))));
  if (st && st.enabled && !writes) body.append(h("div", { class: "regNote" }, "Read-only: writing to the Registry (link, unlink, send plan) is turned off on this server."));
  // Parallel run: is each new UPIN checked with Aumentum, and when were duplicates last looked for.
  if (st) {
    const w = st.upin_watch;
    body.append(h("div", { class: "regNote" }, st.aumentum_check
      ? h("span", {}, "UPINs are checked with Aumentum before commit. ",
          w ? (w.ok ? `Duplicates checked ${regClock(w.at)} UTC (${w.checked} UPIN${w.checked === 1 ? "" : "s"}). ` : `Last duplicate check failed: ${w.error}. `) : "Duplicates not checked yet. ",
          regIconBtn(reg.busy ? "…" : "Check now", "Ask Aumentum now about every UPIN committed here", () => regRun("Duplicate check", async () => {
            const r = await api("/api/v1/registry/upin-watch", { method: "POST" });
            notify(r.ok ? (r.duplicates.length ? `${r.duplicates.length} duplicate UPIN(s) found.` : `No duplicates (${r.checked} UPIN(s) checked).`) : `Aumentum could not be asked: ${r.error}`, r.ok && !r.duplicates.length ? undefined : "warn");
          }), { disabled: reg.busy }))
      : "UPINs are not checked with Aumentum on this server (no parallel-run check configured)."));
  }
  // Parallel run: UPINs given here that Aumentum has since issued too (the duplicate watch).
  const dups = (st && st.upin_watch && st.upin_watch.duplicates) || [];
  if (dups.length) body.append(h("div", { class: "regNote", style: "background:#fde8e8;border-color:#c00;color:#900" },
    h("b", {}, `Duplicate UPIN${dups.length > 1 ? "s" : ""}: `),
    `${dups.map((d) => d.upin).join(", ")} ${dups.length > 1 ? "were" : "was"} committed here and Aumentum has issued ${dups.length > 1 ? "them" : "it"} too. `,
    "Do not link until it is resolved (revert the commit here, or correct the parcel in Aumentum)."));

  // Summary = filters.
  const counts = { waiting: 0, job: 0, unlinked: 0 };
  for (const l of reg.links) { const k = regKind(l); if (k in counts) counts[k]++; }
  const tile = (key, n, label, cls) => h("div", { class: `regTile ${cls}${reg.filter === key ? " sel" : ""}`, title: `Show ${label.toLowerCase()}`,
    onclick: () => { reg.filter = reg.filter === key ? "all" : key; reg.view = "inbox"; renderRegistry(); } }, h("b", {}, String(n)), h("span", {}, label));
  body.append(h("div", { class: "regTiles" }, tile("waiting", counts.waiting, "Waiting", "wait"), tile("job", counts.job, "With job", "job"), tile("unlinked", counts.unlinked, "Unlinked", "gone")));

  // Sub-tabs.
  const tab = (key, label) => h("span", { class: "regTab" + (reg.view === key ? " sel" : ""), onclick: () => { reg.view = key; renderRegistry(); } }, label);
  if (reg.view === "send" && !can("titledeed")) reg.view = "find";
  body.append(h("div", { class: "regTabs" }, tab("find", "Find file"), tab("inbox", "Linked"), can("titledeed") ? tab("send", "Send plan") : null));

  const scroll = h("div", { class: "regScroll" });
  body.append(scroll);
  if (reg.view === "inbox") regInbox(scroll);
  else if (reg.view === "find") regFindView(scroll);
  else regSendView(scroll, writes);
  scroll.scrollTop = keep;
}

function regInbox(box) {
  const q = h("input", { type: "search", class: "regSearch", placeholder: "Find UPIN, property no. or transaction", value: reg.q });
  q.addEventListener("input", () => { reg.q = q.value; regInboxList(list); });
  const filt = reg.filter === "all" ? null : h("span", { class: "regChip", title: "Clear the filter", onclick: () => { reg.filter = "all"; renderRegistry(); } }, REG_KIND[reg.filter][0] + " ✕");
  box.append(h("div", { class: "regBar" }, q, filt));
  const list = h("div", {});
  box.append(list);
  regInboxList(list);
}
function regInboxList(list) {
  list.innerHTML = "";
  const st = reg.status;
  const term = reg.q.trim().toUpperCase();
  const rows = reg.links.filter((l) => (reg.filter === "all" || regKind(l) === reg.filter)
    && (!term || [l.upin, l.property_number, l.transaction_number, l.su_id].some((v) => (v || "").toUpperCase().includes(term))));
  if (!reg.loaded) { list.append(h("p", { class: "regEmpty" }, "Loading…")); return; }
  if (!rows.length) {
    list.append(h("div", { class: "regEmpty" }, h("b", {}, reg.links.length ? "Nothing matches" : "No links yet"),
      h("div", { class: "muted" }, reg.links.length ? "Clear the search or the filter." : st && st.enabled ? "When the Registry admin links a property to a parcel it appears here within a minute." : "")));
    return;
  }
  let day = null;
  for (const l of rows) {
    const d = regDay(l.linked_at);
    if (d !== day) { day = d; list.append(h("div", { class: "regDayHead" }, d)); }
    const kind = regKind(l);
    const main = kind === "waiting" && can("titledeed") ? regIconBtn("Title deed", "Digitised and linked: open (or start) the file's title deed job", () => startEditParcel(l.upin), { primary: true })
      : kind === "job" ? regIconBtn("Open job", `Open ${l.job_name || "the job"} in the Workflow`, () => registryOpenJob(l.job_instance_id)) : null;
    const card = h("div", { class: "regCard " + REG_KIND[kind][1], title: l.parcel_id ? "Click to zoom to the parcel" : "", onclick: () => l.parcel_id && registryZoom(l.upin) },
      h("div", { class: "regCardTop" }, h("span", { class: "regUpin" }, l.upin), h("span", { class: "muted regTime" }, regClock(l.linked_at))),
      regStatus(REG_KIND[kind][1], kind === "job" ? `Job started: ${l.job_name || "#" + l.job_instance_id}` : REG_KIND[kind][0]),
      h("div", { class: "regKv" },
        h("div", {}, h("i", {}, "Property"), l.property_number || "—"),
        h("div", {}, h("i", {}, "Transaction"), h("span", {}, l.transaction_number, h("span", { class: "muted" }, ` · ${l.transaction_status}`))),
        h("div", {}, h("i", {}, "Linked"), l.made_in === "cadastre" ? "from this Cadastre" : "by the Registry")),
      h("div", { class: "regCardFoot" },
        h("span", { class: "regLinks" },
          regLink("Zoom", "Zoom to the parcel", () => registryZoom(l.upin), !l.parcel_id),
          regLink("File", "Show this Registry file", () => regFind(l.transaction_number))),
        main));
    list.append(card);
  }
}

// --- Find file: the digitising unit's search (PL file number or PLS transaction number) ---
async function regFind(q) {
  reg.view = "find"; reg.findQ = (q || "").trim().toUpperCase(); reg.found = null; reg.findErr = "";
  if (!regOpen()) await openRegistry(); else renderRegistry();
  if (!reg.findQ) return;
  reg.finding = true; renderRegistry();
  try { reg.found = await api("/api/v1/registry/find?q=" + encodeURIComponent(reg.findQ)); } catch (e) { reg.findErr = e.message; }
  reg.finding = false; renderRegistry();
}
async function regStartDigitising(f) {
  if (!(await uiConfirm(`Start digitising ${f.property_number || f.transaction_number}?\nA "Deed Plan Creationv2" job is created for Registry transaction ${f.transaction_number}, as the Registry created them.`, { ok: "Start digitising" }))) return;
  await regRun("Start digitising", async () => {
    const r = await api("/api/v1/registry/start-digitising", { method: "POST", body: { transaction_number: f.transaction_number, property_number: f.property_number } });
    notify(r.created ? `${r.name} created.` : `${r.name || "The job"} was already open for ${f.transaction_number}.`);
    await registryOpenJob(r.job_instance_id);
  });
}
function regFindView(box) {
  const inp = h("input", { type: "search", class: "regSearch", placeholder: "File no. (PL68955) or transaction (PLS000139391)", value: reg.findQ });
  const go = () => regFind(inp.value);
  inp.addEventListener("keydown", (e) => { if (e.key === "Enter") go(); });
  box.append(h("div", { class: "regBar" }, inp, regIconBtn("Find", "Find the file in the Registry", go, { primary: true })));
  setTimeout(() => { if (!reg.findQ) inp.focus(); }, 0);
  if (reg.finding) { box.append(h("p", { class: "regEmpty" }, "Reading the Registry…")); return; }
  if (reg.findErr) { box.append(h("div", { class: "regEmpty" }, h("b", {}, "Could not search"), h("div", { class: "muted" }, reg.findErr))); return; }
  if (!reg.found) { box.append(h("div", { class: "regEmpty" }, h("b", {}, "Find a file to digitise"), h("div", { class: "muted" }, "Search by the file number (PL…) or the Registry transaction number (PLS…). Start digitising creates the file's job; open it to add the coordinates and commit."))); return; }
  if (!reg.found.length) { box.append(h("div", { class: "regEmpty" }, h("b", {}, "Not found in the Registry"), h("div", { class: "muted" }, `No transaction or file ${reg.findQ}.`))); return; }
  for (const f of reg.found) {
    const active = f.status_code === "transaction_lodged";
    const linked = !!f.upin;
    const here = f.cadastre_upins || [];
    const done = linked || here.length > 0;   // digitised: here, in Aumentum, or linked in the Registry
    const upin = f.upin || here[0] || null;
    const main = f.job_instance_id
      ? regIconBtn("Open job", `Open ${f.job_name || "the job"} in the Workflow`, () => registryOpenJob(f.job_instance_id), { primary: true })
      : done ? regIconBtn("History", "The file's Registry stages and this Cadastre's events", () => historyWindow({ txn: f.transaction_number, upin, title: f.property_number }), { primary: true })
      : can("digitise") ? regIconBtn("Start digitising", active ? "Create the file's job and open it" : `The transaction is ${f.status}: only Active files are digitised`, () => regStartDigitising(f), { primary: true, disabled: !active || reg.busy }) : null;
    const say = f.job_instance_id ? `Job: ${f.job_name || "#" + f.job_instance_id}`
      : linked ? `Digitised, linked in the Registry to ${f.upin}`
      : done ? `Digitised (UPIN ${here.join(", ")}): waiting for the Registry link at title deed`
      : "Not digitised yet";
    box.append(h("div", { class: "regCard " + (f.job_instance_id || done ? "job" : active ? "wait" : "gone") },
      h("div", { class: "regCardTop" }, h("span", { class: "regUpin" }, f.property_number || f.transaction_number), regTxnPill(f.status_code, f.status)),
      regStatus(f.job_instance_id || done ? "job" : "wait", say),
      h("div", { class: "regKv" },
        h("div", {}, h("i", {}, "Transaction"), f.transaction_number),
        h("div", {}, h("i", {}, "Property"), f.su_id || "—"),
        h("div", {}, h("i", {}, "UPIN"), upin ? (linked ? upin : `${here.join(", ")} (in this Cadastre; not on the Registry file yet)`) : "— (given at commit; the Registry links it at title deed)"),
        regMetaRow({ ...f, upin })),
      h("div", { class: "regCardFoot" }, h("span", { class: "regLinks" },
        regLink("Zoom", "Zoom to the parcel", () => registryZoom(upin), !upin),
        done && !f.job_instance_id && can("digitise") ? regLink("Correct", "Correct & Recommit: the parcel keeps its UPIN and its history", () => correctAndRecommit(upin)) : null,
        linked && active && here.length && can("titledeed") ? regLink("Correct plan", "The plan came back: give the reason, change it and resend it (the parcel stays)", () => correctPlan(f.upin, f.transaction_number)) : null), main)));
  }
}

// Committed / linked date and time on a Registry file card (filled in when the metadata arrives).
function regMetaRow(f) {
  if (!f.upin && !f.job_instance_id) return null;
  const v = h("span", { class: "muted" }, "…");
  parcelMeta({ upins: f.upin ? [f.upin] : [], jobs: f.job_instance_id ? [f.job_instance_id] : [] }).then((list) => {
    const m = list.find((x) => x.upin === f.upin) || list.find((x) => x.committed_at) || null;
    v.classList.remove("muted");
    v.textContent = !m || !m.committed_at ? "Not committed yet" : `${m.upin}: committed ${metaCommitted(m)}; linked: ${metaLinked(m)}`;
  });
  return h("div", {}, h("i", {}, "Committed"), v);
}
function regSendView(box, writes) {
  const jobId = reg.job ? reg.job.id : (wf.editing ? wf.editing.job_instance_id : wf.selected);
  if (!reg.job && jobId) { box.append(h("p", { class: "regEmpty" }, "Loading the job…")); regSetJob(jobId).then(renderRegistry); return; }
  if (!reg.job) {
    box.append(h("div", { class: "regEmpty" }, h("b", {}, "No job chosen"), h("div", { class: "muted" }, "Start or open a job from the Inbox, or open one in the Workflow, then come back here.")));
    return;
  }
  const j = reg.job, t = reg.txn && reg.txn.transaction_number === j.lrs_transaction_number ? reg.txn : null;
  if (j.lrs_transaction_number && !t && !reg.txnLoading && !reg.txnErr) { regLookup(j.lrs_transaction_number).then(renderRegistry); }
  const other = wf.editing ? wf.editing.job_instance_id : wf.selected;
  box.append(h("div", { class: "regTxnHead" },
    h("div", {}, h("span", { class: "regUpin" }, j.name || "Job " + j.id), " ", h("span", { class: "muted" }, "#" + j.id)),
    other && other !== j.id ? regIconBtn("Use current job", "Use the job open in the Workflow", async () => { await regSetJob(other); renderRegistry(); }) : h("span")));
  const step = (ok, title, detail) => h("div", { class: "regStep " + (ok === true ? "ok" : ok === false ? "bad" : "") },
    h("span", { class: "regStepMark" }, ok === true ? "✓" : ok === false ? "!" : "…"), h("div", {}, h("b", {}, title), detail ? h("div", { class: "muted" }, detail) : null));
  box.append(step(!!j.lrs_transaction_number, "Registry transaction", j.lrs_transaction_number || "This job has no Registry transaction."));
  if (!j.lrs_transaction_number) return;
  box.append(step(t ? t.open : reg.txnErr ? false : null, "Transaction is Active", t ? (t.open ? t.status : t.refused) : reg.txnErr || "Checking the Registry…"));
  const files = h("div", { class: "regFiles" });
  for (const f of reg.files) {
    files.append(h("label", { class: "regFile" + (reg.pick === f.id ? " sel" : "") },
      h("input", { type: "radio", name: "regPick", checked: reg.pick === f.id, onchange: () => { reg.pick = f.id; renderRegistry(); } }),
      /\.pdf$/i.test(f.file_name)
        ? h("a", { class: "regPdf", href: `/api/v1/wf/files/${f.id}`, target: "_blank", title: "Open the PDF", onclick: (e) => e.stopPropagation() }, "PDF")
        : h("img", { src: `/api/v1/wf/files/${f.id}`, alt: "", loading: "lazy" }),
      h("span", {}, f.file_name)));
  }
  // Attach the plan here (the same job attachment as Workflow → Attachments). The usual way is
  // Title Deed Plan → Attach as PDF; a PDF or JPG made elsewhere can be attached too.
  const pickFile = h("input", { type: "file", accept: ".pdf,.jpg,.jpeg,application/pdf,image/jpeg", style: "display:none" });
  pickFile.onchange = async () => {
    const file = pickFile.files[0]; if (!file) return;
    await regRun("Attach", async () => {
      await authReady;
      const r = await fetch(`/api/v1/wf/jobs/${j.id}/files?name=${encodeURIComponent(file.name)}`, { method: "POST", body: file, headers: { "content-type": "application/octet-stream", ...authHeaders() } });
      if (!r.ok) throw new Error((await r.json().catch(() => ({}))).message || r.statusText);
      notify(`${file.name} attached to ${j.name || "the job"}.`); await regSetJob(j.id);
      reg.pick = (reg.files.find((x) => x.file_name === file.name) || {}).id || reg.pick;
    });
  };
  const attach = regIconBtn(reg.files.length ? "Attach another…" : "Attach PDF…", "Attach a plan PDF (or JPG, sent as a PDF) to the job", () => pickFile.click(), { disabled: reg.busy });
  box.append(step(reg.files.length ? true : false, "Title deed plan (PDF)",
    reg.files.length ? "Choose the plan to send. It goes to the Registry as a PDF." : "Title Deed Plan → Attach as PDF, or attach a PDF here."));
  box.append(h("div", { style: "margin:-2px 8px 6px 34px" }, attach, pickFile));
  if (reg.files.length) box.append(files);
  const f = reg.files.find((x) => x.id === reg.pick);
  const ready = writes && t && t.open && f && !reg.busy;
  // Sent before to this transaction: the button becomes Resend corrected plan.
  const sentHere = !!reg.correction || reg.sent.some((x) => x.detail && x.detail.transaction_number === j.lrs_transaction_number);
  const pdfName = f ? (/\.pdf$/i.test(f.file_name) ? f.file_name : f.file_name.replace(/\.[^.]+$/, "") + ".pdf") : "";
  box.append(h("div", { class: "regSendBar" },
    h("button", { class: "regAct primary big", disabled: !ready, title: !writes ? "Writing to the Registry is off on this server" : "",
      onclick: () => regSendPlan(f.id, f.file_name, sentHere) },
      reg.busy ? "Sending…" : !f ? "Send plan" : sentHere ? `Resend corrected plan: ${pdfName}` : `Send ${pdfName} to ${j.lrs_transaction_number}`),
    h("div", { class: "muted" }, reg.correction
      ? `Plan correction: ${reg.correction.reason}. The corrected PDF is filed as "Survey Plan (corrected)" with this reason, and the earlier plans in the transaction are marked as replaced; nothing is deleted.`
      : sentHere
      ? "A plan was sent before. The corrected PDF is filed as \"Survey Plan (corrected)\" and the earlier plan is marked as replaced; nothing is deleted."
      : "Filed in the transaction as a Title Deed Plan (\"Survey Plan\"), as a PDF.")));
  if (reg.sent.length) {
    box.append(h("div", { class: "regDayHead" }, "Sent"));
    for (const x of reg.sent) {
      const d = x.detail || {};
      box.append(h("div", { class: "regSent" }, h("span", { class: "regPill ok" }, d.resend ? "Corrected" : "Sent"), ` ${d.file_name} → ${d.transaction_number}`,
        h("span", { class: "muted" }, ` · ${regDay(x.at)} ${regClock(x.at)} · ${x.by}`),
        d.replace_note ? h("div", { class: "muted", style: "color:#a60" }, d.replace_note) : null));
    }
  }
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
// A task's steps, each with its tools and the editor action that runs it.
function wfStepList(steps) {
  const ol = h("ol", { class: "wfSteps" });
  for (const s of steps) {
    // Aumentum tools by name; ArcMap's own built-in tools (not in Aumentum's registry) as a count.
    const named = s.commands.filter((c) => AUM_TOOL_GUIDS[c] || !c.startsWith("{")).map(wfToolName).filter((v, i, a) => a.indexOf(v) === i);
    const builtIn = s.commands.filter((c) => c.startsWith("{") && !AUM_TOOL_GUIDS[c]).length;
    const tools = named.join(" · ") + (builtIn ? `${named.length ? " · " : ""}+ ${builtIn} ArcMap tool${builtIn > 1 ? "s" : ""}` : "");
    let act = wfStepAction(s);
    const need = stepFeature(s.id);
    if (need && !can(need)) act = { label: "—", tip: `Done in the ${VIEW_FOR[need]} view` };
    ol.append(h("li", {}, h("div", { class: "wfStepHead" }, h("b", {}, s.name),
        act.run ? h("button", { class: "btn wfStepBtn", title: act.tip, onclick: act.run }, act.label || "Open")
                : h("span", { class: "muted wfStepNote", title: act.tip }, act.label)),
      h("div", { class: "muted" }, tools || "—")));
  }
  return ol;
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
    box.append(h("div", { class: "wfSub" }, `Steps of "${j.current_task}"`), wfStepList(d.steps));
    // Search Cadastre for Parcels can send the job on to Plot Parcel: its tools are offered here
    // too, so a file whose parcel is not in the cadastre yet is drawn without moving the job first.
    if ((d.plot_steps || []).length) box.append(
      h("div", { class: "wfSub", title: "The parcel is not in the cadastre yet? Draw it here; Post Parcel & Finish Job takes the job through the plotting tasks." }, `${d.plot_task?.name || "Plot Parcel"} tools`),
      wfStepList(d.plot_steps));
  }
  if (j.is_open) box.append(wfEditingView(j, wf.selEditing));
  else box.append(h("div", { class: "frow", style: "margin:6px 0;gap:4px" },
    h("button", { class: "btn", title: "Ask the Registry (10.10.10.5) whether this job's committed parcels are visible there", onclick: () => registryConfirmDialog(j.id) }, "Registry Confirmation"),
    h("button", { class: "btn", title: "The file's Registry stages and this Cadastre's events", onclick: () => historyWindow({ job: j.id, txn: j.lrs_transaction_number, title: j.name }) }, "History")));
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
  } catch (e) { status(e.message); uiAlert(e.message); }
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
      } catch (e) { uiAlert(e.message); }
    } }, "Create"))), { id: "wfNewWin", width: 380 });
}

// What turns a saved draft into a parcel with a UPIN, for the job being edited.
function draftNextStep() {
  return wf.editing && state.job && state.job.id === wf.editing.cadastre_job_id
    ? "Click Post Parcel & Finish to allocate its UPIN."
    : "Post the job (once approved) to allocate a UPIN.";
}
// Refresh the worked-on job's editing state (draft count, area) after an edit.
async function wfRefreshEditing() {
  if (!wf.editing) return;
  try { wf.mutations = await api(`/api/v1/wf/jobs/${wf.editing.job_instance_id}/mutations`); } catch { wf.mutations = []; }
  try { wf.editing = await api(`/api/v1/wf/jobs/${wf.editing.job_instance_id}/editing`); if (wf.selected === wf.editing.job_instance_id) wf.selEditing = wf.editing; renderWorkflow(); updateEditorButtons(); } catch { /* keep the last known state */ }
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
    status(wf.editing.area ? `Working on ${wf.detail?.job?.name || "job " + jobId}: draw parcels inside its job area, then Post Parcel & Finish.` : `Working on ${wf.detail?.job?.name || "job " + jobId}: sketch the parcel — its job area is created automatically — then Post Parcel & Finish.`);
  } catch (e) { uiAlert(e.message); }
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
  } catch (e) { uiAlert(e.message); }
  renderWorkflow();
}
// Commit Parcels: the officer types the beacon number of every corner, from the surveyor's point
// list (Aumentum: beacon numbers are entered at commit, never generated). Corners already on a
// fabric beacon keep its number. A point list (ID Easting Northing per line, as Trimble Business
// Center prints it) can be pasted to fill the numbers of the corners at those coordinates.
// opts.submit(beacons): post the numbers some other way (Post Parcel & Finish Job) instead of /commit.
// Committed by: picked from the users list (search by name or login), or added when the person is
// not in it; recorded in the audit log with the commit (name and user id). The last pick is
// remembered on this PC.
const staffKey = "cad.staff";
function lastStaff() { try { return JSON.parse(localStorage.getItem(staffKey) || "null"); } catch { return null; } }
function keepStaff(v) { try { localStorage.setItem(staffKey, JSON.stringify(v)); } catch { /* storage unavailable */ } }
let staffUsers = null;
async function loadStaffUsers(force) {
  if (!staffUsers || force) {
    try { staffUsers = (await api("/api/v1/wf/principals")).filter((p) => p.is_user && !p.is_locked); } catch { staffUsers = staffUsers || []; }
  }
  return staffUsers;
}
const normName = (s) => String(s || "").toLowerCase().replace(/[^a-z0-9 ]+/g, " ").replace(/\s+/g, " ").trim();
// Users whose name is close to a typed one (same words in another order, initials, one letter off).
function similarUsers(name) {
  const t = normName(name); if (!t) return [];
  const tw = t.split(" ");
  const close = (a, b) => { if (Math.abs(a.length - b.length) > 1) return false; let i = 0, j = 0, d = 0; while (i < a.length && j < b.length) { if (a[i] === b[j]) { i++; j++; } else { d++; if (d > 1) return false; if (a.length > b.length) i++; else if (b.length > a.length) j++; else { i++; j++; } } } return d + (a.length - i) + (b.length - j) <= 1; };
  return (staffUsers || []).filter((u) => {
    const n = normName(u.name || u.code); const nw = n.split(" ");
    if (n === t || close(n, t)) return true;
    const shared = tw.filter((w) => w.length > 2 && nw.some((x) => x === w || close(x, w))).length;
    return shared >= Math.min(2, tw.length);
  }).slice(0, 5);
}
// The picker: a search box with the matching users below, and "Add new staff" at the end.
function staffPicker() {
  const last = lastStaff();
  const st = { sel: last && last.sid ? last : null };
  const inp = h("input", { class: "tinput", value: st.sel ? st.sel.name : "", placeholder: "Search staff by name or login", autocomplete: "off", style: "width:100%" });
  const list = h("div", { class: "staffList hidden" });
  const note = h("div", { class: "muted", style: "font-size:11px;margin-top:2px" }, st.sel ? `Selected: ${st.sel.name} (${st.sel.code})` : "Pick yourself from the list, or add your name.");
  const pick = (u) => { st.sel = { sid: u.sid, name: u.name || u.code, code: u.code }; inp.value = st.sel.name; note.textContent = `Selected: ${st.sel.name} (${st.sel.code})`; list.classList.add("hidden"); };
  const addNew = async () => {
    const name = inp.value.trim().replace(/\s+/g, " ");
    if (name.length < 3) { note.textContent = "Type the full name first."; inp.focus(); return; }
    const sim = similarUsers(name);
    for (const u of sim) {
      if (await uiConfirm(`Did you mean ${u.name} (${u.code})? They are already a user.`, { title: "Add new staff", ok: `Yes, ${u.name}`, cancel: "No, someone else" })) { pick(u); return; }
    }
    const code = await uiPrompt(`Add ${name} as a new staff member.\nLogin (letters and digits):`, (name.split(" ").length > 1 ? name[0] + name.split(" ").pop() : name).toLowerCase().replace(/[^a-z0-9]/g, ""), { title: "Add new staff", ok: "Add" });
    if (code === null) return;
    try {
      const u = await api("/api/v1/wf/principals", { method: "POST", body: { name, code } });
      await loadStaffUsers(true); pick(u); notify(`${u.name} added to the users (${u.code}).`);
    } catch (e) { uiAlert("Adding the staff member failed: " + e.message); }
  };
  const render = () => {
    const q = normName(inp.value);
    const users = (staffUsers || []).filter((u) => !q || normName(`${u.name || ""} ${u.code}`).includes(q)).slice(0, 8);
    list.innerHTML = "";
    for (const u of users) list.append(h("div", { class: "staffRow", onmousedown: (e) => { e.preventDefault(); pick(u); } }, h("b", {}, u.name || u.code), h("span", { class: "muted" }, ` (${u.code})`)));
    if (inp.value.trim() && !users.some((u) => normName(u.name) === q)) list.append(h("div", { class: "staffRow add", onmousedown: (e) => { e.preventDefault(); list.classList.add("hidden"); addNew(); } }, `➕ Add new staff: ${inp.value.trim()}`));
    if (!users.length && !inp.value.trim()) list.append(h("div", { class: "staffRow muted" }, "No users loaded."));
    list.classList.toggle("hidden", !list.children.length);
  };
  inp.addEventListener("input", () => { st.sel = null; note.textContent = "Pick from the list, or add a new staff member."; render(); });
  inp.addEventListener("focus", () => loadStaffUsers().then(render));
  inp.addEventListener("blur", () => setTimeout(() => list.classList.add("hidden"), 150));
  inp.addEventListener("keydown", (e) => { if (e.key === "Escape") list.classList.add("hidden"); });
  loadStaffUsers();
  return {
    el: h("div", { class: "staffPick" }, inp, list, note),
    focus: () => inp.focus(),
    // The picked user; a typed name that is exactly a user's is taken as that user.
    get: () => {
      if (st.sel) return st.sel;
      const q = normName(inp.value); const u = q && (staffUsers || []).find((x) => normName(x.name) === q || x.code.toLowerCase() === q);
      if (u) { pick(u); return st.sel; }
      return null;
    },
  };
}
async function askStaff() {
  const p = staffPicker();
  const ok = await uiDialog({ title: "Commit", message: h("div", {}, h("p", {}, h("b", {}, "Committed by"), " (recorded in the audit log):"), p.el), kind: "question", width: 440,
    buttons: [{ label: "Commit", value: true, primary: true }, { label: "Cancel", value: false, cancel: true }] });
  if (!ok) return null;
  const s = p.get(); if (!s) { await uiAlert("Pick the staff member from the list (or add them) to commit."); return null; }
  keepStaff(s); return s;
}
async function wfCommitDialog(opts = {}) {
  const id = wf.editing?.job_instance_id; if (!id) { status("Choose Work On This Job first."); return; }
  let pts;
  try { pts = await api(`/api/v1/wf/jobs/${id}/editing/corners`); } catch (e) { uiAlert(e.message); return; }
  if (!pts.length && opts.submit) { const staff = await askStaff(); if (!staff) return; try { await opts.submit([], staff); } catch (e) { uiAlert(e.message); } return; }
  if (!pts.length) {
    const ret = (wf.mutations || []).filter((m) => m.kind === "retire");
    if (!ret.length) { uiAlert("The job has no draft parcels to commit."); return; }
    if (!(await uiConfirm(`Commit the retirement of ${ret.flatMap((m) => m.parents).join(", ")}?`, { ok: "Commit" }))) return;
    const staff = await askStaff(); if (!staff) return;
    try {
      const r = await api(`/api/v1/wf/jobs/${id}/commit`, { method: "POST", body: { beacons: [], staff_name: staff.name, staff_sid: staff.sid } });
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
  const staffP = staffPicker();
  const commitBtn = h("button", { class: "btn", onclick: async () => {
    const beacons = pts.filter((p) => !p.beacon_number).map((p) => ({ easting: p.easting, northing: p.northing, number: inputs.get(p.point).value.trim() }));
    const blank = pts.filter((p) => !p.beacon_number && !inputs.get(p.point).value.trim()).map((p) => p.point);
    if (blank.length) { msg.style.color = "#b00"; msg.textContent = `Enter the beacon number of point(s) ${blank.join(", ")}.`; inputs.get(blank[0]).focus(); return; }
    const staff = staffP.get();
    if (!staff) { msg.style.color = "#b00"; msg.textContent = "Committed by: pick the staff member from the list, or add them (it is recorded in the audit log)."; staffP.focus(); return; }
    keepStaff(staff);
    commitBtn.disabled = true; msg.style.color = ""; msg.textContent = "Committing…";
    if (opts.submit) {
      try { await opts.submit(beacons, staff); w.remove(); src()?.setData(empty()); }
      catch (e) { commitBtn.disabled = false; msg.style.color = "#b00"; msg.textContent = e.message; }
      return;
    }
    try {
      const r = await api(`/api/v1/wf/jobs/${id}/commit`, { method: "POST", body: { beacons, staff_name: staff.name, staff_sid: staff.sid } });
      w.remove(); src()?.setData(empty());
      wf.editing = r.editing; wf.selEditing = wf.editing;
      await Promise.all([loadParcels(false), loadBeacons(), loadDrafts()]); showJobArea();
      notify(`Committed ${r.parcel_numbers.length} parcel(s): ${r.parcel_numbers.join(", ")}; ${r.beacons_added.length} beacon(s) added.` + (r.retired?.length ? ` Replaced / retired: ${r.retired.join(", ")}.` : ""));
      loadHistory(); await wfRefreshEditing();
      wf.detail = await api("/api/v1/wf/jobs/" + id); renderWorkflow();
      if (r.parcel_numbers.length) await registryConfirmDialog(id, { headline: `Committed: ${r.parcel_numbers.join(", ")}. ${linkSummary(r.registry_links)}` });
      if (wf.detail?.job && !wf.detail.job.is_open) await goHome(r.parcel_numbers);
    } catch (e) { commitBtn.disabled = false; msg.style.color = "#b00"; msg.textContent = e.message; }
  } }, opts.label || "Commit");
  // The UPIN(s) this commit will give, and whether Aumentum was asked (parallel run). Advisory.
  const upinLine = h("div", { style: "margin:0 0 8px;padding:6px 8px;border:1px solid #ccc;background:#f7f7f7" }, "Working out the UPIN to be issued…");
  api(`/api/v1/wf/jobs/${id}/next-upin`).then((p) => {
    upinLine.innerHTML = "";
    const list = (p.upins || []).join(", ") || "—";
    upinLine.append(h("b", {}, `UPIN${(p.upins || []).length > 1 ? "s" : ""} to be issued: ${list}`), h("br"));
    if (p.aumentum === "checked") {
      upinLine.append(h("span", { style: "color:#060" }, "Checked with Aumentum just now" +
        (p.aumentum_skipped.length ? `; skipped ${p.aumentum_skipped.join(", ")} (already issued in Aumentum).` : ": not issued there.")));
    } else if (p.aumentum === "unavailable") {
      upinLine.style.borderColor = "#c00"; upinLine.style.background = "#fde8e8";
      upinLine.append(h("span", { style: "color:#900" }, `Aumentum could not be asked (${(p.message || "no answer").replace(/\.? Please contact the ICT Admin\.$/, "")}): the commit will be refused until it can. Please contact the ICT Admin.`));
    } else {
      upinLine.append(h("span", { class: "muted" }, "Not checked with Aumentum on this server (no parallel-run check configured)."));
    }
  }).catch((e) => { upinLine.textContent = "Could not work out the next UPIN: " + e.message; });
  const w = floatWindow(opts.title || "Commit Parcels — Beacon Numbers", h("div", {},
    upinLine,
    h("p", { class: "muted", style: "margin:0 0 6px" }, `${pts.length} corner point(s), numbered on the map. Type each beacon number from the surveyor's point list.`),
    h("div", { style: "max-height:320px;overflow:auto" }, t), paste, h("div", { class: "frow" }, fill),
    h("div", { style: "margin-top:6px" }, h("b", {}, "Committed by:"), staffP.el), msg,
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
  if (!working || !can("digitise")) {
    sec.append(h("div", { class: "frow" }, can("digitise") ? h("button", { class: "btn", onclick: () => wfWorkOn(j.id) }, "Work On This Job") : null,
      h("button", { class: "btn", title: "Ask the Registry (10.10.10.5) whether this job's committed parcels are visible there", onclick: () => registryConfirmDialog(j.id) }, "Registry Confirmation"),
      h("span", { class: "muted" }, e?.drafts ? `${e.drafts} draft parcel(s)` : e?.area ? "Job area defined" : "")));
    return sec;
  }
  const ed = wf.editing;
  sec.append(h("div", { class: "muted" }, `Editing job ${ed.reference || ""} · ${ed.drafts} draft parcel(s) · job area: ${ed.area ? `${ed.area_status}, ${Math.round(ed.area_m2).toLocaleString()} m²` : "none"}`));
  sec.append(h("div", { class: "frow", style: "flex-wrap:wrap" },
    h("button", { class: "btn", onclick: wfDrawJobArea }, ed.area ? "Edit Job Area" : "Create Job Area"),
    ed.area ? h("button", { class: "btn", onclick: () => wfEditAction("POST", `/area/${ed.area_status === "closed" ? "reopen" : "close"}`, ed.area_status === "closed" ? "Job area reopened." : "Job area closed.") }, ed.area_status === "closed" ? "Reopen Job Area" : "Close Job Area") : null,
    ed.area ? h("button", { class: "btn", onclick: async () => { if (await uiConfirm("Delete the job area?", { ok: "Delete", kind: "warn" })) wfEditAction("DELETE", "/area", "Job area deleted."); } }, "Delete Job Area") : null,
    h("button", { class: "btn", disabled: !ed.drafts, onclick: async () => { if (await uiConfirm(`Discard the job's ${ed.drafts} uncommitted parcel(s)?`, { ok: "Discard", kind: "warn" })) wfEditAction("POST", "/discard", "Draft parcels discarded."); } }, "Discard Changes"),
    h("button", { class: "btn", onclick: async () => { await loadDrafts(); await wfSelect(j.id); status("Reloaded from the fabric."); } }, "Reload"),
    h("button", { class: "btn", title: "Ask the Registry (10.10.10.5) whether this job's committed parcels are visible there", onclick: () => registryConfirmDialog(j.id) }, "Registry Confirmation"),
    h("button", { class: "btn", title: "The actual view of the plot on the satellite imagery, with overlaps and titles", onclick: orthoOpen }, "Orthophoto"),
    h("button", { class: "btn", title: "The file's Registry stages and this Cadastre's events", onclick: () => historyWindow({ job: j.id, txn: j.lrs_transaction_number, title: j.name }) }, "History"),
    ed.is_commit_task ? h("button", { class: "btn", disabled: !ed.drafts && !(wf.mutations || []).length, title: "Give the job's draft parcels their UPINs and add them to the fabric", onclick: () => wfCommitDialog() }, "Commit Parcels…") : null));
  // Mutations recorded in the job (Phase 4), applied when the parcels are committed.
  if ((wf.mutations || []).length) {
    const ul = h("ul", { class: "wfMutations", style: "margin:4px 0 0;padding-left:18px" });
    for (const m of wf.mutations) {
      const what = m.kind === "retire" ? `Retire ${m.parents.join(", ")} (${m.reason || ""})` : `${MUTATION_LABEL[m.kind] || m.kind} ${m.parents.join(" + ")} → ${m.drafts.length} new parcel(s)`;
      const keep = m.kind === "separate" ? h("select", { title: "Separation Configuration: the part that keeps the parent's UPIN and attribute record", onchange: (e) => mutationKeepUpin(ed.job_instance_id, m, e.target.value) },
        h("option", { value: "" }, "every part gets a new UPIN"),
        ...m.drafts.map((d, i) => { const f = state.drafts.features.find((x) => String(x.properties.id) === String(d));
          return h("option", { value: d, selected: m.keeps_upin === d }, `part ${i + 1} keeps ${m.parents[0]}${f && f.properties.computed_area ? ` (${area2(f.properties.computed_area)} m²)` : ""}`); })) : null;
      ul.append(h("li", {}, what, " ", keep, " ", h("button", { class: "btn", style: "min-width:0;padding:0 6px", title: "Cancel this mutation (its draft parcels are removed)", onclick: async () => {
        if (!(await uiConfirm(`Cancel: ${what}?`, { ok: "Cancel it", cancel: "Keep", kind: "warn" }))) return;
        try { await api(`/api/v1/wf/jobs/${ed.job_instance_id}/mutations/${m.group}`, { method: "DELETE" }); await afterMutation("Mutation cancelled."); } catch (e) { uiAlert(e.message); }
      } }, "Cancel")));
    }
    sec.append(h("div", { class: "wfSub", style: "margin-top:6px" }, "Mutations (applied at commit)"), ul);
  }
  // One-click seamless path: take the job straight to its commit task, post the parcel(s) and
  // finish the job — no stepping through Review/Commit/Generate by hand.
  sec.append(h("div", { class: "frow" },
    h("button", { class: "btn btnPrimary", disabled: !ed.drafts, title: "Post the parcel (allocate its UPIN) and finish the job in one step", onclick: () => wfFinishJob(j) }, "Post Parcel & Finish Job"),
    h("span", { class: "muted" }, ed.drafts ? `${ed.drafts} parcel(s) ready` : "draw a parcel first")));
  return sec;
}
// One-click: post the job's parcel(s) — allocating the UPIN(s) — and finish the workflow. The
// server walks the job to its commit task, commits, and completes it in one transaction.
// Post & finish the job currently being worked on (from the editor toolbar / Editor menu).
async function wfFinishCurrent() {
  const id = wf.editing?.job_instance_id;
  if (!id) { status("Work on a job (Workflow pane) before posting."); return; }
  let job = wf.detail && wf.detail.job && wf.detail.job.id === id ? wf.detail.job : null;
  if (!job) { try { job = (await api("/api/v1/wf/jobs/" + id)).job; } catch (e) { uiAlert(e.message); return; } }
  await wfFinishJob(job);
}
async function wfFinishJob(j, opts = {}) {
  const ed = wf.editing;
  if (!ed || !ed.drafts) { status("Draw a parcel first."); return; }
  if (!opts.skipConfirm && !(await uiConfirm(`Post ${ed.drafts} parcel(s) and finish job ${j.name || "#" + j.id}?\nThis allocates the official UPIN(s) and completes the workflow.`, { ok: "Post & Finish" }))) return;
  // The new corners' beacon numbers first, as for Commit Parcels.
  await wfCommitDialog({ title: "Post Parcel & Finish Job — Beacon Numbers", label: "Post & Finish", submit: (beacons, staff) => wfFinishPost(j, beacons, staff) });
}
async function wfFinishPost(j, beacons, staff) {
  if (wf.busy) return; wf.busy = true;
  try {
    const r = await api(`/api/v1/wf/jobs/${j.id}/finish`, { method: "POST", body: { complete: true, beacons, staff_name: staff?.name, staff_sid: staff?.sid } });
    const nums = (r.parcel_numbers || []).join(", ") || "(none)";
    status(r.completed ? `Done — ${nums} posted; job completed.` : `${nums} posted.`);
    await loadParcels(false); await loadDrafts(); showJobArea();
    const confirmed = registryConfirmDialog(j.id, { headline: `Parcel posted. UPIN: ${nums}. ${r.completed ? "Job completed." : "Job now at: " + (r.job?.current_task || "—")} ${linkSummary(r.registry_links)}` });
    if (r.completed) confirmed.then(() => goHome(r.parcel_numbers || []));
    wf.detail = await api("/api/v1/wf/jobs/" + j.id);
    wf.selEditing = await api(`/api/v1/wf/jobs/${j.id}/editing`).catch(() => wf.selEditing);
    if (wf.editing && wf.editing.job_instance_id === j.id) wf.editing = wf.selEditing;
    const i = wf.jobs.findIndex((x) => x.id === j.id); if (i >= 0 && r.job) wf.jobs[i] = r.job;
    loadBeacons();
  } catch (e) { status(e.message); throw e; }
  finally { wf.busy = false; renderWorkflow(); }
}
// What a workflow step's "Open" does in this editor. Steps whose tools belong to later phases say so.
// --- Title deed plan (Generate Title Deed Plan): the form, the sheet, the plan PDF ---------------
// The server prefills the form from the Registry (Aumentum's R_PROPINFO / R_HOLDERS) and the parcel
// record; the officer confirms or types the rest, saves it with the job, prints the A4 sheet, and
// turns it into the PDF that Send plan files with the Registry transaction.
// A feature of the plan drawn on the map: click its points and double-click to finish (a point
// symbol takes one click), Esc to cancel. onDone gets the UTM points, or null when cancelled.
function tdStartDraw(point, onDone) {
  tdDraw = { pts: [], point, onDone };
  setTool("tdFeature");
  status(point ? "Plan feature: click its position on the map. Esc cancels." : "Plan feature: click along it on the map, double-click to finish. Esc cancels.");
}
function tdClick(ll) {
  if (!tdDraw) return;
  // A double-click also delivers two clicks at the same spot; don't add zero-length segments.
  const last = tdDraw.pts[tdDraw.pts.length - 1];
  if (last) { const a = map.project(last), b = map.project(ll); if (Math.hypot(a.x - b.x, a.y - b.y) < 3) return; }
  tdDraw.pts.push([ll.lng, ll.lat]);
  if (tdDraw.point) { tdFinish(); return; }
  map.getSource("measure").setData({ type: "FeatureCollection", features: tdDraw.pts.length > 1 ? [{ type: "Feature", geometry: { type: "LineString", coordinates: tdDraw.pts }, properties: {} }] : [] });
}
function tdFinish() {
  if (!tdDraw) return;
  if (tdDraw.pts.length < (tdDraw.point ? 1 : 2)) { status("Plan feature: a line needs at least two points."); return; }
  tdEnd(tdDraw.pts);
}
function tdEnd(pts) {
  const d = tdDraw;
  tdDraw = null;
  map.getSource("measure")?.setData(empty());
  setTool("pan");
  d.onDone(pts ? pts.map(([x, y]) => toUTM(x, y).map((v) => Math.round(v * 1000) / 1000)) : null);
}
// The plan's drawn features on the map, while its form is open.
function tdShow(feats) {
  map.getSource("tdfeat")?.setData({ type: "FeatureCollection", features: feats.filter((x) => x.points?.length).map((x) => ({ type: "Feature", properties: {},
    geometry: x.points.length > 1 ? { type: "LineString", coordinates: x.points.map(([e, n]) => toLL(e, n)) } : { type: "Point", coordinates: toLL(...x.points[0]) } })) });
}
// The road or feature copied in the plan editor; it stays for the next plan opened.
let tdClip = null;
const TD_SCALES = [250, 500, 750, 1000, 1250, 1500, 2000, 2500, 3000, 4000, 5000, 7500, 10000, 25000];
async function titleDeedDialog(jobId, upin) {
  let d;
  try { d = await api(`/api/v1/wf/jobs/${jobId}/titledeed` + (upin ? `?upin=${encodeURIComponent(upin)}` : "")); }
  catch (e) { uiAlert("Title deed plan: " + e.message); return; }
  const p = d.plan, f = p.fields;
  const inp = (key, ph) => h("input", { value: f[key] || "", placeholder: ph || "", style: "width:100%;box-sizing:border-box", "data-k": key });
  const row = (label, el) => h("div", { class: "frow" }, h("label", { style: "min-width:120px" }, label), el);
  const lga = h("select", { "data-k": "lga", style: "width:100%" }, h("option", { value: "" }, ""),
    ...[...new Set([...(f.lga ? [f.lga] : []), ...PLATEAU_LGAS])].map((l) => h("option", { value: l, selected: l === f.lga }, l)));
  const scale = h("select", { style: "width:100%" }, h("option", { value: "" }, "Auto (largest standard scale that fits)"),
    ...TD_SCALES.map((sc) => h("option", { value: sc, selected: f.scale === sc }, `1:${sc}`)));
  const n = p.corners.length;
  const names = p.corners.map((c, i) => h("input", { value: (f.corner_names || [])[i] || "", placeholder: c.name, style: "width:100px" }));
  const ct = h("table", { class: "legs" }, h("tr", {}, h("th", {}, "#"), h("th", {}, "Beacon"), h("th", {}, "Easting"), h("th", {}, "Northing")),
    ...p.corners.map((c, i) => h("tr", {}, h("td", { class: "n" }, String(i + 1)), h("td", {}, names[i]), h("td", {}, fmt3(c.e)), h("td", {}, fmt3(c.n)))));
  const nameOf = (i) => names[i].value.trim() || p.corners[i].name;
  // Access roads: any number of sides (two neighbouring sides meet in a junction on the sheet).
  const roadSet = new Set((f.road_sides && f.road_sides.length) ? f.road_sides : (f.road_side != null ? [f.road_side] : []));
  const sideText = (i) => `Side ${i + 1}: ${nameOf(i)} → ${nameOf((i + 1) % n)}`;
  const roadBoxes = p.corners.map((c, i) => h("input", { type: "checkbox", checked: roadSet.has(i) }));
  const roadLabels = p.corners.map((c, i) => h("span", {}, sideText(i)));
  const road = h("div", { style: "width:100%;max-height:90px;overflow:auto" },
    ...p.corners.map((c, i) => h("label", { style: "display:block;white-space:nowrap" }, roadBoxes[i], " ", roadLabels[i])));
  // Grid: ticks on the frame edges at the whole-metre Easting / Northing, on or off.
  const gridOn = h("input", { type: "checkbox", checked: f.grid !== false });
  const grid = h("label", { style: "white-space:nowrap" }, gridOn, " Show grid ticks on the frame edges");
  const roadLabel = inp("road_label", "Access Road");
  // Plan features: a symbol placed along a side of the parcel, or drawn on the map.
  const feats = (f.features || []).map((x) => ({ ...x }));
  const symOf = (id) => d.symbols.find((y) => y.id === id);
  const symSel = h("select", { style: "width:100%" }, ...[...new Set(d.symbols.map((y) => y.group))].map((g) =>
    h("optgroup", { label: g }, ...d.symbols.filter((y) => y.group === g).map((y) => h("option", { value: y.id }, y.label)))));
  const placeSel = h("select", { style: "width:100%" }, h("option", { value: "" }, "Draw on the map"), ...p.corners.map((c, i) => h("option", { value: i }, sideText(i))));
  const featLabel = h("input", { placeholder: "Optional, e.g. River Delimi", style: "width:100%;box-sizing:border-box" });
  const featList = h("div", { style: "max-height:110px;overflow:auto" });
  const featText = (x) => `${symOf(x.symbol)?.label || x.symbol}${x.label ? ` "${x.label}"` : ""}: ${x.side != null ? sideText(x.side) : "drawn on the map"}`;
  const renderFeats = () => {
    featList.replaceChildren(...feats.map((x, i) => h("div", { style: "display:flex;gap:6px;align-items:center;margin-top:3px" }, h("span", { style: "flex:1" }, featText(x)),
      h("button", { class: "btn", onclick: () => { feats.splice(i, 1); dropFeatureMove(i); featsChanged(); } }, "Remove"))));
    tdShow(feats);
  };
  // Label moves of the plan, kept by label: a removed feature's goes, the later ones shift down.
  let moves = { ...(f.label_moves || {}) };
  // Colours and wording set on the sheet's labels, and the plan's free text.
  let styles = { ...(f.styles || {}) };
  const notes = (f.notes || []).map((x) => ({ ...x }));
  const dropFeatureMove = (i) => {
    const shift = (o) => Object.fromEntries(Object.entries(o).flatMap(([k, v]) => {
      const m = /^feature:(\d+)$/.exec(k);
      if (!m || Number(m[1]) < i) return [[k, v]];
      return Number(m[1]) === i ? [] : [[`feature:${Number(m[1]) - 1}`, v]];
    }));
    moves = shift(moves); styles = shift(styles);
  };
  const featsChanged = () => { renderFeats(); save().catch((e) => { msg.style.color = "#900"; msg.textContent = e.message; }); };
  const addFeat = () => {
    const base = { symbol: symSel.value, label: featLabel.value.trim() || null, side: null, points: [] };
    const added = (x) => { feats.push(x); featLabel.value = ""; featsChanged(); };
    if (placeSel.value !== "") { added({ ...base, side: Number(placeSel.value) }); return; }
    w.style.display = "none"; // out of the way of the map while drawing
    zoomToUTMRing(p.corners.map((c) => [c.e, c.n]));
    tdStartDraw(symOf(base.symbol).point, (points) => {
      w.style.display = "";
      if (points) added({ ...base, points }); else status("Plan feature cancelled.");
    });
  };
  const reg = d.registry === "ok" ? h("span", { style: "color:#060" }, "Prefilled from the Registry (read only) and the parcel record.")
    : d.registry === "off" ? h("span", { class: "muted" }, "Registry not configured on this server: type the holder and survey details.")
    : h("span", { style: "color:#900" }, `Registry not read (${d.registry}): type the holder and survey details.`);
  const missing = p.corners.filter((c, i) => !((f.corner_names || [])[i]) && /^P\d+$/.test(c.name)).length;
  const keys = ["title_no", "plan_no", "tracing_no", "region", "location", "address", "holder", "surveyed_by", "survey_date", "plotted_by", "checked_by", "surveyor_general", "prepared_by"];
  const fields = Object.fromEntries(keys.map((k) => [k, inp(k)]));
  // Surveyed by: each word starts with a capital (the server enforces it too).
  const capWords = (v) => v.replace(/(^|[^\p{L}\p{N}'’])(\p{L})/gu, (m, a, b) => a + b.toUpperCase());
  fields.surveyed_by.addEventListener("input", () => {
    const el = fields.surveyed_by, pos = el.selectionStart, v = capWords(el.value);
    if (v !== el.value) { el.value = v; el.setSelectionRange(pos, pos); }
  });
  const collect = () => {
    const out = {};
    for (const k of keys) out[k] = fields[k].value.trim() || null;
    out.lga = lga.value || null;
    out.scale = scale.value ? Number(scale.value) : null;
    out.corner_names = names.map((x) => x.value.trim() || null);
    out.road_side = null;
    out.road_sides = roadBoxes.flatMap((b, i) => (b.checked ? [i] : []));
    out.grid = gridOn.checked;
    out.grid_corner = null;
    out.road_label = roadLabel.value.trim() || null;
    out.features = feats;
    out.label_moves = moves;
    out.notes = notes;
    out.styles = styles;
    return out;
  };
  const msg = h("div", { style: "min-height:1.2em;margin:4px 0" });
  const save = async () => {
    const r = await api(`/api/v1/wf/jobs/${jobId}/titledeed`, { method: "PUT", body: { upin: p.upin, fields: collect() } });
    msg.style.color = "#060"; msg.textContent = "Saved with the job.";
    return r;
  };
  // Edit plan: the sheet full size, as a drawing tool. Roads and other features are drawn on it as
  // curves or straight lines, and reshaped by their points; labels are dragged, recoloured and
  // retyped; text can be added. The beacons and the bearings, distances and area stay as computed
  // from the registered co-ordinates. Nothing is saved until Done: the sheet redraws from the
  // server's preview, and Undo / Cancel work on the editor's own copy.
  const edit = async () => {
    const fail = (e) => { msg.style.color = "#900"; msg.textContent = e.message; hint.textContent = "The sheet could not be redrawn: " + e.message; };
    const SVG = "http://www.w3.org/2000/svg";
    const page = h("div", { class: "peSheet" });
    const hint = h("div", { style: "min-height:1.3em" });
    let el = null, ui = null, zoom = 1, sel = null, node = null, drag = null, draw = null, seq = 0, tool = "select", space = false, pivot = null;
    // What Undo and Cancel put back.
    const snap = () => JSON.stringify({ feats, moves, styles, notes, names: names.map((x) => x.value), sides: roadBoxes.map((b) => b.checked) });
    const restore = (j) => {
      const v = JSON.parse(j);
      feats.splice(0, feats.length, ...v.feats); notes.splice(0, notes.length, ...v.notes);
      moves = v.moves; styles = v.styles;
      names.forEach((x, i) => { x.value = v.names[i]; x.dispatchEvent(new Event("input")); });
      roadBoxes.forEach((b, i) => { b.checked = v.sides[i]; });
      renderFeats();
    };
    const opened = snap();
    const undo = [], redo = [];
    const mark = () => { undo.push(snap()); redo.length = 0; if (undo.length > 100) undo.shift(); bars(); };
    const step = (from, to) => { if (!from.length) return; to.push(snap()); restore(from.pop()); sel = null; node = null; fillPanel(); show().catch(fail); bars(); };
    // The form as it prints, for the preview: what is left blank falls back on the prefill.
    const printed = () => {
      const out = collect();
      for (const k of [...keys, "lga", "road_label"]) out[k] = out[k] ?? d.prefill[k] ?? null;
      return out;
    };
    const show = async () => {
      const mine = ++seq;
      const svg = await api(`/api/v1/wf/jobs/${jobId}/titledeed/preview`, { method: "POST", body: { upin: p.upin, fields: printed() } });
      if (mine !== seq) return; // a newer change is already on its way
      page.innerHTML = svg;
      el = page.querySelector("svg");
      el.removeAttribute("height"); el.setAttribute("width", "100%"); el.style.display = "block";
      scaleText.textContent = `Scale 1:${Math.round(1000 / grid()[2])}`;
      // Thin lines are hard to hit: a wide invisible copy under the pointer. It lives only here;
      // the printed sheet comes from the server.
      for (const g of el.querySelectorAll("[data-feat],[data-road]")) for (const l of [...g.querySelectorAll("polyline,line")]) {
        const c = l.cloneNode(); c.setAttribute("stroke", "#000"); c.setAttribute("stroke-opacity", "0"); c.setAttribute("stroke-width", "3"); c.removeAttribute("stroke-dasharray"); c.setAttribute("pointer-events", "stroke"); g.append(c);
      }
      for (const g of el.querySelectorAll("[data-feat]")) {
        const q = nodesOf(g), x = feats[Number(g.getAttribute("data-feat"))], gap = symOf(x?.symbol)?.double ? x.width ?? symOf(x.symbol).double : 0;
        if (q.length === 1) g.append(svgEl("circle", { cx: q[0][0], cy: q[0][1], r: 2.5, "fill-opacity": 0, "pointer-events": "all" }));
        else if (gap) g.append(grabBand(through(q, x.smooth), gap)); // a double line is held anywhere between its lines
      }
      for (const g of el.querySelectorAll("[data-road]")) g.append(grabBand(nodesOf(g), 8));
      ui = document.createElementNS(SVG, "g");
      el.append(ui);
      overlay();
    };
    // Paper mm <-> ground co-ordinates, through the sheet's data-grid.
    const grid = () => el.getAttribute("data-grid").split(" ").map(Number);
    const mmAt = (e) => { const q = new DOMPoint(e.clientX, e.clientY).matrixTransform(el.getScreenCTM().inverse()); return [q.x, q.y]; };
    const toGround = ([x, y]) => { const [e0, n0, k, x0, y1] = grid(); const r = (v) => Math.round(v * 1000) / 1000; return [r(e0 + (x - x0) / k), r(n0 + (y1 - y) / k)]; };
    const toPaper = ([e, n]) => { const [e0, n0, k, x0, y1] = grid(); return [x0 + (e - e0) * k, y1 - (n - n0) * k]; };
    const nodesOf = (g) => g.getAttribute("data-nodes").split(" ").filter(Boolean).map((q) => q.split(",").map(Number));
    const featEl = (i) => el.querySelector(`[data-feat="${i}"]`);
    // A feature on a side has no points of its own: it takes them from the sheet when first shaped.
    const freeFeature = (i) => {
      const x = feats[i];
      if (x.side != null || !x.points?.length) { x.points = nodesOf(featEl(i)).map(toGround); x.side = null; }
      return x;
    };
    const svgEl = (name, attrs) => { const n = document.createElementNS(SVG, name); for (const [k, v] of Object.entries(attrs)) n.setAttribute(k, v); return n; };
    // The curve the server draws (centripetal Catmull-Rom), for the outline while dragging.
    const curve = (pts) => {
      if (pts.length < 3) return pts;
      const dd = (a, b) => Math.hypot(a[0] - b[0], a[1] - b[1]), n = pts.length;
      const at = (i) => i < 0 ? [2 * pts[0][0] - pts[1][0], 2 * pts[0][1] - pts[1][1]] : i >= n ? [2 * pts[n - 1][0] - pts[n - 2][0], 2 * pts[n - 1][1] - pts[n - 2][1]] : pts[i];
      const mix = (a, b, ta, tb, t) => { const w = (t - ta) / ((tb - ta) || 1e-9); return [a[0] + (b[0] - a[0]) * w, a[1] + (b[1] - a[1]) * w]; };
      const out = [pts[0]];
      for (let i = 0; i < n - 1; i++) {
        const p0 = at(i - 1), p1 = pts[i], p2 = pts[i + 1], p3 = at(i + 2);
        const t1 = Math.sqrt(dd(p0, p1)), t2 = t1 + Math.sqrt(dd(p1, p2)), t3 = t2 + Math.sqrt(dd(p2, p3));
        for (let s = 1; s <= 16; s++) {
          const t = t1 + (t2 - t1) * s / 16;
          const a1 = mix(p0, p1, 0, t1, t), a2 = mix(p1, p2, t1, t2, t), a3 = mix(p2, p3, t2, t3, t);
          out.push(mix(mix(a1, a2, 0, t2, t), mix(a2, a3, t1, t3, t), t1, t2, t));
        }
      }
      return out;
    };
    const through = (pts, smooth) => (smooth ? curve(pts) : pts);
    const grabBand = (pts, width) => svgEl("polyline", { points: pts.map((q) => q.join(",")).join(" "), fill: "none", stroke: "#000", "stroke-opacity": 0, "stroke-width": width, "pointer-events": "stroke" });
    // Selection marks: the outline and its points (round: drag; small square: drag to add a point).
    const overlay = () => {
      if (!ui) return;
      ui.replaceChildren();
      const r = 1.1 / zoom;
      if (draw) {
        ui.append(svgEl("polyline", { points: through(draw.pts, draw.smooth).map((q) => q.join(",")).join(" "), fill: "none", stroke: "#f0f", "stroke-width": 0.35 / zoom }));
        for (const q of draw.pts) ui.append(svgEl("circle", { cx: q[0], cy: q[1], r, fill: "#f0f", "pointer-events": "none" }));
        return;
      }
      if (tool === "rotate") {
        const g = selGroup();
        if (!g || !canTurn()) return;
        const c = pivotOf();
        if (g.hasAttribute("data-nodes") && nodesOf(g).length > 1) {
          const line = sel.kind === "feat" ? through(nodesOf(g), feats[sel.i].smooth) : nodesOf(g);
          ui.append(svgEl("polyline", { points: line.map((q) => q.join(",")).join(" "), fill: "none", stroke: "#06c", "stroke-width": 0.25 / zoom, "stroke-opacity": 0.7, "pointer-events": "none" }));
        }
        const free = sel.kind === "feat" || sel.kind === "road"; // a label turns about its own place
        ui.append(svgEl("circle", { cx: c[0], cy: c[1], r: 2 * r, fill: "#fff", "fill-opacity": 0.7, stroke: "#f60", "stroke-width": 0.3 / zoom, ...(free ? { "data-pivot": 1, style: "cursor:move" } : { "pointer-events": "none" }) }),
          svgEl("path", { d: `M${c[0] - 3 * r} ${c[1]}H${c[0] + 3 * r}M${c[0]} ${c[1] - 3 * r}V${c[1] + 3 * r}`, stroke: "#f60", "stroke-width": 0.25 / zoom, "pointer-events": "none" }));
        return;
      }
      if (sel?.kind !== "feat") return;
      const x = feats[sel.i], g = featEl(sel.i);
      if (!x || !g) return;
      const pts = nodesOf(g);
      if (symOf(x.symbol)?.point) { ui.append(svgEl("circle", { cx: pts[0][0], cy: pts[0][1], r: 3 / zoom, fill: "none", stroke: "#06c", "stroke-width": 0.3 / zoom, "pointer-events": "none" })); return; }
      const line = through(pts, x.smooth);
      ui.append(svgEl("polyline", { points: line.map((q) => q.join(",")).join(" "), fill: "none", stroke: "#06c", "stroke-width": 0.25 / zoom, "stroke-opacity": 0.7, "pointer-events": "none" }));
      for (let i = 0; i + 1 < pts.length; i++) {
        const m = x.smooth && pts.length > 2 ? line[i * 16 + 8] : [(pts[i][0] + pts[i + 1][0]) / 2, (pts[i][1] + pts[i + 1][1]) / 2];
        ui.append(svgEl("rect", { x: m[0] - r * 0.7, y: m[1] - r * 0.7, width: r * 1.4, height: r * 1.4, fill: "#fff", stroke: "#06c", "stroke-width": 0.2 / zoom, "data-mid": i, style: "cursor:copy" }));
      }
      pts.forEach((q, i) => ui.append(svgEl("circle", { cx: q[0], cy: q[1], r, fill: i === node ? "#f60" : "#06c", stroke: "#fff", "stroke-width": 0.2 / zoom, "data-node": i, style: "cursor:grab" })));
    };
    // The panel: what the selected thing can do.
    const panel = h("div", { "data-panel": "props", style: "flex:1 1 55%;min-height:0;padding:10px;overflow:auto;box-sizing:border-box" });
    const layers = h("div", { "data-panel": "layers", style: "flex:1 1 45%;min-height:0;overflow:auto" });
    // Everything drawn on the plan, to pick it from a list as well as on the sheet.
    const fillLayers = () => {
      const same = (a, b) => a?.kind === b.kind && a.i === b.i && a.side === b.side;
      const row = (s0, colour, text) => h("div", { class: `peLayer${same(sel, s0) ? " on" : ""}`, title: text, onclick: () => select(s0) }, h("span", { class: "peSwatch", style: `background:${colour}` }), text);
      const six = (c) => (c || "#000").replace(/^#(.)(.)(.)$/, "#$1$1$2$2$3$3");
      const rows = [
        ...notes.map((x, i) => row({ kind: "note", i }, x.color || "#000000", `Text: ${x.text}`)).reverse(),
        ...feats.map((x, i) => row({ kind: "feat", i }, six(x.color || symOf(x.symbol)?.stroke), `${symOf(x.symbol)?.label || x.symbol}${x.label ? `: ${x.label}` : ""}`)).reverse(),
        ...roadBoxes.flatMap((b, side) => (b.checked ? [row({ kind: "road", side }, styles[`roadline:${side}`]?.color || "#000000", `Access road, side ${side + 1}`)] : [])),
      ];
      layers.replaceChildren(...(rows.length ? rows : [h("div", { style: "padding:10px;color:#aaa" }, "Nothing drawn yet.")]));
    };
    const prop = (label, input) => h("label", { style: "display:block;margin:8px 0" }, h("div", { style: "margin-bottom:2px;color:#ccc" }, label), input);
    const wide = "width:100%;box-sizing:border-box";
    const colourBox = (value, set, clear) => h("div", { style: "display:flex;gap:6px" },
      h("input", { type: "color", value: value, style: "flex:1;height:26px", onchange: (e) => set(e.target.value) }),
      h("button", { class: "btn", onclick: clear }, "Default"));
    const changed = () => { show().catch(fail); fillPanel(); };
    const removeSelected = () => {
      if (sel?.kind === "feat") { mark(); feats.splice(sel.i, 1); dropFeatureMove(sel.i); renderFeats(); }
      else if (sel?.kind === "note") { mark(); notes.splice(sel.i, 1); }
      else return;
      sel = null; node = null; changed();
    };
    const deleteNode = () => {
      if (nodesOf(featEl(sel.i)).length <= 2) { hint.textContent = "A line needs at least two points. Use Delete feature to remove it."; return; }
      mark(); freeFeature(sel.i).points.splice(node, 1); node = null; show().catch(fail);
    };
    // A straight access road becomes a feature with points, so that it can bend.
    const freeRoad = (side, dx = 0, dy = 0, shape = (q) => q) => {
      mark();
      const g = el.querySelector(`[data-road="${side}"]`);
      const key = `road:${side}`, i = feats.length;
      feats.push({ symbol: "road_access", label: styles[key]?.text || roadLabel.value.trim() || d.prefill.road_label || "Access Road", side: null,
        points: nodesOf(g).map(shape).map(([x, y]) => toGround([x + dx, y + dy])), smooth: true, color: styles[`roadline:${side}`]?.color || null, width: null });
      if (moves[key]) { moves[`feature:${i}`] = moves[key]; delete moves[key]; }
      if (styles[key]?.color || styles[key]?.angle) styles[`feature:${i}`] = { color: styles[key].color ?? null, text: null, angle: styles[key].angle ?? null };
      delete styles[key]; delete styles[`roadline:${side}`];
      roadBoxes[side].checked = false;
      sel = { kind: "feat", i }; renderFeats(); changed();
    };
    // Copy and paste: the copy has its own points, so it is edited apart from the original. Each
    // paste lands a little further on, to be dragged into place; one from another plan, whose
    // ground lies off this sheet, comes in at the middle of the frame.
    const copySelected = () => {
      if (sel?.kind === "feat") {
        const x = feats[sel.i];
        tdClip = { feat: { ...x, side: null, points: x.side == null && x.points?.length ? x.points.map((q) => [...q]) : nodesOf(featEl(sel.i)).map(toGround) }, pasted: 0 };
      } else if (sel?.kind === "road") {
        const key = `road:${sel.side}`;
        tdClip = { feat: { symbol: "road_access", label: styles[key]?.text || roadLabel.value.trim() || "Access Road", side: null,
          points: nodesOf(el.querySelector(`[data-road="${sel.side}"]`)).map(toGround), smooth: true, color: styles[`roadline:${sel.side}`]?.color || null, width: null }, pasted: 0 };
      } else { hint.textContent = "Select a road or feature to copy it."; return; }
      hint.textContent = "Copied. Ctrl+V or Paste puts in a copy.";
      bars();
    };
    const paste = () => {
      if (!tdClip || draw) return;
      if (!symOf(tdClip.feat.symbol)) { hint.textContent = "That symbol is not on this plan."; return; }
      const [, , , fx0, fy1] = grid();
      let pts = tdClip.feat.points.map(toPaper);
      if (!pts.some(([x, y]) => x >= fx0 && x <= 210 - fx0 && y >= 60 && y <= fy1)) {
        const cx = pts.reduce((a, q) => a + q[0], 0) / pts.length, cy = pts.reduce((a, q) => a + q[1], 0) / pts.length;
        pts = pts.map(([x, y]) => [x - cx + 105, y - cy + (60 + fy1) / 2]);
      }
      const off = 5 * ++tdClip.pasted;
      mark();
      feats.push({ ...tdClip.feat, points: pts.map(([x, y]) => toGround([x + off, y + off])) });
      sel = { kind: "feat", i: feats.length - 1 }; node = null;
      renderFeats(); changed();
      hint.textContent = "Pasted. Drag it into place.";
    };
    // The arrow keys move the selected line or feature (mm on the sheet).
    const nudge = (dx, dy) => {
      if (sel?.kind === "road") { freeRoad(sel.side, dx, dy); return; }
      if (sel?.kind !== "feat") return;
      mark();
      const x = freeFeature(sel.i);
      x.points = x.points.map((q) => { const [a, b] = toPaper(q); return toGround([a + dx, b + dy]); });
      show().catch(fail);
    };
    const styleOf = (key) => (styles[key] ||= { color: null, text: null });
    const tidyStyle = (key) => { if (styles[key] && !styles[key].color && !styles[key].text && !styles[key].angle) delete styles[key]; };
    // Rotate: a line or road turns about its pivot (the middle, or where it is dragged to), and
    // its points are stored turned; a label or text turns about the place it is written. Angles
    // are clockwise. The beacons and the parcel have no part in it.
    const selGroup = () => !sel ? null : sel.kind === "feat" ? featEl(sel.i) : sel.kind === "road" ? el.querySelector(`[data-road="${sel.side}"]`)
      : el.querySelector(`[data-move="${sel.kind === "note" ? `note:${sel.i}` : sel.key}"]`);
    const pivotOf = () => {
      const g = selGroup();
      if (g && !g.hasAttribute("data-nodes")) return g.getAttribute("data-at").split(" ").map(Number);
      if (pivot?.of === JSON.stringify(sel)) return pivot.at; // set for this selection, not an earlier one
      const q = nodesOf(g), xs = q.map((v) => v[0]), ys = q.map((v) => v[1]);
      return [(Math.min(...xs) + Math.max(...xs)) / 2, (Math.min(...ys) + Math.max(...ys)) / 2];
    };
    const setPivot = (at) => { pivot = { at, of: JSON.stringify(sel) }; };
    const turned = (c, deg) => ([x, y]) => {
      const a = deg * Math.PI / 180, dx = x - c[0], dy = y - c[1];
      return [c[0] + dx * Math.cos(a) - dy * Math.sin(a), c[1] + dx * Math.sin(a) + dy * Math.cos(a)];
    };
    const wrap = (a) => { const v = Math.round((((a + 180) % 360 + 360) % 360 - 180) * 10) / 10; return v === 0 ? null : v; };
    const canTurn = () => sel && !(sel.kind === "feat" && symOf(feats[sel.i].symbol)?.point);
    const applyTurn = (deg) => {
      if (!sel) { hint.textContent = "Click a road, label or text to rotate it."; return; }
      if (!canTurn()) { hint.textContent = "A point symbol has no direction to turn."; return; }
      const c = pivotOf(), turn = turned(c, deg);
      if (sel.kind === "road") { freeRoad(sel.side, 0, 0, turn); setPivot(c); overlay(); return; }
      mark();
      if (sel.kind === "feat") { setPivot(c); const x = freeFeature(sel.i); x.points = x.points.map((q) => toGround(turn(toPaper(q)))); }
      else if (sel.kind === "note") notes[sel.i].angle = wrap((notes[sel.i].angle || 0) + deg);
      else { styleOf(sel.key).angle = wrap((styles[sel.key]?.angle || 0) + deg); tidyStyle(sel.key); }
      changed();
    };
    const straighten = () => {
      if (sel?.kind === "note") { mark(); notes[sel.i].angle = null; changed(); }
      else if (sel?.kind === "label" && styles[sel.key]?.angle) { mark(); styles[sel.key].angle = null; tidyStyle(sel.key); changed(); }
    };
    const fillPanel = () => {
      const rows = [];
      if (!sel) {
        rows.push(h("b", {}, "Nothing selected"), h("p", { style: "color:#ccc;line-height:1.4" },
          "Click a road, feature or label to edit it, or pick it under Layers. The tools on the left draw lines, symbols and text."));
      } else if (sel.kind === "feat") {
        const x = feats[sel.i], y = symOf(x.symbol);
        rows.push(h("b", {}, y?.point ? "Point feature" : "Line feature"));
        rows.push(prop("Type", h("select", { style: wide, onchange: (e) => { mark(); x.symbol = e.target.value; x.width = null; changed(); } },
          ...d.symbols.filter((s) => s.point === !!y?.point).map((s) => h("option", { value: s.id, selected: s.id === x.symbol }, s.label)))));
        if (!y?.point) rows.push(h("label", { style: "display:block;margin:8px 0" }, h("input", { type: "checkbox", checked: !!x.smooth, onchange: (e) => { mark(); x.smooth = e.target.checked; changed(); } }), " Smooth curve (spline)"));
        rows.push(prop("Colour", colourBox(x.color || y?.stroke.replace(/^#(.)(.)(.)$/, "#$1$1$2$2$3$3") || "#000000", (c) => { mark(); x.color = c; changed(); }, () => { mark(); x.color = null; changed(); })));
        if (y?.double) rows.push(prop("Width between lines (mm on the sheet)", h("input", { type: "number", min: 2, max: 30, step: 0.5, value: x.width ?? y.double, style: wide, onchange: (e) => { mark(); x.width = Number(e.target.value) || null; changed(); } })));
        rows.push(prop("Label", h("input", { value: x.label || "", style: wide, onchange: (e) => { mark(); x.label = e.target.value.trim() || null; renderFeats(); changed(); } })));
        if (!y?.point) rows.push(h("p", { style: "color:#ccc;line-height:1.4" }, "Drag a round point to reshape, or a small square to add a point. Double-click a point or press Delete to remove it. Drag the line, or use the arrow keys (Shift for bigger steps), to move all of it."));
        rows.push(h("button", { class: "btn", onclick: removeSelected }, "Delete feature"));
      } else if (sel.kind === "road") {
        const key = `roadline:${sel.side}`;
        rows.push(h("b", {}, `Access road, ${sideText(sel.side)}`));
        rows.push(prop("Colour", colourBox(styles[key]?.color || "#000000", (c) => { mark(); styleOf(key).color = c; changed(); }, () => { mark(); delete styles[key]; changed(); })));
        rows.push(h("button", { class: "btn", style: wide, onclick: () => freeRoad(sel.side) }, "Make editable (bend it, change its type)"));
        rows.push(h("p", { style: "color:#ccc;line-height:1.4" }, "Drag it or use the arrow keys to move it. Moved or made editable, it becomes a drawn road: it no longer follows the side, and its rounded junction with a neighbouring access road goes."));
      } else if (sel.kind === "note") {
        const x = notes[sel.i];
        rows.push(h("b", {}, "Text"));
        rows.push(prop("Words", h("input", { value: x.text, style: wide, onchange: (e) => { mark(); x.text = e.target.value; changed(); } })));
        rows.push(prop("Colour", colourBox(x.color || "#000000", (c) => { mark(); x.color = c; changed(); }, () => { mark(); x.color = null; changed(); })));
        rows.push(prop("Size (mm)", h("input", { type: "number", min: 1.5, max: 12, step: 0.1, value: x.size ?? 2.8, style: wide, onchange: (e) => { mark(); x.size = Number(e.target.value) || null; changed(); } })));
        rows.push(prop("Turn (degrees)", h("input", { type: "number", min: -180, max: 180, step: 1, value: x.angle ?? 0, style: wide, onchange: (e) => { mark(); x.angle = Number(e.target.value) || null; changed(); } })));
        rows.push(h("button", { class: "btn", onclick: removeSelected }, "Delete text"));
      } else {
        const key = sel.key, [kind, idx] = key.split(":"), i = Number(idx);
        rows.push(h("b", {}, "Label"));
        const words = (value, set) => rows.push(prop("Words", h("input", { value, style: wide, onchange: (e) => { mark(); set(e.target.value.trim()); changed(); } })));
        if (kind === "beacon") words(names[i].value || p.corners[i].name, (v) => { names[i].value = v === p.corners[i].name ? "" : v; names[i].dispatchEvent(new Event("input")); });
        else if (kind === "road") words(styles[key]?.text || roadLabel.value || "Access Road", (v) => { styleOf(key).text = v || null; tidyStyle(key); });
        else if (kind === "feature") words(feats[i].label || "", (v) => { feats[i].label = v || null; renderFeats(); });
        else rows.push(h("p", { style: "color:#ccc;line-height:1.4" }, "Computed from the registered co-ordinates, so its words can't be changed. It can be moved and recoloured."));
        rows.push(prop("Colour", colourBox(styles[key]?.color || "#000000", (c) => { mark(); styleOf(key).color = c; changed(); }, () => { mark(); if (styles[key]) { styles[key].color = null; tidyStyle(key); } changed(); })));
        rows.push(prop("Turn (degrees)", h("input", { type: "number", min: -180, max: 180, step: 1, value: styles[key]?.angle ?? 0, style: wide, onchange: (e) => { mark(); styleOf(key).angle = wrap(Number(e.target.value) || 0); tidyStyle(key); changed(); } })));
        rows.push(h("button", { class: "btn", onclick: () => { mark(); delete moves[key]; if (styles[key]) { styles[key].angle = null; tidyStyle(key); } changed(); } }, "Put it back"));
      }
      panel.replaceChildren(...rows);
      fillLayers();
    };
    const select = (s) => { sel = s; node = null; pivot = null; fillPanel(); overlay(); };

    // The workspace, laid out as in an image editor: tools down the left, the tool's options along
    // the top, Properties and Layers on the right, and a status bar.
    const lineSym = symSel.cloneNode(true), spotSym = symSel.cloneNode(true);
    for (const g of [...lineSym.querySelectorAll("optgroup")]) if (g.label === "Point features") g.remove();
    for (const g of [...spotSym.querySelectorAll("optgroup")]) if (g.label !== "Point features") g.remove();
    lineSym.setAttribute("data-opt", "line"); spotSym.setAttribute("data-opt", "spot");
    const adjPlace = placeSel.cloneNode(true);
    adjPlace.options[0].textContent = "Draw on the sheet";
    const adjLabel = h("input", { placeholder: "Label (optional)", style: "width:130px" });
    const adjSmooth = h("input", { type: "checkbox", checked: true });
    const sideBtn = h("button", { class: "btn" }, "Add along the side");
    const undoBtn = h("button", { class: "btn", title: "Ctrl+Z", onclick: () => step(undo, redo) }, "Undo");
    const redoBtn = h("button", { class: "btn", title: "Ctrl+Y", onclick: () => step(redo, undo) }, "Redo");
    const copyBtn = h("button", { class: "btn", title: "Copy the selected road or feature (Ctrl+C)", onclick: copySelected }, "Copy");
    const pasteBtn = h("button", { class: "btn", title: "Put in a copy (Ctrl+V)", onclick: paste }, "Paste");
    const zoomText = h("span", {}, "100%");
    const scaleText = h("span", {}, "");
    const cursorText = h("span", { style: "font-variant-numeric:tabular-nums" }, "");
    const bars = () => { undoBtn.disabled = !undo.length; redoBtn.disabled = !redo.length; pasteBtn.disabled = !tdClip; };
    const setZoom = (z) => { zoom = Math.min(6, Math.max(0.5, z)); page.style.width = `${900 * zoom}px`; zoomText.textContent = `${Math.round(zoom * 100)}%`; overlay(); };
    const idle = "Click something to edit it. Drag labels, points and lines. Ctrl+wheel zooms, Space drags the sheet.";
    const TOOLS = [
      ["select", "V", "Select and move", '<path d="M4 2l11 10-5 .6 3 6-2.4 1-2.8-6L4 17z" fill="currentColor"/>'],
      ["rotate", "R", "Rotate a road, label or text", '<path d="M16.500 10a6.500 6.500 0 11-2-4.700" fill="none" stroke="currentColor" stroke-width="2"/><path d="M12 6.500h5.500V1z" fill="currentColor"/>'],
      ["pen", "P", "Draw a road or line", '<path d="M2 17C6 3 12 17 18 3" fill="none" stroke="currentColor" stroke-width="1.8"/><rect x="0.5" y="15" width="4" height="4" fill="currentColor"/><rect x="15.5" y="1" width="4" height="4" fill="currentColor"/>'],
      ["symbol", "S", "Place a tree, pole or other point", '<circle cx="10" cy="10" r="7" fill="none" stroke="currentColor" stroke-width="1.8"/><circle cx="10" cy="10" r="2" fill="currentColor"/>'],
      ["text", "T", "Add text", '<path d="M3 3h14v4h-1.5l-.5-2H11.2v10.5l2 .5V18H6.8v-2l2-.5V5H5L4.500 7H3z" fill="currentColor"/>'],
      ["hand", "H", "Drag the sheet (or hold Space)", '<path d="M6 10V4.500a1.200 1.200 0 012.400 0V9h.6V3.200a1.200 1.200 0 012.400 0V9h.6V4.200a1.200 1.200 0 012.400 0V12c0 4-2 6-5.500 6-3 0-4.200-1.500-5.500-4L1.800 10.500c-.7-1.300 1.100-2.300 2-1.200z" fill="currentColor"/>'],
      ["zoom", "Z", "Zoom: click to zoom in, Alt+click out", '<circle cx="8" cy="8" r="5.500" fill="none" stroke="currentColor" stroke-width="2"/><path d="M12 12l6 6" stroke="currentColor" stroke-width="2.500"/>'],
    ];
    const toolBtns = Object.fromEntries(TOOLS.map(([id, k, name, icon]) => [id, h("button", { class: "peTool", title: `${name} (${k})`, "data-tool": id, html: `<svg viewBox="0 0 20 20" width="20" height="20">${icon}</svg>`, onclick: () => setEditTool(id) })]));
    const options = h("div", { class: "peRow" });
    const zoomOptions = () => [h("button", { class: "btn", onclick: () => setZoom(zoom / 1.25) }, "Zoom out"), h("button", { class: "btn", onclick: () => setZoom(zoom * 1.25) }, "Zoom in"), h("button", { class: "btn", onclick: () => setZoom(1) }, "Fit")];
    const fillOptions = () => {
      const place = [h("span", {}, "Place:"), adjPlace, h("span", {}, "Label:"), adjLabel];
      const add = adjPlace.value !== "" ? [sideBtn] : [];
      options.replaceChildren(...{
        select: () => [h("span", {}, "Select and move"), h("button", { class: "btn", onclick: () => { mark(); moves = {}; for (const k of Object.keys(styles)) { styles[k].angle = null; tidyStyle(k); } show().catch(fail); } }, "Reset all labels")],
        pen: () => [h("span", {}, "Type:"), lineSym, ...place, h("label", { style: "white-space:nowrap" }, adjSmooth, " Curve"), ...add],
        symbol: () => [h("span", {}, "Symbol:"), spotSym, ...place, ...add],
        text: () => [h("span", {}, "Click the sheet where the text starts.")],
        rotate: () => {
          const by = h("input", { type: "number", "data-opt": "angle", min: -360, max: 360, step: 1, value: 15, style: "width:60px" });
          return [h("span", {}, "Angle:"), by, h("span", {}, "°"), h("button", { class: "btn", title: "Clockwise", onclick: () => applyTurn(Number(by.value) || 0) }, "Rotate"),
            h("button", { class: "btn", onclick: () => applyTurn(-90) }, "Left 90°"), h("button", { class: "btn", onclick: () => applyTurn(90) }, "Right 90°"),
            h("button", { class: "btn", title: "A label or text back to how the sheet writes it", onclick: straighten }, "Straighten")];
        },
        hand: () => [h("span", {}, "Drag the sheet to look around it."), ...zoomOptions()],
        zoom: () => zoomOptions(),
      }[tool]());
    };
    const toolHints = {
      rotate: "Click what to rotate, then drag around the orange pivot (Shift: 15° steps). Drag the pivot to turn a line about another point.",
      pen: "Click along it, double-click or Enter to finish. Backspace takes back a point. Esc cancels.",
      symbol: "Click its position on the sheet. Esc cancels.", text: "Click where the text starts. Esc cancels.",
      hand: "Drag to look around the sheet.", zoom: "Click to zoom in, Alt+click to zoom out.",
    };
    // A drawing tool is ready at once, unless it is to go along a side.
    const arm = () => {
      draw = null;
      if (tool === "text") draw = { pts: [], text: true };
      else if ((tool === "pen" || tool === "symbol") && adjPlace.value === "") draw = { pts: [], point: tool === "symbol", smooth: tool === "pen" && adjSmooth.checked };
    };
    const setEditTool = (t) => {
      tool = t; arm();
      if (draw) { sel = null; node = null; fillPanel(); }
      page.className = `peSheet tool-${t}${draw ? " drawing" : ""}`;
      for (const [id, b] of Object.entries(toolBtns)) b.classList.toggle("on", id === t);
      fillOptions(); hint.textContent = toolHints[t] || idle; overlay();
    };
    const stopDraw = () => setEditTool("select");
    const addFeat = (x) => { mark(); feats.push(x); adjLabel.value = ""; sel = { kind: "feat", i: feats.length - 1 }; node = null; renderFeats(); };
    const currentSym = () => (tool === "symbol" ? spotSym : lineSym).value;
    const finishDraw = () => {
      if (draw.text) return;
      const x = { symbol: currentSym(), label: adjLabel.value.trim() || null, side: null, points: draw.pts.map(toGround), smooth: draw.smooth, color: null, width: null };
      draw = null; addFeat(x); stopDraw(); changed();
    };
    adjPlace.addEventListener("change", () => setEditTool(tool));
    adjSmooth.addEventListener("change", () => { if (draw) { draw.smooth = adjSmooth.checked; overlay(); } });
    sideBtn.addEventListener("click", () => {
      addFeat({ symbol: currentSym(), label: adjLabel.value.trim() || null, side: Number(adjPlace.value), points: [], smooth: false, color: null, width: null });
      adjPlace.value = ""; stopDraw(); changed();
    });
    const onKey = (e) => {
      const typing = /^(INPUT|SELECT|TEXTAREA)$/.test(e.target.tagName);
      if ((e.ctrlKey || e.metaKey) && !typing && e.key.toLowerCase() === "z") { e.preventDefault(); if (e.shiftKey) step(redo, undo); else step(undo, redo); return; }
      if ((e.ctrlKey || e.metaKey) && !typing && e.key.toLowerCase() === "y") { e.preventDefault(); step(redo, undo); return; }
      if ((e.ctrlKey || e.metaKey) && !typing && e.key.toLowerCase() === "c") { e.preventDefault(); copySelected(); return; }
      if ((e.ctrlKey || e.metaKey) && !typing && e.key.toLowerCase() === "v") { e.preventDefault(); paste(); return; }
      if (typing) return;
      if (e.key === " ") { e.preventDefault(); document.activeElement?.blur?.(); space = true; page.classList.add("space"); return; }
      if (e.key === "Escape") { if (tool !== "select") stopDraw(); else select(null); return; }
      const picked = !e.ctrlKey && !e.metaKey && !e.altKey && TOOLS.find((t) => t[1] === e.key.toUpperCase());
      if (picked) { setEditTool(picked[0]); return; }
      if (draw) {
        if (e.key === "Enter" && !draw.point && !draw.text && draw.pts.length >= 2) finishDraw();
        if (e.key === "Backspace") { e.preventDefault(); draw.pts.pop(); overlay(); }
        return;
      }
      const arrow = { ArrowLeft: [-1, 0], ArrowRight: [1, 0], ArrowUp: [0, -1], ArrowDown: [0, 1] }[e.key];
      if (arrow && (sel?.kind === "feat" || sel?.kind === "road")) { e.preventDefault(); const by = e.shiftKey ? 5 : 1; nudge(arrow[0] * by, arrow[1] * by); return; }
      if (e.key === "Delete" || e.key === "Backspace") {
        e.preventDefault();
        if (sel?.kind === "feat" && node != null) deleteNode(); else removeSelected();
      }
    };
    const onKeyUp = (e) => { if (e.key === " ") { space = false; page.classList.remove("space"); } };
    document.addEventListener("keydown", onKey);
    document.addEventListener("keyup", onKeyUp);
    const close = () => { seq++; document.removeEventListener("keydown", onKey); document.removeEventListener("keyup", onKeyUp); ov.remove(); };
    const place = (g, key) => {
      const m = moves[key], a = styles[key]?.angle, at = g.getAttribute("data-at");
      const t = [m ? `translate(${m[0]} ${m[1]})` : "", a ? `rotate(${a} ${at})` : ""].filter(Boolean).join(" ");
      if (t) g.setAttribute("transform", t); else g.removeAttribute("transform");
    };
    const scroller = h("div", { class: "peBoard" }, page);
    const ov = h("div", { class: "peWork" },
      h("style", {}, `.peWork{position:fixed;inset:0;z-index:2000;display:flex;flex-direction:column;background:#2b2b2b;color:#ddd;font:12px sans-serif}
.peRow{display:flex;gap:8px;align-items:center;flex-wrap:wrap}.peBar{flex:none;padding:5px 10px;background:#3a3a3a;border-bottom:1px solid #1e1e1e}
.peWork .btn{background:#505050;color:#eee;border:1px solid #1e1e1e;border-radius:3px;padding:3px 9px;font:12px sans-serif;cursor:pointer}.peWork .btn:hover:not(:disabled){background:#626262}.peWork .btn:disabled{opacity:.4;cursor:default}
.peWork input:not([type=checkbox]):not([type=color]),.peWork select{background-color:#262626;color:#eee;border:1px solid #1a1a1a;border-radius:2px;padding:2px 4px;font:12px sans-serif}
.peWork select{appearance:none;-webkit-appearance:none;padding-right:18px;background-image:linear-gradient(45deg,transparent 50%,#bbb 50%),linear-gradient(135deg,#bbb 50%,transparent 50%);background-size:5px 5px;background-position:right 10px center,right 5px center;background-repeat:no-repeat}.peWork option,.peWork optgroup{background:#262626;color:#eee}
.peTools{flex:none;width:42px;padding:6px 0;background:#3a3a3a;border-right:1px solid #1e1e1e;display:flex;flex-direction:column;align-items:center;gap:3px}
.peTool{width:32px;height:32px;padding:6px;background:none;color:#ccc;border:0;border-radius:3px;cursor:pointer}.peTool:hover{background:#505050}.peTool.on{background:#1e6fc4;color:#fff}
.peBoard{flex:1;overflow:auto;background:#1f1f1f;padding:24px;min-width:0}.peSheet{margin:0 auto;background:#fff;box-shadow:0 0 12px #000;touch-action:none;user-select:none}
.peDock{flex:none;width:270px;display:flex;flex-direction:column;background:#3a3a3a;border-left:1px solid #1e1e1e;min-height:0}.peHead{flex:none;padding:4px 10px;background:#2f2f2f;border-bottom:1px solid #1e1e1e;font-weight:bold}
.peLayer{display:flex;gap:7px;align-items:center;padding:4px 10px;border-bottom:1px solid #303030;cursor:pointer;white-space:nowrap;overflow:hidden}.peLayer:hover{background:#464646}.peLayer.on{background:#1e6fc4;color:#fff}
.peSwatch{flex:none;width:12px;height:12px;border:1px solid #111}.peGap{flex:1}
.peStatus{flex:none;display:flex;gap:18px;align-items:center;padding:3px 10px;background:#2f2f2f;border-top:1px solid #1e1e1e}
.peSheet [data-move]{cursor:move}.peSheet [data-move]:hover text{fill:#06c}.peSheet [data-feat],.peSheet [data-road]{cursor:pointer}
.peSheet.drawing svg,.peSheet.drawing [data-move],.peSheet.drawing [data-feat],.peSheet.drawing [data-road]{cursor:crosshair}
.peSheet.tool-rotate svg,.peSheet.tool-rotate *{cursor:grab}.peSheet.tool-hand svg,.peSheet.tool-hand *,.peSheet.space svg,.peSheet.space *{cursor:grab!important}.peSheet.tool-zoom svg,.peSheet.tool-zoom *{cursor:zoom-in!important}`),
      h("div", { class: "peBar peRow" }, h("b", {}, `Edit plan: ${fields.title_no.value || p.upin}`), undoBtn, redoBtn, copyBtn, pasteBtn, h("span", { class: "peGap" }),
        h("button", { class: "btn", title: "Put the plan back as it was when the editor opened", onclick: () => { restore(opened); close(); } }, "Cancel"),
        h("button", { class: "btn", title: "Save the plan with the job", onclick: () => { close(); save().catch(fail); } }, "Done")),
      h("div", { class: "peBar", style: "min-height:24px" }, options),
      h("div", { style: "flex:1;display:flex;min-height:0" }, h("div", { class: "peTools" }, ...Object.values(toolBtns)), scroller,
        h("div", { class: "peDock" }, h("div", { class: "peHead" }, "Properties"), panel, h("div", { class: "peHead" }, "Layers"), layers)),
      h("div", { class: "peStatus" }, hint, h("span", { class: "peGap" }), cursorText, scaleText, zoomText));
    setZoom(1); bars(); fillPanel(); setEditTool("select");
    try { await show(); } catch (e) { close(); fail(e); return; }
    document.body.append(ov);
    scroller.addEventListener("wheel", (e) => { if (!e.ctrlKey) return; e.preventDefault(); setZoom(zoom * (e.deltaY < 0 ? 1.15 : 1 / 1.15)); }, { passive: false });
    let pressed = null; // the last point pressed: a second press soon after is a double-click
    page.addEventListener("pointerdown", (e) => {
      if (tool === "hand" || space) { drag = { kind: "pan", x: e.clientX, y: e.clientY, left: scroller.scrollLeft, top: scroller.scrollTop, moved: true }; e.preventDefault(); return; }
      if (tool === "zoom") {
        const r = scroller.getBoundingClientRect(), fx = (e.clientX - r.left + scroller.scrollLeft) / scroller.scrollWidth, fy = (e.clientY - r.top + scroller.scrollTop) / scroller.scrollHeight;
        setZoom(zoom * (e.altKey ? 1 / 1.5 : 1.5));
        scroller.scrollLeft = fx * scroller.scrollWidth - r.width / 2; scroller.scrollTop = fy * scroller.scrollHeight - r.height / 2;
        e.preventDefault(); return;
      }
      if (tool === "rotate") {
        if (panel.contains(document.activeElement)) document.activeElement.blur();
        e.preventDefault();
        if (e.target.dataset?.pivot != null) { drag = { kind: "pivot", moved: true }; return; }
        const label = e.target.closest("[data-move]"), f1 = e.target.closest("[data-feat]"), r1 = e.target.closest("[data-road]");
        const key = label?.getAttribute("data-move"), note = key && /^note:(\d+)$/.exec(key);
        const hit = note ? { kind: "note", i: Number(note[1]) } : label ? { kind: "label", key } : f1 ? { kind: "feat", i: Number(f1.getAttribute("data-feat")) } : r1 ? { kind: "road", side: Number(r1.getAttribute("data-road")) } : null;
        // A press on something else picks it; on it, or on the bare sheet, it starts the turn.
        if (hit && !(sel?.kind === hit.kind && sel.i === hit.i && sel.side === hit.side && sel.key === hit.key)) { select(hit); return; }
        if (!sel) { hint.textContent = "Click a road, label or text to rotate it."; return; }
        if (!canTurn()) { hint.textContent = "A point symbol has no direction to turn."; return; }
        const c = pivotOf(), q = mmAt(e), g = selGroup();
        drag = { kind: "turn", c, a0: Math.atan2(q[1] - c[1], q[0] - c[0]), g, base: g.getAttribute("transform") || "", deg: 0, moved: true };
        return;
      }
      if (draw) return;
      // What is being typed in the panel counts before the selection moves on.
      if (panel.contains(document.activeElement)) document.activeElement.blur();
      const start = { x: e.clientX, y: e.clientY, mm: el.getScreenCTM().a, moved: false };
      const t = e.target;
      if (t.dataset?.node != null) {
        node = Number(t.dataset.node);
        const twice = pressed?.i === node && e.timeStamp - pressed.at < 400;
        pressed = { i: node, at: e.timeStamp };
        e.preventDefault();
        if (twice) { pressed = null; deleteNode(); return; }
        const x = feats[sel.i], pts = nodesOf(featEl(sel.i));
        drag = { ...start, kind: "node", x0: x, pts, i: node };
        overlay(); e.preventDefault(); return;
      }
      if (t.dataset?.mid != null) {
        const x = feats[sel.i], pts = nodesOf(featEl(sel.i)), i = Number(t.dataset.mid) + 1;
        const at = [Number(t.getAttribute("x")) + Number(t.getAttribute("width")) / 2, Number(t.getAttribute("y")) + Number(t.getAttribute("height")) / 2];
        pts.splice(i, 0, at);
        node = i;
        drag = { ...start, kind: "node", x0: x, pts, i, added: true };
        e.preventDefault(); return;
      }
      const label = t.closest("[data-move]");
      if (label) {
        const key = label.getAttribute("data-move"), note = /^note:(\d+)$/.exec(key);
        select(note ? { kind: "note", i: Number(note[1]) } : { kind: "label", key });
        drag = { ...start, kind: "label", g: label, key, base: moves[key] || [0, 0], before: snap(), note: note ? Number(note[1]) : null };
        e.preventDefault(); return;
      }
      const f0 = t.closest("[data-feat]");
      if (f0) {
        const i = Number(f0.getAttribute("data-feat"));
        select({ kind: "feat", i });
        drag = { ...start, kind: "line", g: f0, i, pts: nodesOf(f0), before: snap() };
        e.preventDefault(); return;
      }
      const r0 = t.closest("[data-road]");
      if (r0) {
        const side = Number(r0.getAttribute("data-road"));
        select({ kind: "road", side });
        drag = { ...start, kind: "road", g: r0, side };
        e.preventDefault(); return;
      }
      select(null);
    });
    ov.addEventListener("pointermove", (e) => {
      if (el && page.contains(e.target)) {
        const q = mmAt(e), [ge, gn] = toGround(q);
        cursorText.textContent = `E ${ge.toFixed(3)}   N ${gn.toFixed(3)}`;
      }
      if (!drag) return;
      if (drag.kind === "pivot") { setPivot(mmAt(e)); overlay(); return; }
      if (drag.kind === "turn") {
        const q = mmAt(e), raw = (Math.atan2(q[1] - drag.c[1], q[0] - drag.c[0]) - drag.a0) * 180 / Math.PI;
        drag.deg = e.shiftKey ? Math.round(raw / 15) * 15 : Math.round(raw * 10) / 10;
        drag.g.setAttribute("transform", `rotate(${drag.deg} ${drag.c[0]} ${drag.c[1]}) ${drag.base}`);
        hint.textContent = `Rotating ${wrap(drag.deg) ?? 0}°`;
        return;
      }
      if (drag.kind === "pan") { scroller.scrollLeft = drag.left - (e.clientX - drag.x); scroller.scrollTop = drag.top - (e.clientY - drag.y); return; }
      const dx = (e.clientX - drag.x) / drag.mm, dy = (e.clientY - drag.y) / drag.mm;
      if (!drag.moved && Math.hypot(e.clientX - drag.x, e.clientY - drag.y) < 3) return;
      drag.moved = true;
      if (drag.kind === "label") {
        const r = (v) => Math.round(v * 100) / 100;
        moves[drag.key] = [r(drag.base[0] + dx), r(drag.base[1] + dy)];
        place(drag.g, drag.key);
      } else if (drag.kind === "line" || drag.kind === "road") {
        drag.g.setAttribute("transform", `translate(${dx} ${dy})`);
        ui.replaceChildren();
      } else {
        drag.pts[drag.i] = mmAt(e);
        ui.replaceChildren(svgEl("polyline", { points: through(drag.pts, drag.x0.smooth).map((q) => q.join(",")).join(" "), fill: "none", stroke: "#f60", "stroke-width": 0.3 / zoom, "pointer-events": "none" }),
          ...drag.pts.map((q) => svgEl("circle", { cx: q[0], cy: q[1], r: 1 / zoom, fill: "#f60", "pointer-events": "none" })));
      }
    });
    const drop = (e) => {
      const g = drag;
      drag = null;
      if (!g || g.kind === "pan" || g.kind === "pivot") return;
      if (g.kind === "turn") { hint.textContent = toolHints.rotate; if (wrap(g.deg)) applyTurn(wrap(g.deg)); return; }
      if (!g.moved) { if (g.added) overlay(); else { overlay(); fillPanel(); } return; }
      const dx = (e.clientX - g.x) / g.mm, dy = (e.clientY - g.y) / g.mm;
      if (g.kind === "road") { freeRoad(g.side, dx, dy); return; }
      if (g.kind === "label") {
        undo.push(g.before); redo.length = 0; bars();
        if (g.note != null) { // a note keeps its place in co-ordinates, not as a move
          const [x, y] = toPaper(notes[g.note].at);
          notes[g.note].at = toGround([x + dx, y + dy]);
          delete moves[g.key];
          show().catch(fail);
        }
      } else if (g.kind === "line") {
        undo.push(g.before); redo.length = 0; bars();
        const x = feats[g.i];
        x.side = null;
        x.points = g.pts.map(([a, b]) => toGround([a + dx, b + dy]));
        show().catch(fail);
      } else {
        mark();
        const x = freeFeature(sel.i);
        x.points = g.pts.map(toGround);
        show().catch(fail);
      }
    };
    ov.addEventListener("pointerup", drop);
    ov.addEventListener("pointercancel", drop);
    page.addEventListener("click", (e) => {
      if (!draw) return;
      const q = mmAt(e);
      if (draw.text) {
        stopDraw(); mark();
        notes.push({ text: "Text", at: toGround(q), color: null, size: null, angle: null });
        sel = { kind: "note", i: notes.length - 1 }; changed();
        panel.querySelector("input")?.select(); // ready to type over
        return;
      }
      // A double-click also delivers two clicks at the same spot; don't add zero-length segments.
      const last = draw.pts[draw.pts.length - 1];
      if (last && Math.hypot(q[0] - last[0], q[1] - last[1]) < 0.5) return;
      draw.pts.push(q);
      if (draw.point) { finishDraw(); return; }
      overlay();
    });
    page.addEventListener("dblclick", (e) => {
      if (draw) { if (!draw.text && draw.pts.length >= 2) finishDraw(); return; }
      const g = e.target.closest("[data-move]");
      if (!g) return;
      const key = g.getAttribute("data-move");
      if (key.startsWith("note:") || !(moves[key] || styles[key]?.angle)) return;
      mark(); delete moves[key];
      if (styles[key]) { styles[key].angle = null; tidyStyle(key); }
      place(g, key);
    });
  };
  const sheet = async () => { await save(); return api(`/api/v1/wf/jobs/${jobId}/titledeed.svg?upin=${encodeURIComponent(p.upin)}`); };
  const preview = async () => {
    try {
      const svg = await sheet();
      const w = window.open("", "_blank");
      if (!w) { uiAlert("Allow pop-ups for this site to preview the plan."); return; }
      w.document.write(`<!doctype html><html><head><title>Title Deed Plan ${p.upin}</title><style>@page{size:A4;margin:0}body{margin:0;background:#888}
        .page{width:210mm;height:297mm;margin:10px auto;background:#fff;box-shadow:0 0 6px #000}.bar{text-align:center;padding:8px;font:14px sans-serif}
        @media print{body{background:#fff}.page{margin:0;box-shadow:none}.bar{display:none}}</style></head><body>
        <div class="bar"><button onclick="window.print()">Print / Save as PDF</button></div><div class="page">${svg}</div></body></html>`);
      w.document.close();
    } catch (e) { msg.style.color = "#900"; msg.textContent = e.message; }
  };
  // The plan Send plan files with the Registry transaction: the sheet (drawn at A4, 200 dpi) as a
  // one-page A4 PDF, made on the server.
  const attach = async () => {
    try {
      msg.style.color = ""; msg.textContent = "Making the plan PDF…";
      const svg = await sheet();
      const url = URL.createObjectURL(new Blob([svg], { type: "image/svg+xml" }));
      const img = new Image();
      await new Promise((ok, bad) => { img.onload = ok; img.onerror = () => bad(new Error("the sheet could not be drawn")); img.src = url; });
      const c = document.createElement("canvas"); c.width = 1654; c.height = 2339;
      const g = c.getContext("2d"); g.fillStyle = "#fff"; g.fillRect(0, 0, c.width, c.height); g.drawImage(img, 0, 0, c.width, c.height);
      URL.revokeObjectURL(url);
      const blob = await new Promise((ok) => c.toBlob(ok, "image/jpeg", 0.92));
      const name = `Title Deed Plan ${(fields.title_no.value || p.upin).trim()}.pdf`;
      await authReady;
      const r = await fetch(`/api/v1/wf/jobs/${jobId}/titledeed.pdf?name=${encodeURIComponent(name)}`, { method: "POST", body: blob, headers: { "content-type": "image/jpeg", ...authHeaders() } });
      if (!r.ok) throw new Error((await r.json().catch(() => ({}))).message || r.statusText);
      msg.style.color = "#060"; msg.textContent = `${name} attached to the job. Send plan files it with the Registry transaction (or Resend corrected plan, if a plan was sent before).`;
      if (wf.detail?.job?.id === jobId) await wfSelect(jobId);
    } catch (e) { msg.style.color = "#900"; msg.textContent = "Plan PDF: " + e.message; }
  };
  const pick = d.upins.length > 1 ? row("Parcel:", h("select", { onchange: (e) => { w.remove(); titleDeedDialog(jobId, e.target.value); } },
    ...d.upins.map((u) => h("option", { value: u, selected: u === p.upin }, u)))) : null;
  const w = floatWindow(`Title Deed Plan — ${p.upin}`, h("div", {},
    h("div", { style: "margin-bottom:6px" }, reg),
    h("div", { class: "muted", style: "margin-bottom:6px" }, `Area ${(Math.floor(p.area_m2 * 1000 + 1e-7) / 1000).toFixed(3)} m² · ${n} beacons, clockwise from the westernmost`),
    pick,
    row("Title no.:", fields.title_no), row("Plan no.:", fields.plan_no), row("Tracing no.:", fields.tracing_no),
    row("Region:", fields.region), row("Location:", fields.location), row("Address:", fields.address), row("L.G. area:", lga),
    row("Holder:", fields.holder), row("Surveyed by:", fields.surveyed_by), row("Survey date:", fields.survey_date),
    row("Plotted by:", fields.plotted_by), row("Checked by:", fields.checked_by), row("Surveyor-General:", fields.surveyor_general),
    row("Prepared by:", fields.prepared_by), row("Scale:", scale), row("Access roads:", road), row("Road label:", roadLabel), row("Grid:", grid),
    h("div", { class: "cfHdr", style: "margin-top:6px" }, "Plan features"),
    row("Symbol:", symSel), row("Place:", placeSel), row("Label:", featLabel),
    h("div", { class: "frow", style: "justify-content:flex-end" }, h("button", { class: "btn", onclick: addFeat }, "Add feature")),
    featList,
    h("div", { class: "cfHdr", style: "margin-top:6px" }, "Beacon numbers" + (missing ? ` — ${missing} to type (not in the fabric)` : "")),
    h("div", { style: "max-height:180px;overflow:auto" }, ct),
    msg,
    h("div", { class: "frow", style: "flex-wrap:wrap;justify-content:flex-end" },
      h("button", { class: "btn", onclick: () => save().catch((e) => { msg.style.color = "#900"; msg.textContent = e.message; }) }, "Save"),
      h("button", { class: "btn", onclick: edit, title: "On the sheet: draw and reshape roads and features, move, recolour and retype labels, add text" }, "Edit plan"),
      h("button", { class: "btn", onclick: preview }, "Preview / Print"),
      h("button", { class: "btn", onclick: attach, title: "File the plan on the job as a PDF, for Send plan" }, "Attach as PDF"),
      h("button", { class: "btn", onclick: () => openRegistry({ job: jobId, view: "send" }) }, "Send plan…"))),
    { id: "titleDeedWin", width: 470 });
  for (const x of names) x.addEventListener("input", () => {
    roadLabels.forEach((l, i) => { l.textContent = sideText(i); });
    [...placeSel.options].slice(1).forEach((o, i) => { o.textContent = sideText(i); });
    renderFeats();
  });
  renderFeats();
  w.querySelector(".fx").addEventListener("click", () => tdShow([]));
}

// Which unit a workflow step belongs to: the title deed plan steps to Title Deed, the digitising
// steps (job area, construct, attributes, commit, mutations…) to Digitising; search/review to both.
function stepFeature(id) {
  if ([1003, 1016, 1018, 1017].includes(id)) return "titledeed";
  if ([1005, 1007, 1019, 3, 1008, 1010, 1002, 1012, 1020, 1021, 2, 1006].includes(id)) return "digitise";
  return null;
}
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
      ["Digitize Parcel (Sketch)", digitizeParcel], ["Orthophoto (actual view of the plot)", orthoOpen],
      ["Import Feature Class…", fciDialog], ["COGO Tools (survey entry)…", () => openTask("survey")], ["Computation Tools (35)…", openComputeTools]])), tip: "Construct the parcel: digitize, import a feature class, or COGO" };
    case 3: return { run: needJob(() => stepTools("Create Parcel", [
      ["Digitize Parcel (Sketch)", digitizeParcel],
      ["COGO Tools (survey entry)…", () => openTask("survey")], ["Create Connection Line…", connectionLineDialog], ["Create Multi Point Line…", multiPointLineDialog],
      ["Create Donut Parcel (sketch the hole, then click the parcel)", () => startPickDraft("donut")], ["Parcel Separation (sketch the line, then click the parcel)", () => startPickDraft("separate")],
      ["Move Point…", movePointDialog], ["Point Attributes…", pointAttributesDialog], ["List Control Points…", listControlPoints]])), tip: "Create the parcel with Aumentum's computation tools" };
    case 1008: return { run: () => validateFabric(), tip: "Validate the fabric (topology)" };
    case 1010: return { run: needJob(wfAddAttributes), label: "Open", tip: "Enter the draft parcel's attributes (parcel name, property number, …)" };
    case 1011: case 1015: return { run: () => { setTool("identify"); status("Click a parcel to review its attributes."); }, tip: "Review parcel attributes" };
    case 1002: case 1012: return { run: needJob(() => wfCommitDialog()), label: "Commit", tip: "Commit the job's parcels to the fabric" };
    case 1014: return { run: () => findDialog(), tip: "Search the cadastre for parcels" };
    case 1017: return { run: needJob(() => window.open(`/api/v1/jobs/${wf.editing.cadastre_job_id}/siteplan.pdf`, "_blank")), label: "Plan", tip: "Open the job's site plan (Title Deed Plan layout parity is Phase 6)" };
    // TDP, Generate / Save Title Deed Plan carry Aumentum's SendImageCommand: here the plan goes to the Registry as a PDF.
    case 1003: case 1016: case 1018: return { run: needJob(() => stepTools("Title Deed Plan", [
      ["Title Deed Plan… (form, print, PDF)", () => titleDeedDialog(jobId)],
      ["Open the job's site plan", () => window.open(`/api/v1/jobs/${wf.editing.cadastre_job_id}/siteplan.pdf`, "_blank")],
      ["Send the plan (PDF) to the Registry…", () => openRegistry({ job: jobId, view: "send" })]])), label: "Plan", tip: "Title deed plan, and sending the plan (PDF) to the job's Registry transaction" };
    case 1009: return { run: needJob(() => fciDialog()), label: "Import", tip: "Create parcels from a polygon feature class (shapefile)" };
    case 1013: return { run: async () => { const d = wf.detail && wf.detail.job.id === jobId ? wf.detail.job : null; await regFind(d && d.lrs_transaction_number); }, label: "Link", tip: "Link the parcel to the job's Registry transaction (Registry tab)" };
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
    } catch (e) { uiAlert("Attaching failed: " + e.message); }
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
const adm = { tab: "users", data: null, settings: [], sel: null, creds: [], reg: { loaded: false, users: [], groups: [], search: "", error: null } };
async function adminWindow() {
  try { [adm.data, adm.settings, adm.creds] = await Promise.all([api("/api/v1/admin/security"), api("/api/v1/admin/settings"), api("/api/v1/admin/credentials")]); if (!wf.defs) wf.defs = await api("/api/v1/wf/definitions"); }
  catch (e) { uiAlert(e.message); return; }
  floatWindow("Administration", h("div", { id: "admBody" }), { id: "admWin", width: 700 });
  renderAdmin();
}
// A temporary password: shown once here; its owner must change it at first sign-in.
async function admSetPassword(u) {
  const had = adm.creds.some((c) => c.sid === u.sid);
  if (!(await uiConfirm(`${had ? "Reset" : "Set"} the password of ${u.name || u.code} (${u.code})?\n\nA temporary password is made and shown once. ${u.name || u.code} must change it at first sign-in${had ? "; any open session ends" : ""}.`, { ok: had ? "Reset" : "Set password" }))) return;
  try {
    const r = await api(`/api/v1/admin/users/${u.sid}/password`, { method: "POST" });
    adm.creds = await api("/api/v1/admin/credentials");
    renderAdmin();
    await uiAlert(`Username: ${r.username}\nTemporary password: ${r.temporary_password}\n\nGive this to ${u.name || u.code}. It will not be shown again.`, { title: "Temporary password" });
  } catch (e) { uiAlert(e.message); }
}
function admPasswordCell(u) {
  const c = adm.creds.find((x) => x.sid === u.sid);
  const state = !c ? "None" : c.must_change ? "Temporary" : "Set";
  return h("td", { style: "white-space:nowrap" }, h("span", { class: "muted", title: c && c.last_login ? `Last sign-in ${fmtDT(c.last_login)}` : "Never signed in" }, state + " "),
    h("button", { class: "btn", onclick: () => admSetPassword(u) }, c ? "Reset" : "Set"));
}
async function admChange(body) {
  try { adm.data = await api("/api/v1/admin/change", { method: "POST", body }); renderAdmin(); status("Saved."); } catch (e) { uiAlert(e.message); }
}
function renderAdmin() {
  const root = $("admBody"); if (!root) return; root.innerHTML = "";
  const d = adm.data, P = d.principals, name = (sid) => { const p = P.find((x) => x.sid === sid); return p ? p.name || p.code : "sid " + sid; };
  const tabs = [["users", "Users & Groups"], ["registry", "Registry Users"], ["roles", "Roles"], ["tasks", "Task Permissions"], ["jobtypes", "Job Types"], ["settings", "Settings"]];
  root.append(h("div", { class: "frow", style: "flex-wrap:wrap" }, ...tabs.map(([k, l]) => h("button", { class: "btn" + (adm.tab === k ? " active" : ""), style: adm.tab === k ? "font-weight:bold" : "", onclick: () => { adm.tab = k; renderAdmin(); } }, l))));
  const body = h("div", { style: "max-height:460px;overflow:auto;margin-top:6px" }); root.append(body);
  const groups = P.filter((p) => !p.is_user);
  if (adm.tab === "users") {
    const tb = h("table", { class: "wfHist" }, h("tr", {}, h("th", {}, "User"), h("th", {}, "Windows account"), h("th", {}, "E-mail"), ...groups.map((g) => h("th", {}, g.name || g.code)), h("th", {}, "Locked"), h("th", {}, "Password")));
    for (const u of P.filter((p) => p.is_user)) {
      const email = h("input", { value: u.email || "", style: "width:150px", onchange: (e) => admChange({ kind: "principal", sid: u.sid, email: e.target.value }) });
      tb.append(h("tr", {}, h("td", {}, u.name || u.code), h("td", {}, u.os_account || ""), h("td", {}, email),
        ...groups.map((g) => { const on = d.memberships.some(([gs, ms]) => gs === g.sid && ms === u.sid);
          return h("td", {}, h("input", { type: "checkbox", checked: on, onchange: (e) => admChange({ kind: "member", group_sid: g.sid, member_sid: u.sid, add: e.target.checked }) })); }),
        h("td", {}, h("input", { type: "checkbox", checked: u.is_locked, onchange: (e) => admChange({ kind: "principal", sid: u.sid, is_locked: e.target.checked }) })),
        admPasswordCell(u)));
    }
    body.append(tb);
  } else if (adm.tab === "registry") {
    renderRegistryUsers(body);
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
        onclick: async () => (await uiConfirm(`Remove ${name(sid)} from "${t.name}"?`, { ok: "Remove", kind: "warn" })) && admChange({ kind: "task_permission", task_id: t.id, sid, add: false }),
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
      inp.onchange = async () => { try { adm.settings = await api("/api/v1/admin/settings", { method: "PUT", body: { key: st.key, value: inp.value } }); status(`${st.key} saved.`); renderAdmin(); } catch (e) { uiAlert(e.message); } };
      tb.append(h("tr", {}, h("td", {}, st.key), h("td", {}, inp), h("td", {}, st.source || "")));
    }
    const k = h("input", { placeholder: "e.g. mail.smtp_password", style: "width:220px" }), v = h("input", { style: "width:220px" });
    body.append(tb, h("div", { class: "frow" }, h("label", {}, "Add setting:"), k, v, h("button", { class: "btn", onclick: async () => { if (!k.value) return; adm.settings = await api("/api/v1/admin/settings", { method: "PUT", body: { key: k.value, value: v.value } }); renderAdmin(); } }, "Save")));
  }
}

/* =====================================================================================
 * Registry (LRS43 / Alfresco) users & units. The Sysadmin manages the Registry's own accounts and
 * the groups ("units", e.g. Title Deed Plan) through the plagis_admin stored procedures. Reads show
 * the live database; a change takes effect for sign-in only after the Registry (JBoss) restarts.
 * Needs the server built with --features registry-admin and CADASTRE_REGISTRY_SQL_* set.
 * ===================================================================================== */
async function regLoad() {
  const r = adm.reg; r.error = null;
  try {
    const q = r.search ? "?search=" + encodeURIComponent(r.search) : "";
    [r.users, r.groups] = await Promise.all([api("/api/v1/admin/registry/users" + q), api("/api/v1/admin/registry/groups")]);
  } catch (e) { r.error = e.message; r.users = []; r.groups = []; }
  r.loaded = true; renderAdmin();
}
function regLabel(g) { return String(g || "").replace(/^GROUP_/, ""); }
// A small modal form (floatWindow) resolving to the field values, or null on cancel. Field types:
// text/password (default), select (options [{value,label}]), checklist (multi, resolves to an array).
function regForm(title, fields, { ok = "Save", width = 360 } = {}) {
  return new Promise((resolve) => {
    const readers = {};
    const field = (f, inp) => h("div", { style: "margin:5px 0" }, h("label", { style: "display:block;font-size:12px;color:#555" }, f.label), inp);
    const rows = fields.map((f) => {
      if (f.type === "select") {
        const inp = h("select", { style: "width:100%" }, ...f.options.map((o) => h("option", { value: o.value ?? o, selected: (o.value ?? o) === f.value }, o.label ?? o)));
        readers[f.key] = () => inp.value; return field(f, inp);
      }
      if (f.type === "checklist") {
        const boxes = f.options.map((o) => ({ value: o.value ?? o, label: o.label ?? o, el: h("input", { type: "checkbox", checked: (f.value || []).includes(o.value ?? o) }) }));
        readers[f.key] = () => boxes.filter((b) => b.el.checked).map((b) => b.value);
        return field(f, h("div", { style: "max-height:150px;overflow:auto;border:1px solid #ccc;padding:4px" }, ...(boxes.length ? boxes.map((b) => h("label", { style: "display:block;font-size:12px" }, b.el, " " + b.label)) : [h("span", { class: "muted" }, "no units yet")])));
      }
      const inp = h("input", { type: f.type || "text", value: f.value ?? "", placeholder: f.placeholder || "", style: "width:100%" });
      readers[f.key] = () => inp.value; return field(f, inp);
    });
    let win;
    const submit = () => { const out = {}; for (const f of fields) out[f.key] = readers[f.key](); win.remove(); resolve(out); };
    win = floatWindow(title, h("div", {}, ...rows,
      h("div", { class: "frow", style: "margin-top:8px;justify-content:flex-end;gap:6px" },
        h("button", { class: "btn", onclick: () => { win.remove(); resolve(null); } }, "Cancel"),
        h("button", { class: "btn active", style: "font-weight:bold", onclick: submit }, ok))), { width });
  });
}
function renderRegistryUsers(body) {
  const r = adm.reg;
  if (!r.loaded) { body.append(h("div", { class: "muted" }, "Loading Registry users…")); regLoad(); return; }
  const search = h("input", { value: r.search, placeholder: "search name…", style: "width:150px" });
  search.onkeydown = (e) => { if (e.key === "Enter") { r.search = search.value.trim(); regLoad(); } };
  body.append(h("div", { class: "frow", style: "flex-wrap:wrap;gap:6px;margin-bottom:6px" },
    search,
    h("button", { class: "btn", onclick: () => { r.search = search.value.trim(); regLoad(); } }, "Search"),
    h("button", { class: "btn", onclick: () => { r.search = ""; regLoad(); } }, "All"),
    h("span", { style: "flex:1" }),
    h("button", { class: "btn", onclick: regNewUser }, "+ New user"),
    h("button", { class: "btn", onclick: regNewUnit }, "+ New unit")));
  if (r.error) {
    body.append(h("div", {}, h("p", { style: "color:#b00" }, "Registry user administration is not available on this server."),
      h("p", { class: "muted" }, r.error), h("p", { class: "muted" }, "It needs the server built with --features registry-admin and CADASTRE_REGISTRY_SQL_* set.")));
    return;
  }
  body.append(h("div", { class: "muted", style: "margin-bottom:4px" }, `${r.users.length} user(s), ${r.groups.length} unit(s). A change takes effect for sign-in after the Registry (JBoss) service restarts.`));
  const tb = h("table", { class: "wfHist" }, h("tr", {}, h("th", {}, "User"), h("th", {}, "Name"), h("th", {}, "Office"), h("th", {}, "Units"), h("th", {}, "Status"), h("th", {}, "Actions")));
  for (const u of r.users) {
    const enabled = u.enabled !== false;
    const units = h("td", {}, ...(u.groups.length ? u.groups.map((g) => h("span", { class: "wfPill", style: "display:inline-block;margin:1px;cursor:pointer", title: "Remove from " + regLabel(g), onclick: () => regRemoveUnit(u, g) }, regLabel(g), " ×")) : [h("span", { class: "muted" }, "—")]),
      h("button", { class: "btn", style: "margin-left:4px", title: "Add to a unit", onclick: () => regAddUnit(u) }, "+"));
    const stat = u.has_login ? h("span", { class: "wfPill " + (enabled ? "done" : "ended") }, enabled ? "enabled" : "disabled") : h("span", { class: "muted" }, "no login");
    const actions = h("td", { style: "white-space:nowrap" },
      h("button", { class: "btn", onclick: () => regEdit(u) }, "Edit"),
      h("button", { class: "btn", onclick: () => regResetPassword(u) }, "Password"),
      u.has_login ? h("button", { class: "btn", onclick: () => regToggleEnabled(u, !enabled) }, enabled ? "Disable" : "Enable") : null);
    tb.append(h("tr", {}, h("td", {}, u.user_name), h("td", {}, [u.first_name, u.last_name].filter(Boolean).join(" ")), h("td", {}, u.office_id || ""), units, stat, actions));
  }
  body.append(tb);
}
// Sysadmin's Settings button: the add-user form as a modal, the page dimmed behind it.
async function settingsAddUser() {
  await regLoad();
  if (adm.reg.error) { uiAlert(adm.reg.error); return; }
  const open = regNewUser();
  const win = [...document.querySelectorAll(".float")].pop();
  const back = h("div", { style: `position:fixed;inset:0;background:rgba(20,28,36,.45);z-index:${(parseInt(getComputedStyle(win).zIndex, 10) || 100) - 1}` });
  document.body.append(back);
  try { await open; } finally { back.remove(); }
}
async function regNewUser() {
  const r = adm.reg;
  const refs = r.users.map((u) => u.user_name).filter((n) => !["admin", "aumentumws", "mcconfig"].includes(String(n).toLowerCase()));
  const vals = await regForm("New Registry user", [
    { key: "user_name", label: "Username (a–z, 0–9, . _ -)" },
    { key: "first_name", label: "First name" },
    { key: "last_name", label: "Last name" },
    { key: "password", label: "Temporary password (at least 6 characters; changed at first sign-in)" },
    { key: "ref_user", label: "Copy groups & office from", type: refs.length ? "select" : "text", value: refs.includes("cgugin") ? "cgugin" : refs[0], options: refs.map((x) => ({ value: x, label: x })) },
    { key: "extra_groups", label: "Units", type: "checklist", value: [], options: r.groups.map((g) => ({ value: g.group_name, label: regLabel(g.group_name) })) },
  ], { ok: "Create user", width: 380 });
  if (!vals) return;
  const b = { user_name: (vals.user_name || "").trim(), first_name: (vals.first_name || "").trim(), last_name: (vals.last_name || "").trim(), password: vals.password || "", ref_user: (vals.ref_user || "").trim(), extra_groups: (vals.extra_groups || []).join(",") };
  if (!b.user_name || !b.first_name || !b.last_name || !b.password || !b.ref_user) { uiAlert("Username, name, password and a reference user are all required."); return; }
  try { const res = await api("/api/v1/admin/registry/users", { method: "POST", body: b }); status("User created."); await regLoad(); await uiAlert(res.note || "Created.", { title: "User created" }); }
  catch (e) { uiAlert(e.message); }
}
async function regNewUnit() {
  const r = adm.reg;
  const vals = await regForm("New unit (group)", [
    { key: "display_name", label: "Unit name (without GROUP_)", placeholder: "e.g. Title Deed Plan" },
    { key: "ref_group", label: "Copy structure from", type: "select", value: "", options: [{ value: "", label: "— default (Map Compilation Officer) —" }, ...r.groups.map((g) => ({ value: g.group_name, label: regLabel(g.group_name) }))] },
  ], { ok: "Create unit" });
  if (!vals) return;
  const name = (vals.display_name || "").trim();
  if (!name) { uiAlert("A unit name is required."); return; }
  try { const res = await api("/api/v1/admin/registry/groups", { method: "POST", body: { display_name: name, ref_group: vals.ref_group || null } }); status("Unit created."); await regLoad(); await uiAlert(res.note || "Created.", { title: "Unit created" }); }
  catch (e) { uiAlert(e.message); }
}
async function regEdit(u) {
  const vals = await regForm("Edit " + u.user_name, [
    { key: "first_name", label: "First name", value: u.first_name || "" },
    { key: "last_name", label: "Last name", value: u.last_name || "" },
    { key: "office_id", label: "Office id", value: u.office_id || "" },
  ], { ok: "Save" });
  if (!vals) return;
  try { await api(`/api/v1/admin/registry/users/${encodeURIComponent(u.user_name)}`, { method: "PUT", body: { first_name: vals.first_name, last_name: vals.last_name, office_id: vals.office_id } }); status("Saved."); regLoad(); }
  catch (e) { uiAlert(e.message); }
}
async function regResetPassword(u) {
  const p = await uiPrompt(`Temporary password for ${u.user_name} (at least 6 characters). They must change it at their next sign-in. The account is also unlocked.`, "", { ok: "Set password", title: "Reset password" });
  if (p == null) return;
  if (String(p).length < 6) { uiAlert("Password must be at least 6 characters."); return; }
  try { const res = await api(`/api/v1/admin/registry/users/${encodeURIComponent(u.user_name)}/password`, { method: "POST", body: { password: p } }); status("Password set."); await uiAlert(res.note || "Password set.", { title: "Password" }); }
  catch (e) { uiAlert(e.message); }
}
async function regToggleEnabled(u, enable) {
  if (!(await uiConfirm(`${enable ? "Enable" : "Disable"} ${u.user_name}?`, { ok: enable ? "Enable" : "Disable", kind: enable ? "question" : "warn" }))) return;
  try { await api(`/api/v1/admin/registry/users/${encodeURIComponent(u.user_name)}/enabled`, { method: "POST", body: { enabled: enable } }); status(enable ? "Enabled." : "Disabled."); regLoad(); }
  catch (e) { uiAlert(e.message); }
}
async function regAddUnit(u) {
  const choices = adm.reg.groups.map((x) => x.group_name).filter((g) => !u.groups.includes(g));
  if (!choices.length) { uiAlert(u.user_name + " is already in every unit."); return; }
  const vals = await regForm("Add " + u.user_name + " to a unit", [
    { key: "group", label: "Unit", type: "select", value: choices[0], options: choices.map((g) => ({ value: g, label: regLabel(g) })) },
  ], { ok: "Add" });
  if (!vals || !vals.group) return;
  try { await api(`/api/v1/admin/registry/users/${encodeURIComponent(u.user_name)}/groups`, { method: "POST", body: { group: vals.group } }); status("Added to unit."); regLoad(); }
  catch (e) { uiAlert(e.message); }
}
async function regRemoveUnit(u, g) {
  if (!(await uiConfirm(`Remove ${u.user_name} from ${regLabel(g)}?`, { ok: "Remove", kind: "warn" }))) return;
  try { await api(`/api/v1/admin/registry/users/${encodeURIComponent(u.user_name)}/groups/${encodeURIComponent(g)}`, { method: "DELETE" }); status("Removed from unit."); regLoad(); }
  catch (e) { uiAlert(e.message); }
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

/* =====================================================================================
 * Page refresh keeps the work: the map view, the open panel, the workflow job being worked on, the
 * selected job and filters, the Registry lookup, the tool and an unfinished sketch are kept for this
 * browser tab (sessionStorage, so two tabs on different jobs do not overwrite each other) and put
 * back after sign-in and the first data load. Signing out forgets them. Nothing is saved until the
 * restore has finished, so a refresh during start-up cannot overwrite the saved work with a blank page.
 * ===================================================================================== */
const SESSION_KEY = "cad.session";
const SESSION_TOOLS = ["pan", "zoomIn", "zoomOut", "select", "selectEl", "identify", "measure"];
const savedSession = (() => { try { const s = JSON.parse(sessionStorage.getItem(SESSION_KEY) || "null"); return s && s.v === 1 ? s : null; } catch { return null; } })();
let sessionRestored = false;
function snapshotSession() {
  const c = map.getCenter();
  return {
    v: 1, at: Date.now(),
    view: { center: [c.lng, c.lat], zoom: map.getZoom(), bearing: map.getBearing(), pitch: map.getPitch() },
    panel: regOpen() ? "registry" : wfOpen() ? "workflow" : doneOpen() ? "completed" : tasksOpen() ? "tasks" : "toc",
    done: { q: done.q, source: done.source },
    tocVisible: !$("tocPane").classList.contains("hidden"),
    tocView: state.tocView,
    wf: { selected: wf.selected, filter: wf.filter, jobType: wf.jobType, q: wf.q, working: wf.editing?.job_instance_id || null },
    reg: { view: reg.view, txnNo: reg.txn ? reg.txnNo : "" },
    job: state.job?.id || null,
    tool: state.tool,
    edit: edit.active && edit.verts.length ? { template: edit.template, construction: edit.construction, verts: edit.verts, editingId: edit.editingId } : null,
  };
}
function saveSession() {
  if (!sessionRestored) return;
  try { sessionStorage.setItem(SESSION_KEY, JSON.stringify(snapshotSession())); } catch { /* storage unavailable */ }
}
window.addEventListener("pagehide", saveSession);
window.addEventListener("beforeunload", saveSession);
document.addEventListener("visibilitychange", () => { if (document.visibilityState === "hidden") saveSession(); });
setInterval(saveSession, 5000); // also survives a crash or a closed laptop lid
function jumpToSavedView(v) {
  if (!v || !Array.isArray(v.center) || !v.center.every(Number.isFinite) || !Number.isFinite(v.zoom)) return false;
  map.jumpTo({ center: v.center, zoom: v.zoom, bearing: v.bearing || 0, pitch: v.pitch || 0 });
  return true;
}
// Pick the worked-on job back up without creating anything (GET, unlike Work On's POST) and
// without moving the map (the saved view is put back afterwards).
async function wfResume(jobId) {
  const ed = await api(`/api/v1/wf/jobs/${jobId}/editing`);
  if (!ed?.cadastre_job_id) return false;
  try { const d = await api("/api/v1/wf/jobs/" + jobId); if (d.job && !d.job.is_open) return false; } catch { /* resume anyway */ }
  wf.editing = ed; wf.selEditing = ed;
  const job = await api(`/api/v1/jobs/${ed.cadastre_job_id}`);
  if (!state.jobs.some((x) => x.id === job.id)) state.jobs.unshift(job);
  renderJobBox(); $("jobBox").value = job.id; await selectJob(job.id);
  showJobArea();
  return true;
}
async function restoreSession() {
  const s = savedSession;
  const step = async (what, fn) => { try { await fn(); } catch (e) { console.warn(`Restore ${what}:`, e); } };
  try {
    if (!s) return;
    const notes = [];
    if (s.tocVisible === false && !$("tocPane").classList.contains("hidden")) togglePane("tocPane");
    if (s.tocView && s.tocView !== state.tocView) { state.tocView = s.tocView; buildTocTools(); buildTOC(); }
    if (s.wf) { wf.filter = s.wf.filter ?? wf.filter; wf.jobType = s.wf.jobType ?? wf.jobType; wf.q = s.wf.q ?? wf.q; }
    if (s.wf?.working) await step("job", async () => { if (await wfResume(s.wf.working)) notes.push(`working on job ${s.wf.working}`); });
    else if (s.job && state.jobs.some((j) => j.id === s.job)) await step("job", async () => { renderJobBox(); $("jobBox").value = s.job; await selectJob(s.job); });
    if (s.panel === "workflow") await step("workflow", async () => { await openWorkflow(); if (s.wf?.selected) await wfSelect(s.wf.selected); });
    else if (s.panel === "registry") await step("registry", () => openRegistry({ view: s.reg?.view, txn: s.reg?.txnNo || undefined }));
    else if (s.panel === "tasks") { showTasks(true); buildTocTools(); }
    else if (s.panel === "completed") await step("completed", () => { if (s.done) { done.q = s.done.q || ""; done.source = s.done.source || ""; } return openCompleted(); });
    if (s.edit && (s.edit.verts || []).length) await step("sketch", () => {
      selectTemplate(s.edit.template || edit.template);
      startEditing("sketch");
      if (s.edit.construction) edit.construction = s.edit.construction;
      const verts = (s.edit.verts || []).filter((v) => Array.isArray(v) && v.length >= 2 && v.every(Number.isFinite));
      if (verts.length) {
        edit.verts = verts.map((v) => [v[0], v[1]]); edit.sel = edit.verts.length - 1; edit.editingId = s.edit.editingId || null;
        afterVertexChange();
        notes.push(`unfinished sketch (${verts.length} vertices)`);
      }
    });
    else if (SESSION_TOOLS.includes(s.tool)) setTool(s.tool);
    jumpToSavedView(s.view); // last: the steps above may have zoomed elsewhere
    if (notes.length) notify("Restored after refresh: " + notes.join(", ") + ".");
  } finally {
    sessionRestored = true;
  }
}

initialDeepLink = initialViewFromURL(); // apply /editor?center=E,N&scale=N before any data arrives
if (!initialDeepLink && savedSession && jumpToSavedView(savedSession.view)) initialDeepLink = true; // refresh: no fit to all parcels
buildMenus(); buildToolbars(); buildTocTools(); buildTOC(); renderTask(); buildCatalog(); updateNavButtons(); updateScaleBox(); updateEditorButtons();
loadCatalogRoots();
loadCatalogDatabases();
