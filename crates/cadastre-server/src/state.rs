//! Shared application state handed to every handler.

use sqlx::PgPool;

use crate::config::Config;

/// Cloneable state (both fields are cheap to clone: `PgPool` is an `Arc`).
#[derive(Clone)]
pub struct AppState {
    /// PostGIS connection pool.
    pub pool: PgPool,
    /// Runtime configuration.
    pub config: Config,
}
