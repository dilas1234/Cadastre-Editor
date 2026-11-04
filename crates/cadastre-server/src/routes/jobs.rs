//! Job endpoints (FR-JOB-*) and workflow transitions (FR-WF-02/03).

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use sqlx::Row;
use uuid::Uuid;

use serde_json::json;

use crate::auth::AuthUser;
use crate::domain::{resolve_transition, TransitionError};
use crate::error::{ApiResult, AppError};
use crate::models::{JobDto, JobEventDto, JobTransition, NewJob, PostResult};
use crate::routes::workflow::{is_unique_violation, load_transition_defs};
use crate::state::AppState;
use crate::{audit, numbering};

const JOB_COLS: &str = "id, reference, job_type, title, status, assigned_to, created_by, \
     created_at, updated_at";

fn row_to_job(row: &sqlx::postgres::PgRow) -> JobDto {
    JobDto {
        id: row.get("id"),
        reference: row.get("reference"),
        job_type: row.get("job_type"),
        title: row.get("title"),
        status: row.get("status"),
        assigned_to: row.get("assigned_to"),
        created_by: row.get("created_by"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

/// List jobs, newest first.
#[utoipa::path(get, path = "/api/v1/jobs", tag = "jobs",
    responses((status = 200, body = [JobDto])))]
pub async fn list_jobs(State(state): State<AppState>) -> ApiResult<Json<Vec<JobDto>>> {
    let sql = format!("SELECT {JOB_COLS} FROM cadastre.job ORDER BY created_at DESC");
    let rows = sqlx::query(&sql).fetch_all(&state.pool).await?;
    Ok(Json(rows.iter().map(row_to_job).collect()))
}

/// Get one job.
#[utoipa::path(get, path = "/api/v1/jobs/{id}", tag = "jobs",
    params(("id" = Uuid, Path, description = "Job id")),
    responses((status = 200, body = JobDto), (status = 404, description = "Not found")))]
pub async fn get_job(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<JobDto>> {
    Ok(Json(load_job(&state, id).await?))
}

async fn load_job(state: &AppState, id: Uuid) -> ApiResult<JobDto> {
    let sql = format!("SELECT {JOB_COLS} FROM cadastre.job WHERE id = $1");
    let row = sqlx::query(&sql)
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("job {id}")))?;
    Ok(row_to_job(&row))
}

/// Create a job (requires `editor`). Starts at the workflow's initial stage.
#[utoipa::path(post, path = "/api/v1/jobs", tag = "jobs",
    request_body = NewJob,
    responses(
        (status = 201, body = JobDto),
        (status = 409, description = "Reference already exists"),
        (status = 403, description = "Requires editor role"),
    ))]
pub async fn create_job(
    State(state): State<AppState>,
    user: AuthUser,
    Json(new): Json<NewJob>,
) -> ApiResult<(StatusCode, Json<JobDto>)> {
    user.require_role("editor")?;

    let sql = format!(
        "INSERT INTO cadastre.job \
         (reference, job_type, title, assigned_to, created_by, updated_by, current_stage_id, status) \
         SELECT $1, $2, $3, $4, $5, $5, s.id, s.code \
         FROM cadastre.workflow_stage s WHERE s.is_initial \
         RETURNING {JOB_COLS}"
    );
    let result = sqlx::query(&sql)
        .bind(&new.reference)
        .bind(&new.job_type)
        .bind(&new.title)
        .bind(&new.assigned_to)
        .bind(&user.subject)
        .fetch_one(&state.pool)
        .await;

    match result {
        Ok(row) => Ok((StatusCode::CREATED, Json(row_to_job(&row)))),
        Err(e) if is_unique_violation(&e) => Err(AppError::Conflict(format!(
            "job reference '{}' already exists",
            new.reference
        ))),
        Err(e) => Err(e.into()),
    }
}

