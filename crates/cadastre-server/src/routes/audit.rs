//! Audit-log endpoints (NFR-07/11): list recent entries and verify chain integrity.

use axum::extract::State;
use axum::Json;
use sqlx::Row;

use crate::auth::AuthUser;
use crate::error::ApiResult;
use crate::models::{AuditEntryDto, AuditVerifyDto};
use crate::state::AppState;

/// List the most recent audit entries (admin).
#[utoipa::path(get, path = "/api/v1/audit", tag = "audit",
    responses((status = 200, body = [AuditEntryDto]), (status = 403, description = "Requires admin role")))]
pub async fn list_audit(
    State(state): State<AppState>,
    user: AuthUser,
) -> ApiResult<Json<Vec<AuditEntryDto>>> {
    user.require_role("admin")?;
    let rows = sqlx::query(
        "SELECT id, actor, action, entity, entity_id, hash, created_at \
         FROM cadastre.audit_log ORDER BY id DESC LIMIT 200",
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(
        rows.iter()
            .map(|r| AuditEntryDto {
                id: r.get("id"),
                actor: r.get("actor"),
                action: r.get("action"),
                entity: r.get("entity"),
                entity_id: r.get("entity_id"),
                hash: r.get("hash"),
                created_at: r.get("created_at"),
            })
            .collect(),
    ))
}

/// Verify the whole audit chain (admin). Returns whether it is intact and, if not, the first
/// broken entry's id.
#[utoipa::path(get, path = "/api/v1/audit/verify", tag = "audit",
    responses((status = 200, body = AuditVerifyDto), (status = 403, description = "Requires admin role")))]
pub async fn verify_audit(
    State(state): State<AppState>,
    user: AuthUser,
) -> ApiResult<Json<AuditVerifyDto>> {
    user.require_role("admin")?;
    let r = crate::audit::verify_chain(&state.pool).await?;
    Ok(Json(AuditVerifyDto {
        ok: r.ok,
        checked: r.checked,
        first_bad: r.first_bad,
    }))
}
