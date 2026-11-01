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
async function api(path, opts = {}) {
  const init = { ...opts, headers: { ...(opts.body ? { "content-type": "application/json" } : {}), ...(opts.headers || {}) } };
  if (opts.body && typeof opts.body !== "string") init.body = JSON.stringify(opts.body);
  const r = await fetch(path, init);
  const text = await r.text();
  let data = null; try { data = text ? JSON.parse(text) : null; } catch { data = text; }
  if (!r.ok) throw new Error((data && data.message) || `${r.status} ${r.statusText}`);
  return data;
}
const fmt3 = (v) => Number(v).toFixed(3);
const status = (msg) => { $("statusMsg").textContent = msg || ""; };

const REGION = "Plateau_State";
// Plateau State extent (WGS84). "Full extent" is the region's data, ignoring anything outside it.
const REGION_BBOX = [8.2, 8.3, 10.7, 10.5];
const inRegion = ([x, y]) => x >= REGION_BBOX[0] && x <= REGION_BBOX[2] && y >= REGION_BBOX[1] && y <= REGION_BBOX[3];
const APP = "PLAGIS CADASTRE Editor";
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
const edit = { active: false, template: "parcel", geomType: "polygon", construction: "polygon", verts: [], sel: -1, snapping: true };
// Feature templates in the Create Features window; each maps to a layer and a geometry type.
const FEATURE_TEMPLATES = [
  { id: "parcel", name: `${REGION}_Parcels`, geom: "polygon", icon: "polygonFc", tools: ["polygon", "autocomplete", "rightangle"] },
  { id: "line", name: `${REGION}_Lines`, geom: "line", icon: "lineFc", tools: ["line"] },
  { id: "point", name: `${REGION}_CtrlPnts`, geom: "point", icon: "pointFc", tools: ["point"] },
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
  map.addLayer({ id: "parcels-line", type: "line", source: "parcels", paint: { "line-color": "#f4f4f4", "line-width": 1 } });
  map.addLayer({ id: "parcels-focus", type: "line", source: "parcels", filter: ["==", ["get", "id"], ""],
    paint: { "line-color": "#ff0000", "line-width": 2 } });
  map.addLayer({ id: "parcels-sel", type: "line", source: "parcels", filter: ["in", ["get", "id"], ["literal", []]],
    paint: { "line-color": "#00ffff", "line-width": 2.5 } });

  map.addSource("drafts", { type: "geojson", data: empty(), promoteId: "id" });
  map.addLayer({ id: "drafts-fill", type: "fill", source: "drafts", paint: { "fill-color": "#fff", "fill-opacity": 0.08 } });
  map.addLayer({ id: "drafts-line", type: "line", source: "drafts", paint: { "line-color": "#111", "line-width": 1.6 } });

  map.addSource("survey", { type: "geojson", data: empty() });
  map.addLayer({ id: "survey-line", type: "line", source: "survey", filter: ["==", "$type", "LineString"],
    paint: { "line-color": "#e60000", "line-width": 2 } });
  map.addLayer({ id: "survey-pt", type: "symbol", source: "survey", filter: ["==", "$type", "Point"],
    layout: { "icon-image": "cross", "icon-allow-overlap": true } });

  map.addSource("beacons", { type: "geojson", data: empty(), promoteId: "id" });
  map.addLayer({ id: "beacons-pt", type: "circle", source: "beacons",
    paint: { "circle-radius": 4.5, "circle-color": "#55ff00", "circle-stroke-color": "#1a3d00", "circle-stroke-width": 1 } });

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
async function loadBeacons() {
  try { state.beacons = asFC(await api("/api/v1/beacons.geojson")); } catch { state.beacons = empty(); }
  map.getSource("beacons").setData(state.beacons);
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
    { label: "Print…", kbd: "Ctrl+P", action: () => window.print() },
    "-",
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
    { kind: "tool", tool: "sketch", icon: "sketchTool", tip: "Sketch tool: click to add vertices, double-click to finish" },
    { kind: "tool", tool: "editVertex", icon: "editTool", tip: "Edit tool: drag sketch vertices to move them" },
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
    { kind: "btn", icon: "find", tip: "Find parcel in region", action: () => findDialog() },
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
  { id: "cadastral", name: "PLAGIS CADASTRE - Cadastral", group: true, visible: true, open: true, children: [
    { id: "parcels", name: `${REGION}_Parcels`, kind: "polygon", visible: true, open: true,
      map: ["parcels-fill", "parcels-line", "parcels-focus", "parcels-sel"], source: "cadastre.parcel",
      legend: () => [{ sym: "fill", fill: "#fff", stroke: "#555", label: "<all other values>" },
                     ...(state.focusParcel ? [{ sym: "fill", fill: "#fff", stroke: "#ff0000", label: state.focusParcel }] : [])] },
    { id: "jobsArea", name: `${REGION}_JobsArea`, kind: "polygon", visible: true, open: true,
      map: ["drafts-fill", "drafts-line"], source: "cadastre.parcel (job drafts)",
      legend: () => [{ sym: "fill", fill: "#fff", stroke: "#111", label: state.job ? state.job.reference : "" }] },
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
    const b = h("span", { class: `tb${state.tocView === v && !tasksOpen() ? " active" : ""}`, title: tip, html: icon(ic) });
    b.onclick = () => { state.tocView = v; showTasks(false); buildTocTools(); buildTOC(); };
    t.append(b);
  }
  t.append(h("span", { class: "tsep" }));
  const tasksBtn = h("span", { class: `tb${tasksOpen() ? " active" : ""}`, title: "Tasks", html: icon("tasks") });
  tasksBtn.onclick = () => { showTasks(true); buildTocTools(); };
  t.append(tasksBtn);
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
  }
});
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
  buildTOC();
}