/// Move a job along the workflow. The action must be valid from the job's current stage
/// and the caller must hold the transition's required role.
#[utoipa::path(post, path = "/api/v1/jobs/{id}/transition", tag = "jobs",
    params(("id" = Uuid, Path, description = "Job id")),
    request_body = JobTransition,
    responses(
        (status = 200, body = JobDto),
        (status = 400, description = "No such transition from the current stage"),
        (status = 403, description = "Missing the required role"),
        (status = 404, description = "Job not found"),
    ))]
pub async fn transition_job(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
    Json(req): Json<JobTransition>,
) -> ApiResult<Json<JobDto>> {
    let current = load_job(&state, id).await?;
    let defs = load_transition_defs(&state.pool).await?;

    let transition = resolve_transition(&defs, &current.status, &req.action, &user.roles)
        .map_err(|e| match e {
            TransitionError::Unknown => AppError::BadRequest(format!(
                "no '{}' transition from stage '{}'",
                req.action, current.status
            )),
            TransitionError::Forbidden { required_role } => {
                AppError::Forbidden(format!("'{}' requires role '{required_role}'", req.action))
            }
        })?;
    let to_stage = transition.to_stage.clone();

    let mut tx = state.pool.begin().await?;
    let sql = format!(
        "UPDATE cadastre.job \
         SET status = $2, updated_by = $3, updated_at = now(), \
             current_stage_id = (SELECT id FROM cadastre.workflow_stage WHERE code = $2) \
         WHERE id = $1 RETURNING {JOB_COLS}"
    );
    let row = sqlx::query(&sql)
        .bind(id)
        .bind(&to_stage)
        .bind(&user.subject)
        .fetch_one(&mut *tx)
        .await?;

    sqlx::query(
        "INSERT INTO cadastre.job_event (job_id, action, from_stage, to_stage, actor, note) \
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(id)
    .bind(&req.action)
    .bind(&current.status)
    .bind(&to_stage)
    .bind(&user.subject)
    .bind(&req.note)
    .execute(&mut *tx)
    .await?;

    audit::append(
        &mut tx,
        &user.subject,
        "transition",
        "job",
        Some(&id.to_string()),
        json!({ "action": req.action, "to": to_stage }),
    )
    .await?;

    tx.commit().await?;
    Ok(Json(row_to_job(&row)))
}

/// Post a job: assign official numbers to its draft parcels and mark them active (FR-WF-04/05,
/// FR-NUM-01). Requires `approver`. Recorded in the hash-chained audit log.
#[utoipa::path(post, path = "/api/v1/jobs/{id}/post", tag = "jobs",
    params(("id" = Uuid, Path, description = "Job id")),
    responses(
        (status = 200, body = PostResult),
        (status = 403, description = "Requires approver role"),
        (status = 404, description = "Job not found"),
        (status = 409, description = "Job is not approved"),
    ))]
pub async fn post_job(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<PostResult>> {
    user.require_role("approver")?;
    let job = load_job(&state, id).await?;
    if !crate::domain::can_post(&job.status) {
        return Err(AppError::Conflict(format!(
            "job is in stage '{}'; only '{}' jobs can be posted",
            job.status,
            crate::domain::POSTABLE_STAGE
        )));
    }

    let mut tx = state.pool.begin().await?;
    let parcel_numbers = post_drafts(&mut tx, id).await?;

    audit::append(
        &mut tx,
        &user.subject,
        "post",
        "job",
        Some(&id.to_string()),
        json!({ "parcels_posted": parcel_numbers.len() as i64 }),
    )
    .await?;

    tx.commit().await?;
    Ok(Json(PostResult {
        posted: parcel_numbers.len(),
        parcel_numbers,
    }))
}

/// What a commit would give right now (shown in the Commit window before committing).
#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct UpinPreview {
    /// Draft parcels that still need a UPIN.
    pub drafts: i64,
    /// The UPINs a commit would give them, in draft order (advisory: the commit allocates again,
    /// atomically, so a commit made elsewhere in between moves them on).
    pub upins: Vec<String>,
    /// `checked` (Aumentum was asked about each), `not_configured` (no parallel-run check on this
    /// server) or `unavailable` (configured but Aumentum could not be asked: a commit is refused).
    pub aumentum: String,
    /// Numbers skipped because Aumentum has already issued them.
    pub aumentum_skipped: Vec<String>,
    pub message: Option<String>,
}

