# Aumentum Cadastre Editor — Technical Assessment & Replacement Scope

**Prepared:** 2026-10-31 · **Systems inspected:** `PRODUCTIONMC` (10.10.10.6, production), `MC`
(10.10.10.4, staging) · **Method:** read-only inventory scripts run over RDP
(`tools/aumentum-inventory/`), an earlier read-only SQL inspection of the production `sde`
database (2026-10-30, see `docs/aumentum-import.md`), and review of this repository.
No change was made to either server or to the Aumentum application.

## 0. How to read this document

Every finding carries an evidence tag:

| Tag | Meaning |
|-----|---------|
| **[V-PROD]** | Verified on production (10.10.10.6) from the inventory reports of 2026-10-31 |
| **[V-STG]** | Verified on staging (10.10.10.4) from the inventory report of 2026-10-31 |
| **[V-SQL]** | Verified by direct read-only SQL on the production `sde` DB, 2026-10-30 (`docs/aumentum-import.md`) |
| **[V-CODE]** | Verified in this repository |
| **[A]** | Assumption / inference — reasonable but **not** directly observed; confirm before relying on it |
| **[GAP]** | Not yet inspected; listed in §14 with how to close it |

Raw evidence (git-ignored, contains usernames and licence data):
`tools/aumentum-inventory/aumentum-inventory-PRODUCTIONMC-*.txt`,
`aumentum-deep-PRODUCTIONMC-*.txt`, `aumentum-inventory-MC-*.txt`.

---

## 1. Executive summary

1. **What Aumentum is here.** *Aumentum Cadastre 4.0.97* (Thomson Reuters; internally
   "ILS MultiCadastre", originally International Land Systems) — an **ArcMap 10.2.1 add-in**
   plus a **.NET Windows service** (`DocSvr.exe`) and an **ASP.NET web admin**, over an
   **ArcSDE 10.2.1 versioned geodatabase** on **SQL Server 2012 RTM**. Installed 2016, heavily
   customised for PLAGIS by post-install packages 4.0.46 → 4.0.97 and a *Title Deed Plan
   Workflow* package. [V-PROD]
2. **How it is used.** Staff RDP into the production server and run ArcMap there (RDS
   session host). The visible output is **one ArcMap document per parcel/plan**
   (`PL<n>.mxd`, `PSG<n>.mxd`, `BP<n>.mxd`; ~100 found, newest October 2026). [V-PROD]
3. **The platform is end-of-life on every layer.** ArcGIS 10.2.1 (2014), SQL Server 2012
   with **no service pack**, Windows Server 2012 R2 (out of support), .NET Remoting, DevExpress
   10.2, Silverlight in GISViewer. [V-PROD]
4. **It is failing now.** The editor's Aumentum licence repeatedly fails validation
   ("time limit has expired… machine has not been activated", 28 and 31 Oct 2026). [V-PROD]
   Staging's application server will not start at all (its DB host 10.10.10.3 is
   unreachable). [V-STG]
5. **Data scale is modest.** 21,266 current parcels in the DEFAULT version; 164 control
   coordinates; ~15k states in the DEFAULT lineage. [V-SQL] Migration is mostly a
   *correctness* problem (versioned-geodatabase reconstruction), not a volume problem.
6. **The replacement is feasible on the same server**, with constraints: Windows 2012 R2
   build target, a **daily scheduled shutdown**, many co-hosted roles, and ports already
   taken (§9). The repository already implements the survey engine, REST API, workflow,
   jobs, locking, audit chain, PDF reports, the importer (run end-to-end on real Aumentum data)
   and a browser map viewer + editor v1 (§11); the **Tauri desktop shell, plan-layout parity,
   Windows packaging, AD-backed identity and ArcGIS hand-back** remain.

---

## 2. Environment and hosts

| Host | Role (observed) | Evidence |
|------|-----------------|----------|
| **10.10.10.6 `PRODUCTIONMC`** | Production: Aumentum server + ArcGIS Server + ArcSDE + SQL `PRODUCTIONMC\PRODUCTIONMC` + RDS session host for editors. Also AD Certificate Services, Hyper-V, Server Essentials (backup, folder redirection), file shares, IIS, DFS. Dell R910, 64 logical CPUs, 128 GB RAM; C: 129 GB free / 909, D: 2.5 TB free / 3.5 TB, E: 110 GB free. | [V-PROD] |
| **10.10.10.4 `MC`** | Staging/old: same Aumentum 4.0.97 + ArcGIS 10.2.1 stack (services stopped), local SQL `MC\MC` with copies of `sde` and `MCServer`, PostgreSQL 9.2 (Liferay 6.2 portal) and PostgreSQL 14 + PostGIS 3.2.2 + EDB Migration Toolkit (installed Aug 2022 — an earlier migration attempt), WSUS, DNS. Dell R720, 24 CPUs, 32 GB. | [V-STG] |
| 10.10.10.3 | Configured DB server for staging's Aumentum (`DBServer=10.10.10.3`, DB `MCServer`); **unreachable** from staging (service start fails). | [V-STG] |
| 10.10.10.5 | Hosts **`LRS43`** (Aumentum Registry / title data). Not inspected. | [V-SQL] note, [GAP] |
| 10.10.10.2 | Domain controller / DNS for `Plagis.com` (production holds LDAP :389 and RPC sessions to it). | [V-PROD] |
| 10.10.10.9, .128, .211 | Workstations with live RDP sessions into production during the run. | [V-PROD] |
| 10.10.10.128:8080 | Production makes HTTP calls to a service here (purpose unknown). | [V-PROD], [GAP] |

