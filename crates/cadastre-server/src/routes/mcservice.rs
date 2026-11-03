//! The Registry's door into the Cadastre: Aumentum's `ApplicationServerAdmin/WebServices/
//! LRSWebServices.asmx`, served by this Cadastre at the same path, to the same contract (the WSDL
//! captured from the Registry, `assets/LRSWebServices.wsdl`: ASMX document/literal, namespace
//! `http://tempuri.org/`, SOAP 1.1).
//!
//! The Registry (LRS43, `lrs-mc-integration.jar`, decompiled) calls it with the account in its
//! `multicadastre` settings (`mcUserName` / `mcPassword`), at `mcServiceAddress`:
//! `Authenticate` → a session cookie it keeps → the calls → `LogOut`. It
//! * creates a job when a transaction reaches a stage in its `job_stage_description(n)` rules
//!   (`GetAllJobs` to find the job type by name, then `CreateJobInstance` with parameters
//!   `@LRS:[Transaction]:[TransactionNumber]=PLS…`) and keeps the answer in `lr_mc_job`;
//! * refreshes it (`GetJobInstance`), updates it, deletes it, and approves / rejects it when the
//!   transaction is completed / withdrawn;
//! * reads the Cadastre: a parcel's attributes, status, the UPINs of a job or transaction, and a
//!   parcel's boundary points (`GetParcelPointsData`).
//!
//! Switching the Registry to this Cadastre is one Registry setting (`mcServiceAddress`) and
//! `CADASTRE_MC_SERVICE_USER` / `CADASTRE_MC_SERVICE_PASSWORD` here matching `mcUserName` /
//! `mcPassword`. Until then the Registry keeps calling Aumentum.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use axum::body::Bytes;
use axum::extract::{RawQuery, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::json;
use sqlx::Row;

use crate::audit;
use crate::lrs::escape;
use crate::state::AppState;

const NS: &str = "http://tempuri.org/";
const WSDL: &str = include_str!("../../assets/LRSWebServices.wsdl");
const DOMAINS: &str = include_str!("../../assets/aumentum-domains.json");

/// Aumentum's coded-value domains (ParcelAttributeTableDefinition.xml), name → (code, description).
fn domains() -> &'static std::collections::BTreeMap<String, Vec<(String, String)>> {
    static D: OnceLock<std::collections::BTreeMap<String, Vec<(String, String)>>> = OnceLock::new();
    D.get_or_init(|| {
        let v: serde_json::Value = serde_json::from_str(DOMAINS).unwrap_or_default();
        v["domains"].as_object().map(|m| m.iter().map(|(k, vals)| (k.clone(), vals.as_array().map(|a| a.iter()
            .map(|x| (x["code"].as_str().unwrap_or("").to_string(), x["description"].as_str().unwrap_or("").to_string())).collect()).unwrap_or_default())).collect())
            .unwrap_or_default()
    })
}
pub const PATH: &str = "/ApplicationServerAdmin/WebServices/LRSWebServices.asmx";

/// Live sessions (cookie value → user), as ASP.NET kept them for Aumentum's ASMX.
fn sessions() -> &'static Mutex<HashMap<String, String>> {
    static S: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(HashMap::new()))
}

fn credentials() -> Option<(String, String)> {
    let get = |k: &str| std::env::var(k).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    Some((get("CADASTRE_MC_SERVICE_USER")?, get("CADASTRE_MC_SERVICE_PASSWORD")?))
}

/// The operation's local name and its parameters (direct children's text).
pub fn parse_request(xml: &str) -> Option<(String, HashMap<String, String>)> {
    use quick_xml::events::Event;
    let mut r = quick_xml::Reader::from_str(xml);
    let mut path: Vec<String> = Vec::new();
    let mut op: Option<String> = None;
    let mut params = HashMap::new();
    let mut current: Option<(String, String)> = None;
    loop {
        match r.read_event() {
            Ok(Event::Start(e)) => {
                let name = String::from_utf8_lossy(e.local_name().as_ref()).to_string();
                if path.last().map(String::as_str) == Some("Body") && op.is_none() {
                    op = Some(name.clone());
                } else if op.is_some() && path.len() >= 2 && path[path.len() - 2] == "Body" {
                    current = Some((name.clone(), String::new()));
                }
                path.push(name);
            }
            Ok(Event::Empty(e)) => {
                let name = String::from_utf8_lossy(e.local_name().as_ref()).to_string();
                if path.last().map(String::as_str) == Some("Body") && op.is_none() {
                    op = Some(name);
                } else if op.is_some() && path.len() >= 2 && path[path.len() - 2] == "Body" {
                    params.insert(name, String::new());
                }
            }
            Ok(Event::Text(t)) => {
                if let Some((_, v)) = current.as_mut() {
                    v.push_str(&t.unescape().ok()?);
                }
            }
            Ok(Event::CData(t)) => {
                if let Some((_, v)) = current.as_mut() {
                    v.push_str(&String::from_utf8_lossy(&t));
                }
            }
            Ok(Event::End(_)) => {
                if path.len() >= 3 && path[path.len() - 3] == "Body" {
                    if let Some((k, v)) = current.take() {
                        params.insert(k, v);
                    }
                }
                path.pop();
            }
            Ok(Event::Eof) => break,
            Err(_) => return None,
            _ => {}
        }
    }
    op.map(|o| (o, params))
}