/// Preview the UPINs `post_drafts_with_ids` would allocate for `drafts` new parcels, without
/// using any: the next number in the local series past every issued UPIN, skipping numbers in the
/// ledger and (parallel run) numbers Aumentum has issued. Writes nothing.
pub async fn preview_upins(conn: &mut sqlx::PgConnection, drafts: i64) -> ApiResult<UpinPreview> {
    // The allocator's next value, read-only (sync_upin_sequence() would also move the sequence).
    let mut next: i64 = sqlx::query_scalar(
        "SELECT GREATEST( \
            (SELECT CASE WHEN is_called THEN last_value ELSE last_value - 1 END FROM cadastre.parcel_number_seq), \
            COALESCE((SELECT max(substring(upin FROM 3)::bigint) FROM cadastre.issued_upin \
                      WHERE upin ~ '^PL[0-9]{12}$' AND substring(upin FROM 3)::bigint < 1000000000), 0)) + 1",
    )
    .fetch_one(&mut *conn)
    .await?;
    let mut out = UpinPreview { drafts, upins: Vec::new(), aumentum: "not_configured".into(), aumentum_skipped: Vec::new(), message: None };
    if drafts <= 0 {
        return Ok(out);
    }
    let check = if crate::aumentum::UpinCheck::configured() {
        match crate::aumentum::UpinCheck::connect().await {
            Ok(c) => { out.aumentum = "checked".into(); c }
            Err(e) => { out.aumentum = "unavailable".into(); out.message = Some(e.replace("; nothing was committed", "")); None }
        }
    } else {
        None
    };
    let mut tries = 0;
    while (out.upins.len() as i64) < drafts {
        tries += 1;
        if tries > 50 + drafts {
            out.message = Some("no free UPIN found in 50 numbers".into());
            break;
        }
        let upin = numbering::format_upin(next);
        next += 1;
        let taken: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM cadastre.issued_upin WHERE upin = $1)")
            .bind(&upin)
            .fetch_one(&mut *conn)
            .await?;
        if taken {
            continue;
        }
        if let Some(c) = &check {
            match c.issued(&upin).await {
                Ok(true) => { out.aumentum_skipped.push(upin); continue; }
                Ok(false) => {}
                Err(e) => { out.aumentum = "unavailable".into(); out.message = Some(e.replace("; nothing was committed", "")); break; }
            }
        }
        out.upins.push(upin);
    }
    if let Some(c) = check {
        c.close().await;
    }
    Ok(out)
}

/// Post a job's draft parcels: give each its official UPIN (the next in the local series,
/// `PL` + 12 digits counting up; structured UPINs of 1,000,000,000 and over are kept in the ledger
/// but never drive the counter, migration 0018) and make it part of the live fabric. Shared by `POST /jobs/{id}/post` and the
/// Aumentum workflow's commit. Returns the UPINs, in draft order.
pub async fn post_drafts(conn: &mut sqlx::PgConnection, job_id: Uuid) -> ApiResult<Vec<String>> {
    Ok(post_drafts_with_ids(conn, job_id).await?.into_iter().map(|(_, n)| n).collect())
}