OS on both: Windows Server 2012 R2 Standard (6.3.9600), PowerShell 4.0. Production .NET 4.5.1
(release 378675); staging .NET 4.6.2 (460805). [V-PROD][V-STG]

---

## 3. Existing system architecture

```
  Workstations (10.10.10.x) ── RDP ──►  PRODUCTIONMC (10.10.10.6)
                                        ┌──────────────────────────────────────────────────┐
                                        │ RDS session per user                              │
                                        │  ArcMap 10.2.1 (32-bit)                           │
                                        │   └─ AUMENTUM CADASTRE Editor Extension 4.0.97    │
                                        │       Ils.MultiCadastre.*.dll, LRS.*, ServiceAgent│
                                        │       │  .NET Remoting tcp://PRODUCTIONMC:8091     │
                                        │       │  (binary formatter, SSPI "negotiate")      │
                                        │       │                   │ direct-connect / SDE   │
                                        │       ▼                   ▼                        │
                                        │  ILS MultiCadastre Server   ArcSDE giomgr :5151    │
                                        │  DocSvr.exe (x64, LocalSystem)  │                  │
                                        │  :8090 :8091 :8099              │                  │
                                        │  workflow / audit / calendar /  │                  │
                                        │  integration / sync / OCR /     │                  │
                                        │  document store Server\data\YYYY│                  │
                                        │       │ SQL auth user "docsvr"  │                  │
                                        │       ▼                         ▼                  │
                                        │  SQL Server 2012 RTM  PRODUCTIONMC\PRODUCTIONMC    │
                                        │   ├─ MCServer   (app DB: jobs, users, settings…)   │
                                        │   ├─ sde        (ArcSDE versioned geodatabase)     │
                                        │   └─ chartingMinna (purpose unknown)               │
                                        │                                                    │
                                        │  IIS: /ApplicationServerAdmin (WebAdmin, .NET 4,   │
                                        │       Forms auth → DocSvr :8091)                   │
                                        │       /GISViewer (Aumentum Registry GIS viewer,    │
                                        │       proxies ArcGIS Server; LRS auth server)      │
                                        │  ArcGIS Server 10.2.1 :6080  service Aumentum/Parcels│
                                        │  ArcGIS License Manager 10.2.1 (FlexNet :27000)    │
                                        │  Sentinel HASP (hasplms :1947) — Aumentum licensing│
                                        └──────────────────────────────────────────────────┘
                                                  │ LRS web services (registry)
                                                  ▼
                                        10.10.10.5  LRS43 (Aumentum Registry)   [GAP]
```

Evidence: services, ports, processes, registry and configs in the production reports [V-PROD];
LRS link from `Ils.MultiCadastre.Library.LRSWebServicesProxy.dll`, `LRS.*` assemblies and the
GISViewer setup step `lrs_auth_server_config` [V-PROD]; the 10.10.10.5 location of LRS43 is from
the 2026-10-30 inspection [V-SQL note].

---

## 4. Components in detail

