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
    routes/        health, tolerances, beacons, parcels, workflow, jobs, sessions
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

## Testing

- `cargo test -p cadastre-server` runs unit tests (workflow/session logic in `domain.rs`, JWT
  verification in `jwks.rs`) with no external dependencies.
- Set `CADASTRE_TEST_DATABASE_URL` to also run `tests/integration.rs`, which drives the router
  against a live PostGIS (e.g. the `docker compose` db): full job workflow, lock
  contention/reclaim, and auth role enforcement over HTTP. Without the variable those tests
  skip, so the default `cargo test` stays hermetic.
