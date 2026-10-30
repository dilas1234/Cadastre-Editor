//! Parcel endpoints.

use axum::extract::State;
use axum::Json;
use sqlx::Row;

use crate::error::{ApiResult, AppError};
use crate::models::ParcelDto;
use crate::state::AppState;

fn row_to_dto(row: &sqlx::postgres::PgRow) -> ApiResult<ParcelDto> {
    let geojson: String = row.get("geometry");
    let geometry = serde_json::from_str(&geojson)
        .map_err(|_| AppError::BadRequest("stored geometry is not valid GeoJSON".to_string()))?;
    Ok(ParcelDto {
        id: row.get("id"),
        parcel_number: row.get("parcel_number"),
        geometry,
        stated_area: row.get("stated_area"),
        computed_area: row.get("computed_area"),
        status: row.get("status"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

/// List all parcels, ordered by parcel number. Boundaries are returned as GeoJSON.
#[utoipa::path(
    get, path = "/api/v1/parcels", tag = "parcels",
    responses((status = 200, body = [ParcelDto]))
)]
pub async fn list_parcels(State(state): State<AppState>) -> ApiResult<Json<Vec<ParcelDto>>> {
    let rows = sqlx::query(
        "SELECT id, parcel_number, ST_AsGeoJSON(geom) AS geometry, stated_area, \
                computed_area, status, created_at, updated_at \
         FROM cadastre.parcel ORDER BY parcel_number",
    )
    .fetch_all(&state.pool)
    .await?;

    rows.iter().map(row_to_dto).collect::<ApiResult<Vec<_>>>().map(Json)
}
