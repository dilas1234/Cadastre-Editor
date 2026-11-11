//! The title deed plan of a job (Generate Title Deed Plan): its data, the officer's form, and the
//! printable sheet (`crate::titledeed::render_svg`).
//!
//! * `GET  /api/v1/wf/jobs/{id}/titledeed[?upin=]`     the plan: saved form over the prefill
//! * `PUT  /api/v1/wf/jobs/{id}/titledeed`             save the form (`{ upin, fields }`)
//! * `GET  /api/v1/wf/jobs/{id}/titledeed.svg[?upin=]` the sheet (A4, SVG)
//! * `POST /api/v1/wf/jobs/{id}/titledeed/preview`     the sheet of a form not yet saved (the editor)
//!
//! The parcel is the one the job committed, else the file's active parcel (the existing-parcel
//! path: Search Cadastre for Parcels → Generate Title Deed Plan). Prefill, each field only where
//! the saved form leaves it blank:
//! * the Registry (read-only), with Aumentum's own title deed queries `R_PROPINFO` / `R_HOLDERS`
//!   (MCServer `LRSIntegrationQueryDefinition`), keyed on the property's SuID so they work before
//!   the property is linked: location, tracing no., plan no., L.G. area, surveyed by, survey date,
//!   street, holders;
//! * the parcel's attribute record: plan no. / file no., L.G. area (code), Surveyor-General;
//! * the signed-in officer as PLOTTED BY.

use axum::extract::{Path, Query, State};
use axum::http::header;
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use utoipa::{IntoParams, ToSchema};

use crate::auth::AuthUser;
use crate::error::{ApiResult, AppError};
use crate::lrs::{LrsConfig, Session};
use crate::state::AppState;
use crate::titledeed::{cap_words, computed_labels, overrides, plan_order, render_svg, Override, PlanData, PlanFields, SYMBOLS};

#[derive(Debug, Deserialize, IntoParams)]
pub struct PlanQuery {
    /// The parcel, when the job has more than one.
    pub upin: Option<String>,
}

