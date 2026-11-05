//! Keycloak authentication: an `AuthUser` extractor backed by RS256/JWKS verification.
//!
//! When `CADASTRE_AUTH=disabled` every request is a dev superuser. When enabled, the
//! bearer token's signature is verified against the JWKS loaded at startup (see `jwks`),
//! and its `iss`/`aud` are checked against the configured Keycloak realm. With
//! `CADASTRE_AUTH=local`, `gate` signs each request in from its session cookie and decides what
//! it may reach (see `routes::login`).

use axum::extract::{FromRequestParts, Request, State};
use axum::http::request::Parts;
use axum::http::{header, HeaderMap, Method};
use axum::middleware::Next;
use axum::response::{IntoResponse, Redirect, Response};
use sha2::{Digest, Sha256};
use sqlx::Row;

use crate::config::AuthMode;
use crate::error::AppError;
use crate::state::AppState;

/// The authenticated principal for a request.
#[derive(Clone, Debug)]
pub struct AuthUser {
    /// Subject identifier (`sub`).
    pub subject: String,
    /// Human-readable username (`preferred_username`), if present.
    pub username: Option<String>,
    /// Realm roles granted to the user.
    pub roles: Vec<String>,
}

impl AuthUser {
    /// The synthetic user used when `CADASTRE_AUTH=disabled`. Holds every workflow role so
    /// local dev can exercise all endpoints without a token.
    fn dev_user() -> Self {
        Self {
            subject: "dev".to_string(),
            username: Some("dev".to_string()),
            roles: ["admin", "editor", "reviewer", "approver"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
        }
    }

    /// Whether the user holds the given realm role.
    pub fn has_role(&self, role: &str) -> bool {
        self.roles.iter().any(|r| r == role)
    }

    /// Require a role, returning `Forbidden` otherwise.
    pub fn require_role(&self, role: &str) -> Result<(), AppError> {
        if self.has_role(role) {
            Ok(())
        } else {
            Err(AppError::Forbidden(format!("requires role '{role}'")))
        }
    }
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        if state.config.auth_mode == AuthMode::Disabled {
            return Ok(Self::dev_user());
        }
        if state.config.auth_mode == AuthMode::Local {
            return parts.extensions.get::<LocalSession>().map(|s| s.user.clone())
                .ok_or_else(|| AppError::Unauthorized("sign in first".to_string()));
        }

        let header = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|h| h.to_str().ok())
            .ok_or_else(|| AppError::Unauthorized("missing Authorization header".to_string()))?;

        let token = header
            .strip_prefix("Bearer ")
            .ok_or_else(|| AppError::Unauthorized("expected a Bearer token".to_string()))?;

        // Auth is enabled, so the JWKS must have loaded at startup; its absence is a
        // server misconfiguration rather than a client error.
        let jwks = state.jwks.as_ref().ok_or_else(|| {
            AppError::Unauthorized("auth enabled but no verification keys loaded".to_string())
        })?;

        let claims = jwks
            .verify(
                token,
                state.config.keycloak_issuer.as_deref(),
                state.config.keycloak_audience.as_deref(),
            )
            .map_err(|e| AppError::Unauthorized(e.to_string()))?;

        Ok(Self {
            subject: claims.subject,
            username: claims.username,
            roles: claims.roles,
        })
    }
}

/// The session cookie of `CADASTRE_AUTH=local`.
pub const SESSION_COOKIE: &str = "cad_session";

/// What an account may open, from its groups (nested groups count).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Access {
    /// In the Administrators group.
    pub admin: bool,
    /// In the Users group.
    pub staff: bool,
}

/// The Registry groups that open Sysadmin.
const REGISTRY_ADMIN_GROUPS: [&str; 1] = ["GROUP_LRS_ADMINISTRATION_SYSTEM"];
/// The Registry groups that open Digitising and Title Deed.
const REGISTRY_STAFF_GROUPS: [&str; 4] = ["GROUP_Scanning Clerk", "GROUP_Scanning Officer", "GROUP_Map Compilation Officer", "GROUP_Vectorization Clerk"];

