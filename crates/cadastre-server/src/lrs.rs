//! READ-ONLY client for the Aumentum Registry (LRS43, `http://10.10.10.5:8080/lrs/api/`), built
//! exactly as Aumentum's Cadastre server called it (MCServer `IntegrationDataProvider`):
//!
//! 1. `AuthenticationService.startSession(username, password)` → a ticket;
//! 2. `DomainModelService.loadObjects` with an HQL `SimpleQuery`, a WS-Security header carrying a
//!    `Timestamp` (Created / Expires +600 s) and a `UsernameToken` (Username = the account,
//!    Password = the ticket);
//! 3. `AuthenticationService.endSession(ticket)`.
//!
//! Only reads are offered here. The Registry checks the header's timestamp against its own clock,
//! which on PLAGIS's network runs ~19 minutes ahead of the PCs, so the timestamp is taken from the
//! Registry's HTTP `Date` header (as if the request were made on the Registry server).

use chrono::{Duration, NaiveDateTime, Utc};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const SOAP: &str = "http://schemas.xmlsoap.org/soap/envelope/";
const AUTH: &str = "http://www.alfresco.org/ws/service/authentication/1.0";
const DOM: &str = "http://www.landsystems.com/domainmodel-service/1.0";
const LRS: &str = "http://www.landsystems.com/lrs/service";
const WSSE: &str = "http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-wssecurity-secext-1.0.xsd";
const WSU: &str = "http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-wssecurity-utility-1.0.xsd";
const INT: &str = "http://www.landsystems.com/lrs-integration-service/1.0";
const CMS: &str = "http://www.alfresco.org/ws/model/content/1.0";
const LRS_CORE: &str = "http://www.landsystems.com/lrs/model/core/1.0";
const XSI: &str = "http://www.w3.org/2001/XMLSchema-instance";
const PWTEXT: &str = "http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-username-token-profile-1.0#PasswordText";

/// Connection settings (Aumentum's `RegParam`: server, port, protocol, user, password).
#[derive(Clone)]
pub struct LrsConfig {
    /// e.g. `http://10.10.10.5:8080/lrs/api/`
    pub base_url: String,
    pub user: String,
    pub password: String,
    /// Writes to the Registry (send a plan, link/unlink from the Cadastre) are allowed:
    /// `CADASTRE_REGISTRY_WRITE=1`. Off by default; reads always work.
    pub write_enabled: bool,
}

impl std::fmt::Debug for LrsConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LrsConfig").field("base_url", &self.base_url).field("user", &self.user).field("password", &"****").field("write_enabled", &self.write_enabled).finish()
    }
}

