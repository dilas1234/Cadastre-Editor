//! Aumentum workflow API (Phase 2): job types, task graph, steps, job instances and their task
//! history, assignment and hold. Rules live in [`crate::wfengine`]; this module loads state,
//! applies a decision in one transaction, and records it in the audit chain.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{QueryBuilder, Row};
use utoipa::{IntoParams, ToSchema};

use crate::audit;
use crate::auth::AuthUser;
use crate::error::{ApiResult, AppError};
use crate::state::AppState;
use crate::wfengine::{self, Advance, AdvanceError, Edge};

// ---------------------------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------------------------

#[derive(Serialize, ToSchema)]
pub struct WfStepDto {
    pub id: i32,
    pub name: String,
    pub icon: Option<String>,
    /// The tools the step enables, in Aumentum's order (COM GUIDs or class names).
    pub commands: Vec<String>,
}
#[derive(Serialize, ToSchema)]
pub struct WfTaskDto {
    pub id: i32,
    pub name: String,
    pub description: Option<String>,
    pub steps: Vec<WfStepDto>,
    /// Principals granted the task (sids); empty means unrestricted.
    pub permitted_sids: Vec<i32>,
}
#[derive(Serialize, ToSchema)]
pub struct WfEdgeDto {
    pub task_id: i32,
    pub next_task_id: Option<i32>,
    pub task_order: Option<i32>,
}
#[derive(Serialize, ToSchema)]
pub struct WfJobTypeDto {
    pub id: i32,
    pub name: String,
    pub description: Option<String>,
    pub start_task_id: Option<i32>,
    pub end_task_id: Option<i32>,
    pub active: bool,
    pub flow: Vec<WfEdgeDto>,
}
#[derive(Serialize, ToSchema)]
pub struct WfStatusDto {
    pub code: i16,
    pub label: String,
    pub is_open: bool,
    /// Whether PLAGIS has confirmed the meaning of this Aumentum code.
    pub confirmed: bool,
}
#[derive(Serialize, ToSchema)]
pub struct WfDefinitionsDto {
    pub job_types: Vec<WfJobTypeDto>,
    pub tasks: Vec<WfTaskDto>,
    pub statuses: Vec<WfStatusDto>,
}

#[derive(Serialize, ToSchema)]
pub struct WfJobDto {
    pub id: i32,
    pub job_type_id: i32,
    pub job_type: String,
    pub current_task_id: Option<i32>,
    pub current_task: Option<String>,
    pub assigned_sid: Option<i32>,
    pub assigned_to: Option<String>,
    pub status: i16,
    pub status_label: String,
    pub is_open: bool,
    pub priority: Option<String>,
    pub name: Option<String>,
    pub comment: Option<String>,
    pub lrs_transaction_number: Option<String>,
    pub created: Option<NaiveDateTime>,
    pub started: Option<NaiveDateTime>,
    pub updated: Option<NaiveDateTime>,
    pub ended: Option<NaiveDateTime>,
    pub source: String,
}
#[derive(Serialize, ToSchema)]
pub struct WfTaskInstanceDto {
    pub id: i32,
    pub task_id: i32,
    pub task: String,
    pub sid: Option<i32>,
    pub assigned_to: Option<String>,
    pub assigned: Option<NaiveDateTime>,
    pub started: Option<NaiveDateTime>,
    pub ended: Option<NaiveDateTime>,
    pub comment: Option<String>,
}
#[derive(Serialize, ToSchema)]
pub struct WfTaskRefDto {
    pub id: i32,
    pub name: String,
}
#[derive(Serialize, ToSchema)]
pub struct WfJobDetailDto {
    pub job: WfJobDto,
    /// Tasks the job may move to next (a branch offers several; one may be a rework loop).
    pub next_tasks: Vec<WfTaskRefDto>,
    /// Whether the job may be completed from its current task.
    pub can_complete: bool,
    /// The current task's steps and the tools each enables.
    pub steps: Vec<WfStepDto>,
    /// When the current task can send the job on to plotting (Search Cadastre for Parcels →
    /// Plot Parcel), that task's steps too, so a parcel not yet in the cadastre can be drawn
    /// without first moving the job. Steps the current task already has are left out.
    pub plot_steps: Vec<WfStepDto>,
    /// The task `plot_steps` come from.
    pub plot_task: Option<WfTaskRefDto>,
    /// Task history, oldest first.
    pub history: Vec<WfTaskInstanceDto>,
    /// Registry properties and parcel links, as Aumentum stored them (XML).
    pub lrs_properties: Option<String>,
    pub link_parcels_properties: Option<String>,
}
#[derive(Serialize, ToSchema)]
pub struct WfJobListDto {
    pub total: i64,
    pub jobs: Vec<WfJobDto>,
}
#[derive(Serialize, ToSchema)]
pub struct WfPrincipalDto {
    pub sid: i32,
    pub code: String,
    pub name: Option<String>,
    pub is_user: bool,
    pub is_locked: bool,
    pub os_account: Option<String>,
}

#[derive(Deserialize, IntoParams)]
pub struct WfJobQuery {
    /// `true` (default): open jobs only; `false`: all.
    pub open: Option<bool>,
    /// A status code.
    pub status: Option<i16>,
    /// Assignee (principal sid).
    pub assigned: Option<i32>,
    /// Job type id.
    pub job_type: Option<i32>,
    /// Text search: job id, name, comment or Registry transaction number.
    pub q: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}
#[derive(Deserialize, ToSchema)]
pub struct NewWfJob {
    pub job_type_id: i32,
    pub name: Option<String>,
    pub comment: Option<String>,
    pub priority: Option<String>,
    /// The Registry transaction the job serves.
    pub lrs_transaction_number: Option<String>,
    /// Who the first task is assigned to (sid); default: the task's default assignee.
    pub assigned_sid: Option<i32>,
}
#[derive(Deserialize, ToSchema)]
pub struct WfAdvance {
    /// The task to move to; omit to complete the job (at its end task) or to take the only path.
    pub to_task_id: Option<i32>,
    /// Comment recorded on the finished task.
    pub comment: Option<String>,
    /// Assignee of the next task (sid); default: the next task's default assignee.
    pub assign_sid: Option<i32>,
}
#[derive(Deserialize, ToSchema)]
pub struct WfAssign {
    pub sid: i32,
    pub comment: Option<String>,
}
#[derive(Deserialize, ToSchema)]
pub struct WfHold {
    /// `true`: put the job on hold; `false`: resume it.
    pub hold: bool,
    pub comment: Option<String>,
}

// ---------------------------------------------------------------------------------------------
// Who is acting
// ---------------------------------------------------------------------------------------------

/// The caller as an Aumentum principal: matched by login (`code`) or Windows account
/// (`os_account`, e.g. PLAGIS\jdoe). Administrators (the `admin` role, Aumentum's `admins`
/// group or `ds_gral_admin`) may act on any task.
pub(crate) struct Actor {
    pub(crate) sid: Option<i32>,
    pub(crate) sids: Vec<i32>,
    pub(crate) admin: bool,
    pub(crate) label: String,
}

pub(crate) async fn resolve_actor(state: &AppState, user: &AuthUser) -> ApiResult<Actor> {
    let login = user.username.clone().unwrap_or_else(|| user.subject.clone());
    let short = login.rsplit('\\').next().unwrap_or(&login).to_lowercase();
    let sid: Option<i32> = sqlx::query_scalar(
        "SELECT sid FROM cadastre.principal WHERE is_user AND (lower(code) = $1 OR lower(os_account) = $2 \
         OR lower(os_account) LIKE '%\\' || $1) ORDER BY sid LIMIT 1",
    )
    .bind(&short)
    .bind(login.to_lowercase())
    .fetch_optional(&state.pool)
    .await?;
    let membership: Vec<(i32, i32)> = sqlx::query_as("SELECT group_sid, member_sid FROM cadastre.principal_member")
        .fetch_all(&state.pool)
        .await?;
    let mut sids = sid.map(|s| wfengine::effective_sids(s, &membership)).unwrap_or_default();
    // A Registry working group counts as the Users group for the workflow's task permissions.
    if state.config.auth_mode == crate::config::AuthMode::Local && user.has_role("editor") {
        let users: Option<i32> = sqlx::query_scalar("SELECT sid FROM cadastre.principal WHERE lower(code) = 'users' AND NOT is_user").fetch_optional(&state.pool).await?;
        sids.extend(users.filter(|u| !sids.contains(u)));
    }
    let aumentum_admin: bool = if sids.is_empty() {
        false
    } else {
        sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM cadastre.principal p WHERE p.sid = ANY($1) AND lower(p.code) = 'admins') \
             OR EXISTS (SELECT 1 FROM cadastre.principal_role r WHERE r.sid = ANY($1) AND r.role = 'ds_gral_admin' AND r.allow)",
        )
        .bind(&sids)
        .fetch_one(&state.pool)
        .await?
    };
    Ok(Actor { sid, sids, admin: user.has_role("admin") || aumentum_admin, label: login })
}

async fn task_permitted(conn: &mut sqlx::PgConnection, task_id: i32) -> ApiResult<Vec<i32>> {
    Ok(sqlx::query_scalar("SELECT sid FROM cadastre.wf_task_permission WHERE task_id = $1")
        .bind(task_id)
        .fetch_all(conn)
        .await?)
}

pub(crate) async fn default_assignee(conn: &mut sqlx::PgConnection, task_id: i32) -> ApiResult<Option<i32>> {
    Ok(sqlx::query_scalar("SELECT sid FROM cadastre.wf_task_permission WHERE task_id = $1 AND is_default ORDER BY sid LIMIT 1")
        .bind(task_id)
        .fetch_optional(conn)
        .await?)
}

