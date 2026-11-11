//! Parallel run with Aumentum Cadastre: before this Cadastre gives a new UPIN, it asks Aumentum's
//! own job service (`LRSWebServices.asmx`, the one the Registry calls) whether Aumentum has
//! already issued it. Aumentum keeps committing while both systems run, and its numbers continue
//! the same series, so a number Aumentum issued after the data was copied must be skipped.
//!
//! On when `CADASTRE_AUMENTUM_JOBSERVICE_URL` is set (e.g.
//! `http://10.10.10.6/ApplicationServerAdmin/WebServices/LRSWebServices.asmx`); it signs in with
//! `CADASTRE_MC_SERVICE_USER` / `_PASSWORD` (the same `administrator` account). If Aumentum
//! cannot be asked, the commit is refused rather than risk a duplicate UPIN.
//!
//! This cannot stop the reverse: Aumentum does not know this Cadastre's numbers, so it may later
//! issue a number given here. While both systems commit, do not commit in both at once, or give
//! this Cadastre its own range (docs/registry-integration.md).

use crate::lrs::{escape, http};

/// A signed-in Aumentum session. Commits and the duplicate-UPIN watch share one (`SESSION`):
/// Aumentum opens an ArcSDE connection for a session's lookups and does not give it back when the
/// session signs out, so a session per check fills ArcSDE's connections; one kept session holds one.
pub struct UpinCheck {
    url: String,
    cookie: String,
}

/// The shared session (url, cookie), signed in on first use and again when Aumentum drops it.
static SESSION: tokio::sync::Mutex<Option<(String, String)>> = tokio::sync::Mutex::const_new(None);

/// Whether a fresh session may get an answer the kept one did not: Aumentum's fault for a session
/// it no longer knows (expired, or the service restarted), and "Object reference not set", which
/// it also gives a session IIS still accepts after the AUMENTUM CADASTRE Server service restarted
/// behind it. The same fault with ArcSDE out of connections fails again on the fresh session, and
/// costs no connection, since none can be opened.
fn fresh_session_may_help(f: &Failure) -> bool {
    let d = f.detail.as_deref().unwrap_or("").to_lowercase();
    d.contains("not authenticated") || d.contains("credentials are not valid") || d.contains("session")
        || (f.stage == "lookup" && d.contains("object reference not set"))
}

const NS: &str = "http://tempuri.org/";

fn envelope(op: &str, inner: &str) -> String {
    format!("<?xml version=\"1.0\" encoding=\"utf-8\"?><soap:Envelope xmlns:soap=\"http://schemas.xmlsoap.org/soap/envelope/\"><soap:Body><{op} xmlns=\"{NS}\">{inner}</{op}></soap:Body></soap:Envelope>")
}