fn envelope(inner: &str) -> String {
    format!("<?xml version=\"1.0\" encoding=\"utf-8\"?><soap:Envelope xmlns:soap=\"http://schemas.xmlsoap.org/soap/envelope/\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xmlns:xsd=\"http://www.w3.org/2001/XMLSchema\"><soap:Body>{inner}</soap:Body></soap:Envelope>")
}

fn fault(msg: &str) -> Response {
    let body = envelope(&format!("<soap:Fault><faultcode>soap:Server</faultcode><faultstring>{}</faultstring><detail /></soap:Fault>", escape(msg)));
    (StatusCode::INTERNAL_SERVER_ERROR, [(header::CONTENT_TYPE, "text/xml; charset=utf-8")], body).into_response()
}

fn ok(op: &str, result: &str) -> Response {
    let body = envelope(&format!("<{op}Response xmlns=\"{NS}\">{result}</{op}Response>"));
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/xml; charset=utf-8")], body).into_response()
}

/// Element text as .NET's XmlSerializer writes it: only `&`, `<` and `>` escaped.
fn text(v: &str) -> String {
    v.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}
fn el(name: &str, v: impl std::fmt::Display) -> String {
    format!("<{name}>{}</{name}>", text(&v.to_string()))
}
fn opt_el(name: &str, v: &Option<String>) -> String {
    v.as_ref().map(|s| el(name, s)).unwrap_or_default()
}
fn date_el(name: &str, v: Option<chrono::NaiveDateTime>) -> String {
    match v {
        Some(d) => el(name, d.format("%Y-%m-%dT%H:%M:%S%.3f")),
        None => format!("<{name} xsi:nil=\"true\" />"),
    }
}
fn bool_s(b: bool) -> &'static str {
    if b { "true" } else { "false" }
}

/// `@LRS:[Transaction]:[TransactionNumber]=PLS000139391` (several parameters may follow, one
/// per line or separated by `;` / `|`) → the transaction number.
pub fn transaction_from_parameters(p: &str) -> Option<String> {
    p.split(|c| c == '\n' || c == '\r' || c == ';' || c == '|')
        .filter_map(|kv| kv.split_once('='))
        .find(|(k, _)| k.trim().eq_ignore_ascii_case("@LRS:[Transaction]:[TransactionNumber]"))
        .map(|(_, v)| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

pub use crate::report::azimuth_bearing;

fn job_instance_xml(name: &str, r: &sqlx::postgres::PgRow) -> String {
    format!(
        "<{name}>{}{}{}{}{}{}{}{}{}{}{}{}{}</{name}>",
        opt_el("LrsTransactionNumber", &r.get::<Option<String>, _>("lrs_transaction_number")),
        el("ID", r.get::<i32, _>("id")),
        el("JobID", r.get::<i32, _>("job_type_id")),
        el("TaskID", r.get::<Option<i32>, _>("current_task_id").unwrap_or(0)),
        el("Sid", r.get::<Option<i32>, _>("assigned_sid").unwrap_or(0)),
        el("Priority", r.get::<i32, _>("priority_n")),
        date_el("Started", r.get("started")),
        date_el("Updated", r.get("updated")),
        date_el("Ended", r.get("ended")),
        opt_el("Comment", &r.get::<Option<String>, _>("comment")),
        opt_el("Name", &r.get::<Option<String>, _>("name")),
        opt_el("Parameters", &r.get::<Option<String>, _>("parameters")),
        el("JobStatus", r.get::<i16, _>("status")),
    )
}

const JOB_COLS: &str = "id, job_type_id, current_task_id, assigned_sid, CASE WHEN priority ~ '^\\d+$' THEN priority::int ELSE 1 END AS priority_n, \
    started, updated, ended, comment, name, parameters, status, lrs_transaction_number";

fn rows_xml(rows: &[Vec<(String, Option<String>)>]) -> String {
    rows.iter()
        .map(|fields| {
            let f: String = fields
                .iter()
                .map(|(k, v)| format!("<DataField>{}{}</DataField>", el("FieldName", k), v.as_ref().map(|v| el("FieldValue", v)).unwrap_or_default()))
                .collect();
            format!("<DataFieldRow><FieldsRow>{f}</FieldsRow></DataFieldRow>")
        })
        .collect()
}

fn strings_xml(v: &[String]) -> String {
    v.iter().map(|s| el("string", s)).collect()
}

/// Aumentum ParcelAttributeTable names of the parcel attribute record.
const ATTRIBUTE_FIELDS: &[(&str, &str)] = &[
    ("OBJECTID", "object_id::text"), ("DatasetName", "dataset_name"), ("RegionName", "region_name"), ("ParcelID", "parcel_id::text"),
    ("UPIN", "upin"), ("ParcelName", "parcel_name"), ("Parent", "parent"), ("PropertyNumber", "property_number"),
    ("PropertyType", "property_type"), ("Plot", "plot"), ("Block", "block"), ("Location", "location"), ("ParcelArea", "parcel_area::text"),
    ("AreaUnit", "area_unit::text"), ("PlanNumber", "plan_number"), ("LGA", "lga"), ("Status", "status"), ("CanRollback", "can_rollback::text"),
    ("IsStrata", "is_strata::text"), ("MutationType", "mutation_type::text"), ("CreationDate", "creation_date::text"),
    ("LastEditDate", "last_edit_date::text"), ("ConsolidationDate", "consolidation_date::text"), ("ArchiveReason", "archive_reason"),
    ("ArchiveDate", "archive_date::text"), ("LRSID", "lrs_id"), ("LinkageStatus", "linkage_status::text"), ("LinkageDate", "linkage_date"),
    ("SurveyorGeneral", "surveyor_general"),
];

fn int_param(p: &HashMap<String, String>, k: &str) -> Result<i32, String> {
    p.get(k).and_then(|v| v.trim().parse().ok()).ok_or_else(|| format!("missing or invalid {k}"))
}
fn str_param<'a>(p: &'a HashMap<String, String>, k: &str) -> Option<&'a str> {
    p.get(k).map(|s| s.trim()).filter(|s| !s.is_empty())
}