### 4.1 Desktop editor — ArcMap extension [V-PROD]
- Install dir `C:\Program Files (x86)\AUMENTUM\CADASTRE\Desktop\`, registry
  `HKLM\SOFTWARE\Wow6432Node\Aumentum\CADASTRE\Desktop`: `ServerAddress=PRODUCTIONMC`,
  `ServerPort=8091`, `Application=MultiCadastre`, `ForceLogIn=1`, `InstallType=Extension`.
- Core assemblies (4.0.97.0, Thomson Reuters): `Ils.MultiCadastre.Desktop`, `.Extension`,
  `MultiCadastreEditor`, `MultiCadastreMap`, `.Catalog` ("MultiCadastre Manager"),
  `.ParcelAttributeManagement`, `.ParcelLineAge` (parcel lineage), `.UI.Controls.Wizard`,
  `.Utilities.ProductivityLog`, `Grm.Cadastre.RegistryExtension` ("Aumentum Registry
  Extension for ArcGIS").
- Registry (LRS) client stack 4.3.1.10 (Manatron): `LRS.Core/DataModel/Storage/Signature`,
  `ServiceAgent.*`, `ServiceProxy`, `WAssess` (assessment), `WDelivery`, `WScanning`,
  `ILS.Scanning`.
- Supporting: `WorkFlowService`, `AuditService`, `CalendarService`, `IntegrationService`,
  `SecLite` (remoting security sink), `Aumentum.Cadastre.Remote(.Client)` (HP-authored),
  `MapReplication`, `ThumbnailViewer`, DevExpress 10.2 UI, Microsoft ReportViewer, log4net.
- Configuration files present (contents **not yet captured**, [GAP]): `ParcelAttributeConfig.xml`,
  `ParcelAttributeTableDefinition.xml`, `ParcelMutationConfig.xml`, `ParcelLineAge.xml`,
  `job.xml`, `MapTemplateConfig.xml`, `LanguageConfig.xml`, `Remoting.Client.config`,
  `Ils.MultiCadastre.Desktop.dll.config/.Setting`.
- Licensing: DeployLX (`Ils.MultiCadastre.Editor.lic`, `.Map.lic`) + Sentinel HASP. **Failing
  validation** in the October 2026 logs.
- UI languages shipped: en-US, es, ar-EG, prs-AF (Dari). PLAGIS uses en-US.

### 4.2 Application server — `ILS MultiCadastre Server` (`DocSvr.exe`) [V-PROD]
- Windows service, Auto start, **runs as LocalSystem**, x64. Listens on **8090, 8091, 8099**;
  8091 is the .NET Remoting endpoint `tcp://…:8091/DocumentServer.rem` (from WebAdmin config).
- Registry `…\CADASTRE\Server`: `DBServer=PRODUCTIONMC\PRODUCTIONMC`, `DBName=MCServer`,
  `DBEngine=SqlServer`, `MCServerMode=ServerExt`, `SDEConnectionString` empty.
- Connects with **SQL authentication, user `docsvr`** (`Persist Security Info=True`,
  MARS on, pool min 5). [V-PROD log]
- Startup sequence from log: survey document type `sd_titledeedPlan` → DB handlers →
  `Version 4.0.97.1` → *Synchronization service host* → *File Synchronization service host* →
  *Workflow service initialized*. Housekeeping every 5 min: `select … from [deleted_file]`,
  settings from `[dsssettings]`.
- Assemblies: `DocServ`, `dssda`, `DSDBI`, `BlobMnt`/`FSMnt`/`ISMnt` (blob/file/image storage
  mounts), `OCRIntf`, `RAIntf`, `StDesign`, `UsManager` (users), `Loader`, `SyncService`,
  `GuardSink`, plus **ArcObjects 10.2.2** (`ESRI.ArcGIS.Geodatabase`, `.Version`, `.Carto`,
  `.Server`, `.GeoDatabaseDistributed`…) — i.e. the server manipulates geodatabase versions
  directly.
- Document store: `Server\data\2016 … 2026` (per-year folders) [V-PROD]; volumes/types [GAP].
- `DisconnectedSync\OffLineSync.mdf` — an offline/disconnected editing sync DB. [V-PROD]
- `Email\EmailTemplate.xml` — e-mail notifications. [V-PROD] (content [GAP])
- Batch: `SDE_Postinstallation.bat`, `GRMCadastreEnv.ps1`, `ntrights.exe`. [V-PROD]

### 4.3 Web Administrator — `/ApplicationServerAdmin` [V-PROD]
- `MCAdminWebSite.dll` 4.0.97, ASP.NET WebForms + DevExpress, app pool
  `ApplicationServerAdminAppPool` (.NET 4, **Classic** pipeline, NetworkService).
- Forms auth (`Login.aspx`, 40-min ticket, 15-min session), `maximumLoginAttempts=3`, talks to
  DocSvr over remoting (`DSSUri=tcp://localhost:8091/DocumentServer.rem`, SSPI negotiate).
- `compilation debug="true"` and `validateRequest="false"`, `requestValidationMode="2.0"`,
  `maxRequestLength=2147483647` — see §8.
- Recent use: failed logons on 2026-10-29 (`Logon failure: unknown user name or bad password`)
  → it authenticates against **Windows/AD accounts**. [V-PROD log]
- Workflow/user API surface (WCF message types registered on the box): `CreateJob`,
  `CreateJobInstance`, `CreateTask`, `EndTaskInstanceWithComment`, `GetActiveJobInstances`,
  `GetJobInstancesForCurrentUser`, `GetJobPermissions`, `AddGroup/AddMember/AddUser*`,
  `ChangePassword`, `DisablePassword`, `GetAllRoles`, `GetEffectiveRoles`… [V-PROD]
  → **jobs → job instances → tasks**, with users/groups/roles and per-job permissions.

### 4.4 GISViewer — `/GISViewer` [V-PROD]
- *Aumentum Registry GISViewer 1.3.3.0 for ArcGIS 10.2* (`ILS.MCPortal.*`), app pool
  NetworkService, Forms auth with a custom `IdentityMembershipProvider`.