/// Whether a commit could ask Aumentum right now (what the editor's warning shows).
#[derive(Clone, Debug, serde::Serialize, utoipa::ToSchema)]
pub struct Health {
    /// The check is on (`CADASTRE_AUMENTUM_JOBSERVICE_URL`); off means nothing to warn about.
    pub configured: bool,
    pub ok: bool,
    /// Why not, if not.
    pub reason: Option<String>,
    /// Where it failed: `settings` (this server's), `unreachable`, `timeout`, `sign-in` or `lookup`.
    pub stage: Option<String>,
    /// Aumentum's own error text (its SOAP fault), when it gave one.
    pub detail: Option<String>,
    /// What to do about it, for the known causes.
    pub hint: Option<String>,
    /// The job service asked (host[:port]).
    pub server: Option<String>,
    /// The address this server calls from (`CADASTRE_OUTBOUND_ADDR`), if one is set.
    pub outbound_addr: Option<String>,
    /// When the answer was found (reused for up to `HEALTH_OK_SECS`, `HEALTH_FAIL_SECS` while failing).
    pub checked_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Why Aumentum could not be asked: the step, Aumentum's own words, and the message a commit shows.
#[derive(Clone)]
struct Failure {
    stage: &'static str,
    detail: Option<String>,
    message: String,
}

impl Failure {
    fn new(stage: &'static str, detail: Option<String>, message: String) -> Self {
        Self { stage, detail, message }
    }
}

/// The innermost message of a SOAP fault ("Server was unable to process request. ---> No such
/// host is known" gives "No such host is known").
fn fault(body: &str) -> Option<String> {
    let start = body.find("<faultstring>")? + "<faultstring>".len();
    let end = start + body[start..].find("</faultstring>")?;
    let text = body[start..end].replace("&gt;", ">").replace("&lt;", "<").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&");
    let inner = text.rsplit("--->").next().unwrap_or(&text).trim().trim_end_matches('.').to_string();
    (!inner.is_empty()).then_some(inner)
}

/// What to do, for the causes seen on this network.
fn hint(f: &Failure) -> Option<String> {
    let d = f.detail.as_deref().unwrap_or("").to_lowercase();
    if d.contains("no such host is known") {
        let from = std::env::var("CADASTRE_OUTBOUND_ADDR").ok().filter(|s| !s.trim().is_empty());
        return Some(match from {
            None => "Aumentum looks up the caller's address in DNS and found no name for this server's. Set CADASTRE_OUTBOUND_ADDR to an address of this server that has a DNS name (10.10.10.11 is plagis30.plagis.com) and restart it.".into(),
            Some(a) => format!("Aumentum looks up the caller's address in DNS and found no name for {a}. Give {a} a DNS name, or set CADASTRE_OUTBOUND_ADDR to an address of this server that has one."),
        });
    }
    if f.stage == "lookup" && d.contains("object reference not set") {
        return Some("Aumentum signed in but could not open its geodatabase. On the Aumentum server, ArcSDE's log (sde_sde.log) usually says \"Maximum number of connections to instance exceeded\": restarting the ArcSDE service, then the AUMENTUM CADASTRE Server service, frees them.".into());
    }
    if f.stage == "lookup" && d.contains("no answer to the upin lookup") {
        return Some("This server reached Aumentum and signed in, but the lookup never answered: Aumentum is waiting on its geodatabase. On the Aumentum server, count the ArcSDE connections (tasklist /fi \"imagename eq gsrvr.exe\" | find /c \"gsrvr\"); near 64, restart the ArcSDE service, then the AUMENTUM CADASTRE Server service.".into());
    }
    match f.stage {
        "settings" => Some("Set CADASTRE_MC_SERVICE_USER / CADASTRE_MC_SERVICE_PASSWORD in this server's settings and restart it.".into()),
        "unreachable" | "timeout" => Some("Check that this server can reach Aumentum's job service (network, firewall, IIS on the Aumentum server).".into()),
        _ => None,
    }
}

/// host[:port] of a URL, for display.
fn host_of(url: &str) -> Option<String> {
    let rest = url.split("://").nth(1).unwrap_or(url);
    rest.split('/').next().filter(|h| !h.is_empty()).map(str::to_string)
}

/// Asked at most once every ten minutes while Aumentum answers, however many editors are open,
/// and every two minutes while it does not (so the warning clears soon after it is back).
const HEALTH_OK_SECS: u64 = 600;
const HEALTH_FAIL_SECS: u64 = 120;
/// Long enough for Aumentum's slowest normal answer, short enough to warn soon.
const HEALTH_TIMEOUT_SECS: u64 = 15;
static HEALTH: tokio::sync::Mutex<Option<(std::time::Instant, Health)>> = tokio::sync::Mutex::const_new(None);

/// A real lookup's outcome is reported for this long (the duplicate-UPIN watch asks every 15 min).
const LOOKUP_FRESH_SECS: u64 = 1200;
/// The last UPIN lookup a commit or the duplicate-UPIN watch made: `Err` if Aumentum failed it.
static LAST_LOOKUP: std::sync::Mutex<Option<(std::time::Instant, Result<(), Failure>)>> = std::sync::Mutex::new(None);

fn note_lookup(outcome: Result<(), Failure>) {
    let failed = outcome.is_err();
    if let Ok(mut last) = LAST_LOOKUP.lock() {
        *last = Some((std::time::Instant::now(), outcome));
    }
    // The warning follows at once: a cached answer that now says the opposite is dropped.
    if let Ok(mut cached) = HEALTH.try_lock() {
        if cached.as_ref().is_some_and(|(_, h)| h.ok == failed) {
            *cached = None;
        }
    }
}

/// The last real lookup's failure, if it failed recently (a later success clears it).
fn recent_lookup_failure() -> Option<Failure> {
    let last = LAST_LOOKUP.lock().ok()?;
    match last.as_ref()? {
        (at, Err(f)) if at.elapsed().as_secs() < LOOKUP_FRESH_SECS => Some(f.clone()),
        _ => None,
    }
}

/// Sign in and out only. The probe used to look up a UPIN too, but Aumentum opens an ArcSDE
/// connection for every lookup session and does not give it back: a probe a minute filled
/// ArcSDE's 64 connections in under two hours ("Maximum number of connections to instance
/// exceeded", 2026-11-10), after which every lookup failed. Sign-in does not touch ArcSDE. A
/// lookup Aumentum signs in for and then fails is still reported, from the last real lookup.
async fn probe() -> Result<(), Failure> {
    let Some(check) = UpinCheck::sign_in().await? else { return Ok(()) };
    check.sign_out().await;
    match recent_lookup_failure() {
        Some(f) => Err(f),
        None => Ok(()),
    }
}

/// Whether Aumentum answers, from a check at most ten minutes old (two while it fails).
pub async fn health() -> Health {
    let server = std::env::var("CADASTRE_AUMENTUM_JOBSERVICE_URL").ok().and_then(|u| host_of(u.trim()));
    let outbound_addr = std::env::var("CADASTRE_OUTBOUND_ADDR").ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    if !UpinCheck::configured() {
        return Health { configured: false, ok: true, reason: None, stage: None, detail: None, hint: None, server, outbound_addr, checked_at: None };
    }
    let mut cached = HEALTH.lock().await;
    if let Some((at, h)) = cached.as_ref() {
        let keep = if h.ok { HEALTH_OK_SECS } else { HEALTH_FAIL_SECS };
        if at.elapsed().as_secs() < keep {
            return h.clone();
        }
    }
    let answer = tokio::time::timeout(std::time::Duration::from_secs(HEALTH_TIMEOUT_SECS), probe()).await
        .unwrap_or_else(|_| Err(Failure::new("timeout", None, format!("Aumentum did not answer within {HEALTH_TIMEOUT_SECS} s"))));
    let failure = answer.err();
    let h = Health {
        configured: true,
        ok: failure.is_none(),
        hint: failure.as_ref().and_then(hint),
        stage: failure.as_ref().map(|f| f.stage.to_string()),
        detail: failure.as_ref().and_then(|f| f.detail.clone()),
        reason: failure.map(|f| f.message),
        server,
        outbound_addr,
        checked_at: Some(chrono::Utc::now()),
    };
    *cached = Some((std::time::Instant::now(), h.clone()));
    h
}

impl UpinCheck {
    /// Whether the parallel-run check is configured on this server.
    pub fn configured() -> bool {
        std::env::var("CADASTRE_AUMENTUM_JOBSERVICE_URL").ok().is_some_and(|s| !s.trim().is_empty())
    }

