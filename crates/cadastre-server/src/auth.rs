//! Keycloak authentication — **stub** for Phase 2.
//!
//! When auth is enabled this reads the claims out of the bearer JWT but does **not**
//! verify its signature yet. Real JWKS-based verification against the Keycloak realm
//! arrives in Phase 3; the extractor API below will not change when it does.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::Deserialize;

use crate::config::AuthMode;
use crate::error::AppError;
use crate::state::AppState;

/// The authenticated principal for a request.
#[derive(Clone, Debug)]
pub struct AuthUser {
    /// Subject identifier (`sub`).
    pub subject: String,
    /// Human-readable username (`preferred_username`), if present.
    /// Consumed by audit-log entries in Phase 6; captured now so the extractor is stable.
    #[allow(dead_code)]
    pub username: Option<String>,
    /// Realm roles granted to the user.
    pub roles: Vec<String>,
}

impl AuthUser {
    /// The synthetic user used when `CADASTRE_AUTH=disabled`. Holds every role so local
    /// dev can exercise all endpoints without a token.
    fn dev_user() -> Self {
        Self {
            subject: "dev".to_string(),
            username: Some("dev".to_string()),
            roles: vec!["admin".to_string(), "editor".to_string()],
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

/// The subset of Keycloak JWT claims we consume.
#[derive(Debug, Deserialize)]
struct Claims {
    sub: String,
    preferred_username: Option<String>,
    #[serde(default)]
    realm_access: RealmAccess,
}

#[derive(Debug, Default, Deserialize)]
struct RealmAccess {
    #[serde(default)]
    roles: Vec<String>,
}

/// Decode a JWT payload **without verifying the signature** (stub; see module docs).
fn decode_claims_unverified(token: &str) -> Result<Claims, AppError> {
    let payload = token
        .split('.')
        .nth(1)
        .ok_or_else(|| AppError::Unauthorized("malformed bearer token".to_string()))?;
    let bytes = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|_| AppError::Unauthorized("malformed bearer token".to_string()))?;
    serde_json::from_slice(&bytes)
        .map_err(|_| AppError::Unauthorized("token claims unreadable".to_string()))
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

        let header = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|h| h.to_str().ok())
            .ok_or_else(|| AppError::Unauthorized("missing Authorization header".to_string()))?;

        let token = header
            .strip_prefix("Bearer ")
            .ok_or_else(|| AppError::Unauthorized("expected a Bearer token".to_string()))?;

        let claims = decode_claims_unverified(token)?;
        Ok(Self {
            subject: claims.sub,
            username: claims.preferred_username,
            roles: claims.realm_access.roles,
        })
    }
}