- Proxies ArcGIS Server through `gis.gate`/`gis.gate2` handlers; setup steps: LRS auth server,
  SDE connection, layers, service URLs. Relies on **Silverlight** (`silverlightUrl`) [V-PROD] —
  unusable in any modern browser [A: assumed still Silverlight-dependent at runtime].

### 4.5 ArcGIS platform [V-PROD]
- ArcGIS Desktop 10.2.1 (installed 2024-07 re-install), ArcGIS Server 10.2.1 (site created
  2016-09-07, service account `.\arcgis`), ArcSDE 10.2.1 (`giomgr` :5151, application-server
  connections still running), License Manager 10.2.1 (`@PRODUCTIONMC`, floating *Editor* seats).
- ArcGIS Server services: one business service **`Aumentum/Parcels` (MapServer)** —
  dynamic (not cached), 1–2 instances, maxRecordCount 1000, published from
  `E:\MapServices\Parcels.mxd` (Dec 2024). Others are the defaults (SampleWorldCities, System,
  Utilities/Geometry, PrintingTools, Search).
- 7 `ArcSOC.exe` processes; ArcGIS Server listens on :6080 (and :6006/:4000-4004 internally).

### 4.6 Data — SQL Server `PRODUCTIONMC\PRODUCTIONMC`
- SQL Server **2012 RTM 11.0.2100.60** (no SP), instance binaries on D:, Analysis Services
  (`msmdsrv`) also running, SQL Agent **stopped** (so no SQL Agent jobs/backups run). [V-PROD]
- Databases seen: `sde`, `MCServer`, `chartingMinna`, system DBs. [V-PROD event log]
- `sde` (121 GB): ArcSDE **multiversioned** geodatabase; geometry in native SQL `geometry`,
  **SRID 32632 (UTM 32N)**; DEFAULT state 1,057,484; DEFAULT lineage 14,993 states. [V-SQL]
- Feature classes (reg_id): `PLATEAU_STATE_PARCELS` (11, 21,266 current / 286 base rows),
  `PLATEAU_STATE_POINTS` (9), `PLATEAU_STATE_LINES` (10), `PLATEAU_STATE_CTRLPNTS` (8),
  `PLATEAU_STATE_JOBS_*` (16–22, per-job edit layers), `PLAGIS_COORDINATES` (155, not
  versioned, 164 rows). [V-SQL]
- Parcel attributes: `UPIN` (`PL`+12 digits), `Status` (1 pending, 2 active, 3 historic,
  4 archived — confirmed with PLAGIS [V-CODE `cadastre-import/src/status.rs`]), `JobID`,
  `PlanNo`, `CadZone`, `Layout`, `PlotNo`, `BlockNo`, `propertyNo`, `CreateDate`,
  `LastEditDate`, `ModificationType`. [V-SQL]
- `MCServer`: application DB. Tables known from logs: `deleted_file`, `dsssettings` [V-PROD];
  full schema [GAP].

---

## 5. Workflows (as observed)

| # | Workflow | What the evidence shows | Tag |
|---|----------|--------------------------|-----|
| W1 | **Log in** | Users RDP to PRODUCTIONMC, open ArcMap; the extension forces login (`ForceLogIn=1`) against DocSvr using Windows credentials (SSPI). | V-PROD |
| W2 | **Job creation & assignment** | Workflow service with jobs, job instances, tasks, per-user queues and job permissions; editing happens in per-job geodatabase versions (`Plateau_State_JobID_*`) and per-job layers (`PLATEAU_STATE_JOBS_*`). | V-PROD, V-SQL |
| W3 | **Parcel capture / mutation** | Configured by `ParcelMutationConfig.xml`, `ParcelAttributeConfig.xml`; lineage tracked by `ParcelLineAge`. Mutation types (split/merge/new/…) [A] — config not captured. | V-PROD (files), GAP (content) |
| W4 | **Survey/COGO entry** | Done in ArcMap; Trimble Business Center 3.11, Geographic Calculator 6.3 and TransLT are installed alongside for survey data and datum work. | V-PROD |
| W5 | **Title Deed Plan** | Survey doc type `sd_titledeedPlan`; PLAGIS "Title Deed Plan Workflow" package; templates `Plan_.mxd`, `PSG_Plan_.mxd`, `reserve\Plan.mxd`; staff save **one MXD per parcel** (`PL<n>`, `PSG<n>`, `BP<n>`) and print/PDF (PDF24 running). Largely manual layout work. | V-PROD |
| W6 | **Posting** | Job versions reconciled/posted to DEFAULT (ArcObjects `ESRI.ArcGIS.Version` in DocSvr). Parcel `Status` moves pending → active; replaced parcels → historic. | V-SQL, V-PROD, A (exact trigger) |
| W7 | **Documents** | Scans/attachments stored under `Server\data\<year>`, deletions queued in `deleted_file`, OCR interface present. | V-PROD |
| W8 | **Registry link** | Cadastre ↔ LRS43 registry through LRS web services and the Registry extension (title/ownership). | V-PROD (binaries), GAP (data flow) |
| W9 | **Publishing** | `Aumentum/Parcels` map service on ArcGIS Server consumed by GISViewer and others (an external system at 10.10.10.128 calls out/in over :8080 [A]). | V-PROD |
| W10 | **Administration** | WebAdmin: users, groups, roles, workflow definitions, settings, productivity reports (disabled: `viewProductivityReports=false`). | V-PROD |