impl LrsConfig {
    /// `CADASTRE_REGISTRY_URL`, `CADASTRE_REGISTRY_USER`, `CADASTRE_REGISTRY_PASSWORD`; `None`
    /// when any is unset (the Registry sync is then off).
    pub fn from_env() -> Option<Self> {
        let get = |k: &str| std::env::var(k).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
        Some(Self {
            base_url: get("CADASTRE_REGISTRY_URL")?,
            user: get("CADASTRE_REGISTRY_USER")?,
            password: get("CADASTRE_REGISTRY_PASSWORD")?,
            write_enabled: get("CADASTRE_REGISTRY_WRITE").is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true")),
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LrsError {
    #[error("Registry connection: {0}")]
    Http(String),
    #[error("Registry refused: {0}")]
    Fault(String),
    #[error("Registry sign-in returned no ticket")]
    NoTicket,
    #[error("Registry response: {0}")]
    Parse(String),
}

/// One signed-in session. Dropping it does not sign out: call [`Session::end`].
pub struct Session {
    cfg: LrsConfig,
    ticket: String,
    /// Registry clock minus this machine's clock.
    offset: Duration,
}

pub fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

struct HttpResponse {
    status: u16,
    date: Option<String>,
    body: String,
}

/// Minimal HTTP/1.0 request over plain TCP (the Registry serves `http://` only). HTTP/1.0 and
/// `Connection: close` keep the response unchunked and read to EOF.
async fn http(url: &str, method: &str, headers: &[(&str, String)], body: &str) -> Result<HttpResponse, LrsError> {
    http_bytes(url, method, headers, body.as_bytes()).await
}

async fn http_bytes(url: &str, method: &str, headers: &[(&str, String)], body: &[u8]) -> Result<HttpResponse, LrsError> {
    let rest = url.strip_prefix("http://").ok_or_else(|| LrsError::Http(format!("only http:// is supported: {url}")))?;
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => (h, p.parse::<u16>().map_err(|_| LrsError::Http(format!("bad port in {url}")))?),
        None => (authority, 80),
    };
    let fut = async {
        let mut stream = TcpStream::connect((host, port)).await.map_err(|e| LrsError::Http(format!("connect {authority}: {e}")))?;
        let mut req = format!("{method} {path} HTTP/1.0\r\nHost: {authority}\r\nConnection: close\r\nContent-Length: {}\r\n", body.len());
        for (k, v) in headers {
            req.push_str(&format!("{k}: {v}\r\n"));
        }
        req.push_str("\r\n");
        stream.write_all(req.as_bytes()).await.map_err(|e| LrsError::Http(format!("write: {e}")))?;
        stream.write_all(body).await.map_err(|e| LrsError::Http(format!("write: {e}")))?;
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.map_err(|e| LrsError::Http(format!("read: {e}")))?;
        Ok::<_, LrsError>(raw)
    };
    let raw = tokio::time::timeout(std::time::Duration::from_secs(120), fut).await.map_err(|_| LrsError::Http("timed out".into()))??;
    let text = String::from_utf8_lossy(&raw);
    let (head, body) = text.split_once("\r\n\r\n").ok_or_else(|| LrsError::Http("malformed HTTP response".into()))?;
    let status = head.lines().next().and_then(|l| l.split_whitespace().nth(1)).and_then(|s| s.parse().ok()).unwrap_or(0);
    let date = head.lines().find_map(|l| l.split_once(':').filter(|(k, _)| k.eq_ignore_ascii_case("date")).map(|(_, v)| v.trim().to_string()));
    Ok(HttpResponse { status, date, body: body.to_string() })
}

async fn soap(cfg: &LrsConfig, service: &str, action: &str, header: &str, body: &str) -> Result<String, LrsError> {
    let envelope = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?><soap:Envelope xmlns:soap=\"{SOAP}\"><soap:Header>{header}</soap:Header><soap:Body>{body}</soap:Body></soap:Envelope>"
    );
    let url = format!("{}{service}", base(cfg));
    let r = http(&url, "POST", &[("Content-Type", "text/xml; charset=utf-8".into()), ("SOAPAction", format!("\"{action}\""))], &envelope).await?;
    if r.status != 200 {
        let fault = between(&r.body, "<faultstring>", "</faultstring>").unwrap_or_else(|| r.body.chars().take(300).collect());
        return Err(LrsError::Fault(format!("{service} HTTP {}: {}", r.status, fault.trim())));
    }
    Ok(r.body)
}

fn base(cfg: &LrsConfig) -> String {
    format!("{}/", cfg.base_url.trim_end_matches('/'))
}

fn between(s: &str, a: &str, b: &str) -> Option<String> {
    let i = s.find(a)? + a.len();
    let j = s[i..].find(b)? + i;
    Some(s[i..j].to_string())
}

/// The Registry's clock minus ours, from its HTTP `Date` header (0 if it sends none).
async fn clock_offset(cfg: &LrsConfig) -> Result<Duration, LrsError> {
    let r = http(&format!("{}DomainModelService?wsdl", base(cfg)), "GET", &[], "").await?;
    let Some(d) = r.date.and_then(|d| chrono::DateTime::parse_from_rfc2822(&d).ok()) else { return Ok(Duration::zero()) };
    Ok(d.with_timezone(&Utc) - Utc::now())
}

impl Session {
    pub async fn start(cfg: &LrsConfig) -> Result<Self, LrsError> {
        let offset = clock_offset(cfg).await?;
        let body = format!(
            "<auth:startSession xmlns:auth=\"{AUTH}\"><auth:username>{}</auth:username><auth:password>{}</auth:password></auth:startSession>",
            escape(&cfg.user), escape(&cfg.password)
        );
        let xml = soap(cfg, "AuthenticationService", &format!("{AUTH}/startSession"), "", &body).await?;
        let ticket = find_text(&xml, "ticket").ok_or(LrsError::NoTicket)?;
        Ok(Self { cfg: cfg.clone(), ticket, offset })
    }

    /// The Registry clock minus this machine's, in seconds.
    pub fn clock_offset_secs(&self) -> i64 {
        self.offset.num_seconds()
    }

    fn security_header(&self) -> String {
        let now = Utc::now() + self.offset;
        let created = now.format("%Y-%m-%dT%H:%M:%SZ");
        let expires = (now + Duration::seconds(600)).format("%Y-%m-%dT%H:%M:%SZ");
        let id = uuid::Uuid::new_v4();
        format!(
            "<wsse:Security xmlns:wsse=\"{WSSE}\" xmlns:wsu=\"{WSU}\"><wsu:Timestamp wsu:Id=\"Timestamp-{id}\"><wsu:Created>{created}</wsu:Created><wsu:Expires>{expires}</wsu:Expires></wsu:Timestamp>\
             <wsse:UsernameToken wsu:Id=\"SecurityToken-{id}\"><wsse:Username>{}</wsse:Username><wsse:Password Type=\"{PWTEXT}\">{}</wsse:Password><wsu:Created>{created}</wsu:Created></wsse:UsernameToken></wsse:Security>",
            escape(&self.cfg.user), escape(&self.ticket)
        )
    }

    /// Run an HQL query (Aumentum's `LRSIntegrationQueryDefinition` style); each row is the
    /// selected values as text, in select order (empty string for a null).
    pub async fn query(&self, hql: &str, max_results: u32) -> Result<Vec<Vec<String>>, LrsError> {
        let body = format!(
            "<dom:loadObjects xmlns:dom=\"{DOM}\" xmlns:lrs=\"{LRS}\"><dom:Query><lrs:SimpleQuery><lrs:queryString>{}</lrs:queryString><lrs:maxResults>{max_results}</lrs:maxResults></lrs:SimpleQuery></dom:Query></dom:loadObjects>",
            escape(hql)
        );
        let xml = soap(&self.cfg, "DomainModelService", &format!("{DOM}/loadObjects"), &self.security_header(), &body).await?;
        parse_scalar_rows(&xml)
    }

    fn require_write(&self) -> Result<(), LrsError> {
        if self.cfg.write_enabled { Ok(()) } else { Err(LrsError::Fault("writing to the Registry is turned off on this server (CADASTRE_REGISTRY_WRITE)".into())) }
    }

    /// `IntegrationService.SaveEntity(transactionId, entities)` (Aumentum's PersistDataInRegistry
    /// / PersistDocumentInRegistry). `entities_xml`: one or more `<int:entities>` elements from
    /// [`entity_xml`] / [`incoming_document_xml`]. Succeeds when the Registry returns items.
    pub async fn save_entity(&self, transaction_id: i64, entities_xml: &str) -> Result<(), LrsError> {
        self.require_write()?;
        let body = format!(
            "<int:SaveEntity xmlns:int=\"{INT}\" xmlns:lrs=\"{LRS}\" xmlns:cms=\"{CMS}\" xmlns:xsi=\"{XSI}\"><int:transactionId>{transaction_id}</int:transactionId>{entities_xml}</int:SaveEntity>"
        );
        let xml = soap(&self.cfg, "IntegrationService", &format!("{INT}/SaveEntity"), &self.security_header(), &body).await?;
        if xml.contains("className") || xml.contains(":value>") || xml.contains("<value") {
            Ok(())
        } else {
            Err(LrsError::Fault("the Registry saved nothing (no items returned)".into()))
        }
    }

    /// Upload a file to the Registry's repository (`PUT /lrs/upload/{guid}.{ext}?ticket=…`), as
    /// Aumentum's ConvertImageToString; returns the Registry's answer, which is the page content.
    pub async fn upload(&self, extension: &str, bytes: &[u8]) -> Result<String, LrsError> {
        self.require_write()?;
        let root = self.cfg.base_url.trim_end_matches('/').trim_end_matches("/api").trim_end_matches('/');
        let url = format!("{root}/upload/{}.{extension}?ticket={}&encoding=UTF%2D8", uuid::Uuid::new_v4(), self.ticket);
        let r = http_bytes(&url, "PUT", &[("Content-Type", "application/octet-stream".into()), ("ContentType", "application/octet-stream".into())], bytes).await?;
        if !(200..300).contains(&r.status) {
            return Err(LrsError::Fault(format!("upload HTTP {}", r.status)));
        }
        Ok(r.body.trim().to_string())
    }

    pub async fn end(self) {
        let body = format!("<auth:endSession xmlns:auth=\"{AUTH}\"><auth:ticket>{}</auth:ticket></auth:endSession>", escape(&self.ticket));
        if let Err(e) = soap(&self.cfg, "AuthenticationService", &format!("{AUTH}/endSession"), "", &body).await {
            tracing::warn!("Registry sign-out failed: {e}");
        }
    }
}

/// Whether a Registry user's session ticket is valid (`AuthenticationService.isTicketValid`), as
/// Aumentum's GISViewer checked the Registry user its map window was opened for.
pub async fn ticket_valid(cfg: &LrsConfig, user: &str, ticket: &str) -> Result<bool, LrsError> {
    let offset = clock_offset(cfg).await?;
    let s = Session { cfg: cfg.clone(), ticket: ticket.to_string(), offset };
    let header = s.security_header().replace(&format!("<wsse:Username>{}</wsse:Username>", escape(&cfg.user)), &format!("<wsse:Username>{}</wsse:Username>", escape(user)));
    let body = format!("<auth:isTicketValid xmlns:auth=\"{AUTH}\"><auth:ticket>{}</auth:ticket></auth:isTicketValid>", escape(ticket));
    let xml = soap(cfg, "AuthenticationService", &format!("{AUTH}/isTicketValid"), &header, &body).await?;
    Ok(find_text(&xml, "result").is_some_and(|v| v.eq_ignore_ascii_case("true")))
}

fn find_text(xml: &str, local_name: &str) -> Option<String> {
    use quick_xml::events::Event;
    let mut r = quick_xml::Reader::from_str(xml);
    let mut inside = false;
    loop {
        match r.read_event() {
            Ok(Event::Start(e)) if e.local_name().as_ref() == local_name.as_bytes() => inside = true,
            Ok(Event::Text(t)) if inside => return t.unescape().ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()),
            Ok(Event::End(_)) => inside = false,
            Ok(Event::Eof) | Err(_) => return None,
            _ => {}
        }
    }
}

/// `loadObjectsResponse/Result/ResultScalar/items/value` (a record) `/items/value` (a field:
/// `type` + optional `value`) → rows of field texts.
pub fn parse_scalar_rows(xml: &str) -> Result<Vec<Vec<String>>, LrsError> {
    use quick_xml::events::Event;
    let mut r = quick_xml::Reader::from_str(xml);
    // Path of local element names from the root.
    let mut path: Vec<String> = Vec::new();
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut field: Option<String> = None;
    loop {
        match r.read_event() {
            Ok(Event::Start(e)) => {
                let name = String::from_utf8_lossy(e.local_name().as_ref()).to_string();
                path.push(name);
                let n = path.len();
                // ...ResultScalar / items / value(record) / items / value(field) / value(text)
                if n >= 3 && path[n - 1] == "value" && path[n - 2] == "items" && path[n - 3] == "ResultScalar" {
                    rows.push(Vec::new());
                } else if n >= 5 && path[n - 1] == "value" && path[n - 2] == "items" && path[n - 3] == "value" && path[n - 5] == "ResultScalar" {
                    field = Some(String::new());
                }
            }
            Ok(Event::Text(t)) => {
                let n = path.len();
                if n >= 6 && path[n - 1] == "value" && path[n - 2] == "value" && path[n - 6] == "ResultScalar" {
                    if let Some(f) = field.as_mut() {
                        f.push_str(&t.unescape().map_err(|e| LrsError::Parse(e.to_string()))?);
                    }
                }
            }
            Ok(Event::End(_)) => {
                let n = path.len();
                if n >= 5 && path[n - 1] == "value" && path[n - 2] == "items" && path[n - 3] == "value" && path[n - 5] == "ResultScalar" {
                    if let (Some(f), Some(row)) = (field.take(), rows.last_mut()) {
                        row.push(f.trim().to_string());
                    }
                }
                path.pop();
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(LrsError::Parse(e.to_string())),
            _ => {}
        }
    }
    Ok(rows)
}

/// A Registry `dateTime` as the API returns it (`2 Nov 2026 09:20:25 GMT`).
pub fn parse_datetime(s: &str) -> Option<NaiveDateTime> {
    let s = s.trim().trim_end_matches(" GMT").trim_end_matches(" UTC");
    NaiveDateTime::parse_from_str(s, "%d %b %Y %H:%M:%S").ok()
}

fn entity_value(name: &str, value: Option<&str>) -> String {
    match value {
        Some(v) => format!("<lrs:properties><lrs:name>{}</lrs:name><lrs:value>{}</lrs:value></lrs:properties>", escape(name), escape(v)),
        None => format!("<lrs:properties><lrs:name>{}</lrs:name><lrs:value xsi:nil=\"true\"/></lrs:properties>", escape(name)),
    }
}

fn named_value(name: &str, value: &str) -> String {
    format!("<cms:properties><cms:name>{}</cms:name><cms:isMultiValue xsi:nil=\"true\"/><cms:value>{}</cms:value></cms:properties>", escape(name), escape(value))
}

/// A `com.landsystems.lrs.model.<class>` entity, status `modified`, with name/value pairs (a
/// `None` value is sent as nil), as Aumentum's PersistDataInRegistry.
pub fn entity_xml(class: &str, properties: &[(&str, Option<&str>)]) -> String {
    let props: String = properties.iter().map(|(n, v)| entity_value(n, *v)).collect();
    format!("<int:entities><lrs:className>com.landsystems.lrs.model.{}</lrs:className><lrs:status>modified</lrs:status>{props}</int:entities>", escape(class))
}

/// The survey plan as Aumentum's PersistDocumentInRegistry sent it: an `IncomingDocument`
/// (status added) of type `doc_type` (MCServer dictionary SurveyPlanFromMC = sd_titledeedPlan),
/// Comments "Survey Plan", Submission now, PageCount 1; one image page whose properties are the
/// transaction reference, isattachment False, the page name, index 0 and the uploaded content.
pub fn incoming_document_xml(doc_type: &str, transaction_id: i64, file_name: &str, uploaded: &str, submission: &str) -> String {
    let content_props = [
        named_value(&format!("{{{LRS_CORE}}}referenceid"), &transaction_id.to_string()),
        named_value(&format!("{{{LRS_CORE}}}isattachment"), "False"),
        named_value(&format!("{{{LRS_CORE}}}pagename"), file_name),
        named_value(&format!("{{{LRS_CORE}}}pageindex"), "0"),
        named_value(&format!("{{{CMS}}}content"), uploaded),
    ]
    .concat();
    format!(
        "<int:entities xsi:type=\"lrs:DocumentEntity\"><lrs:className>com.landsystems.lrs.model.IncomingDocument</lrs:className><lrs:status>added</lrs:status>{}{}{}{}\
         <lrs:content><lrs:status>saved</lrs:status><lrs:reference xsi:nil=\"true\"/>{content_props}\
         <lrs:pages><lrs:status>added</lrs:status><lrs:reference xsi:nil=\"true\"/>{content_props}<lrs:type>image</lrs:type></lrs:pages></lrs:content></int:entities>",
        entity_value("DocumentType", Some(doc_type)),
        entity_value("Comments", Some("Survey Plan")),
        entity_value("Submission", Some(submission)),
        entity_value("PageCount", Some("1")),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_aumentums_save_entity_payloads() {
        let e = entity_xml("Parcel", &[("Id", Some("10000000000002")), ("upin", None)]);
        assert_eq!(e, "<int:entities><lrs:className>com.landsystems.lrs.model.Parcel</lrs:className><lrs:status>modified</lrs:status>\
<lrs:properties><lrs:name>Id</lrs:name><lrs:value>10000000000002</lrs:value></lrs:properties>\
<lrs:properties><lrs:name>upin</lrs:name><lrs:value xsi:nil=\"true\"/></lrs:properties></int:entities>");
        let d = incoming_document_xml("sd_titledeedPlan", 42, "PL68955 & co.jpg", "ref-1", "2026-11-02T05:10:00.000+01:00");
        assert!(d.starts_with("<int:entities xsi:type=\"lrs:DocumentEntity\"><lrs:className>com.landsystems.lrs.model.IncomingDocument</lrs:className><lrs:status>added</lrs:status>"));
        assert!(d.contains("<lrs:name>DocumentType</lrs:name><lrs:value>sd_titledeedPlan</lrs:value>"));
        assert!(d.contains("<lrs:name>Comments</lrs:name><lrs:value>Survey Plan</lrs:value>"));
        assert!(d.contains("<cms:name>{http://www.landsystems.com/lrs/model/core/1.0}referenceid</cms:name><cms:isMultiValue xsi:nil=\"true\"/><cms:value>42</cms:value>"));
        assert!(d.contains("PL68955 &amp; co.jpg"), "names escaped");
        assert!(d.contains("<cms:name>{http://www.alfresco.org/ws/model/content/1.0}content</cms:name><cms:isMultiValue xsi:nil=\"true\"/><cms:value>ref-1</cms:value>"));
        assert!(d.ends_with("<lrs:type>image</lrs:type></lrs:pages></lrs:content></int:entities>"));
    }

    #[test]
    fn parses_a_real_registry_response() {
        let xml = include_str!("../tests/fixtures/lrs-loadobjects-linked.xml");
        let rows = parse_scalar_rows(xml).unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0], vec!["PLT00040936", "PL000000020609", "PL65835", "2 Nov 2026 09:20:25 GMT", "PLS000133098", "transaction_lodged", "Active", "", ""]);
        assert_eq!(rows[1][7], "1& 3", "entities unescaped");
        assert_eq!(rows[1][8], "E1");
    }

    #[test]
    fn parses_registry_dates() {
        let d = parse_datetime("2 Nov 2026 09:20:25 GMT").unwrap();
        assert_eq!(d.format("%Y-%m-%d %H:%M:%S").to_string(), "2026-11-02 09:20:25");
        assert!(parse_datetime("").is_none());
    }

    #[test]
    fn escapes_xml() {
        assert_eq!(escape("a<b & 'c'"), "a&lt;b &amp; &apos;c&apos;");
    }
}
