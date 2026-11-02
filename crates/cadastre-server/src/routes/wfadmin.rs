//! Phase 2 back office: job attachments, the productivity report, task notifications, and the
//! administration of users, groups, roles, task permissions, job types and settings (Aumentum's
//! Web Administrator).

use std::path::{Component, Path as FsPath, PathBuf};

use axum::body::{Body, Bytes};
use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::Row;
use utoipa::{IntoParams, ToSchema};

use crate::audit;
use crate::auth::AuthUser;
use crate::error::{ApiResult, AppError};
use crate::state::AppState;

// ---------------------------------------------------------------------------------------------
// Attachments
// ---------------------------------------------------------------------------------------------

/// Root of a file store. `aumentum`: Aumentum's document store (its `Server\data` folder, whose
/// device "default" holds the scans), from CADASTRE_AUMENTUM_FILES_DIR. `cadastre`: uploads made
/// in the new system, under CADASTRE_FILES_DIR (default `./files`).
fn store_root(store: &str) -> Option<PathBuf> {
    match store {
        "aumentum" => std::env::var("CADASTRE_AUMENTUM_FILES_DIR").ok().filter(|s| !s.is_empty()).map(PathBuf::from),
        "cadastre" => Some(PathBuf::from(std::env::var("CADASTRE_FILES_DIR").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "files".into()))),
        _ => None,
    }
}

/// Resolve a stored relative path under its root, refusing anything that could leave the root.
/// Aumentum paths start with the device name (`default/2015/05/23/...`), which is the root itself.
pub fn resolve_store_path(root: &FsPath, store: &str, rel: &str) -> Option<PathBuf> {
    let rel = rel.replace('\\', "/");
    let rel = if store == "aumentum" { rel.strip_prefix("default/").unwrap_or(&rel).to_string() } else { rel };
    let p = FsPath::new(&rel);
    if p.components().any(|c| !matches!(c, Component::Normal(_))) {
        return None;
    }
    Some(root.join(p))
}

#[derive(Serialize, ToSchema)]
pub struct WfFileDto {
    pub id: uuid::Uuid,
    pub file_name: String,
    pub size_bytes: Option<i64>,
    pub mime_type: Option<String>,
    pub uploaded_by: Option<String>,
    pub uploaded_at: Option<NaiveDateTime>,
    pub store: String,
    /// Whether the file can be opened from this server (Aumentum's store must be reachable).
    pub available: bool,
}

/// Files attached to a job instance (scanned plans etc.), oldest first.
#[utoipa::path(get, path = "/api/v1/wf/jobs/{id}/files", tag = "wf", params(("id" = i32, Path, description = "Job instance id")),
    responses((status = 200, body = [WfFileDto])))]
pub async fn list_files(State(state): State<AppState>, Path(id): Path<i32>) -> ApiResult<Json<Vec<WfFileDto>>> {
    let rows = sqlx::query(
        "SELECT f.id, f.file_name, f.size_bytes, f.mime_type, COALESCE(p.name, p.code, f.uploaded_by) AS uploaded_by, f.uploaded_at, f.store, f.store_path \
         FROM cadastre.wf_job_file f LEFT JOIN cadastre.principal p ON p.sid = f.uploaded_by_sid WHERE f.job_instance_id = $1 ORDER BY f.uploaded_at, f.file_name",
    )
    .bind(id)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(
        rows.iter()
            .map(|r| {
                let store: String = r.get("store");
                let path: String = r.get("store_path");
                let available = store_root(&store).and_then(|root| resolve_store_path(&root, &store, &path)).map(|p| p.is_file()).unwrap_or(false);
                WfFileDto {
                    id: r.get("id"),
                    file_name: r.get("file_name"),
                    size_bytes: r.get("size_bytes"),
                    mime_type: r.get("mime_type"),
                    uploaded_by: r.get("uploaded_by"),
                    uploaded_at: r.get("uploaded_at"),
                    store,
                    available,
                }
            })
            .collect(),
    ))
}

fn mime_for(name: &str, stored: Option<&str>) -> String {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg".into(),
        "png" => "image/png".into(),
        "tif" | "tiff" => "image/tiff".into(),
        "pdf" => "application/pdf".into(),
        // Aumentum stores every page as application/x-msdownload; the extension is more truthful.
        _ => stored.filter(|m| *m != "application/x-msdownload").unwrap_or("application/octet-stream").into(),
    }
}

