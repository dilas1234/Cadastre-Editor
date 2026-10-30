//! Runtime configuration, read from the environment at startup.

use std::env;

/// Whether the Keycloak auth layer verifies tokens or runs in dev-bypass mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthMode {
    /// Every request is treated as a dev superuser. Never use in production.
    Disabled,
    /// A bearer token is required and its RS256 signature is verified against the JWKS.
    Enabled,
}

/// Server configuration.
#[derive(Clone, Debug)]
pub struct Config {
    /// PostgreSQL/PostGIS connection string.
    pub database_url: String,
    /// Address the HTTP server binds to, e.g. `0.0.0.0:8080`.
    pub bind_addr: String,
    /// Auth enforcement mode.
    pub auth_mode: AuthMode,
    /// Keycloak realm issuer (`iss`). Used to locate the JWKS certs endpoint and to
    /// validate the token's `iss` claim.
    pub keycloak_issuer: Option<String>,
    /// Expected token audience (`aud`). Validated when set.
    pub keycloak_audience: Option<String>,
    /// Inline JWKS JSON (overrides fetching from the issuer). Useful for tests/air-gapped deploys.
    pub keycloak_jwks_json: Option<String>,
    /// Path to a JWKS JSON file (overrides fetching from the issuer).
    pub keycloak_jwks_path: Option<String>,
}

impl Config {
    /// Build configuration from environment variables.
    ///
    /// Required: `CADASTRE_DATABASE_URL` (falls back to `DATABASE_URL`).
    /// Optional: `CADASTRE_BIND` (default `0.0.0.0:8080`), `CADASTRE_AUTH`
    /// (`enabled`/`disabled`, default `disabled`), `CADASTRE_KEYCLOAK_ISSUER`,
    /// `CADASTRE_KEYCLOAK_AUDIENCE`, `CADASTRE_KEYCLOAK_JWKS_JSON`, `CADASTRE_KEYCLOAK_JWKS_PATH`.
    pub fn from_env() -> Result<Self, String> {
        let database_url = env::var("CADASTRE_DATABASE_URL")
            .or_else(|_| env::var("DATABASE_URL"))
            .map_err(|_| "CADASTRE_DATABASE_URL (or DATABASE_URL) must be set".to_string())?;

        let bind_addr = env::var("CADASTRE_BIND").unwrap_or_else(|_| "0.0.0.0:8080".to_string());

        let auth_mode = match env::var("CADASTRE_AUTH").as_deref() {
            Ok("enabled") => AuthMode::Enabled,
            Ok("disabled") | Err(_) => AuthMode::Disabled,
            Ok(other) => {
                return Err(format!("CADASTRE_AUTH must be enabled|disabled, got {other:?}"))
            }
        };

        Ok(Self {
            database_url,
            bind_addr,
            auth_mode,
            keycloak_issuer: non_empty(env::var("CADASTRE_KEYCLOAK_ISSUER").ok()),
            keycloak_audience: non_empty(env::var("CADASTRE_KEYCLOAK_AUDIENCE").ok()),
            keycloak_jwks_json: non_empty(env::var("CADASTRE_KEYCLOAK_JWKS_JSON").ok()),
            keycloak_jwks_path: non_empty(env::var("CADASTRE_KEYCLOAK_JWKS_PATH").ok()),
        })
    }
}

/// Treat an empty or whitespace-only env var as unset.
fn non_empty(v: Option<String>) -> Option<String> {
    v.filter(|s| !s.trim().is_empty())
}