---

## 6. Database interactions (summary)

- **Editor → sde**: ArcObjects through ArcSDE (`giomgr` :5151 application server and/or direct
  connect) — edits in job versions. [V-PROD]
- **DocSvr → MCServer**: ADO.NET, SQL auth `docsvr`, polling every 5 min. [V-PROD]
- **DocSvr → sde**: ArcObjects 10.2.2 (version management/posting). [V-PROD]
- **ArcGIS Server → sde**: map service `Aumentum/Parcels` reads parcels. [V-PROD]
- **Registry**: LRS web services (→ LRS43 on 10.10.10.5). [GAP]
- The new importer reads `sde` read-only and reconstructs DEFAULT with the SQL in
  `docs/aumentum-import.md` — **no Esri libraries needed**. [V-CODE]

---

## 7. Dependencies and versions (production)

| Layer | Component | Version | Support status |
|-------|-----------|---------|----------------|
| OS | Windows Server 2012 R2 | 6.3.9600 | Out of support (Oct 2023) |
| DB | SQL Server 2012 | 11.0.2100.60 **RTM** | Out of support (Jul 2022), unpatched |
| GIS | ArcGIS Desktop / Server / ArcSDE | 10.2.1 | Retired by Esri |
| App | Aumentum Cadastre | 4.0.97 (2016) | Vendor licence failing |
| Runtime | .NET Framework | 4.5.1 | Out of support |
| UI libs | DevExpress 10.2, MS ReportViewer 2008/2012, Silverlight 5 | — | Obsolete |
| Comms | .NET Remoting (binary formatter) | — | Deprecated; insecure by design |
| Licensing | DeployLX 4, Sentinel HASP, FlexNet | — | — |
| Remote access | TeamViewer 12, AnyDesk 7, RDS | — | Third-party remote tools on a server |

---

## 8. Security assessment

| Sev | Finding | Tag |
|-----|---------|-----|
| **High** | SQL Server 2012 **RTM, no service packs** — years of unpatched CVEs on the system of record. | V-PROD |
| **High** | **.NET Remoting with BinaryFormatter** on :8091 (known remote-code-execution class); DocSvr runs as **LocalSystem**. | V-PROD |
| **High** | Production server is also the **Certificate Authority**, Hyper-V host, backup server and RDS host — a compromise of the GIS stack exposes the domain PKI. | V-PROD |
| **High** | **TeamViewer and AnyDesk** services running on production (unmanaged inbound remote access). | V-PROD |
| Med | App DB login uses **SQL authentication** (`docsvr`) with `Persist Security Info=True`. | V-PROD |
| Med | RDP offers only **legacy TLS** (modern OpenSSL refuses it at default security level). | V-PROD (observed when connecting) |
| Med | **SMB1** enabled (both hosts). | V-PROD, V-STG |
| Med | WebAdmin: `compilation debug="true"`, request validation disabled, 2 GB request limit. | V-PROD |
| Med | WinRM :5985 (HTTP) listening. | V-PROD |
| Med | SQL Agent stopped and no evidence of SQL backups in this pass (Windows Server Essentials backup *is* running; VSS shadow copies weekly). | V-PROD, GAP (msdb backup history) |
| Low | Old browsers (Chrome 98, Firefox 49 on staging), Java 7 on staging. | V-PROD, V-STG |
| Low | Shared admin credential used for this assessment; rotate after the work (owner aware). | — |

---

## 9. Deployment constraints on PRODUCTIONMC (for the replacement)

1. **OS = Windows Server 2012 R2.** Rust std needs the tier-3 `x86_64-win7-windows-msvc`
   target (nightly + build-std); the editor's WebView2 must be the **fixed-version** runtime
   (last 2012 R2-compatible build). See `docs/windows-2012r2-build.md`. [V-CODE]
2. **Daily `AutoShutdown` scheduled task** (`shutdown.exe`, daily, runs as PLAGIS\Administrator;
   last run 2026-10-30 15:50). All new services must be Auto-start Windows services, tolerate
   cold start, and hold **no in-memory-only state**; edit sessions/locks already persist with a
   5-min stale TTL. [V-PROD], [V-CODE]
3. **Ports in use** on production: 80, 443, 1098, 1947, 2179, 2382, 3389, 4000–4004, 5151, 5938,
   5985, 6080, 6099, 6600, 6602, 7070, 8090, 8091, 8099, 8912, 27000, 65500, 65520 (+ dynamic).
   **Free and proposed:** `cadastre-server` **:8080**, PostgreSQL **:5432**, identity
   provider **:8081**. [V-PROD]
