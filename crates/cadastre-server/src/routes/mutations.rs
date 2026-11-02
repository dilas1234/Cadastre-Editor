//! Phase 4: parcel mutations and lineage (Aumentum parity, from its dbo.parcelLineage history).
//!
//! In a workflow job an officer records a mutation of posted parcels:
//! - split (Aumentum type 1): one parcel, cut by a line into two or more new parcels;
//! - merge (type 2): two or more parcels sharing boundaries become one;
//! - re-survey (type 3): a parcel is replaced by a re-drawn copy (edited before commit);
//! - retire: a parcel is archived with a reason, and nothing replaces it.
//!
//! The new parcels are the job's drafts; nothing changes in the fabric until the job's parcels
//! are committed. Then the parents become `historic` (a retirement: `archived`), the children get
//! new UPINs, and lineage rows are written as Aumentum writes them: a status-0 row per parent and
//! a status-1 row per child and parent, sharing a mutation id (yyyyMMddHHmm). UPINs are never
//! reused; a parent's UPIN stays with its retired shape.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::Row;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::audit;
use crate::auth::AuthUser;
use crate::error::{ApiResult, AppError};
use crate::routes::wf::{lock_open_job, resolve_actor};
use crate::state::AppState;

/// Aumentum mutation type names (dbo.parcelLineage.mutationTypeID), as Aumentum's own domain
/// `d_mutation_type` names them (ParcelAttributeTableDefinition.xml): 0 None, 1 Split, 2 Merged,
/// 3 Adjusted, 4 Free Mutation, 5 COGO, 6 Separated, 7 Conflation.
pub fn mutation_type_name(code: Option<i16>) -> &'static str {
    match code {
        Some(1) => "Split",
        Some(2) => "Merge",
        Some(3) => "Adjusted (re-survey)",
        Some(4) => "Free Mutation",
        Some(5) => "COGO",
        Some(6) => "Separated",
        Some(7) => "Conflation",
        Some(_) => "Other",
        None => "Retire",
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct MutationRequest {
    /// `split` (1), `merge` (2), `resurvey` (3, Aumentum "Adjusted"), `free` (4, Free Mutation),
    /// `separate` (6, Separated) or `retire`.
    pub kind: String,
    /// The posted parcels (UPINs) the mutation applies to.
    pub upins: Vec<String>,
    /// For a split or a separation: a GeoJSON LineString (EPSG:32632) across the parcel.
    pub line: Option<serde_json::Value>,
    /// For a retirement: why (Aumentum ArchiveReason).
    pub reason: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PendingMutationDto {
    /// Identifies the mutation within the job until commit.
    pub group: String,
    pub kind: String,
    pub parents: Vec<String>,
    /// The draft parcels it created (ids).
    pub drafts: Vec<Uuid>,
    pub reason: Option<String>,
}

fn kind_code(kind: &str) -> ApiResult<Option<i16>> {
    match kind {
        "split" => Ok(Some(1)),
        "merge" => Ok(Some(2)),
        "resurvey" => Ok(Some(3)),
        // Aumentum's history: a Free Mutation is one parcel to one (new UPIN), as an adjustment;
        // a Separation is one parcel cut into parts along a line, as a split.
        "free" => Ok(Some(4)),
        "separate" => Ok(Some(6)),
        "retire" => Ok(None),
        _ => Err(AppError::BadRequest("kind must be split, merge, resurvey, free, separate or retire".into())),
    }
}
fn kind_name(code: Option<i16>) -> &'static str {
    match code {
        Some(1) => "split",
        Some(2) => "merge",
        Some(3) => "resurvey",
        Some(4) => "free",
        Some(6) => "separate",
        _ => "retire",
    }
}

/// A parcel's lineage identity: Aumentum's ParcelID (the attribute table's, active row first) for
/// Aumentum's parcels, so new history joins Aumentum's; else the fabric parcel's own id.
async fn identity_of(conn: &mut sqlx::PgConnection, upin: &str, fabric_id: Uuid) -> ApiResult<Uuid> {
    let aum: Option<Uuid> = sqlx::query_scalar(
        "SELECT parcel_id FROM cadastre.parcel_attribute WHERE upin = $1 AND parcel_id IS NOT NULL \
         ORDER BY (status = '0') DESC NULLS LAST, last_edit_date DESC NULLS LAST, object_id DESC LIMIT 1",
    )
    .bind(upin)
    .fetch_optional(&mut *conn)
    .await?;
    if let Some(a) = aum {
        return Ok(a);
    }
    // A parcel created here has lineage rows under its fabric id.
    Ok(fabric_id)
}

async fn editing_job(conn: &mut sqlx::PgConnection, id: i32) -> ApiResult<Uuid> {
    let cj: Option<Uuid> = sqlx::query_scalar("SELECT cadastre_job_id FROM cadastre.wf_job_instance WHERE id = $1")
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("job instance {id}")))?;
    cj.ok_or_else(|| AppError::Conflict("choose Work On This Job first".into()))
}

