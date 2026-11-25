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
const ACS: &str = "http://www.alfresco.org/ws/service/accesscontrol/1.0";
const ADM: &str = "http://www.alfresco.org/ws/service/administration/1.0";
/// Alfresco's content model: the properties of a person.
const CM: &str = "http://www.alfresco.org/model/content/1.0";
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

/// A signed-in session. Registry tickets never expire and each holds one of its licensed places
/// until `endSession`, so a session always signs out: [`Session::end`] does it and waits briefly;
/// dropping the session (an early return, a request whose browser went away) does it in the
/// background. A shared session ([`Session::start`]) stays signed in for the next caller.
pub struct Session {
    cfg: LrsConfig,
    ticket: String,
    /// Registry clock minus this machine's clock.
    offset: Duration,
    /// Borrowed from [`SHARED`]: not signed out when this user of it is done.
    shared: bool,
}

/// The server's own account's session, reused so that each read does not take a licensed place.
struct Shared {
    cfg: LrsConfig,
    ticket: String,
    offset: Duration,
    /// When the Registry last confirmed the ticket; `None` after a refusal (check before reuse).
    checked: Option<std::time::Instant>,
}

static SHARED: tokio::sync::Mutex<Option<Shared>> = tokio::sync::Mutex::const_new(None);
/// How long a confirmed ticket is reused before the Registry is asked again (it can be deleted in
/// the Registry's User Sessions, and a restart of LRSAS ends it).
const RECHECK: std::time::Duration = std::time::Duration::from_secs(30);
/// Waits before each try at signing out.
const SIGN_OUT_WAITS: [u64; 4] = [0, 1, 5, 15];

/// Sign a ticket out in its own task, which no dropped request cancels; retried.
fn sign_out(cfg: LrsConfig, ticket: String) -> Option<tokio::task::JoinHandle<()>> {
    let rt = tokio::runtime::Handle::try_current().ok()?;
    Some(rt.spawn(async move {
        let body = format!("<auth:endSession xmlns:auth=\"{AUTH}\"><auth:ticket>{}</auth:ticket></auth:endSession>", escape(&ticket));
        for wait in SIGN_OUT_WAITS {
            tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
            match soap(&cfg, "AuthenticationService", &format!("{AUTH}/endSession"), "", &body).await {
                Ok(_) => return,
                Err(e) => tracing::warn!("Registry sign-out failed: {e}"),
            }
        }
        tracing::error!("Registry sign-out given up: a session of {} may still hold a licensed place", cfg.user);
    }))
}

/// Sign the shared session out (the server is stopping).
pub async fn shutdown() {
    let Some(s) = SHARED.lock().await.take() else { return };
    if let Some(task) = sign_out(s.cfg, s.ticket) {
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), task).await;
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if !self.shared && !self.ticket.is_empty() {
            sign_out(self.cfg.clone(), std::mem::take(&mut self.ticket));
        }
    }
}

pub fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

pub(crate) struct HttpResponse {
    pub(crate) status: u16,
    date: Option<String>,
    /// `name=value` of the first Set-Cookie (ASP.NET session cookies).
    pub(crate) cookie: Option<String>,
    pub(crate) body: String,
}

/// Opens a TCP connection to an Aumentum or Registry host. When `CADASTRE_OUTBOUND_ADDR` is set
/// (an IPv4 address of this server, e.g. `10.10.10.11`), the connection leaves from that address.
/// Aumentum's job service reverse-resolves its caller ("No such host is known" otherwise), so the
/// server must call from an address the LAN DNS has a name for. Loopback targets are not bound.
pub(crate) async fn connect(host: &str, port: u16) -> std::io::Result<TcpStream> {
    let bind = std::env::var("CADASTRE_OUTBOUND_ADDR").ok().and_then(|s| s.trim().parse::<std::net::Ipv4Addr>().ok());
    let Some(bind) = bind else { return TcpStream::connect((host, port)).await };
    let mut last = None;
    for addr in tokio::net::lookup_host((host, port)).await? {
        let std::net::SocketAddr::V4(v4) = addr else { continue };
        let sock = tokio::net::TcpSocket::new_v4()?;
        if !v4.ip().is_loopback() {
            sock.bind(std::net::SocketAddr::from((bind, 0)))?;
        }
        match sock.connect(addr).await {
            Ok(s) => return Ok(s),
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, format!("no IPv4 address for {host}"))))
}