/// `GET …/LRSWebServices.asmx?wsdl`: the contract, pointing at this server.
pub async fn get(headers: HeaderMap, RawQuery(q): RawQuery) -> Response {
    let host = headers.get(header::HOST).and_then(|h| h.to_str().ok()).unwrap_or("localhost");
    if q.as_deref().is_some_and(|q| q.to_ascii_lowercase().starts_with("wsdl")) {
        let wsdl = WSDL.replace("http://172.25.31.93/ApplicationServerAdmin/WebServices/LRSWebServices.asmx", &format!("http://{host}{PATH}"));
        return ([(header::CONTENT_TYPE, "text/xml; charset=utf-8")], wsdl).into_response();
    }
    ([(header::CONTENT_TYPE, "text/html; charset=utf-8")],
     format!("<html><body><h3>LRSWebServices</h3><p>The Cadastre's service for the Aumentum Registry. Contract: <a href=\"{PATH}?WSDL\">WSDL</a>.</p></body></html>")).into_response()
}

/// `POST …/LRSWebServices.asmx`: one SOAP call.
pub async fn post(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let text = String::from_utf8_lossy(&body);
    let Some((op, p)) = parse_request(&text) else { return fault("Server did not recognize the request.") };
    let cookie = headers.get(header::COOKIE).and_then(|c| c.to_str().ok()).and_then(|c| {
        c.split(';').filter_map(|kv| kv.trim().split_once('=')).find(|(k, _)| *k == "ASP.NET_SessionId").map(|(_, v)| v.to_string())
    });
    match op.as_str() {
        "Authenticate" => {
            let Some((u, pw)) = credentials() else { return fault("The Cadastre's Registry service is not configured (CADASTRE_MC_SERVICE_USER / _PASSWORD).") };
            let good = str_param(&p, "user").is_some_and(|x| x.eq_ignore_ascii_case(&u)) && p.get("password").is_some_and(|x| *x == pw);
            let mut resp = ok(&op, &el("AuthenticateResult", bool_s(good)));
            if good {
                let sid = uuid::Uuid::new_v4().simple().to_string();
                sessions().lock().unwrap().insert(sid.clone(), u);
                resp.headers_mut().insert(header::SET_COOKIE, format!("ASP.NET_SessionId={sid}; path=/; HttpOnly").parse().unwrap());
            } else {
                tracing::warn!("Registry job service: sign-in refused for {:?}", p.get("user"));
            }
            return resp;
        }
        "LogOut" => {
            if let Some(c) = &cookie {
                sessions().lock().unwrap().remove(c);
            }
            return ok(&op, "");
        }
        _ => {}
    }
    let user = match cookie.and_then(|c| sessions().lock().unwrap().get(&c).cloned()) {
        Some(u) => u,
        None => return fault("Not authenticated: call Authenticate first."),
    };
    match call(&state, &user, &op, &p).await {
        Ok(result) => ok(&op, &result),
        Err(e) => {
            tracing::warn!("Registry job service {op} failed: {e}");
            fault(&e)
        }
    }
}

