# Cadastre server — Windows Server 2012 R2 deploy kit

Built by `scripts/build-windows.sh` (see `docs/windows-deployment.md`). Contents:

| File | Purpose |
|---|---|
| `cadastre-server.exe` | The server. The editor/viewer web app is built in; migrations run on start. Needs only DLLs every 2012 R2 has. |
| `cadastre.env.example` | Settings template; becomes `cadastre.env`. |
| `setup-database.ps1` | Once, first: creates role + database `cadastre` and PostGIS in local PostgreSQL 14 (port 5433), writes the connection string into `C:\Cadastre\cadastre.env`. Asks for the `postgres` password. |
| `install-cadastre.ps1` | Install **or upgrade**: copy to `C:\Cadastre`, register the boot task, open the firewall port, start. |
| `start-cadastre.ps1` / `stop-cadastre.ps1` | Start (waits for `/health`) / stop. |
| `run-cadastre.cmd` | What the task runs: loads `cadastre.env`, runs the exe, restarts it if it exits, logs to `logs\`. |

## First install

In an **elevated** PowerShell, in the unzipped kit folder:

```powershell
powershell -ExecutionPolicy Bypass -File .\setup-database.ps1     # asks for the postgres password
powershell -ExecutionPolicy Bypass -File .\install-cadastre.ps1   # installs and starts
```

Then open `http://<server>:8090/editor`.

## Upgrade

Unzip the new kit and run `install-cadastre.ps1` again. It stops the server, replaces the exe,
keeps `cadastre.env`, `logs\` and `files\`, and starts it.

Logs: `C:\Cadastre\logs\cadastre-server.log` (not rotated yet).
