//! Import endpoints: format preview and applying an imported survey to a job.
//!
//! The heavy parsers live in `cadastre-import` (pure, dependency-free) so the same code can run in
//! the editor's WASM build. These handlers only adapt JSON/HTTP and, for `POST /jobs/{id}/import`,
//! write validated draft parcels/beacons through PostGIS.

use axum::extract::{Path, State};
use axum::Json;
use cadastre_core::area::polygon_area;
use cadastre_core::types::Point;
use cadastre_import::conversion::{
    self, ImportResult, ImportedBoundary, ImportedSurvey, MAX_IMPORT_BYTES,
};
use cadastre_import::worldfile;
use serde::Deserialize;
use serde_json::{json, Value};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::auth::AuthUser;
use crate::error::{ApiResult, AppError};
use crate::state::AppState;

/// Preview one imported file without writing anything.
#[derive(Debug, Deserialize, ToSchema)]
pub struct PreviewRequest {
    /// Original file name (selects the parser).
    pub filename: String,
    /// File text. Binary formats must be converted to ASCII first (DXF) or uploaded as text.
    pub contents: String,
    /// CRS to assume when the file does not state one.
    #[serde(default)]
    pub crs: Option<String>,
    /// Sibling `.prj` text, for world-file imports.
    #[serde(default)]
    pub prj: Option<String>,
}

/// Preview a coordinate list joined with an attribute table.
#[derive(Debug, Deserialize, ToSchema)]
pub struct AttributeImportRequest {
    /// Coordinate-list text.
    pub coordinates: String,
    /// Attribute-table text.
    pub attributes: String,
    /// Key column in the attribute table.
    #[serde(default)]
    pub key_column: Option<String>,
    /// CRS to assume when the file does not state one.
    #[serde(default)]
    pub crs: Option<String>,
}

/// Apply an imported survey to a job as draft parcels and beacons.
#[derive(Debug, Deserialize, ToSchema)]
pub struct JobImportRequest {
    /// Original file name (selects the parser).
    pub filename: String,
    /// File text.
    pub contents: String,
    /// CRS to assume when the file does not state one.
    #[serde(default)]
    pub crs: Option<String>,
    /// Whether points become beacons (default `true`).
    #[serde(default)]
    pub create_beacons: Option<bool>,
    /// Optional attribute-table text to join onto a coordinate list before import.
    #[serde(default)]
    pub attributes: Option<String>,
    /// Key column in `attributes` (defaults to the first id-like column).
    #[serde(default)]
    pub key_column: Option<String>,
}

fn import_error(e: conversion::ImportError) -> AppError {
    match e {
        conversion::ImportError::TooLarge { size, limit } => AppError::UnprocessableEntity(
            format!("file is {size} bytes; the preview limit is {limit}"),
        ),
        other => AppError::BadRequest(other.to_string()),
    }
}

fn to_json(value: ImportResult) -> ApiResult<Json<Value>> {
    serde_json::to_value(value)
        .map(Json)
        .map_err(|e| AppError::BadRequest(format!("could not serialise import: {e}")))
}

/// Detect and preview any supported survey/raster file.
#[utoipa::path(
    post, path = "/api/v1/import/preview", tag = "import",
    responses(
        (status = 200, description = "Parsed survey or raster georeference"),
        (status = 400, description = "Unrecognised or malformed file"),
    )
)]
pub async fn preview(Json(req): Json<PreviewRequest>) -> ApiResult<Json<Value>> {
    if req.contents.len() > MAX_IMPORT_BYTES {
        return Err(AppError::UnprocessableEntity(format!(
            "file is {} bytes; the preview limit is {MAX_IMPORT_BYTES}",
            req.contents.len()
        )));
    }
    let lower = req.filename.to_ascii_lowercase();
    let is_prj = lower.ends_with(".prj");
    let is_world = worldfile::is_world_file(&req.filename) && !is_prj;
    if is_prj {
        return Err(AppError::BadRequest(
            "a .prj is a projection sidecar; send it as `prj` alongside the world file".to_string(),
        ));
    }
    if is_world {
        let raster = conversion::import_world_file(&req.contents, req.prj.as_deref())
            .map_err(import_error)?;
        return to_json(ImportResult::Raster(raster));
    }
    let result = conversion::import_text(
        &req.filename,
        &req.contents,
        req.crs.as_deref(),
    )
    .map_err(import_error)?;
    to_json(result)
}

