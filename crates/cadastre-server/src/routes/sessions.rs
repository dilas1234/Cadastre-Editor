//! Edit sessions and resource locks (concurrent-edit control).
//!
//! A session is a user's working context; it holds locks on resources. A resource can be
//! locked by at most one session. When a lock is contested, a stale holder (no heartbeat
//! within the TTL) or a non-active session is reclaimed; otherwise the request gets 409.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::{DateTime, Duration, Utc};
use sqlx::Row;
use uuid::Uuid;

use crate::auth::AuthUser;
use crate::domain::session_is_stale;
use crate::error::{ApiResult, AppError};
use crate::models::{LockDto, NewLock, NewSession, SessionDto};
use crate::routes::workflow::is_unique_violation;
use crate::state::AppState;

/// A session is considered stale (and its locks reclaimable) after this long without a heartbeat.
const SESSION_TTL_MINUTES: i64 = 5;

const SESSION_COLS: &str =
    "id, job_id, user_subject, status, started_at, last_heartbeat_at";

fn row_to_session(row: &sqlx::postgres::PgRow) -> SessionDto {
    SessionDto {
        id: row.get("id"),
        job_id: row.get("job_id"),
        user_subject: row.get("user_subject"),
        status: row.get("status"),
        started_at: row.get("started_at"),
        last_heartbeat_at: row.get("last_heartbeat_at"),
    }
}

fn row_to_lock(row: &sqlx::postgres::PgRow) -> LockDto {
    LockDto {
        id: row.get("id"),
        session_id: row.get("session_id"),
        resource_type: row.get("resource_type"),
        resource_id: row.get("resource_id"),
        acquired_at: row.get("acquired_at"),
    }
}

/// Open an edit session (requires `editor`).
#[utoipa::path(post, path = "/api/v1/sessions", tag = "sessions",
    request_body = NewSession,
    responses((status = 201, body = SessionDto), (status = 403, description = "Requires editor role")))]
pub async fn open_session(
    State(state): State<AppState>,
    user: AuthUser,
    Json(new): Json<NewSession>,
) -> ApiResult<(StatusCode, Json<SessionDto>)> {
    user.require_role("editor")?;
    let sql = format!(
        "INSERT INTO cadastre.edit_session (job_id, user_subject) VALUES ($1, $2) RETURNING {SESSION_COLS}"
    );
    let row = sqlx::query(&sql)
        .bind(new.job_id)
        .bind(&user.subject)
        .fetch_one(&state.pool)
        .await?;
    Ok((StatusCode::CREATED, Json(row_to_session(&row))))
}

/// Load a session, enforcing that the caller owns it.
async fn owned_session(state: &AppState, id: Uuid, user: &AuthUser) -> ApiResult<SessionDto> {
    let sql = format!("SELECT {SESSION_COLS} FROM cadastre.edit_session WHERE id = $1");
    let row = sqlx::query(&sql)
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("session {id}")))?;
    let session = row_to_session(&row);
    if session.user_subject != user.subject {
        return Err(AppError::Forbidden("not your session".to_string()));
    }
    Ok(session)
}

/// Keep a session alive. Owner only.
#[utoipa::path(post, path = "/api/v1/sessions/{id}/heartbeat", tag = "sessions",
    params(("id" = Uuid, Path, description = "Session id")),
    responses((status = 200, body = SessionDto), (status = 404, description = "Not found")))]
pub async fn heartbeat_session(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<SessionDto>> {
    owned_session(&state, id, &user).await?;
    let sql = format!(
        "UPDATE cadastre.edit_session SET last_heartbeat_at = now() \
         WHERE id = $1 AND status = 'active' RETURNING {SESSION_COLS}"
    );
    let row = sqlx::query(&sql)
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::Conflict("session is not active".to_string()))?;
    Ok(Json(row_to_session(&row)))
}

/// Release a session and drop all its locks. Owner only.
#[utoipa::path(post, path = "/api/v1/sessions/{id}/release", tag = "sessions",
    params(("id" = Uuid, Path, description = "Session id")),
    responses((status = 200, body = SessionDto), (status = 404, description = "Not found")))]
pub async fn release_session(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<SessionDto>> {
    owned_session(&state, id, &user).await?;
    let mut tx = state.pool.begin().await?;
    sqlx::query("DELETE FROM cadastre.edit_lock WHERE session_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    let sql = format!(
        "UPDATE cadastre.edit_session SET status = 'released', released_at = now() \
         WHERE id = $1 RETURNING {SESSION_COLS}"
    );
    let row = sqlx::query(&sql).bind(id).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok(Json(row_to_session(&row)))
}

/// List the locks held by a session. Owner only.
#[utoipa::path(get, path = "/api/v1/sessions/{id}/locks", tag = "sessions",
    params(("id" = Uuid, Path, description = "Session id")),
    responses((status = 200, body = [LockDto]), (status = 404, description = "Not found")))]
pub async fn list_locks(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<LockDto>>> {
    owned_session(&state, id, &user).await?;
    let rows = sqlx::query(
        "SELECT id, session_id, resource_type, resource_id, acquired_at \
         FROM cadastre.edit_lock WHERE session_id = $1 ORDER BY acquired_at",
    )
    .bind(id)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(rows.iter().map(row_to_lock).collect()))
}

