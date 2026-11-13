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
| `CreateJobInstance(jobID, instanceName, priority, comments, parameters)` | Creates the job, status 2, on the type's start task **unassigned** (Aumentum `CreateNonAssignedTaskInstance`), priority as Aumentum's `ConvertUInt2Char` (above 2 → `2`), keeping the parameters. `@LRS:[Transaction]:[TransactionNumber]=PLS…` becomes the job's Registry transaction. Returns the `JobInstance`. |
| `GetJobInstance`, `GetJobInstanceParameters`, `UpdateJobInstance`, `UpdateJobInstanceParameters` | Read / update the job (name, comment, parameters, transaction). |
| `ApproveJob` | Ends the open tasks, status 5 (Aumentum: end every task of the user, `ApproveRejectJob(id, 5)`). |
| `RejectJob` | Discards the job's unposted edits: drafts and pending mutations, Aumentum's `DeleteJobAreaByJobInstanceID`. Then status 7. |
| `CompleteJobInstance` | Ends the open tasks, status 3. Aumentum's `IsJobInstanceCompleted` calls 3 *"Waiting for LRS Commit"*: the Cadastre's work is done and awaits the Registry's approve / reject. |
| `DeleteJobInstance` | Aumentum deletes the job; **here it is cancelled (status 9)** so its history and parcels stay. This is a deliberate difference. |
| `IsJobInstanceCompleted` | Aumentum's wording: 3 → *Waiting for LRS Commit* (completed), 5 / 7 → *Already been approved or rejected* (completed), 2 → *Active*, otherwise *Unknown*. |
| `GetJobStatusByJobInstanceID`, `GetJobStatusByTransaction` | As Aumentum: the job's **job type** (`WFJob`). The transaction match compares the job's `@LRS:[Transaction]:[TransactionNumber]` with the int the Registry passes, so a PLS number never matches (as in Aumentum). |
| `GetCurrentTaskInstance`, `CompleteTaskInstance`, `CancelTaskInstance` | The job's task: complete moves on when the task graph has exactly one way on, or ends the job. |
| `GetUpinsByJobInstance`, `GetUpinsByTransaction` | UPINs the job(s) committed. |
| `GetParcelAttributesByUPIN`, `GetParcelStatus` | The parcel's attribute record (Aumentum field names) and fabric status. |
| `GetParcelPointsData(pUpin)` | The parcel's boundary legs: from/to beacon names (from the fabric's beacons), north/east, distance, azimuth, bearing (D°M'S"). |
| `GetConfiguredRegions`, `GetDomains`, `GetDomainDetailsByName` | `Plateau_State`; Aumentum's coded-value domains (`assets/aumentum-domains.json`). |
| `BatchUpdateDataFromRegistry(parameters)` | `NAME=value` pairs (`|` / `;` / new lines) keyed by `UPIN`, written to the parcel's attribute record; recorded in the audit log. |
| `GetAllJobsInstances(filter)` | As Aumentum: 1 active, 2 all, 3 completed (3 / 5 / 7); otherwise none. |

Checked against Aumentum's own code (`MCAdminWebSite.dll`, WebAdmin, decompiled 2026-11-02:
`MCAdminWebSite.WebServices.LRSWebServices`). `Authenticate` there is a Cadastre user's sign-in
(`DataHelper.Login`), so the Registry's `mcUserName` is a Cadastre account. Here that account is
`CADASTRE_MC_SERVICE_USER` / `_PASSWORD`. `BatchUpdateDataFromRegistry` and `GetParcelPointsData` call
into MCServer's workflow service, whose code is not captured. The `NAME=value|…` format of
BatchUpdate is therefore still inferred.

### 3.3a The Registry's map: where the admin links (`src/routes/gisviewer.rs`)

The Registry admin links in the Registry desktop (Properties page, map). Its `MapWorkerControl`
(`ThomsonReuters.Aumentum.Registry.Services`, decompiled 2026-11-02 from the Registry client capture)
hosts an embedded Internet Explorer on `GISServer`. This Cadastre now serves that page and its service
at Aumentum's paths, so pointing `GISServer` / `LWAMapURL` here is enough (§5 step 3).

