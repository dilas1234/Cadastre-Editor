//! HTTP routing.

pub mod beacons;
pub mod health;
pub mod parcels;
pub mod tolerances;

use axum::routing::get;
use axum::Router;
use tower_http::trace::TraceLayer;

use crate::state::AppState;

/// Build the application router with all routes and middleware.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health::health))
        .route("/ready", get(health::ready))
        .route(
            "/api/v1/tolerances",
            get(tolerances::get_tolerances).put(tolerances::put_tolerances),
        )
        .route(
            "/api/v1/beacons",
            get(beacons::list_beacons).post(beacons::create_beacon),
        )
        .route("/api/v1/beacons/{id}", get(beacons::get_beacon))
        .route("/api/v1/parcels", get(parcels::list_parcels))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