/// What a Registry user's groups allow.
pub fn registry_access(groups: &[String]) -> Access {
    let any = |names: &[&str]| groups.iter().any(|g| names.contains(&g.as_str()));
    Access { admin: any(&REGISTRY_ADMIN_GROUPS), staff: any(&REGISTRY_STAFF_GROUPS) }
}

/// The workstations of the sign-in page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Sysadmin,
    Digitising,
    TitleDeed,
    Reporting,
}

impl View {
    pub fn parse(key: &str) -> Option<Self> {
        match key {
            "sysadmin" => Some(Self::Sysadmin),
            "digitising" => Some(Self::Digitising),
            "titledeed" => Some(Self::TitleDeed),
            "reporting" => Some(Self::Reporting),
            _ => None,
        }
    }
    pub fn key(self) -> &'static str {
        match self { Self::Sysadmin => "sysadmin", Self::Digitising => "digitising", Self::TitleDeed => "titledeed", Self::Reporting => "reporting" }
    }
    pub fn path(self) -> &'static str {
        match self { Self::Sysadmin => "/sys-editor", Self::Digitising => "/digitising", Self::TitleDeed => "/title-deed", Self::Reporting => "/reporting" }
    }
    pub fn label(self) -> &'static str {
        match self { Self::Sysadmin => "Sysadmin", Self::Digitising => "Digitising", Self::TitleDeed => "Title Deed", Self::Reporting => "Charting and Reporting" }
    }
    pub fn allowed(self, a: Access) -> bool {
        match self {
            Self::Sysadmin => a.admin,
            Self::Digitising | Self::TitleDeed => a.admin || a.staff,
            Self::Reporting => true,
        }
    }
}

/// A signed-in request (`CADASTRE_AUTH=local`).
#[derive(Clone, Debug)]
pub struct LocalSession {
    pub sid: i32,
    pub code: String,
    pub must_change: bool,
    /// Signed in with a Registry password: it is changed in the Registry, not here.
    pub from_registry: bool,
    pub access: Access,
    pub user: AuthUser,
}