/// A job file's name, stored mime type, bytes and job instance (shared with the Registry send).
pub async fn read_job_file(pool: &sqlx::PgPool, file_id: uuid::Uuid) -> ApiResult<(String, Option<String>, Vec<u8>, i32)> {
    let row = sqlx::query("SELECT file_name, store, store_path, mime_type, job_instance_id FROM cadastre.wf_job_file WHERE id = $1")
        .bind(file_id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("file {file_id}")))?;
    let (name, store, rel, mime, job): (String, String, String, Option<String>, i32) = (row.get(0), row.get(1), row.get(2), row.get(3), row.get(4));
    let root = store_root(&store).ok_or_else(|| AppError::NotFound(format!(
        "{name}: Aumentum's document store is not reachable from this server (set CADASTRE_AUMENTUM_FILES_DIR to its Server\\data folder)"
    )))?;
    let path = resolve_store_path(&root, &store, &rel).ok_or_else(|| AppError::BadRequest("invalid stored path".into()))?;
    let bytes = tokio::fs::read(&path).await.map_err(|_| AppError::NotFound(format!("{name} is not in the document store at {}", root.display())))?;
    Ok((name, mime, bytes, job))
}

/// Open an attached file.
#[utoipa::path(get, path = "/api/v1/wf/files/{file_id}", tag = "wf", params(("file_id" = uuid::Uuid, Path, description = "File id")),
    responses((status = 200, description = "The file"), (status = 404, description = "Unknown, or its store is not reachable from this server")))]
pub async fn get_file(State(state): State<AppState>, Path(file_id): Path<uuid::Uuid>) -> ApiResult<Response> {
    let (name, mime, bytes, _job) = read_job_file(&state.pool, file_id).await?;
    let disposition = format!("inline; filename=\"{}\"", name.replace('"', ""));
    Ok(([(header::CONTENT_TYPE, mime_for(&name, mime.as_deref())), (header::CONTENT_DISPOSITION, disposition)], Body::from(bytes)).into_response())
}

#[derive(Deserialize, IntoParams)]
pub struct UploadQuery {
    /// The file's name, e.g. PL68955.jpg.
    pub name: String,
}

/// Attach a file to a job (the request body is the file; up to 50 MB).
#[utoipa::path(post, path = "/api/v1/wf/jobs/{id}/files", tag = "wf", params(("id" = i32, Path, description = "Job instance id"), UploadQuery),
    request_body(content = String, content_type = "application/octet-stream", description = "The file's bytes"),
    responses((status = 201, body = WfFileDto), (status = 400, description = "Empty file or bad name")))]
pub async fn upload_file(State(state): State<AppState>, user: AuthUser, Path(id): Path<i32>, Query(q): Query<UploadQuery>, body: Bytes) -> ApiResult<(StatusCode, Json<WfFileDto>)> {
    let name = FsPath::new(&q.name).file_name().and_then(|n| n.to_str()).map(str::to_string).filter(|n| !n.is_empty())
        .ok_or_else(|| AppError::BadRequest("a file name is required".into()))?;
    if body.is_empty() {
        return Err(AppError::BadRequest("the file is empty".into()));
    }
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM cadastre.wf_job_instance WHERE id = $1)").bind(id).fetch_one(&state.pool).await?;
    if !exists {
        return Err(AppError::NotFound(format!("job instance {id}")));
    }
    let file_id = uuid::Uuid::new_v4();
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    let rel = format!("{}/{file_id}{}", chrono::Utc::now().format("%Y/%m"), if ext.is_empty() { String::new() } else { format!(".{ext}") });
    let root = store_root("cadastre").expect("cadastre store always has a root");
    let path = root.join(&rel);
    tokio::fs::create_dir_all(path.parent().unwrap()).await.map_err(|e| AppError::BadRequest(format!("cannot write the file store: {e}")))?;
    tokio::fs::write(&path, &body).await.map_err(|e| AppError::BadRequest(format!("cannot write the file store: {e}")))?;
    let who = user.username.clone().unwrap_or_else(|| user.subject.clone());
    let mut tx = state.pool.begin().await?;
    sqlx::query(
        "INSERT INTO cadastre.wf_job_file (id, job_instance_id, file_name, store, store_path, size_bytes, mime_type, uploaded_by, uploaded_at) \
         VALUES ($1, $2, $3, 'cadastre', $4, $5, $6, $7, now()::timestamp)",
    )
    .bind(file_id)
    .bind(id)
    .bind(&name)
    .bind(&rel)
    .bind(body.len() as i64)
    .bind(mime_for(&name, None))
    .bind(&who)
    .execute(&mut *tx)
    .await?;
    audit::append(&mut tx, &who, "wf_file_upload", "wf_job_instance", Some(&id.to_string()), json!({ "file": name, "bytes": body.len() })).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(WfFileDto { id: file_id, file_name: name.clone(), size_bytes: Some(body.len() as i64), mime_type: Some(mime_for(&name, None)), uploaded_by: Some(who), uploaded_at: None, store: "cadastre".into(), available: true })))
}

