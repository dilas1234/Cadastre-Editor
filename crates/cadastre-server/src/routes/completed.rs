//! Completed jobs (the Table Of Contents' Completed tab) and Correct.
//!
//! A committed job is completed: that is the end of digitising. The Completed tab lists every
//! completed job, from this Cadastre and from Aumentum, newest first, with its metadata (Registry
//! transaction, property number, UPINs, committed / linked date and time, the plan sent) and, for
//! the page shown, what the Registry (10.10.10.5) holds for it, read live and read-only.
//!
//! **Correct** reopens a completed job at its commit task with its parcel back as a draft, keeping
//! the parcel's UPIN for it (the re-commit gives the same number), so the shape, beacons and
//! attributes can be fixed; the corrected plan is then resent. Refused when the parcel is linked in
//! the Registry (unlink there first), has mutation lineage, or shares beacons with other parcels.

use axum::extract::{Path, Query, State};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;
use utoipa::{IntoParams, ToSchema};

use crate::audit;
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
                   FROM cadastre.parcel_attribute pa WHERE j.cadastre_job_id IS NULL AND pg.pn IS NOT NULL AND pa.property_number = pg.pn \
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

/// Correct: reopen a completed job at its commit task with the parcel back as a draft (its UPIN kept
/// for it), so it can be fixed and committed again. Returns the beacon numbers taken off the
/// parcel's corners (by coordinates), so the re-commit offers them again.
#[utoipa::path(post, path = "/api/v1/wf/jobs/{id}/correct", tag = "wf", params(("id" = i32, Path, description = "Job instance id")),
    request_body = CorrectRequest, responses((status = 200), (status = 409, description = "Linked in the Registry, lineage, or shared beacons")))]