4. **Database engine.** The new server is PostGIS-based. **PostgreSQL 14 + PostGIS 3.2.2 is
   proven to run on Windows Server 2012 R2** (it is running on staging). [V-STG] Newer
   PostgreSQL Windows builds are not supported on 2012 R2 [A]. → Pin production to
   **PostgreSQL 14 / PostGIS 3.2–3.3**, and align `docker-compose.yml` (currently
   `postgis/postgis:16-3.4`) to 14 so dev matches prod.
5. **Disk:** install data on **D:** (2.5 TB free); C: has 129 GB free.
6. **Coexistence.** Aumentum, ArcGIS Server and SQL Server stay running during transition;
   the new stack must not take their ports or service accounts.
7. **Identity.** Users are AD accounts in `Plagis.com` (DC 10.10.10.2). The server verifies
   OIDC/JWKS tokens today (Keycloak). Keycloak on 2012 R2 needs a supported JDK [A — confirm
   JDK 17/21 on 2012 R2]; LDAP federation to AD keeps one set of credentials.

---

## 10. Replacement scope

### 10.1 Goals
- Replace Aumentum Cadastre Editor 4.0.97 for **parcel capture, survey computation,
  job workflow, posting, plan production and audit**, on the same server, with no vendor
  licence dependency.
- Make PostGIS the **system of record** for the cadastral fabric after cut-over, with a
  verified one-time import (and re-runnable delta import during parallel running) from `sde`.
- Keep ArcGIS consumers working during transition (§10.5).

### 10.2 Out of scope (Phase 1)
- Replacing the **Aumentum Registry / LRS43** (titles, ownership, assessment). Integrate, don't
  replace. (Registry / R of O / C of O are later phases per the SRS.) [V-CODE README]
- Replacing ArcGIS Server for third-party consumers (keep publishing; see §10.5).
- OCR/scanning; offline/disconnected sync (`OffLineSync.mdf`) — confirm whether used (§14).

### 10.3 Feature parity matrix

Status: ✅ implemented & tested in repo · 🟡 partial · ⬜ not started.

| Aumentum capability | New system | Status |
|---------------------|-----------|--------|
| COGO, traverse closure, area, precision (live) | `cadastre-core::survey`, `cogo`, `area`; `POST /survey/check` | ✅ |
| Least-squares / fabric adjustment | `cadastre-core::adjust`; `POST /fabric/adjust` | ✅ |
| Topology validation | `cadastre-core::topology`; `POST /fabric/validate` | ✅ |
| CRS handling (UTM 32N live; Minna) | `cadastre-core::crs` incl. EPSG:32632; live tables stored in **32632** (migration 0004) | ✅ |
| Jobs, configurable workflow, role-gated transitions | `/jobs`, `/workflow` (stages/transitions), `job_event` | ✅ |
| Job instances → tasks, per-user queues | jobs with `assigned_to` + stages | 🟡 (no task layer; confirm need from `job.xml`) |
| Concurrent editing control (versions) | edit sessions + resource locks, stale reclaim | ✅ (lock-based, replaces versioning) |
| Parcel capture under a job, posting, UPIN numbering `PL`+12 | `/jobs/{id}/parcels`, `/jobs/{id}/post`, `parcel_number_seq` | ✅ |
| Parcel lineage (parent/child on mutation) | — | ⬜ |
| Parcel attribute config / mutation types | — | ⬜ (needs `ParcelAttributeConfig.xml`, `ParcelMutationConfig.xml`) |
| Title Deed Plan / site plan output | `/jobs/{id}/siteplan.pdf`, `/jobs/{id}/report.pdf` | 🟡 (layout parity with `Plan_.mxd`/`PSG_Plan_.mxd` templates needed) |
| Audit trail | hash-chained `audit_log`, `/audit`, `/audit/verify` | ✅ |
| Users / roles / groups | Keycloak (OIDC) + role claims | 🟡 (AD federation, admin UI) |
| Web administration | OpenAPI/Swagger; admin endpoints | 🟡 (no admin UI) |
| Editor (map + layers + traverse entry + job strip) | browser editor v1 at `/editor` (MapLibre, served by the server); Tauri shell `apps/editor` | 🟡 (browser v1 done; Tauri desktop shell not started) |
| Map display / vector tiles | browser viewer `/viewer` + `parcels.geojson`/`beacons.geojson` (WGS84); Martin service in compose | 🟡 (viewer done; Martin config only) |
| Document attachments | — | ⬜ |
| E-mail notifications | — | ⬜ |
| Registry (LRS43) integration | — | ⬜ (needs LRS inspection) |
| Import from Aumentum `sde` | `cadastre-import` (reconstruction SQL, WKT, status, exceptions, file source, PostGIS staging sink) + `POST /import/promote/{run}` | ✅ (real import run: 18,887 parcels, 169 beacons) |
| ArcGIS publishing of parcels | — | ⬜ (§10.5) |
| Windows 2012 R2 packaging & services | build doc | ⬜ |

