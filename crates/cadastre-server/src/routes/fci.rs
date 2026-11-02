//! Create Parcel by FCI (Aumentum's "Feature Class Import" step): the polygons of a shapefile
//! become the job's draft parcels. The shapefile comes from a Catalog folder connection or is
//! uploaded from the officer's PC (.shp with its .prj; .shx/.dbf optional). Coordinates in a
//! known grid (UTM 32N, WGS 84, Minna) are converted to UTM 32N; under a workflow job every parcel
//! must lie inside the job area, else nothing is imported (Aumentum's rule and message). Beacon
//! numbers are not taken from the file: they are entered at commit.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use base64::Engine;
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::auth::AuthUser;
use crate::error::{ApiResult, AppError};
use crate::state::AppState;

/// Upper bound on the vertices read from one shapefile.
const MAX_VERTICES: usize = 400_000;
/// Upper bound on an uploaded .shp (base64-decoded).
const MAX_UPLOAD: usize = 32 * 1024 * 1024;

#[derive(Debug, Deserialize, ToSchema)]
pub struct FciRequest {
    /// Catalog folder connection id and the `.shp` path inside it, or …
    pub folder: Option<usize>,
    pub path: Option<String>,
    /// … an uploaded shapefile: base64 of the `.shp`, and optionally of its `.prj` (text).
    pub shp_base64: Option<String>,
    pub prj: Option<String>,
    /// File name, for messages.
    pub name: Option<String>,
    /// EPSG of the coordinates when the `.prj` is missing or not recognised.
    pub epsg: Option<u32>,
}

fn decode(b64: &str) -> ApiResult<Vec<u8>> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64.trim())
        .map_err(|_| AppError::BadRequest("the shapefile upload is not valid base64".into()))?;
    if bytes.len() > MAX_UPLOAD {
        return Err(AppError::UnprocessableEntity(format!("the shapefile is larger than {} MB", MAX_UPLOAD / 1024 / 1024)));
    }
    Ok(bytes)
}

/// Read the shapefile (from the Catalog or the upload) on a blocking thread.
async fn read(state: &AppState, req: &FciRequest) -> ApiResult<crate::shp::ShapeData> {
    let path = match (&req.folder, &req.path, &req.shp_base64) {
        (Some(folder), Some(path), None) => {
            if !path.to_ascii_lowercase().ends_with(".shp") {
                return Err(AppError::BadRequest("choose a .shp feature class".into()));
            }
            let conn = state.config.catalog_folders.get(*folder).ok_or_else(|| AppError::NotFound(format!("no folder connection {folder}")))?;
            let p = crate::catalog::resolve(&conn.path, path).ok_or_else(|| AppError::BadRequest("invalid catalog path".into()))?;
            return tokio::task::spawn_blocking(move || crate::shp::read_shapefile(&p, MAX_VERTICES))
                .await
                .map_err(|e| AppError::BadRequest(format!("shapefile read failed: {e}")))?
                .map_err(|e| AppError::BadRequest(format!("cannot read shapefile: {e}")));
        }
        (None, None, Some(b64)) => {
            let dir = std::env::temp_dir().join(format!("cadastre-fci-{}", Uuid::new_v4()));
            std::fs::create_dir_all(&dir).map_err(|e| AppError::BadRequest(format!("cannot stage upload: {e}")))?;
            let shp = dir.join("upload.shp");
            std::fs::write(&shp, decode(b64)?).map_err(|e| AppError::BadRequest(format!("cannot stage upload: {e}")))?;
            if let Some(prj) = &req.prj {
                std::fs::write(dir.join("upload.prj"), prj).map_err(|e| AppError::BadRequest(format!("cannot stage upload: {e}")))?;
            }
            shp
        }
        _ => return Err(AppError::BadRequest("give either a Catalog folder and path, or an uploaded .shp".into())),
    };
    let dir = path.parent().map(|d| d.to_path_buf());
    let p = path.clone();
    let out = tokio::task::spawn_blocking(move || crate::shp::read_shapefile(&p, MAX_VERTICES))
        .await
        .map_err(|e| AppError::BadRequest(format!("shapefile read failed: {e}")))?;
    if let Some(d) = dir {
        let _ = std::fs::remove_dir_all(d);
    }
    out.map_err(|e| AppError::BadRequest(format!("cannot read shapefile: {e}")))
}

/// Import a shapefile's polygons into a job as draft parcels (Create Parcel by FCI).
#[utoipa::path(post, path = "/api/v1/jobs/{id}/import/feature-class", tag = "import",
    params(("id" = Uuid, Path, description = "Job id")), request_body = FciRequest,
    responses((status = 201, description = "Draft parcels created"), (status = 400, description = "Not a readable polygon shapefile"),
              (status = 409, description = "Job or job area closed, or a parcel outside the job area"), (status = 404, description = "Job not found")))]
