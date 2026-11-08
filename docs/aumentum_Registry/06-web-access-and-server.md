# 06 — Registry Web Access (lrswa) and the LRS server application on 10.10.10.5

Scope: what runs on the Registry server **10.10.10.5** (host name `REGISTRY`, domain `Plagis.com`,
also reached as `registry.plagis.com`), the **LRS server application** (JBoss `lrs.war`: SOAP API,
admin web UI, Alfresco repository, jBPM, reports, scheduler) and the **Web Access** browser
application (`lrswa.war`). Integrations with other systems are in `07-integrations.md`.

Requirement gathering only. Nothing here was run against the live servers; every fact comes from
the read-only captures listed below.

**Source abbreviations used in this document**

| Abbrev. | Path |
|---|---|
| `$R` | `tools/aumentum-inventory` (repo-relative; git-ignored captures) |
| `$APP` | `$R/lrs-export/registry/app/LRSAS/server/default` (copy of `C:\LRSAS\server\default` on .5) |
| `$INV` | `$R/lrs-inventory-REGISTRY-20261101-1638.txt` (host/services/ports/software/config dump; `-1701` = database dump) |
| `$DATA` | `$R/lrs-export/registry/data/*.psv.gz` (table exports, 2026-11-02) |
| `$J` | the session scratchpad `in/*.jar` (Java jars from `lrs.war` / `lrswa.war` `WEB-INF/lib`, inspected with `unzip -l` and `javap` only) |
| `$D` | the session scratchpad `dec/net/**` (decompiled .NET Registry Desktop and Cashier) |

Statements marked **(inferred)** are deductions, not something a capture shows directly.

---

## 1. The server at a glance

| Item | Value | Source |
|---|---|---|
| OS | Windows Server 2012 R2 Standard 6.3.9600, 64-bit, installed 2014-11-10 | `$INV` HOST |
| Hardware | Dell PowerEdge R910, 64 logical CPUs, 128 GB RAM | `$INV` HOST |
| Disks | C: 402 GB used / 429 free; **D: 1,573 GB used / 270 free** (SQL data and `D:\LRS_STORAGE`); E: 16/3 (Cashier install); M: 12/88 (Metabase, PostgreSQL) | `$INV` HOST |
| Database | SQL Server 2012 RTM 11.0.2100.60 Standard 64-bit, instance `REGISTRY` (default instance, port 1433); DBs `LRS43` (755,526 MB, FULL recovery, last full backup 2026-10-06), `Cashier` (418 MB, last backup 2017-12-20), `ReportServer`/`ReportServerTempDB` (SSRS, effectively empty) | `$R/lrs-inventory-REGISTRY-20261101-1701.txt` |
| App server | Windows service **`LRSAS`** = `C:\LRSAS\bin\i386\jbosssvc.exe -r LRSAS`, runs as `.\Administrator`; "JBoss Application Server 4.2.3.115" packaged by *Manatron, Inc. – A Thomson Reuters Business* (2014-11-25) | `$INV` SERVICES, INSTALLED SOFTWARE |
| JVM | `C:\Program Files\Java\jdk1.7.0_67\bin\java.exe -Xrs -Xms16384M -Xmx32768M -XX:MaxPermSize=256M -XX:NewRatio=2 -XX:+UseParallelOldGC ... org.jboss.Main -b 0.0.0.0` (binds every interface) | `$INV` SERVICES (java.exe pid 8596) |
| Aumentum products installed | Aumentum Registry **Server 4.3.22.53**, **Web Access 4.3.22.53**, **Workstation 4.3.22.53** (all 2019-10-28), **Designer 4.3.53** (2016), **Cashier 3.2.29.194** (2014) | `$INV` INSTALLED SOFTWARE |
| Version strings | `lrswa.war/.../lrs/version.properties`: 4.3 rev 22 build 53; `lrs.war/.../alfresco/version.properties`: Alfresco **3.4.9 Enterprise** build 664, schema 4219, app 4.3.22.53, support e-mail `AumentumInternational.Support@thomsonreuters.com` | `$APP` |
| Other Windows services of interest | `Aumentum Registry Cashier Scheduler` (`E:\Program Files (x86)\Aumentum Registry\Aumentum Registry Cashier\bin\Aumentum.Registry.Cashier.Scheduler.exe`); IIS (`W3SVC`, `IISADMIN`); `ReportServer` (SSRS 2012); `MSSQLServerOLAPService` (SSAS); `MsDtsServer110` (SSIS); `MSMQ`; **`PLAGIS Analytics`** (NSSM wrapper running `java -jar metabase-v0.41.5_2021-12-16.jar` from `M:\Metabase`); `postgresql-x64-17` (`M:\PostgreSQL\17`); TeamViewer; Kaspersky; `SQLSERVERAGENT` **Stopped/Manual** (no SQL Agent jobs run) | `$INV` SERVICES |
| Other software | ArcGIS 10.2.1 for Desktop, Silverlight 5.1, Google Chrome 66, Java 8u151/u211, Microsoft Report Viewer 2012, SQL tools | `$INV` INSTALLED SOFTWARE |

### 1.1 Listening ports (from `netstat`, `$INV` NETWORK)

