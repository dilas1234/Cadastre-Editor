//! Shared application state handed to every handler.

use std::sync::Arc;

use sqlx::PgPool;

use crate::config::Config;
use crate::jwks::Jwks;

/// Cloneable state (`PgPool` and `Arc<Jwks>` are both cheap to clone).
#[derive(Clone)]
pub struct AppState {
    /// PostGIS connection pool.
    pub pool: PgPool,
    /// Runtime configuration.
    pub config: Config,
    /// Verification keys, loaded at startup when auth is enabled.
    pub jwks: Option<Arc<Jwks>>,
}