/// Record the Aumentum "UserTask" notification for an assignment. It is sent by the mail outbox
/// when an SMTP server is configured; otherwise it is kept as 'skipped' with the reason (Aumentum's
/// production has no SMTP server, so it sends nothing either).
pub(crate) async fn notify_assignment(conn: &mut sqlx::PgConnection, job_id: i32, task_id: i32, sid: Option<i32>) -> ApiResult<()> {
    let Some(sid) = sid else { return Ok(()) };
    let row = sqlx::query(
        "SELECT p.is_user, COALESCE(p.name, p.code) AS who, p.email, COALESCE(j.name, 'Job ' || j.id) AS job, t.name AS task,                 m.subject, m.body,                 COALESCE((SELECT value FROM cadastre.app_setting WHERE key = 'mail.smtp_server'), '') <> ''                 AND COALESCE((SELECT value FROM cadastre.app_setting WHERE key = 'mail.from'), '') <> '' AS smtp          FROM cadastre.principal p, cadastre.wf_job_instance j, cadastre.wf_task t, cadastre.mail_template m          WHERE p.sid = $1 AND j.id = $2 AND t.id = $3 AND m.feature = 'UserTask'",
    )
    .bind(sid)
    .bind(job_id)
    .bind(task_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(r) = row else { return Ok(()) };
    if !r.get::<bool, _>("is_user") {
        return Ok(()); // assigned to a group: Aumentum's pool, no personal e-mail
    }
    let (who, job, task): (String, String, String) = (r.get("who"), r.get("job"), r.get("task"));
    let fields = [("FinalUser", who.as_str()), ("JobInstance", job.as_str()), ("Task", task.as_str())];
    let email: Option<String> = r.get("email");
    let (status, detail) = if !r.get::<bool, _>("smtp") {
        ("skipped", Some("no SMTP server configured (mail.smtp_server / mail.from)"))
    } else if email.as_deref().map(|e| e.contains('@')) != Some(true) {
        ("skipped", Some("the assignee has no e-mail address"))
    } else {
        ("pending", None)
    };
    sqlx::query(
        "INSERT INTO cadastre.wf_notification (feature, job_instance_id, task_id, to_sid, to_email, subject, body, status, detail)          VALUES ('UserTask', $1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(job_id)
    .bind(task_id)
    .bind(sid)
    .bind(&email)
    .bind(crate::mail::render(r.get("subject"), &fields))
    .bind(crate::mail::render(r.get("body"), &fields))
    .bind(status)
    .bind(detail)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Reads
// ---------------------------------------------------------------------------------------------

async fn steps_of(pool: &sqlx::PgPool, task_id: Option<i32>) -> ApiResult<Vec<WfStepDto>> {
    let Some(task_id) = task_id else { return Ok(vec![]) };
    let rows = sqlx::query(
        "SELECT s.id, s.name, s.icon, s.commands FROM cadastre.wf_task_step ts JOIN cadastre.wf_step s ON s.id = ts.step_id \
         WHERE ts.task_id = $1 ORDER BY ts.ord, s.id",
    )
    .bind(task_id)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(|r| WfStepDto { id: r.get("id"), name: r.get("name"), icon: r.get("icon"), commands: r.get("commands") }).collect())
}

/// Every job type with its task graph, every task with its steps and permissions, and the
/// status codes.
#[utoipa::path(get, path = "/api/v1/wf/definitions", tag = "wf", responses((status = 200, body = WfDefinitionsDto)))]
pub async fn definitions(State(state): State<AppState>) -> ApiResult<Json<WfDefinitionsDto>> {
    let flows = sqlx::query("SELECT job_type_id, task_id, next_task_id, task_order FROM cadastre.wf_task_flow ORDER BY job_type_id, task_order, task_id")
        .fetch_all(&state.pool)
        .await?;
    let job_types = sqlx::query("SELECT id, name, description, start_task_id, end_task_id, active FROM cadastre.wf_job_type ORDER BY id")
        .fetch_all(&state.pool)
        .await?
        .iter()
        .map(|r| {
            let id: i32 = r.get("id");
            WfJobTypeDto {
                id,
                name: r.get("name"),
                description: r.get("description"),
                start_task_id: r.get("start_task_id"),
                end_task_id: r.get("end_task_id"),
                active: r.get("active"),
                flow: flows
                    .iter()
                    .filter(|f| f.get::<i32, _>("job_type_id") == id)
                    .map(|f| WfEdgeDto { task_id: f.get("task_id"), next_task_id: f.get("next_task_id"), task_order: f.get("task_order") })
                    .collect(),
            }
        })
        .collect();
    let perms: Vec<(i32, i32)> = sqlx::query_as("SELECT task_id, sid FROM cadastre.wf_task_permission").fetch_all(&state.pool).await?;
    let mut tasks = Vec::new();
    for r in sqlx::query("SELECT id, name, description FROM cadastre.wf_task ORDER BY id").fetch_all(&state.pool).await? {
        let id: i32 = r.get("id");
        tasks.push(WfTaskDto {
            id,
            name: r.get("name"),
            description: r.get("description"),
            steps: steps_of(&state.pool, Some(id)).await?,
            permitted_sids: perms.iter().filter(|(t, _)| *t == id).map(|(_, s)| *s).collect(),
        });
    }
    let statuses = sqlx::query("SELECT code, label, is_open, confirmed FROM cadastre.wf_job_status ORDER BY code")
        .fetch_all(&state.pool)
        .await?
        .iter()
        .map(|r| WfStatusDto { code: r.get("code"), label: r.get("label"), is_open: r.get("is_open"), confirmed: r.get("confirmed") })
        .collect();
    Ok(Json(WfDefinitionsDto { job_types, tasks, statuses }))
}

const JOB_SELECT: &str = "SELECT j.id, j.job_type_id, jt.name AS job_type, j.current_task_id, t.name AS current_task, \
    j.assigned_sid, COALESCE(p.name, p.code) AS assigned_to, j.status, s.label AS status_label, s.is_open, j.priority, \
    j.name, j.comment, j.lrs_transaction_number, j.created, j.started, j.updated, j.ended, j.source, \
    j.lrs_properties, j.link_parcels_properties \
    FROM cadastre.wf_job_instance j JOIN cadastre.wf_job_type jt ON jt.id = j.job_type_id \
    JOIN cadastre.wf_job_status s ON s.code = j.status \
    LEFT JOIN cadastre.wf_task t ON t.id = j.current_task_id LEFT JOIN cadastre.principal p ON p.sid = j.assigned_sid";

fn row_to_job(r: &sqlx::postgres::PgRow) -> WfJobDto {
    WfJobDto {
        id: r.get("id"),
        job_type_id: r.get("job_type_id"),
        job_type: r.get("job_type"),
        current_task_id: r.get("current_task_id"),
        current_task: r.get("current_task"),
        assigned_sid: r.get("assigned_sid"),
        assigned_to: r.get("assigned_to"),
        status: r.get("status"),
        status_label: r.get("status_label"),
        is_open: r.get("is_open"),
        priority: r.get("priority"),
        name: r.get("name"),
        comment: r.get("comment"),
        lrs_transaction_number: r.get("lrs_transaction_number"),
        created: r.get("created"),
        started: r.get("started"),
        updated: r.get("updated"),
        ended: r.get("ended"),
        source: r.get("source"),
    }
}

fn push_filters<'a>(qb: &mut QueryBuilder<'a, sqlx::Postgres>, q: &'a WfJobQuery) {
    qb.push(" WHERE true");
    if q.open.unwrap_or(true) && q.status.is_none() {
        qb.push(" AND s.is_open");
    }
    if let Some(st) = q.status {
        qb.push(" AND j.status = ").push_bind(st);
    }
    if let Some(a) = q.assigned {
        qb.push(" AND j.assigned_sid = ").push_bind(a);
    }
    if let Some(t) = q.job_type {
        qb.push(" AND j.job_type_id = ").push_bind(t);
    }
    if let Some(text) = q.q.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        let like = format!("%{}%", text.replace('%', "\\%").replace('_', "\\_"));
        qb.push(" AND (j.id::text = ").push_bind(text.to_string())
            .push(" OR j.name ILIKE ").push_bind(like.clone())
            .push(" OR j.comment ILIKE ").push_bind(like.clone())
            .push(" OR j.lrs_transaction_number ILIKE ").push_bind(like)
            .push(")");
    }
}

/// Job instances (open ones by default), most recently updated first.
#[utoipa::path(get, path = "/api/v1/wf/jobs", tag = "wf", params(WfJobQuery), responses((status = 200, body = WfJobListDto)))]
pub async fn list_jobs(State(state): State<AppState>, Query(q): Query<WfJobQuery>) -> ApiResult<Json<WfJobListDto>> {
    let limit = q.limit.unwrap_or(100).clamp(1, 1000);
    let offset = q.offset.unwrap_or(0).max(0);
    let mut count = QueryBuilder::new("SELECT count(*) FROM cadastre.wf_job_instance j JOIN cadastre.wf_job_status s ON s.code = j.status");
    push_filters(&mut count, &q);
    let total: i64 = count.build_query_scalar().fetch_one(&state.pool).await?;
    let mut qb = QueryBuilder::new(JOB_SELECT);
    push_filters(&mut qb, &q);
    qb.push(" ORDER BY COALESCE(j.updated, j.created) DESC NULLS LAST, j.id DESC LIMIT ").push_bind(limit).push(" OFFSET ").push_bind(offset);
    let jobs = qb.build().fetch_all(&state.pool).await?.iter().map(row_to_job).collect();
    Ok(Json(WfJobListDto { total, jobs }))
}

async fn load_detail(state: &AppState, id: i32) -> ApiResult<WfJobDetailDto> {
    let row = sqlx::query(&format!("{JOB_SELECT} WHERE j.id = $1"))
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("job instance {id}")))?;
    let job = row_to_job(&row);
    let (edges, end_task) = graph(&state.pool, job.job_type_id).await?;
    let (next_tasks, can_complete) = match (job.is_open, job.current_task_id) {
        (true, Some(cur)) => (wfengine::next_tasks(&edges, cur), wfengine::can_complete(&edges, end_task, cur)),
        _ => (vec![], false),
    };
    let steps = if job.is_open { steps_of(&state.pool, job.current_task_id).await? } else { vec![] };
    let (plot_task, plot_steps) = plot_tools(&state.pool, &steps, &next_tasks).await?;
    let next_tasks = task_refs(&state.pool, &next_tasks).await?;
    let history = sqlx::query(
        "SELECT ti.id, ti.task_id, t.name AS task, ti.sid, COALESCE(p.name, p.code) AS assigned_to, ti.assigned, ti.started, ti.ended, ti.comment \
         FROM cadastre.wf_task_instance ti JOIN cadastre.wf_task t ON t.id = ti.task_id LEFT JOIN cadastre.principal p ON p.sid = ti.sid \
         WHERE ti.job_instance_id = $1 ORDER BY COALESCE(ti.assigned, ti.started), ti.id",
    )
    .bind(id)
    .fetch_all(&state.pool)
    .await?
    .iter()
    .map(|r| WfTaskInstanceDto {
        id: r.get("id"),
        task_id: r.get("task_id"),
        task: r.get("task"),
        sid: r.get("sid"),
        assigned_to: r.get("assigned_to"),
        assigned: r.get("assigned"),
        started: r.get("started"),
        ended: r.get("ended"),
        comment: r.get("comment"),
    })
    .collect();
    Ok(WfJobDetailDto {
        lrs_properties: row.get("lrs_properties"),
        link_parcels_properties: row.get("link_parcels_properties"),
        job,
        next_tasks,
        can_complete,
        steps,
        plot_steps,
        plot_task,
        history,
    })
}

