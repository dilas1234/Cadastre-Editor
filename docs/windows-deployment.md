# Deploying the server to Windows Server 2012 R2

Staging: **10.10.10.4** (`MC`, Dell R720, Server 2012 R2). Production Aumentum Cadastre stays on
10.10.10.6 and the Registry keeps calling it until cut-over (`registry-integration.md`).

## 10.10.10.4 is shared: ports already in use

| Port | In use by |
|---|---|
| 80 / 443 | IIS 8.5 |
| 8080 | PLAGIS Liferay portal (Tomcat) — **not available**, so the Cadastre uses **8090** |
| 8081 | EnterpriseDB Apache |
| 5432 | PostgreSQL 9.2 (too old for the Cadastre) |
| 5433 | **PostgreSQL 14 + PostGIS 3.2.2**: the Cadastre's database (local connections only, scram) |

Nothing installed here may disturb Liferay or the existing PostgreSQL.

## 1. Build (Linux)

```sh
sudo apt install gcc-mingw-w64-x86-64                              # once
rustup toolchain install nightly --profile minimal -c rust-src     # once
scripts/build-windows.sh            # -> dist/cadastre-server-windows-<git describe>.zip
```

The script fails if the exe imports an API missing from 2012 R2. See `windows-2012r2-build.md`.

## 2. Database

The server needs PostgreSQL with **PostGIS** and **uuid-ossp** (migrations run
`CREATE EXTENSION` on first start, so the role must be allowed to, or an admin creates them first).

On 10.10.10.4 that is **PostgreSQL 14 on port 5433** (PostGIS 3.2.2 installed; 5432 is 9.2).
`pg_hba.conf` accepts only local connections, so run on the box, elevated:

```powershell
powershell -ExecutionPolicy Bypass -File .\setup-database.ps1
```

It asks for the `postgres` password, creates role and database `cadastre` and the two extensions,
and writes the connection string (generated password) into `C:\Cadastre\cadastre.env`.

## 3. Install

Copy the zip to the box (RDP, or `\\10.10.10.4\C$`), unzip, and in an elevated PowerShell:

```powershell
powershell -ExecutionPolicy Bypass -File .\install-cadastre.ps1   # installs, starts, waits for /health
```

The server runs as the **"Cadastre Server" scheduled task** (at boot, as SYSTEM, no time limit),
via `run-cadastre.cmd`, which restarts it if it exits. It is a task rather than a Windows service
because the exe does not implement the service control protocol; no third-party wrapper is needed.

Check from a PC: `http://10.10.10.4:8090/health`, `/ready`, `/editor`, `/swagger-ui`, or
`tools/smoke-test.sh` against it.

## 4. Upgrade

Build a new kit, unzip it on the box, run `install-cadastre.ps1` again. `cadastre.env`, `logs\`
and `files\` are kept; migrations apply on start.

## Staging rules

- `CADASTRE_AUTH=disabled` makes every request a superuser: internal network only, until
  Keycloak is set up.
- **Never set `CADASTRE_REGISTRY_WRITE=1` on staging**: it writes to the production Registry.
  The read-only link sync (`CADASTRE_REGISTRY_URL/USER/PASSWORD`) is safe but optional.
- The task runs as SYSTEM, which reaches network shares as the computer account `MC$`: catalog
  folders on `\\10.10.10.6\...` need read access granted to it.

## Not done yet

- Log rotation (`logs\cadastre-server.log` grows).
- Keycloak on the box (needs Java 21; 8081 is taken).
- IIS reverse proxy for the Aumentum paths on port 80 (`/ApplicationServerAdmin/...asmx`,
  `/GISViewer/MapWorker.aspx`), needed only when the Registry is pointed at this server.
- PostgreSQL over TLS (sqlx is built without a TLS backend; keep the database local or on a
  trusted LAN).
