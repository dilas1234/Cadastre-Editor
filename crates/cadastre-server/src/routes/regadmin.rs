//! Registry (LRS43) user-administration endpoints (feature `registry-admin`), under
//! `/api/v1/admin/registry/*`. The gate already restricts `/api/v1/admin` to administrators; each
//! handler re-checks the `admin` role and passes the signed-in name as the audit actor.
//!
//! These drive the Sysadmin "Users & Units" dashboard: list users and groups, create a user into a
//! unit, move users between units, reset a password and enable/disable an account. Every write
//! lands in `plagis_admin.audit_log` on the Registry's SQL Server.
//!
//! Writes obey `CADASTRE_REGISTRY_WRITE`, like every other write to the Registry. A password set
//! here is temporary: its owner must replace it at their next sign-in (`routes::login`).

use std::future::Future;

use axum::extract::{Path, Query, State};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::audit;
use crate::auth::AuthUser;
use crate::error::{ApiResult, AppError};
use crate::regadmin::{self, CreateGroup, CreateUser, RegistryGroup, RegistryUser, UpdateUser};
use crate::routes::registry::write_config;
use crate::state::AppState;

/// Shown after any write: Alfresco caches users and groups, so a change needs a JBoss restart.
const RESTART_NOTE: &str = "Saved. Restart the Registry (JBoss) service for the change to take effect.";

/// Map a Registry-admin error onto an HTTP error. A procedure's refusal is shown to the
/// administrator; connection/configuration problems say so; anything else is logged as internal.
fn map(e: regadmin::RegAdminError) -> AppError {
    use regadmin::RegAdminError::*;
    match e {
        Refused(message) => AppError::BadRequest(message),
        NotConfigured => AppError::UnprocessableEntity(
            "Registry user administration is not configured on this server (CADASTRE_REGISTRY_SQL_*).".to_string(),
        ),
        Connect(detail) => AppError::UnprocessableEntity(format!("could not reach the Registry database: {detail}")),
        Db(detail) => AppError::Internal(format!("Registry database error: {detail}")),
    }
}

/// The configured Registry SQL connection, or a clear error when the dashboard is off.
fn cfg(state: &AppState) -> Result<&crate::config::RegistrySqlConfig, AppError> {
    state.config.registry_sql.as_ref().ok_or_else(|| map(regadmin::RegAdminError::NotConfigured))
}

/// The connection for a change only the `plagis_admin` procedures can make.
fn procedures(state: &AppState) -> Result<&crate::config::RegistrySqlConfig, AppError> {
    let c = cfg(state)?;
    if c.direct {
        return Err(AppError::Conflict("This change needs the plagis_admin procedures, which are not installed yet. Only adding a user works for now.".into()));
    }
    Ok(c)
}

/// The audit actor: the signed-in username (falls back to the subject).
fn actor(user: &AuthUser) -> String {
    user.username.clone().unwrap_or_else(|| user.subject.clone())
}

/// Run a Registry write that gives `login` a password chosen by an administrator, and list that
/// login in `cadastre.registry_password_reset` so its owner must replace it at sign-in. The row is
/// held in a transaction across the write: a refused write leaves no row, and a row that cannot be
/// saved stops the write, so a user never ends up with an administrator's password and no prompt.
pub async fn with_forced_change(pool: &sqlx::PgPool, login: &str, by: &str, write: impl Future<Output = ApiResult<()>>) -> ApiResult<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("INSERT INTO cadastre.registry_password_reset (login, set_by) VALUES ($1, $2) \
                 ON CONFLICT (login) DO UPDATE SET set_by = EXCLUDED.set_by, created_at = now()")
        .bind(login.trim().to_lowercase()).bind(by).execute(&mut *tx).await?;
    write.await?;
    tx.commit().await?;
    Ok(())
}

#[derive(Debug, Deserialize)]
pub struct UserSearch {
    #[serde(default)]
    pub search: Option<String>,
}

/// GET `/api/v1/admin/registry/users` — users with name, office, enabled flag and groups.
pub async fn list_users(State(state): State<AppState>, user: AuthUser, Query(q): Query<UserSearch>) -> ApiResult<Json<Vec<RegistryUser>>> {
    user.require_role("admin")?;
    let search = q.search.as_deref().map(str::trim).filter(|s| !s.is_empty());
    regadmin::list_users(cfg(&state)?, search).await.map(Json).map_err(map)
}

/// GET `/api/v1/admin/registry/groups` — groups ("units") with display name and member count.
pub async fn list_groups(State(state): State<AppState>, user: AuthUser) -> ApiResult<Json<Vec<RegistryGroup>>> {
    user.require_role("admin")?;
    regadmin::list_groups(cfg(&state)?).await.map(Json).map_err(map)
}

/// GET `/api/v1/admin/registry/users/{user}/groups` — the groups of one user.
pub async fn user_groups(State(state): State<AppState>, user: AuthUser, Path(name): Path<String>) -> ApiResult<Json<Vec<String>>> {
    user.require_role("admin")?;
    regadmin::user_groups(cfg(&state)?, &name).await.map(Json).map_err(map)
}