/// Aumentum's "Construct Parcel" step: the task that carries it is where parcels are plotted.
const CONSTRUCT_PARCEL_STEP: i32 = 1019;

/// The plotting task among `next` (the tasks the current one may send the job to) and its steps,
/// minus those the current task already has (Load_Region and Load Region are the same step).
/// Nothing when the current task plots itself or cannot reach plotting in one move.
async fn plot_tools(pool: &sqlx::PgPool, current: &[WfStepDto], next: &[i32]) -> ApiResult<(Option<WfTaskRefDto>, Vec<WfStepDto>)> {
    if current.is_empty() || current.iter().any(|s| s.id == CONSTRUCT_PARCEL_STEP) {
        return Ok((None, vec![]));
    }
    let norm = |n: &str| n.replace('_', " ").to_lowercase();
    for &task in next {
        let steps = steps_of(pool, Some(task)).await?;
        if !steps.iter().any(|s| s.id == CONSTRUCT_PARCEL_STEP) {
            continue;
        }
        let extra = steps.into_iter().filter(|s| !current.iter().any(|c| c.id == s.id || norm(&c.name) == norm(&s.name))).collect();
        return Ok((task_refs(pool, &[task]).await?.into_iter().next(), extra));
    }
    Ok((None, vec![]))
}

async fn graph(pool: &sqlx::PgPool, job_type_id: i32) -> ApiResult<(Vec<Edge>, Option<i32>)> {
    let edges = sqlx::query_as::<_, (i32, Option<i32>)>("SELECT task_id, next_task_id FROM cadastre.wf_task_flow WHERE job_type_id = $1")
        .bind(job_type_id)
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|(task_id, next_task_id)| Edge { task_id, next_task_id })
        .collect();
    let end: Option<i32> = sqlx::query_scalar("SELECT end_task_id FROM cadastre.wf_job_type WHERE id = $1").bind(job_type_id).fetch_one(pool).await?;
    Ok((edges, end))
}

async fn task_refs(pool: &sqlx::PgPool, ids: &[i32]) -> ApiResult<Vec<WfTaskRefDto>> {
    Ok(sqlx::query("SELECT id, name FROM cadastre.wf_task WHERE id = ANY($1) ORDER BY id")
        .bind(ids)
        .fetch_all(pool)
        .await?
        .iter()
        .map(|r| WfTaskRefDto { id: r.get("id"), name: r.get("name") })
        .collect())
}

/// One job instance: its current task and steps, where it may go next, and its task history.
#[utoipa::path(get, path = "/api/v1/wf/jobs/{id}", tag = "wf", params(("id" = i32, Path, description = "Job instance id")),
    responses((status = 200, body = WfJobDetailDto), (status = 404, description = "Not found")))]
pub async fn get_job(State(state): State<AppState>, Path(id): Path<i32>) -> ApiResult<Json<WfJobDetailDto>> {
    Ok(Json(load_detail(&state, id).await?))
}

/// Users and groups (Aumentum principals).
#[utoipa::path(get, path = "/api/v1/wf/principals", tag = "wf", responses((status = 200, body = [WfPrincipalDto])))]
pub async fn principals(State(state): State<AppState>) -> ApiResult<Json<Vec<WfPrincipalDto>>> {
    let rows = sqlx::query("SELECT sid, code, name, is_user, is_locked, os_account FROM cadastre.principal ORDER BY is_user, lower(COALESCE(name, code))")
        .fetch_all(&state.pool)
        .await?;
    Ok(Json(rows.iter().map(|r| WfPrincipalDto { sid: r.get("sid"), code: r.get("code"), name: r.get("name"), is_user: r.get("is_user"), is_locked: r.get("is_locked"), os_account: r.get("os_account") }).collect()))
}

#[derive(Deserialize, ToSchema)]
pub struct NewStaffRequest {
    /// Full name, e.g. "Grace Pam".
    pub name: String,
    /// Login; made from the name when empty (e.g. "gpam").
    #[serde(default)]
    pub code: Option<String>,
}

/// A login made from a name: first initial + last name, letters only ("Grace Pam" -> "gpam").
pub fn login_from_name(name: &str) -> String {
    let words: Vec<String> = name.split_whitespace().map(|w| w.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_lowercase()).filter(|w| !w.is_empty()).collect();
    match words.as_slice() {
        [] => "user".into(),
        [one] => one.clone(),
        [first, .., last] => format!("{}{last}", &first[..1]),
    }
}

/// Add a staff member to the users list (from Committed by, when they are not in it yet). New
/// users are numbered from 1,000,000 up, so a reload of Aumentum's users never meets them. An
/// existing user with the same name or login is refused (pick them instead).
#[utoipa::path(post, path = "/api/v1/wf/principals", tag = "wf", request_body = NewStaffRequest,
    responses((status = 201, body = WfPrincipalDto), (status = 409, description = "Already a user")))]
pub async fn add_principal(State(state): State<AppState>, user: AuthUser, Json(req): Json<NewStaffRequest>) -> ApiResult<(axum::http::StatusCode, Json<WfPrincipalDto>)> {
    let actor = resolve_actor(&state, &user).await?;
    let name = req.name.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.len() < 3 {
        return Err(AppError::BadRequest("type the staff member's full name".into()));
    }
    let mut tx = state.pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(4207011)").execute(&mut *tx).await?;
    let same: Option<(i32, String, Option<String>)> = sqlx::query_as("SELECT sid, code, name FROM cadastre.principal WHERE is_user AND lower(COALESCE(name, '')) = lower($1) LIMIT 1")
        .bind(&name).fetch_optional(&mut *tx).await?;
    if let Some((_, code, n)) = same {
        return Err(AppError::Conflict(format!("{} is already a user ({code}): pick them from the list", n.unwrap_or_default())));
    }
    let wanted = req.code.as_deref().map(str::trim).filter(|c| !c.is_empty()).map(|c| c.to_lowercase()).unwrap_or_else(|| login_from_name(&name));
    if !wanted.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-') {
        return Err(AppError::BadRequest("the login may use letters, digits, . _ and - only".into()));
    }
    let mut code = wanted.clone();
    for n in 2..100 {
        let taken: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM cadastre.principal WHERE lower(code) = lower($1))").bind(&code).fetch_one(&mut *tx).await?;
        if !taken { break; }
        if req.code.as_deref().is_some_and(|c| !c.trim().is_empty()) {
            return Err(AppError::Conflict(format!("the login {code} is taken")));
        }
        code = format!("{wanted}{n}");
    }
    let sid: i32 = sqlx::query_scalar("SELECT GREATEST(1000000, COALESCE(max(sid), 0)) + 1 FROM cadastre.principal").fetch_one(&mut *tx).await?;
    sqlx::query("INSERT INTO cadastre.principal (sid, code, name, description, is_user, is_locked, is_system) VALUES ($1, $2, $3, 'Added in the Cadastre (Committed by)', true, false, false)")
        .bind(sid).bind(&code).bind(&name).execute(&mut *tx).await?;
    audit::append(&mut tx, &actor.label, "principal_added", "principal", Some(&sid.to_string()), json!({ "sid": sid, "code": code, "name": name })).await?;
    tx.commit().await?;
    Ok((axum::http::StatusCode::CREATED, Json(WfPrincipalDto { sid, code, name: Some(name), is_user: true, is_locked: false, os_account: None })))
}

// ---------------------------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------------------------

/// Start a job of a job type: it opens on the type's start task, assigned to the given principal
/// or the task's default assignee.
#[utoipa::path(post, path = "/api/v1/wf/jobs", tag = "wf", request_body = NewWfJob,
    responses((status = 201, body = WfJobDetailDto), (status = 403, description = "Not permitted"), (status = 404, description = "Unknown job type")))]
pub async fn create_job(State(state): State<AppState>, user: AuthUser, Json(req): Json<NewWfJob>) -> ApiResult<(StatusCode, Json<WfJobDetailDto>)> {
    let actor = resolve_actor(&state, &user).await?;
    let start: Option<Option<i32>> = sqlx::query_scalar("SELECT start_task_id FROM cadastre.wf_job_type WHERE id = $1 AND active")
        .bind(req.job_type_id)
        .fetch_optional(&state.pool)
        .await?;
    let start = start
        .ok_or_else(|| AppError::NotFound(format!("job type {}", req.job_type_id)))?
        .ok_or_else(|| AppError::BadRequest(format!("job type {} has no start task", req.job_type_id)))?;
    let mut tx = state.pool.begin().await?;
    if !wfengine::may_act(&task_permitted(&mut tx, start).await?, &actor.sids, actor.admin) {
        return Err(AppError::Forbidden(format!("{} may not start this job type's first task", actor.label)));
    }
    let assignee = match req.assigned_sid {
        Some(s) => Some(s),
        None => default_assignee(&mut tx, start).await?.or(actor.sid),
    };
    let id: i32 = sqlx::query_scalar(
        "INSERT INTO cadastre.wf_job_instance (job_type_id, current_task_id, assigned_sid, priority, status, name, comment, lrs_transaction_number, created, started, updated, source) \
         VALUES ($1, $2, $3, $4, 2, $5, $6, $7, now()::timestamp, now()::timestamp, now()::timestamp, 'cadastre') RETURNING id",
    )
    .bind(req.job_type_id)
    .bind(start)
    .bind(assignee)
    .bind(req.priority.as_deref().unwrap_or("N"))
    .bind(&req.name)
    .bind(&req.comment)
    .bind(&req.lrs_transaction_number)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO cadastre.wf_task_instance (job_instance_id, job_type_id, task_id, sid, assigned, started) \
         VALUES ($1, $2, $3, $4, now()::timestamp, now()::timestamp)",
    )
    .bind(id)
    .bind(req.job_type_id)
    .bind(start)
    .bind(assignee)
    .execute(&mut *tx)
    .await?;
    notify_assignment(&mut tx, id, start, assignee).await?;
    audit::append(&mut tx, &actor.label, "wf_create", "wf_job_instance", Some(&id.to_string()),
        json!({ "job_type_id": req.job_type_id, "task_id": start, "assigned_sid": assignee, "lrs_transaction_number": req.lrs_transaction_number })).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(load_detail(&state, id).await?)))
}