    /// The shared session to Aumentum's job service (signed in if there is none yet), or
    /// `Ok(None)` when the check is not configured.
    pub async fn connect() -> Result<Option<Self>, String> {
        Self::shared().await.map_err(|f| f.message)
    }

    async fn shared() -> Result<Option<Self>, Failure> {
        let mut session = SESSION.lock().await;
        let url = std::env::var("CADASTRE_AUMENTUM_JOBSERVICE_URL").ok().map(|s| s.trim().to_string()).unwrap_or_default();
        if let Some((u, cookie)) = session.as_ref().filter(|(u, _)| *u == url) {
            return Ok(Some(Self { url: u.clone(), cookie: cookie.clone() }));
        }
        let fresh = Self::sign_in().await?;
        *session = fresh.as_ref().map(|c| (c.url.clone(), c.cookie.clone()));
        Ok(fresh)
    }

    /// Whether Aumentum has an attribute record for this UPIN (in any status). When the kept
    /// session fails in a way a fresh one may not (`fresh_session_may_help`), it is dropped, a new
    /// one signed in, and the question asked once more.
    pub async fn issued(&self, upin: &str) -> Result<bool, String> {
        match self.lookup(upin).await {
            Err(f) if fresh_session_may_help(&f) => {
                {
                    let mut session = SESSION.lock().await;
                    if session.as_ref().is_some_and(|(_, c)| *c == self.cookie) {
                        *session = None;
                    }
                }
                let again = Self::shared().await.map_err(|f| f.message)?.ok_or("the Aumentum UPIN check is not configured")?;
                again.lookup(upin).await.map_err(|f| f.message)
            }
            r => r.map_err(|f| f.message),
        }
    }

