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

    job_area_guard(&state, job_id, Some(&new.geometry)).await?;

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

/// Under an Aumentum job, edits follow its job area: the job must be open and have an open job
/// area, and a parcel geometry (when given) must lie inside it (Aumentum's rule and message).
pub(crate) async fn job_area_guard(state: &AppState, job_id: Uuid, geometry: Option<&serde_json::Value>) -> ApiResult<()> {
    let guard = sqlx::query(
        "SELECT s.is_open, a.status AS area_status, \
                CASE WHEN $2::text IS NULL THEN true ELSE ST_CoveredBy(ST_SetSRID(ST_GeomFromGeoJSON($2), 32632), ST_Buffer(a.geom, 0.01)) END AS inside \
         FROM cadastre.wf_job_instance w JOIN cadastre.wf_job_status s ON s.code = w.status \
         LEFT JOIN cadastre.job_area a ON a.wf_job_instance_id = w.id WHERE w.cadastre_job_id = $1",
    )
    .bind(job_id)
    .bind(geometry.map(|g| g.to_string()))
    .fetch_optional(&state.pool)
    .await
    .map_err(|_| AppError::BadRequest("invalid GeoJSON geometry".to_string()))?;
    if let Some(g) = guard {
        let (open, area_status, inside): (bool, Option<String>, Option<bool>) = (g.get("is_open"), g.get("area_status"), g.get("inside"));
        if !open {
            return Err(AppError::Conflict("the job is not open".into()));
        }
        match area_status.as_deref() {
            None => return Err(AppError::Conflict("create the job area before drawing parcels".into())),
            Some("closed") => return Err(AppError::Conflict("the job area is closed; reopen it to edit".into())),
            _ => {}
        }
        if inside != Some(true) {
            return Err(AppError::Conflict(crate::routes::wf::OUTSIDE_JOB_AREA.into()));
        }
    }
    Ok(())
}

/// Change a draft parcel's boundary (Edit Vertices). Only drafts; the job-area rules apply.
#[utoipa::path(put, path = "/api/v1/jobs/{id}/parcels/{parcel_id}", tag = "parcels",
    params(("id" = Uuid, Path, description = "Job id"), ("parcel_id" = Uuid, Path, description = "Draft parcel id")),
    request_body = NewParcel,
    responses((status = 200, body = ParcelDto), (status = 400, description = "Invalid geometry"), (status = 404, description = "No such draft in this job"), (status = 409, description = "Job closed, job area closed, or outside the job area")))]
pub async fn update_job_parcel(
    State(state): State<AppState>,
    user: AuthUser,
    Path((job_id, parcel_id)): Path<(Uuid, Uuid)>,
    Json(new): Json<NewParcel>,
) -> ApiResult<Json<ParcelDto>> {
    user.require_role("editor")?;
    job_area_guard(&state, job_id, Some(&new.geometry)).await?;
    let reason: Option<String> = sqlx::query_scalar("SELECT CASE WHEN ST_IsValid(g) THEN NULL ELSE ST_IsValidReason(g) END FROM (SELECT ST_GeomFromGeoJSON($1) AS g) s")
        .bind(new.geometry.to_string())
        .fetch_one(&state.pool)
        .await
        .map_err(|_| AppError::BadRequest("invalid GeoJSON geometry".to_string()))?;
    if let Some(r) = reason {
        return Err(AppError::BadRequest(format!("not a valid parcel shape: {r}")));
    }
    let sql = format!(
        "UPDATE cadastre.parcel SET geom = ST_SetSRID(ST_GeomFromGeoJSON($3), 32632), \
             stated_area = COALESCE($4, stated_area), updated_at = now() \
         WHERE id = $2 AND job_id = $1 AND status = 'draft' RETURNING {PARCEL_SELECT}"
    );
    let row = sqlx::query(&sql)
        .bind(job_id)
        .bind(parcel_id)
        .bind(new.geometry.to_string())
        .bind(new.stated_area)
        .fetch_optional(&state.pool)
        .await
        .map_err(|e| match e.as_database_error().and_then(|d| d.code()) {
            Some(code) if code == "XX000" || code.starts_with("22") => AppError::BadRequest("invalid GeoJSON geometry".to_string()),
            _ => AppError::from(e),
        })?
        .ok_or_else(|| AppError::NotFound(format!("draft parcel {parcel_id} in job {job_id}")))?;
    Ok(Json(row_to_dto(&row)?))
}

/// A line that separates a draft parcel into parts (Aumentum: Parcel Separation).
#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct SeparateRequest {
    /// GeoJSON LineString (EPSG:32632) crossing the parcel from edge to edge.
    pub line: serde_json::Value,
}

/// Separate a draft parcel along a line: the draft is replaced by its parts, each a new draft.
#[utoipa::path(post, path = "/api/v1/jobs/{id}/parcels/{parcel_id}/separate", tag = "parcels",
    params(("id" = Uuid, Path, description = "Job id"), ("parcel_id" = Uuid, Path, description = "Draft parcel id")),
    request_body = SeparateRequest,
    responses((status = 201, body = [ParcelDto]), (status = 400, description = "The line does not cross the parcel"), (status = 404, description = "No such draft in this job"), (status = 409, description = "Job or job area closed")))]