/// An entry of the plan's symbol list, for the form.
#[derive(Debug, Serialize, ToSchema)]
pub struct SymbolDto {
    pub id: String,
    pub label: String,
    pub group: String,
    /// Placed at one spot; the others are lines.
    pub point: bool,
    /// Drawn as two lines; this far apart on paper (mm) unless the feature says otherwise.
    pub double: f64,
    /// Its colour, unless the feature says otherwise.
    pub stroke: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TitleDeedDto {
    /// The plan as it prints (saved form over the prefill).
    pub plan: PlanData,
    /// The prefill alone (what the Registry and the parcel record give).
    pub prefill: PlanFields,
    /// The officer's saved form, if any.
    pub saved: Option<PlanFields>,
    /// Other committed parcels the job could print.
    pub upins: Vec<String>,
    /// `ok`, `off` (not configured) or why the Registry could not be read.
    pub registry: String,
    /// The symbols a feature on the plan can use, in the form's order.
    pub symbols: Vec<SymbolDto>,
    /// Labels the plan prints typed over what the sheet computes; `stale` = the co-ordinates have
    /// changed since they were typed.
    pub overrides: Vec<Override>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SaveTitleDeed {
    pub upin: String,
    pub fields: PlanFields,
}

/// Plateau's L.G. areas in Aumentum's code order (1 = Barkin Ladi … 6 = Jos South …). Used only
/// when the Registry cannot be asked; the officer confirms it on the form.
const LGAS: [&str; 17] = ["Barkin Ladi", "Bassa", "Bokkos", "Jos East", "Jos North", "Jos South", "Kanam", "Kanke",
    "Langtang North", "Langtang South", "Mangu", "Mikang", "Pankshin", "Qua'an Pan", "Riyom", "Shendam", "Wase"];

fn non_empty(s: Option<String>) -> Option<String> {
    s.map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

/// A value safe to put inside an HQL string literal (UPINs, SuIDs, transaction numbers).
fn hql_safe(s: &str) -> Option<&str> {
    Some(s).filter(|v| !v.is_empty() && v.len() <= 40 && v.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '/'))
}

/// The job's committed parcels, else the file's active parcel(s); and the job's file no. / transaction.
async fn job_parcels(state: &AppState, id: i32) -> ApiResult<(Vec<String>, Option<String>, Option<String>)> {
    let job = sqlx::query(
        "SELECT w.name, w.lrs_transaction_number, w.cadastre_job_id, l.property_number \
         FROM cadastre.wf_job_instance w LEFT JOIN cadastre.wf_job_linkage l ON l.wf_job_instance_id = w.id WHERE w.id = $1",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("job instance {id}")))?;
    let txn: Option<String> = job.get("lrs_transaction_number");
    let file_no = non_empty(job.get("property_number"))
        .or_else(|| job.get::<Option<String>, _>("name").and_then(|n| n.strip_prefix("Job-").map(str::to_string)));
    let mut upins: Vec<String> = sqlx::query_scalar(
        "SELECT p.parcel_number FROM cadastre.parcel p WHERE p.job_id = $1 AND p.status = 'posted' AND p.parcel_number IS NOT NULL ORDER BY p.posted_at, p.parcel_number",
    )
    .bind(job.get::<Option<uuid::Uuid>, _>("cadastre_job_id"))
    .fetch_all(&state.pool)
    .await?;
    if upins.is_empty() {
        if let Some(f) = &file_no {
            upins = sqlx::query_scalar(
                "SELECT DISTINCT a.upin FROM cadastre.parcel_attribute a JOIN cadastre.parcel p ON p.parcel_number = a.upin AND p.status = 'posted' \
                 WHERE upper(replace(btrim(a.property_number), ' ', '')) = upper(replace($1, ' ', '')) AND COALESCE(a.status, '0') = '0' ORDER BY a.upin",
            )
            .bind(f)
            .fetch_all(&state.pool)
            .await?;
        }
    }
    Ok((upins, file_no, txn))
}

/// What the Registry holds for a file (read-only), and a note if it could not be read.
pub(crate) async fn registry_file_details(txn: Option<&str>, file_no: Option<&str>) -> (PlanFields, String) {
    let mut f = PlanFields::default();
    let note = registry_prefill(&mut f, None, txn, file_no).await;
    (f, note)
}

/// Registry prefill (read-only). Errors are reported, not fatal: the form can still be filled.
async fn registry_prefill(f: &mut PlanFields, su_id: Option<String>, txn: Option<&str>, file_no: Option<&str>) -> String {
    let Some(cfg) = LrsConfig::from_env() else { return "off".into() };
    let session = match Session::start(&cfg).await {
        Ok(s) => s,
        Err(e) => return format!("the Registry could not be reached: {e:?}"),
    };
    let result: Result<(), String> = async {
        // The property's SuID: the parcel's link, else the transaction's property for this file.
        let mut su = su_id;
        if su.is_none() {
            if let Some(t) = txn.and_then(hql_safe) {
                if let Ok(Some(tx)) = crate::routes::registry::load_transaction(&session, t).await {
                    let want = file_no.map(|s| s.replace(' ', "").to_uppercase());
                    su = tx.properties.iter()
                        .find(|p| want.is_some() && p.refnum.as_ref().map(|r| r.replace(' ', "").to_uppercase()) == want)
                        .or(tx.properties.first())
                        .map(|p| p.su_id.clone());
                }
            }
        }
        let Some(su) = su.as_deref().and_then(hql_safe).map(str::to_string) else {
            return Err("no Registry property for this file".into());
        };
        // Aumentum's R_PROPINFO, keyed on the SuID (it keys on the UPIN, which is set only once linked).
        let info = session.query(&format!(
            "select z.Label, s.TracingNumber, s.Refnum, d.Label, s.SurveyName, s.SurveyDate, s.SurveyPlanNo, r.Label \
             from com.landsystems.lrs.model.Transaction t join t.Properties p join p.SpatialUnits s join s.Address a \
             join a.LocalGovernmentArea d join s.Zone z join a.Region r where s.SuID = '{su}' and t.TransactionNumber not like 'P-%'"), 5)
            .await.map_err(|e| format!("{e:?}"))?;
        if let Some(r) = info.into_iter().find(|r| r.len() >= 8) {
            let v = |i: usize| non_empty(Some(r[i].clone()));
            f.location = f.location.take().or(v(0));
            f.tracing_no = f.tracing_no.take().or(v(1));
            f.plan_no = f.plan_no.take().or(v(6)).or(v(2));
            f.lga = v(3).or(f.lga.take());
            f.surveyed_by = f.surveyed_by.take().or(v(4));
            f.survey_date = f.survey_date.take().or_else(|| {
                // Registry dates are UTC; the plan shows the local (WAT) date.
                crate::lrs::parse_datetime(&r[5]).map(|d| (d + chrono::Duration::hours(1)).format("%-d-%-m-%Y").to_string()).or(v(5))
            });
            f.region = f.region.take().or(v(7));
        }
        // Aumentum's R_HOLDERS.
        let holders = session.query(&format!(
            "select distinct a.Name, s.Upin from com.landsystems.lrs.model.RRR r join r.Party a join r.LaUnit p join p.SpatialUnits s where s.SuID = '{su}'"), 20)
            .await.map_err(|e| format!("{e:?}"))?;
        let names: Vec<String> = holders.iter().filter_map(|r| non_empty(r.first().cloned())).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
        if !names.is_empty() {
            f.holder = f.holder.take().or(Some(names.join(" & ")));
        }
        let street = session.query(&format!(
            "select s.SuID, a.StreetName from com.landsystems.lrs.model.Transaction t join t.Properties l join l.SpatialUnits s join s.Address a where s.SuID = '{su}' and t.TransactionNumber not like 'P-%'"), 5)
            .await.map_err(|e| format!("{e:?}"))?;
        if let Some(st) = street.iter().find_map(|r| non_empty(r.get(1).cloned())) {
            f.address = f.address.take().or(Some(st));
        }
        Ok(())
    }
    .await;
    session.end().await;
    match result { Ok(()) => "ok".into(), Err(e) => e }
}

/// Merge: each field of `saved` that is set wins over `prefill`.
fn merge(prefill: &PlanFields, saved: &PlanFields) -> PlanFields {
    macro_rules! pick { ($($f:ident),*) => { PlanFields { $($f: non_empty(saved.$f.clone()).or_else(|| prefill.$f.clone()),)*
        scale: saved.scale.or(prefill.scale),
        corner_names: if saved.corner_names.iter().any(|c| c.as_deref().is_some_and(|s| !s.trim().is_empty())) { saved.corner_names.clone() } else { prefill.corner_names.clone() },
        road_side: saved.road_side.or(prefill.road_side),
        road_sides: if saved.road_sides.is_empty() { prefill.road_sides.clone() } else { saved.road_sides.clone() },
        grid: saved.grid.or(prefill.grid),
        grid_corner: saved.grid_corner.or(prefill.grid_corner),
        features: saved.features.clone(),
        label_moves: saved.label_moves.clone(),
        notes: saved.notes.clone(),
        styles: saved.styles.clone(),
        road_label: non_empty(saved.road_label.clone()).or_else(|| prefill.road_label.clone()) } } }
    pick!(region, location, plan_no, tracing_no, title_no, address, lga, holder, surveyed_by, survey_date, plotted_by, checked_by, surveyor_general, prepared_by)
}

/// Overrides as they are kept: the words typed over a computed label, with what the sheet computed
/// when they were typed (`was`, set here, never taken from the browser). Words equal to the computed
/// ones are no override; the UPIN is never typed over; the middle's old one-block key is split.
/// Returns what changed since the saved form, for the audit log.
fn settle_overrides(fields: &mut PlanFields, old: &PlanFields, calc: &std::collections::BTreeMap<String, String>) -> Vec<serde_json::Value> {
    fields.split_parcel_block();
    for (key, st) in fields.styles.iter_mut() {
        let words = st.text.as_deref().map(str::trim).filter(|t| !t.is_empty()).map(str::to_string);
        let Some(words) = words.filter(|_| key != "upin") else {
            st.text = None;
            st.was = None;
            continue;
        };
        match calc.get(key) {
            // An access road's label, or a label not on the sheet now (the grid turned off).
            None => {
                if key.starts_with("road:") { st.was = None; }
                st.text = Some(words);
            }
            Some(c) if *c == words => { st.text = None; st.was = None; }
            Some(c) => {
                let kept = old.styles.get(key).filter(|o| o.text.as_deref().map(str::trim) == Some(words.as_str())).and_then(|o| o.was.clone());
                st.was = Some(kept.unwrap_or_else(|| c.clone()));
                st.text = Some(words);
            }
        }
    }
    fields.styles.retain(|_, st| st.color.is_some() || st.text.is_some() || st.angle.is_some());
    let typed = |f: &PlanFields| -> std::collections::BTreeMap<String, (String, Option<String>)> {
        f.styles.iter().filter(|(k, st)| !k.starts_with("road:") && st.text.is_some()).map(|(k, st)| (k.clone(), (st.text.clone().unwrap_or_default(), st.was.clone()))).collect()
    };
    let (before, after) = (typed(old), typed(fields));
    let mut changes = Vec::new();
    for (key, (words, was)) in &after {
        match before.get(key) {
            Some((w, _)) if w == words => {}
            Some((w, _)) => changes.push(serde_json::json!({ "key": key, "change": "changed", "calculated": was, "from": w, "typed": words })),
            None => changes.push(serde_json::json!({ "key": key, "change": "added", "calculated": was, "typed": words })),
        }
    }
    for (key, (words, was)) in &before {
        if !after.contains_key(key) {
            changes.push(serde_json::json!({ "key": key, "change": "removed", "calculated": calc.get(key).or(was.as_ref()), "typed": words }));
        }
    }
    changes
}

/// The plan. `registry` = ask the Registry for the prefill too, which takes a session of its own;
/// the sheet's editor, redrawing as the officer works, does without.
async fn load(state: &AppState, id: i32, want: Option<String>, user: Option<&AuthUser>, registry: bool) -> ApiResult<TitleDeedDto> {
    let (upins, file_no, txn) = job_parcels(state, id).await?;
    let upin = match want.map(|u| u.trim().to_uppercase()).filter(|u| !u.is_empty()) {
        Some(u) => u,
        None => upins.first().cloned().ok_or_else(|| AppError::Conflict(
            "this job has no committed parcel yet: commit the parcel (Commit Parcel), or open the job of a file whose parcel exists".into()))?,
    };
    let area: f64 = sqlx::query_scalar("SELECT ST_Area(geom) FROM cadastre.parcel WHERE parcel_number = $1 AND status = 'posted' LIMIT 1")
        .bind(&upin)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("no posted parcel {upin}")))?;
    let legs = crate::report::parcel_legs(&state.pool, &upin).await?;
    let mut ring: Vec<(f64, f64, String)> = legs.iter().map(|l| (l.from_e, l.from_n, l.from.clone())).collect();
    if let Some(l) = legs.last() {
        ring.push((l.to_e, l.to_n, l.to.clone()));
    }
    let corners = plan_order(&ring);

    // Prefill from the parcel record, then the Registry.
    let attr = sqlx::query("SELECT property_number, plan_number, lga, surveyor_general, lrs_id, region_name FROM cadastre.parcel_attribute WHERE upin = $1 ORDER BY (status = '0') DESC NULLS LAST, object_id DESC LIMIT 1")
        .bind(&upin)
        .fetch_optional(&state.pool)
        .await?;
    let mut prefill = PlanFields::default();
    let mut su_id = None;
    if let Some(a) = &attr {
        let file = non_empty(a.get("property_number")).or_else(|| file_no.clone());
        prefill.plan_no = non_empty(a.get("plan_number")).or_else(|| file.clone());
        prefill.title_no = file;
        prefill.surveyor_general = non_empty(a.get("surveyor_general"));
        prefill.lga = non_empty(a.get("lga")).and_then(|c| c.parse::<usize>().ok()).and_then(|i| LGAS.get(i.wrapping_sub(1)).map(|s| s.to_string()));
        su_id = non_empty(a.get("lrs_id"));
    } else {
        prefill.title_no = file_no.clone();
        prefill.plan_no = file_no.clone();
    }
    let file_for_registry = prefill.title_no.clone();
    let registry = if registry { registry_prefill(&mut prefill, su_id, txn.as_deref(), file_for_registry.as_deref()).await } else { "off".to_string() };
    // The region: the Registry's, else the parcel record's dataset region (Plateau_State →
    // Plateau), else Plateau.
    if prefill.region.is_none() {
        let ds: Option<String> = attr.as_ref().and_then(|a| a.try_get::<Option<String>, _>("region_name").ok().flatten());
        prefill.region = Some(ds.as_deref().and_then(|r| r.strip_suffix("_State")).map(|r| r.replace('_', " ")).unwrap_or_else(|| "Plateau".into()));
    }
    prefill.surveyed_by = prefill.surveyed_by.as_deref().map(cap_words);
    if let Some(u) = user {
        if let Ok(actor) = crate::routes::wf::resolve_actor(state, u).await {
            prefill.plotted_by = Some(actor.label);
        }
    }
    prefill.corner_names = corners.iter().map(|c| Some(c.name.clone()).filter(|n| !(n.starts_with('P') && n[1..].chars().all(|d| d.is_ascii_digit())))).collect();

    let saved: Option<PlanFields> = sqlx::query_scalar::<_, serde_json::Value>("SELECT fields FROM cadastre.title_deed_plan WHERE wf_job_instance_id = $1 AND upin = $2")
        .bind(id)
        .bind(&upin)
        .fetch_optional(&state.pool)
        .await?
        .and_then(|v| serde_json::from_value(v).ok());
    let mut fields = match &saved { Some(s) => merge(&prefill, s), None => prefill.clone() };
    fields.surveyed_by = fields.surveyed_by.as_deref().map(cap_words);
    let symbols = SYMBOLS.iter().map(|y| SymbolDto { id: y.id.into(), label: y.label.into(), group: y.group.into(), point: y.point, double: y.double, stroke: y.stroke().into() }).collect();
    let plan = PlanData { upin, area_m2: area, corners, fields };
    let overrides = overrides(&plan);
    Ok(TitleDeedDto { plan, prefill, saved, upins, registry, symbols, overrides })
}