    async fn sign_in() -> Result<Option<Self>, Failure> {
        let Some(url) = std::env::var("CADASTRE_AUMENTUM_JOBSERVICE_URL").ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) else {
            return Ok(None);
        };
        let get = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        let (Some(user), Some(pw)) = (get("CADASTRE_MC_SERVICE_USER"), get("CADASTRE_MC_SERVICE_PASSWORD")) else {
            return Err(Failure::new("settings", None, "the Aumentum UPIN check needs CADASTRE_MC_SERVICE_USER / _PASSWORD".into()));
        };
        let body = envelope("Authenticate", &format!("<user>{}</user><password>{}</password>", escape(&user), escape(&pw)));
        let r = http(&url, "POST", &[("Content-Type", "text/xml; charset=utf-8".into()), ("SOAPAction", format!("\"{NS}Authenticate\""))], &body)
            .await
            .map_err(|e| Failure::new("unreachable", Some(e.to_string()),
                format!("Aumentum could not be asked whether the UPIN is free ({e}); nothing was committed. Please contact the ICT Admin.")))?;
        match (r.body.contains("<AuthenticateResult>true</AuthenticateResult>"), r.cookie) {
            (true, Some(cookie)) => Ok(Some(Self { url, cookie })),
            _ => {
                let detail = fault(&r.body);
                let said = detail.as_deref().map(|d| format!(" (HTTP {}: {d})", r.status)).unwrap_or_default();
                Err(Failure::new("sign-in", detail,
                    format!("Aumentum's job service refused the sign-in for the UPIN check{said}; nothing was committed. Please contact the ICT Admin.")))
            }
        }
    }

    async fn lookup(&self, upin: &str) -> Result<bool, Failure> {
        let asked = self.ask(upin).await;
        note_lookup(asked.as_ref().map(|_| ()).map_err(Failure::clone));
        asked
    }

    async fn ask(&self, upin: &str) -> Result<bool, Failure> {
        let body = envelope("GetParcelAttributesByUPIN", &format!("<regionName>Plateau_State</regionName><upin>{}</upin>", escape(upin)));
        let r = http(&self.url, "POST", &[("Content-Type", "text/xml; charset=utf-8".into()), ("SOAPAction", format!("\"{NS}GetParcelAttributesByUPIN\"")), ("Cookie", self.cookie.clone())], &body)
            .await
            .map_err(|e| match e {
                // Signed in, then no answer: Aumentum was reached, its lookup hung (not the network).
                crate::lrs::LrsError::Http(ref m) if m == "timed out" => Failure::new("lookup", Some("no answer to the UPIN lookup".into()),
                    format!("Aumentum accepted the sign-in but did not answer when asked about {upin}; nothing was committed. Please contact the ICT Admin.")),
                e => Failure::new("unreachable", Some(e.to_string()),
                    format!("Aumentum could not be asked whether {upin} is free ({e}); nothing was committed. Please contact the ICT Admin.")),
            })?;
        if r.status != 200 {
            let detail = fault(&r.body);
            let said = detail.as_deref().map(|d| format!(": {d}")).unwrap_or_default();
            return Err(Failure::new("lookup", detail,
                format!("Aumentum answered HTTP {} when asked about {upin}{said}; nothing was committed. Please contact the ICT Admin.", r.status)));
        }
        Ok(r.body.contains("<FieldName>UPIN</FieldName>"))
    }

