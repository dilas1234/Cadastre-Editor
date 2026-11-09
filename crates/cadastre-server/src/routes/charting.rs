//! Charting and Reporting (`/reporting`; docs/charting-and-reporting.docx, parts of C2 and C4).
//!
//! Charting reviews the parcel committed at digitising against the old scanned cadastral sheets,
//! and the Survey Report on Right of Occupancy is filled in advance and confirmed by two officers:
//! the charting officer signs the first part, a different officer the Final Survey Report.
//! The sheets are the base of this view: the existing parcel is run against the legacy map to show
//! that the two agree, which is not the imagery Digitising and Title Deed work on.

use std::path::{Path as FsPath, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::extract::{Path, Query, State};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::Row;

use crate::audit;
use crate::auth::AuthUser;
use crate::error::{ApiResult, AppError};
use crate::state::AppState;

const MAX_ANSWERS_BYTES: usize = 64 * 1024;
const MAX_DEPTH: usize = 6;
const MAX_ENTRIES: usize = 20_000;
const INDEX_TTL: Duration = Duration::from_secs(600);

fn actor(user: &AuthUser) -> String {
    user.username.clone().unwrap_or_else(|| user.subject.clone())
}

#[derive(Debug, Deserialize)]
pub struct FilesQuery {
    #[serde(default)]
    pub q: String,
    /// `to_chart`, `to_report`, `to_finalise`, `done`, or empty for all.
    #[serde(default)]
    pub stage: String,
    #[serde(default)]
    pub offset: i64,
}

// Completed jobs with the file number and UPINs, as `completed::list` reads them. Until PLAGIS
// names the stage, every completed file counts as ready for charting.
const FILES: &str = "WITH base AS ( \
    SELECT j.id, j.name, j.lrs_transaction_number AS txn, j.cadastre_job_id, j.ended, \
           COALESCE(NULLIF(btrim(l.property_number), ''), substring(j.name from '^Job-(.+)$')) AS pn \
    FROM cadastre.wf_job_instance j LEFT JOIN cadastre.wf_job_linkage l ON l.wf_job_instance_id = j.id WHERE j.status = 3 \
), file AS ( \
    SELECT b.*, COALESCE(r.state, 'none') AS state, r.version, r.charted_by, r.final_by, \
           COALESCE((SELECT array_agg(p.parcel_number ORDER BY p.posted_at) FROM cadastre.parcel p \
                      WHERE b.cadastre_job_id IS NOT NULL AND p.job_id = b.cadastre_job_id AND p.status = 'posted' AND p.parcel_number IS NOT NULL), \
                    (SELECT array_agg(pa.upin ORDER BY pa.upin) FROM cadastre.parcel_attribute pa \
                      WHERE b.cadastre_job_id IS NULL AND b.pn IS NOT NULL AND pa.property_number = b.pn \
                        AND (pa.status = '0' OR pa.status IS NULL) AND pa.upin IS NOT NULL), '{}'::text[]) AS upins \
    FROM base b LEFT JOIN cadastre.charting_report r ON r.wf_job_instance_id = b.id \
) ";

fn file_json(r: &sqlx::postgres::PgRow) -> Value {
    json!({
        "id": r.get::<i32, _>("id"),
        "name": r.get::<Option<String>, _>("name"),
        "property_number": r.get::<Option<String>, _>("pn"),
        "transaction_number": r.get::<Option<String>, _>("txn"),
        "upins": r.get::<Vec<String>, _>("upins"),
        "state": r.get::<String, _>("state"),
        "version": r.get::<Option<i32>, _>("version"),
        "charted_by": r.get::<Option<String>, _>("charted_by"),
        "final_by": r.get::<Option<String>, _>("final_by"),
    })
}

/// The charting queue: files by where their report stands.
pub async fn files(State(state): State<AppState>, Query(q): Query<FilesQuery>) -> ApiResult<Json<Value>> {
    let states: &[&str] = match q.stage.as_str() {
        "to_chart" => &["none"],
        "to_report" => &["draft"],
        "to_finalise" => &["charted"],
        "done" => &["final"],
        _ => &["none", "draft", "charted", "final"],
    };
    let states: Vec<String> = states.iter().map(|s| s.to_string()).collect();
    let like = format!("%{}%", q.q.trim().to_uppercase());
    const WHERE: &str = "WHERE cardinality(upins) > 0 AND state = ANY($1) AND ($2 = '%%' OR upper(COALESCE(pn, '')) LIKE $2 OR upper(COALESCE(name, '')) LIKE $2 \
        OR upper(COALESCE(txn, '')) LIKE $2 OR EXISTS (SELECT 1 FROM unnest(upins) u WHERE upper(u) LIKE $2))";
    let total: i64 = sqlx::query_scalar(&format!("{FILES} SELECT count(*) FROM file {WHERE}"))
        .bind(&states).bind(&like).fetch_one(&state.pool).await?;
    let rows = sqlx::query(&format!("{FILES} SELECT * FROM file {WHERE} ORDER BY ended DESC NULLS LAST, id DESC LIMIT 100 OFFSET $3"))
        .bind(&states).bind(&like).bind(q.offset.max(0)).fetch_all(&state.pool).await?;
    Ok(Json(json!({ "total": total, "files": rows.iter().map(file_json).collect::<Vec<_>>() })))
}

/// One file: what is known of it, and its report.
pub async fn file(State(state): State<AppState>, user: AuthUser, Path(id): Path<i32>) -> ApiResult<Json<Value>> {
    let row = sqlx::query(&format!("{FILES} SELECT * FROM file WHERE id = $1")).bind(id).fetch_optional(&state.pool).await?
        .ok_or_else(|| AppError::NotFound(format!("completed file {id}")))?;
    let mut out = file_json(&row);
    let report = sqlx::query("SELECT answers, state, version, charted_by, charted_at, final_by, final_at, updated_by, updated_at FROM cadastre.charting_report WHERE wf_job_instance_id = $1")
        .bind(id).fetch_optional(&state.pool).await?;
    out["report"] = report.map_or(Value::Null, |r| json!({
        "answers": r.get::<Value, _>("answers"), "state": r.get::<String, _>("state"), "version": r.get::<i32, _>("version"),
        "charted_by": r.get::<Option<String>, _>("charted_by"), "charted_at": r.get::<Option<chrono::DateTime<chrono::Utc>>, _>("charted_at"),
        "final_by": r.get::<Option<String>, _>("final_by"), "final_at": r.get::<Option<chrono::DateTime<chrono::Utc>>, _>("final_at"),
    }));
    // The signed-in officer, as this server knows them.
    let me = actor(&user);
    let officer = sqlx::query("SELECT COALESCE(NULLIF(btrim(name), ''), code) AS name, job_title FROM cadastre.principal WHERE is_user AND lower(code) = lower($1)")
        .bind(&me).fetch_optional(&state.pool).await?;
    out["officer"] = officer.map_or_else(|| json!({ "name": me, "rank": null }), |r| json!({ "name": r.get::<String, _>("name"), "rank": r.get::<Option<String>, _>("job_title") }));
    let (txn, pn) = (row.get::<Option<String>, _>("txn"), row.get::<Option<String>, _>("pn"));
    let (fields, registry) = crate::routes::titledeed::registry_file_details(txn.as_deref(), pn.as_deref()).await;
    out["registry"] = json!({ "applicant": fields.holder, "location": fields.location, "address": fields.address, "lga": fields.lga, "tracing_no": fields.tracing_no, "plan_no": fields.plan_no, "note": registry });
    Ok(Json(out))
}

#[derive(Debug, Deserialize)]
pub struct SaveBody {
    pub answers: Value,
    /// The version this edit started from, so that one officer cannot overwrite another's work.
    #[serde(default)]
    pub version: Option<i32>,
}

/// What may change at this stage: only the part not yet signed.
pub fn merge_answers(state: &str, stored: &Value, incoming: &Value) -> Value {
    let part = |v: &Value, k: &str| v.get(k).cloned().unwrap_or_else(|| json!({}));
    match state {
        "charted" => json!({ "charting": part(stored, "charting"), "final": part(incoming, "final") }),
        _ => json!({ "charting": part(incoming, "charting"), "final": part(stored, "final") }),
    }
}

/// What a part still needs before it can be signed, in the form's words. A follow-up question only
/// counts when the answer above it calls for it.
pub fn missing(part: &str, answers: &Value) -> Vec<&'static str> {
    let a = &answers[part];
    let has = |k: &str| a[k].as_str().is_some_and(|v| !v.trim().is_empty());
    let is = |k: &str, v: &str| a[k].as_str() == Some(v);
    let mut out: Vec<&'static str> = Vec::new();
    let mut need = |k: &str, label: &'static str| if !has(k) { out.push(label); };
    if part == "charting" {
        for (k, l) in [("officer_name", "Charting officer"), ("rank", "Rank"), ("ref_no", "Ref no"), ("property_no", "Property No"), ("applicant", "Name of Applicant"),
                       ("area", "Area"), ("location", "Place / Location"), ("identified", "Question 1"), ("ground_open", "Question 2"), ("beaconed", "Question 3"),
                       ("town_plan", "Question 4"), ("under_title", "Question 5"), ("rail", "Question 6"), ("major_road", "Question 7"), ("settlement", "Question 8")] {
            need(k, l);
        }
        if is("ground_open", "No") { need("ground_detail", "What lies on the land (question 2)"); }
        if is("beaconed", "Yes") { need("tracing_no", "Tracing No (question 3)"); }
        if is("town_plan", "Yes") { need("tpo_no", "TPO No (question 4)"); }
        if is("under_title", "Yes") { need("title_no", "Title No (question 5)"); }
    } else {
        for (k, l) in [("officer_name", "Officer"), ("rank", "Rank"), ("ground_opened", "The ground is opened"), ("satisfactory", "Satisfactory (1)"),
                       ("description", "The correct description (2)"), ("check_survey", "Check survey (3)"), ("recommended", "Recommended (5)")] {
            need(k, l);
        }
        if is("satisfactory", "No") { need("satisfactory_detail", "Why it is not satisfactory (1)"); }
        if is("recommended", "Yes") { need("recommended_detail", "Subject to (5)"); }
    }
    out
}

