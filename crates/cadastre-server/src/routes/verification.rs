//! Checking and Data Verification unit (`/verification`; docs/checking-and-data-verification.md).
//!
//! The unit checks a surveyor's boundary (imported from CAD or a coordinate list, or sketched)
//! before any Digitising job exists. Check-only mode runs the system check's land checks on the
//! boundary and saves nothing: overlap with committed parcels here and in Aumentum, the drop
//! point, a title on the overlapped land and gaps to neighbours.

use axum::extract::State;
use axum::Json;
use serde::Deserialize;
use utoipa::ToSchema;

use crate::auth::AuthUser;
use crate::error::{ApiResult, AppError};
use crate::routes::precheck::{self, PrecheckReport};
use crate::state::AppState;

#[derive(Debug, Deserialize, ToSchema)]
pub struct VerificationCheckRequest {
    /// The boundary's corners (UTM 32N metres, ring order, not closed).
    pub vertices: Vec<[f64; 2]>,
}

/// Check a boundary without a job, saving nothing (the unit's check-only mode).
#[utoipa::path(post, path = "/api/v1/verification/check", tag = "verification",
    request_body = VerificationCheckRequest,
    responses((status = 200, body = PrecheckReport), (status = 400, description = "Fewer than 3 corners, or corners that are not numbers")))]
pub async fn check(State(state): State<AppState>, user: AuthUser, Json(req): Json<VerificationCheckRequest>) -> ApiResult<Json<PrecheckReport>> {
    user.require_role("editor")?;
    if req.vertices.len() < 3 {
        return Err(AppError::BadRequest("a boundary needs at least 3 corners".into()));
    }
    Ok(Json(precheck::run(&state, None, &req.vertices, None).await?))
}
