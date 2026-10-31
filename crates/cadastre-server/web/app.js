"use strict";
// PLAGIS Cadastre Editor — browser client over the REST API. Geometry CRS is EPSG:32632
// (WGS84/UTM 32N); the map is WGS84, so we project with proj4 for readouts and survey drawing.
proj4.defs("EPSG:32632", "+proj=utm +zone=32 +datum=WGS84 +units=m +no_defs");
const toUTM = (lng, lat) => proj4("EPSG:4326", "EPSG:32632", [lng, lat]);
const toLL = (e, n) => proj4("EPSG:32632", "EPSG:4326", [e, n]);
const $ = (id) => document.getElementById(id);

const IMAGERY = "https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/{z}/{y}/{x}";

const map = new maplibregl.Map({
  container: "map",
  style: {
    version: 8,
    sources: { imagery: { type: "raster", tiles: [IMAGERY], tileSize: 256, attribution: "Esri World Imagery" } },
    layers: [{ id: "imagery", type: "raster", source: "imagery" }],
  },
  center: [8.9, 9.9], zoom: 12,
});
map.addControl(new maplibregl.NavigationControl());
map.addControl(new maplibregl.ScaleControl({ unit: "metric" }));

const empty = () => ({ type: "FeatureCollection", features: [] });

map.on("load", async () => {
  map.addSource("parcels", { type: "geojson", data: empty() });
  map.addLayer({ id: "parcels-fill", type: "fill", source: "parcels",
    paint: { "fill-color": "#39b", "fill-opacity": 0.2 } });
  map.addLayer({ id: "parcels-line", type: "line", source: "parcels",
    paint: { "line-color": "#6cf", "line-width": 0.8 } });

  map.addSource("beacons", { type: "geojson", data: empty() });
  map.addLayer({ id: "beacons-pt", type: "circle", source: "beacons",
    paint: { "circle-radius": 3, "circle-color": "#ff4", "circle-stroke-color": "#333", "circle-stroke-width": 0.8 } });

  map.addSource("survey", { type: "geojson", data: empty() });
  map.addLayer({ id: "survey-fill", type: "fill", source: "survey",
    filter: ["==", "$type", "Polygon"], paint: { "fill-color": "#f80", "fill-opacity": 0.2 } });
  map.addLayer({ id: "survey-line", type: "line", source: "survey",
    filter: ["==", "$type", "LineString"], paint: { "line-color": "#f80", "line-width": 2 } });
  map.addLayer({ id: "survey-pt", type: "circle", source: "survey",
    filter: ["==", "$type", "Point"], paint: { "circle-radius": 4, "circle-color": "#f80", "circle-stroke-color": "#fff", "circle-stroke-width": 1 } });

  await loadParcels(true);
  loadBeacons();
  loadJobs();
});

async function loadParcels(fit) {
  const r = await fetch("/api/v1/parcels.geojson?limit=6000");
  const fc = await r.json();
  map.getSource("parcels").setData(fc);
  $("count").textContent = fc.features.length + " parcels";
  if (fit && fc.features.length) {
    const b = new maplibregl.LngLatBounds();
    for (const f of fc.features) {
      const g = f.geometry; if (!g) continue;
      const rings = g.type === "MultiPolygon" ? g.coordinates.flat() : g.coordinates;
      for (const ring of rings) for (const c of ring) b.extend(c);
    }
    map.fitBounds(b, { padding: 30, maxZoom: 17 });
  }
}
async function loadBeacons() {
  const r = await fetch("/api/v1/beacons.geojson");
  map.getSource("beacons").setData(await r.json());
}

// --- layer toggles ---
const vis = (layers, on) => layers.forEach((l) => map.getLayer(l) && map.setLayoutProperty(l, "visibility", on ? "visible" : "none"));
$("lyr-imagery").onchange = (e) => vis(["imagery"], e.target.checked);
$("lyr-parcels").onchange = (e) => vis(["parcels-fill", "parcels-line"], e.target.checked);
$("lyr-beacons").onchange = (e) => vis(["beacons-pt"], e.target.checked);