async fn job_exists(pool: &sqlx::PgPool, id: i32) -> ApiResult<()> {
    let found: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM cadastre.wf_job_instance WHERE id = $1 AND status = 3)").bind(id).fetch_one(pool).await?;
    if found { Ok(()) } else { Err(AppError::NotFound(format!("completed file {id}"))) }
}

async fn keep_version(tx: &mut sqlx::PgConnection, id: i32, by: &str) -> ApiResult<()> {
    sqlx::query("INSERT INTO cadastre.charting_report_version (wf_job_instance_id, version, answers, state, saved_by) \
                 SELECT wf_job_instance_id, version, answers, state, $2 FROM cadastre.charting_report WHERE wf_job_instance_id = $1")
        .bind(id).bind(by).execute(&mut *tx).await?;
    Ok(())
}

/// Save the report. The part already signed does not change.
pub async fn save(State(state): State<AppState>, user: AuthUser, Path(id): Path<i32>, Json(req): Json<SaveBody>) -> ApiResult<Json<Value>> {
    user.require_role("editor")?;
    if !req.answers.is_object() || req.answers.to_string().len() > MAX_ANSWERS_BYTES {
        return Err(AppError::BadRequest("the report is not valid".into()));
    }
    job_exists(&state.pool, id).await?;
    let by = actor(&user);
    let mut tx = state.pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(4207012, $1)").bind(id).execute(&mut *tx).await?;
    let current = sqlx::query("SELECT answers, state, version FROM cadastre.charting_report WHERE wf_job_instance_id = $1").bind(id).fetch_optional(&mut *tx).await?;
    let version = match current {
        None => {
            let answers = merge_answers("draft", &json!({}), &req.answers);
            sqlx::query("INSERT INTO cadastre.charting_report (wf_job_instance_id, answers, updated_by) VALUES ($1, $2, $3)")
                .bind(id).bind(answers).bind(&by).execute(&mut *tx).await?;
            1
        }
        Some(r) => {
            let (st, version): (String, i32) = (r.get("state"), r.get("version"));
            if st == "final" {
                return Err(AppError::Conflict("This report is signed and final.".into()));
            }
            if req.version.is_some_and(|v| v != version) {
                return Err(AppError::Conflict("Someone else has saved this report. Open it again to see their changes.".into()));
            }
            let answers = merge_answers(&st, &r.get::<Value, _>("answers"), &req.answers);
            sqlx::query("UPDATE cadastre.charting_report SET answers = $2, version = version + 1, updated_by = $3, updated_at = now() WHERE wf_job_instance_id = $1")
                .bind(id).bind(answers).bind(&by).execute(&mut *tx).await?;
            version + 1
        }
    };
    keep_version(&mut tx, id, &by).await?;
    audit::append(&mut tx, &by, "charting_report_saved", "wf_job_instance", Some(&id.to_string()), json!({ "version": version })).await?;
    tx.commit().await?;
    Ok(Json(json!({ "version": version })))
}

