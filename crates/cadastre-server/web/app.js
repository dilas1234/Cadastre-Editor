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
};

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
  if (ev.key === "Escape") { closeMenus(); cancelInteraction(); }
  if (ev.key === "F5") { ev.preventDefault(); refreshAll(); }
  if (ev.key === "Delete" && !/INPUT|SELECT|TEXTAREA/.test(document.activeElement.tagName)) deleteSelectedGraphics();
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
  { id: "tb-cadastre", name: "Cadastre", row: 2, indent: true, items: [
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
  if (state.tocView === "drawing") LAYERS.forEach((l) => ul.append(layerNode(l)));
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
  const fc = { parcels: state.parcels, jobsArea: state.drafts, ctrl: state.beacons, cadastral: state.parcels,
    lines: state.surveyFC, points: state.surveyFC }[l.id];
  if (!fc || !zoomToFC(fc)) status(`${l.name}: nothing to zoom to`);
}
function layerContextMenu(l, x, y) {
  contextMenu([
    { label: "Zoom To Layer", icon: "fullExtent", action: () => zoomToLayer(l), disabled: l.kind === "raster" || l.kind === "graphics" },
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
};
function setTool(tool) {
  state.tool = tool;
  for (const [t, btns] of toolButtons) btns.forEach((b) => b.classList.toggle("active", t === tool));
  const c = map.getContainer(); c.className = c.className.replace(/\btool-\S+/g, "").trim(); c.classList.add("tool-" + tool);
  const cv = map.getCanvas(); if (cv) cv.style.cursor = tool in TOOL_CURSOR ? TOOL_CURSOR[tool] : "crosshair";
  // Pan is the only tool that drags the map; the others own the mouse.
  if (tool === "pan") map.dragPan.enable(); else map.dragPan.disable();
  if (tool !== "measure") clearMeasure();
}
function cancelInteraction() { if (box) { box.el.remove(); box = null; } if (state.tool === "measure") clearMeasure(); }

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
  }
});
map.on("dblclick", (e) => { if (state.tool === "measure") { e.preventDefault(); finishMeasure(); } });

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
 * Phase 5 — Catalog
 * ===================================================================================== */
