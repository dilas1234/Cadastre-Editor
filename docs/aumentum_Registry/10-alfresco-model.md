# 10 - The Alfresco model inside the Registry (documents, queue, users, permissions)

The LRS server (`lrs.war`) is **Alfresco 3.4.9 Enterprise** (build 664, schema 4219) customised by ILS
(`lrs.war/WEB-INF/classes/alfresco/version.properties`). Every scanned page, the Desktop work queue, the
mirror of every jBPM process and task, the users, groups and named permissions, and the BIRT report
templates are stored as Alfresco nodes. Those nodes live in the `alf_*` tables of LRS43 (schema `LRSAdmin`),
and the files live on `D:\LRS_STORAGE`. This document explains that model so the replica can read the same
data from the same database and disk.

It does not repeat:

- the table counts and roles in 02 §6.3;
- the SOAP operations in 03 §3.3 and §4.3;
- the scanning screens in 05 §2.8;
- the server layout in 06 §2.6–2.7.

It links to them instead.

> **Correction to earlier documents.** 02 §6.3 says (marked inferred) that an uploaded file "creates an
> alf_node in `lrs://rootStore`". This is wrong. Scans live in **`workspace://SpacesStore`** under
> `/app:company_home/cm:dss_content/cm:scan_documents`. `lrs://rootStore` holds only the queue and lodgment
> nodes and the permission nodes. Both the Desktop (`ScanServiceAgent.Initialize`) and the server
> (`IntegrationServiceImpl.DSS_STORE_PATH` / `DSS_CONTAINER_PATH`) confirm this (§5).

## Source abbreviations used below

