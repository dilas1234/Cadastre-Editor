# cadastre-server (Phases 2–3)

The REST API and persistence layer for the Cadastre Editor. axum + sqlx/PostGIS, with an
OpenAPI document generated from the handlers. Built as a library (`cadastre_server`) plus a
thin binary, so integration tests drive the same router in-process.

## Layout

```
crates/cadastre-server/
  migrations/
    0001_init.sql            tolerances, beacons, parcels, audit (embedded via sqlx::migrate!)
    0002_jobs_workflow.sql   workflow config, job columns, job_event, edit_session, edit_lock
  src/
    lib.rs         module tree (pub, so tests can use it); main.rs is a thin wrapper
    main.rs        bootstrap: config → jwks → pool+migrate → router → serve (graceful shutdown)
    config.rs      env-driven Config (DATABASE_URL, bind, auth mode, Keycloak/JWKS settings)
    db.rs          pool construction + migration; pins search_path (see below)
    state.rs       AppState { pool, config, jwks } shared with every handler
    auth.rs        AuthUser extractor (RS256/JWKS verification; dev-bypass when disabled)
    jwks.rs        JWKS parsing + RS256 signature/claims verification (pure-Rust crypto)
    idp.rs         loads the JWKS at startup (inline / file / http fetch from the issuer)
    domain.rs      pure workflow + session logic (unit-tested without a DB)
    error.rs       AppError → JSON { status, error, message }
    models.rs      request/response DTOs (decoupled from rows and cadastre-core types)
    openapi.rs     ApiDoc aggregating #[utoipa::path] handlers
    routes/        health, tolerances, beacons, parcels, workflow, jobs, sessions, survey
  tests/
    integration.rs DB-backed tests (skipped unless CADASTRE_TEST_DATABASE_URL is set)
```

## Schema

All geometry is stored in **EPSG:26392** (Minna / Nigeria Mid Belt), matching `cadastre-core`.
Tables live in the `cadastre` schema:

- **0001**: `tolerance_config` (FR-ADM-03, seeded with `Tolerances::default()`), `beacon`,
  `parcel` + `parcel_corner`, `audit_log` (hash-chained; chaining logic lands in Phase 6).
- **0002**: `workflow_stage` + `workflow_transition` (FR-WF-01, FR-ADM-02; seeded with a
  draft→submitted→under_review→approved/rejected default), extended `job` columns
  (title, assigned_to, current_stage_id), `job_event` (per-job history), and `edit_session`
  + `edit_lock` for concurrent-edit control.

## Workflow, jobs and sessions (Phase 3)

- **Workflow** (`GET /api/v1/workflow`, admin `POST .../stages|.../transitions`): a
  configurable stage graph. Transitions are `(from_stage, action) -> to_stage` guarded by a
  role. The decision logic lives in `domain::resolve_transition` (pure, unit-tested).
- **Jobs** (`/api/v1/jobs`): create (role `editor`), list/get, `POST .../transition` (applies
  a workflow action, role-gated, recorded in `job_event`), and `.../events` history.
- **Edit sessions** (`/api/v1/sessions`): a user opens a session and acquires `edit_lock`s on
  resources (parcel|beacon|job). A resource has at most one lock. Contention returns **409**,
  unless the holding session is non-active or stale (no heartbeat within the 5-minute TTL), in
  which case it is reclaimed. Staleness is `domain::session_is_stale` (pure, unit-tested).

## Live survey check (Phase 4)

`POST /api/v1/survey/check` runs `cadastre_core::survey::SurveySession` against the active
tolerance config and returns closure, area, precision and pass/fail flags. The desktop editor
runs the *same* engine locally (WASM) for its sub-100ms live check (NFR-03); this endpoint is the
authoritative server-side re-validation, so submission can't drift from what the surveyor saw.
Legs are posted as `{"type":"line","bearing":<deg>,"distance":<m>}` or
`{"type":"curve","radius","arc_length","chord_bearing"}`. See `docs/phase-4-editor.md`.

