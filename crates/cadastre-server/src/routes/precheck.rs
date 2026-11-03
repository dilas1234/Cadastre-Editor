//! System check before a sketched parcel moves on, and again at commit: nothing is committed onto
//! land that is taken. Read-only towards Aumentum and the Registry.
//!
//! | check            | looks at                                                                   | result |
//! |------------------|----------------------------------------------------------------------------|--------|
//! | `fabric_overlap` | parcels committed in this Cadastre                                         | fail   |
//! | `aumentum`       | Aumentum's live parcel layer (`gis.gate/0/0/query`, through the bridge)    | fail   |
//! | `drop_point`     | the plot's position (Easting/Northing): already digitised here or in Aumentum? | fail |
//! | `title`          | a title on the land: a parcel there linked to a Registry property, or the job's Registry property already holding a UPIN | fail |
//! | `transaction`    | the job's Registry transaction is still Active                             | fail   |
//! | `near_miss`      | gaps under 10 cm against committed neighbours                              | warn   |
//!
//! Parcels this job's mutations replace (split, merge, re-survey …) are not counted against it.
//! A source that cannot be reached is a warning, not a pass: the officer must say why to go on.

use axum::extract::{Path, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::Row;
use utoipa::ToSchema;

use crate::audit;
use crate::auth::AuthUser;
use crate::error::{ApiResult, AppError};
use crate::routes::gisviewer_passthrough::{aumentum_get, pct_encode};
use crate::state::AppState;

/// Overlaps smaller than this (m²) are drawing noise, as in the commit's rule.
const MIN_OVERLAP_M2: f64 = 0.01;
/// Gaps narrower than this (m) to a committed neighbour are reported as near misses.
const NEAR_MISS_M: f64 = 0.10;

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PrecheckItem {
    /// `fabric_overlap`, `aumentum`, `drop_point`, `title`, `transaction`, `near_miss`.
    pub check: String,
    pub label: String,
    /// `pass`, `warn`, `fail` or `skipped`.
    pub result: String,
    pub detail: String,
    /// The parcels concerned.
    pub upins: Vec<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PrecheckReport {
    /// No check failed.
    pub ok: bool,
    /// At least one warning (going on needs a reason).
    pub warnings: bool,
    pub items: Vec<PrecheckItem>,
    pub checked_at: String,
}

#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct PrecheckRequest {
    /// The sketch to check (UTM 32N metres, ring order). Empty: the job's draft parcels.
    #[serde(default)]
    pub vertices: Vec<[f64; 2]>,
    /// The id of the draft being reshaped, which is not checked against itself.
    #[serde(default)]
    pub editing_id: Option<uuid::Uuid>,
    /// The officer's reason for going on despite warnings (recorded in the audit log).
    #[serde(default)]
    pub acknowledge: Option<String>,
}

fn item(check: &str, label: &str, result: &str, detail: impl Into<String>, upins: Vec<String>) -> PrecheckItem {
    PrecheckItem { check: check.into(), label: label.into(), result: result.into(), detail: detail.into(), upins }
}

fn list(u: &[String]) -> String {
    if u.len() > 6 { format!("{} and {} more", u[..6].join(", "), u.len() - 6) } else { u.join(", ") }
}

/// Run every check for job instance `id`, on `vertices` (a sketch) or on the job's draft parcels.
pub async fn run(state: &AppState, id: i32, vertices: &[[f64; 2]], editing_id: Option<uuid::Uuid>) -> ApiResult<PrecheckReport> {
    let mut conn = state.pool.acquire().await?;
    let job: Option<(Option<uuid::Uuid>, Option<String>)> =
        sqlx::query_as("SELECT cadastre_job_id, lrs_transaction_number FROM cadastre.wf_job_instance WHERE id = $1")
            .bind(id).fetch_optional(&mut *conn).await?;
    let (cadastre_job, txn) = job.ok_or_else(|| AppError::NotFound(format!("job instance {id}")))?;
    let property_number: Option<String> = sqlx::query_scalar("SELECT property_number FROM cadastre.wf_job_linkage WHERE wf_job_instance_id = $1")
        .bind(id).fetch_optional(&mut *conn).await?.flatten();
    let mutated = crate::routes::mutations::pending_parents(&mut conn, id).await?;
    // Parcels this same job committed earlier (a job with several parcels): not a title clash.
    let own_posted: Vec<String> = match cadastre_job {
        Some(j) => sqlx::query_scalar("SELECT parcel_number FROM cadastre.parcel WHERE job_id = $1 AND status = 'posted' AND parcel_number IS NOT NULL")
            .bind(j).fetch_all(&mut *conn).await?,
        None => Vec::new(),
    };

    // The shape to check, as GeoJSON in EPSG:32632.
    let shape: Option<String> = if vertices.len() >= 3 {
        if vertices.iter().flatten().any(|v| !v.is_finite()) {
            return Err(AppError::BadRequest("vertices must be numbers (UTM 32N metres)".into()));
        }
        let mut ring: Vec<[f64; 2]> = vertices.to_vec();
        ring.push(vertices[0]);
        Some(json!({ "type": "Polygon", "coordinates": [ring] }).to_string())
    } else if let Some(j) = cadastre_job {
        sqlx::query_scalar("SELECT ST_AsGeoJSON(ST_Union(geom)) FROM cadastre.parcel WHERE job_id = $1 AND status = 'draft' AND ($2::uuid IS NULL OR id <> $2)")
            .bind(j).bind(editing_id).fetch_one(&mut *conn).await?
    } else {
        None
    };
    let Some(shape) = shape else {
        return Ok(PrecheckReport { ok: true, warnings: false, checked_at: chrono::Utc::now().to_rfc3339(),
            items: vec![item("fabric_overlap", "Overlap with this Cadastre", "skipped", "Nothing to check: no sketch and no draft parcels.", vec![])] });
    };
    let g = "ST_MakeValid(ST_SetSRID(ST_GeomFromGeoJSON($1), 32632))";
    let mut items = Vec::new();

    // 1. Overlap with parcels committed here.
    let ours: Vec<(String, f64)> = sqlx::query_as(&format!(
        "SELECT p.parcel_number, ST_Area(ST_Intersection(p.geom, {g})) AS a FROM cadastre.parcel p \
         WHERE p.status = 'posted' AND p.geom && {g} AND NOT (p.parcel_number = ANY($2)) \
           AND ST_Area(ST_Intersection(p.geom, {g})) > $3 ORDER BY a DESC LIMIT 50"))
        .bind(&shape).bind(&mutated).bind(MIN_OVERLAP_M2).fetch_all(&mut *conn).await?;
    let our_upins: Vec<String> = ours.iter().map(|o| o.0.clone()).collect();
    items.push(if ours.is_empty() {
        item("fabric_overlap", "Overlap with this Cadastre", "pass", "No committed parcel is overlapped.", vec![])
    } else {
        let d = ours.iter().take(6).map(|(u, a)| format!("{u} ({a:.2} m²)")).collect::<Vec<_>>().join(", ");
        item("fabric_overlap", "Overlap with this Cadastre", "fail", format!("The parcel overlaps {d}."), our_upins.clone())
    });

    // 2 + 3. Aumentum's live parcels: overlap, and whether the plot's position is already digitised there.
    let (pe, pn): (f64, f64) = {
        let r = sqlx::query(&format!("SELECT ST_X(p) AS e, ST_Y(p) AS n FROM (SELECT ST_PointOnSurface({g}) AS p) s")).bind(&shape).fetch_one(&mut *conn).await?;
        (r.get("e"), r.get("n"))
    };
    let at_point_here: Vec<String> = sqlx::query_scalar(
        "SELECT parcel_number FROM cadastre.parcel WHERE status = 'posted' AND ST_Contains(geom, ST_SetSRID(ST_MakePoint($1, $2), 32632)) AND NOT (parcel_number = ANY($3))")
        .bind(pe).bind(pn).bind(&mutated).fetch_all(&mut *conn).await?;
    let mut aum_upins: Vec<String> = Vec::new();
    let mut aum_at_point: Vec<String> = Vec::new();
    let aum_item = {
        let env: (f64, f64, f64, f64) = {
            let r = sqlx::query(&format!("SELECT ST_XMin(b) x0, ST_YMin(b) y0, ST_XMax(b) x1, ST_YMax(b) y1 FROM (SELECT Box2D({g}) b) s")).bind(&shape).fetch_one(&mut *conn).await?;
            (r.get("x0"), r.get("y0"), r.get("x1"), r.get("y1"))
        };
        let geometry = format!("{},{},{},{}", env.0, env.1, env.2, env.3);
        let q = format!(
            "/GISViewer/gis.gate/0/0/query?geometry={}&geometryType=esriGeometryEnvelope&inSR=32632&spatialRel=esriSpatialRelIntersects&outFields=UPIN,propertyNo&returnGeometry=true&outSR=32632&f=json",
            pct_encode(&geometry));
        match aumentum_get(&q).await {
            None => item("aumentum", "Overlap with Aumentum", "warn", "Aumentum's parcels could not be checked: the Aumentum link is not configured on this server.", vec![]),
            Some(Err(e)) => item("aumentum", "Overlap with Aumentum", "warn", format!("Aumentum's parcels could not be checked: {e}."), vec![]),
            Some(Ok(body)) => match serde_json::from_slice::<Value>(&body) {
                Err(_) => item("aumentum", "Overlap with Aumentum", "warn", "Aumentum's answer could not be read.", vec![]),
                Ok(v) if v.get("error").is_some() => item("aumentum", "Overlap with Aumentum", "warn", format!("Aumentum refused the search: {}", v["error"]), vec![]),
                Ok(v) => {
                    let mut hits: Vec<(String, f64)> = Vec::new();
                    for f in v["features"].as_array().cloned().unwrap_or_default() {
                        let upin = f["attributes"]["UPIN"].as_str().unwrap_or("").trim().to_string();
                        let rings = &f["geometry"]["rings"];
                        if !rings.is_array() || (!upin.is_empty() && mutated.contains(&upin)) { continue; }
                        let other = json!({ "type": "Polygon", "coordinates": rings }).to_string();
                        let r = sqlx::query(&format!(
                            "SELECT ST_Area(ST_Intersection(o, {g})) AS a, ST_Contains(o, ST_SetSRID(ST_MakePoint($3, $4), 32632)) AS at_point \
                             FROM (SELECT ST_MakeValid(ST_SetSRID(ST_GeomFromGeoJSON($2), 32632)) AS o) s"))
                            .bind(&shape).bind(&other).bind(pe).bind(pn).fetch_one(&mut *conn).await;
                        let Ok(r) = r else { continue };
                        let a: f64 = r.get::<Option<f64>, _>("a").unwrap_or(0.0);
                        let label = if upin.is_empty() { "an Aumentum parcel without UPIN".to_string() } else { upin.clone() };
                        if r.get::<Option<bool>, _>("at_point").unwrap_or(false) && !aum_at_point.contains(&label) { aum_at_point.push(label.clone()); }
                        if a > MIN_OVERLAP_M2 && !hits.iter().any(|h| h.0 == label) { hits.push((label, a)); }
                    }
                    hits.sort_by(|a, b| b.1.total_cmp(&a.1));
                    aum_upins = hits.iter().map(|h| h.0.clone()).collect();
                    if hits.is_empty() {
                        item("aumentum", "Overlap with Aumentum", "pass", "No parcel in Aumentum is overlapped.", vec![])
                    } else {
                        let d = hits.iter().take(6).map(|(u, a)| format!("{u} ({a:.2} m²)")).collect::<Vec<_>>().join(", ");
                        item("aumentum", "Overlap with Aumentum", "fail", format!("The parcel overlaps Aumentum's {d}."), aum_upins.clone())
                    }
                }
            },
        }
    };
    let aumentum_checked = aum_item.result != "warn";
    items.push(aum_item);
    let mut taken: Vec<String> = at_point_here.clone();
    for u in &aum_at_point { if !taken.contains(u) { taken.push(u.clone()); } }
    items.push(if !taken.is_empty() {
        item("drop_point", "Drop point (is the plot free?)", "fail",
            format!("The plot at E {pe:.3}, N {pn:.3} is already digitised: {}.", list(&taken)), taken.clone())
    } else if aumentum_checked {
        item("drop_point", "Drop point (is the plot free?)", "pass", format!("Nothing is digitised at E {pe:.3}, N {pn:.3}, here or in Aumentum."), vec![])
    } else {
        item("drop_point", "Drop point (is the plot free?)", "warn", format!("Nothing is digitised at E {pe:.3}, N {pn:.3} here; Aumentum could not be asked."), vec![])
    });

    // 4. A title on the land: overlapped parcels linked to a Registry property, and the job's own
    //    Registry property already holding a UPIN.
    let mut touched: Vec<String> = our_upins.clone();
    for u in aum_upins.iter().chain(taken.iter()) { if u.starts_with("PL") && !touched.contains(u) { touched.push(u.clone()); } }
    let titled: Vec<(String, Option<String>, Option<String>)> = if touched.is_empty() { Vec::new() } else {
        sqlx::query_as(
            "SELECT u.upin, COALESCE(r.su_id, a.lrs_id), r.refnum FROM unnest($1::text[]) AS u(upin) \
             LEFT JOIN cadastre.registry_property r ON r.upin = u.upin \
             LEFT JOIN LATERAL (SELECT lrs_id FROM cadastre.parcel_attribute x WHERE x.upin = u.upin AND x.linkage_status = 1 \
                                AND (x.status = '0' OR x.status IS NULL) LIMIT 1) a ON true \
             WHERE r.su_id IS NOT NULL OR a.lrs_id IS NOT NULL")
            .bind(&touched).fetch_all(&mut *conn).await?
    };
    let mut title_fail: Vec<String> = titled.iter().map(|(u, su, rn)| match (su, rn) {
        (Some(s), Some(r)) => format!("{u} is titled to Registry property {s} ({r})"),
        (Some(s), None) => format!("{u} is titled to Registry property {s}"),
        _ => format!("{u} is linked in the Registry"),
    }).collect();
    let mut title_upins: Vec<String> = titled.iter().map(|t| t.0.clone()).collect();
    let mut title_warn: Option<String> = None;
    let mut txn_item = item("transaction", "Registry transaction", "skipped", "The job has no Registry transaction.", vec![]);
    if let Some(number) = txn.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
        let read = async {
            let cfg = crate::routes::registry::config()?;
            let session = crate::lrs::Session::start(&cfg).await.map_err(|e| AppError::Conflict(e.to_string()))?;
            let t = crate::routes::registry::load_transaction(&session, number).await;
            session.end().await;
            t
        }.await;
        match read {
            Ok(Some(t)) => {
                txn_item = match &t.refused {
                    None => item("transaction", "Registry transaction", "pass", format!("{} is {}.", t.transaction_number, t.status), vec![]),
                    Some(why) => item("transaction", "Registry transaction", "fail", why.clone(), vec![]),
                };
                let prop = t.properties.iter()
                    .find(|p| property_number.as_deref().is_some_and(|n| p.refnum.as_deref().is_some_and(|r| r.eq_ignore_ascii_case(n))))
                    .or(t.properties.first());
                if let Some(p) = prop {
                    if let Some(u) = p.upin.as_deref().filter(|u| !mutated.iter().chain(own_posted.iter()).any(|m| m.eq_ignore_ascii_case(u))) {
                        title_fail.push(format!("Registry property {} ({}) already holds UPIN {u}", p.su_id, p.refnum.as_deref().unwrap_or("no property number")));
                        if !title_upins.iter().any(|x| x == u) { title_upins.push(u.to_string()); }
                    }
                }
            }
            Ok(None) => txn_item = item("transaction", "Registry transaction", "fail", format!("The Registry has no transaction {number}."), vec![]),
            Err(e) => {
                let why = match e { AppError::Conflict(m) | AppError::BadRequest(m) | AppError::NotFound(m) => m, other => other.to_string() };
                txn_item = item("transaction", "Registry transaction", "warn", format!("The Registry could not be asked about {number}: {why}."), vec![]);
                title_warn = Some(format!("The job's Registry property could not be checked ({why})."));
            }
        }
    }
    items.push(if !title_fail.is_empty() {
        item("title", "Title on the land", "fail", format!("A title already exists: {}.", title_fail.join("; ")), title_upins)
    } else if let Some(w) = title_warn {
        item("title", "Title on the land", "warn", format!("No title found on the land, but {}", w.to_lowercase()), vec![])
    } else {
        item("title", "Title on the land", "pass", "No title exists on this land, and the Registry property holds no UPIN.", vec![])
    });
    items.push(txn_item);

    // 5. Near misses: tiny gaps to committed neighbours.
    let near: Vec<(String, f64)> = sqlx::query_as(&format!(
        "SELECT p.parcel_number, ST_Distance(p.geom, {g}) AS d FROM cadastre.parcel p \
         WHERE p.status = 'posted' AND ST_DWithin(p.geom, {g}, $2) AND NOT ST_Intersects(p.geom, {g}) AND NOT (p.parcel_number = ANY($3)) \
         ORDER BY d LIMIT 10"))
        .bind(&shape).bind(NEAR_MISS_M).bind(&mutated).fetch_all(&mut *conn).await?;
    items.push(if near.is_empty() {
        item("near_miss", "Gaps to neighbours", "pass", "No gap under 10 cm to a neighbouring parcel.", vec![])
    } else {
        let d = near.iter().map(|(u, d)| format!("{u} ({:.1} cm)", d * 100.0)).collect::<Vec<_>>().join(", ");
        item("near_miss", "Gaps to neighbours", "warn", format!("Small gaps to {d}: snap the shared corners if they are the same beacons."), near.iter().map(|n| n.0.clone()).collect())
    });

    let ok = !items.iter().any(|i| i.result == "fail");
    let warnings = items.iter().any(|i| i.result == "warn");
    Ok(PrecheckReport { ok, warnings, items, checked_at: chrono::Utc::now().to_rfc3339() })
}

