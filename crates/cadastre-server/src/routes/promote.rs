//! Promote a staged import run into the live cadastre fabric (Phase 8).
//!
//! Reads the `import` schema (populated by `cadastre-import`) and upserts the current fabric:
//! only **active** parcels become live (pending/historic/archived remain in staging as history),
//! and all beacons are promoted. Idempotent (upsert on the natural key) and recorded in the audit
//! log. Aumentum `JobID`s are not cadastre jobs, so promoted parcels have no `job_id`.

use axum::extract::{Path, State};
use axum::Json;
use serde_json::json;

use crate::auth::AuthUser;
use crate::error::{ApiResult, AppError};
use crate::models::PromoteResult;
use crate::state::AppState;
use crate::audit;

/// Promote one import run.
#[utoipa::path(post, path = "/api/v1/import/promote/{run_id}", tag = "import",
    params(("run_id" = i64, Path, description = "import.run id")),
    responses(
        (status = 200, body = PromoteResult),
        (status = 403, description = "Requires admin role"),
        (status = 404, description = "Import run not found"),
    ))]
pub async fn promote_run(
    State(state): State<AppState>,
    user: AuthUser,
    Path(run_id): Path<i64>,
) -> ApiResult<Json<PromoteResult>> {
    user.require_role("admin")?;

    let exists: Option<i64> = sqlx::query_scalar("SELECT id FROM import.run WHERE id = $1")
        .bind(run_id)
        .fetch_optional(&state.pool)
        .await?;
    if exists.is_none() {
        return Err(AppError::NotFound(format!("import run {run_id}")));
    }

    let mut tx = state.pool.begin().await?;

    let beacons = sqlx::query(
        "INSERT INTO cadastre.beacon (number, geom, is_control) \
         SELECT number, geom, false FROM import.beacon WHERE run_id = $1 \
         ON CONFLICT (number) DO UPDATE SET geom = EXCLUDED.geom, updated_at = now()",
    )
    .bind(run_id)
    .execute(&mut *tx)
    .await?
    .rows_affected();

    let parcels = sqlx::query(
        "INSERT INTO cadastre.parcel (parcel_number, geom, computed_area, status) \
         SELECT parcel_number, geom, computed_area, 'posted' FROM import.parcel \
         WHERE run_id = $1 AND status = 'active' AND parcel_number IS NOT NULL \
         ON CONFLICT (parcel_number) DO UPDATE \
           SET geom = EXCLUDED.geom, computed_area = EXCLUDED.computed_area, \
               status = 'posted', updated_at = now()",
    )
    .bind(run_id)
    .execute(&mut *tx)
    .await?
    .rows_affected();

    audit::append(
        &mut tx,
        &user.subject,
        "promote",
        "import_run",
        Some(&run_id.to_string()),
        json!({ "parcels": parcels as i64, "beacons": beacons as i64 }),
    )
    .await?;

    tx.commit().await?;
    Ok(Json(PromoteResult {
        run_id,
        parcels_promoted: parcels as usize,
        beacons_promoted: beacons as usize,
    }))
}