pub async fn correct(State(state): State<AppState>, user: AuthUser, Path(id): Path<i32>, Json(req): Json<CorrectRequest>) -> ApiResult<Json<Value>> {
    let actor = crate::routes::wf::resolve_actor(&state, &user).await?;
    let upin = req.upin.trim().to_uppercase();
    let reason = req.reason.trim().to_string();
    if reason.is_empty() {
        return Err(AppError::BadRequest("say why the parcel is corrected".into()));
    }
    let job = sqlx::query("SELECT job_type_id, cadastre_job_id, status, lrs_transaction_number FROM cadastre.wf_job_instance WHERE id = $1")
        .bind(id).fetch_optional(&state.pool).await?.ok_or_else(|| AppError::NotFound(format!("job instance {id}")))?;
    let (job_type, cjob, status, txn): (i32, Option<uuid::Uuid>, i16, Option<String>) = (job.get(0), job.get(1), job.get(2), job.get(3));
    let cjob = cjob.ok_or_else(|| AppError::Conflict("this job has no parcels committed in this Cadastre (an Aumentum job is corrected in a new job)".into()))?;
    // The Registry must not hold the UPIN on a property (a linked parcel is unlinked there first).
    if let Some(number) = txn.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
        if let Ok(cfg) = crate::routes::registry::config() {
            if let Ok(session) = crate::lrs::Session::start(&cfg).await {
                let t = crate::routes::registry::load_transaction(&session, number).await;
                session.end().await;
                if let Ok(Some(t)) = t {
                    if let Some(p) = t.properties.iter().find(|p| p.upin.as_deref().is_some_and(|u| u.eq_ignore_ascii_case(&upin))) {
                        return Err(AppError::Conflict(format!("{upin} is linked in the Registry to property {} ({}): unlink it there first", p.su_id, p.refnum.as_deref().unwrap_or("-"))));
                    }
                }
            }
        }
    }
    let mut tx = state.pool.begin().await?;
    let p = sqlx::query("SELECT id, posted_at FROM cadastre.parcel WHERE parcel_number = $1 AND status = 'posted' AND job_id = $2 FOR UPDATE")
        .bind(&upin).bind(cjob).fetch_optional(&mut *tx).await?
        .ok_or_else(|| AppError::Conflict(format!("{upin} is not a committed parcel of this job")))?;
    let (pid, posted_at): (uuid::Uuid, chrono::DateTime<chrono::Utc>) = (p.get(0), p.get(1));
    let linked: i64 = sqlx::query_scalar("SELECT count(*) FROM cadastre.parcel_attribute WHERE upin = $1 AND (linkage_status = 1 OR lrs_id IS NOT NULL)")
        .bind(&upin).fetch_one(&mut *tx).await?;
    if linked > 0 {
        return Err(AppError::Conflict(format!("{upin} is linked to the Registry: unlink it there first")));
    }
    let lineage: i64 = sqlx::query_scalar("SELECT count(*) FROM cadastre.parcel_lineage WHERE parcel_id = $1 OR parent_id = $1 OR upin = $2 OR parent_upin = $2")
        .bind(pid).bind(&upin).fetch_one(&mut *tx).await?;
    if lineage > 0 {
        return Err(AppError::Conflict(format!("{upin} has mutation lineage (split, merge …): correct it with a new mutation job")));
    }
    // Beacons the commit added (its audit record lists them) and only this parcel uses come off with
    // it (numbers handed back). A beacon that was in the fabric before stays.
    let added: Vec<String> = sqlx::query_scalar::<_, Option<Value>>(
        "SELECT payload->'beacons_added' FROM cadastre.audit_log WHERE entity = 'wf_job_instance' AND entity_id = $1 AND action = 'wf_commit' \
           AND payload->'parcel_numbers' ? $2 ORDER BY id DESC LIMIT 1")
        .bind(id.to_string()).bind(&upin).fetch_optional(&mut *tx).await?.flatten()
        .and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
    let beacons: Vec<(uuid::Uuid, String, f64, f64)> = sqlx::query_as(
        "SELECT b.id, b.number, ST_X(b.geom), ST_Y(b.geom) FROM cadastre.parcel_corner pc JOIN cadastre.beacon b ON b.id = pc.beacon_id \
         WHERE pc.parcel_id = $1 AND b.number = ANY($2) \
           AND NOT EXISTS (SELECT 1 FROM cadastre.parcel_corner x WHERE x.beacon_id = b.id AND x.parcel_id <> $1) ORDER BY pc.seq")
        .bind(pid).bind(&added).fetch_all(&mut *tx).await?;
    let _ = posted_at;
    let attr = sqlx::query(
        "SELECT object_id, property_number, property_type, plot, block, location, plan_number, lga, parcel_name, area_unit, surveyor_general, layout, area_name \
         FROM cadastre.parcel_attribute WHERE upin = $1 AND parcel_id = $2 AND object_id >= 1000000000")
        .bind(&upin).bind(pid).fetch_optional(&mut *tx).await?;
    let commit_task: i32 = sqlx::query_scalar(
        "SELECT f.task_id FROM cadastre.wf_task_flow f JOIN cadastre.wf_task_step ts ON ts.task_id = f.task_id JOIN cadastre.wf_step s ON s.id = ts.step_id \
         WHERE f.job_type_id = $1 AND EXISTS (SELECT 1 FROM unnest(s.commands) c WHERE upper(c) = upper($2)) ORDER BY f.task_id LIMIT 1")
        .bind(job_type).bind(crate::routes::wf::COMMIT_TOOL).fetch_optional(&mut *tx).await?
        .ok_or_else(|| AppError::Conflict("this job type has no commit task".into()))?;
    if let Some(a) = &attr {
        sqlx::query(
            "INSERT INTO cadastre.parcel_draft_attribute (parcel_id, property_number, property_type, plot, block, location, plan_number, lga, parcel_name, area_unit, surveyor_general, status, layout, area_name) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, 'Active', $12, $13) \
             ON CONFLICT (parcel_id) DO UPDATE SET property_number = EXCLUDED.property_number, property_type = EXCLUDED.property_type, plot = EXCLUDED.plot, \
               block = EXCLUDED.block, location = EXCLUDED.location, plan_number = EXCLUDED.plan_number, lga = EXCLUDED.lga, parcel_name = EXCLUDED.parcel_name, \
               area_unit = EXCLUDED.area_unit, surveyor_general = EXCLUDED.surveyor_general, layout = EXCLUDED.layout, area_name = EXCLUDED.area_name")
            .bind(pid).bind(a.get::<Option<String>, _>("property_number")).bind(a.get::<Option<String>, _>("property_type"))
            .bind(a.get::<Option<String>, _>("plot")).bind(a.get::<Option<String>, _>("block")).bind(a.get::<Option<String>, _>("location"))
            .bind(a.get::<Option<String>, _>("plan_number")).bind(a.get::<Option<String>, _>("lga")).bind(a.get::<Option<String>, _>("parcel_name"))
            .bind(a.get::<Option<i16>, _>("area_unit")).bind(a.get::<Option<String>, _>("surveyor_general"))
            .bind(a.get::<Option<String>, _>("layout")).bind(a.get::<Option<String>, _>("area_name"))
            .execute(&mut *tx).await?;
        sqlx::query("DELETE FROM cadastre.parcel_attribute WHERE object_id = $1").bind(a.get::<i32, _>("object_id")).execute(&mut *tx).await?;
    }
    sqlx::query("DELETE FROM cadastre.parcel_corner WHERE parcel_id = $1").bind(pid).execute(&mut *tx).await?;
    for (bid, _, _, _) in &beacons {
        sqlx::query("DELETE FROM cadastre.beacon WHERE id = $1").bind(bid).execute(&mut *tx).await?;
    }
    // Back to a draft that keeps its UPIN: the re-commit gives this same number (post_drafts keeps
    // a draft's number; the ledger already holds it).
    sqlx::query("UPDATE cadastre.parcel SET status = 'draft', posted_at = NULL, updated_at = now() WHERE id = $1")
        .bind(pid).execute(&mut *tx).await?;
    let sid: Option<i32> = crate::routes::wf::default_assignee(&mut tx, commit_task).await?;
    sqlx::query("UPDATE cadastre.wf_job_instance SET status = 2, ended = NULL, current_task_id = $2, updated = now()::timestamp WHERE id = $1")
        .bind(id).bind(commit_task).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO cadastre.wf_task_instance (job_instance_id, job_type_id, task_id, sid, assigned, started) VALUES ($1, $2, $3, $4, now()::timestamp, now()::timestamp)")
        .bind(id).bind(job_type).bind(commit_task).bind(sid).execute(&mut *tx).await?;
    audit::append(&mut tx, &actor.label, "wf_reopened_for_correction", "wf_job_instance", Some(&id.to_string()), json!({
        "upin": upin, "parcel_id": pid, "reason": reason, "was_status": status, "reopened_at_task": commit_task,
        "beacons_removed": beacons.iter().map(|b| &b.1).collect::<Vec<_>>(),
        "attribute_object_id": attr.as_ref().map(|a| a.get::<i32, _>("object_id")),
    })).await?;
    tx.commit().await?;
    Ok(Json(json!({
        "upin": upin, "parcel_id": pid, "reopened_at_task": commit_task,
        "beacons": beacons.iter().map(|(_, n, e, nn)| json!({ "number": n, "easting": e, "northing": nn })).collect::<Vec<_>>(),
    })))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CorrectAumentumRequest {
    /// The parcel committed in Aumentum.
    pub upin: String,
    /// Why (recorded with the job).
    pub reason: String,
}