### search_path pin (why `db.rs` sets it)

The dev DB user is `cadastre`, so Postgres's default `search_path` of `"$user", public, …`
resolves `$user` to a schema literally named `cadastre`. The first migration *creates* that
schema, so an unqualified table reference (notably sqlx's own `_sqlx_migrations` bookkeeping
table) would resolve to `public` on the first run and to `cadastre` on the next — making sqlx
think no migrations had been applied and re-running them. We pin `search_path` to `public` on
every connection so bookkeeping is stable; domain queries fully-qualify `cadastre.*` and PostGIS
functions live in `public`, so nothing else is affected.

## Auth (Phase 3: real RS256/JWKS verification)

`AuthUser` is an axum extractor:

- `CADASTRE_AUTH=disabled` (dev default): every request is a synthetic superuser holding
  `admin`, `editor`, `reviewer`, `approver`. Never use in production.
- `CADASTRE_AUTH=enabled`: a `Bearer` JWT is required. Its **RS256 signature is verified**
  against the JWKS (`jwks.rs`), `exp` is checked (60s leeway), and `iss`/`aud` are validated
  against `CADASTRE_KEYCLOAK_ISSUER`/`_AUDIENCE` when configured. Claims consumed: `sub`,
  `preferred_username`, `realm_access.roles`.

The JWKS is loaded once at startup (`idp::load_jwks`), resolved in order:
`CADASTRE_KEYCLOAK_JWKS_JSON` (inline) → `CADASTRE_KEYCLOAK_JWKS_PATH` (file) → fetch from
`{issuer}/protocol/openid-connect/certs`. The live fetch is **http:// only** for now; https is
deferred to Phase 9 with the TLS stack (use the inline/file override meanwhile).

Role checks: `admin` for tolerances/workflow config; `editor` for beacons, jobs, sessions and
job `submit`; `reviewer`/`approver` for the corresponding workflow transitions.

## Crypto / TLS / win7 note

All crypto here is **pure-Rust** (`rsa` + `sha2`, no `ring`), and sqlx is built with
`runtime-tokio` and **no TLS backend** — dev connects to Postgres over localhost without TLS.
Keeping ring/OpenSSL out of the tree protects the tier-3 `x86_64-win7-windows-msvc` server
target (NFR-02). A TLS backend (for Postgres and HTTPS JWKS fetch) and a vetted crypto provider
are selected in the Phase 9 packaging work.

## Fabric, posting, audit, output (Phases 5–7)

- **Fabric** (`POST /api/v1/fabric/validate`, `.../adjust`): topology checks (invalid rings,
  slivers, overlaps, duplicate beacons) and weighted least-squares network adjustment with
  blunder flagging — both reuse `cadastre-core`, validated against the active tolerances.
- **Posting & numbering** (`POST /api/v1/jobs/{id}/parcels`, `.../post`): parcels are captured as
  drafts under a job (no number yet), then posting allocates official numbers from
  `parcel_number_seq` (UPIN `PL`+12 digits, FR-NUM-01), marks them active, and records the event.
  Posting requires the `approver` role **and** the job to be in stage `approved`
  (`domain::can_post`); any other stage returns **409**, so no UPIN is issued before review.
- **Audit** (`GET /api/v1/audit`, `.../verify`): every job transition and posting appends a
  SHA-256 **hash-chained** entry (NFR-07/11); appends are serialised with a transaction advisory
  lock so the chain can't fork. `verify` recomputes the whole chain and reports the first break.
- **Search** (`GET /api/v1/parcels/search`): by number prefix, status, and/or bbox.
- **Output** (`GET /api/v1/jobs/{id}/report.pdf`, `.../siteplan.pdf`): a dependency-free PDF writer
  (`pdf.rs`) renders a job report and a scaled site plan. The SRS names typst for richer output;
  this keeps the server self-contained for now (revisit in Phase 9).
