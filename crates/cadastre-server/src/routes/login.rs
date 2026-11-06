//! Sign-in on this server (`CADASTRE_AUTH=local`): the login page, sessions and passwords.
//!
//! The Registry's users sign in with their Registry password, checked by the Registry itself
//! (`AuthenticationService`), and its groups say what they may open (`auth::registry_access`).
//! Staff who are not in the Registry use this server's accounts (`cadastre.principal`): Argon2
//! hashes, a temporary password set by an administrator and changed at first sign-in. Sysadmin is
//! for either admin group, Digitising and Title Deed for the working groups and Users, Charting
//! and Reporting for anyone signed in. Five wrong passwords lock that name for 15 minutes.

use argon2::password_hash::rand_core::{OsRng, RngCore};
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap};
use axum::response::{IntoResponse, Redirect, Response};
use axum::{Extension, Json};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::Row;
use utoipa::ToSchema;

use crate::audit;
use crate::auth::{self, AuthUser, LocalSession, SESSION_COOKIE};
use crate::config::AuthMode;
use crate::error::{ApiResult, AppError};
use crate::lrs::{LrsConfig, LrsError, Session};
use crate::state::AppState;

const MAX_FAILURES: i32 = 5;
const LOCK_MINUTES: i32 = 15;
const SESSION_HOURS: i32 = 12;
const REMEMBER_DAYS: i32 = 30;
const BAD_LOGIN: &str = "Incorrect username or password.";

fn hash_password(password: &str) -> Result<String, String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default().hash_password(password.as_bytes(), &salt).map(|h| h.to_string()).map_err(|e| e.to_string())
}

fn verify_password(password: &str, hash: &str) -> bool {
    PasswordHash::new(hash).is_ok_and(|h| Argon2::default().verify_password(password.as_bytes(), &h).is_ok())
}

/// Argon2 is deliberately slow: off the async threads.
async fn verify_blocking(password: String, hash: String) -> ApiResult<bool> {
    tokio::task::spawn_blocking(move || verify_password(&password, &hash))
        .await.map_err(|e| AppError::BadRequest(format!("the password could not be checked: {e}")))
}

async fn hash_blocking(password: String) -> ApiResult<String> {
    tokio::task::spawn_blocking(move || hash_password(&password))
        .await.map_err(|e| AppError::BadRequest(format!("the password could not be set: {e}")))?
        .map_err(AppError::BadRequest)
}

/// Why a new password is refused, if it is.
pub fn password_problem(password: &str, login: &str) -> Option<&'static str> {
    if password.chars().count() < 8 {
        return Some("Use at least 8 characters.");
    }
    if !password.chars().any(|c| c.is_alphabetic()) || !password.chars().any(|c| c.is_numeric()) {
        return Some("Use both letters and numbers.");
    }
    if password.eq_ignore_ascii_case(login) {
        return Some("The password cannot be your username.");
    }
    None
}

/// A temporary password to read out: no characters that are easily confused.
fn temporary_password() -> String {
    const CHARS: &[u8] = b"abcdefghjkmnpqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ";
    const DIGITS: &[u8] = b"23456789";
    let mut bytes = [0u8; 12];
    OsRng.fill_bytes(&mut bytes);
    bytes.iter().enumerate().map(|(i, b)| {
        let set = if i % 4 == 3 { DIGITS } else { CHARS };
        set[*b as usize % set.len()] as char
    }).collect()
}

fn session_cookie(token: &str, max_age: Option<i64>) -> String {
    // Not `Secure`: the server is reached over plain HTTP on the LAN.
    let age = max_age.map(|s| format!("; Max-Age={s}")).unwrap_or_default();
    format!("{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Lax{age}")
}

async fn log(state: &AppState, actor: &str, action: &str, login: &str, payload: Value) -> ApiResult<()> {
    let mut tx = state.pool.begin().await?;
    audit::append(&mut tx, actor, action, "auth", Some(login), payload).await?;
    tx.commit().await?;
    Ok(())
}

/// `/`: to the sign-in page, or to the editor while sign-in is off.
pub async fn root(State(state): State<AppState>) -> Redirect {
    Redirect::temporary(if state.config.auth_mode == AuthMode::Local { "/login" } else { "/digitising" })
}