// ---------------------------------------------------------------------------------------------
// Productivity report (Aumentum: productivity log, Web Administrator reports)
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, IntoParams)]
pub struct PeriodQuery {
    /// First day (YYYY-MM-DD), inclusive. Default: 30 days ago.
    pub from: Option<chrono::NaiveDate>,
    /// Last day (YYYY-MM-DD), inclusive. Default: today.
    pub to: Option<chrono::NaiveDate>,
}
#[derive(Serialize, ToSchema)]
pub struct ProductivityRowDto {
    pub sid: i32,
    pub user: String,
    /// Tasks finished in the period.
    pub tasks_completed: i64,
    /// Median hours from a task's assignment to its end, in the period.
    pub median_task_hours: Option<f64>,
    /// Tasks assigned now and not finished.
    pub open_tasks: i64,
    /// Jobs whose commit task this user finished in the period.
    pub commits: i64,
    /// Aumentum events in the period (parcel created / mutated / status changed, job created).
    pub parcels_created: i64,
    pub parcels_mutated: i64,
    pub jobs_created: i64,
}

/// Work done per user in a period, from the task history and Aumentum's event log.
#[utoipa::path(get, path = "/api/v1/wf/productivity", tag = "wf", params(PeriodQuery), responses((status = 200, body = [ProductivityRowDto])))]
pub async fn productivity(State(state): State<AppState>, Query(q): Query<PeriodQuery>) -> ApiResult<Json<Vec<ProductivityRowDto>>> {
    let to = q.to.unwrap_or_else(|| chrono::Local::now().date_naive());
    let from = q.from.unwrap_or(to - chrono::Duration::days(30));
    let rows = sqlx::query(
        "WITH t AS (SELECT ti.sid, ti.task_id, ti.assigned, ti.started, ti.ended FROM cadastre.wf_task_instance ti WHERE ti.sid IS NOT NULL), \
              commit_tasks AS (SELECT DISTINCT ts.task_id FROM cadastre.wf_task_step ts JOIN cadastre.wf_step s ON s.id = ts.step_id \
                               WHERE EXISTS (SELECT 1 FROM unnest(s.commands) c WHERE upper(c) = upper($3))), \
              done AS (SELECT sid, count(*) AS n, percentile_cont(0.5) WITHIN GROUP (ORDER BY extract(epoch FROM ended - COALESCE(assigned, started)) / 3600.0) AS med, \
                              count(*) FILTER (WHERE task_id IN (SELECT task_id FROM commit_tasks)) AS commits \
                       FROM t WHERE ended >= $1 AND ended < $2 + 1 GROUP BY sid), \
              open AS (SELECT sid, count(*) AS n FROM t WHERE ended IS NULL GROUP BY sid), \
              ev AS (SELECT user_sid AS sid, count(*) FILTER (WHERE event_id = 1) AS created, count(*) FILTER (WHERE event_id IN (3)) AS mutated, \
                            count(*) FILTER (WHERE event_id = 7) AS jobs FROM cadastre.aumentum_event_log WHERE time >= $1 AND time < $2 + 1 GROUP BY user_sid) \
         SELECT p.sid, COALESCE(p.name, p.code) AS who, COALESCE(done.n, 0) AS done, done.med, COALESCE(open.n, 0) AS open, COALESCE(done.commits, 0) AS commits, \
                COALESCE(ev.created, 0) AS created, COALESCE(ev.mutated, 0) AS mutated, COALESCE(ev.jobs, 0) AS jobs \
         FROM cadastre.principal p LEFT JOIN done ON done.sid = p.sid LEFT JOIN open ON open.sid = p.sid LEFT JOIN ev ON ev.sid = p.sid \
         WHERE p.is_user AND (done.n IS NOT NULL OR open.n IS NOT NULL OR ev.sid IS NOT NULL) ORDER BY COALESCE(done.n, 0) DESC, who",
    )
    .bind(from)
    .bind(to)
    .bind(crate::routes::wf::COMMIT_TOOL)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(rows.iter().map(|r| ProductivityRowDto {
        sid: r.get("sid"),
        user: r.get("who"),
        tasks_completed: r.get("done"),
        median_task_hours: r.get("med"),
        open_tasks: r.get("open"),
        commits: r.get("commits"),
        parcels_created: r.get("created"),
        parcels_mutated: r.get("mutated"),
        jobs_created: r.get("jobs"),
    }).collect()))
}