/// Record a mutation in a workflow job: its new parcels become the job's drafts; the fabric
/// changes when the job's parcels are committed.
#[utoipa::path(post, path = "/api/v1/wf/jobs/{id}/mutations", tag = "wf", request_body = MutationRequest,
    params(("id" = i32, Path, description = "Job instance id")),
    responses((status = 201, body = PendingMutationDto), (status = 400, description = "Invalid mutation"),
              (status = 409, description = "Parcel not posted, already being mutated, outside the job area, or job closed")))]
pub async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i32>,
    Json(req): Json<MutationRequest>,
) -> ApiResult<(StatusCode, Json<PendingMutationDto>)> {
    let actor = resolve_actor(&state, &user).await?;
    let code = kind_code(&req.kind)?;
    let upins: Vec<String> = req.upins.iter().map(|u| u.trim().to_uppercase()).filter(|u| !u.is_empty()).collect::<BTreeSet<_>>().into_iter().collect();
    match code {
        Some(2) if upins.len() < 2 => return Err(AppError::BadRequest("a merge needs two or more parcels".into())),
        Some(1) | Some(3) | Some(4) | Some(6) if upins.len() != 1 => return Err(AppError::BadRequest(format!("a {} applies to one parcel", req.kind))),
        None if upins.is_empty() => return Err(AppError::BadRequest("choose the parcel(s) to retire".into())),
        _ => {}
    }
    let reason = req.reason.as_deref().map(str::trim).filter(|r| !r.is_empty()).map(str::to_string);
    if code.is_none() && reason.is_none() {
        return Err(AppError::BadRequest("give the reason for retiring (Aumentum's Archive Reason)".into()));
    }

    let mut tx = state.pool.begin().await?;
    lock_open_job(&mut tx, id, &actor).await?;
    let job_id = editing_job(&mut tx, id).await?;

    // Every parent: a posted parcel, not already in a pending mutation (in any job).
    let mut parents: Vec<(String, Uuid, Uuid, String)> = Vec::new(); // upin, fabric id, identity, geojson
    for u in &upins {
        let row = sqlx::query("SELECT id, ST_AsGeoJSON(geom, 3) AS g FROM cadastre.parcel WHERE parcel_number = $1 AND status = 'posted' FOR UPDATE")
            .bind(u)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| AppError::Conflict(format!("{u} is not a posted parcel")))?;
        let busy: Option<String> = sqlx::query_scalar(
            "SELECT COALESCE(w.name, 'job ' || w.id::text) FROM cadastre.parcel_lineage l JOIN cadastre.wf_job_instance w ON w.id = l.wf_job_instance_id \
             WHERE l.pending AND (l.parent_upin = $1 OR (l.upin = $1 AND l.status = 0)) LIMIT 1",
        )
        .bind(u)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(job) = busy {
            return Err(AppError::Conflict(format!("{u} is already being mutated in {job}")));
        }
        let fid: Uuid = row.get("id");
        let ident = identity_of(&mut tx, u, fid).await?;
        parents.push((u.clone(), fid, ident, row.get("g")));
    }

    // The new shapes.
    let new_shapes: Vec<String> = match code {
        Some(1) | Some(6) => {
            let line = req.line.as_ref().ok_or_else(|| AppError::BadRequest(format!("a {} needs a line across the parcel", if code == Some(6) { "separation" } else { "split" })))?;
            let parts: Vec<String> = sqlx::query_scalar(
                "SELECT ST_AsGeoJSON((ST_Dump(ST_CollectionExtract(ST_Split(ST_SetSRID(ST_GeomFromGeoJSON($1), 32632), ST_SetSRID(ST_GeomFromGeoJSON($2), 32632)), 3))).geom, 3)",
            )
            .bind(&parents[0].3)
            .bind(line.to_string())
            .fetch_all(&mut *tx)
            .await
            .map_err(|_| AppError::BadRequest("the split line is not a valid line".into()))?;
            if parts.len() < 2 {
                return Err(AppError::BadRequest("the line must cross the parcel from one edge to another".into()));
            }
            parts
        }
        Some(2) => {
            let shapes: Vec<String> = parents.iter().map(|p| p.3.clone()).collect();
            let (gtype, g): (String, String) = sqlx::query_as(
                "SELECT GeometryType(u), ST_AsGeoJSON(u, 3) FROM (SELECT ST_UnaryUnion(ST_Collect(ST_SetSRID(ST_GeomFromGeoJSON(g), 32632))) AS u FROM unnest($1::text[]) g) s",
            )
            .bind(&shapes)
            .fetch_one(&mut *tx)
            .await?;
            if gtype != "POLYGON" {
                return Err(AppError::BadRequest("the parcels to merge must share boundaries (their union is not one parcel)".into()));
            }
            vec![g]
        }
        Some(_) => vec![parents[0].3.clone()],
        None => Vec::new(),
    };
    // Aumentum's rule: edits inside the job area (for a retirement, the retired parcel).
    let check: Vec<String> = if new_shapes.is_empty() { parents.iter().map(|p| p.3.clone()).collect() } else { new_shapes.clone() };
    for g in &check {
        let v: serde_json::Value = serde_json::from_str(g).unwrap_or_default();
        crate::routes::parcels::job_area_guard(&state, job_id, Some(&v)).await?;
    }

    let group = format!("P-{}", Uuid::new_v4().simple());
    let mut drafts = Vec::new();
    for g in &new_shapes {
        let did: Uuid = sqlx::query_scalar(
            "INSERT INTO cadastre.parcel (job_id, geom, computed_area, status) VALUES ($1, ST_SetSRID(ST_GeomFromGeoJSON($2), 32632), ST_Area(ST_SetSRID(ST_GeomFromGeoJSON($2), 32632)), 'draft') RETURNING id",
        )
        .bind(job_id)
        .bind(g)
        .fetch_one(&mut *tx)
        .await?;
        drafts.push(did);
        for (u, _, ident, _) in &parents {
            sqlx::query(
                "INSERT INTO cadastre.parcel_lineage (parcel_id, status, parent_id, parent_upin, mutation_id, mutation_type, pending, wf_job_instance_id, created_by) \
                 VALUES ($1, 1, $2, $3, $4, $5, true, $6, $7)",
            )
            .bind(did).bind(ident).bind(u).bind(&group).bind(code).bind(id).bind(&actor.label)
            .execute(&mut *tx)
            .await?;
        }
    }
    if code.is_none() {
        for (u, _, ident, _) in &parents {
            sqlx::query(
                "INSERT INTO cadastre.parcel_lineage (parcel_id, upin, status, mutation_id, reason, pending, wf_job_instance_id, created_by) \
                 VALUES ($1, $2, 0, $3, $4, true, $5, $6)",
            )
            .bind(ident).bind(u).bind(&group).bind(&reason).bind(id).bind(&actor.label)
            .execute(&mut *tx)
            .await?;
        }
    }
    audit::append(&mut tx, &actor.label, "wf_mutation_recorded", "wf_job_instance", Some(&id.to_string()),
        json!({ "kind": req.kind, "parents": upins, "drafts": drafts.len(), "reason": reason })).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(PendingMutationDto { group, kind: kind_name(code).into(), parents: upins, drafts, reason })))
}

