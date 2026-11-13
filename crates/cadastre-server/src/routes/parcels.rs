//! Parcel endpoints: listing, capture under a job, and search.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use sqlx::{QueryBuilder, Row};
use uuid::Uuid;

use serde::Deserialize;

use crate::auth::AuthUser;
use crate::error::{ApiResult, AppError};
use crate::models::{NewParcel, ParcelAttributesInput, ParcelDto, ParcelSearch};
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
        "INSERT INTO cadastre.parcel (job_id, geom, stated_area, computed_area, status) \
         VALUES ($1, ST_SetSRID(ST_GeomFromGeoJSON($2), 32632), $3, ST_Area(ST_SetSRID(ST_GeomFromGeoJSON($2), 32632)), 'draft') \
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

    // Stash any property-document attributes against the draft; they are written into
    // cadastre.parcel_attribute, keyed by the UPIN, when the job is committed.
    if let Some(attrs) = new.attributes.clone().map(|a| a.cleaned()) {
        if attrs.has_values() {
            upsert_draft_attributes(&state.pool, row.get::<Uuid, _>("id"), &attrs).await?;
        }
    }

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
             computed_area = ST_Area(ST_SetSRID(ST_GeomFromGeoJSON($3), 32632)), \
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
        sqlx::query(
            "INSERT INTO cadastre.parcel_draft_attribute (parcel_id, property_number, property_type, plot, block, location, layout, area_name, plan_number, lga, \
                parcel_name, area_unit, surveyor_general, status, lrs_id) \
             SELECT $2, property_number, property_type, plot, block, location, layout, area_name, plan_number, lga, parcel_name, area_unit, surveyor_general, status, lrs_id \
             FROM cadastre.parcel_draft_attribute WHERE parcel_id = $1",
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

/// Write (insert or update) a draft parcel's property-document attributes. Blanks become NULL.
async fn upsert_draft_attributes<'c, E>(conn: E, parcel_id: Uuid, attrs: &ParcelAttributesInput) -> ApiResult<()>
where
    E: sqlx::Executor<'c, Database = sqlx::Postgres>,
{
    sqlx::query(
        "INSERT INTO cadastre.parcel_draft_attribute \
           (parcel_id, property_number, property_type, plot, block, location, plan_number, lga, parcel_name, area_unit, surveyor_general, status, lrs_id, layout, area_name) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15) \
         ON CONFLICT (parcel_id) DO UPDATE SET \
           property_number = EXCLUDED.property_number, property_type = EXCLUDED.property_type, \
           plot = EXCLUDED.plot, block = EXCLUDED.block, location = EXCLUDED.location, \
           plan_number = EXCLUDED.plan_number, lga = EXCLUDED.lga, parcel_name = EXCLUDED.parcel_name, \
           area_unit = EXCLUDED.area_unit, surveyor_general = EXCLUDED.surveyor_general, \
           status = EXCLUDED.status, lrs_id = EXCLUDED.lrs_id, layout = EXCLUDED.layout, area_name = EXCLUDED.area_name, updated_at = now()",
    )
    .bind(parcel_id)
    .bind(&attrs.property_number).bind(&attrs.property_type).bind(&attrs.plot).bind(&attrs.block)
    .bind(&attrs.location).bind(&attrs.plan_number).bind(&attrs.lga).bind(&attrs.parcel_name)
    .bind(attrs.area_unit).bind(&attrs.surveyor_general).bind(&attrs.status).bind(&attrs.lrs_id)
    .bind(&attrs.layout).bind(&attrs.area_name)
    .execute(conn)
    .await?;
    Ok(())
}

/// Read a draft parcel's captured property-document attributes (for the Attributes editor).
#[utoipa::path(get, path = "/api/v1/parcels/{id}/attributes", tag = "parcels",
    params(("id" = Uuid, Path, description = "Parcel id")),
    responses((status = 200, body = Object)))]