/// As [`post_drafts`], with each draft's id beside its UPIN (lineage needs which became which).
pub async fn post_drafts_with_ids(conn: &mut sqlx::PgConnection, job_id: Uuid) -> ApiResult<Vec<(Uuid, String)>> {
    let rows = sqlx::query(
        "SELECT id, parcel_number FROM cadastre.parcel \
         WHERE job_id = $1 AND status = 'draft' ORDER BY created_at",
    )
    .bind(job_id)
    .fetch_all(&mut *conn)
    .await?;

    // Keep the allocator above every local-series UPIN ever issued (imports included); the ledger
    // still refuses any UPIN issued before, structured ones included.
    sqlx::query("SELECT cadastre.sync_upin_sequence()").execute(&mut *conn).await?;

    let mut parcel_numbers = Vec::with_capacity(rows.len());
    let mut aumentum: Option<Option<crate::aumentum::UpinCheck>> = None;
    for row in &rows {
        let parcel_id: Uuid = row.get("id");
        let fresh = row.get::<Option<String>, _>("parcel_number").is_none();
        let number = match row.get::<Option<String>, _>("parcel_number") {
            Some(existing) => {
                sqlx::query(
                    "INSERT INTO cadastre.issued_upin (upin, source) VALUES ($1, 'posted') \
                     ON CONFLICT (upin) DO NOTHING",
                )
                .bind(&existing)
                .execute(&mut *conn)
                .await?;
                existing
            }
            None => {
                // Parallel run: ask Aumentum (once per commit) before giving a number it may have
                // issued since the data was copied; refuse the commit if it cannot be asked.
                if aumentum.is_none() {
                    aumentum = Some(crate::aumentum::UpinCheck::connect().await.map_err(AppError::Conflict)?);
                }
                let mut tries = 0;
                loop {
                    tries += 1;
                    if tries > 50 {
                        return Err(AppError::Conflict("no free UPIN found in 50 numbers; nothing was committed".into()));
                    }
                    // nextval is atomic: two officers committing at once never get the same number.
                    let seq: i64 = sqlx::query_scalar("SELECT nextval('cadastre.parcel_number_seq')")
                        .fetch_one(&mut *conn)
                        .await?;
                    let upin = numbering::format_upin(seq);
                    if let Some(Some(check)) = &aumentum {
                        if check.issued(&upin).await.map_err(AppError::Conflict)? {
                            // Aumentum's: record it so it is never offered again, and move on.
                            sqlx::query("INSERT INTO cadastre.issued_upin (upin, source) VALUES ($1, 'aumentum:live') ON CONFLICT (upin) DO NOTHING")
                                .bind(&upin)
                                .execute(&mut *conn)
                                .await?;
                            continue;
                        }
                    }
                    // A newly allocated UPIN must never have been issued before (registry links to a
                    // completed transaction would otherwise move to this parcel). The ledger's unique
                    // key settles any race; an issued number is skipped, never reused.
                    let recorded = sqlx::query(
                        "INSERT INTO cadastre.issued_upin (upin, source) VALUES ($1, 'posted') \
                         ON CONFLICT (upin) DO NOTHING",
                    )
                    .bind(&upin)
                    .execute(&mut *conn)
                    .await?
                    .rows_affected();
                    if recorded == 1 {
                        break upin;
                    }
                }
            }
        };
        // A new UPIN gets its attribute record here; so does a corrected parcel, which keeps its UPIN
        // but whose record went back to the draft (Correct). A pre-numbered parcel with a record keeps it.
        let needs_record = fresh || !sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM cadastre.parcel_attribute WHERE upin = $1)")
            .bind(&number).fetch_one(&mut *conn).await?;
        sqlx::query(
            "UPDATE cadastre.parcel \
             SET parcel_number = $2, status = 'posted', posted_at = now(), \
                 computed_area = ST_Area(geom), updated_at = now() \
             WHERE id = $1",
        )
        .bind(parcel_id)
        .bind(&number)
        .execute(&mut *conn)
        .await?;

        // Write the property-document attributes + Registry linkage for the new UPIN, merging what
        // was captured on the draft with the job's default linkage (its LRS transaction's
        // property). linkage_status = 1 (linked) when an LRSID is known, else 0 (pending). Status is
        // stored as Aumentum's code (0 Active, 1 Historic).
        sqlx::query(
            "INSERT INTO cadastre.parcel_attribute \
               (object_id, upin, parcel_id, dataset_name, region_name, property_number, property_type, \
                plot, block, location, plan_number, lga, parcel_name, area_unit, surveyor_general, \
                parcel_area, status, lrs_id, linkage_status, linkage_date, creation_date, last_edit_date, updated_at, layout, area_name) \
             SELECT nextval('cadastre.parcel_attribute_objectid_seq'), $2, p.id, \
                'PLATEAU_STATE_PARCELS', 'Plateau_State', \
                COALESCE(d.property_number, l.property_number), COALESCE(d.property_type, l.property_type), \
                COALESCE(d.plot, l.plot), COALESCE(d.block, l.block), d.location, \
                COALESCE(d.plan_number, l.plan_number), COALESCE(d.lga, l.lga), d.parcel_name, d.area_unit, \
                d.surveyor_general, trunc(ST_Area(p.geom)::numeric, 2), CASE lower(COALESCE(d.status, 'active')) WHEN 'active' THEN '0' WHEN 'historic' THEN '1' ELSE d.status END, COALESCE(d.lrs_id, l.lrs_id), \
                CASE WHEN COALESCE(d.lrs_id, l.lrs_id) IS NOT NULL THEN COALESCE(l.linkage_status, 1) ELSE 0 END, \
                CASE WHEN COALESCE(d.lrs_id, l.lrs_id) IS NOT NULL THEN to_char(now(), 'YYYYMMDD') END, \
                now(), now(), now(), d.layout, d.area_name \
             FROM cadastre.parcel p \
             LEFT JOIN cadastre.parcel_draft_attribute d ON d.parcel_id = p.id \
             LEFT JOIN LATERAL ( \
                 SELECT lk.* FROM cadastre.wf_job_linkage lk \
                 JOIN cadastre.wf_job_instance wi ON wi.id = lk.wf_job_instance_id \
                 WHERE wi.cadastre_job_id = p.job_id ORDER BY lk.updated_at DESC LIMIT 1 \
             ) l ON true \
             WHERE p.id = $1 AND $3 AND (d.parcel_id IS NOT NULL OR l.wf_job_instance_id IS NOT NULL)",
        )
        .bind(parcel_id)
        .bind(&number)
        .bind(needs_record)
        .execute(&mut *conn)
        .await?;
        sqlx::query("DELETE FROM cadastre.parcel_draft_attribute WHERE parcel_id = $1")
            .bind(parcel_id)
            .execute(&mut *conn)
            .await?;

        parcel_numbers.push((parcel_id, number));
    }

    if let Some(Some(check)) = aumentum {
        check.close().await;
    }
    Ok(parcel_numbers)
}

/// The workflow history of a job, oldest first.
#[utoipa::path(get, path = "/api/v1/jobs/{id}/events", tag = "jobs",
    params(("id" = Uuid, Path, description = "Job id")),
    responses((status = 200, body = [JobEventDto]), (status = 404, description = "Job not found")))]
pub async fn list_job_events(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<JobEventDto>>> {
    // 404 if the job does not exist, so an empty list is unambiguous.
    load_job(&state, id).await?;

    let rows = sqlx::query(
        "SELECT id, action, from_stage, to_stage, actor, note, created_at \
         FROM cadastre.job_event WHERE job_id = $1 ORDER BY id",
    )
    .bind(id)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(
        rows.iter()
            .map(|r| JobEventDto {
                id: r.get("id"),
                action: r.get("action"),
                from_stage: r.get("from_stage"),
                to_stage: r.get("to_stage"),
                actor: r.get("actor"),
                note: r.get("note"),
                created_at: r.get("created_at"),
            })
            .collect(),
    ))
}