/// Correct a parcel committed in Aumentum (it has no job here): the file's job is opened on its
/// Active Registry transaction (the open one, or a new Job-<file number>), and the parcel goes back
/// to a draft of that job at its commit task, keeping its UPIN; its attribute record moves to the
/// draft (the old record is kept in the audit). The re-commit gives the same UPIN. Refused when
/// the parcel is linked in the Registry (unlink there first) or has mutation lineage.
#[utoipa::path(post, path = "/api/v1/wf/correct-aumentum", tag = "wf", request_body = CorrectAumentumRequest,
    responses((status = 200), (status = 404, description = "No such committed parcel"), (status = 409, description = "Linked, lineage, committed here, or no Active transaction")))]
pub async fn correct_aumentum(State(state): State<AppState>, user: AuthUser, Json(req): Json<CorrectAumentumRequest>) -> ApiResult<Json<Value>> {
    let actor = crate::routes::wf::resolve_actor(&state, &user).await?;
    let upin = req.upin.trim().to_uppercase();
    let reason = req.reason.trim().to_string();
    if reason.is_empty() {
        return Err(AppError::BadRequest("say why the parcel is corrected".into()));
    }
    let p = sqlx::query("SELECT id, job_id FROM cadastre.parcel WHERE parcel_number = $1 AND status = 'posted'")
        .bind(&upin).fetch_optional(&state.pool).await?.ok_or_else(|| AppError::NotFound(format!("{upin} is not a committed parcel")))?;
    let (pid, own_job): (uuid::Uuid, Option<uuid::Uuid>) = (p.get(0), p.get(1));
    if own_job.is_some() {
        return Err(AppError::Conflict(format!("{upin} was committed in this Cadastre: use Correct on its job")));
    }
    let attr = sqlx::query(
        "SELECT a.object_id, row_to_json(a)::jsonb AS rec, a.property_number, a.lrs_id, a.linkage_status FROM cadastre.parcel_attribute a \
         WHERE a.upin = $1 AND (a.status = '0' OR a.status IS NULL) ORDER BY a.object_id DESC LIMIT 1")
        .bind(&upin).fetch_optional(&state.pool).await?
        .ok_or_else(|| AppError::Conflict(format!("{upin} has no current attribute record")))?;
    if attr.get::<Option<String>, _>("lrs_id").is_some() || attr.get::<Option<i16>, _>("linkage_status") == Some(1)
        || sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM cadastre.registry_property WHERE upin = $1)").bind(&upin).fetch_one(&state.pool).await? {
        return Err(AppError::Conflict(format!("{upin} is linked to the Registry: unlink it there first")));
    }
    let lineage: i64 = sqlx::query_scalar("SELECT count(*) FROM cadastre.parcel_lineage WHERE parcel_id = $1 OR parent_id = $1 OR upin = $2 OR parent_upin = $2")
        .bind(pid).bind(&upin).fetch_one(&state.pool).await?;
    if lineage > 0 {
        return Err(AppError::Conflict(format!("{upin} has mutation lineage (split, merge …): correct it with a new mutation job")));
    }
    let file = attr.get::<Option<String>, _>("property_number").map(|f| f.split_whitespace().collect::<String>().to_uppercase()).filter(|f| !f.is_empty())
        .ok_or_else(|| AppError::Conflict(format!("{upin} has no file number on its attributes: its Registry file is not known")))?;
    let file = crate::routes::registry::ident("file number", &file)?;
    // The file's Active transaction in the Registry; the Registry must not hold this UPIN.
    let cfg = crate::routes::registry::config()?;
    let session = crate::lrs::Session::start(&cfg).await.map_err(|e| AppError::Conflict(format!("the Registry (10.10.10.5) cannot be reached: {e}")))?;
    let rows = session.query(&format!(
        "select t.TransactionNumber, ts.Code, s.Upin from com.landsystems.lrs.model.Transaction t join t.TransactionStatus ts \
         join t.Properties l join l.SpatialUnits s where upper(s.Refnum) = '{file}' and l.EndLifespanVersion is null and s.EndLifespanVersion is null \
         order by t.TransactionNumber desc"), 50).await;
    session.end().await;
    let rows = rows.map_err(|e| AppError::Conflict(format!("the Registry (10.10.10.5) could not be read: {e}")))?;
    if rows.iter().any(|r| r.get(2).is_some_and(|u| u.trim().eq_ignore_ascii_case(&upin))) {
        return Err(AppError::Conflict(format!("{upin} is linked in the Registry to {file}: unlink it there first")));
    }
    let txn = rows.iter().find(|r| r.get(1).is_some_and(|c| c.trim() == "transaction_lodged")).map(|r| r[0].trim().to_string())
        .ok_or_else(|| AppError::Conflict(format!("{file} has no Active transaction in the Registry: the file's job cannot be opened")))?;
    // The file's open job, or a new one.
    let open: Option<(i32, Option<String>)> = sqlx::query_as("SELECT j.id, j.name FROM cadastre.wf_job_instance j JOIN cadastre.wf_job_status s ON s.code = j.status \
                                       WHERE j.lrs_transaction_number = $1 AND s.is_open ORDER BY j.id DESC LIMIT 1")
        .bind(&txn).fetch_optional(&state.pool).await?;
    let (job, name, created) = match open {
        Some((id, n)) => (id, n.unwrap_or_default(), false),
        None => { let (id, n) = crate::routes::registry::create_file_job(&state.pool, &actor.label, &txn, Some(&file), "wf.correct-aumentum").await?; (id, n, true) }
    };
    let result = reopen_aumentum(&state, &actor.label, job, pid, &upin, &attr, &reason, &txn, created).await;
    if result.is_err() && created {
        let _ = sqlx::query("DELETE FROM cadastre.wf_job_instance WHERE id = $1").bind(job).execute(&state.pool).await;
    }
    let commit_task = result?;
    Ok(Json(json!({ "upin": upin, "parcel_id": pid, "job_instance_id": job, "job_name": name, "created": created, "transaction_number": txn, "reopened_at_task": commit_task, "beacons": [] })))
}

