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
            .map_err(|e| format!("Aumentum could not be asked whether the UPIN is free ({e}); nothing was committed"))?;
        match (r.body.contains("<AuthenticateResult>true</AuthenticateResult>"), r.cookie) {
            (true, Some(cookie)) => Ok(Some(Self { url, cookie })),
            _ => Err("Aumentum's job service refused the sign-in for the UPIN check; nothing was committed".into()),
        }
    }

    /// Whether Aumentum has an attribute record for this UPIN (in any status).
    pub async fn issued(&self, upin: &str) -> Result<bool, String> {
        let body = envelope("GetParcelAttributesByUPIN", &format!("<regionName>Plateau_State</regionName><upin>{}</upin>", escape(upin)));
        let r = http(&self.url, "POST", &[("Content-Type", "text/xml; charset=utf-8".into()), ("SOAPAction", format!("\"{NS}GetParcelAttributesByUPIN\"")), ("Cookie", self.cookie.clone())], &body)
            .await
            .map_err(|e| format!("Aumentum could not be asked whether {upin} is free ({e}); nothing was committed"))?;
        if r.status != 200 {
            return Err(format!("Aumentum answered HTTP {} when asked about {upin}; nothing was committed", r.status));
        }
        Ok(r.body.contains("<FieldName>UPIN</FieldName>"))
    }

    pub async fn close(self) {
        let _ = http(&self.url, "POST", &[("Content-Type", "text/xml; charset=utf-8".into()), ("SOAPAction", format!("\"{NS}LogOut\"")), ("Cookie", self.cookie.clone())], &envelope("LogOut", "")).await;
    }
}
