//! Beacon endpoints.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use sqlx::Row;
use uuid::Uuid;

use crate::auth::AuthUser;
use crate::error::{ApiResult, AppError};
use crate::models::{BeaconDto, NewBeacon};
use crate::state::AppState;

const SELECT_COLS: &str = "id, number, ST_X(geom) AS easting, ST_Y(geom) AS northing, \
     accuracy, is_control, created_at, updated_at";

fn row_to_dto(row: &sqlx::postgres::PgRow) -> BeaconDto {
    BeaconDto {
        id: row.get("id"),
        number: row.get("number"),
        easting: row.get("easting"),
        northing: row.get("northing"),
        accuracy: row.get("accuracy"),
        is_control: row.get("is_control"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

/// List all beacons, ordered by number.
#[utoipa::path(
    get, path = "/api/v1/beacons", tag = "beacons",
    responses((status = 200, body = [BeaconDto]))
)]
pub async fn list_beacons(State(state): State<AppState>) -> ApiResult<Json<Vec<BeaconDto>>> {
    let sql = format!("SELECT {SELECT_COLS} FROM cadastre.beacon ORDER BY number");
    let rows = sqlx::query(&sql).fetch_all(&state.pool).await?;
    Ok(Json(rows.iter().map(row_to_dto).collect()))
}

/// Get one beacon by id.
#[utoipa::path(
    get, path = "/api/v1/beacons/{id}", tag = "beacons",
    params(("id" = Uuid, Path, description = "Beacon id")),
    responses((status = 200, body = BeaconDto), (status = 404, description = "Not found"))
)]
pub async fn get_beacon(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<BeaconDto>> {
    let sql = format!("SELECT {SELECT_COLS} FROM cadastre.beacon WHERE id = $1");
    let row = sqlx::query(&sql)
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("beacon {id}")))?;
    Ok(Json(row_to_dto(&row)))
}

/// Create a beacon (requires the `editor` role).
#[utoipa::path(
    post, path = "/api/v1/beacons", tag = "beacons",
    request_body = NewBeacon,
    responses(
        (status = 201, body = BeaconDto),
        (status = 400, description = "Duplicate beacon number"),
        (status = 403, description = "Requires editor role"),
    )
)]
pub async fn create_beacon(
    State(state): State<AppState>,
    user: AuthUser,
    Json(new): Json<NewBeacon>,
) -> ApiResult<(StatusCode, Json<BeaconDto>)> {
    user.require_role("editor")?;

    let sql = format!(
        "INSERT INTO cadastre.beacon (number, geom, accuracy, is_control) \
         VALUES ($1, ST_SetSRID(ST_MakePoint($2, $3), 32632), $4, $5) \
         RETURNING {SELECT_COLS}"
    );
    let result = sqlx::query(&sql)
        .bind(&new.number)
        .bind(new.easting)
        .bind(new.northing)
        .bind(new.accuracy.unwrap_or(0.05))
        .bind(new.is_control.unwrap_or(false))
        .fetch_one(&state.pool)
        .await;

    match result {
        Ok(row) => Ok((StatusCode::CREATED, Json(row_to_dto(&row)))),
        Err(e) if is_unique_violation(&e) => Err(AppError::BadRequest(format!(
            "beacon number '{}' already exists",
            new.number
        ))),
        Err(e) => Err(e.into()),
    }
}

/// Whether a sqlx error is a Postgres unique-constraint violation (SQLSTATE 23505).
fn is_unique_violation(e: &sqlx::Error) -> bool {
    e.as_database_error()
        .and_then(|d| d.code())
        .is_some_and(|c| c == "23505")
}
