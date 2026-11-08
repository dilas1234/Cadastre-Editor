# 08 — What still has to be captured from .5 before the build

Everything in 02–07 comes from what is already captured (the 2026-11-02 LRS export, the live WSDLs, the
Registry Desktop client copied from a workstation, the JBoss config). The items below are **not** in those
captures. Each one is needed to replicate a specific part exactly. All are **read-only** copies from
10.10.10.5 (or a Registry workstation); none changes the Registry.

Priority: **P1** = blocks a build phase, **P2** = needed before parallel run, **P3** = nice to have.

## A. Database (LRS43, schema LRSAdmin) — read-only SELECTs

| # | Pri | What | Why | How |
|---|---|---|---|---|
| A1 | P1 | ✅ **Done 2026-11-06** (`export-registry-workflow.sh`, see 14). **jBPM process definitions**: `JBPM_PROCESSDEFINITION`, `JBPM_NODE` (807), `JBPM_TRANSITION` (1,081), `JBPM_TASK` (582), `JBPM_SWIMLANE` (351), `JBPM_ACTION`, `JBPM_EVENT`, `JBPM_DELEGATION`, `JBPM_MODULEDEFINITION`, and the `processdefinition.xml` bytes in `JBPM_BYTEBLOCK` / `JBPM_BYTEARRAY` | The exact stages, transitions and role (swimlane) of every workflow. Without these the workflow engine is rebuilt from the desktop configs only | Extend `lrs-export.ps1` with these tables (no personal data in them) |
| A2 | P1 | ◐ Counts done 2026-11-06 (route and open-task counts, no actors); the per-file history and jBPM instances are still to do. `lr_process_history` (976,928 rows) without the actor-name column; `JBPM_PROCESSINSTANCE`, `JBPM_TOKEN`, `JBPM_TASKINSTANCE` (no actor names), `JBPM_VARIABLEINSTANCE` names + types (values only for ids) | Where each live transaction is in its workflow; needed to migrate in-flight work and to show history | Same kit, columns listed in 02 §10 |
| A3 | P1 | `lr_fee_rate` (29,003 rows, all columns) and `lr_ground_rent` (no names) | Fee calculation and ground-rent cannot be reproduced without them | Same kit |
| A4 | P1 | Non-personal columns of `lr_rrr`, `lr_source_document`, `lr_transaction_record`, `lr_transaction_party`, `lr_party` (ids/types/dates only), `lr_site_analysis`, `lr_document_ext`, `lr_rental_amount_revision` | The core rights and document model; used to test the replica's reads against real shapes | Same kit (02 §10 lists columns) |
| A5 | P1 | Alfresco: `alf_qname` (all), `alf_namespace`, `alf_store`, `alf_content_url` (urls + sizes only), `alf_node` / `alf_node_properties` for one sample transaction | How a document row reaches its file in `D:\LRS_STORAGE`; needed for viewing scans and generated certificates | Same kit, or a one-transaction sample |
| A6 | P2 | `alf_authority` (188) + `alf_access_control_entry` / `alf_permission` / `alf_authority_alias` | Groups, roles and who holds which permission (lrsPostponeRegistration, Web Access …) | Same kit (user names only) |
| A7 | P1 | Current `lr_setting` and `lr_setting_backup_trial`, and `lr_simplecounter` at the moment of capture | The settings changed on 2026-11-04; counters must be re-read just before the replica writes | `bash tools/aumentum-inventory/registry-settings.sh check` (read-only) |
| A8 | P2 | `Cashier` database: `Parameters`, `InventoryType`, `InventoryTypeAccount`, `v_FeeSchedule` | Fee schedule behind `CashierService.CalculateFee` | Read-only export |

## B. Files on 10.10.10.5