/// Finish the current task and move the job along its task graph (or complete it at the end).
#[utoipa::path(post, path = "/api/v1/wf/jobs/{id}/advance", tag = "wf", request_body = WfAdvance, params(("id" = i32, Path, description = "Job instance id")),
    responses((status = 200, body = WfJobDetailDto), (status = 403, description = "Not permitted"), (status = 409, description = "Not a next task, or the job is not open")))]
pub async fn advance(State(state): State<AppState>, user: AuthUser, Path(id): Path<i32>, Json(req): Json<WfAdvance>) -> ApiResult<Json<WfJobDetailDto>> {
    let actor = resolve_actor(&state, &user).await?;
    let mut tx = state.pool.begin().await?;
    // Lock the job so two officers cannot advance it at once.
    let row = sqlx::query(
        "SELECT j.job_type_id, j.current_task_id, s.is_open FROM cadastre.wf_job_instance j JOIN cadastre.wf_job_status s ON s.code = j.status \
         WHERE j.id = $1 FOR UPDATE OF j",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("job instance {id}")))?;
    let (job_type_id, current, is_open): (i32, Option<i32>, bool) = (row.get(0), row.get(1), row.get(2));
    let current = current.ok_or_else(|| AppError::Conflict("job has no current task".into()))?;
    if !wfengine::may_act(&task_permitted(&mut tx, current).await?, &actor.sids, actor.admin) {
        return Err(AppError::Forbidden(format!("{} may not act on this task", actor.label)));
    }
    if is_commit_task(&mut tx, Some(current)).await? {
        let drafts: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM cadastre.parcel p JOIN cadastre.wf_job_instance w ON w.cadastre_job_id = p.job_id WHERE w.id = $1 AND p.status = 'draft'",
        )
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
        if drafts > 0 {
            return Err(AppError::Conflict(format!("commit (or discard) the job's {drafts} parcel(s) before finishing this task")));
        }
        if crate::routes::mutations::has_pending_retirement(&mut *tx, id).await? {
            return Err(AppError::Conflict("commit (or discard) the job's parcel retirement before finishing this task".into()));
        }
    }
    let (edges, end_task) = graph(&state.pool, job_type_id).await?;
    let decision = wfengine::resolve_advance(&edges, end_task, current, is_open, req.to_task_id).map_err(|e| match e {
        AdvanceError::NotOpen => AppError::Conflict("the job is not open".into()),
        AdvanceError::NotANextTask { requested, allowed } => AppError::Conflict(format!("task {requested} does not follow the current task; next tasks: {allowed:?}")),
        AdvanceError::ChooseNextTask { allowed } => AppError::Conflict(format!("choose the next task: {allowed:?}")),
    })?;
    // Close the open task instance of the current task.
    sqlx::query(
        "UPDATE cadastre.wf_task_instance SET ended = now()::timestamp, started = COALESCE(started, now()::timestamp), \
         comment = COALESCE($3, comment) WHERE job_instance_id = $1 AND task_id = $2 AND ended IS NULL",
    )
    .bind(id)
    .bind(current)
    .bind(&req.comment)
    .execute(&mut *tx)
    .await?;
    let payload = match decision {
        Advance::Complete => {
            sqlx::query("UPDATE cadastre.wf_job_instance SET status = 3, ended = now()::timestamp, updated = now()::timestamp WHERE id = $1")
                .bind(id)
                .execute(&mut *tx)
                .await?;
            json!({ "from_task": current, "completed": true, "comment": req.comment })
        }
        Advance::To(next) => {
            let assignee = match req.assign_sid {
                Some(s) => Some(s),
                None => default_assignee(&mut tx, next).await?,
            };
            sqlx::query("UPDATE cadastre.wf_job_instance SET current_task_id = $2, assigned_sid = $3, status = 2, updated = now()::timestamp WHERE id = $1")
                .bind(id)
                .bind(next)
                .bind(assignee)
                .execute(&mut *tx)
                .await?;
            sqlx::query("INSERT INTO cadastre.wf_task_instance (job_instance_id, job_type_id, task_id, sid, assigned) VALUES ($1, $2, $3, $4, now()::timestamp)")
                .bind(id)
                .bind(job_type_id)
                .bind(next)
                .bind(assignee)
                .execute(&mut *tx)
                .await?;
            notify_assignment(&mut tx, id, next, assignee).await?;
            json!({ "from_task": current, "to_task": next, "assigned_sid": assignee, "comment": req.comment })
        }
    };
    audit::append(&mut tx, &actor.label, "wf_advance", "wf_job_instance", Some(&id.to_string()), payload).await?;
    tx.commit().await?;
    Ok(Json(load_detail(&state, id).await?))
}

/// Reassign the job's current task (Aumentum role `ds_reasignTask`, or an administrator).
#[utoipa::path(post, path = "/api/v1/wf/jobs/{id}/assign", tag = "wf", request_body = WfAssign, params(("id" = i32, Path, description = "Job instance id")),
    responses((status = 200, body = WfJobDetailDto), (status = 403, description = "Not permitted")))]
pub async fn assign(State(state): State<AppState>, user: AuthUser, Path(id): Path<i32>, Json(req): Json<WfAssign>) -> ApiResult<Json<WfJobDetailDto>> {
    let actor = resolve_actor(&state, &user).await?;
    let may_reassign: bool = actor.admin
        || sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM cadastre.principal_role WHERE sid = ANY($1) AND role = 'ds_reasignTask' AND allow)")
            .bind(&actor.sids)
            .fetch_one(&state.pool)
            .await?;
    if !may_reassign {
        return Err(AppError::Forbidden(format!("{} may not reassign tasks", actor.label)));
    }
    let mut tx = state.pool.begin().await?;
    let cur: Option<Option<i32>> = sqlx::query_scalar("SELECT current_task_id FROM cadastre.wf_job_instance WHERE id = $1 FOR UPDATE")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?;
    let cur = cur.ok_or_else(|| AppError::NotFound(format!("job instance {id}")))?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM cadastre.principal WHERE sid = $1)").bind(req.sid).fetch_one(&mut *tx).await?;
    if !exists {
        return Err(AppError::NotFound(format!("principal {}", req.sid)));
    }
    sqlx::query("UPDATE cadastre.wf_job_instance SET assigned_sid = $2, updated = now()::timestamp WHERE id = $1")
        .bind(id)
        .bind(req.sid)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE cadastre.wf_task_instance SET sid = $3, assigned = now()::timestamp WHERE job_instance_id = $1 AND task_id = $2 AND ended IS NULL")
        .bind(id)
        .bind(cur)
        .bind(req.sid)
        .execute(&mut *tx)
        .await?;
    if let Some(cur) = cur {
        notify_assignment(&mut tx, id, cur, Some(req.sid)).await?;
    }
    audit::append(&mut tx, &actor.label, "wf_assign", "wf_job_instance", Some(&id.to_string()), json!({ "sid": req.sid, "task_id": cur, "comment": req.comment })).await?;
    tx.commit().await?;
    Ok(Json(load_detail(&state, id).await?))
}

/// Put an open job on hold (status 8) or resume it (status 2).
#[utoipa::path(post, path = "/api/v1/wf/jobs/{id}/hold", tag = "wf", request_body = WfHold, params(("id" = i32, Path, description = "Job instance id")),
    responses((status = 200, body = WfJobDetailDto), (status = 409, description = "The job is closed")))]
pub async fn hold(State(state): State<AppState>, user: AuthUser, Path(id): Path<i32>, Json(req): Json<WfHold>) -> ApiResult<Json<WfJobDetailDto>> {
    let actor = resolve_actor(&state, &user).await?;
    let mut tx = state.pool.begin().await?;
    let row = sqlx::query("SELECT j.current_task_id, s.is_open FROM cadastre.wf_job_instance j JOIN cadastre.wf_job_status s ON s.code = j.status WHERE j.id = $1 FOR UPDATE OF j")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("job instance {id}")))?;
    let (cur, is_open): (Option<i32>, bool) = (row.get(0), row.get(1));
    if !is_open {
        return Err(AppError::Conflict("the job is closed".into()));
    }
    if let Some(cur) = cur {
        if !wfengine::may_act(&task_permitted(&mut tx, cur).await?, &actor.sids, actor.admin) {
            return Err(AppError::Forbidden(format!("{} may not act on this task", actor.label)));
        }
    }
    sqlx::query("UPDATE cadastre.wf_job_instance SET status = $2, updated = now()::timestamp WHERE id = $1")
        .bind(id)
        .bind(if req.hold { 8i16 } else { 2i16 })
        .execute(&mut *tx)
        .await?;
    audit::append(&mut tx, &actor.label, if req.hold { "wf_hold" } else { "wf_resume" }, "wf_job_instance", Some(&id.to_string()), json!({ "comment": req.comment })).await?;
    tx.commit().await?;
    Ok(Json(load_detail(&state, id).await?))
}

// ---------------------------------------------------------------------------------------------
// Editing under a job: the job's editing job (drafts), its job area, discard and commit
// ---------------------------------------------------------------------------------------------

/// Aumentum's "save job area to parcel fabric" tool. A task whose steps use it is a commit task:
/// the job's parcels are committed there, and the job may not leave it with work uncommitted.
pub const COMMIT_TOOL: &str = "{E9CFA8EC-BCBF-420d-8BA8-1ACE00F05C83}";
/// Aumentum's AddUpinsCommand: a task with it gives new parcels their UPINs (e.g. Merge Parcel),
/// so it commits like a Commit Parcel task.
pub const ADD_UPINS_TOOL: &str = "{4F7520B2-C09A-4abd-843E-CDD9F241AE16}";
/// Aumentum's message when a parcel is drawn outside the job area (from the editor's own log).
pub const OUTSIDE_JOB_AREA: &str = "You have a point outside the Job Area. All edits must be inside Job Area.";

