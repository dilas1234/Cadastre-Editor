# 05 - The Workstation: Registry Desktop client and Cashier

Scope: the Windows "Aumentum Registry Workstation 4.3" (WinForms .NET 2.0-era, DevExpress grids/editors, SandDock docking,
LeadTools imaging, WSE 3.0 SOAP), the PLAGIS customisation assemblies, its ExternalConfig plug-in files, and the
"Aumentum Registry Cashier" web folder. Requirement gathering only - this is the build reference for the new
workstation UI that must run against the same LRS43 database.

Path conventions used below:

- `$R` = `tools/aumentum-inventory` (captures). Client files: `$R/lrs-export/map-capture/registry-client/bin/...`
  (shortened to `bin/...`), Cashier: `$R/lrs-export/map-capture/registry-client/Aumentum Registry Cashier/...`.
- `$D` = `scratchpad/dec/net/<assembly>/...` (ILSpy output). Class names are given as `Namespace-tail/Class`.
  `Controls` = `ThomsonReuters.Aumentum.Registry.Controls.dll`, `PLAGIS.Controls` = `ThomsonReuters.Aumentum.Registry.PLAGIS.Controls.dll`,
  `Services` = `...Registry.Services.dll`, `DataModel` = `...Registry.DataModel.dll`, `Desktop` = `...Registry.RegistryDesktop.exe`.
- Data: `$R/lrs-export/registry/data/<table>.psv.gz`, DDL `$R/lrs-export/registry/schema/lrs-tables.sql`.
- "(inferred)" marks anything not read directly from code/config.

---

## 0. Executive summary (what the developer must know first)

1. **The client is a thin, metadata-driven shell.** Almost no screen layout is hard-coded. Every page of every
   workflow stage is chosen by `bin/WorkflowConfig/*.config` (`<wizard stages=... meta="all;<TRN codes>">` ->
   ordered list of `<editor type=PageClass ...attributes>`), and every field on every page/dialog/grid is rendered
   from the **`lr_formfield`** table (1,725 rows, 170 form names - exported, see Appendix A) by
   `Controls/Core/BindingFactory2.CreateAndBindEditors(container, formName, object)`. Replicating
   "WorkflowConfig + lr_formfield + ~35 page types" reproduces the workstation.
2. **There is only one worker.** `Workers.config` maps every PLAGIS stage to
   `PLAGIS.Controls/WorkerAssess/CustomAssessWorker` (subclass of `Controls/WorkerAssess/AssessWorker`). It opens a
   wizard (`WorkerWizard`) per transaction in the batch, pages = editors configured for (stage, transaction code).
3. **The save model is "whole batch".** Every save sends the whole `Batch` (lodgment) object graph to
   `ReceptionService.AcceptApplication(processInstanceUuid, batch, "com.landsystems.lrs.model.Batch")`
   (`AssessWorker.DoSaveCallback`). New lodgements first start a jBPM process
   (`WorkflowService.startProcessInstance(definitionId, [transactionTypeCode])`). Completing a stage =
   client-side rules (`rules=` attribute) -> save -> `WorkflowService.CompleteTask(taskNodeRef, transitionName, comment)`
   -> optional `AssignTask`.
4. **Business rules live in the client**, in two static classes: `Controls/WorkerAssess/Rules` and
   `PLAGIS.Controls/WorkerAssess/CustomRules`, invoked by name from `rules="Name:args;..."` on the
   `CompleteAssignPage` editor. These include fee-paid gates, cadastre (MC) job create/approve/reject,
   unlinked-parcel checks, receipt/demand-notice numbering and ground-rent posting. **`ExternalConfig/Rules.config` is
   effectively dead config** (its consumer `RuleServiceAgent` is never registered - see section 4).
5. **Payments**: fees are calculated through the server `CashierService.CalculateFee(cashierTrnCode, params)` using a
   parameter schema `cashier.xsd` fetched with `GetSchema`; payments are recorded as `lr_payment` /
   `lr_payment_transaction` rows inside the batch by the desktop (DemandNoticePage). The separate "Cashier" web app
   capture contains **only a Web.config and 4 helper DLLs - its own code is not captured** (section 7).
6. **Printing**: all letters/certificates are **BIRT `.rptdesign` templates rendered on the server**
   (`ReportsService.ProduceReport`) and returned as PDF/DOC bytes; the client shows them in an embedded browser and
   attaches them as outgoing documents (`sd_*` document types). Templates are not in the capture.
7. **Scanning**: LeadTools TWAIN/WIA in `ILS.Scanning.dll`; pages are uploaded with HTTP
   `PUT /lrs/upload/{guid.ext}?ticket=...&encoding=UTF-8`, then Alfresco `lrs:scanneddocument` / `lrs:scannedimage`
   nodes are created through `RepositoryService.update` (CML).
8. **Map**: an embedded IE browser hosting the Cadastre GISViewer
   `http://10.10.10.6/GISViewer/MapWorker.aspx?ct=indexer&justmap=1&map=...` with POSTed `lrsuser/lrsticket` and a
   JavaScript bridge `doAction(...)` (section 2.11). This repo already serves that page (`src/routes/gisviewer.rs`).

---

## 1. Architecture of the client

### 1.1 Assemblies

| Assembly | Role | Source |
|---|---|---|
| `ThomsonReuters.Aumentum.Registry.PLAGIS.RegistryDesktop.exe` | PLAGIS entry point. Only `ApplicationLauncher.Main` -> `ShellContext.Main(args)`. Ships the PLAGIS `.exe.config`. | `$D/...PLAGIS.RegistryDesktop.exe/.../ApplicationLauncher.cs` |
| `ThomsonReuters.Aumentum.Registry.RegistryDesktop.exe` | Product shell: `ShellContext` (startup/login), `ShellForm` (main window, queue grid, menus), `TaskFilterView`, `NotificationView`, `DetailsForm`, `ChangePasswordForm`, `NewForm`, `InstallUpdateManager` (ClickOnce) | `$D/...RegistryDesktop.exe/ThomsonReuters.Aumentum.Registry.RegistryDesktop/*.cs` |
| `ThomsonReuters.Aumentum.Registry.Controls.dll` | All product screens: worker framework, wizard, pages, dialogs, rules, scanning worker, map form, fee configuration | 437 files |
| `ThomsonReuters.Aumentum.Registry.PLAGIS.Controls.dll` | PLAGIS overrides (section 3) | 73 files |
| `ThomsonReuters.Aumentum.Registry.DataModel.dll` | Domain classes (`Batch`, `Transaction`, `LAUnit`, `SpatialUnit`, `Party`, `RRR`, `Payment`...), product overrides of service agents (`DomainModelServiceAgent`, `BusinessLogicServiceAgent`, `CashierServiceAgent`), `DocumentProcessor` (scan import) | 124 files |
| `ThomsonReuters.Aumentum.Registry.PLAGIS.DataModel.dll` | Constants only (`CustomTransactionTypeCode`, `CustomFormName`, `RegionCode.Plateau="plateau"`, `DistrictCode.Barkin="plateau_1"`) + empty `CustomBusinessLogicServiceAgent : BusinessLogicServiceAgent` | 7 files |
| `ThomsonReuters.Aumentum.Registry.Services.dll` (+ `.XmlSerializers.dll`) | WSE SOAP proxies and service agents for all `/lrs/api/*` services, `AgentFactory`, `NameSpaces`, MapWorker control | 560 files |
| `ThomsonReuters.Aumentum.Registry.Common.dll`, `LRS.Common.dll`, `ILS.Common.dll`, `ILS.UI2.dll`, `ILS.Messaging.dll` | Base forms (LoginForm, SplashForm, OptionsForm, BaseForm), settings providers (Windows registry), service container, JMS-style messaging | |
| `ILS.Scanning.dll` | LeadTools-based scan/view/annotate component (TWAIN `Leadtools.Twain`, `WIAScanDriver`, image filters, PDF engine) | 172 files |
| Not captured but referenced | DevExpress (XtraGrid 2.5.1, XtraEditors 2.2.1, Utils, Data, XtraLayout), SandDock, Leadtools.*, Microsoft.Web.Services3, Microsoft.Office.Interop.Word (DOC preview) | `bin/...exe.config` bindingRedirects; `using` lines |

### 1.2 Startup and sign-in

Sequence (`Desktop/ShellContext.Main`, `ShellContext()` ctor, `ShellContext.Authenticate`):

1. Args: `-showstack` (show stack traces in errors), `-culture:<name>`. Else culture = setting `UICulture` (en-US),
   or the last login language if `SupportBilingual`.
2. Single instance: `SingleInstanceApplication.Uri = "Aumentum Registry Workstation 4.3"`; a second launch activates the first.
3. Splash form; `AddServices()` creates `AuthenticationServiceAgent`, `MessagingServiceAgent`, `AdministrationServiceAgent`,
   `SignatureManager`, `MessageBoxService`, then **instantiates every `<agent>` of `ExternalConfig/Agents.config`** by
   reflection and registers it in a service container under its `interface` (section 1.4). Optional proxy
   (`UseDefaultProxy`/`UseCustomProxy`).
4. `LoginForm.Login(...)` (the shell uses `Controls.Core.LoginForm`; `LRS.Common/LoginForm` is a legacy copy, see 11 §2): fields **Server** (combo `cbServer`, history kept in `LoginSettings.ServerList`
   in the Windows registry), **User name**, **Password**, optional **Language** (only if bilingual). `WindowsAuthentication`
   is hard-wired `false`. Then `AuthenticationAgent.SignIn(server, user, password)` -> `AuthenticationService.startSession`
   -> ticket.
5. On `AuthenticationUpdated`, every service agent rebuilds: `UsernameToken(user, ticket, PasswordOption=2 /*SendPlainText*/)`,
   WSE timestamp TTL 300 s, and **the authority of every configured URL is replaced by the login Server**
   (`ServiceAgentBase.BuildConnectionUrl`: `host` if it contains a port, else `host:8080`). So the `localhost:8080`
   URLs in the exe.config are templates only.
6. Post-login check: HQL `from RegistryOffice r where r.Active = true` (via `DomainModelService`); none -> "no active office" and sign-out.
7. `ShellForm` constructed (section 2.1). Session expiry (`SessionExpiredException`) -> error + log off.
   Clock skew: a SOAP fault containing "Invalid timestamp..." becomes "time synchronization" error that points the
   user at `http://<host>/lrswa` (`ServiceAgentBase.HandleException(SoapException)`).
8. File > Login (reconnect) first calls `InstallUpdateManager.InstallUpdateSyncWithInfo` (ClickOnce
   `ApplicationDeployment.CheckForDetailedUpdate/Update` - only if network-deployed).

Current registry office: `DataModel/DomainModelServiceAgent.Initialize` reads the user node property `officeId`
(Alfresco person) -> `RegistryOffice where office_name='<officeId>'`; fallback first `RegistryOffice where active=1 and office_type='registration'`.
It also loads every server XSD (`GetSchemaList`/`GetSchema`) into dictionaries, and `transaction_type` from
`from TransactionMetaData where hidden=0 order by sort_order,label`.

### 1.3 Settings in `bin/ThomsonReuters.Aumentum.Registry.PLAGIS.RegistryDesktop.exe.config`

No secrets in this file (only public key tokens, masked in the capture).

| Section / setting | Value | Meaning |
|---|---|---|
| `RegistryDesktop.Properties.Settings/HelpFile` | `WorkstationHelp.htm` | F1/Help>Contents file (per-culture subfolder first) |
| `UICulture` | `en-US` (fa-IR commented out) | UI language; `fa-IR`, `ps-AF` satellite folders exist (product heritage, Afghanistan) |
| `UseDefaultProxy`, `UseCustomProxy`, `CustomProxyAddress` | False, False, empty | HTTP proxy |
| `WindowsAuthentication` | False | |
| `SupportSignature` | False | digital signature enrolment menu hidden |
| `ApplicationName` | `Aumentum Registry Workstation` | title |
| `ProjectName` | `Aumentum Registry for Plateau State (PLAGIS)` | title suffix |
| `SupportRegistry` | False | File>New lists **transaction groups** (not registries) |
| `SupportBilingual`, `UICultureList` | False, empty | |
| `SupportCryptoServer` | False | |
| `Services.Properties.Settings/*_URL` | `http://localhost:8080/lrs/api/{DomainModelService, IntegrationService, WorkflowWebService, MessagingService, AuthenticationService, AccessControlService, DictionaryService, PersonWebService, ReportsService, ContentService, AuthoringService, AdministrationService, ReceptionService}`, `/lrs/ws/BusinessLogicService`, `/lrs/cxfapi/RepositoryWebService` | host replaced at login. Defaults compiled in `Services/Properties/Settings.cs` add `CashierWebService_URL=/lrs/api/CashierService`, `FeeService_URL=/lrs/api/FeeService`, `/lrs/cxfapi/BusinessLogicService`, and `MCPortal/WebServices/CadastralWebService.asmx` |
| `configSections` | `Workers, Helps, Wizards, Rules, History, Agents, Dictionaries, InternalViewers, ViewPlugins, DraftEditors` handlers in `Controls/Core/*SectionHandler` | |
| `<Wizards>` | 14 top-level `WorkflowConfig\*.config` + 26 `WorkflowConfig\SubsequentRegistration\*.config` | **the 26 per-transaction files are not in the capture** (the folder was captured as a 0-byte file); a combined `SubsequentRegistration.config` (211 KB) is present and covers ABWG..UPMO (section 10) |
| `<History>`, `<ViewPlugins>` | empty | |
| `microsoft.web.services3/diagnostics/detailedErrors` | true | |

### 1.4 Plug-in model (ExternalConfig)

All files carry "generated by the configuration generator tool" headers.

**Agents.config** - service objects created at start (`ShellContext.AddServices`):

| Interface | Implementation (assembly) | Server endpoint |
|---|---|---|
| `IDomainModelServiceAgent` | `DataModel.DomainModel.DomainModelServiceAgent` | `/lrs/api/DomainModelService` (HQL `loadObjects`, save, `GenerateNumber`, settings, metainfo/lr_formfield, dictionaries) |
| `IBusinessLogicServiceAgent` | **`PLAGIS.DataModel.BusinessLogic.CustomBusinessLogicServiceAgent`** (empty subclass of `DataModel.BusinessLogic.BusinessLogicServiceAgent`) | `/lrs/ws/BusinessLogicService` + client logic (postpone/withdraw/continue) |
| `IMetaDataServiceAgent` | `Services...Data.MetaData.MetaDataServiceAgent` | bilingual flags |
| `IPersonServiceAgent` | `Services...Repository.Person.PersonServiceAgent` | `/lrs/api/PersonWebService` (groups are Alfresco authorities `GROUP_<name>`) |
| `IDictionaryServiceAgent` | `...Repository.Dictionary.DictionaryServiceAgent` | `/lrs/api/DictionaryService` (Alfresco model metainfo for queue columns) |
| `IRepositoryServiceAgent` | `...Repository.Repository.RepositoryServiceAgent` | `/lrs/cxfapi/RepositoryWebService` (Lucene queries on `workspace://LandRegister` store, CML updates) |
| `IAccessControlServiceAgent` | `...Repository.AccessControl.AccessControlServiceAgent` | `/lrs/api/AccessControlService` (`IsPermissionExist`, `GetPermission`) |
| `IWorkflowServiceAgent` | `...Repository.Workflow.WorkflowServiceAgent` | `/lrs/api/WorkflowWebService` (jBPM) |
| `IContentServiceAgent` | `...Repository.Content.ContentServiceAgent` | `/lrs/api/ContentService` + `PUT /lrs/upload/{name}` |
| `IReportServiceAgent` | `...Repository.Report.ReportServiceAgent` | `/lrs/api/ReportsService` |
| `IAuthoringServiceAgent` | `...Repository.Authoring.AuthoringServiceAgent` | `/lrs/api/AuthoringService` (lock/unlock nodes, versions) |
| `IScanServiceAgent` | `...Repository.Scan.ScanServiceAgent` | Repository store `SpacesStore` |
| `IReceptionServiceAgent` | `...Data.Reception.ReceptionServiceAgent` | `/lrs/api/ReceptionService` (`AcceptApplication`, `Withdrawal`) |
| `IIntegrationServiceAgent` | `...Data.Integration.IntegrationServiceAgent` | `/lrs/api/IntegrationService` (`saveJob`, `getJob`, `getParcelPointsData`, SaveEntity) |
| `IDocumentManager` | `Controls.WorkerAssess.Scan.DocumentManager` | client-side |
| `IQueueService` | `Controls.WorkerAssess.Queue.QueueService` | adds an extra filter string to the queue query |
| `IWorkerAssessFormFactory` | **`PLAGIS.Controls.WorkerAssess.Forms.CustomWorkerAssessFormFactory`** | swaps in PLAGIS dialogs |
| `ICashierServiceAgent` | `DataModel.Cashier.CashierServiceAgent` | `/lrs/api/CashierService` |
| `IFeeServiceAgent` | `Services...Data.Fee.FeeServiceAgent` | `/lrs/api/FeeService` (`calculateFee(FeeRateDTO)`) |

Not registered (so unusable at runtime): `IRuleServiceAgent` (`Services...Data.Rule.RuleServiceAgent`) - see section 4.

**Workers.config** - `<worker stages="..." registries="registration" assembly type>`; all 14 entries point to
`PLAGIS.Controls.WorkerAssess.CustomAssessWorker`. `ShellForm.GetWorker(registry, stage)` picks the entry whose
`stages` contains the jBPM task name (stage) and `registries` contains the transaction registry; if none,
`WorkerFactory.LaunchCompleteWorker` (a bare "complete task" wizard) is used. The union of stage names in Workers.config
is the PLAGIS stage list (`PLAGIS.Controls/WorkerAssess/CustomWorkflowTask` lists the same 80 constants).

**InternalViewers.config** - toolbar buttons in the worker's "View" section (`AssessWorker.BuildViewToolbar/WizardCallsMe`):

| method | caption key | shown | action |
|---|---|---|---|
| `ShowDocuments` | strScannedDocuments | all transactions/stages except INSC at New/LodgmentApplication/IndexDeed | opens the scanning worker for the current transaction (permission filter `lrsAddScanned, lrsIndexingScanned, lrsAddAnnotation`) |
| `ShowPropertyInfo` | Property Info | all except INSC at New/LodgmentApplication/IndexDeed, and all at New | `CustomPropertyInfoForm` |
| `ShowRejectionLetter` | Rejection Letter | `accept=""` (never by default) | `WorkerFactory.LaunchRejectWorker` |
| `ShowHistory` | View History | as ShowPropertyInfo | `HistoryForm` (process history) |

`extensionForm="TransactionExtForm|PropertyExtForm|PartyExtForm|DocumentExtForm"` is declared but those forms are not in `lr_formfield`.

**Helps.config** - maps form names to help topics (`OptionsForm 22, HistoryForm 242, TransactionForm 240, ImageControl 44,
RejectPage #_Toc414448212, AssignPage #_Toc414448205, ScanningClientPanel 44`); every `<editor help="#_Toc...">` in WorkflowConfig does the same for pages.
`WorkstationHelp.htm` is not captured.

**Dictionaries.config** - see section 5. **Rules.config** - section 4.

### 1.5 How WorkflowConfig maps stages to screens

`Controls/Core/WizardSectionHandler` reads each `<workflow filePath>`; each `<wizard stages="A,B" assembly meta="all;CODE1,CODE2">`
holds ordered `<editor name type help [mode] [page] ...custom attributes>`. `WizardFactory.CreateDrafterWizard(owner, asm,
stage, "metaType;transactionCode", env, mode)` picks the wizard matching (stage, code) and instantiates the page classes in order.

Editor `name` syntax: `<pageKey>[:<formName>[:<secondForm>[:RO]]]`, e.g. `transaction:TransactionWithDocFRGForm`,
`applicant:::RO` (read-only page), `properties:PropertyGrid`. Any other XML attribute is read by the page through
`GetAttributeValue<T>("attr")` (all attributes per page are listed in section 2 and Appendix B).

Run-time flow inside `AssessWorker.EditObject` (task opened from the queue):

1. Load batch: `RepositoryAgent.GetLodgmentIDByProcess(procInstanceRef)` then HQL `from com.landsystems.lrs.model.Batch where id=?`;
   init scanned documents for each transaction (`DocumentProcessor.InitTransactionScannedDocuments`).
2. Env keys: `EDIT`=batch, `TRANS_CODE`, `stage`=task name, `_TASK`, `_REJECTED` (process variable `isRejected`), `_REJECT_FORM="ReturnForm"`.
3. If `mcintegration/IntegrationWithMC=1` and `multicadastre/job_creation=1`: walk `multicadastre/job_stage_description(n)` =
   `CODE:Stage:jobSettingName:Warning|Confirm`; if a cadastre job of that name is still open
   (`IntegrationService.getJob(transactionId)` row with `JobComments=jobName` and empty `JobEnded`) -> **block opening
   the task** (Warning) or ask (Confirm). 95 such entries exist (e.g. `APRO:PlotParcelInCadastre:job_plot_parcels:Warning`).
4. One wizard per non-past transaction of the batch (caption "<type label> No. <number> (k of n)"); the `complete` page
   is removed except on the last one.
5. Results: Discard/None -> return reserved counter numbers (`HandleCounterPoolForCancel`); Save -> `SaveChanges`.

New lodgement (`AssessWorker.CreateObject`, from File>New): new `Batch` with current office; for INSC and SROE an
applicant is auto-added (PLAGIS presenter, section 3); wizard stage `New`, meta `all;<code>`.

### 1.6 Main shell (`Desktop/ShellForm`)

Docked layout (SandDock, persisted in `ApplicationSettings.Layout`, Windows registry):

- **Registration Queue** (centre, DevExpress banded grid) - the work list.
- **Transaction Filter** dock (`TaskFilterView`, form `TaskFilterForm`).
- **Notification Window** dock (`NotificationView`).
- Status bar: Connected / Loading / Disconnected, total records.

Menus (`ShellForm.ApplyResourceStrings`, `InitializeMainMenu`):

| Menu | Item | Visible/enabled when | Action / service calls |
|---|---|---|---|
| File | New > {transaction group} > {transaction type} (also toolbar `tsbNew`) | authenticated and user can start >=1 process definition | groups/types from `lr_transaction_metadata` (`GetTransactionGroupList`, `GetTransactionListByGroup`) filtered by `WorkflowAgent.AuthorizedProcessDefinitionList` (jBPM definitions keyed by `workflow` name), `hidden=0`, `config/HideNormalTransactionCode` (LEAS,RLEA hidden) and `config/WFShowGroups` (all). Click -> `ICreateWorker.CreateObject(code)` |
| File | Login / Logout / Exit | | ClickOnce update check + `Authenticate`; `endSession` |
| Edit | Edit Task (F2, toolbar, double-click) | exactly one stage selected across selection and user is actor or member of actor group | see "Edit" below |
| Edit | Unlock | selected node locked by me | `AuthoringService.UnLock` |
| Edit | Assign Task | permission `lrsAssignTask` | `WorkerFactory.LaunchAssignWorker` -> `AssignPage` -> `WorkflowService.AssignTask(taskRef, userOrGroupRef)` + `AddComment`; locks nodes via `AuthoringService.Lock` |
| Edit | Diary Book | `config/CheckDiaryBook=1` and `lrsDiaryBook` | `LaunchDiaryBookWorker` (setting not present in lr_setting -> hidden at PLAGIS) |
| Edit | Lodgment Extension | `config/LodgmentExtension=1` | `LaunchExtensionWorker` (hidden at PLAGIS) |
| Edit | Postpone | permission `lrsPostponeRegistration`, single row, not suspended | confirm -> `RejectPage` in Postpone mode (reasons, `PostponeForm`) -> `WorkflowService.SuspendProcessInstance(processId)` -> batch status `application_postponed`, transactions `transaction_postponed` (saved via AcceptApplication) |
| Edit | Continue | `lrsContinueRegistration`, row suspended | `ResumeProcessInstance` -> `application_lodged` / `transaction_lodged` |
| Edit | Withdraw | `lrsCancelRegistration`; stage not in `TransactionMetaData.nonWithdrawableTasks` | `RejectPage` (Withdraw mode, WithdrawalLetter) -> ProcessHistory row (`taskTransition="Withdraw"`) -> `ReceptionService.Withdrawal(processId)`; if MC integration: `IntegrationService.saveJob(McJob status=rejected, TransactionId)`; DCT: end-date the corrected properties/parties/documents/RRRs; GRR: delete new `RentalAmountRevision` rows |
| View | Details (F3) | one row | `DetailsForm` with `ShellFormQueueDetails` fields |
| View | View in Web Access (F4) | | opens `config/LwaUrl` (`http://{0}/lrswa/welcome.do`) `?locale=..&user=..&ticket=..#detail:Transaction:<id>` |
| View | Workflow Diagram (F7) | | `http://<host>/lrs/downloadBPM?definitionId=<id>&ticket=<ticket>` in `BrowserForm` |
| View | Map (+ toolbar) | permission `lrsMapAccess` and `mcintegration/IntegrationWithMC=1` | `Core/MapForm.ShowModal` with `mcintegration/GISServer`, `GISMapTransaction`, `LWASearchTransaction` |
| View | Transaction Filter, Notification Window, History, Refresh | | |
| Tools | Change Password | `config/RestrictChangePswd=0` | `AdministrationService.ChangePassword(old,new)` (old checked locally) |
| Tools | Options | | `OptionsForm` with `SubscriptionOptionPanel` (notification subscriptions) |
| Tools | Signature Enrolment | `AdministrationAgent.IsSignatureSupported` | hidden (SupportSignature=False) |
| Toolbar | Configuration Fee > Fee Structure / Fee Rate | permission `lrsFeeConfiguration` | `FeeStructureExtForm`, `FeeRateExtForm` (lr_fee_structure / lr_fee_rate; FeeService.calculateFee test) |
| Toolbar | Transaction View: All / Assigned to me / Assigned to my groups | `config/RestrictTaskView` (0) | queue query variants |
| Help | Contents, About | | |

**Queue query** (`ShellForm.RefreshTaskList`) is a Lucene query on the Alfresco LandRegister store (`RepositoryService.query`,
sorted by task-create desc), not SQL:

- base `TYPE:"{http://www.landsystems.com/lrs/model/core/1.0}Transaction" AND (@lrs\:taskCompleted:"false")`
- All: `AND ((@lrs\:taskActorName:"<user>") OR (@lrs\:taskSwimlane:"<group1>") OR ...)`
- Mine: `AND (@lrs\:taskActorName:"<user>")`; My groups: `AND (taskActorName:"<group>" OR ...)`; Custom: filter string from `TaskFilterView` inserted.
- `IQueueService.Filter` appended. Limit `bounds/QueueLimit` = 40000.

Columns = `lr_formfield` form `ShellFormQueue` (Process ID, Transaction Number, Transaction, Workflow, Workflow Stage,
Applicant Name, Assigned To, Source Property Number, Target Property Number, Submission Date, Locked By, Postponed),
merged with Alfresco model metainfo (`DictionaryService.GetMetaInfo(TYPE_TRANSACTION, aspects lockable+tempTransaction)`).
Row colouring: postponed rows maroon; any `*dueDate` cell red if past, yellow if due within 2 days. Grid column
layout is persisted per user (`DataViewerColumnSettings`).

**Edit Task logic** (`miEditEdit_Click`): if the process is suspended, only members of group **"Postpone Clerk"** may
continue it (prompt, then `ContinueRegistration`). If the transaction's property has a caveat in progress or an active caveat RRR
and the transaction code is not in `config/ValidTransactionCodesForCaveatTransaction` (GRPM,GRR,SEAR,LOCE) -> silently refuses.
Then `WorkerFactory.LaunchWorker`: `WorkflowService.LockTask(task)` -> worker -> `UnLockTask`. Completed tasks raise
"cannot edit completed task".

**Notification window** (`NotificationView`): `MessagingManager.CreateSubscription("", "lrs", "domain='messaging' and (user='_all_' or user='<user>')")`
plus per-event-type subscriptions chosen in Options (`message_type`, dictionary `event_type`); messages cached in
`%AppData%\_lrsIndexerMessage_<user>.xml`; system messages pop up as an alert.

---

## 2. Screens, by area

Notation: "Fields: form X" means the editors come from `lr_formfield.form_name = X` (full field lists in Appendix A).
"Svc" = server calls. Page captions are from `Controls.Properties.Resources.resx`.

### 2.1 Login, shell, work list - see 1.2 and 1.6

| Screen | Class | Fields | Svc |
|---|---|---|---|
| Splash | `LRS.Common/SplashForm` | title, project | - |
| Login | `Controls.Core.LoginForm` (not `LRS.Common`, see 11) | Server (combo with history), User name, Password, Language (bilingual only) | `AuthenticationService.startSession`; HQL RegistryOffice |
| Main window / Registration Queue | `Desktop/ShellForm` | `ShellFormQueue` | RepositoryService.query (Lucene), WorkflowService.getTaskInstance/getProcessInstance, AuthoringService.isLocked |
| Transaction Filter | `Desktop/TaskFilterView` | `TaskFilterForm`: Transaction No., Transaction (dictionary transaction_type), Applicant Name, Source/Target Property Number, Submission Date, Workflow Stage, Assigned To (authority; excludes ALFRESCO_ADMINISTRATORS, EMAIL_CONTRIBUTORS, guest, webclient) | builds Lucene clause; saved in registry |
| Details | `Desktop/DetailsForm` | `ShellFormQueueDetails` | - |
| History | `Controls/Core/HistoryForm`, `HistoryView` | `HistoryViewForm` (Started, Finished, ...), `HistoryViewFormEvent` | process history (`lr_process_history` via DomainModel; inferred) |
| Change password | `Desktop/ChangePasswordForm` | old, new, confirm | `AdministrationService.ChangePassword` |
| Options | `Controls/Core/OptionsForm` + `Desktop/SubscriptionOptionPanel` | event-type subscriptions | Messaging |
| About | `Controls/Core/AboutForm` | | |
| Browser | `Controls/Core/BrowserForm` | | workflow diagram, LWA |
| New (registry picker) | `Desktop/NewForm` | registry list (`from REGISTRY`) | only when SupportRegistry=True (not PLAGIS) |

### 2.2 The worker wizard frame

`Controls/WorkerWizard/WorkerWizard` (left step list, page area, Back/Next/Finish/Save/Suspend/Cancel buttons, a "View"
toolbar section from InternalViewers). Results: `werCommit` (finished via complete page), `werSave` (Save & close),
`werDiscard`, `werNone`. Mandatory fields are red labels; `CheckPageDBFields` validates mandatory + `validation_rule`
before leaving a page. Every page derives from `WizardPage` with `BeforeExecute` (load), `PageActivate`,
`AfterExecute(direction)` (validate/store), `GetAttributeValue<T>`.

### 2.3 Reception / lodgement (stage `New`, `LodgmentApplication`, `LodgeMemorandum`, `IndexData`)