#[derive(Debug, Deserialize)]
pub struct SignBody {
    /// `charting` or `final`.
    pub part: String,
}

/// Sign a part as the signed-in officer. The final part needs a second officer.
pub async fn sign(State(state): State<AppState>, user: AuthUser, Path(id): Path<i32>, Json(req): Json<SignBody>) -> ApiResult<Json<Value>> {
    user.require_role("editor")?;
    let by = actor(&user);
    let mut tx = state.pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(4207012, $1)").bind(id).execute(&mut *tx).await?;
    let r = sqlx::query("SELECT state, charted_by, answers FROM cadastre.charting_report WHERE wf_job_instance_id = $1").bind(id).fetch_optional(&mut *tx).await?
        .ok_or_else(|| AppError::Conflict("Save the report before signing it.".into()))?;
    let (st, charted_by, answers): (String, Option<String>, Value) = (r.get("state"), r.get("charted_by"), r.get("answers"));
    if req.part == "final" && st == "charted" && charted_by.as_deref().is_some_and(|c| c.eq_ignore_ascii_case(&by)) {
        return Err(AppError::Conflict("The Final Survey Report must be signed by a second officer.".into()));
    }
    if matches!((req.part.as_str(), st.as_str()), ("charting", "draft") | ("final", "charted")) {
        let gaps = missing(&req.part, &answers);
        if !gaps.is_empty() {
            return Err(AppError::Conflict(format!("Complete these before signing: {}.", gaps.join(", "))));
        }
    }
    let next = match (req.part.as_str(), st.as_str()) {
        ("charting", "draft") => {
            sqlx::query("UPDATE cadastre.charting_report SET state = 'charted', charted_by = $2, charted_at = now(), version = version + 1, updated_by = $2, updated_at = now() WHERE wf_job_instance_id = $1")
                .bind(id).bind(&by).execute(&mut *tx).await?;
            "charted"
        }
        ("final", "charted") => {
            sqlx::query("UPDATE cadastre.charting_report SET state = 'final', final_by = $2, final_at = now(), version = version + 1, updated_by = $2, updated_at = now() WHERE wf_job_instance_id = $1")
                .bind(id).bind(&by).execute(&mut *tx).await?;
            "final"
        }
        ("charting" | "final", _) => return Err(AppError::Conflict(format!("This part cannot be signed while the report is {st}."))),
        _ => return Err(AppError::BadRequest("sign `charting` or `final`".into())),
    };
    keep_version(&mut tx, id, &by).await?;
    // The officer's name and rank are remembered for their next report.
    let part = &answers[&req.part];
    sqlx::query("UPDATE cadastre.principal SET name = $2, job_title = $3 WHERE is_user AND NOT is_system AND lower(code) = lower($1)")
        .bind(&by).bind(part["officer_name"].as_str().map(str::trim)).bind(part["rank"].as_str().map(str::trim)).execute(&mut *tx).await?;
    audit::append(&mut tx, &by, "charting_report_signed", "wf_job_instance", Some(&id.to_string()), json!({ "part": req.part })).await?;
    tx.commit().await?;
    Ok(Json(json!({ "state": next })))
}

