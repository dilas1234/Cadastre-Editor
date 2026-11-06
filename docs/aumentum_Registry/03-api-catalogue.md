# 03 — Registry API catalogue (LRS43 on 10.10.10.5)

This is the reference for every interface the Aumentum Registry server exposes, who calls it, and
what a replacement server must answer byte-for-byte so the old clients keep working during the
parallel run.

Path abbreviations used below:

* `$R` = `tools/aumentum-inventory` (read-only captures in this repo, ignored by git)
* `$W` = `$R/lrs-export/registry/app/LRSAS/server/default/deploy/lrs.war`
* `$D` = the ILSpy-decompiled .NET client (`dec/net/<assembly>/…` in the session scratchpad)
* `jar:` = a jar from `$W/WEB-INF/lib`, read with `unzip -p` / `javap -p -constants -c` only

Anything marked **(inferred)** is a deduction; the captures do not show it directly.

---

## 1. Transport and endpoints

### 1.1 The servlets (source: `$W/WEB-INF/web.xml`)

The Registry runs as one web app, `lrs.war`, under JBoss AS 4 / JBossWeb 2.0.1, on port 8080, plain
HTTP. A second web app, `lrswa.war` (Web Access), sits next to it. Session timeout is 60 min
(`<session-timeout>`). `web.service.session.timeout=7200` is set in
`$W/WEB-INF/classes/lrs/lrs-config.properties` (the ticket lifetime, inferred to be in seconds).

| URL pattern (under `/lrs`) | Servlet / class | Purpose | Auth |
|---|---|---|---|
| `/api/*` | `org.apache.axis.transport.http.AxisServlet` (Axis 1.x) | **All SOAP services** (§3). Service list is turned off (`axis.disableServiceList=true`); `?wsdl` serves the WSDL named in `<wsdlFile>`. | WS-Security per service (§2) |
| `/cxfapi/*` | `org.apache.cxf.transport.servlet.CXFServlet` | Apache CXF endpoints. The packaged `business-logic-service.wsdl` gives `…/lrs/cxfapi/BusinessLogicService`; the desktop's `RepositoryWebService_URL` points to `…/lrs/cxfapi/RepositoryWebService`. No CXF Spring config was captured, so what CXF actually publishes is not known. | (unknown) |
| `/cmis/*` | `CXFServlet` | Alfresco CMIS 1.0 web services (`CMISWS-Service.wsdl`, §3.4) | Alfresco |
| `/upload/*` | `org.alfresco.web.app.servlet.UploadContentServlet` | `PUT /lrs/upload/{name}?ticket=…&encoding=UTF%2D8`: raw upload of a page or file; the response body is the content URL that is then given in a `cm:content` property (§2.6) | `ticket` query parameter |
| `/uploadFileServlet` | `UploadFileServlet` | Alfresco web-client multipart upload | session |
| `/download/*`, `/d/*` | `DownloadContentServlet` | `GET /lrs/d/a/{store}/{id}/{uuid}/{name}?ticket=…` style content download (Alfresco 3.4 format, inferred). The desktop adds `&ticket=…` to the URL that `ContentService.read` returns (`ContentServiceAgent.GetContentAddress`). | `ticket` or session |
| `/dr` | `DownloadRawContentServlet` | raw store download (admin) | admin |
| `/guestDownload/*`, `/gd/*` | `GuestDownloadContentServlet` | guest download | guest |
| `/template/*`, `/t/*`, `/guestTemplate/*`, `/gt/*` | Template servlets | FreeMarker templates over nodes | session / guest |
| `/navigate/*`, `/n/*` | `ExternalAccessServlet` | Alfresco web-client deep links | session |
| `/command/*`, `/c/*` | `CommandServlet` | Alfresco web-client commands | session |
| `/ajax/*` | `AjaxServlet` | Alfresco web-client AJAX | session |
| `/faces/*` | MyFaces `FacesServlet` | Admin UI (`/lrs/faces/jsp/dashboard/dashboard.jsp`) | form login |
| `/dwr/*` | DWR `DwrSpringServlet` | admin UI AJAX | session |
| `/webdav/*` | `org.alfresco.repo.webdav.WebDAVServlet` | WebDAV over `workspace://SpacesStore` (`/app:company_home`) | basic |
| `/service/*`, `/s/*` | Spring Surf `WebScriptServlet` (basic auth) | Alfresco web scripts | HTTP basic / `alf_ticket` |
| `/wcservice/*`, `/wcs/*` | `WebScriptServlet` (web-client auth) | web scripts behind the web-client session | session |
| `/168service/*`, `/168s/*` | `WebScriptServlet` | portal web scripts | session |
| `/facebook/*`, `/fb/*`, `/fbservice/*`, `/fbs/*` | Facebook servlets | stock Alfresco; unused (inferred) | — |
| `/remoting/*` | Spring `DispatcherServlet` over `classpath*:*/remoting/*-services-context.xml` | Spring remoting (HTTP invoker); config not captured | (unknown) |
| `/remoteauth/*` | `com.ils.remoting.authentication.RemoteAuthenticationServlet` | ILS remote sign-in for remoting | (unknown) |
| `/proxy` | `com.ils.web.app.servlet.HTTPProxyServlet` | HTTP proxy (used by Web Access for the map, inferred) | session |
| `/downloadBPM` | `com.ils.web.app.servlet.ProcessDefinitionDownloadServlet` | `GET /lrs/downloadBPM?definitionId={id}&ticket={t}`: the jBPM process image/definition. The desktop opens this from the queue (`ShellForm.cs:1610`). | `ticket` |
| `/jbpm/deployprocess` | `JBPMDeployProcessServlet` | deploys a process archive | admin |
| `/workflowdefinitionimage/*` | `WorkflowDefinitionImageServlet` | process image | session |
| `/cmisbrowse` | OpenCMIS `BrowseServlet` | CMIS browser | — |
| `/linear`, `/pdf417`, `/datamatrix`, `/qrcode` | barcodelib servlets | barcode images for printed documents | none |
| `/` (`index.jsp`) | — | `GET http://{host}:8080/lrs/` with Windows credentials returns `username|TICKET_…` (first 128 bytes); this is the desktop's **single sign-on** (`AuthenticationServiceAgent.SignIn(host)`) | NTLM / default credentials |
| `/images/reports/{resource}` | static | report images (`ReportServiceAgent.cs:320`) | none |

The filter `com.ils.core.security.authentication.ClearThreadSecurityContextFilter` closes the HTTP
session after every call on `/api,/cxfapi,/cmis,/upload,/uploadFileServlet,/download,/d,/guestDownload,/gd`
("webservices creates a new http session for each remote call"), so the SOAP API is **stateless
apart from the ticket**.

### 1.2 Web Access (`/lrswa`)

`lrswa.war/WEB-INF/web.xml` maps one Spring MVC `DispatcherServlet`, `LrsWebClient`, to `*.do` and
`/*`. The controllers' `.do` names are not in the captured XML (they are in jars). What the desktop
opens (all with `user` + `ticket` in the query, so Web Access accepts a Registry ticket):

| URL | Opened from |
|---|---|
| `http://{host}/lrswa/welcome.do?locale={l}&user={u}&ticket={t}#detail:Transaction:{transactionId}` | `ShellForm.cs:1634`, `HistoryExtView.cs:696` |
| `http://{host}/lrswa/welcome.do?locale={l}&user={u}&ticket={t}#result:SourceDocument:Transaction.TransactionNumber={no}` | `TransactionDeedPage.cs:740`, `TransactionCorrectionPage.cs:658`, `TransactionWithDocumentsPage.cs:1010` |
| `http://{host}/lrswa` | shown in the clock-skew error text (§2.4) |

Web Access itself is covered in its own document; this file covers only the URLs other programs open.

### 1.3 Axis service → implementation map

Source: `$W/WEB-INF/server-config.wsdd` (the merged Axis configuration) and the `deploy.wsdd` files in
the jars (`unzip -p jar com/…/deploy.wsdd`). All services are `provider="java:SpringRPC"
style="wrapped" use="literal"` (document/literal wrapped).

| Service (URL `/lrs/api/…`) | Spring bean | Implementation class | Jar (deploy.wsdd) | WSDL served (`<wsdlFile>`) | Request handlers |
|---|---|---|---|---|---|
| `AuthenticationService` | `authenticationWebService` | `org.alfresco.repo.webservice.authentication.AuthenticationServiceSoapBindingImpl` | Alfresco (server-config only) | `/wsdl/authentication-service.wsdl` | `LocaleConfigHandler` only (**no WS-Security**) |
| `RepositoryService` | `repositoryWebService` | `org.alfresco.repo.webservice.repository.RepositoryServiceSoapBindingImpl` | Alfresco | `repository-service.wsdl` | Ticket, WSSecurity, `QueryConfigHandler` |
| `ContentService` | `contentWebService` | `…content.ContentServiceSoapBindingImpl` | Alfresco | `content-service.wsdl` | Ticket, WSSecurity |
| `AuthoringService` | `authoringWebService` | `…authoring.AuthoringServiceSoapBindingImpl` | Alfresco | `authoring-service.wsdl` | Ticket, WSSecurity |
| `ClassificationService` | `classificationWebService` | `…classification.ClassificationServiceSoapBindingImpl` | Alfresco | `classification-service.wsdl` | Ticket, WSSecurity |
| `ActionService` | `actionWebService` | `…action.ActionServiceSoapBindingImpl` | Alfresco | `action-service.wsdl` | Ticket, WSSecurity |
| `AccessControlService` | `accessControlWebService` | `…accesscontrol.AccessControlServiceSoapBindingImpl` | Alfresco | `access-control-service.wsdl` | Ticket, WSSecurity |
| `AdministrationService` | `administrationWebService` | `…administration.AdministrationServiceSoapBindingImpl` | Alfresco | `administration-service.wsdl` | Ticket, WSSecurity, QueryConfig |
| `DictionaryService` | `dictionaryWebService` | `…dictionary.DictionaryServiceSoapBindingImpl` | Alfresco | `dictionary-service.wsdl` | Ticket, WSSecurity, QueryConfig |
| `PersonWebService` | `personWebService` | `com.ils.core.person.webservice.PersonServiceSOAPImpl` | `ils-core.jar` | `person-service.wsdl` | Ticket, WSSecurity |
| `WorkflowWebService` | `workflowWebService` | `com.ils.sdms.workflow.webservice.WorkflowServiceSOAPImpl` | `ils-core.jar` | `workflow-service.wsdl` | Ticket, WSSecurity |
| `MessagingService` | `messagingWebService` | `com.ils.sdms.messaging.webservice.MessagingServiceSOAPImpl` | `ils-core.jar` | `messaging-service.wsdl` | Ticket, WSSecurity |
| `ReportsService` | `reportsWebService` | `com.ils.sdms.reports.webservice.ReportsServiceSOAPImpl` | `ils-reports.jar` | `reports-service.wsdl` | Ticket, WSSecurity |
| `HibernateDataAccessService` | `hibernateDataAccessWebService` | `com.ils.sdms.data.hibernate.webservice.HibernateDataAccessServiceSoapBindingImpl` | `ils-reports-das.jar` | `data-access-service.wsdl` | Ticket, WSSecurity |
| `DomainModelService` | `domainmodelWebService` | `com.ils.lrs.webservice.model.DomainModelServiceSOAPImpl` | `lrs-domain-model.jar` | `domainmodel-service.wsdl` | Ticket, WSSecurity |
| `IntegrationService` | `integrationWebService` | `com.ils.lrs.webservice.integration.IntegrationServiceSOAPImpl` | `lrs-mc-integration.jar` | `lrs-integration-service.wsdl` | Ticket, WSSecurity |
| `FeeService` | `FeeService` | `com.ils.lrs.fee.FeeServiceImpl` | `lrs-web.jar` | `fee-service.wsdl` | Ticket, WSSecurity |
| `CashierService` | `cashierWebService` | `com.ils.lrs.webservice.cashier.CashierServiceSOAPImpl` | `lrs-web.jar` | `cashier-service.wsdl` | Ticket, WSSecurity |
| `ReceptionService` | `receptionWebService` | `com.ils.lrs.webservice.reception.ReceptionServiceSOAPImpl` | `lrs-web.jar` | `reception-service.wsdl` | Ticket, WSSecurity |

"Ticket" = handler `CreateSpringTicketCallback` (`org.alfresco.repo.webservice.axis.TicketCallbackSpringHandler`);
"WSSecurity" = `org.apache.ws.axis.security.WSDoAllReceiver` with `action="UsernameToken Timestamp"`;
"QueryConfig" = `org.alfresco.repo.webservice.axis.QueryConfigHandler` (reads the `QueryConfiguration`
SOAP header). In the wsdd the Java method names are lower-case (`saveEntity`, `submitInvoice`,
`acceptApplication`) but the wire `qname` keeps the WSDL case (`ns:SaveEntity`, `ns:SubmitInvoice`,
`ns:AcceptApplication`).

Global Axis options (`server-config.wsdd` `<globalConfiguration>`): `sendMultiRefs=true`,
`disablePrettyXML=true`, `sendXMLDeclaration=true`, `sendXsiTypes=true`,
`enableNamespacePrefixOptimization=true`, `axis.sendMinimizedElements=true`,
`axis.servicesPath=/api/`, attachments via `org.apache.axis.attachments.AttachmentsImpl` (so
`ContentService.writeWithAttachment` takes a SOAP-with-attachments/DIME part, inferred). The admin
password parameter is present in the file: `<redacted>`.

**Response shape notes for a byte-compatible server** (from the Axis settings; inferred where noted):
responses start with `<?xml version="1.0" encoding="UTF-8"?>`, carry `xsi:type` attributes, and may use
`multiRef` elements (`sendMultiRefs=true`; for document/literal Axis normally does not emit multiRefs,
inferred). The .NET client uses `XmlSerializer` (`ThomsonReuters.Aumentum.Registry.Services.XmlSerializers.dll`),
which ignores unknown attributes and namespace prefixes but **requires the element names and
namespaces of the WSDL** (element-qualified where `elementFormDefault="qualified"`).

---

## 2. Authentication

### 2.1 Flow

1. `AuthenticationService.startSession(username, password)` → `AuthenticationResult{username, ticket, sessionid}`.
   No WS-Security header is needed (the service has no `WSSecurity` handler). Header `LocaleHeader`
   (`headers:LocaleConfiguration{locale}`) is optional; the desktop sends its UI culture
   (`AuthenticationServiceAgent.Initialize`).
2. Every other call carries a **WS-Security header** with a `Timestamp` and a `UsernameToken` whose
   **Username is the user and Password is the ticket**, sent as `PasswordText`.
   `CreateSpringTicketCallback` makes WSS4J validate the token's password as an Alfresco ticket
   (the callback resolves the ticket, not a password; behaviour of Alfresco 3.4, inferred from the class name).
3. `AuthenticationService.isTicketValid(ticket)` → `boolean` (used by the Cadastre to check a ticket
   handed to it by the desktop map, §8.3).
4. `AuthenticationService.endSession(ticket)` on sign-out (desktop retries once on failure).

Alternative single sign-on (desktop only): `GET http://{host}:8080/lrs/` with
`UseDefaultCredentials=true`, `User-Agent: ILS WS Client`, `Accept-Language`, `Accept-Charset: utf-8`;
if the first 128 bytes contain `|TICKET_`, the text before it is the user name and the rest the ticket
(`AuthenticationServiceAgent.SignIn(string host)`, `$D/net/ThomsonReuters.Aumentum.Registry.Services.dll/…Repository.Authentication/AuthenticationServiceAgent.cs`).

The **customer code** (`THOMSONREUTERS`) is not part of any SOAP message seen in the client; it
belongs to the licence (`org.alfresco.service.license.LicenseException:` faults are unwrapped by the
client, `ServiceAgentBase.HandleException`) and to the Cadastre's `RegParam` (see
`docs/registry-integration.md`).

### 2.2 How the Registry Desktop builds the header (WSE 3.0)