/// The sign-in page.
pub async fn page(State(state): State<AppState>) -> Response {
    if state.config.auth_mode != AuthMode::Local {
        return Redirect::temporary("/digitising").into_response();
    }
    crate::routes::editor::serve("login/login.html")
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
    /// `sysadmin`, `digitising`, `titledeed` or `reporting`.
    pub view: String,
    #[serde(default)]
    pub remember: bool,
}

/// The Registry's answer for a login: it accepted the password, and these are its groups.
async fn registry_sign_in(login: &str, password: &str) -> Option<Vec<String>> {
    // Alfresco's guest needs no real password, and an empty password proves nothing.
    if password.is_empty() || login.eq_ignore_ascii_case("guest") {
        return None;
    }
    let service = LrsConfig::from_env()?;
    let own = LrsConfig { user: login.to_string(), password: password.to_string(), ..service.clone() };
    match Session::start(&own).await {
        Ok(s) => s.end().await,
        // A refusal, or the Registry cannot be reached: local accounts can still sign in.
        Err(e) => {
            if !matches!(e, LrsError::Fault(_)) {
                tracing::warn!("sign-in could not ask the Registry: {e}");
            }
            return None;
        }
    }
    // Groups through the server's account. Unreadable groups give no access beyond reporting.
    let groups = match Session::start(&service).await {
        Ok(s) => {
            let g = s.user_groups(login).await;
            s.end().await;
            g
        }
        Err(e) => Err(e),
    };
    Some(groups.unwrap_or_else(|e| {
        tracing::warn!("sign-in could not read the Registry groups of {login}: {e}");
        Vec::new()
    }))
}

/// The account of a Registry user, made at first sign-in; `None` if it is locked here.
async fn registry_principal(state: &AppState, login: &str) -> ApiResult<Option<(i32, String)>> {
    let mut tx = state.pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(4207011)").execute(&mut *tx).await?;
    let found = sqlx::query("SELECT sid, code, is_locked FROM cadastre.principal WHERE is_user AND lower(code) = lower($1)").bind(login).fetch_optional(&mut *tx).await?;
    if let Some(r) = found {
        return Ok((!r.get::<bool, _>("is_locked")).then(|| (r.get("sid"), r.get("code"))));
    }
    let sid: i32 = sqlx::query_scalar("SELECT GREATEST(1000000, COALESCE(max(sid), 0)) + 1 FROM cadastre.principal").fetch_one(&mut *tx).await?;
    sqlx::query("INSERT INTO cadastre.principal (sid, code, name, description, is_user, is_locked, is_system) VALUES ($1, $2, $2, 'Registry user', true, false, false)")
        .bind(sid).bind(login).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO cadastre.principal_member (group_sid, member_sid) SELECT sid, $1 FROM cadastre.principal WHERE lower(code) = 'everyone' AND NOT is_user ON CONFLICT DO NOTHING")
        .bind(sid).execute(&mut *tx).await?;
    audit::append(&mut tx, login, "auth_registry_user_added", "auth", Some(login), json!({ "sid": sid })).await?;
    tx.commit().await?;
    Ok(Some((sid, login.to_string())))
}

/// A wrong login: counts towards the lockout of that name, whether or not it exists.
async fn refuse(state: &AppState, name: &str, view: auth::View) -> ApiResult<Response> {
    sqlx::query("INSERT INTO cadastre.login_throttle (login, failed_count) VALUES ($1, 1) ON CONFLICT (login) DO UPDATE SET \
                 failed_count = CASE WHEN cadastre.login_throttle.failed_count + 1 >= $2 THEN 0 ELSE cadastre.login_throttle.failed_count + 1 END, \
                 locked_until = CASE WHEN cadastre.login_throttle.failed_count + 1 >= $2 THEN now() + make_interval(mins => $3) ELSE cadastre.login_throttle.locked_until END")
        .bind(name).bind(MAX_FAILURES).bind(LOCK_MINUTES).execute(&state.pool).await?;
    log(state, "anonymous", "auth_login_failed", name, json!({ "view": view.key() })).await?;
    Err(AppError::Unauthorized(BAD_LOGIN.into()))
}