| Page (type in WorkflowConfig) | Caption | What it shows / does | Attributes | Svc |
|---|---|---|---|---|
| `PLAGIS CustomTransactionListPage` (base `Controls.Pages.TransactionListPage`) | Add Transactions | Grid `TransactionListGrid` of transactions in the batch; Add/Edit/Delete open the **Transaction dialog** (2.3.1). Add creates `Transaction` with type from TRANS_CODE, `isSearchOldfile=false`; stores the chosen jBPM `ProcessDefinition` in env `_DEFINITION`. Removing a Subsequent Registration / Ground Rent transaction also clears batch parties. Withdraw of a not-yet-saved process: `ReceptionService.Withdrawal(processId)` | `transaction_dialog_form` (mandatory-check form on Next), `validate_property` (`checkNotPending, checkLinkCadastre, checkTransactionMetadata, checkRegisterAlready, checkGroundRentReviewTransactionCanStart`), `validate_right`, `transactionListForm` | DomainModel GetMetaInfo, CreateServerInstance |
| `PLAGIS CustomMultiApplicantsPage` (base `MultiApplicantsPage`) | Applicants | Grid `PartiesGrid`; Add/Edit/Delete/Search applicant. Applicant dialog = `CustomApplicantForm` (forms `ApplicantIndividualForm`, `ApplicantCorporationForm`, `ApplicantCorporationStateForm`, `MultiApplicantForm`); search = `ApplicantSearchForm` (`SearchIndApplicantForm`/`SearchCorpApplicantForm`, grid `SearchApplicantGrid`) | - | HQL on Applicant |
| `PLAGIS CustomPropertiesPage` (base `PropertiesPage`) | Properties | Grid `PropertyGrid` (or `PropertyCofOGrid`); New/Edit/Delete property (`CustomPropertyForm`), Search (`PropertySearchForm`), View owners, Map (link parcel), Add source to target, View/Map source property. Generates `suid_number` for new spatial units | `mainForm` (PropertyForm*, e.g. `PropertyFormForLodgment`), `plotForm`, `unitForm`, `buildingForm`, `siteAnalysisForm`, `searchvisible`, `mapvisible`, `mapreadonly`, `maplinktransaction`, `addbuttondisabled`, `deletebuttondisabled`, `addtotargetpropertybuttondisabled` (+ settings `DisablePropertiesAdd/Delete/AddToTargetButtonOnStageName`) | GetRegisteredProperties, GetPropertyForBackfile, broken-chain checks, Save |
| `FeeListSelectionPage` | Fee | Two grids: `AvailableFeeGrid` (fee types, dictionary) and `CalculatedFeeGrid` (PaymentTransaction: type, calculated fee...). Select / Deselect / Deselect all / **Recalculate** (opens `FeeCalculationForm`). Blocks Next if no fee amount (except SSG with no cashier code) or fees not all calculated; stamps `demandnoticeNumber` on each fee line. Ground rent lines (`feetype_groundrent_C_GR`, `feetype_groundrentpenalty_C_GRP`) are managed by OutstandingPage | `feesForTargetProperties` (multiply fees by number of target properties) | CashierService.CalculateFee |
| `PLAGIS CustomOutstandingPage` / `OutstandingPage` | Outstanding Ground Rent | Grids `OutstandingGrid(MultiProp)`, `PenaltyGrid(MultiProp)`; add/edit/delete outstanding year and penalty (dialog `OutstandingForm`: year, amount). Validation: current-year ground rent required, year > 0, amount > 0, no duplicate year | `feetype`, `capture_past_outstanding`, `view_past_outstanding`, `show_penalty_section`, `current_year_validation` | - |
| `PLAGIS CustomPrintDocuments2Page` | Documents | Lodgement letter + demand notice (see 2.9) | `reports=LodgementLetter:pdf:1:sd_lodgement_letter;DemandNotice:pdf:1:sd_demandnotice` | ReportsService |
| `CustomSearchPropertyPage` (SEAR/CETC) | Search Properties | "Search for Property Number" box; grids Found / Not Found; New/Edit/Delete, View parcel. Caps number of properties (`max_prop_number`), multi-property only for codes in `config/TransactionCodeMultipleProperties`; sets `sourcePropertyIDs` | `max_prop_number`, `validate_property` | GetRegisteredProperties, GetArchiveRegisteredProperties |
| `SurveyPlanPage` (SPC) | Survey Plan | grid `SurveyPlanGrid`, dialog `SurveyPlanForm` (25 fields: plan no., surveyor, dates, area...) | | |
| `GroundRentReviewPage` (GRR) | Ground Rent Review | Edit `GroundRentReviewAttributeForm` (Effective Date, Annual Rental Amount >= 0) per property; `AnnualRentalAmountForm` | | `RentalAmountRevision` |
| `CompleteAssignPage` | Complete Task | see 2.10 | | |

#### 2.3.1 Transaction dialog (`PLAGIS CustomTransactionForm` : `Controls.Forms.TransactionForm`)

- Layout: transaction fields (`TransactionLayoutControl`, form per stage e.g. `TransactionForm`: Transaction No.(RO),
  Transaction (RO), Tenure Type*, Old File No., Search By Certificate No., Certificate No.*, File No. (= `sourcePropertyIDs`)*, Comments),
  "Transaction parameters" group (cashier entity editor, hidden by PLAGIS `UpdateCashEditor` override), Supporting documents list.
- Process definition = `WorkflowAgent.AuthorizedProcessDefinitionList[transactionType.workflow]`; error if user cannot start it.
- Property checks per `validate_property`: **checkNotPending** (no other lodged/issued/postponed transaction on the property
  in another batch, HQL in `DataModelUtilities.HasPendingTransactions`; warns if property already registered or archived),
  **checkTransactionMetadata** (`relatedTransactionCode` must have been registered on the property; `blockingTransactionCode`
  `CODE:status;status` must not be pending/completed), **checkLinkCadastre** (property must exist and every spatial unit
  must have a UPIN when IntegrationWithMC), **checkRegisterAlready**. RRR checks: active/expired RRRs of `transactionType.rightType`.
- PLAGIS OK-button additions (section 3): MORT/UPMO/RMOR source property re-pointed to the target of the active mortgage RRR's source transaction;
  single-property enforcement unless code in `TransactionCodeMultipleProperties`; INSC duplicate check and broken-chain
  check; UPMO consideration currency check; default fee lines from `cashierTransactionCode`; **demand-notice number**
  (`GenerateNumber("demandnotice_number")`, INV prefix) for groups SSG, FRG, GRT, MCG, SVG; strip spaces from property IDs; GRR clears target.

### 2.4 Search (SEAR, CETC) - `Search.config`

Stages New/LodgmentApplication (CustomSearchPropertyPage, applicants, documents) -> ScanIncomingDocuments -> AwaitingFeePayment ->
RegisterFeePaymentAndGenerateReceipt -> GenerateSearchLetter (`CustomPrintSearchPage`, report `SearchLetter -> sd_search_report`) / RetrievePropertyFile ->
CertifyDocuments (CETC) / DispatchPropertyFileApplicantReview (SEAR) -> DeliveryOfDocuments -> ScanOutgoingDocuments.
Rules: `CheckUnpaidFee`, `CheckFoundProperties` (no "Generate Search Letter" when nothing found; no "Retrieve Property File" when something found),
`AllowTransition` / `ForbidTransition` per code.
Property search dialogs elsewhere: `PropertySearchForm` (fields `PropertySearchForm`, grid `PropertyGrid`), `PropertySearchFormINSC`,
`TransactionSearchForm` (`TransactionSearchForm`/`TransactionSearchGrid`), `PartySearchForm`, `ApplicantSearchForm`, `PartyContactSearchForm`.
Search result limit `bounds/SearchLimit` (100).

### 2.5 Property / LA unit