**What the Registry does, and what this Cadastre answers:**

| Registry desktop (MapWorkerControl) | This Cadastre |
|---|---|
| POSTs `{GISServer}?ct=indexer&justmap=1&map=property-select&lang=…` with `auth=1&lrsuser=…&lrsticket=…&initparam_inputobjects=…&initparam_extent=…` | `GET/POST /GISViewer/MapWorker.aspx`. The ticket is checked with the Registry (`AuthenticationService.isTicketValid`), then a map page is served. A missing or bad ticket gets a refusal page (checked live 2026-11-02). |
| Input objects: `ScriptableInputEntitiesGroup[]` (the properties being linked and their search fields, e.g. UPIN), as JSON → base64 → `[BEGIN]…[END]` (or `<base64Binary …>…[END]`) | Decoded. Each property is listed on the left, its search values are looked up, matching parcels are drawn and pre-selected, and the map zooms to them. |
| Waits for element `silverlightControl`; sets `indexerEvents` to `Map Ready` and clicks it | Both elements exist. `indexerEvents` becomes `Map Ready` once the searches finish. |
| `doAction("SelectionSet","GetSelectedObjects")` | `ScriptableEntitiesGroup[]`: per parcel `ObjectID` (the attribute record's OBJECTID, Aumentum's own), `InputUUID` (which property it is for), `Attributes` (UPIN, LRSID, PropertyNumber, Plot, Block, ParcelID, LinkageStatus), encoded `[BEGIN]…[END]`. |
| `doAction("Map","GetExtentAsString")` | `ScriptableEnvelope` in EPSG:32632 (WKID 32632). |
| `doAction("Editor","IsEditInProgress")` | `false` (the map does not edit). |
| On Link: copies the UPIN into the Registry property itself, then calls `{GISServer}/../WebServices/CadastralWebService.asmx` → `UpdateCadastralData` with `SecurityHeader{User, Password = ticket}` | `POST /GISViewer/WebServices/CadastralWebService.asmx` (namespace `http://www.landsystems.com/multicadastre/in4/service`). It checks the ticket, then writes the values (LRSID, LinkageStatus, LinkageDate, PropertyNumber, Plot, Block, …) onto the parcel's attribute record, either by `UpdateRecordByOID` (OBJECTID) or by `UpdateRecordByFields` (matching `PrimaryFields`, skipping `ExculdeFeatures`). Audit: `registry_map_update`. `GetVariables` and `DetectNeighbours` answer empty. |

The page is ES5 with Leaflet 1.9.4 (`web/vendor/leaflet/`) and `X-UA-Compatible: IE=edge`, because
the Registry's browser is IE11. Esri World Imagery is the background. Parcels load by view
(`GET /GISViewer/data/parcels?token=…&bbox=…`) or by search (`…&field=UPIN|PropertyNumber|LRSID|Plot|Block&value=…`).
The `token` is the map session, issued only after the Registry ticket is accepted, and lasts 8 hours.
The admin can also find a parcel by hand and click parcels to select or deselect them.

**OK is the link, and the desktop saves it later.** The form's OK button runs `ConnectObjectsAsync`:
it sends `UpdateCadastralData` and puts the UPIN in the file open on the desktop, which is only
stored when the officer leaves the file with Suspend, or completes the stage (confirmed live on
PL63576, 5 Nov 2026). Cancel loses it, and the Cadastre's half then stands alone.
With `CADASTRE_REGISTRY_LINK_ON_MAP_OK=1` the server also writes the Registry's half itself, through
`link_parcel`, after the desktop has its answer (60-second limit). It acts only on one link in one
call, on a property with one Active transaction, and with `link_parcel`'s checks; anything else is
recorded as `registry_link_failed` (how `map_ok`) and listed in link problems until the Registry
holds the link. Whether this is safe while the desktop has the file open is what the first live
test must show: press OK, check the Registry, then Suspend and check there is no error.

Two paths write the link into the Cadastre, and either one is enough. `UpdateCadastralData`
writes it at once. The link poll (§3.1) also sees it within a minute. The second write changes
nothing, because values are written only where they differ.