pub async fn get_parcel_attributes(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<serde_json::Value>> {
    let row = sqlx::query(
        "SELECT property_number, property_type, plot, block, location, layout, area_name, plan_number, lga, \
                parcel_name, area_unit, surveyor_general, status, lrs_id \
         FROM cadastre.parcel_draft_attribute WHERE parcel_id = $1",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await?;
    let v = match row {
        Some(r) => serde_json::json!({
            "property_number": r.get::<Option<String>, _>("property_number"),
            "property_type": r.get::<Option<String>, _>("property_type"),
            "plot": r.get::<Option<String>, _>("plot"),
            "block": r.get::<Option<String>, _>("block"),
            "location": r.get::<Option<String>, _>("location"),
            "layout": r.get::<Option<String>, _>("layout"),
            "area_name": r.get::<Option<String>, _>("area_name"),
            "plan_number": r.get::<Option<String>, _>("plan_number"),
            "lga": r.get::<Option<String>, _>("lga"),
            "parcel_name": r.get::<Option<String>, _>("parcel_name"),
            "area_unit": r.get::<Option<i16>, _>("area_unit"),
            "surveyor_general": r.get::<Option<String>, _>("surveyor_general"),
            "status": r.get::<Option<String>, _>("status"),
            "lrs_id": r.get::<Option<String>, _>("lrs_id"),
        }),
        None => serde_json::json!({}),
    };
    Ok(Json(v))
}

/// Edit a draft parcel's captured attributes (Aumentum "Update Parcel Attribute"). Only drafts can
/// be edited; a committed parcel's attributes are keyed by its UPIN.
#[utoipa::path(put, path = "/api/v1/parcels/{id}/attributes", tag = "parcels",
    params(("id" = Uuid, Path, description = "Parcel id")),
    request_body = ParcelAttributesInput,
    responses((status = 200, body = ParcelDto), (status = 409, description = "Parcel is not a draft")))]
pub async fn put_parcel_attributes(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
    Json(attrs): Json<ParcelAttributesInput>,
) -> ApiResult<Json<ParcelDto>> {
    user.require_role("editor")?;
    let status: Option<String> = sqlx::query_scalar("SELECT status FROM cadastre.parcel WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.pool)
        .await?;
    match status.as_deref() {
        None => return Err(AppError::NotFound(format!("parcel {id}"))),
        Some("draft") => {}
        Some(_) => return Err(AppError::Conflict("only draft parcels can be edited; this parcel is already posted".into())),
    }
    upsert_draft_attributes(&state.pool, id, &attrs.cleaned()).await?;
    let sql = format!("SELECT {PARCEL_SELECT} FROM cadastre.parcel WHERE id = $1");
    let row = sqlx::query(&sql).bind(id).fetch_one(&state.pool).await?;
    Ok(Json(row_to_dto(&row)?))
}

/// A cut line for the Cut Polygons tool (GeoJSON LineString in the server CRS, UTM 32N).
#[derive(Deserialize, utoipa::ToSchema)]
pub struct CutBody {
    /// Cut line as GeoJSON LineString (crosses the parcel). In the server CRS (EPSG:32632).
    #[schema(value_type = Object)]
    pub line: serde_json::Value,
}

/// Cut a draft parcel along a drawn line (Aumentum "Cut Polygons Tool"). The original draft is
/// replaced by the pieces, each keeping the original's captured attributes. The parcel must be a
/// draft under the given job, and the line must actually split it into two or more parts.
#[utoipa::path(post, path = "/api/v1/jobs/{id}/parcels/{pid}/cut", tag = "parcels",
    params(("id" = Uuid, Path, description = "Job id"), ("pid" = Uuid, Path, description = "Parcel id")),
    request_body = CutBody,
    responses((status = 200, body = [ParcelDto]), (status = 409, description = "Not a draft / line does not cut it")))]