pub async fn separate_job_parcel(
    State(state): State<AppState>,
    user: AuthUser,
    Path((job_id, parcel_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<SeparateRequest>,
) -> ApiResult<(StatusCode, Json<Vec<ParcelDto>>)> {
    user.require_role("editor")?;
    job_area_guard(&state, job_id, None).await?;
    let mut tx = state.pool.begin().await?;
    let found: Option<Uuid> = sqlx::query_scalar("SELECT id FROM cadastre.parcel WHERE id = $2 AND job_id = $1 AND status = 'draft' FOR UPDATE")
        .bind(job_id)
        .bind(parcel_id)
        .fetch_optional(&mut *tx)
        .await?;
    if found.is_none() {
        return Err(AppError::NotFound(format!("draft parcel {parcel_id} in job {job_id}")));
    }
    let parts: Vec<String> = sqlx::query_scalar(
        "SELECT ST_AsGeoJSON((ST_Dump(ST_CollectionExtract(ST_Split(p.geom, ST_SetSRID(ST_GeomFromGeoJSON($2), 32632)), 3))).geom, 3) \
         FROM cadastre.parcel p WHERE p.id = $1",
    )
    .bind(parcel_id)
    .bind(req.line.to_string())
    .fetch_all(&mut *tx)
    .await
    .map_err(|e| match e.as_database_error().and_then(|d| d.code()) {
        Some(code) if code == "XX000" || code.starts_with("22") => AppError::BadRequest("the separation line is not a valid line".to_string()),
        _ => AppError::from(e),
    })?;
    if parts.is_empty() {
        return Err(AppError::NotFound(format!("draft parcel {parcel_id} in job {job_id}")));
    }
    if parts.len() < 2 {
        return Err(AppError::BadRequest("the line must cross the parcel from one edge to another".into()));
    }
    let mut out = Vec::new();
    let sql = format!(
        "INSERT INTO cadastre.parcel (job_id, geom, computed_area, status) \
         VALUES ($1, ST_SetSRID(ST_GeomFromGeoJSON($2), 32632), ST_Area(ST_SetSRID(ST_GeomFromGeoJSON($2), 32632)), 'draft') RETURNING {PARCEL_SELECT}"
    );
    for g in &parts {
        let row = sqlx::query(&sql).bind(job_id).bind(g).fetch_one(&mut *tx).await?;
        let dto = row_to_dto(&row)?;
        // A mutation's new parcel separated further: each part inherits its recorded lineage.
        sqlx::query(
            "INSERT INTO cadastre.parcel_lineage (parcel_id, status, parent_id, parent_upin, mutation_id, mutation_type, pending, wf_job_instance_id, created_by) \
             SELECT $2, status, parent_id, parent_upin, mutation_id, mutation_type, true, wf_job_instance_id, created_by \
             FROM cadastre.parcel_lineage WHERE pending AND parcel_id = $1 AND status = 1",
        )
        .bind(parcel_id)
        .bind(dto.id)
        .execute(&mut *tx)
        .await?;
        out.push(dto);
    }
    sqlx::query("DELETE FROM cadastre.parcel_lineage WHERE pending AND parcel_id = $1 AND status = 1").bind(parcel_id).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM cadastre.parcel WHERE id = $1").bind(parcel_id).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(out)))
}

/// Delete one draft parcel (posted parcels are never deleted here).
#[utoipa::path(delete, path = "/api/v1/jobs/{id}/parcels/{parcel_id}", tag = "parcels",
    params(("id" = Uuid, Path, description = "Job id"), ("parcel_id" = Uuid, Path, description = "Draft parcel id")),
    responses((status = 204), (status = 404, description = "No such draft in this job"), (status = 409, description = "Job or job area closed")))]
pub async fn delete_job_parcel(
    State(state): State<AppState>,
    user: AuthUser,
    Path((job_id, parcel_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    user.require_role("editor")?;
    job_area_guard(&state, job_id, None).await?;
    let n = sqlx::query("DELETE FROM cadastre.parcel WHERE id = $2 AND job_id = $1 AND status = 'draft'")
        .bind(job_id)
        .bind(parcel_id)
        .execute(&state.pool)
        .await?
        .rows_affected();
    if n == 0 {
        return Err(AppError::NotFound(format!("draft parcel {parcel_id} in job {job_id}")));
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Search parcels by number (full UPIN, short plot number, or any part), status, and/or
/// bounding box (FR-VIEW).
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

    // Plot-number search: the full UPIN for short forms (5621 -> PL000000005621), else anywhere.
    let number = q.number.as_deref().and_then(crate::numbering::parse_number_query);
    // The Registry property number the map labels show (e.g. PL58813), typed with or without spaces.
    let property = q.number.as_deref().map(|s| s.split_whitespace().collect::<String>().to_uppercase()).filter(|s| !s.is_empty());
    const BY_PROPERTY: &str = "parcel_number IN (SELECT pa.upin FROM cadastre.parcel_attribute pa \
         WHERE upper(replace(btrim(pa.property_number), ' ', '')) = ";
    if let Some(n) = &number {
        qb.push(" AND (parcel_number ILIKE ").push_bind(n.contains.clone());
        if let Some(exact) = &n.exact {
            qb.push(" OR parcel_number = ").push_bind(exact.clone());
        }
        if let Some(p) = &property {
            qb.push(" OR ").push(BY_PROPERTY).push_bind(p.clone()).push(")");
        }
        qb.push(")");
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
    qb.push(" ORDER BY ");
    if let (Some(p), Some(_)) = (&property, &number) {
        qb.push("(").push(BY_PROPERTY).push_bind(p.clone()).push(")) DESC, "); // the labelled parcel first
    }
    if let Some(exact) = number.as_ref().and_then(|n| n.exact.clone()) {
        qb.push("(parcel_number = ").push_bind(exact).push(") DESC, "); // exact UPIN first
    }
    qb.push("parcel_number NULLS LAST LIMIT 500");

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
