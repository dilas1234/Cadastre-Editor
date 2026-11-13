//! Completed jobs (the Table Of Contents' Completed tab) and Correct.
//!
//! A committed job is completed: that is the end of digitising. The Completed tab lists every
//! completed job, from this Cadastre and from Aumentum, newest first, with its metadata (Registry
//! transaction, property number, UPINs, committed / linked date and time, the plan sent) and, for
//! the page shown, what the Registry (10.10.10.5) holds for it, read live and read-only.
//!
//! **Correct** (see [`crate::routes::correction`]) reopens the file's job at its commit task with a
//! copy of the parcel under the same UPIN, so the shape, beacons and attributes can be fixed while
//! the committed shape and its Registry link stay; the corrected plan is then resent.

use axum::extract::{Path, Query, State};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;
use utoipa::{IntoParams, ToSchema};

use crate::auth::AuthUser;
use crate::error::{ApiResult, AppError};
use crate::state::AppState;

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct CompletedQuery {
    /// Job name, transaction, property number or UPIN (part of).
    #[serde(default)]
    pub q: String,
    /// `cadastre` (committed in this editor), `aumentum`, or empty for both.
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub offset: i64,
    #[serde(default)]
    pub limit: Option<i64>,
    /// 1: also read what the Registry holds for the jobs on this page (live, read-only).
    #[serde(default)]
    pub registry: Option<u8>,
}

