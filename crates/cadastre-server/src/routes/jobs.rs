//! Job endpoints (FR-JOB-*) and workflow transitions (FR-WF-02/03).

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use sqlx::Row;
use uuid::Uuid;

use crate::auth::AuthUser;
use crate::domain::{resolve_transition, TransitionError};
use crate::error::{ApiResult, AppError};
use crate::models::{JobDto, JobEventDto, JobTransition, NewJob};
use crate::routes::workflow::{is_unique_violation, load_transition_defs};
use crate::state::AppState;

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

    tx.commit().await?;
    Ok(Json(row_to_job(&row)))
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
