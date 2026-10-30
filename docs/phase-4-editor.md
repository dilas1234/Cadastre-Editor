# Phase 4 — Editor app (architecture + scaffold)

The desktop editor for parcel capture and maintenance: Tauri 2 shell, Leptos (Rust→WASM)
UI, MapLibre GL map. SRS: FR-SUR-* (survey entry), FR-COG-02 (live closure), NFR-03
(live check < 100 ms).

> **Status.** The *engine* this app is built on is implemented and tested now:
> `cadastre_core::survey::SurveySession` (live closure/area/precision) and the authoritative
> server endpoint `POST /api/v1/survey/check`. The GUI shell below is specified and scaffolded
> but **not runtime-verified from this environment** — it needs the Tauri toolchain and, for the
> win7 target, on-target hardware (see the Phase 0 gate). This doc is the contract the GUI build
> implements.

## Why the engine already exists

The whole point of the shared `cadastre-core` crate (SRS §3) is that the editor's live check and
the server's submission check run **the same code**. `cadastre-core` compiles to wasm32 (verified:
`cargo build -p cadastre-core --target wasm32-unknown-unknown`), so the Leptos UI calls
`SurveySession::summary()` in-process — no round-trip — and gets closure, area, precision and the
red/green tolerance flags in microseconds (NFR-03 has a unit test proving < 50 ms for 2000 legs).
On submit, the server re-runs the identical engine via `/api/v1/survey/check` against the active
tolerance config, so what the surveyor saw cannot drift from what is validated.

## Layout (`apps/editor/`, outside the cargo workspace)

Kept out of `members` in the root `Cargo.toml` so `cargo build --workspace` stays green without
the Tauri system dependencies. Built on its own with the Tauri CLI.

```
apps/editor/
  src-tauri/            Tauri 2 Rust backend (window, bundled WebView2, IPC commands)
    Cargo.toml          depends on cadastre-core (+ cadastre-server client later)
    tauri.conf.json     window + bundle config; pins a fixed-version WebView2 (Phase 0)
    src/main.rs         Tauri commands: survey_check(session) -> SurveySummary, etc.
  src/                  Leptos frontend (compiled to WASM, served into the webview)
    app.rs              root component
    traverse_grid.rs    editable leg grid bound to SurveySession (the live check UI)
    map.rs              MapLibre GL wrapper (parcel fabric + live figure overlay)
  index.html, Trunk.toml / Vite config for the frontend bundle
```

## Traverse-grid UX (FR-SUR-*, FR-COG-02)

The core of the editor. A spreadsheet-like grid, one row per leg:

| # | Type | Bearing (DMS) | Distance (m) | Radius | Arc len |
|---|------|---------------|--------------|--------|---------|

- Each edit maps to a `SurveySession` op: `push_course`/`push_curve`, `replace(i, …)`,
  `insert(i, …)`, `remove(i)` — all already implemented and unit-tested in `survey.rs`.
- After every keystroke the UI calls `session.summary(&tolerances)` and repaints a status
  panel: **linear misclosure**, **precision ratio** (1:N), **computed area**, **area vs stated**,
  and three indicators driven by `SurveyChecks` — *closes on start*, *meets precision*,
  *area within tolerance*. Green/amber/red, no server round-trip.
- Bearings display/parse as DMS via `cadastre_core::units` (`Bearing::from_dms` / `to_dms`),
  which are lossless to the stored decimal-seconds representation.

## Map (MapLibre GL)

- Base fabric (existing parcels/beacons) served as vector tiles by the server (Martin, Phase 7)
  or fetched as GeoJSON for small jobs.
- The in-progress figure is drawn as an overlay from `SurveySession::vertices()`; the misclosure
  gap is highlighted using `closure.misclosure_*`.
- Projected CRS is EPSG:26392; MapLibre renders in its own web-mercator view, so coordinates are
  transformed for display via `cadastre_core::crs` (WGS84) — the stored geometry stays in 26392.

## Phase 0 gate (the #1 project risk) — do before shipping the GUI

A trivial Tauri 2 window bundling a **fixed-version WebView2 runtime** (the last build compatible
with Windows Server 2012 R2 — Evergreen WebView2 dropped 2012 R2 in early 2024) must **launch on a
real 2012 R2 VM**. If Tauri 2 requires newer WebView2 APIs than that runtime exposes, fall back per
the plan (older WebView2 shim, or a non-WebView UI). This gate cannot be exercised from macOS; it
needs the VM. Until it passes, treat the GUI stack choice as provisional.

See `docs/windows-2012r2-build.md` for the tier-3 `x86_64-win7-windows-msvc` build invocation used
for the editor `.exe`.
