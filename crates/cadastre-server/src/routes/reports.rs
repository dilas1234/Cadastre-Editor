//! PDF output endpoints (FR-OUT-*): job report and site plan.

use axum::extract::{Path, State};
use axum::http::header;
use axum::response::IntoResponse;
use serde_json::Value;
use sqlx::Row;
use uuid::Uuid;

use crate::error::{ApiResult, AppError};
use crate::report::{job_report_pdf, site_plan_pdf, JobReportData, ParcelLine};
use crate::state::AppState;

fn pdf_response(filename: &str, bytes: Vec<u8>) -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "application/pdf".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("inline; filename=\"{filename}\""),
            ),
        ],
        bytes,
    )
}

/// A job report as a PDF.
#[utoipa::path(get, path = "/api/v1/jobs/{id}/report.pdf", tag = "output",
    params(("id" = Uuid, Path, description = "Job id")),
    responses((status = 200, description = "PDF report", content_type = "application/pdf"),
              (status = 404, description = "Job not found")))]
pub async fn job_report(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let job = sqlx::query("SELECT reference, status FROM cadastre.job WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("job {id}")))?;

    let rows = sqlx::query(
        "SELECT parcel_number, computed_area, status FROM cadastre.parcel \
         WHERE job_id = $1 ORDER BY parcel_number NULLS LAST",
    )
    .bind(id)
    .fetch_all(&state.pool)
    .await?;

    let data = JobReportData {
        reference: job.get("reference"),
        status: job.get("status"),
        parcels: rows
            .iter()
            .map(|r| ParcelLine {
                number: r.get("parcel_number"),
                area_m2: r.get("computed_area"),
                status: r.get("status"),
            })
            .collect(),
    };
    Ok(pdf_response("job-report.pdf", job_report_pdf(&data)))
}

/// A site plan (parcel boundaries) as a PDF.
#[utoipa::path(get, path = "/api/v1/jobs/{id}/siteplan.pdf", tag = "output",
    params(("id" = Uuid, Path, description = "Job id")),
    responses((status = 200, description = "PDF site plan", content_type = "application/pdf"),
              (status = 404, description = "Job not found")))]
pub async fn site_plan(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let job = sqlx::query("SELECT reference FROM cadastre.job WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("job {id}")))?;
    let reference: String = job.get("reference");

    let rows = sqlx::query(
        "SELECT ST_AsGeoJSON(geom) AS g FROM cadastre.parcel \
         WHERE job_id = $1 AND geom IS NOT NULL",
    )
    .bind(id)
    .fetch_all(&state.pool)
    .await?;

    let rings: Vec<Vec<(f64, f64)>> = rows
        .iter()
        .filter_map(|r| first_ring(&r.get::<String, _>("g")))
        .collect();

    let pdf = site_plan_pdf(&format!("PLAGIS Site Plan - {reference}"), &rings);
    Ok(pdf_response("site-plan.pdf", pdf))
}

/// Extract the exterior ring of a GeoJSON Polygon/MultiPolygon as (x, y) pairs.
fn first_ring(geojson: &str) -> Option<Vec<(f64, f64)>> {
    let v: Value = serde_json::from_str(geojson).ok()?;
    let coords = match v.get("type")?.as_str()? {
        "Polygon" => v.get("coordinates")?.get(0)?.clone(),
        "MultiPolygon" => v.get("coordinates")?.get(0)?.get(0)?.clone(),
        _ => return None,
    };
    let ring = coords
        .as_array()?
        .iter()
        .filter_map(|p| {
            let a = p.as_array()?;
            Some((a.first()?.as_f64()?, a.get(1)?.as_f64()?))
        })
        .collect::<Vec<_>>();
    (!ring.is_empty()).then_some(ring)
}
