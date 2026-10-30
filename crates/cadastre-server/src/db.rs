//! Database pool construction and migration.

use std::time::Duration;

use sqlx::postgres::PgPoolOptions;
use sqlx::{Executor, PgPool};

/// Create the connection pool and run embedded migrations.
pub async fn connect_and_migrate(database_url: &str) -> Result<PgPool, sqlx::Error> {
    let pool = PgPoolOptions::new()
        .max_connections(16)
        .acquire_timeout(Duration::from_secs(10))
        // Pin the schema search path. The default is `"$user", public, ...`, and our DB
        // user is `cadastre`; once the migration below creates the `cadastre` schema,
        // `$user` would start resolving there — shifting where sqlx's unqualified
        // `_sqlx_migrations` bookkeeping table lives between runs and re-running
        // migrations. Pinning to `public` keeps it stable. Domain queries fully-qualify
        // `cadastre.*`, and PostGIS functions live in `public`, so both still resolve.
        .after_connect(|conn, _meta| {
            Box::pin(async move {
                conn.execute("SET search_path TO public").await?;
                Ok(())
            })
        })
        .connect(database_url)
        .await?;

    // Migrations are embedded from crates/cadastre-server/migrations at compile time,
    // so deployment needs no sqlx-cli and no loose .sql files.
    sqlx::migrate!("./migrations").run(&pool).await?;

    Ok(pool)
}