pub async fn import_feature_class(
    State(state): State<AppState>,
    user: AuthUser,
    Path(job_id): Path<Uuid>,
    Json(req): Json<FciRequest>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    user.require_role("editor")?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM cadastre.job WHERE id = $1)").bind(job_id).fetch_one(&state.pool).await?;
    if !exists {
        return Err(AppError::NotFound(format!("job {job_id}")));
    }
    let data = read(&state, &req).await?;
    if data.geometry_type != "polygon" {
        return Err(AppError::BadRequest(format!("this is a {} feature class; parcels come from a polygon feature class", data.geometry_type)));
    }
    if data.truncated {
        return Err(AppError::UnprocessableEntity(format!("the feature class has more than {MAX_VERTICES} vertices; import it in parts")));
    }
    let mut warnings: Vec<String> = Vec::new();
    // The source grid: the .prj, else the one given, else UTM 32N when the coordinates look like it.
    let looks_utm = data.bbox[0] > 100_000.0 && data.bbox[2] < 900_000.0 && data.bbox[1] >= 0.0 && data.bbox[3] < 9_400_000.0;
    let srid = match (data.epsg, req.epsg) {
        (Some(e), _) => e,
        (None, Some(e)) => e,
        (None, None) if looks_utm => {
            warnings.push("no recognised .prj: the coordinates are taken as WGS 84 / UTM 32N (EPSG:32632)".into());
            32632
        }
        (None, None) => return Err(AppError::BadRequest("the coordinate system is unknown (no recognised .prj); give its EPSG code".into())),
    };
    let known: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM spatial_ref_sys WHERE srid = $1)").bind(srid as i32).fetch_one(&state.pool).await?;
    if !known {
        return Err(AppError::BadRequest(format!("EPSG:{srid} is not a coordinate system the server knows")));
    }
    if srid != 32632 {
        warnings.push(format!("converted from EPSG:{srid} to WGS 84 / UTM 32N"));
    }

    // Each feature as a UTM 32N GeoJSON polygon (multi-part features stay one parcel).
    let features = data.geojson["features"].as_array().cloned().unwrap_or_default();
    let mut parcels: Vec<(i64, String)> = Vec::new();
    let mut skipped = Vec::new();
    for f in &features {
        let fid = f["properties"]["FID"].as_i64().unwrap_or(-1);
        let row = sqlx::query(
            "SELECT ST_AsGeoJSON(g, 3) AS geojson, ST_IsValid(g) AS valid, ST_Area(g) AS area FROM \
             (SELECT ST_Transform(ST_SetSRID(ST_GeomFromGeoJSON($1), $2), 32632) AS g) s",
        )
        .bind(f["geometry"].to_string())
        .bind(srid as i32)
        .fetch_one(&state.pool)
        .await;
        match row {
            Ok(r) if r.get::<bool, _>("valid") && r.get::<f64, _>("area") > 0.01 => parcels.push((fid, r.get("geojson"))),
            Ok(_) => skipped.push(json!({"fid": fid, "reason": "not a valid polygon (self-intersecting, or no area)"})),
            Err(_) => skipped.push(json!({"fid": fid, "reason": "unreadable geometry"})),
        }
    }
    if parcels.is_empty() {
        return Err(AppError::BadRequest("the feature class has no valid polygons to import".into()));
    }

    // Under a workflow job: every parcel inside the open job area, or nothing is imported.
    let mut outside = Vec::new();
    for (fid, g) in &parcels {
        let v: Value = serde_json::from_str(g).unwrap_or(Value::Null);
        match crate::routes::parcels::job_area_guard(&state, job_id, Some(&v)).await {
            Ok(()) => {}
            Err(AppError::Conflict(m)) if m == crate::routes::wf::OUTSIDE_JOB_AREA => outside.push(fid.to_string()),
            Err(e) => return Err(e),
        }
    }
    if !outside.is_empty() {
        return Err(AppError::Conflict(format!("{} Feature(s) outside it: FID {}.", crate::routes::wf::OUTSIDE_JOB_AREA, outside.join(", "))));
    }

    let mut tx = state.pool.begin().await?;
    for (_, g) in &parcels {
        sqlx::query("INSERT INTO cadastre.parcel (job_id, geom, computed_area, status) VALUES ($1, ST_SetSRID(ST_GeomFromGeoJSON($2), 32632), ST_Area(ST_SetSRID(ST_GeomFromGeoJSON($2), 32632)), 'draft')")
            .bind(job_id)
            .bind(g)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "job_id": job_id,
            "name": req.name.clone().or(req.path.clone()),
            "source_epsg": srid,
            "features": features.len(),
            "created_parcels": parcels.len(),
            "skipped": skipped,
            "warnings": warnings,
        })),
    ))
}