| Tag | Source |
|---|---|
| NS | `$D/net/ThomsonReuters.Aumentum.Registry.Services.dll/…/WebService.Common/NameSpaces.cs`: every QName the Desktop uses |
| RQN | `…/WebService.Repository/RepositoryQueryNames.cs`, `AlfrescoTypes.cs` |
| DP | `$D/net/ThomsonReuters.Aumentum.Registry.DataModel.dll/…/Scan/DocumentProcessor.cs`, `PageItem.cs`, `ScannedDocument.cs` |
| ISLD | `$D/net/ThomsonReuters.Aumentum.Registry.Controls.dll/…/Storage/ImagerStorageLrsDriver.cs`, `LrsImageListAddress.cs` |
| ICDM | `javap` of `com.ils.lrs.model.ICoreDomainModel` in `lrs-web.jar` (constants used by the server's `TransactionBean`) |
| CDM | `javap` of `com.ils.lrs.model.CoreDomainModel` in `lrs-domain-model.jar` (older/alternative constant set; holds the Employee aspect) |
| WFM | `javap` of `com.ils.sdms.workflow.model.WorkflowModel` in `ils-workflow.jar` |
| MFS | `unzip -p ils-core.jar com/ils/core/repo/content/multiFileStoresModel.xml`. This is the **only full model XML captured**, so types and mandatory flags are confirmed |
| RPT | `javap` of `com.ils.sdms.reports.model.ReportsModel` / `ScheduledReportsModel` in `ils-reports.jar` |
| TB | `javap` of `com.ils.lrs.bean.TransactionBean` in `lrs-web.jar`: the server code that builds the queue nodes |
| ISI | `javap` of `com.ils.lrs.integration.service.IntegrationServiceImpl` in `lrs-mc-integration.jar`: server-side document creation, used by the Cadastre upload |
| PMB / UMB / WMB | `javap` of `PermsMngBean`, `UserMngBean`, `WorkflowMngBean` in `lrs-web-admin.jar` (admin UI) |
| INV | `$R/lrs-inventory-REGISTRY-20261101-1701.txt`: row counts, plus the first 100 rows of the small `alf_*` tables |
| DDL | `$R/lrs-export/registry/schema/lrs-tables.sql`, `lrs-indexes.sql` |
| FF | `$R/lrs-export/registry/data/lr_formfield.psv.gz` |
| REGADMIN | `tools/registry-admin/plagis_admin_procedures.sql` in this repo. These are the user procedures already run on the live server: CRC32, MD4 and the node layout of users were verified against real rows |
| RP | `$R/lrs-export/registry/app/LRSAS/server/default/conf/ils/lrs/repository.properties`: the live overrides |

Status column in the tables:

- **C** = confirmed: the exact QName string appears in code that runs at PLAGIS, or in captured DB rows.
- **I** = inferred: the type is deduced from how the code reads or writes the value. It still has to be
  confirmed from the model XML (gap G1).

Only the multi-file-store model (MFS) has its XML. The model files for the lrs, wf, perm, rpt and srpt
namespaces were **not** captured. They are in jars or `alfresco/extension` files that were not copied (§11).

---

## 1. What Alfresco does inside the Registry

### 1.1 Roles

| Role | Alfresco feature used | Notes |
|---|---|---|
| Document storage (scanned pages, attachments, thumbnails, annotations, signed outgoing documents) | `cm:content` nodes plus the ILS multi-file-store content service | §3.3, §5 |
| Work queue of the Desktop | `lrs:Transaction` nodes in `lrs://rootStore`, searched with **Lucene** | §6 |
| Mirror of the jBPM engine | `wf:processDefinition` / `wf:processInstance` / `wf:taskRef` / `wf:comment` nodes in `workflow://rootStore`, kept in step by ILS `WorkflowService` | §8 |
| Users, groups, roles | Alfresco `usr:user` (credentials), `cm:person`, `cm:authorityContainer`, and the `alf_authority` table | §7 |
| Named permissions (`lrs*`, Web Access) | `perm:permission` nodes with Alfresco ACLs | §7.4 |
| Report templates (BIRT) and produced reports | `rpt:*` nodes in `reports://reportsStore`, and `srpt:*` nodes in `reports://scheduledReportsStore` | §3.6 |
| Letter and document templates | `/app:company_home/cm:dss_content/cm:doc_templates` (`lrs/lrs-config.properties` `documents.root.folder.path`) | |
| Report resources (images such as the watermark) | `/app:company_home/cm:report_content/cm:report_resources` (`ReportServiceAgent`) | |
| Version history of scans | `cm:versionable` plus `workspace://version2Store` | §10 |
| Locks while a user works a task | `cm:lockable` aspect set by `AuthoringService.lock` | §10 |
| Full-text and property search | Lucene indexes in `D:/LRS_STORAGE/lucene-indexes` | §9 |

### 1.2 The stores (`alf_store`, all 10 rows: INV)

| id | Store | Root node | What is in it (and source) |
|---:|---|---:|---|
| 1 | `user://alfrescoUserStore` | 1 | `usr:user` credential nodes under `/sys:system/sys:people`: user name, password hash, enabled/locked flags. `repository.properties` `alfresco_user_store.*`; REGADMIN `fn_login_node` |
| 2 | `system://system` | 5 | system descriptor (`sys:descriptor`, schema/version), Alfresco internals (`repository.properties` `system.store`) |
| 3 | `workspace://lightWeightVersionStore` | 11 | deprecated version store (v1). `version.store.deprecated.lightWeightVersionStore`; probably empty or legacy (inferred) |
| 5 | `workspace://version2Store` | 15 | **version history** of `cm:versionable` nodes (scanned documents and pages when versioning is on). `version.store.version2Store` |
| 6 | `archive://SpacesStore` | 16 | **deleted** nodes of SpacesStore (Alfresco "trash"). `spaces.archive.store` |
| 7 | `workspace://SpacesStore` | 17 | company home (`app:company_home`) with `cm:dss_content/cm:scan_documents` (**all scans**), `cm:doc_templates`, `cm:report_content/cm:report_resources`, the Data Dictionary (`app:dictionary`, with `app:models` and `app:workflow_defs`), plus `/sys:system/sys:people` (`cm:person`), `/sys:system/sys:authorities` (groups) and `/sys:system/sys:zones`. Sources: `repository.properties`, `ScanServiceAgent`, `AdministrationServiceAgent.GetPersonNode`, `AuthorityDictionaryProvider` |
| 8 | **`lrs://rootStore`** | 538 | `sys:rootContainer` holding `lrs:tempLodgments`, `lrs:permanentLodgments` (lodgment nodes and **`lrs:Transaction` queue nodes**) and `lrs:permissionsRoot` (`perm:permission` nodes). `lrs/lrs-model.properties` |
| 9 | `reports://reportsStore` | 552 | `rpt:container` with `rpt:reportTemplate`, `rpt:reportResource`, `rpt:reportParam*`, `rpt:reportDocument` (RPT) |
| 10 | `reports://scheduledReportsStore` | 554 | `srpt:scheduledReportTask` and related nodes (RPT) |
| 11 | **`workflow://rootStore`** | 556 | `wf:container`: `wf:processDefinition` → `wf:processInstance` → `wf:taskRef` → `wf:comment` (WFM, `WorkflowModel.PROTOCOL_WORKFLOW` = `workflow`, `PROTOCOL_ROOT_STORE` = `rootStore`) |

There is no store id 4 (a dropped store, inferred). The custom stores (8–11) are created by ILS patches
(`ils/bootstrap/bootstrap-filestores.properties`: `patch.rootStoresPermissions` "Ensures the allowed access to
'reports', 'workflow' and 'lrs' root stores"). Root node ids 538–556 show they were created right after the
2014-11-25 bootstrap.

### 1.3 AVM

AVM is Alfresco's web-content store (`avm_*`, 10 tables). It is **unused**. INV holds one store, `sitestore`,
with 3 directory nodes, all created on 2014-11-25 by System, and 0 rows in `avm_node_properties`. The replica
can ignore all `avm_*` tables.

### 1.4 Other Alfresco tables the replica meets

| Table | Rows (INV) | Meaning |
|---|---:|---|
| `alf_transaction` | 1,423,378 | Alfresco's own commit log. `alf_node.transaction_id` points here, and `commit_time_ms` drives **Lucene index tracking** (§9). Not Registry transactions |
| `alf_lock` / `alf_lock_resource` | 59 / 59 | the **JobLockService** (cluster locks for background jobs): `alfrescojbpmjobexecutor`, `contentstorecleaner`, `nodecleanup`, `usersessionslock`, `messenger1..53`, … All are `not-locked`. These are **not** the task locks (those are `cm:lockable`, §10) |
| `alf_server` | 3 | servers seen: 10.10.10.5, 127.0.0.1, 169.254.14.225 |
| `alf_encoding` | 2 | `utf-8`, `utf8` |
| `alf_locale` | 4 | `en_US_`, `.default`, `fa_IR_`, `ps_AF_` (Afghanistan leftovers of the ILS product) |
| `alf_mimetype` | 11 | `text/xml`, `text/plain`, `text/html`, `application/x-javascript`, `application/png`, `application/octet-stream`, `image/jpeg`, `image/tiff`, `image/png`, `application/pdf`, docx |
| `alf_audit_*` | 1 app, 3 models, 0 entries | auditing is configured but nothing is recorded (`audit.alfresco-access.enabled=false`) |
| `alf_applied_patch` | 174 | Alfresco and ILS patches applied |
| `alf_prop_*`, `alf_attributes` | ~0 | attribute service. Only the tagging-service marker is present |
| `sdms_bootstrap` | 192 | ILS bootstrap history, see 02 §6.4. `contentdata-updates.xml`, `node-properties-index.xml` and `lr-root-property.xml` (`domainModelDDL`, 2014-11-25) are ILS DDL against the Alfresco tables. They probably added `CONTENT_URL_REF_IDX` on `alf_content_data` and `ACT_PERS_TYPES_IDX` on `alf_node_properties` (DDL indexes; the link is inferred) |

---

## 2. Namespaces and prefixes

All 19 rows of `alf_namespace` are known (INV). A namespace appears in that table only once a QName in it has
been **persisted**. So a namespace that is missing, for example `bpm:`, means no node, property or association
of that namespace was ever stored.

| `alf_namespace.id` | Prefix | URI | Owner | What uses it at PLAGIS |
|---:|---|---|---|---|
| 1 | `sys` | `http://www.alfresco.org/model/system/1.0` | Alfresco | store roots, `sys:children`, `sys:base`, `sys:container`, descriptor |
| 2 | `usr` | `http://www.alfresco.org/model/user/1.0` | Alfresco | `usr:user`, `usr:username`, `usr:password`, `usr:enabled`, `usr:accountLocked`, `usr:accountExpires`, `usr:credentialsExpire` (alf_qname 5–11) |
| 3 | `sec` (inferred) | `http://www.alfresco.org/model/security/1.0` | Alfresco | permission group `All` (alf_qname 21) |
| 4 | `ver` | `http://www.alfresco.org/model/versionstore/1.0` | Alfresco | lightweight version store root |
| 5 | **`mtfs`** | `http://www.landsystems.com/sdms/model/multifs/1.0` | ILS | file-store clusters and file stores (MFS) |
| 6 | `ver2` | `http://www.alfresco.org/model/versionstore/2.0` | Alfresco | version2Store |
| 7 | `cm` | `http://www.alfresco.org/model/content/1.0` | Alfresco | content, folder, person, authorityContainer, auditable, lockable, versionable, name, title |
| 8 | `app` | `http://www.alfresco.org/model/application/1.0` | Alfresco | `app:company_home`, `app:uifacets`, `app:icon` |
| 9 | **`wf`** | `http://www.landsystems.com/sdms/model/workflow/1.0` | ILS | jBPM mirror (WFM). Note this is **not** Alfresco's `wf:` (`http://www.alfresco.org/model/workflow/1.0`), which is absent |
| 10 | `rule` | `http://www.alfresco.org/model/rule/1.0` | Alfresco | rules on folders (stock) |
| 11 | `act` | `http://www.alfresco.org/model/action/1.0` | Alfresco | actions (stock) |
| 12 | `st` | `http://www.alfresco.org/model/site/1.0` | Alfresco | `st:sites` container (stock, unused) |
| 13 | `emailserver` | `http://www.alfresco.org/model/emailserver/1.0` | Alfresco | `emailserver:aliasable` (stock) |
| 14 | `trx` | `http://www.alfresco.org/model/transfer/1.0` | Alfresco | transfer (stock) |
| 15 | — | `.empty` | Alfresco | the empty namespace |
| 16 | **`lrs`** | `http://www.landsystems.com/lrs/model/core/1.0` | ILS/LRS | lodgments, queue `Transaction`, scanned documents and pages, Employee and Signature aspects |
| 17 | **`perm`** (prefix inferred) | `http://www.landsystems.com/permission/model/1.0` | ILS/LRS | named permissions (`perm:permission`, `perm:permissions`, `perm:strategy`, `perm:permissionStore`) |
| 18 | **`rpt`** | `http://www.landsystems.com/sdms/model/reports/1.0` | ILS | report templates (`ReportsModel.REPORTS_MODEL_PREFIX` = `rpt`) |
| 19 | **`srpt`** | `http://www.landsystems.com/sdms/model/scheduledReports/1.0` | ILS | scheduled reports (`SCHED_REPORTS_MODEL_PREFIX` = `srpt`) |

Namespaces that are **not** present: `bpm:` (`http://www.alfresco.org/model/bpm/1.0`), Alfresco's own `wf:`,
`wcm`, `wca`, `fm` (forum) and `dl` (data lists). So **no Alfresco advanced-workflow task data was ever
persisted**. The stock `wfa:*` process definitions are deployed but were never run (§8). The old ILS
namespace `http://www.landsystems.com/dss/model/core/1.0` is used only as a constant in
`ConformContentStorageWithModelPatch` (`lrs-domain-model.jar`) and was never stored.

Prefix ↔ URI mapping in `alf_qname`: `alf_qname(id, ns_id → alf_namespace.id, local_name)`. Only the first 100
of 278 rows are captured. Useful ids from them: 4 `sys:children`, 19 `sys:name`, 34 `cm:folder`,
35 `cm:auditable`, **42 `wf:processDefinition`**, 43 `cm:contains`, **44 `cm:content`**, 48 `cm:cmobject`,
49 `sys:base`, 50 `cm:person`, 52 `cm:userName`, 55 `cm:email`, 56 `cm:lastName`, 58 `cm:firstName`,
94 `cm:authorityContainer`, 95 `cm:authorityName`, 96 `cm:zone`, 97 `cm:inZone`, **98 `cm:member`**,
23–32 the `mtfs:*` names. **Never hard-code these ids**: resolve every QName by `(uri, local_name)` at start-up
(§5.4). Ids 101–278, which hold most `lrs:*` and `wf:*` property names, are gap G2.

---

## 3. Custom types, aspects, properties and associations

Data types are Alfresco dictionary types (`d:text`, `d:long`, `d:int`, `d:boolean`, `d:datetime`,
`d:content`, `d:noderef`). "Multi" = multi-valued. "Idx" = searched with Lucene somewhere in the code (so
indexed). Alfresco indexes every property by default unless the model says otherwise, so "—" means only that
no query was seen.

### 3.1 Lodgments and the queue (`lrs:`, store `lrs://rootStore`)

Node layout (TB constructor `(tempPath, permPath)`, `lrs/lrs-model.properties`, `TransactionBean.AssociateEntityWithProcessInstance`):

```
lrs://rootStore  (root)
└── sys:rootContainer
    ├── lrs:tempLodgments        lrs:lodgmentsStore (inferred type)   [lodgments of files still in workflow]
    │   └── lrs:Lodgment  (assoc lrs:lodgments; aspect lrs:TempLodgment)   lrs:lodgmentId = lr_batch.id
    │       └── lrs:Transaction  (assoc lrs:transactions; aspect lrs:TempTransaction, cm:lockable when locked)
    ├── lrs:permanentLodgments   [lodgments after FinalizePermanentRegistration (inferred move)]
    └── lrs:permissionsRoot      perm:permissionStore → perm:permission nodes  (§7.4)
workflow://rootStore  wf:processInstance ──lrs:lodgmentRefs (secondary child, aspect lrs:LodgmentRef)──► lrs:Lodgment
```

`AssociateEntityWithProcessInstance(processNodeRef, batch)`:

1. Creates `lrs:Lodgment` under the temp container with `lrs:lodgmentId`.
2. `addChild(processNode, lodgment, lrs:lodgmentRefs)`. This is why the Desktop finds the lodgment with
   `./*[subtypeOf('lrs:Lodgment')]` under the process node (`RepositoryServiceAgent.GetLodgmentIDByProcess`).
3. For each `lr_transaction` of the batch, creates an `lrs:Transaction` child (assoc `lrs:transactions`) and
   copies the transaction fields listed below.
4. Copies the open task fields from `*/.[subtypeOf('wf:taskRef') and not (@wf:taskEndDate)]` of the process
   (`mapTask2TrnProps`).

From then on, policies on `wf:taskRef` and `wf:processInstance` (`onCreateNode`, `onUpdateProperties`,
`beforeDeleteNode`) and `onRemoveAspect(cm:lockable)` keep the queue node in step
(`SynchronizeWithTaskInstance`, `SynchronizeWithProcessInstance`, `duplicateTransaction`,
`isTransactionByTaskIdExist`). **There is one `lrs:Transaction` node per transaction per open task**
(inferred from `duplicateTransaction`/`isTransactionByTaskIdExist`).

Whether a lodgment's primary parent sits in `lrs://rootStore` (as `lrs-model.properties` says) or in the
workflow store is not proven. Alfresco normally refuses child associations across stores. Gap G6 checks it
with one SELECT.

#### Type `lrs:Lodgment` (subtypes `lrs:RegLodgment`, `lrs:CadLodgment`, `lrs:OrLodgment`)

| Property / assoc | Type | Multi | Idx | Meaning | Source | Status |
|---|---|---|---|---|---|---|
| `lrs:lodgmentId` | d:long | no | yes (xpath) | `lr_batch.id` | NS `PROP_LODGMENT_ID`, ICDM `PROP_LODGEMENT_ID` | C (name), I (type) |
| `lrs:lodgment_id` | d:long? | no | yes (`@lrs\:lodgment_id:` in TB `LODGMENT_ID_QUERY_PART`) | snake-case twin in CDM. May be legacy | CDM, TB | I: which of the two holds data is gap G2 |
| `lrs:lodgmentNumber` / `lrs:lodgment_number` | d:text | no | — | lodgment number | ICDM / CDM | I |
| `lrs:lodgmentDetails` | d:text | no | — | serialized details | ICDM, TB | I |
| `lrs:project_number`, `lrs:lodgment_no_ext`, `lrs:lodgment_ext_date` | text/date | no | — | other-country fields (`patch.projectNumberPatch`) | CDM | I, probably empty at PLAGIS |
| assoc `lrs:transactions` (child) | → `lrs:Transaction` | many | | transactions of the lodgment | ICDM `ASSOC_LODGEMENT_TRANSACTIONS`, TB | C |
| assoc `lrs:lodgments` (child, from container) | `lrs:lodgmentsStore` → `lrs:Lodgment` | many | | | ICDM `TYPE_LODGMENTS_STORE`, `ASSOC_LODGEMENTS` | C |

| Aspect | Properties | Source | Status |
|---|---|---|---|
| `lrs:TempLodgment` | `lrs:permanentLodgment` (d:noderef to the permanent copy) | ICDM `ASPECT_TEMP_LODGEMENT`, `PROP_TLODGEMENT_PERM_REF` | C (name), I (type) |
| `lrs:LodgmentRef` (on `wf:processInstance`) | child assoc `lrs:lodgmentRefs` → `lrs:Lodgment` | ICDM `ASPECT_LODGEMENT_REF`, `ASSOC_LODGEMENT_REFS`; NS `LODGMENT` = `lrs:taskLodgment` (an older name, unused) | C |

#### Type `lrs:Transaction` (the queue row)

Properties come from the `lr_transaction` row (and `_ext` rows), resolved by TB through Hibernate entity
attributes. The attribute is shown in brackets.

| Property | Type | Multi | Idx | Filled from | Source | Status |
|---|---|---|---|---|---|---|
| `lrs:transactionId` | d:long | no | **yes** (`@lrs\:transactionId:`) | `lr_transaction.id` | NS, ICDM, TB; `AssessWorker` query | C |
| `lrs:transactionNumber` | d:text | no | yes (xpath `NODE_TRANSACTION_BY_NUMBER`) | [TransactionNumber] e.g. PLS000138850 | NS, ICDM, TB, FF `ShellFormQueue` | C |
| `lrs:transactionType` | d:text | no | yes (TaskFilter) | [TransactionType.Code] (`lr_transaction_metadata` code) | NS, ICDM, TB, FF (`lrs:TransactionMetaData`) | C |
| `lrs:applicantName` | d:text | no | yes | applicant name(s) | NS, ICDM, TB, FF | C |
| `lrs:applicantName2` | d:text | no | — | second applicant | ICDM | I |
| `lrs:invoiceNumber` | d:text | no | — | [InvoiceNumber] | NS, ICDM, TB | C |
| `lrs:sourceRids` | d:text | no | yes | [SourcePropertyIDs] (comma list of property numbers) | NS, ICDM, TB, FF | C |
| `lrs:targetRids` | d:text | no | yes | [TargetPropertyIDs] | ICDM, TB, FF | C |
| `lrs:createDate` | d:datetime | no | yes | [CreateDate] (submission date) | NS `PROP_TRN_ISSUE_DATE`, ICDM, TB, FF (`xsd:dateTime`) | C |
| `lrs:regDate` | d:datetime | no | — | [RegistrationDate] | NS, ICDM, TB | C |
| `lrs:past` | d:boolean | no | — | `TransactionSource.Code = 'past'` | NS, ICDM, TB | C |
| `lrs:isDelivered` | d:boolean | no | — | [IsDelivered] | NS, ICDM, TB | C |
| `lrs:isOverdue` | d:boolean | no | — | overdue flag (scheduler) | NS, ICDM | I |
| `lrs:isRegistered` | d:boolean | no | — | | ICDM | I |
| `lrs:registry` | d:text | no | — | registry code | NS | I |
| `lrs:upin` | d:text | no | — | [Properties.SpatialUnits.Upin] (Cadastre UPIN) | ICDM `PROP_TRANSACTION_UPIN`, TB | C |
| `lrs:volume`, `lrs:folio`, `lrs:volumefolio` | d:text | no | — | [Volume], [Folio], [Volumefolio] | ICDM, TB | C (name) |
| `lrs:blockplot` | d:text | no | — | [Blockplot] | ICDM, TB | C (name) |
| `lrs:generalno`, `lrs:linkedgeneralno` | d:text | no | — | [GeneralNumber], [RelatedNumber] | ICDM, TB | C (name) |
| `lrs:deedCase`, `lrs:filetype`, `lrs:district`, `lrs:region` | d:text | no | — | `.Code` of dictionary fields | ICDM, TB | C (name) |
| `lrs:parentPropertyIDs` | d:text | no | — | [ParentPropertyIDs] | ICDM, TB | C (name) |
| `lrs:oldPropertyId` | d:text | no | — | [OldPropertyId] (old file number) | ICDM, TB | C (name) |
| `lrs:certno` | d:text | no | — | [CertNos] | ICDM, TB | C (name) |
| `lrs:lodgmentnumber` | d:text | no | — | [Lodgmentnumber] | ICDM, TB | C (name) |
| `lrs:transactionDetails`, `lrs:transactionComments` | d:text | no | — | | ICDM | I |
| snake-case twins `lrs:transaction_id`, `transaction_number`, `transaction_type`, `applicant_name`, `invoice_number`, `source_rids`, `issue_date`, `reg_date`, `is_delivered`, `is_overdue`, `is_registered`, `transaction_comments`, `parish`, `group_number`, `servit_type`, `cert_nos`, `reg_page`, `misc_number`, `plan_number` | | | | not used by the PLAGIS Desktop or TB | CDM only | I: probably absent from the data; gap G2 |

Aspect `lrs:TempTransaction`. The Desktop asks the dictionary for `lrs:Transaction` together with the aspects
`cm:lockable` and `lrs:TempTransaction` to build the queue columns (`ShellForm.InitializeMetaInfo`). So the
process and task fields probably sit on this aspect (inferred):

| Property | Type | Idx | Filled from | Source | Status |
|---|---|---|---|---|---|
| `lrs:processId` | d:long | — | `wf:instanceId` = `JBPM_PROCESSINSTANCE.ID_` | NS, ICDM, FF (`xsd:long` "Process ID") | C |
| `lrs:processName` | d:text | — | process definition name ("Workflow") | NS, ICDM, FF | C |
| `lrs:processDescription` | d:text | — | | NS, ICDM | C (name) |
| `lrs:processSuspended` | d:boolean | — | `wf:suspended` ("Postponed" column) | NS, ICDM, TB, FF (`xsd:boolean`) | C |
| `lrs:rootProcessInstance` | d:noderef (inferred) | — | root process node (sub-processes) | NS, ICDM, TB | C (name) |
| `lrs:permanentLodgment` | d:noderef | — | | ICDM `PROP_TRANSACTION_PERM_REF` | I |
| `lrs:taskId` | d:long | — | `wf:taskId` = `JBPM_TASKINSTANCE.ID_` | NS, ICDM, TB map | C |
| `lrs:taskName` | d:text | — | `wf:taskName` (jBPM task name = stage code) | NS, TB map | C |
| `lrs:taskDescription` | d:text | yes (TaskFilter) | `wf:taskDescription` ("Workflow Stage") | NS, TB map, FF | C |
| `lrs:taskCreatedDate` | d:datetime | **sort key** | `wf:taskCreatedDate` | NS, TB map; `ShellForm` sorts on it descending | C |
| `lrs:taskStartDate`, `lrs:taskEndDate` | d:datetime | — | `wf:taskStartDate`, `wf:taskEndDate` | NS, TB map | C |
| `lrs:taskCompleted` | d:boolean | **yes** (`@lrs\:taskCompleted:"false"`) | `wf:taskCompleted` | NS, TB map, `ShellForm` | C |
| `lrs:taskActorRef` | d:noderef | — | person or group node of the actor | NS, ICDM | C (name), I (type) |
| `lrs:taskActorName` | d:text | **yes** | user name, or group name **without** `GROUP_` (`PersonServiceAgent.GetUserGroupList` strips the prefix) | NS, ICDM, `ShellForm`, FF (`lrs:Authority`) | C |
| `lrs:taskActorIsGroup` | d:boolean | — | | NS, ICDM | C (name) |
| `lrs:taskSwimlane` | d:text | **yes** | `wf:taskSwimlane` (jBPM swimlane name = group without prefix) | NS, TB map, `ShellForm` | C |

The aspect `cm:lockable` (`cm:lockOwner`, `cm:lockType`, `cm:expiryDate`) gives the "Locked By" column (FF
`ShellFormQueue.lockOwner`; ShellForm maps `lockOwner` to `{cm}lockOwner`).

Other `lrs:` QNames found in server code, all **not** PLAGIS-relevant (inferred): `lrs:ScheduledTransaction`
(aspect on `wf:processInstance` for "PCA-Schedule Process"; properties `lrs:srcTransactionType`,
`srcTransactionNumber`, `srcInstrumentNumber`, `tgtTransactionType`, `dueDate`; WMB
`GET_ALL_SCHEDULED_TRANSACTIONS_TEMPLS`).

### 3.2 Scanned documents and pages (`lrs:`, store `workspace://SpacesStore`)

Node layout. This is confirmed by DP, ISLD, ISI and `ScanServiceAgent`:

```
workspace://SpacesStore
└── app:company_home / cm:dss_content / cm:scan_documents            ← AgentFactory.ScanAgent.RootNode
    └── lrs:supporting_scanned_document | lrs:outgoing_scanned_document   (assoc cm:contains; child name = the type QName)
        │   lrs:referenceid = lr_transaction.id, lrs:entityid = lr_source_document.id
        └── lrs:scannedimage  (assoc lrs:imagelist; child name lrs:scannedimage)   one per page / attachment
              cm:content → the page file; lrs:thumbnail → JPG; lrs:annotation → LEADTOOLS XML
```

The Desktop sets `NameSpaces.ASSOC_DOCUMENT_LIST = "{cm}contains"` at start-up (`ScanServiceAgent.Initialize`).
So in practice documents hang under the scan folder by `cm:contains`, not by `lrs:documentlist`. The
`lrs:documentlist` association (ICDM `ASSOC_TRANSACTION_DOC_LIST`) from an `lrs:Transaction` is created only
when a `TransactionRef` is passed, and every Desktop call site passes `null` (ScanViewControl, ScanForm,
AssessWorker). So **the link from a DB row to its scan is by property value, not by association**:
`lrs:entityid` = `lr_source_document.id`.

The document class comes from `lr_dictionary` category `scanned_objects`: id 32 `supporting_scanned_document`
"Supporting Document" and id 33 `outgoing_scanned_document` "Outgoing Document". Supporting is used for an
`IncomingDocument`, outgoing for an `OutgoingDocument` (`ScanViewControl`). The `sd_*` / `outd_*` document
types are **not** Alfresco types. They are `lr_dictionary` rows (`document_type`, `outdoc_type`) referenced
by `lr_source_document.document_type`.

#### Type `lrs:scanneddocument` (subtypes `lrs:supporting_scanned_document`, `lrs:outgoing_scanned_document`)

Its parent type is probably `cm:content` or `cm:cmobject`: `NODE_CONTENT_BY_PARENT_AND_TYPE` uses
`subtypeOf('cm:content')`, and documents are archived on delete (inferred).

| Property / assoc | Type | Multi | Idx | Meaning | Source | Status |
|---|---|---|---|---|---|---|
| `lrs:referenceid` | d:long | no | — | `lr_transaction.id` (`ScannedDocument.ParentId`) | NS `PROP_SCANNED_DOCUMENT_PARENT_ID`, ISI | C (name), I (type) |
| `lrs:entityid` | d:long | no | **yes** (`@lrs\:entityid:"{id}"`) | **`lr_source_document.id`** | NS, DP `GetDocumentList`, ISI | C (name; read as `long`) |
| `lrs:entitydata` | d:text | no | — | the whole SourceDocument entity serialised (Desktop `SerializeObjectToString`; server `ModelServiceHelper.Marshall` JAXB XML), so versions can show old metadata | NS, DP, ISI | C (name). Often longer than 1024 characters, then stored as a Java-serialised value (§5.3) |
| `lrs:pageCount` | d:int | no | — | number of pages (the Desktop writes a string of digits) | NS, DP, ISI; patch `ConformContentStorageWithModelPatch` recounts it | C (name) |
| `cm:name`, `cm:auditable` fields | | | | stock | | |
| assoc `lrs:imagelist` (child) | → `lrs:scannedimage` | many | | pages | NS `ASSOC_IMAGE_LIST`, CDM, ISI | C |
| assoc `lrs:documentlist` | `lrs:Transaction` → document | many | | unused at PLAGIS (see above) | NS, ICDM | C (name) |
| `lrs:parent_id`, `lrs:object_type` | ? | | xpath | used only by `RepositoryQueryNames.NODE_CONTENT_BY_PARENT_AND_TYPE`; no caller found | RQN | I (probably dead) |
| aspect `cm:versionable` | `cm:initialVersion=false`, `cm:autoVersion=false` | | | added when `VersioningSupported` (AssessWorker forces it to `true`; `config/ScanVersioningSupported=0` per 05 §2.8, conflicting; check with G7) | DP, ISLD | C |

#### Type `lrs:scannedimage` (one page or attachment)

| Property | Type | Multi | Idx | Meaning | Source | Status |
|---|---|---|---|---|---|---|
| `cm:content` | d:content | no | full text (only if a text transformer exists) | **the page image or attachment**. Written as the `contentUrl=…\|mimetype=…\|size=…\|encoding=…\|locale=…` string returned by `PUT /lrs/upload` | DP (`NameSpaces.TYPE_CONTENT`), ISLD, 03 §2.6 | C |
| `lrs:pageindex` | d:int | no | — | page order (0- or 1-based, inferred) | NS, DP, ISI | C (name) |
| `lrs:pagename` | d:text | no | — | original file name | NS, DP | C (name) |
| `lrs:pagefilesize` | d:long | no | — | bytes | NS, DP | C (name) |
| `lrs:isattachment` | d:boolean | no | — | true = non-image attachment (PDF, docx…) | NS, DP | C (name) |
| `lrs:pageheight`, `lrs:pagewidth`, `lrs:pagebitperpixel`, `lrs:pageformat` | d:int | no | — | written as 0 by `DocumentProcessor` (older path) | NS, DP, ISI | C (name) |
| `lrs:imageinfo` | d:text | **yes** | — | `key=value` list `pageheight=…`, `pagewidth=…`, `pagebpp=…`, `pageformat=<LEADTOOLS RasterImageFormat int>` (ISLD). The reader looks for key `pagebitperpixel` but the writer stores `pagebpp`, an ILS bug, so bits-per-pixel always reads 0 | ISLD `GetImageProperties`, `PageItem.GetPropertyValue` | C |
| `lrs:thumbnail` | d:content (inferred: value is a content-URL string) | no | — | JPG thumbnail (uploaded as `<guid>.jpg`, LEADTOOLS format 10, 24 bpp) | NS, DP, ISLD `UploadThumbnailToContent` | C (name) |
| `lrs:annotation` | d:content (inferred) | no | — | LEADTOOLS annotation container serialised to XML (`AnnCodecs.Save(... format 5 ...)`, uploaded as `<guid>.xml`). Null when there are no annotations | NS, ISLD `UploadAnnotationToContent` | C (name) |
| aspect `cm:versionable` | | | | as on the document | DP | C |

Page count per document: (2,104,102 content URLs) ÷ (364,663 `lr_source_document` rows) ≈ 5.8 URLs per
document. Each page has 2–3 content items (image, thumbnail, sometimes an annotation) plus any versions, so
this suggests about 2 pages per scanned document on average (inferred).

### 3.3 Multi-file-store model (`mtfs:`), the only fully captured model

`ils-core.jar!/com/ils/core/repo/content/multiFileStoresModel.xml`, "Multi Filestores Domain Model", ILS Inc.
2006, version 1.0. Imports only `d:` and `sys:`. **Every value here is confirmed (C).**

Constraints:

- `mtfs:uncname`: REGEX `.*["*><?|\xA3\xAC%&+;]+.*`, `requiresMatch=false`, so a UNC path may not contain those
  characters.
- `mtfs:guidpattern`: REGEX for a lower- or upper-case GUID, `requiresMatch=true`.

| Class | Kind / parent | Property | Type | Mandatory | Notes |
|---|---|---|---|---|---|
| `mtfs:cluster` | type, parent `sys:container` | `mtfs:clusterName` | d:text | yes (enforced) | default cluster name `(default)` (`ils/messages/multiFileStores-service.properties` `mtfs.default_cluster.name`) |
| | | `mtfs:description` | d:text | no | "The default filestores cluster" |
| `mtfs:filestore` | type, parent `sys:container` | `mtfs:storeID` | d:text, constraint `guidpattern` | yes (enforced) | main bootstrap store id `ffffffff-ffff-ffff-ffff-ffffffffffff` (`bootstrap-filestores.properties` `mtfs.bootstrap.storeID`). Code also uses `00000000-0000-0000-0000-000000000000` for the "main store" (`Cluster$FileStore`) |
| | | `mtfs:shareunc` | d:text, constraint `uncname` | yes (enforced) | **root folder of that store** (local path or `\\server\share`) |
| | | `mtfs:freethreshold` | d:long | yes | free-space threshold. When reached the store stops being active |
| | | `mtfs:activestatus` | d:text | yes | `started`, `suspended` (bootstrap), plus `stopped` and `off mode` (messages) |
| | | `mtfs:computerName` | d:text | no | host of the share |
| | | `mtfs:storeDescription` | d:text | no | "The main filestore" |
| `mtfs:spaceCredit` | type, parent `sys:base` | `mtfs:spaceCreditSize`, `mtfs:usedSpaceAtCreate` | d:long | yes | space reservation |
| | | `mtfs:spaceCreditTime` | d:datetime | yes | |
| | | `mtfs:spaceCredit4Folder` | d:noderef | no | |
| | | `mtfs:spaceCreditAbsPath` | d:text | no | |
| `mtfs:assignedCluster` | aspect (on folders) | `mtfs:assignedClusterName` | d:text, default `(default)` | — | which cluster new content under that folder goes to |
| `mtfs:physicalSubFolder` | aspect | `mtfs:folderURL` | d:text | yes | physical folder for a repository folder |
| `mtfs:physicalRootFolder` | aspect, parent `mtfs:physicalSubFolder` | — | | | |

Where the nodes live: a `sys:filestores` container (`repository.properties`
`system.filestores_container.childname=sys:filestores`; patch `filestoresContainer` "Ensures the existence of
the filestores container") under `sys:system`. The store is not proven (gap G5).

How it works (`MultiFileStoresService`, `ContentServiceImpl`, `SmartFileContentStore` in `ils-core.jar`,
javap):

1. **Writing.** `ContentServiceImpl.getStore(nodeRef)` finds the primary parent folder and its assigned cluster
   (`mtfs:assignedCluster`, default `(default)`), then calls `getActiveStore(cluster)`. That returns the
   cluster's file store with status *started* that still has free space. If there is none and
   `redirectToDefault` is on, it uses the default cluster; otherwise it fails with "Cluster '…' has not active
   filestore. Probably disk space has been exceeded". The writer is a `SmartFileContentStore` (a subclass of
   Alfresco's `FileContentStore`) rooted at that store's `mtfs:shareunc`. A background thread
   (`FileStoragesChecker`) checks free space and mails `admin` when a store stops or starts.
2. **Reading.** `getContentStoreByUrl(url)` calls `AbstractContentStore.extractSroreId(url)` (sic). This is an
   **ILS-patched Alfresco class** that is not in the captured jars. If the URL carries no store id, the file is
   read from the **main store** (`dir.contentstore` = `D:/LRS_STORAGE/contentstore`). Otherwise it is read from
   the `mtfs:filestore` whose `mtfs:storeID` matches.
3. So a content URL is either the stock `store://yyyy/M/d/H/m/<uuid>.bin` (main store) or a variant that embeds
   a store id. **The exact form of the embedded store id is not captured** (gap G4). The admin UI ("Add
   Filestore Wizard", "Assign Filestorages Cluster", `fs.*` keys in `alfresco/extension/webclient.properties`)
   shows that PLAGIS *could* have added stores. D: holds 1,573 GB used (06 §1), so check whether a second store
   was ever added.

### 3.4 Workflow mirror model (`wf:` = `http://www.landsystems.com/sdms/model/workflow/1.0`, store `workflow://rootStore`)

All names come from WFM (javap). This is the full constant list of `WorkflowModel`. Types are inferred.

```
workflow://rootStore
└── wf:container  (assoc wf:rootcontainer)
    └── wf:processDefinition  (assoc wf:registeredObj; child name wf:ProcessDef_<JBPM_PROCESSDEFINITION.ID_>)
        └── wf:processInstance  (assoc wf:startedInstances; child name wf:ProcessInst_<JBPM_PROCESSINSTANCE.ID_>)
            ├── wf:taskRef  (assoc wf:containsTasks; child name wf:TaskInst_<JBPM_TASKINSTANCE.ID_>)
            │   └── wf:comment (assoc wf:comments; child name wf:TaskComent_<taskId>)  [sic "Coment"]
            ├── wf:processInstance  (assoc wf:subWorkflows, sub-processes such as Raise Title Deed Plan)
            └── lrs:Lodgment (secondary, assoc lrs:lodgmentRefs; §3.1)
```

The child-name prefixes are confirmed: `WorkflowModel.PROCESS_DEF_PFX`/`PROCESS_INST_PFX`/`TASK_INST_PFX`/
`TASK_COMMENT_PFX` and NS, and `WorkflowServiceAgent.AddComment` uses `TaskComent_` + task id. That the number
after each prefix is the jBPM id is inferred. jBPM process variables `_alfProcessInst`, `_alfTaskInst` and
`_lastActorName` (string constants in `ils-workflow.jar`) probably hold the back-references (inferred).

| Class | Property / assoc | Type (I) | Meaning | Status |
|---|---|---|---|---|
| `wf:container` (type) | assoc `wf:rootcontainer`, `wf:registeredObj` | | root of the store | C |
| `wf:processDefinition` (type) | `wf:defName` | d:text | process name (= `JBPM_PROCESSDEFINITION.NAME_`, e.g. "First Registration") | C |
| | `wf:defDescription` | d:text | | C |
| | `wf:defInitial` | d:boolean? | "initial" flag in the admin UI (WMB) | C (name) |
| | `wf:defReference` | d:long | `JBPM_PROCESSDEFINITION.ID_` (xpath `*[@wf:defReference=…]`) | C (name) |
| | assoc `wf:startedInstances` | | its instances | C |
| | ACL | | **Consumer / Manager** permissions (alf_permission rows 2 and 6 have `type_qname_id` 42 = `wf:processDefinition`). The Desktop lists startable workflows with `getACLs(..., permission "Manager")` (03 §6) | C |
| `wf:processInstance` (type) | `wf:instanceId` | d:long | `JBPM_PROCESSINSTANCE.ID_` (NS `PROP_PROCESS_ID`; xpath `*//.[@wf:instanceId=…]`) | C |
| | `wf:instanceStartedDate`, `wf:instanceCompletedDate` | d:datetime | | C (name) |
| | `wf:instanceState` | d:text | | C (name) |
| | `wf:instanceCompleted` | d:boolean | (`@wf\:instanceCompleted:false`, WMB) | C |
| | `wf:instanceDefName` | d:text | definition name (`@wf\:instanceDefName:"…"`) | C |
| | `wf:suspended` | d:boolean | postponed (copied to `lrs:processSuspended`) | C (name) |
| | `wf:startTaskId` | d:long | | C (name) |
| | `wf:instanceAssocWith` | d:noderef (multi?) | nodes the instance is associated with (`PROCESS_INSTANCE_ASSOC_NODE_VARIABLE_NAME` = `associatedNode`) | C (name) |
| | assoc `wf:basedOn` → processDefinition; `wf:containsTasks` → taskRef; `wf:subWorkflows` → processInstance | | | C |
| `wf:taskRef` (type) | `wf:taskId` | d:long | `JBPM_TASKINSTANCE.ID_` | C |
| | `wf:taskName` | d:text | jBPM task name | C |
| | `wf:taskDescription` | d:text | stage label | C |
| | `wf:taskActorId` | d:text | actor (xpath `*//.[@wf:taskActorId=$wf:query and (not (@wf:taskEndDate))]`) | C |
| | `wf:taskPooledActors` | d:text, multi (inferred) | pooled groups (NS `PROP_TASK_GROUP`; `@wf:taskPooledActors=$wf:query`) | C |
| | `wf:taskUserName` | d:text | assigned user (FF `HistoryViewForm`) | C |
| | `wf:prevtaskUserName` | d:text | user who assigned it ("assigned by") | C |
| | `wf:taskGroupName` | d:text | assigned group | C |
| | `wf:taskProcInstRef` | d:noderef | its process instance (`@wf\:taskProcInstRef:"{ref}"`, `CompletePage`) | C |
| | `wf:taskPriority` | d:int | | C (name) |
| | `wf:taskSwimlane` | d:text | jBPM swimlane = group name without prefix | C |
| | `wf:taskCreatedDate`, `wf:taskStartDate`, `wf:taskEndDate`, `wf:taskDueDate` | d:datetime | | C |
| | `wf:taskCompleted` | d:boolean | (`@wf\:taskCompleted:"false"`) | C |
| | `wf:taskDuration` | d:long | ms or minutes (FF "Duration" `xsd:long`; `HistoryView` formats it specially) | C (name) |
| | assoc `wf:comments` → `wf:comment` | | | C |
| | aspect `cm:lockable` while a user has the task open (§10) | | | C |
| aspect `wf:taskTransition` | `wf:transitionName` | d:text | transition chosen when the task ended (FF `HistoryViewForm.transitionName`) | C |
| aspect `wf:workflowDriven` | `wf:useAsTemplate` (d:boolean); assocs `wf:linkedWorkflowDefinition`, `wf:linkedWorkflow` | | "Apply Workflow Aspect to the current node" (`ils/messages/workflow-service.properties`). Not used by PLAGIS (inferred) | C (name) |
| `wf:comment` (type) | `wf:commentText` | d:text | comment body | C |
| | `wf:commentType` | d:text | `task-comment`, `task-suspend`, `task-resume` (`lr_dictionary` category `task_event`) | C |
| | `wf:commentAcs` | d:text | `task_event_public` / `task_event_private` (`task_event_access`) | C |
| | `cm:creator`, `cm:created` | | who and when (FF `HistoryViewFormEvent`) | C |

### 3.5 People, groups and the Employee and Signature aspects

| Class | Store / path | Property | Type | Meaning | Source | Status |
|---|---|---|---|---|---|---|
| `usr:user` | `user://alfrescoUserStore`, `/sys:system/sys:people` | `usr:username` | d:text | login (lower-case in practice; `user.name.caseSensitive=false`) | alf_qname 11; REGADMIN | C |
| | | `usr:password` | d:text | **32-hex MD4 hash** (NTLM hash = MD4 of the UTF-16LE password). Upper- or lower-case hex, both seen (REGADMIN handles both) | REGADMIN `usp_create_user` (MD4 self-test `HASHBYTES('MD4', N'password') = 0x8846F7EA…`); `authentication.chain=alfrescoNtlm1:alfrescoNtlm` | C |
| | | `usr:enabled`, `usr:accountLocked`, `usr:credentialsExpire`, `usr:accountExpires` | d:boolean | account flags | alf_qname 6–9; REGADMIN | C |
| | | (`usr:salt`) | | REGADMIN copies it if present. Alfresco 3.4 has no salt (inferred), so expect no rows | REGADMIN | I |
| `cm:person` | `workspace://SpacesStore`, `/sys:system/sys:people` | `cm:userName`, `cm:firstName`, `cm:middleName`, `cm:lastName`, `cm:email`, `cm:organizationId`, `cm:jobtitle`, `cm:telephone`, `cm:owner`, `cm:homeFolder`, `cm:homeFolderProvider`, `cm:sizeCurrent` | stock | display data. The Desktop reads `fullName` or first + last | UMB (`ContentModel.PROP_*`), `PersonServiceAgent`, REGADMIN | C |
| aspect **`lrs:Employee`** (on `cm:person`) | | `lrs:employeeNumber` | d:text | "Employee No." | CDM `ASPECT_EMPLOYEE`, `PROP_EMPLOYEE_NO`; UMB; `DataModelUtilities.GetUserEmployeeNumber` | C |
| | | `lrs:jobTitle` | d:text | job title, synchronised with `cm:jobtitle` (`EmployeeAspectSync.onUpdateProperties/onAddAspect`) | CDM, `EmployeeAspectSync` | C |
| | | **`lrs:officeId`** | d:text (REGADMIN reads `string_value`) | the user's **registry office**. The Desktop runs `RegistryOffice where office_name='<officeId>'` (`DomainModelServiceAgent.Initialize`) | CDM, UMB, REGADMIN `usp_update_user` | C |
| | | `lrs:employeeComment` | d:text | "General Comments" | CDM, UMB | C |
| | | `lrs:salutation` | d:text | | CDM, UMB | C (name) |
| | | `lrs:notifyByEmail` | d:boolean | e-mail on task assignment (`WorkflowMessageListener`) | NS `PROP_EMPLOYEE_NOTIFY_BY_EMAIL`, `PersonServiceAgent.setNotifyByEmail` | C (name). Which aspect owns it is not known |
| aspect **`lrs:Signature`** (on `cm:person`) | | `lrs:signatureData` | d:content | the user's **encrypted signature image**, read as a content stream (`AdministrationServiceAgent.GetSignature` / `EnrollSignature` / `RevokeSignature`). Used only if the dictionary defines the aspect | NS `ASPECT_SIGNATURE`, `PROP_SIGNATURE_DATA` | C (name) |
| `cm:authorityContainer` | `workspace://SpacesStore`, `/sys:system/sys:authorities` | `cm:authorityName` (`GROUP_…`), `cm:authorityDisplayName`; member links `cm:member` (child assoc); `cm:inZone` to zones | stock | groups | alf_qname 94–98; `AuthorityDictionaryProvider`; REGADMIN | C |

### 3.6 Reports (`rpt:`, `srpt:`): summary

Full constant lists come from RPT (javap). The replica needs them only to find templates during the
transition (the templates themselves are capture item B4).

| Class | Key properties / assocs | Status |
|---|---|---|
| `rpt:container` (assoc `rpt:rootContainer`, `rpt:registeredReports`) | | C |
| `rpt:reportTemplate` | `rpt:reportId`, `reportName` (e.g. `RejectionLetter.rptdesign`), `reportCategory`, `reportDesc`, `reportTitle`, `reportAuthor`, `reportPageSize`, `reportPageOrientation`, `reportMarginsTop/Bottom/Left/Right` (the left and right constants are swapped in the code), `deployedDate`; assocs `rpt:templateResources`, `rpt:templateParams`, `rpt:producedReports`, `rpt:reportContent` | C |
| `rpt:reportDocument` | `rpt:documentId`, `documentDate`, `documentRunTime`, `documentParams`, `documentOwner`; assocs `rpt:basedOn`, `rpt:documentContent` | C |
| `rpt:reportResource` | `rpt:resLanguage`; assoc `rpt:resContent` | C |
| `rpt:reportParam`, `rpt:reportListParam`, `rpt:reportDateRangeParam` | `paramName`, `paramLabel`, `paramCtrlType`, `paramDefaultValue`, `paramMask`, `paramOrder`, `paramSourceQuery`, `paramDisplayColumn`, `paramValueColumn`, `paramStartDateColumn`, `paramEndDateColumn` | C |
| `srpt:scheduledReportTask` (+ `taskParam`, `taskDateParam`, `taskSubscriber`, `reportDocument`) | `srpt:taskName`, `reportName`, `createdDate`, `lastRunDate`, `nextRunDate`, `schedule`, `locale`, `startDate`, `endDate`, `taskEnabled`, `dynamicDate`, `UnitsAgo`, `numberOfUnitsAgo`, `subscriber` | C |

The Desktop lists reports with `TYPE:"{…reports/1.0}reportTemplate"` in `reports://reportsStore` and reads
`reportName`, `reportTitle` and `reportDesc` (`ReportServiceAgent.LoadAuthorizedReportList`). Report
permissions are ACLs on the template nodes (UMB `reportPermissions`, "Execute"/"Read").

### 3.7 Named permissions (`perm:`, store `lrs://rootStore`)

| Class | Property | Type | Meaning | Source | Status |
|---|---|---|---|---|---|
| `perm:permissionStore` (type of `sys:rootContainer/lrs:permissionsRoot`, inferred) | assoc `perm:permissions` (child) | | container. PMB finds it with `TYPE:"{…permission/model/1.0}permissionStore"` | PMB | C |
| `perm:permission` | `cm:name` | d:text | **permission key**, e.g. `lrsAssignTask` (05 §8 lists all) | PMB `QNAME_CM_NAME`; `AccessControlServiceAgent.Initialize` reads column `name` | C |
| | `cm:title` | d:text (MLText) | "Permission Goal" label | PMB | C |
| | `perm:strategy` | d:text | Straight / View Access / Print Access / Web Access (`perm_strategy_straight`, `_readonly`, `_printonly`, `_web_access` in `webclient.properties`). The stored literal is unknown (G9) | PMB | C (name), I (values) |
| | ACL | | `ROLE_ADMINISTRATOR` → **All**; every selected user or group → **Read** (`PermsMngBean.setUpdateToPerm`) | PMB | C |

### 3.8 Standard Alfresco aspects and properties the Desktop reads or writes

| QName | Used for | Source |
|---|---|---|
| `cm:content` (type and property), `cm:folder`, `cm:contains`, `sys:children` | documents, folders | NS, AlfrescoTypes |
| `cm:name` | node name, permission key | NS `PROP_CONTENT_NAME` |
| `cm:auditable`: `cm:created`, `cm:creator`, `cm:modified`, `cm:modifier`, `cm:accessed` | history ("User", "Date"). Stored in `alf_node.audit_*` columns, **not** in alf_node_properties (Alfresco 3.4) | NS; DDL `alf_node.audit_created nvarchar(30)` |
| `cm:lockable`: `cm:lockOwner`, `cm:lockType`, `cm:expiryDate` | task locks | NS; TB `ContentModel.ASPECT_LOCKABLE` |
| `cm:versionable`: `cm:versionLabel`, `cm:initialVersion`, `cm:autoVersion`, `cm:versionType` | scan versions | NS, DP |
| `cm:referenceable` | node uuid | TB |

---

## 4. Content model diagram

```mermaid
classDiagram
    direction LR
    class cm_content["cm:content"]
    class cm_person["cm:person"]{
      +cm:userName
      +cm:firstName
      +cm:lastName
    }
    class usr_user["usr:user (user://alfrescoUserStore)"]{
      +usr:username
      +usr:password (MD4 hex)
      +usr:enabled
      +usr:accountLocked
    }
    class lrs_Employee["«aspect» lrs:Employee"]{
      +lrs:officeId
      +lrs:employeeNumber
      +lrs:jobTitle
      +lrs:employeeComment
      +lrs:salutation
    }
    class lrs_Signature["«aspect» lrs:Signature"]{
      +lrs:signatureData : d:content
    }
    class cm_authorityContainer["cm:authorityContainer (group)"]{
      +cm:authorityName GROUP_*
    }
    class lrs_scanneddocument["lrs:scanneddocument (SpacesStore /cm:dss_content/cm:scan_documents)"]{
      +lrs:entityid = lr_source_document.id
      +lrs:referenceid = lr_transaction.id
      +lrs:entitydata (serialized entity)
      +lrs:pageCount
    }
    class lrs_supporting["lrs:supporting_scanned_document"]
    class lrs_outgoing["lrs:outgoing_scanned_document"]
    class lrs_scannedimage["lrs:scannedimage"]{
      +cm:content (page file)
      +lrs:thumbnail : d:content
      +lrs:annotation : d:content
      +lrs:pageindex
      +lrs:pagename
      +lrs:pagefilesize
      +lrs:isattachment
      +lrs:imageinfo[]
    }
    class lrs_Lodgment["lrs:Lodgment (lrs://rootStore)"]{
      +lrs:lodgmentId = lr_batch.id
    }
    class lrs_Transaction["lrs:Transaction (queue row)"]{
      +lrs:transactionId
      +lrs:transactionNumber
      +lrs:transactionType
      +lrs:applicantName
      +lrs:sourceRids / targetRids
      +lrs:upin
      +lrs:createDate
    }
    class lrs_TempTransaction["«aspect» lrs:TempTransaction"]{
      +lrs:processId
      +lrs:taskId
      +lrs:taskName / taskDescription
      +lrs:taskActorName
      +lrs:taskSwimlane
      +lrs:taskCompleted
      +lrs:taskCreatedDate
      +lrs:processSuspended
    }
    class cm_lockable["«aspect» cm:lockable"]{
      +cm:lockOwner
      +cm:lockType
      +cm:expiryDate
    }
    class wf_processDefinition["wf:processDefinition (workflow://rootStore)"]{
      +wf:defName
      +wf:defReference = JBPM_PROCESSDEFINITION.ID_
    }
    class wf_processInstance["wf:processInstance"]{
      +wf:instanceId = JBPM_PROCESSINSTANCE.ID_
      +wf:instanceDefName
      +wf:suspended
      +wf:instanceCompleted
    }
    class wf_taskRef["wf:taskRef"]{
      +wf:taskId = JBPM_TASKINSTANCE.ID_
      +wf:taskName
      +wf:taskUserName
      +wf:taskPooledActors
      +wf:taskSwimlane
      +wf:taskCompleted
      +wf:taskProcInstRef
    }
    class wf_taskTransition["«aspect» wf:taskTransition"]{
      +wf:transitionName
    }
    class wf_comment["wf:comment"]{
      +wf:commentText
      +wf:commentType
      +wf:commentAcs
    }
    class perm_permission["perm:permission (lrs://rootStore/lrs:permissionsRoot)"]{
      +cm:name (lrs* key)
      +perm:strategy
    }
    class mtfs_cluster["mtfs:cluster"]{
      +mtfs:clusterName
    }
    class mtfs_filestore["mtfs:filestore"]{
      +mtfs:storeID (GUID)
      +mtfs:shareunc (root path)
      +mtfs:activestatus
      +mtfs:freethreshold
    }
    class rpt_reportTemplate["rpt:reportTemplate (reports://reportsStore)"]{
      +rpt:reportName (*.rptdesign)
    }

    cm_content <|-- lrs_scanneddocument : parent (inferred)
    lrs_scanneddocument <|-- lrs_supporting
    lrs_scanneddocument <|-- lrs_outgoing
    lrs_scanneddocument "1" *-- "many" lrs_scannedimage : lrs:imagelist
    cm_person ..> lrs_Employee : aspect
    cm_person ..> lrs_Signature : aspect
    cm_authorityContainer "1" o-- "many" cm_person : cm:member
    cm_person .. usr_user : same user name
    lrs_Lodgment "1" *-- "many" lrs_Transaction : lrs:transactions
    lrs_Transaction ..> lrs_TempTransaction : aspect
    lrs_Transaction ..> cm_lockable : aspect (when locked)
    wf_processDefinition "1" *-- "many" wf_processInstance : wf:startedInstances
    wf_processInstance "1" *-- "many" wf_taskRef : wf:containsTasks
    wf_processInstance "1" o-- "many" wf_processInstance : wf:subWorkflows
    wf_processInstance "1" o-- "1" lrs_Lodgment : lrs:lodgmentRefs
    wf_taskRef "1" *-- "many" wf_comment : wf:comments
    wf_taskRef ..> wf_taskTransition : aspect (when ended)
    wf_taskRef ..> cm_lockable : aspect (while worked)
    mtfs_cluster "1" *-- "many" mtfs_filestore
```

---

## 5. From a Registry DB row to the file on disk

### 5.1 The chain

```
lr_transaction (id) ── lr_transaction_document ──► lr_source_document (id, Hjtype IncomingDocument/OutgoingDocument)
                                                         │  value match, no FK
                                                         ▼
alf_node_properties (qname lrs:entityid, long_value = lr_source_document.id)
   └── alf_node  (store workspace://SpacesStore, type lrs:supporting_scanned_document | lrs:outgoing_scanned_document)
         └── alf_child_assoc (parent = document node, type lrs:imagelist) ──► alf_node (type lrs:scannedimage), one per page
               └── alf_node_properties (qname cm:content, actual_type_n = 21 CONTENT_DATA_ID, long_value = alf_content_data.id)
                     └── alf_content_data (content_url_id, content_mimetype_id, content_encoding_id, content_locale_id)
                           └── alf_content_url (content_url 'store://yyyy/M/d/H/m/<uuid>.bin', content_size)
                                 └── file  D:\LRS_STORAGE\contentstore\yyyy\M\d\H\m\<uuid>.bin
                                      (or <mtfs:shareunc of the matching mtfs:filestore>\... if the URL carries a store id, §3.3)
```

Notes:

- **No `lr_*` column holds a node ref.** The DDL of `lr_source_document`, `lr_incoming_document`,
  `lr_document_ext` and `lr_transaction_document` has no uuid or URL column. Only the value of `lrs:entityid`
  joins the two worlds. The same is true for the queue (`lrs:transactionId`), lodgments (`lrs:lodgmentId`)
  and workflow (`wf:instanceId`, `wf:taskId`).
- `lr_source_document.page_count` is the DB copy of the page count; `lrs:pageCount` is the node's copy.
- The Cadastre's title-deed-plan upload (`PUT /lrs/upload` followed by `IntegrationService.SaveEntity` with an
  `IncomingDocument` of type `sd_titledeedPlan`) runs `IntegrationServiceImpl.proceedDocumentEntity`. That
  creates the same structure: an `lrs:supporting_scanned_document` under `cm:scan_documents`, then
  `lrs:scannedimage` children, then `VersionService.createVersion` if versioning is on.
- Generated letters (rejection, postponement, withdrawal, R of O, C of O…) are **not stored**. They are
  rendered by BIRT and printed (`RejectPage` → `ReportAgent.ProduceReport` → `BrowserForm.Print`). A signed
  outgoing document is scanned back as an `lrs:outgoing_scanned_document`.

### 5.2 Content-URL to path

Stock Alfresco 3.4 `FileContentStore`, main store:

| Part | Example | Rule |
|---|---|---|
| `content_url` | `store://2019/3/14/10/27/6b0e…-….bin` | protocol `store://`, then year / month / day / hour / minute **without zero padding**, then a uuid `.bin` (Alfresco convention, inferred; confirm with G4) |
| file | `D:\LRS_STORAGE\contentstore\2019\3\14\10\27\6b0e…-….bin` | strip `store://`, replace `/` with `\`, prefix `dir.contentstore` (RP: `${dir.root}/contentstore`, `dir.root=D:/LRS_STORAGE/`) |
| deleted files | `D:\LRS_STORAGE\contentstore.deleted\…` | the orphan cleaner (`system.content.orphanCleanup.cronExpression=0 0 4 * * ?`, `orphanProtectDays=1`) moves unreferenced content here |
| file extension | `.bin` always | mime type comes from `alf_content_data.content_mimetype_id` → `alf_mimetype`, not from the file name |

Any URL that does not start with `store://` or carries an extra store-id segment must be resolved through
`mtfs:filestore` (§3.3). Gap G4 gives the query that shows whether such URLs exist.

### 5.3 How property values are stored (`alf_node_properties`)

This follows Alfresco 3.4 `NodePropertyValue`. The type ids are inferred from Alfresco 3.4 and **must be checked
with G3**. One captured row supports them: `avm_store_properties` holds a date with actual 7 / persisted 6 (a
date persisted as a string) and a string with 6 / 6.

| `actual_type_n` | Meaning | `persisted_type_n` | Column holding the value |
|---:|---|---:|---|
| 1 | BOOLEAN | 1 | `boolean_value` (0/1) |
| 2 | INTEGER | 3 | `long_value` |
| 3 | LONG | 3 | `long_value` |
| 4 / 5 | FLOAT / DOUBLE | 4 / 5 | `float_value` / `double_value` |
| 6 | STRING | 6 (or 9 if longer than 1024 characters) | `string_value` (nvarchar(1024)); **longer strings go to `serializable_value` as Java-serialised bytes** (so `lrs:entitydata` usually needs Java deserialisation, which is why the replica should ignore it) |
| 7 | DATE | 6 | `string_value` ISO-8601 with offset, e.g. `2014-11-25T03:46:12.825-08:00` (captured). **The offset is the JVM timezone at write time** (−08:00 at install), so convert to UTC when reading |
| 9 | SERIALIZABLE | 9 | `serializable_value` |
| 10 | MLTEXT (`cm:title`, `cm:description`) | 6 | `string_value`, one row per `locale_id` |
| 12 | NODEREF | 6 | `string_value` = `workspace://SpacesStore/<uuid>` |
| 15 | QNAME | 3? | id into alf_qname (rare) |
| 21 | CONTENT_DATA_ID (`d:content`) | 3 | `long_value` = **`alf_content_data.id`** |

- **Multi-valued** properties have one row per value with `list_index` 0, 1, 2, …. Single values use
  `list_index = -1` (inferred). `locale_id` points to `alf_locale`; `.default` (id 2) is used for non-MLText.
  The PK is (`node_id`, `qname_id`, `list_index`, `locale_id`).
- Aspects are in `alf_node_aspects(node_id, qname_id)`; the node type is `alf_node.type_qname_id`.
- Child associations are in `alf_child_assoc`: `parent_node_id`, `child_node_id`, `type_qname_id` (assoc
  type), (`qname_ns_id`, `qname_localname`) = the child's association QName, and `is_primary`.
  `child_node_name` is the `cm:name` if there is one; otherwise a generated GUID with a **negated** CRC
  (REGADMIN). Peer associations are in `alf_node_assoc`.
- Deleted nodes have `alf_node.node_deleted = 1` (or move to `archive://SpacesStore`). Always filter
  `node_deleted = 0`.

### 5.4 Step-by-step SQL to open a document (design)

These queries are a **design**, written against the DDL column names. Run them read-only, under
`SET TRANSACTION ISOLATION LEVEL READ COMMITTED` (never `NOLOCK` on `alf_content_*`).

```sql
-- Step 0. Resolve the ids once at start-up and cache them. Never hard-code alf_qname ids.
DECLARE @ns_lrs bigint = (SELECT id FROM LRSAdmin.alf_namespace WHERE uri = N'http://www.landsystems.com/lrs/model/core/1.0');
DECLARE @ns_cm  bigint = (SELECT id FROM LRSAdmin.alf_namespace WHERE uri = N'http://www.alfresco.org/model/content/1.0');
DECLARE @q_entityid   bigint = (SELECT id FROM LRSAdmin.alf_qname WHERE ns_id = @ns_lrs AND local_name = N'entityid');
DECLARE @q_referenceid bigint = (SELECT id FROM LRSAdmin.alf_qname WHERE ns_id = @ns_lrs AND local_name = N'referenceid');
DECLARE @q_imagelist  bigint = (SELECT id FROM LRSAdmin.alf_qname WHERE ns_id = @ns_lrs AND local_name = N'imagelist');
DECLARE @q_pageindex  bigint = (SELECT id FROM LRSAdmin.alf_qname WHERE ns_id = @ns_lrs AND local_name = N'pageindex');
DECLARE @q_pagename   bigint = (SELECT id FROM LRSAdmin.alf_qname WHERE ns_id = @ns_lrs AND local_name = N'pagename');
DECLARE @q_isattach   bigint = (SELECT id FROM LRSAdmin.alf_qname WHERE ns_id = @ns_lrs AND local_name = N'isattachment');
DECLARE @q_thumbnail  bigint = (SELECT id FROM LRSAdmin.alf_qname WHERE ns_id = @ns_lrs AND local_name = N'thumbnail');
DECLARE @q_annotation bigint = (SELECT id FROM LRSAdmin.alf_qname WHERE ns_id = @ns_lrs AND local_name = N'annotation');
DECLARE @q_content    bigint = (SELECT id FROM LRSAdmin.alf_qname WHERE ns_id = @ns_cm  AND local_name = N'content');
DECLARE @st_spaces    bigint = (SELECT id FROM LRSAdmin.alf_store WHERE protocol = N'workspace' AND identifier = N'SpacesStore');

-- Step 1. lr_source_document.id -> scanned-document node(s).
--   Normally exactly one. More than one means the document was re-scanned without a delete; take the newest by audit_modified.
--   The value column depends on the model type (G3). Match both until G3 is answered.
DECLARE @docId numeric(19,0) = 123456;   -- lr_source_document.id
SELECT n.id AS doc_node_id, n.uuid, t.local_name AS doc_class, n.audit_created, n.audit_modified
FROM LRSAdmin.alf_node_properties p
JOIN LRSAdmin.alf_node  n ON n.id = p.node_id AND n.node_deleted = 0 AND n.store_id = @st_spaces
JOIN LRSAdmin.alf_qname t ON t.id = n.type_qname_id
WHERE p.qname_id = @q_entityid
  AND (p.long_value = @docId OR p.string_value = CAST(@docId AS nvarchar(30)));
--   Cost: index fk_alf_nprop_qn (qname_id) narrows to the ~365k entityid rows, then filters them.
--   For the web viewer build a mapping table in plagis_ext instead (§5.5).

-- Step 2. Pages of the document, in order.
DECLARE @docNode bigint = /* from step 1 */ 0;
SELECT c.child_node_id AS page_node_id, pn.uuid,
       pidx.long_value  AS page_index,
       pnm.string_value AS page_name,
       patt.boolean_value AS is_attachment
FROM LRSAdmin.alf_child_assoc c
JOIN LRSAdmin.alf_node pn ON pn.id = c.child_node_id AND pn.node_deleted = 0
LEFT JOIN LRSAdmin.alf_node_properties pidx ON pidx.node_id = pn.id AND pidx.qname_id = @q_pageindex
LEFT JOIN LRSAdmin.alf_node_properties pnm  ON pnm.node_id  = pn.id AND pnm.qname_id  = @q_pagename
LEFT JOIN LRSAdmin.alf_node_properties patt ON patt.node_id = pn.id AND patt.qname_id = @q_isattach
WHERE c.parent_node_id = @docNode
  AND c.type_qname_id  = @q_imagelist           -- the child-assoc type is lrs:imagelist
ORDER BY pidx.long_value, c.assoc_index, c.id;  -- index idx_alf_cass_pri / fk_alf_cass_pnode

-- Step 3. Content of one page: image, thumbnail and annotation in one go.
DECLARE @pageNode bigint = /* from step 2 */ 0;
SELECT q.local_name AS which,                    -- content | thumbnail | annotation
       cu.content_url, cu.content_size, mt.mimetype_str, en.encoding_str, lo.locale_str
FROM LRSAdmin.alf_node_properties p
JOIN LRSAdmin.alf_qname        q  ON q.id  = p.qname_id
JOIN LRSAdmin.alf_content_data cd ON cd.id = p.long_value          -- actual_type_n 21 (CONTENT_DATA_ID)
JOIN LRSAdmin.alf_content_url  cu ON cu.id = cd.content_url_id
LEFT JOIN LRSAdmin.alf_mimetype mt ON mt.id = cd.content_mimetype_id
LEFT JOIN LRSAdmin.alf_encoding en ON en.id = cd.content_encoding_id
LEFT JOIN LRSAdmin.alf_locale   lo ON lo.id = cd.content_locale_id
WHERE p.node_id = @pageNode
  AND p.qname_id IN (@q_content, @q_thumbnail, @q_annotation)
  AND p.actual_type_n = 21;
--   If G3 shows content stored differently (actual_type_n 11, the ContentData string in string_value),
--   parse 'contentUrl=store://…|mimetype=…|size=…|encoding=…|locale=…' instead.

-- Step 4 (application code, not SQL). Map content_url -> path:
--   if url LIKE 'store://%' and has no store-id segment: D:\LRS_STORAGE\contentstore\ + replace(substring(url, 9), '/', '\')
--   else: look up the mtfs:filestore whose mtfs:storeID matches (Step 5) and use its mtfs:shareunc as the root.
--   Open read-only. Stream it with the mime type from Step 3. Never rename, move or delete the file.

-- Step 5. File-store table (run once and cache).
DECLARE @ns_mtfs bigint = (SELECT id FROM LRSAdmin.alf_namespace WHERE uri = N'http://www.landsystems.com/sdms/model/multifs/1.0');
SELECT n.id, n.uuid,
       MAX(CASE WHEN q.local_name = N'storeID'          THEN p.string_value END) AS store_id,
       MAX(CASE WHEN q.local_name = N'shareunc'         THEN p.string_value END) AS root_path,
       MAX(CASE WHEN q.local_name = N'activestatus'     THEN p.string_value END) AS active_status,
       MAX(CASE WHEN q.local_name = N'freethreshold'    THEN p.long_value   END) AS free_threshold,
       MAX(CASE WHEN q.local_name = N'computerName'     THEN p.string_value END) AS computer_name,
       MAX(CASE WHEN q.local_name = N'storeDescription' THEN p.string_value END) AS description
FROM LRSAdmin.alf_node n
JOIN LRSAdmin.alf_qname t ON t.id = n.type_qname_id AND t.ns_id = @ns_mtfs AND t.local_name = N'filestore'
JOIN LRSAdmin.alf_node_properties p ON p.node_id = n.id
JOIN LRSAdmin.alf_qname q ON q.id = p.qname_id
WHERE n.node_deleted = 0
GROUP BY n.id, n.uuid;
```

From a transaction (all documents of one file):

```sql
SELECT td.document_index, sd.id AS document_id, sd.Hjtype, sd.document_number, sd.page_count, n.uuid AS doc_node_uuid
FROM LRSAdmin.lr_transaction_document td
JOIN LRSAdmin.lr_source_document sd ON sd.id = td.document_id
LEFT JOIN LRSAdmin.alf_node_properties p ON p.qname_id = @q_entityid AND p.long_value = sd.id
LEFT JOIN LRSAdmin.alf_node n ON n.id = p.node_id AND n.node_deleted = 0
WHERE td.transaction_id = @transactionId
ORDER BY td.document_index;
```

### 5.5 Recommended read path for the replica

- Build **`plagis_ext.doc_node_map(document_id, doc_node_id, doc_class, transaction_id, last_alf_txn)`** once,
  with one pass over `lrs:entityid` rows. Then keep it up to date incrementally: select `alf_node` rows whose
  `transaction_id` is above the stored watermark (index `fk_alf_node_txn`). This gives O(1) lookups without
  touching `LRSAdmin` structure (D2).
- Serve pages from disk through the replica (the image viewer of R2/R5). Convert TIFF to PNG or JPEG on the fly
  for browsers. LEADTOOLS annotations (XML) can be shown as an overlay later. Phase 1 can ignore them, but must
  keep them.
- Fallback while JBoss runs: `ContentService.read` → `/lrs/download/...&ticket=` (03 §3.3). This is useful to
  validate the direct path on 50 sample files (R1 exit criterion).

---

## 6. The work queue

### 6.1 What the Desktop asks Lucene

All queries come from `ShellForm.cs:1139–1193`. Store `lrs://rootStore`, sorted by `lrs:taskCreatedDate`
descending, `fetchSize` = setting `bounds/QueueLimit` (40,000). See 03 §4.3.

| View | Lucene | Meaning |
|---|---|---|
| All tasks | `TYPE:"{lrs}Transaction" AND (@lrs\:taskCompleted:"false") AND ((@lrs\:taskActorName:"<user>") OR ((@lrs\:taskSwimlane:"<g1>") OR …))` | my tasks plus tasks in a swimlane of any of my groups |
| My tasks | `… AND (@lrs\:taskActorName:"<user>")` | assigned to me |
| My groups | `… AND ((@lrs\:taskActorName:"<g1>") OR …)` | assigned to one of my groups (actor = group) |
| Custom | `… <TaskFilter clauses> AND ((@lrs\:taskActorName:"<user>") OR (swimlanes…))` | TaskFilterForm fields: `transactionNumber`, `transactionType`, `applicantName`, `sourceRids`, `targetRids`, `createDate`, `taskDescription`, `taskActorName` (FF `TaskFilterForm`) |
| Plus | `IQueueService.Filter` (PLAGIS plug-in) appended | |

Group names are the authority without `GROUP_` (`PersonServiceAgent.GetUserGroupList`). Columns shown: FF
`ShellFormQueue` / `ShellFormQueueDetails`, in this order: `processId`, `transactionNumber`, `transactionType`,
`processName`, `taskDescription`, `applicantName`, `taskActorName`, `sourceRids`, `targetRids`, `createDate`,
`lockOwner`, `processSuspended`.

Lucene semantics that matter when reproducing it:

- `@prop:"x"` on a `d:text` property is a **tokenised, case-insensitive phrase match**, not equality. So
  `"Intake Clerk"` also matches `"Senior Intake Clerk"` (inferred from Alfresco's default analyser).
- `TYPE:` includes subtypes.
- The index lags the database by up to `index.tracking.cronExpression` (5 s) in a cluster. On a single node,
  in-transaction indexing is on (`disableInTransactionIndexing=false`).

### 6.2 Equivalent SQL over the Alfresco mirror (design, read-only)

This is faithful to what the old Desktop sees, including locks. It is heavy, because it pivots
alf_node_properties for every open queue node. Use it to compare old and new results, not as the production
queue.

```sql
-- ids resolved as in §5.4: @st_lrs = alf_store(lrs, rootStore); @t_trn = qname lrs:Transaction;
-- @q_taskCompleted, @q_taskActorName, @q_taskSwimlane, @q_taskCreatedDate, @q_transactionId, @q_transactionNumber,
-- @q_taskDescription, @q_processId, @q_lockOwner (cm:lockOwner)
WITH q AS (
  SELECT n.id, n.uuid,
    MAX(CASE WHEN p.qname_id = @q_taskCompleted   THEN CAST(p.boolean_value AS int) END) AS task_completed,
    MAX(CASE WHEN p.qname_id = @q_taskActorName   THEN p.string_value END) AS actor,
    MAX(CASE WHEN p.qname_id = @q_taskSwimlane    THEN p.string_value END) AS swimlane,
    MAX(CASE WHEN p.qname_id = @q_taskCreatedDate THEN p.string_value END) AS task_created_iso,
    MAX(CASE WHEN p.qname_id = @q_transactionId   THEN p.long_value   END) AS transaction_id,
    MAX(CASE WHEN p.qname_id = @q_transactionNumber THEN p.string_value END) AS transaction_number,
    MAX(CASE WHEN p.qname_id = @q_taskDescription THEN p.string_value END) AS stage,
    MAX(CASE WHEN p.qname_id = @q_processId       THEN p.long_value   END) AS process_id,
    MAX(CASE WHEN p.qname_id = @q_lockOwner       THEN p.string_value END) AS lock_owner
  FROM LRSAdmin.alf_node n
  JOIN LRSAdmin.alf_node_properties p ON p.node_id = n.id
  WHERE n.store_id = @st_lrs AND n.type_qname_id = @t_trn AND n.node_deleted = 0
    AND p.qname_id IN (@q_taskCompleted, @q_taskActorName, @q_taskSwimlane, @q_taskCreatedDate, @q_transactionId,
                       @q_transactionNumber, @q_taskDescription, @q_processId, @q_lockOwner)
  GROUP BY n.id, n.uuid)
SELECT * FROM q
WHERE task_completed = 0
  AND (actor = @user OR swimlane IN (SELECT grp FROM @myGroups))   -- @myGroups = group names without GROUP_
ORDER BY task_created_iso DESC;   -- ISO strings with the same offset sort correctly; convert to UTC to be safe
```

### 6.3 What the replica should query instead (recommended)

The authoritative state is jBPM plus `lr_process_history`. The Alfresco nodes are a **copy** that ILS
`TransactionBean` keeps in step. The replica's own queue should read the source tables. This is also what
lr_setting `gisintegration.queryTransactionByUser` does (02 §6.1).

```sql
-- Open tasks for user @user with groups @myGroups (names without GROUP_), newest first.
SELECT ti.ID_ AS task_id, ti.NAME_ AS task_name, ti.DESCRIPTION_ AS stage, ti.CREATE_ AS task_created,
       ti.ACTORID_ AS actor, sli.NAME_ AS swimlane, pi.ID_ AS process_id, pd.NAME_ AS process_name,
       pi.ISSUSPENDED_ AS postponed, t.id AS transaction_id, t.transaction_number
FROM LRSAdmin.JBPM_TASKINSTANCE ti
JOIN LRSAdmin.JBPM_PROCESSINSTANCE pi    ON pi.ID_ = ti.PROCINST_
JOIN LRSAdmin.JBPM_PROCESSDEFINITION pd  ON pd.ID_ = pi.PROCESSDEFINITION_
LEFT JOIN LRSAdmin.JBPM_SWIMLANEINSTANCE sli ON sli.ID_ = ti.SWIMLANINSTANCE_
JOIN LRSAdmin.lr_process_history ph      ON ph.task_id = ti.ID_ AND ph.task_complete IS NULL
JOIN LRSAdmin.lr_transaction t           ON t.id = ph.transaction_id
WHERE ti.END_ IS NULL AND ISNULL(ti.ISCANCELLED_, 0) = 0 AND ISNULL(ti.ISOPEN_, 1) = 1
  AND (   ti.ACTORID_ = @user
       OR (ti.ACTORID_ IS NULL AND sli.NAME_ IN (SELECT grp FROM @myGroups))
       OR EXISTS (SELECT 1 FROM LRSAdmin.JBPM_TASKACTORPOOL tap
                  JOIN LRSAdmin.JBPM_POOLEDACTOR pa ON pa.ID_ = tap.POOLEDACTOR_
                  WHERE tap.TASKINSTANCE_ = ti.ID_ AND pa.ACTORID_ IN (SELECT grp FROM @myGroups)))
ORDER BY ti.CREATE_ DESC;
```

The jBPM columns are from the DDL. How `ACTORID_` and `POOLEDACTOR` hold group names (with or without
`GROUP_`) must be checked with one sample (A2).

The lock owner is not in jBPM. During parallel run read it from the `cm:lockable` aspect of the matching
`wf:taskRef` node (`wf:taskId` = `ti.ID_`). Once the old Desktop is gone, keep it in the replica's own
`plagis_ext.task_lock`.

**Consequence for parallel run.** If the replica completes or assigns tasks by writing jBPM tables directly
(D3), the `wf:taskRef` and `lrs:Transaction` mirror nodes are **not** updated, so the old Desktop's queue shows
stale rows. Either:

- make every task change through the old server's `WorkflowWebService` while the old Desktop is live (the
  mirror then stays right); or
- cut over one transaction type at a time and stop serving those types in the old Desktop (D3).

Writing mirror nodes by SQL is not recommended (§10).

---

## 7. Users, groups, permissions and sign-in

### 7.1 Where each piece is

| Piece | Storage | Source |
|---|---|---|
| Credentials | `usr:user` node in `user://alfrescoUserStore` (`usr:username`, `usr:password`, flags) | §3.5 |
| Person (display, office) | `cm:person` in `workspace://SpacesStore` `/sys:system/sys:people` (+ `lrs:Employee`) | §3.5 |
| Authority names | `alf_authority(authority, crc)`, 188 rows: users, `GROUP_*`, `ROLE_*`, `GROUP_EVERYONE`, `guest`, `admin` (INV). User names are not reproduced here | INV |
| Group nodes and membership | `cm:authorityContainer` nodes under `/sys:system/sys:authorities`; membership = `alf_child_assoc` of type `cm:member` (parent = group, child = person or sub-group) | REGADMIN `usp_add_to_group`, `fn_group_node` |
| ACLs | `alf_node.acl_id` → `alf_access_control_list` (926) → `alf_acl_member` (2,157; `pos` = inheritance depth, 0 = set on the node) → `alf_access_control_entry` (201; `allowed`) → `alf_permission` (6) and `alf_authority` | DDL, INV |

Registry groups seen in `alf_authority` (INV, ids ≤ 60): `GROUP_LRS`, `GROUP_Survey Department`,
`GROUP_Scanning Operator`, `GROUP_Scanning Clerk`, `GROUP_Postpone Clerk`, `GROUP_Withdrawal Clerk`,
`GROUP_Intake Clerk`, `GROUP_Cashier`, `GROUP_Deeds Registrar`, `GROUP_Lands Officer`, `GROUP_Delivery Clerk`,
`GROUP_Recommendation Creator`, `GROUP_Inspection Officer`, `GROUP_Search Officer`, `GROUP_Director of Lands`,
`GROUP_Rejection Letter Creator`, `GROUP_LRS_FEE_CONFIGURATION`, `GROUP_Rent Officer`, `GROUP_Scanning Officer`.
Code also uses `GROUP_LRS_ADMINISTRATION_SECURITY` (UMB) and `GROUP_LRS_ADMINISTRATION_SYSTEM` (this repo's
dashboard). Authorities **without** a prefix also appear: `Surveyor General`, `Director of Town Planning`,
`Records Clerk`, `Intake Clerk`, `Deeds Registrar`, `Correction Officer`, `Rent Officer`. They receive
**Manager** on process definitions (ACEs 18–22 → permission 6). They look like swimlane or role names entered
without `GROUP_` (inferred); jBPM swimlanes resolve to these names (04 §1).

### 7.2 The six `alf_permission` rows (INV)

| id | `type_qname_id` → QName | name | Used for |
|---:|---|---|---|
| 1 | 21 → `sec:All` | `All` | full control. `ROLE_ADMINISTRATOR` on every `perm:permission` node (PMB) |
| 2 | 42 → `wf:processDefinition` | `Consumer` | may see a workflow |
| 3 | 48 → `cm:cmobject` | `Contributor` | stock |
| 4 | 49 → `sys:base` | `Read` | **holders of an `lrs*` permission** (PMB grants `Read`); `GROUP_EVERYONE` and `guest` on stock folders |
| 5 | 48 → `cm:cmobject` | `Collaborator` | stock |
| 6 | 42 → `wf:processDefinition` | `Manager` | **may start that workflow** (the Desktop filters its New menu with `getACLs(…, "Manager")`) |

The ILS permission model (Consumer/Manager on `wf:processDefinition`) and the `perm:` model XML were not
captured (G1). Permission groups such as `Write` are not stored. Alfresco expands them from `All`.

### 7.3 How the Desktop decides `lrs*` access

`AccessControlServiceAgent.Initialize`:

1. Gets `sys:rootContainer/lrs:permissionsRoot` in `lrs://rootStore`.
2. For each child of type `perm:permission`, reads `name`.
3. Calls `hasPermissions(node, ["Read"])` and `hasPermissions(node, ["Write"])`. Both → `accessFull`, Read only
   → `accessRead`, neither → absent.
4. `IsPermissionExist("lrsAssignTask")` and the like then gate menus (05 §8).

**Replica design (SQL):**

```sql
-- Who holds each named permission (direct ACEs only, pos = 0). Expand groups to members in application code.
DECLARE @ns_perm bigint = (SELECT id FROM LRSAdmin.alf_namespace WHERE uri = N'http://www.landsystems.com/permission/model/1.0');
SELECT nm.string_value AS permission_key, st.string_value AS strategy, a.authority, pm.name AS alf_permission, ace.allowed
FROM LRSAdmin.alf_node pn
JOIN LRSAdmin.alf_qname t   ON t.id = pn.type_qname_id AND t.ns_id = @ns_perm AND t.local_name = N'permission'
JOIN LRSAdmin.alf_node_properties nm ON nm.node_id = pn.id AND nm.qname_id = @q_cm_name
LEFT JOIN LRSAdmin.alf_node_properties st ON st.node_id = pn.id AND st.qname_id = @q_perm_strategy
JOIN LRSAdmin.alf_acl_member m            ON m.acl_id = pn.acl_id AND m.pos = 0
JOIN LRSAdmin.alf_access_control_entry ace ON ace.id = m.ace_id
JOIN LRSAdmin.alf_permission pm           ON pm.id = ace.permission_id
JOIN LRSAdmin.alf_authority a             ON a.id = ace.authority_id
WHERE pn.node_deleted = 0
ORDER BY permission_key, a.authority;
```

A user's effective authorities are:

- the user name;
- every group reached through `cm:member`, transitively (the parent group of a group the user is in);
- `GROUP_EVERYONE`;
- `ROLE_ADMINISTRATOR` if the user is in `GROUP_ALFRESCO_ADMINISTRATORS` (Alfresco 3.4 rule, inferred) or is
  `admin`.

`All` means full access, `Read` alone means read access. Load this map once per session; it has only 201 ACEs.

Web Access: a named permission with strategy "Web Access: use View, Print access to documents"; users without
it are refused (06 §2.7). The strategy literal stored in `perm:strategy` is G9.

### 7.4 Sign-in: verify existing passwords without changing them (design)

Today:

- `authentication.chain=alfrescoNtlm1:alfrescoNtlm` (RP). Only the internal Alfresco user store is used; the
  LDAP/AD/Kerberos/passthru subsystems are present but not chained.
- `user.name.caseSensitive=false`.
- `authentication.ticket.ticketsExpire=false` (tickets never expire).
- `alfresco.authentication.allowGuestLogin=false`.
- ILS password rules (`ils/messages/password-control.properties` keys): minimum length, one capital, one small
  letter, one digit, one special character, no spaces, must not start with `!`/`?`, must not be the login or
  the login reversed or the e-mail, must not contain the first three letters of the login, no repeated
  characters.

Replica sign-in (design):

1. Normalise the user name (trim, lower-case; compare case-insensitively).
2. Find the `usr:user` node: `alf_store(user, alfrescoUserStore)` and `usr:username` = name, `node_deleted = 0`
   (REGADMIN `fn_login_node`). Read `usr:password`, `usr:enabled`, `usr:accountLocked`, `usr:accountExpires`,
   `usr:credentialsExpire`, and the expiry date property if present (G8 lists the property names).
3. Refuse if `enabled = 0` or `accountLocked = 1` ("Logon failure: your account was disabled", from
   `password-control.properties`).
4. Compute `MD4(UTF-16LE(password))`, hex-encode it, and compare **case-insensitively** with `usr:password`
   (both cases exist, see REGADMIN). This is exactly the NTLM hash, so any MD4 library works (SQL Server
   `HASHBYTES('MD4', N'…')` gives the same bytes, as REGADMIN's self-test shows). Use a constant-time compare.
5. Find the `cm:person` with `cm:userName` = name for display and `lrs:officeId`, then load groups (§7.3).
6. **Never rewrite the hash on login.** On a password change during parallel run, write **only** the MD4 hex in
   the same case style, so the old JBoss still accepts it. Because Alfresco caches users, the old server sees
   the change only after a restart (REGADMIN README). Alternatively, change passwords through the old
   `AdministrationService.changePassword` while JBoss lives.
7. Upgrade path: MD4 is unsalted and weak. After JBoss is retired, store a modern hash (argon2id) in
   `plagis_ext.user_credential` at the next successful login, and stop reading `usr:password`.

The Cadastre server in this repo already signs in through `AuthenticationService.startSession` and manages
users with the tested `plagis_admin` procedures (`crates/cadastre-server/src/regadmin.rs`,
`tools/registry-admin/`). The replica can reuse that layer.

---

## 8. Workflow model: stock Alfresco definitions vs. the LRS definitions

| | Stock Alfresco (`lrs.war/WEB-INF/classes/alfresco/workflow/*.xml`) | LRS (stored in the DB) |
|---|---|---|
| Definitions | `wfa:adhoc`, `wfa:review`, `wfa:reviewpooled`, `wfa:parallelreview`, `wfa:parallelgroupreview`, `wcmwf:submit`, `wcmwf:changerequest`, `wcmwf:submitdirect`, `inwf:invitation-nominated`, `imwf:invitation-moderated` (JBPM_PROCESSDEFINITION ids 1–10) | 15 names, versions 1–4 (ids 11–64): Raise Title Deed Plan, Survey Plan Checking and Approval, Raise Site Analysis Report, Indexing and Scanning, Recertification Registration, Ground Rent Payment, First Registration, Subsequent Registration, Application for Certificate of Occupancy, Search, Replacement Of Title, Set Right Of Occupancy As Expired, Data Correction, Ground Rent Review, Survey Plan Checking (INV) |
| Naming | QName names (`wfa:…`) with task types such as `wfa:adhocTask`, `wfa:reviewTask` from the Alfresco **bpm task model** | plain names, no prefix. Tasks are plain jBPM task names (stage codes such as `RegisterCofO`) |
| Engine path | Alfresco `WorkflowService` (`jbpm$<id>`), task properties `bpm:*` | **ILS `WorkflowService`** (`ils-workflow.jar`, SOAP `WorkflowWebService`), mirrored into `wf:` nodes (§3.4) |
| Used at PLAGIS? | **No**: the `bpm:` namespace is absent from `alf_namespace`, so no task was ever persisted | Yes: 99,808 instances, 674,828 tasks |
| Deployed by | Alfresco bootstrap | ILS `workflowBootstrap` (`sdms_bootstrap`: `lrs-workflow-bootstrap-1.xml` v4/6/11) and the admin UI "Workflow definitions" (`WorkflowMngBean`, `/lrs/downloadBPM`; `system.workflow.deployservlet.enabled=false`) |

The task properties the Desktop uses are all `wf:`/`lrs:` (§3.1, §3.4) plus jBPM process variables set
through `WorkflowWebService.setProcessVariables` (for example `rejection` = `lr_rejection.id`, `isRejected`
via `JBPM_VARIABLEACCESS`; 03 §6). There are **no `bpm:` task properties and no custom LRS task types per
stage** in Alfresco's dictionary sense. Stage-specific behaviour lives in the Desktop's WorkflowConfig files
and in `lr_transaction_metadata` (04 §1.3, §1.7). The replica therefore needs no Alfresco task model; it needs
the jBPM tables (A1, A2) and the `wf:` mirror only while the old Desktop runs.

Comments and history: `wf:comment` children of `wf:taskRef` (types `task-comment`, `task-suspend`,
`task-resume`; access public or private). The history grid is `HistoryViewForm` / `HistoryViewFormEvent` (FF).
`lr_process_history` holds the same task timeline in a Registry table. **Comments exist only in Alfresco**:
`JBPM_COMMENT` has 0 rows and there is no `lr_` comment table. So the replica must read `wf:comment` nodes to
show the old notes (design SQL: child assocs of type `wf:comments` under the `wf:taskRef` whose `wf:taskId` is
the jBPM task id).

---

## 9. Search and indexing

| Item | Value | Source |
|---|---|---|
| Index root | `dir.indexes=${dir.root}/lucene-indexes` → `D:/LRS_STORAGE/lucene-indexes`; backups `D:/LRS_STORAGE/backup-lucene-indexes`; locks `…/lucene-indexes/locks` | RP |
| Layout | one index per store, `lucene-indexes/<protocol>/<identifier>` (for example `lrs/rootStore`, `workflow/rootStore`, `workspace/SpacesStore`) | Alfresco convention (inferred) |
| Recovery | `index.recovery.mode=VALIDATE` (checks the last transactions at start-up; does not rebuild) | RP |
| Tracking | `index.tracking.cronExpression=0/5 * * * * ?`: re-indexes nodes of new `alf_transaction` rows by `commit_time_ms` (also picks up SQL-inserted nodes if a new `alf_transaction` row is written, as REGADMIN `usp__new_txn` does) | RP, REGADMIN |
| Limits | `lucene.query.maxClauses=10000`, `lucene.indexer.maxFieldLength=10000` | RP |
| Full text | content transformers to text. For TIFF/JPEG scans there is no OCR, so there is effectively **no full text** of scans (inferred) | RP `transformer.*` |

**What the replica loses without Lucene**, and the replacement for each:

| Lucene use | Replacement |
|---|---|
| Queue (`lrs:Transaction`) | SQL over jBPM + `lr_process_history` (§6.3). No loss |
| Scans by document id (`@lrs\:entityid`) | `plagis_ext.doc_node_map` (§5.5). No loss |
| Task, comment and lodgment lookups (`@wf\:taskProcInstRef`, xpath under process nodes) | joins on `alf_child_assoc` by parent, or the jBPM tables. No loss |
| Group lookup (`PATH:"/sys:system/sys:authorities//*"`) | `alf_authority` + `cm:member` assocs (REGADMIN). No loss |
| Report templates by type | one-time export of the templates (B4) |
| Full-text search of content | none exists today for scans. Upgrade U7 (OCR plus a full-text index such as SQL Server FTS or Tantivy) is new value, not a loss |

The replica should **never open or write the Lucene files**. They belong to the running Alfresco, which holds
locks on them.

---

## 10. Risks for the replica and the recommended strategy

| # | Risk | Detail | Mitigation |
|---|---|---|---|
| 1 | **Schema auto-update** | Alfresco `db.schema.update=true` and Hibernate `hibernate.hbm2ddl.auto=update` (`alfresco/domain/hibernate-cfg.properties`); ILS `ShemaUpdater` plus `sdms_bootstrap` DDL run at every JBoss start | Add nothing to `LRSAdmin` (D2). Keep new tables in `plagis_ext`. Adding indexes is an ICT decision (01 §6) |
| 2 | **Alfresco caches** | EHCache for nodes, properties, aspects, qnames, ACLs, people and authorities (`hibernate.cache.use_second_level_cache=true`, `cache.strategy=read-write`). SQL writes are invisible to the running server until a restart. REGADMIN already needs a JBoss restart after each user change | Do not write `alf_*` by SQL except through the tested REGADMIN procedures, and always followed by a restart |
| 3 | **Lucene drift** | Nodes written by SQL without an `alf_transaction` row (with `commit_time_ms`) are never indexed, so the old Desktop cannot find them (queue, scans, groups). With such a row, tracking indexes them within 5 s (inferred) | Same as 2. For documents use the narrow API path below |
| 4 | **Versioning** | `cm:versionable` documents and pages copy into `workspace://version2Store` on `createVersion`. A SQL writer would have to build version nodes | Do not version from the replica. Write new nodes without `cm:versionable` (allowed: versioning is optional, `VersioningSupported`) or go through the API |
| 5 | **Task locks** | `AuthoringService.lock` adds `cm:lockable` (`cm:lockOwner`, `cm:lockType` WRITE, `cm:expiryDate`) to the `wf:taskRef`; TB mirrors it onto the queue node and clears it on `onRemoveAspect`. Crashed clients leave stale locks | During parallel run, read locks from the mirror and **respect** them. Release only through `AuthoringService.unlock`. After cut-over keep locks in `plagis_ext.task_lock` |
| 6 | **Optimistic versions** | `alf_node.version` and other `version` columns are Hibernate optimistic locks. A blind SQL update breaks the old server's next write ("stale state") | Read-only on `alf_*` |
| 7 | **Orphan cleaner** | daily 04:00 the cleaner moves content no longer referenced (`alf_content_url.orphan_time`) to `contentstore.deleted` after 1 day (`orphanProtectDays=1`). A file the replica writes into `contentstore` without rows is never registered and is untouched (inferred), but is invisible to the old system | Never put replica files inside `contentstore`. Use a separate root (below) |
| 8 | **Large or serialised values** | strings over 1024 characters (e.g. `lrs:entitydata`) are Java-serialised in `serializable_value` | Ignore `entitydata`. The same data is in the `lr_*` tables |
| 9 | **Timezone of dates** | ISO strings carry the JVM offset (−08:00 seen in 2014 data) | Parse with the offset and normalise to UTC |
| 10 | **Two constant sets** | `ICoreDomainModel` (camelCase, used by TB and the Desktop) and `CoreDomainModel` (snake_case, Employee aspect) coexist | Use the names in §3.1 (the Desktop's). Confirm with the full `alf_qname` list (G2) |
| 11 | **Multi-file stores** | a URL may point to a non-default file store (§3.3) | Resolve through `mtfs:filestore` (G4, G5) |
| 12 | **Security** | tickets never expire; MD4 password hashes; JBoss consoles with sample accounts (01 §6) | Replica tickets expire; plan the hash upgrade (§7.4) |

### Recommendation

1. **Parallel run: Alfresco stays the document service.** It agrees with D4/D5.
   - The replica **reads** documents directly (`alf_*` read-only, files read-only on `D:\LRS_STORAGE`, §5).
   - It **writes new scans only through the narrow API path** that already works for the Cadastre:
     `PUT /lrs/upload/<guid>.<ext>?ticket=` for each page, then one `RepositoryService.update` CML batch that
     creates the `lrs:supporting_scanned_document` or `lrs:outgoing_scanned_document` under
     `cm:scan_documents` (properties `lrs:referenceid`, `lrs:entityid`, `lrs:pageCount`) and its
     `lrs:scannedimage` children with `cm:content`, `lrs:pageindex`, `lrs:pagename`, `lrs:pagefilesize`,
     `lrs:isattachment` and optionally a thumbnail. These are the same statements `DocumentProcessor` sends
     (03 §2.6, 05 §2.8). Alternatively use `IntegrationService.SaveEntity` with a `DocumentEntity`, as the
     Cadastre does.
   - This keeps Alfresco's caches, Lucene, the queue mirror and versions consistent, so the old Desktop and Web
     Access see the new pages at once.
   - Task moves go through `WorkflowWebService` for any file the old Desktop can still open (§6.3).
2. **After JBoss is retired: the replica owns new documents.**
   - Store new files under a new root on the same disk, e.g. `D:\LRS_STORAGE\replica\yyyy\MM\dd\<uuid>.<ext>`.
   - Keep metadata in `plagis_ext.document_page(document_id, page_index, path, mime, size, sha256, thumbnail_path, annotation_path, created_by, created_at)`.
   - The viewer reads old pages through `alf_*` (frozen, read-only) and new pages through `plagis_ext`.
   - No migration of the 2.1 M legacy content items is needed. An optional later job can copy legacy metadata
     into `plagis_ext.document_page` to drop the `alf_*` dependency.
3. **Never** create Alfresco nodes, ACLs or versions by raw SQL from the replica. The one exception is the
   existing, tested REGADMIN user procedures (with a JBoss restart). Do not touch the Lucene indexes.

---

## 11. Gaps / still to capture

Each gap says exactly what to fetch. Every item is a read-only SELECT on LRS43 or a file copy from .5; none
changes the Registry. These extend capture items A5/A6/B3 in 08.

| # | Gap | What to fetch |
|---|---|---|
| **G1** | The **model XML** for `lrs:`, `wf:`, `perm:`, `rpt:`/`srpt:` and the ILS permission model (`permissionDefinitions` for `Consumer`/`Manager` on `wf:processDefinition`). This gives the exact types, mandatory flags, parents, `multiple`, `index` settings, and which aspect holds which property | (a) Files: copy `lrs.war\WEB-INF\classes\alfresco\extension\**\*.xml`, `lrs.war\WEB-INF\classes\**\*Model*.xml`, `**\*model*.xml`, `**\*-context.xml`, plus every jar in `lrs.war\WEB-INF\lib` not yet copied (only 21 were), and list `*.xml` inside each (`unzip -l`). (b) DB, for models deployed into the repository: `SELECT n.uuid, cu.content_url, cu.content_size FROM LRSAdmin.alf_node n JOIN LRSAdmin.alf_qname t ON t.id=n.type_qname_id AND t.local_name='dictionaryModel' JOIN LRSAdmin.alf_node_properties p ON p.node_id=n.id AND p.actual_type_n=21 JOIN LRSAdmin.alf_content_data cd ON cd.id=p.long_value JOIN LRSAdmin.alf_content_url cu ON cu.id=cd.content_url_id WHERE n.node_deleted=0;` then copy those files from `D:\LRS_STORAGE\contentstore` |
| **G2** | **All 278 `alf_qname` rows** (only 100 captured) | `SELECT q.id, ns.uri, q.local_name FROM LRSAdmin.alf_qname q JOIN LRSAdmin.alf_namespace ns ON ns.id=q.ns_id ORDER BY q.id;` (no personal data) |
| **G3** | Value storage per property (`actual_type_n`/`persisted_type_n`, multi-valued) | `SELECT TOP 1 WITH TIES ns.uri, q.local_name, p.actual_type_n, p.persisted_type_n, CASE WHEN p.list_index>=0 THEN 1 ELSE 0 END AS multi FROM LRSAdmin.alf_node_properties p JOIN LRSAdmin.alf_qname q ON q.id=p.qname_id JOIN LRSAdmin.alf_namespace ns ON ns.id=q.ns_id WHERE ns.uri LIKE 'http://www.landsystems.com/%' ORDER BY ROW_NUMBER() OVER (PARTITION BY p.qname_id, p.actual_type_n, p.persisted_type_n ORDER BY p.node_id);` (one row per combination; large table, run off-hours) |
| **G4** | **Content-URL format** and whether more than one file store is used | `SELECT TOP 20 content_url, content_size FROM LRSAdmin.alf_content_url ORDER BY id;` the same with `ORDER BY id DESC`; and `SELECT LEFT(content_url, CHARINDEX('/', content_url + '/', 9)) AS prefix, COUNT(*) FROM LRSAdmin.alf_content_url GROUP BY LEFT(content_url, CHARINDEX('/', content_url + '/', 9));` plus `SELECT COUNT(*) FROM LRSAdmin.alf_content_url WHERE content_url NOT LIKE 'store://[12][0-9][0-9][0-9]/%';` |
| **G5** | **File-store and cluster nodes** (`mtfs:cluster`, `mtfs:filestore`, their store and path) | §5.4 step 5 query, plus the same for `local_name='cluster'`, plus `SELECT s.protocol, s.identifier, COUNT(*) FROM LRSAdmin.alf_node n JOIN LRSAdmin.alf_store s ON s.id=n.store_id JOIN LRSAdmin.alf_qname t ON t.id=n.type_qname_id WHERE t.ns_id=(SELECT id FROM LRSAdmin.alf_namespace WHERE uri LIKE '%multifs%') GROUP BY s.protocol, s.identifier;` |
| **G6** | **Node counts per store and type** (confirms the layout in §1.2/§3, the cross-store lodgment question, and the one-node-per-task assumption) | `SELECT s.protocol, s.identifier, ns.uri, t.local_name, n.node_deleted, COUNT(*) FROM LRSAdmin.alf_node n JOIN LRSAdmin.alf_store s ON s.id=n.store_id JOIN LRSAdmin.alf_qname t ON t.id=n.type_qname_id JOIN LRSAdmin.alf_namespace ns ON ns.id=t.ns_id GROUP BY s.protocol, s.identifier, ns.uri, t.local_name, n.node_deleted ORDER BY 6 DESC;` and for lodgments: `SELECT ps.protocol AS parent_store, cs.protocol AS child_store, ca.is_primary, COUNT(*) FROM LRSAdmin.alf_child_assoc ca JOIN LRSAdmin.alf_node p ON p.id=ca.parent_node_id JOIN LRSAdmin.alf_node c ON c.id=ca.child_node_id JOIN LRSAdmin.alf_store ps ON ps.id=p.store_id JOIN LRSAdmin.alf_store cs ON cs.id=c.store_id JOIN LRSAdmin.alf_qname q ON q.id=c.type_qname_id WHERE q.local_name='Lodgment' GROUP BY ps.protocol, cs.protocol, ca.is_primary;` |
| **G7** | **One sample document** end to end (validates §5.4) | pick one recent `lr_source_document.id` with `page_count > 0`; run §5.4 steps 1–3 and return node ids, types, aspects (`alf_node_aspects`), property names and types, **but not** `lrs:entitydata` values; plus the matching `alf_content_url` rows and the existence (`dir`) of the files on `D:\` |
| **G8** | **One sample user** layout (names only, no values) | `SELECT q.local_name, p.actual_type_n, p.persisted_type_n FROM LRSAdmin.alf_node_properties p JOIN LRSAdmin.alf_qname q ON q.id=p.qname_id WHERE p.node_id IN (<usr:user node of a test account>, <its cm:person node>);` and the aspects of both nodes; plus counts `SELECT LEN(p.string_value) AS len, CASE WHEN p.string_value COLLATE Latin1_General_BIN LIKE '%[a-f]%' THEN 'lower' WHEN p.string_value COLLATE Latin1_General_BIN LIKE '%[A-F]%' THEN 'upper' ELSE 'digits' END AS hexcase, COUNT(*) FROM … WHERE q.local_name='password' GROUP BY …` (no hashes leave the server) |
| **G9** | **Permission nodes and every ACE** (201 rows) | §7.3 query with `pos` included (not only 0), plus `SELECT * FROM LRSAdmin.alf_access_control_entry;`, `alf_acl_member`, `alf_access_control_list` (ids and flags only), and the full non-user `alf_authority` list (`WHERE authority LIKE 'GROUP[_]%' OR authority LIKE 'ROLE[_]%' OR authority NOT IN (SELECT user names)`). Also the distinct values of `perm:strategy` |
| **G10** | **Group hierarchy** | `SELECT gp.string_value AS parent_group, ISNULL(cg.string_value, cu.string_value) AS member, ca.is_primary FROM LRSAdmin.alf_child_assoc ca JOIN LRSAdmin.alf_qname q ON q.id=ca.type_qname_id AND q.local_name='member' JOIN LRSAdmin.alf_node_properties gp ON gp.node_id=ca.parent_node_id AND gp.qname_id=(authorityName) LEFT JOIN LRSAdmin.alf_node_properties cg ON cg.node_id=ca.child_node_id AND cg.qname_id=(authorityName) LEFT JOIN LRSAdmin.alf_node_properties cu ON cu.node_id=ca.child_node_id AND cu.qname_id=(userName);` (export group→group links in full; for group→user links export counts per group only, or user names if PLAGIS agrees) |
| **G11** | **One open queue node** (`lrs:Transaction`) and its `wf:taskRef` | for one transaction id: the property names and values of its `lrs:Transaction` node(s) (names are not personal; mask `applicantName`) and the aspects. Also count `lrs:Transaction` nodes per `transactionId` to confirm one node per task |
| **G12** | `wf:comment` volume and sample | `SELECT COUNT(*) …type wf:comment…`; one sample's property names; the distinct values of `wf:commentType`/`commentAcs` |
| **G13** | Disk layout | on .5: `dir /s /b D:\LRS_STORAGE` limited to two levels (`tree /a` of the top folders), folder sizes for `contentstore`, `contentstore.deleted`, `lucene-indexes`, `backup-lucene-indexes`, `audit.contentstore`; and the names of the folders under `lucene-indexes` |
| **G14** | `alf_applied_patch` (174) | `SELECT id, description, applied_to_schema, applied_on_date, succeeded FROM LRSAdmin.alf_applied_patch ORDER BY applied_on_date;` (shows the ILS patches: `filestoresContainer`, `rootStoresPermissions`, `conform2model`, `GrantGroupsWithPermissionsPatch`, …) |
| **G15** | The ILS-patched `AbstractContentStore.extractSroreId` and the content-store Spring beans (`mainStore`, `multiFileStoresService` constructor arguments, `redirectToDefault`, `checkInterval`) | copy `lrs.war\WEB-INF\lib\alfresco-repository*.jar` (inspect only with `javap`) and the `*-context.xml` that defines `MultiFileStoresService` |
| **G16** | Version store use | `SELECT COUNT(*) FROM LRSAdmin.alf_node n JOIN LRSAdmin.alf_store s ON s.id=n.store_id WHERE s.identifier='version2Store';` and the count of `cm:versionable` aspects on `lrs:scanneddocument` nodes (answers the `ScanVersioningSupported` conflict) |
| **G17** | jBPM ↔ mirror variables | `SELECT NAME_, CLASS_, COUNT(*) FROM LRSAdmin.JBPM_VARIABLEINSTANCE WHERE NAME_ LIKE '[_]alf%' OR NAME_ LIKE '[_]last%' GROUP BY NAME_, CLASS_;` (confirms `_alfProcessInst`/`_alfTaskInst`) |

Also still open from other documents and relevant here: A1/A2 (jBPM definitions and runtime, for §6.3 and §8)
and B4 (report templates in `reports://reportsStore`; they can be extracted with the G1(b) pattern using
`rpt:reportTemplate` and its `rpt:reportContent` association).