/// Sign in: the Registry checks the password first, then this server's own accounts (staff who
/// are not in the Registry). Then the account must be allowed the chosen view.
#[utoipa::path(post, path = "/api/v1/auth/login", tag = "auth", request_body = LoginRequest,
    responses((status = 200), (status = 401, description = "Wrong username or password"), (status = 403, description = "Account locked, or the view is not allowed")))]
pub async fn login(State(state): State<AppState>, Json(req): Json<LoginRequest>) -> ApiResult<Response> {
    if state.config.auth_mode != AuthMode::Local {
        return Err(AppError::Conflict("sign-in is not turned on on this server".into()));
    }
    let view = auth::View::parse(&req.view).ok_or_else(|| AppError::BadRequest("choose a workstation".into()))?;
    let typed: String = req.username.trim().chars().take(64).collect();
    let name = typed.rsplit('\\').next().unwrap_or(&typed).to_string();
    let short = name.to_lowercase();
    let locked: bool = sqlx::query_scalar("SELECT COALESCE((SELECT locked_until > now() FROM cadastre.login_throttle WHERE login = $1), false)")
        .bind(&short).fetch_one(&state.pool).await?;
    if locked {
        return Err(AppError::Forbidden(format!("Too many wrong passwords. Try again in {LOCK_MINUTES} minutes, or contact the ICT Unit.")));
    }
    let groups = if name.is_empty() { None } else { registry_sign_in(&name, &req.password).await };
    let from_registry = groups.is_some();
    let extra = groups.as_deref().map(auth::registry_access).unwrap_or(auth::Access { admin: false, staff: false });
    let (sid, code) = if from_registry {
        match registry_principal(&state, &name).await? {
            Some(p) => p,
            None => return refuse(&state, &short, view).await,
        }
    } else {
        let row = sqlx::query(
            "SELECT p.sid, p.code, c.password_hash FROM cadastre.principal p JOIN cadastre.user_credential c ON c.sid = p.sid \
             WHERE p.is_user AND NOT p.is_locked AND lower(p.code) = $1")
            .bind(&short).fetch_optional(&state.pool).await?;
        let Some(r) = row else {
            // The same work and the same answer as a wrong password: usernames cannot be probed.
            let _ = hash_blocking(req.password).await;
            return refuse(&state, &short, view).await;
        };
        if !verify_blocking(req.password, r.get("password_hash")).await? {
            return refuse(&state, &short, view).await;
        }
        (r.get("sid"), r.get("code"))
    };
    let local = auth::access_of(&state.pool, sid).await?;
    let access = auth::Access { admin: local.admin || extra.admin, staff: local.staff || extra.staff };
    if !view.allowed(access) {
        log(&state, &code, "auth_view_refused", &code, json!({ "view": view.key() })).await?;
        return Err(AppError::Forbidden(format!("Your account cannot open {}. Choose another workstation, or contact the ICT Unit.", view.label())));
    }
    let mut raw = [0u8; 32];
    OsRng.fill_bytes(&mut raw);
    let token: String = raw.iter().map(|b| format!("{b:02x}")).collect();
    let hours = if req.remember { REMEMBER_DAYS * 24 } else { SESSION_HOURS };
    let source = if from_registry { "registry" } else { "local" };
    let mut tx = state.pool.begin().await?;
    sqlx::query("DELETE FROM cadastre.user_session WHERE expires_at < now()").execute(&mut *tx).await?;
    sqlx::query("DELETE FROM cadastre.login_throttle WHERE login = $1").bind(&short).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO cadastre.user_session (token_hash, sid, view, expires_at, source, registry_admin, registry_staff) VALUES ($1, $2, $3, now() + make_interval(hours => $4), $5, $6, $7)")
        .bind(auth::token_hash(&token)).bind(sid).bind(view.key()).bind(hours).bind(source).bind(extra.admin).bind(extra.staff).execute(&mut *tx).await?;
    let must_change: bool = if from_registry {
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM cadastre.registry_password_reset WHERE login = $1)").bind(&short).fetch_one(&mut *tx).await?
    } else {
        sqlx::query_scalar("UPDATE cadastre.user_credential SET last_login = now() WHERE sid = $1 RETURNING must_change").bind(sid).fetch_one(&mut *tx).await?
    };
    audit::append(&mut tx, &code, "auth_login", "auth", Some(&code), json!({ "view": view.key(), "remember": req.remember, "source": source })).await?;
    tx.commit().await?;
    let cookie = session_cookie(&token, req.remember.then_some(hours as i64 * 3600));
    Ok(([(header::SET_COOKIE, cookie)], Json(json!({ "username": code, "must_change": must_change, "redirect": view.path() }))).into_response())
}