Source: `ServiceAgentBase.OnAuthenticationUpdated` (`$D/net/ThomsonReuters.Aumentum.Registry.Services.dll/…WebService.Common/ServiceAgentBase.cs`):

```csharp
_userToken = new UsernameToken(authArgs.Data.UserName, authArgs.Data.Ticket, (PasswordOption)2); // 2 = SendPlainText
_serviceProxy.Url = BuildConnectionUrl(authArgs.Data.Host, _serviceProxy.Url);  // swap host:port, keep path
_serviceProxy.RequestSoapContext.Security.Timestamp.TtlInSeconds = 300L;        // Expires = Created + 300 s
_serviceProxy.RequestSoapContext.Security.Tokens.Add(_userToken);
```

Every proxy is a `Microsoft.Web.Services3.WebServicesClientProtocol` (`*ServiceWse` classes), with
`EnableDecompression = true`. WSE 3.0 therefore puts this on the wire (shape of WSE 3.0 output;
element order and IDs are WSE's, inferred from the API, not captured on the wire):

```xml
<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/"
               xmlns:wsa="http://schemas.xmlsoap.org/ws/2004/08/addressing"
               xmlns:wsse="http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-wssecurity-secext-1.0.xsd"
               xmlns:wsu="http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-wssecurity-utility-1.0.xsd">
  <soap:Header>
    <wsa:Action>{SOAPAction}</wsa:Action>
    <wsa:MessageID>urn:uuid:…</wsa:MessageID>
    <wsa:ReplyTo><wsa:Address>http://schemas.xmlsoap.org/ws/2004/08/addressing/role/anonymous</wsa:Address></wsa:ReplyTo>
    <wsa:To>http://{host}:8080/lrs/api/{Service}</wsa:To>
    <wsse:Security soap:mustUnderstand="1">
      <wsu:Timestamp wsu:Id="Timestamp-…">
        <wsu:Created>2026-11-06T08:00:00Z</wsu:Created>
        <wsu:Expires>2026-11-06T08:05:00Z</wsu:Expires>
      </wsu:Timestamp>
      <wsse:UsernameToken wsu:Id="SecurityToken-…">
        <wsse:Username>{user}</wsse:Username>
        <wsse:Password Type="http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-username-token-profile-1.0#PasswordText">{ticket}</wsse:Password>
        <wsse:Nonce>…</wsse:Nonce>
        <wsu:Created>2026-11-06T08:00:00Z</wsu:Created>
      </wsse:UsernameToken>
    </wsse:Security>
  </soap:Header>
  <soap:Body>…</soap:Body>
</soap:Envelope>
```

A replacement server must therefore: accept and ignore `wsa:*` headers (WS-Addressing), honour
`mustUnderstand="1"` on `wsse:Security`, accept an optional `Nonce`, and validate the ticket.

### 2.3 How this repo builds it (`crates/cadastre-server/src/lrs.rs`)

`Session::security_header` builds the same header without WS-Addressing and without Nonce:

```xml
<wsse:Security xmlns:wsse="…secext-1.0.xsd" xmlns:wsu="…utility-1.0.xsd">
  <wsu:Timestamp wsu:Id="Timestamp-{uuid}"><wsu:Created>{now+offset}</wsu:Created><wsu:Expires>{+600 s}</wsu:Expires></wsu:Timestamp>
  <wsse:UsernameToken wsu:Id="SecurityToken-{uuid}"><wsse:Username>{user}</wsse:Username>
    <wsse:Password Type="…#PasswordText">{ticket}</wsse:Password><wsu:Created>{now+offset}</wsu:Created></wsse:UsernameToken>
</wsse:Security>
```

It posts `Content-Type: text/xml; charset=utf-8` and `SOAPAction: "{tns}/{operation}"` (quoted).
The time `now+offset` uses the Registry's clock: `clock_offset()` does `GET …/DomainModelService?wsdl` and
takes the HTTP `Date` header. This repo calls: `AuthenticationService.startSession` / `endSession`,
`AccessControlService.getParentAuthorities`, `AdministrationService.changePassword` / `createUsers`,
`DomainModelService.loadObjects`, `IntegrationService.SaveEntity`, and `PUT /lrs/upload/…`
(`lrs.rs`; `ticket_valid()` calls `isTicketValid`). Both clients put the ticket in the Password, so
the replacement must keep that convention.

### 2.4 The clock rule

WSS4J (`WSDoAllReceiver`, action `Timestamp`) rejects a message whose `Expires` is in the past by the
**server's** clock. The Registry runs about +19 minutes ahead of the PCs (earlier work). The desktop
uses its local clock with a 300 s TTL, so a PC more than ~5 min behind gets the fault text
"Invalid timestamp The security semantics of the message have expired" or "The timestamp could not
be validated"; the client turns it into its resource `errTimeSynchronization`: "There is a time
difference between the Client and Server. Please verify current server time at: http://{host}/lrswa
and correct your desktop time as needed." (`ServiceAgentBase.HandleException`,
`…WebService.Common.Properties.Resources.resx`). This repo avoids it by taking the time from the
Registry's `Date` header.

A new server should either accept a wide skew (recommended: Created up to ±30 min, ignore Expires
or allow skew), or keep WSS4J's rule and run with a correct clock. Note that the existing PCs are
set to the *wrong* (Registry) time in some places (inferred from the +19 min drift) — a new server
on correct time will reject their messages unless it allows skew.

### 2.5 Faults the client recognises

`ServiceAgentBase.HandleException` looks in `detail` for one of these elements, in the given
namespace, and shows its `message` child: `AuthenticationFault` (Alfresco auth ns),
`AuthoringFault`, `WorkflowServiceFault` (`http://www.landsystems.com/sdms/workflow/service`),
`RepositoryFault`, `AdministrationFault`, `AccessControlFault`, `ContentFault`, `DictionaryFault`,
`PersonServiceFault` (`http://www.alfresco.org/ws/service/person/1.0` in the client — note this
differs from the server tns `http://www.landsystems.com/core/person/service`), `ActionFault`,
`BusinessServiceFault` (`http://www.landsystems.com/business-service/1.0`), `ReceptionServiceFault`,
`CashierServiceFault`. Message text containing `org.jbpm.taskmgmt.exe.TaskInstance` → "can't load
task". `faultstring` containing "security processing failed" or "security token could not be
authenticated" → **session expired** (re-login). The replacement should produce these same
`faultstring` texts for an expired/unknown ticket so the desktop re-prompts instead of crashing.

### 2.6 Upload

`PUT http://{host}:8080/lrs/upload/{fileName}?ticket={ticket}&encoding=UTF%2D8`,
`Content-Type: application/octet-stream`, body = file bytes (`ContentServiceAgent.PutContentStream`,
`GetPutRequest`). The response body (ASCII, read up to 100 KB) is the content-URL string that the
client then writes as the `{http://www.alfresco.org/ws/model/content/1.0}content` property of a page
(`lrs.rs::incoming_document_xml` does the same). Alfresco 3.4's `UploadContentServlet` answers
`contentUrl=store://…|mimetype=…|size=…|encoding=…|locale=…` (inferred from Alfresco 3.4; this repo
treats the body as opaque and passes it back).

---

## 3. Service catalogue (every operation)

Sources: live WSDLs `$R/lrs-capture/wsdl/*.wsdl` (fetched from `http://10.10.10.5:8080/lrs/api/{Service}?wsdl`)
and packaged WSDLs/XSDs `$W/wsdl/*`. Parsed with a Python script over the WSDL/XSD.

**SOAPAction rule:** for all 145 operations of the 19 Axis services the SOAPAction is exactly
`{targetNamespace}/{operationName}` (checked by script, 0 exceptions). Every message is
document/literal wrapped: the body element is `<tns:{operation}>` with the listed children, and the
reply is `<tns:{operation}Response>`. Each service declares one fault, `{Service}Fault`
(`errorCode:int?`, `message:string`). In the tables `[]` = repeated (maxOccurs unbounded), `?` = optional,
`—` = empty element.

### 3.0 Live vs packaged WSDL

| Live (`lrs-capture/wsdl`) | Packaged (`lrs.war/wsdl`) | Difference |
|---|---|---|
| 19 files, named by service (`DomainModelService.wsdl` …) | same 19 under different names (`domainmodel-service.wsdl` …) + `business-logic-service.wsdl`, `LRSWebServices.wsdl`, `CMISWS-Service.wsdl`, `Alfresco-CMISWS-Service.wsdl` and the XSDs | **Same operations, same messages, same types** in all 19 (script diff: 0 operation or signature differences). Differences are only Axis re-serialisation (attribute order, whitespace, `<x></x>` vs `<x/>`) and the `soap:address`: live = `http://localhost:8080/lrs/api/{Service}`; packaged = `…/alfresco/api/…` (Alfresco services, Person), `…/sdms/api/…` (Workflow, Messaging, Reports, HibernateDataAccess) or `…/lrs/api/…` (LRS services). The `localhost` in the live files means the server never rewrites the address; clients replace host and port themselves. |
| — | `business-logic-service.wsdl` (`…/lrs/cxfapi/BusinessLogicService`) | Not in Axis; CXF. Not captured live. |
| — | `LRSWebServices.wsdl` (`http://172.25.31.93/ApplicationServerAdmin/WebServices/LRSWebServices.asmx`) | Outbound contract to the Cadastre (§8), a vendor test address. |

Imported schemas: `types.xsd` (Alfresco content model, ns `http://www.alfresco.org/ws/model/content/1.0`),
`cml.xsd`, `headers.xsd`, `lrs-service.xsd` and `lrs-service-types.xsd` (ns `http://www.landsystems.com/lrs/service`),
`workflow-types.xsd` (`http://www.landsystems.com/sdms/workflow/types`, **unqualified** elements),
`core-types.xsd` (`http://www.landsystems.com/ws/core/types`), `reports-service.xsd`
(`http://www.landsystems.com/reports/service`). Live WSDLs reference them relatively
(`../wsdl/types.xsd`, `lrs-service.xsd`), so `GET /lrs/wsdl/{name}.xsd` must also be served for a
client that re-imports the WSDL (inferred).

### 3.1 LRS (Land Registration) services — `com.ils.lrs.*`

#### DomainModelService — generic read/write of the Hibernate domain model (the main data API)

**DomainModelService.wsdl** — tns `http://www.landsystems.com/domainmodel-service/1.0`, address in file `http://localhost:8080/lrs/api/DomainModelService`

| Operation | Request fields | Response fields | Notes |
|---|---|---|---|
| `getRegisteredSchemas` | — | result: string[] |  |
| `getSchema` | schemaName: string | result: string |  |
| `loadObject` | Query: QueryType | Result: ResultType |  |
| `loadObjects` | Query: QueryType[] | Result: ResultType[] |  |
| `saveOrUpdate` | ResultCollection: ResultCollectionType | ids: long[] |  |
| `deleteObjects` | className: string, ids: long[] | — |  |
| `createInstance` | className: string, skipDefaults: boolean | Result: ResultType |  |
| `generateNumber` | counterName: string | numberValue: string |  |

Semantics (from the client, `DomainModelServiceAgent`, `QueryTypeFactory`, and server strings):

* `QueryType` is a **choice** of `SimpleQuery{queryString (HQL), queryParams?, maxResults?, @className?}`,
  `QueryById{entityClass, entityId{value, @type}}`, `SPQuery{queryString, queryParams?, @className?}`
  (stored procedure). `queryParams` = repeated `param{value, @type}`; the desktop puts the Java/XSD
  type name in `@type` (`"long"`, `"string"`) and the value as text; parameters bind to `?` in order.
* Result `ResultType` is a **choice**:
  * `ResultCollection{className, items{value: string[]}}` when `@className` is given: each `value` is
    one whole entity **serialised as XML text** (the HyperJAXB3 / JAXB form of the
    `com.landsystems.lrs.model.*` class; the hbm mappings use `org.jvnet.hyperjaxb3` accessors). The
    desktop deserialises it with `XmlSerializer` into its own DataModel classes (after
    `CorrectXmlFromJava` date fixes).
  * `ResultScalar{items{value: ScalarRecordType{items{value: ScalarValueType{type, value}}[]}[]}}` when
    no class name is given (projections). `type` is an XSD type name used by
    `RepositoryUtilities.ResolveNullableXSDType`; dates come back as `dd MMM yyyy HH:mm:ss`
    (`lrs.rs::parse_datetime`).
* `saveOrUpdate(ResultCollection{className, items[]})` takes the same XML strings back and returns the
  ids. The desktop serialises with `XmlSerializer` and rewrites dates to `yyyy-MM-ddzzz` /
  `yyyy-MM-ddTHH:mm:ss.fffzzz` (`DomainModelServiceAgent.CorrectXml4Dates`).
* `createInstance(className, skipDefaults)` returns a new entity (with defaults and ids filled server-side)
  as a one-item `ResultCollection`. Class name = `com.landsystems.lrs.model.{TypeName}` (`ClassNames.ResolveDataModelType`;
  extension classes `com.landsystems.lrs.model.ext.{TypeName}`).
* `generateNumber(counterName)` returns the next formatted number from `lr_simplecounter`
  (counters used by the desktop: `property_number`, `instrument_number`, `document_number`,
  `past_number`, `plan_number`, `suid_number`, `demandnotice_number`).
* `getRegisteredSchemas()` / `getSchema(name)` return schema names and XSD text for the classes (inferred).
* The desktop uses **`loadObject`** (one query per call); `loadObjects` (array) is what the Cadastre's
  MCServer and this repo use.

#### IntegrationService — Cadastre ↔ Registry integration (parcels, documents, MultiCadastre jobs)

**IntegrationService.wsdl** — tns `http://www.landsystems.com/lrs-integration-service/1.0`, address in file `http://localhost:8080/lrs/api/IntegrationService`

| Operation | Request fields | Response fields | Notes |
|---|---|---|---|
| `SaveEntity` | transactionId: long, entities: Entity[] | result: EntityCollection |  |
| `SaveJob` | entities: Entity[] | result: EntityCollection |  |
| `GetJob` | transactionId: long | result: EntityCollection |  |
| `GetParcelPointsData` | pUin: string | result: EntityCollection |  |

Types (`lrs-service-types.xsd`): `Entity{className, status: EntityStatus, properties: EntityValue{name, value~}[]}`,
`DocumentEntity` extends `Entity` with `content: DocumentContent`, `DocumentContent` extends
`EntityContent{status, reference?: Reference, properties: NamedValue[]}` with `pages: PageContent[]`,
`PageContent` extends `EntityContent` with `type: PageType (image|attachment)`,
`EntityCollection{items{value: Entity[]}}`. `EntityStatus` = `saved | added | modified | deleted | approved | rejected`.

Server behaviour (`jar:lrs-mc-integration.jar`, `com.ils.lrs.integration.service.IntegrationServiceImpl`, javap strings):

* `SaveEntity(transactionId, entities)`: loads `com.landsystems.lrs.model.Transaction` by id
  ("LRS Entity … by ID not found"); entities whose class is in setting `gisintegration/lrsParcelClass`
  (`com.landsystems.lrs.model.Parcel`) are added/modified/removed on the transaction's spatial units;
  classes in `gisintegration/lrsDocumentClass` (`com.landsystems.lrs.model.IncomingDocument`) become
  documents: Alfresco nodes are created under `workspace://SpacesStore`
  `/app:company_home/cm:dss_content/cm:scan_documents` of type `lrs:scanneddocument` /
  `lrs:supporting_scanned_document`, pages as children, versions created when setting
  `config/ScanVersioningSupported` = `1` (PLAGIS: `0`). Property names must exist on the class
  ("Invalid attribute name … for LRS Entity"); `Dictionary` values are looked up by code
  (`IMetaDataService.getDictionaryItem`); dates parse as `yyyy-MM-dd'T'HH:mm:ss`. Returns the saved
  entities with ids.
