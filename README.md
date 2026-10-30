# Cadastre Editor

A Rust land-administration system replacing Aumentum Cadastre Editor 4.0.97.0 for parcel
capture and maintenance at PLAGIS. Phase 1 of a wider system (Registry / R of O / C of O
come later). See `../../Documents/Lovetech/PLAGIS/SRS.pdf` for the full specification.

## Workspace layout

| Path | What it is | Status |
|------|-----------|--------|
| `crates/cadastre-core` | Shared survey geometry engine (COGO, closure, area, adjustment, topology, CRS). Compiles to native **and** wasm32. | ✅ built |
| `crates/cadastre-server` | axum REST API, jobs/workflow, posting, PostGIS | planned |
| `crates/cadastre-import` | Read-only Aumentum MSSQL import | planned |
| `apps/editor` | Tauri 2 + Leptos desktop editor | planned |
| `apps/web` | Leptos web viewer + admin console | planned |

## Build & test

```sh
cargo test -p cadastre-core                              # unit + property + integration tests
cargo clippy --all-targets                               # lints
cargo build -p cadastre-core --target wasm32-unknown-unknown   # proves the shared-engine WASM target
```

## Deployment target

Both the editor and server ship as native `.exe` for **Windows Server 2012 R2**. This is a
deliberately old target that constrains the whole project — see
[docs/windows-2012r2-build.md](docs/windows-2012r2-build.md).
