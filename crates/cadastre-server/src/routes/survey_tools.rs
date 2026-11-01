//! The 35-tool computation registry (SRS COGO/curve/area/transformation tools).
//!
//! The editor executes these locally through the same `cadastre-core::tools` functions. These
//! endpoints exist so headless clients, reports and the server-side golden-file tests can run the
//! identical engine, and so the tool catalogue has one authoritative source.

use axum::extract::Path;
use axum::Json;
use cadastre_core::tools;
use serde_json::{json, Value};

use crate::error::{ApiResult, AppError};

/// List every computation tool and the inputs it accepts.
#[utoipa::path(
    get, path = "/api/v1/survey/tools", tag = "survey",
    responses((status = 200, description = "The 35 computation tools"))
)]
pub async fn list_tools() -> Json<Value> {
    Json(json!({
        "count": tools::TOOL_COUNT,
        "tools": tools::catalog(),
    }))
}

/// Describe one tool.
#[utoipa::path(
    get, path = "/api/v1/survey/tools/{id}", tag = "survey",
    params(("id" = String, Path, description = "Tool id, e.g. `inverse`")),
    responses(
        (status = 200, description = "Tool specification"),
        (status = 404, description = "Unknown tool"),
    )
)]
pub async fn get_tool(Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let spec = tools::tool_spec(&id).ok_or_else(|| AppError::NotFound(format!("tool `{id}`")))?;
    Ok(Json(json!(spec)))
}

/// Execute one computation tool against a JSON input object.
#[utoipa::path(
    post, path = "/api/v1/survey/tools/{id}", tag = "survey",
    params(("id" = String, Path, description = "Tool id, e.g. `inverse`")),
    responses(
        (status = 200, description = "Tool result"),
        (status = 400, description = "Invalid input for this tool"),
        (status = 404, description = "Unknown tool"),
    )
)]
pub async fn execute_tool(
    Path(id): Path<String>,
    Json(input): Json<Value>,
) -> ApiResult<Json<Value>> {
    let spec = tools::tool_spec(&id).ok_or_else(|| AppError::NotFound(format!("tool `{id}`")))?;
    let result =
        tools::execute(&id, &input).map_err(|e| AppError::BadRequest(format!("{id}: {e}")))?;
    Ok(Json(json!({
        "tool": id,
        "name": spec.name,
        "result": result,
    })))
}