#[derive(Serialize, ToSchema)]
pub struct WfEditingDto {
    pub job_instance_id: i32,
    /// The editing job that holds this job's draft parcels (created on first use).
    pub cadastre_job_id: Option<uuid::Uuid>,
    pub reference: Option<String>,
    /// The job area in WGS84 (for the map) and in UTM 32N (for drawing), if one exists.
    pub area: Option<serde_json::Value>,
    pub area_utm: Option<serde_json::Value>,
    pub area_status: Option<String>,
    pub area_m2: Option<f64>,
    /// Uncommitted draft parcels.
    pub drafts: i64,
    /// Whether the job's current task is where its parcels are committed.
    pub is_commit_task: bool,
}
#[derive(Deserialize, ToSchema)]
pub struct WfAreaBody {
    /// A GeoJSON Polygon in UTM zone 32N (EPSG:32632) metres.
    pub geometry: serde_json::Value,
}
#[derive(Serialize, ToSchema)]
pub struct WfCommitDto {
    /// UPINs given to the committed parcels.
    pub parcel_numbers: Vec<String>,
    /// Beacon numbers added to the fabric (as typed by the officer).
    pub beacons_added: Vec<String>,
    /// Parcels the job's mutations made historic or archived.
    pub retired: Vec<String>,
    pub editing: WfEditingDto,
    /// The link to the job's Registry file made right after the commit, per parcel.
    #[schema(value_type = Vec<Object>)]
    pub registry_links: Vec<serde_json::Value>,
}
#[derive(Default, Deserialize, ToSchema)]
pub struct WfFinishBody {
    /// Also carry the job to its end task and complete it (default true). False stops at commit.
    pub complete: Option<bool>,
    pub comment: Option<String>,
    /// Beacon numbers for the new corners (as for Commit Parcels).
    #[serde(default)]
    pub beacons: Vec<crate::routes::wfbeacons::WfBeaconNumber>,
    /// The staff member committing (recorded in the audit log with the commit).
    #[serde(default)]
    pub staff_name: Option<String>,
    /// Their user id (cadastre.principal), when picked from the users list.
    #[serde(default)]
    pub staff_sid: Option<i32>,
}
#[derive(Serialize, ToSchema)]
pub struct WfFinishDto {
    /// UPINs given to the posted parcels.
    pub parcel_numbers: Vec<String>,
    /// The tasks the job passed through to reach (and leave) its commit task.
    pub tasks_passed: Vec<WfTaskRefDto>,
    /// Whether the job was completed.
    pub completed: bool,
    pub job: WfJobDto,
    /// The link to the job's Registry file made right after the commit, per parcel.
    #[schema(value_type = Vec<Object>)]
    pub registry_links: Vec<serde_json::Value>,
}

pub(crate) async fn is_commit_task(conn: &mut sqlx::PgConnection, task_id: Option<i32>) -> ApiResult<bool> {
    let Some(task_id) = task_id else { return Ok(false) };
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM cadastre.wf_task_step ts JOIN cadastre.wf_step s ON s.id = ts.step_id \
         WHERE ts.task_id = $1 AND EXISTS (SELECT 1 FROM unnest(s.commands) c WHERE upper(c) IN (upper($2), upper($3))))",
    )
    .bind(task_id)
    .bind(COMMIT_TOOL)
    .bind(ADD_UPINS_TOOL)
    .fetch_one(conn)
    .await?)
}

pub(crate) async fn editing_info(state: &AppState, id: i32) -> ApiResult<WfEditingDto> {
    let row = sqlx::query(
        "SELECT w.current_task_id, w.cadastre_job_id, j.reference, \
                ST_AsGeoJSON(ST_Transform(a.geom, 4326))::json AS area, ST_AsGeoJSON(a.geom)::json AS area_utm, a.status AS area_status, ST_Area(a.geom) AS area_m2, \
                (SELECT count(*) FROM cadastre.parcel p WHERE p.job_id = w.cadastre_job_id AND p.status = 'draft') AS drafts \
         FROM cadastre.wf_job_instance w LEFT JOIN cadastre.job j ON j.id = w.cadastre_job_id \
         LEFT JOIN cadastre.job_area a ON a.wf_job_instance_id = w.id WHERE w.id = $1",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("job instance {id}")))?;
    let mut conn = state.pool.acquire().await?;
    Ok(WfEditingDto {
        job_instance_id: id,
        cadastre_job_id: row.get("cadastre_job_id"),
        reference: row.get("reference"),
        area: row.get("area"),
        area_utm: row.get("area_utm"),
        area_status: row.get("area_status"),
        area_m2: row.get("area_m2"),
        drafts: row.get("drafts"),
        is_commit_task: is_commit_task(&mut conn, row.get("current_task_id")).await?,
    })
}

/// Lock an open job for a change by `actor` on its current task; returns its current task.
pub(crate) async fn lock_open_job(tx: &mut sqlx::PgConnection, id: i32, actor: &Actor) -> ApiResult<Option<i32>> {
    let row = sqlx::query(
        "SELECT j.current_task_id, s.is_open FROM cadastre.wf_job_instance j JOIN cadastre.wf_job_status s ON s.code = j.status \
         WHERE j.id = $1 FOR UPDATE OF j",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("job instance {id}")))?;
    let (cur, open): (Option<i32>, bool) = (row.get(0), row.get(1));
    if !open {
        return Err(AppError::Conflict("the job is not open".into()));
    }
    if let Some(cur) = cur {
        if !wfengine::may_act(&task_permitted(tx, cur).await?, &actor.sids, actor.admin) {
            return Err(AppError::Forbidden(format!("{} may not act on this job's current task", actor.label)));
        }
    }
    Ok(cur)
}

/// The job's editing state (does not create anything).
#[utoipa::path(get, path = "/api/v1/wf/jobs/{id}/editing", tag = "wf", params(("id" = i32, Path, description = "Job instance id")),
    responses((status = 200, body = WfEditingDto)))]
pub async fn get_editing(State(state): State<AppState>, Path(id): Path<i32>) -> ApiResult<Json<WfEditingDto>> {
    Ok(Json(editing_info(&state, id).await?))
}

/// The UPINs a commit of this job would give now, and whether Aumentum was asked (parallel run).
/// Advisory: shown in the Commit window; nothing is allocated or recorded.
#[utoipa::path(get, path = "/api/v1/wf/jobs/{id}/next-upin", tag = "wf", params(("id" = i32, Path, description = "Job instance id")),
    responses((status = 200, body = crate::routes::jobs::UpinPreview), (status = 404, description = "Job not found")))]
pub async fn next_upin(State(state): State<AppState>, Path(id): Path<i32>) -> ApiResult<Json<crate::routes::jobs::UpinPreview>> {
    let mut conn = state.pool.acquire().await?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM cadastre.wf_job_instance WHERE id = $1)").bind(id).fetch_one(&mut *conn).await?;
    if !exists {
        return Err(AppError::NotFound(format!("job instance {id}")));
    }
    let drafts: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM cadastre.parcel p JOIN cadastre.wf_job_instance w ON w.cadastre_job_id = p.job_id \
         WHERE w.id = $1 AND p.status = 'draft' AND p.parcel_number IS NULL",
    )
    .bind(id)
    .fetch_one(&mut *conn)
    .await?;
    Ok(Json(crate::routes::jobs::preview_upins(&mut conn, drafts).await?))
}

/// Start editing under the job: creates its editing job (which holds its draft parcels) if needed.
#[utoipa::path(post, path = "/api/v1/wf/jobs/{id}/editing", tag = "wf", params(("id" = i32, Path, description = "Job instance id")),
    responses((status = 200, body = WfEditingDto), (status = 409, description = "The job is not open")))]
pub async fn start_editing(State(state): State<AppState>, user: AuthUser, Path(id): Path<i32>) -> ApiResult<Json<WfEditingDto>> {
    let actor = resolve_actor(&state, &user).await?;
    let mut tx = state.pool.begin().await?;
    lock_open_job(&mut tx, id, &actor).await?;
    let existing: Option<uuid::Uuid> = sqlx::query_scalar("SELECT cadastre_job_id FROM cadastre.wf_job_instance WHERE id = $1").bind(id).fetch_one(&mut *tx).await?;
    if existing.is_none() {
        let job_id: uuid::Uuid = sqlx::query_scalar(
            "INSERT INTO cadastre.job (reference, job_type, title, created_by, updated_by, current_stage_id, status) \
             SELECT 'WF-' || w.id, 'aumentum:' || jt.name, w.name, $2, $2, s.id, s.code \
             FROM cadastre.wf_job_instance w JOIN cadastre.wf_job_type jt ON jt.id = w.job_type_id, cadastre.workflow_stage s \
             WHERE w.id = $1 AND s.is_initial RETURNING id",
        )
        .bind(id)
        .bind(&actor.label)
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query("UPDATE cadastre.wf_job_instance SET cadastre_job_id = $2, updated = now()::timestamp WHERE id = $1").bind(id).bind(job_id).execute(&mut *tx).await?;
        audit::append(&mut tx, &actor.label, "wf_start_editing", "wf_job_instance", Some(&id.to_string()), json!({ "cadastre_job_id": job_id })).await?;
    }
    tx.commit().await?;
    Ok(Json(editing_info(&state, id).await?))
}

/// Create or redraw the job area (Aumentum: Create / Edit Job Area). Reopens a closed area.
#[utoipa::path(put, path = "/api/v1/wf/jobs/{id}/area", tag = "wf", request_body = WfAreaBody, params(("id" = i32, Path, description = "Job instance id")),
    responses((status = 200, body = WfEditingDto), (status = 400, description = "Not a valid polygon"), (status = 409, description = "Drafts would fall outside the new area")))]