const host = location.hostname || "localhost";
let catSelected = null; const catOpen = new Set(["root", "gdb", "jobs"]); const catHistory = [];
function catalogModel() {
  const fc = (id, name, ic, layer) => ({ id, name, icon: ic, layer });
  return { id: "root", name: `\\\\${host.toUpperCase()}\\Cadastre`, icon: "server", children: [
    { id: "gdb", name: `${REGION}.gdb`, icon: "gdb", children: [
      fc("fc-parcels", `${REGION}_Parcels`, "polygonFc", "parcels"), fc("fc-jobs", `${REGION}_JobsArea`, "polygonFc", "jobsArea"),
      fc("fc-lines", `${REGION}_Lines`, "lineFc", "lines"), fc("fc-points", `${REGION}_Points`, "pointFc", "points"),
      fc("fc-ctrl", `${REGION}_CtrlPnts`, "pointFc", "ctrl")] },
    { id: "jobs", name: "Jobs", icon: "folder", children: state.jobs.map((j) => ({ id: "job-" + j.id, name: `${j.reference} (${j.status})`, icon: "jobOpen", job: j.id })) },
    { id: "imagery", name: "Imagery", icon: "folder", children: [fc("img-world", "World_Imagery", "raster", "imagery")] },
    { id: "reports", name: "Reports", icon: "folder", children: state.job ? [
      { id: "rep-plan", name: `${state.job.reference}_SitePlan.pdf`, icon: "sitePlan", pdf: "siteplan" },
      { id: "rep-report", name: `${state.job.reference}_Report.pdf`, icon: "report", pdf: "report" }] : [] },
    { id: "toolboxes", name: "Toolboxes", icon: "toolbox", children: [
      { id: "tool-validate", name: "Validate Fabric", icon: "validate", run: validateFabric },
      { id: "tool-survey", name: "Survey Closure Check", icon: "survey", run: () => openTask("survey") }] },
  ] };
}
function catFind(node, id) { if (node.id === id) return node; for (const c of node.children || []) { const r = catFind(c, id); if (r) return r; } return null; }
function catPath(node, id, acc = []) { if (node.id === id) return [...acc, node]; for (const c of node.children || []) { const r = catPath(c, id, [...acc, node]); if (r) return r; } return null; }
function catActivate(n) {
  if (n.layer) { const l = findLayer(n.layer); if (l) { l.visible = true; buildTOC(); zoomToLayer(l); } }
  else if (n.job) { $("jobBox").value = n.job; selectJob(n.job); }
  else if (n.pdf) openPdf(n.pdf);
  else if (n.run) n.run();
  else if (n.children) { catOpen.add(n.id); catSelect(n.id); }
}
function catSelect(id) { if (catSelected && catSelected !== id) catHistory.push(catSelected); catSelected = id; buildCatalog(); }
function buildCatalog() {
  const model = catalogModel(); if (!catSelected) catSelected = "gdb";
  const tree = $("catTree"); tree.innerHTML = "";
  const nodeEl = (n) => {
    const li = h("li"); const kids = n.children && n.children.length;
    const open = catOpen.has(n.id);
    const row = h("div", { class: "node" }, kids ? twisty(open, () => { open ? catOpen.delete(n.id) : catOpen.add(n.id); buildCatalog(); }) : h("span", { class: "tw none" }),
      h("span", { html: icon(open && n.icon === "folder" ? "folderOpen" : n.icon) }), h("span", { class: `lbl${catSelected === n.id ? " sel" : ""}` }, n.name));
    row.onclick = () => catSelect(n.id); row.ondblclick = () => catActivate(n);
    li.append(row);
    if (kids && open) { const ul = h("ul"); n.children.forEach((c) => ul.append(nodeEl(c))); li.append(ul); }
    return li;
  };
  tree.append(h("ul", { class: "tree" }, nodeEl(model)));
  // location box
  const loc = $("catLocation"); loc.innerHTML = "";
  const walk = (n, depth) => { if (n.children) { loc.append(h("option", { value: n.id, selected: n.id === catSelected }, " ".repeat(depth * 3) + n.name)); n.children.forEach((c) => walk(c, depth + 1)); } };
  walk(model, 0);
  const selNode = catFind(model, catSelected);
  if (selNode && !selNode.children) { const p = catPath(model, catSelected); if (p && p.length > 1) loc.value = p[p.length - 2].id; }
  // contents list
  const contents = $("catContents"); contents.innerHTML = "";
  const parent = selNode && selNode.children ? selNode : (catPath(model, catSelected) || []).slice(-2)[0];
  const ul = h("ul", { class: "tree" });
  for (const c of parent?.children || []) {
    const row = h("div", { class: "node" }, h("span", { html: icon(c.icon) }), h("span", { class: `lbl${catSelected === c.id ? " sel" : ""}` }, c.name));
    row.onclick = () => { catSelected = c.id; buildCatalog(); }; row.ondblclick = () => catActivate(c);
    ul.append(h("li", {}, row));
  }
  if (!parent?.children?.length) ul.append(h("li", { class: "muted", style: "padding:4px" }, "(empty)"));
  contents.append(ul);
}
$("catLocation").addEventListener("change", (e) => { catOpen.add(e.target.value); catSelect(e.target.value); });
(function buildCatTools() {
  const t = $("catTools");
  const add = (ic, tip, fn, dis) => t.append(h("span", { class: `tb${dis ? " disabled" : ""}`, title: tip, html: icon(ic), onclick: dis ? null : fn }));
  add("back", "Back", () => { const p = catHistory.pop(); if (p) { catSelected = p; buildCatalog(); } });
  add("up", "Up One Level", () => { const p = catPath(catalogModel(), catSelected); if (p && p.length > 1) { catSelected = p[p.length - 2].id; buildCatalog(); } });
  add("home", "Default Home Folder", () => catSelect("root"));
  add("connectFolder", "Connect To Folder", null, true);
  add("views", "Views", null, true);
  add("refresh", "Refresh", () => refreshAll());
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
function addGraphic(g) { state.graphics.push({ id: crypto.randomUUID?.() || String(Date.now()), ...drawStyle(), ...g }); renderGraphics(); }
function addTextGraphic(ll) { const text = prompt("Text:", "Text"); if (text) addGraphic({ type: "text", at: [ll.lng, ll.lat], text }); }
const gfxMarkers = [];
function renderGraphics() {
  saveGraphics();
  const feats = state.graphics.filter((g) => g.type === "rect").map((g) => ({ type: "Feature", properties: { id: g.id, stroke: g.stroke, fill: g.fill, fillOpacity: 0.35, selected: !!g.selected },
    geometry: { type: "Polygon", coordinates: [[[g.sw[0], g.sw[1]], [g.ne[0], g.sw[1]], [g.ne[0], g.ne[1]], [g.sw[0], g.ne[1]], [g.sw[0], g.sw[1]]]] } }));
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
  const b = new maplibregl.LngLatBounds(); gs.forEach((g) => (g.type === "rect" ? (b.extend(g.sw), b.extend(g.ne)) : b.extend(g.at)));
  if (!b.isEmpty()) map.fitBounds(b, { padding: 60, maxZoom: 18 });
}

/* =====================================================================================
 * Boot
 * ===================================================================================== */
async function refreshAll() { await Promise.all([loadParcels(false), loadBeacons(), loadJobs(), loadDrafts()]); buildTOC(); buildCatalog(); status("Refreshed"); }
initialDeepLink = initialViewFromURL(); // apply /editor?center=E,N&scale=N before any data arrives
buildMenus(); buildToolbars(); buildTocTools(); buildTOC(); renderTask(); buildCatalog(); updateNavButtons(); updateScaleBox();