/// A georeferenced scan in a Catalog folder.
#[derive(Debug, Clone, Serialize)]
pub struct Sheet {
    pub folder: usize,
    pub path: String,
    pub name: String,
    #[serde(skip)]
    pub bbox: [f64; 4],
}

/// Whether a sheet's extent touches the plot's.
pub fn overlaps(a: [f64; 4], b: [f64; 4]) -> bool {
    a[0] <= b[2] && b[0] <= a[2] && a[1] <= b[3] && b[1] <= a[3]
}

/// Every georeferenced TIFF in UTM 32N (or stating no system) under a folder. Anything unreadable
/// is reported, and a scan without georeferencing is left out: it cannot be placed on the map.
pub fn index_folder(folder: usize, root: &FsPath, out: &mut Vec<Sheet>, problems: &mut Vec<String>) {
    let mut stack: Vec<(PathBuf, usize)> = vec![(root.to_path_buf(), 0)];
    let mut seen = 0;
    while let Some((dir, depth)) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) => { problems.push(format!("{}: {e}", dir.display())); continue; }
        };
        for entry in entries.flatten() {
            seen += 1;
            if seen > MAX_ENTRIES {
                problems.push(format!("stopped after {MAX_ENTRIES} files"));
                return;
            }
            let path = entry.path();
            if path.is_dir() {
                if depth < MAX_DEPTH { stack.push((path, depth + 1)); }
                continue;
            }
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
            if ext != "tif" && ext != "tiff" { continue; }
            let Ok(info) = crate::raster::read_info(&path) else { continue };
            let Ok(rel) = path.strip_prefix(root) else { continue };
            if info.bbox == [0.0; 4] || info.epsg.is_some_and(|e| e != 32632) { continue; }
            out.push(Sheet {
                folder,
                path: rel.to_string_lossy().replace('\\', "/"),
                name: path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                bbox: info.bbox,
            });
        }
    }
}