#[allow(clippy::too_many_arguments)]
async fn reopen_aumentum(state: &AppState, actor: &str, job: i32, pid: uuid::Uuid, upin: &str, attr: &sqlx::postgres::PgRow, reason: &str, txn: &str, created: bool) -> ApiResult<i32> {
    let mut tx = state.pool.begin().await?;
    let w = sqlx::query("SELECT job_type_id, cadastre_job_id, status FROM cadastre.wf_job_instance WHERE id = $1 FOR UPDATE").bind(job).fetch_one(&mut *tx).await?;
    let (job_type, cjob, was): (i32, Option<uuid::Uuid>, i16) = (w.get(0), w.get(1), w.get(2));
    let cjob = match cjob {
        Some(c) => c,
        None => {
            let c: uuid::Uuid = sqlx::query_scalar(
                "INSERT INTO cadastre.job (reference, job_type, title, created_by, updated_by, current_stage_id, status) \
                 SELECT 'WF-' || w.id, 'aumentum:' || jt.name, w.name, $2, $2, s.id, s.code \
                 FROM cadastre.wf_job_instance w JOIN cadastre.wf_job_type jt ON jt.id = w.job_type_id, cadastre.workflow_stage s \
                 WHERE w.id = $1 AND s.is_initial RETURNING id")
                .bind(job).bind(actor).fetch_one(&mut *tx).await?;
            sqlx::query("UPDATE cadastre.wf_job_instance SET cadastre_job_id = $2, updated = now()::timestamp WHERE id = $1").bind(job).bind(c).execute(&mut *tx).await?;
            c
        }
    };
    let commit_task: i32 = sqlx::query_scalar(
        "SELECT f.task_id FROM cadastre.wf_task_flow f JOIN cadastre.wf_task_step ts ON ts.task_id = f.task_id JOIN cadastre.wf_step s ON s.id = ts.step_id \
         WHERE f.job_type_id = $1 AND EXISTS (SELECT 1 FROM unnest(s.commands) c WHERE upper(c) = upper($2)) ORDER BY f.task_id LIMIT 1")
        .bind(job_type).bind(crate::routes::wf::COMMIT_TOOL).fetch_optional(&mut *tx).await?
        .ok_or_else(|| AppError::Conflict("the file's job type has no commit task".into()))?;
    let rec: Value = attr.get("rec");
    let t = |k: &str| rec.get(k).and_then(|v| v.as_str()).map(str::to_string);
    sqlx::query(
        "INSERT INTO cadastre.parcel_draft_attribute (parcel_id, property_number, property_type, plot, block, location, plan_number, lga, parcel_name, area_unit, surveyor_general, status, layout, area_name) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, 'Active', $12, $13) \
         ON CONFLICT (parcel_id) DO UPDATE SET property_number = EXCLUDED.property_number, property_type = EXCLUDED.property_type, plot = EXCLUDED.plot, \
           block = EXCLUDED.block, location = EXCLUDED.location, plan_number = EXCLUDED.plan_number, lga = EXCLUDED.lga, parcel_name = EXCLUDED.parcel_name, \
           area_unit = EXCLUDED.area_unit, surveyor_general = EXCLUDED.surveyor_general, layout = EXCLUDED.layout, area_name = EXCLUDED.area_name")
        .bind(pid).bind(t("property_number")).bind(t("property_type")).bind(t("plot")).bind(t("block")).bind(t("location"))
        .bind(t("plan_number")).bind(t("lga")).bind(t("parcel_name")).bind(rec.get("area_unit").and_then(|v| v.as_i64()).map(|v| v as i16))
        .bind(t("surveyor_general")).bind(t("layout")).bind(t("area_name"))
        .execute(&mut *tx).await?;
    sqlx::query("DELETE FROM cadastre.parcel_attribute WHERE object_id = $1").bind(attr.get::<i32, _>("object_id")).execute(&mut *tx).await?;
    // Back to a draft of the file's job that keeps its UPIN (the ledger already holds it).
    sqlx::query("UPDATE cadastre.parcel SET status = 'draft', posted_at = NULL, job_id = $2, updated_at = now() WHERE id = $1")
        .bind(pid).bind(cjob).execute(&mut *tx).await?;
    sqlx::query("UPDATE cadastre.wf_task_instance SET ended = now()::timestamp, comment = coalesce(comment, 'moved to the commit task for a correction') \
                 WHERE job_instance_id = $1 AND ended IS NULL AND task_id <> $2").bind(job).bind(commit_task).execute(&mut *tx).await?;
    let at_commit: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM cadastre.wf_task_instance WHERE job_instance_id = $1 AND task_id = $2 AND ended IS NULL)")
        .bind(job).bind(commit_task).fetch_one(&mut *tx).await?;
    if !at_commit {
        let sid: Option<i32> = crate::routes::wf::default_assignee(&mut tx, commit_task).await?;
        sqlx::query("INSERT INTO cadastre.wf_task_instance (job_instance_id, job_type_id, task_id, sid, assigned, started) VALUES ($1, $2, $3, $4, now()::timestamp, now()::timestamp)")
            .bind(job).bind(job_type).bind(commit_task).bind(sid).execute(&mut *tx).await?;
    }
    sqlx::query("UPDATE cadastre.wf_job_instance SET status = 2, ended = NULL, current_task_id = $2, updated = now()::timestamp WHERE id = $1")
        .bind(job).bind(commit_task).execute(&mut *tx).await?;
    audit::append(&mut tx, actor, "wf_reopened_for_correction", "wf_job_instance", Some(&job.to_string()), json!({
        "upin": upin, "parcel_id": pid, "reason": reason, "was_status": was, "reopened_at_task": commit_task,
        "committed_in": "aumentum", "transaction_number": txn, "job_created": created,
        "beacons_removed": [], "aumentum_attribute_record": rec,
    })).await?;
    tx.commit().await?;
    Ok(commit_task)
}
