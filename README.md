# Cadastre Editor

A Rust land-administration system replacing Aumentum Cadastre Editor 4.0.97.0 for parcel
capture and maintenance at PLAGIS. Phase 1 of a wider system (Registry / R of O / C of O
come later). See `../../Documents/Lovetech/PLAGIS/SRS.pdf` for the full specification.

## Workspace layout

| Path | What it is | Status |
|------|-----------|--------|
| `crates/cadastre-core` | Shared survey geometry engine (COGO, closure, area, adjustment, topology, CRS). Compiles to native **and** wasm32. | ✅ built |
| `crates/cadastre-server` | axum REST API + PostGIS + Keycloak auth. Tolerances, beacons/parcels, workflow, jobs, sessions/locks, live survey check, fabric topology + least-squares adjustment, posting/numbering, hash-chained audit, parcel search, PDF reports/site-plans. | ✅ Phases 2–3, 5–7 (server) |
| `crates/cadastre-import` | Read-only Aumentum ArcSDE import: DEFAULT-version reconstruction, WKT geometry, status/exception mapping. Pure logic tested; live tiberius reader behind `--features live`. | 🚧 Phase 8 (logic done) |
| `apps/editor` | Tauri 2 + Leptos + MapLibre desktop editor. Live-survey engine + server check done; GUI shell scaffolded (see `docs/phase-4-editor.md`). | 🚧 engine done, GUI scaffold (Phase 4) |
| `apps/web` | Leptos web viewer + admin console | planned |

## Build & test

```sh
cargo test -p cadastre-core                              # unit + property + integration tests
cargo clippy --all-targets                               # lints
cargo build -p cadastre-core --target wasm32-unknown-unknown   # proves the shared-engine WASM target
```

## Running the server (Phase 2)

```sh
docker compose up -d db                 # PostGIS on :5432 (+ `keycloak` service on :8081)
cp .env.example .env                    # dev config (CADASTRE_AUTH=disabled)
set -a; . ./.env; set +a
cargo run -p cadastre-server            # migrations run on startup; API on :8080
```

- OpenAPI/Swagger UI: `http://localhost:8080/swagger-ui`, spec at `/api-docs/openapi.json`.
- Probes: `GET /health` (liveness), `GET /ready` (DB reachable).
- Resources: tolerances, beacons, parcels (+ search), `workflow`, `jobs` (+ transitions/events/
  parcels/post/import), `sessions` (+ locks), `survey/check` (live closure/area), `survey/tools`
  (the 35 computation tools), `import` (preview/attributes/world files), `fabric/validate` +
  `fabric/adjust`, `audit` (+ verify), and job `report.pdf` / `siteplan.pdf`. See
  `docs/server.md` and `docs/survey-computation-and-import.md`.
- `CADASTRE_AUTH=disabled` runs every request as a dev superuser. Set `enabled` to require a
  Keycloak bearer token whose RS256 signature is verified against the JWKS (fetched from the
  issuer, or supplied via `CADASTRE_KEYCLOAK_JWKS_JSON`/`_PATH`).

### Testing

```sh
cargo test -p cadastre-server            # hermetic: workflow/session + JWT unit tests
CADASTRE_TEST_DATABASE_URL=postgres://cadastre:cadastre@localhost:5432/cadastre \
  cargo test -p cadastre-server          # also runs DB-backed integration tests
```

## Deployment target

Both the editor and server ship as native `.exe` for **Windows Server 2012 R2**. This is a
deliberately old target that constrains the whole project — see
[docs/windows-2012r2-build.md](docs/windows-2012r2-build.md).
