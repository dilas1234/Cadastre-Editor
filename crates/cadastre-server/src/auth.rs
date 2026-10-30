//! Keycloak authentication: an `AuthUser` extractor backed by RS256/JWKS verification.
//!
//! When `CADASTRE_AUTH=disabled` every request is a dev superuser. When enabled, the
//! bearer token's signature is verified against the JWKS loaded at startup (see `jwks`),
//! and its `iss`/`aud` are checked against the configured Keycloak realm.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;

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