/// Sign out: the session ends on the server too.
#[utoipa::path(post, path = "/api/v1/auth/logout", tag = "auth", responses((status = 200)))]
pub async fn logout(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Response> {
    if let Some(token) = auth::cookie_token(&headers) {
        let gone = sqlx::query("DELETE FROM cadastre.user_session s USING cadastre.principal p WHERE p.sid = s.sid AND s.token_hash = $1 RETURNING p.code")
            .bind(auth::token_hash(&token)).fetch_optional(&state.pool).await?;
        if let Some(r) = gone {
            let code: String = r.get("code");
            log(&state, &code, "auth_logout", &code, json!({})).await?;
        }
    }
    Ok(([(header::SET_COOKIE, session_cookie("", Some(0)))], Json(json!({ "ok": true }))).into_response())
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

/// Change the signed-in user's own password (required after a temporary one). The user's other
/// sessions end.
#[utoipa::path(post, path = "/api/v1/auth/change-password", tag = "auth", request_body = ChangePasswordRequest,
    responses((status = 200), (status = 400, description = "The new password is refused"), (status = 401, description = "Wrong current password")))]
pub async fn change_password(State(state): State<AppState>, session: Option<Extension<LocalSession>>, headers: HeaderMap, Json(req): Json<ChangePasswordRequest>) -> ApiResult<Json<Value>> {
    let Some(Extension(s)) = session else { return Err(AppError::Conflict("sign-in is not turned on on this server".into())) };
    if let Some(why) = password_problem(&req.new_password, &s.code) {
        return Err(AppError::BadRequest(why.into()));
    }
    if req.new_password == req.current_password {
        return Err(AppError::BadRequest("Choose a password different from the current one.".into()));
    }
    if s.from_registry {
        return change_registry_password(&state, &s.code, req.current_password, req.new_password).await;
    }
    let hash: String = sqlx::query_scalar("SELECT password_hash FROM cadastre.user_credential WHERE sid = $1").bind(s.sid).fetch_one(&state.pool).await?;
    if !verify_blocking(req.current_password, hash).await? {
        return Err(AppError::Unauthorized("The current password is not correct.".into()));
    }
    let new_hash = hash_blocking(req.new_password).await?;
    let keep = auth::cookie_token(&headers).map(|t| auth::token_hash(&t)).unwrap_or_default();
    let mut tx = state.pool.begin().await?;
    sqlx::query("UPDATE cadastre.user_credential SET password_hash = $2, must_change = false, set_by = $3, updated_at = now() WHERE sid = $1")
        .bind(s.sid).bind(new_hash).bind(&s.code).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM cadastre.user_session WHERE sid = $1 AND token_hash <> $2").bind(s.sid).bind(keep).execute(&mut *tx).await?;
    audit::append(&mut tx, &s.code, "auth_password_changed", "auth", Some(&s.code), json!({})).await?;
    tx.commit().await?;
    Ok(Json(json!({ "ok": true })))
}