// identify
function identifyAt(e) {
  const feats = map.queryRenderedFeatures([[e.point.x - 3, e.point.y - 3], [e.point.x + 3, e.point.y + 3]],
    { layers: ["beacons-pt", "drafts-fill", "parcels-fill"].filter((l) => map.getLayer(l)) });
  const [E, N] = toUTM(e.lngLat.lng, e.lngLat.lat);
  const names = { "beacons-pt": `${REGION}_CtrlPnts`, "drafts-fill": `${REGION}_JobsArea`, "parcels-fill": `${REGION}_Parcels` };
  const body = h("div", {}, h("div", { class: "frow" }, h("label", {}, "Location:"), h("span", {}, `${fmt3(E)}  ${fmt3(N)} Meters`)));
  if (!feats.length) body.append(h("p", { class: "muted" }, "No identified features."));
  for (const f of feats) {
    const p = f.properties;
    const rows = Object.entries(p).filter(([k]) => k !== "id").map(([k, v]) => h("tr", {}, h("th", {}, k), h("td", {}, v == null || v === "null" ? "<Null>" : String(v))));
    body.append(h("h4", { style: "margin:8px 0 3px" }, names[f.layer.id]), h("table", {}, ...rows));
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
function findDialog() {
  const input = h("input", { style: "flex:1", placeholder: "UPIN or plot number, e.g. 5621", value: "" });
  const results = h("div", { style: "margin-top:6px" });
  const run = async () => {
    const q = input.value.trim(); if (!q) return;
    results.textContent = "Searching…";
    try {
      const list = await api("/api/v1/parcels/search?number=" + encodeURIComponent(q));
      results.innerHTML = "";
      if (!list.length) {
        results.append(h("div", {}, `No parcel matches "${q}".`), h("div", { class: "muted" }, "Try the plot number alone (e.g. 5621) or part of the UPIN."));
        return;
      }
      const t = h("table", {}, h("tr", {}, h("th", {}, "Parcel"), h("th", {}, "Status"), h("th", {}, "Area (m²)")));
      for (const p of list.slice(0, 200)) {
        const area = p.computed_area ?? p.stated_area;
        const tr = h("tr", { style: "cursor:pointer", title: "Zoom to parcel" }, h("td", {}, p.parcel_number || "(unposted)"), h("td", {}, p.status), h("td", {}, area != null ? Number(area).toFixed(1) : "—"));
        tr.onclick = () => goToResult(p);
        t.append(tr);
      }
      results.append(h("div", { class: "muted" }, `${list.length} match(es)${list.length > 200 ? " (first 200 shown)" : ""} — click a row to zoom`), t);
      if (list.length === 1) goToResult(list[0]); // a single hit: go straight there, as Find does
      else selectFeatures(list.map((p) => p.id));
    } catch (e) { results.textContent = "Search failed: " + e.message; }
  };
  input.addEventListener("keydown", (e) => e.key === "Enter" && run());
  floatWindow("Find", h("div", {}, h("div", { class: "frow" }, h("label", {}, "Find:"), input, h("button", { class: "btn", onclick: run }, "Find")),
    h("div", { class: "frow" }, h("label", {}, "In layer:"), h("span", {}, `${REGION}_Parcels (parcel_number)`)), results), { id: "findWin", width: 380 });
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
  const beacon = h("select", { style: "flex:1;min-width:0", onchange: (ev) => { const f = state.beacons.features.find((x) => String(x.properties.id) === ev.target.value); if (f) { task.start = [Number(f.properties.easting), Number(f.properties.northing)]; drawTraverse(); renderTask(); } } },
    h("option", { value: "" }, "— from control point —"), ...state.beacons.features.slice(0, 2000).map((f) => h("option", { value: f.properties.id }, f.properties.number)));
  page.append(subHead("Start point"), h("p", { class: "muted" }, "Click the map (crosshair), pick a control point, or type UTM 32N coordinates."),
    h("div", { class: "frow" }, h("label", {}, "Easting:"), E), h("div", { class: "frow" }, h("label", {}, "Northing:"), N),
    h("div", { class: "frow" }, h("label", {}, "Beacon:"), beacon));

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
    status(`Saved draft parcel to ${state.job.reference} — ready for the next. Post the job (once approved) to allocate a UPIN.`);
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
    { label: "Edit Sketch Properties…", icon: "sketchProps", action: openSketchProps, disabled: () => !edit.verts.length },
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
function clearSketch() { edit.verts = []; edit.sel = -1; map.getSource("sketch")?.setData(empty()); }
function canFinishSketch() { const n = edit.verts.length; return edit.geomType === "polygon" ? n >= 3 : edit.geomType === "line" ? n >= 2 : n >= 1; }
async function finishSketch() {
  if (!canFinishSketch()) { status(`Add ${edit.geomType === "polygon" ? "at least 3" : edit.geomType === "line" ? "at least 2" : "a"} vertex(es) before finishing.`); return; }
  if (edit.geomType === "polygon") await saveSketchParcel();
  else if (edit.geomType === "point") await saveSketchPoint();
  else saveSketchLine();
}
async function saveSketchParcel() {
  if (!state.job) { alert("Choose a Job Area first — a digitized parcel is saved as a draft under a job."); return; }
  const utm = edit.verts.slice();
  try {
    await api(`/api/v1/jobs/${state.job.id}/parcels`, { method: "POST", body: { geometry: { type: "Polygon", coordinates: [[...utm, utm[0]]] }, stated_area: null } });
    status(`Digitized parcel saved as a draft to ${state.job.reference}. Post the job (once approved) to allocate a UPIN.`);
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
  set("edFinish", canFinishSketch()); set("edDelete", has); set("edProps", has || edit.active);
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
    const shown = FEATURE_TEMPLATES.filter((t) => !q || t.name.toLowerCase().includes(q));
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
  if (!edit.verts.length) tbl.append(h("tr", {}, h("td", { colspan: 4, class: "muted" }, "No sketch. Pick the Sketch tool and click the map.")));
  const body = h("div", {},
    h("div", { class: "muted" }, `${edit.geomType} sketch — ${edit.verts.length} vertex(es), WGS 84 / UTM 32N metres`),
    tbl,
    h("div", { class: "taskNav" },
      h("button", { class: "btn", disabled: !canFinishSketch(), onclick: finishSketch }, "Finish Sketch"),
      h("button", { class: "btn", disabled: !edit.verts.length, onclick: deleteSketch }, "Delete Sketch")));
  floatWindow("Edit Sketch Properties", body, { id: "sketchPropsWin", width: 320 });
}
function refreshSketchProps() { if ($("sketchPropsWin")) openSketchProps(); }

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