`CADASTRE_GISVIEWER_OPEN=1` opens the map without a ticket. It is for development only, never production.

### 3.3 Writing to the Registry: sending the plan back (`src/routes/registry.rs`)

Only on a user's action, and only with `CADASTRE_REGISTRY_WRITE=1`:

* **Send plan** (`POST /api/v1/wf/jobs/{id}/registry/send-plan`), Aumentum's `SendImageCommand`:
  the title deed plan goes to the job's transaction **as a PDF** (Title Deed Plan → *Attach as PDF*
  files it on the job; a JPG attached by hand is turned into a one-page A4 PDF first; PNG is
  refused). Only Active transactions are accepted, never `P-`. The PDF is uploaded with
  `PUT /lrs/upload/{guid}.pdf?ticket=…`, then `SaveEntity` adds an `IncomingDocument` of type
  `SurveyPlanFromMC` (`sd_titledeedPlan`), Comments "Survey Plan", with two pages: first the
  sheet's JPEG as an `image` page (isattachment False), then the PDF as an `attachment` page
  (isattachment True). The Registry shows image pages only; a PDF on its own is a file to open, so
  a plan sent as a PDF alone (before 12 Nov 2026, e.g. PL66724) shows no picture. The JPEG is the
  one *Attach as PDF* embedded in the PDF, taken out byte for byte (`pdf::jpeg_in_pdf`), uploaded
  as `{guid}.jpg`. A PDF made elsewhere has no such JPEG: it is sent alone and the officer is told.
  A JPEG over the Registry's image limit (`scan.MaxImageSize`, 1024 KB) is not sent either; Attach
  as PDF lowers the JPEG quality until the sheet fits. Settings: `aumentum.Integration.PlanImage`
  is `page` (default, as above), `document` (the JPEG and the PDF as two documents of the same
  type) or `off` (PDF only); `aumentum.Integration.PlanPageType` is the PDF page's type.
* **Resend corrected plan** (same call, `"resend": true`): once a plan has been sent to the
  transaction, a plain send is refused (409) so it does not collect duplicates by accident. A resend
  files the corrected PDF as "Survey Plan (corrected, <date>)", then marks the plans filed before
  (looked up read-only by transaction and document type) with Comments "Survey Plan - REPLACED by
  the corrected plan of <date>". Nothing is deleted in the Registry. If the earlier plan cannot be
  found or marked, the resend still succeeds and says so; the officer marks it in the Registry.
* **Correct plan** (`POST /api/v1/registry/correct-plan`, `{ upin, transaction_number?, reason }`):
  for a plan that came back, including one Aumentum made. The parcel, UPIN and link are not touched.
  Allowed only for a parcel linked on an Active transaction at Raise Title Deed Plan, read live from
  the Registry. It uses the transaction's open job, or a new `Job-<file number>` that completes when
  the plan is sent, and carries over the parcel's latest saved form. Send plan then always resends,
  with the reason in the Comments, and marks the plans the Registry holds as replaced, so Aumentum's
  plan is included. On this Registry those are found through `Transaction.Documents`.
  **First test on a test transaction:** the PDF page type and the lookup of earlier documents have
  not been tried against the live Registry yet.
