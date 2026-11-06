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

/// How to reach the Aumentum Registry's SQL Server (LRS43) for the user-admin write-path. A plain
/// value (no driver dependency); the `registry-admin` feature's `regadmin` module consumes it.
#[derive(Clone)]
pub struct RegistrySqlConfig {
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    pub password: String,
    /// Use TLS. SQL Server 2012 RTM offers only TLS 1.0, which rustls refuses, so that server needs
    /// `CADASTRE_REGISTRY_SQL_ENCRYPT=off`: the login and every value then cross the LAN unprotected.
    pub encrypt: bool,
}

impl std::fmt::Debug for RegistrySqlConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RegistrySqlConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("database", &self.database)
            .field("username", &self.username)
            .field("password", &"****")
            .field("encrypt", &self.encrypt)
            .finish()
    }
}

/// Encryption stays on unless it is plainly turned off, so a mistyped value cannot weaken it.
fn encrypt_setting(value: Option<&str>) -> bool {
    !value.is_some_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "off" | "false" | "0"))
}

impl RegistrySqlConfig {
    /// `CADASTRE_REGISTRY_SQL_HOST`, `_USER`, `_PASSWORD` are required; `_PORT` (default 1433),
    /// `_DATABASE` (default `LRS43`) and `_ENCRYPT` (default on) are optional. `None` when any
    /// required var is unset — the Registry-admin dashboard is then off.
    pub fn from_env() -> Option<Self> {
        let get = |k: &str| env::var(k).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
        Some(Self {
            host: get("CADASTRE_REGISTRY_SQL_HOST")?,
            port: get("CADASTRE_REGISTRY_SQL_PORT").and_then(|v| v.parse().ok()).unwrap_or(1433),
            database: get("CADASTRE_REGISTRY_SQL_DATABASE").unwrap_or_else(|| "LRS43".to_string()),
            username: get("CADASTRE_REGISTRY_SQL_USER")?,
            password: get("CADASTRE_REGISTRY_SQL_PASSWORD")?,
            encrypt: encrypt_setting(get("CADASTRE_REGISTRY_SQL_ENCRYPT").as_deref()),
        })
    }
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
    /// Registry (LRS43) SQL Server connection for the user-admin dashboard, when configured
    /// (`CADASTRE_REGISTRY_SQL_*`). Used only by the `registry-admin` feature.
    pub registry_sql: Option<RegistrySqlConfig>,
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
            registry_sql: RegistrySqlConfig::from_env(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_sql_encryption_is_off_only_when_plainly_asked() {
        for v in ["off", "OFF", " false ", "0"] {
            assert!(!encrypt_setting(Some(v)), "{v}");
        }
        for v in [None, Some("on"), Some("of"), Some("no"), Some("")] {
            assert!(encrypt_setting(v), "{v:?}");
        }
    }
}
