//! Tolerance configuration endpoints (FR-ADM-03).

use axum::extract::State;
use axum::Json;
use sqlx::Row;

use crate::auth::AuthUser;
use crate::error::{ApiResult, AppError};
use crate::models::TolerancesDto;
use crate::state::AppState;

fn row_to_dto(row: &sqlx::postgres::PgRow) -> TolerancesDto {
    TolerancesDto {
        closure_precision: row.get("closure_precision"),
        area_fraction: row.get("area_fraction"),
        join_distance: row.get("join_distance"),
        sliver_area: row.get("sliver_area"),
        duplicate_beacon: row.get("duplicate_beacon"),
        distance_residual: row.get("distance_residual"),
        bearing_residual_sec: row.get("bearing_residual_sec"),
    }
}

const SELECT_COLS: &str = "closure_precision, area_fraction, join_distance, sliver_area, \
     duplicate_beacon, distance_residual, bearing_residual_sec";

/// Load the active tolerance set, erroring if none is configured. Shared by the survey check.
pub async fn load_active(pool: &sqlx::PgPool) -> ApiResult<TolerancesDto> {
    let sql = format!("SELECT {SELECT_COLS} FROM cadastre.tolerance_config WHERE is_active");
    let row = sqlx::query(&sql)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| AppError::NotFound("active tolerance config".to_string()))?;
    Ok(row_to_dto(&row))
}

/// Get the active tolerance set.
#[utoipa::path(
    get, path = "/api/v1/tolerances", tag = "config",
    responses((status = 200, body = TolerancesDto))
)]
pub async fn get_tolerances(State(state): State<AppState>) -> ApiResult<Json<TolerancesDto>> {
    Ok(Json(load_active(&state.pool).await?))
}

/// Replace the active tolerance set (admin only). The previous set is retained for audit.
#[utoipa::path(
    put, path = "/api/v1/tolerances", tag = "config",
    request_body = TolerancesDto,
    responses(
        (status = 200, body = TolerancesDto),
        (status = 403, description = "Requires admin role"),
    )
)]
pub async fn put_tolerances(
    State(state): State<AppState>,
    user: AuthUser,
    Json(dto): Json<TolerancesDto>,
) -> ApiResult<Json<TolerancesDto>> {
    user.require_role("admin")?;

    let mut tx = state.pool.begin().await?;
    sqlx::query("UPDATE cadastre.tolerance_config SET is_active = false WHERE is_active")
        .execute(&mut *tx)
        .await?;

    let sql = format!(
        "INSERT INTO cadastre.tolerance_config \
         (closure_precision, area_fraction, join_distance, sliver_area, duplicate_beacon, \
          distance_residual, bearing_residual_sec, is_active, updated_by) \
         VALUES ($1,$2,$3,$4,$5,$6,$7, true, $8) RETURNING {SELECT_COLS}"
    );
    let row = sqlx::query(&sql)
        .bind(dto.closure_precision)
        .bind(dto.area_fraction)
        .bind(dto.join_distance)
        .bind(dto.sliver_area)
        .bind(dto.duplicate_beacon)
        .bind(dto.distance_residual)
        .bind(dto.bearing_residual_sec)
        .bind(&user.subject)
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;

    Ok(Json(row_to_dto(&row)))
}