* `SaveJob(entities)`: class must be `gisintegration/lrsMcJobClass` (`com.landsystems.lrs.model.McJob`,
  table `lr_mc_job`). By `status`: `added` → needs `TransactionId`, `JobName`, `JobParameters`; looks up
  the Cadastre job type by name (`GetAllJobs`), calls `CreateJobInstance`, saves `McJob`;
  `modified` → `UpdateJobInstance`; `deleted` → `DeleteJobInstance`; `approved` → `ApproveJob`;
  `rejected` → `RejectJob`; the last two find the latest job of the transaction with
  `from McJob where TransactionId = ? and id=(select max(id) from McJob …)`. Setting
  `multicadastre/mcAsyncMode` is read (async creation, inferred).
* `GetJob(transactionId)`: `from McJob where TransactionId = ?`, each refreshed from the Cadastre
  (`GetJobInstance`) and returned with `Id, JobId, JobName, JobInstanceId, JobInstanceName, JobPriority,
  JobComments, JobParameters, TransactionId, JobStatus, JobStarted, JobUpdated, JobEnded`.
* `GetParcelPointsData(pUin)`: proxies the Cadastre's `GetParcelPointsData(pUpin)`; each point is an
  `Entity` with properties `FromPointName, FromNorth, FromEast, ToPointName, ToNorth, ToEast, Distance,
  Azimuth, Bearing`.