async fn pending_list(conn: &mut sqlx::PgConnection, id: i32) -> ApiResult<Vec<PendingMutationDto>> {
    let rows = sqlx::query(
        "SELECT mutation_id, mutation_type, status, parcel_id, upin, parent_upin, reason FROM cadastre.parcel_lineage \
         WHERE pending AND wf_job_instance_id = $1 ORDER BY created_at, mutation_id",
    )
    .bind(id)
    .fetch_all(&mut *conn)
    .await?;
    let mut by: BTreeMap<String, PendingMutationDto> = BTreeMap::new();
    for r in &rows {
        let g: String = r.get("mutation_id");
        let code: Option<i16> = r.get("mutation_type");
        let e = by.entry(g.clone()).or_insert_with(|| PendingMutationDto { group: g, kind: kind_name(code).into(), parents: vec![], drafts: vec![], reason: r.get("reason") });
        let st: i16 = r.get("status");
        let parent: Option<String> = if st == 0 { r.get("upin") } else { r.get("parent_upin") };
        if let Some(p) = parent {
            if !e.parents.contains(&p) {
                e.parents.push(p);
            }
        }
        if st == 1 {
            let d: Uuid = r.get("parcel_id");
            if !e.drafts.contains(&d) {
                e.drafts.push(d);
            }
        }
    }
    Ok(by.into_values().collect())
}

