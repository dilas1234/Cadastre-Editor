# 09 — Build phases (Registry replica)

These phases are written in the same style as the Cadastre roll-out (phases 0–8). A phase is **done** only when three things are true:
- its tests pass against `LRS43_COPY`;
- PLAGIS has signed off the screens against the real ones;
- every row in its checklist is closed.

Order follows the volume in 04 §4. APRO, INSC, GRPM, APCO and APCA are 87 % of all files.

| Phase | Name | Delivers | Depends on | Done when |
|---|---|---|---|---|
| **R0** | Capture | Every P1 item in [08](08-capture-checklist.md): jBPM definitions, fee rates, Web Access pages and config, report templates, Alfresco model and content-URL sample, workstation configs, screenshots, the screen recording of one file from lodgement to C of O, wire samples | Read-only access to .5 (its admin runs the kit) | Every P1 row has a file in the capture folder |
| **R1** | Foundations | New repo skeleton, plus: <ul><li>the LRS43 data layer, following the rules in 02 §9: counters, `Hjtype`, joined and `_ext` rows, dictionary ids, lifespan versioning, dense indexes</li><li>a TLS 1.0-capable connection (D7)</li><li>settings and dictionary caches</li><li>sign-in against the Alfresco users (10 §7), keeping their existing passwords</li><li>permissions `lrs*`</li><li>the `plagis_ext` schema</li><li>the test harness on `LRS43_COPY`</li></ul> | R0 (A5–A7) | Reads of every core entity match the old server field for field on 50 sample files. A counter is shared safely with a simulated old server |
| **R2** | Web Access (read-only) | <ul><li>login</li><li>search and result grids, plus detail views per entity, both from the `*LWA` `lr_formfield` rows</li><li>history and document viewer (reads Alfresco and `D:\LRS_STORAGE`)</li><li>map tab served by the new Cadastre GISViewer, not Silverlight</li><li>deep links `welcome.do?…#detail:` / `#result:`</li></ul> | R1, B1–B2 | PLAGIS compares 20 files side by side with no difference. The old Desktop's "Web Access" button opens the new pages |
| **R3** | Workflow engine | <ul><li>loads the jBPM definitions (D3)</li><li>inbox/queue per user and group (swimlanes)</li><li>claim/lock, complete with the chosen transition, assign, postpone/continue, withdraw, reject</li><li>`lr_process_history`</li><li>the stage rules (05 §4.2)</li></ul> | R1, A1–A2, A6 | Replayed on the copy for 200 historic APRO files: same stages, same history rows, and the old Desktop inbox shows the new tasks correctly |
| **R4** | Workstation core: First Registration (APRO/APSL/CONT/RAOL/REGR) | The about 25 page types (05 §9, 11), driven by WorkflowConfig and `lr_formfield`, for these pages: <ul><li>reception and lodgement</li><li>applicants and parties</li><li>properties</li><li>documents</li><li>fee list and outstanding</li><li>payment</li><li>print documents</li><li>complete and assign</li></ul> Also fee calculation identical to today (A3, A8), and the receipt and demand-notice numbers | R3 | An APRO file is walked from lodgement to R of O on the copy. Fees match 30 historic files exactly, and every page matches its screenshot |
| **R5** | Documents, scanning, printing | <ul><li>local scan agent (TWAIN/WIA) and upload</li><li>indexing and scanning (INSC)</li><li>the BIRT templates rebuilt as identical PDFs: R of O, C of O with watermark and deed-plan image, acknowledgement, demand notice, rejection letter, …</li><li>document store (D5)</li></ul> | R4, B4, A5 | PLAGIS signs printed outputs side by side; the INSC flow is complete |
| **R6** | Cadastre and map | <ul><li>outbound job service: `CreateMCJob`/`Approve`/`Reject` at the configured stages, keeping `lr_mc_job` the same</li><li>the stage guard (95 `job_stage_description` rules)</li><li>UPIN link at Raise Title Deed Plan, from the new Cadastre map</li><li>title deed plan image intake</li></ul> | R3, existing Cadastre (`routes/mcservice.rs`, `gisviewer.rs`) | Raise Title Deed Plan → Cadastre job → commit → link → C of O works on the copy, and the UPIN rule holds |
| **R7** | C of O and ground rent | APCO, APRF, REPT, GRPM, GRR, SROE, SPC and the sub-processes (Raise Site Analysis Report, Survey Plan Checking and Approval) | R4–R6 | Each type walked end to end; GRPM postings match |
| **R8** | Subsequent registration and the rest | The one Subsequent Registration process and its 26 dealing types (APCA, ASSI, APCM, MORT, RMOR, …), plus DCT, Search (SEAR/CETC) and Backlog Data | R4–R5 | Each type's route (04 §2) is tested |
| **R9** | Admin and reports | <ul><li>users, groups, permissions, sessions</li><li>dictionaries, settings, fee structures and rates, business calendar, form fields</li><li>the 8 reports (06 §2.5)</li><li>audit</li></ul> | R1 | The admin can run the office without the JBoss admin UI |
| **R10** | Compatibility layer, parallel run, cut-over | <ul><li>`/lrs/api` subset (03 §9.1): Authentication, WS-Security, DomainModelService HQL reads, IntegrationService.SaveEntity, `/lrs/upload`, `?wsdl` + `Date` header</li><li>parallel run per transaction type on live LRS43, with the old Desktop on the old JBoss (D4)</li><li>switch `lr_setting` and the Cadastre's Registry URL</li><li>stop LRSAS last</li></ul> | All | Every type has moved; Cadastre and Metabase work with LRSAS stopped for 2 weeks; rollback plan tested |

## Upgrades (after parity, each in `plagis_ext`, linked to `LRSAdmin` ids)

| # | Upgrade | Links to |
|---|---|---|
| U1 | Online payment / Remita and bank reconciliation, with receipts issued by the system (today they are typed in by hand) | `lr_payment`, `lr_payment_transaction`, fee rules |
| U2 | E-mail/SMS notifications at stage changes | `lr_process_history` events |
| U3 | Applicant status portal (public, read-only) | transaction number, stage |
| U4 | Charting & reporting step (docs/charting-and-reporting.docx) feeding the R of O stage | Cadastre job, transaction |
| U5 | Dashboards and KPIs (time in stage, backlog per unit), alongside Metabase | process history |
| U6 | Search index (transaction, property, UPIN, name) | read model |
| U7 | Document OCR and full-text search | Alfresco content |
| U8 | Data-quality tools: UPIN clean-up queue, duplicate parties, mis-categorised dictionary rows (fixed only with PLAGIS approval) | 01 §6 |

## Test strategy

- **Golden files:** pick about 50 historic files per main type. The test harness checks that, on `LRS43_COPY`, the replica produces the same rows the old server produced: ids, numbers, statuses, history, fees.
- **Side-by-side:** during R10, every file worked in the new system is checked overnight by a report that compares its rows against the rules in 02 §9.
- **No live writes before R10.** Live reads, for screenshots and comparison, only with PLAGIS ICT's approval.