- **Tiles**: the `tiles` (Martin) compose service publishes the PostGIS geometry tables as vector
  tiles for the map. Config only — not exercised by the test suite.

**Deferred GUIs (consistent with the Phase 4 editor):** the fabric-editing UI (Phase 5) and the
Leptos viewer/admin console (Phase 7) are not built here — they need the browser/Tauri stack and
on-target verification. The server endpoints and `cadastre-core` engine they bind to are complete
and tested.

## Testing

- `cargo test -p cadastre-server` runs unit tests (workflow/session logic in `domain.rs`, JWT
  verification in `jwks.rs`) with no external dependencies.
- Set `CADASTRE_TEST_DATABASE_URL` to also run `tests/integration.rs`, which drives the router
  against a live PostGIS (e.g. the `docker compose` db): full job workflow, lock
  contention/reclaim, and auth role enforcement over HTTP. Without the variable those tests
  skip, so the default `cargo test` stays hermetic.

## Editor UI (`/editor`)

A browser editor laid out like the desktop GIS editor PLAGIS staff already use: title bar
(`<Region> - PLAGIS CADASTRE Editor - Connected as <user>`, from `GET /api/v1/me`), menu bar,
Tools / Standard / Drawing toolbars, the **Cadastre** toolbar (Region, Job Area, job workflow,
site plan / report, validate), a docked **Table Of Contents** (drawing-order tree with legend,
list by source / visibility / selection, and the **Tasks** wizard), a docked **Catalog**
(Location, tree, contents) and a status bar with live UTM 32N coordinates in metres.

- Assets: `web/` (`index.html`, `style.css`, `icons.js` — original SVG icons, `app.js`), embedded
  with rust-embed. MapLibre and proj4 are **vendored** (`web/vendor/`), so the editor works on a LAN
  without internet; only the imagery basemap (Esri World Imagery tiles) needs internet.
- Layers and data are wired on `style.load`, not `load`, so an unreachable imagery server never
  blocks jobs, parcels or tools.
- Deep link: `/editor?center=<E>,<N>&scale=<n>` (UTM 32N metres) opens straight to a view.
- Tools: zoom in/out (click or box), pan, full extent (= region extent), fixed zoom, previous/next
  extent, select (click/box, Shift adds), identify, find (UPIN), measure, go to XY, scale box,
  bookmarks; attribute table and layer properties from the TOC context menu.
- Tasks: *New parcel from survey (COGO)* — job → start point (map click, control point or typed)
  → traverse (bearings in decimal degrees or `d m s`) with the live server closure check → save as
  a draft; *Job workflow* — stage, actions, post (approved jobs only), history.
- Drawing: rectangles and text with font, size, B/I/U and colours; kept in the browser
  (`localStorage`), not in the database.
- Demo data for a dev server: `tools/seed-demo.sh` (control points, a posted block, an open job
  near Jos).

### Catalog (`GET /api/v1/catalog`, `GET /api/v1/catalog/items?folder=&path=`)

The editor's Catalog pane browses real folder connections such as `\\PRODUCTIONMC\Image`,
configured with `CADASTRE_CATALOG_FOLDERS` (`name=path;…`; on PRODUCTIONMC:
`\\PRODUCTIONMC\Image=D:\Image`). Read-only, listed the way ArcCatalog lists a folder: folders,
file geodatabases, toolboxes, then datasets; sidecars (`.shx/.dbf/.prj/.ovr/.aux.xml/.tfw`…) and
non-GIS files (PDF, Office, `Thumbs.db`) hidden; shapefiles carry their geometry type (read from the
`.shp` header); TIFF/BigTIFF rasters expand into `Band_n` (band count read from the header only —
rasters on the share exceed 100 GB). Paths are confined to the configured roots (`..`, absolute
paths and drive letters are rejected). The server's service account needs read access to the share.