| Port | Process | What it is |
|---|---|---|
| **8080** | JBoss (pid 8596) | HTTP/1.1 connector (Tomcat/JBossWeb). Serves `/lrs/*` and `/lrswa/*`. Plain HTTP only. `server.xml`: `<Connector port="8080" address="${jboss.bind.address}" protocol="HTTP/1.1" emptySessionPath="true" enableLookups="false" redirectPort="8443">`; the 8443 HTTPS connector is commented out. |
| 8009 | JBoss | AJP/1.3 connector (unused, no front web server proxies to it as far as captured) |
| 8083 | JBoss | JBoss WebService/RMI class-loading port (stock JBoss 4) |
| 1098, 1099, 4444, 4445, 4446, 3873 | JBoss | JNDI / RMI / invoker / EJB3 remoting (stock JBoss 4) |
| 8093 | JBoss | JBossMQ UIL2 (JMS) — used internally (4 established loopback connections) by the LRS scheduler (§6) |
| 50500–50508 | JBoss | Alfresco RMI services (`alfresco-shared.properties` `alfresco.rmi.services.port=50500`; `repository.properties`: avm 50501, avmsync 50502, authentication 50504, repo 50505, action 50506, deployment 50507, monitor 50508) |
| 49199–49201, 49314 | JBoss | further RMI/ephemeral listeners of the JBoss process |
| **1433** | sqlservr | SQL Server (JBoss holds ~45 pooled connections from loopback) |
| 80, 443 | System (IIS / http.sys) | IIS sites — the **Aumentum Registry Cashier** web application and its `ILSCashSrv/ILSCash4LRS.asmx` service (inferred from `lr_setting config.CashierServiceAddress = http://localhost/ILSCashSrv/ILSCash4LRS.asmx`; the IIS site list itself was not captured) |
| 3000 | java (Metabase) | **PLAGIS Analytics** (Metabase 0.41.5); has an open connection to SQL Server on 127.0.0.1:1433 |
| 5432 | postgres | PostgreSQL 17 (on `M:`; purpose not captured; probably Metabase's application DB (inferred)) |
| 2383 | msmdsrv | SSAS |
| 1801, 2103, 2105, 2107 | mqsvc | MSMQ |
| 5022 | sqlservr | SQL Server endpoint (mirroring/Service Broker) |
| 3389, 5938, 5985, 445/139, 135 | Windows | RDP, TeamViewer, WinRM, SMB, RPC |

Established connections at capture time: three desktops (10.10.10.175, .211, .217) on 8080 — the
Registry Desktop clients; 10.10.10.6 on WinRM 5985 and 10.10.10.127 on SMB (the capture tooling).

### 1.2 What is deployed in JBoss (`C:\LRSAS\server\default\deploy`)

| Deployment | Context | Purpose | Source |
|---|---|---|---|
| `lrs.war` | `/lrs` | **The LRS server**: display name "AUMENTUM Registry Web Client". Alfresco 3.4.9 Enterprise repository + Alfresco Explorer (JSF/MyFaces) web client customised as the Registry admin UI, Axis 1 SOAP API (`/lrs/api/*`), CXF (`/lrs/cxfapi/*`, `/lrs/cmis/*`), content upload/download servlets, jBPM 3 workflow engine, ILS reports (BIRT), JMS scheduler. | `$APP/deploy/lrs.war/WEB-INF/web.xml` |
| `lrswa.war` | `/lrswa` | **Registry Web Access**: display name "LRS Web Access Application", Spring MVC `DispatcherServlet` `LrsWebClient` mapped to `*.do`, Spring Security filter chain on `/*`. | `$APP/deploy/lrswa.war/WEB-INF/web.xml` |
| `ils-ds.xml` | JNDI `IlsDS` | jTDS `jdbc:jtds:sqlserver://REGISTRY:1433/LRS43`, pool 30–100, blocking timeout 15 s, validation `select 1`, type mapping MS SQLSERVER2000. User/password: **<redacted, in the file>**. Used by `lrs.war`. | `$APP/deploy/ils-ds.xml` |
| `ils-lrswa-ds.xml` | JNDI `IlsLRSWADS` | same database `LRS43`, same driver, `MssqlExceptionSorter`, default pool. Credentials **<redacted>**. Used by `lrswa.war`. | `$APP/deploy/ils-lrswa-ds.xml` |
| `hsqldb-ds.xml` | `DefaultDS` | stock JBoss Hypersonic (`data/hypersonic/localDB`, HSQL 1.8.0) — JBossMQ persistence etc. | `$APP/deploy/hsqldb-ds.xml` |
| `jms/` | `JmsXA`, `DefaultJMSProvider` | stock JBossMQ (used by the LRS scheduler, §6) | `$APP/deploy/jms/jms-ds.xml` |
| `jmx-console.war`, `management/console-mgr.sar/web-console.war`, `http-invoker.sar`, `jbossws.sar`, `jboss-web.deployer/ROOT.war` | `/jmx-console`, `/web-console`, `/invoker`, `/jbossws`, `/` | stock JBoss 4.2.3 consoles. They are protected by BASIC auth (`java:/jaas/jmx-console`), but **`conf/props/jmx-console-users.properties` still holds the stock JBoss sample account (credentials redacted)** and `jbossws-users.properties` the stock sample user. Security finding for PLAGIS ICT. | `$APP/conf/props/*`, `$APP/deploy/jmx-console.war/WEB-INF/*` |

`conf/jndi.properties` is the stock JBoss one (`org.jnp.interfaces.NamingContextFactory`).

Both WARs use **Hibernate with `hibernate.hbm2ddl.auto=update`** against SQL Server
(`lrs.war/.../alfresco/domain/hibernate-cfg.properties`, `lrswa.war/.../alfresco/domain/hibernate-cfg.properties`,
`lrswa.war/.../lrs/domain/hibernate/hibernate-cfg.properties`; dialects `AlfrescoSQLServerDialect` /
`SQLServerDialect`, isolation 2 = READ COMMITTED, JTA via JBoss transaction manager). **Consequence for
the replica:** while the old JBoss runs against LRS43, it may add missing columns/tables at start-up;
the replica must never drop or rename anything the old mappings expect, and must tolerate columns the
old server adds.

---

## 2. The LRS server application (`lrs.war`)

### 2.1 Technology

| Layer | Technology | Evidence |
|---|---|---|
| Repository / DMS | Alfresco 3.4.9 Enterprise, embedded (stores `workspace://SpacesStore`, `user://alfrescoUserStore`, `system://system`, `archive://SpacesStore`, `lrs://rootStore`, version stores; Lucene indexes) | `alfresco/version.properties`, `$APP/conf/ils/lrs/repository.properties`, `lrs/lrs-model.properties` |
| Web UI | JSF 1.x on **Apache MyFaces** + Facelets (`*.xhtml`, taglibs `userctlr.taglib.xml`, `sdms.taglib.xml`), DWR (`/dwr/*`, `DwrSpringServlet` with a JSON converter) for AJAX, Alfresco Explorer web client servlets | `lrs.war/WEB-INF/web.xml` context params and servlets |
| Spring | Spring 2.5/3 contexts `/WEB-INF/web-application-context.xml` (not captured) | `web.xml` |
| SOAP | **Apache Axis 1** `AxisServlet` on `/api/*` (19 services, `WEB-INF/server-config.wsdd`); **Apache CXF** on `/cxfapi/*` and `/cmis/*` (BusinessLogicService, RepositoryWebService, CMIS) | `web.xml`, `server-config.wsdd`, `$APP/deploy/lrs.war/wsdl/*` |
| Workflow | jBPM 3 (Alfresco-embedded): `JBPM_*` tables in LRS43 (64 process definitions, 99,808 process instances, 674,828 task instances, `JBPM_JOB` = 0 rows) | `$R/lrs-inventory-REGISTRY-20261101-1701.txt` |
| Reports | ILS Reports service on **Eclipse BIRT** (`.rptdesign` templates stored in the Alfresco repository; ODA drivers `com.ils.alfresco.oda.runtime.driver_2.3.0.04.jar`, `com.ils.hibernate.oda.runtime.driver_2.3.0.01.jar`, `org.eclipse.datatools.*`) | `$J` jar list; `$D/.../CustomPrintDocumentsMultiProp2Page.cs` builds `"{name}.rptdesign"` |
| Barcodes | `com.barcodelib.barcode` servlets `/linear`, `/pdf417`, `/datamatrix`, `/qrcode` (images for printed documents) | `web.xml` |
| Messaging | JBossMQ JMS + `com.ils.sdms.messaging` (`MessagingService` SOAP) | `web.xml`, `lrs-web.jar` |
| Logging | log4j 1.2; boot log `C:\LRSAS\bin\log4j.properties` → `${jboss.server.log.dir}/boot.log`; server log and `localhost_access_log.*.log` (Tomcat `AccessLogValve`, pattern `common`) in `C:\LRSAS\server\default\log` (**5,564 files**) | `$INV` config dump, folder map |

### 2.2 URL map of `/lrs` (from `lrs.war/WEB-INF/web.xml`)

| URL pattern | Servlet / filter | Purpose |
|---|---|---|
| `/lrs/faces/*` | `FacesServlet` + Alfresco `AuthenticationFilter` | the admin web UI (JSF). Start page `/lrs/faces/jsp/dashboard/dashboard.jsp` (known). Admin-only (Alfresco `AdminAuthenticationFilter`): `/faces/jsp/admin/*`, `/faces/jsp/categories/*`, `/faces/jsp/groups/*`, `/faces/jsp/users/users.jsp`, `/faces/jsp/users/delete-user.jsp`, `/faces/jsp/admin/system-info.jsp` |
| `/lrs/api/*` | Axis `AxisServlet` | **the SOAP API** (§2.3) |
| `/lrs/cxfapi/*`, `/lrs/cmis/*` | CXF | BusinessLogicService (`/lrs/cxfapi/BusinessLogicService`), RepositoryWebService (`/lrs/cxfapi/RepositoryWebService`, used by the Registry Desktop), CMIS web services (`Alfresco-CMISWS-Service.wsdl`, `CMISWS-Service.wsdl`) |
| `/lrs/cmisbrowse` | OpenCMIS `BrowseServlet` | CMIS AtomPub browser |
| `/lrs/upload/*` | Alfresco `UploadContentServlet` | **PUT** content into the repository: `PUT /lrs/upload/{guid}.{ext}?ticket=…&encoding=UTF-8` (used by the Cadastre to send title-deed plans and by the Desktop's scanner) |
| `/lrs/uploadFileServlet` | `UploadFileServlet` | multipart upload for the web UI |
| `/lrs/download/*`, `/lrs/d/*`, `/lrs/dr` | `DownloadContentServlet`, `DownloadRawContentServlet` | download content by node ref (`/d/a/workspace/SpacesStore/{uuid}/{name}` Alfresco convention) |
| `/lrs/guestDownload/*`, `/lrs/gd/*`, `/lrs/guestTemplate/*`, `/lrs/gt/*` | Guest download/template servlets | Alfresco guest access (guest login disabled, `alfresco.authentication.allowGuestLogin=false`) |
| `/lrs/template/*`, `/lrs/t/*` | `TemplateContentServlet` | FreeMarker templates over nodes |
| `/lrs/navigate/*`, `/lrs/n/*`, `/lrs/command/*`, `/lrs/c/*`, `/lrs/ajax/*` | Alfresco Explorer servlets | external access, commands, AJAX |
| `/lrs/service/*`, `/lrs/s/*`, `/lrs/wcservice/*`, `/lrs/wcs/*`, `/lrs/168service/*`, `/lrs/168s/*` | Spring Surf `WebScriptServlet` | Alfresco web scripts (basic / web-client auth) |
| `/lrs/facebook/*`, `/lrs/fb/*`, `/lrs/fbservice/*`, `/lrs/fbs/*` | Alfresco Facebook servlets | stock, unused (inferred) |
| `/lrs/webdav/*` | `WebDAVServlet` | WebDAV access to the repository |
| `/lrs/remoting/*`, `/lrs/remoteauth/*` | Spring `DispatcherServlet` (`classpath*:*/remoting/*-services-context.xml`), `RemoteAuthenticationServlet` | ILS remoting (`ils-integration.jar`, `com.ils.remoting.*`) |
| `/lrs/dwr/*` | DWR | AJAX calls of the admin UI to the `*WebBean` beans (§2.4) |
| `/lrs/proxy` | `com.ils.web.app.servlet.HTTPProxyServlet` | HTTP proxy used by the web UI (target not captured) |
| `/lrs/downloadBPM` | `ProcessDefinitionDownloadServlet` | download a jBPM process archive |
| `/lrs/jbpm/deployprocess` | `JBPMDeployProcessServlet` | **disabled**: `system.workflow.deployservlet.enabled=false` |
| `/lrs/workflowdefinitionimage/*` | Alfresco | workflow diagram images |
| `/lrs/linear`, `/lrs/pdf417`, `/lrs/datamatrix`, `/lrs/qrcode` | barcodelib | barcode images |

Filters: `ClearThreadSecurityContextFilter` on `/*` (auto-closes the user session for `/api,/cxfapi,/cmis,/upload,/uploadFileServlet,/download,/d,/guestDownload,/gd`), `GlobalLocalizationFilter`, Alfresco authentication filters (`/faces/*`, `/dwr/*`, `/navigate/*` …), `RestoreRemoteSessionFilter` (`/*`), `CacheExpiresFilter` (30 days on jpg/png/gif/css/js), JSF session synchronisation. Session timeout **60 min**; error page `/jsp/error.jsp`; welcome `index.jsp`. JSF state saving on the server, one view per session.

### 2.3 SOAP services (Axis, `/lrs/api/<Service>`)

From `$APP/deploy/lrs.war/WEB-INF/server-config.wsdd` and the live WSDLs (`$R/lrs-capture/wsdl/*.wsdl`).
Detailed contracts are covered elsewhere; this table is the inventory the replica must keep.

| Service | Implementation class | Operations (from WSDL) | Main callers |
|---|---|---|---|
| AuthenticationService | `org.alfresco.repo.webservice.authentication.AuthenticationServiceSoapBindingImpl` | startSession, endSession, isTicketValid … | Desktop, Cashier web app, Cadastre (this repo), Aumentum MCServer, GISViewer |
| AccessControlService | Alfresco | getAuthorities, getParentAuthorities, getChildAuthorities, createAuthorities, addChildAuthorities, … ACL calls | Desktop, Cadastre sign-in (`lrs.rs` `user_groups`) |
| AdministrationService | Alfresco | createUsers, updateUsers, changePassword, deleteUsers, queryUsers, getUser | Desktop, Cadastre (`change_password`, `create_user`) |
| ActionService, AuthoringService, ClassificationService, ContentService, DictionaryService, RepositoryService, PersonWebService | Alfresco 3.4 web services (`RepositoryService` also via ILS `ils-remote-api2.jar` `org.alfresco.repo.webservice2.*`) | stock | Desktop, Cashier |
| **DomainModelService** | `com.ils.lrs.webservice.model.DomainModelServiceSOAPImpl` | loadObjects (HQL / SP / by-id queries), saveObjects, createInstance, … | Desktop (all data), Cadastre, MCServer |
| **IntegrationService** | `com.ils.lrs.webservice.integration.IntegrationServiceSOAPImpl` (`lrs-mc-integration.jar`) | SaveEntity, SaveJob, GetJob, GetParcelPointsData | Desktop (Cadastre jobs), Cadastre / MCServer (writes) — see 07 |
| WorkflowWebService | `com.ils.sdms.workflow.webservice.WorkflowServiceSOAPImpl` | task/process operations | Desktop |
| ReceptionService | `com.ils.lrs.webservice.reception.ReceptionServiceSOAPImpl` | AcceptApplication, SyncData, Withdrawal | Desktop |
| CashierService | `com.ils.lrs.webservice.cashier.CashierServiceSOAPImpl` | SubmitInvoice, CheckStatus, GetSchema, CalculateFee, GetPayment | Desktop → Cashier (07 §4) |
| FeeService | `com.ils.lrs.fee.FeeServiceImpl` | calculateFee, calculateFeeByEmptyProperty | Desktop |
| ReportsService | `com.ils.sdms.reports.webservice.ReportsServiceSOAPImpl` | addReport, deployXML, produceReport, removeReport, renderReport, retrieveReport, runReport | Desktop (printing documents), admin UI |
| MessagingService | `com.ils.sdms.messaging.webservice.MessagingServiceSOAPImpl` | createMessenger, sendMessage, receiveMessage, releaseMessenger | Desktop, Cashier web app |
| HibernateDataAccessService | `com.ils.sdms.data.hibernate.webservice.HibernateDataAccessServiceSoapBindingImpl` | getQueryMetaData, getQueryResultSet | BIRT Hibernate ODA driver (reports) |

Authentication of every call except `startSession`: WS-Security UsernameToken (user + **ticket** as
password, `PasswordText`) with a `wsu:Timestamp` — the server rejects messages outside its clock window
(Registry clock ~+19 min fast; the Desktop then tells users to "verify current server time at
`http://<host>/lrswa`", `$D/ThomsonReuters.Aumentum.Registry.Services.dll/...WebService.Common/ServiceAgentBase.cs`,
resource `errTimeSynchronization`). Ticket settings: `authentication.ticket.ticketsExpire=false`,
`expiryMode=AFTER_FIXED_TIME`, `validDuration=PT1H` → **tickets never expire** until `endSession`
(`$APP/conf/ils/lrs/repository.properties`). `lrs-config.properties`: `web.service.session.timeout=7200`.

### 2.4 The admin web UI (`/lrs/faces/...`) — feature list

Reconstructed from the admin message bundle `lrs.war/WEB-INF/classes/alfresco/extension/webclient.properties`
(1,055 keys, "AUMENTUM Registry (Nigeria)"), the JSF/DWR bean classes in `$J/lrs-web-admin.jar`
(`com.ils.lrs.web.bean.*WebBean`, `com.ils.lrs.model.bean.*Bean`) and `$J/lrs-web.jar`. The JSPs/XHTML
pages themselves were **not captured**, so layouts and exact navigation are unknown.

**Dashboards** (titles from `dashboard_*_title`):

| Dashboard | What the user sees / does | Backing bean (methods) |
|---|---|---|
| **Welcome** ("AUMENTUM Registry (Nigeria) - Welcome") | System information panel: System name, System time, users online / authenticated users, number of users and groups, number of transactions (pending, registered, rejected-not-relodged), number of workflows, number of report forms and other reports, licences, active office, **Diary Book** status | `WelcomeWebBean`: getSystemInfo, getAuthenticatedUsers, getDiaryBookStatus, getActiveOffice, getUsersNumber, getUsersGroups, getTrnsNumber, getWorkflowsNumber, getFormsNumber, getOtherReportsNumber, getLicencesNumber, getPendingTrnsNumber, getRegisteredTrnsNumber, getRejectedNotRelodgedTrnsNumber |
| **Indexer** ("AUMENTUM Registry (Nigeria) - Indexer") | Registration queue/indexer view; queue limit is `lr_setting bounds.QueueLimit = 40000` | (JSP not captured) |
| **Workflow** ("System Dashboard": "manage workflows, tasks, started processes or run system commands") | My Active tasks, My tasks, My pooled tasks; task details; **Assign/Reassign the task to another user**; Start/End date | `WorkflowWebBean` getWorkflow, getWorkflowTasks, doAction |
| **Reports** | Tabs **Management** (registration, generation) and **My Reports** (generated reports); wizards *Registration Report* (upload template zip, properties, category Forms/Others, localisation resources, **report permissions** per user/group), *Generate Report* (parameters, output name/format), *Scheduler* (task properties; mode One time only / Daily / Weekly / Monthly; start time; template parameters; **Report Subscribers** — users/groups receive the document **by e-mail**) | `ContentReportsWebBean`/`ContentReportsBean`: getReports, getMyReports, getScheduledReports, produceReport, produceProcessingReport, getReportParams, getOutputFormats, getCategoryList, getReportToScheduleList, doCRUDActionOnObject (create/update/remove schedule, template, template folder); `ReportManager`, `ScheduledReportInfo`, `CronExpression` |
| **Administration** ("AUMENTUM Registry Administrator") | see next table | — |

**Administration functions** (keys `*_management`, `manage_*`):

| Function | Details | Bean / tables |
|---|---|---|
| **Users** (Users, User Details, User Groups, Users Online) | create / validate user, update, delete (refused if user has assigned tasks), change password (own / admin), change username; fields first, middle, last name, e-mail, telephone, job title, organisation, office, administrator flag, account enabled; report permissions per user; **export / import users** | `UserMngWebBean`, `UserMngBean` (createUser, updateUser, changeUserPassword, deleteUser, userHasAssignedTasks, doExport, doImport, getOffices); Alfresco people + `alf_authority` |
| **User Sessions** | list sessions (creation time), delete selected sessions | `UserSessionsWebBean.deleteUserSessions` |
| **Groups** | create/edit/delete group, members (available/selected lists); `groups_err_exists` | `GroupsWebBean`, `GroupsBean` |
| **Permissions** (Manage Workflow Permissions, perms) | named permissions with authorities (users/groups) and a **strategy**: *Straight* (Read, Write, All on records), *View Access*, *Print Access*, ***Web Access*** ("use View, Print access to documents") | `PremsWebBean`, `PermsMngBean` |
| **Dictionaries** | Dictionary Groups (view), Dictionary Group Terms (add/edit/remove; refused if used) | `DictionaryWebBean` → `lr_dictionary` |
| **Option Categories / Option Items** ("AUMENTUM Registry Administrator Options") | edit the settings table by category | `SettingsWebBean` (searchCategories, searchSettings, getSetting) → **`lr_setting`** |
| **Extended Attributes** | add/edit/remove extended attributes | → `lr_extension_link` and `*_ext` tables (inferred) |
| **Registration Offices** | office name, code, digital code, type (registration/cadastral), address, registrar (head name), phone, fax, e-mail, web site, central (main) flag, active, search limit, queue limit, comments | `OfficesWebBean` → `lr_registry_office` (1 row: `PLT`, digital code 10, "Plateau State") |
| **Transactions** (Manage Transaction MetaInformation) — Registration / Cadastral / Pledge / Other transactions | add/edit/remove transaction types (refused if used by a workflow), workflow name, group codes, Cashier transaction code, "Cashier Transaction Code Based On Source Property" | `TransactionWebBean` → `lr_transaction_metadata` (42 rows) |
| **Calendar** | working week, periods (24 h), non-working days / holidays (add/edit/delete) | `BusinessCalendarWebBean` → `lr_business_calendar` (8 rows) |
| **Workflow definitions** | add (upload jBPM process archive zip + properties), update, list tasks, workflow permissions, add workflow aspect to a node | `WorkflowWebBean`; `JBPM_*` |
| **Storages / Filestores** | manage physical storages, hard disks and their locations, tape catalogues; **multi-filestore clusters**: create cluster, add/modify/delete filestore (statuses Started/Stopped/Suspended/Off mode; e-mail to admin when a filestore stops/starts) | `multiFileStores-service.properties` (`mtfs.*`), `bootstrap-filestores.properties` |
| **License Details** | licence type (Enterprise/Trial/Development/Commercial/Evaluation), expiry, licensed users; offline activation, register/upgrade/deactivate licence (e-mail to ILS) | `LicenseWebBean`; `ils/messages/licensing.properties` |
| **Audit / Logs** | tabs **Server Log**, **Workflow Log**, **Registry Log** (security log: user, action); search log; log file menu (errors "Log file … has very big size") | `AuditWebBean` (searchLog, getSysLogList, searchSystemLog), `AuditBean` |
| **System information / configuration / content** | `sys_info`, `sys_config`, `sys_content`, `system_users` | (JSPs not captured) |
| **Fee configuration** | group `GROUP_LRS_FEE_CONFIGURATION` exists in `alf_authority` → fee structures/rates are maintained by that group; UI not captured | `lr_fee_structure` (55), `lr_fee_rate` (29,003) |

Password policy messages exist (`ils/messages/password-control.properties`: length, capital, small,
numeric, special, no spaces, not login, not reversed login, not e-mail, no 3 repeating characters);
which rules are enabled is not captured (`lr_setting config.RestrictChangePswd` exists, value redacted).

### 2.5 Reports (bootstrap report definitions)

The `.rptdesign` files are stored in the Alfresco repository (bootstrapped from
`lrs-report-bootstrap-1…4.xml`, `-r11.xml`, `-r12.xml`, recorded in `LRSAdmin.sdms_bootstrap`, versions up to 7).
Only the **English label bundles** of eight standard reports were captured
(`$APP/deploy/lrs.war/WEB-INF/classes/lrs/bootstrap/reports/*_en.properties`):

| Report | Description (from bundle) | Parameters | Columns / content |
|---|---|---|---|
| Applicant Register | (no description) | not stated (inferred: period) | Name, ID Document, Address, Country, Phone; totals; "No transactions were found for specified parameters" |
| Content Summary | summary statistic on transactions registered, grouped by transaction type | Period start date, Period end date | Transaction Type, Count, Total |
| Instruments Ageing | number of transactions in queue grouped by task | Workflow | Task, Total Count, Duration Min/Max/Average, Number of transactions by ageing (days 1-5, 6-10, 11-20, >20); chart "Number of transactions by tasks" |
| Instruments Ageing Completed | number of tasks processed for completed transactions over a period | Period start date, Period end date | same columns as Instruments Ageing |
| Pending Transactions | list of pending transactions with ageing, grouped by transaction type | not stated | Type, Transaction No., Applicant, Creation Date, Current Stage, Transaction Duration (d/h/m); totals per type |
| Staff List Report | users registered in the system | none | Login, Name, E-Mail, Organization, Group; totals by group |
| Staff Productivity Detailed | transactions processed over a period | Period start date, Period end date | per User / Process: Transaction No., Task, Applicant, Date, Duration |
| Staff Productivity Summary | summary statistics about transactions processed over a period | Period start date, Period end date | per User / Process / Task: Transaction Type, Count |

The PLAGIS document templates (C of O, letters, demand notices, title deed outputs …) are also BIRT
reports run by `ReportsService` from the Desktop; they are covered in the Desktop documents. The
report bootstrap schema `ils/bootstrap/report/report-bootstrap.xsd` and classes in `$J/ils-reports.jar`
(`ReportDefinition`, `ReportParamType`, `ReportDateRangeParamType`, `ReportListParamType`,
`ScheduledReportDefinition`, `ScheduledReportSubscriberType`, `ReportPolicyType`, `ReportPageLayout`, `Margins`)
show what a report definition holds: parameters (simple, date range, list), policies (permissions),
resources (localisation bundles), page layout/margins, and for scheduled reports: parameters
(text/int/date/boolean) and subscribers. SSRS (`ReportServer` DB) holds only its root folder: **not used**.

### 2.6 Document storage

| Item | Value | Source |
|---|---|---|
| Content root | **`D:/LRS_STORAGE/`** (`dir.root`) | `$APP/conf/ils/lrs/repository.properties` |
| Content store | `D:/LRS_STORAGE/contentstore` (files); deleted content `contentstore.deleted`; audit content `audit.contentstore` | same |
| Indexes | `D:/LRS_STORAGE/lucene-indexes`, backups `backup-lucene-indexes`; index recovery `VALIDATE`; tracking every 5 s | same |
| Metadata | Alfresco tables in **LRS43 schema `LRSAdmin`**: `alf_node` 3,768,763; `alf_node_properties` 41,420,196; `alf_content_url` 2,104,102; `alf_content_data` 2,115,723; `alf_child_assoc` 3,867,209 | `$R/lrs-inventory-REGISTRY-20261101-1701.txt` |
| Registry stores | `lrs://rootStore` with `lrs:tempLodgments`, `lrs:permanentLodgments`, `lrs:permissionsRoot`; document templates folder `/app:company_home/cm:dss_content/cm:doc_templates` | `lrs/lrs-model.properties`, `lrs/lrs-config.properties` |
| Scanned documents model | namespace `http://www.landsystems.com/lrs/model/core/1.0`: types scanned document, supporting scanned document, outgoing scanned document, scanned image; properties parent id, entity id, entity data, page count; image page index, page name, size, height, width, bpp, format, thumbnail, annotation, **isattachment**, image info; association image list | `$J/lrs-mc-integration.jar` `IntegrationServiceImpl` constants |
| Multi-filestore | ILS "mtfs" clusters of filestores (default cluster "(default)", main filestore; bootstrap store id `ffffffff-…`) | `ils/bootstrap/bootstrap-filestores.properties`, `ils/messages/multiFileStores-service.properties` |
| Orphan cleanup | `system.content.orphanCleanup.cronExpression=0 0 4 * * ?` (04:00 daily), `orphanProtectDays=1` | `repository.properties` |
| Upload limits | `lr_setting scan.MaxImageSize = 1024` (KB per scanned page), `scan.MaxAttachmentSize = 100` (MB per attachment) | `$DATA/lr_setting` |

Replica note: the scanned pages and attachments (2.1 M content URLs) live as Alfresco content files
under `D:\LRS_STORAGE\contentstore\YYYY\M\D\H\m\<uuid>.bin` (standard Alfresco 3.4 layout, **inferred**),
referenced by `alf_content_url.content_url` (`store://…`). The replica can read them directly from that
folder plus the `alf_*` metadata, or keep using `/lrs/download` and `/lrs/upload` while JBoss runs.

### 2.7 Security model

| Aspect | Value | Source |
|---|---|---|
| User store | Alfresco internal (`authentication.chain=alfrescoNtlm1:alfrescoNtlm`): people nodes in `user://alfrescoUserStore`, authorities in `LRSAdmin.alf_authority` (188 rows). LDAP/AD, Kerberos, passthru subsystems present but not in the chain. Guest login disabled. User names case-insensitive. Sync cron `0 0 0 * * ?` (only relevant for external directories). | `repository.properties`, `subsystems/Authentication/*`, `default-synchronization.properties` |
| Roles | `ROLE_ADMINISTRATOR`, `ROLE_OWNER` (Alfresco); admin pages gated by `AdminAuthenticationFilter`; `UserMngBean.isAdministrator` | `$R/...-1701.txt` `alf_authority`, `web.xml` |
| Groups (sample from `alf_authority`) | `GROUP_EVERYONE`, `GROUP_LRS` (all Registry users; GISViewer allows `WebServicesAccess`/`CursorCoordinates` to it), `GROUP_LRS_ADMINISTRATION_SYSTEM`, `GROUP_LRS_FEE_CONFIGURATION`, `GROUP_Survey Department`, `GROUP_Scanning Operator`, `GROUP_Scanning Clerk`, `GROUP_Scanning Officer`, `GROUP_Postpone Clerk`, `GROUP_Withdrawal Clerk`, `GROUP_Intake Clerk`, `GROUP_Cashier`, `GROUP_Deeds Registrar`, `GROUP_Lands Officer`, `GROUP_Delivery Clerk`, `GROUP_Recommendation Creator`, `GROUP_Inspection Officer`, `GROUP_Search Officer`, `GROUP_Director of Lands`, `GROUP_Rejection Letter Creator`, `GROUP_Rent Officer`, `GROUP_Map Compilation Officer`, `GROUP_Vectorization Clerk`, `GROUP_Title Deed Plan` (the last four from `docs/registry-integration.md` / Cadastre sign-in). Authorities without `GROUP_` such as `Surveyor General`, `Director of Town Planning`, `Records Clerk`, `Correction Officer` look like role/zone names (inferred). Service-type accounts `MCConfig`, `AumentumWS` exist. | `$R/lrs-inventory-REGISTRY-20261101-1701.txt` (`## LRSAdmin.alf_authority`) |
| Workflow permissions | jBPM swimlanes resolved to groups (`JBPM_POOLEDACTOR` 7.06 M rows, `JBPM_TASKACTORPOOL` 12.6 M) | same |
| Web Access permission | a named permission with strategy **"Web Access: use View, Print access to documents"**; a user without it gets "You don't have permission to access Web Access" | `webclient.properties` (`perm_strategy_web_access`), lrswa `messages.properties` (`login_no_permission`) |
| Document access type | dictionary `access_type_public_access` / `access_type_authorized_only`; default for IncomingDocument / SourceDocument / OutgoingDocument = **public access** | `$DATA/lr_setting` category `defaults` |
| Sessions | `lrs.war` HTTP session 60 min; `lrswa.war` 30 min; SOAP tickets do not expire | web.xml files, `repository.properties` |
| Transport | **HTTP only** on 8080 (HTTPS connector commented out); SQL Server allows TLS 1.0 only (known) | `server.xml` in `$INV` |
| Password rules | see §2.4 | `password-control.properties` |
| Licensing | Alfresco/ILS licence checked on calls (`org.alfresco.service.license.LicenseException` handled by the Desktop) | `$D/.../ServiceAgentBase.cs` |

### 2.8 Scheduled jobs, timers and background processing

| Job | Schedule / trigger | What it does | Source |
|---|---|---|---|
| Alfresco index tracking | `0/5 * * * * ?` | keeps Lucene indexes in step | `repository.properties` |
| Orphan content cleanup | `0 0 4 * * ?` | removes unreferenced content files after 1 day | same |
| OpenOffice test | `0 * * * * ?` | probes the OOo transformer (stock) | same |
| User-registry sync | `0 0 0 * * ?`, on start-up | only for external user directories (none in chain) | `default-synchronization.properties` |
| Quartz | `MonitoredRAMJobStore` (in-memory), thread priority 3 | Alfresco/ILS scheduler | `alfresco/enterprise/repository-quartz.properties`, `alfresco/domain/quartz.properties` |
| **Scheduled reports** | per report schedule (one time / daily / weekly / monthly) | `com.ils.sdms.reports.scheduler.ScheduledReportJob` runs a report and e-mails subscribers from `reports.service.scheduler_from_address=ScheduledReports@landsystems.com` | `$J/ils-reports.jar`, `lrs/lrs-config.properties` |
| **Workflow event listener** | JMS messages (JBossMQ, port 8093) on task-create / task-assign / task-end / task-suspend / task-resume / timer | `com.ils.lrs.scheduler.WorkflowMessageListener`: writes the workflow log (`com.landsystems.lrs.model.ProcessHistory`, i.e. `lr_process_history`, 976,928 rows), creates **assignment** and **overdue/reminder** notifications, sends **e-mail** to the assignee or group (`notifyByEmail`, JavaMailSender) | `$J/lrs-web.jar` (strings `lrs.service.mail_assign_to_user_subject` etc.) |
| **Cashier listener** | JMS messages `invoiceCommit` / `invoiceVoid` with `invoiceNumber` | `CashierMessageListener`: finds the workflow task waiting for that invoice (`@lrs\:invoiceNumber:`) and moves it along the configured transition; updates financial history | `$J/lrs-web.jar` |
| Dispatcher | JMS | `GeneralMessageListener` (delegates by operation name, delay period) | `$J/lrs-web.jar` |
| Notification scheduler (overdue/expired/lapsed/cancelled transactions) | `lr_setting bounds.NotifyLastExecutionDate = 1/27/07 4:00 AM` | messages `lrs.service.evt_overdue`, `evt_expired`, `evt_lapsed`, `evt_canceled`; **the date shows it has not run on this installation** | `$DATA/lr_setting`, `lrscore-service.properties` |
| jBPM timers | `JBPM_JOB` = 0 rows | no pending jBPM timers/async jobs | `$R/...-1701.txt` |
| SQL Server Agent | service stopped (manual) | no SQL jobs (backups are not automated by Agent; last LRS43 full backup 2026-10-06) | `$INV` |

`lr_process_event` has 0 rows: event records (`createEventRecord`) are not used in PLAGIS (inferred).

### 2.9 Mail

The outbound SMTP subsystem is at the Alfresco **defaults** (`mail.host=smtp.alfresco.com`, port 25,
user `anonymous`, `mail.from.default=alfresco@demo.alfresco.org`; `outboundSMTP.properties`), the
inbound SMTP server is disabled, IMAP disabled. The LRS mail sender used by the workflow listener and
scheduled reports is configured in a Spring context that was not captured. With the defaults above,
**e-mail notifications most likely do not reach anyone (inferred)**; no mail settings exist in `lr_setting`.

### 2.10 Other files in `lrs.war/WEB-INF/classes`

| File | Content |
|---|---|
| `lrs/lrs-config.properties` | report/notify from-address, reports service host `localhost:8080/lrs`, `db.default.schema=LRSAdmin`, `web.service.session.timeout=7200`, templates folder, `format.date=MM/dd/yyyy` |
| `lrs/calendar/business-calendar.properties` | Mon–Fri 9:00–12:00 & 13:00–18:00, Sat/Sun off; day = 8 h, week 40 h, month 21 business days, year 220 |
| `lrs/messages/lrscore-service.properties` | service errors, Property-number rules (`XXXXDDDDDDD` = office code + 7 digits), event texts, registration workflow step names (Intake, Pay Registration Fee, Initial Verification, Scan Incoming Documents, Data Entry, Final Verification, Scan Outgoing Documents, Delivery, Reject/Cancel/Redo/Relodge/End Registration), Cashier errors, mail subjects/bodies, delivery notice "Please come to the Ministry of Lands and take your documents." |
| `lrs/messages/lrs-mc-service.properties` | `lrs.service.mc_err_connect_server=The connection to ILS MultiCadastre server could not be established.` |
| `lrs/domain/schemas/lrs-ladm-core.xsd`, `lrs-extension.xsd`, `refactored.xsd` | the LADM-based domain model XML schema (JAXB) used by `DomainModelService` (covered in the data-model document) |
| `lrs/bootstrap/data/bootstrap-entity.xsd`, `ils/bootstrap/{ddl,groups,report,workflow}/*.xsd` | schemas of the bootstrap files (data, DDL updates, groups, reports, workflows) — the bootstrap XML files themselves are inside jars/DB, not captured |
| `ils/workflow-model.properties` | workflow action handlers (empty), `autoassign=false` |
| `alfresco/workflow/*` | stock Alfresco jBPM sample processes (adhoc, review, parallel review, invitation, WCM submit) and their messages — not PLAGIS workflows |
| `alfresco/messages/*` (413 files) | stock Alfresco Explorer messages in many languages |
| `WEB-INF/liferay-plugin-package.properties` | `speed-filters-enabled=false` (the WAR can also be deployed in Liferay; not used here) |

---

## 3. Registry Web Access (`lrswa.war`)

### 3.1 Technology and configuration

| Item | Value | Source |
|---|---|---|
| Framework | Spring MVC `DispatcherServlet` "LrsWebClient" (`/WEB-INF/LrsWebClient-servlet.xml`), URL pattern **`*.do`**; root contexts `/WEB-INF/applicationContext-hibernate.xml` and `/WEB-INF/applicationContext-security.xml` | `$APP/deploy/lrswa.war/WEB-INF/web.xml` |
| Security | **Spring Security** (`springSecurityFilterChain` `DelegatingFilterProxy` on `/*`; `HttpSessionEventPublisher` → concurrent-session control possible). Session timeout **30 min**. | same |
| Data access | **Direct Hibernate to LRS43** through datasource `IlsLRSWADS`, with its own copy of the domain mappings `$J/lwa-hibernate-model.jar` (`com.landsystems.lrs.model.*` classes + `.hbm.xml`, incl. `ext/*Ext`, `McJob`, `Setting`, `FormField`, `SiteAnalysis`, `SurveyPlan`, `GroundRent`, `RentalAmountRevision`, `Payment*`, `ProcessHistory`, …). It does **not** go through the SOAP API for data. | `ils-lrswa-ds.xml`, `lrs/domain/hibernate/hibernate-cfg.properties`, jar listing |
| Repository access | Alfresco web-service client to `repository.location=http://localhost:8080/lrs/api` (tickets, document content) | `lrswa.war/.../alfresco/webserviceclient.properties` |
| Service API | `com.ils.lrs.webaccess.WebClient` (`$J/lwa-domain.jar`): `getDictionaryCategory(category[, parent])`, `getSettings(category)`, **`getFormFields()`** (the `lr_formfield` table), `searchCount(entity, criteria)`, `search(entity, criteria)`, `getEntity(class, id)`, `getEntityMap(class, id)`, `getAlfrescoSettings()`, `listEntities(class)`, `getSerializerType()` (JSONLib or Jackson). `WebClientRequestHandler`: `executeRequest(name, params)`, `search(hql, args…)`, `search(DetachedCriteria)`, paged `search(hql, first, max, args…)`, `searchCount(hql, args…)`, `getQuery(name)`. `WebClientRequestExecutor.execute(client, params)` — pluggable request handlers. | `javap` of `$J/lwa-domain.jar` |
| Front end | AJAX single-page client receiving JSON (serializer types JSONLib/Jackson); URL **hash routes** (`#detail:…`, `#result:…`, §3.3). Date formats `d/m/Y` and `d/m/Y g:i A` are ExtJS-style format strings → the client is very likely **ExtJS** (inferred). Requires JavaScript ("Active Scripting/JavaScript option must be on"). | `messages.properties` |
| Named queries | `lrs/queries.properties` is **empty** → searches are built dynamically from form fields (Criteria) (inferred) | `$APP/deploy/lrswa.war/WEB-INF/classes/lrs/queries.properties` |
| Localisation | `app.localization.project=Afghanistan-BF` (product template origin), `app.used.locales=en_US`; Dari (`fa_IR`) label present; `lr_setting webaccess.DefaultLanguage=en_US`, `ShowLanguageSelection=0` | `lrswa.properties`, `version.properties`, `$DATA/lr_setting` |
| Branding | "AUMENTUM Registry Web Access", "AUMENTUM Registry Nigeria (PLAGIS)", "About AUMENTUM Registry Web Access (Nigeria)" | `messages.properties` |

`lr_setting` category **`webaccess`** (all 7 rows, `$DATA/lr_setting.psv.gz`):

| Name | Value | Meaning |
|---|---|---|
| DefaultLanguage | `en_US` | default UI language |
| ShowLanguageSelection | `0` | language drop-down hidden |
| EnableSearchByBlankFields | `1` | searches with all criteria blank are allowed |
| EnableWildcards | `1` | wildcards allowed in criteria |
| NumberOfRecordsInSearchResultsList | `25` | default page size of every grid (overridable per table) |
| personNameFormat | `{firstName} {secondName} {lastName}, {lastName2}` | how person names are displayed |
| **EnableGuestUser** | **`0`** | **guest (anonymous) access is OFF** |

Related settings read by Web Access: `config.LwaUrl = http://{0}/lrswa/welcome.do` (`{0}` = Registry host), `bounds.SearchLimit = 100` ("maximal number of records returned during Registry Browser search"), `mcintegration.ShowMapInLWA = 1`, `LWAMapURL`, `GISPropertyParam = upin:UPIN`, `GISPropertyParamUnit`, `LWASearchSpatialUnit = Plateau_State_Parcels;upin:UPIN`, `TOCSearchCriteria = Region:Province;District:District;Plot Number:Lot` (map, see 07 §3.4); `config.HideNormalTransactionCode = LEAS,RLEA`, `config.HidePastTransactionCode = LEAS,RLEA,DCT,GRR,SPC,SEAR` (inferred to also filter Web Access lists).

### 3.2 Entry points and the deep-link contract (must be kept)

The Registry Desktop opens Web Access in the user's browser, already signed in, with these URLs
(decompiled client):

| Desktop place | URL it opens | Source |
|---|---|---|
| Queue (ShellForm, "Show in LWA") | `{LwaUrl}?locale={ui culture with _}&user={user}&ticket={ticket}#detail:Transaction:{transaction id}` | `$D/ThomsonReuters.Aumentum.Registry.RegistryDesktop.exe/.../ShellForm.cs` ~l.1627 |
| History view | `{LwaUrl}?user={user}&ticket={ticket}#detail:Transaction:{transaction id}` | `$D/ThomsonReuters.Aumentum.Registry.Controls.dll/.../Core/HistoryExtView.cs` ~l.690 |
| Transaction pages "view registered documents" | `http://{host}/lrswa/welcome.do?[locale={…}&]user={user}&ticket={ticket}#result:SourceDocument:Transaction.TransactionNumber={PLS…}` | `TransactionDeedPage.cs` l.740, `TransactionCorrectionPage.cs` l.658, `TransactionWithDocumentsPage.cs` l.1010 |
| Time-sync error | "Please verify current server time at `http://{host}/lrswa`" | `ServiceAgentBase.cs` |

So the replica's Web Access must:

1. answer **`/lrswa/welcome.do`** and **`/lrswa`** on the same host as the Registry API (port 8080 today);
2. accept **`user` + `ticket`** query parameters as single sign-on (validate the ticket with
   `AuthenticationService.isTicketValid` or its own session store), and `locale`;
3. understand the hash routes **`#detail:<Class>:<id>`** (at least `Transaction` by numeric id) and
   **`#result:<Class>:<property path>=<value>`** (at least `SourceDocument` filtered by
   `Transaction.TransactionNumber`);
4. show the server time on the landing page (the Desktop sends users there to check the clock) (inferred).

`LwaUrl` is read from `lr_setting` (`config` category) so the address can be changed in the database.

### 3.3 Web Access feature list (reconstructed)

Sources: `lrswa.war/WEB-INF/classes/lrs/messages/messages*.properties` (215 keys),
`lrs/forms/messages/wf*.properties` (search object titles and field labels), the `*LWA` form definitions
in `$DATA/lr_formfield.psv.gz`, the `WebClient` interface, and `lr_setting`. The JSPs and controller
classes were **not captured**, so the page list below is a reconstruction; marked (inferred) where
the grouping is a deduction.

#### A. Sign-in, session, shell

| Feature | Details (labels quoted from bundles) |
|---|---|
| Login page | "Enter login details", User Name, Password, **Log In**, **"I'm guest"** link (shown only when `EnableGuestUser=1`; now 0). Errors: "Unable to login - unknown username/password.", "Your login attempt was not successful, try again.", "Reason:", **"You don't have permission to access Web Access"** (needs the Web Access permission, §2.7). Optional "Use crypto-server" (`use_crypt_server`; Desktop setting `SupportCryptoServer=False`). |
| Single sign-on | `welcome.do?user=…&ticket=…` from the Desktop (§3.2) |
| Session expiry | "For security reasons you were logged off automatically. Please click on the link below to continue your work." / "Your session has expired." (30-min HTTP session) |
| Shell | logo "AUMENTUM Registry Web Access", "You are logged in as", Logout, Welcome, Language (hidden), Help (localised help folder `en-US`), About (vendor text, third-party components), "Loading...", "In Progress...", "No connection to Server." |
| Dashboard tabs | **Search** ("What do you want to search"), **Digital Documents**, **Reports**, **Administration**; "Return to previous view"; "Vertical layout"/"Horizontal layout"; "Activate form"; "Printer friendly version" |

#### B. Search

| Search object (from `wf.properties`) | Title / description | Criteria form → result grid (from `lr_formfield`) |
|---|---|---|
| **Transaction** | "Any legal agreement registered or being registered in the Registry" — sub-kinds **Registered Transaction** and **Transaction in Process** | `TransactionFormSearchLWA` (21 fields: Transaction Number, Transaction Type, Status, Created as, Application Date, Transaction Start Date, Registration Date, Date of Commencement, Expiration Date, Certificate No., Instrument Number, Volume, Page Number, Related Instrument/Volume/Page Number, Property Number, Old File No., State, Local Government Area, Cancelled/Inactive Instrument) → `TransactionGridLWA` (13 cols) or `TransactionGridWithPropertySourceTargetLWA` (adds Source/Target) |
| **Guest transaction search** | "Invalid search criteria. Please specify Type and Number of Transaction." | `TransactionGuestLWA`: Transaction Number (mandatory), Transaction Type (mandatory) |
| **Property** | "Land, building, apartment registered in the Land Registry" | `PropertySearchForm` (Property Number, UPIN, Plan Number, Block Number, Plot Number, Property Type, Property Area, Location, State, Local Government Area, Village) (inferred shared form) → `PropertyGridWithSourcePropertyLWA` (Property Number, UPIN, Property Type, Area, Measurement Unit, Plan Number, Block, Plot, Layout, Tracing No., Location, Property Area, LGA, Source/Target) |
| **Party** / **Person** | "Physical or juridical subject of legal transaction registered against real estate" / "Person Search" | `PartiesSearchForm` (Name, Party Type, ID Document, ID Document No., Street Name, Village, State, LGA, Country) → `PartiesSearchGrid` (inferred shared) |
| **Applicant** | "Any person, organization representative, or lawyer providing in person documents for the registration" | `SearchIndApplicantForm` / `SearchCorpApplicantForm` (inferred) → **`SearchApplicantGridLWA`** (Type, Address, Town/Municipal Division, State/Province, LGA) |
| **Document** (Supporting / Outgoing / BackFile) | "Document Search"; Supporting = "Documents provided by applicants or parties"; Outgoing; **BackFile Documents** "Documents imported during BackFile scanning process" | `SupportedDocGrid` (Document No., Document Type, Issued By, Date Issued) (inferred) |
| **Certificate of Title** | "Document proving legal status of title to a property according to the Land Registry" | (form not identified) |
| **Survey Plan** | "Survey Plan Search" | `SurveyPlanForm` (inferred) → **`SurveyPlanGridLWA`** (Plan Number, Survey Plan Type, Surveyor Type, Surveyor Name, Surveyor Plan Number, Area, Measurement Unit, Plots Amount, Layout, Tracing Number, Location, LGA) |
| **File** | "Files, Volumes and tracking history" | (form not identified) |

Search mechanics: **Basic Search** and **Advanced Search** (condition builder: Add/Remove condition,
operators Equal to, Not Equal to, Greater than, Greater or equal, Less than, Less or equal, Like,
Range, And/Or; date templates Today, Yesterday, This week, Last Week, This Month, Last Month,
This Year, Last Year; saved "Templates"), Search / Clear, "Search Parameters", "Found {0} items",
paging (First/Previous/Next/Last, "Page %s of %s", "Jump to", "Items per page"), sort column,
"Details". Validation: "Please enter at least one attribute to conduct search." (unless
`EnableSearchByBlankFields=1`), "Invalid search criteria. For valid search at least one field should be
specified!", guest rule above. Wildcards allowed (`EnableWildcards=1`). Results capped by
`bounds.SearchLimit=100` (inferred).

#### C. Detail views

| View | Sections / fields (labels from `wf.properties`; forms from `lr_formfield`) |
|---|---|
| **Transaction details** | General / General Information (`TransactionFormLWA`: Transaction Number, Transaction Type, Status, Application Date, Transaction Start Date, Transaction End Date, Registration Date, Execution Date, Instrument Number, Volume, Page Number, Related Transaction, Related Instrument/Volume/Page Number, Comments); Transaction meta information; **History / Transaction History / Registration History** (task, user, started, completed, duration); **Fees** (Calculated Fee, Paid Fee), **Payment / Payment Details**, **Outstanding Ground Rent** (`GroundRentGridLWA`: Ground Rent Year, Rental Amount, Amount Paid, Amount Outstanding); **Decisions/Notes**; **Applicants** / Applicant Details; **Parties** / Party Details; **Representatives** (`RepresentativeGridLWA`); **Documents** (Supporting, Outgoing, Primary Document); **Old Properties / New Properties** (source/target); **RRR / RRR Details** (`RRRAttributeLWA`: Right Type, Status, Date of Commencement, Expiration Date, Term years/months/days, Rental Amount, Revision Period, Consideration (+currency), Improvement Amount (+currency), Erection Period); **Survey Plan / Survey Details**; Lodgment form; Invoice Number; Created as; Task |
| **Property details** | Property Details (`PlotFormNonMandatoryLWA`: Land Use Category, Land Use Type, Description, Survey Date, Surveyor Name, Surveyor Plan Number, C of O Volume/Page/Instrument Number); **Site Analysis** (`SiteAnaFormNonMandatoryLWA`, 54 fields: road access/width/status, density, development status, topography, soil, distances to special features, proximities to utilities/services, set-backs, planning scheme, TPS-NOTPA number …); Property Parts; Property Registration Book; System PIN / System Property Number; transactions on the property; **Lineage of Title** (parent/child); **Show on Map** |
| **Party / Applicant details** | Full Name, First/Second/Last Name, Corporation Name, Party Type, Role, Capacity, Personal ID / ID Card / ID Number, addresses (street, city, postal code, LGA, state, country), phone; applicant properties |
| **Document details** | Document Properties, Document Content, Pages count, **image viewer** (thumbnails "Click an image thumbnail to view its full content", Show/Hide thumbnails, Zoom In/Out, Fit Page, Fit Width, Actual Size, First/Previous/Next/Last Page, Page #, Print Page, Print all pages, Print Setup; "Failed to load image to Image Viewer for it is not installed." → the viewer may be a browser plug-in (inferred)); "No Images Found For The Document", "No Documents Found". Document types shown with `dss_*` labels: Certificate Of Title, Lease Agreement, Sketch Plan, Survey Diagram, Green Contract, Property File (with Property #, Contract No./Date, Lease #, Certificate #, Survey Record #, Diagram #, Province, Row, Shelf) |

Access to documents follows the permission strategy (View / Print) and the document access type
(`access_type_public_access` vs `access_type_authorized_only`) (inferred).

#### D. Map tab (`map.do`)

Shown when `mcintegration.ShowMapInLWA=1` (it is). "Show on Map" from a property detail searches the map by
`GISPropertyParam = upin:UPIN` (Registry attribute `upin` → Cadastre field `UPIN`). The tab is
Aumentum Cadastre's **Silverlight GISViewer client**: it probes `{GISViewer}/Assets/s.gif`
("No connection to Cadastre Server." on failure, "Cannot display map."), loads
`{GISViewer}/Client/Client.xap` and map config **`web-access`** (`Conf/Maps/Web-Access.xml`:
`login="false"`, `allow="*"`, one dynamic layer "Parcels" (service 0), extensions script, featureclick,
scaleline, cursor, progressbar, tools, scale, layers, search, selection; editor off) and draws through
`GISViewer/gis.gate/*` (`docs/registry-integration.md` §6, `$R/lrs-export/map-capture/gisviewer/Conf/Maps/Web-Access.xml`).
Navigation labels in Web Access: Zoom In/Out, Print. Details of the replacement are in 07 §3.4.

#### E. Reports and Administration tabs

Labels exist ("Reports", "Administration", "Property Report", report transaction types "Search
Transaction", "Cadastral Transaction", "Reregistration", "Primary", "Secondary", "New Property",
right types ownership/encumbrance/preport/search/Cancellation/Change Name) but what they contain is
**not captured** (JSPs/controllers missing). The Desktop prints PLAGIS documents through ReportsService;
Web Access probably offers a property report / search report printout (inferred).

### 3.4 Who uses Web Access today

No capture shows Web Access traffic volumes. The Desktop links into it (§3.2) and the Registry
help/error texts point to it. Guest access is off, so all users are Registry users with the Web Access
permission (inferred).

---

## 4. Other applications on .5 (for completeness)

| Application | Technology | Notes | Source |
|---|---|---|---|
| **Aumentum Registry Cashier** (web app) | ASP.NET (WSE 3.0), IIS, Windows authentication, `httpRuntime maxRequestLength=40960`, `executionTimeout=240`; calls LRS `/lrs/api/AuthenticationService`, `MessagingService`, `PersonWebService`, `RepositoryService` on `localhost:8080`; bin: `ILS.Common.dll`, `ILS.Messaging.dll`, `ILS.UI2.dll`, `LRS.Common.dll` | Version 3.2.29.194; DB `Cashier` (Cashier DB version 3.0.85, "PLAGIS Cashier profile for clean installation for RR11"); see 07 §4 | `$R/lrs-export/map-capture/registry-client/Aumentum Registry Cashier/Web.config`, `$INV`, `-1701.txt` |
| **ILSCashSrv / ILSCash4LRS.asmx** | ASP.NET SOAP (`Cash4LRS`) | the Cashier service the LRS server calls (`config.CashierServiceAddress`) | `$DATA/lr_setting`, `$J/lrs-web.jar` |
| **Aumentum Registry Cashier Scheduler** | Windows service (.NET) | not decompiled; presumably posts invoice events to LRS (MessagingService → `CashierMessageListener`) (inferred) | `$INV` SERVICES |
| **Aumentum Registry Workstation** (Registry Desktop) | .NET WinForms, installed also on the server (`C:\Program Files (x86)\Aumentum Registry\bin`, `ExternalConfig`, `WorkflowConfig`, `WorkerAssess\Forms`) | client; covered in the Desktop documents | `$INV` |
| **Aumentum Registry Designer 4.3.53** | .NET | form/workflow designer (not captured) | `$INV` |
| **PLAGIS Analytics** | Metabase 0.41.5 (Java 8u211) on port **3000**, PostgreSQL 17 on 5432 | reads SQL Server locally (open connection to 1433) — a reporting consumer of LRS43 (inferred); see 07 §8 | `$INV` SERVICES / NETWORK |
| SSRS / SSAS / SSIS | SQL Server 2012 | installed and running, no content found in `ReportServer` (only root folder) | `-1701.txt` |
| ArcGIS 10.2.1 Desktop | Esri | `lr_setting gisintegration.extensionTasks = Manage Spatial Data,Review Spatial Data` (ArcMap extension tasks) — an ArcMap–Registry extension exists in the product; use at PLAGIS not evidenced (inferred) | `$DATA/lr_setting` |

---

## 5. Page / feature inventory

| # | Area | Page / feature | URL / entry | Data / service behind it | Must the replica keep it? |
|---|---|---|---|---|---|
| 1 | Web Access | Landing / welcome (shows server time (inferred)) | `/lrswa/welcome.do`, `/lrswa` | — | **Yes** (Desktop deep links, time-sync message) |
| 2 | Web Access | Login (user/password), guest link (off) | `welcome.do` → login (`*.do`, name not captured) | Alfresco authentication, Web Access permission | Yes |
| 3 | Web Access | SSO from Desktop | `welcome.do?locale=&user=&ticket=` | ticket validation | **Yes (contract)** |
| 4 | Web Access | Transaction search (registered / in process) | hash route `#search…` (inferred) | `lr_transaction` (+ metadata, status, ext) via `TransactionFormSearchLWA` | Yes |
| 5 | Web Access | Guest transaction lookup (number + type) | — | `TransactionGuestLWA` | Optional (setting off) |
| 6 | Web Access | Property search | — | `lr_spatial_unit`/`lr_parcel`/`lr_launit`/`lr_address` | Yes |
| 7 | Web Access | Party / Person / Applicant search | — | `lr_party`, `lr_person`, `lr_individual`, `lr_corporation`, `lr_applicant` | Yes |
| 8 | Web Access | Document search (supporting, outgoing, backfile) | `#result:SourceDocument:Transaction.TransactionNumber=…` | `lr_source_document`, `lr_incoming_document`, `lr_transaction_document`, Alfresco content | **Yes (contract)** |
| 9 | Web Access | Certificate of Title / Survey Plan / File search | — | `lr_survey_plan` (0 rows), certificates in documents | Yes (low use) |
| 10 | Web Access | Transaction detail (all tabs in §3.3 C) | `#detail:Transaction:{id}` | as above + `lr_process_history`, `lr_payment*`, `lr_ground_rent`, `lr_rrr`, `lr_transaction_note` | **Yes (contract)** |
| 11 | Web Access | Property detail, lineage, site analysis | `#detail:…` (inferred) | `lr_launit_lineage`, `lr_site_analysis` | Yes |
| 12 | Web Access | Document viewer (thumbnails, zoom, print) | — | Alfresco content (`/lrs/download` or content store) | Yes |
| 13 | Web Access | Map tab / Show on Map | `map.do` | GISViewer (Cadastre) | Yes (replace Silverlight) |
| 14 | Web Access | Reports tab, Administration tab | — | not captured | To clarify |
| 15 | Server | SOAP API (19 Axis services + CXF) | `/lrs/api/*`, `/lrs/cxfapi/*` | LRS43 + Alfresco | **Yes** while any old client (Desktop, Cashier, Aumentum .6, this Cadastre) remains |
| 16 | Server | Content upload/download | `/lrs/upload/*`, `/lrs/download/*`, `/lrs/d/*` | content store | **Yes** |
| 17 | Server admin | Welcome dashboard (system info, counts, users online, diary book) | `/lrs/faces/jsp/dashboard/dashboard.jsp` | `WelcomeWebBean` | Yes (new UI) |
| 18 | Server admin | Indexer dashboard | — | queue (`QueueLimit`) | To clarify |
| 19 | Server admin | Workflow dashboard (my/pooled tasks, reassign) | — | jBPM | Yes |
| 20 | Server admin | Reports management, generation, My Reports, scheduler with e-mail | — | ILS Reports / BIRT, Quartz | Yes |
| 21 | Server admin | Users (CRUD, password, import/export), Groups, User Sessions | `/lrs/faces/jsp/users/*`, `/groups/*` | Alfresco people/authorities | Yes (partly done: Cadastre "Users & Units" via `plagis_admin` procedures) |
| 22 | Server admin | Permissions (incl. Web Access strategy) | — | `alf_permission`/ACL | Yes |
| 23 | Server admin | Dictionaries, Option categories (settings), Extended attributes | — | `lr_dictionary`, `lr_setting`, `lr_extension_link` | Yes |
| 24 | Server admin | Registration offices | — | `lr_registry_office` | Yes |
| 25 | Server admin | Transaction metadata (registration/cadastral/pledge/other) | — | `lr_transaction_metadata` | Yes |
| 26 | Server admin | Business calendar / holidays | — | `lr_business_calendar` | Yes |
| 27 | Server admin | Workflow definitions upload/update | — | jBPM | Replace with replica's workflow admin |
| 28 | Server admin | Storages / filestores | — | mtfs | Replace (content store management) |
| 29 | Server admin | Licence details | — | ILS licence | Drop |
| 30 | Server admin | Audit: Server log, Workflow log, Registry (security) log | — | log files, `lr_process_history` | Yes (new audit) |
| 31 | Server | Background: workflow log + assignment e-mail, cashier invoice events, scheduled reports, orphan cleanup | JMS / Quartz | §2.8 | Yes (workflow log); e-mail optional |
| 32 | Server | JBoss consoles | `/jmx-console`, `/web-console` | — | Drop |
| 33 | Other | Cashier web app + ILSCash4LRS | IIS on .5 | `Cashier` DB | See 07 §4 |
| 34 | Other | PLAGIS Analytics (Metabase) | `:3000` | LRS43 (inferred) | Keep LRS43 compatible |

---

## 6. Notes for the replica build

1. **Two applications share one database.** `lrs.war` and `lrswa.war` both write/read LRS43 directly
   with Hibernate (`hbm2ddl=update`). Any replica running in parallel must coexist with that.
2. **Web Access is read-only and data-driven.** Search and detail screens are defined by `lr_formfield`
   rows (1,725 rows; the `*LWA` forms listed above), dictionaries (`lr_dictionary`) and settings
   (`lr_setting webaccess`, `mcintegration`). A replica that renders forms from `lr_formfield` reproduces
   the configured screens and keeps administration in the database.
3. **Keep the deep-link contract** (§3.2) or the Desktop's "Show in LWA" / "view registered documents"
   buttons break while the Desktop is still in use.
4. **Keep `/lrs/api/*` + `/lrs/upload` alive** (old JBoss or a compatible façade) until the Desktop,
   Cashier, Aumentum Cadastre (.6) and this repo's Cadastre no longer call them — see 07 for the exact
   operations each one uses.
5. Security debts to fix in the replica: HTTP only, tickets that never expire, default JBoss console
   accounts, documents public by default.

---

## 7. Gaps / still to capture

| Missing | Why it matters | Exactly what to fetch (from 10.10.10.5, read-only copy) |
|---|---|---|
| **lrswa JSPs, JavaScript and static files** | page list, layouts, the ExtJS client, map tab, hash-route grammar | everything under `C:\LRSAS\server\default\deploy\lrswa.war\` except `WEB-INF\lib` already taken: `*.jsp`, `*.js`, `*.css`, `*.html`, `images\`, `help\` |
| **lrswa Spring config** | controller → URL mapping (`*.do` names), security rules (who may log in, guest user, permission check), datasource/hibernate wiring, request executors | `lrswa.war\WEB-INF\LrsWebClient-servlet.xml`, `applicationContext-hibernate.xml`, `applicationContext-security.xml`, any other `WEB-INF\*.xml`, `WEB-INF\*.tld` |
| **lrswa controller classes** | the controllers and `WebClient` implementation live in `lrswa.war\WEB-INF\classes\**\*.class` (the captured jars only hold interfaces and model) | `lrswa.war\WEB-INF\classes\com\**` and any jars in `lrswa.war\WEB-INF\lib` not yet copied (only `lwa-domain.jar`, `lwa-hibernate-model.jar`, `commons-beanutils` were captured) |
| **lrs.war JSPs / XHTML and faces config** | admin UI pages and navigation | `lrs.war\jsp\**`, `lrs.war\*.xhtml`, `WEB-INF\faces-config-*.xml` (7 files), `WEB-INF\web-application-context.xml`, `WEB-INF\*-context.xml`, `WEB-INF\userctlr.taglib.xml`, `WEB-INF\sdms.taglib.xml` |
| lrs.war Spring contexts in jars/classes | mail sender bean (SMTP host), JMS queues, scheduler triggers, Cashier task names (`cashierTaskNames`), report scheduler | `lrs.war\WEB-INF\classes\alfresco\extension\*-context.xml`, `classes\**\*-context.xml`, `classes\lrs\**\*.xml`, plus the remaining `WEB-INF\lib\*.jar` (only 21 of them were copied) |
| Report bootstrap XML and `.rptdesign` templates | report parameters/queries, PLAGIS document templates | the `lrs-report-bootstrap-*.xml` inside jars or `classes\lrs\bootstrap\reports\`; and export the report nodes from Alfresco (Company Home → reports) via WebDAV `http://10.10.10.5:8080/lrs/webdav/` read-only |
| JBoss logs | real usage of Web Access (URLs called, users), errors | last few days of `C:\LRSAS\server\default\log\localhost_access_log.*.log` and `server.log` (masked) — the inventory's log section came back empty |
| IIS configuration | sites/apps/ports of Cashier and ILSCashSrv, bindings on 80/443 | `%windir%\system32\inetsrv\config\applicationHost.config` (mask credentials) and the ILSCashSrv `Web.config` |
| Full `alf_authority` and permission tables | complete group list, who holds the Web Access permission | `SELECT authority FROM LRSAdmin.alf_authority` (188 rows) + `alf_access_control_entry`/`alf_permission` joined (no personal data needed beyond user names) |
| Metabase / PostgreSQL purpose | whether "PLAGIS Analytics" depends on LRS43 views/tables | Metabase admin → Databases list (connection targets only, no passwords) |
| Silverlight `Client.xap` request log | to replace the Web Access map | the `gisviewer_passthrough` request log from this repo's server after Web Access is used against it |