/// Completed jobs, newest first, with their metadata.
#[utoipa::path(get, path = "/api/v1/wf/completed", tag = "wf", params(CompletedQuery), responses((status = 200)))]
pub async fn list(State(state): State<AppState>, Query(q): Query<CompletedQuery>) -> ApiResult<Json<Value>> {
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let offset = q.offset.max(0);
    let term = q.q.trim().to_uppercase();
    let like = format!("%{term}%");
    let source = match q.source.as_str() { "cadastre" => Some(true), "aumentum" => Some(false), _ => None };
    // The jobs, with the property number from the job's linkage or its name ("Job-PL68928").
    const BASE: &str = "FROM cadastre.wf_job_instance j \
         JOIN cadastre.wf_job_type jt ON jt.id = j.job_type_id \
         LEFT JOIN cadastre.wf_job_linkage l ON l.wf_job_instance_id = j.id \
         CROSS JOIN LATERAL (SELECT COALESCE(NULLIF(btrim(l.property_number), ''), substring(j.name from '^Job-(.+)$')) AS pn) k \
         WHERE j.status = 3 \
           AND ($1 = '' OR upper(j.name) LIKE $2 OR upper(COALESCE(j.lrs_transaction_number, '')) LIKE $2 OR upper(COALESCE(k.pn, '')) LIKE $2 \
                OR EXISTS (SELECT 1 FROM cadastre.parcel p WHERE p.job_id = j.cadastre_job_id AND p.parcel_number LIKE $2) \
                OR ($1 LIKE 'PL0%' AND EXISTS (SELECT 1 FROM cadastre.parcel_attribute pa WHERE pa.upin = $1 AND pa.property_number = k.pn))) \
           AND ($3::bool IS NULL OR ((j.source IS DISTINCT FROM 'aumentum') = $3))";
    let total: i64 = sqlx::query_scalar(&format!("SELECT count(*) {BASE}"))
        .bind(&term).bind(&like).bind(source).fetch_one(&state.pool).await?;
    let rows = sqlx::query(&format!(
        "WITH page AS (SELECT j.id, k.pn, jt.name AS job_type {BASE} ORDER BY j.ended DESC NULLS LAST, j.id DESC LIMIT {limit} OFFSET {offset}) \
         SELECT j.id, j.name, pg.job_type, j.source, j.lrs_transaction_number AS txn, pg.pn AS pn, j.created, j.ended, j.cadastre_job_id, \
                (SELECT a.actor FROM cadastre.audit_log a WHERE a.entity = 'wf_job_instance' AND a.entity_id = j.id::text AND a.action IN ('wf_commit', 'wf_complete') ORDER BY a.id DESC LIMIT 1) AS completed_by, \
                (SELECT json_agg(json_build_object('upin', p.parcel_number, 'committed_at', p.posted_at, 'area', round(COALESCE(p.computed_area, ST_Area(p.geom))::numeric, 2), \
                         'linked', COALESCE(x.linkage_status = 1, false), 'linked_at', x.linkage_date, 'lrs_id', x.lrs_id) ORDER BY p.posted_at) \
                   FROM cadastre.parcel p LEFT JOIN LATERAL (SELECT linkage_status, linkage_date, lrs_id FROM cadastre.parcel_attribute y WHERE y.upin = p.parcel_number \
                        ORDER BY (y.status = '0') DESC NULLS LAST, y.object_id DESC LIMIT 1) x ON true \
                  WHERE j.cadastre_job_id IS NOT NULL AND p.job_id = j.cadastre_job_id AND p.status = 'posted' AND p.parcel_number IS NOT NULL) AS committed, \
                (SELECT json_agg(json_build_object('upin', pa.upin, 'committed_at', pa.creation_date, 'linked', COALESCE(pa.linkage_status = 1, false), \
                         'linked_at', pa.linkage_date, 'lrs_id', pa.lrs_id) ORDER BY pa.creation_date) \
                   FROM cadastre.parcel_attribute pa WHERE pg.pn IS NOT NULL AND pa.property_number = pg.pn \
                    AND (pa.status = '0' OR pa.status IS NULL) AND pa.upin IS NOT NULL) AS aumentum_parcels, \
                (SELECT json_build_object('action', a.action, 'at', a.created_at, 'by', a.actor, 'file', a.payload->>'file_name', 'resend', a.payload->'resend', 'reason', a.payload->>'reason') \
                   FROM cadastre.audit_log a WHERE a.entity = 'wf_job_instance' AND a.entity_id = j.id::text \
                    AND a.action IN ('registry_plan_sent', 'registry_plan_withdrawn') ORDER BY a.id DESC LIMIT 1) AS plan, \
                (SELECT json_build_object('ok', (a.payload->>'ok')::bool, 'at', a.created_at) FROM cadastre.audit_log a WHERE a.entity = 'wf_job_instance' \
                    AND a.entity_id = j.id::text AND a.action = 'registry_confirmation' ORDER BY a.id DESC LIMIT 1) AS confirmation \
         FROM page pg JOIN cadastre.wf_job_instance j ON j.id = pg.id ORDER BY j.ended DESC NULLS LAST, j.id DESC"))
        .bind(&term).bind(&like).bind(source).fetch_all(&state.pool).await?;
    let mut jobs: Vec<Value> = rows.iter().map(|r| {
        let mine = r.get::<Option<String>, _>("source").as_deref() != Some("aumentum");
        let parcels: Value = if mine { r.get::<Option<Value>, _>("committed") } else { r.get::<Option<Value>, _>("aumentum_parcels") }.unwrap_or_else(|| json!([]));
        json!({
            "id": r.get::<i32, _>("id"),
            "name": r.get::<Option<String>, _>("name"),
            "job_type": r.get::<String, _>("job_type"),
            "source": if mine { "cadastre" } else { "aumentum" },
            "transaction_number": r.get::<Option<String>, _>("txn"),
            "property_number": r.get::<Option<String>, _>("pn"),
            "created": r.get::<Option<chrono::NaiveDateTime>, _>("created").map(|d| d.format("%Y-%m-%dT%H:%M:%S").to_string()),
            "completed": r.get::<Option<chrono::NaiveDateTime>, _>("ended").map(|d| d.format("%Y-%m-%dT%H:%M:%S").to_string()),
            "completed_by": r.get::<Option<String>, _>("completed_by"),
            "parcels": parcels,
            "plan": r.get::<Option<Value>, _>("plan"),
            "confirmation": r.get::<Option<Value>, _>("confirmation"),
            // Aumentum's jobs too: their parcels are corrected in the file's job here (correct-aumentum).
            "can_correct": !mine || r.get::<Option<uuid::Uuid>, _>("cadastre_job_id").is_some(),
        })
    }).collect();
    let mut registry_error: Option<String> = None;
    if q.registry == Some(1) {
        if let Err(e) = registry_details(&mut jobs).await { registry_error = Some(e); }
    }
    Ok(Json(json!({ "total": total, "offset": offset, "limit": limit, "jobs": jobs, "registry_error": registry_error })))
}

