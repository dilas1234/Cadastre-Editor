//! Runtime configuration, read from the environment at startup.

use std::env;

/// Whether the Keycloak auth layer verifies tokens or runs in dev-bypass mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthMode {
    /// Every request is treated as an `admin` dev user. Never use in production.
    Disabled,
    /// A bearer token is required; claims are read from it (see `auth.rs`).
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
    /// Expected Keycloak issuer (`iss`) when auth is enabled. Wired into JWKS signature
    /// verification in Phase 3; read from the environment now so config is stable.
    #[allow(dead_code)]
    pub keycloak_issuer: Option<String>,
    /// Expected token audience (`aud`) when auth is enabled. See `keycloak_issuer`.
    #[allow(dead_code)]
    pub keycloak_audience: Option<String>,
}

impl Config {
    /// Build configuration from environment variables.
    ///
    /// Required: `CADASTRE_DATABASE_URL` (falls back to `DATABASE_URL`).
    /// Optional: `CADASTRE_BIND` (default `0.0.0.0:8080`), `CADASTRE_AUTH`
    /// (`enabled`/`disabled`, default `disabled`), `CADASTRE_KEYCLOAK_ISSUER`,
    /// `CADASTRE_KEYCLOAK_AUDIENCE`.
    pub fn from_env() -> Result<Self, String> {
        let database_url = env::var("CADASTRE_DATABASE_URL")
            .or_else(|_| env::var("DATABASE_URL"))
            .map_err(|_| "CADASTRE_DATABASE_URL (or DATABASE_URL) must be set".to_string())?;

        let bind_addr = env::var("CADASTRE_BIND").unwrap_or_else(|_| "0.0.0.0:8080".to_string());

        let auth_mode = match env::var("CADASTRE_AUTH").as_deref() {
            Ok("enabled") => AuthMode::Enabled,
            Ok("disabled") | Err(_) => AuthMode::Disabled,
            Ok(other) => return Err(format!("CADASTRE_AUTH must be enabled|disabled, got {other:?}")),
        };

        Ok(Self {
            database_url,
            bind_addr,
            auth_mode,
            keycloak_issuer: env::var("CADASTRE_KEYCLOAK_ISSUER").ok(),
            keycloak_audience: env::var("CADASTRE_KEYCLOAK_AUDIENCE").ok(),
        })
    }
}
