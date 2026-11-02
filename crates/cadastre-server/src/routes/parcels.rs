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

    // Under an Aumentum job, edits follow its job area: the job must be open and have an open job
    // area, and the parcel must lie inside it (Aumentum's rule and message).
    let guard = sqlx::query(
        "SELECT s.is_open, a.status AS area_status, \
                ST_CoveredBy(ST_SetSRID(ST_GeomFromGeoJSON($2), 32632), ST_Buffer(a.geom, 0.01)) AS inside \
         FROM cadastre.wf_job_instance w JOIN cadastre.wf_job_status s ON s.code = w.status \
         LEFT JOIN cadastre.job_area a ON a.wf_job_instance_id = w.id WHERE w.cadastre_job_id = $1",
    )
    .bind(job_id)
    .bind(new.geometry.to_string())
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

    // Stash any property-document attributes against the draft; they are written into
    // cadastre.parcel_attribute, keyed by the UPIN, when the job is committed.
    if let Some(attrs) = new.attributes.clone().map(|a| a.cleaned()) {
        if attrs.has_values() {
            upsert_draft_attributes(&state.pool, row.get::<Uuid, _>("id"), &attrs).await?;
        }
    }

    Ok((StatusCode::CREATED, Json(row_to_dto(&row)?)))
}

/// Write (insert or update) a draft parcel's property-document attributes. Blanks become NULL.
async fn upsert_draft_attributes<'c, E>(conn: E, parcel_id: Uuid, attrs: &ParcelAttributesInput) -> ApiResult<()>
where
    E: sqlx::Executor<'c, Database = sqlx::Postgres>,
{
    sqlx::query(
        "INSERT INTO cadastre.parcel_draft_attribute \
           (parcel_id, property_number, property_type, plot, block, location, plan_number, lga, parcel_name, area_unit, surveyor_general, status, lrs_id) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13) \
         ON CONFLICT (parcel_id) DO UPDATE SET \
           property_number = EXCLUDED.property_number, property_type = EXCLUDED.property_type, \
           plot = EXCLUDED.plot, block = EXCLUDED.block, location = EXCLUDED.location, \
           plan_number = EXCLUDED.plan_number, lga = EXCLUDED.lga, parcel_name = EXCLUDED.parcel_name, \
           area_unit = EXCLUDED.area_unit, surveyor_general = EXCLUDED.surveyor_general, \
           status = EXCLUDED.status, lrs_id = EXCLUDED.lrs_id, updated_at = now()",
    )
    .bind(parcel_id)
    .bind(&attrs.property_number).bind(&attrs.property_type).bind(&attrs.plot).bind(&attrs.block)
    .bind(&attrs.location).bind(&attrs.plan_number).bind(&attrs.lga).bind(&attrs.parcel_name)
    .bind(attrs.area_unit).bind(&attrs.surveyor_general).bind(&attrs.status).bind(&attrs.lrs_id)
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
        "SELECT property_number, property_type, plot, block, location, plan_number, lga, \
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
        "SELECT property_number, property_type, plot, block, location, plan_number, lga, \
                parcel_name, area_unit, surveyor_general, status, lrs_id \
         FROM cadastre.parcel_draft_attribute WHERE parcel_id = $1",
    )
    .bind(parcel_id)
    .fetch_optional(&mut *tx)
    .await?
    .map(|r| ParcelAttributesInput {
        property_number: r.get("property_number"), property_type: r.get("property_type"),
        plot: r.get("plot"), block: r.get("block"), location: r.get("location"),
        plan_number: r.get("plan_number"), lga: r.get("lga"), parcel_name: r.get("parcel_name"),
        area_unit: r.get("area_unit"), surveyor_general: r.get("surveyor_general"),
        status: r.get("status"), lrs_id: r.get("lrs_id"),
    });

    // Replace the original draft with its pieces.
    sqlx::query("DELETE FROM cadastre.parcel WHERE id = $1").bind(parcel_id).execute(&mut *tx).await?;
    let insert = format!(
        "INSERT INTO cadastre.parcel (job_id, geom, status) \
         VALUES ($1, ST_SetSRID(ST_GeomFromGeoJSON($2), 32632), 'draft') RETURNING {PARCEL_SELECT}"
    );
    let mut out = Vec::with_capacity(pieces.len());
    for piece in &pieces {
        let row = sqlx::query(&insert).bind(job_id).bind(piece).fetch_one(&mut *tx).await?;
        let new_id: Uuid = row.get("id");
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
         FROM cadastre.parcel p \
         LEFT JOIN cadastre.parcel_attribute pa ON pa.upin = p.parcel_number \
         LEFT JOIN cadastre.parcel_draft_attribute pda ON pda.parcel_id = p.id WHERE 1=1",
    );

    // Match the UPIN (short forms expand to the full UPIN: 5621 -> PL000000005621) and, as a plain
    // substring, the property-document fields the parcel is known by.
    let number = q.number.as_deref().and_then(crate::numbering::parse_number_query);
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
            for col in ["pa.property_number", "pa.plot", "pa.plan_number", "pa.parcel_name", "pda.property_number", "pda.plot"] {
                qb.push(format!(" OR {col} ILIKE ")).push_bind(rl.clone());
            }
        }
        qb.push(")");
    }
    if let Some(status) = q.status.as_deref().filter(|s| !s.is_empty()) {
        qb.push(" AND p.status = ").push_bind(status.to_string());
    }
    if let Some(bbox) = q.bbox.as_deref().filter(|s| !s.is_empty()) {
        let (minx, miny, maxx, maxy) = parse_bbox(bbox)?;
        qb.push(" AND p.geom && ST_MakeEnvelope(")
            .push_bind(minx).push(", ").push_bind(miny).push(", ")
            .push_bind(maxx).push(", ").push_bind(maxy).push(", 32632)");
    }
    qb.push(" ORDER BY ");
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