/// Acquire a lock for a session. Reclaims a stale/dead holder; otherwise 409.
#[utoipa::path(post, path = "/api/v1/sessions/{id}/locks", tag = "sessions",
    params(("id" = Uuid, Path, description = "Session id")),
    request_body = NewLock,
    responses(
        (status = 201, body = LockDto),
        (status = 400, description = "Invalid resource_type"),
        (status = 409, description = "Resource locked by another active session"),
    ))]
pub async fn acquire_lock(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
    Json(new): Json<NewLock>,
) -> ApiResult<(StatusCode, Json<LockDto>)> {
    if !matches!(new.resource_type.as_str(), "parcel" | "beacon" | "job") {
        return Err(AppError::BadRequest(format!(
            "resource_type must be parcel|beacon|job, got '{}'",
            new.resource_type
        )));
    }
    let session = owned_session(&state, id, &user).await?;
    if session.status != "active" {
        return Err(AppError::Conflict("session is not active".to_string()));
    }

    let ttl = Duration::minutes(SESSION_TTL_MINUTES);
    let now = Utc::now();

    let mut tx = state.pool.begin().await?;

    // Is the resource already locked? Lock the row for the duration of the transaction.
    let existing = sqlx::query(
        "SELECT l.id AS lock_id, l.session_id, s.status AS session_status, \
                s.last_heartbeat_at \
         FROM cadastre.edit_lock l JOIN cadastre.edit_session s ON s.id = l.session_id \
         WHERE l.resource_type = $1 AND l.resource_id = $2 FOR UPDATE OF l",
    )
    .bind(&new.resource_type)
    .bind(new.resource_id)
    .fetch_optional(&mut *tx)
    .await?;

    if let Some(row) = existing {
        let holder_session: Uuid = row.get("session_id");
        if holder_session == id {
            // Already held by this session — idempotent.
            let lock_id: Uuid = row.get("lock_id");
            tx.commit().await?;
            return Ok((StatusCode::OK, Json(get_lock(&state, lock_id).await?)));
        }
        let holder_status: String = row.get("session_status");
        let holder_heartbeat: DateTime<Utc> = row.get("last_heartbeat_at");
        let reclaimable =
            holder_status != "active" || session_is_stale(holder_heartbeat, now, ttl);
        if !reclaimable {
            return Err(AppError::Conflict(format!(
                "{} {} is locked by another active session",
                new.resource_type, new.resource_id
            )));
        }
        // Reclaim: drop the stale lock and expire its (still-active) session.
        let lock_id: Uuid = row.get("lock_id");
        sqlx::query("DELETE FROM cadastre.edit_lock WHERE id = $1")
            .bind(lock_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "UPDATE cadastre.edit_session SET status = 'expired', released_at = now() \
             WHERE id = $1 AND status = 'active'",
        )
        .bind(holder_session)
        .execute(&mut *tx)
        .await?;
    }

    let insert = sqlx::query(
        "INSERT INTO cadastre.edit_lock (session_id, resource_type, resource_id) \
         VALUES ($1, $2, $3) RETURNING id, session_id, resource_type, resource_id, acquired_at",
    )
    .bind(id)
    .bind(&new.resource_type)
    .bind(new.resource_id)
    .fetch_one(&mut *tx)
    .await;

    match insert {
        Ok(row) => {
            tx.commit().await?;
            Ok((StatusCode::CREATED, Json(row_to_lock(&row))))
        }
        // A concurrent acquirer won the race between our check and insert.
        Err(e) if is_unique_violation(&e) => Err(AppError::Conflict(format!(
            "{} {} is locked by another active session",
            new.resource_type, new.resource_id
        ))),
        Err(e) => Err(e.into()),
    }
}

async fn get_lock(state: &AppState, lock_id: Uuid) -> ApiResult<LockDto> {
    let row = sqlx::query(
        "SELECT id, session_id, resource_type, resource_id, acquired_at \
         FROM cadastre.edit_lock WHERE id = $1",
    )
    .bind(lock_id)
    .fetch_one(&state.pool)
    .await?;
    Ok(row_to_lock(&row))
}

/// Release a single lock by id. Owner only.
#[utoipa::path(delete, path = "/api/v1/sessions/{id}/locks/{lock_id}", tag = "sessions",
    params(
        ("id" = Uuid, Path, description = "Session id"),
        ("lock_id" = Uuid, Path, description = "Lock id"),
    ),
    responses((status = 204, description = "Released"), (status = 404, description = "Not found")))]
pub async fn release_lock(
    State(state): State<AppState>,
    user: AuthUser,
    Path((id, lock_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    owned_session(&state, id, &user).await?;
    let done = sqlx::query("DELETE FROM cadastre.edit_lock WHERE id = $1 AND session_id = $2")
        .bind(lock_id)
        .bind(id)
        .execute(&state.pool)
        .await?;
    if done.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("lock {lock_id} on session {id}")));
    }
    Ok(StatusCode::NO_CONTENT)
}
