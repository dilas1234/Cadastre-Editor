//! Correct a committed parcel in place: one file, one UPIN, one job.
//!
//! Correct reopens the file's own job at its commit task (the open one, else its latest; a job is
//! made only when the file has none here) with a **copy** of the parcel as a draft under the same
//! UPIN. The committed shape stays live, on the map and in the Registry's map, until the correction
//! is committed; then the old shape is kept as history (`superseded`) and the corrected one takes
//! its place. The attribute record stays where it is, so the Registry link (LRSID, LinkageStatus,
//! link date) is never broken and nothing is unlinked in the Registry. A parcel archived in
//! Aumentum with no reason and nothing in its place comes back the same way.
//!
//! Only the file's **current** UPIN is corrected ([`why_not_current`]). Aumentum's practice was to
//! archive a wrong parcel and digitise it again under a new UPIN, so a file can carry old UPINs;
//! those are never brought back while the file has a current one.
//!
//! Still refused: a parcel with split / merge lineage (its UPIN gave way to its children; that is
//! a mutation job), and one archived with a reason (retired on purpose).

use axum::{extract::State, Json};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::audit;
use crate::auth::AuthUser;
use crate::error::{ApiResult, AppError};
use crate::state::AppState;

/// The status of a shape a correction replaced: kept as history under the same UPIN.
pub const SUPERSEDED: &str = "superseded";

#[derive(Debug, Deserialize, ToSchema)]
pub struct CorrectFileRequest {
    /// The parcel to correct (committed here or in Aumentum, or archived in Aumentum by mistake).
    pub upin: String,
    /// Why (recorded with the job).
    pub reason: String,
}

/// Correct: reopen the file's job with a copy of the parcel to fix, under the same UPIN.
#[utoipa::path(post, path = "/api/v1/wf/correct", tag = "wf", request_body = CorrectFileRequest,
    responses((status = 200), (status = 404, description = "No shape for that UPIN"),
              (status = 409, description = "Lineage, archived on purpose, or the file's job cannot be found or made")))]
pub async fn correct(State(state): State<AppState>, user: AuthUser, Json(req): Json<CorrectFileRequest>) -> ApiResult<Json<Value>> {
    let actor = crate::routes::wf::resolve_actor(&state, &user).await?;
    Ok(Json(begin(&state, &actor.label, &req.upin, &req.reason, None).await?))
}

/// The shape a correction starts from.
struct Shape {
    id: Uuid,
    archived: bool,
    job_id: Option<Uuid>,
}