/// The failures of a report, one line, for refusing a commit.
pub fn failures(r: &PrecheckReport) -> String {
    r.items.iter().filter(|i| i.result == "fail").map(|i| format!("{}: {}", i.label, i.detail)).collect::<Vec<_>>().join(" ")
}

/// Run the system check for a job's sketch (or its draft parcels). With `acknowledge`, the
/// officer's reason for going on despite warnings is recorded in the audit log.
#[utoipa::path(post, path = "/api/v1/wf/jobs/{id}/precheck", tag = "wf", params(("id" = i32, Path, description = "Job instance id")),
    request_body = PrecheckRequest, responses((status = 200, body = PrecheckReport)))]
pub async fn precheck(State(state): State<AppState>, user: AuthUser, Path(id): Path<i32>, Json(req): Json<PrecheckRequest>) -> ApiResult<Json<PrecheckReport>> {
    let report = run(&state, id, &req.vertices, req.editing_id).await?;
    if let Some(reason) = req.acknowledge.as_deref().map(str::trim).filter(|r| !r.is_empty()) {
        if !report.ok {
            return Err(AppError::Conflict(format!("The system check failed; this cannot be overridden. {}", failures(&report))));
        }
        let actor = crate::routes::wf::resolve_actor(&state, &user).await?;
        let mut tx = state.pool.begin().await?;
        audit::append(&mut tx, &actor.label, "precheck_acknowledged", "wf_job_instance", Some(&id.to_string()), json!({
            "reason": reason,
            "warnings": report.items.iter().filter(|i| i.result == "warn").map(|i| json!({ "check": i.check, "detail": i.detail })).collect::<Vec<_>>(),
        })).await?;
        tx.commit().await?;
    }
    Ok(Json(report))
}