static INDEX: Mutex<Option<(Instant, Vec<Sheet>, Vec<String>)>> = Mutex::new(None);

#[derive(Debug, Deserialize)]
pub struct SheetsQuery {
    pub upin: String,
}

/// The scanned sheets under a parcel.
pub async fn sheets(State(state): State<AppState>, Query(q): Query<SheetsQuery>) -> ApiResult<Json<Value>> {
    let extent = sqlx::query("SELECT ST_XMin(e) AS x0, ST_YMin(e) AS y0, ST_XMax(e) AS x1, ST_YMax(e) AS y1 FROM \
        (SELECT ST_Extent(geom)::box3d AS e FROM cadastre.parcel WHERE parcel_number = $1 AND status = 'posted') s")
        .bind(q.upin.trim()).fetch_one(&state.pool).await?;
    let Some(x0) = extent.get::<Option<f64>, _>("x0") else {
        return Err(AppError::NotFound(format!("parcel {}", q.upin.trim())));
    };
    let plot = [x0, extent.get("y0"), extent.get("x1"), extent.get("y1")];
    let cached = INDEX.lock().ok().and_then(|g| g.as_ref().filter(|(at, _, _)| at.elapsed() < INDEX_TTL).map(|(_, s, p)| (s.clone(), p.clone())));
    let (all, problems) = match cached {
        Some(c) => c,
        None => {
            let folders: Vec<PathBuf> = state.config.catalog_folders.iter().map(|f| f.path.clone()).collect();
            let built = tokio::task::spawn_blocking(move || {
                let (mut s, mut p) = (Vec::new(), Vec::new());
                for (i, f) in folders.iter().enumerate() { index_folder(i, f, &mut s, &mut p); }
                (s, p)
            }).await.map_err(|e| AppError::Internal(format!("the sheets could not be read: {e}")))?;
            if let Ok(mut g) = INDEX.lock() { *g = Some((Instant::now(), built.0.clone(), built.1.clone())); }
            built
        }
    };
    let found: Vec<&Sheet> = all.iter().filter(|s| overlaps(s.bbox, plot)).collect();
    Ok(Json(json!({ "sheets": found, "indexed": all.len(), "problems": problems.iter().take(5).collect::<Vec<_>>() })))
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/charting/files", get(files))
        .route("/api/v1/charting/files/{id}", get(file))
        .route("/api/v1/charting/files/{id}/report", put(save))
        .route("/api/v1/charting/files/{id}/sign", post(sign))
        .route("/api/v1/charting/sheets", get(sheets))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blank_part_cannot_be_signed_and_follow_ups_count_only_when_called_for() {
        assert!(missing("charting", &json!({})).len() >= 15);
        assert!(missing("final", &json!({})).contains(&"Recommended (5)"));
        let done = json!({ "charting": { "officer_name": "A B", "rank": "C.T.O. Carto", "ref_no": "PL1", "property_no": "PL1", "applicant": "C", "area": "1", "location": "D",
            "identified": "Yes", "ground_open": "Yes", "beaconed": "No", "town_plan": "No", "under_title": "No", "rail": "No", "major_road": "No", "settlement": "No" } });
        assert!(missing("charting", &done).is_empty(), "{:?}", missing("charting", &done));
        let mut q3 = done.clone();
        q3["charting"]["beaconed"] = json!("Yes");
        assert_eq!(missing("charting", &q3), ["Tracing No (question 3)"]);
        let mut spaces = done.clone();
        spaces["charting"]["rank"] = json!("   ");
        assert_eq!(missing("charting", &spaces), ["Rank"]);
    }

    #[test]
    fn a_signed_part_does_not_change() {
        let stored = json!({ "charting": { "q": "signed" }, "final": { "f": "old" } });
        let incoming = json!({ "charting": { "q": "tampered" }, "final": { "f": "new" } });
        assert_eq!(merge_answers("charted", &stored, &incoming), json!({ "charting": { "q": "signed" }, "final": { "f": "new" } }));
        assert_eq!(merge_answers("draft", &stored, &incoming), json!({ "charting": { "q": "tampered" }, "final": { "f": "old" } }));
    }

    fn write_tiff(path: &FsPath) {
        let file = std::fs::File::create(path).unwrap();
        let mut enc = tiff::encoder::TiffEncoder::new(file).unwrap();
        enc.write_image::<tiff::encoder::colortype::Gray8>(4, 2, &[0u8; 8]).unwrap();
    }

    #[test]
    fn sheets_with_a_world_file_are_indexed_and_others_left_out() {
        let dir = std::env::temp_dir().join(format!("charting-sheets-{}", std::process::id()));
        let sub = dir.join("Sheets");
        std::fs::create_dir_all(&sub).unwrap();
        write_tiff(&sub.join("487_1089_NW_172.TIF"));
        std::fs::write(sub.join("487_1089_NW_172.tfw"), "2.0\n0.0\n0.0\n-2.0\n488001.0\n1097999.0\n").unwrap();
        write_tiff(&sub.join("no_world_file.tif"));
        write_tiff(&sub.join("rotated.tif"));
        std::fs::write(sub.join("rotated.tfw"), "2.0\n0.5\n0.5\n-2.0\n488001.0\n1097999.0\n").unwrap();
        let (mut found, mut problems) = (Vec::new(), Vec::new());
        index_folder(3, &dir, &mut found, &mut problems);
        std::fs::remove_dir_all(&dir).ok();
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(found.len(), 1, "{found:?}");
        let s = &found[0];
        assert_eq!((s.folder, s.path.as_str(), s.name.as_str()), (3, "Sheets/487_1089_NW_172.TIF", "487_1089_NW_172.TIF"));
        // 4x2 pixels of 2 m, the top-left pixel centred on 488001, 1097999.
        assert_eq!(s.bbox, [488000.0, 1097996.0, 488008.0, 1098000.0]);
    }

    #[test]
    fn sheets_are_found_by_extent() {
        let sheet = [488000.0, 1097000.0, 489000.0, 1098000.0];
        assert!(overlaps(sheet, [488134.0, 1097202.0, 488150.0, 1097220.0]), "inside");
        assert!(overlaps(sheet, [488990.0, 1097500.0, 489010.0, 1097510.0]), "across the edge");
        assert!(!overlaps(sheet, [490000.0, 1097500.0, 490010.0, 1097510.0]), "elsewhere");
    }
}