pub async fn cut_job_parcel(
    State(state): State<AppState>,
    user: AuthUser,
    Path((job_id, parcel_id)): Path<(Uuid, Uuid)>,
    Json(body): Json<CutBody>,
) -> ApiResult<Json<Vec<ParcelDto>>> {
    user.require_role("editor")?;
    job_area_guard(&state, job_id, None).await?;
    let mut tx = state.pool.begin().await?;

    // The parcel must be a draft under this job (committed parcels are part of the fabric).
    let src: Option<String> = sqlx::query_scalar(
        "SELECT status FROM cadastre.parcel WHERE id = $1 AND job_id = $2",
    )
    .bind(parcel_id)
    .bind(job_id)
    .fetch_optional(&mut *tx)
    .await?;
    match src.as_deref() {
        None => return Err(AppError::NotFound(format!("draft parcel {parcel_id} under job {job_id}"))),
        Some("draft") => {}
        Some(_) => return Err(AppError::Conflict("only draft parcels can be cut".into())),
    }

    // Split the parcel by the line; keep only polygonal pieces with a real area.
    let pieces: Vec<String> = sqlx::query_scalar(
        "SELECT ST_AsGeoJSON(g) FROM ( \
           SELECT (ST_Dump(ST_CollectionExtract( \
             ST_Split(p.geom, ST_SetSRID(ST_GeomFromGeoJSON($2), 32632)), 3))).geom AS g \
           FROM cadastre.parcel p WHERE p.id = $1 \
         ) parts WHERE ST_Area(g) > 0.01",
    )
    .bind(parcel_id)
    .bind(body.line.to_string())
    .fetch_all(&mut *tx)
    .await
    .map_err(|_| AppError::BadRequest("invalid cut line geometry".to_string()))?;
    if pieces.len() < 2 {
        return Err(AppError::Conflict("the cut line must cross the parcel so it splits into two or more parts".into()));
    }

    // Carry the original draft's captured attributes onto each piece.
    let attrs: Option<ParcelAttributesInput> = sqlx::query(
        "SELECT property_number, property_type, plot, block, location, layout, area_name, plan_number, lga, \
                parcel_name, area_unit, surveyor_general, status, lrs_id \
         FROM cadastre.parcel_draft_attribute WHERE parcel_id = $1",
    )
    .bind(parcel_id)
    .fetch_optional(&mut *tx)
    .await?
    .map(|r| ParcelAttributesInput {
        property_number: r.get("property_number"), property_type: r.get("property_type"),
        plot: r.get("plot"), block: r.get("block"), location: r.get("location"), layout: r.get("layout"), area_name: r.get("area_name"),
        plan_number: r.get("plan_number"), lga: r.get("lga"), parcel_name: r.get("parcel_name"),
        area_unit: r.get("area_unit"), surveyor_general: r.get("surveyor_general"),
        status: r.get("status"), lrs_id: r.get("lrs_id"),
    });

    // Replace the original draft with its pieces (its pending mutation lineage moves to them).
    let lineage: Vec<sqlx::postgres::PgRow> = sqlx::query(
        "SELECT status, parent_id, parent_upin, mutation_id, mutation_type, wf_job_instance_id, created_by \
         FROM cadastre.parcel_lineage WHERE pending AND parcel_id = $1 AND status = 1",
    )
    .bind(parcel_id)
    .fetch_all(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM cadastre.parcel_lineage WHERE pending AND parcel_id = $1 AND status = 1").bind(parcel_id).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM cadastre.parcel WHERE id = $1").bind(parcel_id).execute(&mut *tx).await?;
    let insert = format!(
        "INSERT INTO cadastre.parcel (job_id, geom, computed_area, status) \
         VALUES ($1, ST_SetSRID(ST_GeomFromGeoJSON($2), 32632), ST_Area(ST_SetSRID(ST_GeomFromGeoJSON($2), 32632)), 'draft') RETURNING {PARCEL_SELECT}"
    );
    let mut out = Vec::with_capacity(pieces.len());
    for piece in &pieces {
        let row = sqlx::query(&insert).bind(job_id).bind(piece).fetch_one(&mut *tx).await?;
        let new_id: Uuid = row.get("id");
        for l in &lineage {
            sqlx::query(
                "INSERT INTO cadastre.parcel_lineage (parcel_id, status, parent_id, parent_upin, mutation_id, mutation_type, pending, wf_job_instance_id, created_by) \
                 VALUES ($1, $2, $3, $4, $5, $6, true, $7, $8)",
            )
            .bind(new_id)
            .bind(l.get::<i16, _>("status")).bind(l.get::<Option<Uuid>, _>("parent_id")).bind(l.get::<Option<String>, _>("parent_upin"))
            .bind(l.get::<Option<String>, _>("mutation_id")).bind(l.get::<Option<i16>, _>("mutation_type"))
            .bind(l.get::<Option<i32>, _>("wf_job_instance_id")).bind(l.get::<Option<String>, _>("created_by"))
            .execute(&mut *tx)
            .await?;
        }
        if let Some(a) = attrs.as_ref() {
            if a.has_values() {
                upsert_draft_attributes(&mut *tx, new_id, a).await?;
            }
        }
        out.push(row_to_dto(&row)?);
    }
    tx.commit().await?;
    Ok(Json(out))
}