/// The job's title deed plan: data, prefill and saved form.
#[utoipa::path(get, path = "/api/v1/wf/jobs/{id}/titledeed", tag = "wf", params(("id" = i32, Path, description = "Job instance id"), PlanQuery),
    responses((status = 200, body = TitleDeedDto), (status = 404, description = "Job or parcel not found"), (status = 409, description = "No committed parcel yet")))]
pub async fn get_plan(State(state): State<AppState>, user: AuthUser, Path(id): Path<i32>, Query(q): Query<PlanQuery>) -> ApiResult<Json<TitleDeedDto>> {
    Ok(Json(load(&state, id, q.upin, Some(&user), true).await?))
}

/// Save the plan form (so a reprint is identical).
#[utoipa::path(put, path = "/api/v1/wf/jobs/{id}/titledeed", tag = "wf", params(("id" = i32, Path, description = "Job instance id")),
    request_body = SaveTitleDeed, responses((status = 200, body = TitleDeedDto)))]
pub async fn put_plan(State(state): State<AppState>, user: AuthUser, Path(id): Path<i32>, Json(req): Json<SaveTitleDeed>) -> ApiResult<Json<TitleDeedDto>> {
    let actor = crate::routes::wf::resolve_actor(&state, &user).await?;
    let upin = req.upin.trim().to_uppercase();
    let (upins, _, _) = job_parcels(&state, id).await?;
    if !upins.contains(&upin) {
        return Err(AppError::Conflict(format!("{upin} is not a committed parcel of this job")));
    }
    let mut fields = req.fields;
    fields.surveyed_by = non_empty(fields.surveyed_by).map(|s| cap_words(&s));
    let before = load(&state, id, Some(upin.clone()), None, false).await?;
    let old = before.saved.clone().unwrap_or_default();
    let calc = computed_labels(&PlanData { fields: merge(&before.prefill, &fields), ..before.plan.clone() });
    let changes = settle_overrides(&mut fields, &old, &calc);
    let mut tx = state.pool.begin().await?;
    sqlx::query(
        "INSERT INTO cadastre.title_deed_plan (wf_job_instance_id, upin, fields, updated_by) VALUES ($1, $2, $3, $4) \
         ON CONFLICT (wf_job_instance_id, upin) DO UPDATE SET fields = EXCLUDED.fields, updated_by = EXCLUDED.updated_by, updated_at = now()",
    )
    .bind(id)
    .bind(&upin)
    .bind(serde_json::to_value(&fields).map_err(|e| AppError::BadRequest(e.to_string()))?)
    .bind(&actor.label)
    .execute(&mut *tx)
    .await?;
    if !changes.is_empty() {
        crate::audit::append(&mut tx, &actor.label, "title_deed_plan_override", "wf_job_instance", Some(&id.to_string()),
            serde_json::json!({ "upin": upin, "changes": changes })).await?;
    }
    tx.commit().await?;
    Ok(Json(load(&state, id, Some(upin), Some(&user), true).await?))
}