/// Create a job as the Registry's CreateJobInstance does (shared with the editor's Start
/// digitising): on the job type's start task, its default assignee, carrying the parameters and
/// the transaction number they name.
pub async fn create_job_instance(pool: &sqlx::PgPool, job_type: i32, name: Option<&str>, comment: Option<&str>, parameters: Option<&str>,
    priority: &str, actor: &str, via: &str, assign_default: bool) -> Result<i32, String> {
    let db = |e: sqlx::Error| e.to_string();
    let txn = parameters.and_then(transaction_from_parameters);
    let start: Option<i32> = sqlx::query_scalar::<_, Option<i32>>("SELECT start_task_id FROM cadastre.wf_job_type WHERE id = $1 AND active")
        .bind(job_type).fetch_optional(pool).await.map_err(db)?.ok_or_else(|| format!("job {job_type} not found"))?;
    let start = start.ok_or_else(|| format!("job {job_type} has no start task"))?;
    let mut tx = pool.begin().await.map_err(db)?;
    // Aumentum's CreateJobInstance leaves the first task unassigned (CreateNonAssignedTaskInstance).
    let assignee = if assign_default { crate::routes::wf::default_assignee(&mut tx, start).await.map_err(|e| e.to_string())? } else { None };
    let id: i32 = sqlx::query_scalar(
        "INSERT INTO cadastre.wf_job_instance (job_type_id, current_task_id, assigned_sid, priority, status, name, comment, parameters, lrs_transaction_number, created, started, updated, source) \
         VALUES ($1, $2, $3, $4, 2, $5, $6, $7, $8, now()::timestamp, now()::timestamp, now()::timestamp, 'cadastre') RETURNING id",
    )
    .bind(job_type).bind(start).bind(assignee).bind(priority).bind(name).bind(comment).bind(parameters).bind(&txn)
    .fetch_one(&mut *tx).await.map_err(db)?;
    sqlx::query("INSERT INTO cadastre.wf_task_instance (job_instance_id, job_type_id, task_id, sid, assigned, started) VALUES ($1, $2, $3, $4, now()::timestamp, now()::timestamp)")
        .bind(id).bind(job_type).bind(start).bind(assignee).execute(&mut *tx).await.map_err(db)?;
    if assignee.is_some() {
        crate::routes::wf::notify_assignment(&mut tx, id, start, assignee).await.map_err(|e| e.to_string())?;
    }
    audit::append(&mut tx, actor, "wf_create", "wf_job_instance", Some(&id.to_string()),
        json!({ "job_type_id": job_type, "task_id": start, "assigned_sid": assignee, "lrs_transaction_number": txn, "parameters": parameters, "via": via }))
        .await.map_err(db)?;
    tx.commit().await.map_err(db)?;
    Ok(id)
}

async fn job_row(state: &AppState, id: i32) -> Result<Option<sqlx::postgres::PgRow>, String> {
    sqlx::query(&format!("SELECT {JOB_COLS} FROM cadastre.wf_job_instance WHERE id = $1")).bind(id).fetch_optional(&state.pool).await.map_err(|e| e.to_string())
}

async fn set_status(state: &AppState, user: &str, id: i32, status: i16, action: &str) -> Result<bool, String> {
    let mut tx = state.pool.begin().await.map_err(|e| e.to_string())?;
    let n = sqlx::query("UPDATE cadastre.wf_job_instance SET status = $2, ended = COALESCE(ended, now()::timestamp), updated = now()::timestamp WHERE id = $1")
        .bind(id).bind(status).execute(&mut *tx).await.map_err(|e| e.to_string())?.rows_affected();
    if n > 0 {
        sqlx::query("UPDATE cadastre.wf_task_instance SET ended = now()::timestamp WHERE job_instance_id = $1 AND ended IS NULL").bind(id).execute(&mut *tx).await.map_err(|e| e.to_string())?;
        audit::append(&mut tx, &format!("registry:{user}"), action, "wf_job_instance", Some(&id.to_string()), json!({ "status": status })).await.map_err(|e| e.to_string())?;
    }
    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(n > 0)
}

/// UPINs of the parcels a job committed (its Cadastre job's posted parcels and their lineage).
async fn upins_of_jobs(state: &AppState, ids: &[i32]) -> Result<Vec<String>, String> {
    sqlx::query_scalar(
        "SELECT DISTINCT p.parcel_number FROM cadastre.wf_job_instance w JOIN cadastre.parcel p ON p.job_id = w.cadastre_job_id \
         WHERE w.id = ANY($1) AND p.parcel_number IS NOT NULL AND p.status <> 'draft' \
         UNION SELECT DISTINCT l.upin FROM cadastre.parcel_lineage l WHERE l.wf_job_instance_id = ANY($1) AND l.status = 1 AND l.upin IS NOT NULL ORDER BY 1",
    )
    .bind(ids)
    .fetch_all(&state.pool)
    .await
    .map_err(|e| e.to_string())
}

/// Jobs of a transaction, as Aumentum matched them: the job's
/// `@LRS:[Transaction]:[TransactionNumber]` parameter equal (ignoring case) to the number the
/// Registry passes (an int in the contract, so PLS numbers match only if stored as digits).
async fn jobs_of_transaction(state: &AppState, number: String) -> Result<Vec<i32>, String> {
    let rows: Vec<(i32, Option<String>)> = sqlx::query_as("SELECT id, parameters FROM cadastre.wf_job_instance WHERE parameters ILIKE '%@LRS:[Transaction]:[TransactionNumber]%' ORDER BY id")
        .fetch_all(&state.pool).await.map_err(|e| e.to_string())?;
    Ok(rows.into_iter().filter(|(_, p)| p.as_deref().and_then(transaction_from_parameters).is_some_and(|t| t.eq_ignore_ascii_case(&number))).map(|(id, _)| id).collect())
}