### 10.4 Target architecture on PRODUCTIONMC

```
 Workstations ──HTTPS──► cadastre-server (:8080, Windows service, D:\Cadastre)
   │                        ├─ PostgreSQL 14 + PostGIS (:5432, Windows service, D:\pgdata)
   │                        ├─ identity: Keycloak (:8081) federated to AD Plagis.com
   │                        └─ tiles: Martin (vector tiles from PostGIS)
   └─ Cadastre Editor (.exe, Tauri, fixed WebView2) — runs on workstations or in RDS sessions
 cadastre-import ──read-only──► SQL Server sde (DEFAULT reconstruction)
 parcel publish  ──write──────► SQL Server table/view for ArcGIS (§10.5)
```

### 10.5 ArcGIS integration strategy
- **Read path (migration):** `cadastre-import` reads `sde` with plain T-SQL (no ArcObjects).
  [V-CODE]
- **Write-back for map consumers (transition):** ArcGIS Server 10.2.1 can only consume
  PostgreSQL versions it certifies (10.2.x era) [A]; PostgreSQL 14 is not among them [A]. So,
  rather than pointing ArcGIS at PostGIS, publish posted parcels from PostGIS into a
  **non-versioned SQL Server table** (native `geometry`, SRID 32632) that a re-published
  `Parcels` map service reads (query layer). Owned by a scheduled `cadastre-publish` job.
- **Longer term:** serve OGC/vector tiles from the new stack and retire ArcGIS Server once
  consumers (GISViewer, 10.10.10.128) are moved.

### 10.6 Data migration
1. Restore a **copy** of production `sde` (and `MCServer`) on staging 10.10.10.4 (it already
   hosts SQL `MC\MC`). Never run the importer against production.
2. Run `cadastre-import --features live` → PostGIS staging tables → validation (topology,
   closure, duplicate UPIN, empty placeholders — 50 known) → **exceptions report**.
3. Reconcile counts: 21,266 parcels (21,213 Polygon + 3 MultiPolygon + 50 empty), 164 control
   points. [V-SQL]
4. Decide treatment of **unposted job versions** (in-flight work) — migrate as open jobs or
   finish in Aumentum first.
5. Parallel run: Aumentum stays master; nightly delta imports; compare. Cut-over: freeze
   Aumentum edits, final import, switch master, start publish-back.

---

## 11. State of the new system (this repository) [V-CODE]

| Crate / app | Contents | Status |
|-------------|----------|--------|
| `cadastre-core` | COGO, survey session, area, closure, adjustment, topology, CRS (EPSG:26392 + 32632), units/DMS; native + wasm32 | built, tested |
| `cadastre-server` | axum + sqlx/PostGIS; tolerances, beacons, parcels (+search, GeoJSON), fabric validate/adjust, workflow, jobs (+events, post, parcels), sessions/locks, survey check, audit chain + verify, job report & site plan PDF, import promotion; RS256/JWKS auth; OpenAPI; embedded browser **viewer** (`/viewer`) and **editor v1** (`/editor`); migrations 0001–0004 | built, tested (see build/test record below) |
| `cadastre-import` | ArcSDE DEFAULT reconstruction SQL builder, row mapping, WKT/WKB geometry, status mapping, exceptions, file source + PostGIS staging sink; live tiberius reader behind `live` feature | tested; run end-to-end on real Aumentum data (18,887 parcels, 169 beacons) |
| `apps/editor` | Tauri 2 + Leptos + MapLibre — architecture + scaffold | not buildable yet (browser editor v1 is the runnable path) |

### 11.1 Build & test record (2026-10-31)

Built and tested in a `rust:1-bookworm` container (Rust 1.99.1) against PostGIS from
`docker compose up -d db`. [V-CODE]

| Check | Result |
|-------|--------|
| `cargo build --workspace --all-targets` | ✅ |
| `cargo test --workspace` (incl. DB integration tests) | ✅ 77 passed, 0 failed (after merging origin/main) |
| `cargo build -p cadastre-core --target wasm32-unknown-unknown` | ✅ |
| `cargo build -p cadastre-import --features live` | ✅ |
| `cargo clippy --workspace --all-targets` | ✅ clean |
| `tools/smoke-test.sh` against a running server | ✅ 24/24 — survey check, job → submit → review → approve → post (UPIN `PL…`), locks (409 on contention), search, PDFs, audit chain verify |

**Defect found and fixed during testing:** `POST /jobs/{id}/post` posted jobs in **any** stage, so
a draft job could consume an official UPIN without review. Posting now requires stage
`approved` (`domain::can_post`; otherwise **409**), with a unit test and an integration
regression test (`posting_requires_an_approved_job`). A pre-existing unit test
(`job_report_is_valid_pdf_with_parcel_numbers`) asserted the unescaped `(unposted)` text and
was corrected to expect the PDF-escaped form.

