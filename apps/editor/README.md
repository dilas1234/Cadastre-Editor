# apps/editor (Phase 4 — scaffold)

Tauri 2 + Leptos + MapLibre desktop editor for survey entry and parcel maintenance.

**This directory is a scaffold, not yet a buildable app.** It is intentionally **outside** the
Cargo workspace (`members` in the root `Cargo.toml`) so `cargo build --workspace` does not require
the Tauri system toolchain. The architecture, the traverse-grid UX, and the engine binding are
specified in [`../../docs/phase-4-editor.md`](../../docs/phase-4-editor.md).

## What is already done and tested (the foundation this app binds to)

- `cadastre_core::survey::SurveySession` — live, editable traverse with
  `summary(&Tolerances)` returning closure, area, precision and pass/fail checks. Unit-tested,
  including an NFR-03 timing test (< 50 ms for 2000 legs). Compiles to wasm32, so the Leptos UI
  runs it in-process for the live check.
- `POST /api/v1/survey/check` on `cadastre-server` — the identical engine re-run server-side
  against the active tolerance config, for authoritative submission-time validation.

## Building (on a dev machine with the Tauri toolchain)

Not wired yet. When implemented:

```sh
# prerequisites: Rust, Node/pnpm (or Trunk for a pure-Rust/Leptos frontend), tauri-cli,
# and the platform webview deps (WebView2 on Windows; webkit2gtk on Linux).
cd apps/editor
cargo tauri dev        # run the editor in development
cargo tauri build      # produce an installer / .exe
```

The Windows Server 2012 R2 `.exe` uses the tier-3 `x86_64-win7-windows-msvc` target and a
**bundled fixed-version WebView2** — gated by the Phase 0 proof-of-life on a real 2012 R2 VM
(see the architecture doc). That gate is the #1 project risk and must pass before the GUI stack
is considered final.