/// The printable sheet (A4 SVG).
#[utoipa::path(get, path = "/api/v1/wf/jobs/{id}/titledeed.svg", tag = "wf", params(("id" = i32, Path, description = "Job instance id"), PlanQuery),
    responses((status = 200, description = "Title deed plan", content_type = "image/svg+xml")))]
pub async fn svg(State(state): State<AppState>, Path(id): Path<i32>, Query(q): Query<PlanQuery>) -> ApiResult<impl IntoResponse> {
    let d = load(&state, id, q.upin, None, true).await?;
    Ok(([(header::CONTENT_TYPE, "image/svg+xml; charset=utf-8"), (header::CACHE_CONTROL, "no-store")], render_svg(&d.plan)))
}

/// The sheet with the given form, not saved: the sheet's editor redrawing as the officer works. The
/// form is printed as sent, so it comes with the prefill already filled in.
#[utoipa::path(post, path = "/api/v1/wf/jobs/{id}/titledeed/preview", tag = "wf", params(("id" = i32, Path, description = "Job instance id")),
    request_body = SaveTitleDeed, responses((status = 200, description = "Title deed plan", content_type = "image/svg+xml"), (status = 409, description = "Not a committed parcel of this job")))]
pub async fn preview(State(state): State<AppState>, _user: AuthUser, Path(id): Path<i32>, Json(req): Json<SaveTitleDeed>) -> ApiResult<impl IntoResponse> {
    let upin = req.upin.trim().to_uppercase();
    let mut d = load(&state, id, Some(upin.clone()), None, false).await?;
    if !d.upins.contains(&upin) {
        return Err(AppError::Conflict(format!("{upin} is not a committed parcel of this job")));
    }
    d.plan.fields = req.fields;
    Ok(([(header::CONTENT_TYPE, "image/svg+xml; charset=utf-8"), (header::CACHE_CONTROL, "no-store")], render_svg(&d.plan)))
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct AttachPdfQuery {
    /// The PDF's file name, e.g. `Title Deed Plan PL67363.pdf`.
    pub name: String,
}

/// Attach as PDF: the title deed sheet as the editor drew it (a JPEG of the A4 sheet, the request
/// body) becomes a one-page A4 PDF filed on the job. Send plan sends that PDF to the Registry.
#[utoipa::path(post, path = "/api/v1/wf/jobs/{id}/titledeed.pdf", tag = "wf", params(("id" = i32, Path, description = "Job instance id"), AttachPdfQuery),
    request_body(content = Vec<u8>, content_type = "image/jpeg", description = "The sheet as a JPEG"),
    responses((status = 201, body = crate::routes::wfadmin::WfFileDto), (status = 400, description = "Not a JPEG")))]
pub async fn attach_pdf(State(state): State<AppState>, user: AuthUser, Path(id): Path<i32>, Query(q): Query<AttachPdfQuery>, body: axum::body::Bytes)
    -> ApiResult<(axum::http::StatusCode, Json<crate::routes::wfadmin::WfFileDto>)> {
    let pdf = crate::pdf::jpeg_a4_pdf(&body).map_err(|e| AppError::BadRequest(format!("the plan sheet: {e}")))?;
    let stem = q.name.trim().trim_end_matches(".pdf").trim_end_matches(".PDF").to_string();
    let name = format!("{}.pdf", if stem.is_empty() { format!("Title Deed Plan {id}") } else { stem });
    crate::routes::wfadmin::upload_file(State(state), user, Path(id), Query(crate::routes::wfadmin::UploadQuery { name }), axum::body::Bytes::from(pdf)).await
}