pub async fn put_area(State(state): State<AppState>, user: AuthUser, Path(id): Path<i32>, Json(body): Json<WfAreaBody>) -> ApiResult<Json<WfEditingDto>> {
    let actor = resolve_actor(&state, &user).await?;
    let mut tx = state.pool.begin().await?;
    lock_open_job(&mut tx, id, &actor).await?;
    let ok: Option<bool> = sqlx::query_scalar(
        "SELECT ST_GeometryType(g) = 'ST_Polygon' AND ST_IsValid(g) AND ST_Area(g) > 0 FROM (SELECT ST_SetSRID(ST_GeomFromGeoJSON($1), 32632) AS g) x",
    )
    .bind(body.geometry.to_string())
    .fetch_one(&mut *tx)
    .await
    .map_err(|_| AppError::BadRequest("the job area must be a GeoJSON Polygon in UTM zone 32N".into()))?;
    if ok != Some(true) {
        return Err(AppError::BadRequest("the job area must be a single valid polygon with an area".into()));
    }
    // UTM zone 32N coordinates: eastings 100 km..900 km, northings within the northern hemisphere
    // band. Outside that the geometry cannot be shown on the map (it projects to infinity).
    let in_zone: bool = sqlx::query_scalar(
        "SELECT ST_XMin(g) >= 100000 AND ST_XMax(g) <= 900000 AND ST_YMin(g) >= 0 AND ST_YMax(g) <= 9400000          FROM (SELECT ST_SetSRID(ST_GeomFromGeoJSON($1), 32632) AS g) x",
    )
    .bind(body.geometry.to_string())
    .fetch_one(&mut *tx)
    .await?;
    if !in_zone {
        return Err(AppError::BadRequest("the job area is outside UTM zone 32N (EPSG:32632); coordinates must be eastings/northings in metres".into()));
    }
    // Redrawing must keep the job's existing drafts inside the area.
    let outside: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM cadastre.parcel p JOIN cadastre.wf_job_instance w ON w.cadastre_job_id = p.job_id \
         WHERE w.id = $1 AND p.status = 'draft' AND NOT ST_CoveredBy(p.geom, ST_Buffer(ST_SetSRID(ST_GeomFromGeoJSON($2), 32632), 0.01))",
    )
    .bind(id)
    .bind(body.geometry.to_string())
    .fetch_one(&mut *tx)
    .await?;
    if outside > 0 {
        return Err(AppError::Conflict(format!("{outside} draft parcel(s) of this job would fall outside the new job area")));
    }
    sqlx::query(
        "INSERT INTO cadastre.job_area (wf_job_instance_id, geom, status, created_by, updated_by) \
         VALUES ($1, ST_SetSRID(ST_GeomFromGeoJSON($2), 32632), 'open', $3, $3) \
         ON CONFLICT (wf_job_instance_id) DO UPDATE SET geom = EXCLUDED.geom, status = 'open', closed_at = NULL, updated_by = $3, updated_at = now()",
    )
    .bind(id)
    .bind(body.geometry.to_string())
    .bind(&actor.label)
    .execute(&mut *tx)
    .await?;
    audit::append(&mut tx, &actor.label, "wf_job_area_save", "wf_job_instance", Some(&id.to_string()), json!({ "geometry": body.geometry })).await?;
    tx.commit().await?;
    Ok(Json(editing_info(&state, id).await?))
}

/// Close (no more edits) or reopen the job area.
#[utoipa::path(post, path = "/api/v1/wf/jobs/{id}/area/{action}", tag = "wf",
    params(("id" = i32, Path, description = "Job instance id"), ("action" = String, Path, description = "close | reopen")),
    responses((status = 200, body = WfEditingDto), (status = 404, description = "No job area")))]
pub async fn area_action(State(state): State<AppState>, user: AuthUser, Path((id, action)): Path<(i32, String)>) -> ApiResult<Json<WfEditingDto>> {
    let actor = resolve_actor(&state, &user).await?;
    let status = match action.as_str() {
        "close" => "closed",
        "reopen" => "open",
        _ => return Err(AppError::BadRequest("action must be close or reopen".into())),
    };
    let mut tx = state.pool.begin().await?;
    lock_open_job(&mut tx, id, &actor).await?;
    let n = sqlx::query(
        "UPDATE cadastre.job_area SET status = $2, closed_at = CASE WHEN $2 = 'closed' THEN now() END, updated_by = $3, updated_at = now() WHERE wf_job_instance_id = $1",
    )
    .bind(id)
    .bind(status)
    .bind(&actor.label)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if n == 0 {
        return Err(AppError::NotFound("this job has no job area".into()));
    }
    audit::append(&mut tx, &actor.label, &format!("wf_job_area_{action}"), "wf_job_instance", Some(&id.to_string()), json!({})).await?;
    tx.commit().await?;
    Ok(Json(editing_info(&state, id).await?))
}

/// Delete the job area (Aumentum: Delete Job Area). Refused while the job has uncommitted drafts.
#[utoipa::path(delete, path = "/api/v1/wf/jobs/{id}/area", tag = "wf", params(("id" = i32, Path, description = "Job instance id")),
    responses((status = 200, body = WfEditingDto), (status = 409, description = "Discard or commit the drafts first")))]
pub async fn delete_area(State(state): State<AppState>, user: AuthUser, Path(id): Path<i32>) -> ApiResult<Json<WfEditingDto>> {
    let actor = resolve_actor(&state, &user).await?;
    let mut tx = state.pool.begin().await?;
    lock_open_job(&mut tx, id, &actor).await?;
    let drafts: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM cadastre.parcel p JOIN cadastre.wf_job_instance w ON w.cadastre_job_id = p.job_id WHERE w.id = $1 AND p.status = 'draft'",
    )
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;
    if drafts > 0 {
        return Err(AppError::Conflict(format!("the job has {drafts} uncommitted parcel(s): discard or commit them first")));
    }
    sqlx::query("DELETE FROM cadastre.job_area WHERE wf_job_instance_id = $1").bind(id).execute(&mut *tx).await?;
    audit::append(&mut tx, &actor.label, "wf_job_area_delete", "wf_job_instance", Some(&id.to_string()), json!({})).await?;
    tx.commit().await?;
    Ok(Json(editing_info(&state, id).await?))
}

/// Discard the job's uncommitted draft parcels (Aumentum: Discard Changes).
#[utoipa::path(post, path = "/api/v1/wf/jobs/{id}/discard", tag = "wf", params(("id" = i32, Path, description = "Job instance id")),
    responses((status = 200, body = WfEditingDto)))]
pub async fn discard(State(state): State<AppState>, user: AuthUser, Path(id): Path<i32>) -> ApiResult<Json<WfEditingDto>> {
    let actor = resolve_actor(&state, &user).await?;
    let mut tx = state.pool.begin().await?;
    lock_open_job(&mut tx, id, &actor).await?;
    let n = sqlx::query(
        "DELETE FROM cadastre.parcel p USING cadastre.wf_job_instance w WHERE w.id = $1 AND p.job_id = w.cadastre_job_id AND p.status = 'draft'",
    )
    .bind(id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    // Discarding the job's edits also drops its recorded mutations (Phase 4).
    let m = sqlx::query("DELETE FROM cadastre.parcel_lineage WHERE pending AND wf_job_instance_id = $1").bind(id).execute(&mut *tx).await?.rows_affected();
    audit::append(&mut tx, &actor.label, "wf_discard", "wf_job_instance", Some(&id.to_string()), json!({ "drafts_discarded": n as i64, "mutation_rows_discarded": m as i64 })).await?;
    tx.commit().await?;
    Ok(Json(editing_info(&state, id).await?))
}

/// Commit the job's draft parcels to the fabric (Aumentum: Commit Parcel / save job area to the
/// parcel fabric). Only from a commit task. Each parcel receives its UPIN; with
/// `editor.allow_overlap` = 0 (Aumentum's setting) a parcel overlapping the fabric is refused.
#[utoipa::path(post, path = "/api/v1/wf/jobs/{id}/commit", tag = "wf", params(("id" = i32, Path, description = "Job instance id")),
    request_body(content = crate::routes::wfbeacons::WfCommitRequest, description = "Beacon numbers typed for the new corners (optional body)"),
    responses((status = 200, body = WfCommitDto), (status = 409, description = "Not a commit task, nothing to commit, overlaps, or beacon numbers missing / already used")))]
pub async fn commit(State(state): State<AppState>, user: AuthUser, Path(id): Path<i32>, body: axum::body::Bytes) -> ApiResult<Json<WfCommitDto>> {
    let req: crate::routes::wfbeacons::WfCommitRequest = if body.iter().all(u8::is_ascii_whitespace) {
        Default::default()
    } else {
        serde_json::from_slice(&body).map_err(|e| AppError::BadRequest(format!("invalid commit request: {e}")))?
    };
    let actor = resolve_actor(&state, &user).await?;
    // The system check again at commit (overlaps here and in Aumentum, drop point, title, transaction):
    // the land may have been taken since the sketch was checked.
    let check = crate::routes::precheck::run(&state, id, &[], None).await?;
    if !check.ok {
        return Err(AppError::Conflict(format!("System check failed, nothing was committed. {}", crate::routes::precheck::failures(&check))));
    }
    let mut tx = state.pool.begin().await?;
    let cur = lock_open_job(&mut tx, id, &actor).await?;
    if !is_commit_task(&mut tx, cur).await? {
        return Err(AppError::Conflict("parcels are committed at the job's commit task (e.g. Commit Parcel)".into()));
    }
    let c = commit_drafts(&mut tx, id, &req).await?;
    let staff = staff_of(&state.pool, req.staff_sid, req.staff_name.as_deref()).await?;
    let who = committed_by(staff.as_deref(), &actor.label);
    audit::append(&mut tx, &who, "wf_commit", "wf_job_instance", Some(&id.to_string()), json!({ "parcel_numbers": c.numbers, "beacons_added": c.beacons_added, "retired": c.retired, "staff_name": staff, "staff_sid": req.staff_sid, "login": actor.label })).await?;
    // A commit is the end of digitising: the job is completed (Correct reopens it).
    let job_type_id: i32 = sqlx::query_scalar("SELECT job_type_id FROM cadastre.wf_job_instance WHERE id = $1").bind(id).fetch_one(&mut *tx).await?;
    let (edges, end_task) = graph(&state.pool, job_type_id).await?;
    if let Some(cur) = cur {
        complete_after_commit(&mut tx, id, job_type_id, cur, &edges, end_task, &who, None).await?;
    }
    tx.commit().await?;
    // On commit the UPIN goes onto the Registry file (with its link date), as Map → Connect would.
    let registry_links = crate::routes::registry::link_after_commit(&state, &who, id, &c.numbers).await;
    Ok(Json(WfCommitDto { parcel_numbers: c.numbers, beacons_added: c.beacons_added, retired: c.retired, editing: editing_info(&state, id).await?, registry_links }))
}