async fn call(state: &AppState, user: &str, op: &str, p: &HashMap<String, String>) -> Result<String, String> {
    let db = |e: sqlx::Error| e.to_string();
    Ok(match op {
        "GetAllJobs" => {
            let rows = sqlx::query("SELECT id, name, description, start_task_id, end_task_id FROM cadastre.wf_job_type WHERE active ORDER BY id")
                .fetch_all(&state.pool).await.map_err(db)?;
            let jobs: String = rows.iter().map(|r| format!("<WFJob>{}{}{}{}{}</WFJob>", el("ID", r.get::<i32, _>("id")),
                opt_el("Name", &r.get::<Option<String>, _>("name")), opt_el("Description", &r.get::<Option<String>, _>("description")),
                el("StartTaskId", r.get::<Option<i32>, _>("start_task_id").unwrap_or(0)), el("EndTaskId", r.get::<Option<i32>, _>("end_task_id").unwrap_or(0)))).collect();
            format!("<GetAllJobsResult>{jobs}</GetAllJobsResult>")
        }
        "CreateJobInstance" => {
            let job_type = int_param(p, "jobID")?;
            // Aumentum ConvertUInt2Char: priorities above 2 become '2'.
            let pr = str_param(p, "priority").and_then(|v| v.parse::<u16>().ok()).unwrap_or(1);
            let pr = if pr > 2 { "2".to_string() } else { pr.to_string() };
            let id = create_job_instance(&state.pool, job_type, str_param(p, "instanceName"), str_param(p, "comments"), str_param(p, "parameters"),
                &pr, &format!("registry:{user}"), "LRSWebServices.CreateJobInstance", false).await?;
            let r = job_row(state, id).await?.ok_or("job vanished")?;
            job_instance_xml("CreateJobInstanceResult", &r)
        }
        "GetJobInstance" => {
            let r = job_row(state, int_param(p, "jobInstanceId")?).await?;
            r.map(|r| job_instance_xml("GetJobInstanceResult", &r)).unwrap_or_default()
        }
        "GetJobInstanceParameters" => {
            let r = job_row(state, int_param(p, "jobInstanceId")?).await?;
            r.and_then(|r| r.get::<Option<String>, _>("parameters")).map(|v| el("GetJobInstanceParametersResult", v)).unwrap_or_default()
        }
        "UpdateJobInstance" | "UpdateJobInstanceParameters" => {
            let id = int_param(p, "jobInstanceID")?;
            let parameters = str_param(p, "parameters").map(str::to_string);
            let txn = parameters.as_deref().and_then(transaction_from_parameters);
            let n = if op == "UpdateJobInstance" {
                sqlx::query("UPDATE cadastre.wf_job_instance SET name = COALESCE($2, name), comment = COALESCE($3, comment), parameters = COALESCE($4, parameters), \
                             lrs_transaction_number = COALESCE($5, lrs_transaction_number), updated = now()::timestamp WHERE id = $1")
                    .bind(id).bind(str_param(p, "instanceName")).bind(str_param(p, "comments")).bind(&parameters).bind(&txn)
                    .execute(&state.pool).await.map_err(db)?.rows_affected()
            } else {
                sqlx::query("UPDATE cadastre.wf_job_instance SET parameters = $2, lrs_transaction_number = COALESCE($3, lrs_transaction_number), updated = now()::timestamp WHERE id = $1")
                    .bind(id).bind(&parameters).bind(&txn).execute(&state.pool).await.map_err(db)?.rows_affected()
            };
            el(&format!("{op}Result"), bool_s(n > 0))
        }
        "ApproveJob" => el("ApproveJobResult", bool_s(set_status(state, user, int_param(p, "jobInstanceID")?, 5, "wf_registry_approved").await?)),
        // Aumentum: DeleteJobAreaByJobInstanceID (the job's unposted edits) then status 7.
        "RejectJob" => {
            let id = int_param(p, "jobInstanceID")?;
            sqlx::query("DELETE FROM cadastre.parcel_lineage WHERE pending AND wf_job_instance_id = $1").bind(id).execute(&state.pool).await.map_err(db)?;
            sqlx::query("DELETE FROM cadastre.parcel p USING cadastre.wf_job_instance w WHERE w.id = $1 AND p.job_id = w.cadastre_job_id AND p.status = 'draft'")
                .bind(id).execute(&state.pool).await.map_err(db)?;
            el("RejectJobResult", bool_s(set_status(state, user, id, 7, "wf_registry_rejected").await?))
        }
        "CompleteJobInstance" => el("CompleteJobInstanceResult", bool_s(set_status(state, user, int_param(p, "jobInstanceID")?, 3, "wf_registry_completed").await?)),
        // A job the Registry withdraws is cancelled, not erased: its history and parcels stay.
        "DeleteJobInstance" => el("DeleteJobInstanceResult", bool_s(set_status(state, user, int_param(p, "jobInstanceId")?, 9, "wf_registry_deleted").await?)),
        // Aumentum's wording: 3 "Waiting for LRS Commit", 5/7 "Already been approved or rejected",
        // 2 "Active", otherwise "Unknown".
        "IsJobInstanceCompleted" => {
            let st: Option<i16> = sqlx::query_scalar("SELECT status FROM cadastre.wf_job_instance WHERE id = $1").bind(int_param(p, "jobinstanceID")?)
                .fetch_optional(&state.pool).await.map_err(db)?;
            let (done, label) = match st {
                Some(3) => (true, "Waiting for LRS Commit"),
                Some(5) | Some(7) => (true, "Already been approved or rejected"),
                Some(2) => (false, "Active"),
                _ => (false, "Unknown"),
            };
            format!("<IsJobInstanceCompletedResult>{}{}</IsJobInstanceCompletedResult>", el("Completed", bool_s(done)), el("Status", label))
        }
        "GetCurrentTaskInstance" => {
            let r = sqlx::query("SELECT id, job_type_id, task_id, sid, assigned, started, ended, comment, job_instance_id FROM cadastre.wf_task_instance \
                                 WHERE job_instance_id = $1 ORDER BY (ended IS NULL) DESC, id DESC LIMIT 1")
                .bind(int_param(p, "jobInstanceID")?).fetch_optional(&state.pool).await.map_err(db)?;
            match r {
                Some(r) => format!("<GetCurrentTaskInstanceResult>{}{}{}{}{}{}{}{}{}</GetCurrentTaskInstanceResult>",
                    el("ID", r.get::<i32, _>("id")), el("JobID", r.get::<Option<i32>, _>("job_type_id").unwrap_or(0)), el("TaskID", r.get::<i32, _>("task_id")),
                    el("Sid", r.get::<Option<i32>, _>("sid").unwrap_or(0)), date_el("Assigned", r.get("assigned")), date_el("Started", r.get("started")),
                    date_el("Ended", r.get("ended")), opt_el("Comment", &r.get::<Option<String>, _>("comment")), el("JobInstanceID", r.get::<i32, _>("job_instance_id"))),
                None => String::new(),
            }
        }
        // Finish the current task: move on when the task graph has exactly one way on.
        "CompleteTaskInstance" | "CancelTaskInstance" => {
            let id = int_param(p, "jobInstanceID")?;
            let mut tx = state.pool.begin().await.map_err(db)?;
            let cur = sqlx::query("SELECT job_type_id, current_task_id FROM cadastre.wf_job_instance WHERE id = $1 FOR UPDATE").bind(id).fetch_optional(&mut *tx).await.map_err(db)?;
            let Some(cur) = cur else { return Ok(el(&format!("{op}Result"), "false")) };
            let (jt, task): (i32, Option<i32>) = (cur.get("job_type_id"), cur.get("current_task_id"));
            sqlx::query("UPDATE cadastre.wf_task_instance SET ended = now()::timestamp, comment = COALESCE(comment, $2) WHERE job_instance_id = $1 AND ended IS NULL")
                .bind(id).bind(if op == "CancelTaskInstance" { Some("Cancelled by the Registry") } else { None }).execute(&mut *tx).await.map_err(db)?;
            let mut moved = op == "CancelTaskInstance";
            if op == "CompleteTaskInstance" {
                let next: Vec<Option<i32>> = sqlx::query_scalar("SELECT DISTINCT next_task_id FROM cadastre.wf_task_flow WHERE job_type_id = $1 AND task_id = $2")
                    .bind(jt).bind(task).fetch_all(&mut *tx).await.map_err(db)?;
                match next.as_slice() {
                    [Some(n)] => {
                        let a = crate::routes::wf::default_assignee(&mut tx, *n).await.map_err(|e| e.to_string())?;
                        sqlx::query("UPDATE cadastre.wf_job_instance SET current_task_id = $2, assigned_sid = $3, updated = now()::timestamp WHERE id = $1").bind(id).bind(n).bind(a).execute(&mut *tx).await.map_err(db)?;
                        sqlx::query("INSERT INTO cadastre.wf_task_instance (job_instance_id, job_type_id, task_id, sid, assigned, started) VALUES ($1, $2, $3, $4, now()::timestamp, now()::timestamp)")
                            .bind(id).bind(jt).bind(n).bind(a).execute(&mut *tx).await.map_err(db)?;
                        moved = true;
                    }
                    [] | [None] => {
                        sqlx::query("UPDATE cadastre.wf_job_instance SET status = 3, ended = now()::timestamp, updated = now()::timestamp WHERE id = $1").bind(id).execute(&mut *tx).await.map_err(db)?;
                        moved = true;
                    }
                    _ => {}
                }
            }
            audit::append(&mut tx, &format!("registry:{user}"), if op == "CancelTaskInstance" { "wf_registry_task_cancelled" } else { "wf_registry_task_completed" },
                "wf_job_instance", Some(&id.to_string()), json!({ "task_id": task, "moved": moved })).await.map_err(db)?;
            tx.commit().await.map_err(db)?;
            el(&format!("{op}Result"), bool_s(moved))
        }
        "GetAllJobsInstances" => {
            let filter = p.get("filter").and_then(|v| v.trim().parse::<i32>().ok()).unwrap_or(0);
            // Aumentum: 1 active, 2 all, 3 completed; any other filter gives nothing.
            let cond = match filter { 1 => "status IN (1, 2, 8)", 2 => "true", 3 => "status IN (3, 5, 7)", _ => "false" };
            let rows = sqlx::query(&format!("SELECT {JOB_COLS} FROM cadastre.wf_job_instance WHERE {cond} ORDER BY id DESC LIMIT 20000"))
                .fetch_all(&state.pool).await.map_err(db)?;
            let rows: Vec<Vec<(String, Option<String>)>> = rows.iter().map(|r| vec![
                ("ID".into(), Some(r.get::<i32, _>("id").to_string())), ("JobID".into(), Some(r.get::<i32, _>("job_type_id").to_string())),
                ("Name".into(), r.get("name")), ("JobStatus".into(), Some(r.get::<i16, _>("status").to_string())),
                ("LrsTransactionNumber".into(), r.get("lrs_transaction_number")),
            ]).collect();
            format!("<GetAllJobsInstancesResult>{}</GetAllJobsInstancesResult>", rows_xml(&rows))
        }
        "GetParcelAttributesByUPIN" => {
            let upin = str_param(p, "upin").ok_or("missing upin")?.to_uppercase();
            let cols: Vec<String> = ATTRIBUTE_FIELDS.iter().map(|(_, c)| c.to_string()).collect();
            let rows = sqlx::query(&format!("SELECT {} FROM cadastre.parcel_attribute WHERE upin = $1 ORDER BY (status = '0') DESC NULLS LAST, object_id DESC", cols.join(", ")))
                .bind(&upin).fetch_all(&state.pool).await.map_err(db)?;
            let rows: Vec<Vec<(String, Option<String>)>> = rows.iter().map(|r| ATTRIBUTE_FIELDS.iter().enumerate().map(|(i, (n, _))| (n.to_string(), r.get::<Option<String>, _>(i))).collect()).collect();
            format!("<GetParcelAttributesByUPINResult>{}</GetParcelAttributesByUPINResult>", rows_xml(&rows))
        }
        "GetParcelStatus" => {
            let upin = str_param(p, "upin").ok_or("missing upin")?.to_uppercase();
            let rows = sqlx::query("SELECT parcel_number, status, round(ST_Area(geom)::numeric, 4)::text AS area, posted_at::text AS posted FROM cadastre.parcel WHERE parcel_number = $1 ORDER BY (status = 'posted') DESC")
                .bind(&upin).fetch_all(&state.pool).await.map_err(db)?;
            let rows: Vec<Vec<(String, Option<String>)>> = rows.iter().map(|r| vec![("UPIN".into(), r.get("parcel_number")), ("Status".into(), Some(r.get::<String, _>("status"))),
                ("Area".into(), r.get("area")), ("PostedAt".into(), r.get("posted"))]).collect();
            format!("<GetParcelStatusResult>{}</GetParcelStatusResult>", rows_xml(&rows))
        }
        "GetUpinsByJobInstance" => format!("<GetUpinsByJobInstanceResult>{}</GetUpinsByJobInstanceResult>", strings_xml(&upins_of_jobs(state, &[int_param(p, "jobInstanceID")?]).await?)),
        "GetUpinsByTransaction" => {
            let jobs = jobs_of_transaction(state, p.get("transactionNumber").map(|v| v.trim().to_string()).ok_or("missing transactionNumber")?).await?;
            format!("<GetUpinsByTransactionResult>{}</GetUpinsByTransactionResult>", strings_xml(&upins_of_jobs(state, &jobs).await?))
        }
        // Aumentum returns the job's job type (WorkflowService.GetJob(item.JobID)).
        "GetJobStatusByTransaction" | "GetJobStatusByJobInstanceID" => {
            let ids = if op == "GetJobStatusByTransaction" {
                jobs_of_transaction(state, p.get("transactionNumber").map(|v| v.trim().to_string()).ok_or("missing transactionNumber")?).await?
            } else {
                vec![int_param(p, "jobInstanceID")?]
            };
            let rows = sqlx::query("SELECT jt.id, jt.name, jt.description, COALESCE(jt.start_task_id, 0) AS st, COALESCE(jt.end_task_id, 0) AS et \
                                    FROM cadastre.wf_job_instance j JOIN cadastre.wf_job_type jt ON jt.id = j.job_type_id WHERE j.id = ANY($1) ORDER BY j.id")
                .bind(&ids).fetch_all(&state.pool).await.map_err(db)?;
            let one = |r: &sqlx::postgres::PgRow, tag: &str| format!("<{tag}>{}{}{}{}{}</{tag}>", el("ID", r.get::<i32, _>("id")), opt_el("Name", &r.get::<Option<String>, _>("name")),
                opt_el("Description", &r.get::<Option<String>, _>("description")), el("StartTaskId", r.get::<i32, _>("st")), el("EndTaskId", r.get::<i32, _>("et")));
            if op == "GetJobStatusByTransaction" {
                format!("<GetJobStatusByTransactionResult>{}</GetJobStatusByTransactionResult>", rows.iter().map(|r| one(r, "WFJob")).collect::<String>())
            } else {
                rows.first().map(|r| one(r, "GetJobStatusByJobInstanceIDResult")).unwrap_or_default()
            }
        }
        "GetParcelPointsData" => {
            let upin = str_param(p, "pUpin").ok_or("missing pUpin")?.to_uppercase();
            let legs: String = crate::report::parcel_legs(&state.pool, &upin).await.map_err(db)?.iter().map(|l| {
                format!("<PointInformationDTO>{}{}{}{}{}{}{}{}{}</PointInformationDTO>", el("FromPointName", &l.from), el("FromNorth", l.from_n), el("FromEast", l.from_e),
                    el("ToPointName", &l.to), el("ToNorth", l.to_n), el("ToEast", l.to_e), el("Distance", format!("{:.3}", l.distance)), el("Azimuth", &l.azimuth), el("Bearing", &l.bearing))
            }).collect();
            format!("<GetParcelPointsDataResult>{legs}</GetParcelPointsDataResult>")
        }
        "GetConfiguredRegions" => format!("<GetConfiguredRegionsResult>{}</GetConfiguredRegionsResult>", strings_xml(&["Plateau_State".to_string()])),
        "GetDomains" => format!("<GetDomainsResult>{}</GetDomainsResult>", strings_xml(&domains().keys().cloned().collect::<Vec<_>>())),
        "GetDomainDetailsByName" => {
            let rows: Vec<Vec<(String, Option<String>)>> = domains().get(str_param(p, "domainName").unwrap_or("")).map(|v| v.iter()
                .map(|(c, d)| vec![("Code".to_string(), Some(c.clone())), ("Description".to_string(), Some(d.clone()))]).collect()).unwrap_or_default();
            format!("<GetDomainDetailsByNameResult>{}</GetDomainDetailsByNameResult>", rows_xml(&rows))
        }
        // The Registry's push of linkage values: `NAME=value` pairs (`|`, `;` or new lines), keyed
        // by UPIN; the Aumentum fields known here are written to the parcel's attribute record.
        "BatchUpdateDataFromRegistry" => {
            let raw = p.get("parameters").cloned().unwrap_or_default();
            let mut pairs: Vec<(String, Option<String>)> = Vec::new();
            for kv in raw.split(|c| c == '|' || c == ';' || c == '\n' || c == '\r') {
                if let Some((k, v)) = kv.split_once('=') {
                    pairs.push((k.trim().to_string(), Some(v.trim().to_string()).filter(|v| !v.is_empty())));
                }
            }
            let upin = pairs.iter().find(|(k, _)| k.eq_ignore_ascii_case("UPIN")).and_then(|(_, v)| v.clone());
            let mut messages = Vec::new();
            let mut tx = state.pool.begin().await.map_err(db)?;
            let mut changed = 0;
            match &upin {
                Some(u) => {
                    let updates: Vec<(String, Option<String>)> = pairs.iter().filter(|(k, _)| !k.eq_ignore_ascii_case("UPIN")).cloned().collect();
                    changed = crate::registry_sync::write_attributes(&mut tx, &u.to_uppercase(), &updates).await.map_err(db)?;
                    if changed == 0 { messages.push(format!("No attribute record of {u} changed.")); }
                }
                None => messages.push("No UPIN in the parameters.".to_string()),
            }
            audit::append(&mut tx, &format!("registry:{user}"), "registry_batch_update", "registry_link", upin.as_deref(), json!({ "parameters": raw, "changed": changed })).await.map_err(db)?;
            tx.commit().await.map_err(db)?;
            format!("<BatchUpdateDataFromRegistryResult>{}{}<Messages>{}</Messages></BatchUpdateDataFromRegistryResult>",
                el("Completed", bool_s(upin.is_some())), el("HasError", bool_s(upin.is_none())), strings_xml(&messages))
        }
        _ => return Err(format!("Server did not recognize the value of HTTP Header SOAPAction: {NS}{op}.")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_axis_requests() {
        let x = r#"<?xml version="1.0" encoding="UTF-8"?><soapenv:Envelope xmlns:soapenv="http://schemas.xmlsoap.org/soap/envelope/"><soapenv:Body><CreateJobInstance xmlns="http://tempuri.org/"><jobID>9</jobID><instanceName>Job-PL67779</instanceName><priority>1</priority><comments>Deed Plan Creationv2</comments><parameters>@LRS:[Transaction]:[TransactionNumber]=PLS000139832</parameters></CreateJobInstance></soapenv:Body></soapenv:Envelope>"#;
        let (op, p) = parse_request(x).unwrap();
        assert_eq!(op, "CreateJobInstance");
        assert_eq!(p["jobID"], "9");
        assert_eq!(p["instanceName"], "Job-PL67779");
        assert_eq!(transaction_from_parameters(&p["parameters"]).as_deref(), Some("PLS000139832"));
        let (op, p) = parse_request(r#"<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/"><s:Body><LogOut xmlns="http://tempuri.org/"/></s:Body></s:Envelope>"#).unwrap();
        assert_eq!((op.as_str(), p.len()), ("LogOut", 0));
    }

    #[test]
    fn bearings_as_aumentum_lists_them() {
        let (d, az, b) = azimuth_bearing((0.0, 0.0), (10.0, 10.0));
        assert!((d - 14.142).abs() < 0.001);
        assert_eq!((az.as_str(), b.as_str()), ("45.000000", "45°00'00\""));
        let (_, _, b) = azimuth_bearing((0.0, 0.0), (-1.0, -1.0));
        assert_eq!(b, "225°00'00\"");
        assert_eq!(transaction_from_parameters("x=1\n@LRS:[Transaction]:[TransactionNumber]= PLS1 ").as_deref(), Some("PLS1"));
        assert_eq!(transaction_from_parameters("nothing"), None);
    }
}