/// Start (or resume) a correction of `upin`. `prefer`: the job Correct was pressed on, used when it
/// is this file's job.
pub async fn begin(state: &AppState, actor: &str, upin: &str, reason: &str, prefer: Option<i32>) -> ApiResult<Value> {
    let upin = upin.trim().to_uppercase();
    let reason = reason.trim().to_string();
    if reason.is_empty() {
        return Err(AppError::BadRequest("say why the parcel is corrected".into()));
    }
    // Already being corrected: carry on in that job (reopened if it was closed meanwhile).
    let open: Option<(i32, Option<String>, Uuid)> = sqlx::query_as(
        "SELECT w.id, w.name, d.id FROM cadastre.parcel d JOIN cadastre.wf_job_instance w ON w.cadastre_job_id = d.job_id \
         WHERE d.parcel_number = $1 AND d.status = 'draft' ORDER BY w.id DESC LIMIT 1")
        .bind(&upin).fetch_optional(&state.pool).await?;
    if let Some((job, name, draft)) = open {
        let mut tx = state.pool.begin().await?;
        let task = reopen_at_commit(&mut tx, job).await?;
        tx.commit().await?;
        return Ok(json!({ "upin": upin, "parcel_id": draft, "job_instance_id": job, "job_name": name, "created": false, "resumed": true,
                          "reopened_at_task": task, "beacons": [] }));
    }
    let shape = shape_of(state, &upin).await?;
    if let Some(why) = why_not_current(&state.pool, &upin).await? {
        return Err(AppError::Conflict(format!("{upin} cannot be corrected: {why}")));
    }
    let lineage: i64 = sqlx::query_scalar("SELECT count(*) FROM cadastre.parcel_lineage WHERE parcel_id = $1 OR parent_id = $1 OR upin = $2 OR parent_upin = $2")
        .bind(shape.id).bind(&upin).fetch_one(&state.pool).await?;
    if lineage > 0 {
        return Err(AppError::Conflict(format!("{upin} has split / merge history: correct it with a mutation job")));
    }
    // Its attribute record: the current one, else the latest (an archived parcel's).
    let attr = sqlx::query(
        "SELECT a.object_id, row_to_json(a)::jsonb AS rec, a.property_number, NULLIF(trim(a.archive_reason), '') AS archive_reason \
         FROM cadastre.parcel_attribute a WHERE a.upin = $1 ORDER BY (a.status = '0' OR a.status IS NULL) DESC, a.object_id DESC LIMIT 1")
        .bind(&upin).fetch_optional(&state.pool).await?;
    if shape.archived {
        if let Some(why) = attr.as_ref().and_then(|a| a.get::<Option<String>, _>("archive_reason")) {
            return Err(AppError::Conflict(format!("{upin} was archived on purpose ({why}): it is not brought back")));
        }
    }
    let file = attr.as_ref().and_then(|a| a.get::<Option<String>, _>("property_number"))
        .map(|f| f.split_whitespace().collect::<String>().to_uppercase()).filter(|f| !f.is_empty());
    let (job, name, created, txn) = file_job(state, actor, &upin, shape.job_id, file.as_deref(), prefer).await?;
    let result = start(state, actor, job, &shape, &upin, attr.as_ref().map(|a| a.get::<Value, _>("rec")), &reason, txn.as_deref(), created).await;
    if result.is_err() && created {
        let _ = sqlx::query("DELETE FROM cadastre.wf_job_instance WHERE id = $1").bind(job).execute(&state.pool).await;
    }
    let (draft, task, beacons) = result?;
    Ok(json!({ "upin": upin, "parcel_id": draft, "job_instance_id": job, "job_name": name, "created": created, "resumed": false,
               "reinstated": shape.archived, "transaction_number": txn, "reopened_at_task": task,
               "beacons": beacons.iter().map(|(n, e, nn)| json!({ "number": n, "easting": e, "northing": nn })).collect::<Vec<_>>() }))
}

/// One UPIN of a file, for [`why_not_current`].
#[derive(sqlx::FromRow)]
struct FileUpin {
    upin: String,
    live: bool,
    /// The Registry holds it on a property (the last Registry sync).
    registry: bool,
    /// Live, and on the same land as the UPIN asked about (over half the smaller shape).
    same_land: bool,
}