// --- coordinate readout ---
map.on("mousemove", (e) => {
  const { lng, lat } = e.lngLat;
  $("coordLL").textContent = `${lat.toFixed(6)}, ${lng.toFixed(6)}`;
  const [E, N] = toUTM(lng, lat);
  $("coordUTM").textContent = `E ${E.toFixed(2)}  N ${N.toFixed(2)}  (EPSG:32632)`;
});

// --- parcel popup ---
map.on("click", "parcels-fill", (e) => {
  const p = e.features[0].properties;
  new maplibregl.Popup().setLngLat(e.lngLat)
    .setHTML(`<b>${p.parcel_number || "(unposted)"}</b><br/>status: ${p.status}<br/>area: ${p.area} m²`)
    .addTo(map);
});
map.on("mouseenter", "parcels-fill", () => (map.getCanvas().style.cursor = "pointer"));
map.on("mouseleave", "parcels-fill", () => (map.getCanvas().style.cursor = ""));

// --- search ---
$("search").addEventListener("keydown", async (ev) => {
  if (ev.key !== "Enter") return;
  const q = ev.target.value.trim(); if (!q) return;
  const r = await fetch("/api/v1/parcels/search?number=" + encodeURIComponent(q));
  const list = await r.json();
  $("searchResults").textContent = list.length + " match(es)";
});

// --- tabs ---
let surveyActive = false;
document.querySelectorAll(".tab").forEach((t) => t.onclick = () => {
  document.querySelectorAll(".tab").forEach((x) => x.classList.remove("active"));
  t.classList.add("active");
  const name = t.dataset.tab;
  surveyActive = name === "survey";
  document.querySelectorAll(".panel").forEach((p) => p.classList.toggle("hidden", p.dataset.panel !== name));
});

// --- jobs + workflow ---
let workflow = { transitions: [] };
async function loadJobs() {
  workflow = await (await fetch("/api/v1/workflow")).json();
  const jobs = await (await fetch("/api/v1/jobs")).json();
  const sel = $("jobSelect");
  jobs.slice(0, 200).forEach((j) => {
    const o = document.createElement("option");
    o.value = j.id; o.textContent = `${j.reference} (${j.status})`; o.dataset.status = j.status;
    sel.appendChild(o);
  });
}
$("jobSelect").onchange = (e) => {
  const opt = e.target.selectedOptions[0];
  const stage = opt ? opt.dataset.status : null;
  renderJob(e.target.value, stage);
};
function renderJob(id, stage) {
  $("jobStage").textContent = stage || "—";
  const acts = $("jobActions"); acts.innerHTML = "";
  if (!id) return;
  workflow.transitions.filter((t) => t.from_stage === stage).forEach((t) => {
    const b = document.createElement("button");
    b.textContent = t.action; b.className = "ghost";
    b.onclick = () => transition(id, t.action);
    acts.appendChild(b);
  });
}
async function transition(id, action) {
  const r = await fetch(`/api/v1/jobs/${id}/transition`, {
    method: "POST", headers: { "content-type": "application/json" },
    body: JSON.stringify({ action }),
  });
  if (r.ok) { const j = await r.json(); $("jobStage").textContent = j.status; renderJob(id, j.status); }
  else { alert("transition failed: " + r.status); }
}

// --- survey entry ---
const legsBody = document.querySelector("#legs tbody");
function addLegRow(bearing = "", dist = "") {
  const tr = document.createElement("tr");
  tr.innerHTML = `<td class="n"></td>
    <td><input class="brg" type="number" step="0.0001" value="${bearing}"></td>
    <td><input class="dst" type="number" step="0.001" value="${dist}"></td>
    <td><button class="rm">×</button></td>`;
  tr.querySelector(".rm").onclick = () => { tr.remove(); refreshSurvey(); };
  tr.querySelectorAll("input").forEach((i) => i.oninput = refreshSurvey);
  legsBody.appendChild(tr);
  renumber();
}
function renumber() { legsBody.querySelectorAll("tr").forEach((tr, i) => tr.querySelector(".n").textContent = i + 1); }
$("addLeg").onclick = () => addLegRow();
$("startE").oninput = refreshSurvey; $("startN").oninput = refreshSurvey; $("statedArea").oninput = refreshSurvey;