/// The mutations recorded in a job, not yet committed.
#[utoipa::path(get, path = "/api/v1/wf/jobs/{id}/mutations", tag = "wf", params(("id" = i32, Path, description = "Job instance id")),
    responses((status = 200, body = [PendingMutationDto])))]
pub async fn list(State(state): State<AppState>, Path(id): Path<i32>) -> ApiResult<Json<Vec<PendingMutationDto>>> {
    let mut conn = state.pool.acquire().await?;
    tidy_pending(&mut conn, id).await?;
    Ok(Json(pending_list(&mut conn, id).await?))
}

/// Cancel a recorded mutation: its draft parcels are removed and the parents are left as they are.
#[utoipa::path(delete, path = "/api/v1/wf/jobs/{id}/mutations/{group}", tag = "wf",
    params(("id" = i32, Path, description = "Job instance id"), ("group" = String, Path, description = "Mutation group")),
    responses((status = 204), (status = 404, description = "No such pending mutation")))]
pub async fn cancel(State(state): State<AppState>, user: AuthUser, Path((id, group)): Path<(i32, String)>) -> ApiResult<StatusCode> {
    let actor = resolve_actor(&state, &user).await?;
    let mut tx = state.pool.begin().await?;
    lock_open_job(&mut tx, id, &actor).await?;
    let drafts: Vec<Uuid> = sqlx::query_scalar("SELECT DISTINCT parcel_id FROM cadastre.parcel_lineage WHERE pending AND wf_job_instance_id = $1 AND mutation_id = $2 AND status = 1")
        .bind(id).bind(&group).fetch_all(&mut *tx).await?;
    let n = sqlx::query("DELETE FROM cadastre.parcel_lineage WHERE pending AND wf_job_instance_id = $1 AND mutation_id = $2")
        .bind(id).bind(&group).execute(&mut *tx).await?.rows_affected();
    if n == 0 {
        return Err(AppError::NotFound(format!("pending mutation {group}")));
    }
    sqlx::query("DELETE FROM cadastre.parcel WHERE id = ANY($1) AND status = 'draft'").bind(&drafts).execute(&mut *tx).await?;
    audit::append(&mut tx, &actor.label, "wf_mutation_cancelled", "wf_job_instance", Some(&id.to_string()), json!({ "group": group })).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Drop pending lineage rows whose draft no longer exists (a draft deleted or discarded), and
/// carry a separated draft's lineage onto its parts. Called before commit and by discard.
pub async fn tidy_pending(conn: &mut sqlx::PgConnection, id: i32) -> ApiResult<()> {
    sqlx::query(
        "DELETE FROM cadastre.parcel_lineage l WHERE l.pending AND l.wf_job_instance_id = $1 AND l.status = 1 \
         AND NOT EXISTS (SELECT 1 FROM cadastre.parcel p WHERE p.id = l.parcel_id AND p.status = 'draft')",
    )
    .bind(id)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// The UPINs a job's pending mutations retire: they are left out of the overlap check at commit.
pub async fn pending_parents(conn: &mut sqlx::PgConnection, id: i32) -> ApiResult<Vec<String>> {
    tidy_pending(conn, id).await?;
    Ok(sqlx::query_scalar::<_, Option<String>>(
        "SELECT DISTINCT COALESCE(parent_upin, upin) FROM cadastre.parcel_lineage WHERE pending AND wf_job_instance_id = $1 \
         AND (status = 1 OR (status = 0 AND mutation_type IS NULL))",
    )
    .bind(id)
    .fetch_all(&mut *conn)
    .await?
    .into_iter()
    .flatten()
    .collect())
}

/// Whether a job has a retirement waiting (a job may commit with no drafts if so).
pub async fn has_pending_retirement(conn: &mut sqlx::PgConnection, id: i32) -> ApiResult<bool> {
    Ok(sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM cadastre.parcel_lineage WHERE pending AND wf_job_instance_id = $1 AND status = 0 AND mutation_type IS NULL)")
        .bind(id)
        .fetch_one(&mut *conn)
        .await?)
}

/// At commit, before the drafts are posted: retire the parents (so their UPINs' active slot is
/// free and they leave the active fabric). Returns the retired UPINs.
pub async fn retire_parents(conn: &mut sqlx::PgConnection, id: i32) -> ApiResult<Vec<String>> {
    let rows = sqlx::query(
        "SELECT DISTINCT COALESCE(parent_upin, upin) AS upin, mutation_type IS NULL AND status = 0 AS retire, reason \
         FROM cadastre.parcel_lineage WHERE pending AND wf_job_instance_id = $1 AND (status = 1 OR (status = 0 AND mutation_type IS NULL))",
    )
    .bind(id)
    .fetch_all(&mut *conn)
    .await?;
    let mut out = Vec::new();
    for r in &rows {
        let upin: String = r.get("upin");
        let retire: bool = r.get("retire");
        let reason: Option<String> = r.get("reason");
        let new_status = if retire { "archived" } else { "historic" };
        let n = sqlx::query("UPDATE cadastre.parcel SET status = $2, updated_at = now() WHERE parcel_number = $1 AND status = 'posted'")
            .bind(&upin).bind(new_status).execute(&mut *conn).await?.rows_affected();
        if n == 0 {
            return Err(AppError::Conflict(format!("{upin} is no longer a posted parcel; cancel that mutation")));
        }
        // Aumentum's attribute record of the retired parcel.
        sqlx::query(
            "UPDATE cadastre.parcel_attribute SET archive_date = now()::timestamp, archive_reason = COALESCE($2, archive_reason), last_edit_date = now()::timestamp \
             WHERE upin = $1 AND (status = '0' OR status IS NULL)",
        )
        .bind(&upin)
        .bind(if retire { reason.clone() } else { None })
        .execute(&mut *conn)
        .await?;
        out.push(upin);
    }
    Ok(out)
}

/// At commit, after the drafts are posted: write the lineage as Aumentum does.
pub async fn finalise(conn: &mut sqlx::PgConnection, id: i32, posted: &[(Uuid, String)]) -> ApiResult<usize> {
    inherit_attributes(conn, id, posted).await?;
    let upin_of: HashMap<Uuid, &String> = posted.iter().map(|(d, u)| (*d, u)).collect();
    let mutation_id: String = sqlx::query_scalar("SELECT to_char(now(), 'YYYYMMDDHH24MI')").fetch_one(&mut *conn).await?;
    let rows = sqlx::query(
        "SELECT id, parcel_id, status, parent_id, parent_upin, mutation_type, mutation_id FROM cadastre.parcel_lineage WHERE pending AND wf_job_instance_id = $1",
    )
    .bind(id)
    .fetch_all(&mut *conn)
    .await?;
    let mut parents_written: BTreeSet<(Uuid, String)> = BTreeSet::new();
    for r in &rows {
        let row_id: Uuid = r.get("id");
        let st: i16 = r.get("status");
        if st == 1 {
            let child: Uuid = r.get("parcel_id");
            let upin = upin_of.get(&child).map(|u| u.to_string());
            sqlx::query("UPDATE cadastre.parcel_lineage SET upin = $2, changed_to_active = now()::timestamp, mutation_id = $3, pending = false WHERE id = $1")
                .bind(row_id).bind(&upin).bind(&mutation_id).execute(&mut *conn).await?;
            // The parent's history row (once per parent per mutation).
            let parent: Option<Uuid> = r.get("parent_id");
            let group: String = r.get("mutation_id");
            if let (Some(p), Some(pu)) = (parent, r.get::<Option<String>, _>("parent_upin")) {
                if parents_written.insert((p, group.clone())) {
                    sqlx::query(
                        "INSERT INTO cadastre.parcel_lineage (parcel_id, upin, status, changed_to_history, mutation_id, mutation_type, wf_job_instance_id) \
                         VALUES ($1, $2, 0, now()::timestamp, $3, $4, $5)",
                    )
                    .bind(p).bind(pu).bind(&mutation_id).bind(r.get::<Option<i16>, _>("mutation_type")).bind(id)
                    .execute(&mut *conn).await?;
                }
            }
        } else {
            sqlx::query("UPDATE cadastre.parcel_lineage SET changed_to_history = now()::timestamp, mutation_id = $2, pending = false WHERE id = $1")
                .bind(row_id).bind(&mutation_id).execute(&mut *conn).await?;
        }
    }
    Ok(rows.len())
}

/// Aumentum parity (ParcelAttributeConfig IsMutationInherited, and its recorded mutations): each
/// new parcel of a mutation gets its own attribute record. From the parent (for a merge, the first
/// parent, i.e. the lowest attribute record) it copies the dataset, region, plot (Lot), block,
/// location, LGA, property type, area unit, rollback and strata flags, the Registry link fields
/// (LRSID, LinkageStatus) and Parent. It gets ParcelName PCL1, PCL2, ... in order, ParcelArea from
/// its own shape, the MutationType, status 0 (active) and today's dates. Never copied: the
/// Registry property number, plan number, linkage date and archive fields. A parent without an
/// attribute record gives nothing to inherit: only the new parcel's own values are written.
/// A pending mutation: (type, its new parcels, its parents' UPINs).
type MutationGroup = (Option<i16>, Vec<Uuid>, Vec<String>);

async fn inherit_attributes(conn: &mut sqlx::PgConnection, id: i32, posted: &[(Uuid, String)]) -> ApiResult<usize> {
    // New attribute records continue Aumentum's OBJECTID numbering.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext('cadastre.parcel_attribute.object_id'))").execute(&mut *conn).await?;
    let order: HashMap<Uuid, usize> = posted.iter().enumerate().map(|(i, (d, _))| (*d, i)).collect();
    let rows = sqlx::query(
        "SELECT mutation_id, mutation_type, parcel_id, parent_upin FROM cadastre.parcel_lineage \
         WHERE pending AND wf_job_instance_id = $1 AND status = 1",
    )
    .bind(id)
    .fetch_all(&mut *conn)
    .await?;
    let mut groups: BTreeMap<String, MutationGroup> = BTreeMap::new();
    for r in &rows {
        let g = groups.entry(r.get("mutation_id")).or_insert((r.get("mutation_type"), vec![], vec![]));
        let child: Uuid = r.get("parcel_id");
        if !g.1.contains(&child) {
            g.1.push(child);
        }
        if let Some(p) = r.get::<Option<String>, _>("parent_upin") {
            if !g.2.contains(&p) {
                g.2.push(p);
            }
        }
    }
    let upin_of: HashMap<Uuid, &String> = posted.iter().map(|(d, u)| (*d, u)).collect();
    let mut written = 0;
    for (code, mut children, parents) in groups.into_values() {
        children.sort_by_key(|c| order.get(c).copied().unwrap_or(usize::MAX));
        // The source record: the parents' active attribute rows, lowest record first.
        let source: Option<i32> = sqlx::query_scalar(
            "SELECT object_id FROM cadastre.parcel_attribute WHERE upin = ANY($1) \
             ORDER BY (status = '0') DESC NULLS LAST, object_id LIMIT 1",
        )
        .bind(&parents)
        .fetch_optional(&mut *conn)
        .await?;
        for (k, child) in children.iter().enumerate() {
            let Some(upin) = upin_of.get(child) else { continue };
            sqlx::query(
                "INSERT INTO cadastre.parcel_attribute (object_id, upin, parcel_id, parcel_name, parcel_area, status, mutation_type, \
                    creation_date, last_edit_date, dataset_name, region_name, plot, block, location, lga, property_type, area_unit, \
                    can_rollback, is_strata, lrs_id, linkage_status, parent) \
                 SELECT (SELECT COALESCE(max(object_id), 0) + 1 FROM cadastre.parcel_attribute), $1, $2, $3, \
                    (SELECT ST_Area(geom) FROM cadastre.parcel WHERE id = $2), '0', $4, now()::timestamp, now()::timestamp, \
                    s.dataset_name, s.region_name, s.plot, s.block, s.location, s.lga, s.property_type, s.area_unit, \
                    s.can_rollback, s.is_strata, s.lrs_id, s.linkage_status, s.parent \
                 FROM (SELECT 1) one LEFT JOIN cadastre.parcel_attribute s ON s.object_id = $5",
            )
            .bind(upin.as_str())
            .bind(child)
            .bind(format!("PCL{}", k + 1))
            .bind(code)
            .bind(source)
            .execute(&mut *conn)
            .await?;
            written += 1;
        }
    }
    Ok(written)
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct LineageQuery {
    /// The parcel's UPIN.
    pub upin: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct LineageEdgeDto {
    pub parent_upin: Option<String>,
    pub child_upin: Option<String>,
    pub mutation_type: Option<i16>,
    pub mutation: String,
    pub mutation_id: Option<String>,
    pub date: Option<chrono::NaiveDateTime>,
    pub source: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct LineageDto {
    pub upin: String,
    /// Parent -> child relations reachable from the parcel, oldest first.
    pub edges: Vec<LineageEdgeDto>,
    /// Retirements (archived with a reason) among those parcels.
    pub retirements: Vec<serde_json::Value>,
    /// Every shape in the fabric of the UPINs involved: upin, status, area.
    pub shapes: Vec<serde_json::Value>,
}

/// A parcel's lineage: where it came from and what it became, across Aumentum's history and the
/// mutations made here.
#[utoipa::path(get, path = "/api/v1/parcels/lineage", tag = "parcels", params(LineageQuery),
    responses((status = 200, body = LineageDto)))]
pub async fn lineage(State(state): State<AppState>, Query(q): Query<LineageQuery>) -> ApiResult<Json<LineageDto>> {
    let upin = q.upin.trim().to_uppercase();
    let rows = sqlx::query(
        "WITH RECURSIVE ids AS ( \
            SELECT parcel_id AS pid FROM cadastre.parcel_lineage WHERE NOT pending AND upin = $1 \
            UNION SELECT parent_id FROM cadastre.parcel_lineage WHERE NOT pending AND parent_upin = $1 AND parent_id IS NOT NULL \
         ), walk(pid, depth) AS ( \
            SELECT pid, 0 FROM ids \
            UNION SELECT CASE WHEN l.parent_id = w.pid THEN l.parcel_id ELSE l.parent_id END, w.depth + 1 \
            FROM cadastre.parcel_lineage l JOIN walk w ON (l.parent_id = w.pid OR l.parcel_id = w.pid) \
            WHERE NOT l.pending AND l.parent_id IS NOT NULL AND w.depth < 30 \
         ) \
         SELECT DISTINCT ON (l.parent_id, l.parcel_id) l.parent_upin, l.upin, l.mutation_type, l.mutation_id, \
                COALESCE(l.changed_to_active, l.created_at::timestamp) AS date, l.source \
         FROM cadastre.parcel_lineage l WHERE NOT l.pending AND l.parent_id IS NOT NULL \
           AND (l.parcel_id IN (SELECT pid FROM walk) OR l.parent_id IN (SELECT pid FROM walk)) \
         ORDER BY l.parent_id, l.parcel_id, date",
    )
    .bind(&upin)
    .fetch_all(&state.pool)
    .await?;
    let mut edges: Vec<LineageEdgeDto> = rows
        .iter()
        .map(|r| {
            let t: Option<i16> = r.get("mutation_type");
            LineageEdgeDto {
                parent_upin: r.get("parent_upin"),
                child_upin: r.get("upin"),
                mutation_type: t,
                mutation: mutation_type_name(t).to_string(),
                mutation_id: r.get("mutation_id"),
                date: r.get("date"),
                source: r.get("source"),
            }
        })
        .collect();
    edges.sort_by_key(|e| e.date);
    let mut upins: BTreeSet<String> = edges.iter().flat_map(|e| [e.parent_upin.clone(), e.child_upin.clone()]).flatten().collect();
    upins.insert(upin.clone());
    let list: Vec<String> = upins.into_iter().collect();
    let retirements = sqlx::query(
        "SELECT upin, reason, changed_to_history, mutation_id, source FROM cadastre.parcel_lineage \
         WHERE NOT pending AND status = 0 AND mutation_type IS NULL AND upin = ANY($1) ORDER BY changed_to_history",
    )
    .bind(&list)
    .fetch_all(&state.pool)
    .await?
    .iter()
    .map(|r| json!({ "upin": r.get::<Option<String>, _>("upin"), "reason": r.get::<Option<String>, _>("reason"),
        "date": r.get::<Option<chrono::NaiveDateTime>, _>("changed_to_history"), "mutation_id": r.get::<Option<String>, _>("mutation_id") }))
    .collect();
    let shapes = sqlx::query("SELECT parcel_number, status, round(ST_Area(geom)::numeric, 2)::float8 AS area FROM cadastre.parcel WHERE parcel_number = ANY($1) ORDER BY parcel_number, status")
        .bind(&list)
        .fetch_all(&state.pool)
        .await?
        .iter()
        .map(|r| json!({ "upin": r.get::<Option<String>, _>("parcel_number"), "status": r.get::<String, _>("status"), "area": r.get::<f64, _>("area") }))
        .collect();
    Ok(Json(LineageDto { upin, edges, retirements, shapes }))
}
