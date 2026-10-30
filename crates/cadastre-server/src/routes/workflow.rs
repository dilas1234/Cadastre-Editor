//! Workflow configuration endpoints (FR-WF-01, FR-ADM-02).

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use sqlx::{PgPool, Row};

use crate::auth::AuthUser;
use crate::domain::TransitionDef;
use crate::error::{ApiResult, AppError};
use crate::models::{NewStage, NewTransition, StageDto, TransitionDto, WorkflowDto};
use crate::state::AppState;

/// Load all transitions as `TransitionDef`s (stage codes), for `resolve_transition`.
pub async fn load_transition_defs(pool: &PgPool) -> Result<Vec<TransitionDef>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT t.action, f.code AS from_stage, s.code AS to_stage, t.required_role \
         FROM cadastre.workflow_transition t \
         JOIN cadastre.workflow_stage f ON f.id = t.from_stage_id \
         JOIN cadastre.workflow_stage s ON s.id = t.to_stage_id \
         ORDER BY f.seq, t.action",
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .iter()
        .map(|r| TransitionDef {
            action: r.get("action"),
            from_stage: r.get("from_stage"),
            to_stage: r.get("to_stage"),
            required_role: r.get("required_role"),
        })
        .collect())
}

/// Get the full workflow definition.
#[utoipa::path(get, path = "/api/v1/workflow", tag = "workflow",
    responses((status = 200, body = WorkflowDto)))]
pub async fn get_workflow(State(state): State<AppState>) -> ApiResult<Json<WorkflowDto>> {
    let stage_rows =
        sqlx::query("SELECT code, name, seq, is_initial, is_terminal FROM cadastre.workflow_stage ORDER BY seq")
            .fetch_all(&state.pool)
            .await?;
    let stages = stage_rows
        .iter()
        .map(|r| StageDto {
            code: r.get("code"),
            name: r.get("name"),
            seq: r.get("seq"),
            is_initial: r.get("is_initial"),
            is_terminal: r.get("is_terminal"),
        })
        .collect();

    let transitions = load_transition_defs(&state.pool)
        .await?
        .into_iter()
        .map(|t| TransitionDto {
            action: t.action,
            from_stage: t.from_stage,
            to_stage: t.to_stage,
            required_role: t.required_role,
        })
        .collect();

    Ok(Json(WorkflowDto { stages, transitions }))
}

/// Add a workflow stage (admin).
#[utoipa::path(post, path = "/api/v1/workflow/stages", tag = "workflow",
    request_body = NewStage,
    responses((status = 201, body = StageDto), (status = 409, description = "Stage code exists")))]
pub async fn create_stage(
    State(state): State<AppState>,
    user: AuthUser,
    Json(new): Json<NewStage>,
) -> ApiResult<(StatusCode, Json<StageDto>)> {
    user.require_role("admin")?;

    let result = sqlx::query(
        "INSERT INTO cadastre.workflow_stage (code, name, seq, is_terminal) \
         VALUES ($1, $2, $3, $4) RETURNING code, name, seq, is_initial, is_terminal",
    )
    .bind(&new.code)
    .bind(&new.name)
    .bind(new.seq)
    .bind(new.is_terminal.unwrap_or(false))
    .fetch_one(&state.pool)
    .await;

    match result {
        Ok(r) => Ok((
            StatusCode::CREATED,
            Json(StageDto {
                code: r.get("code"),
                name: r.get("name"),
                seq: r.get("seq"),
                is_initial: r.get("is_initial"),
                is_terminal: r.get("is_terminal"),
            }),
        )),
        Err(e) if is_unique_violation(&e) => {
            Err(AppError::Conflict(format!("stage '{}' already exists", new.code)))
        }
        Err(e) => Err(e.into()),
    }
}

/// Add a workflow transition (admin).
#[utoipa::path(post, path = "/api/v1/workflow/transitions", tag = "workflow",
    request_body = NewTransition,
    responses(
        (status = 201, body = TransitionDto),
        (status = 400, description = "Unknown stage code"),
        (status = 409, description = "Transition for (stage, action) exists"),
    ))]
pub async fn create_transition(
    State(state): State<AppState>,
    user: AuthUser,
    Json(new): Json<NewTransition>,
) -> ApiResult<(StatusCode, Json<TransitionDto>)> {
    user.require_role("admin")?;

    let result = sqlx::query(
        "INSERT INTO cadastre.workflow_transition (action, from_stage_id, to_stage_id, required_role) \
         SELECT $1, f.id, t.id, $4 \
         FROM cadastre.workflow_stage f, cadastre.workflow_stage t \
         WHERE f.code = $2 AND t.code = $3 \
         RETURNING action, required_role",
    )
    .bind(&new.action)
    .bind(&new.from_stage)
    .bind(&new.to_stage)
    .bind(&new.required_role)
    .fetch_optional(&state.pool)
    .await;

    match result {
        // No row inserted → one of the stage codes did not exist.
        Ok(None) => Err(AppError::BadRequest(format!(
            "unknown stage code in transition {} -> {}",
            new.from_stage, new.to_stage
        ))),
        Ok(Some(_)) => Ok((
            StatusCode::CREATED,
            Json(TransitionDto {
                action: new.action,
                from_stage: new.from_stage,
                to_stage: new.to_stage,
                required_role: new.required_role,
            }),
        )),
        Err(e) if is_unique_violation(&e) => Err(AppError::Conflict(format!(
            "a '{}' transition already exists from '{}'",
            new.action, new.from_stage
        ))),
        Err(e) => Err(e.into()),
    }
}

/// Whether a sqlx error is a Postgres unique-constraint violation (SQLSTATE 23505).
pub(crate) fn is_unique_violation(e: &sqlx::Error) -> bool {
    e.as_database_error()
        .and_then(|d| d.code())
        .is_some_and(|c| c == "23505")
}
