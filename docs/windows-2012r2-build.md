# Building for Windows Server 2012 R2

Windows Server 2012 R2 (NT 6.3, the Windows 8.1 era) is the deployment target for **both**
the server and the desktop editor. It predates what the modern Rust toolchain and the Tauri /
WebView2 stack assume, so it drives several decisions across the codebase. This document is the
single reference for them.

## Why it's hard

1. **Rust std dropped Windows 8.1 / Server 2012 R2 for the standard MSVC targets in Rust 1.78.**
   `x86_64-pc-windows-msvc` now requires Windows 10. The 2012 R2-capable path is the tier-3
   target **`x86_64-win7-windows-msvc`** (NT 6.1 baseline — runs on anything newer, incl. 2012 R2),
   which must be built with **nightly + `-Z build-std`** because tier-3 targets ship no prebuilt std.

2. **WebView2 Runtime dropped Windows 7 / 8.1 / Server 2012 R2 support in early 2024.** The last
   compatible build is the Edge 109-era runtime. The editor must **bundle a fixed-version WebView2
   Runtime** (Tauri's fixed-version distribution mode) rather than rely on the Evergreen runtime,
   which will not install. **This is the project's #1 risk** — see the spike below.

## Rules baked into the code

- **No native C dependencies** that would complicate the win7 cross-build:
  - CRS math is pure Rust (`cadastre-core::crs`), not the `proj` C library.
  - Use `rustls` for TLS, not OpenSSL/schannel.
- Daily development uses **stable** (`rust-toolchain.toml`); the win7 build is an explicit,
  separate invocation (below).
- Builds run **on Windows** (a Windows box or Windows CI runner). Cross-compiling MSVC from macOS
  is not the supported path.

## The win7 build command (server and editor logic)

Requires the **nightly** toolchain with the `rust-src` component, on Windows:

```powershell
rustup toolchain install nightly
rustup component add rust-src --toolchain nightly

cargo +nightly build --release `
  --target x86_64-win7-windows-msvc `
  -Z build-std=std,panic_abort `
  -Z build-std-features=panic_immediate_abort `
  -p cadastre-server
```

CI should run this build from Phase 0 so a dependency that quietly requires Windows 10 APIs is
caught immediately, not at packaging time.

## Phase 0 gate — the WebView2 spike (do before any editor UI work)

Before building the Tauri editor, prove the shell runs on the target:

1. Build a trivial Tauri 2 app that opens one window.
2. Package it with a **bundled fixed-version WebView2 Runtime** (last 2012 R2-compatible build).
3. Launch it on a **real Windows Server 2012 R2 VM**.

If it launches → proceed. If it fails (Tauri 2 needs newer WebView2 APIs than v109 provides),
fall back down this ladder:

1. Tauri **v1** (older Wry / WebView2 API surface).
2. An older Wry pinned to a 2012 R2-compatible WebView2.
3. A native Win32 / `egui` shell embedding MapLibre via a lighter webview.

The server has no WebView2 dependency and is the low-risk half of the deliverable.