| # | Pri | What | Path | Why |
|---|---|---|---|---|
| B1 | P1 | **Web Access UI** — JSP, JS, CSS, images, help. *Partly done 2026-11-06: the client JS (ExtJS 3.3, `js/*.js`), CSS and images were saved from a read-only visit into `tools/aumentum-inventory/lrs-capture/lrswa-web/`. Still missing: the JSP sources, `welcome.do` output and the `detail.do` / `details/*.do` fragments* | `C:\LRSAS\server\default\deploy\lrswa.war\**` (except `WEB-INF\lib` already copied) | The Web Access pages themselves (only `web.xml`, properties and two model jars were captured) |
| B2 | P1 | Web Access Spring config and controllers | `lrswa.war\WEB-INF\*.xml` (`LrsWebClient-servlet.xml`, `applicationContext-security.xml`, `applicationContext-hibernate.xml`), `WEB-INF\classes\com\**`, all of `WEB-INF\lib` | URL → controller map (`*.do`), login and permission rules |
| B3 | P1 | Remaining `lrs.war\WEB-INF\lib\*.jar` (only 21 copied), `WEB-INF\*-context.xml`, `classes\alfresco\extension\**`, `faces-config-*.xml`, JSP/XHTML | Server behaviour of `AcceptApplication`, `Withdrawal`, `GenerateNumber`, saveJob, mail, JMS, scheduler, cashier task names |
| B4 | P1 | **Report templates** (`*.rptdesign`, `Border.png`, `CertificateWatermark.png`) and report bootstrap XML | Alfresco Company Home → reports (WebDAV `http://10.10.10.5:8080/lrs/webdav/`, read-only) or the jars | R of O, C of O, letters, demand notices — must print identically |
| B5 | P2 | IIS `applicationHost.config`, `ILSCashSrv` folder + `Web.config`, Cashier web app pages (`*.aspx`, `App_Code`, `LRS.Client.ServiceProxy.dll`), `Aumentum.Registry.Cashier.Scheduler.exe.config` | Cashier screens and fee service (mask credentials) |
| B6 | P2 | JBoss logs: last week of `localhost_access_log.*` and `server.log` | Which Web Access URLs and SOAP calls are really used, by whom (masked) |
| B7 | P3 | Metabase ("PLAGIS Analytics", port 3000) data-source list | Whether dashboards read LRS43 directly — they must keep working on the same DB |

## C. From a Registry workstation

| # | Pri | What | Why |
|---|---|---|---|
| C1 | P1 | The full `bin\WorkflowConfig\` folder, including `SubsequentRegistration\*.config` (26 files captured as 0 bytes) and `Lease.config` / `ReleaseLease.config` (referenced, not captured) | Per-transaction stage pages |
| C2 | P2 | `LRS.Data.Controls.dll` and any assembly missing from the capture; `WorkstationHelp.htm` | Map form and help topics |
| C3 | P1 | **Screenshots** (Web Access search screens done 2026-11-06 → `screens/real/web/`; results, details, history and document viewer still to do; the Workstation needs a Registry PC) of every page type in 05 §9 (queue, add transaction, properties, parties, payment, print documents, complete/assign task, scanning, map, search) for at least one R of O and one C of O file | Screen layout and captions; the DB gives fields, not layout |
| C4 | P1 | A **screen recording** of one file walked from lodgement to C of O (like the charting videos) with the officers naming each step | Confirms the stage order and who does what |

## D. Wire samples (read-only SOAP calls with the existing `admin` connection)

| # | Pri | What | Why |
|---|---|---|---|
| D1 | P1 | One `startSession`, one `loadObject` with `@className` (entity XML), one `ResultScalar`, one SOAP fault for an expired ticket, one `PUT /lrs/upload` response | Byte-compatible replies (03 §10) |
| D2 | P2 | `GET /lrs/cxfapi/` service list | Whether CXF endpoints are live |

## E. Questions for PLAGIS (answers in writing)

1. Which roles/units do each stage today (Reception, Search, Lands, Survey, Planning, Legal, Registrar, Governor's office, Title deed unit, Cashier)? Who signs what?
2. Which transaction types are still used (02 §8 has counts; types with 0 in five years can be deferred)?
3. Is Web Access used by anyone outside PLAGIS (banks, lawyers, the public)? Should guest access stay off?
4. Payments: are receipts always typed in by hand? Is a bank/Remita link wanted (upgrade)?
5. Which certificates/letters must keep the exact layout (C of O, R of O, demand notice, acknowledgement)?
6. Meaning of `lr_mc_job.job_status` 2/3/5/7 and job ids 5/9 (Cadastre side).
7. Postponed files (133) and "Postpone Clerk": still a real role?
