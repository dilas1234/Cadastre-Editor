//! Liveness and readiness probes.

use axum::extract::State;
use axum::Json;
use serde_json::{json, Value};

use crate::error::ApiResult;
use crate::state::AppState;

/// Liveness: the process is up. Does not touch the database.
#[utoipa::path(
    get, path = "/health", tag = "ops",
    responses((status = 200, description = "Process is alive"))
)]
pub async fn health() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

/// Readiness: the database is reachable and migrations are applied.
#[utoipa::path(
    get, path = "/ready", tag = "ops",
    responses(
        (status = 200, description = "Database reachable"),
        (status = 500, description = "Database unreachable"),
    )
)]
pub async fn ready(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    sqlx::query("SELECT 1").execute(&state.pool).await?;
    Ok(Json(json!({ "status": "ready" })))
}