/// A Registry user's password changes in the Registry, in their own session there.
async fn change_registry_password(state: &AppState, code: &str, current: String, new: String) -> ApiResult<Json<Value>> {
    let service = LrsConfig::from_env().ok_or_else(|| AppError::Conflict("the Registry connection is not configured on this server".into()))?;
    let own = LrsConfig { user: code.to_string(), password: current.clone(), ..service };
    let session = Session::start(&own).await.map_err(|e| match e {
        LrsError::Fault(_) => AppError::Unauthorized("The current password is not correct.".into()),
        other => AppError::UnprocessableEntity(format!("the Registry could not be reached: {other}")),
    })?;
    let changed = session.change_password(code, &current, &new).await;
    session.end().await;
    changed.map_err(|e| AppError::UnprocessableEntity(format!("the Registry did not change the password: {e}")))?;
    let mut tx = state.pool.begin().await?;
    sqlx::query("DELETE FROM cadastre.registry_password_reset WHERE login = lower($1)").bind(code).execute(&mut *tx).await?;
    audit::append(&mut tx, code, "auth_password_changed", "auth", Some(code), json!({ "source": "registry" })).await?;
    tx.commit().await?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CredentialDto {
    pub sid: i32,
    pub must_change: bool,
    pub last_login: Option<chrono::DateTime<chrono::Utc>>,
}

/// Which accounts have a password (Sysadmin → Users & Groups). No hashes.
#[utoipa::path(get, path = "/api/v1/admin/credentials", tag = "admin", responses((status = 200, body = [CredentialDto]), (status = 403, description = "Not an administrator")))]
pub async fn credentials(State(state): State<AppState>, user: AuthUser) -> ApiResult<Json<Vec<CredentialDto>>> {
    user.require_role("admin")?;
    let rows = sqlx::query("SELECT sid, must_change, last_login FROM cadastre.user_credential").fetch_all(&state.pool).await?;
    Ok(Json(rows.iter().map(|r| CredentialDto { sid: r.get("sid"), must_change: r.get("must_change"), last_login: r.get("last_login") }).collect()))
}

/// Set a temporary password: shown once, and its owner must change it at first sign-in. The
/// account's lock and its sessions are cleared.
#[utoipa::path(post, path = "/api/v1/admin/users/{sid}/password", tag = "admin", params(("sid" = i32, Path, description = "The user")),
    responses((status = 200), (status = 403, description = "Not an administrator"), (status = 404, description = "No such user")))]
pub async fn set_temporary_password(State(state): State<AppState>, user: AuthUser, Path(sid): Path<i32>) -> ApiResult<Json<Value>> {
    user.require_role("admin")?;
    let by = user.username.clone().unwrap_or_else(|| user.subject.clone());
    let code: String = sqlx::query_scalar("SELECT code FROM cadastre.principal WHERE sid = $1 AND is_user")
        .bind(sid).fetch_optional(&state.pool).await?.ok_or_else(|| AppError::NotFound(format!("user {sid}")))?;
    let password = temporary_password();
    let hash = hash_blocking(password.clone()).await?;
    let mut tx = state.pool.begin().await?;
    sqlx::query("INSERT INTO cadastre.user_credential (sid, password_hash, must_change, set_by) VALUES ($1, $2, true, $3) \
                 ON CONFLICT (sid) DO UPDATE SET password_hash = EXCLUDED.password_hash, must_change = true, set_by = EXCLUDED.set_by, updated_at = now()")
        .bind(sid).bind(hash).bind(&by).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM cadastre.user_session WHERE sid = $1").bind(sid).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM cadastre.login_throttle WHERE login = lower($1)").bind(&code).execute(&mut *tx).await?;
    audit::append(&mut tx, &by, "auth_password_set", "auth", Some(&code), json!({ "temporary": true })).await?;
    tx.commit().await?;
    Ok(Json(json!({ "username": code, "temporary_password": password })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passwords_hash_and_verify() {
        let h = hash_password("Plateau2026").unwrap();
        assert!(h.starts_with("$argon2"));
        assert!(verify_password("Plateau2026", &h));
        assert!(!verify_password("plateau2026", &h));
        assert!(!verify_password("x", "not a hash"));
    }

    #[test]
    fn weak_passwords_are_refused() {
        assert!(password_problem("short1", "ikem").is_some());
        assert!(password_problem("onlyletters", "ikem").is_some());
        assert!(password_problem("12345678", "ikem").is_some());
        assert!(password_problem("Ikem2026", "IKEM2026").is_some());
        assert!(password_problem("Plateau2026", "ikem").is_none());
    }

    #[test]
    fn temporary_passwords_pass_the_rules() {
        for _ in 0..50 {
            let p = temporary_password();
            assert_eq!(p.len(), 12);
            assert_eq!(password_problem(&p, "ikem"), None);
        }
    }
}