    /// Done with the check. The shared session stays signed in for the next one (see `UpinCheck`).
    pub async fn close(self) {}

    /// Sign out (the health probe's own session, which never looks anything up).
    async fn sign_out(self) {
        let _ = http(&self.url, "POST", &[("Content-Type", "text/xml; charset=utf-8".into()), ("SOAPAction", format!("\"{NS}LogOut\"")), ("Cookie", self.cookie.clone())], &envelope("LogOut", "")).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fault_gives_aumentums_innermost_message() {
        let body = "<soap:Fault><faultcode>soap:Server</faultcode><faultstring>Server was unable to process request. ---&gt; No such host is known</faultstring></soap:Fault>";
        assert_eq!(fault(body).as_deref(), Some("No such host is known"));
        let body = "<faultstring>Invalid operation, the user credentials are not valid or the user is not authenticated</faultstring>";
        assert_eq!(fault(body).as_deref(), Some("Invalid operation, the user credentials are not valid or the user is not authenticated"));
        assert_eq!(fault("<html>500</html>"), None);
    }

    #[test]
    fn the_last_real_lookup_is_what_the_probe_reports() {
        note_lookup(Ok(()));
        assert!(recent_lookup_failure().is_none());
        let max = Failure::new("lookup", Some("Object reference not set to an instance of an object".into()), "Aumentum answered HTTP 500".into());
        note_lookup(Err(max));
        let f = recent_lookup_failure().expect("a failed lookup is reported");
        assert_eq!((f.stage, f.detail.as_deref()), ("lookup", Some("Object reference not set to an instance of an object")));
        note_lookup(Ok(()));
        assert!(recent_lookup_failure().is_none(), "a later success clears it");
    }

    #[test]
    fn which_failures_a_fresh_session_is_tried_for() {
        let f = |stage, d: &str| Failure::new(stage, Some(d.into()), String::new());
        assert!(fresh_session_may_help(&f("lookup", "Invalid operation, the user credentials are not valid or the user is not authenticated")));
        assert!(fresh_session_may_help(&f("lookup", "Object reference not set to an instance of an object")));
        assert!(!fresh_session_may_help(&f("sign-in", "Object reference not set to an instance of an object")));
        assert!(!fresh_session_may_help(&f("lookup", "Timeout expired")));
        assert!(!fresh_session_may_help(&Failure::new("lookup", None, String::new())));
    }

    #[test]
    fn no_such_host_points_at_the_outbound_address() {
        let f = Failure::new("sign-in", Some("No such host is known".into()), String::new());
        assert!(hint(&f).is_some_and(|h| h.contains("CADASTRE_OUTBOUND_ADDR")));
        assert!(hint(&Failure::new("lookup", Some("Object reference not set to an instance of an object".into()), String::new())).is_some_and(|h| h.contains("ArcSDE")));
        assert!(hint(&Failure::new("lookup", Some("Timeout expired".into()), String::new())).is_none());
        assert!(hint(&Failure::new("lookup", Some("no answer to the UPIN lookup".into()), String::new())).is_some_and(|h| h.contains("gsrvr")));
        assert_eq!(host_of("http://10.10.10.6/ApplicationServerAdmin/WebServices/LRSWebServices.asmx").as_deref(), Some("10.10.10.6"));
    }
}