/// Minimal HTTP/1.0 request over plain TCP (the Registry serves `http://` only). HTTP/1.0 and
/// `Connection: close` keep the response unchunked and read to EOF.
pub(crate) async fn http(url: &str, method: &str, headers: &[(&str, String)], body: &str) -> Result<HttpResponse, LrsError> {
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
        let mut stream = connect(host, port).await.map_err(|e| LrsError::Http(format!("connect {authority}: {e}")))?;
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
    let cookie = head.lines().find_map(|l| l.split_once(':').filter(|(k, _)| k.eq_ignore_ascii_case("set-cookie")).map(|(_, v)| v.trim().split(';').next().unwrap_or("").to_string()));
    Ok(HttpResponse { status, date, cookie, body: body.to_string() })
}

async fn soap(cfg: &LrsConfig, service: &str, action: &str, header: &str, body: &str) -> Result<String, LrsError> {
    let envelope = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?><soap:Envelope xmlns:soap=\"{SOAP}\"><soap:Header>{header}</soap:Header><soap:Body>{body}</soap:Body></soap:Envelope>"
    );
    let url = format!("{}{service}", base(cfg));
    let r = http(&url, "POST", &[("Content-Type", "text/xml; charset=utf-8".into()), ("SOAPAction", format!("\"{action}\""))], &envelope).await?;
    if r.status != 200 {
        // The Registry often leaves <faultstring> empty and gives the reason in <detail><message>.
        let fault = between(&r.body, "<faultstring>", "</faultstring>").filter(|f| !f.trim().is_empty())
            .or_else(|| find_text(&r.body, "message"))
            .unwrap_or_else(|| r.body.chars().take(300).collect());
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
    /// The server's own session: shared, confirmed with the Registry when not used lately, signed
    /// in when there is none. For a password check use [`Session::sign_in`].
    pub async fn start(cfg: &LrsConfig) -> Result<Self, LrsError> {
        let mut slot = SHARED.lock().await;
        if let Some(s) = slot.as_mut() {
            let same = s.cfg.base_url == cfg.base_url && s.cfg.user == cfg.user && s.cfg.password == cfg.password;
            let mut usable = same && s.checked.is_some_and(|t| t.elapsed() < RECHECK);
            if same && !usable {
                match ticket_check(cfg, &cfg.user, &s.ticket).await {
                    Ok((true, offset)) => {
                        s.offset = offset;
                        s.checked = Some(std::time::Instant::now());
                        usable = true;
                    }
                    Ok((false, _)) | Err(LrsError::Fault(_)) => {}
                    Err(e) => return Err(e),
                }
            }
            if usable {
                return Ok(Self { cfg: cfg.clone(), ticket: s.ticket.clone(), offset: s.offset, shared: true });
            }
        }
        // Signed out even if the Registry called it invalid: it must not hold a place.
        if let Some(old) = slot.take() {
            sign_out(old.cfg, old.ticket);
        }
        let mut own = Self::sign_in(cfg).await?;
        *slot = Some(Shared { cfg: cfg.clone(), ticket: own.ticket.clone(), offset: own.offset, checked: Some(std::time::Instant::now()) });
        own.shared = true;
        Ok(own)
    }

    /// A session of its own: the Registry checks the password now. Runs in its own task, so a caller
    /// that goes away before the ticket arrives still gets it signed out (dropped when it arrives).
    pub async fn sign_in(cfg: &LrsConfig) -> Result<Self, LrsError> {
        let cfg = cfg.clone();
        tokio::spawn(async move {
            let offset = clock_offset(&cfg).await?;
            let body = format!(
                "<auth:startSession xmlns:auth=\"{AUTH}\"><auth:username>{}</auth:username><auth:password>{}</auth:password></auth:startSession>",
                escape(&cfg.user), escape(&cfg.password)
            );
            let xml = soap(&cfg, "AuthenticationService", &format!("{AUTH}/startSession"), "", &body).await?;
            let ticket = find_text(&xml, "ticket").ok_or(LrsError::NoTicket)?;
            Ok(Self { cfg, ticket, offset, shared: false })
        }).await.map_err(|e| LrsError::Http(format!("sign-in task: {e}")))?
    }

    /// Call the Registry with this session. After a refusal a shared ticket is confirmed before its
    /// next use (it is not signed out here: others may be using it).
    async fn call(&self, service: &str, action: &str, body: &str) -> Result<String, LrsError> {
        let r = soap(&self.cfg, service, action, &self.security_header(), body).await;
        if self.shared && r.is_err() {
            if let Some(s) = SHARED.lock().await.as_mut().filter(|s| s.ticket == self.ticket) {
                s.checked = None;
            }
        }
        r
    }

    /// The Registry groups a user belongs to, directly or through other groups (Alfresco's
    /// AccessControlService), as `GROUP_…` names. Read-only.
    pub async fn user_groups(&self, username: &str) -> Result<Vec<String>, LrsError> {
        let body = format!(
            "<acs:getParentAuthorities xmlns:acs=\"{ACS}\"><acs:authority>{}</acs:authority><acs:filter><acs:authorityType>GROUP</acs:authorityType><acs:immediate>false</acs:immediate></acs:filter></acs:getParentAuthorities>",
            escape(username)
        );
        let xml = self.call("AccessControlService", &format!("{ACS}/getParentAuthorities"), &body).await?;
        Ok(xml.split("GROUP_").skip(1).filter_map(|t| t.split('<').next()).map(|n| format!("GROUP_{}", n.replace("&amp;", "&"))).collect())
    }

    /// Change a Registry password (`AdministrationService`). The session is the user's own, so the
    /// Registry checks the old password itself.
    pub async fn change_password(&self, username: &str, old: &str, new: &str) -> Result<(), LrsError> {
        let body = format!(
            "<adm:changePassword xmlns:adm=\"{ADM}\"><adm:userName>{}</adm:userName><adm:oldPassword>{}</adm:oldPassword><adm:newPassword>{}</adm:newPassword></adm:changePassword>",
            escape(username), escape(old), escape(new)
        );
        self.call("AdministrationService", &format!("{ADM}/changePassword"), &body).await?;
        Ok(())
    }

    /// Create a Registry user with no groups (`AdministrationService`). Needs an administrator's session.
    pub async fn create_user(&self, username: &str, password: &str, first_name: &str, last_name: &str) -> Result<(), LrsError> {
        let prop = |name: &str, value: &str| format!(
            "<adm:properties xmlns:cms=\"{CMS}\"><cms:name>{{{CM}}}{name}</cms:name><cms:isMultiValue>false</cms:isMultiValue><cms:value>{}</cms:value></adm:properties>", escape(value));
        let body = format!(
            "<adm:createUsers xmlns:adm=\"{ADM}\"><adm:newUsers><adm:userName>{}</adm:userName><adm:password>{}</adm:password>{}{}{}</adm:newUsers></adm:createUsers>",
            escape(username), escape(password), prop("userName", username), prop("firstName", first_name), prop("lastName", last_name)
        );
        self.call("AdministrationService", &format!("{ADM}/createUsers"), &body).await?;
        Ok(())
    }

    /// The Registry clock minus this machine's, in seconds.
    pub fn clock_offset_secs(&self) -> i64 {
        self.offset.num_seconds()
    }

    fn security_header(&self) -> String {
        security_header(&self.cfg.user, &self.ticket, self.offset)
    }

    /// Run an HQL query (Aumentum's `LRSIntegrationQueryDefinition` style); each row is the
    /// selected values as text, in select order (empty string for a null).
    pub async fn query(&self, hql: &str, max_results: u32) -> Result<Vec<Vec<String>>, LrsError> {
        let body = format!(
            "<dom:loadObjects xmlns:dom=\"{DOM}\" xmlns:lrs=\"{LRS}\"><dom:Query><lrs:SimpleQuery><lrs:queryString>{}</lrs:queryString><lrs:maxResults>{max_results}</lrs:maxResults></lrs:SimpleQuery></dom:Query></dom:loadObjects>",
            escape(hql)
        );
        let xml = self.call("DomainModelService", &format!("{DOM}/loadObjects"), &body).await?;
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
        let xml = self.call("IntegrationService", &format!("{INT}/SaveEntity"), &body).await?;
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

    /// Done with the session: sign out (a shared one stays for the next caller).
    pub async fn end(mut self) {
        if self.shared {
            return;
        }
        if let Some(task) = sign_out(self.cfg.clone(), std::mem::take(&mut self.ticket)) {
            let _ = tokio::time::timeout(std::time::Duration::from_secs(5), task).await;
        }
    }
}

fn security_header(user: &str, ticket: &str, offset: Duration) -> String {
    let now = Utc::now() + offset;
    let created = now.format("%Y-%m-%dT%H:%M:%SZ");
    let expires = (now + Duration::seconds(600)).format("%Y-%m-%dT%H:%M:%SZ");
    let id = uuid::Uuid::new_v4();
    format!(
        "<wsse:Security xmlns:wsse=\"{WSSE}\" xmlns:wsu=\"{WSU}\"><wsu:Timestamp wsu:Id=\"Timestamp-{id}\"><wsu:Created>{created}</wsu:Created><wsu:Expires>{expires}</wsu:Expires></wsu:Timestamp>\
         <wsse:UsernameToken wsu:Id=\"SecurityToken-{id}\"><wsse:Username>{}</wsse:Username><wsse:Password Type=\"{PWTEXT}\">{}</wsse:Password><wsu:Created>{created}</wsu:Created></wsse:UsernameToken></wsse:Security>",
        escape(user), escape(ticket)
    )
}

/// Whether a Registry user's session ticket is valid (`AuthenticationService.isTicketValid`), as
/// Aumentum's GISViewer checked the Registry user its map window was opened for.
pub async fn ticket_valid(cfg: &LrsConfig, user: &str, ticket: &str) -> Result<bool, LrsError> {
    ticket_check(cfg, user, ticket).await.map(|(valid, _)| valid)
}

/// [`ticket_valid`], with the Registry's clock offset read for it.
async fn ticket_check(cfg: &LrsConfig, user: &str, ticket: &str) -> Result<(bool, Duration), LrsError> {
    let offset = clock_offset(cfg).await?;
    let body = format!("<auth:isTicketValid xmlns:auth=\"{AUTH}\"><auth:ticket>{}</auth:ticket></auth:isTicketValid>", escape(ticket));
    let xml = soap(cfg, "AuthenticationService", &format!("{AUTH}/isTicketValid"), &security_header(user, ticket, offset), &body).await?;
    Ok((find_text(&xml, "result").is_some_and(|v| v.eq_ignore_ascii_case("true")), offset))
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

/// The plan document as Aumentum's PersistDocumentInRegistry sent it: an `IncomingDocument`
/// (status added) of type `doc_type` (MCServer dictionary SurveyPlanFromMC = sd_titledeedPlan),
/// `comments` (Aumentum: "Survey Plan"), Submission now, PageCount 1; one page of `page_type`
/// whose properties are the transaction reference, isattachment, the page name, index 0 and the
/// uploaded content. The Registry's page types are `image` (Aumentum's JPG plan) and
/// `attachment` (a file kept as it is: the PDF plan); isattachment follows the page type.
pub fn incoming_document_xml(doc_type: &str, transaction_id: i64, file_name: &str, uploaded: &str, submission: &str, comments: &str, page_type: &str) -> String {
    incoming_document_pages_xml(doc_type, transaction_id, &[DocPage { name: file_name, uploaded, kind: page_type }], submission, comments)
}

/// One page of a document: its file name, the upload answer and its type (`image` or
/// `attachment`).
pub struct DocPage<'a> {
    pub name: &'a str,
    pub uploaded: &'a str,
    pub kind: &'a str,
}

/// An `IncomingDocument` with several pages in order (index 0, 1, …), as the Registry's scanning
/// files them: each page has its own name, content, type and isattachment; PageCount is the
/// number of pages. The document's content carries the first page's properties, as with one page.
/// The title deed plan goes as the sheet's JPEG (`image`, which the Registry shows) followed by
/// the PDF (`attachment`).
pub fn incoming_document_pages_xml(doc_type: &str, transaction_id: i64, pages: &[DocPage], submission: &str, comments: &str) -> String {
    let props = |i: usize, p: &DocPage| {
        let attachment = p.kind != "image";
        [
            named_value(&format!("{{{LRS_CORE}}}referenceid"), &transaction_id.to_string()),
            named_value(&format!("{{{LRS_CORE}}}isattachment"), if attachment { "True" } else { "False" }),
            named_value(&format!("{{{LRS_CORE}}}pagename"), p.name),
            named_value(&format!("{{{LRS_CORE}}}pageindex"), &i.to_string()),
            // The content model's cm:content (CM), not the web-service namespace (CMS): under CMS
            // Alfresco keeps the upload answer as a plain text property, the page has no file, and
            // the Registry shows "page not found" (every plan sent 2–16 Nov 2026).
            named_value(&format!("{{{CM}}}content"), p.uploaded),
        ]
        .concat()
    };
    let content_props = pages.first().map(|p| props(0, p)).unwrap_or_default();
    let page_xml: String = pages.iter().enumerate().map(|(i, p)| format!(
        "<lrs:pages><lrs:status>added</lrs:status><lrs:reference xsi:nil=\"true\"/>{}<lrs:type>{}</lrs:type></lrs:pages>",
        props(i, p), if p.kind == "image" { "image" } else { "attachment" })).collect();
    format!(
        "<int:entities xsi:type=\"lrs:DocumentEntity\"><lrs:className>com.landsystems.lrs.model.IncomingDocument</lrs:className><lrs:status>added</lrs:status>{}{}{}{}\
         <lrs:content><lrs:status>saved</lrs:status><lrs:reference xsi:nil=\"true\"/>{content_props}{page_xml}</lrs:content></int:entities>",
        entity_value("DocumentType", Some(doc_type)),
        entity_value("Comments", Some(comments)),
        entity_value("Submission", Some(submission)),
        entity_value("PageCount", Some(&pages.len().to_string())),
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
        let d = incoming_document_xml("sd_titledeedPlan", 42, "PL68955 & co.jpg", "ref-1", "2026-11-02T05:10:00.000+01:00", "Survey Plan", "image");
        assert!(d.starts_with("<int:entities xsi:type=\"lrs:DocumentEntity\"><lrs:className>com.landsystems.lrs.model.IncomingDocument</lrs:className><lrs:status>added</lrs:status>"));
        assert!(d.contains("<lrs:name>DocumentType</lrs:name><lrs:value>sd_titledeedPlan</lrs:value>"));
        assert!(d.contains("<lrs:name>Comments</lrs:name><lrs:value>Survey Plan</lrs:value>"));
        assert!(d.contains("<cms:name>{http://www.landsystems.com/lrs/model/core/1.0}referenceid</cms:name><cms:isMultiValue xsi:nil=\"true\"/><cms:value>42</cms:value>"));
        assert!(d.contains("PL68955 &amp; co.jpg"), "names escaped");
        assert!(d.contains("<cms:name>{http://www.alfresco.org/model/content/1.0}content</cms:name><cms:isMultiValue xsi:nil=\"true\"/><cms:value>ref-1</cms:value>"));
        assert!(!d.contains("ws/model/content/1.0}content"), "the page's file is cm:content, or the Registry finds no page");
        assert!(d.ends_with("<lrs:type>image</lrs:type></lrs:pages></lrs:content></int:entities>"));
        assert!(d.contains("isattachment</cms:name><cms:isMultiValue xsi:nil=\"true\"/><cms:value>False</cms:value>"));
        // The PDF plan: an attachment page, flagged as an attachment, with the officer's comment.
        let p = incoming_document_xml("sd_titledeedPlan", 42, "Title Deed Plan PL67363.pdf", "ref-2", "2026-11-03T10:00:00.000+01:00", "Survey Plan (corrected)", "attachment");
        assert!(p.ends_with("<lrs:type>attachment</lrs:type></lrs:pages></lrs:content></int:entities>"));
        assert!(p.contains("isattachment</cms:name><cms:isMultiValue xsi:nil=\"true\"/><cms:value>True</cms:value>"));
        assert!(p.contains("<lrs:name>Comments</lrs:name><lrs:value>Survey Plan (corrected)</lrs:value>"));
        // Anything but "image" is sent as an attachment (the Registry knows only these two).
        assert!(incoming_document_xml("t", 1, "a.pdf", "r", "s", "c", "pdf").contains("<lrs:type>attachment</lrs:type>"));
        // The plan as the sheet's JPEG (shown by the Registry) and the PDF: one document, two pages.
        let two = incoming_document_pages_xml("sd_titledeedPlan", 42, &[DocPage { name: "PL1.jpg", uploaded: "ref-j", kind: "image" }, DocPage { name: "PL1.pdf", uploaded: "ref-p", kind: "attachment" }], "s", "Survey Plan");
        assert!(two.contains("<lrs:name>PageCount</lrs:name><lrs:value>2</lrs:value>"));
        let pages: Vec<&str> = two.split("<lrs:pages>").skip(1).collect();
        assert_eq!(pages.len(), 2);
        assert!(pages[0].contains("PL1.jpg") && pages[0].contains("ref-j") && pages[0].contains("pageindex</cms:name><cms:isMultiValue xsi:nil=\"true\"/><cms:value>0<")
            && pages[0].contains("isattachment</cms:name><cms:isMultiValue xsi:nil=\"true\"/><cms:value>False<") && pages[0].contains("<lrs:type>image</lrs:type>"));
        assert!(pages[1].contains("PL1.pdf") && pages[1].contains("ref-p") && pages[1].contains("pageindex</cms:name><cms:isMultiValue xsi:nil=\"true\"/><cms:value>1<")
            && pages[1].contains("isattachment</cms:name><cms:isMultiValue xsi:nil=\"true\"/><cms:value>True<") && pages[1].contains("<lrs:type>attachment</lrs:type>"));
        // One page is exactly the document sent before.
        assert_eq!(incoming_document_xml("t", 1, "a.pdf", "r", "s", "c", "attachment"),
            incoming_document_pages_xml("t", 1, &[DocPage { name: "a.pdf", uploaded: "r", kind: "attachment" }], "s", "c"));
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