| Screen | Class | Fields | Notes |
|---|---|---|---|
| Property dialog | `PLAGIS CustomPropertyForm` : `PropertyForm` | main form per attribute `mainForm` (`PropertyForm`, `PropertyFormForLodgment`, `PropertyFormINSC`, `PropertyFormDCT`, `PropertyAPCAForm`, `PropertyFormCHPC`, `PropertyFormForMERT`, `PropertyFormForEXTTandPSUR`, `PropertyFormNonMandatory`, `PropertyUnitForm`), plot sub-form (`PlotForm*`: Land Use Category, Land Use Type, Survey Date, Surveyor's Name, Surveyor Plan Number, Comment), `UnitForm`, `BuildingForm`, `SiteAnalysisForm` (55 fields), `PropertyExtensionForm` | PropertyForm: Property Number (RO), Property Type (spatial_unit_type, RO), Plan Number*, State* (region), LGA* (region_lga, cascading via AddressHierarchyHelper), Property Area* (property_area), Location* (location), Street No/Name, Layout Name (layout_type), Plot No, Block No (become mandatory when Layout chosen - PLAGIS `HandleLayoutChanged`), District, Town, Village, Ward, Area* (n3, >=0), Measurement Unit*, Valuation Amount, Tracing Number, UPIN (RO). Land use category -> type cascade. PLAGIS: Rental Amount field becomes a button-edit that opens `FeeCalculationForm` for fee type `feetype_groundrentpublicland_C_GRPL` (except stage PrepareValuationReport) |
| Property info | `PLAGIS CustomPropertyInfoForm` : `PropertyInfoForm` | `PropertyInfoForm`, `PropertyGrid`, `PartiesGrid`, `PropertyTransactionsGrid`, `SupportedDocGrid`, `PaymentGrid`, `TransactionIndexForm` | read-only dossier; active RRRs via `GetActiveRRRs` |
| Map link | `Controls.Forms.MapForm` | | section 2.11 |

Registered-property rule (`DataModelUtilities.GetRegisteredProperties`): `LAUnit` with `Uid=?`, both LAUnit and SpatialUnit
`BeginLifespanVersion not null and EndLifespanVersion null`, highest `Version`.

### 2.6 Parties / persons

| Screen | Class | Fields | Buttons |
|---|---|---|---|
| Parties page | `PLAGIS CustomPartiesPage` : `PartiesPage` | grid `PartiesGrid`; dialog forms `partyForm`/`individualForm`/`organizationForm`/`governmentForm`/`partyRepresentativeForm`/`partyCommentForm` attributes; `PartyForm`, `PartyShortForm`, `PartyContactForm` | Add, Edit, Delete, Add Applicant (as party), Add Owners (`AddOwnersForm` - pull registered owners), Search Party, Specify transfer rules. PLAGIS: Add-Owners hidden, Add-Applicant visible unless `add_applicant_button_visible="0"`; on Next requires one party with role = `transactionType.targetPartyType` marked title holder ("to whom all correspondence ... will be sent"); marks parties that hold the active RRR as "enforced" |
| Party dialog | `PLAGIS CustomPartyForm` : `PartyForm` | `IndividualForm`/`IndividualEditableForm` (26), `CorporationForm`/`CorporationEditableForm` (17), `CorporationStateForm`, `CorporationMunicipalityForm`, `PartyContactForm` (27), `PastIndividualForm`, `PastCorporationForm` | contacts panel (`PartyContactsGrid`) |
| Applicant dialog | `PLAGIS CustomApplicantForm` | `ApplicantIndividualForm`, `ApplicantCorporationForm`, `ApplicantRegistrationOfficeForm`, `ApplicantScanCorporationForm` | condition rules hide fields for party types party_scan / representative / party_mlstp |
| Party search | `PLAGIS CustomPartySearchForm` -> `PartySearchForm` | `PartiesSearchForm`, `PartiesSearchGrid` | Search |
| Representative search | `PartyContactSearchForm` | `PartyContactsSearchForm/Grid` | |

### 2.7 Rights (RRR), instrument registration, past transactions

| Page | Used at | Behaviour | Svc |
|---|---|---|---|
| `RrrPage` | New, ReviewSubmission, GenerateCofO, RegisterCofO, PrepareLandForm5, GRR stages, Scan*, Delivery (19 wizards) | Three panels: properties (`PropertyGrid`, add/edit/delete/search), target parties and source parties (`PartiesRrrPageGrid`) and the RRR attributes form (`rrrForm`, default `RRRAttributeForm`; also `RRRAttributeDisableForFRTrForm`, `RRRAttributeUPMOForm`): Term years/months/days, Date of Commencement, Expiration Date (RO), Rental Amount, Revision Period, Erection Period, Consideration + currency, Improvement Amount + currency, Security Number #1/#2. Read-only-for lists per transaction code (Appendix A). PLAGIS checks: mandatory fields per `set_mandatory_fields` | GetActiveRRRs, GetRRRBySecurityNumber |
| `RegisterInstrumentPage` | RegisterCofO, RegisterInstrument | Form `TransactionRegisterInstrumentForm`/`RegisterInstrumentForm` (volume, folio, registration date). Button **Register Instrument**: `GenerateNumber("instrument_number")` (prefix INS), `page_number|<ins>` (P, resets every 500), `volume_number|<ins>` (V, increments with page). Sets `transaction.recordingno/folio/volume/registrationDate=now`; per spatial unit `volumefolio/folio/volume` (separate instrument per property when >1 property; skipped for codes in `config/DeactiveCofOTransactionCodes`). Numbers are reserved in `env["COUNTER"]` and committed to `lr_tempcounter` on save or returned to the pool on cancel | DomainModel.GenerateNumber |
| `RegisterPlanNumberPage` | RegisterSurveyPlan (SPC) | `RegisterPlanNumberForm`; button generates `plan_number` (prefix DP) | |
| `CustomPastTransactionPage` : `PastTransactionPage` | INSC IndexData/NewIndexData/ReviewDataINSC/BLFinalReview | "Title Transactions (Input Past Transactions)": grid `PropertyTransactionsGrid`, dialog `CustomPastTransactionForm` (`PastTransactionForm` 25 fields, parties, `PastPaymentForm/Grid`, `RegUnitForm`). Past transaction numbers `GenerateNumber("past_number")` (P-PLS). PLAGIS: validate past transaction can be registered given active RRRs; forbids adding past consent/assignment/mortgage without the needed active RRR | |
| `CustomRelatedTransactionPage` | (not used in captured configs) | Add/Remove related transaction | |
| `TransactionCorrectionPage` + `DataCorrectionRrrPage` | DCT New/ScanCorrectedDocuments/ReviewCorrection | Correction Transaction: search the transaction to correct (blocked codes `DCT,INSC,GRR,SPC`), editable fields per `set_enable_fields` (applicationDate, recordingno, folio, volume, relatedInstrNumber/PageNumber/Volume, dataCorrectedComments); RRR/property/party corrections. On "Finish Data Correction" rule `SaveDataCorrection` swaps ids/versions between the DCT copy and the original (section 4) | |

### 2.8 Documents and scanning

| Screen | Class | Fields / behaviour | Svc |
|---|---|---|---|
| Transaction with documents page | `PLAGIS CustomTransactionWithDocumentsPage` : `TransactionWithDocumentsPage` (used in ~290 wizards) | Top: transaction form named in editor (`TransactionWithDocFRGForm`, `...SSGForm`, `...GRPMForm`, `...SPCForm`, `...FormAPRF`, `...FormREPT`, `...FormSROE`, consideration variants, `TransactionIndexFormINSC`); bottom: supporting documents grid `SupportedDocGrid`, New/Edit/Delete/View pages/View registered. Document dialog `CustomDocumentForm` (`SupportedDocForm`: Document Type* (document_type), Document Number, Date Issued, Primary Document, Issued By (issued_by), Property File Page, Description; `SupportedDocFormOut` for outgoing). Attributes: `documents` (auto-add document types), `docEnabled`, `checkPages` (require scanned pages), `set_enable_fields`, `set_mandatory_fields`. PLAGIS: old file number uniqueness (`CheckOldFileNoInReg`), cashier flag | `ScanAgent.DocumentManager` |
| Scanning worker | `Controls/WorkerScanning/ScanWorker`, `ScanForm`, `ScanViewForm`, `ScanningClientPanel`, `BatchContentDockableWindow`, `PropertiesDockableWindow` | Left: documents of the transaction; right: page thumbnails/viewer with annotations (LeadTools), Scan (TWAIN `ILS.Scanning/TwainWrapper` or WIA), Add file, reorder, delete, rotate/deskew/despeckle/crop filters, export (hidden by `scan/ScanHideExportButton`), show file names (`config/ScanShowFileNames=1`), document number/status columns (`scan/ShowDocumentNumber`, `ShowDocumentStatus`), max image 1024 KB / attachment 100 MB (`scan/MaxImageSize`, `MaxAttachmentSize`). Permissions `lrsAddScanned` (add/edit pages), `lrsIndexingScanned` (index info), `lrsAddAnnotation`, `lrsCheckScanned` | see below |

Upload flow (`Controls/Storage/ImagerStorageLrsDriver`, `DataModel/Scan/DocumentProcessor`, `Services/.../ContentServiceAgent`):

1. Each new page/thumbnail/annotation: `PUT http://<host>:8080/lrs/upload/<guid>.<ext>?ticket=<ticket>&encoding=UTF-8` -> content URL.
2. CML batch via `RepositoryService.update`: create `{lrs}scanneddocument` node under the scan root (store `workspace://SpacesStore`) with
   `{lrs}referenceid` = transaction id, `{lrs}entityid` = SourceDocument id and the serialized document entity; child
   `{lrs}scannedimage` nodes per page with `cm:content`, thumbnail and annotation properties; deletes for removed pages.
3. Optional versioning (`cm:versionable` aspect, `AuthoringService.createVersion`) - `config/ScanVersioningSupported=0` at PLAGIS.
4. Reading: `ContentService.read` / `ReadContentStream(nodeRef, "{cm}content")`.
5. On save the document's `pageCount`, `modifiedAt/By` are set; if documents changed the batch is flagged MODIFIED.

Mandatory-document gates: `AssessWorker.IsMandatoryDocumentNotMissing` (env `DOCUMENTS`), `TransactionMetaData.mandatory_docs_codes`,
rule `CheckUnscannedDocuments[:Confirm|:Warning]`, `config/OnlyMandatoryDocs=0`, `config/SkipMissingDocWarning`.

### 2.9 Payments, fees and printing pages

| Page | Caption | Behaviour | Svc |
|---|---|---|---|
| `DemandNoticePage` (168 wizards; every "Register ... Fee Payment and Generate Receipt" and most review stages) | Payment | Grid `PaymentGrid` (payments made) + calculated fee grid; Add/Edit/Delete payment -> `DemandNoticeForm` "Payment Info": `PaymentForm` = Amount Paid (c2, RO = sum of selected fees), **F.I. Receipt Number*** (bank teller receipt), Date Paid* (<= today), Financial Institution Name* (dictionary `bank_list`), Comment; select/deselect fee lines (`CalculatedFeeGrid`) to cover. Duplicate check: same `receiptNo` + `bankType` already used (`DataModelUtilities.GetActivePayments`). New payments carry `internalReceiptNo` of the transaction. `payments_validation="true"` requires a new payment at this stage | HQL Payment |
| `PLAGIS CustomCalculateFeePage` / `CalculateFeePage`, `CalculateGroundFeePage`, `PaymentsFeePage`, `TotalFeePage`, `RegistryPaymentPage`, `PaymentPage`, `PastPaymentsFeePage`, `PrintDocumentsFee` | Fee / Payments / Payment Advice / Register Fee Payment / Confirmation of Lodgment | product pages **not referenced** by PLAGIS WorkflowConfig (inventory only) | |
| `FeeCalculationForm` (dialog from FeeListSelection Recalculate, property rental amount) | | Fee structure aware (`lr_fee_structure` flags: transaction type, property area, size, location, land-use category/type, size range); cashier parameters editor generated from `cashier.xsd` for `CashierTransactionCode` (suffix after `_C_` in fee type code, inferred); Calculate -> `CashierService.CalculateFee(code, "name=value;\n...")` | CashierService |
| `CalculateFeeForm`, `CalculateGroundFeeForm` | | same engine (`CreateCashEntity` + `CalculateFee`) | |
| Fee configuration (`Core/ConfigurationFee/FeeStructureExtForm`, `FeeRateExtForm`, `FeeRateForm`, `FeeStructureGrid`) | | maintain `lr_fee_structure` / `lr_fee_rate`; test via `FeeService.calculateFee(FeeRateDTO)` | FeeService |
| `PLAGIS CustomPrintDocuments2Page` (65), `CustomPrintDocumentsMultiProp2Page` (21; one report per target property), `CustomPrintSearchPage`, `PrintDocuments2Page` | Documents / Multiple Properties Document | Grid of configured reports (No, Title, Description, Generated, Printed checkbox - cannot tick Printed before generating). Clicking a report: auto-add `documents` types, **save the batch first if modified** (`AssessWorker.FinalSave`), check parameters, `ReportsService.ProduceReport(name, PDF|DOC, {}, params)`, write to `%AppData%\<title>...pdf`, show in embedded browser (DOC opens Word via COM). If the report spec has a document type, an outgoing `IncomingDocument` of that type is added to the transaction (`AddDocumentToTransaction`; number `outd_`+`document_number` when blank) | ReportsService.getReportList, produceReport, getReportResourceUrl; ContentService |

Report spec syntax (`reports=` attribute): `ReportName:format:mandatory(1/0):documentType` separated by `;` - the template is
`<ReportName>.rptdesign` from `ReportsService.GetReportList()`. DemandNotice is dropped for codes in `config/HideFeeComputation` (RCOO,RROC,SURO).

### 2.10 Completion, assignment, rejection, postponement

| Screen | Class | Behaviour | Svc |
|---|---|---|---|
| Complete Task (last page of every wizard) | `Controls/Core/CompleteAssignPage` : `CompletePage` | List of outgoing transitions (`WorkflowService.GetTaskTransitionInfo(taskRef)`), comment box, optional assignee list for the next task (users of the next task's swimlane group: `PersonService.GetGroupUserList`). Transition filtering: `excludeTransition="CODE*CODE|Transition,..."`; `config/DeactiveCofOTransactionCodes` + `DeactiveCofOHide/ShowTransitionNames` ("none"); `HideFeeComputation` codes hide `HideFeeComputationTransition` ("Awaiting Fee Payment", "Awaiting for Payment", "Register Processing Fee Payment and Generate Receipt"); `hideCadastreTransitionForArchivedProperty` hides cadastre transitions when a spatial unit status is `archived`; stages in `config/RejectedStages` ("Reject Registration, Rejected, Generate Rejection Letter") and `DeniedStages` ("Denied") get special handling. Finish -> `CompletePage.CompleteTask`: optional confirm (`config/confirm_complete_task`), task not already ended, **before_push** = `AssessWorker.OnComplete` (mandatory docs, `rules=` execution, FinalSave), then `WorkflowService.CompleteTask(taskRef, transition, comment)`, `AddComment` for queued notes, `AssignTask(nextTaskRef, userRef)` if an assignee was chosen | Workflow, Person, Repository.query (`TYPE:"{sdms wf}taskRef" AND @wf\:taskCompleted:"false" AND @wf\:taskProcInstRef:"..."`) |
| Assign Task | `Core/AssignPage` | users of task swimlane group + the group itself; comment | `WorkflowService.AssignTask`, `AddComment` |
| Rejection Reasons | `Core/RejectPage` (+ `RejectPagePresenter`, `ListEditorT`) | Grid `RejectFormGrid`, dialog form `RejectForm` (Reason* dictionary `returned_reason`, In Addition text). Modes Reject / Postpone / Withdraw. Save: `BusinessLogic.CustomReject` (PLAGIS DataModel: all transactions -> `transaction_rejected`, end-date LA units and spatial units), `Rejection` saved (`lr_rejection`, `lr_rejection_reason`), process variable `rejection=<id>` (`WorkflowService.SetProcessVariables`). Prints letter if `drafter/PrintOnReject=1`: RejectionLetter / PostponeLetter / WithdrawalLetter `.rptdesign` with `CurrentUser` | DomainModel.Save, Workflow, Reports |
| Prepare Rejection Letter stage | `RejectPage` in wizard + `CustomPrintDocuments2Page` (`RejectionLetter:pdf:1:sd_reject_letter`) | | |
| Postpone Transaction | `Core/PostponePage` | `PostponeForm` (reason) attached rejection | |
| Return form | env `_REJECT_FORM="ReturnForm"` (`ReturnForm` fields) | sending back a task | |

### 2.11 Map (Cadastre GISViewer)

- Shell map (`Controls/Core/MapForm`) and property-link map (`Controls/WorkerAssess/Forms/MapForm`, OK/Close) host
  `Services/MapWorker/MapWorkerControl` (IE WebBrowser).
- URL: `<mcintegration/GISServer>?ct=indexer&justmap=1&map=<mapName>&lang=<culture>[&mapcfg=..]` with POST body
  `auth=1&lrsuser=<user>&lrsticket=<ticket>&initparam_inputobjects=<json>&initparam_extent=<env>`. GISServer =
  `http://10.10.10.6/GISViewer/MapWorker.aspx`.
- Config strings from lr_setting `mcintegration`: `GISLinkageConfig` = `MapName=property-select;MapLayer=Plateau_State_Parcels;SDELayer=sde.SDE.Plateau_State_Parcels->ParcelID->ParcelID->sde.SDE.ParcelAttributeTable`,
  `GISSearchFields` = `uid::false:Property UID,upin:UPIN:false:Upin:true,district::false:Country`, `GISRetrievedFields` = `upin:UPIN`,
  `GISUpdatedFields` = `suID:LRSID,1:LinkageStatus,spatialDate1:LinkageDate,refnum:PropertyNumber,plotNumber:Plot,blockNumber:Block`,
  `AreaUnitMatch` = `1:sqft;2:acre;3:sqm;4:sqkm;5sqyrd`, `LRSNonUpdatableKeyFields`/`LRSTransAllowUpdateKeyFields` empty.
- JS bridge: `doAction("Editor","IsEditInProgress")`, `doAction("Map","GetExtentAsString")`, `doAction("SelectionSet","GetSelectedObjects")`;
  page signals via hidden element `indexerEvents` (`value` attribute + onclick). Linking writes the GIS attributes back with
  `CadastralWebService.UpdateCadastralData` (`MCPortalServiceUrl`) and copies UPIN/area/plot/block into the LRS spatial unit.
- Already reimplemented server-side in this repo (`src/routes/gisviewer.rs`, `src/routes/mcservice.rs`); the new workstation only needs to embed/redirect.

### 2.12 Admin-type screens in the client

Fee configuration (2.9), Change password, Options/subscriptions, Signature enrolment (off). User/group/role administration is
**not** in the desktop - it is in the server admin UI (`/lrs/faces/jsp/dashboard`). Batch content viewer `BatchContentForm` (`SimpleTransactionGrid`).

### 2.13 Unused product workers (present but not configured at PLAGIS)

`WorkerDrafter/DrafterWorker` (+ `ListOfInstruments`, drafter Party/Property forms), `WorkerDelivery/DeliveryWorker`
(Select/Delivered/Finish pages; reads `config/CheckPrintReceipt`), and pages `ApplicantPage`, `BlockingTransactionPage`,
`CalculateFeePage`, `CalculateGroundFeePage`, `NotesPage`, `PageApplicationVerify`, `PageCoTMemo`, `PagePropReportPrinting`,
`PastPartiesPage`, `PastPaymentsFeePage`, `PaymentPage`, `PaymentsFeePage`, `PrintDocumentsFee`, `PrintDocumentsPage`,
`RecommendationNotesPage` (`RecommendationForm/Grid`), `RegistryPaymentPage`, `RelatedTransactionPage`,
`RrrPageWithSourceParties`, `TotalFeePage`, `TransactionDeedPage`. Not needed for parity unless the missing
`SubsequentRegistration\*.config` files use them (check when captured).

---

## 3. PLAGIS customisations (PLAGIS.* assemblies)

| Item | Overrides | What it adds |
|---|---|---|
| `CustomAssessWorker` | `AssessWorker.InitializePresenter` | uses `CustomAssessWorkerPresenter` |
| `CustomAssessWorkerPresenter` | `SetMethod`, `IsApplicantAutomaticallyAdded`, `CreateApplicant` | rule lookup **CustomRules first, then Rules**; INSC and SROE get an automatic applicant: existing Applicant with party type `party_scan` (INSC) / `party_mlstp` (SROE) or a new Corporation named after the current registry office (address, phone) |
| `CustomRules` | adds/overrides rules | section 4 |
| `CustomWorkerAssessFormFactory` | dialogs factory | Custom Party/Applicant/PropertyInfo/Transaction/PastTransaction/Property/Document/CalculateFee forms |
| `CustomTransactionForm` (+Presenter, `CustomTransactionLayoutControlPresenter`, `CustomDocumentListViewPresenter`) | OK button, cashier editor | mortgage re-pointing, single-property rule, INSC duplicate & broken-chain check, UPMO consideration check, default fee lines from `cashierTransactionCode` (HQL `from Dictionary where Code=?...`), demand notice number |
| `CustomPropertyForm` | load, layout change, spatial-unit-type change | rental amount fee button (`feetype_groundrentpublicland_C_GRPL`), plot/block mandatory when layout set (not INSC), region->LGA and land-use cascade |
| `CustomPartiesPage` | buttons, Next | title holder check, enforced parties, applicant button |
| `CustomTransactionListPage` (+Presenter) | add/remove | Subsequent codes list (ABWG, APCA, APCM, ASSI, CEMP, CHNT, CHPC, COMP, DEVO, EXTT, LEAS, LOCE, MERT, MORT, PSUR, RCOO, RLEA, RMOR, RSUB, SUBD, SUBL, RROC, SURO, UPMO, PWOA, RPOA): clear term/expiry fields except CHNT, DEVO; SROE copies comments from related transaction; non-INSC clears target property on reset |
| `CustomSearchPropertyPage` (+Presenter) | new page | property search for SEAR/CETC |
| `CustomOutstandingPage` | multi-property ground-rent arrears | grids `OutstandingGridMultiProp`, `PenaltyGridMultiProp` |
| `CustomPrintDocuments2Page/Presenter`, `CustomPrintDocumentsMultiProp2Page`, `CustomPrintSearchPage` | report parameters | C of O: `WatermarkURL` (`CertificateWatermark.png` report resource) and `DeedPlanURL` = content URL of page 2 (or 1) of the transaction's `sd_titledeedplan` document whose number = property number; check one title deed plan per target property with matching numbers. RegistrationMemorandumMemo: `Consideration` (from empty RRRs or UPMO tempConsideration, `#,##0.00`). RecommendationMemorandumMemo: `Addressee` = dictionary `document_type.descr` |
| `CustomPastTransactionForm/Page/Presenter`, `CustomRelatedTransactionPage`, `CustomMultiApplicantsPage`, `CustomApplicantForm/Page`, `CustomPartyForm`, `CustomPartySearchForm`, `CustomDocumentForm`, `CustomCalculateFeeForm/Page`, `CustomPropertiesPage`, `CustomPropertyInfoForm`, `CustomMultiplePropertiesPage` | mostly thin | past-transaction RRR validation (ASC ordering), applicant dictionaries |
| `PLAGIS.DataModel` | constants | transaction codes (section 8), PLAGIS form names, region/district codes |
| PLAGIS resource strings | `PLAGIS.Controls.Properties.Resources.resx` (62 strings) | user messages quoted in section 4 |

---

## 4. Rules

### 4.1 `ExternalConfig/Rules.config` (as captured)

| ruleset stages | meta | class | rule | meaning |
|---|---|---|---|---|
| BLFinalReview | INSC | `Controls.WorkerAssess.Rules` | `SetComplete` params "" | Intended: add condition inscription, set registrationDate=now, sort transactions, mark past transactions `isRightRequired=false`, attach the property to past transactions, cancel past encumbrances. **With empty params it returns immediately** (`if (string.IsNullOrEmpty(args)) return`). |
| BLFinalReview | INSC | `PLAGIS...CustomRules` | `UpdateBalance` | copy `transaction.balanceSum/balanceDate` to the first spatial unit |
| IndexData | INSC | `CustomRules` | `LinkPropertyAndPastTransaction` | past transactions without properties (and with a right type) get the main transaction's first property |

Consumer: `Services/WebService/Data/Rule/RuleServiceAgent.Execute(env, stage, meta, page)` - but `IRuleServiceAgent` is not in
Agents.config and no code calls `AgentFactory.RuleAgent`, so **this file has no runtime effect** (inferred from the absence of callers).
The same rules are reachable through `rules=` attributes (LinkPropertyAndPastTransaction is used at INSC IndexData Complete).

### 4.2 Live rules (`rules=` on CompleteAssignPage; executed in `AssessWorker.ExecuteRules`)

Syntax `Rule[:args][;Rule...]`. Method resolved in `CustomRules` then `Rules`; signature `(IDictionary env, string args)`.
A rule cancels completion by setting `env["_PUSH_STEP"]=null` (or throwing "noaction"). `_PUSH_STEP` = chosen transition name.

| Rule | Args | Meaning | Used in |
|---|---|---|---|
| `CheckUnpaidFee` | `T1|T2[|T3]` | skip if code in `HideFeeComputation`. If any fee line has `calculatedFee` (= unpaid) and transition is T1 or T3 -> block "All calculated fees have not been paid" (". Please select 'Awaiting Fee Payment'..." when T2 given). If transition is T2 and nothing unpaid -> block "All calculated fees have already been paid" | APCO, APRF, REPT, FR, SEAR, SPC, GRPM, SSG |
| `CheckUnscannedDocuments` | none / `Confirm` / `Warning` | message listing documents without pages; Warning blocks, Confirm asks | most stages |
| `SaveGroundRentData` | | on "Finish Transaction" with MODIFIED: for each batch payment line `feetype_groundrent_C_GR` upsert `GroundRent(propertyNumber, paidYear, amount=manualFee, paidSum)` (adds to existing paidSum except INSC) | many |
| `MapPaymentWithGroundRentInfo` | | on "Finish Transaction": move ground-rent fee lines from transaction payments into batch-level `Payment` rows (paidYear, paidSum, balanceOwed=calculatedFee) | INSC BLFinalReview |
| `BlockIfPropertyIsNotValid` | | on "Finish Transaction": block if parcel in broken chain ("Transaction is required to go to ReDo Index data"), if the property-forming transaction is not completed, or any source property has another pending transaction | INSC |
| `CheckTranDict` | | on "Finish Transaction": block if `tranDict1` (property registration type) empty | INSC |
| `LinkPropertyAndPastTransaction` | | see 4.1 | INSC IndexData |
| `SetStatusForUnlinkedPastTransaction` | | if current transaction is in a broken chain, other past transactions of the batch set `transaction_rejected` and saved | INSC |
| `CreateMCJob` | `Transition|jobSettingName` | if IntegrationWithMC and `multicadastre/job_creation` and transition matches: unless an open job exists, `IntegrationService.saveJob` new `com.landsystems.lrs.model.McJob` entity (status added): JobName=`multicadastre/<jobSettingName>` value, JobInstanceName=`Job-<property uid or survey plan no>`, JobPriority 2, JobComments=jobName, JobParameters=`@LRS:[Transaction]:[TransactionNumber]=<no>`, TransactionId. One per target property when job name in `mcJob_multipleTargetProperties` ("Deed Plan Creationv2"). Errors decoded (UnknownHost, Logon failure, timeout) and completion blocked | INSC CreateParcel ("Parcel Creation"), Plot Parcel ("Plot Parcels"), SPC Plot Survey Plan, Raise Title Deed Plan ("Deed Plan Creationv2") |
| `ApproveMCJob` / `RejectMCJob` | `Transition|jobSettingName` | saveJob with status approved / rejected and TransactionId | Delivery of Documents (deed plan), INSC Finish (parcel) |
| `CheckUnlinkedParcel` (Rules) | `Warning|Error|Confirm` | any spatial unit without UPIN -> ask / block | INSC, survey plan approval |
| `CheckUnlinkedParcels` (Custom) | `Confirm|Warning` `|codes|exemptTransitions` | as above, listing property codes; only for listed codes | Raise Title Deed Plan |
| `CheckOccupancyReportsParameters` | | number of `sd_titledeedplan` documents = number of target properties, numbers match property ids, no duplicates | Raise Title Deed Plan |
| `UpdateSimpleCounterValue` | `receipt_number` / `demandnotice_number` | `GenerateNumber(counter)` -> `transaction.internalReceiptNo` (RCT+9 digits) or `demandnoticeNumber` (INV+9) | payment registration stages |
| `AddOutgoingSupportingDocument` | `Delivery of Documents` (unused arg) | add outgoing documents for `transactionType.completeReportsCodes` (or `rejectReportsCodes` when rejected) | delivery stages |
| `BlockTransition` | `CODE*CODE|Transition,...` | forbid listed transitions for listed codes ("'X' is not allowed for this transaction") | SSG |
| `AllowTransition` / `ForbidTransition` | `CODE|Transition` | only / never that transition for that code | Search |
| `CheckFoundProperties` | `T1|T2` | T1 needs >=1 found property; T2 needs none | Search |
| `UpdateTargetPartiesForPartSurrender` | | target-role parties get `unitUid = targetPropertyIDs` | PSUR |
| `SetRentalAmountRevisionAsActive` | | new `RentalAmountRevision` rows with an amount get `beginLifespanVersion=now` | GRR |
| `SaveDataCorrection` | | on "Finish Data Correction": swap ids/lifespans between DCT copy and original Transaction, LAUnits, SpatialUnits, Parties, RRRs; copy corrected header fields to original; clone scanned documents both ways | DCT ReviewCorrection |
| Others available, not referenced in captured configs | | `ChangeRegOffice`, `GenerateRights`, `GenerateOwnershipRights`, `SetCompleteOld`, `CancelEncubrancePast`, `UpdatePaymentBalance`, `CheckValidGRMUPIN`, `CheckParcelNotFound`, `UpdateTargetPropertyIDsToTargetParties`, `VerifyRetrievedPartiesForMergeOfTitles`, `UpdateConsiderationValue`, `InvoiceStatusRule` (AssessWorker; cashier invoice status 2 or 5 = unpaid -> confirm) | |

Field-level helper rules: `SetDisableFields`, `SetEnableFields`, `SetMandatoryFields` (attributes `set_disable_fields` etc.),
`HandleCountryChanged` (address region/district/subcounty/cell/ward enabled only for country `ng`), `HandleTenancyTypeChanged`
(share numerator/denominator only for `tenancy_common_spec` / `tenancy_common_equal`), `HandleLayoutChanged`.

### 4.3 Field metadata rules (lr_formfield)

`validation_rule` values: `LE today`, `GE today`, `GE 0`, `GT 0`, `LE 99/11/31`, `GT 0&LE 999`, `OE 21` (operators GE/LE/GT/LT/EQ/NE/OE; `&` = and; right side
may be a field name e.g. `LE surveyEndDate`). `condition_rule` hides a field unless the object type matches (`!=party_scan`, `==representative`, `!=INSC`,
`Settings.config.HideNormalTransactionCode`). `readonly_for` (547 rows `all`) / `enabled_for` (`ng`, `all`) are lists of object types (transaction code,
spatial-unit type, country) - `BindingFactory2.ModifyEditors`. `format`: `dd/MM/yyyy`, `c2` (currency), `n2/n3`, regex (`\d{1,2}`, e-mail).
`mru` = most-recently-used list. `category` = `lr_dictionary.category` for `lrs:Dictionary` fields.

---

## 5. Dictionaries cached by the client

`ExternalConfig/Dictionaries.config`:

| Provider | Categories |
|---|---|
| `DataModel.DomainModel.TransMetaInfoDictionaryProvider` | `transaction_type` (as `TransMetaInfo` for the filter) |
| `Services...Person.AuthorityDictionaryProvider` | `user`, `group`, `authority` (Alfresco people/groups; query `@cm\:authorityName:"GROUP_<name>"`) |

All other dictionaries are lazy-loaded and cached per session by `DataModel.DomainModelServiceAgent.GetDictionary(category)`:
HQL `from Dictionary where category=? and hidden=0 and begin_lifespan_version is not null and end_lifespan_version is null order by sort_order,label`
(table `lr_dictionary`), with the `null` category's blank item prepended. `transaction_type` comes from `TransactionMetaData where hidden=0`.
Settings are cached per category (`from Setting where category=?`). Categories referenced by `lr_formfield` (Appendix A, column
"dictionary") include region, region_lga, property_area, location, layout_type, spatial_unit_type, actual_use_type, use_specific,
area_measurement_unit, tenure_type, currency, bank_list, document_type, issued_by, returned_reason, party_role_type, party_type, ...;
code paths add transaction_status, application_status, event_type, document_type, party_type, party_role_type.
The Cashier parameter dictionaries come from `cashier.xsd` (`CASH:<code>:<param>`), not lr_dictionary.

---

## 6. Printing / reports

- Engine: **server-side BIRT** (`lrs.war/WEB-INF/lib/ils-reports.jar`, `ils-reports-das.jar`, `wsdl/reports-service.wsdl`); client calls
  `ReportsService.getReportList` and `produceReport(name, format, dataSources, params)` -> bytes. No Crystal, no local templates.
- Standard parameters (`PrintDocuments2PagePresenter.SetReportParameter`): `LodgmentId`, `TransactionId`, `CurrentUser` (full name),
  `CurrentLogin`, `JobTitle`, `EmployeeNumber` (from Alfresco person node), `Border` (resource `Border.png`), `PropertyId`, `PropertyNumber`;
  PLAGIS extras in section 3.
- Reports configured in PLAGIS WorkflowConfig (template -> outgoing document type):

| Report (`.rptdesign`) | Doc type | Where |
|---|---|---|
| LodgementLetter | sd_lodgement_letter | New/LodgmentApplication (9 wizards) |
| DemandNotice | sd_demandnotice | New/Lodgment, GenerateFinalDemandNotice |
| FeePaymentVerificationReceipt | sd_payment_receipt | every Register...Payment...Receipt stage (21) |
| CertificateOfOccupancyTitle | sd_Co_title | GenerateCofO (12) |
| RegistrationMemorandumMemo | sd_regist_memoMem | RegisterCofO / RegisterInstrument |
| RecommendationMemorandum | sd_recMemo_Com | PrepareRecommendationMemorandum |
| RejectionLetter | sd_reject_letter | PrepareRejectionLetter |
| DeliveryLetter | sd_delivery_letter | DeliveryOfDocuments (14) |
| Landform5 | sd_landform_5 | PrepareLandForm5 |
| AcceptanceOfTermsAndConditions | sd_rightOcc_accept | PrepareRightOccupancyAcceptanceTermsConditionGrant |
| SearchLetter | sd_search_report | GenerateSearchLetter |
| GroundRentReviewConveyance | sd_groundrent_conveyance | GRR ApplyNewGroundRentalAmount |
| SiteAnalysis | sd_SAR | PrepareSiteAnalysisReport |

Product constants also name `CadRejectionLetter`, `OrPostponeLetter`, `PrPostponeLetter`, `PostponeLetter`, `WithdrawalLetter` (`Controls/WorkerAssess/ReportName`).
Other expected document types (from `documents=` attributes): sd_traverse_compute, sd_finSurvey_report, sd_surbReport_RightOcc, sd_landfoem_7,
sd_valuatReport, sd_recMemo_Gov, sd_COrecom_Gov, sd_survey_67, sd_surveyPlan, sd_devolte_order, sd_certified_document, sd_CM, sd_titledeedplan.
Server bootstrap report property files (ApplicantRegister, ContentSummary, InstrumentsAgeing, PendingTransactions, StaffList, StaffProductivity*)
are management reports of the admin UI, not the desktop.

---

## 7. Cashier

### 7.1 What the capture contains

`Aumentum Registry Cashier/Web.config` + `bin/{ILS.Common, ILS.Messaging, ILS.UI2, LRS.Common}.dll` only.

- ASP.NET app (classic, `compilation debug="true"`, `authentication mode="Windows"`, `httpRuntime maxRequestLength=40960`,
  `executionTimeout=240`, default document **`Default.asp`**), WSE 3.0.
- Settings section `LRS.Client.ServiceProxy.Properties.Settings`: `AuthenticationWebService_URL`, `MessagingWebService_URL`,
  `PersonWebService_URL`, `RepositoryWebService_URL=/lrs/api/RepositoryService` (all localhost:8080).
- No page/handler/DLL implementing receipts is present (no `LRS.Client.ServiceProxy.dll`, no `.aspx/.asp`), so its screens cannot be described from evidence.
  The settings suggest it signs in to LRS, listens to messaging and reads the repository (inferred: a cashier inbox of invoices).

### 7.2 Cashier integration as seen from the desktop and server

- lr_setting `config/IntegrationWithCashier=1`, `config/CashierServiceAddress=http://localhost/ILSCashSrv/ILSCash4LRS.asmx`
  (an external "ILS Cash" .asmx service the LRS server calls; error text "The connection to ILS Cashier server could not be established" in
  `lrscore-service_en_US.properties`). `config/DisableCashierIntakeForTransactions` (absent), `IntakeTransactions=INSC,APRF`.
- Server `CashierService` (`server-config.wsdd`, class `com.ils.lrs.webservice.cashier`, `$R/lrs-capture/wsdl/CashierService.wsdl`):
  `SubmitInvoice(transactionId, ResultCollection) -> id`, `CheckStatus(transactionId, ResultCollection) -> long`, `GetSchema(name) -> xml`,
  `CalculateFee(transaction_code, parameters) -> double`, `GetPayment(invoiceNumber) -> string`. The .NET proxy also declares
  `SearchCustomer`, `CreateCustomer` (not in the live WSDL).
- Desktop usage: `CalculateFee` (fee dialogs), `GetSchema("cashier.xsd")` (parameter definitions: `<transaction name><parameters><parameter name type value description mandatory position/>`;
  type codes 1 int(n0), 2/6 money(n2), 8 int range `[min,max]default`, dictionaries as `id;label;id*;label` with `*` default), `CheckInvoiceStatus(batch.id)`
  in `AssessWorker.InvoiceStatusRule` (status 2 or 5 = unpaid, not referenced in PLAGIS configs). `SubmitInvoice` is not called by the desktop.
- **In practice at PLAGIS the receipt flow is in the desktop** (inferred from configs and data): demand notice (INV number, `lr_payment_transaction` lines with
  `calculated_fee`, `demandnotice_number`) -> applicant pays at the bank -> "Register Fee Payment and Generate Receipt" stage: DemandNoticePage records
  `lr_payment` (receipt_no = bank teller number, bank_type, payment_date, paid_sum, internal_receipt_no = RCT number via `UpdateSimpleCounterValue:receipt_number`,
  `batch_id`) and marks fee lines paid -> FeePaymentVerificationReceipt report -> `CheckUnpaidFee` gates the next transition.
- Table links: `lr_payment.batch_id -> lr_batch.id`, `lr_payment.transaction_number`, `lr_payment_transaction.payment -> lr_payment.id`,
  `lr_payment_transaction.transaction_id -> lr_transaction.id`, `lr_batch.invoice_number` (cashier invoice), `lr_payment.invoice_number/receipt_number/is_paid`.

---

## 8. Hard-coded business rules, numbering, roles

Permissions (Alfresco/LRS permission names checked with `AccessControlService`): `lrsAssignTask`, `lrsCancelRegistration` (Withdraw),
`lrsContinueRegistration`, `lrsPostponeRegistration`, `lrsDiaryBook`, `lrsFeeConfiguration`, `lrsMapAccess`, `lrsAddScanned`,
`lrsIndexingScanned`, `lrsAddAnnotation`, `lrsCheckScanned`, `lrsMcJobClass` (string also used as a settings key).
Groups / authorities named in code: **"Postpone Clerk"** (only members can open a postponed task), **`ADMINISTRATOR`** authority (may take over a
lock owned by another user in Assign), Alfresco groups `GROUP_*` = workflow swimlanes; system groups excluded from pickers: ALFRESCO_ADMINISTRATORS,
EMAIL_CONTRIBUTORS, guest, webclient. Actual PLAGIS group names live in Alfresco and jBPM swimlanes (not in the client; see the workflow document).

Who can do what:
- Edit a task: task actor = user, or actor is a group the user belongs to (`IsSecurityValid`); selection must be one stage.
- Unlock: only the lock owner. Assign: lock owner or ADMINISTRATOR.
- Postpone/Continue/Withdraw: single selection, not locked by someone else, plus permission; Withdraw not allowed at `nonWithdrawableTasks`
  (e.g. RegisterCofO, DeliveryOfDocuments, ScanOutgoingDocuments for FR).

Statuses written by the client: `transaction_lodged`, `transaction_postponed`, `transaction_rejected`, `transaction_completed` (read), `transaction_withdrawn` (read),
`transaction_issued` (pending set); batch `application_lodged`, `application_postponed`, `application_issued` (default). Spatial unit status `archived` hides cadastre transitions.
Transaction source codes `normal` / `past`. Past transactions are always skipped when finding "the main transaction" (last non-past in the batch).

Numbering (`DomainModelService.GenerateNumber`, table `lr_simplecounter`, reservations in `lr_tempcounter`):

| Counter | Format (prefix + length) | Generated by |
|---|---|---|
| transaction_number | PLS + 9 (e.g. PLS000138850) | server default for Transaction |
| past_number | P-PLS + 9 | PastTransaction / RelatedTransaction pages |
| property_number | PL + 1 (min length; e.g. PL68317) | NotesPage; property file opening (inferred server) |
| suid_number | office prefix + 8 | PropertiesPagePresenter |
| document_number | office prefix + 7 | outgoing doc numbers `outd_<n>` |
| instrument_number / page_number / volume_number | INS / P / V; page resets every 500 per instrument, volume increments with page | RegisterInstrumentPage |
| plan_number | DP | RegisterPlanNumberPage |
| receipt_number | RCT + 9 | rule UpdateSimpleCounterValue |
| demandnotice_number | INV + 9 | CustomTransactionForm, rule |
| lodgment_number (reset yearly), deed_number, rid_number, state_act (PLTS), *_id counters | | server |

Hard-coded transaction-code logic: INSC & SROE auto applicant; INSC duplicate check (`config/AllowDuplicatedPendingINSC=0`); MORT/UPMO/RMOR mortgage source; UPMO consideration
addition; CHNT at GenerateCofO keeps read-only state after printing; DCT/GRR special withdraw; GRR has no target property; codes allowed with caveat
(`ValidTransactionCodesForCaveatTransaction`); multi-property codes (`TransactionCodeMultipleProperties`: INSC, CEMP, APCM, LOCE, MERT, MORT, RLEA, RMOR, RSUB, SUBD, UPMO, SEAR);
groups BLT, FRG, SSG, GRT, MCG, SVG. Transaction groups (File>New): BackLogs, First Registration, Ground Rent, Miscellaneous Registration, Subsequent Registration, Survey
(`Controls/WorkerAssess/GroupName`, `lr_transaction_metadata.group_name`). Workflow names: Indexing and Scanning, First Registration, Subsequent Registration, Recertification Registration,
Replacement Of Title, Application for Certificate of Occupancy, Search, Data Correction, Ground Rent Payment, Ground Rent Review, Set Right of Occupancy as Expired,
Survey Plan Checking, Survey Plan Checking and Approval, Raise Site Analysis Report, Raise Title Deed Plan.

PLAGIS transaction codes (`PLAGIS.DataModel/CustomTransactionTypeCode`): APRF, INSC, GRPM, SROE, APSL, APRO, CONT, RAOL, REGR, ABWG, APCA, APCM, ASSI, CEMP, CHNT, CHPC, COMP, DEVO,
EXTT, LEAS, LOCE, MERT, MORT, PSUR, RCOO, RLEA, RMOR, RSUB, SUBD, SUBL, RROC, SURO, UPMO, CETC, SEAR, REPT, APCO, PWOA, RPOA, DCT, GRR, SPC (labels and fee codes:
`lr_transaction_metadata`).

Other lr_setting keys read by the client (values at capture): config/SoapClientTimeout 350000 ms, ShowTransactionCodes 0, ShowDescriptionTooltip 1,
UseMasksInGUI, BilingualUI 0, Languages en-US, ReadOnlyStages, terminat_prompt, HidePastTransactionCode, businesscalendar/WorkingHoursPerDay 8, NonDurableStages,
drafter/CheckLegalSupDoc 1, workergui/* (GUI layout per form), webaccess/* (LWA). Defaults for new objects come from category `defaults`
(e.g. Address.country `ng`, region `plateau`, Parcel areaUnitType `sqm`, Transaction status `transaction_lodged`, source `normal`).

---

## 9. Screen inventory (build checklist)

| Screen | Area | Used by stages (examples) | Main service calls |
|---|---|---|---|
| Login | shell | start | Authentication.startSession, DomainModel (RegistryOffice) |
| Registration Queue + menus | shell | all | Repository.query (Lucene), Workflow.getTaskInstance/lockTask, Authoring.lock/unlock |
| Transaction Filter | shell | - | (client) |
| Notification window / Options | shell | - | Messaging.createSubscription |
| Details / History / Workflow diagram / View in LWA | shell | any | DomainModel, /lrs/downloadBPM, /lrswa |
| Change password | shell | - | Administration.changePassword |
| Fee structure / Fee rate config | admin | - | DomainModel save, Fee.calculateFee |
| Worker wizard frame | workflow | all | Reception.AcceptApplication, Workflow.startProcessInstance |
| Add Transactions (+ Transaction dialog) | reception | New, LodgmentApplication, IndexData, LodgeMemorandum | DomainModel (HQL checks), Workflow.AuthorizedProcessDefinitionList, Cashier.CreateCashEntity |
| Applicants (+ applicant dialog/search) | parties | New..ReviewSubmission (99 wizards) | DomainModel |
| Parties (+ party dialog, search, add owners) | parties | Lodgment, Review, GenerateCofO, RegisterCofO/Instrument (104) | DomainModel |
| Properties (+ property dialog, property info, search) | property | 196 wizards | DomainModel, GenerateNumber(suid_number) |
| Search Properties | search | SEAR/CETC New, GenerateSearchLetter | DomainModel |
| RRR page | rights | New, ReviewSubmission, GenerateCofO, RegisterCofO, GRR | DomainModel |
| Register Instrument | rights | RegisterCofO, RegisterInstrument | GenerateNumber(instrument/page/volume) |
| Register Plan Number | survey | RegisterSurveyPlan | GenerateNumber(plan_number) |
| Survey Plan | survey | SPC stages | DomainModel |
| Past transactions | backlog | INSC IndexData..BLFinalReview | GenerateNumber(past_number) |
| Data correction (transaction + RRR) | correction | DCT | DomainModel.Save (swap) |
| Ground Rent Review | ground rent | GRR | DomainModel |
| Outstanding ground rent | payments | INSC, New, PrepareRightOccupancy... | - |
| Fee list selection (+ fee calculation) | payments | New, OpenNewPropertyFile, GenerateFinalDemandNotice | Cashier.GetSchema/CalculateFee |
| Payment (demand notice / receipts) | payments | 168 wizards | DomainModel (Payment) |
| Transaction with documents (+ document dialog) | documents | 293 wizards | DomainModel |
| Scanning worker (scan/upload/view/annotate) | documents | any (View toolbar) | PUT /lrs/upload, Repository.update (CML), Content.read, Authoring.createVersion |
| Documents (print) / multi-property / search letter | printing | 65+21+1+6 wizards | Reports.getReportList/produceReport/getReportResourceUrl, Content |
| Complete Task (transitions + rules + assign) | workflow | every wizard (308) | Workflow.getTaskTransitionInfo/completeTask/addComment/assignTask, Integration.saveJob/getJob |
| Assign Task | workflow | queue | Workflow.assignTask |
| Rejection reasons / Postpone / Withdraw | workflow | PrepareRejectionLetter, queue menu | DomainModel.Save(Rejection), Workflow.setProcessVariables/suspend/resume, Reception.Withdrawal, Reports |
| Map (shell / property link) | map | Properties page, queue | GISViewer MapWorker.aspx, CadastralWebService.UpdateCadastralData |
| Batch content | misc | View toolbar | - |

---

## 10. Gaps / still to capture

1. **`bin/WorkflowConfig/SubsequentRegistration/*.config`** (26 files referenced by the exe.config) - captured as a 0-byte file. The combined
   `SubsequentRegistration.config` covers the codes but the per-file versions may differ; copy the folder from a workstation.
2. **Cashier web app code**: `Default.asp`, any `.aspx`, `App_Code`, `LRS.Client.ServiceProxy.dll`; and the ILS Cash service
   (`ILSCashSrv/ILSCash4LRS.asmx`) on the server; plus `cashier.xsd` content (call `CashierService.GetSchema` or copy from the server/repository).
3. **BIRT report templates** (`*.rptdesign`, `Border.png`, `CertificateWatermark.png`) - stored in the Alfresco repository / reports folder on .5; export them.
4. **Third-party assemblies** not captured: DevExpress, SandDock, LeadTools, WSE3 (only needed to run the old client), and any `LRS.Data.Controls.dll` if present on workstations.
5. `WorkstationHelp.htm` (help topics referenced by `_Toc` anchors).
6. **Screenshots** of each page type in the inventory (queue, add transaction, properties, parties, payment, documents print, complete task, scanning, map) - layout is generated, but captions/order on screen should be confirmed.
7. Alfresco group names and swimlane-to-group mapping, user `officeId`/`jobTitle`/`employeeNumber` properties (person nodes) - needed for the queue and report parameters.
8. Full `lr_dictionary` export exists; confirm the categories used by `lr_formfield` are all present (cross-check with the data agent's document).
9. `lr_process_history` / History form data source and `HistoryViewForm` columns - DDL exists, data not exported.
10. Registry-side `McJob` storage (`com.landsystems.lrs.model.McJob` -> `lr_mc_job`) behaviour of `IntegrationService.saveJob/getJob` on the server (Java, not decompiled).
11. Behaviour of server operations called here (`ReceptionService.AcceptApplication`, `Withdrawal`, `GenerateNumber`, `CashierService.*`) is only seen from the client side.

---

## Appendix A - lr_formfield: every form and field

Source: `$R/lrs-export/registry/data/lr_formfield.psv.gz` (1,725 rows). Rendered by `BindingFactory2`; field order = `Id` order.
Columns: attribute (object path), caption, type (xsd/lrs type, length), req (mandatory), dictionary (`lr_dictionary.category` or provider),
format, read-only for (object types; `all`), validation / condition (validation_rule; `if` = condition_rule; enabled_for).
Forms ending in `LWA` belong to Web Access; `Mc*` forms are used by the cadastre integration (inferred); the rest are desktop dialogs, pages and grids.


#### ApplicantCorporationForm  (class Corporation; 18 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| name | Organization Name | string(150) | Y |  |  |  |  |
| address.country | Country | Dictionary | Y | country |  |  |  |
| address.region | State | Dictionary | Y | region |  |  | enabled_for ng |
| address.localGovernmentArea | Local Government Area | Dictionary |  | region_lga |  |  |  |
| address.streetNumber | Street Number | string(50) |  |  |  |  |  |
| address.streetName | Street Name | string(100) |  |  |  |  |  |
| address.layoutName | Layout Name | Dictionary |  | layout_type |  |  |  |
| address.districtText | District | string |  |  |  |  |  |
| address.city | Town | string(50) |  |  |  |  |  |
| address.cell | Village | string(100) |  |  |  |  |  |
| address.ward | Ward | string(100) |  |  |  |  |  |
| phone1Number | Phone | string(20) |  |  |  |  |  |
| email | Email Address | string(50) |  |  | (regex: e-mail) |  |  |
| birthDate | Registration Date | date | Y |  | dd/MM/yyyy |  |  |
| gsmNumber | GSM Number | string(50) |  |  |  |  |  |
| tin | TIN | string(50) | Y |  |  |  |  |
| cacNumber | CAC Registration Number | string(50) | Y |  |  |  |  |
| party.comments | Comment | string(800) |  |  |  |  |  |

#### ApplicantCorporationStateForm  (class Corporation; 14 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| name | Organization Name | string(150) | Y |  |  |  |  |
| address.country | Country | Dictionary | Y | country |  |  |  |
| address.region | State | Dictionary | Y | region |  |  | enabled_for ng |
| address.localGovernmentArea | Local Government Area | Dictionary |  | region_lga |  |  |  |
| address.streetNumber | Street Number | string(50) |  |  |  |  |  |
| address.streetName | Street Name | string(100) |  |  |  |  |  |
| address.layoutName | Layout Name | Dictionary |  | layout_type |  |  |  |
| address.districtText | District | string |  |  |  |  |  |
| address.city | Town | string(50) |  |  |  |  |  |
| address.cell | Village | string(100) |  |  |  |  |  |
| address.ward | Ward | string(100) |  |  |  |  |  |
| phone1Number | Phone | string(20) |  |  |  |  |  |
| email | Email Address | string(50) |  |  | (regex: e-mail) |  |  |
| party.comments | Comment | string(800) |  |  |  |  |  |

#### ApplicantIndividualForm  (class Individual; 27 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| salutation | Salutation | Dictionary |  | salutation |  |  |  |
| firstName | First Name | string(150) | Y |  |  |  |  |
| secondName | Middle Name | string(128) |  |  |  |  |  |
| lastName | Last Name | string(150) | Y |  |  |  |  |
| gender | Gender | Dictionary |  | gender |  |  |  |
| address.originRegion | State of Origin | Dictionary |  | region |  |  |  |
| address.country | Country | Dictionary | Y | country |  |  |  |
| address.region | State | Dictionary | Y | region |  |  | enabled_for ng |
| address.localGovernmentArea | Local Government Area | Dictionary |  | region_lga |  |  |  |
| address.streetNumber | Street Number | string(50) |  |  |  |  |  |
| address.streetName | Street Name | string(100) |  |  |  |  |  |
| address.layoutName | Layout Name | Dictionary |  | layout_type |  |  |  |
| address.districtText | District | string |  |  |  |  |  |
| address.city | Town | string(50) |  |  |  |  |  |
| address.cell | Village | string(100) |  |  |  |  |  |
| address.ward | Ward | string(100) |  |  |  |  |  |
| occupation | Occupation | string(100) |  |  |  |  |  |
| phone1Number | Phone | string(20) |  |  |  |  |  |
| email | Email Address | string(50) |  |  | (regex: e-mail) |  |  |
| birthDate | Birth Date | date | Y |  | dd/MM/yyyy |  | OE 21 |
| payeNumber | PAYE Number | string(50) |  |  |  |  |  |
| tin | TIN | string(50) | Y |  |  |  |  |
| personIdType | ID Document | Dictionary |  | id_document_type |  |  | if !=incorp_reg_doc |
| personIdNumber | ID Document No. | string(64) |  |  |  |  |  |
| personIdDate | ID Document Date Issued | date |  |  | dd/MM/yyyy |  |  |
| personIdIssue | ID Document Issued By | string(150) |  |  |  |  |  |
| party.comments | Comment | string(800) |  |  |  |  |  |

#### ApplicantPartyForm  (class Applicant; 2 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| partyType | Capacity Type | Dictionary | Y | party_type |  |  | if !=party_scan |
| partyRoleType | Role | Dictionary | Y | applicant_role_type |  |  |  |

#### ApplicantRegistrationOfficeForm  (class Corporation; 3 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| name | Organization | string(50) | Y |  |  |  |  |
| address.streetName | Address | string(250) |  |  |  |  |  |
| phone1Number | Phone | string(20) |  |  |  |  |  |

#### ApplicantScanCorporationForm  (class Corporation; 3 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| name | Organization | string(50) | Y |  |  |  |  |
| address.streetName | Address | string(250) |  |  |  |  |  |
| phone1Number | Phone | string(20) |  |  |  |  |  |

#### AvailableFeeGrid  (class Dictionary; 1 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| label | Fee | string |  |  |  |  |  |

#### BuildingForm  (class Building; 3 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| complNumber | Building Number | string(30) | Y |  |  |  |  |
| numberOfFloors | Number of Floors | int(0) |  |  |  |  |  |
| label | Description | string(512) |  |  |  |  |  |

#### BuildingFormNonMandatory  (class Building; 3 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| complNumber | Building Number | string(30) |  |  |  |  |  |
| numberOfFloors | Number of Floors | int(0) |  |  |  |  |  |
| label | Description | string(512) |  |  |  |  |  |

#### CalculatedFeeGrid  (class PaymentTransaction; 4 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| paymentType | Fee Type | Dictionary |  | fee_type |  |  |  |
| calculatedFee | Fee Amount | double |  |  | c2 |  | GE 0 |
| paymentDate | Date of Calculation | date |  |  | dd/MM/yyyy |  |  |
| comments | Comment | string |  |  |  |  |  |

#### CalculatedGroundRentGrid  (class PaymentTransaction; 4 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| paymentType | Fee Type | Dictionary |  | payment_type |  | all |  |
| paidSum | Fee | double | Y |  | c2 |  |  |
| penalty | Penalty | double | Y |  | c2 |  |  |
| paidYear | Paid Year | integer | Y |  |  |  |  |

#### CalculatedPaymentGrid  (class PaymentTransaction; 4 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| paymentType | Fee Type | Dictionary |  | payment_type |  | all |  |
| paidSum | Fee | double | Y |  | c2 |  | GE 0 |
| paymentDate | Date of Calculation | date | Y |  | dd/MM/yyyy |  |  |
| comments | Comment | string |  |  |  |  |  |

#### CorporationEditableForm  (class Corporation; 17 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| name | Organization Name | string(150) | Y |  |  |  |  |
| address.country | Country | Dictionary | Y | country |  |  |  |
| address.region | State | Dictionary | Y | region |  |  | enabled_for ng |
| address.localGovernmentArea | Local Government Area | Dictionary |  | region_lga |  |  |  |
| address.streetNumber | Street Number | string(50) |  |  |  |  |  |
| address.streetName | Street Name | string(100) |  |  |  |  |  |
| address.layoutName | Layout Name | Dictionary |  | layout_type |  |  |  |
| address.districtText | District | string |  |  |  |  |  |
| address.city | Town | string(50) |  |  |  |  |  |
| address.cell | Village | string(100) |  |  |  |  |  |
| address.ward | Ward | string(100) |  |  |  |  |  |
| phone1Number | Phone | string(20) |  |  |  | all |  |
| email | E-mail | string(50) |  |  | (regex: e-mail) | all |  |
| birthDate | Registration Date | date | Y |  | dd/MM/yyyy | all |  |
| gsmNumber | GSM Number | string(50) |  |  |  | all |  |
| tin | TIN | string(50) |  |  |  | all |  |
| cacNumber | CAC Registration Number | string(50) | Y |  |  | all |  |

#### CorporationForm  (class Corporation; 17 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| name | Organization Name | string(150) | Y |  |  |  |  |
| address.country | Country | Dictionary | Y | country |  |  |  |
| address.region | State | Dictionary | Y | region |  |  | enabled_for ng |
| address.localGovernmentArea | Local Government Area | Dictionary |  | region_lga |  |  |  |
| address.streetNumber | Street Number | string(50) |  |  |  |  |  |
| address.streetName | Street Name | string(100) |  |  |  |  |  |
| address.layoutName | Layout Name | Dictionary |  | layout_type |  |  |  |
| address.districtText | District | string |  |  |  |  |  |
| address.city | Town | string(50) |  |  |  |  |  |
| address.cell | Village | string(100) |  |  |  |  |  |
| address.ward | Ward | string(100) |  |  |  |  |  |
| phone1Number | Phone | string(20) |  |  |  |  |  |
| email | E-mail | string(50) |  |  | (regex: e-mail) |  |  |
| birthDate | Registration Date | date | Y |  | dd/MM/yyyy |  |  |
| gsmNumber | GSM Number | string(50) |  |  |  |  |  |
| tin | TIN | string(50) | Y |  |  |  |  |
| cacNumber | CAC Registration Number | string(50) | Y |  |  |  |  |

#### CorporationMunicipalityForm  (class Corporation; 6 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| address.country | Country | Dictionary | Y | country |  | all |  |
| name | Name | string(150) | Y |  |  |  |  |
| address.streetName | Address | string(100) |  |  |  |  |  |
| address.region | Province | Dictionary | Y | region |  |  | enabled_for ng |
| phone1Number | Phone | string(20) |  |  |  |  |  |
| email | E-mail | string(50) |  |  | (regex: e-mail) |  |  |

#### CorporationStateEditableForm  (class Corporation; 13 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| name | Organization Name | string(150) | Y |  |  |  |  |
| address.country | Country | Dictionary | Y | country |  |  |  |
| address.region | State | Dictionary | Y | region |  |  | enabled_for ng |
| address.localGovernmentArea | Local Government Area | Dictionary |  | region_lga |  |  |  |
| address.streetNumber | Street Number | string(50) |  |  |  |  |  |
| address.streetName | Street Name | string(100) |  |  |  |  |  |
| address.layoutName | Layout Name | Dictionary |  | layout_type |  |  |  |
| address.districtText | District | string |  |  |  |  |  |
| address.city | Town | string(50) |  |  |  |  |  |
| address.cell | Village | string(100) |  |  |  |  |  |
| address.ward | Ward | string(100) |  |  |  |  |  |
| phone1Number | Phone | string(20) |  |  |  | all |  |
| email | Email Address | string(50) |  |  | (regex: e-mail) | all |  |

#### CorporationStateForeignForm  (class Corporation; 3 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| address.country | Country | Dictionary | Y | country |  |  |  |
| address.city | City/Town | string(30) | Y |  |  |  |  |
| address.streetName | Address | string(100) |  |  |  |  |  |

#### CorporationStateForm  (class Corporation; 13 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| name | Organization Name | string(150) | Y |  |  |  |  |
| address.country | Country | Dictionary | Y | country |  |  |  |
| address.region | State | Dictionary | Y | region |  |  | enabled_for ng |
| address.localGovernmentArea | Local Government Area | Dictionary |  | region_lga |  |  |  |
| address.streetNumber | Street Number | string(50) |  |  |  |  |  |
| address.streetName | Street Name | string(100) |  |  |  |  |  |
| address.layoutName | Layout Name | Dictionary |  | layout_type |  |  |  |
| address.districtText | District | string |  |  |  |  |  |
| address.city | Town | string(50) |  |  |  |  |  |
| address.cell | Village | string(100) |  |  |  |  |  |
| address.ward | Ward | string(100) |  |  |  |  |  |
| phone1Number | Phone | string(20) |  |  |  |  |  |
| email | Email Address | string(50) |  |  | (regex: e-mail) |  |  |

#### DeedForm  (class Transaction; 6 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| sourcePropertyIDs | System Property No. | string(250) |  |  |  | all |  |
| generalNumber | Deed No. | string(50) | Y |  | [0-9]+ |  |  |
| serialNumber | Serial No. | string(5) | Y |  | [0-9]+ |  |  |
| volume | Book No. | string(3) | Y |  |  |  |  |
| folio | Page No. | long | Y |  | [0-9]{1,3} |  | GT 0&LE 999 |
| comments | Comments | string(800) |  |  |  |  |  |

#### GroundRentGridLWA  (class GroundRent; 4 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| paidYear | Ground Rent Year | string |  |  |  |  |  |
| amount | Rental Amount | double |  |  | n2 |  |  |
| paidSum | Amount Paid | double |  |  | n2 |  |  |
| outstandingAmount | Amount Outstanding | double |  |  | n2 |  |  |

#### GroundRentReviewAttributeForm  (class RentalAmountRevision; 2 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| effectiveDate | Effective Date | date |  |  | dd/MM/yyyy | all |  |
| annualRentalAmount | Annual Rental Amount | double | Y |  | c2 |  | GE 0 |

#### HistoryViewForm  (class LrQueue; 6 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| taskCreatedDate | Started | dateTime |  |  | dd/MM/yyyy hh:mm tt |  |  |
| taskEndDate | Finished | dateTime |  |  | dd/MM/yyyy hh:mm tt |  |  |
| taskDuration | Duration | long |  |  |  |  |  |
| taskDescription | Task | string |  |  |  |  |  |
| transitionName | Transition | string |  |  |  |  |  |
| taskUserName | User | Authority |  | authority |  |  |  |

#### HistoryViewFormEvent  (class LrQueue; 4 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| {http://www.landsystems.com/sdms/model/workflow/1.0}commentText | Description | string |  |  |  |  |  |
| {http://www.landsystems.com/sdms/model/workflow/1.0}commentAcs | Type | Dictionary |  | task_event_access |  |  |  |
| {http://www.alfresco.org/model/content/1.0}creator | User | Authority |  | authority |  |  |  |
| {http://www.alfresco.org/model/content/1.0}created | Date | dateTime |  |  | dd/MM/yyyy hh:mm tt |  |  |

#### IndividualEditableForm  (class Individual; 26 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| salutation | Salutation | Dictionary |  | salutation |  |  |  |
| firstName | First Name | string(150) | Y |  |  |  |  |
| secondName | Middle Name | string(150) |  |  |  |  |  |
| lastName | Last Name | string(150) | Y |  |  |  |  |
| gender | Gender | Dictionary |  | gender |  | all |  |
| address.originRegion | State of Origin | Dictionary |  | region |  | all |  |
| address.country | Country | Dictionary | Y | country |  |  |  |
| address.region | State | Dictionary | Y | region |  |  | enabled_for ng |
| address.localGovernmentArea | Local Government Area | Dictionary |  | region_lga |  |  |  |
| address.streetNumber | Street Number | string(50) |  |  |  |  |  |
| address.streetName | Street Name | string(100) |  |  |  |  |  |
| address.layoutName | Layout Name | Dictionary |  | layout_type |  |  |  |
| address.districtText | District | string |  |  |  |  |  |
| address.city | Town | string(50) |  |  |  |  |  |
| address.cell | Village | string(100) |  |  |  |  |  |
| address.ward | Ward | string(100) |  |  |  |  |  |
| occupation | Occupation | string(100) |  |  |  |  |  |
| phone1Number | Phone | string(20) |  |  |  | all |  |
| email | Email Address | string(50) |  |  | (regex: e-mail) | all |  |
| birthDate | Birth Date | date |  |  | dd/MM/yyyy | all | OE 21 |
| payeNumber | PAYE Number | string(50) |  |  |  | all |  |
| tin | TIN | string(50) |  |  |  | all |  |
| personIdType | ID Document | Dictionary |  | id_document_type |  | all | if !=incorp_reg_doc |
| personIdNumber | ID Document No. | string(64) |  |  |  | all |  |
| personIdDate | ID Document Date Issued | date |  |  | dd/MM/yyyy | all |  |
| personIdIssue | ID Document Issued By | string(100) |  |  |  | all |  |

#### IndividualForm  (class Individual; 26 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| salutation | Salutation | Dictionary |  | salutation |  |  |  |
| firstName | First Name | string(150) | Y |  |  |  |  |
| secondName | Middle Name | string(150) |  |  |  |  |  |
| lastName | Last Name | string(150) | Y |  |  |  |  |
| gender | Gender | Dictionary |  | gender |  |  |  |
| address.originRegion | State of Origin | Dictionary |  | region |  |  |  |
| address.country | Country | Dictionary | Y | country |  |  |  |
| address.region | State | Dictionary | Y | region |  |  | enabled_for ng |
| address.localGovernmentArea | Local Government Area | Dictionary |  | region_lga |  |  |  |
| address.streetNumber | Street Number | string(50) |  |  |  |  |  |
| address.streetName | Street Name | string(100) |  |  |  |  |  |
| address.layoutName | Layout Name | Dictionary |  | layout_type |  |  |  |
| address.districtText | District | string |  |  |  |  |  |
| address.city | Town | string(50) |  |  |  |  |  |
| address.cell | Village | string(100) |  |  |  |  |  |
| address.ward | Ward | string(100) |  |  |  |  |  |
| occupation | Occupation | string(100) |  |  |  |  |  |
| phone1Number | Phone | string(20) |  |  |  |  |  |
| email | Email Address | string(50) |  |  | (regex: e-mail) |  |  |
| birthDate | Birth Date | date |  |  | dd/MM/yyyy |  | OE 21 |
| payeNumber | PAYE Number | string(50) |  |  |  |  |  |
| tin | TIN | string(50) | Y |  |  |  |  |
| personIdType | ID Document | Dictionary |  | id_document_type |  |  | if !=incorp_reg_doc |
| personIdNumber | ID Document No. | string(64) |  |  |  |  |  |
| personIdDate | ID Document Date Issued | date |  |  | dd/MM/yyyy |  |  |
| personIdIssue | ID Document Issued By | string(100) |  |  |  |  |  |

#### InstrumCertificateFormAPRF  (class Transaction; 12 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| sourcePropertyIDs | File No. | string(250) |  |  |  | all |  |
| tenureType | Tenure Type | Dictionary(250) |  | tenure_type |  | all |  |
| transactionNumber | Transaction No. | string(50) | Y |  |  | all |  |
| transactionType | Transaction | TransactionMetaData | Y | transaction_type |  | all |  |
| createDate | Submission Date | dateTime |  |  | dd/MM/yyyy hh:mm tt | all | LE today |
| oldPropertyId | Old File No. | string(50) |  |  |  | all |  |
| certNos | Certificate No. | string(255) |  |  |  | all |  |
| leaseExtension | Lease Extension | Dictionary |  | lease_extension |  |  |  |
| firstpayDate | Date of First GR Payment | date | Y |  | dd/MM/yyyy |  | LE today |
| payPeriodYear | GR Revision Period | integer | Y |  | \d{1,2} |  | GE 0 |
| balanceDate | Last GR Payment Date | date | Y |  | dd/MM/yyyy |  | LE today |
| comments | Comments | string(800) |  |  |  |  |  |

#### InstrumFormReg  (class Transaction; 13 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| sourcePropertyIDs | File No. | string(250) |  |  |  | all |  |
| oldPropertyId | Old File No. | string(50) |  |  |  | all |  |
| transactionNumber | Transaction No. | string(50) |  |  |  | all |  |
| transactionType | Transaction | TransactionMetaData |  | transaction_type |  | all |  |
| createDate | Submission Date | dateTime |  |  | dd/MM/yyyy hh:mm tt | all | LE today |
| tenureType | Tenure Type | Dictionary(250) |  | tenure_type |  | all |  |
| balanceDate | Last GR Payment Date | date |  |  | dd/MM/yyyy | all | LE today |
| specialCondition | Special Conditions | string(50) |  |  |  | all |  |
| signDate | Execution Date | dateTime | Y |  | dd/MM/yyyy |  | LE today |
| registrationDate | Registration Date | dateTime | Y |  | dd/MM/yyyy hh:mm tt |  | LE today |
| volume | Volume | string(20) | Y |  |  |  |  |
| folio | Page Number | string(20) | Y |  |  |  |  |
| comments | Comments | string(800) |  |  |  |  |  |

#### InstrumFullFormAPRF  (class Transaction; 11 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| sourcePropertyIDs | File No. | string(250) |  |  |  | all |  |
| oldPropertyId | Old File No. | string(50) |  |  |  | all |  |
| transactionNumber | Transaction No. | string(50) | Y |  |  | all |  |
| transactionType | Transaction | TransactionMetaData | Y | transaction_type |  | all |  |
| tenureType | Tenure Type | Dictionary(250) |  | tenure_type |  | all |  |
| createDate | Submission Date | dateTime |  |  | dd/MM/yyyy hh:mm tt | all | LE today |
| leaseExtension | Lease Extension | Dictionary |  | lease_extension |  |  |  |
| firstpayDate | Date of First GR Payment | date | Y |  | dd/MM/yyyy |  | LE today |
| payPeriodYear | GR Revision Period | integer | Y |  | \d{1,2} |  | GE 0 |
| balanceDate | Last GR Payment Date | date | Y |  | dd/MM/yyyy |  | LE today |
| comments | Comments | string(800) |  |  |  |  |  |

#### InstrumFullFormAPRF_Publ  (class Transaction; 12 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| sourcePropertyIDs | File No. | string(250) |  |  |  | all |  |
| oldPropertyId | Old File No. | string(50) |  |  |  | all |  |
| transactionNumber | Transaction No. | string(50) |  |  |  | all |  |
| transactionType | Transaction | TransactionMetaData |  | transaction_type |  | all |  |
| createDate | Submission Date | dateTime |  |  | dd/MM/yyyy hh:mm tt | all | LE today |
| tenureType | Tenure Type | Dictionary(250) |  | tenure_type |  | all |  |
| leaseExtension | Lease Extension | Dictionary |  | lease_extension |  |  |  |
| firstpayDate | Date of First GR Payment | date | Y |  | dd/MM/yyyy |  | LE today |
| payPeriodYear | GR Revision Period | integer | Y |  | \d{1,2} |  | GE 0 |
| balanceDate | Last GR Payment Date | date | Y |  | dd/MM/yyyy |  | LE today |
| specialCondition | Special Conditions | string(50) |  |  |  |  |  |
| comments | Comments | string(800) |  |  |  |  |  |

#### McDocumentForm  (class SourceDocument; 7 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| d.Id | Id | long |  |  |  |  |  |
| d.DocumentType.Label | Document Type | string |  |  |  |  |  |
| d.DocumentNumber | Document No. | string(50) |  |  |  |  |  |
| d.PageCount | Pages/Files | int |  |  |  |  |  |
| d.CreateDate | Create Date | date |  |  | d |  |  |
| d.ModifiedAt | Last Edited | date |  |  | d |  |  |
| d.Comments | Comments | string(800) |  |  |  |  |  |

#### McParcelForm  (class SpatialUnit; 20 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| s.Id | Id | long |  |  |  |  |  |
| s.SuID | Spatial Unit ID | string |  |  |  |  |  |
| s.County.Label | County | string |  |  |  |  |  |
| s.County.ExtCode | County Id. | string |  |  |  |  |  |
| s.TownshipFull | Township | string |  |  |  |  |  |
| s.Township | Township Id. | string |  |  |  |  |  |
| s.RangeFull | Range | string |  |  |  |  |  |
| s.Range | Range Id. | string |  |  |  |  |  |
| s.Section | Section | string |  |  |  |  |  |
| s.FortyNumber.Label | Forty | string |  |  |  |  |  |
| s.FortyNumber.Descr | Forty Id. | string |  |  |  |  |  |
| s.LotNumber | Lot | string |  |  |  |  |  |
| s.LandType.Label | Land Type | string |  |  |  |  |  |
| s.LandType.Descr | Land Type Code | string |  |  |  |  |  |
| s.RangeDirection.Descr | Range Code | string |  |  |  |  |  |
| s.Area | Area | double |  |  | n4 |  |  |
| s.Address.AddressAreaName | Address Line 1 | string(250) |  |  |  |  |  |
| s.Address.StreetName | Address Line 2 | string(100) |  |  |  |  |  |
| s.Address.City | City | string(100) |  |  |  |  |  |
| s.Address.PostalCode | Post Code | string(10) |  |  |  |  |  |

#### McPartyForm  (class Party, Person; 6 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| p.Id | Person Id | string |  |  |  |  |  |
| pp.PartyRoleType.Label | Legal Role | string |  |  |  |  |  |
| pp.PartyType.Label | Person Type | string |  |  |  |  |  |
| pp.Name | Name | string(250) |  |  |  |  |  |
| pp.Person.Email | Email | string(64) |  |  |  |  |  |
| pp.Person.Phone1Number | Phone | string(20) |  |  |  |  |  |

#### McTransactionForm  (class Transaction; 6 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| t.Id | Transaction Id | long |  |  |  |  |  |
| task.id | Task Id | long |  |  |  |  |  |
| t.TransactionNumber | Transaction No. | string |  |  |  |  |  |
| t.TransactionType.Label | Transaction | string |  |  |  |  |  |
| ph.TaskDescription | Task | string |  |  |  |  |  |
| ph.TaskCreate | Created | date |  |  |  |  |  |

#### MultiApplicantForm  (class Applicant; 2 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| partyType | Party Type | Dictionary | Y | party_type |  |  | if !=party_scan,!=representative,!=party_mlstp |
| partyRoleType | Role | Dictionary | Y | applicant_role_type |  |  |  |

#### OutstandingForm  (class PaymentTransaction; 4 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| paidYear | Ground Rent Year | integer | Y |  |  |  |  |
| manualFee | Rental Amount | double | Y |  | c2 |  |  |
| paidSum | Amount Paid | double | Y |  | c2 |  |  |
| calculatedFee | Amount Outstanding | double | Y |  | c2 | all |  |

#### OutstandingGrid  (class PaymentTransaction; 4 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| paidYear | Ground Rent Year | integer | Y |  |  |  |  |
| manualFee | Rental Amount | double | Y |  | c2 |  |  |
| paidSum | Amount Paid | double | Y |  | c2 |  |  |
| calculatedFee | Amount Outstanding | double | Y |  | c2 | all |  |

#### OutstandingGridMultiProp  (class PaymentTransaction; 5 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| paidYear | Ground Rent Year | integer | Y |  |  |  |  |
| manualFee | Rental Amount | double | Y |  | c2 |  |  |
| paidSum | Amount Paid | double | Y |  | c2 |  |  |
| calculatedFee | Amount Outstanding | double | Y |  | c2 | all |  |
| propertyNumber | Property Number | string | Y |  |  |  |  |

#### PartiesGrid  (class Party; 7 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| partyType | Type | Dictionary |  | party_type |  |  | if !=representative |
| partyRoleType | Role | Dictionary |  | party_role_type |  |  |  |
| name | Name | string(250) |  |  |  |  |  |
| person.address.region | State | Dictionary | Y | region |  |  |  |
| person.address.localGovernmentArea | Local Government Area | Dictionary | Y | region_lga |  |  |  |
| person.address.originRegion | State of Origin | Dictionary | Y | region |  |  |  |
| person.address.streetName | Street Name | string(100) |  |  |  |  |  |

#### PartiesRrrPageGrid  (class Party; 8 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| IsSelected |   | boolean(100) |  |  |  |  |  |
| partyType | Type | Dictionary |  | party_type |  |  | if !=representative |
| partyRoleType | Role | Dictionary |  | party_role_type |  |  |  |
| name | Name | string(250) |  |  |  |  |  |
| person.address.region | State | Dictionary | Y | region |  |  |  |
| person.address.localGovernmentArea | Local Government Area | Dictionary | Y | region_lga |  |  |  |
| person.address.originRegion | State of Origin | Dictionary | Y | region |  |  |  |
| person.address.streetName | Street Name | string(100) |  |  |  |  |  |

#### PartiesSearchForm  (class Party; 9 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| partyType | Party Type | Dictionary |  | party_type |  |  | if !=party_scan,!=representative,!=party_mlstp |
| name | Name | string(250) |  |  |  |  |  |
| person.personIdType | ID Document | Dictionary |  | id_document_type |  |  |  |
| person.personIdNumber | ID Document No. | string(64) |  |  |  |  |  |
| person.address.country | Country | Dictionary |  | country |  |  |  |
| person.address.region | State | Dictionary |  | region |  |  |  |
| person.address.localGovernmentArea | Local Government Area | Dictionary |  | region_lga |  |  |  |
| person.address.cell | Village | string(100) |  |  |  |  |  |
| person.address.streetName | Street Name | string(100) |  |  |  |  |  |

#### PartiesSearchGrid  (class Party; 7 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| partyType | Party Type | Dictionary |  | party_type |  |  | if !=representative |
| name | Name | string(250) |  |  |  |  |  |
| person.personIdType | ID Document | Dictionary |  | id_document_type |  |  |  |
| person.personIdNumber | ID Document No. | string(64) |  |  |  |  |  |
| person.address.country | Country | Dictionary | Y | country |  |  |  |
| person.address.region | State | Dictionary | Y | region |  |  |  |
| person.address.localGovernmentArea | Local Government Area | Dictionary | Y | region_lga |  |  |  |

#### PartyCommentForm  (class Party; 1 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| comments | Comment | string(250) |  |  |  |  |  |

#### PartyCommentROForm  (class Party; 1 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| comments | Comment | string(250) |  |  |  | all |  |

#### PartyContactEditableForm  (class PartyContact; 27 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| contactType | Party Type | Dictionary |  | party_type |  | all | if !=natural,!=juridical,!=party_state,!=party_scan,!=party_mlstp |
| salutation | Salutation | Dictionary |  | salutation |  |  |  |
| firstName | First Name | string(150) | Y |  |  |  |  |
| secondName | Middle Name | string(150) |  |  |  |  |  |
| lastName | Last Name | string(150) | Y |  |  |  |  |
| gender | Gender | Dictionary |  | gender |  | all |  |
| address.originRegion | State of Origin | Dictionary |  | region |  | all |  |
| address.country | Country | Dictionary | Y | country |  |  |  |
| address.region | State | Dictionary | Y | region |  |  |  |
| address.localGovernmentArea | Local Government Area | Dictionary |  | region_lga |  |  |  |
| address.streetNumber | Street Number | string(50) |  |  |  |  |  |
| address.streetName | Street Name | string(100) |  |  |  |  |  |
| address.layoutName | Layout Name | Dictionary |  | layout_type |  |  |  |
| address.districtText | District | string |  |  |  |  |  |
| address.city | Town | string(50) |  |  |  |  |  |
| address.cell | Village | string(100) |  |  |  |  |  |
| address.ward | Ward | string(100) |  |  |  |  |  |
| occupation | Occupation | string(100) |  |  |  |  |  |
| phone1Number | Phone | string(20) |  |  |  | all |  |
| email | Email Address | string(50) |  |  | (regex: e-mail) | all |  |
| birthDate | Birth Date | date | Y |  | dd/MM/yyyy | all | OE 21 |
| payeNumber | PAYE Number | string(50) |  |  |  | all |  |
| tin | TIN | string(50) |  |  |  | all |  |
| personIdType | ID Document | Dictionary |  | id_document_type |  | all | if !=incorp_reg_doc |
| personIdNumber | ID Document No. | string(64) |  |  |  | all |  |
| personIdDate | ID Document Date Issued | date |  |  | dd/MM/yyyy | all |  |
| personIdIssue | ID Document Issued By | string(100) |  |  |  | all |  |

#### PartyContactForm  (class PartyContact; 27 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| contactType | Party Type | Dictionary |  | party_type |  | all | if !=natural,!=juridical,!=party_state,!=party_scan,!=party_mlstp |
| salutation | Salutation | Dictionary |  | salutation |  |  |  |
| firstName | First Name | string(150) | Y |  |  |  |  |
| secondName | Middle Name | string(150) |  |  |  |  |  |
| lastName | Last Name | string(150) | Y |  |  |  |  |
| gender | Gender | Dictionary |  | gender |  |  |  |
| address.originRegion | State of Origin | Dictionary |  | region |  |  |  |
| address.country | Country | Dictionary | Y | country |  |  |  |
| address.region | State | Dictionary | Y | region |  |  | enabled_for ng |
| address.localGovernmentArea | Local Government Area | Dictionary |  | region_lga |  |  |  |
| address.streetNumber | Street Number | string(50) |  |  |  |  |  |
| address.streetName | Street Name | string(100) |  |  |  |  |  |
| address.layoutName | Layout Name | Dictionary |  | layout_type |  |  |  |
| address.districtText | District | string |  |  |  |  |  |
| address.city | Town | string(50) |  |  |  |  |  |
| address.cell | Village | string(100) |  |  |  |  |  |
| address.ward | Ward | string(100) |  |  |  |  |  |
| occupation | Occupation | string(100) |  |  |  |  |  |
| phone1Number | Phone | string(20) |  |  |  |  |  |
| email | Email Address | string(50) |  |  | (regex: e-mail) |  |  |
| birthDate | Birth Date | date | Y |  | dd/MM/yyyy |  | OE 21 |
| payeNumber | PAYE Number | string(50) |  |  |  |  |  |
| tin | TIN | string(50) | Y |  |  |  |  |
| personIdType | ID Document | Dictionary |  | id_document_type |  |  | if !=incorp_reg_doc |
| personIdNumber | ID Document No. | string(64) |  |  |  |  |  |
| personIdDate | ID Document Date Issued | date |  |  | dd/MM/yyyy |  |  |
| personIdIssue | ID Document Issued By | string(100) |  |  |  |  |  |

#### PartyContactsGrid  (class PartyContact; 7 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| contactType | Type | Dictionary |  | party_type |  |  |  |
| firstName | First Name | string(150) |  |  |  |  |  |
| lastName | Last Name | string(150) |  |  |  |  |  |
| address.region | State | Dictionary | Y | region |  |  |  |
| address.localGovernmentArea | Local Government Area | Dictionary | Y | region_lga |  |  |  |
| address.originRegion | State of Origin | Dictionary | Y | region |  |  |  |
| address.streetName | Street Name | string(100) |  |  |  |  |  |

#### PartyContactsSearchForm  (class Individual; 9 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| firstName | First Name | string(250) |  |  |  |  |  |
| lastName | Last Name | string(250) |  |  |  |  |  |
| personIdType | ID Document | Dictionary |  | id_document_type |  |  |  |
| personIdNumber | ID Document No. | string(64) |  |  |  |  |  |
| address.country | Country | Dictionary |  | country |  |  |  |
| address.region | State | Dictionary |  | region |  |  |  |
| address.localGovernmentArea | Local Government Area | Dictionary |  | region_lga |  |  |  |
| address.streetName | Street Name | string(100) |  |  |  |  |  |
| address.cell | Village | string(100) |  |  |  |  |  |

#### PartyContactsSearchGrid  (class Individual; 7 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| firstName | First Name | string(250) |  |  |  |  |  |
| lastName | Last Name | string(250) |  |  |  |  |  |
| personIdType | ID Document | Dictionary |  | id_document_type |  |  |  |
| personIdNumber | ID Document No. | string(64) |  |  |  |  |  |
| address.country | Country | Dictionary | Y | country |  |  |  |
| address.region | State | Dictionary | Y | region |  |  |  |
| address.localGovernmentArea | Local Government Area | Dictionary | Y | region_lga |  |  |  |

#### PartyForm  (class Party; 2 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| partyType | Party Type | Dictionary | Y | party_type |  |  | if !=party_scan,!=representative,!=party_mlstp |
| partyRoleType | Legal Role | Dictionary | Y | party_role_type |  |  |  |

#### PartyROForm  (class Party; 2 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| partyType | Party Type | Dictionary | Y | party_type |  | all | if !=party_scan,!=representative,!=party_mlstp |
| partyRoleType | Legal Role | Dictionary | Y | party_role_type |  | all |  |

#### PartyShortForm  (class Party; 2 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| partyType | Party Type | Dictionary | Y | party_type |  |  | if !=party_scan,!=representative,!=party_mlstp |
| partyRoleType | Role | Dictionary | Y | party_role_type |  |  |  |

#### PastCorporationForm  (class Corporation; 17 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| name | Organization Name | string(150) | Y |  |  |  |  |
| address.country | Country | Dictionary | Y | country |  |  |  |
| address.region | State | Dictionary | Y | region |  |  | enabled_for ng |
| address.localGovernmentArea | Local Government Area | Dictionary |  | region_lga |  |  |  |
| address.streetNumber | Street Number | string(50) |  |  |  |  |  |
| address.streetName | Street Name | string(100) |  |  |  |  |  |
| address.layoutName | Layout Name | Dictionary |  | layout_type |  |  |  |
| address.districtText | District | string |  |  |  |  |  |
| address.city | Town | string(50) |  |  |  |  |  |
| address.cell | Village | string(100) |  |  |  |  |  |
| address.ward | Ward | string(100) |  |  |  |  |  |
| phone1Number | Phone | string(20) |  |  |  |  |  |
| email | E-mail | string(50) |  |  | (regex: e-mail) |  |  |
| birthDate | Registration Date | date | Y |  | dd/MM/yyyy |  |  |
| gsmNumber | GSM Number | string(50) |  |  |  |  |  |
| tin | TIN | string(50) |  |  |  |  |  |
| cacNumber | CAC Registration Number | string(50) | Y |  |  |  |  |

#### PastIndividualForm  (class Individual; 26 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| salutation | Salutation | Dictionary |  | salutation |  |  |  |
| firstName | First Name | string(150) | Y |  |  |  |  |
| secondName | Middle Name | string(150) |  |  |  |  |  |
| lastName | Last Name | string(150) | Y |  |  |  |  |
| gender | Gender | Dictionary |  | gender |  |  |  |
| address.originRegion | State of Origin | Dictionary |  | region |  |  |  |
| address.country | Country | Dictionary | Y | country |  |  |  |
| address.region | State | Dictionary | Y | region |  |  | enabled_for ng |
| address.localGovernmentArea | Local Government Area | Dictionary |  | region_lga |  |  |  |
| address.streetNumber | Street Number | string(50) |  |  |  |  |  |
| address.streetName | Street Name | string(100) |  |  |  |  |  |
| address.layoutName | Layout Name | Dictionary |  | layout_type |  |  |  |
| address.districtText | District | string |  |  |  |  |  |
| address.city | Town | string(50) |  |  |  |  |  |
| address.cell | Village | string(100) |  |  |  |  |  |
| address.ward | Ward | string(100) |  |  |  |  |  |
| occupation | Occupation | string(100) |  |  |  |  |  |
| phone1Number | Phone | string(20) |  |  |  |  |  |
| email | Email Address | string(50) |  |  | (regex: e-mail) |  |  |
| birthDate | Birth Date | date |  |  | dd/MM/yyyy |  | OE 21 |
| payeNumber | PAYE Number | string(50) |  |  |  |  |  |
| tin | TIN | string(50) |  |  |  |  |  |
| personIdType | ID Document | Dictionary |  | id_document_type |  |  | if !=incorp_reg_doc |
| personIdNumber | ID Document No. | string(64) |  |  |  |  |  |
| personIdDate | ID Document Date Issued | date |  |  | dd/MM/yyyy |  |  |
| personIdIssue | ID Document Issued By | string(100) |  |  |  |  |  |

#### PastInstrumGrid  (class Transaction; 6 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| transactionType | Transaction | TransactionMetaData |  | transaction_type |  |  |  |
| transactionNumber | Transaction No. | string |  |  |  |  |  |
| sourcePropertyIDs | System Property No. | string(250) |  |  |  | all |  |
| registrationDate | Registration Date | date | Y |  | dd/MM/yyyy hh:mm tt |  | LE today |
| recordingno | Instrument No. | string(30) |  |  |  |  |  |
| transactionStatus | Transaction Status | Dictionary |  | transaction_status |  |  |  |

#### PastPartyContactForm  (class PartyContact; 27 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| contactType | Party Type | Dictionary |  | party_type |  | all | if !=natural,!=juridical,!=party_state,!=party_scan,!=party_mlstp |
| salutation | Salutation | Dictionary |  | salutation |  |  |  |
| firstName | First Name | string(150) | Y |  |  |  |  |
| secondName | Middle Name | string(150) |  |  |  |  |  |
| lastName | Last Name | string(150) | Y |  |  |  |  |
| gender | Gender | Dictionary |  | gender |  |  |  |
| address.originRegion | State of Origin | Dictionary |  | region |  |  |  |
| address.country | Country | Dictionary | Y | country |  |  |  |
| address.region | State | Dictionary | Y | region |  |  | enabled_for ng |
| address.localGovernmentArea | Local Government Area | Dictionary |  | region_lga |  |  |  |
| address.streetNumber | Street Number | string(50) |  |  |  |  |  |
| address.streetName | Street Name | string(100) |  |  |  |  |  |
| address.layoutName | Layout Name | Dictionary |  | layout_type |  |  |  |
| address.districtText | District | string |  |  |  |  |  |
| address.city | Town | string(50) |  |  |  |  |  |
| address.cell | Village | string(100) |  |  |  |  |  |
| address.ward | Ward | string(100) |  |  |  |  |  |
| occupation | Occupation | string(100) |  |  |  |  |  |
| phone1Number | Phone | string(20) |  |  |  |  |  |
| email | Email Address | string(50) |  |  | (regex: e-mail) |  |  |
| birthDate | Birth Date | date |  |  | dd/MM/yyyy |  | OE 21 |
| payeNumber | PAYE Number | string(50) |  |  |  |  |  |
| tin | TIN | string(50) |  |  |  |  |  |
| personIdType | ID Document | Dictionary |  | id_document_type |  |  | if !=incorp_reg_doc |
| personIdNumber | ID Document No. | string(64) |  |  |  |  |  |
| personIdDate | ID Document Date Issued | date |  |  | dd/MM/yyyy |  |  |
| personIdIssue | ID Document Issued By | string(100) |  |  |  |  |  |

#### PastPaymentForm  (class Payment; 6 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| paidSum | Amount Paid | double | Y |  | c2 |  |  |
| paymentDate | Date Paid | dateTime | Y |  | dd/MM/yyyy |  | LE today |
| receiptNo | Receipt Number | string(30) | Y |  |  |  |  |
| balanceOwed | Balance Owed | double | Y |  | c2 |  |  |
| bankType | Financial Institution Name | Dictionary | Y | bank_list |  |  |  |
| comments | Comment | string(800) |  |  |  |  |  |

#### PastPaymentGrid  (class Payment; 4 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| receiptNo | Receipt Number | string(50) | Y |  |  |  |  |
| paidSum | Amount Paid | double | Y |  | c2 |  | GT 0 |
| paymentDate | Date Paid | dateTime | Y |  | dd/MM/yyyy |  | LE today |
| comments | Comment | string(800) |  |  |  |  |  |

#### PastTransactionForm  (class Transaction; 25 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| transactionNumber | Transaction Number | string(50) |  |  |  | all |  |
| transactionType | Transaction Type | TransactionMetaData | Y | transaction_type |  |  | if !=INSC |
| createDate | Transaction Start Date | dateTime | Y |  | dd/MM/yyyy hh:mm tt |  | LE today |
| completeDate | Transaction End Date | dateTime |  |  | dd/MM/yyyy hh:mm tt |  | LE today |
| applicationDate | Application Date | dateTime |  |  | dd/MM/yyyy |  | LE today |
| sourcePropertyIDs | Property Number | string(250) |  |  |  | all |  |
| consideration | Consideration | double |  |  | n2 |  |  |
| considerationCurrency | Consideration Currency | Dictionary |  | currency |  |  |  |
| relatedNumber | Related Transaction | string(50) |  |  |  |  |  |
| recordingno | Instrument Number | string(30) |  |  |  |  |  |
| folio | Page Number | string(20) |  |  |  |  |  |
| volume | Volume | string(20) |  |  |  |  |  |
| startDate | Date of Commencement | date |  |  | dd/MM/yyyy |  | LE today |
| termYear | Term (years) | integer |  |  | \d{1,2} |  | LE 99 |
| termMonth | Term (months) | integer |  |  | \d{1,2} |  | LE 11 |
| termDay | Term (days) | integer |  |  | \d{1,2} |  | LE 31 |
| expirationDate | Expiration Date | date |  |  | dd/MM/yyyy | all |  |
| payPeriodYear | Revision Period (years) | integer |  |  | \d{1,2} |  | LE 99 |
| balanceSum | Rental Amount | double |  |  | c2 |  |  |
| buildValue | Improvement Amount | double |  |  | n2 |  |  |
| improvementAmountCurrency | Improvement Amount Currency | Dictionary |  | currency |  |  |  |
| buildTerm | Erection (years) | integer |  |  | \d{1,2} |  | LE 99 |
| signDate | Execution Date | dateTime |  |  | dd/MM/yyyy |  | LE today |
| registrationDate | Registration Date | dateTime | Y |  | dd/MM/yyyy hh:mm tt |  | LE today |
| comments | Comment | string(800) |  |  |  |  |  |

#### PaymentForm  (class Payment; 5 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| paidSum | Amount Paid | double | Y |  | c2 | all |  |
| receiptNo | F.I. Receipt Number | string(30) | Y |  |  |  |  |
| paymentDate | Date Paid | dateTime | Y |  | dd/MM/yyyy |  | LE today |
| bankType | Financial Institution Name | Dictionary | Y | bank_list |  |  |  |
| comments | Comment | string(800) |  |  |  |  |  |

#### PaymentGrid  (class Payment; 5 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| paidSum | Amount Paid | double |  |  | c2 | all |  |
| receiptNo | F.I. Receipt Number | string(30) |  |  |  | all |  |
| paymentDate | Date Paid | dateTime |  |  | dd/MM/yyyy | all | LE today |
| bankName | Financial Institution Name | string(50) |  |  |  | all |  |
| comments | Comment | string(800) |  |  |  | all |  |

#### PaymentTransactionForm  (class PaymentTransaction; 7 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| paymentType | Fee Type | Dictionary |  | fee_type |  | all |  |
| paidSum | Total | double |  |  | c2 | all |  |
| paymentDate | Date of Calculation | dateTime |  |  | dd/MM/yyyy | all |  |
| bankType | Bank | Dictionary | Y | bank_details |  |  |  |
| bankName | Bank Name | string(50) |  |  |  | all |  |
| accountNumber | Account | string(50) |  |  |  | all |  |
| comments | Comments | string(800) |  |  |  |  |  |

#### PenaltyGrid  (class PaymentTransaction; 3 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| paidYear | Year | integer | Y |  |  |  |  |
| calculatedFee | Amount | double | Y |  | c2 |  |  |
| paymentDate | Date of Calculation | dateTime | Y |  | dd/MM/yyyy |  |  |

#### PenaltyGridMultiProp  (class PaymentTransaction; 4 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| paidYear | Year | integer | Y |  |  |  |  |
| calculatedFee | Amount | double | Y |  | c2 |  |  |
| paymentDate | Date of Calculation | dateTime | Y |  | dd/MM/yyyy |  |  |
| propertyNumber | Property Number | string | Y |  |  |  |  |

#### PlotAPCAForm  (class Parcel; 6 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| actualUseType | Land Use Category | Dictionary | Y | actual_use_type |  | all |  |
| landPurpose | Land Use Type | Dictionary | Y | use_specific |  | all |  |
| surveyDate | Survey Date | date | Y |  | dd/MM/yyyy | all |  |
| surveyName | Surveyor's Name | string(32) | Y |  |  | all |  |
| surveyPlanNo | Surveyor Plan Number | string(50) | Y |  |  | all |  |
| label | Comment | string(512) |  |  |  | all |  |

#### PlotForm  (class Parcel; 6 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| actualUseType | Land Use Category | Dictionary | Y | actual_use_type |  |  |  |
| landPurpose | Land Use Type | Dictionary | Y | use_specific |  |  |  |
| surveyDate | Survey Date | date | Y |  | dd/MM/yyyy |  |  |
| surveyName | Surveyor's Name | string(32) | Y |  |  |  |  |
| surveyPlanNo | Surveyor Plan Number | string(50) | Y |  |  |  |  |
| label | Comment | string(512) |  |  |  |  |  |

#### PlotFormCHPC  (class Parcel; 6 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| actualUseType | Land Use Category | Dictionary | Y | actual_use_type |  |  |  |
| landPurpose | Land Use Type | Dictionary | Y | use_specific |  |  |  |
| surveyDate | Survey Date | date | Y |  | dd/MM/yyyy |  |  |
| surveyName | Surveyor's Name | string(32) | Y |  |  |  |  |
| surveyPlanNo | Surveyor Plan Number | string(50) | Y |  |  |  |  |
| label | Comment | string(512) |  |  |  |  |  |

#### PlotFormForLodgment  (class Parcel; 6 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| actualUseType | Land Use Category | Dictionary | Y | actual_use_type |  |  |  |
| landPurpose | Land Use Type | Dictionary | Y | use_specific |  |  |  |
| surveyDate | Survey Date | date |  |  | dd/MM/yyyy |  |  |
| surveyName | Surveyor's Name | string(32) |  |  |  |  |  |
| surveyPlanNo | Surveyor Plan Number | string(50) |  |  |  |  |  |
| label | Comment | string(512) |  |  |  |  |  |

#### PlotFormForMERT  (class Parcel; 6 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| actualUseType | Land Use Category | Dictionary | Y | actual_use_type |  |  |  |
| landPurpose | Land Use Type | Dictionary | Y | use_specific |  |  |  |
| surveyDate | Survey Date | date | Y |  | dd/MM/yyyy | all |  |
| surveyName | Surveyor's Name | string(32) | Y |  |  | all |  |
| surveyPlanNo | Surveyor Plan Number | string(50) | Y |  |  | all |  |
| label | Comment | string(512) |  |  |  | all |  |

#### PlotFormForPSUR  (class Parcel; 6 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| actualUseType | Land Use Category | Dictionary | Y | actual_use_type |  | all |  |
| landPurpose | Land Use Type | Dictionary | Y | use_specific |  | all |  |
| surveyDate | Survey Date | date | Y |  | dd/MM/yyyy |  |  |
| surveyName | Surveyor's Name | string(32) | Y |  |  |  |  |
| surveyPlanNo | Surveyor Plan Number | string(50) | Y |  |  |  |  |
| label | Comment | string(512) |  |  |  |  |  |

#### PlotFormNonMandatoryLWA  (class Parcel, SpatialUnit; 9 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| volumefolio | Instrument Number(CofO) | string(150) |  |  |  |  |  |
| folio | Page number(CofO) | string(20) |  |  |  |  |  |
| volume | Volume Number(CofO) | string(20) |  |  |  |  |  |
| actualUseType | Land Use Category | Dictionary |  | actual_use_type |  |  |  |
| landPurpose | Land Use Type | Dictionary |  | use_specific |  |  |  |
| surveyPlanNo | Surveyor Plan Number | string(30) |  |  |  |  |  |
| surveyDate | Survey Date | date |  |  | dd/MM/yyyy |  |  |
| surveyName | Surveyor Name | string(30) |  |  |  |  |  |
| label | Description | string(512) |  |  |  |  |  |

#### PlotFormRO  (class Parcel; 6 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| actualUseType | Land Use Category | Dictionary | Y | actual_use_type |  | all |  |
| landPurpose | Land Use Type | Dictionary | Y | use_specific |  | all |  |
| surveyDate | Survey Date | date | Y |  | dd/MM/yyyy | all |  |
| surveyName | Surveyor's Name | string(32) | Y |  |  | all |  |
| surveyPlanNo | Surveyor Plan Number | string(50) | Y |  |  | all |  |
| label | Comment | string(512) |  |  |  | all |  |

#### PlotINSCForm  (class Parcel; 6 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| actualUseType | Land Use Category | Dictionary | Y | actual_use_type |  |  |  |
| landPurpose | Land Use Type | Dictionary | Y | use_specific |  |  |  |
| surveyDate | Survey Date | date |  |  | dd/MM/yyyy |  |  |
| surveyName | Surveyor's Name | string(32) |  |  |  |  |  |
| surveyPlanNo | Surveyor Plan Number | string(50) |  |  |  |  |  |
| label | Comment | string(512) |  |  |  |  |  |

#### PostponeForm  (class RejectionReason; 1 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| comments | Reason of Postponement | string(2000) | Y |  |  |  |  |

#### PropertyAPCAForm  (class SpatialUnit; 21 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| launit | Property Number | LAUnit(0) |  | launit |  | all |  |
| spatialUnitType | Property Type | Dictionary | Y | spatial_unit_type |  | all |  |
| planNumber | Plan Number | string(40) | Y |  |  | all |  |
| address.region | State | Dictionary | Y | region |  | all |  |
| address.localGovernmentArea | Local Government Area | Dictionary | Y | region_lga |  | all |  |
| propertyArea | Property Area | Dictionary | Y | property_area |  | all |  |
| zone | Location | Dictionary | Y | location |  | all |  |
| address.streetNumber | Street Number | string(50) |  |  |  | all |  |
| address.streetName | Street Name | string(100) |  |  |  | all |  |
| layout | Layout Name | Dictionary |  | layout_type |  | all |  |
| lotNumber | Plot Number | string(10) |  |  |  | all |  |
| blockNumber | Block Number | string(10) |  |  |  | all |  |
| address.districtText | District | string(100) |  |  |  | all |  |
| address.city | Town | string(50) |  |  |  | all |  |
| address.cell | Village | string(100) |  |  |  | all |  |
| address.ward | Ward | string(100) |  |  |  | all |  |
| area | Area | double | Y |  | n3 | all | GE 0 |
| areaUnitType | Measurement Unit | Dictionary | Y | area_measurement_unit |  | all |  |
| balanceSum | Valuation Amount | double |  |  | c2 |  |  |
| tracingNumber | Tracing Number | string(32) |  |  |  | all |  |
| upin | UPIN | string(50) |  |  |  | all |  |

#### PropertyCofOGrid  (class SpatialUnit; 4 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| uid | Property Number | string(0) |  |  |  | all |  |
| volumefolio | Instrument Number(CofO) | string(150) | Y |  |  |  |  |
| folio | Page number(CofO) | string(20) | Y |  |  |  |  |
| volume | Volume Number(CofO) | string(20) | Y |  |  |  |  |

#### PropertyForm  (class SpatialUnit; 21 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| launit | Property Number | LAUnit(0) |  | launit |  | all |  |
| spatialUnitType | Property Type | Dictionary | Y | spatial_unit_type |  | all |  |
| planNumber | Plan Number | string(40) | Y |  |  |  |  |
| address.region | State | Dictionary | Y | region |  |  |  |
| address.localGovernmentArea | Local Government Area | Dictionary | Y | region_lga |  |  |  |
| propertyArea | Property Area | Dictionary | Y | property_area |  |  |  |
| zone | Location | Dictionary | Y | location |  |  |  |
| address.streetNumber | Street Number | string(50) |  |  |  |  |  |
| address.streetName | Street Name | string(100) |  |  |  |  |  |
| layout | Layout Name | Dictionary |  | layout_type |  |  |  |
| lotNumber | Plot Number | string(10) |  |  |  |  |  |
| blockNumber | Block Number | string(10) |  |  |  |  |  |
| address.districtText | District | string(100) |  |  |  |  |  |
| address.city | Town | string(50) |  |  |  |  |  |
| address.cell | Village | string(100) |  |  |  |  |  |
| address.ward | Ward | string(100) |  |  |  |  |  |
| area | Area | double | Y |  | n3 |  | GE 0 |
| areaUnitType | Measurement Unit | Dictionary | Y | area_measurement_unit |  |  |  |
| balanceSum | Valuation Amount | double |  |  | c2 |  | GE 0 |
| tracingNumber | Tracing Number | string(32) |  |  |  |  |  |
| upin | UPIN | string(50) |  |  |  | all |  |

#### PropertyFormCHPC  (class SpatialUnit; 21 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| launit | Property Number | LAUnit(0) |  | launit |  | all |  |
| spatialUnitType | Property Type | Dictionary | Y | spatial_unit_type |  | all |  |
| planNumber | Plan Number | string(40) | Y |  |  | all |  |
| address.region | State | Dictionary | Y | region |  | all |  |
| address.localGovernmentArea | Local Government Area | Dictionary | Y | region_lga |  | all |  |
| propertyArea | Property Area | Dictionary | Y | property_area |  | all |  |
| zone | Location | Dictionary | Y | location |  | all |  |
| address.streetNumber | Street Number | string(50) |  |  |  |  |  |
| address.streetName | Street Name | string(100) |  |  |  | all |  |
| layout | Layout Name | Dictionary |  | layout_type |  | all |  |
| lotNumber | Plot Number | string(10) |  |  |  |  |  |
| blockNumber | Block Number | string(10) |  |  |  | all |  |
| address.districtText | District | string(100) |  |  |  | all |  |
| address.city | Town | string(50) |  |  |  | all |  |
| address.cell | Village | string(100) |  |  |  | all |  |
| address.ward | Ward | string(100) |  |  |  | all |  |
| area | Area | double | Y |  | n3 | all | GE 0 |
| areaUnitType | Measurement Unit | Dictionary | Y | area_measurement_unit |  | all |  |
| balanceSum | Valuation Amount | double |  |  | c2 |  | GE 0 |
| tracingNumber | Tracing Number | string(32) |  |  |  | all |  |
| upin | UPIN | string(50) |  |  |  | all |  |

#### PropertyFormDCT  (class SpatialUnit; 21 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| launit | Property Number | LAUnit(0) |  | launit |  | all |  |
| spatialUnitType | Property Type | Dictionary | Y | spatial_unit_type |  | all |  |
| planNumber | Plan Number | string(40) |  |  |  |  |  |
| address.region | State | Dictionary | Y | region |  |  |  |
| address.localGovernmentArea | Local Government Area | Dictionary | Y | region_lga |  |  |  |
| propertyArea | Property Area | Dictionary | Y | property_area |  |  |  |
| zone | Location | Dictionary | Y | location |  |  |  |
| address.streetNumber | Street Number | string(50) |  |  |  |  |  |
| address.streetName | Street Name | string(100) |  |  |  |  |  |
| layout | Layout Name | Dictionary |  | layout_type |  |  |  |
| lotNumber | Plot Number | string(10) |  |  |  |  |  |
| blockNumber | Block Number | string(10) |  |  |  |  |  |
| address.districtText | District | string(100) |  |  |  |  |  |
| address.city | Town | string(50) |  |  |  |  |  |
| address.cell | Village | string(100) |  |  |  |  |  |
| address.ward | Ward | string(100) |  |  |  |  |  |
| area | Area | double | Y |  | n3 |  | GE 0 |
| areaUnitType | Measurement Unit | Dictionary | Y | area_measurement_unit |  |  |  |
| balanceSum | Valuation Amount | double |  |  | c2 |  | GE 0 |
| tracingNumber | Tracing Number | string(32) |  |  |  |  |  |
| upin | UPIN | string(50) |  |  |  | all |  |

#### PropertyFormForEXTTandPSUR  (class SpatialUnit; 21 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| launit | Property Number | LAUnit(0) |  | launit |  | all |  |
| spatialUnitType | Property Type | Dictionary | Y | spatial_unit_type |  | all |  |
| planNumber | Plan Number | string(40) | Y |  |  | all |  |
| address.region | State | Dictionary | Y | region |  | all |  |
| address.localGovernmentArea | Local Government Area | Dictionary | Y | region_lga |  | all |  |
| propertyArea | Property Area | Dictionary | Y | property_area |  | all |  |
| zone | Location | Dictionary | Y | location |  | all |  |
| address.streetNumber | Street Number | string(50) |  |  |  |  |  |
| address.streetName | Street Name | string(100) |  |  |  | all |  |
| layout | Layout Name | Dictionary |  | layout_type |  | all |  |
| lotNumber | Plot Number | string(10) |  |  |  |  |  |
| blockNumber | Block Number | string(10) |  |  |  | all |  |
| address.districtText | District | string(100) |  |  |  | all |  |
| address.city | Town | string(50) |  |  |  | all |  |
| address.cell | Village | string(100) |  |  |  | all |  |
| address.ward | Ward | string(100) |  |  |  | all |  |
| area | Area | double | Y |  | n3 |  | GE 0 |
| areaUnitType | Measurement Unit | Dictionary | Y | area_measurement_unit |  |  |  |
| balanceSum | Valuation Amount | double |  |  | c2 |  | GE 0 |
| tracingNumber | Tracing Number | string(32) |  |  |  | all |  |
| upin | UPIN | string(50) |  |  |  | all |  |

#### PropertyFormForLodgment  (class SpatialUnit; 21 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| launit | Property Number | LAUnit(0) |  | launit |  | all |  |
| spatialUnitType | Property Type | Dictionary | Y | spatial_unit_type |  | all |  |
| planNumber | Plan Number | string(40) |  |  |  |  |  |
| address.region | State | Dictionary |  | region |  |  |  |
| address.localGovernmentArea | Local Government Area | Dictionary |  | region_lga |  |  |  |
| propertyArea | Property Area | Dictionary | Y | property_area |  |  |  |
| zone | Location | Dictionary | Y | location |  |  |  |
| address.streetNumber | Street Number | string(50) |  |  |  |  |  |
| address.streetName | Street Name | string(100) |  |  |  |  |  |
| layout | Layout Name | Dictionary |  | layout_type |  |  |  |
| lotNumber | Plot Number | string(10) |  |  |  |  |  |
| blockNumber | Block Number | string(10) |  |  |  |  |  |
| address.districtText | District | string(100) |  |  |  |  |  |
| address.city | Town | string(50) |  |  |  |  |  |
| address.cell | Village | string(100) |  |  |  |  |  |
| address.ward | Ward | string(100) |  |  |  |  |  |
| area | Area | double |  |  | n3 |  | GE 0 |
| areaUnitType | Measurement Unit | Dictionary |  | area_measurement_unit |  |  |  |
| balanceSum | Valuation Amount | double |  |  | c2 |  | GE 0 |
| tracingNumber | Tracing Number | string(32) |  |  |  |  |  |
| upin | UPIN | string(50) |  |  |  | all |  |

#### PropertyFormForMERT  (class SpatialUnit; 21 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| launit | Property Number | LAUnit(0) |  | launit |  | all |  |
| spatialUnitType | Property Type | Dictionary | Y | spatial_unit_type |  | all |  |
| planNumber | Plan Number | string(40) | Y |  |  | all |  |
| address.region | State | Dictionary | Y | region |  | all |  |
| address.localGovernmentArea | Local Government Area | Dictionary | Y | region_lga |  | all |  |
| propertyArea | Property Area | Dictionary | Y | property_area |  | all |  |
| zone | Location | Dictionary | Y | location |  | all |  |
| address.streetNumber | Street Number | string(50) |  |  |  | all |  |
| address.streetName | Street Name | string(100) |  |  |  | all |  |
| layout | Layout Name | Dictionary |  | layout_type |  | all |  |
| lotNumber | Plot Number | string(10) |  |  |  | all |  |
| blockNumber | Block Number | string(10) |  |  |  | all |  |
| address.districtText | District | string(100) |  |  |  | all |  |
| address.city | Town | string(50) |  |  |  | all |  |
| address.cell | Village | string(100) |  |  |  | all |  |
| address.ward | Ward | string(100) |  |  |  | all |  |
| area | Area | double | Y |  | n3 |  | GE 0 |
| areaUnitType | Measurement Unit | Dictionary | Y | area_measurement_unit |  |  |  |
| balanceSum | Valuation Amount | double |  |  | c2 | all | GE 0 |
| tracingNumber | Tracing Number | string(32) |  |  |  | all |  |
| upin | UPIN | string(50) |  |  |  | all |  |

#### PropertyFormINSC  (class SpatialUnit; 21 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| launit | Property Number | LAUnit(0) |  | launit |  | all |  |
| spatialUnitType | Property Type | Dictionary | Y | spatial_unit_type |  | all |  |
| planNumber | Plan Number | string(40) |  |  |  |  |  |
| address.region | State | Dictionary | Y | region |  | all |  |
| address.localGovernmentArea | Local Government Area | Dictionary | Y | region_lga |  |  |  |
| propertyArea | Property Area | Dictionary | Y | property_area |  |  |  |
| zone | Location | Dictionary | Y | location |  |  |  |
| address.streetNumber | Street Number | string(50) |  |  |  |  |  |
| address.streetName | Street Name | string(100) |  |  |  |  |  |
| layout | Layout Name | Dictionary |  | layout_type |  |  |  |
| lotNumber | Plot Number | string(10) |  |  |  |  |  |
| blockNumber | Block Number | string(10) |  |  |  |  |  |
| address.districtText | District | string(100) |  |  |  |  |  |
| address.city | Town | string(50) |  |  |  |  |  |
| address.cell | Village | string(100) |  |  |  |  |  |
| address.ward | Ward | string(100) |  |  |  |  |  |
| area | Area | double | Y |  | n3 |  | GE 0 |
| areaUnitType | Measurement Unit | Dictionary | Y | area_measurement_unit |  |  |  |
| balanceSum | Valuation Amount | double |  |  | c2 |  |  |
| tracingNumber | Tracing Number | string(32) |  |  |  |  |  |
| upin | UPIN | string(50) |  |  |  | all |  |

#### PropertyFormNonMandatory  (class SpatialUnit; 18 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| launit | Property No. | LAUnit(0) |  | launit |  | all |  |
| spatialUnitType | Property Type | Dictionary | Y | spatial_unit_type |  |  |  |
| planNumber | Plan No. | string(10) |  |  |  |  |  |
| address.region | State | Dictionary |  | region |  |  |  |
| address.localGovernmentArea | Local Government Area | Dictionary |  | region_lga |  |  |  |
| propertyArea | Property Area | Dictionary | Y | property_area |  |  |  |
| zone | Location | Dictionary |  | zone_type |  |  | if !=zone_urban |
| address.streetNumber | Street Number | string(50) |  |  |  |  |  |
| address.streetName | Street Name | string(100) |  |  |  |  |  |
| layout | Layout Name | Dictionary |  | layout_type |  |  |  |
| lotNumber | Plot Number | string(50) |  |  |  |  |  |
| blockNumber | Block Number | string(10) |  |  |  |  |  |
| address.cell | Village | string(100) |  |  |  |  |  |
| address.ward | Ward | string(100) |  |  |  |  |  |
| area | Area | double |  |  | n3 |  | GE 0 |
| areaUnitType | Measurement Unit | Dictionary |  | area_measurement_unit |  |  |  |
| tracingNumber | Tracing No. | string(30) |  |  |  |  |  |
| upin | UPIN | string(50) |  |  |  |  |  |

#### PropertyGrid  (class SpatialUnit; 13 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| uid | Property Number | string(0) |  |  |  | all |  |
| planNumber | Plan Number | string(40) | Y |  |  |  |  |
| spatialUnitType | Property Type | Dictionary | Y | spatial_unit_type |  | all |  |
| address.localGovernmentArea | Local Government Area | Dictionary | Y | region_lga |  |  |  |
| propertyArea | Property Area | Dictionary | Y | property_area |  |  |  |
| zone | Location | Dictionary | Y | location |  |  |  |
| layout | Layout Name | Dictionary |  | layout_type |  |  |  |
| lotNumber | Plot Number | string(10) |  |  |  |  |  |
| tracingNumber | Tracing No. | string(32) |  |  |  |  |  |
| blockNumber | Block Number | string(10) |  |  |  |  |  |
| area | Area | double | Y |  | n3 |  | GE 0 |
| areaUnitType | Measurement Unit | Dictionary |  | area_measurement_unit |  |  |  |
| upin | UPIN | string(50) |  |  |  |  |  |

#### PropertyGridShort  (class SpatialUnit; 10 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| spatialUnitType | Property Type | Dictionary |  | spatial_unit_type |  |  | if !=spatial_unit_building |
| address.region | State | Dictionary | Y | region |  |  |  |
| address.localGovernmentArea | Local Government Area | Dictionary | Y | region_lga |  |  |  |
| propertyArea | Property Area | Dictionary | Y | property_area |  |  |  |
| address.subcounty | Subcounty/Town | string(30) | Y |  |  |  |  |
| address.ward | Parish/Zone | string(100) |  |  |  |  |  |
| address.cell | Village/Cell | string(100) |  |  |  |  |  |
| suID | LRS ID | string |  |  |  |  |  |
| uid | Property UID | string |  |  |  |  |  |
| upin | UPIN | string(50) |  |  |  |  |  |

#### PropertyGridWithSourcePropertyLWA  (class SpatialUnit; 14 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| uid | Property Number | string(0) |  |  |  | all |  |
| planNumber | Plan Number | string(40) | Y |  |  |  |  |
| spatialUnitType | Property Type | Dictionary | Y | spatial_unit_type |  | all |  |
| address.localGovernmentArea | Local Government Area | Dictionary | Y | region_lga |  |  |  |
| propertyArea | Property Area | Dictionary | Y | property_area |  |  |  |
| zone | Location | Dictionary | Y | location |  |  |  |
| layout | Layout Name | Dictionary |  | layout_type |  |  |  |
| lotNumber | Plot Number | string(10) |  |  |  |  |  |
| tracingNumber | Tracing No. | string(32) |  |  |  |  |  |
| blockNumber | Block Number | string(10) |  |  |  |  |  |
| area | Area | double | Y |  | n3 |  | GE 0 |
| areaUnitType | Measurement Unit | Dictionary |  | area_measurement_unit |  |  |  |
| upin | UPIN | string(50) |  |  |  |  |  |
| sourceTarget | Source/Target | string(0) |  |  |  | all |  |

#### PropertySearchForm  (class SpatialUnit; 11 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| spatialUnitType | Property Type | Dictionary |  | spatial_unit_type |  |  |  |
| uid | Property Number | string |  |  |  |  |  |
| planNumber | Plan Number | string(40) | Y |  |  |  |  |
| upin | UPIN | string(50) |  |  |  |  |  |
| address.region | State | Dictionary |  | region |  |  |  |
| address.localGovernmentArea | Local Government Area | Dictionary | Y | region_lga |  |  |  |
| propertyArea | Property Area | Dictionary | Y | property_area |  |  |  |
| zone | Location | Dictionary | Y | location |  |  |  |
| blockNumber | Block Number | string(10) |  |  |  |  |  |
| lotNumber | Plot Number | string(10) |  |  |  |  |  |
| address.cell | Village | string(100) |  |  |  |  |  |

#### PropertyTransactionsGrid  (class Transaction; 5 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| transactionNumber | Transaction No. | string |  |  |  |  |  |
| transactionType | Transaction | TransactionMetaData |  | transaction_type |  |  |  |
| createDate | Submission Date | dateTime |  |  | dd/MM/yyyy hh:mm tt |  |  |
| registrationDate | Date Registered | dateTime | Y |  | dd/MM/yyyy hh:mm tt |  | LE today |
| transactionStatus | Transaction Status | Dictionary |  | transaction_status |  |  |  |

#### PropertyUnitForm  (class SpatialUnit; 17 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| launit | System Property No. | LAUnit(0) |  | launit |  | all |  |
| spatialUnitType | Property Type | Dictionary | Y | spatial_unit_type |  |  |  |
| purpose | Use Type | Dictionary |  | actual_use_type |  |  |  |
| specific | Use Specific | string(50) |  |  |  |  |  |
| address.region | Province | Dictionary | Y | region |  |  |  |
| address.localGovernmentArea | Local Government Area | Dictionary | Y | region_lga |  |  |  |
| propertyArea | Property Area | Dictionary | Y | property_area |  |  |  |
| address.city | City/Village | string(100) |  |  |  |  |  |
| address.subcounty | Sub-District (Nahia) | string(30) | Y |  |  |  |  |
| address.cell | Guzar | string(100) | Y |  |  |  |  |
| address.streetName | Street Name | string(100) |  |  |  |  |  |
| address.buildingNumber | Building Number | int(0) |  |  |  |  |  |
| blockNumber | Block No. | string(50) |  |  |  |  |  |
| lotNumber | Plot No. | string(50) |  |  |  |  |  |
| area | Area | double |  |  | n3 |  |  |
| areaUnitType | Area Units | Dictionary |  | area_measurement_unit |  |  |  |
| comments | Comments | string(800) |  |  |  |  |  |

#### RRRAttributeDisableForFRTrForm  (class RRR; 14 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| termYear | Term (years) | integer |  |  | \d{1,2} | APRO,APSL,CONT,RAOL,REGR | LE 99 |
| termMonth | Term (months) | integer |  |  | \d{1,2} | APRO,APSL,CONT,RAOL,REGR | LE 11 |
| termDay | Term (days) | integer |  |  | \d{1,2} | APRO,APSL,CONT,RAOL,REGR | LE 31 |
| startDate | Date of Commencement | date |  |  | dd/MM/yyyy |  | LE today |
| expirationDate | Expiration Date | date |  |  | dd/MM/yyyy | APRO,APSL,CONT,RAOL,REGR |  |
| rentalAmount | Rental Amount | double |  |  | c2 | APRO,APSL,CONT,RAOL,REGR | GE 0 |
| payPeriodYear | Revision Period | integer |  |  | \d{1,2} | APRO,APSL,CONT,RAOL,REGR | LE 99 |
| buildTerm | Erection Period | integer |  |  | \d{1,2} | APRO,APSL,CONT,RAOL,REGR | LE 99 |
| consideration | Consideration | double |  |  | n2 | APRO,APSL,CONT,RAOL,REGR | enabled_for all |
| considerationCurrency | Consideration Currency | Dictionary |  | currency |  | APRO,APSL,CONT,RAOL,REGR |  |
| buildValue | Improvement Amount | double |  |  | n2 | APRO,APSL,CONT,RAOL,REGR | GE 0 |
| improvementAmountCurrency | Improvement Amount Currency | Dictionary |  | currency |  | APRO,APSL,CONT,RAOL,REGR |  |
| securityNumber1 | Security Number #1 | integer |  |  | \d{1,9} |  | GT 0 |
| securityNumber2 | Security Number #2 | integer |  |  | \d{1,9} |  |  |

#### RRRAttributeForm  (class RRR; 14 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| termYear | Term (years) | integer |  |  | \d{1,2} | APCA,APCM,CETC,CHNT,COMP,GRPM,LOCE,RM... | LE 99 |
| termMonth | Term (months) | integer |  |  | \d{1,2} | APSL,APRO,ASSI,ASWG,CEMP,CETC,CHNT,CH... | LE 11 |
| termDay | Term (days) | integer |  |  | \d{1,2} | APCA,APCM,APSL,APRO,ASSI,ASWG,CEMP,CE... | LE 31 |
| startDate | Date of Commencement | date |  |  | dd/MM/yyyy | CETC,CHNT,COMP,GRPM,LOCE,RMOR,RSUB,RC... | LE today |
| expirationDate | Expiration Date | date |  |  | dd/MM/yyyy | all |  |
| rentalAmount | Rental Amount | double |  |  | c2 | CETC,COMP,GRPM,LOCE,RMOR,RSUB,RCOO,RR... | GE 0 |
| payPeriodYear | Revision Period | integer |  |  | \d{1,2} | CETC,COMP,GRPM,LOCE,RMOR,RSUB,RCOO,RR... | LE 99 |
| buildTerm | Erection Period | integer |  |  | \d{1,2} | CETC,COMP,GRPM,LOCE,RMOR,RSUB,RCOO,RR... | LE 99 |
| consideration | Consideration | double |  |  | n2 | APCA,APSL,APRO,ASSI,ASWG,CEMP,CETC,CH... | enabled_for all |
| considerationCurrency | Consideration Currency | Dictionary |  | currency |  | APCA,APSL,APRO,ASSI,ASWG,CEMP,CETC,CH... |  |
| buildValue | Improvement Amount | double |  |  | n2 | CETC,COMP,GRPM,LOCE,RMOR,RSUB,RCOO,RR... | GE 0 |
| improvementAmountCurrency | Improvement Amount Currency | Dictionary |  | currency |  | CETC,COMP,GRPM,LOCE,RMOR,RSUB,RCOO,RR... |  |
| securityNumber1 | Security Number #1 | integer |  |  | \d{1,9} | CETC,COMP,GRPM,LOCE,RMOR,RSUB,RCOO,RR... | GT 0 |
| securityNumber2 | Security Number #2 | integer |  |  | \d{1,9} | CETC,COMP,GRPM,LOCE,RMOR,RSUB,RCOO,RR... |  |

#### RRRAttributeLWA  (class RRR; 14 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| rrrType | Right Type | RrrMetaData |  |  |  | all |  |
| startDate | Date of Commencement | date |  |  | dd/MM/yyyy | all |  |
| termYear | Term (years) | integer |  |  | \d{1,2} | all |  |
| termMonth | Term (months) | integer |  |  | \d{1,2} | all | LE 11 |
| termDay | Term (days) | integer |  |  | \d{1,2} | all | LE 31 |
| expirationDate | Expiration Date | date |  |  | dd/MM/yyyy | all |  |
| rentalAmount | Rental Amount | double |  |  | c2 | all |  |
| consideration | Consideration | double |  |  | n2 | all |  |
| considerationCurrency | Consideration Currency | Dictionary |  | currency |  |  |  |
| payPeriodYear | Revision Period | integer |  |  | \d{1,2} | all |  |
| buildValue | Improvement Amount | double |  |  | n2 | all |  |
| improvementAmountCurrency | Improvement Amount Currency | Dictionary |  | currency |  | all |  |
| buildTerm | Erection Period | integer |  |  | \d{1,2} | all |  |
| status | Status | string |  |  |  | all |  |

#### RRRAttributeUPMOForm  (class RRR; 14 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| termYear | Term (years) | integer |  |  | \d{1,2} |  | LE 99 |
| termMonth | Term (months) | integer |  |  | \d{1,2} | UPMO | LE 11 |
| termDay | Term (days) | integer |  |  | \d{1,2} | UPMO | LE 31 |
| startDate | Date of Commencement | date |  |  | dd/MM/yyyy |  | LE today |
| expirationDate | Expiration Date | date |  |  | dd/MM/yyyy | all |  |
| rentalAmount | Rental Amount | double |  |  | c2 |  | GE 0 |
| payPeriodYear | Revision Period | integer |  |  | \d{1,2} | all | LE 99 |
| buildTerm | Erection Period | integer |  |  | \d{1,2} |  | LE 99 |
| addedConsideration | Consideration | double | Y |  | n2 |  |  |
| considerationCurrency | Consideration Currency | Dictionary | Y | currency |  |  |  |
| buildValue | Improvement Amount | double |  |  | n2 |  | GE 0 |
| improvementAmountCurrency | Improvement Amount Currency | Dictionary |  | currency |  |  |  |
| securityNumber1 | Security Number #1 | integer |  |  | \d{1,9} |  | GT 0 |
| securityNumber2 | Security Number #2 | integer |  |  | \d{1,9} |  |  |

#### RecommendationForm  (class Condition; 6 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| conditionType | Type | Dictionary |  | condition_type |  | all |  |
| lastEdited | Date of Decision/Note | date(0) | Y |  | dd/MM/yyyy |  | LE today |
| condStatus | Status | Dictionary | Y | condition_status |  |  |  |
| conditionText | Decision/Note Text | string(1500) |  |  |  |  |  |
| originalText | Reason for Decision/Note | string(1000) |  |  |  |  |  |
| lastEditedBy | Booked By | string(0) |  |  |  | all |  |

#### RecommendationGrid  (class Condition; 5 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| conditionType | Type | Dictionary | Y | condition_type |  | all |  |
| lastEdited | Date of Decision/Note | date(0) | Y |  | dd/MM/yyyy |  |  |
| condStatus | Status | Dictionary | Y | condition_status |  |  |  |
| conditionText | Decision/Note Text | string(1500) |  |  |  |  |  |
| lastEditedBy | Decision By | string(0) |  |  |  | all |  |

#### RegUnitForm  (class LAUnit; 3 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| uid | System Property No. | string |  |  |  | all |  |
| regDate | File Creation Date | date | Y |  | dd/MM/yyyy |  | LE today |
| certNumbers | Certificate No. | string(50) |  |  |  |  |  |

#### RegisterInstrumentForm  (class Transaction; 4 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| recordingno | Instrument Number | string(30) | Y |  |  | all |  |
| folio | Page Number | string(20) | Y |  |  | all |  |
| volume | Volume | string(20) | Y |  |  | all |  |
| registrationDate | Registration Date | dateTime | Y |  | dd/MM/yyyy hh:mm tt | all | LE today |

#### RegisterPlanNumberForm  (class SurveyPlan; 2 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| planNumber | Plan Number | string(30) | Y |  |  | all |  |
| approvalDate | Approval Date | dateTime | Y |  | dd/MM/yyyy hh:mm tt | all | LE today |

#### RejectForm  (class RejectionReason; 2 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| reasonType | Reason | Dictionary | Y | returned_reason |  |  |  |
| comments | In Addition | string(2000) |  |  |  |  |  |

#### RejectFormGrid  (class RejectionReason; 2 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| comments | In Addition | string(2000) |  |  |  |  |  |
| reasonType | Reason | Dictionary |  | returned_reason |  |  |  |

#### RelatedTransactionGrid  (class Transaction; 5 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| transactionType | Transaction Type | TransactionMetaData |  | transaction_type |  | all |  |
| transactionNumber | Transaction No. | string |  |  |  | all |  |
| recordingno | Instrument Number | string(30) |  |  |  | all |  |
| folio | Page Number | string(20) |  |  |  | all |  |
| volume | Volume | string(20) |  |  |  | all |  |

#### RelatedTransactionHistoryForm  (class Transaction; 4 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| transactionNumber | Transaction No. | string(50) | Y |  |  | all |  |
| transactionType | Transaction Type | Dictionary | Y | transaction_type |  |  |  |
| transactionStatus | Transaction Status | Dictionary | Y | transaction_status |  |  |  |
| createDate | Transaction Creation Date | dateTime | Y |  | dd/MM/yyyy hh:mm tt | all |  |

#### RepresentativeGridLWA  (class Party; 7 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| contactType | Type | Dictionary |  | party_type |  |  | if ==representative |
| firstName | First Name | string |  |  |  |  |  |
| lastName | Last Name | string |  |  |  |  |  |
| address.region | State | Dictionary |  | region |  |  |  |
| address.localGovernmentArea | Local Government Area | Dictionary |  | local_government_area |  |  |  |
| address.originRegion | State of Origin | Dictionary | Y | region |  |  |  |
| address.streetName | Address | string |  |  |  |  |  |

#### ReturnForm  (class RejectionReason; 2 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| reasonType | Reason | Dictionary | Y | returned_reason |  |  |  |
| comments | In Addition | string(2000) |  |  |  |  |  |

#### SearchApplicantGrid  (class Applicant; 4 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| partyType | Capacity | Dictionary |  | party_type |  |  | if !=party_scan |
| partyRoleType | Role | Dictionary |  | applicant_role_type |  |  |  |
| name | Name/Organization | string(150) |  |  |  |  |  |
| person.phone1Number | Phone | string(20) |  |  |  |  |  |

#### SearchApplicantGridLWA  (class Applicant; 5 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| partyType | Type | Dictionary |  | party_type |  |  | if !=representative |
| person.address.region | State/Province | Dictionary |  | region |  |  |  |
| person.address.localGovernmentArea | Local Government Area | Dictionary |  | region_lga |  |  |  |
| person.address.city | Town/Municipal Division | string |  |  |  |  |  |
| person.address.streetName | Address | string |  |  |  |  |  |

#### SearchCorpApplicantForm  (class Applicant; 9 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| partyType | Capacity | Dictionary |  | party_type |  |  | if !=party_scan |
| partyRoleType | Role | Dictionary |  | applicant_role_type |  |  |  |
| person.name | Organization Name | string(150) |  |  |  |  |  |
| person.address.country | Country | Dictionary |  | country |  |  |  |
| person.address.region | State | Dictionary |  | region |  |  |  |
| person.address.localGovernmentArea | Local Government Area | Dictionary |  | region_lga |  |  |  |
| person.address.cell | Village | string(100) |  |  |  |  |  |
| person.address.streetName | Street Name | string(100) |  |  |  |  |  |
| person.phone1Number | Phone | string(20) |  |  |  |  |  |

#### SearchIndApplicantForm  (class Applicant; 11 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| partyType | Capacity | Dictionary |  | party_type |  |  | if !=party_scan |
| partyRoleType | Role | Dictionary |  | applicant_role_type |  |  |  |
| person.firstName | First Name | string(150) |  |  |  |  |  |
| person.secondName | Middle Name | string(150) |  |  |  |  |  |
| person.lastName | Last Name | string(150) |  |  |  |  |  |
| person.address.country | Country | Dictionary |  | country |  |  |  |
| person.address.region | State | Dictionary |  | region |  |  |  |
| person.address.localGovernmentArea | Local Government Area | Dictionary |  | region_lga |  |  |  |
| person.address.streetName | Street Name | string(100) |  |  |  |  |  |
| person.address.cell | Village | string(100) |  |  |  |  |  |
| person.phone1Number | Phone | string(20) |  |  |  |  |  |

#### ShellFormQueue  (class Queue; 12 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| processId | Process ID | long |  |  |  |  |  |
| transactionNumber | Transaction Number | string | Y |  |  |  |  |
| transactionType | Transaction | TransactionMetaData | Y | transaction_type |  |  |  |
| processName | Workflow | string |  |  |  |  |  |
| taskDescription | Workflow Stage | string |  |  |  |  |  |
| applicantName | Applicant Name | string | Y |  |  |  |  |
| taskActorName | Assigned To | Authority |  | authority |  |  |  |
| sourceRids | Source Property Number | string | Y |  |  |  |  |
| targetRids | Target Property Number | string | Y |  |  |  |  |
| createDate | Submission Date | dateTime |  |  | dd/MM/yyyy |  |  |
| lockOwner | Locked By | string |  |  |  |  |  |
| processSuspended | Postponed | boolean |  |  |  |  |  |

#### ShellFormQueueDetails  (class Queue; 12 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| processId | Process ID | long |  |  |  |  |  |
| transactionNumber | Transaction Number | string | Y |  |  |  |  |
| transactionType | Transaction | TransactionMetaData | Y | transaction_type |  |  |  |
| processName | Workflow | string |  |  |  |  |  |
| taskDescription | Workflow Stage | string |  |  |  |  |  |
| applicantName | Applicant Name | string | Y |  |  |  |  |
| taskActorName | Assigned To | Authority |  | authority |  |  |  |
| sourceRids | Source Property Number | string |  |  |  |  |  |
| targetRids | Target Property Number | string |  |  |  |  |  |
| createDate | Submission Date | dateTime |  |  | dd/MM/yyyy |  |  |
| lockOwner | Locked By | string |  |  |  |  |  |
| processSuspended | Postponed | boolean |  |  |  |  |  |

#### SimpleTransactionGrid  (class Transaction; 3 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| transactionType | Transaction | TransactionMetaData |  | transaction_type |  |  |  |
| transactionNumber | Transaction No. | string |  |  |  |  |  |
| sourcePropertyIDs | System Property No. | string |  |  |  |  |  |

#### SiteAnaFormNonMandatoryLWA  (class SiteAnalysis; 54 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| neighbourhood | Neighbourhood | string(50) |  |  |  |  |  |
| maxHighAllow | Maximum Height Allowable | Dictionary |  | maximum_height_allowable |  |  |  |
| developmentStatus | Development Status | Dictionary |  | development_status |  |  |  |
| density | Density | Dictionary |  | density |  |  |  |
| withPlanningSchema | Within Planning Scheme | boolean |  |  |  |  |  |
| tpsNoptaNumber | TPS-NOTPA Number | string(50) |  |  |  |  |  |
| maxAreaDevelopment | Maximum Area Developable | double |  |  | n3 |  |  |
| topography | Topography | Dictionary |  | topography |  |  |  |
| soliCondition | Soil condition | Dictionary |  | soil_condition |  |  |  |
| accessRoadStatus | Access Road Status | Dictionary |  | access_road_status |  |  |  |
| hierachyRoad | Hierarchy Road | string(50) |  |  |  |  |  |
| federalRoadAccess | Federal Road Access | boolean |  |  |  |  |  |
| accessRoadWidth | Access Road Width | double |  |  | n3 |  |  |
| accessRoadTerminate | Access Road Terminate at Plot | boolean |  |  |  |  |  |
| developmentHinderFutureDevelopment | Development hinder future Development | boolean |  |  |  |  |  |
| proximityToSchool | Proximity to School | double |  |  | n3 |  |  |
| proximityToHospital | Proximity to Hospital | double |  |  | n3 |  |  |
| proximityToShop | Proximity to Shopping Centre | double |  |  | n3 |  |  |
| proximityToWorship | Proximity to Place of Worship | double |  |  | n3 |  |  |
| proximityToSportarena | Proximity to Sport Arena | double |  |  | n3 |  |  |
| proximityToOffice | Proximity to Office | double |  |  | n3 |  |  |
| proximityToIndustry | Proximity to Industry | double |  |  | n3 |  |  |
| proximityToOther | Proximity to Other | double |  |  | n3 |  |  |
| proximityComment | Proximity Comment | string(800) |  |  |  |  |  |
| proximityToUtility | Proximity to Utility Line | double |  |  | n3 |  |  |
| proximityToElectricity | Proximity to High Tension Electricity Line | double |  |  | n3 |  |  |
| capacity | Capacity | double |  |  | n3 |  |  |
| proximityToWater | Proximity to Water Main | double |  |  | n3 |  |  |
| proximityToNnpc | Proximity to N.N.P.C. Pipe Line | double |  |  | n3 |  |  |
| proximityToTelephone | Proximity to Telephone Line | double |  |  | n3 |  |  |
| proximityFurtherComment | Proximity Further Comment | string(800) |  |  |  |  |  |
| distanceToLake | Distance to Special Feature (Lake) | double |  |  | n3 |  |  |
| distanceToMountain | Distance to Special Feature (Mountain) | double |  |  | n3 |  |  |
| distanceToQuarry | Distance to Special Feature (Quarry Site) | double |  |  | n3 |  |  |
| distanceToRiver | Distance to Special Feature (River) | double |  |  | n3 |  |  |
| distanceToGully | Distance to Special Feature (Gully) | double |  |  | n3 |  |  |
| distanceToRailway | Distance to Special Feature (Railway Lines) | double |  |  | n3 |  |  |
| distanceToExpressway | Distance to Special Feature (Expressway) | double |  |  | n3 |  |  |
| distanceToOthers | Distance to Special Feature (Others) | double |  |  | n3 |  |  |
| distanceToFeatureComment | Distance to Special Feature (Comment) | string(800) |  |  |  |  |  |
| setbackBuilding1 | Setback Building 1 | double |  |  | n3 |  |  |
| setbackBuilding1LandUse | Setback Building 1 Land Use Category | Dictionary |  | actual_use_type |  |  |  |
| setbackBuilding2 | Setback Building 2 | double |  |  | n3 |  |  |
| setbackBuilding2LandUse | Setback Building 2 Land Use Category | Dictionary |  | actual_use_type |  |  |  |
| setbackBuilding3 | Setback Building 3 | double |  |  | n3 |  |  |
| setbackBuilding3LandUse | Setback Building 3 Land Use Category | Dictionary |  | actual_use_type |  |  |  |
| setbackBuilding4 | Setback Building 4 | double |  |  | n3 |  |  |
| setbackBuilding4LandUse | Setback Building 4 Land Use Category | Dictionary |  | actual_use_type |  |  |  |
| setbackAcccessRoad | Setback Access Road | double |  |  | n3 |  |  |
| setbackOtherComment | Setback Other Comment | string(800) |  |  |  |  |  |
| householdUtilityAccess | Household Utility lines accessible | boolean |  |  |  |  |  |
| conformityOfDevelopment | Conformity of Development | string(50) |  |  |  |  |  |
| fencingEncourage | Fencing Encourage | boolean |  |  |  |  |  |
| fencing_type | Fencing Type | string(50) |  | fencing_type |  |  |  |

#### SiteAnalysisForm  (class SiteAnalysis; 55 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| neighbourhood | Neighbourhood | string(50) |  |  |  |  |  |
| maxHighAllow | Maximum Height Allowable | Dictionary |  | maximum_height_allowable |  |  |  |
| developmentStatus | Development Status | Dictionary |  | development_status |  |  |  |
| density | Density | Dictionary |  | density |  |  |  |
| withPlanningSchema | Within Planning Scheme | boolean |  |  |  |  |  |
| tpsNoptaNumber | TPS-NOTPA Number | string(50) |  |  |  |  |  |
| maxAreaDevelopment | Maximum Area Developable | double |  |  |  |  |  |
| topography | Topography | Dictionary |  | topography |  |  |  |
| soliCondition | Soil condition | Dictionary |  | soil_condition |  |  |  |
| accessRoadStatus | Access Road Status | Dictionary |  | access_road_status |  |  |  |
| hierachyRoad | Hierarchy Road | string(50) |  |  |  |  |  |
| federalRoadAccess | Federal Road Access | boolean |  |  |  |  |  |
| accessRoadWidth | Access Road Width | double |  |  |  |  |  |
| accessRoadTerminate | Access Road Terminate at Plot | boolean |  |  |  |  |  |
| developmentHinderFutureDevelopment | Development hinder future Development | boolean |  |  |  |  |  |
| proximityToSchool | Proximity to School | double |  |  | n3 |  |  |
| proximityToHospital | Proximity to Hospital | double |  |  | n3 |  |  |
| proximityToShop | Proximity to Shopping Centre | double |  |  | n3 |  |  |
| proximityToWorship | Proximity to Place of Worship | double |  |  | n3 |  |  |
| proximityToSportarena | Proximity to Sport Arena | double |  |  | n3 |  |  |
| proximityToOffice | Proximity to Office | double |  |  | n3 |  |  |
| proximityToIndustry | Proximity to Industry | double |  |  | n3 |  |  |
| proximityToOther | Proximity to Other | double |  |  | n3 |  |  |
| proximityComment | Proximity Comment | string(800) |  |  |  |  |  |
| proximityToUtility | Proximity to Utility Line | double |  |  | n3 |  |  |
| proximityToElectricity | Proximity to High Tension Electricity Line | double |  |  | n3 |  |  |
| capacity | Capacity | double |  |  | n3 |  |  |
| proximityToWater | Proximity to Water Main | double |  |  | n3 |  |  |
| proximityToNnpc | Proximity to N.N.P.C Pipe Line | double |  |  | n3 |  |  |
| proximityToTelephone | Proximity to Telephone Line | double |  |  | n3 |  |  |
| proximityFurtherComment | Proximity Further Comment | string(800) |  |  |  |  |  |
| distanceToLake | Distance to Special Feature (Lake) | double |  |  | n3 |  |  |
| distanceToMountain | Distance to Special Feature (Mountain) | double |  |  | n3 |  |  |
| distanceToQuarry | Distance to Special Feature (Quarry Site) | double |  |  | n3 |  |  |
| distanceToRiver | Distance to Special Feature (River) | double |  |  | n3 |  |  |
| distanceToGully | Distance to Special Feature (Gully) | double |  |  | n3 |  |  |
| distanceToRailway | Distance to Special Feature (Railway Lines) | double |  |  | n3 |  |  |
| distanceToExpressway | Distance to Special Feature (Expressway) | double |  |  | n3 |  |  |
| distanceToOthers | Distance to Special Feature (Others) | double |  |  | n3 |  |  |
| distanceToFeatureComment | Distance to Special Feature (Comment) | string(800) |  |  |  |  |  |
| setbackBuilding1 | Setback Building 1 | double |  |  | n3 |  |  |
| setbackBuilding1LandUse | Setback Building 1 Land Use Category | Dictionary |  | actual_use_type |  |  |  |
| setbackBuilding2 | Setback Building 2 | double |  |  | n3 |  |  |
| setbackBuilding2LandUse | Setback Building 2 Land Use Category | Dictionary |  | actual_use_type |  |  |  |
| setbackBuilding3 | Setback Building 3 | double |  |  | n3 |  |  |
| setbackBuilding3LandUse | Setback Building 3 Land Use Category | Dictionary |  | actual_use_type |  |  |  |
| setbackBuilding4 | Setback Building 4 | double |  |  | n3 |  |  |
| setbackBuilding4LandUse | Setback Building 4 Land Use Category | Dictionary |  | actual_use_type |  |  |  |
| setbackAccessRoad | Setback Access Road | double |  |  | n3 |  |  |
| setbackOtherComment | Setback Other Comment | string(800) |  |  |  |  |  |
| householdUtilityAccess | Household Utility lines accessible | boolean |  |  |  |  |  |
| conformityOfDevelopment | Conformity of Development | string(50) |  |  |  |  |  |
| fencingEncourage | Fencing Encourage | boolean |  |  |  |  |  |
| fencingType | Fencing Type | string(50) |  |  |  |  |  |
| setbackAccessRoad | Setback Access Road | double |  |  | n3 | all |  |

#### SiteAnalysisReadOnlyForm  (class SiteAnalysis; 53 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| neighbourhood | Neighbourhood | string(50) |  |  |  | all |  |
| maxHighAllow | Maximum Height Allowable | Dictionary |  | maximum_height_allowable |  | all |  |
| developmentStatus | Development Status | Dictionary |  | development_status |  | all |  |
| density | Density | Dictionary |  | density |  | all |  |
| withPlanningSchema | Within Planning Scheme | boolean |  |  |  | all |  |
| tpsNoptaNumber | TPS-NOTPA Number | string(50) |  |  |  | all |  |
| maxAreaDevelopment | Maximum Area Developable | double |  |  |  | all |  |
| topography | Topography | Dictionary |  | topography |  | all |  |
| soliCondition | Soil condition | Dictionary |  | soil_condition |  | all |  |
| accessRoadStatus | Access Road Status | Dictionary |  | access_road_status |  | all |  |
| hierachyRoad | Hierarchy Road | string(50) |  |  |  | all |  |
| federalRoadAccess | Federal Road Access | boolean |  |  |  | all |  |
| accessRoadWidth | Access Road Width | double |  |  |  | all |  |
| accessRoadTerminate | Access Road Terminate at Plot | boolean |  |  |  | all |  |
| developmentHinderFutureDevelopment | Development hinder future Development | boolean |  |  |  | all |  |
| proximityToSchool | Proximity to School | double |  |  | n3 | all |  |
| proximityToHospital | Proximity to Hospital | double |  |  | n3 | all |  |
| proximityToShop | Proximity to Shopping Centre | double |  |  | n3 | all |  |
| proximityToWorship | Proximity to Place of Worship | double |  |  | n3 | all |  |
| proximityToSportarena | Proximity to Sport Arena | double |  |  | n3 | all |  |
| proximityToOffice | Proximity to Office | double |  |  | n3 | all |  |
| proximityToIndustry | Proximity to Industry | double |  |  | n3 | all |  |
| proximityToOther | Proximity to Other | double |  |  | n3 | all |  |
| proximityComment | Proximity Comment | string(800) |  |  |  | all |  |
| proximityToUtility | Proximity to Utility Line | double |  |  | n3 | all |  |
| proximityToElectricity | Proximity to High Tension Electricity Line | double |  |  | n3 | all |  |
| capacity | Capacity | double |  |  | n3 | all |  |
| proximityToWater | Proximity to Water Main | double |  |  | n3 | all |  |
| proximityToNnpc | Proximity to N.N.P.C Pipe Line | double |  |  | n3 | all |  |
| proximityToTelephone | Proximity to Telephone Line | double |  |  | n3 | all |  |
| proximityFurtherComment | Proximity Further Comment | string(800) |  |  |  | all |  |
| distanceToLake | Distance to Special Feature (Lake) | double |  |  | n3 | all |  |
| distanceToMountain | Distance to Special Feature (Mountain) | double |  |  | n3 | all |  |
| distanceToQuarry | Distance to Special Feature (Quarry Site) | double |  |  | n3 | all |  |
| distanceToRiver | Distance to Special Feature (River) | double |  |  | n3 | all |  |
| distanceToGully | Distance to Special Feature (Gully) | double |  |  | n3 | all |  |
| distanceToRailway | Distance to Special Feature (Railway Lines) | double |  |  | n3 | all |  |
| distanceToExpressway | Distance to Special Feature (Expressway) | double |  |  | n3 | all |  |
| distanceToOthers | Distance to Special Feature (Others) | double |  |  | n3 | all |  |
| distanceToFeatureComment | Distance to Special Feature (Comment) | string(800) |  |  |  | all |  |
| setbackBuilding1 | Setback Building 1 | double |  |  | n3 | all |  |
| setbackBuilding1LandUse | Setback Building 1 Land Use Category | Dictionary |  | actual_use_type |  | all |  |
| setbackBuilding2 | Setback Building 2 | double |  |  | n3 | all |  |
| setbackBuilding2LandUse | Setback Building 2 Land Use Category | Dictionary |  | actual_use_type |  | all |  |
| setbackBuilding3 | Setback Building 3 | double |  |  | n3 | all |  |
| setbackBuilding3LandUse | Setback Building 3 Land Use Category | Dictionary |  | actual_use_type |  | all |  |
| setbackBuilding4 | Setback Building 4 | double |  |  | n3 | all |  |
| setbackBuilding4LandUse | Setback Building 4 Land Use Category | Dictionary |  | actual_use_type |  | all |  |
| setbackOtherComment | Setback Other Comment | string(800) |  |  |  | all |  |
| householdUtilityAccess | Household Utility lines accessible | boolean |  |  |  | all |  |
| conformityOfDevelopment | Conformity of Development | string(50) |  |  |  | all |  |
| fencingEncourage | Fencing Encourage | boolean |  |  |  | all |  |
| fencingType | Fencing Type | string(50) |  |  |  | all |  |

#### SupportedDocForm  (class IncomingDocument; 7 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| documentType | Document Type | Dictionary | Y | document_type |  |  |  |
| documentNumber | Document Number | string(50) |  |  |  |  |  |
| submission | Date Issued | date |  |  | dd/MM/yyyy |  |  |
| establishRight | Primary Document | boolean |  |  |  |  |  |
| issuedByName | Issued By | Dictionary |  | issued_by |  |  |  |
| propertyFilePage | Property File Page | string(50) |  |  |  |  |  |
| comments | Description | string(800) |  |  |  |  |  |

#### SupportedDocFormOut  (class IncomingDocument; 7 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| documentType | Document | Dictionary | Y | outdoc_type |  |  |  |
| documentNumber | Document No. | string(50) |  |  |  |  |  |
| submission | Date Issued | date |  |  | dd/MM/yyyy |  |  |
| issuedBy | Issued By | string(50) |  |  |  |  |  |
| establishRight | Primary Document | boolean |  |  |  |  |  |
| duplicate | Duplicate | boolean |  |  |  |  |  |
| comments | Description | string(800) |  |  |  |  |  |

#### SupportedDocGrid  (class IncomingDocument; 4 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| documentType | Document Type | Dictionary |  | document_type |  | all |  |
| documentNumber | Document No. | string(50) |  |  |  | all |  |
| submission | Date Issued | date |  |  | dd/MM/yyyy | all |  |
| issuedBy | Issued By | string(50) |  |  |  | all |  |

#### SurveyPlanForm  (class SurveyPlan; 25 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| surveyType | Survey Plan Type | Dictionary | Y | survey_plan_type |  |  |  |
| plotsAmount | Plots Amount | int | Y |  |  |  | GE 0 |
| terrain_Type | Terrain Type | Dictionary | Y | terrain_Type |  |  |  |
| surveyor_Type | Surveyor Type | Dictionary | Y | surveyor_type |  |  |  |
| planNumber | Plan Number | string(40) |  |  |  | all |  |
| localGovernmentArea | Local Government Area | Dictionary | Y | region_lga |  |  |  |
| zone | Location | Dictionary | Y | location |  |  |  |
| layout | Layout Name | Dictionary |  | layout_type |  |  |  |
| city | Town | string(50) |  |  |  |  |  |
| districtText | District | string(100) |  |  |  |  |  |
| cell | Village | string(100) |  |  |  |  |  |
| ward | Ward | string(100) |  |  |  |  |  |
| region | State | Dictionary | Y | region |  |  |  |
| country | Country | Dictionary | Y | country |  |  |  |
| surveyorPlanNo | Surveyor Plan Number | string(50) | Y |  |  |  |  |
| area | Area | double | Y |  | n3 |  | GE 0 |
| areaUnitType | Measurement Unit | Dictionary | Y | area_measurement_unit |  |  |  |
| surveyStartDate | Survey Start Date | date | Y |  | dd/MM/yyyy |  | LE today&LE surveyEndDate |
| surveyEndDate | Survey End Date | date | Y |  | dd/MM/yyyy |  | LE today&GE surveyStartDate |
| surveyorName | Surveyor Name | string(32) | Y |  |  |  |  |
| tracingNumber | Tracing Number | string(32) |  |  |  |  |  |
| beaconAmountReEstablish | Beacon Amount (Re-Establish) | int |  |  |  |  | GE 0 |
| beaconAmountPerimeter | Beacon Amount (Perimeter) | int |  |  |  |  | GE 0 |
| approvalDate | Approval Date | date |  |  | dd/MM/yyyy | all | GE today |
| comments | Comment | string(512) |  |  |  |  |  |

#### SurveyPlanGrid  (class SurveyPlan; 10 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| surveyorPlanNo | Surveyor Plan Number | string(50) |  |  |  |  |  |
| surveyor_Type | Surveyor Type | Dictionary | Y | surveyor_type |  |  |  |
| localGovernmentArea | Local Government Area | Dictionary | Y | region_lga |  |  |  |
| zone | Location | Dictionary | Y | location |  |  |  |
| layout | Layout Name | Dictionary |  | layout_type |  |  |  |
| plotsAmount | Plots Amount | int | Y |  |  |  |  |
| tracingNumber | Tracing Number | string(32) |  |  |  |  |  |
| area | Area | double | Y |  | n3 |  |  |
| areaUnitType | Measurement Unit | Dictionary | Y | area_measurement_unit |  |  |  |
| planNumber | Plan Number | string(40) |  |  |  |  |  |

#### SurveyPlanGridLWA  (class SurveyPlan; 12 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| surveyorPlanNo | Surveyor Plan Number | string(50) |  |  |  |  |  |
| surveyor_Type | Surveyor Type | Dictionary | Y | surveyor_type |  |  |  |
| localGovernmentArea | Local Government Area | Dictionary | Y | region_lga |  |  |  |
| zone | Location | Dictionary | Y | location |  |  |  |
| layout | Layout Name | Dictionary |  | layout_type |  |  |  |
| plotsAmount | Plots Amount | int | Y |  |  |  |  |
| tracingNumber | Tracing Number | string(32) |  |  |  |  |  |
| area | Area | double | Y |  | n3 |  |  |
| areaUnitType | Measurement Unit | Dictionary | Y | area_measurement_unit |  |  |  |
| planNumber | Plan Number | string(40) |  |  |  |  |  |
| surveyType | Survey Plan Type | Dictionary | Y | survey_plan_type |  |  |  |
| surveyorName | Surveyor Name | string(32) | Y |  |  |  |  |

#### TaskFilterForm  (class TaskFilter; 8 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| transactionNumber | Transaction No. | string |  |  |  |  |  |
| transactionType | Transaction | TransMetaInfo |  | transaction_type |  |  | if Settings.config.HideNormalTransactionCode |
| applicantName | Applicant Name | string |  |  |  |  |  |
| sourceRids | Source Property Number | string |  |  |  |  |  |
| targetRids | Target Property Number | string |  |  |  |  |  |
| createDate | Submission Date | date |  |  | dd/MM/yyyy hh:mm tt |  |  |
| taskDescription | Workflow Stage | string |  |  |  |  |  |
| taskActorName | Assigned To | Authority |  | authority |  |  | if !=ALFRESCO_ADMINISTRATORS,!=EMAIL_CONTRIBUTORS,!=guest,!=webclient |

#### TransactionConsiderationForm  (class Transaction; 8 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| transactionNumber | Transaction Number | string |  |  |  | all |  |
| transactionType | Transaction Type | TransactionMetaData | Y | transaction_type |  | all |  |
| createDate | Transaction Start Date | date |  |  | dd/MM/yyyy hh:mm tt | all |  |
| applicationDate | Application Date | dateTime | Y |  | dd/MM/yyyy |  | LE today |
| sourcePropertyIDs | Property Number | string(250) | Y |  |  |  |  |
| tempConsideration | Consideration | string(250) | Y |  | n2 |  |  |
| considerationCurrency | Consideration Currency | Dictionary | Y | currency |  |  |  |
| comments | Comment | string(800) |  |  |  |  |  |

#### TransactionConsiderationROForm  (class Transaction; 8 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| transactionNumber | Transaction Number | string |  |  |  | all |  |
| transactionType | Transaction Type | TransactionMetaData | Y | transaction_type |  | all |  |
| createDate | Transaction Start Date | date |  |  | dd/MM/yyyy hh:mm tt | all |  |
| applicationDate | Application Date | dateTime | Y |  | dd/MM/yyyy |  | LE today |
| sourcePropertyIDs | Property Number | string(250) | Y |  |  |  |  |
| tempConsideration | Consideration | string(250) |  |  | n2 | all |  |
| considerationCurrency | Consideration Currency | Dictionary |  | currency |  | all |  |
| comments | Comment | string(800) |  |  |  |  |  |

#### TransactionDCTForm  (class Transaction; 4 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| transactionNumber | Transaction Number | string |  |  |  | all |  |
| transactionType | Transaction Type | TransactionMetaData | Y | transaction_type |  | all |  |
| createDate | Transaction Start Date | date |  |  | dd/MM/yyyy hh:mm tt | all |  |
| comments | Comment | string(800) |  |  |  |  |  |

#### TransactionFRGForm  (class Transaction; 5 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| transactionNumber | Transaction Number | string |  |  |  | all |  |
| transactionType | Transaction Type | TransactionMetaData | Y | transaction_type |  | all |  |
| createDate | Transaction Start Date | date |  |  | dd/MM/yyyy hh:mm tt | all |  |
| applicationDate | Application Date | dateTime | Y |  | dd/MM/yyyy |  | LE today |
| comments | Comment | string(800) |  |  |  |  |  |

#### TransactionForm  (class Transaction; 8 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| transactionNumber | Transaction No. | string |  |  |  | all |  |
| transactionType | Transaction | TransactionMetaData | Y | transaction_type |  | all |  |
| tenureType | Tenure Type | Dictionary(250) | Y | tenure_type |  |  |  |
| oldPropertyId | Old File No. | string(50) |  |  |  |  |  |
| isSearchOldfile | Search By Certificate No. | boolean |  |  |  |  |  |
| certNos | Certificate No. | string(50) | Y |  |  |  |  |
| sourcePropertyIDs | File No. | string(250) | Y |  |  |  |  |
| comments | Comments | string(800) |  |  |  |  |  |

#### TransactionFormAPRF  (class Transaction; 6 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| transactionNumber | Transaction Number | string |  |  |  | all |  |
| transactionType | Transaction Type | TransactionMetaData | Y | transaction_type |  | all |  |
| createDate | Transaction Start Date | date |  |  | dd/MM/yyyy hh:mm tt | all |  |
| applicationDate | Application Date | dateTime | Y |  | dd/MM/yyyy |  | LE today |
| sourcePropertyIDs | Property Number | string(250) | Y |  |  |  |  |
| comments | Comment | string(800) |  |  |  |  |  |

#### TransactionFormINSC  (class Transaction; 7 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| transactionNumber | Transaction Number | string |  |  |  | all |  |
| transactionType | Transaction Type | TransactionMetaData | Y | transaction_type |  | all |  |
| createDate | Transaction Start Date | date |  |  | dd/MM/yyyy hh:mm tt | all |  |
| tranDict1 | Property Registration Type | Dictionary | Y | property_reg |  |  |  |
| sourcePropertyIDs | Source Property Number | string(250) | Y |  |  |  |  |
| targetPropertyIDs | Target Property Number | string(250) | Y |  |  |  |  |
| comments | Comment | string(800) |  |  |  |  |  |

#### TransactionFormINSCA  (class Transaction; 8 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| transactionType | Transaction | TransactionMetaData | Y | transaction_type |  | all |  |
| transactionNumber | Transaction No. | string |  |  |  | all |  |
| sourcePropertyIDs | System Property No. | string(250) |  |  |  | all |  |
| region | Province | Dictionary | Y | region |  |  |  |
| localGovernmentArea | Local Government Area | Dictionary | Y | localGovernmentArea |  |  |  |
| block | Block No. | string(50) | Y |  | [0-9A-Za-z]{0,50} |  |  |
| plot | Plot No. | string(50) | Y |  | [0-9A-Za-z:]{0,50} |  |  |
| comments | Comments | string(800) |  |  |  |  |  |

#### TransactionFormLWA  (class Transaction; 16 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| transactionNumber | Transaction Number | string |  |  |  | all |  |
| transactionType | Transaction Type | TransactionMetaData | Y | transaction_type |  |  |  |
| transactionStatus | Status | Dictionary | Y | transaction_status |  |  |  |
| createDate | Transaction Start Date | dateTime |  |  | dd/MM/yyyy hh:mm tt |  |  |
| applicationDate | Application Date | date |  |  | dd/MM/yyyy |  |  |
| completeDate | Transaction End Date | date |  |  | dd/MM/yyyy |  |  |
| relatedNumber | Related Transaction | string(50) |  |  |  |  |  |
| relatedInstrNumber | Related Instrument Number | string(50) |  |  |  |  |  |
| relatedVolume | Related Volume Number | string(50) |  |  |  |  |  |
| relatedPageNumber | Related Page Number | string(50) |  |  |  |  |  |
| signDate | Execution Date | dateTime | Y |  | dd/MM/yyyy |  | LE today |
| registrationDate | Registration Date | dateTime | Y |  | dd/MM/yyyy hh:mm tt |  | LE today |
| recordingno | Instrument Number | string(30) |  |  |  | all |  |
| volume | Volume | string(20) |  |  |  |  |  |
| folio | Page Number | string(20) |  |  |  |  |  |
| comments | Comments | string(800) |  |  |  |  |  |

#### TransactionFormSearchLWA  (class Transaction; 21 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| transactionNumber | Transaction Number | string |  |  |  | all |  |
| transactionType | Transaction Type | TransactionMetaData | Y | transaction_type |  |  |  |
| region | State | Dictionary | Y | region |  |  |  |
| localGovernmentArea | Local Government Area | Dictionary | Y | local_government_area |  |  |  |
| volume | Volume | string(20) | Y |  |  |  |  |
| folio | Page Number | string(20) | Y |  |  |  |  |
| createDate | Transaction Start Date | dateTime |  |  | dd/MM/yyyy hh:mm tt |  |  |
| applicationDate | Application Date | dateTime |  |  | dd/MM/yyyy |  |  |
| registrationDate | Registration Date | dateTime | Y |  | dd/MM/yyyy hh:mm tt |  | LE today |
| startDate | Date of Commencement | date | Y |  | dd/MM/yyyy |  | LE today |
| expirationDate | Expiration Date | date |  |  | dd/MM/yyyy |  |  |
| sourcePropertyIDs | Property Number | string(250) |  |  |  |  |  |
| oldPropertyId | Old File No. | string(50) |  |  |  |  |  |
| relatedInstrNumber | Related Instrument Number | string(50) |  |  |  |  |  |
| relatedPageNumber | Related Page Number | string(50) | Y |  |  | APSL,APRO,CONT |  |
| relatedVolume | Related Volume Number | string(50) | Y |  |  | APSL,APRO,CONT |  |
| certNos | Certificate No. | string(255) |  |  |  |  |  |
| recordingno | Instrument Number | string(30) |  |  |  |  |  |
| isInactive | Cancelled/Inactive Instrument | boolean |  |  |  |  |  |
| transactionStatus | Status | Dictionary | Y | transaction_status |  |  |  |
| transactionSource | Created as | Dictionary | Y | record_source |  |  |  |

#### TransactionFormSmall  (class Transaction; 6 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| transactionType | Transaction Type | TransactionMetaData | Y | transaction_type |  | all |  |
| transactionNumber | Transaction No. | string(50) | Y |  |  | all |  |
| registrationDate | Registration Date | date | Y |  | dd/MM/yyyy |  |  |
| createDate | Transaction Start Date | dateTime |  |  | dd/MM/yyyy hh:mm tt | all |  |
| sourcePropertyIDs | Property No. | string(250) |  |  |  | all |  |
| comments | Comment | string(800) |  |  |  |  |  |

#### TransactionGRPMForm  (class Transaction; 6 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| transactionNumber | Transaction Number | string |  |  |  | all |  |
| transactionType | Transaction Type | TransactionMetaData | Y | transaction_type |  | all |  |
| createDate | Transaction Start Date | date |  |  | dd/MM/yyyy hh:mm tt | all |  |
| applicationDate | Application Date | dateTime | Y |  | dd/MM/yyyy |  | LE today |
| sourcePropertyIDs | Property Number | string(250) | Y |  |  |  |  |
| comments | Comment | string(800) |  |  |  |  |  |

#### TransactionGridLWA  (class Transaction; 13 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| transactionNumber | Transaction Number | string |  |  |  | all |  |
| transactionType | Transaction Type | TransactionMetaData | Y | transaction_type |  |  |  |
| sourcePropertyIDs | Property Number | string |  |  |  |  |  |
| recordingno | Instrument Number | string(30) |  |  |  |  |  |
| oldPropertyId | Old File No. | string(50) |  |  |  | all |  |
| applicationDate | Application Date | date |  |  | dd/MM/yyyy |  | LE today |
| applicationDate | Application Date | date |  |  | dd/MM/yyyy |  | LE today |
| applicationDate | Application Date | date |  |  | dd/MM/yyyy |  |  |
| completeDate | Transaction End Date | date |  |  | dd/MM/yyyy |  |  |
| recordingno | Instrument Number | string(30) |  |  |  |  |  |
| volume | Volume | string(20) |  |  |  |  |  |
| folio | Page Number | string(20) |  |  |  |  |  |
| transactionStatus | Status | Dictionary | Y | transaction_status |  |  |  |

#### TransactionGridWithPropertySourceTargetLWA  (class Transaction; 14 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| transactionNumber | Transaction Number | string |  |  |  | all |  |
| transactionType | Transaction Type | TransactionMetaData | Y | transaction_type |  |  |  |
| sourcePropertyIDs | Property Number | string |  |  |  |  |  |
| recordingno | Instrument Number | string(30) |  |  |  |  |  |
| oldPropertyId | Old File No. | string(50) |  |  |  | all |  |
| applicationDate | Application Date | dateTime |  |  | dd/MM/yyyy |  | LE today |
| applicationDate | Application Date | dateTime |  |  | dd/MM/yyyy |  | LE today |
| applicationDate | Application Date | dateTime |  |  | dd/MM/yyyy |  |  |
| completeDate | Transaction End Date | date |  |  | dd/MM/yyyy |  |  |
| recordingno | Instrument Number | string(30) |  |  |  |  |  |
| volume | Volume | string(20) |  |  |  |  |  |
| folio | Page Number | string(20) |  |  |  |  |  |
| transactionStatus | Status | Dictionary | Y | transaction_status |  |  |  |
| sourceTarget | Source/Target | string(0) |  |  |  |  |  |

#### TransactionGuestLWA  (class Transaction; 2 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| transactionType | Transaction Type | TransactionMetaData | Y | transaction_type |  | all |  |
| transactionNumber | Transaction Number | string | Y |  |  | all |  |

#### TransactionIndexForm  (class Transaction; 8 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| transactionType | Transaction | TransactionMetaData | Y | transaction_type |  | all |  |
| transactionNumber | Transaction No. | string(50) | Y |  |  | all |  |
| createDate | Submission Date | dateTime |  |  | dd/MM/yyyy hh:mm tt | all |  |
| tenureType | Tenure Type | Dictionary(250) | Y | tenure_type |  | all |  |
| sourcePropertyIDs | File No. | string(250) |  |  |  | all |  |
| oldPropertyId | Old File No. | string(50) |  |  |  | all |  |
| balanceDate | Last GR Payment Date | date |  |  | dd/MM/yyyy |  | LE today |
| comments | Comments | string(800) |  |  |  |  |  |

#### TransactionIndexFormAPRF  (class Transaction; 8 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| transactionType | Transaction | TransactionMetaData | Y | transaction_type |  | all |  |
| transactionNumber | Transaction No. | string(50) | Y |  |  | all |  |
| createDate | Submission Date | dateTime |  |  | dd/MM/yyyy hh:mm tt | all |  |
| tenureType | Tenure Type | Dictionary(250) | Y | tenure_type |  | all |  |
| sourcePropertyIDs | File No. | string(250) |  |  |  | all |  |
| oldPropertyId | Old File No. | string(50) | Y |  |  |  |  |
| balanceDate | Last GR Payment Date | date |  |  | dd/MM/yyyy |  | LE today |
| comments | Comments | string(800) |  |  |  |  |  |

#### TransactionIndexFormINSC  (class Transaction; 5 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| transactionNumber | Transaction Number | string(50) | Y |  |  | all |  |
| transactionType | Transaction Type | TransactionMetaData | Y | transaction_type |  | all |  |
| createDate | Transaction Start Date | dateTime |  |  | dd/MM/yyyy hh:mm tt | all |  |
| sourcePropertyIDs | Property Number | string(50) |  |  |  | all |  |
| comments | Comment | string(800) |  |  |  |  |  |

#### TransactionIndexFormWithReadOnlyRegisDate  (class Transaction; 13 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| transactionType | Transaction | TransactionMetaData | Y | transaction_type |  | all |  |
| transactionNumber | Transaction No. | string(50) | Y |  |  | all |  |
| sourcePropertyIDs | System Property No. | string(250) |  |  |  | all |  |
| createDate | Transaction Creation Date | dateTime |  |  | dd/MM/yyyy | all |  |
| region | Province | Dictionary |  | region |  |  |  |
| localGovernmentArea | Local Government Area | Dictionary |  | local_government_area |  |  |  |
| registrationDate | Registration Date | date | Y |  | dd/MM/yyyy | all | LE today |
| consideration | Amount | double |  |  | c2 |  | GE 0 |
| generalNumber | Deed No. | string(50) | Y |  |  |  |  |
| serialNumber | Serial No. | string(5) | Y |  |  |  |  |
| volume | Book No. | string(3) | Y |  |  |  |  |
| folio | Page No. | long | Y |  | [0-9]{1,3} |  | GT 0&LE 999 |
| comments | Comments | string(800) |  |  |  |  |  |

#### TransactionIndexFormWithoutRegistrationDate  (class Transaction; 11 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| transactionType | Transaction | TransactionMetaData | Y | transaction_type |  | all |  |
| transactionNumber | Transaction No. | string(50) | Y |  |  | all |  |
| sourcePropertyIDs | System Property No. | string(250) |  |  |  | all |  |
| createDate | Transaction Creation Date | dateTime |  |  | dd/MM/yyyy | all |  |
| region | Province | Dictionary |  | region |  |  |  |
| localGovernmentArea | Local Government Area | Dictionary |  | local_government_area |  |  |  |
| generalNumber | Deed No. | string(50) | Y |  |  |  |  |
| serialNumber | Serial No. | string(5) | Y |  |  |  |  |
| volume | Book No. | string(3) | Y |  |  |  |  |
| folio | Page No. | long | Y |  | [0-9]{1,3} |  | GT 0&LE 999 |
| comments | Comments | string(800) |  |  |  |  |  |

#### TransactionInfoForm  (class Transaction; 16 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| transactionNumber | Transaction Number | string(50) |  |  |  | all |  |
| transactionType | Transaction Type | TransactionMetaData |  | transaction_type |  |  | if !=INSC |
| createDate | Transaction Start Date | dateTime |  |  | dd/MM/yyyy hh:mm tt |  | LE today |
| completeDate | Transaction End Date | dateTime |  |  | dd/MM/yyyy hh:mm tt |  | LE today |
| applicationDate | Application Date | dateTime |  |  | dd/MM/yyyy |  | LE today |
| sourcePropertyIDs | Property Number | string(250) |  |  |  | all |  |
| relatedInstrNumber | Related Instrument Number | string(50) |  |  |  | APSL,APRO,CONT |  |
| relatedPageNumber | Related Page Number | string(50) |  |  |  | APSL,APRO,CONT |  |
| relatedVolume | Related Volume Number | string(50) |  |  |  | APSL,APRO,CONT |  |
| relatedNumber | Related Transaction | string(50) |  |  |  | APSL,APRO,CONT |  |
| recordingno | Instrument Number | string(30) |  |  |  |  |  |
| folio | Page Number | string(20) |  |  |  |  |  |
| volume | Volume | string(20) |  |  |  |  |  |
| signDate | Execution Date | dateTime |  |  | dd/MM/yyyy |  | LE today |
| registrationDate | Registration Date | dateTime |  |  | dd/MM/yyyy hh:mm tt |  | LE today |
| comments | Comment | string(800) |  |  |  |  |  |

#### TransactionListGrid  (class Transaction; 3 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| transactionNumber | Transaction Number | string |  |  |  |  |  |
| transactionType | Transaction Type | TransactionMetaData |  | transaction_type |  |  |  |
| sourcePropertyIDs | Property Number | string |  |  |  |  |  |

#### TransactionNoteForm  (class TransactionNote; 4 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| userAuthority | User | Authority |  | authority |  | all |  |
| createDate | Date/Time | dateTime |  |  | g | all |  |
| noteType | Type | Dictionary | Y | task_event_access |  |  |  |
| comments | Comments | string(800) | Y |  |  |  |  |

#### TransactionNoteGrid  (class TransactionNote; 5 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| comments | Comments | string(800) |  |  |  |  |  |
| createDate | Date/Time | dateTime |  |  | g |  |  |
| userAuthority | User | Authority |  | authority |  |  |  |
| taskName | Task | string(800) |  |  |  |  |  |
| noteType | Type | Dictionary |  | task_event_access |  |  |  |

#### TransactionNoteSimpleForm  (class TransactionNote; 4 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| userAuthority | User | Authority |  | authority |  | all |  |
| createDate | Date/Time | dateTime |  |  | g | all |  |
| noteType | Type | Dictionary | Y | task_event_access |  |  |  |
| comments | Comments | string(800) |  |  |  |  |  |

#### TransactionREGRForm  (class Transaction; 6 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| transactionNumber | Transaction Number | string |  |  |  | all |  |
| transactionType | Transaction Type | TransactionMetaData | Y | transaction_type |  | all |  |
| createDate | Transaction Start Date | date |  |  | dd/MM/yyyy hh:mm tt | all |  |
| applicationDate | Application Date | dateTime | Y |  | dd/MM/yyyy |  | LE today |
| sourcePropertyIDs | Property Number | string(250) | Y |  |  |  |  |
| comments | Comment | string(800) |  |  |  |  |  |

#### TransactionRegisterInstrumentForm  (class Transaction; 4 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| transactionNumber | Transaction Number | string(50) |  |  |  | all |  |
| transactionType | Transaction Type | TransactionMetaData |  | transaction_type |  | all |  |
| sourcePropertyIDs | Property Number | string(250) |  |  |  | all |  |
| comments | Comment | string(800) |  |  |  |  |  |

#### TransactionSPCForm  (class Transaction; 6 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| transactionNumber | Transaction Number | string |  |  |  | all |  |
| transactionType | Transaction Type | TransactionMetaData | Y | transaction_type |  | all |  |
| createDate | Transaction Start Date | date |  |  | dd/MM/yyyy hh:mm tt | all |  |
| completeDate | Transaction End Date | date |  |  | dd/MM/yyyy hh:mm tt | all |  |
| applicationDate | Application Date | dateTime | Y |  | dd/MM/yyyy |  | LE today |
| comments | Comment | string(800) |  |  |  |  |  |

#### TransactionSSGForm  (class Transaction; 6 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| transactionNumber | Transaction Number | string |  |  |  | all |  |
| transactionType | Transaction Type | TransactionMetaData | Y | transaction_type |  | all |  |
| createDate | Transaction Start Date | date |  |  | dd/MM/yyyy hh:mm tt | all |  |
| applicationDate | Application Date | dateTime | Y |  | dd/MM/yyyy |  | LE today |
| sourcePropertyIDs | Property Number | string(250) | Y |  |  |  |  |
| comments | Comment | string(800) |  |  |  |  |  |

#### TransactionSearchForm  (class Transaction; 6 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| transactionType | Transaction Type | TransactionMetaData |  | transaction_type |  |  | if !=SPC,!=GRR |
| transactionNumber | Transaction Number | string(50) |  |  |  |  |  |
| sourcePropertyIDs | Property Number | string(250) |  |  |  |  |  |
| recordingno | Instrument Number | string(30) |  |  |  |  |  |
| folio | Page Number | string(20) |  |  |  |  |  |
| volume | Volume | string(20) |  |  |  |  |  |

#### TransactionSearchGrid  (class Transaction; 8 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| sourcePropertyIDs | Property Number | string(250) |  |  |  | all |  |
| transactionType | Transaction Type | TransactionMetaData |  | transaction_type |  | all |  |
| transactionNumber | Transaction Number | string(50) |  |  |  | all |  |
| signDate | Execution Date | dateTime |  |  | dd/MM/yyyy | all | LE today |
| registrationDate | Registration Date | dateTime | Y |  | dd/MM/yyyy hh:mm tt | all | LE today |
| recordingno | Instrument Number | string(30) |  |  |  | all |  |
| folio | Page Number | string(20) |  |  |  | all |  |
| volume | Volume | string(20) |  |  |  | all |  |

#### TransactionShortForm  (class Transaction; 7 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| transactionType | Transaction | TransactionMetaData | Y | transaction_type |  | all |  |
| transactionNumber | Transaction No. | string(50) | Y |  |  | all |  |
| createDate | Submission Date | dateTime |  |  | dd/MM/yyyy hh:mm tt | all |  |
| tenureType | Tenure Type | Dictionary(250) | Y | tenure_type |  | all |  |
| sourcePropertyIDs | File No. | string(250) |  |  |  | all |  |
| oldPropertyId | Old File No. | string(50) |  |  |  | all |  |
| comments | Comments | string(800) |  |  |  |  |  |

#### TransactionShortFormINSC  (class Transaction; 8 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| transactionType | Transaction | TransactionMetaData | Y | transaction_type |  | all |  |
| transactionNumber | Transaction No. | string(50) | Y |  |  | all |  |
| sourcePropertyIDs | System Property No. | string(250) |  |  |  | all |  |
| createDate | Transaction Creation Date | dateTime |  |  | dd/MM/yyyy hh:mm tt | all |  |
| registrationDate | Registration Date | date | Y |  | dd/MM/yyyy hh:mm tt | all | LE today |
| consideration | Amount | double |  |  | c2 |  |  |
| relatedNumber | Next Previous Deed | string |  |  |  | all |  |
| comments | Comments | string(800) |  |  |  |  |  |

#### TransactionStatusGrid  (class Transaction; 3 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| transactionType | Transaction | TransactionMetaData |  | transaction_type |  |  |  |
| transactionNumber | Transaction No. | string |  |  |  |  |  |
| transactionStatus | Transaction Status | Dictionary |  | transaction_status |  |  |  |

#### TransactionUPMOForm  (class Transaction; 8 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| transactionNumber | Transaction Number | string |  |  |  | all |  |
| transactionType | Transaction Type | TransactionMetaData | Y | transaction_type |  | all |  |
| createDate | Transaction Start Date | date |  |  | dd/MM/yyyy hh:mm tt | all |  |
| applicationDate | Application Date | dateTime | Y |  | dd/MM/yyyy |  | LE today |
| sourcePropertyIDs | Property Number | string(250) | Y |  |  |  |  |
| tempConsideration | Consideration | string(250) | Y |  | n2 |  |  |
| considerationCurrency | Consideration Currency | Dictionary | Y | currency |  |  |  |
| comments | Comment | string(800) |  |  |  |  |  |

#### TransactionWithDocDCTForm  (class Transaction; 15 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| transactionType | Transaction Type | TransactionMetaData |  | transaction_type |  | all |  |
| transactionNumber | Transaction Number | string(50) |  |  |  | all |  |
| createDate | Transaction Start Date | dateTime |  |  | dd/MM/yyyy hh:mm tt | all | LE today |
| completeDate | Transaction End Date | dateTime |  |  | dd/MM/yyyy hh:mm tt | all | LE today |
| applicationDate | Application Date | dateTime |  |  | dd/MM/yyyy | all | LE today |
| sourcePropertyIDs | Property Number | string(250) |  |  |  | all |  |
| recordingno | Instrument Number | string(30) |  |  |  | all |  |
| folio | Page Number | string(20) |  |  |  | all |  |
| volume | Volume | string(20) |  |  |  | all |  |
| relatedInstrNumber | Related Instrument Number | string(50) |  |  |  | all |  |
| relatedPageNumber | Related Page Number | string(50) |  |  |  | all |  |
| relatedVolume | Related Volume Number | string(50) |  |  |  | all |  |
| signDate | Execution Date | dateTime |  |  | dd/MM/yyyy | all | LE today |
| registrationDate | Registration Date | dateTime |  |  | dd/MM/yyyy hh:mm tt | all | LE today |
| comments | Comment | string(800) |  |  |  | all |  |

#### TransactionWithDocDCTModifyForm  (class Transaction; 15 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| dataCorrectedTransactionType | Transaction Type | TransactionMetaData |  | transaction_type |  | all |  |
| dataCorrectedTransactionNumber | Transaction Number | string(50) |  |  |  | all |  |
| dataCorrectedCreateDate | Transaction Start Date | dateTime |  |  | dd/MM/yyyy hh:mm tt | all | LE today |
| dataCorrectedCompleteDate | Transaction End Date | dateTime |  |  | dd/MM/yyyy hh:mm tt | all | LE today |
| applicationDate | Application Date | dateTime |  |  | dd/MM/yyyy | all | LE today |
| sourcePropertyIDs | Property Number | string(250) |  |  |  | all |  |
| recordingno | Instrument Number | string(30) |  |  |  | all |  |
| folio | Page Number | string(20) |  |  |  | all |  |
| volume | Volume | string(20) |  |  |  | all |  |
| relatedInstrNumber | Related Instrument Number | string(50) |  |  |  | all |  |
| relatedPageNumber | Related Page Number | string(50) |  |  |  | all |  |
| relatedVolume | Related Volume Number | string(50) |  |  |  | all |  |
| signDate | Execution Date | dateTime |  |  | dd/MM/yyyy | all | LE today |
| registrationDate | Registration Date | dateTime |  |  | dd/MM/yyyy hh:mm tt | all | LE today |
| dataCorrectedComments | Comment | string(800) |  |  |  | all |  |

#### TransactionWithDocFRGForm  (class Transaction; 15 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| transactionNumber | Transaction Number | string(50) |  |  |  | all |  |
| transactionType | Transaction Type | TransactionMetaData |  | transaction_type |  | all |  |
| createDate | Transaction Start Date | dateTime |  |  | dd/MM/yyyy hh:mm tt | all | LE today |
| applicationDate | Application Date | dateTime |  |  | dd/MM/yyyy | all | LE today |
| sourcePropertyIDs | Property Number | string(250) |  |  |  | all |  |
| relatedInstrNumber | Related Instrument Number | string(50) | Y |  |  | all |  |
| relatedPageNumber | Related Page Number | string(50) | Y |  |  | all |  |
| relatedVolume | Related Volume Number | string(50) | Y |  |  | all |  |
| relatedNumber | Related Transaction | string(50) | Y |  |  | all |  |
| signDate | Execution Date | dateTime |  |  | dd/MM/yyyy | all | LE today |
| registrationDate | Registration Date | dateTime | Y |  | dd/MM/yyyy hh:mm tt | all | LE today |
| recordingno | Instrument Number | string(30) |  |  |  | all |  |
| folio | Page Number | string(20) |  |  |  | all |  |
| volume | Volume | string(20) |  |  |  | all |  |
| comments | Comment | string(800) |  |  |  |  |  |

#### TransactionWithDocFormAPRF  (class Transaction; 15 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| transactionNumber | Transaction Number | string(50) |  |  |  | all |  |
| transactionType | Transaction Type | TransactionMetaData |  | transaction_type |  | all |  |
| createDate | Transaction Start Date | dateTime |  |  | dd/MM/yyyy hh:mm tt | all | LE today |
| applicationDate | Application Date | dateTime |  |  | dd/MM/yyyy | all | LE today |
| sourcePropertyIDs | Property Number | string(250) |  |  |  | all |  |
| relatedInstrNumber | Related Instrument Number | string(50) | Y |  |  | all |  |
| relatedPageNumber | Related Page Number | string(50) | Y |  |  | all |  |
| relatedVolume | Related Volume Number | string(50) | Y |  |  | all |  |
| relatedNumber | Related Transaction | string(50) | Y |  |  | all |  |
| signDate | Execution Date | dateTime |  |  | dd/MM/yyyy | all | LE today |
| registrationDate | Registration Date | dateTime | Y |  | dd/MM/yyyy hh:mm tt | all | LE today |
| recordingno | Instrument Number | string(30) |  |  |  | all |  |
| folio | Page Number | string(20) |  |  |  | all |  |
| volume | Volume | string(20) |  |  |  | all |  |
| comments | Comment | string(800) |  |  |  |  |  |

#### TransactionWithDocFormREPT  (class Transaction; 16 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| transactionNumber | Transaction Number | string(50) |  |  |  | all |  |
| transactionType | Transaction Type | TransactionMetaData |  | transaction_type |  | all |  |
| createDate | Transaction Start Date | dateTime |  |  | dd/MM/yyyy hh:mm tt | all | LE today |
| completeDate | Transaction End Date | dateTime |  |  | dd/MM/yyyy hh:mm tt | all | LE today |
| applicationDate | Application Date | dateTime |  |  | dd/MM/yyyy | all | LE today |
| sourcePropertyIDs | Property Number | string(250) |  |  |  | all |  |
| consideration | Consideration | double |  |  | c2 | all |  |
| siteAnalysis | Site Analysis Recommendation | string(800) |  |  |  | all |  |
| inspectionOfficer | Inspection Officer | string(50) |  |  |  | all |  |
| recordingno | Instrument Number | string(30) |  |  |  | all |  |
| folio | Page Number | string(20) |  |  |  | all |  |
| volume | Volume | string(20) |  |  |  | all |  |
| signDate | Execution Date | dateTime |  |  | dd/MM/yyyy | all | LE today |
| registrationDate | Registration Date | dateTime | Y |  | dd/MM/yyyy hh:mm tt | all | LE today |
| payPeriodYear | Revision Period (years) | integer |  |  | \d{1,2} | all | LE 99 |
| comments | Comment | string(800) |  |  |  |  |  |

#### TransactionWithDocFormSROE  (class Transaction; 15 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| transactionNumber | Transaction Number | string(50) |  |  |  | all |  |
| transactionType | Transaction Type | TransactionMetaData |  | transaction_type |  | all |  |
| createDate | Transaction Start Date | dateTime |  |  | dd/MM/yyyy hh:mm tt | all | LE today |
| applicationDate | Application Date | dateTime |  |  | dd/MM/yyyy | all | LE today |
| sourcePropertyIDs | Property Number | string(250) |  |  |  | all |  |
| relatedInstrNumber | Related Instrument Number | string(50) | Y |  |  | all |  |
| relatedVolume | Related Volume Number | string(50) | Y |  |  | all |  |
| relatedNumber | Related Transaction | string(50) | Y |  |  | all |  |
| signDate | Execution Date | dateTime |  |  | dd/MM/yyyy | all | LE today |
| completeDate | Transaction End Date | dateTime |  |  | dd/MM/yyyy hh:mm tt |  | LE today |
| registrationDate | Registration Date | dateTime | Y |  | dd/MM/yyyy hh:mm tt | all | LE today |
| recordingno | Instrument Number | string(30) |  |  |  | all |  |
| folio | Page Number | string(20) |  |  |  | all |  |
| volume | Volume | string(20) |  |  |  | all |  |
| comments | Comment | string(800) |  |  |  |  |  |

#### TransactionWithDocFormWithRelatedTransTypeAPRF  (class Transaction; 16 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| transactionNumber | Transaction Number | string(50) |  |  |  | all |  |
| transactionType | Transaction Type | TransactionMetaData |  | transaction_type |  | all |  |
| createDate | Transaction Start Date | dateTime |  |  | dd/MM/yyyy hh:mm tt | all | LE today |
| applicationDate | Application Date | dateTime |  |  | dd/MM/yyyy | all | LE today |
| sourcePropertyIDs | Property Number | string(250) |  |  |  | all |  |
| relatedInstrNumber | Related Instrument Number | string(50) | Y |  |  | all |  |
| relatedPageNumber | Related Page Number | string(50) | Y |  |  | all |  |
| relatedVolume | Related Volume Number | string(50) | Y |  |  | all |  |
| relatedNumber | Related Transaction | string(50) | Y |  |  | all |  |
| relatedNumberType | Related Transaction Type | string(50) | Y |  |  | all |  |
| signDate | Execution Date | dateTime |  |  | dd/MM/yyyy | all | LE today |
| registrationDate | Registration Date | dateTime | Y |  | dd/MM/yyyy hh:mm tt | all | LE today |
| recordingno | Instrument Number | string(30) |  |  |  | all |  |
| folio | Page Number | string(20) |  |  |  | all |  |
| volume | Volume | string(20) |  |  |  | all |  |
| comments | Comment | string(800) |  |  |  |  |  |

#### TransactionWithDocGRPMForm  (class Transaction; 15 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| transactionNumber | Transaction Number | string(50) |  |  |  | all |  |
| transactionType | Transaction Type | TransactionMetaData |  | transaction_type |  | all |  |
| createDate | Transaction Start Date | dateTime |  |  | dd/MM/yyyy hh:mm tt | all | LE today |
| applicationDate | Application Date | dateTime |  |  | dd/MM/yyyy | all | LE today |
| sourcePropertyIDs | Property Number | string(250) |  |  |  | all |  |
| relatedInstrNumber | Related Instrument Number | string(50) | Y |  |  | all |  |
| relatedPageNumber | Related Page Number | string(50) | Y |  |  | all |  |
| relatedVolume | Related Volume Number | string(50) | Y |  |  | all |  |
| relatedNumber | Related Transaction | string(50) | Y |  |  | all |  |
| signDate | Execution Date | dateTime |  |  | dd/MM/yyyy | all | LE today |
| registrationDate | Registration Date | dateTime | Y |  | dd/MM/yyyy hh:mm tt | all | LE today |
| recordingno | Instrument Number | string(30) |  |  |  | all |  |
| folio | Page Number | string(20) |  |  |  | all |  |
| volume | Volume | string(20) |  |  |  | all |  |
| comments | Comment | string(800) |  |  |  |  |  |

#### TransactionWithDocSPCForm  (class Transaction; 6 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| transactionNumber | Transaction Number | string(50) |  |  |  | all |  |
| transactionType | Transaction Type | TransactionMetaData |  | transaction_type |  | all |  |
| createDate | Transaction Start Date | dateTime |  |  | dd/MM/yyyy hh:mm tt | all | LE today |
| completeDate | Transaction End Date | date |  |  | dd/MM/yyyy hh:mm tt | all |  |
| applicationDate | Application Date | dateTime |  |  | dd/MM/yyyy | all | LE today |
| comments | Comment | string(800) |  |  |  |  |  |

#### TransactionWithDocSSGForm  (class Transaction; 15 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| transactionNumber | Transaction Number | string(50) |  |  |  | all |  |
| transactionType | Transaction Type | TransactionMetaData |  | transaction_type |  | all |  |
| createDate | Transaction Start Date | dateTime |  |  | dd/MM/yyyy hh:mm tt | all | LE today |
| applicationDate | Application Date | dateTime |  |  | dd/MM/yyyy | all | LE today |
| sourcePropertyIDs | Property Number | string(250) |  |  |  | all |  |
| relatedInstrNumber | Related Instrument Number | string(50) | Y |  |  | all |  |
| relatedPageNumber | Related Page Number | string(50) | Y |  |  | all |  |
| relatedVolume | Related Volume Number | string(50) | Y |  |  | all |  |
| relatedNumber | Related Transaction | string(50) | Y |  |  | all |  |
| signDate | Execution Date | dateTime |  |  | dd/MM/yyyy | all | LE today |
| registrationDate | Registration Date | dateTime | Y |  | dd/MM/yyyy hh:mm tt | all | LE today |
| recordingno | Instrument Number | string(30) |  |  |  | all |  |
| folio | Page Number | string(20) |  |  |  | all |  |
| volume | Volume | string(20) |  |  |  | all |  |
| comments | Comment | string(800) |  |  |  |  |  |

#### TransactionWithDocSSGRecomForm  (class Transaction; 16 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| transactionNumber | Transaction Number | string(50) |  |  |  | all |  |
| transactionType | Transaction Type | TransactionMetaData |  | transaction_type |  | all |  |
| createDate | Transaction Start Date | dateTime |  |  | dd/MM/yyyy hh:mm tt | all | LE today |
| applicationDate | Application Date | dateTime |  |  | dd/MM/yyyy | all | LE today |
| sourcePropertyIDs | Property Number | string(250) |  |  |  | all |  |
| relatedInstrNumber | Related Instrument Number | string(50) | Y |  |  | all |  |
| relatedPageNumber | Related Page Number | string(50) | Y |  |  | all |  |
| relatedVolume | Related Volume Number | string(50) | Y |  |  | all |  |
| relatedNumber | Related Transaction | string(50) | Y |  |  | all |  |
| signDate | Execution Date | dateTime |  |  | dd/MM/yyyy | all | LE today |
| registrationDate | Registration Date | dateTime | Y |  | dd/MM/yyyy hh:mm tt | all | LE today |
| recordingno | Instrument Number | string(30) |  |  |  | all |  |
| folio | Page Number | string(20) |  |  |  | all |  |
| volume | Volume | string(20) |  |  |  | all |  |
| recommendation | Recommendation | string(800) |  |  |  |  |  |
| comments | Comment | string(800) |  |  |  |  |  |

#### TransactionWithoutPropertyListGrid  (class Transaction; 2 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| transactionNumber | Transaction Number | string |  |  |  |  |  |
| transactionType | Transaction Type | TransactionMetaData |  | transaction_type |  |  |  |

#### UnitForm  (class BuildingUnit; 2 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| unitNumber | Unit No. | string(50) | Y |  |  |  |  |
| label | Description | string(512) |  |  |  |  |  |

#### UnitFormNonMandatory  (class BuildingUnit; 2 fields)

| attribute | caption | type | req | dictionary | format | read-only for | validation / condition |
|---|---|---|---|---|---|---|---|
| unitNumber | Unit No. | string(50) |  |  |  |  |  |
| label | Description | string(512) |  |  |  |  |  |

## Appendix B - page types: stages and configuration attributes

Source: all `bin/WorkflowConfig/*.config` parsed (count = number of wizard definitions using the page). Attributes are the extra XML attributes read via `GetAttributeValue`.


### CustomTransactionListPage (15 wizards)

- attributes: ['transactionListForm', 'transaction_dialog_form', 'validate_property', 'validate_right']
- stages: IndexData, LodgeMemorandum, LodgmentApplication, New

### CustomMultiApplicantsPage (99 wizards)

- attributes: []
- stages: AwaitingFeePaymentRegReceiptGen, ChartingAndReporting, ChartingAndReportingSurveyPlanChecking, CompareAndVerifySurveyPlan, ConfirmSurveyPlan, GenerateFinalDemandNotice, GenerateSearchLetter, InspectProperty, InvestigateComplaint, LodgeMemorandum, LodgmentApplication, New, OpenNewPropertyFile, OpenNewPropertyFileandAssignPropertyNumber, PlotParcelInCadastre, PlotSurveyPlanlInCadastre, PrepareSiteAnalysisReport, PrepareValuationReport, ReceiveReviewDocumentsAndAssignTask, RegisterAssessedFeePaymentandGenerateReceipt, RegisterCofOFeesPaymentGenerateReceipt, RegisterFeePaymentAndGenerateReceipt, RegisterProcessingFeePaymentGenerateReceipt, ReviewApproveSurveyPlan, ReviewApproveSurveyPlanChecking, ReviewReportAndApprove, ReviewRequestForSiteAnalysis, ReviewSiteAnalysisReport, ReviewSiteAnalysisReportAndApprove, ReviewSubmission, ReviewValuationReport, RunTraverseComputations, RunTraverseComputationsSurveyPlanChecking, SGReviewSubmittedSurveyPlan, SGReviewSubmittedSurveyPlanChecking, ScheduleSiteAnalysis

### CustomPropertiesPage (196 wizards)

- attributes: ['addbuttondisabled', 'addtotargetpropertybuttondisabled', 'buildingForm', 'deletebuttondisabled', 'mainForm', 'maplinktransaction', 'mapreadonly', 'mapvisible', 'plotForm', 'searchvisible', 'siteAnalysisForm', 'unitForm']
- stages: ApplyNewGroundRentalAmount, ApproveCofO, ApproveLandForm5, ApproveRecommendation(Commissioner), ApproveRecommendation(Governor), ApproveTitleDeedPlan, AwaitingAcceptanceStatutoryRightOccupancyPrinciple, AwaitingFeePaymentRegReceiptGen, BLFinalReview, ChartingAndReporting, CompareAndVerifySurveyPlan, ConfirmSurveyPlan, GenerateCofO, GenerateFinalDemandNotice, IndexData, InspectProperty, InvestigateComplaint, LodgeMemorandum, LodgmentApplication, New, NewIndexData, OpenNewPropertyFile, OpenNewPropertyFileandAssignPropertyNumber, PlotParcelInCadastre, PrepareLandForm5, PrepareRecommendationMemorandum, PrepareRightOccupancyAcceptanceTermsConditionGrant, PrepareSiteAnalysisReport, PrepareTitleDeedPlan, PrepareValuationReport, PropertyFileVerification, ReceiveReviewDocumentsAndAssignTask, RedoPrepareMemorandum, RegisterAssessedFeePaymentandGenerateReceipt, RegisterCofO, RegisterCofOFeesPaymentGenerateReceipt, RegisterFeePaymentAndGenerateReceipt, RegisterInstrument, RegisterProcessingFeePaymentGenerateReceipt, ReviewAndSignMemorandum, ReviewApproveSurveyPlan, ReviewCofO, ReviewDataINSC, ReviewLandForm5, ReviewMemorandum, ReviewRecommendation, ReviewReportAndApprove, ReviewRequestForSiteAnalysis, ReviewSignedCofO, ReviewSignedLandForm5, ReviewSignedMemorandum, ReviewSignedRecommendation, ReviewSiteAnalysisReport, ReviewSiteAnalysisReportAndApprove, ReviewSubmission, ReviewTitleDeedPlanSG, ReviewTitleDeedPlanSQCO, ReviewValuationReport, RunTraverseComputations, SGReviewSubmittedSurveyPlan, ScheduleSiteAnalysis

### OutstandingPage (10 wizards)

- attributes: ['capture_past_outstanding', 'current_year_validation', 'feetype', 'show_penalty_section', 'view_past_outstanding']
- stages: BLFinalReview, IndexData, LodgmentApplication, New, NewIndexData, PrepareRightOccupancyAcceptanceTermsConditionGrant, ReviewDataINSC

### FeeListSelectionPage (19 wizards)

- attributes: ['feesForTargetProperties']
- stages: GenerateFinalDemandNotice, LodgmentApplication, New, OpenNewPropertyFile, PrepareRightOccupancyAcceptanceTermsConditionGrant

### CustomPrintDocuments2Page (65 wizards)

- attributes: ['reports']
- stages: ApplyNewGroundRentalAmount, AwaitingFeePaymentRegReceiptGen, DeliveryOfDocuments, GenerateCofO, GenerateFinalDemandNotice, LodgmentApplication, New, PrepareLandForm5, PrepareRejectionLetter, PrepareRightOccupancyAcceptanceTermsConditionGrant, RegisterAssessedFeePaymentandGenerateReceipt, RegisterCofO, RegisterCofOFeesPaymentGenerateReceipt, RegisterFeePaymentAndGenerateReceipt, RegisterProcessingFeePaymentGenerateReceipt

### CompleteAssignPage (308 wizards)

- attributes: ['excludeTransition', 'hideCadastreTransitionForArchivedProperty', 'rules']
- stages: ApplyNewGroundRentalAmount, ApproveCofO, ApproveLandForm5, ApproveRecommendation(Commissioner), ApproveRecommendation(Governor), ApproveTitleDeedPlan, AwaitingAcceptanceStatutoryRightOccupancyPrinciple, AwaitingAssessedFeePayment, AwaitingCofOFeesPayment, AwaitingFeePayment, AwaitingFeePaymentRegReceiptGen, AwaitingRectificationAndResubmission, BLFinalReview, CertifyDocuments, ChartingAndReporting, ChartingAndReportingSurveyPlanChecking, CompareAndVerifySurveyPlan, ConfirmSurveyPlan, CreateParcel, DeliveryOfDocuments, DispatchPropertyFileApplicantReview, GenerateCofO, GenerateFinalDemandNotice, GenerateSearchLetter, IndexData, InspectProperty, InvestigateComplaint, LodgeMemorandum, LodgmentApplication, New, NewIndexData, OpenNewPropertyFile, OpenNewPropertyFileandAssignPropertyNumber, PlotParcelInCadastre, PlotSurveyPlanlInCadastre, PrepareLandForm5, PrepareRecommendationMemorandum, PrepareRejectionLetter, PrepareRightOccupancyAcceptanceTermsConditionGrant, PrepareSiteAnalysisReport, PrepareTitleDeedPlan, PrepareValuationReport, PropertyFileVerification, ReceiveReviewDocumentsAndAssignTask, RedoPrepareMemorandum, RegisterAssessedFeePaymentandGenerateReceipt, RegisterCofO, RegisterCofOFeesPaymentGenerateReceipt, RegisterFeePaymentAndGenerateReceipt, RegisterInstrument, RegisterProcessingFeePaymentGenerateReceipt, RegisterSurveyPlan, RetrievePropertyFile, ReviewAndSignMemorandum, ReviewApproveSurveyPlan, ReviewApproveSurveyPlanChecking, ReviewCofO, ReviewCorrection, ReviewDataINSC, ReviewLandForm5, ReviewMemorandum, ReviewRecommendation, ReviewReportAndApprove, ReviewRequestForSiteAnalysis, ReviewSignedCofO, ReviewSignedLandForm5, ReviewSignedMemorandum, ReviewSignedRecommendation, ReviewSiteAnalysisReport, ReviewSiteAnalysisReportAndApprove, ReviewSubmission, ReviewTitleDeedPlanSG, ReviewTitleDeedPlanSQCO, ReviewValuationReport, RunTraverseComputations, RunTraverseComputationsSurveyPlanChecking, SGReviewSubmittedSurveyPlan, SGReviewSubmittedSurveyPlanChecking, ScanCorrectedDocuments, ScanIncomingDocuments, ScanOutgoingDocuments, ScanPropertyFile, ScanTitleDeedPlan, ScheduleSiteAnalysis

### CustomTransactionWithDocumentsPage (293 wizards)

- attributes: ['checkPages', 'docEnabled', 'documents', 'set_enable_fields', 'set_mandatory_fields']
- stages: ApplyNewGroundRentalAmount, ApproveCofO, ApproveLandForm5, ApproveRecommendation(Commissioner), ApproveRecommendation(Governor), ApproveTitleDeedPlan, AwaitingAcceptanceStatutoryRightOccupancyPrinciple, AwaitingAssessedFeePayment, AwaitingCofOFeesPayment, AwaitingFeePayment, AwaitingFeePaymentRegReceiptGen, AwaitingRectificationAndResubmission, BLFinalReview, CertifyDocuments, ChartingAndReporting, ChartingAndReportingSurveyPlanChecking, CompareAndVerifySurveyPlan, ConfirmSurveyPlan, CreateParcel, DeliveryOfDocuments, DispatchPropertyFileApplicantReview, GenerateCofO, GenerateFinalDemandNotice, GenerateSearchLetter, IndexData, InspectProperty, InvestigateComplaint, LodgeMemorandum, LodgmentApplication, New, NewIndexData, OpenNewPropertyFile, OpenNewPropertyFileandAssignPropertyNumber, PlotParcelInCadastre, PlotSurveyPlanlInCadastre, PrepareLandForm5, PrepareRecommendationMemorandum, PrepareRejectionLetter, PrepareRightOccupancyAcceptanceTermsConditionGrant, PrepareSiteAnalysisReport, PrepareTitleDeedPlan, PrepareValuationReport, PropertyFileVerification, ReceiveReviewDocumentsAndAssignTask, RedoPrepareMemorandum, RegisterAssessedFeePaymentandGenerateReceipt, RegisterCofO, RegisterCofOFeesPaymentGenerateReceipt, RegisterFeePaymentAndGenerateReceipt, RegisterInstrument, RegisterProcessingFeePaymentGenerateReceipt, RegisterSurveyPlan, RetrievePropertyFile, ReviewAndSignMemorandum, ReviewApproveSurveyPlan, ReviewApproveSurveyPlanChecking, ReviewCofO, ReviewDataINSC, ReviewLandForm5, ReviewMemorandum, ReviewRecommendation, ReviewReportAndApprove, ReviewRequestForSiteAnalysis, ReviewSignedCofO, ReviewSignedLandForm5, ReviewSignedMemorandum, ReviewSignedRecommendation, ReviewSiteAnalysisReport, ReviewSiteAnalysisReportAndApprove, ReviewSubmission, ReviewTitleDeedPlanSG, ReviewTitleDeedPlanSQCO, ReviewValuationReport, RunTraverseComputations, RunTraverseComputationsSurveyPlanChecking, SGReviewSubmittedSurveyPlan, SGReviewSubmittedSurveyPlanChecking, ScanIncomingDocuments, ScanOutgoingDocuments, ScanPropertyFile, ScanTitleDeedPlan, ScheduleSiteAnalysis

### DemandNoticePage (168 wizards)

- attributes: ['payments_validation']
- stages: ApproveCofO, ApproveLandForm5, ApproveRecommendation(Commissioner), ApproveRecommendation(Governor), AwaitingAcceptanceStatutoryRightOccupancyPrinciple, AwaitingFeePaymentRegReceiptGen, ChartingAndReporting, ChartingAndReportingSurveyPlanChecking, CompareAndVerifySurveyPlan, ConfirmSurveyPlan, GenerateCofO, GenerateSearchLetter, InspectProperty, InvestigateComplaint, OpenNewPropertyFileandAssignPropertyNumber, PlotParcelInCadastre, PlotSurveyPlanlInCadastre, PrepareLandForm5, PrepareRecommendationMemorandum, PrepareRightOccupancyAcceptanceTermsConditionGrant, PrepareSiteAnalysisReport, PrepareValuationReport, RegisterAssessedFeePaymentandGenerateReceipt, RegisterCofO, RegisterCofOFeesPaymentGenerateReceipt, RegisterFeePaymentAndGenerateReceipt, RegisterInstrument, RegisterProcessingFeePaymentGenerateReceipt, RegisterSurveyPlan, ReviewApproveSurveyPlan, ReviewApproveSurveyPlanChecking, ReviewCofO, ReviewLandForm5, ReviewRecommendation, ReviewReportAndApprove, ReviewRequestForSiteAnalysis, ReviewSignedCofO, ReviewSignedLandForm5, ReviewSignedRecommendation, ReviewSiteAnalysisReport, ReviewSiteAnalysisReportAndApprove, ReviewSubmission, ReviewValuationReport, RunTraverseComputations, RunTraverseComputationsSurveyPlanChecking, SGReviewSubmittedSurveyPlan, SGReviewSubmittedSurveyPlanChecking, ScheduleSiteAnalysis

### CustomPartiesPage (104 wizards)

- attributes: ['add_applicant_button_visible']
- stages: ApproveCofO, ApproveLandForm5, ApproveRecommendation(Commissioner), ApproveRecommendation(Governor), AwaitingAcceptanceStatutoryRightOccupancyPrinciple, GenerateCofO, LodgmentApplication, New, PrepareLandForm5, PrepareRecommendationMemorandum, PrepareRightOccupancyAcceptanceTermsConditionGrant, ReceiveReviewDocumentsAndAssignTask, RegisterCofO, RegisterInstrument, ReviewCofO, ReviewLandForm5, ReviewRecommendation, ReviewSignedCofO, ReviewSignedLandForm5, ReviewSignedRecommendation, ReviewSubmission

### RrrPage (19 wizards)

- attributes: ['isRetrieveCurrentOwner', 'rrrForm', 'set_mandatory_fields']
- stages: ApplyNewGroundRentalAmount, DeliveryOfDocuments, GenerateCofO, LodgeMemorandum, New, PrepareLandForm5, PrepareRightOccupancyAcceptanceTermsConditionGrant, RedoPrepareMemorandum, RegisterCofO, ReviewAndSignMemorandum, ReviewMemorandum, ReviewSignedMemorandum, ReviewSubmission, ScanIncomingDocuments, ScanOutgoingDocuments

### RegisterInstrumentPage (10 wizards)

- attributes: []
- stages: RegisterCofO, RegisterInstrument

### RejectPage (7 wizards)

- attributes: []
- stages: PrepareRejectionLetter

### CustomPastTransactionPage (4 wizards)

- attributes: ['PartyContactForm', 'bodyForm', 'gridForm', 'pastPartiesGrid', 'pastPartyForm', 'pastPaymentForm', 'pastPaymentGrid', 'pastTransactionForm', 'payments_optional', 'view_registered_transaction']
- stages: BLFinalReview, IndexData, NewIndexData, ReviewDataINSC

### TransactionCorrectionPage (3 wizards)

- attributes: ['block_search_transaction', 'ignore_related_validation', 'secondary_form', 'set_disable_fields', 'set_enable_fields']
- stages: New, ReviewCorrection, ScanCorrectedDocuments

### DataCorrectionRrrPage (2 wizards)

- attributes: ['mainForm', 'plotForm', 'siteAnalysisForm', 'validate_related_party']
- stages: New, ReviewCorrection

### GroundRentReviewPage (6 wizards)

- attributes: []
- stages: ApplyNewGroundRentalAmount, LodgeMemorandum, New, RedoPrepareMemorandum, ReviewAndSignMemorandum, ReviewMemorandum, ReviewSignedMemorandum

### CustomPrintDocumentsMultiProp2Page (21 wizards)

- attributes: ['reports']
- stages: GenerateCofO, PrepareRecommendationMemorandum, PrepareSiteAnalysisReport, RegisterInstrument

### CustomSearchPropertyPage (5 wizards)

- attributes: ['max_prop_number', 'validate_property']
- stages: GenerateSearchLetter, LodgmentApplication, New, RegisterFeePaymentAndGenerateReceipt

### CustomOutstandingPage (2 wizards)

- attributes: ['capture_past_outstanding', 'current_year_validation', 'feetype', 'show_penalty_section', 'view_past_outstanding']
- stages: LodgmentApplication, New

### PrintDocuments2Page (6 wizards)

- attributes: ['reports']
- stages: DeliveryOfDocuments, LodgmentApplication, New, RegisterFeePaymentAndGenerateReceipt

### CustomPrintSearchPage (1 wizards)

- attributes: ['reports']
- stages: GenerateSearchLetter

### SurveyPlanPage (8 wizards)

- attributes: []
- stages: ChartingAndReportingSurveyPlanChecking, LodgmentApplication, New, PlotSurveyPlanlInCadastre, RegisterFeePaymentAndGenerateReceipt, RegisterSurveyPlan, ReviewApproveSurveyPlanChecking, RunTraverseComputationsSurveyPlanChecking, SGReviewSubmittedSurveyPlanChecking

### RegisterPlanNumberPage (1 wizards)

- attributes: []
- stages: RegisterSurveyPlan

## Appendix C - form names referenced by WorkflowConfig

```
BuildingForm <- ['CustomPropertiesPage']
PartyForm <- ['CustomPastTransactionPage']
PastPartyContactForm <- ['CustomPastTransactionPage']
PastPaymentForm <- ['CustomPastTransactionPage']
PastTransactionForm <- ['CustomPastTransactionPage']
PlotAPCAForm <- ['CustomPropertiesPage']
PlotForm <- ['CustomPropertiesPage', 'DataCorrectionRrrPage']
PlotFormCHPC <- ['CustomPropertiesPage']
PlotFormForLodgment <- ['CustomPropertiesPage']
PlotFormRO <- ['CustomPropertiesPage']
PlotINSCForm <- ['CustomPropertiesPage']
PropertyAPCAForm <- ['CustomPropertiesPage']
PropertyCofOGrid <- ['CustomPropertiesPage(name)']
PropertyForm <- ['CustomPropertiesPage']
PropertyFormCHPC <- ['CustomPropertiesPage']
PropertyFormDCT <- ['DataCorrectionRrrPage']
PropertyFormForEXTTandPSUR <- ['CustomPropertiesPage']
PropertyFormForLodgment <- ['CustomPropertiesPage']
PropertyFormINSC <- ['CustomPropertiesPage']
PropertyGrid <- ['CustomPropertiesPage(name)', 'CustomSearchPropertyPage(name)']
PropertyTransactionsGrid <- ['CustomPastTransactionPage']
RRRAttributeDisableForFRTrForm <- ['RrrPage']
RegUnitForm <- ['CustomPastTransactionPage']
RegisterPlanNumberForm <- ['RegisterPlanNumberPage(name)']
SiteAnalysisForm <- ['CustomPropertiesPage', 'DataCorrectionRrrPage']
TransactionDCTForm <- ['TransactionCorrectionPage(name)']
TransactionFormINSC <- ['CustomTransactionListPage']
TransactionFormSmall <- ['CustomPastTransactionPage(name)']
TransactionIndexFormINSC <- ['CustomTransactionWithDocumentsPage(name)']
TransactionRegisterInstrumentForm <- ['RegisterInstrumentPage(name)']
TransactionSPCForm <- ['CustomTransactionListPage']
TransactionWithDocConsiMandatoryAndAllTermsForm <- ['CustomTransactionWithDocumentsPage(name)']
TransactionWithDocConsiMandatoryAndTermMonthForm <- ['CustomTransactionWithDocumentsPage(name)']
TransactionWithDocConsiMandatoryForm <- ['CustomTransactionWithDocumentsPage(name)']
TransactionWithDocDCTForm <- ['TransactionCorrectionPage']
TransactionWithDocDCTModifyForm <- ['TransactionCorrectionPage']
TransactionWithDocFRGForm <- ['CustomTransactionWithDocumentsPage(name)']
TransactionWithDocFormAPRF <- ['CustomTransactionWithDocumentsPage(name)']
TransactionWithDocFormREPT <- ['CustomTransactionWithDocumentsPage(name)']
TransactionWithDocFormSROE <- ['CustomTransactionWithDocumentsPage(name)']
TransactionWithDocFormWithConsiNoneMandatory <- ['CustomTransactionWithDocumentsPage(name)']
TransactionWithDocFormWithRelatedTransTypeAPRF <- ['CustomTransactionWithDocumentsPage(name)']
TransactionWithDocGRPMForm <- ['CustomTransactionWithDocumentsPage(name)']
TransactionWithDocSPCForm <- ['CustomTransactionWithDocumentsPage(name)']
TransactionWithDocSSGAndAllTermsForm <- ['CustomTransactionWithDocumentsPage(name)']
TransactionWithDocSSGForm <- ['CustomTransactionWithDocumentsPage(name)']
TransactionWithoutPropertyListGrid <- ['CustomTransactionListPage']
Unitform <- ['CustomPropertiesPage']
```