/// Preview a coordinate list with attributes joined onto it.
#[utoipa::path(
    post, path = "/api/v1/import/attributes", tag = "import",
    responses(
        (status = 200, description = "Parsed coordinates with joined attributes"),
        (status = 400, description = "Malformed coordinate or attribute text"),
    )
)]
pub async fn attributes(
    Json(req): Json<AttributeImportRequest>,
) -> ApiResult<Json<ImportedSurvey>> {
    let survey = conversion::import_coordinates_with_attributes(
        &req.coordinates,
        &req.attributes,
        req.key_column.as_deref(),
        req.crs.as_deref(),
    )
    .map_err(import_error)?;
    Ok(Json(survey))
}

fn ring_geojson(ring: &[Point]) -> Option<Value> {
    let mut coords: Vec<[f64; 2]> = ring.iter().map(|p| [p.x, p.y]).collect();
    if coords.len() < 3 {
        return None;
    }
    if coords.first() != coords.last() {
        if let Some(first) = coords.first().copied() {
            coords.push(first);
        }
    }
    // GeoJSON polygons must have at least four positions including the closing vertex.
    if coords.len() < 4 {
        return None;
    }
    Some(json!({ "type": "Polygon", "coordinates": [coords] }))
}

/// Import a file into a job: closed boundaries become draft parcels, points become beacons.
///
/// Nothing is numbered or posted here; the normal workflow (submit/review/post) still applies.
#[utoipa::path(
    post, path = "/api/v1/jobs/{id}/import", tag = "import",
    params(("id" = Uuid, Path, description = "Job id")),
    responses(
        (status = 201, description = "Imported draft parcels/beacons"),
        (status = 400, description = "Malformed file"),
        (status = 403, description = "Requires editor role"),
        (status = 404, description = "Job not found"),
    )
)]
pub async fn import_into_job(
    State(state): State<AppState>,
    user: AuthUser,
    Path(job_id): Path<Uuid>,
    Json(req): Json<JobImportRequest>,
) -> ApiResult<(axum::http::StatusCode, Json<Value>)> {
    user.require_role("editor")?;

    let exists: Option<(Uuid,)> = sqlx::query_as("SELECT id FROM cadastre.job WHERE id = $1")
        .bind(job_id)
        .fetch_optional(&state.pool)
        .await?;
    if exists.is_none() {
        return Err(AppError::NotFound(format!("job {job_id}")));
    }

    let result = match req.attributes.as_deref() {
        Some(attributes) => {
            let lower = req.filename.to_ascii_lowercase();
            if !(lower.ends_with(".csv")
                || lower.ends_with(".tsv")
                || lower.ends_with(".txt")
                || lower.ends_with(".psv"))
            {
                return Err(AppError::BadRequest(
                    "attributes can only be joined onto a delimited coordinate list".to_string(),
                ));
            }
            ImportResult::Survey(
                conversion::import_coordinates_with_attributes(
                    &req.contents,
                    attributes,
                    req.key_column.as_deref(),
                    req.crs.as_deref(),
                )
                .map_err(import_error)?,
            )
        }
        None => {
            conversion::import_text(&req.filename, &req.contents, req.crs.as_deref())
                .map_err(import_error)?
        }
    };
    let ImportResult::Survey(survey) = result else {
        return Err(AppError::BadRequest(
            "a raster world file cannot be imported as job parcels".to_string(),
        ));
    };

    let mut warnings = survey.warnings.clone();
    if let Some(crs) = survey.crs.as_deref() {
        let normalised = crs.to_ascii_uppercase();
        if !(normalised.contains("32632") || normalised.contains("UTM")) {
            warnings.push(format!(
                "file CRS `{crs}` is stored as EPSG:32632 without reprojection; reproject before import if it is not UTM 32N"
            ));
        }
    }

    // Under an Aumentum workflow job the job's rules apply: every boundary inside the open job
    // area (else nothing is imported), and beacon numbers are entered at commit, so the file's
    // point IDs are returned for the Beacon Numbers window instead of becoming beacons now.
    let workflow_job: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM cadastre.wf_job_instance WHERE cadastre_job_id = $1)")
        .bind(job_id)
        .fetch_one(&state.pool)
        .await?;
    if workflow_job {
        let mut outside = Vec::new();
        for boundary in &survey.boundaries {
            if let Some(g) = ring_geojson(&boundary.ring) {
                match crate::routes::parcels::job_area_guard(&state, job_id, Some(&g)).await {
                    Ok(()) => {}
                    Err(AppError::Conflict(m)) if m == crate::routes::wf::OUTSIDE_JOB_AREA => outside.push(boundary.id.clone()),
                    Err(e) => return Err(e),
                }
            }
        }
        if survey.boundaries.is_empty() {
            crate::routes::parcels::job_area_guard(&state, job_id, None).await?;
        }
        if !outside.is_empty() {
            return Err(AppError::Conflict(format!("{} Outside it: {}.", crate::routes::wf::OUTSIDE_JOB_AREA, outside.join(", "))));
        }
    }

    let mut tx = state.pool.begin().await?;
    let mut created_parcels = 0usize;
    let mut created_beacons = 0usize;
    let mut skipped_boundaries = Vec::new();
    let mut skipped_beacons = Vec::new();

    for boundary in &survey.boundaries {
        let ImportedBoundary { id, kind, ring, .. } = boundary;
        let Some(geometry) = ring_geojson(ring) else {
            skipped_boundaries.push(json!({
                "id": id, "kind": kind,
                "reason": "fewer than three distinct boundary vertices",
            }));
            continue;
        };
        let computed_area = polygon_area(ring).ok();
        let stated_area = boundary.stated_area;
        sqlx::query(
            "INSERT INTO cadastre.parcel (job_id, geom, stated_area, computed_area, status) \
             VALUES ($1, ST_SetSRID(ST_GeomFromGeoJSON($2), 32632), $3, $4, 'draft')",
        )
        .bind(job_id)
        .bind(geometry.to_string())
        .bind(stated_area)
        .bind(computed_area)
        .execute(&mut *tx)
        .await
        .map_err(|e| match e.as_database_error().and_then(|d| d.code()) {
            Some(code) if code == "XX000" || code.starts_with("22") => {
                AppError::BadRequest(format!("boundary `{id}` is not a valid polygon"))
            }
            _ => AppError::from(e),
        })?;
        created_parcels += 1;
    }

    let points_for_commit: Vec<Value> = if workflow_job {
        survey.points.iter().filter(|p| !p.id.trim().is_empty())
            .map(|p| json!({"id": p.id.trim(), "easting": p.point.x, "northing": p.point.y})).collect()
    } else {
        Vec::new()
    };
    if !workflow_job && req.create_beacons.unwrap_or(true) {
        for point in &survey.points {
            if point.id.trim().is_empty() {
                skipped_beacons.push(json!({"reason": "empty beacon number"}));
                continue;
            }
            let inserted: Option<(Uuid,)> = sqlx::query_as(
                "INSERT INTO cadastre.beacon (number, geom, accuracy) \
                 VALUES ($1, ST_SetSRID(ST_MakePoint($2, $3), 32632), $4) \
                 ON CONFLICT (number) DO NOTHING RETURNING id",
            )
            .bind(&point.id)
            .bind(point.point.x)
            .bind(point.point.y)
            .bind(0.05)
            .fetch_optional(&mut *tx)
            .await?;
            match inserted {
                Some(_) => created_beacons += 1,
                None => skipped_beacons.push(json!({
                    "id": point.id,
                    "reason": "a beacon with this number already exists",
                })),
            }
        }
    }

    tx.commit().await?;

    Ok((
        axum::http::StatusCode::CREATED,
        Json(json!({
            "job_id": job_id,
            "format": survey.format,
            "crs": survey.crs,
            "epsg": survey.epsg,
            "created_parcels": created_parcels,
            "created_beacons": created_beacons,
            "skipped_boundaries": skipped_boundaries,
            "skipped_beacons": skipped_beacons,
            "workflow_job": workflow_job,
            "points_for_commit": points_for_commit,
            "issues": survey.issues,
            "warnings": warnings,
        })),
    ))
}