/// Post a job's draft parcels to the fabric: beacon numbers for the new corners, the job's
/// mutations (parents retired, lineage, inherited attributes), then the UPINs. Enforces Aumentum's
/// no-overlap rule (unless `editor.allow_overlap` = 1). The caller has already confirmed the job is
/// at its commit task.
async fn commit_drafts(tx: &mut sqlx::PgConnection, id: i32, req: &crate::routes::wfbeacons::WfCommitRequest) -> ApiResult<Committed> {
    let job_id: Option<uuid::Uuid> = sqlx::query_scalar("SELECT cadastre_job_id FROM cadastre.wf_job_instance WHERE id = $1").bind(id).fetch_one(&mut *tx).await?;
    let job_id = job_id.ok_or_else(|| AppError::Conflict("the job has no parcels to commit".into()))?;
    let drafts: i64 = sqlx::query_scalar("SELECT count(*) FROM cadastre.parcel WHERE job_id = $1 AND status = 'draft'").bind(job_id).fetch_one(&mut *tx).await?;
    // Parcels this job's mutations replace or retire (Phase 4): not counted as overlaps.
    let mutated = crate::routes::mutations::pending_parents(&mut *tx, id).await?;
    if drafts == 0 && !crate::routes::mutations::has_pending_retirement(&mut *tx, id).await? {
        return Err(AppError::Conflict("the job has no parcels to commit".into()));
    }
    let allow_overlap: Option<String> = sqlx::query_scalar("SELECT value FROM cadastre.app_setting WHERE key = 'editor.allow_overlap'").fetch_optional(&mut *tx).await?;
    if allow_overlap.as_deref() != Some("1") {
        let clashes: Vec<String> = sqlx::query_scalar(
            "SELECT DISTINCT p.parcel_number FROM cadastre.parcel d JOIN cadastre.parcel p \
               ON p.status = 'posted' AND p.geom && d.geom AND ST_Area(ST_Intersection(p.geom, d.geom)) > 0.01 AND NOT (p.parcel_number = ANY($2)) \
             WHERE d.job_id = $1 AND d.status = 'draft' ORDER BY 1 LIMIT 20",
        )
        .bind(job_id)
        .bind(&mutated)
        .fetch_all(&mut *tx)
        .await?;
        if !clashes.is_empty() {
            return Err(AppError::Conflict(format!("parcels may not overlap the fabric (Editor.AllowOverlap = 0); overlaps: {}", clashes.join(", "))));
        }
    }
    let beacons_added = crate::routes::wfbeacons::apply(&mut *tx, job_id, req).await?;
    // Mutations: parents leave the active fabric before their successors take UPINs.
    let retired = crate::routes::mutations::retire_parents(&mut *tx, id).await?;
    let posted = crate::routes::jobs::post_drafts_with_ids(&mut *tx, job_id).await?;
    crate::routes::mutations::finalise(&mut *tx, id, &posted).await?;
    let numbers: Vec<String> = posted.into_iter().map(|(_, n)| n).collect();
    sqlx::query("UPDATE cadastre.wf_job_instance SET updated = now()::timestamp WHERE id = $1").bind(id).execute(&mut *tx).await?;
    Ok(Committed { numbers, beacons_added, retired })
}

/// What a commit did: the UPINs given, the beacon numbers added, the parcels retired.
struct Committed {
    numbers: Vec<String>,
    beacons_added: Vec<String>,
    retired: Vec<String>,
}


/// The job type's commit task: the task in its graph whose steps carry the Commit Parcel tool.
/// The staff member picked at commit: their name from the users list when a user id is given.
pub(crate) async fn staff_of(pool: &sqlx::PgPool, sid: Option<i32>, typed: Option<&str>) -> ApiResult<Option<String>> {
    let Some(sid) = sid else { return Ok(typed.map(str::to_string)) };
    let name: Option<(Option<String>, String, bool, bool)> = sqlx::query_as("SELECT name, code, is_user, is_locked FROM cadastre.principal WHERE sid = $1")
        .bind(sid).fetch_optional(pool).await?;
    match name {
        Some((n, code, true, false)) => Ok(Some(n.filter(|n| !n.trim().is_empty()).unwrap_or(code))),
        Some((_, code, _, true)) => Err(AppError::Conflict(format!("the user {code} is locked"))),
        _ => Err(AppError::BadRequest(format!("no user {sid}"))),
    }
}

/// Who a commit is recorded under: the staff name typed at commit (with the login when there is
/// one), else the signed-in user.
pub(crate) fn committed_by(staff: Option<&str>, login: &str) -> String {
    match staff.map(|s| s.split_whitespace().collect::<Vec<_>>().join(" ")).filter(|s| !s.is_empty()) {
        Some(s) if login.is_empty() || login == "dev" || login.eq_ignore_ascii_case(&s) => s.chars().take(80).collect(),
        Some(s) => format!("{} ({login})", s.chars().take(80).collect::<String>()),
        None => login.to_string(),
    }
}

/// After a commit: walk the job on to its end task and complete it. A committed job is the end of
/// digitising (PLAGIS); the title deed plan is made on the completed job, and Correct reopens it.
/// Returns whether the job was completed and the tasks passed on the way.
#[allow(clippy::too_many_arguments)]
async fn complete_after_commit(
    tx: &mut sqlx::PgConnection, id: i32, job_type_id: i32, mut current: i32, edges: &[Edge], end_task: Option<i32>, actor: &str, comment: Option<&str>,
) -> ApiResult<(bool, Vec<i32>)> {
    let mut passed = Vec::new();
    if let Some(goal) = end_task.filter(|&g| !wfengine::can_complete(edges, end_task, current) && g != current) {
        if let Some(path) = wfengine::path_to(edges, current, goal) {
            for next in path {
                close_open_task(&mut *tx, id, current, None).await?;
                let assignee = default_assignee(&mut *tx, next).await?;
                open_next_task(&mut *tx, id, job_type_id, next, assignee).await?;
                audit::append(&mut *tx, actor, "wf_advance", "wf_job_instance", Some(&id.to_string()), json!({ "from_task": current, "to_task": next, "fast_track": true })).await?;
                passed.push(next);
                current = next;
            }
        }
    }
    if wfengine::can_complete(edges, end_task, current) {
        close_open_task(&mut *tx, id, current, comment).await?;
        sqlx::query("UPDATE cadastre.wf_job_instance SET status = 3, ended = now()::timestamp, updated = now()::timestamp WHERE id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        audit::append(&mut *tx, actor, "wf_complete", "wf_job_instance", Some(&id.to_string()), json!({ "from_task": current, "fast_track": true })).await?;
        return Ok((true, passed));
    }
    Ok((false, passed))
}

async fn commit_task_of(conn: &mut sqlx::PgConnection, job_type_id: i32) -> ApiResult<Option<i32>> {
    Ok(sqlx::query_scalar(
        "SELECT f.task_id FROM cadastre.wf_task_flow f \
         JOIN cadastre.wf_task_step ts ON ts.task_id = f.task_id \
         JOIN cadastre.wf_step s ON s.id = ts.step_id \
         WHERE f.job_type_id = $1 AND EXISTS (SELECT 1 FROM unnest(s.commands) c WHERE upper(c) = upper($2)) \
         ORDER BY f.task_id LIMIT 1",
    )
    .bind(job_type_id)
    .bind(COMMIT_TOOL)
    .fetch_optional(conn)
    .await?)
}

/// Close the job's open instance of `task` (Aumentum stamps the task's end time and comment).
async fn close_open_task(conn: &mut sqlx::PgConnection, id: i32, task: i32, comment: Option<&str>) -> ApiResult<()> {
    sqlx::query(
        "UPDATE cadastre.wf_task_instance SET ended = now()::timestamp, started = COALESCE(started, now()::timestamp), \
         comment = COALESCE($3, comment) WHERE job_instance_id = $1 AND task_id = $2 AND ended IS NULL",
    )
    .bind(id).bind(task).bind(comment)
    .execute(conn)
    .await?;
    Ok(())
}

/// Move the job onto `next`, opening a task instance for it and assigning its default principal.
async fn open_next_task(conn: &mut sqlx::PgConnection, id: i32, job_type_id: i32, next: i32, assignee: Option<i32>) -> ApiResult<()> {
    sqlx::query("UPDATE cadastre.wf_job_instance SET current_task_id = $2, assigned_sid = $3, status = 2, updated = now()::timestamp WHERE id = $1")
        .bind(id).bind(next).bind(assignee).execute(&mut *conn).await?;
    sqlx::query("INSERT INTO cadastre.wf_task_instance (job_instance_id, job_type_id, task_id, sid, assigned) VALUES ($1, $2, $3, $4, now()::timestamp)")
        .bind(id).bind(job_type_id).bind(next).bind(assignee).execute(&mut *conn).await?;
    notify_assignment(conn, id, next, assignee).await?;
    Ok(())
}

/// Fast-track a job from its current task straight to its commit task, post the drafts (allocating
/// UPINs), and — unless `complete` is false — carry it on to its end task and complete it. This is
/// the editor's one-click "post this parcel" for a seamless digitize-and-register flow; it applies
/// the same rules as stepping through each task by hand (permissions, the no-overlap check, audit),
/// all in one transaction.
#[utoipa::path(post, path = "/api/v1/wf/jobs/{id}/finish", tag = "wf", request_body = WfFinishBody,
    params(("id" = i32, Path, description = "Job instance id")),
    responses((status = 200, body = WfFinishDto), (status = 409, description = "No draft, no commit task, or blocked")))]