* **Linking from the Cadastre** (Aumentum's desktop `LinkParcel`) exists, tested, but is **off**:
  PLAGIS links in the Registry only. It answers 409 unless `CADASTRE_REGISTRY_CADASTRE_LINKING=1`.

### 3.4 The editor

* **Registry tab** (building icon in the left pane; the count = links waiting for a job):
  * **Find file**: PL or PLS number, read live. Shows the transaction, status, property, UPIN once
    linked, and **Start digitising** (job *Deed Plan Creationv2*, named `Job-<file no>`, with the
    transaction's parameters). If a job for the transaction is already open, **Open job** is shown instead.
  * **Linked**: links from the Registry by day, with Waiting / With job / Unlinked filters.
    Each link has Zoom, File, and Start job / Open job.
  * **Send plan**: a checklist (transaction, Active, plan PDF with Attach PDF…) → Send, or
    **Resend corrected plan** once a plan was sent. History below (Sent / Corrected).
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
| GET/POST | `/GISViewer/MapWorker.aspx` | The Registry's map (§3.3a) |
| GET | `/GISViewer/data/parcels` | The map's parcels (map session token) |
| POST | `/GISViewer/WebServices/CadastralWebService.asmx` | `UpdateCadastralData` on Link (§3.3a) |

---

## 4. Configuration

In the server's environment (`~/.config/cadastre/server.env` on the dev PC):

| Variable | Meaning |
|---|---|
| `CADASTRE_REGISTRY_URL` | `http://10.10.10.5:8080/lrs/api/` (the Registry API). Unset: no Registry features. |
| `CADASTRE_REGISTRY_USER` / `_PASSWORD` | The Registry account (Aumentum's MCServer used `admin`). |
| `CADASTRE_REGISTRY_WRITE` | `1`: allow writing to the Registry on user action (send plan). |
| `CADASTRE_REGISTRY_LINK_ON_MAP_OK` | `1`: pressing OK on the Registry's map also writes the UPIN and link date to the Registry (§3.3a). Off unless set; needs `CADASTRE_REGISTRY_WRITE=1`. |
| `CADASTRE_REGISTRY_POLL_SECS` / `_RECONCILE_SECS` | Link check (60) and unlink check (900) intervals. |
| `CADASTRE_MC_SERVICE_USER` / `_PASSWORD` | The account the Registry signs in with to the job service. It must equal the Registry's `multicadastre` `mcUserName` / `mcPassword`. Set on 2026-11-02 from `.env`: `administrator`, the Registry's `mcUserName`. Checked: `Authenticate` accepts it and refuses a wrong password. The password must equal the Registry's `mcPassword`. |
| `CADASTRE_AUMENTUM_JOBSERVICE_URL` | Parallel run with Aumentum: before giving a new UPIN the Cadastre asks Aumentum's job service (`http://10.10.10.6/ApplicationServerAdmin/WebServices/LRSWebServices.asmx`, signed in with `CADASTRE_MC_SERVICE_USER` / `_PASSWORD`) whether Aumentum already issued it; such numbers are recorded (`aumentum:live`) and skipped. If Aumentum cannot be asked the commit is refused. Remove after cut-over. |
| `CADASTRE_AUMENTUM_WATCH_SECS` | Parallel run: how often the duplicate-UPIN watch asks Aumentum about the UPINs committed here (default 900, minimum 60). Runs only while `CADASTRE_AUMENTUM_JOBSERVICE_URL` is set. |
| `CADASTRE_GISVIEWER_OPEN` | `1` opens the Registry's map without a ticket. Development only. |
| `CADASTRE_REGISTRY_CADASTRE_LINKING` | `1` re-enables linking from the Cadastre (off: PLAGIS links in the Registry). |

Settings in the database (`cadastre.app_setting`): `aumentum.Integration.SurveyPlanFromMC`
(`sd_titledeedPlan`) and `aumentum.Mutation.keepParentIdAfterMutation`. These are system values:
`registry.sync.since` (link watermark), `registry.sync.status`, `registry.reconcile.status`.

---

## 5. Cut-over: switching the Registry to this Cadastre

Until these steps are done, the Registry keeps creating jobs in, and linking against, Aumentum.
Digitisers can still start jobs here (Find file → Start digitising), but **parcels committed here
cannot be linked by the Registry admin until step 3 is done**: until then the Registry's map shows
Aumentum's parcels only.

1. Set `CADASTRE_MC_SERVICE_USER` / `_PASSWORD` (see §4) and restart the server.
2. Registry admin: `multicadastre` → `mcServiceAddress` = `http://<this server>/ApplicationServerAdmin/WebServices/LRSWebServices.asmx`.
   The Registry then creates jobs here and reads parcels and points from here.
3. Registry admin: `mcintegration` → `GISServer` and `LWAMapURL` = `http://<this server>/GISViewer/MapWorker.aspx`
   (§3.3a). The Registry desktops must reach this server over HTTP, and this server must reach the
   Registry API (it checks each ticket there). The admin then links parcels committed in this Cadastre
   exactly as before.
4. Sync the Registry server's clock (it runs ~19 minutes fast).
5. Run both systems side by side on a few files, then retire Aumentum's job service and map.

---

## 5a. Parallel run: UPINs

Both systems number new parcels in the same series (`PL` + 12 digits, counting up). PLAGIS chose
to continue after Aumentum (2026-11-03), so:

* every commit here asks Aumentum first (`CADASTRE_AUMENTUM_JOBSERVICE_URL`) and never takes a number
  Aumentum has issued; commits here never share a number with each other (one atomic counter, and
  the issued-UPIN ledger's unique key);
* Aumentum does **not** know this Cadastre's numbers: if a parcel is committed here first, Aumentum
  will later issue the same number. **While both run, commit in one system at a time** (or freeze
  commits in Aumentum during the trial).
* A commit made by mistake can be undone before it is linked:
  `cargo run -p cadastre-server --example revert_commit -- <job> <UPIN> [property no.] [--apply]`
  (2026-11-03: job 17511, PL000000023896, which Aumentum had already given to PL68346).
* **The Commit window shows the UPIN(s) a commit will give** before committing
  (`GET /api/v1/wf/jobs/{id}/next-upin`): checked with Aumentum just now, the numbers skipped
  because Aumentum has them, or a red line when Aumentum cannot be asked (the commit will be
  refused). Advisory: the commit allocates again, atomically.
* **Duplicate watch** (`src/upin_watch.rs`): every `CADASTRE_AUMENTUM_WATCH_SECS` the Cadastre asks
  Aumentum about each UPIN committed here (ledger source `posted`). One Aumentum has issued since
  is a duplicate: it is shown in `GET /api/v1/registry/status` (`upin_watch`) and as a red line in
  the Registry tab, and audited once (`duplicate_upin_found`). Check now:
  `POST /api/v1/registry/upin-watch`. Do not link a duplicate until it is resolved.

### Job numbers in the parallel run

Jobs and task instances created here are numbered **from 1,000,001** (migration 0019); Aumentum's
ids stay below 1,000,000 and never move these counters, so reloading Aumentum's workflow
(`tools/import/load-aumentum-workflow.sh`) cannot meet a number used here. Jobs created here
before the migration were renumbered into the range (`cadastre.wf_renumbered` keeps old → new).
The loader refuses an export with ids of 1,000,000 or more, refreshes a task row only when it is
Aumentum's own row of the same job, and reports any number clash in its reconciliation.

Areas are recorded and shown to 2 decimals, cut not rounded (903.1976 m² is 903.19 m²); the
geometry keeps its exact area.

## 6. Open items

| Item | Needs | Status |
|---|---|---|
| **The Registry's map** (where the admin links) | Built (§3.3a) from the decompiled `MapWorkerControl`. Tested as the Registry drives it, and the ticket check was tried live. The Registry desktop has not opened it yet. After §5 step 3, link one test file first and check the parcel and the Registry property. | Built; first Registry trial to do |
| **Registry Web Access map tab** (`lrswa` `map.do`) | It is Aumentum's Silverlight viewer, not MapWorker: it probes `{GISViewer}/Assets/s.gif` ("No connection to Cadastre Server" on failure), then loads `Client/Client.xap`, `Client/Conf/web-access.xml` and draws via `gis.gate/*` from Aumentum's database. Bridge (2026-11-03): every `/GISViewer/*` path this server does not answer is forwarded to `CADASTRE_GISVIEWER_UPSTREAM` (`http://10.10.10.6`) and logged. Searches Aumentum answers empty (`gis.gate/0/0/query`, `UPIN`/`propertyNo`/`PlotNo`/`BlockNo`/`PlanNo`/`ParcelName`) are answered from this Cadastre's posted parcels in Aumentum's JSON, so new parcels are found and outlined; the drawn image still shows only Aumentum's parcels. Replacing it (answer `gis.gate` from PostGIS, from the logged requests) is needed before Aumentum is switched off. | Bridged; replacement to do |
| **Job service vs Aumentum** | Checked against `MCAdminWebSite.dll` and corrected (2026-11-02): unassigned start, priority, RejectJob discards the job's edits, status wording, job type answers, exact transaction match. BatchUpdate / points live in MCServer (not captured). | Done, except BatchUpdate's format |
| **Title deed template parity**: Aumentum's `Plan_.mxd` layout, print, and sending the plan straight from it. | Built 2026-11-03 (`src/titledeed.rs`, `routes/titledeed.rs`), laid out on a printed PLAGIS plan (PL68061): A4, clockwise legs from the westernmost beacon, auto-fit standard scale, Registry prefill with Aumentum's own `R_PROPINFO` / `R_HOLDERS` queries, a form saved per job (beacon numbers, road side, plotted/checked by), Print/PDF and the plan JPEG that Send plan files. `PSG_Plan_.mxd` (the PSG variant) not yet seen. | Built; review by the title deed unit |
| `CADASTRE_MC_SERVICE_USER` / `_PASSWORD` | Set to `administrator` (2026-11-02); the password must equal the Registry's `mcPassword`. | Done |
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
* `registry_map_like_aumentum_gisviewer`: the map page opened with a good and a bad ticket, the
  Registry's encoded input objects, the data endpoints, and `UpdateCadastralData` by OBJECTID and by fields.
* Headless Chrome (2026-11-02, live data): `Map Ready`, `GetSelectedObjects` → OBJECTID 593031 /
  PL000000022581, extent in WKID 32632, no script errors.
* Unit tests in `lrs.rs` (a real Registry response, payload builders), `registry.rs` (link rules),
  `registry_sync.rs` and `mcservice.rs` (request parsing, bearings).

Live checks (read-only) used the Registry account in the repo's `.env` (`REGISTRY_USER` /
`REGISTRY_PASSWORD`).

## Sign-in (`CADASTRE_AUTH=local`)

The server has its own sign-in page at `/login`: username, password and workstation. Keycloak
(`CADASTRE_AUTH=enabled`) is not needed.

**Who signs in.** The Registry's users, with their Registry password, checked first by the Registry
(`AuthenticationService.startSession`); their account here is made at first sign-in and no Registry
password is kept. Staff who are not in the Registry use this server's accounts. If the Registry
cannot be reached, only those local accounts can sign in. The Registry's users and groups live in
its Alfresco store (`LRSAdmin.alf_authority` in LRS43), read with `AccessControlService`. `guest` and
empty passwords never pass through the Registry.

| Workstation | Address | Registry group | This server's group |
|---|---|---|---|
| Sysadmin | `/sys-editor` | `GROUP_LRS_ADMINISTRATION_SYSTEM` | Administrators |
| Digitising, Title Deed | `/digitising`, `/title-deed` | `GROUP_Scanning Clerk`, `GROUP_Scanning Officer`, `GROUP_Map Compilation Officer`, `GROUP_Vectorization Clerk` | Users |
| Charting and Reporting | `/reporting` | Any signed-in user, read-only unless in a group above | |

An administrator may leave the workstation empty and goes to User Management (`/user-management`,
the Registry's users and units); everyone else must choose.

A Registry user listed in `cadastre.registry_password_reset` must set a new password at first sign-in
(Sysadmin → Registry Users lists them when it creates a user or resets a password);
it is changed in the Registry (`AdministrationService.changePassword`, in the user's own session) and
the row goes. Users created with `AdministrationService.createUsers` wait on any long query holding
`LRSAdmin.alf_node`, so a heavy search on that table blocks new users.

**Beacon numbers at commit.** A typed number another beacon already has is checked first
(`POST /api/v1/wf/jobs/{id}/editing/beacon-check`). On a property sharing a boundary with this parcel,
within `commit.adjoining_beacon_m` (1 m), it is the same beacon: the corner takes its position. Anywhere
else the digitizer sees the property holding it (file number, UPIN, Zoom, Point List) and goes on only
with a reason; the number then repeats (`beacon.duplicate_seq`). The property is found by position,
because Aumentum's parcels have no corner records. Both are kept in the commit's `beacon_notes`.

**Correcting points, at the digitizing stage only.** While sketching, Edit Sketch Properties has a beacon
number box for each new corner, and the number stays with the corner when its coordinates are corrected;
a corner on a fabric beacon shows that beacon's number. On a draft, the Point List's Edit Points and beacon
number boxes. After commit, Edit on a Point List row corrects a wrong number with a reason
(`POST /api/v1/parcels/beacon-number`); that Edit is refused once the parcel is linked in the
Registry, and a number on a beacon shared with a linked parcel is refused too, as it would change that file.

**Correct (one file, one UPIN, one job).** Wrong coordinates, shape or attributes after commit go through
Correct (`POST /api/v1/wf/correct`; also from a job, `/wf/jobs/{id}/correct`). It reopens the file's own
job at its commit task: the job that committed the parcel, else the file's latest job here (an open one
first); a job is made on the file's Registry transaction only when the file has none. A copy of the
parcel opens as a draft under the same UPIN, so nothing is archived and no new UPIN is given. The
committed shape stays live (map, Registry map, attribute record) until the correction is committed; then
it is kept as history (`superseded`) and the corrected shape takes its place. The attribute record stays,
so the Registry link (LRSID, LinkageStatus, link date) is never broken and a **linked parcel needs no
unlink**; the corrected plan is resent with Correct plan. A parcel archived in Aumentum with no reason and
nothing committed in its place is brought back the same way. The system check does not count the
parcel's own shape (here or in Aumentum) or its own Registry property against it, and a closed
transaction only warns. Corners kept in place keep their beacons; a moved corner takes its number again
(the beacon its earlier commit added at the old place is removed). Still refused: split / merge lineage
(a mutation job), and a parcel archived on purpose (with a reason). Correct again on the same parcel
carries on in the same job.

**Only the file's current UPIN is corrected.** Aumentum archived a wrong parcel and digitised it again under
a new UPIN, so about 1,100 files hold several UPINs. The current one is the file's live UPIN; an archived
UPIN never comes back while the file has one ("it is an old UPIN of file …; the file's current UPIN is …").
With nothing live, it is the UPIN the Registry holds, else the file's most recent. Several live UPINs on
different land are separate plots, each current; several on the same land are duplicates, and only the one
the Registry holds is current. With none held, Correct waits until the duplicate is retired.

If Aumentum cannot answer the UPIN check, commits are refused and Digitising, Title Deed and Sysadmin
show a red warning to contact the ICT Admin (`GET /api/v1/aumentum/status`: signs in and asks about a
UPIN, at most once a minute). It clears itself when Aumentum answers.

A session keeps the Registry groups read at sign-in, so a change in the Registry applies at the next
sign-in. The workflow treats those working groups as the Users group for its task permissions.

* **Enforced on the server** (`auth::gate`) for pages and API, not only in the screens.
  `/api/v1/admin`, `/api/v1/audit` and Swagger need Administrators.
* **Passwords of local accounts**: Sysadmin → Administration → Users & Groups → Set / Reset makes a temporary password,
  shown once; its owner must change it at first sign-in. This also works with sign-in off, so accounts
  are prepared before it is turned on. There is no e-mail reset: the ICT Unit resets.
* **Limits**: five wrong passwords lock that login name for 15 minutes, Registry or local (a reset clears it). Sessions last
  12 hours, or 30 days with Remember me. Locking an account ends its sessions at once.
* **Stay public, because the Registry and Aumentum call them without a browser session**: the job
  service (`/ApplicationServerAdmin/WebServices/LRSWebServices.asmx`) and `/GISViewer/*`.
* **Turning it on**: set `CADASTRE_AUTH=local` in `~/.config/cadastre/server.env` and restart.
  `CADASTRE_AUTH=disabled` and a restart turns it off again.
* The server is plain HTTP, so passwords cross the LAN unencrypted until it is put behind HTTPS.
* `tools/titledeed/*.ps1` call the API and have no session: they stop working once sign-in is on.