/// What the Registry holds for each job's transaction: its status, and the property (number, UPIN,
/// link date). One Registry session for the page. Read-only.
async fn registry_details(jobs: &mut [Value]) -> Result<(), String> {
    let txns: Vec<String> = jobs.iter().filter_map(|j| j["transaction_number"].as_str())
        .map(|t| t.trim().to_uppercase()).filter(|t| !t.is_empty() && t.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
        .collect::<std::collections::BTreeSet<_>>().into_iter().collect();
    if txns.is_empty() { return Ok(()); }
    let cfg = crate::routes::registry::config().map_err(|e| e.to_string())?;
    let session = crate::lrs::Session::start(&cfg).await.map_err(|e| e.to_string())?;
    let list = txns.iter().map(|t| format!("'{t}'")).collect::<Vec<_>>().join(", ");
    let status = session.query(&format!(
        "select t.TransactionNumber, ts.Label from com.landsystems.lrs.model.Transaction t join t.TransactionStatus ts where t.TransactionNumber in ({list})"), 500).await;
    let props = session.query(&format!(
        "select t.TransactionNumber, s.SuID, s.Refnum, s.Upin, s.SpatialDate1 from com.landsystems.lrs.model.Transaction t join t.Properties l join l.SpatialUnits s \
         where t.TransactionNumber in ({list}) and l.EndLifespanVersion is null and s.EndLifespanVersion is null"), 2000).await;
    let open = session.query(&format!(
        "select t.TransactionNumber, ph.ProcessName, ph.TaskDescription, ph.TaskCreate from com.landsystems.lrs.model.ProcessHistory ph, com.landsystems.lrs.model.Transaction t \
         where ph.TransactionId = t.Id and ph.TaskComplete is null and t.TransactionNumber in ({list})"), 2000).await;
    session.end().await;
    let status = status.map_err(|e| e.to_string())?;
    let props = props.unwrap_or_default();
    let open = open.unwrap_or_default();
    for j in jobs.iter_mut() {
        let Some(t) = j["transaction_number"].as_str().map(|t| t.trim().to_uppercase()) else { continue };
        let st = status.iter().find(|r| r.len() >= 2 && r[0].trim().eq_ignore_ascii_case(&t)).map(|r| r[1].trim().to_string());
        let pn = j["property_number"].as_str().unwrap_or("").to_uppercase();
        let mine: Vec<&Vec<String>> = props.iter().filter(|r| r.len() >= 5 && r[0].trim().eq_ignore_ascii_case(&t)).collect();
        let p = mine.iter().find(|r| r[2].trim().eq_ignore_ascii_case(&pn)).or(mine.first());
        let stages: Vec<Value> = open.iter().filter(|r| r.len() >= 4 && r[0].trim().eq_ignore_ascii_case(&t))
            .map(|r| json!({ "workflow": r[1].trim(), "stage": r[2].trim(), "since": crate::lrs::parse_datetime(&r[3]).map(|d| d.format("%Y-%m-%dT%H:%M:%SZ").to_string()) })).collect();
        j["registry"] = json!({
            "stages": stages,
            "found": st.is_some(),
            "status": st,
            "property": p.map(|r| r[1].trim().to_string()),
            "refnum": p.map(|r| r[2].trim().to_string()).filter(|s| !s.is_empty()),
            "upin": p.map(|r| r[3].trim().to_string()).filter(|s| !s.is_empty()),
            "linked_at": p.and_then(|r| crate::lrs::parse_datetime(&r[4])).map(|d| d.format("%Y-%m-%dT%H:%M:%SZ").to_string()),
        });
    }
    Ok(())
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CorrectRequest {
    /// The committed parcel to correct.
    pub upin: String,
    /// Why (recorded with the job).
    pub reason: String,
}

/// Correct from a job: as `POST /api/v1/wf/correct` (the file's job reopened with a copy of the
/// parcel under the same UPIN; the Registry link stays), preferring this job when it is the file's.
#[utoipa::path(post, path = "/api/v1/wf/jobs/{id}/correct", tag = "wf", params(("id" = i32, Path, description = "Job instance id")),
    request_body = CorrectRequest, responses((status = 200), (status = 409, description = "Lineage, or archived on purpose")))]
pub async fn correct(State(state): State<AppState>, user: AuthUser, Path(id): Path<i32>, Json(req): Json<CorrectRequest>) -> ApiResult<Json<Value>> {
    let actor = crate::routes::wf::resolve_actor(&state, &user).await?;
    sqlx::query_scalar::<_, i32>("SELECT id FROM cadastre.wf_job_instance WHERE id = $1").bind(id).fetch_optional(&state.pool).await?
        .ok_or_else(|| AppError::NotFound(format!("job instance {id}")))?;
    Ok(Json(crate::routes::correction::begin(&state, &actor.label, &req.upin, &req.reason, Some(id)).await?))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CorrectAumentumRequest {
    /// The parcel committed in Aumentum.
    pub upin: String,
    /// Why (recorded with the job).
    pub reason: String,
}

/// Correct a parcel committed in Aumentum: as `POST /api/v1/wf/correct` (kept for older pages).
#[utoipa::path(post, path = "/api/v1/wf/correct-aumentum", tag = "wf", request_body = CorrectAumentumRequest,
    responses((status = 200), (status = 404, description = "No such parcel"), (status = 409, description = "Lineage, or the file's job cannot be found or made")))]
pub async fn correct_aumentum(State(state): State<AppState>, user: AuthUser, Json(req): Json<CorrectAumentumRequest>) -> ApiResult<Json<Value>> {
    let actor = crate::routes::wf::resolve_actor(&state, &user).await?;
    Ok(Json(crate::routes::correction::begin(&state, &actor.label, &req.upin, &req.reason, None).await?))
}