/// Why `upin` is not its file's current UPIN, or `None` when it is (or its file is not known).
/// Aumentum archived a wrong parcel and digitised it again under a new UPIN, so a file can hold
/// several. The current one is, in order:
/// 1. the file's live UPIN: an archived UPIN never comes back while the file has a live one;
/// 2. with none live: the UPIN the Registry holds, else the file's most recent;
/// 3. several live: each on its own land is current (a file of several plots); several on the same
///    land are duplicates, and only the one the Registry holds is current; with none held, none is,
///    until the duplicate is retired.
pub async fn why_not_current(pool: &sqlx::PgPool, upin: &str) -> ApiResult<Option<String>> {
    let file: Option<String> = sqlx::query_scalar(
        "SELECT upper(replace(property_number, ' ', '')) FROM cadastre.parcel_attribute WHERE upin = $1 AND coalesce(trim(property_number), '') <> '' \
         ORDER BY (status = '0' OR status IS NULL) DESC, object_id DESC LIMIT 1")
        .bind(upin).fetch_optional(pool).await?;
    let Some(file) = file else { return Ok(None) };
    // The file's UPINs, most recent first.
    let all: Vec<FileUpin> = sqlx::query_as(
        "SELECT a.upin, \
                EXISTS (SELECT 1 FROM cadastre.parcel p WHERE p.parcel_number = a.upin AND p.status = 'posted') AS live, \
                EXISTS (SELECT 1 FROM cadastre.registry_property r WHERE r.upin = a.upin) AS registry, \
                (a.upin <> $2 AND EXISTS (SELECT 1 FROM cadastre.parcel p JOIN cadastre.parcel q ON q.parcel_number = $2 AND q.status = 'posted' AND q.geom && p.geom \
                   WHERE p.parcel_number = a.upin AND p.status = 'posted' \
                     AND ST_Area(ST_Intersection(p.geom, q.geom)) > 0.5 * least(ST_Area(p.geom), ST_Area(q.geom)))) AS same_land \
         FROM cadastre.parcel_attribute a WHERE upper(replace(a.property_number, ' ', '')) = $1 AND a.upin IS NOT NULL \
         GROUP BY a.upin ORDER BY max(a.creation_date) DESC NULLS LAST, a.upin DESC")
        .bind(&file).bind(upin).fetch_all(pool).await?;
    let Some(me) = all.iter().find(|u| u.upin == upin) else { return Ok(None) };
    let others: Vec<&FileUpin> = all.iter().filter(|u| u.upin != upin).collect();
    if others.is_empty() {
        return Ok(None);
    }
    let names = |v: &[&FileUpin]| v.iter().map(|u| u.upin.as_str()).collect::<Vec<_>>().join(", ");
    if !me.live {
        let live: Vec<&FileUpin> = others.iter().copied().filter(|u| u.live).collect();
        if !live.is_empty() {
            return Ok(Some(format!("it is an old UPIN of file {file}; the file's current UPIN is {}: correct that instead", names(&live))));
        }
        if !me.registry {
            if let Some(held) = others.iter().find(|u| u.registry) {
                return Ok(Some(format!("it is an old UPIN of file {file}; the Registry holds {} for the file: correct that instead", held.upin)));
            }
            if all[0].upin != upin {
                return Ok(Some(format!("it is an old UPIN of file {file}; the file's most recent UPIN is {}: correct that instead", all[0].upin)));
            }
        }
        return Ok(None);
    }
    let dupes: Vec<&FileUpin> = others.iter().copied().filter(|u| u.same_land).collect();
    if dupes.is_empty() || me.registry {
        return Ok(None);
    }
    Ok(Some(match dupes.iter().find(|u| u.registry) {
        Some(held) => format!("file {file} has another live UPIN on the same land, {}, and the Registry holds that one: correct {} instead", held.upin, held.upin),
        None => format!("file {file} has several live UPINs on the same land ({upin}, {}) and the Registry holds none of them: retire the duplicate first", names(&dupes)),
    }))
}

/// The committed shape, else one archived (by mistake) with nothing committed in its place.
async fn shape_of(state: &AppState, upin: &str) -> ApiResult<Shape> {
    let row = sqlx::query("SELECT id, status, job_id FROM cadastre.parcel WHERE parcel_number = $1 AND status IN ('posted', 'archived') \
                           ORDER BY (status = 'posted') DESC, updated_at DESC LIMIT 1")
        .bind(upin).fetch_optional(&state.pool).await?
        .ok_or_else(|| AppError::NotFound(format!("{upin} has no committed or archived shape in this Cadastre")))?;
    Ok(Shape { id: row.get("id"), archived: row.get::<String, _>("status") == "archived", job_id: row.get("job_id") })
}

