//! Parcel endpoints: listing, capture under a job, and search.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use sqlx::{QueryBuilder, Row};
use uuid::Uuid;

use crate::auth::AuthUser;
use crate::error::{ApiResult, AppError};
use crate::models::{NewParcel, ParcelDto, ParcelSearch};
use crate::state::AppState;

const PARCEL_SELECT: &str = "id, parcel_number, job_id, ST_AsGeoJSON(geom) AS geometry, \
     stated_area, computed_area, status, created_at, updated_at";

fn row_to_dto(row: &sqlx::postgres::PgRow) -> ApiResult<ParcelDto> {
    let geojson: String = row.get("geometry");
    let geometry = serde_json::from_str(&geojson)
        .map_err(|_| AppError::BadRequest("stored geometry is not valid GeoJSON".to_string()))?;
    Ok(ParcelDto {
        id: row.get("id"),
        parcel_number: row.get("parcel_number"),
        job_id: row.get("job_id"),
        geometry,
        stated_area: row.get("stated_area"),
        computed_area: row.get("computed_area"),
        status: row.get("status"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

/// List all parcels, newest first.
#[utoipa::path(get, path = "/api/v1/parcels", tag = "parcels",
    responses((status = 200, body = [ParcelDto])))]
pub async fn list_parcels(State(state): State<AppState>) -> ApiResult<Json<Vec<ParcelDto>>> {
    let sql = format!("SELECT {PARCEL_SELECT} FROM cadastre.parcel ORDER BY created_at DESC LIMIT 500");
    let rows = sqlx::query(&sql).fetch_all(&state.pool).await?;
    rows.iter().map(row_to_dto).collect::<ApiResult<Vec<_>>>().map(Json)
}

/// Capture a draft parcel under a job (requires `editor`). The official number is assigned
/// later, at posting (FR-NUM-01), so a draft has no `parcel_number`.
#[utoipa::path(post, path = "/api/v1/jobs/{id}/parcels", tag = "parcels",
    params(("id" = Uuid, Path, description = "Job id")),
    request_body = NewParcel,
    responses((status = 201, body = ParcelDto), (status = 400, description = "Invalid geometry")))]
pub async fn create_job_parcel(
    State(state): State<AppState>,
    user: AuthUser,
    Path(job_id): Path<Uuid>,
    Json(new): Json<NewParcel>,
) -> ApiResult<(StatusCode, Json<ParcelDto>)> {
    user.require_role("editor")?;

    let sql = format!(
        "INSERT INTO cadastre.parcel (job_id, geom, stated_area, status) \
         VALUES ($1, ST_SetSRID(ST_GeomFromGeoJSON($2), 32632), $3, 'draft') \
         RETURNING {PARCEL_SELECT}"
    );
    let row = sqlx::query(&sql)
        .bind(job_id)
        .bind(new.geometry.to_string())
        .bind(new.stated_area)
        .fetch_one(&state.pool)
        .await
        .map_err(|e| match e.as_database_error().and_then(|d| d.code()) {
            // Bad GeoJSON / geometry errors surface as a client error, not a 500.
            Some(code) if code == "XX000" || code.starts_with("22") => {
                AppError::BadRequest("invalid GeoJSON geometry".to_string())
            }
            _ => AppError::from(e),
        })?;
    Ok((StatusCode::CREATED, Json(row_to_dto(&row)?)))
}

/// Search parcels by number prefix, status, and/or bounding box (FR-VIEW).
#[utoipa::path(get, path = "/api/v1/parcels/search", tag = "parcels",
    params(ParcelSearch),
    responses((status = 200, body = [ParcelDto]), (status = 400, description = "Invalid bbox")))]
pub async fn search_parcels(
    State(state): State<AppState>,
    Query(q): Query<ParcelSearch>,
) -> ApiResult<Json<Vec<ParcelDto>>> {
    let mut qb: QueryBuilder<sqlx::Postgres> =
        QueryBuilder::new("SELECT ");
    qb.push(PARCEL_SELECT).push(" FROM cadastre.parcel WHERE 1=1");

    if let Some(number) = q.number.as_deref().filter(|s| !s.is_empty()) {
        qb.push(" AND parcel_number ILIKE ").push_bind(format!("{number}%"));
    }
    if let Some(status) = q.status.as_deref().filter(|s| !s.is_empty()) {
        qb.push(" AND status = ").push_bind(status.to_string());
    }
    if let Some(bbox) = q.bbox.as_deref().filter(|s| !s.is_empty()) {
        let (minx, miny, maxx, maxy) = parse_bbox(bbox)?;
        qb.push(" AND geom && ST_MakeEnvelope(")
            .push_bind(minx).push(", ").push_bind(miny).push(", ")
            .push_bind(maxx).push(", ").push_bind(maxy).push(", 32632)");
    }
    qb.push(" ORDER BY parcel_number NULLS LAST LIMIT 500");

    let rows = qb.build().fetch_all(&state.pool).await?;
    rows.iter().map(row_to_dto).collect::<ApiResult<Vec<_>>>().map(Json)
}

/// Parse a `minx,miny,maxx,maxy` bounding box.
fn parse_bbox(s: &str) -> ApiResult<(f64, f64, f64, f64)> {
    let nums: Vec<f64> = s.split(',').filter_map(|v| v.trim().parse().ok()).collect();
    match nums.as_slice() {
        [minx, miny, maxx, maxy] => Ok((*minx, *miny, *maxx, *maxy)),
        _ => Err(AppError::BadRequest(
            "bbox must be 'minx,miny,maxx,maxy'".to_string(),
        )),
    }
}