map.on("click", (e) => {
  if (!surveyActive) return;
  const [E, N] = toUTM(e.lngLat.lng, e.lngLat.lat);
  $("startE").value = E.toFixed(3); $("startN").value = N.toFixed(3);
  refreshSurvey();
});

function currentLegs() {
  return [...legsBody.querySelectorAll("tr")].map((tr) => ({
    bearing: parseFloat(tr.querySelector(".brg").value),
    distance: parseFloat(tr.querySelector(".dst").value),
  })).filter((l) => isFinite(l.bearing) && isFinite(l.distance) && l.distance > 0);
}

function drawSurvey(startE, startN, legs) {
  let E = startE, N = startN;
  const utm = [[E, N]];
  for (const l of legs) {
    const r = (l.bearing * Math.PI) / 180;
    E += l.distance * Math.sin(r); N += l.distance * Math.cos(r);
    utm.push([E, N]);
  }
  const ll = utm.map(([e, n]) => toLL(e, n));
  const feats = [{ type: "Feature", geometry: { type: "LineString", coordinates: ll } }];
  ll.forEach((c) => feats.push({ type: "Feature", geometry: { type: "Point", coordinates: c } }));
  if (ll.length >= 4) feats.push({ type: "Feature", geometry: { type: "Polygon", coordinates: [ll] } });
  map.getSource("survey").setData({ type: "FeatureCollection", features: feats });
}

let checkTimer = null;
function refreshSurvey() {
  const startE = parseFloat($("startE").value), startN = parseFloat($("startN").value);
  const legs = currentLegs();
  if (!isFinite(startE) || !isFinite(startN)) { $("checkOut").textContent = "set a start point"; return; }
  drawSurvey(startE, startN, legs);
  if (!legs.length) { $("checkOut").textContent = "add legs to check closure"; return; }
  clearTimeout(checkTimer);
  checkTimer = setTimeout(() => runCheck(startE, startN, legs), 150);
}

async function runCheck(startE, startN, legs) {
  const stated = parseFloat($("statedArea").value);
  const body = {
    start: { x: startE, y: startN },
    legs: legs.map((l) => ({ type: "line", bearing: l.bearing, distance: l.distance })),
    stated_area: isFinite(stated) ? stated : null,
  };
  const r = await fetch("/api/v1/survey/check", {
    method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body),
  });
  if (!r.ok) { $("checkOut").textContent = "check failed: " + r.status; return; }
  const s = await r.json();
  const flag = (ok, label) => ok == null ? `<span class="muted">${label}: n/a</span>`
    : `<span class="flag-${ok ? "ok" : "bad"}">${ok ? "✓" : "✗"} ${label}</span>`;
  const prec = s.closure && s.closure.precision_ratio ? "1:" + Math.round(s.closure.precision_ratio) : "exact";
  const mis = s.closure ? s.closure.linear_misclosure.toFixed(3) + " m" : "—";
  const area = s.area != null ? s.area.toFixed(1) + " m²" : "—";
  $("checkOut").innerHTML =
    `legs ${s.leg_count}, vertices ${s.vertex_count}\n` +
    `misclosure: ${mis}\nprecision: ${prec}\narea: ${area}\n` +
    `${flag(s.checks.closes_on_start, "closes")}  ` +
    `${flag(s.checks.meets_precision, "precision")}  ` +
    `${flag(s.checks.area_within_tolerance, "area")}`;
}

addLegRow(); // start with one empty leg