/// The file's job: the one Correct was pressed on (if it is this file's), the job that committed
/// the parcel, else the file's latest job here (an open one first); else one is made on the
/// file's Registry transaction (its Active one, else its latest). Cancelled jobs are skipped.
/// Returns (id, name, created, transaction).
async fn file_job(state: &AppState, actor: &str, upin: &str, own: Option<Uuid>, file: Option<&str>, prefer: Option<i32>)
    -> ApiResult<(i32, String, bool, Option<String>)> {
    let local: Option<(i32, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT w.id, w.name, w.lrs_transaction_number FROM cadastre.wf_job_instance w \
         LEFT JOIN cadastre.wf_job_status s ON s.code = w.status LEFT JOIN cadastre.wf_job_linkage l ON l.wf_job_instance_id = w.id \
         WHERE w.status <> 9 AND ((w.cadastre_job_id = $1) \
            OR ($2::text IS NOT NULL AND (upper(regexp_replace(coalesce(w.name, ''), '\\s', '', 'g')) = 'JOB-' || $2 \
                                          OR upper(regexp_replace(coalesce(l.property_number, ''), '\\s', '', 'g')) = $2))) \
         ORDER BY (w.id = $3) DESC, (w.cadastre_job_id IS NOT DISTINCT FROM $1 AND $1 IS NOT NULL) DESC, coalesce(s.is_open, false) DESC, w.id DESC LIMIT 1")
        .bind(own).bind(file).bind(prefer.unwrap_or(-1)).fetch_optional(&state.pool).await?;
    if let Some((id, name, txn)) = local {
        return Ok((id, name.unwrap_or_default(), false, txn));
    }
    let file = file.ok_or_else(|| AppError::Conflict(format!("{upin} has no file number on its attributes and no job here: its file is not known")))?;
    let file = crate::routes::registry::ident("file number", file)?;
    let cfg = crate::routes::registry::config()?;
    let session = crate::lrs::Session::start(&cfg).await.map_err(|e| AppError::Conflict(format!("the Registry cannot be reached to find {file}'s transaction: {e}")))?;
    let rows = session.query(&format!(
        "select t.TransactionNumber, ts.Code from com.landsystems.lrs.model.Transaction t join t.TransactionStatus ts \
         join t.Properties l join l.SpatialUnits s where upper(s.Refnum) = '{file}' and l.EndLifespanVersion is null and s.EndLifespanVersion is null \
         order by t.TransactionNumber desc"), 50).await;
    session.end().await;
    let rows = rows.map_err(|e| AppError::Conflict(format!("the Registry could not be read: {e}")))?;
    let txns: Vec<String> = rows.iter().filter_map(|r| r.first()).map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect();
    if txns.is_empty() {
        return Err(AppError::Conflict(format!("{file} has no transaction in the Registry: its job cannot be made")));
    }
    // A job here on one of the file's transactions (named differently).
    let on_txn: Option<(i32, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT w.id, w.name, w.lrs_transaction_number FROM cadastre.wf_job_instance w LEFT JOIN cadastre.wf_job_status s ON s.code = w.status \
         WHERE w.status <> 9 AND w.lrs_transaction_number = ANY($1) ORDER BY coalesce(s.is_open, false) DESC, w.id DESC LIMIT 1")
        .bind(&txns).fetch_optional(&state.pool).await?;
    if let Some((id, name, txn)) = on_txn {
        return Ok((id, name.unwrap_or_default(), false, txn));
    }
    let txn = rows.iter().find(|r| r.get(1).is_some_and(|c| c.trim() == "transaction_lodged")).map(|r| r[0].trim().to_string())
        .unwrap_or_else(|| txns[0].clone());
    let (id, name) = crate::routes::registry::create_file_job(&state.pool, actor, &txn, Some(&file), "wf.correct").await?;
    Ok((id, name, true, Some(txn)))
}

/// The job type's commit task.
async fn commit_task(tx: &mut sqlx::PgConnection, job_type: i32) -> ApiResult<i32> {
    sqlx::query_scalar(
        "SELECT f.task_id FROM cadastre.wf_task_flow f JOIN cadastre.wf_task_step ts ON ts.task_id = f.task_id JOIN cadastre.wf_step s ON s.id = ts.step_id \
         WHERE f.job_type_id = $1 AND EXISTS (SELECT 1 FROM unnest(s.commands) c WHERE upper(c) = upper($2)) ORDER BY f.task_id LIMIT 1")
        .bind(job_type).bind(crate::routes::wf::COMMIT_TOOL).fetch_optional(&mut *tx).await?
        .ok_or_else(|| AppError::Conflict("the file's job type has no commit task".into()))
}

/// Open the job at its commit task (other open tasks end). Returns the task.
async fn reopen_at_commit(tx: &mut sqlx::PgConnection, job: i32) -> ApiResult<i32> {
    let job_type: i32 = sqlx::query_scalar("SELECT job_type_id FROM cadastre.wf_job_instance WHERE id = $1 FOR UPDATE").bind(job).fetch_one(&mut *tx).await?;
    let task = commit_task(&mut *tx, job_type).await?;
    sqlx::query("UPDATE cadastre.wf_task_instance SET ended = now()::timestamp, comment = coalesce(comment, 'moved to the commit task for a correction') \
                 WHERE job_instance_id = $1 AND ended IS NULL AND task_id <> $2").bind(job).bind(task).execute(&mut *tx).await?;
    let at_commit: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM cadastre.wf_task_instance WHERE job_instance_id = $1 AND task_id = $2 AND ended IS NULL)")
        .bind(job).bind(task).fetch_one(&mut *tx).await?;
    if !at_commit {
        let sid: Option<i32> = crate::routes::wf::default_assignee(&mut *tx, task).await?;
        sqlx::query("INSERT INTO cadastre.wf_task_instance (job_instance_id, job_type_id, task_id, sid, assigned, started) VALUES ($1, $2, $3, $4, now()::timestamp, now()::timestamp)")
            .bind(job).bind(job_type).bind(task).bind(sid).execute(&mut *tx).await?;
    }
    sqlx::query("UPDATE cadastre.wf_job_instance SET status = 2, ended = NULL, current_task_id = $2, updated = now()::timestamp WHERE id = $1")
        .bind(job).bind(task).execute(&mut *tx).await?;
    Ok(task)
}

/// The draft copy under the same UPIN, its attributes from the record (the record stays), and the
/// job at its commit task. Returns (draft id, task, the old shape's beacons by coordinates).
#[allow(clippy::too_many_arguments)]
async fn start(state: &AppState, actor: &str, job: i32, shape: &Shape, upin: &str, rec: Option<Value>, reason: &str, txn: Option<&str>, created: bool)
    -> ApiResult<(Uuid, i32, Vec<(String, f64, f64)>)> {
    let mut tx = state.pool.begin().await?;
    let w = sqlx::query("SELECT cadastre_job_id, status FROM cadastre.wf_job_instance WHERE id = $1 FOR UPDATE").bind(job).fetch_one(&mut *tx).await?;
    let (cjob, was): (Option<Uuid>, i16) = (w.get(0), w.get(1));
    let cjob = match cjob {
        Some(c) => c,
        None => {
            let c: Uuid = sqlx::query_scalar(
                "INSERT INTO cadastre.job (reference, job_type, title, created_by, updated_by, current_stage_id, status) \
                 SELECT 'WF-' || w.id, 'aumentum:' || jt.name, w.name, $2, $2, s.id, s.code \
                 FROM cadastre.wf_job_instance w JOIN cadastre.wf_job_type jt ON jt.id = w.job_type_id, cadastre.workflow_stage s \
                 WHERE w.id = $1 AND s.is_initial RETURNING id")
                .bind(job).bind(actor).fetch_one(&mut *tx).await?;
            sqlx::query("UPDATE cadastre.wf_job_instance SET cadastre_job_id = $2, updated = now()::timestamp WHERE id = $1").bind(job).bind(c).execute(&mut *tx).await?;
            c
        }
    };
    let draft = Uuid::new_v4();
    sqlx::query("INSERT INTO cadastre.parcel (id, parcel_number, geom, stated_area, computed_area, status, job_id, created_at, updated_at) \
                 SELECT $2, parcel_number, geom, stated_area, ST_Area(geom), 'draft', $3, now(), now() FROM cadastre.parcel WHERE id = $1")
        .bind(shape.id).bind(draft).bind(cjob).execute(&mut *tx).await?;
    if let Some(rec) = &rec {
        let t = |k: &str| rec.get(k).and_then(|v| v.as_str()).map(str::to_string);
        sqlx::query(
            "INSERT INTO cadastre.parcel_draft_attribute (parcel_id, property_number, property_type, plot, block, location, plan_number, lga, parcel_name, area_unit, surveyor_general, status, layout, area_name) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, 'Active', $12, $13)")
            .bind(draft).bind(t("property_number")).bind(t("property_type")).bind(t("plot")).bind(t("block")).bind(t("location"))
            .bind(t("plan_number")).bind(t("lga")).bind(t("parcel_name")).bind(rec.get("area_unit").and_then(|v| v.as_i64()).map(|v| v as i16))
            .bind(t("surveyor_general")).bind(t("layout")).bind(t("area_name"))
            .execute(&mut *tx).await?;
    }
    let task = reopen_at_commit(&mut tx, job).await?;
    let beacons: Vec<(String, f64, f64)> = sqlx::query_as(
        "SELECT b.number, ST_X(b.geom), ST_Y(b.geom) FROM cadastre.parcel_corner pc JOIN cadastre.beacon b ON b.id = pc.beacon_id WHERE pc.parcel_id = $1 ORDER BY pc.seq")
        .bind(shape.id).fetch_all(&mut *tx).await?;
    audit::append(&mut tx, actor, "wf_reopened_for_correction", "wf_job_instance", Some(&job.to_string()), json!({
        "upin": upin, "parcel_id": draft, "replaces": shape.id, "reinstated": shape.archived, "reason": reason, "was_status": was,
        "reopened_at_task": task, "transaction_number": txn, "job_created": created, "committed_in": if shape.job_id.is_some() { "cadastre" } else { "aumentum" },
    })).await?;
    tx.commit().await?;
    Ok((draft, task, beacons))
}

/// UPINs of a job's drafts that correct a parcel (a draft carrying a UPIN), apart from those a
/// mutation of the job replaces. The system check and the commit do not count the parcel's own
/// committed shape (here or in Aumentum) or its Registry property against them.
pub async fn correcting_upins(conn: &mut sqlx::PgConnection, job_id: Uuid, mutated: &[String]) -> ApiResult<Vec<String>> {
    Ok(sqlx::query_scalar("SELECT DISTINCT parcel_number FROM cadastre.parcel WHERE job_id = $1 AND status = 'draft' AND parcel_number IS NOT NULL \
                           AND NOT (parcel_number = ANY($2)) ORDER BY 1")
        .bind(job_id).bind(mutated).fetch_all(conn).await?)
}

/// Beacons a correction lets go when it is committed: on the replaced shape's corners only, added
/// by a commit of that parcel (a beacon that was in the fabric before stays), not at the
/// correction's boundary and not on another committed parcel's boundary. A corner that moved takes
/// the number again at its new place.
pub async fn releasable_beacons(conn: &mut sqlx::PgConnection, job_id: Uuid, upins: &[String], tol: f64) -> ApiResult<Vec<Uuid>> {
    if upins.is_empty() {
        return Ok(Vec::new());
    }
    Ok(sqlx::query_scalar(
        "WITH old AS (SELECT id FROM cadastre.parcel WHERE status = 'posted' AND parcel_number = ANY($2)), \
              added AS (SELECT jsonb_array_elements_text(payload->'beacons_added') AS number FROM cadastre.audit_log \
                        WHERE action = 'wf_commit' AND jsonb_typeof(payload->'beacons_added') = 'array' AND payload->'parcel_numbers' ?| $2::text[]) \
         SELECT DISTINCT b.id FROM old JOIN cadastre.parcel_corner pc ON pc.parcel_id = old.id JOIN cadastre.beacon b ON b.id = pc.beacon_id \
         WHERE b.number IN (SELECT number FROM added) \
           AND NOT EXISTS (SELECT 1 FROM cadastre.parcel_corner x WHERE x.beacon_id = b.id AND x.parcel_id NOT IN (SELECT id FROM old)) \
           AND NOT EXISTS (SELECT 1 FROM cadastre.parcel d WHERE d.job_id = $1 AND d.status = 'draft' AND ST_DWithin(ST_Boundary(d.geom), b.geom, $3)) \
           AND NOT EXISTS (SELECT 1 FROM cadastre.parcel n WHERE n.status = 'posted' AND NOT (n.parcel_number = ANY($2)) AND n.geom && ST_Expand(b.geom, $3) \
                           AND ST_DWithin(ST_Boundary(n.geom), b.geom, $3))")
        .bind(job_id).bind(upins).bind(tol).fetch_all(conn).await?)
}

/// Before the commit's beacons: the replaced shape's corners come off, and the beacons it lets go
/// ([`releasable_beacons`]) are removed.
pub async fn release_corners(conn: &mut sqlx::PgConnection, job_id: Uuid, upins: &[String], tol: f64) -> ApiResult<()> {
    if upins.is_empty() {
        return Ok(());
    }
    let gone = releasable_beacons(&mut *conn, job_id, upins, tol).await?;
    sqlx::query("DELETE FROM cadastre.parcel_corner WHERE parcel_id IN (SELECT id FROM cadastre.parcel WHERE status = 'posted' AND parcel_number = ANY($1))")
        .bind(upins).execute(&mut *conn).await?;
    sqlx::query("DELETE FROM cadastre.beacon WHERE id = ANY($1)").bind(&gone).execute(&mut *conn).await?;
    Ok(())
}

/// At the commit, before the drafts are posted: each corrected parcel's attribute record takes the
/// draft's attributes and new area (its Registry link untouched), and the shape it replaces leaves
/// the live fabric as history under the same UPIN.
pub async fn supersede(conn: &mut sqlx::PgConnection, job_id: Uuid, upins: &[String]) -> ApiResult<()> {
    if upins.is_empty() {
        return Ok(());
    }
    sqlx::query(
        "UPDATE cadastre.parcel_attribute a SET property_number = da.property_number, property_type = da.property_type, plot = da.plot, block = da.block, \
           location = da.location, plan_number = da.plan_number, lga = da.lga, parcel_name = da.parcel_name, area_unit = da.area_unit, \
           surveyor_general = da.surveyor_general, layout = da.layout, area_name = da.area_name \
         FROM cadastre.parcel d JOIN cadastre.parcel_draft_attribute da ON da.parcel_id = d.id \
         WHERE d.job_id = $1 AND d.status = 'draft' AND d.parcel_number = ANY($2) AND a.upin = d.parcel_number AND (a.status = '0' OR a.status IS NULL)")
        .bind(job_id).bind(upins).execute(&mut *conn).await?;
    sqlx::query(
        "UPDATE cadastre.parcel_attribute a SET parcel_area = trunc(ST_Area(d.geom)::numeric, 2)::float8, last_edit_date = now()::timestamp, updated_at = now(), \
           parcel_id = CASE WHEN a.parcel_id IN (SELECT o.id FROM cadastre.parcel o WHERE o.parcel_number = d.parcel_number AND o.status IN ('posted', 'archived')) THEN d.id ELSE a.parcel_id END \
         FROM cadastre.parcel d WHERE d.job_id = $1 AND d.status = 'draft' AND d.parcel_number = ANY($2) AND a.upin = d.parcel_number AND (a.status = '0' OR a.status IS NULL)")
        .bind(job_id).bind(upins).execute(&mut *conn).await?;
    sqlx::query(&format!("UPDATE cadastre.parcel SET status = '{SUPERSEDED}', updated_at = now() WHERE status = 'posted' AND parcel_number = ANY($1)"))
        .bind(upins).execute(&mut *conn).await?;
    Ok(())
}