The exact `SaveEntity` payload this repo sends (and Aumentum's MCServer sent) is in `lrs.rs`
(`entity_xml`, `incoming_document_xml`) and `docs/registry-integration.md`.

#### ReceptionService — lodging/updating an application (Batch) and withdrawing it

**ReceptionService.wsdl** — tns `http://www.landsystems.com/reception-service/1.0`, address in file `http://localhost:8080/lrs/api/ReceptionService`

| Operation | Request fields | Response fields | Notes |
|---|---|---|---|
| `AcceptApplication` | referenceUuid: string, ResultCollection: ResultCollectionType | id: long |  |
| `Withdrawal` | instanceId: long | — |  |
| `SyncData` | referenceUuid: string, ResultCollection: ResultCollectionType | id: long |  |

`ResultCollection{className, items[]}` as in DomainModelService (XML of the `Batch` with its
transactions). `referenceUuid` = the jBPM process instance's Alfresco node uuid
(`processInstance.nodeRef.uuid`). `AcceptApplication` saves the Batch and its transactions and
updates the `lrs:Transaction` queue nodes (inferred from use: the desktop calls it after every
status change, `DataModel…BusinessLogicServiceAgent.UpdateBatchStatus`). `Withdrawal(instanceId)` =
withdraw the application of a process instance. `SyncData` = same input as `AcceptApplication`
without the reception step (inferred from name).

#### CashierService — invoices, payments, fees for the Cashier

**CashierService.wsdl** — tns `http://www.landsystems.com/cashier-service/1.0`, address in file `http://localhost:8080/lrs/api/CashierService`

| Operation | Request fields | Response fields | Notes |
|---|---|---|---|
| `SubmitInvoice` | transactionId: long, ResultCollection: ResultCollectionType | id: long |  |
| `CheckStatus` | transactionId: long, ResultCollection: ResultCollectionType | id: long |  |
| `GetSchema` | schemaName: string | schema: string |  |
| `CalculateFee` | transaction_code: string, parameters: string | fee: double |  |
| `GetPayment` | invoiceNumber: string | payment: string |  |

The desktop proxy (`…Proxy.CashierWebService.CashierServiceWse`) also has `SearchCustomer(ResultCollection)
→ Result` and `CreateCustomer(ResultCollection) → id`, which **the server WSDL does not have**; the
configured PLAGIS agent (`ThomsonReuters.Aumentum.Registry.DataModel.Cashier.CashierServiceAgent`,
`ExternalConfig/Agents.config`) only calls `CalculateFee`, `GetPayment`, `GetSchema`.

#### FeeService — fee calculation from the fee tables

**FeeService.wsdl** — tns `http://www.landsystems.com/FeeService/1.0`, address in file `http://localhost:8080/lrs/api/FeeService`

| Operation | Request fields | Response fields | Notes |
|---|---|---|---|
| `calculateFee` | feeRate: FeeRateDTO | result: FeeRateDTO |  |

`FeeRateDTO` (`lrs-service.xsd`): `id, feeName, transactionType, propertyArea, size,
sizeMeasurementUnitType, location, landUseTypeCategory, landUseType, sizeRangeUpperBound,
sizeRangeLowerBound, sizeRangeType, rateType, feeStructureId, rate, area, areaMeasurementUnitType`
(longs are dictionary ids, decimals amounts). Note: the WSDL's schema for FeeService is
`schemaUnqualified` (`http://www.landsystems.com/FeeService/1.0`), so the wrapper's children are
**unqualified** (server-config.wsdd).

### 3.2 ILS SDMS services — `com.ils.sdms.*`, `com.ils.core.*`

#### WorkflowWebService — jBPM process and task API

**WorkflowWebService.wsdl** — tns `http://www.landsystems.com/sdms/workflow/service`, address in file `http://localhost:8080/lrs/api/WorkflowWebService`

| Operation | Request fields | Response fields | Notes |
|---|---|---|---|
| `deployProcessDefinition` | definitionBody: base64Binary | processDefinition: ProcessDefinition |  |
| `undeployProcessDefinition` | definitionId: long | — |  |
| `getLatestProcessDefinitions` | — | result: ProcessDefinition[] |  |
| `getProcessDefinition` | definitionId: long | result: ProcessDefinition |  |
| `getProcessDefinitionByProcessId` | instanceId: long | result: ProcessDefinition |  |
| `startProcessInstance` | definitionId: long, initParams: NamedValue[], associatedWith: Reference | result: ProcessInstance |  |
| `suspendProcessInstance` | instanceId: long | — |  |
| `resumeProcessInstance` | instanceId: long | — |  |
| `deleteProcessInstance` | instanceId: long | — |  |
| `getProcessInstance` | instanceId: long | result: ProcessInstance |  |
| `getAllProcessInstances` | statusActive: boolean | result: ProcessInstance[] |  |
| `getTask` | taskId: long | result: TaskInstance |  |
| `getUserTasks` | instanceRef: Reference?, statusActive: boolean | result: TaskInstance[] |  |
| `getPooledTasks` | instanceRef: Reference?, statusActive: boolean | result: TaskInstance[] |  |
| `startTask` | taskRef: Reference | — |  |
| `completeTask` | taskNodeRef: Reference, transition: string?, comment: string? | — |  |
| `getTaskTransitions` | taskRef: Reference | result: string[] |  |
| `getTaskTransitionInfo` | taskRef: Reference | result: TransitionInfo[] |  |
| `getProcessDefinitionFile` | definitionId: long, fileName: string | result: base64Binary |  |
| `getProcessVariables` | instanceRef: Reference, variableNames: string[], tokenName: string | variable: ProcessVariable[] |  |
| `setProcessVariables` | instanceRef: Reference, variables: ProcessVariable[], tokenName: string? | — |  |
| `deleteVariables` | instanceRef: Reference, names: string[], tokenName: string? | — |  |
| `getContainerNodeRef` | — | rootRef: Reference |  |
| `assignTask` | taskRef: Reference, userRef: Reference | — |  |
| `addComment` | taskId: long, comment: string | — |  |
| `getTaskComments` | taskId: long | comments: CommentLog[] |  |
| `getProcessInstanceLog` | procInstId: long | logs: WorkflowLog[] |  |

See §6 for how the desktop uses it. Types (`workflow-types.xsd`, unqualified): `ProcessDefinition{nodeRef, id, name, description?, version, initial?}`,
`ProcessInstance{nodeRef, id, startDate?, endDate?, state?, tasks: TaskInstance[], association?, suspended?}`,
`TaskInstance{nodeRef, id, name, description?, startDate?, endDate?, assignedUser?, assignedGroup?, procInstanceRef, userName?, groupName?}`,
`TransitionInfo{name, description?, tasks: TaskDefinitionInfo{name, description?, swimlane?}[]}`,
`ProcessVariable` = `core:Variable{type: VariableType, name, value: VariableValue}`,
`WorkflowLog{log: {piCreatedLog | piEndLog | taskAssignLog | taskCreateLog | taskEndLog | transitionLog | commentLog | signalLog}}`
each with `type, time, description?` plus `TaskAssignLog{taskName, userName?, taskId}`,
`TaskCreateLog{taskName, userName?, taskId, comments[]}`, `TransitionLog{fromNode, toNode}`,
`SignalLog{transitionName}`, `TaskEndLog{taskId}`. `VariableType` = `string | boolean | float | double | long | integer | short | byte | date | bytearray | noderef | datetime`.

#### PersonWebService — users and groups (read)

**PersonWebService.wsdl** — tns `http://www.landsystems.com/core/person/service`, address in file `http://localhost:8080/lrs/api/PersonWebService`

| Operation | Request fields | Response fields | Notes |
|---|---|---|---|
| `getPerson` | userName: string | result: Reference |  |
| `getAllPeople` | — | result: Reference[] |  |
| `getAllPeopleNames` | — | result: string[] |  |
| `getAllGroups` | — | result: string[] |  |
| `getGroupName` | groupRef: Reference | result: string |  |
| `getGroupUsers` | groupName: string, immediate: boolean | result: string[] |  |
| `getGroupGroups` | groupName: string, immediate: boolean | result: string[] |  |
| `getGroup` | groupName: string | result: Reference |  |
| `getUserGroups` | userName: string | result: Reference[] |  |
| `getUserAuthorities` | userName: string | result: string[] |  |

#### MessagingService — JMS-style subscriptions (queue refresh notifications)

**MessagingService.wsdl** — tns `http://www.landsystems.com/sdms/messaging/service`, address in file `http://localhost:8080/lrs/api/MessagingService`

| Operation | Request fields | Response fields | Notes |
|---|---|---|---|
| `createMessenger` | params: Variable[] | result: string |  |
| `releaseMessenger` | msgId: string, unsubscribeDurable: boolean | — |  |
| `sendMessage` | message: Message, msgId: string | — |  |
| `receiveMessage` | msgId: string | result: Message[] |  |

`Message{value: choice SimpleMsg | TextMsg{text} | ObjectMsg{object: Variable} | MapMsg{map: Variable[]} | ByteMsg{byte_array}}`,
each with `properties{props: Variable[]}`. Used by the desktop's `MessagingManager` to subscribe to
events (`SubscriptionEventType`) and refresh the queue.

#### ReportsService — BIRT reports

**ReportsService.wsdl** — tns `http://www.landsystems.com/reports-service/1.0`, address in file `http://localhost:8080/lrs/api/ReportsService`

| Operation | Request fields | Response fields | Notes |
|---|---|---|---|
| `addReport` | reportName: string, reportTemplate: base64Binary, ReportInfo:  | — |  |
| `deployXML` | localFileName: string, dataBody: base64Binary | — |  |
| `removeReport` | reportName: string | — |  |
| `retrieveReport` | reportName: string | result: base64Binary |  |
| `produceReport` | reportName: string, reportFormat: string, resLanguage: string, NamedValueSet: , DataSourceSet:  | result: base64Binary |  |
| `runReport` | reportName: string, resLanguage: string, NamedValueSet: , DataSourceSet:  | RunReportResult:  |  |
| `renderReport` | tempDocumentFilePath: string, reportFormat: string | result: base64Binary |  |

`NamedValueSet{name, list: NamedValue{name, value~}[]}` (report parameters), `DataSourceSet{sourceType,
sourceList: DataSource{sourceName, sourceBody}[]}`, `ReportInfo{reportName, reportAuthor, reportTitle, reportDesc?}`,
`RunReportResult{reportName, tempDocumentFilePath, pageCount}`. `reportFormat` e.g. `pdf`/`html` (inferred).
Report service connection: `reports.service.connection.properties.{hostname=localhost, port=8080, servicesaddress=lrs}`
(`lrs-config.properties`).

#### HibernateDataAccessService — XML query access for the BIRT ODA driver

**HibernateDataAccessService.wsdl** — tns `http://www.landsystems.com/sdms/data/hibernate/service`, address in file `http://localhost:8080/lrs/api/HibernateDataAccessService`

| Operation | Request fields | Response fields | Notes |
|---|---|---|---|
| `getQueryMetaData` | queryXml: string | metaDataXml: string |  |
| `getQueryResultSet` | queryXml: string, deepLevel: int | resultSetXml: string |  |

Used by `com.ils.hibernate.oda.runtime.driver` (report designer/runtime), not by the desktop.

### 3.3 Alfresco 3.4 standard web services

#### AuthenticationService — sign-in, ticket check, sign-out

**AuthenticationService.wsdl** — tns `http://www.alfresco.org/ws/service/authentication/1.0`, address in file `http://localhost:8080/lrs/api/AuthenticationService`

| Operation | Request fields | Response fields | Notes |
|---|---|---|---|
| `startSession` | username: string, password: string | startSessionReturn: AuthenticationResult | header: LocaleHeader |
| `endSession` | ticket: string | — |  |
| `isTicketValid` | ticket: string | result: boolean |  |

#### RepositoryService — node store queries and CML updates (queue, documents, lodgments)

**RepositoryService.wsdl** — tns `http://www.alfresco.org/ws/service/repository/1.0`, address in file `http://localhost:8080/lrs/api/RepositoryService`

| Operation | Request fields | Response fields | Notes |
|---|---|---|---|
| `createStore` | scheme: string, address: string | createStoreReturn: Store |  |
| `getStores` | — | getStoresReturn: Store[] |  |
| `query` | store: Store, query: Query, includeMetaData: boolean | queryReturn: QueryResult | header: QueryConfiguration |
| `queryChildren` | node: Reference | queryReturn: QueryResult | header: QueryConfiguration |
| `queryParents` | node: Reference | queryReturn: QueryResult | header: QueryConfiguration |
| `queryAssociated` | node: Reference, association: Association | queryReturn: QueryResult | header: QueryConfiguration |
| `fetchMore` | querySession: string | queryReturn: QueryResult |  |
| `update` | statements: CML | updateReturn: UpdateResult[] |  |
| `describe` | items: Predicate | describeReturn: NodeDefinition[] |  |
| `get` | where: Predicate | getReturn: Node[] |  |

`Store{scheme, address}`; `Reference{store, uuid?, path?}`; `Predicate{nodes: Reference[], store?, query?: Query}`;
`Query{language ("lucene"|"xpath"), statement, sort: SortDefinition{name, ascending}[]}`;
`QueryResult{querySession?, resultSet: ResultSet{rows: ResultSetRow{rowIndex, columns: NamedValue[], score?, node?{id}}[], totalRowCount, metaData?}}`;
`NamedValue{name, isMultiValue, value, values[]}`; `CML{create[], addAspect[], removeAspect[], update[], writeContent[], delete[], move[], copy[], addChild[], removeChild[], createAssociation[], removeAssociation[]}`.
SOAP header `QueryConfiguration{fetchSize, maxResults}` (ns `http://www.alfresco.org/ws/headers/1.0`) on
the query operations; the desktop sets `fetchSize` from setting `bounds/QueueLimit` (PLAGIS 40000).
Stores used: **`lrs://rootStore`** (queue: `lrs:Transaction` nodes) and **`workflow://rootStore`**
(process/task/lodgment nodes) (`RepositoryServiceAgent`, lines 27–52).

#### ContentService — read/write node content

**ContentService.wsdl** — tns `http://www.alfresco.org/ws/service/content/1.0`, address in file `http://localhost:8080/lrs/api/ContentService`

| Operation | Request fields | Response fields | Notes |
|---|---|---|---|
| `read` | items: Predicate, property: string | content: Content[] |  |
| `write` | node: Reference, property: string, content: base64Binary, format: ContentFormat | content: Content |  |
| `writeWithAttachment` | node: Reference, property: string, format: ContentFormat | content: Content |  |
| `clear` | items: Predicate, property: string | content: Content[] |  |
| `transform` | source: Reference, property: string, destinationReference: Reference, destinationProperty: string, destinationFormat: ContentFormat | content: Content |  |

`Content{node, property, length, format?{mimetype, encoding?}, url?}`. The `url` is a `/lrs/download/…`
URL; the client appends `&ticket=`.

#### AuthoringService — lock/unlock, check-in/out, versions

**AuthoringService.wsdl** — tns `http://www.alfresco.org/ws/service/authoring/1.0`, address in file `http://localhost:8080/lrs/api/AuthoringService`

| Operation | Request fields | Response fields | Notes |
|---|---|---|---|
| `checkout` | items: Predicate, destination: ParentReference | checkoutReturn: CheckoutResult |  |
| `checkin` | items: Predicate, comments: NamedValue[], keepCheckedOut: boolean | checkinReturn: CheckinResult |  |
| `checkinExternal` | items: Reference, comments: NamedValue[], keepCheckedOut: boolean, format: ContentFormat, content: base64Binary | checkinExternalReturn: Reference |  |
| `cancelCheckout` | items: Predicate | cancelCheckoutReturn: CancelCheckoutResult |  |
| `lock` | items: Predicate, lockChildren: boolean, lockType: LockTypeEnum | lockReturn: Reference[] |  |
| `unlock` | items: Predicate, unlockChildren: boolean | unlockReturn: Reference[] |  |
| `getLockStatus` | items: Predicate | getLockStatusReturn: LockStatus[] |  |
| `createVersion` | items: Predicate, comments: NamedValue[], versionChildren: boolean | createVersionReturn: VersionResult |  |
| `getVersionHistory` | node: Reference | getVersionHistoryReturn: VersionHistory |  |
| `revertVersion` | node: Reference, versionLabel: string | — |  |
| `deleteAllVersions` | node: Reference | deleteAllVersionsReturn: VersionHistory |  |

`LockTypeEnum` = `read | write`. **The desktop "claims" a task by `lock(write)` on the task node**
(§6).

#### AccessControlService — ACLs and authorities (groups)

**AccessControlService.wsdl** — tns `http://www.alfresco.org/ws/service/accesscontrol/1.0`, address in file `http://localhost:8080/lrs/api/AccessControlService`

| Operation | Request fields | Response fields | Notes |
|---|---|---|---|
| `getACLs` | predicate: Predicate, filter: ACE | acls: ACL[] |  |
| `addACEs` | predicate: Predicate, aces: ACE[] | acls: ACL[] |  |
| `removeACEs` | predicate: Predicate, aces: ACE[] | acls: ACL[] |  |
| `getPermissions` | predicate: Predicate | result: GetPermissionsResult[] |  |
| `getClassPermissions` | classNames: string[] | result: GetClassPermissionsResult[] |  |
| `hasPermissions` | predicate: Predicate, permissions: string[] | results: HasPermissionsResult[] |  |
| `setInheritPermission` | predicate: Predicate, inheritPermission: boolean | acls: ACL[] |  |
| `getOwners` | predicate: Predicate | results: OwnerResult[] |  |
| `setOwners` | predicate: Predicate, owner: string | results: OwnerResult[] |  |
| `getAllAuthorities` | filter: AuthorityFilter | results: string[] |  |
| `getAuthorities` | — | results: string[] |  |
| `createAuthorities` | parentAuthority: string?, newAuthorites: NewAuthority[] | results: string[] |  |
| `deleteAuthorities` | authorities: string[] | — |  |
| `addChildAuthorities` | parentAuthority: string, authorities: string[] | results: string[] |  |
| `removeChildAuthorities` | parentAuthority: string, authorities: string[] | — |  |
| `getChildAuthorities` | authority: string, filter: SiblingAuthorityFilter | results: string[] |  |
| `getParentAuthorities` | authority: string, filter: SiblingAuthorityFilter | results: string[] |  |

`ACE{authority, permission, accessStatus (acepted|declined — sic)}`; `AuthorityFilter{authorityType?, rootOnly}`;
`SiblingAuthorityFilter{authorityType?, immediate}`.

#### AdministrationService — users

**AdministrationService.wsdl** — tns `http://www.alfresco.org/ws/service/administration/1.0`, address in file `http://localhost:8080/lrs/api/AdministrationService`

| Operation | Request fields | Response fields | Notes |
|---|---|---|---|
| `queryUsers` | filter: UserFilter | result: UserQueryResults |  |
| `fetchMoreUsers` | querySession: string | result: UserQueryResults |  |
| `getUser` | userName: string | result: UserDetails |  |
| `createUsers` | newUsers: NewUserDetails[] | result: UserDetails[] |  |
| `updateUsers` | users: UserDetails[] | result: UserDetails[] |  |
| `changePassword` | userName: string, oldPassword: string, newPassword: string | — |  |
| `deleteUsers` | userNames: string[] | — |  |

`UserDetails{userName, properties: NamedValue[]}`, `NewUserDetails{userName, password, properties[]}`,
`UserFilter{userName}`, `UserQueryResults{querySession?, userDetails[]}`. Properties use `{http://www.alfresco.org/model/content/1.0}firstName` etc. (see `lrs.rs::create_user`).

#### DictionaryService — content-model metadata

**DictionaryService.wsdl** — tns `http://www.alfresco.org/ws/service/dictionary/1.0`, address in file `http://localhost:8080/lrs/api/DictionaryService`

| Operation | Request fields | Response fields | Notes |
|---|---|---|---|
| `getClasses` | types: ClassPredicate, aspects: ClassPredicate | getClassesReturn: ClassDefinition[] |  |
| `getProperties` | propertyNames: string[] | getPropertiesReturn: PropertyDefinition[] |  |
| `getAssociations` | associationNames: string[] | getAssociationsReturn: AssociationDefinition[] |  |
| `isSubClass` | className: string, isSubClassOfName: string | isSubClassReturn: boolean |  |

#### ClassificationService — categories

**ClassificationService.wsdl** — tns `http://www.alfresco.org/ws/service/classification/1.0`, address in file `http://localhost:8080/lrs/api/ClassificationService`

| Operation | Request fields | Response fields | Notes |
|---|---|---|---|
| `getClassifications` | store: Store | getClassificationsReturn: Classification[] |  |
| `getChildCategories` | parentCategory: Reference | getChildCategoriesReturn: Category[] |  |
| `getCategories` | items: Predicate | getCategoriesReturn: CategoriesResult[] |  |
| `setCategories` | items: Predicate, categories: AppliedCategory[] | setCategoriesReturn: CategoriesResult[] |  |
| `describeClassification` | classification: Name | describeClassificationReturn: ClassDefinition |  |

#### ActionService — Alfresco actions and rules

**ActionService.wsdl** — tns `http://www.alfresco.org/ws/service/action/1.0`, address in file `http://localhost:8080/lrs/api/ActionService`

| Operation | Request fields | Response fields | Notes |
|---|---|---|---|
| `getConditionDefinitions` | — | actionItemDefinition: ActionItemDefinition[] |  |
| `getActionDefinitions` | — | actionItemDefinition: ActionItemDefinition[] |  |
| `getActionItemDefinition` | name: string, definitionType: ActionItemDefinitionType | actionItemDefinition: ActionItemDefinition |  |
| `getRuleTypes` | — | ruleType: RuleType[] |  |
| `getRuleType` | name: string | ruleType: RuleType |  |
| `getActions` | reference: Reference, filter: ActionFilter | action: Action[] |  |
| `saveActions` | reference: Reference, actions: Action[] | action: Action[] |  |
| `removeActions` | reference: Reference, actions: Action[] | — |  |
| `executeActions` | predicate: Predicate, actions: Action[] | actionExecutionResult: ActionExecutionResult[] |  |
| `getRules` | reference: Reference, ruleFilter: RuleFilter | rule: Rule[] |  |
| `saveRules` | reference: Reference, rules: Rule[] | rule: Rule[] |  |
| `removeRules` | reference: Reference, rules: Rule[] | — |  |

### 3.4 Other packaged contracts

**CMIS 1.0 web services** (`/lrs/cmis/*`, `CMISWS-Service.wsdl`, ns `http://docs.oasis-open.org/ns/cmis/ws/200908/`): stock
Alfresco 3.4. Ports and operations — Discovery: `query getContentChanges`; MultiFiling: `addObjectToFolder removeObjectFromFolder`;
Navigation: `getDescendants getChildren getFolderParent getFolderTree getObjectParents getCheckedOutDocs`;
Object: `createDocument createDocumentFromSource createFolder createRelationship createPolicy getAllowableActions getObject getProperties getRenditions getObjectByPath getContentStream updateProperties moveObject deleteObject deleteTree setContentStream deleteContentStream`;
Policy: `applyPolicy removePolicy getAppliedPolicies`; Relationship: `getObjectRelationships`;
Repository: `getRepositories getRepositoryInfo getTypeChildren getTypeDescendants getTypeDefinition`;
Versioning: `checkOut cancelCheckOut checkIn getObjectOfLatestVersion getPropertiesOfLatestVersion getAllVersions`;
ACL: `getACL applyACL`. No client in the captures uses CMIS.

**BusinessLogicService** (`/lrs/cxfapi/BusinessLogicService`, `business-logic-service.wsdl`, tns
`http://www.thomsonreuters.com/grm`, elements unqualified; SOAPAction empty):

| Operation | Request | Response |
|---|---|---|
| `BuildApplicantFullName` | `applicantEntity?: EntityType{className?, item?}` | `applicant?: EntityType` |
| `CreateApplicant` | `transactionCode?` | `applicant?: EntityType` |
| `CreateApplicantByPersonType` | `personClassName?` | `party?: EntityType` |
| `SearchApplicant` | `queryType?: {parameters: {key?, operator?, type?, value?}[], query?}`, `caseInsensitiveSearch` | `applicants?: EntityTypeCollection{className?, items: string[]}` |
| `SearchProperties` | `transactionEntity?: EntityType` | `laUnits?: EntityTypeCollection` |
| `GetTransactionMetaDataByGroup` | `includeGroupCodes: string[]`, `excludeGroupCodes: string[]` | `TransactionMetaData?: EntityTypeCollection` |
| `CreateTransactionInstance` | `transactionCode?` | `transaction?: EntityType` |
| `SearchTransactionNumber` | `transactionEntity?: EntityType` | `TransactionNumber?` |
| `Dummy` | `entityTypeCollection?` | `entityTypeCollection?` |

The desktop has a proxy for it (`…Proxy.BusinessLogicWebService.BusinessLogicServiceWse`, URL setting
`ServiceProxy_BusinessObjWebService_business_service`) but **no code calls its methods**: all
`IBusinessLogicServiceAgent` methods (`ContinueRegistration`, `PostponeRegistration`, `CustomReject`,
`WithdrawRegistration`, `GetFullName` …) run locally in the client and call other services.
Whether CXF publishes it on .5 is not captured.

---

## 4. What the Registry Desktop actually calls

Client structure: each service has an agent class (`$D/net/ThomsonReuters.Aumentum.Registry.Services.dll/…/*ServiceAgent.cs`)
that wraps a WSE proxy (`*ServiceWse`). Agents are wired by `bin/ExternalConfig/Agents.config`
(PLAGIS overrides: `DataModel.DomainModel.DomainModelServiceAgent`, `PLAGIS.DataModel.BusinessLogic.CustomBusinessLogicServiceAgent`,
`DataModel.Cashier.CashierServiceAgent`). Screens reach them through `AgentFactory.{X}Agent`.
URLs come from `ThomsonReuters.Aumentum.Registry.PLAGIS.RegistryDesktop.exe.config`: all
`http://localhost:8080/lrs/api/{Service}` (host/port replaced at sign-in by `BuildConnectionUrl`),
except `RepositoryWebService_URL = http://localhost:8080/lrs/cxfapi/RepositoryWebService` and
`BusinessLogicWebService_URL = http://localhost:8080/lrs/ws/BusinessLogicService`. Not overridden
(compiled defaults, `Services.Properties.Settings`): `CashierWebService_URL` and `FeeService_URL`
= `…/lrs/api/CashierService`, `…/lrs/api/FeeService`.

> **Open point:** with `RepositoryWebService_URL` on `/lrs/cxfapi/RepositoryWebService`, the queue
> works on the live system, so either CXF publishes a Repository endpoint there, or the deployed
> config differs from the captured copy. Check on .5 before relying on `/lrs/api/RepositoryService`
> for the desktop.

### 4.1 Operations called, by service (proxy calls found in each agent)

| Service | Operations the desktop calls | Main callers (class → screen/use) | Purpose |
|---|---|---|---|
| AuthenticationService | `startSession`, `endSession` (+ `GET /lrs/` SSO) | `AuthenticationServiceAgent` ← login form, `ShellForm` | sign in/out |
| DomainModelService | `loadObject` (111× `GetObjectList`, 38× `GetObject`, `Search`), `saveOrUpdate` (33× `Save`), `deleteObjects` (15×), `createInstance` (143× `CreateServerInstance`), `generateNumber` (23×), `getSchema`, `getRegisteredSchemas` | everywhere: `AssessWorker`, `WorkerFactory`, `DataModelUtilities`, `TransactionDeedPage`, `RejectPage`, `PostponePage`, `FeeListSelectionPage`, `DataModelQueries`, `Rules`, `CustomRules`, dictionaries (`EntityDictionaryProvider`, 250× `GetDictionary`), settings (121× `GetSettings`) | all reads (HQL, §4.2) and writes of the domain model |
| ReceptionService | `AcceptApplication` (7×), `Withdrawal` (3×), `SyncData` (1×) | `DataModel.BusinessLogic.BusinessLogicServiceAgent` (postpone/continue/withdraw/status updates), `AssessWorker`/finish pages | save the whole application (Batch) and refresh queue |
| WorkflowWebService | `getProcessInstance`, `startProcessInstance`, `suspendProcessInstance`, `resumeProcessInstance`, `deleteProcessInstance`, `getProcessInstanceLog`, `getProcessDefinition`, `getProcessDefinitionByProcessId`, `getLatestProcessDefinitions`, `getProcessVariables`, `setProcessVariables`, `deleteVariables`, `getPooledTasks`, `getUserTasks`, `assignTask`, `startTask`, `getTask`, `completeTask`, `getTaskTransitions`, `getTaskTransitionInfo` | `WorkflowServiceAgent` ← `AssessWorker` (start), `WorkerFactory`/`ListOfInstruments` (open task), `CompletePage`/`CompleteForm`/`FinishPage` (complete), `AssignPage`, `RejectPage`/`RejectForm`/`PostponePage` (variables), `IndexerWizardPage` (cancel new) | §6 |
| RepositoryService | `getStores`, `query` (lucene), `get` (xpath), `queryChildren`, `queryParents`, `queryAssociated`, `fetchMore`, `update` (CML create/update/addAspect/delete/removeChild) | `ShellForm.QueryQueue` (the work queue), `CompletePage` (find task to assign), `HistoryExtView`, `AssessWorker` (transaction node by id), `ScanServiceAgent` (documents), `WorkflowServiceAgent.AddComment` (CML create comment), `GetLodgmentIDByProcess` | queue, task comments, scanned documents, lodgment id |
| ContentService | `read`, `write`, `clear` (+ `PUT /lrs/upload`) | `ContentServiceAgent` ← scanning/document manager (`Controls.WorkerAssess.Scan.DocumentManager`), signatures | page images and attachments |
| AuthoringService | `lock`, `unlock`, `getLockStatus`, `checkout`, `checkin`, `checkinExternal`, `cancelCheckout`, `createVersion`, `getVersionHistory`, `revertVersion`, `deleteAllVersions` | `WorkflowServiceAgent.LockTask/UnLockTask` (task claim), document versioning | claim tasks, document versions |
| PersonWebService | `getPerson`, `getAllPeople`, `getAllPeopleNames`, `getAllGroups`, `getGroup`, `getGroupName`, `getGroupUsers`, `getUserAuthorities` | `PersonServiceAgent` ← queue (`MyGroupList`), assign page, full names | users and groups |
| AccessControlService | `getACLs`, `getPermissions`, `hasPermissions`, `getClassPermissions` | `AccessControlServiceAgent` ← `WorkflowServiceAgent.LoadAuthorizedProcessDefinitionList` (ACL "Manager" on process definitions → which workflows a user may start), menu permissions | permissions |
| AdministrationService | `queryUsers`, `changePassword` | change-password dialog, signature support check | users |
| DictionaryService | `getClasses`, `getAssociations`, `isSubClass` | metadata (`DictionaryAgent.GetMetaInfo`) | content-model metadata |
| ReportsService | `produceReport`, `deployXML` | `ReportServiceAgent` ← print pages (`PrintDocumentsPage`, `PrintDocumentsFee`, `PagePropReportPrinting`) | print certificates, receipts, letters |
| MessagingService | `createMessenger`, `receiveMessage`, `releaseMessenger`, `sendMessage` | `MessagingManager` | queue refresh events |
| IntegrationService | `SaveJob`, `GetJob`, `GetParcelPointsData` (and `SaveEntity` in the proxy) | `Controls.WorkerAssess.DataModelUtilities.CreateMCJob/ApproveMCJob/RejectMCJob/IsJobPresent/GetMCJobComplete/GetParcelPointsData` ← `Rules.cs` (workflow rules) and `AssessWorker`; `DataModel…BusinessLogicServiceAgent.WithdrawRegistration` (reject jobs) | create/approve/reject Cadastre jobs, read beacon legs |
| CashierService | `CalculateFee`, `GetPayment`, `GetSchema` (PLAGIS agent); base agent also `SubmitInvoice`, `CheckStatus` | `CashierAgent.CreateCashEntity`, `CalculateFee`, `CheckInvoiceStatus` ← fee pages | invoices / payment lookup |
| FeeService | `calculateFee` | `FeeServiceAgent` | fee from rate table |
| ClassificationService, ActionService, HibernateDataAccessService, BusinessLogicService, CMIS | — | not called | — |

### 4.2 The read API: HQL sent to `DomainModelService.loadObject`

All are `SimpleQuery`; `{ClassNames.ResolveDataModelType(typeof(X))}` expands to
`com.landsystems.lrs.model.X`. Many are prefixes to which the code appends `and …` filters with `?`
parameters. The full list (extracted verbatim, file and line) is in **Appendix A**; the most used:

| Query | Used for |
|---|---|
| `from com.landsystems.lrs.model.Batch where id=?` (`long` lodgment id) | open the application of a task (WorkerFactory, AssessWorker, FinishPage, DeliveryWorker, CommentHistoryPanel, ListOfInstruments, BusinessLogic) |
| `from com.landsystems.lrs.model.Transaction where batch=? and isDelivered=?` | delivery |
| `from com.landsystems.lrs.model.Rejection where id=?` / `… where lodgment_id=?` | reject/postpone reasons |
| `from com.landsystems.lrs.model.Setting where category=?` / `… and name=?` | all settings (`GetSettings`) |
| `from com.landsystems.lrs.model.Dictionary where category=? and hidden=0 and begin_lifespan_version is not null and end_lifespan_version is null order by sort_order,label` | dictionaries (`GetDictionary`) |
| `from com.landsystems.lrs.model.TransactionMetaData where hidden=0 order by sort_order,label` | transaction types |
| `from com.landsystems.lrs.model.RegistryOffice where active=1 and office_type='registration'`, `from com.landsystems.lrs.model.REGISTRY` | offices/registries |
| `from com.landsystems.lrs.model.FormField where form_name=? order by id` | form layouts |
| `from com.landsystems.lrs.model.LrWorkflowLog where task_id=?` | task history |
| `from com.landsystems.lrs.model.GroundRent where propertyNumber=? and paidYear=?` | ground-rent rule (PLAGIS `CustomRules`) |
| `select su from com.landsystems.lrs.model.LAUnit p join p.SpatialUnits su where 1=1 …` | property search, printing |
| `select t from com.landsystems.lrs.model.Transaction t join t.Properties p join p.SpatialUnits su where 1=1 …` | transactions on a property |
| `select o from com.landsystems.lrs.model.Right r join r.LaUnit p join r.Party o where 1=1 …` | owners |
| `select tr from …Transaction tr join tr.TransactionType tt join tr.Properties pr, …Restriction rrr where tr.EndLifespanVersion is null and tt.Code='CV' and pr.Uid=? and rrr.TransactionSource=tr.TransactionNumber and rrr.EndDate is null and rrr.EndLifespanVersion is null` | caveat check (`VerifyPropertyID`) |
| `from …Transaction t where 1=1 and t.Id <> {id} and (t.SourcePropertyIDs=? or t.TargetPropertyIDs=?) and t.TransactionStatus.Code='transaction_lodged'` | pending-transaction check |

Other readers of the same API: the Cadastre MCServer (`LRSIntegrationQueryDefinition`, see
`docs/registry-integration.md`) and the ArcMap RegistryExtension, which takes HQL templates from
`lr_setting` category `gisintegration`:
`queryTransactionByUser` = `select {0} from com.landsystems.lrs.model.ProcessHistory ph,com.landsystems.lrs.model.Transaction t,org.jbpm.taskmgmt.exe.TaskInstance task where ph.TaskComplete is null and task.end is null and {{1}} and ph.TaskId = task.id and (task.actorId = ? or (task.actorId = null and task.swimlaneInstance.name in ({{0}}) )) and ph.TransactionId = t.Id order by ph.Id`;
`queryPropertyByTransaction` = `select distinct {0} from com.landsystems.lrs.model.Transaction t join t.Properties l join l.SpatialUnits s join s.County county where t.Id = ?`;
`queryPartyByTransaction` = `select {0} from com.landsystems.lrs.model.Transaction t join t.Parties pp join pp.Person p where t.Id = ?`;
`queryDocumentsByTransaction` = `select {0} from com.landsystems.lrs.model.Transaction t join t.Documents d where t.Id = ?`;
`extensionTasks` / `extensionTasksCanComplete` = `Manage Spatial Data,Review Spatial Data`
(`$R/lrs-export/registry/data/lr_setting.psv.gz`). Note the first one queries **jBPM's own Hibernate
entity** `org.jbpm.taskmgmt.exe.TaskInstance` — the replacement's HQL layer must map the `JBPM_*` tables too.

### 4.3 Lucene queries (RepositoryService.query) — the work queue

The desktop queue (`ShellForm.cs:1139–1193`) queries store `lrs://rootStore`, sorted by
`{http://www.landsystems.com/lrs/model/core/1.0}taskCreatedDate` descending:

```
All tasks:   TYPE:"{…lrs/model/core/1.0}Transaction" AND (@lrs\:taskCompleted:"false")
             AND ((@lrs\:taskActorName:"{user}")  OR ((@lrs\:taskSwimlane:"{group1}") OR (@lrs\:taskSwimlane:"{group2}")…))
My tasks:    TYPE:"…Transaction" AND (@lrs\:taskCompleted:"false") AND (@lrs\:taskActorName:"{user}")
My groups:   TYPE:"…Transaction" AND (@lrs\:taskCompleted:"false") AND ((@lrs\:taskActorName:"{group1}") OR …)
Custom:      … {filter} AND ((@lrs\:taskActorName:"{user}") OR (swimlanes…))
```
plus `IQueueService.Filter` appended. Rows carry the `lrs:Transaction` node properties
(`NameSpaces.cs`: `transactionId, transactionNumber, transactionType, applicantName, invoiceNumber,
sourceRids, createDate, regDate, past, isDelivered, isOverdue, registry, processId, processName,
processDescription, processSuspended, rootProcessInstance, taskId, taskName, taskDescription,
taskCreatedDate, taskStartDate, taskEndDate, taskCompleted, taskActorRef, taskActorName,
taskActorIsGroup, taskSwimlane`). Other queries: `TYPE:"{…sdms/model/workflow/1.0}taskRef" AND
(@wf\:taskCompleted:"false") AND (@wf\:taskProcInstRef:"{ref}") [AND (@wf\:taskName:"{name}")]` in
`workflow://rootStore` (`CompletePage`), `… AND (@lrs\:transactionId:"{id}")` (`AssessWorker`),
`… (@lrs\:entityid:"{id}")` (`ScanServiceAgent`), and xpath via `get`:
`./*[subtypeOf('lrs:Lodgment')]` under the process node → `lrs:lodgmentId` (= `lr_batch.id`),
`*//.[subtypeOf('wf:taskRef')]` after starting a process (`RepositoryQueryNames.cs`).

**A replica that keeps the desktop must therefore serve this Lucene subset over a table/view of open
tasks** (inferred design consequence).

### 4.4 Entity classes written

`DomainModelService.saveOrUpdate` class names seen: `Transaction`, `ProcessHistory`, `FeeStructure`,
`FeeRate`, `Right`, `Restriction`, `Responsibility`, `Payment`, `RentalAmountRevision`, `Rejection`
(reject/postpone), plus generic `Save(className, …)` callers. `deleteObjects`: `Transaction`, `LAUnit`,
`Party`, `Payment`, `SourceDocument`, `Right`, `Restriction`, `Responsibility`, `FeeRate`,
`FeeStructure`, `RentalAmountRevision`, extension classes. `createInstance` (by count): `Address`(36),
`PaymentTransaction`(22), `Corporation`(17), `Individual`(11), `IncomingDocument`(10), `PartyContact`(7),
`LAUnit`(7), `Right`/`Restriction`/`Responsibility`/`Parcel`/`Applicant`(6), `Party`(5), `Transaction`,
`Payment`, `BuildingUnit`(4), `SiteAnalysis`, `Building`(3), `TransactionRecord`, `TransactionNote`,
`RentalAmountRevision`, `GroupParty`, `Condition`(2), `TempCounter`, `SurveyPlan`, `PartyMember`,
`LAUnitParty`, `GroundRent`, `Batch`(1). The whole application is written through
`ReceptionService.AcceptApplication` with class `com.landsystems.lrs.model.Batch`.

`IntegrationService.SaveJob` from the desktop (`DataModelUtilities.CreateMCJob`): `Entity{className =
setting gisintegration/lrsMcJobClass (com.landsystems.lrs.model.McJob), status, properties}`:

| status | properties |
|---|---|
| `added` | `JobName` = job type (e.g. `Deed Plan Creationv2`, `Plot Parcels`, `Parcel Creation`), `JobInstanceName` = `Job-{propertyNumber or surveyorPlanNo}`, `JobPriority` = `2`, `JobComments` = job name, `JobParameters` = `@LRS:[Transaction]:[TransactionNumber]={transactionNumber}`, `TransactionId` |
| `approved` / `rejected` | `TransactionId` |

One job per target property when the job name is listed in `multicadastre/mcJob_multipleTargetProperties`
(PLAGIS: `Deed Plan Creationv2`). Which stage creates which job: `lr_setting` `gisintegration`
`job_stage_description(n)` = `{transactionCode}:{stage}:{jobKey}:Warning`, e.g.
`APRO:PrepareTitleDeedPlan:job_deedplan_creation:Warning`, `INSC:CreateParcel:job_parcel_creation:Warning`,
`SPC:PlotSurveyPlanlInCadastre:job_plot_survey_plan:Warning` (95 entries).

`IntegrationService.SaveEntity` writers: the Cadastre (MCServer / this repo) with `Parcel` (`modified`)
and `IncomingDocument` (`added`, type `sd_titledeedPlan`), see `lrs.rs`.

---

## 5. Domain objects on the wire

### 5.1 Envelope types (summary of the XSDs)

| Schema (ns) | Types |
|---|---|
| `lrs-service.xsd` (`http://www.landsystems.com/lrs/service`, qualified) | `QueryType` (choice SimpleQuery/QueryById/SPQuery), `QueryTypeInt{queryString, queryParams?}`, `QueryParamType{param{value, @type}[]}`, `ResultType` (choice ResultScalar/ResultCollection), `ScalarRecordType{items{value[]}}`, `ScalarValueType{type, value?}`, `ResultCollectionType{className, items{value: string[]}}`, `FeeRateDTO`, element `FeeStructureDTO` |
| `lrs-service-types.xsd` (same ns) | `EntityStatus` enum, `PageType` enum, `EntityValue`, `Entity`, `EntityContent`, `DocumentContent`, `PageContent`, `DocumentEntity`, `EntityCollection` |
| `types.xsd` (Alfresco content 1.0) | `Store, Reference, ParentReference, Predicate, Query, SortDefinition, Node, NamedValue, ContentFormat, ResultSet, ResultSetRow, ResultSetRowNode, ResultSetMetaData, ClassDefinition, PropertyDefinition, AssociationDefinition, RoleDefinition, NodeDefinition, Cardinality (0..1|1|*|1..*), ValueDefinition, Version, VersionHistory, Category, Classification`, simple `Name, UUID, Path` |
| `cml.xsd` | CML statements (§3.3) |
| `headers.xsd` | `QueryConfiguration{fetchSize, maxResults}`, `LocaleConfiguration{locale?}`, `NamespaceConfiguration{mapping{prefix, uri}[]}` |
| `workflow-types.xsd` (unqualified) | §3.2 |
| `core-types.xsd` | `VariableType` enum, `VariableValue` (choice of typed values incl. `NodeRefValue: Reference`), `Variable{type, name, value}` |
| `reports-service.xsd` | §3.2 |

### 5.2 Domain entities (inside `ResultCollection.items` / `Entity.className`)

73 Hibernate mappings (`$R/lrs-export/registry/hbm/com/landsystems/lrs/model/*.hbm.xml`, HyperJAXB3
accessors, discriminator column `Hjtype` holding the Java class name):
`Address AdminSourceDocument Applicant Batch Billing BillingDistLine BillingLine Building BuildingUnit
BusinessDay Condition Corporation Dictionary DocumentExt FeeRate FeeStructure FormField GroundRent
GroupParty IncomingDocument Individual InternalDocument LAUnit LAUnitLineage LAUnitParty
LrExtensionLink LrHoliDay LrReplicationContent LrReplicationLog LrReplicationPackage LrSimpleCounter
LrSimpleCounterPool LrWorkingDay McJob Mortgage OutgoingDocument Parcel Party PartyContact PartyExt
PartyMember Payment PaymentTransaction Person ProcessEvent ProcessHistory PropertyExt Registry
RegistryOffice Rejection RejectionReason RelatedBatch RentalAmountRevision Responsibility Restriction
Right RRR RrrMetaData Setting SiteAnalysis SourceDocument SpatialUnit StaffMember SurveyPlan Taxation
TempCounter Transaction TransactionExt TransactionMetaData TransactionNote TransactionRecord Usage
Valuation`. Field lists per class are in the schema document (01/02); HQL uses the **property** names
of these mappings (e.g. `TransactionNumber`, `TransactionStatus.Code`, `SourcePropertyIDs`,
`Properties`, `SpatialUnits`, `BeginLifespanVersion`, `EndLifespanVersion`), and some queries use
**column** names (`office_name`, `sort_order`, `lodgment_id`, `form_name`) — Hibernate 3 accepts
both in simple cases; the replica's HQL translator must too.

Status dictionaries used through the API: `transaction_status` codes `transaction_lodged`,
`transaction_completed`, `transaction_postponed`, `transaction_rejected`, `transaction_withdrawn`;
`application_status` codes `application_lodged`, `application_postponed`, `application_rejected`
(`DataModel…BusinessLogicServiceAgent`, `Services…BusinessLogicServiceAgent`).

---

## 6. Workflow API in practice

The engine is jBPM 3 behind `WorkflowWebService`, mirrored into Alfresco nodes: process nodes
(`wf:processInstance`), task nodes (`wf:taskRef`, properties `wf:taskId, taskName, taskPooledActors,
taskUserName, taskGroupName, taskProcInstRef, taskCompleted, taskSwimlane, …`), a lodgment child
(`lrs:Lodgment` with `lrs:lodgmentId` = `lr_batch.id`) in `workflow://rootStore`, and one
`lrs:Transaction` node per transaction in `lrs://rootStore` for the queue (`NameSpaces.cs`).

| User action | Calls (in order) | Source |
|---|---|---|
| Start a new application | `AccessControlService.getACLs` on process definitions (permission `Manager`) to list allowed workflows; `WorkflowWebService.startProcessInstance(definitionId, initParams, null)`; `RepositoryService.get(process node, "*//.[subtypeOf('wf:taskRef')]")` | `WorkflowServiceAgent`, `AssessWorker.cs:598` |
| Cancel a new application | `deleteProcessInstance(id)` | `AssessWorker.cs:1155`, `IndexerWizardPage.cs:81` |
| Show the queue | `RepositoryService.query` (lucene, §4.3); events via MessagingService | `ShellForm` |
| Open (claim) a task | `getTask(taskId)`; check `cm:lockOwner` via `RepositoryService.get`/`PersonWebService.getPerson`; **`AuthoringService.lock(task node, write)`**; `RepositoryService.get(process, "./*[subtypeOf('lrs:Lodgment')]")` → lodgment id; `DomainModelService.loadObject("from …Batch where id=?")` | `WorkerFactory.cs:115…`, `WorkflowServiceAgent.LockTask` |
| Release a task | `AuthoringService.unlock` | `UnLockTask` |
| Save work | `DomainModelService.saveOrUpdate` / `ReceptionService.AcceptApplication(processUuid, Batch)` | workers |
| **Next stage (complete)** | `getTaskTransitionInfo(taskRef)` or `getTaskTransitions(taskRef)` → user picks; `completeTask(taskRef, transition, comment)`; then comments `RepositoryService.update` (CML create `wf:comment` under the task, assoc `wf:comments`, props `wf:commentText`, `wf:commentAcs`); if the next task must go to a chosen user: lucene query for the new `wf:taskRef` then `assignTask(taskRef, userRef)` | `CompletePage.cs:71–148`, `CompleteForm.cs` |
| Delivery finish | `getTaskTransitions` → `completeTask(ref, transition, null)` | `FinishPage.cs:114` |
| **Reject** | `DomainModelService.saveOrUpdate(Rejection{rejectionReasons, lodgment_id, rejectionDate})` → id; `setProcessVariables(taskRef, [rejection = id])`; `completeTask` with the reject transition; `CustomReject`: all transactions → `transaction_rejected`, their properties and spatial units get `endLifespanVersion = now`; batch status `application_rejected` | `RejectPage.cs:144`, `RejectForm.cs:92`, `DataModel…BusinessLogicServiceAgent.CustomReject` |
| **Postpone** (stage) | as reject, `PostponePage` saves a `Rejection` and sets variable `rejection`, then completes | `PostponePage.cs:89` |
| **Postpone** (queue menu) | `suspendProcessInstance(id)`; batch `application_postponed`, transactions `transaction_postponed`; `ReceptionService.AcceptApplication` | `BusinessLogicServiceAgent.PostponeRegistration` |
| **Continue** | `resumeProcessInstance(id)`; `application_lodged` / `transaction_lodged`; `AcceptApplication` | `ContinueRegistration` |
| **Withdraw** | `ProcessHistory` update; `AcceptApplication` (clears property counters of PL numbers in use elsewhere); `ReceptionService.Withdrawal(processId)`; if `mcintegration/IntegrationWithMC`=1: `IntegrationService.SaveJob(McJob, status rejected, TransactionId)` per transaction | `WithdrawRegistration` |
| Reassign | `assignTask(taskRef, userRef)` + comment | `AssignPage.cs:61` |
| History | `getProcessInstanceLog`, `LrWorkflowLog` HQL, `/lrs/downloadBPM` | `HistoryExtView`, `ShellForm` |
| Cadastre job rules | `IntegrationService.SaveJob` / `GetJob` from workflow rules `Rules.cs:893/938/983`; `AssessWorker.cs:329` waits for job completion (`JobEnded` set) | `DataModelUtilities` |

Transition names come from the jBPM process definitions (`WorkflowConfig/*.config` on the client and
the process archives on the server — see the workflow document).

---

## 7. Other services — purpose and use

| Service | Purpose | Desktop use | Replica need |
|---|---|---|---|
| CashierService | Cashier: submit an invoice for a transaction (`SubmitInvoice`), poll status, compute fee for a transaction code (`CalculateFee(transaction_code, parameters)`), fetch payment by invoice number (`GetPayment` → string), return schema (`GetSchema`) | fees and payment check | keep `CalculateFee`, `GetPayment` if the desktop/Cashier stay |
| FeeService | rate-table fee | via FeeServiceAgent | low |
| ReceptionService | lodge/update/withdraw an application | core | **must** (desktop) |
| ReportsService | produce BIRT reports server-side | printing | must while desktop prints |
| ContentService / RepositoryService | documents, queue, comments | core | **must** (desktop) |
| DictionaryService | Alfresco model metadata | metadata | small |
| PersonWebService | users/groups | core | **must** |
| ClassificationService | Alfresco categories | none | no |
| AccessControlService | ACLs/groups | workflow permissions; this repo: `getParentAuthorities` | **must** (`getACLs`, `getPermissions`, `hasPermissions`, `getParentAuthorities`) |
| AdministrationService | users | change password; this repo: `createUsers`, `changePassword` | yes |
| MessagingService | push events | queue refresh | can answer empty (inferred: client polls) |
| ActionService, AuthoringService(versions), HibernateDataAccessService, CMIS | Alfresco extras / reporting ODA | lock/unlock only | `lock`, `unlock`, `getLockStatus` |

The **Aumentum Registry Cashier** web app (`registry-client/Aumentum Registry Cashier/Web.config`)
points to `…/lrs/api/AuthenticationService`, `MessagingService`, `PersonWebService`,
`RepositoryService` (settings section `LRS.Client.ServiceProxy.Properties.Settings`); its proxy
assembly is not in the capture.

---

## 8. Outbound calls made by the Registry server

### 8.1 To the Cadastre job service (`jar:lrs-mc-integration.jar`)

Classes: `com.ils.lrs.mc.service.MultiCadastreServiceImpl` (implements `IMultiCadastreService`),
Axis 1 stub `com.ils.webservice.mc.LRSWebServicesSoapStub` / `LRSWebServicesLocator` (service QName
`{http://tempuri.org/}LRSWebServices`, port `LRSWebServicesSoap`, compiled default endpoint
`http://172.25.31.93/ApplicationServerAdmin/WebServices/LRSWebServices.asmx`).

| Setting (`lr_setting`, category `multicadastre`) | Value on .5 |
|---|---|
| `mcServiceAddress` | `http://10.10.10.6/ApplicationServerAdmin/WebServices/LRSWebServices.asmx` (code default `http://localhost/ApplicationServerAdmin/WebServices/LRSWebServices.asmx`) |
| `mcUserName` | a Cadastre account name |
| `mcPassword` | `<redacted, in lr_setting>` |
| `mcAsyncMode` | read by `IntegrationServiceImpl` (row not in export) |
| `mcParcelAttribute` | read by `CreateParcel(Batch)` / `BuildParamsByEntity` (row not in export) |
| `mcJob_multipleTargetProperties` | `Deed Plan Creationv2` |

Stub settings: `setTimeout(60000)`, `setMaintainSession(true)` (cookie session). Each call does
`Authenticate(user, password)` first (`Login`), then the operation, then `LogOut` (inferred from
method order; error "Unexpected MultiCadastreService error occurred while Authenticate call",
message key `lrs.service.mc_err_connect_server` = "The connection to ILS MultiCadastre server could
not be established.").

Cadastre operations the Registry calls: `Authenticate`, `LogOut`, `GetAllJobs` (find job type by
name), `CreateJobInstance`, `UpdateJobInstance`, `GetJobInstance`, `DeleteJobInstance`, `ApproveJob`,
`RejectJob`, `BatchUpdateDataFromRegistry`, `GetParcelPointsData` (javap: `invokevirtual
LRSWebServicesSoapStub.*`). `CreateParcel(Batch)` builds parameters `name=value|…` from
`mcParcelAttribute` and creates a job per spatial unit, stamping `LrsTransactionNumber`.

The full LRSWebServices contract (SOAP 1.1 + 1.2 bindings, SOAPAction `http://tempuri.org/{op}`):

**LRSWebServices.wsdl** — tns `http://tempuri.org/`, address in file `http://172.25.31.93/ApplicationServerAdmin/WebServices/LRSWebServices.asmx`

| Operation | Request fields | Response fields | Notes |
|---|---|---|---|
| `GetAllJobs` | — | GetAllJobsResult: ArrayOfWFJob? |  |
| `GetAllJobsInstances` | filter: int | GetAllJobsInstancesResult: ArrayOfDataFieldRow? |  |
| `IsJobInstanceCompleted` | jobinstanceID: int | IsJobInstanceCompletedResult: JobInstanceStatus? |  |
| `ApproveJob` | jobInstanceID: int | ApproveJobResult: boolean |  |
| `RejectJob` | jobInstanceID: int | RejectJobResult: boolean |  |
| `CreateJobInstance` | jobID: int, instanceName: string?, priority: unsignedShort, comments: string?, parameters: string? | CreateJobInstanceResult: JobInstance? |  |
| `CompleteJobInstance` | jobInstanceID: int | CompleteJobInstanceResult: boolean |  |
| `DeleteJobInstance` | jobInstanceId: int | DeleteJobInstanceResult: boolean |  |
| `CompleteTaskInstance` | jobInstanceID: int | CompleteTaskInstanceResult: boolean |  |
| `CancelTaskInstance` | jobInstanceID: int | CancelTaskInstanceResult: boolean |  |
| `GetCurrentTaskInstance` | jobInstanceID: int | GetCurrentTaskInstanceResult: WFTaskInstance? |  |
| `UpdateJobInstance` | jobInstanceID: int, instanceName: string?, priority: char, comments: string?, parameters: string? | UpdateJobInstanceResult: boolean |  |
| `GetJobInstanceParameters` | jobInstanceId: int | GetJobInstanceParametersResult: string? |  |
| `UpdateJobInstanceParameters` | jobInstanceID: int, parameters: string? | UpdateJobInstanceParametersResult: boolean |  |
| `GetJobInstance` | jobInstanceId: int | GetJobInstanceResult: JobInstance? |  |
| `GetParcelAttributesByUPIN` | regionName: string?, upin: string? | GetParcelAttributesByUPINResult: ArrayOfDataFieldRow? |  |
| `GetConfiguredRegions` | — | GetConfiguredRegionsResult: ArrayOfString? |  |
| `GetDomains` | — | GetDomainsResult: ArrayOfString? |  |
| `GetDomainDetailsByName` | domainName: string? | GetDomainDetailsByNameResult: ArrayOfDataFieldRow? |  |
| `GetParcelStatus` | regionName: string?, upin: string? | GetParcelStatusResult: ArrayOfDataFieldRow? |  |
| `GetUpinsByJobInstance` | jobInstanceID: int | GetUpinsByJobInstanceResult: ArrayOfString? |  |
| `GetUpinsByTransaction` | transactionNumber: int | GetUpinsByTransactionResult: ArrayOfString? |  |
| `GetJobStatusByTransaction` | transactionNumber: int | GetJobStatusByTransactionResult: ArrayOfWFJob? |  |
| `GetJobStatusByJobInstanceID` | jobInstanceID: int | GetJobStatusByJobInstanceIDResult: WFJob? |  |
| `BatchUpdateDataFromRegistry` | parameters: string? | BatchUpdateDataFromRegistryResult: BatchUpdateStatus? |  |
| `GetParcelPointsData` | pUpin: string? | GetParcelPointsDataResult: ArrayOfPointInformationDTO? |  |
| `Authenticate` | user: string?, password: string? | AuthenticateResult: boolean |  |
| `LogOut` | — | — |  |

`JobInstance{LrsTransactionNumber?, ID, JobID, TaskID, Sid, Priority, Started~, Updated~, Ended~,
Comment?, Name?, Parameters?, JobStatus}`, `WFJob{ID, Name?, Description?, StartTaskId, EndTaskId,
WorkflowDesignXML?}`, `WFTaskInstance{ID, JobID, TaskID, Sid, Assigned, Started, Ended, Comment?,
JobInstanceID}`, `DataFieldRow{FieldsRow{DataField{FieldName?, FieldValue?}[]}}`,
`BatchUpdateStatus{Completed, HasError, Messages: string[]}`, `PointInformationDTO{FromPointName?,
FromNorth, FromEast, ToPointName?, ToNorth, ToEast, Distance, Azimuth?, Bearing?}`. This repo already
implements this service (`crates/cadastre-server/src/routes/mcservice.rs`, `docs/registry-integration.md` §3.2).

### 8.2 Other outbound

* Reports: to itself at `localhost:8080/lrs` (`reports.service.connection.*`).
* E-mail: `reports.service.scheduler_from_address`, `business.service.notify_from_address` (SMTP
  config not captured).
* `/proxy` (`HTTPProxyServlet`) forwards browser requests (Web Access map) to the Cadastre GIS viewer
  `mcintegration/LWAMapURL` = `http://10.10.10.6/GISViewer/MapWorker.aspx` (inferred from settings).

### 8.3 Calls from Registry clients to the Cadastre (for completeness)

* Desktop map: `mcintegration/GISServer` = `http://10.10.10.6/GISViewer/MapWorker.aspx`, posted with
  `auth=1&lrsuser={user}&lrsticket={ticket}…` (`MapWorkerControl.cs:251`); the Cadastre validates the
  ticket with **`AuthenticationService.isTicketValid`** on the Registry.
* Desktop `CadastralWebService` proxy (`MapWorker.CadastralService.CadastralWebService`, default
  `http://localhost:2222/MCPortal/WebServices/CadastralWebService.asmx`, with a `SecurityHeader`):
  update records, detect neighbours (not called on PLAGIS's config, inferred: no URL override).

---

## 9. Replica contract

### 9.1 Must be byte-compatible (other systems call these)

| # | Endpoint / operation | Caller(s) | Notes |
|---|---|---|---|
| 1 | `POST /lrs/api/AuthenticationService` `startSession`, `endSession`, `isTicketValid` | Cadastre MCServer & desktop RegistryExtension, this repo's cadastre-server, Registry Desktop, Cashier, Cadastre GISViewer ticket check | `AuthenticationResult{username, ticket, sessionid}`; ticket format free but opaque; keep `TICKET_` prefix (desktop SSO parses `|TICKET_`) |
| 2 | WS-Security receiver: `UsernameToken` (password = ticket, PasswordText, optional Nonce/Created), `Timestamp`; ignore WS-Addressing; clock-skew tolerance | all | §2 |
| 3 | `POST /lrs/api/DomainModelService` `loadObjects` and `loadObject` with `SimpleQuery` HQL → `ResultScalar` (and `ResultCollection` when `@className`) | Cadastre MCServer (`loadObjects`), this repo, RegistryExtension (gisintegration query templates), desktop (`loadObject`) | needs an HQL→SQL layer for the model **and** jBPM's `TaskInstance` |
| 4 | `POST /lrs/api/IntegrationService` `SaveEntity` (Parcel modified, IncomingDocument added with pages) | Cadastre MCServer / this repo | §3.1 |
| 5 | `PUT /lrs/upload/{guid}.{ext}?ticket=…` returning a content URL accepted back in `cm:content` | Cadastre MCServer / this repo, desktop scanning | |
| 6 | `POST /lrs/api/AccessControlService` `getParentAuthorities`; `POST /lrs/api/AdministrationService` `createUsers`, `changePassword` | this repo (sign-in groups, user provisioning) | |
| 7 | `GET /lrs/api/{Service}?wsdl` (+ the `.xsd` files) and an HTTP `Date` header | this repo's clock offset; any client re-importing | |
| 8 | `/lrswa/welcome.do?user=&ticket=#…` deep links | desktop | if desktop kept |

### 9.2 Needed only while the old Registry Desktop runs in parallel

`DomainModelService` (`loadObject`, `saveOrUpdate`, `deleteObjects`, `createInstance`, `generateNumber`,
`getSchema`, `getRegisteredSchemas`) with the **exact JAXB XML entity format** in `items`;
`ReceptionService` (all 3); `WorkflowWebService` (20 ops of §4.1); `RepositoryService` (`getStores`,
`query` lucene subset of §4.3, `get` xpath subset, `queryChildren`, `queryParents`, `queryAssociated`,
`fetchMore`, `update` CML create/update/addAspect/delete/removeChild) over stores `lrs://rootStore`
and `workflow://rootStore`; `ContentService` (`read`, `write`, `clear`) + `/lrs/download`/`/lrs/d`;
`AuthoringService` (`lock`, `unlock`, `getLockStatus`, versions); `PersonWebService` (8 ops);
`AccessControlService` (`getACLs`, `getPermissions`, `hasPermissions`, `getClassPermissions`);
`AdministrationService` (`queryUsers`, `changePassword`); `DictionaryService` (`getClasses`,
`getAssociations`, `isSubClass`); `ReportsService` (`produceReport`, `deployXML`); `MessagingService`
(4 ops; may return no messages); `IntegrationService` (`SaveJob`, `GetJob`, `GetParcelPointsData`);
`CashierService` (`CalculateFee`, `GetPayment`, `GetSchema`); `FeeService.calculateFee`;
`/lrs/downloadBPM`; the SSO `GET /lrs/`. This is effectively the whole Alfresco+jBPM surface — a
strong argument for **not** running the old desktop against the new server: run the old desktop
against the old server during the parallel run and keep only §9.1 on the new one (recommendation).

### 9.3 Not needed (no caller found)

`ActionService`, `ClassificationService`, `HibernateDataAccessService` (only the BIRT designer),
CMIS (`/lrs/cmis`, `/cmisbrowse`), `BusinessLogicService` (CXF; no call sites), Alfresco web
client (`/faces`, `/navigate`, `/command`, `/ajax`, `/dwr`), web scripts (`/service`, `/wcs`,
`/168s`, Facebook), `/remoting`, `/remoteauth`, WebDAV, barcode servlets (unless printed
templates embed `/lrs/qrcode` URLs — not checked), `AccessControlService` write operations,
`AdministrationService.deleteUsers/updateUsers/fetchMoreUsers/getUser`, workflow
`deployProcessDefinition/undeployProcessDefinition/getProcessDefinitionFile/getContainerNodeRef/
getAllProcessInstances/addComment/getTaskComments` (desktop adds comments via CML instead).

### 9.4 Outbound contract the new server must keep

Call the Cadastre's `LRSWebServices.asmx` exactly as §8.1 (`Authenticate` with cookie session,
`GetAllJobs`, `CreateJobInstance`, `UpdateJobInstance`, `GetJobInstance`, `DeleteJobInstance`,
`ApproveJob`, `RejectJob`, `BatchUpdateDataFromRegistry`, `GetParcelPointsData`, `LogOut`), keeping
`lr_mc_job` rows as `SaveJob` does, and reading the same `lr_setting` keys.

---

## 10. Gaps / still to capture

1. **Wire captures.** No real request/response pair from .5 is in the captures (except what
   `lrs.rs` tests encode). Capture, read-only, one `loadObject` with `@className` (to get the exact
   JAXB XML of an entity), one `ResultScalar`, one `startSession`, one WSE-built request from a
   desktop (to confirm the WS-Addressing/Nonce shape), one SOAP fault for an expired ticket, and the
   `PUT /lrs/upload` response body.
2. **CXF endpoints.** No CXF Spring config captured: is `/lrs/cxfapi/RepositoryWebService` (desktop
   setting) or `/lrs/cxfapi/BusinessLogicService` live? Fetch `GET /lrs/cxfapi/` service list.
3. **Spring bean config** (`web-application-context.xml`, `*-services-context.xml`, Alfresco
   `*-context.xml` in jars): ticket lifetime, WSS4J timestamp TTL / future-time tolerance, the
   `remoting` services, the `RemoteAuthenticationServlet` contract.
4. **Server-side code of the LRS services** was not decompiled (rule: javap only). The behaviour
   of `DomainModelServiceSOAPImpl`, `ReceptionServiceSOAPImpl.AcceptApplication/SyncData/Withdrawal`,
   `CashierServiceSOAPImpl`, `FeeServiceImpl` is inferred from client use; a `javap -c` pass over
   `lrs-domain-model.jar` and `lrs-web.jar` would pin down validation and side effects (queue-node
   updates, counters).
5. **Web Access `.do` controllers** (`lrswa.war` jars) — covered elsewhere; not listed here.
6. **Cashier web app proxy** (`LRS.Client.ServiceProxy`) not in the capture; which Cashier operations
   it calls is unknown.
7. **Messaging**: event names and payloads (`SubscriptionEventType`) not traced; whether the desktop
   works with an always-empty `receiveMessage` is unverified.
8. **Report names/parameters** used by `produceReport` (in the desktop configs / server repository)
   not listed here.
9. `multicadastre/mcAsyncMode` and `mcParcelAttribute` rows were not in the `lr_setting` export
   (code reads them).
10. The `PERSON_SERVICE` fault namespace in the client (`http://www.alfresco.org/ws/service/person/1.0`)
    differs from the server tns — check what the server's `PersonServiceFault` detail really uses.
11. ACL model: which `Manager` ACEs exist on process definitions (controls which workflows a user can
    start) — needs a read of the Alfresco ACL tables.

---

## Appendix A — HQL strings in the Registry Desktop (verbatim)

Extracted by `grep` over `$D/net/**.cs` for string literals beginning with `select`/`from` (case
insensitive), grouped as `assembly class`, then `line  "query"`. `{…}` are C# interpolations;
`{ClassNames.ResolveDataModelType(typeof(X))}` = `com.landsystems.lrs.model.X`. Strings that are
only the start of a query are completed in code with `and …` clauses and `?` parameters.

```
## Controls ApplicantSearchForm
   176 "from {ClassNames.ResolveDataModelType<Party>()} a join a.Person p where 1=1 "
   195 "from {ClassNames.ResolveDataModelType<Party>()} a2 join a2.Person p2 where 1=1 "
## Controls AssessWorker
   236 "from com.landsystems.lrs.model.Batch where id=?"
   483 "from com.landsystems.lrs.model.Batch where id=?"
   766 "from com.landsystems.lrs.model.Batch where id=?"
## Controls AssessWorkerPresenter
   196 "select sc from {ClassNames.ResolveDataModelType(typeof(LrSimpleCounter))} sc where counter =?"
## Controls BlockingTransactionPagePresenter
   101 "select la from {0} la where 1=1"
   122 "select tr from {0} tr where 1=1"
   157 "select tr from {0} tr where 1=1"
## Controls ChangeFileNo
   54 "select tr from {ClassNames.ResolveDataModelType(typeof(Transaction))} tr where tr.TransactionStatus.Code<>'transaction_withdrawn' and (tr.SourcePropertyIDs = ?)"
## Controls CommentHistoryPanel
   37 "from com.landsystems.lrs.model.Batch where id=?"
## Controls DataCorrectionRrrPagePresenter
   154 "select t from {ClassNames.ResolveDataModelType(typeof(Transaction))} t where 1=1"
## Controls DataModelQueries
   22 "SELECT feeStrusture FROM {ClassNames.ResolveDataModelType(typeof(FeeStructure))} feeStrusture"
   44 "SELECT feeStrusture FROM {ClassNames.ResolveDataModelType(typeof(FeeStructure))} feeStrusture WHERE feeStrusture.id={id} "
   109 "SELECT transac FROM {ClassNames.ResolveDataModelType(typeof(TransactionMetaData))} transac"
   211 "SELECT t FROM {ClassNames.ResolveDataModelType(typeof(Dictionary))} t WHERE t.Code like '%{category.code}%'"
   263 "from com.landsystems.lrs.model.FeeRate where 1=1"
   391 "SELECT feeStructure FROM {ClassNames.ResolveDataModelType(typeof(FeeStructure))} feeStructure"
   413 "SELECT feeRate FROM {ClassNames.ResolveDataModelType(typeof(FeeRate))} feeRate WHERE feeRate.id={id}"
   435 "SELECT t FROM {ClassNames.ResolveDataModelType(typeof(FeeStructure))} t WHERE t.id={id}"
   457 "SELECT t FROM {ClassNames.ResolveDataModelType(typeof(FeeRate))} t WHERE t.IdFeeStructure={feeStructure.Id}"
## Controls DataModelUtilities
   27 "select t from {ClassNames.ResolveDataModelType(typeof(Transaction))} t join t.Properties p join p.SpatialUnits su where 1=1"
   44 "select t from {ClassNames.ResolveDataModelType(typeof(Transaction))} t join t.Properties p join p.SpatialUnits su where 1=1"
   59 "select su from {ClassNames.ResolveDataModelType(typeof(Transaction))} t join t.Properties p join p.SpatialUnits su where 1=1"
   80 "select t from {ClassNames.ResolveDataModelType(typeof(Transaction))} t join t.Properties p where 1=1"
   96 "from {ClassNames.ResolveDataModelType(typeof(LAUnit))} p where 1=1"
   128 "select o from {ClassNames.ResolveDataModelType(typeof(Right))} r join r.LaUnit p join r.Party o where 1=1"
   154 "select o from {ClassNames.ResolveDataModelType(typeof(Transaction))} t join t.Parties o where 1=1"
   169 "from {ClassNames.ResolveDataModelType(typeof(LAUnitLineage))} ll where 1=1 and ll.ArchiveDate is null and ll.ParentUid='{parentPropertyID}'"
   183 "from {ClassNames.ResolveDataModelType(typeof(Right))} r join r.LaUnit p join r.Party o where 1=1"
   203 "from {ClassNames.ResolveDataModelType(typeof(TransactionMetaData))} where RightType='right_type_ownership'"
   226 "from {ClassNames.ResolveDataModelType(typeof(TransactionMetaData))} where RightType='right_type_ownership'"
   39 "select t from {ClassNames.ResolveDataModelType(typeof(Transaction))} t join t.Properties p join p.SpatialUnits su where 1=1"
   57 "select t from {ClassNames.ResolveDataModelType(typeof(Transaction))} t where 1=1"
   99 "select su from {ClassNames.ResolveDataModelType(typeof(Transaction))} t join t.Properties p join p.SpatialUnits su where 1=1"
   120 "from {0} where Code='"
   138 "from {ClassNames.ResolveDataModelType(typeof(TransactionMetaData))} "
   176 "select t from {ClassNames.ResolveDataModelType(typeof(Transaction))} t join t.Properties p where 1=1"
   189 "select r from {ClassNames.ResolveDataModelType(typeof(RrrMetaData))} r where 1=1"
   272 "from {ClassNames.ResolveDataModelType(typeof(Transaction))} t where 1=1"
   316 "select r from {ClassNames.ResolveDataModelType(typeof(RrrMetaData))} r where 1=1"
   356 "select r from {ClassNames.ResolveDataModelType(typeof(RrrMetaData))} r where 1=1"
   370 "select r from {ClassNames.ResolveDataModelType(typeof(RRR))} r where 1=1"
   426 "select r from {ClassNames.ResolveDataModelType(typeof(RRR))} r , {ClassNames.ResolveDataModelType(typeof(Transaction))} t where 1=1"
   430 "select r from {ClassNames.ResolveDataModelType(typeof(RRR))} r where 1=1"
   500 "from {ClassNames.ResolveDataModelType(typeof(RentalAmountRevision))} where 1=1"
   517 "select r from {ClassNames.ResolveDataModelType(typeof(RRR))} r where 1=1"
   535 "select r from {ClassNames.ResolveDataModelType(typeof(RRR))} r where 1=1"
   565 "select r from {ClassNames.ResolveDataModelType(typeof(RRR))} r where 1=1"
   607 "select r from {ClassNames.ResolveDataModelType(typeof(RRR))} r where 1=1"
   677 "select r from {ClassNames.ResolveDataModelType(typeof(RRR))} r where 1=1"
   732 "select r from {ClassNames.ResolveDataModelType(typeof(RrrMetaData))} r where 1=1"
   841 "from {ClassNames.ResolveDataModelType(typeof(LAUnit))} p join p.SpatialUnits s where 1=1"
   863 "from {ClassNames.ResolveDataModelType(typeof(LAUnit))} p join p.SpatialUnits s where 1=1"
   885 "from {ClassNames.ResolveDataModelType(typeof(LAUnit))} p where 1=1"
   906 "from {ClassNames.ResolveDataModelType(typeof(LAUnit))} p where 1=1"
   958 "select o from {ClassNames.ResolveDataModelType(typeof(Right))} r join r.LaUnit p join r.Party o where 1=1"
   1001 "select t from {ClassNames.ResolveDataModelType(typeof(Transaction))} t where 1=1"
   1032 "select o from {ClassNames.ResolveDataModelType(typeof(Right))} r join r.LaUnit p join r.Party o where 1=1"
   1068 "select o from {ClassNames.ResolveDataModelType(typeof(Transaction))} t join t.Parties o where 1=1"
   1089 "from {ClassNames.ResolveDataModelType(typeof(RentalAmountRevision))} p where 1=1"
   1106 "from {0} ll where ll.ParentUid=?"
   1120 "from {ClassNames.ResolveDataModelType(typeof(Right))} r join r.LaUnit p join r.Party o where 1=1"
   1140 "from {ClassNames.ResolveDataModelType(typeof(TransactionMetaData))} where RightType='right_type_ownership'"
   1163 "from {ClassNames.ResolveDataModelType(typeof(TransactionMetaData))} where RightType='right_type_ownership'"
   1431 "from com.landsystems.lrs.model.TransactionMetaData where GroupCode='"
   1438 "from com.landsystems.lrs.model.TransactionMetaData order by Code"
   1448 "from {ClassNames.ResolveDataModelType(typeof(Transaction))} where TransactionNumber = '{transactionNumber}'"
   1458 "from {ClassNames.ResolveDataModelType(typeof(TransactionMetaData))} where Code = '{transactionTypeCode}'"
   1468 "from {ClassNames.ResolveDataModelType(typeof(RRR))} where TransactionSource = '{transactionNumber}'"
   1478 "from {ClassNames.ResolveDataModelType(typeof(RRR))} r1 where r1.TransactionSource = '{transactionSourceNumber}'"
   1492 "from {ClassNames.ResolveDataModelType(typeof(RRR))} where TransactionSource = '{transactionSourceNumber}' and Party.Id = '{item.partyId}' and LaUnit.Uid = '{item.propertyNo}'"
   1512 "from {0} where Code='"
   1630 "select t from {ClassNames.ResolveDataModelType(typeof(Transaction))} t join t.Properties p join p.SpatialUnits su where 1=1"
   1671 "from com.landsystems.lrs.model.Transaction where BeginLifespanVersion is not null and EndLifespanVersion is null and "
   1672 "from com.landsystems.lrs.model.Transaction tr where tr.TransactionStatus.Code='transaction_completed' and tr.TransactionType.RightType='right_type_ownership' and tr.TransactionType.ActionType=0 and tr.TransactionSource.Code<>'past'"
   1721 "from com.landsystems.lrs.model.Right where BeginLifespanVersion is not null and EndLifespanVersion is null and "
   1724 "select r from {ClassNames.ResolveDataModelType(typeof(Right))} r join r.LaUnit p join r.Party o where 1=1"
   1796 "select t from {ClassNames.ResolveDataModelType(typeof(Transaction))} t join t.Properties p where 1=1"
   1828 "select t from {ClassNames.ResolveDataModelType(typeof(Batch))} b join b.Transactions t where 1=1"
   2024 "from com.landsystems.lrs.model.Right where BeginLifespanVersion is not null and EndLifespanVersion is null and "
   2027 "select r from {ClassNames.ResolveDataModelType(typeof(Right))} r join r.LaUnit p join r.Party o where 1=1"
   2053 "from com.landsystems.lrs.model.Restriction where BeginLifespanVersion is not null and EndLifespanVersion is null and "
   2172 "from {0} p where p.TransactionNumber='"
   2183 "from {0} b join b.Payments p where p.ReceiptNo='"
   2448 "from {ClassNames.ResolveDataModelType(typeof(LAUnit))} p join p.SpatialUnits s where 1=1"
   2496 "from {ClassNames.ResolveDataModelType(typeof(LAUnit))} p join p.SpatialUnits s where 1=1"
   2546 "from {ClassNames.ResolveDataModelType(typeof(Transaction))} t where "
   2574 "from {ClassNames.ResolveDataModelType(typeof(LAUnit))} p join p.SpatialUnits s where 1=1"
   2616 "from {ClassNames.ResolveDataModelType(typeof(Transaction))} t join t.Properties p join p.SpatialUnits s where 1=1"
   2632 "from {ClassNames.ResolveDataModelType(typeof(SurveyPlan))} s where 1=1"
   2644 "select r from {ClassNames.ResolveDataModelType(typeof(RRR))} r where 1=1"
   2658 "select t from {ClassNames.ResolveDataModelType(typeof(Transaction))} t join t.Properties p join p.SpatialUnits s where 1=1"
   2684 "select s from {ClassNames.ResolveDataModelType(typeof(SurveyPlan))} s where 1=1"
   2742 "SELECT p from {ClassNames.ResolveDataModelType(typeof(ProcessHistory))} p "
## Controls DeliveryWorker
   64 "from com.landsystems.lrs.model.Batch where id=?"
## Controls DocumentListView
   183 "from com.landsystems.lrs.model.Dictionary where 0=1"
## Controls FeeListSelectionPage
   278 "SELECT feeStrusture FROM {ClassNames.ResolveDataModelType(typeof(FeeStructure))} feeStrusture WHERE FeeName.Id={feeId}"
## Controls FinishPage
   144 "from com.landsystems.lrs.model.Batch where id=?"
## Controls HierarchyEditorHelper
   49 "from {ClassNames.ResolveDataModelType(typeof(Dictionary))} where {filter} and category='{category}' and BeginLifespanVersion is not null and EndLifespanVersion is null order by {orderBy}"
## Controls HistoryExtView
   318 "from {ClassNames.ResolveDataModelType(typeof(Batch))} where Id in (select ParentId from {ClassNames.ResolveDataModelType(typeof(RelatedBatch))} where ChildId = {_lodgmentID} and ChildProcess < {_processID})"
   318 "from {ClassNames.ResolveDataModelType(typeof(Batch))} where Id in (select ChildId from {ClassNames.ResolveDataModelType(typeof(RelatedBatch))} where ParentId = {_lodgmentID} and ChildProcess > {_processID})"
## Controls ListOfInstruments
   138 "from {ClassNames.ResolveDataModelType<Batch>()} where id=?"
   433 "from {ClassNames.ResolveDataModelType<Batch>()} where id=?"
   464 "from com.landsystems.lrs.model.SimpleCounter where WithRollback=1"
   490 "from com.landsystems.lrs.model.LAUnit where 0=1"
## Controls MapForm
   341 "from {0} where Label='"
## Controls OutstandingPagePresenter
   152 "select g from {ClassNames.ResolveDataModelType(typeof(GroundRent))} g"
## Controls PagePropReportPrinting
   101 "select su from {ClassNames.ResolveDataModelType(typeof(LAUnit))} p join p.SpatialUnits su where 1=1"
## Controls PartiesPagePresenter
   226 "select p1 from {ClassNames.ResolveDataModelType(typeof(Transaction))} t1 "
   228 "select t2.Id from {ClassNames.ResolveDataModelType(typeof(Transaction))} t2"
   229 "select max(t.CompleteDate) from {ClassNames.ResolveDataModelType(typeof(Transaction))} t"
   257 "select p from {0} t join t.Parties p where t.id = (select max(t1.id) from {0} t1 join t1.Properties lu1 where t1.TransactionType.Code in ({1}) and lu1.Uid = '{2}')"
   277 "select p from {ClassNames.ResolveDataModelType(typeof(Transaction))} t join t.Parties p join t.Properties la where t.TransactionNumber = '{relatedNumber}' and la.Uid = '{propertyUid}'"
## Controls PartyPresenter
   36 "select party "
   37 "from {ClassNames.ResolveDataModelType(typeOfRrrFromRrrMetaData)} as r, "
   70 "select rrrMetaData "
   71 "from {ClassNames.ResolveDataModelType(typeof(RrrMetaData))} as rrrMetaData "
## Controls PartySearchFormPresenter
   29 "from {ClassNames.ResolveDataModelType<Party>()} p where 1=1"
   53 "from {ClassNames.ResolveDataModelType<Individual>()} i where 1=1"
## Controls PastTransactionPage
   220 "select p from {ClassNames.ResolveDataModelType(typeof(LAUnit))} p join p.SpatialUnits su where 1=1"
## Controls PastTransactionPagePresenter
   50 "select p from {ClassNames.ResolveDataModelType(typeof(LAUnit))} p join p.SpatialUnits su where 1=1"
   144 "select t from {ClassNames.ResolveDataModelType(typeof(Transaction))} t where 1=1"
   151 "select p from {ClassNames.ResolveDataModelType(typeof(Batch))} b join b.Payments p where p.TransactionNumber in (select t.TransactionNumber from {ClassNames.ResolveDataModelType(typeof(Transaction))} t"
   158 "select t from {ClassNames.ResolveDataModelType(typeof(Batch))} b join b.Transactions t where 1=1"
   173 "select p from {ClassNames.ResolveDataModelType(typeof(Batch))} b join b.Payments p where p.TransactionNumber in (select t.TransactionNumber from {ClassNames.ResolveDataModelType(typeof(Transaction))} t"
## Controls PrintDocumentsFee
   234 "select su from {ClassNames.ResolveDataModelType(typeof(LAUnit))} p join p.SpatialUnits su where 1=1"
## Controls PrintDocumentsPage
   166 "select su from {ClassNames.ResolveDataModelType(typeof(LAUnit))} p join p.SpatialUnits su where 1=1"
## Controls PropertiesPagePresenter
   291 "select t from {ClassNames.ResolveDataModelType(typeof(Transaction))} t join t.Properties p where p.id={parcelId}"
## Controls PropertyInfoForm
   148 "select su from {ClassNames.ResolveDataModelType(typeof(LAUnit))} p join p.SpatialUnits su where 1=1"
   205 "from {ClassNames.ResolveDataModelType(typeof(Transaction))} tr join tr.Properties pr"
## Controls PropertyInfoFormPresenter
   50 "select p from {ClassNames.ResolveDataModelType(typeof(LAUnit))} p join p.SpatialUnits su where 1=1"
   82 "select t from {ClassNames.ResolveDataModelType(typeof(Transaction))} t where 1=1"
## Controls PropertyPresenter
   29 "from {classNameObject.UpdateResolvedataModelType(typeof(LAUnit))} p join p.SpatialUnits su where su.Id={property.id}"
## Controls PropertySearchFormPresenter
   21 "select su from {0} p join p.SpatialUnits su where 1=1 and p.id=(select max(p1.id) from {0} p1 where p.Uid=p1.Uid and p1.BeginLifespanVersion is not null and p1.EndLifespanVersion is null) and su.BeginLifespanVersion is not null and su.EndLifespanVersion is null"
## Controls RejectPage
   222 "from com.landsystems.lrs.model.Rejection where lodgment_id=?"
## Controls RejectPagePresenter
   77 "from com.landsystems.lrs.model.Rejection where id<>{num} and LodgmentId={(Lodgement as Batch).id}"
## Controls RelatedTransactionPagePresenter
   58 "select tr from {ClassNames.ResolveDataModelType(typeof(Transaction))} tr, {ClassNames.ResolveDataModelType(typeof(Restriction))} rr join rr.LaUnit p where 1=1"
   73 "select tr from {ClassNames.ResolveDataModelType(typeof(Transaction))} tr, {ClassNames.ResolveDataModelType(typeof(Restriction))} rr join rr.LaUnit p where 1=1"
   95 "select t from {ClassNames.ResolveDataModelType(typeof(Transaction))} t where 1=1"
## Controls RrrPage
   872 "from {ClassNames.ResolveDataModelType(typeof(Transaction))} where TransactionNumber = '{rrr.transactionSource}'"
## Controls Rules
   157 "from com.landsystems.lrs.model.Transaction where BeginLifespanVersion is not null and EndLifespanVersion is null and TransactionStatus.Code='transaction_completed' and TransactionNumber in('"
## Controls SearchApplicantView
   184 "from {ClassNames.ResolveDataModelType<Applicant>()} a join a.Person p where 1=1 "
## Controls TransactionCorrectionPagePresenter
   503 "select t from {ClassNames.ResolveDataModelType(typeof(Transaction))} t"
   511 "select t from {ClassNames.ResolveDataModelType(typeof(Transaction))} t"
## Controls TransactionDeedPage
   353 "select t from {ClassNames.ResolveDataModelType(typeof(Transaction))} t where 1=1"
   1033 "from {ClassNames.ResolveDataModelType(typeof(Transaction))} t where t.TransactionNumber<>'{transactionNumber}' and t.GeneralNumber='{generalNumber}'"
   1041 "from {ClassNames.ResolveDataModelType(typeof(Transaction))} t where t.TransactionNumber<>'{transactionNumber}' and t.SerialNumber='{serialNumber}' and t.TransactionType.Code='{transactionCode}'"
   1049 "from {ClassNames.ResolveDataModelType(typeof(Transaction))} t where t.CompleteDate is null and t.GeneralNumber='{generalNumber}'"
   1224 "from {ClassNames.ResolveDataModelType(typeof(Transaction))} t where 1=1"
   1264 "select p from {ClassNames.ResolveDataModelType(typeof(Transaction))} t join t.Properties p join p.SpatialUnits su where 1=1"
## Controls TransactionLayoutControlPresenter
   46 "select p from {ClassNames.ResolveDataModelType(typeof(Transaction))} t join t.Properties p join p.SpatialUnits su where 1=1"
   82 "from com.landsystems.lrs.model.TransactionMetaData where GroupCode<>'BLT' order by Code"
   85 "from com.landsystems.lrs.model.TransactionMetaData order by Code"
   94 "from {ClassNames.ResolveDataModelType(typeof(Transaction))} t where 1=1 and t.TransactionStatus.Code=? and t.Id<>?"
   129 "select t from {ClassNames.ResolveDataModelType(typeof(Batch))} b join b.Transactions t where 1=1"
   158 "select t from {ClassNames.ResolveDataModelType(typeof(Batch))} b join b.Transactions t left join t.Properties p where 1=1"
   207 "select t from {ClassNames.ResolveDataModelType(typeof(Transaction))} t where 1=1"
## Controls TransactionPresenter
   54 "from com.landsystems.lrs.model.TransactionMetaData order by Code"
   54 "from com.landsystems.lrs.model.TransactionMetaData where GroupCode<>'BLT' order by Code"
## Controls TransactionSearchFormPresenter
   29 "select t from {ClassNames.ResolveDataModelType<Transaction>()} t where 1=1"
## Controls TransactionWithDocumentsPresenter
   106 "select t from {text} t where 1=1 and t.TransactionType.Code='APC' and t.TransactionStatus.Code='transaction_completed'  and t.SourcePropertyIDs=?"
## Controls WorkerFactory
   67 "from com.landsystems.lrs.model.Batch where id=?"
   206 "from com.landsystems.lrs.model.Batch where id=?"
   284 "from com.landsystems.lrs.model.Batch where id=?"
   353 "from com.landsystems.lrs.model.Batch where id=?"
   562 "from com.landsystems.lrs.model.Batch where id=?"
## Controls WorkerHelper
   49 "from com.landsystems.lrs.model.Rejection where id=?"
## DataModel BusinessLogicServiceAgent
   57 "select su from {ClassNames.ResolveDataModelType(typeof(LAUnit))} p join p.SpatialUnits su where 1=1"
   58 "select su from {ClassNames.ResolveDataModelType(typeof(LAUnit))} p join p.SpatialUnits su where 1=1"
   120 "select tr from "
   154 "select t from {ClassNames.ResolveDataModelType(typeof(Batch))} b join b.Transactions t where 1=1"
   180 "select t from {ClassNames.ResolveDataModelType(typeof(Transaction))} t join t.Properties p where 1=1"
   213 "from com.landsystems.lrs.model.Batch where id=?"
   232 "from {ClassNames.ResolveDataModelType(typeof(Transaction))} t where 1=1"
   255 "from com.landsystems.lrs.model.Batch where id=?"
   282 "from com.landsystems.lrs.model.Batch where id=?"
   285 "from com.landsystems.lrs.model.ProcessHistory where ProcessId="
   402 "from com.landsystems.lrs.model.Batch where id=?"
## DataModel DomainModelServiceAgent
   74 "from com.landsystems.lrs.model.RegistryOffice where office_name='{namedValue}'"
   81 "from com.landsystems.lrs.model.RegistryOffice"
   82 "from com.landsystems.lrs.model.RegistryOffice where active=1 and office_type='registration'"
   253 "from com.landsystems.lrs.model.TransactionMetaData where hidden=0 order by sort_order,label"
   334 "from com.landsystems.lrs.model.Dictionary where category=? and hidden=0 and begin_lifespan_version is not null and end_lifespan_version is null order by sort_order,label"
   493 "from com.landsystems.lrs.model.REGISTRY"
   526 "from com.landsystems.lrs.model.Setting where category=?"
   649 "from com.landsystems.lrs.model.FormField where form_name=? order by id"
## DataModel QueryNames
   5 "from com.landsystems.lrs.model.RegistryOffice"
   7 "from com.landsystems.lrs.model.RegistryOffice where active=1 and office_type='registration'"
   9 "from com.landsystems.lrs.model.RegistryOffice where office_name='{0}'"
   11 "from com.landsystems.lrs.model.Batch where id=?"
   13 "from com.landsystems.lrs.model.Transaction where batch=? and isDelivered=?"
   15 "from com.landsystems.lrs.model.Rejection where id=?"
   17 "from com.landsystems.lrs.model.Setting where category=? and name=?"
   19 "from com.landsystems.lrs.model.Setting where category=?"
   21 "from com.landsystems.lrs.model.LrWorkflowLog where task_id=?"
   23 "from com.landsystems.lrs.model.REGISTRY"
   25 "from com.landsystems.lrs.model.TransactionMetaData where hidden=0 order by sort_order,label"
   27 "from com.landsystems.lrs.model.GroundRent where propertyNumber=? and paidYear=?"
## PLAGIS.Controls CustomAssessWorkerPresenter
   107 "select p from {ClassNames.ResolveDataModelType(typeof(Party))} p "
## PLAGIS.Controls CustomPastTransactionPagePresenter
   319 "select r from {ClassNames.ResolveDataModelType(typeof(RrrMetaData))} r"
## PLAGIS.Controls CustomRelatedTransactionPagePresenter
   38 "select t from {ClassNames.ResolveDataModelType(typeof(Transaction))} t where 1=1"
## PLAGIS.Controls CustomRules
   511 "from com.landsystems.lrs.model.GroundRent where propertyNumber=? and paidYear=?"
## PLAGIS.Controls CustomTransactionForm
   71 "from {ClassNames.ResolveDataModelType(typeof(Transaction))} t where 1=1"
## PLAGIS.Controls CustomTransactionFormPresenter
   33 "from com.landsystems.lrs.model.Dictionary where 0=1"
## PLAGIS.Controls CustomTransactionLayoutControlPresenter
   84 "from com.landsystems.lrs.model.TransactionMetaData order by label"
## RegistryDesktop ShellContext
   222 "from {ClassNames.ResolveDataModelType(typeof(RegistryOffice))} r where r.Active = true "
## Services EntityDictionaryProvider
   28 "from {ClassNames.ResolveDataModelType<T>()} where 1=1  order by sort_order,label"
   33 "from {ClassNames.ResolveDataModelType<T>()} where code=? order by sort_order,label"
```
