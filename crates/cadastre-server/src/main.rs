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

    // Take the port before anything else. A second copy of the server must stop here: started by a
    // service that retries every few seconds, it would otherwise migrate the database and sign in to
    // the Registry each time, and leave a session holding a licensed place there when it exits.
    let bind_addr = config.bind_addr.clone();
    let listener = tokio::net::TcpListener::bind(&bind_addr).await.map_err(|e| {
        tracing::error!("cannot listen on {bind_addr}: {e} (is another cadastre-server running?)");
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
        if config.auth_mode == AuthMode::Disabled {
            tracing::warn!("CADASTRE_AUTH=disabled — all requests run as a dev superuser");
        }
        None
    };

    let pool = db::connect_and_migrate(&config.database_url).await?;
    tracing::info!("database connected and migrations applied");

    // Task notifications: sent while an SMTP server is configured (mail.* settings).
    tokio::spawn(cadastre_server::mail::run_outbox(pool.clone()));
    // Registry links: read from the Aumentum Registry every minute while it is configured
    // (CADASTRE_REGISTRY_URL / _USER / _PASSWORD). The Registry is only read.
    match cadastre_server::lrs::LrsConfig::from_env() {
        Some(cfg) => { tokio::spawn(cadastre_server::registry_sync::run(pool.clone(), cfg)); }
        None => tracing::info!("Registry link sync off (CADASTRE_REGISTRY_URL / _USER / _PASSWORD not set)"),
    }
    // Parallel run with Aumentum: watch for UPINs Aumentum issued after this Cadastre gave them.
    if cadastre_server::aumentum::UpinCheck::configured() {
        tokio::spawn(cadastre_server::upin_watch::run(pool.clone()));
    }
    let state = AppState { pool, config, jwks };

    let app = routes::gated(
        routes::router(state.clone()).merge(SwaggerUi::new("/swagger-ui").url("/api-docs/openapi.json", ApiDoc::openapi())),
        state,
    );

    tracing::info!("cadastre-server listening on http://{bind_addr} (docs at /swagger-ui)");
    // Which build this is, in the log, so a build without Users & Units is seen at start.
    if cfg!(feature = "registry-admin") {
        tracing::info!("Users & Units (registry-admin): built in");
    } else {
        tracing::warn!("Users & Units (registry-admin): NOT built in, /user-management will answer 501; rebuild with the default features");
    }

    let (stop, mut stopping) = tokio::sync::watch::channel(false);
    let serve = axum::serve(listener, app).with_graceful_shutdown(async move {
        shutdown_signal().await;
        let _ = stop.send(true);
    });
    // Requests still running get a few seconds; then the Registry session is signed out, so a
    // restart does not leave one holding a licensed place.
    tokio::select! {
        r = async { serve.await } => r?,
        _ = async {
            if stopping.wait_for(|s| *s).await.is_err() { std::future::pending::<()>().await; }
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        } => {}
    }
    cadastre_server::lrs::shutdown().await;

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

/// Resolve when the process receives Ctrl-C (SIGINT) or, on Unix, SIGTERM (`kill`, a restart).
async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(e) = tokio::signal::ctrl_c().await {
            tracing::error!("failed to install Ctrl-C handler: {e}");
            std::future::pending::<()>().await;
        }
    };
    #[cfg(unix)]
    let term = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => { s.recv().await; }
            Err(e) => {
                tracing::error!("failed to install SIGTERM handler: {e}");
                std::future::pending::<()>().await;
            }
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! { _ = ctrl_c => {}, _ = term => {} }
    tracing::info!("shutdown signal received");
}
