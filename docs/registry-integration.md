# Cadastre ↔ Registry integration (Phase 5)

This document describes how the new Cadastre works with the Aumentum Registry (LRS43 on
**10.10.10.5**) as Aumentum Cadastre (**10.10.10.6**) did. It records how the old link worked,
what this Cadastre provides in its place, how to configure it, how to switch the Registry over,
and what is still open.

Status (2026-11-02): **in development, running on the dev server**. Every part below is built and
tested. The Registry has not been switched over yet: it still calls Aumentum (see
[Cut-over](#cut-over-switching-the-registry-to-this-cadastre)). Packaging as a Windows install for
the server and desktops comes later (see `windows-2012r2-build.md`).

---

## 1. The flow

| # | Who | Where | What happens |
|---|---|---|---|
| 1 | Applicant, Registry staff | Registry | Basics and payment. The Registry creates the **transaction** (PLS…), whose property carries the **file number** (PL…, e.g. PL68955). |
| 2 | Registry (automatic) | Registry → Cadastre | When the transaction reaches a configured stage, the Registry **creates the Cadastre job** with the transaction number (see §3.2). |
| 3 | Digitising unit | Cadastre: **Registry → Find file** | Searches by PL or PLS number, then **Start digitising** (or **Open job** when one exists). Works the job: Plot Parcel (coordinates, construction) → Review → **Commit**, which gives the parcel its **UPIN**. |
| 4 | Registry admin | **Registry** (never the Cadastre) | **Links** the Registry property to the parcel (UPIN) on the Cadastre map shown inside the Registry. |
| 5 | Cadastre (automatic) | Cadastre | Picks up the link **within a minute**: the parcel takes its link fields, and the file appears under **Registry → Linked**. |
| 6 | Title deed unit | Cadastre map | **Right-click or double-click** the parcel → **Start edit — title deed**. This opens the parcel's job, starts work on it, and opens the title deed tools (template, print). |
| 7 | Title deed unit | Cadastre: **Registry → Send plan** | Sends the finished plan back to the transaction, filed as a *Title Deed Plan* ("Survey Plan"). This updates the Registry file. |

Not yet (later): RBAC, with an admin assigning linked jobs to staff, and separate dashboards for
digitising and title deed. Today one admin view runs the whole flow.

---

## 2. How Aumentum did it (the evidence)

Everything here was read from Aumentum's own code and data, never guessed:

| Source | What it showed |
|---|---|
| `Grm.Cadastre.RegistryExtension.dll`, `IntegrationService.dll` (Cadastre desktop / MCServer, decompiled) | The Cadastre's Registry calls: one Registry account held by MCServer (`dsssettings` Integration/RegParam: 10.10.10.5:8080, user `admin`, customer code THOMSONREUTERS). The calls are `AuthenticationService.startSession` → ticket in a WS-Security UsernameToken, HQL reads with `DomainModelService.loadObjects` (queries in MCServer `LRSIntegrationQueryDefinition`), writes with `IntegrationService.SaveEntity`, and plan upload with `PUT /lrs/upload/…` + an `IncomingDocument` of type `sd_titledeedPlan`. The extension also shows the link status rule: Withdrawn, Postponed, Completed and Rejected transactions are refused. |
| `lrs-mc-integration.jar` (Registry, decompiled with CFR) | The Registry's calls into the Cadastre. `mcServiceAddress` = `http://10.10.10.6/ApplicationServerAdmin/WebServices/LRSWebServices.asmx`, signed in with `multicadastre` settings `mcUserName` (`administrator`) / `mcPassword`, session cookie. It creates jobs (`GetAllJobs` by name → `CreateJobInstance`), refreshes, updates, approves, rejects and deletes them, and reads parcel points. |
| Registry settings (`lr_setting`, read live) | `multicadastre`: 95 `job_stage_description(n)` rules, `<transaction type>:<Registry stage>:<job>`. For example, PrepareTitleDeedPlan / ReviewTitleDeedPlanSQCO → *Deed Plan Creationv2* (64 rules), PlotParcelInCadastre → *Plot Parcels* (29). `mcintegration`: `GISUpdatedFields`, `GISRetrievedFields`, `GISSearchFields`, `LinkageFields`, `GISServer` / `LWAMapURL` = `http://10.10.10.6/GISViewer/MapWorker.aspx`. |
| `LRS.Data.Controls.dll` (Registry desktop client, decompiled) | How the admin links. The Registry's `MapForm` hosts `MapWorkerControl` (an embedded browser) on the GISViewer page, signed in with the Registry user and ticket. It opens the `property-select` map, the user picks parcels, and the control returns their UPIN to the Registry property. |
| GISViewer help (`MultiCadastre_and_LRS_Integration.htm`) and logs | "Linking map object to LRS Property object" happens in the Registry. GISRetrievedFields go Cadastre → Registry (UPIN) and GISUpdatedFields go Registry → Cadastre. The logs show Registry users (admin, ragoro, ljohn, …) signing in to the map daily. |
| Workflow job type 9 *Deed Plan Creationv2* | Search Cadastre for Parcels → (new parcel) Plot Parcel → Review Plotted Parcel → Commit Parcel → Generate Title Deed Plan; (existing parcel) → Generate Title Deed Plan. The plan is sent to the Registry from the Title Deed Plan steps (`SendImageCommand`). 17,104 of its 17,182 jobs carry a transaction. |
| LRS43 `lr_mc_job` | The Registry's record of the jobs it created (16,945). Status 5 occurs only with Completed transactions and 7 only with Withdrawn ones, so the Registry approves (5) or rejects (7) the job. |

Decompiled sources and captures stay outside git (session scratchpad and
`tools/aumentum-inventory/lrs-export/`, which is ignored); only the conclusions are recorded here.

---

## 3. What this Cadastre provides

### 3.1 Reading the Registry: links appear automatically (`src/lrs.rs`, `src/registry_sync.rs`)

* **Client** (`lrs.rs`): read-only by default, exactly Aumentum's calls (`startSession`, HQL `loadObjects`, `endSession`).
  **The Registry's clock runs ~19 minutes ahead** of the office PCs, so the WS-Security timestamp
  is taken from the Registry's own HTTP `Date` header; otherwise every call fails with "The message
  has expired". IT should sync the clocks: Aumentum's GISViewer log shows Registry users refused for
  the same reason on 13 Oct (61 times) and 29 Oct (12 times).
* **Link sync**, every minute (`CADASTRE_REGISTRY_POLL_SECS`): reads properties linked since the last check
  (`SpatialDate1`, current versions, UPIN set) and writes the Registry's own **`GISUpdatedFields`**
  (read live each run) onto the parcel's attribute record: LRSID ← SuID, LinkageStatus = 1,
  LinkageDate ← SpatialDate1 (yyyy-mm-dd), PropertyNumber ← Refnum. Plot and Block get the
  literal text `plotNumber` / `blockNumber`, exactly as Aumentum wrote them (6,496 records); change
  the setting in the Registry to change this. Each link is recorded once as an audit event.
* **Unlink reconcile**, every 15 minutes (`CADASTRE_REGISTRY_RECONCILE_SECS`): snapshots the
  Registry's properties holding a UPIN (`cadastre.registry_property`, migration 0014). A property
  whose UPIN changed or went unlinks the parcel it held (LinkageStatus 0, other fields kept, as in
  Aumentum's data). A property linked to several parcels holds only one UPIN, so it is not an
  unlink. A short Registry answer (under half the last snapshot) is refused rather than unlinking
  everything.

### 3.2 The Registry reading and writing this Cadastre: the job service (`src/routes/mcservice.rs`)

Aumentum's **`/ApplicationServerAdmin/WebServices/LRSWebServices.asmx`**, at the same path and to
the same contract (`crates/cadastre-server/assets/LRSWebServices.wsdl`, captured from the
Registry): SOAP 1.1, document/literal, namespace `http://tempuri.org/`. `GET …?WSDL` returns the
contract pointing at this server.

| Operation | What it does here |
|---|---|
| `Authenticate(user, password)` / `LogOut` | Checks `CADASTRE_MC_SERVICE_USER` / `_PASSWORD` (= the Registry's `mcUserName` / `mcPassword`); sets the `ASP.NET_SessionId` cookie every later call needs. |
| `GetAllJobs` | Active job types (ID, Name, …). The Registry finds its job type by **name**. |
| `CreateJobInstance(jobID, instanceName, priority, comments, parameters)` | Creates the job on the type's start task and default assignee, keeping the parameters. `@LRS:[Transaction]:[TransactionNumber]=PLS…` becomes the job's Registry transaction. Returns the `JobInstance`. |
| `GetJobInstance`, `GetJobInstanceParameters`, `UpdateJobInstance`, `UpdateJobInstanceParameters` | Read / update the job (name, comment, parameters, transaction). |
| `ApproveJob` / `RejectJob` / `CompleteJobInstance` / `DeleteJobInstance` | Status 5 *Approved by the Registry* / 7 *Rejected by the Registry* / 3 *Completed* / 9 *Cancelled* (a deleted job is cancelled, not erased). Migration 0015 labels 5 and 7. |
| `IsJobInstanceCompleted`, `GetJobStatusByJobInstanceID`, `GetJobStatusByTransaction` | Job status (the transaction number is passed as an int: its digits are matched). |
| `GetCurrentTaskInstance`, `CompleteTaskInstance`, `CancelTaskInstance` | The job's task: complete moves on when the task graph has exactly one way on, or ends the job. |
| `GetUpinsByJobInstance`, `GetUpinsByTransaction` | UPINs the job(s) committed. |
| `GetParcelAttributesByUPIN`, `GetParcelStatus` | The parcel's attribute record (Aumentum field names) and fabric status. |
| `GetParcelPointsData(pUpin)` | The parcel's boundary legs: from/to beacon names (from the fabric's beacons), north/east, distance, azimuth, bearing (D°M'S"). |
| `GetConfiguredRegions`, `GetDomains`, `GetDomainDetailsByName` | `Plateau_State`; Aumentum's coded-value domains (`assets/aumentum-domains.json`). |
| `BatchUpdateDataFromRegistry(parameters)` | `NAME=value` pairs (`|` / `;` / new lines) keyed by `UPIN`, written to the parcel's attribute record; recorded in the audit log. |
| `GetAllJobsInstances(filter)` | Open jobs (filter 0) or jobs of that status. |

The parameter format of `BatchUpdateDataFromRegistry`, `GetAllJobsInstances`' filter, and the
exact semantics of Approve / Reject / Delete are inferred from the Registry's code and data. They
are to be confirmed against Aumentum's WebAdmin code once captured (§6).

### 3.3 Writing to the Registry: sending the plan back (`src/routes/registry.rs`)

Only on a user's action, and only with `CADASTRE_REGISTRY_WRITE=1`:

* **Send plan** (`POST /api/v1/wf/jobs/{id}/registry/send-plan`), Aumentum's `SendImageCommand`:
  a JPG or PNG attached to the job goes to the job's transaction. Only Active transactions are
  accepted, never `P-`. The image is uploaded with `PUT /lrs/upload/{guid}.jpg?ticket=…`, then
  `SaveEntity` adds an `IncomingDocument` of type `SurveyPlanFromMC` (`sd_titledeedPlan`), Comments
  "Survey Plan", one image page.
* **Linking from the Cadastre** (Aumentum's desktop `LinkParcel`) exists, tested, but is **off**:
  PLAGIS links in the Registry only. It answers 409 unless `CADASTRE_REGISTRY_CADASTRE_LINKING=1`.

### 3.4 The editor

* **Registry tab** (building icon in the left pane; the count = links waiting for a job):
  * **Find file**: PL or PLS number, read live. Shows the transaction, status, property, UPIN once
    linked, and **Start digitising** (job *Deed Plan Creationv2*, named `Job-<file no>`, with the
    transaction's parameters). If a job for the transaction is already open, **Open job** is shown instead.
  * **Linked**: links from the Registry by day, with Waiting / With job / Unlinked filters.
    Each link has Zoom, File, and Start job / Open job.
  * **Send plan**: a checklist (transaction, Active, plan image with Attach image…) → Send. History below.
  * Header: connection light, last check, one refresh (links + unlinks).
* **Map**: right-click or double-click a parcel → **Start edit — title deed**, Zoom To Parcel,
  Registry File…, Parcel Lineage…. Start edit opens the parcel's job (the transaction of its
  Registry link). If the transaction has no open job yet it offers to start one. It then runs Work On
  This Job and opens the title deed tools (template, Send plan).
* **Workflow steps**: Link Parcel (1013) opens the file in Find file. TDP / Generate / Save Title
  Deed Plan (1003 / 1016 / 1018) offer the site plan and Send the plan to the Registry.

### 3.5 API summary

| Method | Path | |
|---|---|---|
| GET | `/api/v1/registry/status` | Connection, last link check, last unlink check, writing on/off |
| GET | `/api/v1/registry/links?days=14` | Links (synced or made), each parcel, its job, linked now |
| POST | `/api/v1/registry/sync`, `/api/v1/registry/reconcile` | Check now |
| GET | `/api/v1/registry/find?q=PL…\|PLS…` | Find a Registry file, with its open job |
| POST | `/api/v1/registry/start-digitising` | Create (or return) the file's job |
| GET | `/api/v1/registry/transactions/{number}` | A transaction and its properties |
| GET | `/api/v1/registry/parcels/{upin}/work` | A parcel's link, transaction and job (Start edit) |
| GET | `/api/v1/registry/parcels/{upin}/events` | A parcel's link history |
| POST | `/api/v1/wf/jobs/{id}/registry/send-plan`, GET `…/sent` | Send the plan; what was sent |
| POST | `/api/v1/registry/link`, `/api/v1/registry/unlink` | Off (linking is done in the Registry) |
| GET/POST | `/ApplicationServerAdmin/WebServices/LRSWebServices.asmx` | The Registry's job service (§3.2) |

---

## 4. Configuration

In the server's environment (`~/.config/cadastre/server.env` on the dev PC):

| Variable | Meaning |
|---|---|
| `CADASTRE_REGISTRY_URL` | `http://10.10.10.5:8080/lrs/api/` (the Registry API). Unset: no Registry features. |
| `CADASTRE_REGISTRY_USER` / `_PASSWORD` | The Registry account (Aumentum's MCServer used `admin`). |
| `CADASTRE_REGISTRY_WRITE` | `1`: allow writing to the Registry on user action (send plan). |
| `CADASTRE_REGISTRY_POLL_SECS` / `_RECONCILE_SECS` | Link check (60) and unlink check (900) intervals. |
| `CADASTRE_MC_SERVICE_USER` / `_PASSWORD` | The account the Registry signs in with to the job service: set them to the Registry's `multicadastre` `mcUserName` (`administrator`) and `mcPassword`. **Not set yet**: copying the Registry's password is left to the administrator. Until set, the job service answers "not configured". |
| `CADASTRE_REGISTRY_CADASTRE_LINKING` | `1` re-enables linking from the Cadastre (off: PLAGIS links in the Registry). |

Settings in the database (`cadastre.app_setting`): `aumentum.Integration.SurveyPlanFromMC`
(`sd_titledeedPlan`) and `aumentum.Mutation.keepParentIdAfterMutation`. These are system values:
`registry.sync.since` (link watermark), `registry.sync.status`, `registry.reconcile.status`.

---

## 5. Cut-over: switching the Registry to this Cadastre

Until these steps are done, the Registry keeps creating jobs in, and linking against, Aumentum.
Digitisers can still start jobs here (Find file → Start digitising), but **parcels committed here
cannot be linked by the Registry admin until step 3 is done**: the Registry's map still shows
Aumentum's parcels only.

1. Set `CADASTRE_MC_SERVICE_USER` / `_PASSWORD` (see §4) and restart the server.
2. Registry admin: `multicadastre` → `mcServiceAddress` = `http://<this server>/ApplicationServerAdmin/WebServices/LRSWebServices.asmx`.
   The Registry then creates jobs here and reads parcels and points from here.
3. Registry admin: `mcintegration` → `GISServer` and `LWAMapURL` = this server's map page for the
   Registry. **Still to build:** it must speak the bridge the Registry's `MapWorkerControl`
   expects (§6). Then the admin links parcels committed in this Cadastre.
4. Sync the Registry server's clock (it runs ~19 minutes fast).
5. Run both systems side by side on a few files, then retire Aumentum's job service and map.

---

## 6. Open items

| Item | Needs | Status |
|---|---|---|
| **The Registry's map** (where the admin links). Replace GISViewer's `MapWorker.aspx` so the Registry's unchanged `MapWorkerControl` shows and returns this Cadastre's parcels. | Run `tools/aumentum-inventory/RUN-ME-REGISTRY-MAP-CAPTURE.cmd` on PRODUCTIONMC. It copies the Silverlight client (`Client.xap`), GISViewer's assemblies, WebAdmin's code, and the Registry client's `LRS.Client.MapWorker.dll`, read-only. | Kit ready; waiting for the run |
| **Confirm the job service's behaviour** against Aumentum's WebAdmin code (Approve / Reject / Delete, BatchUpdate format). | The same capture (WebAdmin). | Built from the contract and data; to confirm |
| **Title deed template parity**: Aumentum's `Plan_.mxd` / `PSG_Plan_.mxd` layout, print, and sending the plan straight from it. | Phase 6 (deed plans). Today the template is the site plan PDF; the plan image is attached and sent. | Planned |
| `CADASTRE_MC_SERVICE_USER` / `_PASSWORD` | Administrator copies the Registry's `mcUserName` / `mcPassword`. | To do |
| Registry server clock (+19 min) | IT. | To do |
| RBAC: assigning linked jobs to staff; digitising / title deed dashboards | Later phase. | Planned |
| Plot / Block literal values (`plotNumber` / `blockNumber`) | PLAGIS may change `GISUpdatedFields` in the Registry; this Cadastre follows. | Decision for PLAGIS |

---

## 7. Testing

`cargo test --workspace` with `CADASTRE_TEST_DATABASE_URL` set. The Registry tests run without a
Registry:

* `registry_links_sync_into_the_cadastre`: links → attribute records, events, list, job.
* `registry_unlinks_by_snapshot`: baseline, unlinks, a multi-parcel property, a short answer.
* `registry_write_back_like_aumentum`: against a **fake Registry** (a local SOAP server). Covers
  link / unlink (switched on for the test), send plan, the refusals, and the exact `SaveEntity` /
  upload payloads.
* `registry_job_service_like_aumentum`: the job service called as the Registry's Java client calls
  it (Authenticate, cookie, GetAllJobs, CreateJobInstance, …, LogOut, WSDL).
* Unit tests in `lrs.rs` (a real Registry response, payload builders), `registry.rs` (link rules),
  `registry_sync.rs` and `mcservice.rs` (request parsing, bearings).

Live checks (read-only) used the Registry account in the repo's `.env` (`REGISTRY_USER` /
`REGISTRY_PASSWORD`).