pub async fn finish(State(state): State<AppState>, user: AuthUser, Path(id): Path<i32>, Json(req): Json<WfFinishBody>) -> ApiResult<Json<WfFinishDto>> {
    let actor = resolve_actor(&state, &user).await?;
    let complete = req.complete.unwrap_or(true);
    // Post & Finish commits too: the same system check first.
    let check = crate::routes::precheck::run(&state, id, &[], None).await?;
    if !check.ok {
        return Err(AppError::Conflict(format!("System check failed, nothing was committed. {}", crate::routes::precheck::failures(&check))));
    }
    let mut tx = state.pool.begin().await?;
    let row = sqlx::query(
        "SELECT j.job_type_id, j.current_task_id, s.is_open FROM cadastre.wf_job_instance j \
         JOIN cadastre.wf_job_status s ON s.code = j.status WHERE j.id = $1 FOR UPDATE OF j",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("job instance {id}")))?;
    let (job_type_id, current, is_open): (i32, Option<i32>, bool) = (row.get(0), row.get(1), row.get(2));
    if !is_open {
        return Err(AppError::Conflict("the job is not open".into()));
    }
    let mut current = current.ok_or_else(|| AppError::Conflict("job has no current task".into()))?;

    // There must be a parcel to post, and a commit task to post it at.
    let drafts: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM cadastre.parcel p JOIN cadastre.wf_job_instance w ON w.cadastre_job_id = p.job_id WHERE w.id = $1 AND p.status = 'draft'",
    )
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;
    if drafts == 0 {
        return Err(AppError::Conflict("draw a parcel before posting (no draft parcels to commit)".into()));
    }
    let commit_task = commit_task_of(&mut tx, job_type_id)
        .await?
        .ok_or_else(|| AppError::Conflict("this job type has no commit task configured".into()))?;

    let (edges, end_task) = graph(&state.pool, job_type_id).await?;
    let mut passed: Vec<i32> = Vec::new();

    // Walk to the commit task, acting on each task the actor is permitted for. (Any refusal here
    // rolls the whole transaction back, so nothing is posted until the walk succeeds.)
    if current != commit_task {
        let path = wfengine::path_to(&edges, current, commit_task)
            .ok_or_else(|| AppError::Conflict("no path from the current task to the commit task".into()))?;
        for next in path {
            if !wfengine::may_act(&task_permitted(&mut tx, current).await?, &actor.sids, actor.admin) {
                return Err(AppError::Forbidden(format!("{} may not act on a task on the way to commit", actor.label)));
            }
            close_open_task(&mut tx, id, current, req.comment.as_deref()).await?;
            let assignee = default_assignee(&mut tx, next).await?;
            open_next_task(&mut tx, id, job_type_id, next, assignee).await?;
            audit::append(&mut tx, &actor.label, "wf_advance", "wf_job_instance", Some(&id.to_string()), json!({ "from_task": current, "to_task": next, "fast_track": true })).await?;
            passed.push(next);
            current = next;
        }
    }

    // Post the drafts at the commit task.
    if !wfengine::may_act(&task_permitted(&mut tx, current).await?, &actor.sids, actor.admin) {
        return Err(AppError::Forbidden(format!("{} may not commit this job", actor.label)));
    }
    let c = commit_drafts(&mut tx, id, &crate::routes::wfbeacons::WfCommitRequest { beacons: req.beacons.clone(), staff_name: req.staff_name.clone(), staff_sid: req.staff_sid }).await?;
    let numbers = c.numbers;
    let staff = staff_of(&state.pool, req.staff_sid, req.staff_name.as_deref()).await?;
    let who = committed_by(staff.as_deref(), &actor.label);
    audit::append(&mut tx, &who, "wf_commit", "wf_job_instance", Some(&id.to_string()), json!({ "parcel_numbers": numbers, "beacons_added": c.beacons_added, "retired": c.retired, "fast_track": true, "staff_name": staff, "staff_sid": req.staff_sid, "login": actor.label })).await?;

    // Carry on to the end task and complete. Best-effort: the parcels are already posted, so a
    // restricted downstream task simply leaves the job open there rather than failing the post.
    let mut completed = false;
    if complete {
        let (done, more) = complete_after_commit(&mut tx, id, job_type_id, current, &edges, end_task, &who, req.comment.as_deref()).await?;
        completed = done;
        passed.extend(more);
    }

    tx.commit().await?;
    let registry_links = crate::routes::registry::link_after_commit(&state, &who, id, &numbers).await;
    let detail = load_detail(&state, id).await?;
    let tasks_passed = task_refs(&state.pool, &passed).await?;
    Ok(Json(WfFinishDto { parcel_numbers: numbers, tasks_passed, completed, job: detail.job, registry_links }))
}

/// A job's Registry linkage target (the property its LRS transaction serves). Parcels committed
/// under the job inherit this (per-parcel capture overrides it field by field).
#[derive(Default, Serialize, Deserialize, ToSchema)]
pub struct WfLinkageDto {
    /// The job's LRS transaction number (read-only, carried on the job).
    pub lrs_transaction_number: Option<String>,
    /// Registry property id (LRSID) committed parcels link to.
    pub lrs_id: Option<String>,
    pub property_number: Option<String>,
    pub property_type: Option<String>,
    pub plot: Option<String>,
    pub block: Option<String>,
    pub plan_number: Option<String>,
    pub lga: Option<String>,
    /// 1 = linked, 0 = pending.
    pub linkage_status: Option<i16>,
}

/// Read the job's Registry linkage target.
#[utoipa::path(get, path = "/api/v1/wf/jobs/{id}/linkage", tag = "wf", params(("id" = i32, Path, description = "Job instance id")),
    responses((status = 200, body = WfLinkageDto)))]
pub async fn get_linkage(State(state): State<AppState>, Path(id): Path<i32>) -> ApiResult<Json<WfLinkageDto>> {
    let txn: Option<String> = sqlx::query_scalar("SELECT lrs_transaction_number FROM cadastre.wf_job_instance WHERE id = $1")
        .bind(id).fetch_optional(&state.pool).await?.flatten();
    let mut dto = WfLinkageDto { lrs_transaction_number: txn, ..Default::default() };
    if let Some(r) = sqlx::query(
        "SELECT lrs_id, property_number, property_type, plot, block, plan_number, lga, linkage_status \
         FROM cadastre.wf_job_linkage WHERE wf_job_instance_id = $1",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await?
    {
        dto.lrs_id = r.get("lrs_id");
        dto.property_number = r.get("property_number");
        dto.property_type = r.get("property_type");
        dto.plot = r.get("plot");
        dto.block = r.get("block");
        dto.plan_number = r.get("plan_number");
        dto.lga = r.get("lga");
        dto.linkage_status = r.get("linkage_status");
    }
    Ok(Json(dto))
}

/// Set the job's Registry linkage target (requires `editor`).
#[utoipa::path(put, path = "/api/v1/wf/jobs/{id}/linkage", tag = "wf", params(("id" = i32, Path, description = "Job instance id")),
    request_body = WfLinkageDto, responses((status = 200, body = WfLinkageDto)))]
pub async fn put_linkage(State(state): State<AppState>, user: AuthUser, Path(id): Path<i32>, Json(req): Json<WfLinkageDto>) -> ApiResult<Json<WfLinkageDto>> {
    let actor = resolve_actor(&state, &user).await?;
    let clean = |s: &Option<String>| s.as_ref().map(|x| x.trim().to_string()).filter(|x| !x.is_empty());
    let mut tx = state.pool.begin().await?;
    sqlx::query(
        "INSERT INTO cadastre.wf_job_linkage \
           (wf_job_instance_id, lrs_id, property_number, property_type, plot, block, plan_number, lga, linkage_status) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,COALESCE($9,1)) \
         ON CONFLICT (wf_job_instance_id) DO UPDATE SET \
           lrs_id = EXCLUDED.lrs_id, property_number = EXCLUDED.property_number, property_type = EXCLUDED.property_type, \
           plot = EXCLUDED.plot, block = EXCLUDED.block, plan_number = EXCLUDED.plan_number, lga = EXCLUDED.lga, \
           linkage_status = EXCLUDED.linkage_status, updated_at = now()",
    )
    .bind(id)
    .bind(clean(&req.lrs_id)).bind(clean(&req.property_number)).bind(clean(&req.property_type))
    .bind(clean(&req.plot)).bind(clean(&req.block)).bind(clean(&req.plan_number)).bind(clean(&req.lga))
    .bind(req.linkage_status)
    .execute(&mut *tx)
    .await?;
    audit::append(&mut tx, &actor.label, "wf_linkage_set", "wf_job_instance", Some(&id.to_string()), json!({ "lrs_id": clean(&req.lrs_id) })).await?;
    tx.commit().await?;
    get_linkage(State(state), Path(id)).await
}

#[derive(Deserialize, ToSchema)]
pub struct WfEnd {
    /// `completed` (status 3) or `cancelled` (status 9).
    pub outcome: String,
    pub comment: Option<String>,
}

/// End a job from any task (Aumentum: end / delete job instance). Administrators, or holders of
/// Aumentum's `ds_delete_jobinstance` role. Refused while the job has uncommitted parcels.
#[utoipa::path(post, path = "/api/v1/wf/jobs/{id}/end", tag = "wf", request_body = WfEnd, params(("id" = i32, Path, description = "Job instance id")),
    responses((status = 200, body = WfJobDetailDto), (status = 403, description = "Not allowed to end jobs"), (status = 409, description = "Closed already, or uncommitted parcels")))]
pub async fn end_job(State(state): State<AppState>, user: AuthUser, Path(id): Path<i32>, Json(req): Json<WfEnd>) -> ApiResult<Json<WfJobDetailDto>> {
    let actor = resolve_actor(&state, &user).await?;
    let status: i16 = match req.outcome.as_str() {
        "completed" => 3,
        "cancelled" => 9,
        _ => return Err(AppError::BadRequest("outcome must be completed or cancelled".into())),
    };
    let allowed = actor.admin
        || sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM cadastre.principal_role WHERE sid = ANY($1) AND role = 'ds_delete_jobinstance' AND allow)")
            .bind(&actor.sids)
            .fetch_one(&state.pool)
            .await?;
    if !allowed {
        return Err(AppError::Forbidden(format!("{} may not end jobs", actor.label)));
    }
    let mut tx = state.pool.begin().await?;
    let row = sqlx::query("SELECT j.current_task_id, s.is_open FROM cadastre.wf_job_instance j JOIN cadastre.wf_job_status s ON s.code = j.status WHERE j.id = $1 FOR UPDATE OF j")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("job instance {id}")))?;
    let (cur, open): (Option<i32>, bool) = (row.get(0), row.get(1));
    if !open {
        return Err(AppError::Conflict("the job is already closed".into()));
    }
    let drafts: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM cadastre.parcel p JOIN cadastre.wf_job_instance w ON w.cadastre_job_id = p.job_id WHERE w.id = $1 AND p.status = 'draft'",
    )
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;
    if drafts > 0 {
        return Err(AppError::Conflict(format!("commit or discard the job's {drafts} parcel(s) before ending it")));
    }
    if crate::routes::mutations::has_pending_retirement(&mut *tx, id).await? {
        return Err(AppError::Conflict("commit or discard the job's parcel retirement before ending it".into()));
    }
    sqlx::query("UPDATE cadastre.wf_task_instance SET ended = now()::timestamp, comment = COALESCE($2, comment) WHERE job_instance_id = $1 AND ended IS NULL")
        .bind(id)
        .bind(&req.comment)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE cadastre.wf_job_instance SET status = $2, ended = now()::timestamp, updated = now()::timestamp WHERE id = $1")
        .bind(id)
        .bind(status)
        .execute(&mut *tx)
        .await?;
    audit::append(&mut tx, &actor.label, "wf_end", "wf_job_instance", Some(&id.to_string()), json!({ "outcome": req.outcome, "at_task": cur, "comment": req.comment })).await?;
    tx.commit().await?;
    Ok(Json(load_detail(&state, id).await?))
}