pub fn token_hash(token: &str) -> String {
    Sha256::digest(token.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

pub fn cookie_token(headers: &HeaderMap) -> Option<String> {
    headers.get_all(header::COOKIE).iter().filter_map(|h| h.to_str().ok()).flat_map(|h| h.split(';'))
        .find_map(|c| c.trim().strip_prefix(SESSION_COOKIE).and_then(|v| v.strip_prefix('=')).map(str::to_string))
        .filter(|t| !t.is_empty())
}

pub async fn access_of(pool: &sqlx::PgPool, sid: i32) -> Result<Access, sqlx::Error> {
    let r = sqlx::query(
        "WITH RECURSIVE g(sid) AS (SELECT $1::integer UNION SELECT m.group_sid FROM cadastre.principal_member m JOIN g ON m.member_sid = g.sid) \
         SELECT COALESCE(bool_or(lower(p.code) = 'admins'), false) AS admin, COALESCE(bool_or(lower(p.code) = 'users'), false) AS staff \
         FROM g JOIN cadastre.principal p ON p.sid = g.sid AND NOT p.is_user")
        .bind(sid).fetch_one(pool).await?;
    Ok(Access { admin: r.get("admin"), staff: r.get("staff") })
}

/// The workflow roles of a signed-in account. Staff work on the fabric; `admin` is only for the
/// Administrators group. Anyone else reads.
fn roles_of(a: Access) -> Vec<String> {
    let mut roles: Vec<&str> = Vec::new();
    if a.admin { roles.push("admin"); }
    if a.admin || a.staff { roles.extend(["editor", "reviewer", "approver"]); }
    roles.into_iter().map(str::to_string).collect()
}

async fn load_session(pool: &sqlx::PgPool, token: &str) -> Result<Option<LocalSession>, sqlx::Error> {
    let row = sqlx::query(
        "SELECT p.sid, p.code, s.source = 'registry' AS from_registry, s.registry_admin, s.registry_staff, COALESCE(c.must_change, false) AS must_change \
         FROM cadastre.user_session s JOIN cadastre.principal p ON p.sid = s.sid LEFT JOIN cadastre.user_credential c ON c.sid = s.sid \
         WHERE s.token_hash = $1 AND s.expires_at > now() AND p.is_user AND NOT p.is_locked AND (s.source = 'registry' OR c.sid IS NOT NULL)")
        .bind(token_hash(token)).fetch_optional(pool).await?;
    let Some(r) = row else { return Ok(None) };
    let (sid, code): (i32, String) = (r.get("sid"), r.get("code"));
    let local = access_of(pool, sid).await?;
    let from_registry: bool = r.get("from_registry");
    let access = Access { admin: local.admin || r.get::<bool, _>("registry_admin"), staff: local.staff || r.get::<bool, _>("registry_staff") };
    let user = AuthUser { subject: code.clone(), username: Some(code.clone()), roles: roles_of(access) };
    Ok(Some(LocalSession { sid, code, must_change: !from_registry && r.get::<bool, _>("must_change"), from_registry, access, user }))
}

/// Reachable without signing in: the sign-in page, health checks, and what the Registry and
/// Aumentum call themselves (their job service and map), which have no browser session.
pub fn is_public(path: &str) -> bool {
    matches!(path, "/" | "/login" | "/health" | "/ready" | "/api/v1/auth/config" | "/api/v1/auth/login" | "/api/v1/auth/logout")
        || path == crate::routes::mcservice::PATH
        || path.starts_with("/GISViewer/")
        || path.starts_with("/editor/login/")
        || path.starts_with("/editor/vendor/")
}

/// Whether a signed-in account may reach this address.
pub fn may_reach(path: &str, method: &Method, a: Access) -> bool {
    let staff = a.admin || a.staff;
    match path {
        "/sys-editor" => return a.admin,
        "/digitising" | "/editor" | "/title-deed" => return staff,
        _ => {}
    }
    if path.starts_with("/api/v1/admin") || path.starts_with("/api/v1/audit") || path.starts_with("/swagger-ui") || path.starts_with("/api-docs") {
        return a.admin;
    }
    if path.starts_with("/api/") && !path.starts_with("/api/v1/auth/") && !matches!(*method, Method::GET | Method::HEAD) {
        return staff;
    }
    true
}

/// Every request under `CADASTRE_AUTH=local`: signs it in from its cookie and refuses what the
/// account may not reach. Pages go to the sign-in page, API calls get 401 or 403.
pub async fn gate(State(state): State<AppState>, mut req: Request, next: Next) -> Response {
    if state.config.auth_mode != AuthMode::Local {
        return next.run(req).await;
    }
    let path = req.uri().path().to_string();
    if is_public(&path) {
        return next.run(req).await;
    }
    let api = path.starts_with("/api/");
    let session = match cookie_token(req.headers()) {
        Some(t) => match load_session(&state.pool, &t).await {
            Ok(s) => s,
            Err(e) => return AppError::from(e).into_response(),
        },
        None => None,
    };
    let Some(s) = session else {
        return if api { AppError::Unauthorized("sign in first".to_string()).into_response() } else { Redirect::temporary("/login").into_response() };
    };
    if s.must_change && !matches!(path.as_str(), "/api/v1/auth/change-password" | "/api/v1/me") {
        return if api { AppError::Forbidden("change your temporary password first".to_string()).into_response() } else { Redirect::temporary("/login?change=1").into_response() };
    }
    if !may_reach(&path, req.method(), s.access) {
        return if api { AppError::Forbidden("your account may not do this".to_string()).into_response() } else { Redirect::temporary("/login?denied=1").into_response() };
    }
    req.extensions_mut().insert(s);
    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::*;

    const ADMIN: Access = Access { admin: true, staff: false };
    const STAFF: Access = Access { admin: false, staff: true };
    const OTHER: Access = Access { admin: false, staff: false };

    #[test]
    fn views_follow_the_groups() {
        assert!(View::Sysadmin.allowed(ADMIN) && !View::Sysadmin.allowed(STAFF) && !View::Sysadmin.allowed(OTHER));
        for v in [View::Digitising, View::TitleDeed] {
            assert!(v.allowed(ADMIN) && v.allowed(STAFF) && !v.allowed(OTHER));
        }
        assert!(View::Reporting.allowed(OTHER));
        for k in ["sysadmin", "digitising", "titledeed", "reporting"] {
            assert_eq!(View::parse(k).unwrap().key(), k);
        }
        assert_eq!(View::parse("root"), None);
    }

    #[test]
    fn addresses_follow_the_groups() {
        let (get, post) = (Method::GET, Method::POST);
        assert!(may_reach("/sys-editor", &get, ADMIN) && !may_reach("/sys-editor", &get, STAFF));
        assert!(!may_reach("/api/v1/admin/security", &get, STAFF) && !may_reach("/api/v1/audit", &get, STAFF));
        assert!(!may_reach("/swagger-ui/", &get, STAFF));
        assert!(may_reach("/title-deed", &get, STAFF) && !may_reach("/title-deed", &get, OTHER));
        assert!(may_reach("/reporting", &get, OTHER));
        assert!(may_reach("/api/v1/jobs", &get, OTHER) && !may_reach("/api/v1/jobs", &post, OTHER));
        assert!(may_reach("/api/v1/jobs", &post, STAFF));
        assert!(may_reach("/api/v1/auth/change-password", &post, OTHER));
    }

    #[test]
    fn the_registry_keeps_its_services() {
        assert!(is_public(crate::routes::mcservice::PATH));
        assert!(is_public("/GISViewer/MapWorker.aspx") && is_public("/editor/vendor/proj4.js"));
        assert!(!is_public("/editor/app.js") && !is_public("/digitising") && !is_public("/api/v1/jobs") && !is_public("/viewer"));
    }

    #[test]
    fn registry_groups_give_access() {
        let g = |names: &[&str]| names.iter().map(|n| n.to_string()).collect::<Vec<_>>();
        assert_eq!(registry_access(&g(&["GROUP_LRS", "GROUP_Scanning Clerk"])), STAFF);
        assert_eq!(registry_access(&g(&["GROUP_Vectorization Clerk"])), STAFF);
        assert_eq!(registry_access(&g(&["GROUP_LRS_ADMINISTRATION_SYSTEM"])), ADMIN);
        assert_eq!(registry_access(&g(&["GROUP_LRS", "GROUP_Cashier", "GROUP_Data Entry (Charting and Reporting)"])), OTHER);
        assert_eq!(registry_access(&[]), OTHER);
    }

    #[test]
    fn roles_follow_the_groups() {
        assert!(roles_of(ADMIN).contains(&"admin".to_string()));
        assert!(!roles_of(STAFF).contains(&"admin".to_string()) && roles_of(STAFF).contains(&"editor".to_string()));
        assert!(roles_of(OTHER).is_empty());
    }

    #[test]
    fn the_cookie_is_found_among_others() {
        let mut h = HeaderMap::new();
        h.insert(header::COOKIE, "a=1; cad_session=abc; b=2".parse().unwrap());
        assert_eq!(cookie_token(&h).as_deref(), Some("abc"));
        h.insert(header::COOKIE, "cad_sessions=abc".parse().unwrap());
        assert_eq!(cookie_token(&h), None);
    }
}
