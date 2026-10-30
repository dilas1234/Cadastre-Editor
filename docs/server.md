# cadastre-server (Phase 2)

The REST API and persistence layer for the Cadastre Editor. axum + sqlx/PostGIS, with an
OpenAPI document generated from the handlers.

## Layout

```
crates/cadastre-server/
  migrations/0001_init.sql   embedded via sqlx::migrate! (no sqlx-cli needed)
  src/
    main.rs        bootstrap: config → pool+migrate → router → serve (graceful shutdown)
    config.rs      env-driven Config (DATABASE_URL, bind, auth mode, Keycloak settings)
    db.rs          pool construction + migration; pins search_path (see below)
    state.rs       AppState { pool, config } shared with every handler
    auth.rs        AuthUser extractor — Keycloak JWT claims (STUB, see below)
    error.rs       AppError → JSON { status, error, message }
    models.rs      request/response DTOs (decoupled from rows and cadastre-core types)
    openapi.rs     ApiDoc aggregating #[utoipa::path] handlers
    routes/        health, tolerances, beacons, parcels
```

## Schema

All geometry is stored in **EPSG:26392** (Minna / Nigeria Mid Belt), matching `cadastre-core`.
Tables live in the `cadastre` schema: `tolerance_config` (FR-ADM-03, seeded with
`Tolerances::default()`), `beacon`, `parcel` + `parcel_corner`, `job` (Phase 3 placeholder),
and `audit_log` (hash-chained; chaining logic lands in Phase 6).

### search_path pin (why `db.rs` sets it)

The dev DB user is `cadastre`, so Postgres's default `search_path` of `"$user", public, …`
resolves `$user` to a schema literally named `cadastre`. The first migration *creates* that
schema, so an unqualified table reference (notably sqlx's own `_sqlx_migrations` bookkeeping
table) would resolve to `public` on the first run and to `cadastre` on the next — making sqlx
think no migrations had been applied and re-running them. We pin `search_path` to `public` on
every connection so bookkeeping is stable; domain queries fully-qualify `cadastre.*` and PostGIS
functions live in `public`, so nothing else is affected.

## Auth (STUB — Phase 2)

`AuthUser` is an axum extractor:

- `CADASTRE_AUTH=disabled` (dev default): every request is a synthetic user holding `admin` and
  `editor`. Never use in production.
- `CADASTRE_AUTH=enabled`: a `Bearer` JWT is required. The extractor decodes the claims
  (`sub`, `preferred_username`, `realm_access.roles`) **without verifying the signature**.

Phase 3 replaces the unverified decode with JWKS-based verification against the Keycloak realm
(`CADASTRE_KEYCLOAK_ISSUER` / `_AUDIENCE`). The extractor's public shape does not change.

Role checks: `PUT /api/v1/tolerances` needs `admin`; `POST /api/v1/beacons` needs `editor`.

## TLS / win7 note

sqlx is built with `runtime-tokio` and **no TLS backend**: dev connects to Postgres over
localhost without TLS, and keeping the rustls/ring crypto stack out of the tree protects the
tier-3 `x86_64-win7-windows-msvc` server target (NFR-02). A TLS backend and a vetted crypto
provider are selected in the Phase 9 packaging work.
