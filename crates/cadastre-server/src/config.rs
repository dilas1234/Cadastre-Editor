//! Runtime configuration, read from the environment at startup.

use std::env;
use std::path::PathBuf;

/// Whether the Keycloak auth layer verifies tokens or runs in dev-bypass mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthMode {
    /// Every request is treated as a dev superuser. Never use in production.
    Disabled,
    /// A bearer token is required and its RS256 signature is verified against the JWKS.
    Enabled,
    /// This server's own sign-in page and session cookie (`routes::login`).
    Local,
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
    /// Catalog folder connections (`CADASTRE_CATALOG_FOLDERS`, `name=path;…`), read-only.
    pub catalog_folders: Vec<crate::catalog::CatalogFolder>,
    /// Catalog database connections (`CADASTRE_CATALOG_DATABASES`, `name=jsonpath;…`) — geodatabase
    /// feature-class lists read from a snapshot JSON.
    pub catalog_databases: Vec<crate::catalog::DatabaseConnection>,
}

impl Config {
    /// Build configuration from environment variables.
    ///
    /// Required: `CADASTRE_DATABASE_URL` (falls back to `DATABASE_URL`).
    /// Optional: `CADASTRE_BIND` (default `0.0.0.0:8080`), `CADASTRE_AUTH`
    /// (`local`/`enabled`/`disabled`, default `disabled`), `CADASTRE_KEYCLOAK_ISSUER`,
    /// `CADASTRE_KEYCLOAK_AUDIENCE`, `CADASTRE_KEYCLOAK_JWKS_JSON`, `CADASTRE_KEYCLOAK_JWKS_PATH`,
    /// `CADASTRE_CATALOG_FOLDERS` (e.g. `\\PRODUCTIONMC\Image=D:\Image`).
    pub fn from_env() -> Result<Self, String> {
        let database_url = env::var("CADASTRE_DATABASE_URL")
            .or_else(|_| env::var("DATABASE_URL"))
            .map_err(|_| "CADASTRE_DATABASE_URL (or DATABASE_URL) must be set".to_string())?;

        let bind_addr = env::var("CADASTRE_BIND").unwrap_or_else(|_| "0.0.0.0:8080".to_string());

        let auth_mode = match env::var("CADASTRE_AUTH").as_deref() {
            Ok("enabled") => AuthMode::Enabled,
            Ok("local") => AuthMode::Local,
            Ok("disabled") | Err(_) => AuthMode::Disabled,
            Ok(other) => {
                return Err(format!("CADASTRE_AUTH must be local|enabled|disabled, got {other:?}"))
            }
        };

        let mut catalog_folders =
            crate::catalog::parse_folders(&env::var("CADASTRE_CATALOG_FOLDERS").unwrap_or_default());
        let mut catalog_databases =
            crate::catalog::parse_databases(&env::var("CADASTRE_CATALOG_DATABASES").unwrap_or_default());

        // Fall back to the saved catalog config file when the env vars are unset, so the Catalog
        // connections survive a restart that forgets them (per-list: env wins when present).
        let config_path = catalog_config_path();
        if catalog_folders.is_empty() || catalog_databases.is_empty() {
            if let Some(cfg) = config_path.as_deref().and_then(crate::catalog::load_config) {
                if catalog_folders.is_empty() {
                    catalog_folders = cfg.folders;
                }
                if catalog_databases.is_empty() {
                    catalog_databases = cfg.databases;
                }
            }
        }
        // Persist whatever we resolved, so a later bare restart (no env) still has the connections.
        if let Some(path) = &config_path {
            if !catalog_folders.is_empty() || !catalog_databases.is_empty() {
                let cfg = crate::catalog::CatalogConfig {
                    folders: catalog_folders.clone(),
                    databases: catalog_databases.clone(),
                };
                if let Err(e) = crate::catalog::save_config(path, &cfg) {
                    eprintln!("warning: could not save catalog config to {}: {e}", path.display());
                }
            }
        }

        Ok(Self {
            database_url,
            bind_addr,
            auth_mode,
            keycloak_issuer: non_empty(env::var("CADASTRE_KEYCLOAK_ISSUER").ok()),
            keycloak_audience: non_empty(env::var("CADASTRE_KEYCLOAK_AUDIENCE").ok()),
            keycloak_jwks_json: non_empty(env::var("CADASTRE_KEYCLOAK_JWKS_JSON").ok()),
            keycloak_jwks_path: non_empty(env::var("CADASTRE_KEYCLOAK_JWKS_PATH").ok()),
            catalog_folders,
            catalog_databases,
        })
    }
}

/// Where the catalog config file lives: `CADASTRE_CATALOG_CONFIG` if set, else
/// `~/.cadastre/catalog.json` (`%USERPROFILE%\.cadastre\catalog.json` on Windows).
fn catalog_config_path() -> Option<PathBuf> {
    if let Ok(p) = env::var("CADASTRE_CATALOG_CONFIG") {
        if !p.trim().is_empty() {
            return Some(PathBuf::from(p.trim()));
        }
    }
    let home = env::var("HOME").or_else(|_| env::var("USERPROFILE")).ok()?;
    Some(PathBuf::from(home).join(".cadastre").join("catalog.json"))
}

/// Treat an empty or whitespace-only env var as unset.
fn non_empty(v: Option<String>) -> Option<String> {
    v.filter(|s| !s.trim().is_empty())
}