/// POST `/api/v1/admin/registry/users` — create a user by cloning a reference user.
pub async fn create_user(State(state): State<AppState>, user: AuthUser, Json(req): Json<CreateUser>) -> ApiResult<Json<Value>> {
    user.require_role("admin")?;
    write_config()?;
    let (sql, by) = (cfg(&state)?, actor(&user));
    with_forced_change(&state.pool, &req.user_name, &by, async { regadmin::create_user(sql, &req, &by).await.map_err(map) }).await?;
    // The procedures log to the Registry's audit table; without them the record is kept here.
    if sql.direct {
        let mut tx = state.pool.begin().await?;
        audit::append(&mut tx, &by, "registry_user_created", "auth", Some(&req.user_name.trim().to_lowercase()), json!({ "ref_user": req.ref_user, "units": req.extra_groups })).await?;
        tx.commit().await?;
    }
    Ok(Json(json!({ "user_name": req.user_name, "note": RESTART_NOTE })))
}

/// POST `/api/v1/admin/registry/groups` — create a group ("unit").
pub async fn create_group(State(state): State<AppState>, user: AuthUser, Json(req): Json<CreateGroup>) -> ApiResult<Json<Value>> {
    user.require_role("admin")?;
    write_config()?;
    regadmin::create_group(procedures(&state)?, &req, &actor(&user)).await.map_err(map)?;
    Ok(Json(json!({ "group_name": format!("GROUP_{}", req.display_name.trim()), "note": RESTART_NOTE })))
}

#[derive(Debug, Deserialize)]
pub struct GroupBody {
    pub group: String,
}

/// POST `/api/v1/admin/registry/users/{user}/groups` — add the user to a group.
pub async fn add_to_group(State(state): State<AppState>, user: AuthUser, Path(name): Path<String>, Json(req): Json<GroupBody>) -> ApiResult<Json<Value>> {
    user.require_role("admin")?;
    write_config()?;
    regadmin::add_to_group(procedures(&state)?, &name, &req.group, &actor(&user)).await.map_err(map)?;
    Ok(Json(json!({ "note": RESTART_NOTE })))
}

/// DELETE `/api/v1/admin/registry/users/{user}/groups/{group}` — remove the user from a group.
pub async fn remove_from_group(State(state): State<AppState>, user: AuthUser, Path((name, group)): Path<(String, String)>) -> ApiResult<Json<Value>> {
    user.require_role("admin")?;
    write_config()?;
    regadmin::remove_from_group(procedures(&state)?, &name, &group, &actor(&user)).await.map_err(map)?;
    Ok(Json(json!({ "note": RESTART_NOTE })))
}

#[derive(Debug, Deserialize)]
pub struct PasswordBody {
    pub password: String,
}

/// POST `/api/v1/admin/registry/users/{user}/password` — set a temporary password and unlock.
pub async fn reset_password(State(state): State<AppState>, user: AuthUser, Path(name): Path<String>, Json(req): Json<PasswordBody>) -> ApiResult<Json<Value>> {
    user.require_role("admin")?;
    write_config()?;
    let (sql, by) = (procedures(&state)?, actor(&user));
    with_forced_change(&state.pool, &name, &by, async { regadmin::reset_password(sql, &name, &req.password, &by).await.map_err(map) }).await?;
    Ok(Json(json!({ "note": RESTART_NOTE })))
}

#[derive(Debug, Deserialize)]
pub struct EnabledBody {
    pub enabled: bool,
}

/// POST `/api/v1/admin/registry/users/{user}/enabled` — disable or enable an account.
pub async fn set_enabled(State(state): State<AppState>, user: AuthUser, Path(name): Path<String>, Json(req): Json<EnabledBody>) -> ApiResult<Json<Value>> {
    user.require_role("admin")?;
    write_config()?;
    regadmin::set_enabled(procedures(&state)?, &name, req.enabled, &actor(&user)).await.map_err(map)?;
    Ok(Json(json!({ "note": RESTART_NOTE })))
}

/// PUT `/api/v1/admin/registry/users/{user}` — change first name, last name or office.
pub async fn update_user(State(state): State<AppState>, user: AuthUser, Path(name): Path<String>, Json(req): Json<UpdateUser>) -> ApiResult<Json<Value>> {
    user.require_role("admin")?;
    write_config()?;
    regadmin::update_user(procedures(&state)?, &name, &req, &actor(&user)).await.map_err(map)?;
    Ok(Json(json!({ "note": RESTART_NOTE })))
}

/// The Registry-admin routes, merged into the main router under the admin gate.
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/admin/registry/users", get(list_users).post(create_user))
        .route("/api/v1/admin/registry/groups", get(list_groups).post(create_group))
        .route("/api/v1/admin/registry/users/{user}", put(update_user))
        .route("/api/v1/admin/registry/users/{user}/groups", get(user_groups).post(add_to_group))
        .route("/api/v1/admin/registry/users/{user}/groups/{group}", axum::routing::delete(remove_from_group))
        .route("/api/v1/admin/registry/users/{user}/password", post(reset_password))
        .route("/api/v1/admin/registry/users/{user}/enabled", post(set_enabled))
}