/// Search parcels by number — the UPIN (full or short plot number), **or** the property-document
/// fields a surveyor knows the parcel by: property number (e.g. `PL68755`), plot, plan number or
/// parcel name — plus status and/or bounding box (FR-VIEW). Property fields come from
/// `parcel_attribute` for posted parcels and `parcel_draft_attribute` for drafts.
#[utoipa::path(get, path = "/api/v1/parcels/search", tag = "parcels",
    params(ParcelSearch),
    responses((status = 200, body = [ParcelDto]), (status = 400, description = "Invalid bbox")))]
pub async fn search_parcels(
    State(state): State<AppState>,
    Query(q): Query<ParcelSearch>,
) -> ApiResult<Json<Vec<ParcelDto>>> {
    let mut qb: QueryBuilder<sqlx::Postgres> = QueryBuilder::new(
        "SELECT p.id, p.parcel_number, p.job_id, ST_AsGeoJSON(p.geom) AS geometry, \
                p.stated_area, p.computed_area, p.status, p.created_at, p.updated_at \
         FROM cadastre.parcel p WHERE 1=1",
    );

    // Match the UPIN (short forms expand to the full UPIN: 5621 -> PL000000005621) and, as a plain
    // substring, the property-document fields the parcel is known by.
    let number = q.number.as_deref().and_then(crate::numbering::parse_number_query);
    // The Registry property number the map labels show (e.g. PL58813), typed with or without spaces.
    let property = q.number.as_deref().map(|s| s.split_whitespace().collect::<String>().to_uppercase()).filter(|s| !s.is_empty());
    const BY_PROPERTY: &str = "p.parcel_number IN (SELECT pa.upin FROM cadastre.parcel_attribute pa \
         WHERE upper(replace(btrim(pa.property_number), ' ', '')) = ";
    let raw = q.number.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let raw_like = raw.map(|s| format!("%{}%", s.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")));
    if raw.is_some() {
        qb.push(" AND (false");
        if let Some(n) = &number {
            qb.push(" OR p.parcel_number ILIKE ").push_bind(n.contains.clone());
            if let Some(exact) = &n.exact {
                qb.push(" OR p.parcel_number = ").push_bind(exact.clone());
            }
        }
        if let Some(rl) = &raw_like {
            // Property-document fields: the parcel's attribute records (posted) or its draft capture.
            qb.push(" OR EXISTS (SELECT 1 FROM cadastre.parcel_attribute pa WHERE pa.upin = p.parcel_number AND (false");
            for col in ["pa.property_number", "pa.plot", "pa.plan_number", "pa.parcel_name"] {
                qb.push(format!(" OR {col} ILIKE ")).push_bind(rl.clone());
            }
            qb.push(")) OR EXISTS (SELECT 1 FROM cadastre.parcel_draft_attribute pda WHERE pda.parcel_id = p.id AND (false");
            for col in ["pda.property_number", "pda.plot"] {
                qb.push(format!(" OR {col} ILIKE ")).push_bind(rl.clone());
            }
            qb.push("))");
        }
        if let Some(p) = &property {
            qb.push(" OR ").push(BY_PROPERTY).push_bind(p.clone()).push(")");
        }
        qb.push(")");
    }
    if let Some(status) = q.status.as_deref().filter(|s| !s.is_empty()) {
        qb.push(" AND p.status = ").push_bind(status.to_string());
    } else {
        // A shape a correction replaced (same UPIN) only when asked for.
        qb.push(" AND p.status <> 'superseded'");
    }
    if let Some(bbox) = q.bbox.as_deref().filter(|s| !s.is_empty()) {
        let (minx, miny, maxx, maxy) = parse_bbox(bbox)?;
        qb.push(" AND p.geom && ST_MakeEnvelope(")
            .push_bind(minx).push(", ").push_bind(miny).push(", ")
            .push_bind(maxx).push(", ").push_bind(maxy).push(", 32632)");
    }
    qb.push(" ORDER BY ");
    if let (Some(p), Some(_)) = (&property, &number) {
        qb.push("(").push(BY_PROPERTY).push_bind(p.clone()).push(")) DESC, "); // the labelled parcel first
    }
    if let Some(exact) = number.as_ref().and_then(|n| n.exact.clone()) {
        qb.push("(p.parcel_number = ").push_bind(exact).push(") DESC, "); // exact UPIN first
    }
    qb.push("p.parcel_number NULLS LAST LIMIT 500");

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

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct ParcelMetadataQuery {
    /// UPINs, comma-separated (at most 200).
    #[serde(default)]
    pub upins: String,
    /// Workflow job instances, comma-separated: their committed parcels are added.
    #[serde(default)]
    pub jobs: String,
}

/// When and by whom each parcel was committed, its job, and when and to which Registry property it
/// was linked: the metadata Find, Identify and the Registry file search show with a parcel.
/// Parcels brought over from Aumentum carry Aumentum's creation date (`committed_in` = "aumentum").
#[utoipa::path(get, path = "/api/v1/parcels/metadata", tag = "parcels", params(ParcelMetadataQuery), responses((status = 200)))]
pub async fn parcel_metadata(State(state): State<AppState>, Query(q): Query<ParcelMetadataQuery>) -> ApiResult<Json<Vec<serde_json::Value>>> {
    let mut upins: Vec<String> = q.upins.split(',').map(|u| u.trim().to_uppercase()).filter(|u| !u.is_empty()).take(200).collect();
    let jobs: Vec<i32> = q.jobs.split(',').filter_map(|j| j.trim().parse().ok()).take(50).collect();
    if !jobs.is_empty() {
        let more: Vec<String> = sqlx::query_scalar(
            "SELECT p.parcel_number FROM cadastre.wf_job_instance w JOIN cadastre.parcel p ON p.job_id = w.cadastre_job_id \
             WHERE w.id = ANY($1) AND p.status = 'posted' AND p.parcel_number IS NOT NULL ORDER BY p.posted_at")
            .bind(&jobs).fetch_all(&state.pool).await?;
        for u in more { if !upins.contains(&u) && upins.len() < 200 { upins.push(u); } }
    }
    if upins.is_empty() {
        return Ok(Json(Vec::new()));
    }
    let rows = sqlx::query(
        "SELECT u.upin, p.status, p.posted_at, pa.creation_date, \
                (SELECT a.actor FROM cadastre.audit_log a WHERE a.action = 'wf_commit' AND a.payload->'parcel_numbers' ? u.upin ORDER BY a.id DESC LIMIT 1) AS committed_by, \
                w.id AS job_instance_id, w.name AS job_name, \
                pa.linkage_status, pa.linkage_date, pa.lrs_id, pa.property_number, \
                (SELECT COALESCE(l.payload->>'linked_at', to_char(l.created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')) FROM cadastre.audit_log l \
                  WHERE l.entity = 'registry_link' AND l.entity_id = u.upin AND l.action IN ('registry_link_synced', 'registry_link_made', 'registry_map_update') \
                  ORDER BY l.id DESC LIMIT 1) AS link_event \
         FROM unnest($1::text[]) AS u(upin) \
         LEFT JOIN LATERAL (SELECT * FROM cadastre.parcel x WHERE x.parcel_number = u.upin ORDER BY (x.status = 'posted') DESC, x.updated_at DESC LIMIT 1) p ON true \
         LEFT JOIN LATERAL (SELECT id, name FROM cadastre.wf_job_instance wj WHERE p.job_id IS NOT NULL AND wj.cadastre_job_id = p.job_id ORDER BY wj.id DESC LIMIT 1) w ON true \
         LEFT JOIN LATERAL (SELECT * FROM cadastre.parcel_attribute y WHERE y.upin = u.upin ORDER BY (y.status = '0') DESC NULLS LAST, y.object_id DESC LIMIT 1) pa ON true",
    )
    .bind(&upins)
    .fetch_all(&state.pool)
    .await?;
    let iso = |t: Option<chrono::DateTime<chrono::Utc>>| t.map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
    Ok(Json(rows.iter().map(|r| {
        let posted: Option<chrono::DateTime<chrono::Utc>> = r.get("posted_at");
        let created: Option<chrono::NaiveDateTime> = r.get("creation_date");
        let linked = r.get::<Option<i16>, _>("linkage_status").map(i32::from) == Some(1);
        let link_event: Option<String> = r.get("link_event");
        let linkage_date: Option<String> = r.get("linkage_date");
        serde_json::json!({
            "upin": r.get::<String, _>("upin"),
            "status": r.get::<Option<String>, _>("status"),
            "committed_at": iso(posted).or_else(|| created.map(|c| c.format("%Y-%m-%dT%H:%M:%S").to_string())),
            "committed_in": if posted.is_some() && r.get::<Option<String>, _>("committed_by").is_some() { "cadastre" } else if posted.is_some() || created.is_some() { "aumentum" } else { "" },
            "committed_by": r.get::<Option<String>, _>("committed_by"),
            "job_instance_id": r.get::<Option<i32>, _>("job_instance_id"),
            "job_name": r.get::<Option<String>, _>("job_name"),
            "linked": linked,
            "linked_at": if linked { link_event.or(linkage_date) } else { None },
            "lrs_id": r.get::<Option<String>, _>("lrs_id"),
            "property_number": r.get::<Option<String>, _>("property_number"),
        })
    }).collect()))
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct SketchOverlapRequest {
    /// The sketch (UTM 32N metres, ring order, no closing repeat).
    pub vertices: Vec<[f64; 2]>,
    /// A draft being reshaped (not checked against itself).
    #[serde(default)]
    pub editing_id: Option<Uuid>,
}

/// For the Orthophoto window, as the digitiser sketches: the committed parcels the sketch
/// overlaps by more than the overlap width (`editor.overlap_min_width_m`, 5 cm) (with the overlapping area as GeoJSON in WGS 84, to shade red), whether the plot's
/// position is already digitised, and whether a title exists on the land (a parcel there linked
/// to a Registry property). This Cadastre's fabric only, so it answers at once; the System Check
/// adds Aumentum's live parcels and the Registry before anything is saved.
#[utoipa::path(post, path = "/api/v1/parcels/overlaps", tag = "parcels", request_body = SketchOverlapRequest, responses((status = 200)))]
pub async fn sketch_overlaps(State(state): State<AppState>, Json(req): Json<SketchOverlapRequest>) -> ApiResult<Json<serde_json::Value>> {
    if req.vertices.len() < 3 {
        return Ok(Json(serde_json::json!({ "free": true, "overlaps": [], "at_point": [], "titled": [] })));
    }
    if req.vertices.len() > 2000 || req.vertices.iter().flatten().any(|v| !v.is_finite()) {
        return Err(AppError::BadRequest("vertices must be numbers (UTM 32N metres)".into()));
    }
    let mut ring: Vec<[f64; 2]> = req.vertices.clone();
    ring.push(req.vertices[0]);
    let shape = serde_json::json!({ "type": "Polygon", "coordinates": [ring] }).to_string();
    let width = crate::routes::precheck::overlap_min_width(&mut *state.pool.acquire().await?).await?;
    let rows = sqlx::query(&format!(
        "WITH s AS (SELECT ST_MakeValid(ST_SetSRID(ST_GeomFromGeoJSON($1), 32632)) AS g), \
              pt AS (SELECT ST_PointOnSurface(g) AS p FROM s) \
         SELECT p.parcel_number, ST_Area(ST_Intersection(p.geom, s.g)) AS a, {} AS counts, \
                ST_AsGeoJSON(ST_Transform(ST_Intersection(p.geom, s.g), 4326), 7)::json AS geom, \
                ST_Contains(p.geom, (SELECT p FROM pt)) AS at_point, \
                COALESCE(r.su_id, x.lrs_id) AS su_id, COALESCE(r.refnum, x.property_number) AS refnum, \
                (r.su_id IS NOT NULL OR x.lrs_id IS NOT NULL) AS titled \
         FROM s JOIN cadastre.parcel p ON p.status = 'posted' AND p.geom && s.g AND ST_Intersects(p.geom, s.g) \
         LEFT JOIN cadastre.registry_property r ON r.upin = p.parcel_number \
         LEFT JOIN LATERAL (SELECT lrs_id, property_number FROM cadastre.parcel_attribute y WHERE y.upin = p.parcel_number AND y.linkage_status = 1 \
                            AND (y.status = '0' OR y.status IS NULL) LIMIT 1) x ON true \
         WHERE ($2::uuid IS NULL OR (p.id <> $2 AND p.parcel_number IS DISTINCT FROM (SELECT e.parcel_number FROM cadastre.parcel e WHERE e.id = $2))) \
         ORDER BY a DESC LIMIT 30",
        crate::routes::precheck::counts_sql("p.geom", "s.g", "$3"),
    ))
    .bind(&shape)
    .bind(req.editing_id)
    .bind(width)
    .fetch_all(&state.pool)
    .await?;
    let mut overlaps = Vec::new();
    let (mut at_point, mut titled) = (Vec::new(), Vec::new());
    for r in &rows {
        let upin: Option<String> = r.get("parcel_number");
        let a: f64 = r.get::<Option<f64>, _>("a").unwrap_or(0.0);
        if r.get::<Option<bool>, _>("at_point").unwrap_or(false) { at_point.push(upin.clone()); }
        // A strip narrower than the overlap width is a shared boundary drawn a little differently.
        if !r.get::<Option<bool>, _>("counts").unwrap_or(false) { continue; }
        if r.get::<Option<bool>, _>("titled").unwrap_or(false) {
            titled.push(serde_json::json!({ "upin": upin, "su_id": r.get::<Option<String>, _>("su_id"), "refnum": r.get::<Option<String>, _>("refnum") }));
        }
        overlaps.push(serde_json::json!({ "upin": upin, "area_m2": (a * 100.0).floor() / 100.0, "geometry": r.get::<Option<serde_json::Value>, _>("geom") }));
    }
    Ok(Json(serde_json::json!({ "free": overlaps.is_empty() && at_point.is_empty(), "overlaps": overlaps, "at_point": at_point, "titled": titled })))
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct ParcelPointsQuery {
    /// The committed parcel's UPIN.
    pub upin: String,
}

/// A committed parcel's point list (right-click → Point List / Show Beacons): each corner's beacon
/// (or P-number), coordinates, and the bearing and distance to the next corner, with the area; and
/// whether it can be corrected here (Correct: the file's job reopened with a copy under the same
/// UPIN, linked or not) or, if not, why. A parcel archived in Aumentum with no reason and nothing
/// committed in its place is listed too, and can be brought back by Correct.
#[utoipa::path(get, path = "/api/v1/parcels/points", tag = "parcels", params(ParcelPointsQuery), responses((status = 200), (status = 404)))]
pub async fn parcel_points(State(state): State<AppState>, Query(q): Query<ParcelPointsQuery>) -> ApiResult<Json<serde_json::Value>> {
    let upin = q.upin.trim().to_uppercase();
    let row = sqlx::query(
        "SELECT p.id, p.job_id, p.posted_at, trunc(ST_Area(p.geom)::numeric, 2)::float8 AS area, \
                (SELECT w.id FROM cadastre.wf_job_instance w WHERE p.job_id IS NOT NULL AND w.cadastre_job_id = p.job_id ORDER BY w.id DESC LIMIT 1) AS job_instance_id, \
                (SELECT w.name FROM cadastre.wf_job_instance w WHERE p.job_id IS NOT NULL AND w.cadastre_job_id = p.job_id ORDER BY w.id DESC LIMIT 1) AS job_name, \
                EXISTS (SELECT 1 FROM cadastre.parcel_attribute a WHERE a.upin = p.parcel_number AND (a.linkage_status = 1 OR a.lrs_id IS NOT NULL)) \
                  OR EXISTS (SELECT 1 FROM cadastre.registry_property r WHERE r.upin = p.parcel_number) AS linked, \
                EXISTS (SELECT 1 FROM cadastre.parcel_lineage l WHERE l.parcel_id = p.id OR l.parent_id = p.id OR l.upin = p.parcel_number OR l.parent_upin = p.parcel_number) AS lineage, \
                p.status = 'archived' AS archived, \
                (SELECT NULLIF(trim(a.archive_reason), '') FROM cadastre.parcel_attribute a WHERE a.upin = p.parcel_number \
                   ORDER BY (a.status = '0' OR a.status IS NULL) DESC, a.object_id DESC LIMIT 1) AS archive_reason, \
                (SELECT w.name FROM cadastre.parcel d JOIN cadastre.wf_job_instance w ON w.cadastre_job_id = d.job_id \
                   WHERE d.parcel_number = p.parcel_number AND d.status = 'draft' ORDER BY w.id DESC LIMIT 1) AS correcting_in \
         FROM cadastre.parcel p WHERE p.parcel_number = $1 AND p.status IN ('posted', 'archived') ORDER BY (p.status = 'posted') DESC, p.updated_at DESC LIMIT 1",
    )
    .bind(&upin)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("{upin} has no committed or archived shape in this Cadastre")))?;
    let legs = crate::report::parcel_legs(&state.pool, &upin).await?;
    let job: Option<i32> = row.get("job_instance_id");
    let (linked, lineage): (bool, bool) = (row.get::<Option<bool>, _>("linked").unwrap_or(false), row.get::<Option<bool>, _>("lineage").unwrap_or(false));
    // Linked or not, Correct keeps the UPIN, so the Registry link stays.
    let archived = row.get::<Option<bool>, _>("archived").unwrap_or(false);
    let archive_reason: Option<String> = row.get("archive_reason");
    let why_not: Option<String> = if lineage { Some("it has split / merge history: correct it with a mutation job".into()) }
        else if archived && archive_reason.is_some() { Some(format!("it was archived on purpose ({})", archive_reason.as_deref().unwrap_or_default())) }
        else { crate::routes::correction::why_not_current(&state.pool, &upin).await? };
    Ok(Json(serde_json::json!({
        "upin": upin,
        "area_m2": row.get::<Option<f64>, _>("area"),
        "committed_at": row.get::<Option<chrono::DateTime<chrono::Utc>>, _>("posted_at").map(|t| t.to_rfc3339()),
        "job_instance_id": job,
        "job_name": row.get::<Option<String>, _>("job_name"),
        "can_correct": why_not.is_none(),
        "linked": linked,
        "committed_in": if job.is_none() { "aumentum" } else { "cadastre" },
        "why_not": why_not,
        "archived": archived,
        "correcting_in": row.get::<Option<String>, _>("correcting_in"),
        "points": legs.iter().map(|l| serde_json::json!({
            "beacon": l.from, "easting": (l.from_e * 1000.0).round() / 1000.0, "northing": (l.from_n * 1000.0).round() / 1000.0,
            "to": l.to, "distance": (l.distance * 1000.0).round() / 1000.0, "bearing": l.bearing,
        })).collect::<Vec<_>>(),
    })))
}
