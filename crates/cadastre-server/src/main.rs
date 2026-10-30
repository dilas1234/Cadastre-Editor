//! PLAGIS Cadastre Editor server entry point.

use std::sync::Arc;

use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

use cadastre_server::config::{AuthMode, Config};
use cadastre_server::openapi::ApiDoc;
use cadastre_server::state::AppState;
use cadastre_server::{db, idp, routes};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_tracing();

    let config = Config::from_env().map_err(|e| {
        tracing::error!("configuration error: {e}");
        e
    })?;

    let jwks = if config.auth_mode == AuthMode::Enabled {
        let keys = idp::load_jwks(&config).await.map_err(|e| {
            tracing::error!("failed to load Keycloak JWKS: {e}");
            e
        })?;
        tracing::info!("loaded {} Keycloak verification key(s)", keys.len());
        Some(Arc::new(keys))
    } else {
        tracing::warn!("CADASTRE_AUTH=disabled — all requests run as a dev superuser");
        None
    };

    let pool = db::connect_and_migrate(&config.database_url).await?;
    tracing::info!("database connected and migrations applied");

    let bind_addr = config.bind_addr.clone();
    let state = AppState { pool, config, jwks };

    let app = routes::router(state).merge(
        SwaggerUi::new("/swagger-ui").url("/api-docs/openapi.json", ApiDoc::openapi()),
    );

    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
    tracing::info!("cadastre-server listening on http://{bind_addr} (docs at /swagger-ui)");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

fn init_tracing() {
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            EnvFilter::new("info,cadastre_server=debug,tower_http=debug")
        }))
        .with(fmt::layer())
        .init();
}

/// Resolve when the process receives Ctrl-C (SIGINT).
async fn shutdown_signal() {
    if let Err(e) = tokio::signal::ctrl_c().await {
        tracing::error!("failed to install Ctrl-C handler: {e}");
    }
    tracing::info!("shutdown signal received");
}