After merging origin/main (browser viewer/editor, import sink/promotion, CRS 32632), the
teammate's posting test and the browser editor were updated to the approval rule: the editor's
**Post job** button now appears only for `approved` jobs.

Not yet verified: the Windows 2012 R2 (`x86_64-win7-windows-msvc`) build, auth-enabled mode
against a real Keycloak, and browser-editor interactivity (not headlessly tested).

**Storage CRS — resolved.** Migration 0004 moved live storage from EPSG:26392 to **EPSG:32632**,
the CRS of the Aumentum fabric. Derive Minna on output only where a workflow requires it.

**Count to reconcile:** the import promoted 18,887 parcels; the DEFAULT version holds 21,266
(18,899 with `Status=2` active). Only active parcels are promoted, so the ~12-parcel difference
from 18,899 should be explained in the import exceptions report before cut-over.

---

## 12. Implementation roadmap

| Phase | Deliverable | Depends on | Status |
|-------|-------------|------------|--------|
| 0 | WebView2 fixed-runtime spike on a real 2012 R2 VM (use staging 10.10.10.4) | — | ⬜ **gate** |
| 1–3 | Core engine; server skeleton; jobs/workflow/sessions/auth | — | ✅ |
| 4 | Editor: browser v1 (map, layers, survey entry, save draft, job workflow) ✅; Tauri shell ⬜ | Phase 0 (Tauri only) | 🟡 |
| 5 | Parcel mutation types, attribute schema config, **lineage** | `ParcelMutationConfig.xml`, `ParcelAttributeConfig.xml` (pass 3) | ⬜ |
| 6 | Posting, UPIN numbering, audit chain, PDF reports | — | ✅ (plan layout parity ⬜) |
| 7 | Web viewer ✅; vector tiles (Martin) config only | Phase 6 | 🟡 |
| 8 | Import: real run + promotion ✅; reconciliation of counts, unposted job versions ⬜ | — | 🟡 |
| 9 | Windows packaging: win7-target builds, services, TLS (rustls), PG14 install, Keycloak+AD | Phase 0 | ⬜ |
| 10 | ArcGIS publish-back, LRS43 integration, documents, notifications | LRS inspection | ⬜ |
| 11 | Parallel run on production, cut-over, Aumentum decommission | all | ⬜ |

---

## 13. Risks

| Risk | Impact | Mitigation |
|------|--------|-----------|
| WebView2 fixed runtime does not run Tauri 2 on 2012 R2 | Editor GUI blocked | Phase 0 spike; fallback ladder in `windows-2012r2-build.md` (Tauri v1 / older Wry / browser client served by the server) |
| Aumentum licence lapses before cut-over | Editing stops | Prioritise import + editor; keep read-only ArcMap for viewing; engage vendor for a bridge licence |
| Daily auto-shutdown during long imports/jobs | Partial runs | Import is idempotent/re-runnable [V-CODE]; schedule around it or ask PLAGIS why it exists |
| Hidden business rules in Aumentum configs/MCServer | Parity gaps | Pass 3 capture; user-guide review; workshop with PLAGIS editors |
| Co-hosting on an overloaded, multi-role server | Instability/security | Dedicated service accounts, D: install, firewall rules; recommend moving CA/remote tools off |
| CRS ambiguity (Minna vs UTM 32N) | Wrong coordinates on plans | Decide storage CRS (§11); golden-file tests on known plans |

---

## 14. Gaps and open questions

| # | Gap / question | How to close |
|---|----------------|--------------|
| G1 | `MCServer` schema, row counts, lookup tables | Run `tools/aumentum-inventory/RUN-ME-3.cmd` on production |
| G2 | Desktop/Server config contents (attribute, mutation, job, map template, server config) | Pass 3 |
| G3 | Aumentum PDF guides (editor/map user guide, admin guides) | Pass 3 copies them to `tools/aumentum-inventory/aumentum-docs/` |
| G4 | SQL backup history (msdb) | Pass 3 |
| G5 | LRS43 on 10.10.10.5: schema and the UPIN ↔ title link | Same inventory kit, run on 10.10.10.5 |
| G6 | What is 10.10.10.128:8080, and who consumes `Aumentum/Parcels`? | Ask PLAGIS IT; ArcGIS Server logs |
| G7 | Purpose of `chartingMinna` DB | Pass 3 lists it; ask PLAGIS |
| G8 | Why the daily `AutoShutdown` exists; maintenance window | Ask PLAGIS IT |
| G9 | Are offline/disconnected sync and OCR used? | Ask editors; check `OffLineSync.mdf` size/date |
| G10 | Status of unposted job versions | Query `sde.SDE_versions` (pass 3) |
| G11 | ~~Storage CRS decision~~ — resolved: 32632 (migration 0004) | — |
| G12 | What is 10.10.10.3 (staging's configured DB host) — retired? | Ask PLAGIS IT |
