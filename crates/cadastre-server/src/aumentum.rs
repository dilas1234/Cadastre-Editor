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

pub struct UpinCheck {
    url: String,
    cookie: String,
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
}

/// Asked at most once a minute, however many editors are open.
const HEALTH_SECS: u64 = 60;
/// Long enough for Aumentum's slowest normal answer, short enough to warn soon.
const HEALTH_TIMEOUT_SECS: u64 = 15;
static HEALTH: tokio::sync::Mutex<Option<(std::time::Instant, Health)>> = tokio::sync::Mutex::const_new(None);

/// A number that is never issued: only the answer matters, not what it says.
const PROBE_UPIN: &str = "PL000000000000";

/// Sign in and ask about a UPIN, as a commit does: Aumentum has been seen to sign in and then
/// answer every lookup with HTTP 500.
async fn probe() -> Result<(), String> {
    let Some(check) = UpinCheck::connect().await? else { return Ok(()) };
    let asked = check.issued(PROBE_UPIN).await;
    check.close().await;
    asked.map(|_| ())
}

/// Whether Aumentum answers, from a check at most a minute old.
pub async fn health() -> Health {
    if !UpinCheck::configured() {
        return Health { configured: false, ok: true, reason: None };
    }
    let mut cached = HEALTH.lock().await;
    if let Some((at, h)) = cached.as_ref() {
        if at.elapsed().as_secs() < HEALTH_SECS {
            return h.clone();
        }
    }
    let answer = tokio::time::timeout(std::time::Duration::from_secs(HEALTH_TIMEOUT_SECS), probe()).await
        .unwrap_or_else(|_| Err("Aumentum did not answer in time".into()));
    let h = Health { configured: true, ok: answer.is_ok(), reason: answer.err() };
    *cached = Some((std::time::Instant::now(), h.clone()));
    h
}

impl UpinCheck {
    /// Whether the parallel-run check is configured on this server.
    pub fn configured() -> bool {
        std::env::var("CADASTRE_AUMENTUM_JOBSERVICE_URL").ok().is_some_and(|s| !s.trim().is_empty())
    }

    /// Signed in to Aumentum's job service, or `Ok(None)` when the check is not configured.
    pub async fn connect() -> Result<Option<Self>, String> {
        let Some(url) = std::env::var("CADASTRE_AUMENTUM_JOBSERVICE_URL").ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) else {
            return Ok(None);
        };
        let get = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        let (Some(user), Some(pw)) = (get("CADASTRE_MC_SERVICE_USER"), get("CADASTRE_MC_SERVICE_PASSWORD")) else {
            return Err("the Aumentum UPIN check needs CADASTRE_MC_SERVICE_USER / _PASSWORD".into());
        };
        let body = envelope("Authenticate", &format!("<user>{}</user><password>{}</password>", escape(&user), escape(&pw)));
        let r = http(&url, "POST", &[("Content-Type", "text/xml; charset=utf-8".into()), ("SOAPAction", format!("\"{NS}Authenticate\""))], &body)
            .await
            .map_err(|e| format!("Aumentum could not be asked whether the UPIN is free ({e}); nothing was committed. Please contact the ICT Admin."))?;
        match (r.body.contains("<AuthenticateResult>true</AuthenticateResult>"), r.cookie) {
            (true, Some(cookie)) => Ok(Some(Self { url, cookie })),
            _ => Err("Aumentum's job service refused the sign-in for the UPIN check; nothing was committed. Please contact the ICT Admin.".into()),
        }
    }

    /// Whether Aumentum has an attribute record for this UPIN (in any status).
    pub async fn issued(&self, upin: &str) -> Result<bool, String> {
        let body = envelope("GetParcelAttributesByUPIN", &format!("<regionName>Plateau_State</regionName><upin>{}</upin>", escape(upin)));
        let r = http(&self.url, "POST", &[("Content-Type", "text/xml; charset=utf-8".into()), ("SOAPAction", format!("\"{NS}GetParcelAttributesByUPIN\"")), ("Cookie", self.cookie.clone())], &body)
            .await
            .map_err(|e| format!("Aumentum could not be asked whether {upin} is free ({e}); nothing was committed. Please contact the ICT Admin."))?;
        if r.status != 200 {
            return Err(format!("Aumentum answered HTTP {} when asked about {upin}; nothing was committed. Please contact the ICT Admin.", r.status));
        }
        Ok(r.body.contains("<FieldName>UPIN</FieldName>"))
    }

    pub async fn close(self) {
        let _ = http(&self.url, "POST", &[("Content-Type", "text/xml; charset=utf-8".into()), ("SOAPAction", format!("\"{NS}LogOut\"")), ("Cookie", self.cookie.clone())], &envelope("LogOut", "")).await;
    }
}