#[derive(Serialize, ToSchema)]
pub struct NotificationDto {
    pub id: i64,
    pub job_instance_id: Option<i32>,
    pub to: Option<String>,
    pub to_email: Option<String>,
    pub subject: String,
    pub status: String,
    pub detail: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Recent task notifications and whether they were sent.
#[utoipa::path(get, path = "/api/v1/wf/notifications", tag = "wf", responses((status = 200, body = [NotificationDto])))]
pub async fn notifications(State(state): State<AppState>) -> ApiResult<Json<Vec<NotificationDto>>> {
    let rows = sqlx::query(
        "SELECT n.id, n.job_instance_id, COALESCE(p.name, p.code) AS who, n.to_email, n.subject, n.status, n.detail, n.created_at \
         FROM cadastre.wf_notification n LEFT JOIN cadastre.principal p ON p.sid = n.to_sid ORDER BY n.id DESC LIMIT 200",
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(rows.iter().map(|r| NotificationDto { id: r.get("id"), job_instance_id: r.get("job_instance_id"), to: r.get("who"), to_email: r.get("to_email"), subject: r.get("subject"), status: r.get("status"), detail: r.get("detail"), created_at: r.get("created_at") }).collect()))
}

// ---------------------------------------------------------------------------------------------
// Administration (Aumentum Web Administrator): users, groups, roles, task permissions,
// job types, settings. Requires the admin role; every change is audited.
// ---------------------------------------------------------------------------------------------

#[derive(Serialize, ToSchema)]
pub struct AdminSecurityDto {
    pub principals: Vec<serde_json::Value>,
    pub memberships: Vec<(i32, i32)>,
    pub roles: Vec<(String, Option<String>)>,
    pub grants: Vec<(i32, String, bool)>,
    pub task_permissions: Vec<(i32, i32, bool)>,
}

/// Everything the security screens show.
#[utoipa::path(get, path = "/api/v1/admin/security", tag = "admin", responses((status = 200, body = AdminSecurityDto), (status = 403, description = "Requires admin")))]
pub async fn security(State(state): State<AppState>, user: AuthUser) -> ApiResult<Json<AdminSecurityDto>> {
    user.require_role("admin")?;
    let principals = sqlx::query_scalar::<_, serde_json::Value>(
        "SELECT json_build_object('sid', sid, 'code', code, 'name', name, 'is_user', is_user, 'is_locked', is_locked, 'is_system', is_system, \
                'os_account', os_account, 'email', email, 'department', department, 'job_title', job_title) FROM cadastre.principal ORDER BY is_user, lower(COALESCE(name, code))",
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(AdminSecurityDto {
        principals,
        memberships: sqlx::query_as("SELECT group_sid, member_sid FROM cadastre.principal_member ORDER BY 1, 2").fetch_all(&state.pool).await?,
        roles: sqlx::query_as("SELECT code, description FROM cadastre.role ORDER BY code").fetch_all(&state.pool).await?,
        grants: sqlx::query_as("SELECT sid, role, allow FROM cadastre.principal_role ORDER BY 1, 2").fetch_all(&state.pool).await?,
        task_permissions: sqlx::query_as("SELECT task_id, sid, is_default FROM cadastre.wf_task_permission ORDER BY 1, 2").fetch_all(&state.pool).await?,
    }))
}

#[derive(Deserialize, ToSchema)]
pub struct AdminChange {
    /// `member` (group_sid, member_sid), `grant` (sid, role), `task_permission` (task_id, sid,
    /// is_default), `principal` (sid, is_locked / email / name), `job_type` (id, active).
    pub kind: String,
    /// Add (true) or remove (false), for member / grant / task_permission.
    pub add: Option<bool>,
    pub group_sid: Option<i32>,
    pub member_sid: Option<i32>,
    pub sid: Option<i32>,
    pub role: Option<String>,
    pub allow: Option<bool>,
    pub task_id: Option<i32>,
    pub is_default: Option<bool>,
    pub is_locked: Option<bool>,
    pub email: Option<String>,
    pub name: Option<String>,
    pub id: Option<i32>,
    pub active: Option<bool>,
}

fn need<T>(v: Option<T>, what: &str) -> ApiResult<T> {
    v.ok_or_else(|| AppError::BadRequest(format!("{what} is required")))
}

/// Apply one administration change.
#[utoipa::path(post, path = "/api/v1/admin/change", tag = "admin", request_body = AdminChange, responses((status = 200, body = AdminSecurityDto), (status = 403, description = "Requires admin")))]
pub async fn change(State(state): State<AppState>, user: AuthUser, Json(c): Json<AdminChange>) -> ApiResult<Json<AdminSecurityDto>> {
    user.require_role("admin")?;
    let who = user.username.clone().unwrap_or_else(|| user.subject.clone());
    let mut tx = state.pool.begin().await?;
    let add = c.add.unwrap_or(true);
    match c.kind.as_str() {
        "member" => {
            let (g, m) = (need(c.group_sid, "group_sid")?, need(c.member_sid, "member_sid")?);
            if g == m {
                return Err(AppError::BadRequest("a group cannot contain itself".into()));
            }
            if add {
                sqlx::query("INSERT INTO cadastre.principal_member (group_sid, member_sid) VALUES ($1, $2) ON CONFLICT DO NOTHING").bind(g).bind(m).execute(&mut *tx).await?;
            } else {
                sqlx::query("DELETE FROM cadastre.principal_member WHERE group_sid = $1 AND member_sid = $2").bind(g).bind(m).execute(&mut *tx).await?;
            }
        }
        "grant" => {
            let (s, r) = (need(c.sid, "sid")?, need(c.role.clone(), "role")?);
            if add {
                sqlx::query("INSERT INTO cadastre.principal_role (sid, role, allow) VALUES ($1, $2, $3) ON CONFLICT (sid, role) DO UPDATE SET allow = EXCLUDED.allow")
                    .bind(s).bind(&r).bind(c.allow.unwrap_or(true)).execute(&mut *tx).await?;
            } else {
                sqlx::query("DELETE FROM cadastre.principal_role WHERE sid = $1 AND role = $2").bind(s).bind(&r).execute(&mut *tx).await?;
            }
        }
        "task_permission" => {
            let (t, s) = (need(c.task_id, "task_id")?, need(c.sid, "sid")?);
            if add {
                sqlx::query("INSERT INTO cadastre.wf_task_permission (task_id, sid, is_default) VALUES ($1, $2, $3) ON CONFLICT (task_id, sid) DO UPDATE SET is_default = EXCLUDED.is_default")
                    .bind(t).bind(s).bind(c.is_default.unwrap_or(false)).execute(&mut *tx).await?;
            } else {
                sqlx::query("DELETE FROM cadastre.wf_task_permission WHERE task_id = $1 AND sid = $2").bind(t).bind(s).execute(&mut *tx).await?;
            }
        }
        "principal" => {
            let s = need(c.sid, "sid")?;
            sqlx::query("UPDATE cadastre.principal SET is_locked = COALESCE($2, is_locked), email = COALESCE($3, email), name = COALESCE($4, name) WHERE sid = $1")
                .bind(s).bind(c.is_locked).bind(&c.email).bind(&c.name).execute(&mut *tx).await?;
        }
        "job_type" => {
            let id = need(c.id, "id")?;
            sqlx::query("UPDATE cadastre.wf_job_type SET active = $2 WHERE id = $1").bind(id).bind(need(c.active, "active")?).execute(&mut *tx).await?;
        }
        other => return Err(AppError::BadRequest(format!("unknown change kind '{other}'"))),
    }
    audit::append(&mut tx, &who, "admin_change", &c.kind, None, json!({
        "add": add, "group_sid": c.group_sid, "member_sid": c.member_sid, "sid": c.sid, "role": c.role, "allow": c.allow,
        "task_id": c.task_id, "is_default": c.is_default, "is_locked": c.is_locked, "email": c.email, "name": c.name, "id": c.id, "active": c.active,
    })).await?;
    tx.commit().await?;
    security(State(state), user).await
}

#[derive(Serialize, Deserialize, ToSchema)]
pub struct SettingDto {
    pub key: String,
    /// Secret values (passwords) are never returned: "****" when set.
    pub value: Option<String>,
    pub source: Option<String>,
}

fn is_secret(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    k.contains("password") || k.contains("pass") || k.contains("secret")
}

/// Application settings (Aumentum dsssettings). Secrets are masked.
#[utoipa::path(get, path = "/api/v1/admin/settings", tag = "admin", responses((status = 200, body = [SettingDto]), (status = 403, description = "Requires admin")))]
pub async fn settings(State(state): State<AppState>, user: AuthUser) -> ApiResult<Json<Vec<SettingDto>>> {
    user.require_role("admin")?;
    let rows = sqlx::query("SELECT key, value, source FROM cadastre.app_setting ORDER BY key").fetch_all(&state.pool).await?;
    Ok(Json(rows.iter().map(|r| {
        let key: String = r.get("key");
        let value: Option<String> = r.get("value");
        let value = if is_secret(&key) { value.filter(|v| !v.is_empty()).map(|_| "****".to_string()) } else { value };
        SettingDto { key, value, source: r.get("source") }
    }).collect()))
}

/// Set a setting.
#[utoipa::path(put, path = "/api/v1/admin/settings", tag = "admin", request_body = SettingDto, responses((status = 200, body = [SettingDto]), (status = 403, description = "Requires admin")))]
pub async fn put_setting(State(state): State<AppState>, user: AuthUser, Json(s): Json<SettingDto>) -> ApiResult<Json<Vec<SettingDto>>> {
    user.require_role("admin")?;
    let who = user.username.clone().unwrap_or_else(|| user.subject.clone());
    let mut tx = state.pool.begin().await?;
    sqlx::query("INSERT INTO cadastre.app_setting (key, value, source, updated_by, updated_at) VALUES ($1, $2, 'cadastre', $3, now()) \
                 ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, source = 'cadastre', updated_by = EXCLUDED.updated_by, updated_at = now()")
        .bind(&s.key).bind(&s.value).bind(&who).execute(&mut *tx).await?;
    // Never write a secret into the audit chain.
    audit::append(&mut tx, &who, "admin_setting", "app_setting", Some(&s.key), json!({ "value": if is_secret(&s.key) { json!("****") } else { json!(s.value) } })).await?;
    tx.commit().await?;
    settings(State(state), user).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_paths_cannot_leave_the_store() {
        let root = FsPath::new("/srv/aumentum/data");
        assert_eq!(resolve_store_path(root, "aumentum", "default/2015/05/23/0067/x.0.101").unwrap(), root.join("2015/05/23/0067/x.0.101"));
        assert_eq!(resolve_store_path(root, "aumentum", "default\\2015\\05\\a.jpg").unwrap(), root.join("2015/05/a.jpg"));
        assert!(resolve_store_path(root, "aumentum", "default/../../etc/passwd").is_none());
        assert!(resolve_store_path(root, "cadastre", "/etc/passwd").is_none());
        assert!(resolve_store_path(root, "cadastre", "2026/11/../../x").is_none());
    }

    #[test]
    fn secrets_are_recognised() {
        assert!(is_secret("mail.smtp_password"));
        assert!(is_secret("aumentum.Office.smtpPass"));
        assert!(!is_secret("editor.allow_overlap"));
    }

    #[test]
    fn mime_types_follow_the_file_name() {
        assert_eq!(mime_for("PL9455.jpg", Some("application/x-msdownload")), "image/jpeg");
        assert_eq!(mime_for("plan.PDF", None), "application/pdf");
    }
}
