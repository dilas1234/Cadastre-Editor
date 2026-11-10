//! "Linked from the Registry": the parcels the Registry admin has linked to a Registry property,
//! as the background sync (`registry_sync`) records them, with each parcel's place in the fabric
//! and the Cadastre job (if any) serving its transaction.

use axum::extract::{Path, Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::audit;
use crate::auth::AuthUser;
use crate::error::{ApiResult, AppError};
use crate::lrs::{self, LrsConfig, Session};
use crate::registry::{self, TransactionStatus};
use crate::registry_sync::{self, ReconcileReport, SyncReport};
use crate::state::AppState;

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct LinksQuery {
    /// How many days back (by link time); default 14, at most 365.
    pub days: Option<i32>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RegistryLinkDto {
    pub upin: String,
    /// The Registry property (SuID), written to the parcel's LRSID.
    pub su_id: String,
    /// The Registry document number (refnum), written to PropertyNumber.
    pub property_number: Option<String>,
    pub transaction_number: String,
    pub transaction_status: String,
    pub transaction_status_code: String,
    /// When the Registry linked it (its SpatialDate1, Registry time).
    pub linked_at: String,
    /// When this Cadastre picked it up.
    pub synced_at: String,
    pub lot: Option<String>,
    pub block: Option<String>,
    /// The posted parcel in the fabric (for zooming), if there is one.
    pub parcel_id: Option<Uuid>,
    /// The Cadastre job serving this transaction, if one exists.
    pub job_instance_id: Option<i32>,
    pub job_name: Option<String>,
    pub job_status: Option<i16>,
    /// The parcel is linked to this property in the Cadastre now (false after an unlink).
    pub linked_now: bool,
    /// `registry` (made by the Registry admin) or `cadastre` (made from this editor).
    pub made_in: String,
}

/// Parcels linked in the Registry, newest first.
#[utoipa::path(get, path = "/api/v1/registry/links", tag = "registry", params(LinksQuery),
    responses((status = 200, body = [RegistryLinkDto])))]
pub async fn links(State(state): State<AppState>, Query(q): Query<LinksQuery>) -> ApiResult<Json<Vec<RegistryLinkDto>>> {
    let days = q.days.unwrap_or(14).clamp(1, 365);
    let rows = sqlx::query(
        "SELECT a.entity_id AS upin, a.payload, a.created_at, \
                (SELECT p.id FROM cadastre.parcel p WHERE p.parcel_number = a.entity_id AND p.status = 'posted' LIMIT 1) AS parcel_id, \
                j.id AS job_id, j.name AS job_name, j.status AS job_status, a.action, \
                EXISTS (SELECT 1 FROM cadastre.parcel_attribute pa WHERE pa.upin = a.entity_id AND pa.lrs_id = a.payload->>'su_id' \
                        AND pa.linkage_status = 1 AND (pa.status = '0' OR pa.status IS NULL)) AS linked_now \
         FROM cadastre.audit_log a \
         LEFT JOIN LATERAL (SELECT w.id, w.name, w.status FROM cadastre.wf_job_instance w \
                            WHERE w.lrs_transaction_number = a.payload->>'transaction_number' ORDER BY w.id DESC LIMIT 1) j ON true \
         WHERE a.entity = 'registry_link' AND a.action IN ('registry_link_synced', 'registry_link_made') \
           AND (a.payload->>'linked_at')::timestamp >= (now() AT TIME ZONE 'UTC') - make_interval(days => $1) \
         ORDER BY a.payload->>'linked_at' DESC, a.id DESC",
    )
    .bind(days)
    .fetch_all(&state.pool)
    .await?;
    let s = |p: &serde_json::Value, k: &str| p.get(k).and_then(|v| v.as_str()).map(str::to_string);
    Ok(Json(rows.iter().map(|r| {
        let p: serde_json::Value = r.get("payload");
        RegistryLinkDto {
            upin: r.get("upin"),
            su_id: s(&p, "su_id").unwrap_or_default(),
            property_number: s(&p, "property_number"),
            transaction_number: s(&p, "transaction_number").unwrap_or_default(),
            transaction_status: s(&p, "transaction_status").unwrap_or_default(),
            transaction_status_code: s(&p, "transaction_status_code").unwrap_or_default(),
            linked_at: s(&p, "linked_at").unwrap_or_default(),
            synced_at: r.get::<chrono::DateTime<chrono::Utc>, _>("created_at").to_rfc3339(),
            lot: s(&p, "lot"),
            block: s(&p, "block"),
            parcel_id: r.get("parcel_id"),
            job_instance_id: r.get("job_id"),
            job_name: r.get("job_name"),
            job_status: r.get("job_status"),
            linked_now: r.get("linked_now"),
            made_in: if r.get::<String, _>("action") == "registry_link_made" { "cadastre".into() } else { "registry".into() },
        }
    }).collect()))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RegistryStatusDto {
    /// The Registry sync is configured on this server.
    pub enabled: bool,
    /// Writes to the Registry (send a plan, link/unlink) are allowed (CADASTRE_REGISTRY_WRITE).
    pub write_enabled: bool,
    pub url: Option<String>,
    /// The last link check: `{ ok, at, report | error }`.
    #[schema(value_type = Object)]
    pub last: Option<serde_json::Value>,
    /// The last unlink reconcile: `{ ok, at, report | error }`.
    #[schema(value_type = Object)]
    pub last_reconcile: Option<serde_json::Value>,
    /// Parallel run: whether new UPINs are checked with Aumentum (`CADASTRE_AUMENTUM_JOBSERVICE_URL`).
    pub aumentum_check: bool,
    /// Parallel run: the last duplicate-UPIN watch (`at`, `ok`, `checked`, `duplicates`, `error`).
    #[schema(value_type = Object)]
    pub upin_watch: Option<serde_json::Value>,
}

/// Whether the Registry sync runs, and how its last check went.
#[utoipa::path(get, path = "/api/v1/registry/status", tag = "registry", responses((status = 200, body = RegistryStatusDto)))]
pub async fn status(State(state): State<AppState>) -> ApiResult<Json<RegistryStatusDto>> {
    let cfg = LrsConfig::from_env();
    Ok(Json(RegistryStatusDto {
        enabled: cfg.is_some(),
        write_enabled: cfg.as_ref().is_some_and(|c| c.write_enabled),
        url: cfg.map(|c| c.base_url),
        last: registry_sync::last_status(&state.pool).await,
        last_reconcile: registry_sync::last_reconcile(&state.pool).await,
        aumentum_check: crate::aumentum::UpinCheck::configured(),
        upin_watch: crate::upin_watch::last_status(&state.pool).await,
    }))
}

/// Parallel run: ask Aumentum about this Cadastre's UPINs now (the duplicate watch, instead of
/// waiting for its next pass). Read-only towards Aumentum.
#[utoipa::path(post, path = "/api/v1/registry/upin-watch", tag = "registry",
    responses((status = 200, description = "The watch outcome", body = Object), (status = 409, description = "The parallel-run check is not configured")))]
pub async fn upin_watch_now(State(state): State<AppState>, _user: AuthUser) -> ApiResult<Json<serde_json::Value>> {
    if !crate::aumentum::UpinCheck::configured() {
        return Err(AppError::Conflict("the parallel-run check is not configured on this server (CADASTRE_AUMENTUM_JOBSERVICE_URL)".into()));
    }
    Ok(Json(crate::upin_watch::check_once(&state.pool).await))
}

/// Check the Registry for new links now (instead of waiting for the next minute).
#[utoipa::path(post, path = "/api/v1/registry/sync", tag = "registry",
    responses((status = 200, body = SyncReport), (status = 409, description = "The Registry is not configured"), (status = 422, description = "The Registry could not be read")))]
pub async fn sync_now(State(state): State<AppState>, _user: AuthUser) -> ApiResult<Json<SyncReport>> {
    let cfg = LrsConfig::from_env().ok_or_else(|| AppError::Conflict("the Registry connection is not configured on this server".into()))?;
    registry_sync::sync_and_record(&state.pool, &cfg).await.map(Json).map_err(AppError::UnprocessableEntity)
}

/// Check the Registry for unlinks now (instead of waiting for the next reconcile).
#[utoipa::path(post, path = "/api/v1/registry/reconcile", tag = "registry",
    responses((status = 200, body = ReconcileReport), (status = 409, description = "The Registry is not configured"), (status = 422, description = "The Registry could not be read")))]
pub async fn reconcile_now(State(state): State<AppState>, _user: AuthUser) -> ApiResult<Json<ReconcileReport>> {
    let cfg = config()?;
    registry_sync::reconcile_and_record(&state.pool, &cfg).await.map(Json).map_err(AppError::UnprocessableEntity)
}

pub(crate) fn config() -> ApiResult<LrsConfig> {
    LrsConfig::from_env().ok_or_else(|| AppError::Conflict("the Registry connection is not configured on this server".into()))
}

pub(crate) fn write_config() -> ApiResult<LrsConfig> {
    let c = config()?;
    if !c.write_enabled {
        return Err(AppError::Conflict("writing to the Registry is turned off on this server (CADASTRE_REGISTRY_WRITE)".into()));
    }
    Ok(c)
}

fn registry_err(e: impl std::fmt::Display) -> AppError {
    AppError::UnprocessableEntity(e.to_string())
}

/// A transaction number, property id or UPIN as typed: letters, digits and '-' only (they go
/// into the Registry's HQL, as in Aumentum's query definitions).
pub(crate) fn ident(kind: &str, v: &str) -> ApiResult<String> {
    let v = v.trim();
    if v.is_empty() || v.len() > 40 || !v.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err(AppError::BadRequest(format!("invalid {kind}: {v}")));
    }
    Ok(v.to_string())
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct RegistryPropertyDto {
    /// lr_spatial_unit.id (the entity id the Registry is written with).
    pub id: String,
    pub su_id: String,
    /// The UPIN the Registry holds for the property (empty: not linked).
    pub upin: Option<String>,
    pub refnum: Option<String>,
    pub lot: Option<String>,
    pub block: Option<String>,
    pub linked_at: Option<String>,
    /// The property's location (Registry dictionary `location`, e.g. "Jos - Bukuru Metropolis").
    #[serde(default)]
    pub location: Option<String>,
    /// The property's layout (Registry dictionary `layout_type`).
    #[serde(default)]
    pub layout: Option<String>,
    /// The property's area (Registry dictionary `property_area`).
    #[serde(default)]
    pub area: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RegistryTransactionDto {
    pub id: i64,
    pub transaction_number: String,
    pub status_code: String,
    pub status: String,
    /// Link, unlink and sending a plan are allowed (Active only, as in Aumentum).
    pub open: bool,
    /// Why not, if not (Aumentum's message).
    pub refused: Option<String>,
    pub properties: Vec<RegistryPropertyDto>,
}

pub(crate) async fn load_transaction(session: &Session, number: &str) -> ApiResult<Option<RegistryTransactionDto>> {
    let t = session
        .query(&format!("select t.Id, t.TransactionNumber, s.Code, s.Label from com.landsystems.lrs.model.Transaction t join t.TransactionStatus s where t.TransactionNumber = '{number}'"), 5)
        .await
        .map_err(registry_err)?;
    let Some(t) = t.into_iter().find(|r| r.len() >= 4) else { return Ok(None) };
    let props = session
        .query(&format!(
            "select s.Id, s.SuID, s.Upin, s.Refnum, s.LotNumber, s.BlockNumber, s.SpatialDate1, z.Label, y.Label, pa.Label from com.landsystems.lrs.model.Transaction t \
             join t.Properties l join l.SpatialUnits s left join s.Zone z left join s.Layout y left join s.PropertyArea pa \
             where t.TransactionNumber = '{number}' and l.EndLifespanVersion is null and s.EndLifespanVersion is null"
        ), 500)
        .await
        .map_err(registry_err)?;
    let o = |s: &String| Some(s.trim().to_string()).filter(|s| !s.is_empty());
    let status = TransactionStatus { code: t[2].clone(), label: t[3].clone() };
    let refused = registry::check_link_allowed(number, Some(&status)).err().map(|e| e.to_string());
    Ok(Some(RegistryTransactionDto {
        id: t[0].trim().parse().map_err(|_| registry_err("the Registry returned no transaction id"))?,
        transaction_number: t[1].clone(),
        status_code: t[2].clone(),
        status: t[3].clone(),
        open: refused.is_none(),
        refused,
        properties: props.iter().filter(|r| r.len() >= 7).map(|r| RegistryPropertyDto {
            id: r[0].trim().to_string(), su_id: r[1].trim().to_string(), upin: o(&r[2]), refnum: o(&r[3]),
            lot: o(&r[4]), block: o(&r[5]), linked_at: lrs::parse_datetime(&r[6]).map(|d| d.format("%Y-%m-%dT%H:%M:%S").to_string()),
            location: r.get(7).and_then(o), layout: r.get(8).and_then(o), area: r.get(9).and_then(o),
        }).collect(),
    }))
}

/// A Registry transaction: its status and properties (read live from the Registry).
/// The Registry's place lists for the parcel form: `location` (e.g. "Jos - Bukuru Metropolis"),
/// `layout_type` and `property_area`, read from the Registry's dictionary (read-only). A copy is kept
/// (`registry.lookups`) and refreshed when older than a day, so the form still works when the
/// Registry cannot be reached (`stale` is then true and `error` says why).
#[utoipa::path(get, path = "/api/v1/registry/lookups", tag = "registry", responses((status = 200)))]
pub async fn lookups(State(state): State<AppState>) -> ApiResult<Json<serde_json::Value>> {
    const KEY: &str = "registry.lookups";
    let cached: Option<serde_json::Value> = sqlx::query_scalar::<_, String>("SELECT value FROM cadastre.app_setting WHERE key = $1")
        .bind(KEY).fetch_optional(&state.pool).await?.and_then(|v| serde_json::from_str(&v).ok());
    let fresh = cached.as_ref()
        .and_then(|c| c.get("fetched_at")).and_then(|v| v.as_str())
        .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
        .is_some_and(|t| chrono::Utc::now().signed_duration_since(t) < chrono::Duration::hours(24));
    if fresh {
        return Ok(Json(cached.unwrap_or_default()));
    }
    let read = async {
        let cfg = config()?;
        let session = Session::start(&cfg).await.map_err(registry_err)?;
        let rows = session.query(
            "select d.Id, d.Code, d.Label, d.Category from com.landsystems.lrs.model.Dictionary d \
             where d.Category in ('location', 'layout_type', 'property_area') and d.EndLifespanVersion is null \
             order by d.Category, d.SortOrder, d.Label", 5000).await;
        session.end().await;
        rows.map_err(registry_err)
    }.await;
    match read {
        Ok(rows) => {
            let mut lists: std::collections::BTreeMap<String, Vec<serde_json::Value>> = ["location", "layout_type", "property_area"].iter().map(|k| (k.to_string(), Vec::new())).collect();
            for r in rows.iter().filter(|r| r.len() >= 4) {
                let label = r[2].trim();
                if label.is_empty() { continue; }
                if let Some(l) = lists.get_mut(r[3].trim()) {
                    l.push(serde_json::json!({ "id": r[0].trim(), "code": r[1].trim(), "label": label }));
                }
            }
            let mut out = serde_json::json!({ "fetched_at": chrono::Utc::now().to_rfc3339(), "stale": false });
            for (k, v) in lists { out[k] = serde_json::Value::Array(v); }
            sqlx::query("INSERT INTO cadastre.app_setting (key, value, source, updated_by) VALUES ($1, $2, 'registry', 'registry-lookups') \
                         ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_by = EXCLUDED.updated_by, updated_at = now()")
                .bind(KEY).bind(out.to_string()).execute(&state.pool).await?;
            Ok(Json(out))
        }
        Err(e) => match cached {
            Some(mut c) => { c["stale"] = true.into(); c["error"] = e.to_string().into(); Ok(Json(c)) }
            None => Err(AppError::Conflict(format!("The Registry's place lists could not be read ({e}) and no copy is kept yet."))),
        },
    }
}

#[utoipa::path(get, path = "/api/v1/registry/transactions/{number}", tag = "registry",
    params(("number" = String, Path, description = "Transaction number, e.g. PLS000138648")),
    responses((status = 200, body = RegistryTransactionDto), (status = 404, description = "No such transaction"), (status = 422, description = "The Registry could not be read")))]
pub async fn transaction(Path(number): Path<String>) -> ApiResult<Json<RegistryTransactionDto>> {
    let number = ident("transaction number", &number)?;
    let cfg = config()?;
    let session = Session::start(&cfg).await.map_err(registry_err)?;
    let t = load_transaction(&session, &number).await;
    session.end().await;
    t?.map(Json).ok_or_else(|| AppError::NotFound(format!("Cannot retrieve information of the Transaction. Transaction number:'{number}'.")))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RegistryLinkRequest {
    pub transaction_number: String,
    /// The Registry property (SuID) of that transaction.
    pub su_id: String,
    /// The Cadastre parcel.
    pub upin: String,
}

/// Link a parcel to a Registry property from the Cadastre (Aumentum LinkageManager.LinkParcel):
/// Active transactions only; neither side already linked; the Registry property takes the
/// parcel's UPIN and the parcel takes the property's fields (the Registry's LinkageFields).
#[utoipa::path(post, path = "/api/v1/registry/link", tag = "registry", request_body = RegistryLinkRequest,
    responses((status = 200, body = RegistryPropertyDto), (status = 400, description = "Not a property of that transaction"),
              (status = 409, description = "Already linked, transaction not Active, or writing turned off"), (status = 422, description = "The Registry refused")))]
pub async fn link(State(state): State<AppState>, user: AuthUser, Json(req): Json<RegistryLinkRequest>) -> ApiResult<Json<RegistryPropertyDto>> {
    cadastre_linking_allowed()?;
    let actor = crate::routes::wf::resolve_actor(&state, &user).await?;
    let number = ident("transaction number", &req.transaction_number)?;
    let su = ident("property id", &req.su_id)?;
    let upin = ident("UPIN", &req.upin)?.to_uppercase();
    Ok(Json(link_parcel(&state, &actor.label, &number, Some(&su), None, &upin, "cadastre").await?))
}

/// Link a committed parcel to its Registry file's property as the Registry's own Map → Connect
/// does: the property takes the UPIN and the link date (SpatialDate1, the Registry's clock), and the
/// parcel takes the property's fields (LinkageFields: LRSID, property number, …), LinkageStatus 1
/// and the link date. `su` names the property; else the one whose number is `property_number`, or
/// the transaction's only property. Refused, with the reason, unless it is safe: the transaction is
/// Active, the property holds no other UPIN, and no other property holds this UPIN.
pub(crate) async fn link_parcel(state: &AppState, actor: &str, number: &str, su: Option<&str>, property_number: Option<&str>, upin: &str, how: &str)
    -> ApiResult<RegistryPropertyDto> {
    let number = ident("transaction number", number)?;
    let upin = ident("UPIN", upin)?.to_uppercase();
    let cfg = write_config()?;
    let posted: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM cadastre.parcel WHERE parcel_number = $1 AND status = 'posted')").bind(&upin).fetch_one(&state.pool).await?;
    if !posted {
        return Err(AppError::BadRequest(format!("{upin} is not a posted parcel")));
    }
    let linked: Option<String> = sqlx::query_scalar("SELECT lrs_id FROM cadastre.parcel_attribute WHERE upin = $1 AND linkage_status = 1 AND (status = '0' OR status IS NULL) LIMIT 1")
        .bind(&upin).fetch_optional(&state.pool).await?.flatten();
    let session = Session::start(&cfg).await.map_err(registry_err)?;
    let result = async {
        let t = load_transaction(&session, &number).await?
            .ok_or_else(|| AppError::NotFound(format!("Cannot retrieve information of the Transaction. Transaction number:'{number}'.")))?;
        if let Some(why) = &t.refused {
            return Err(AppError::Conflict(why.clone()));
        }
        let prop = match su {
            Some(su) => t.properties.iter().find(|p| p.su_id.eq_ignore_ascii_case(su)).cloned()
                .ok_or_else(|| AppError::BadRequest(format!("{su} is not a property of {number}")))?,
            None => {
                let by_number = property_number.map(str::trim).filter(|n| !n.is_empty())
                    .and_then(|n| t.properties.iter().find(|p| p.refnum.as_deref().is_some_and(|r| r.eq_ignore_ascii_case(n))));
                match (by_number, t.properties.as_slice()) {
                    (Some(p), _) => p.clone(),
                    (None, [only]) => only.clone(),
                    (None, []) => return Err(AppError::Conflict(format!("{number} has no property to link to"))),
                    (None, _) => return Err(AppError::Conflict(format!("{number} has {} properties and none is {}: link it in the Registry (Map → Connect)",
                        t.properties.len(), property_number.unwrap_or("the job's property number")))),
                }
            }
        };
        if let Some(other) = prop.upin.as_deref().filter(|u| !u.eq_ignore_ascii_case(&upin)) {
            return Err(AppError::Conflict(format!("The Registry property {} is already linked to {other}.", prop.su_id)));
        }
        // After an OK on the Registry's map the parcel already holds this property: the desktop's
        // UpdateCadastralData wrote the Cadastre's half, and the Registry's half is what is missing.
        if let Some(p) = linked.as_deref().filter(|p| !(how == MAP_OK && p.eq_ignore_ascii_case(&prop.su_id))) {
            return Err(AppError::Conflict(format!("The parcel {upin} is already linked to the Registry property {p}.")));
        }
        // No other current property may hold this UPIN.
        let holders = session.query(&format!(
            "select s.SuID from com.landsystems.lrs.model.SpatialUnit s where s.Upin = '{upin}' and s.EndLifespanVersion is null"), 50).await.map_err(registry_err)?;
        if let Some(h) = holders.iter().filter_map(|r| r.first()).map(|v| v.trim().to_string()).find(|h| !h.is_empty() && !h.eq_ignore_ascii_case(&prop.su_id)) {
            return Err(AppError::Conflict(format!("{upin} is already held by the Registry property {h}")));
        }
        let settings = registry_sync::registry_settings(&session).await.map_err(registry_err)?;
        let (to_registry, to_parcel) = registry::parse_linkage_fields(settings.get("LinkageFields").map(String::as_str).unwrap_or("upin:UPIN;LRSID:suid,PropertyNumber:refnum,Plot:plotNumber,Block:blockNumber"));
        // The link date on the Registry's clock, as Map → Connect stamps it.
        let registry_now = chrono::Local::now() + chrono::Duration::seconds(session.clock_offset_secs());
        let stamp = registry_now.format("%Y-%m-%dT%H:%M:%S%.3f%:z").to_string();
        // The Registry side first (Aumentum: SaveProperty per LinkageFields entry, plus the link date).
        let upin_field = to_registry.iter().find(|m| m.target.eq_ignore_ascii_case("UPIN")).map(|m| m.source.clone()).unwrap_or_else(|| "upin".into());
        session.save_entity(t.id, &lrs::entity_xml("Parcel", &[("Id", Some(&prop.id)), (&upin_field, Some(&upin)), ("SpatialDate1", Some(&stamp))]))
            .await.map_err(registry_err)?;
        let cols = [("SuID", Some(prop.su_id.clone())), ("Refnum", prop.refnum.clone()), ("LotNumber", prop.lot.clone()), ("BlockNumber", prop.block.clone()), ("Upin", Some(upin.clone())), ("Id", Some(prop.id.clone()))];
        let updates = registry::cadastre_link_updates(&to_parcel, &cols, registry_now.date_naive());
        Ok::<_, AppError>((t, prop, updates, registry_now))
    }
    .await;
    session.end().await;
    let (t, prop, updates, registry_now) = result?;
    let mut tx = state.pool.begin().await?;
    ensure_attribute_record(&mut tx, &upin).await?;
    registry_sync::write_attributes(&mut tx, &upin, &updates).await?;
    sqlx::query("INSERT INTO cadastre.registry_property (su_id, registry_id, upin, refnum) VALUES ($1, $2, $3, $4) ON CONFLICT (su_id) DO UPDATE SET upin = EXCLUDED.upin, registry_id = EXCLUDED.registry_id, refnum = EXCLUDED.refnum, seen_at = now()")
        .bind(&prop.su_id).bind(prop.id.parse::<i64>().ok()).bind(&upin).bind(&prop.refnum).execute(&mut *tx).await?;
    let linked_at = registry_now.format("%Y-%m-%dT%H:%M:%S").to_string();
    audit::append(&mut tx, actor, "registry_link_made", "registry_link", Some(&upin), serde_json::json!({
        "su_id": prop.su_id, "transaction_number": t.transaction_number, "property_number": prop.refnum, "written": updates,
        "linked_at": linked_at, "link_date_written": true, "how": how, "transaction_status": t.status, "transaction_status_code": t.status_code,
    })).await?;
    tx.commit().await?;
    Ok(RegistryPropertyDto { upin: Some(upin), linked_at: Some(linked_at), ..prop })
}

/// After a commit: link each committed parcel to the job's Registry file (CADASTRE_REGISTRY_LINK_ON_COMMIT,
/// default on when writing to the Registry is allowed). The commit itself already stands; a parcel
/// that cannot be linked is reported (and listed under link problems) instead of failing it.
pub(crate) async fn link_after_commit(state: &AppState, actor: &str, job_instance_id: i32, upins: &[String]) -> Vec<serde_json::Value> {
    let on = std::env::var("CADASTRE_REGISTRY_LINK_ON_COMMIT").map(|v| v.trim() != "0").unwrap_or(true);
    if !on { return Vec::new(); }
    link_after_commit_forced(state, actor, job_instance_id, upins).await
}
async fn link_after_commit_forced(state: &AppState, actor: &str, job_instance_id: i32, upins: &[String]) -> Vec<serde_json::Value> {
    if upins.is_empty() || write_config().is_err() {
        return Vec::new();
    }
    let job: Option<(Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT j.lrs_transaction_number, l.property_number FROM cadastre.wf_job_instance j LEFT JOIN cadastre.wf_job_linkage l ON l.wf_job_instance_id = j.id WHERE j.id = $1")
        .bind(job_instance_id).fetch_optional(&state.pool).await.ok().flatten();
    let Some((Some(txn), pn)) = job.map(|(t, p)| (t.map(|t| t.trim().to_string()).filter(|t| !t.is_empty()), p)) else { return Vec::new() };
    let mut out = Vec::new();
    for upin in upins {
        match link_parcel(state, actor, &txn, None, pn.as_deref(), upin, "commit").await {
            Ok(p) => out.push(serde_json::json!({ "upin": upin, "linked": true, "su_id": p.su_id, "property_number": p.refnum, "linked_at": p.linked_at })),
            Err(e) => {
                let why = match e { AppError::Conflict(m) | AppError::BadRequest(m) | AppError::NotFound(m) => m, other => other.to_string() };
                if let Ok(mut tx) = state.pool.begin().await {
                    if audit::append(&mut tx, actor, "registry_link_failed", "registry_link", Some(upin), serde_json::json!({
                        "transaction_number": txn, "job_instance_id": job_instance_id, "reason": why })).await.is_ok() { let _ = tx.commit().await; }
                }
                out.push(serde_json::json!({ "upin": upin, "linked": false, "reason": why }));
            }
        }
    }
    out
}

/// How a link made by pressing OK on the Registry's map is recorded.
const MAP_OK: &str = "map_ok";

/// CADASTRE_REGISTRY_LINK_ON_MAP_OK=1: pressing OK on the Registry's map also writes the UPIN and
/// the link date to the Registry, so the link no longer waits for the officer to save the file
/// (Suspend). Off unless set. It needs CADASTRE_REGISTRY_WRITE=1 as well.
pub(crate) fn link_on_map_ok() -> bool {
    std::env::var("CADASTRE_REGISTRY_LINK_ON_MAP_OK").map(|v| v.trim() == "1").unwrap_or(false)
}

/// The property's one Active transaction, from rows of (id, number, status code, …). The map's OK
/// does not say which file is open, so none or several Active leaves the choice to the officer.
fn only_active_transaction(rows: &[Vec<String>]) -> Result<String, String> {
    let mut active: Vec<&str> = rows.iter().filter(|r| r.len() >= 3 && r[2] == "transaction_lodged").map(|r| r[1].trim()).collect();
    active.sort_unstable();
    active.dedup();
    match active.as_slice() {
        [one] => Ok(one.to_string()),
        [] => Err("the property has no Active transaction".into()),
        many => Err(format!("the property has {} Active transactions ({}): save the file in the Registry", many.len(), many.join(", "))),
    }
}

/// The Registry's half of an OK on its map. `Ok(false)`: the Registry already holds it. Every check
/// is `link_parcel`'s, and it is `link_parcel` that writes.
async fn write_map_ok_link(state: &AppState, actor: &str, su: &str, property_number: Option<&str>, upin: &str) -> ApiResult<bool> {
    let su = ident("property id", su)?;
    let upin = ident("UPIN", upin)?.to_uppercase();
    let cfg = write_config()?;
    let session = Session::start(&cfg).await.map_err(registry_err)?;
    let found = async {
        let rows = session.query(&format!(
            "select t.Id, t.TransactionNumber, s.Code, s.Label, p.Id, p.Upin from com.landsystems.lrs.model.Transaction t join t.TransactionStatus s \
             join t.Properties l join l.SpatialUnits p where p.SuID = '{su}' and l.EndLifespanVersion is null and p.EndLifespanVersion is null"), 200).await.map_err(registry_err)?;
        let number = only_active_transaction(&rows).map_err(AppError::Conflict)?;
        let t = load_transaction(&session, &number).await?
            .ok_or_else(|| AppError::NotFound(format!("Cannot retrieve information of the Transaction. Transaction number:'{number}'.")))?;
        let prop = t.properties.into_iter().find(|p| p.su_id.eq_ignore_ascii_case(&su))
            .ok_or_else(|| AppError::Conflict(format!("{su} is not a property of {number}")))?;
        Ok::<_, AppError>((number, prop))
    }
    .await;
    session.end().await;
    let (number, prop) = found?;
    if let (Some(sent), Some(held)) = (property_number.map(str::trim).filter(|n| !n.is_empty()), prop.refnum.as_deref()) {
        if !sent.eq_ignore_ascii_case(held) {
            return Err(AppError::Conflict(format!("the map sent property {sent} but {su} is {held}")));
        }
    }
    if prop.linked_at.is_some() && prop.upin.as_deref().is_some_and(|u| u.eq_ignore_ascii_case(&upin)) {
        return Ok(false);
    }
    link_parcel(state, actor, &number, Some(&su), None, &upin, MAP_OK).await?;
    Ok(true)
}

/// Record that the Registry's half of an OK was not written, for link problems and the file's
/// history. The Cadastre's half stands; saving the file in the Registry still completes the link.
pub(crate) async fn map_ok_not_written(state: &AppState, actor: &str, upin: &str, su: Option<&str>, reason: &str) {
    tracing::warn!("Registry map: {upin} was not written to the Registry on OK: {reason}");
    if let Ok(mut tx) = state.pool.begin().await {
        if audit::append(&mut tx, actor, "registry_link_failed", "registry_link", Some(upin), serde_json::json!({
            "su_id": su, "how": MAP_OK, "reason": reason })).await.is_ok() { let _ = tx.commit().await; }
    }
}

/// After an OK on the Registry's map: write its half of the link. It runs after the desktop has
/// its answer, with a time limit, so a slow Registry never holds the desktop; it reports instead of
/// failing, because the desktop's own save remains a complete way to finish the link.
pub(crate) async fn link_after_map_ok(state: &AppState, actor: &str, su: &str, property_number: Option<&str>, upin: &str) {
    let why = match tokio::time::timeout(std::time::Duration::from_secs(60), write_map_ok_link(state, actor, su, property_number, upin)).await {
        Ok(Ok(true)) => { tracing::info!("Registry map: {upin} written to the Registry property {su} on OK"); return; }
        Ok(Ok(false)) => { tracing::info!("Registry map: the Registry property {su} already holds {upin}"); return; }
        Ok(Err(AppError::Conflict(m) | AppError::BadRequest(m) | AppError::NotFound(m))) => m,
        Ok(Err(other)) => other.to_string(),
        Err(_) => "the Registry did not answer within 60 seconds".to_string(),
    };
    map_ok_not_written(state, actor, upin, Some(su), &why).await;
}

/// Aumentum kept one ParcelAttributeTable record per parcel; a parcel committed here may not have
/// one yet. Give it one (continuing Aumentum's OBJECTIDs) so the link has a home.
async fn ensure_attribute_record(conn: &mut sqlx::PgConnection, upin: &str) -> ApiResult<()> {
    let has: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM cadastre.parcel_attribute WHERE upin = $1 AND (status = '0' OR status IS NULL))").bind(upin).fetch_one(&mut *conn).await?;
    if !has {
        sqlx::query("INSERT INTO cadastre.parcel_attribute (object_id, upin, status, creation_date, last_edit_date, parcel_area) \
                     SELECT nextval('cadastre.parcel_attribute_objectid_seq'), $1, '0', now()::timestamp, now()::timestamp, \
                            (SELECT trunc(ST_Area(geom)::numeric, 2) FROM cadastre.parcel WHERE parcel_number = $1 AND status = 'posted' LIMIT 1)")
            .bind(upin).execute(&mut *conn).await?;
    }
    Ok(())
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RegistryUnlinkRequest {
    pub upin: String,
}

/// Unlink a parcel from its Registry property from the Cadastre (Aumentum LinkParcel, Unlink):
/// the property's transaction must be Active; the Registry property's UPIN is cleared and the
/// parcel's link fields with it.
#[utoipa::path(post, path = "/api/v1/registry/unlink", tag = "registry", request_body = RegistryUnlinkRequest,
    responses((status = 200), (status = 409, description = "Not linked, transaction not Active, or writing turned off"), (status = 422, description = "The Registry refused")))]
pub async fn unlink(State(state): State<AppState>, user: AuthUser, Json(req): Json<RegistryUnlinkRequest>) -> ApiResult<Json<serde_json::Value>> {
    cadastre_linking_allowed()?;
    let actor = crate::routes::wf::resolve_actor(&state, &user).await?;
    let upin = ident("UPIN", &req.upin)?.to_uppercase();
    let cfg = write_config()?;
    let su: String = sqlx::query_scalar::<_, Option<String>>("SELECT lrs_id FROM cadastre.parcel_attribute WHERE upin = $1 AND linkage_status = 1 AND (status = '0' OR status IS NULL) LIMIT 1")
        .bind(&upin).fetch_optional(&state.pool).await?.flatten()
        .ok_or_else(|| AppError::Conflict(format!("The parcel {upin} should be linked to unlink it.")))?;
    let su = ident("property id", &su)?;
    let session = Session::start(&cfg).await.map_err(registry_err)?;
    let result = async {
        // The property's transactions (Aumentum E_TRANSPROPPARSUID); the Active one, if any.
        let rows = session.query(&format!(
            "select t.Id, t.TransactionNumber, s.Code, s.Label, p.Id, p.Upin from com.landsystems.lrs.model.Transaction t join t.TransactionStatus s \
             join t.Properties l join l.SpatialUnits p where p.SuID = '{su}' and p.EndLifespanVersion is null"), 50).await.map_err(registry_err)?;
        let row = rows.iter().filter(|r| r.len() >= 6).find(|r| r[2] == "transaction_lodged").or_else(|| rows.iter().find(|r| r.len() >= 6))
            .ok_or_else(|| AppError::Conflict(format!("No Registry transaction holds the property {su}.")))?;
        registry::check_link_allowed(&row[1], Some(&TransactionStatus { code: row[2].clone(), label: row[3].clone() })).map_err(|e| AppError::Conflict(e.to_string()))?;
        let tid: i64 = row[0].trim().parse().map_err(|_| registry_err("the Registry returned no transaction id"))?;
        let settings = registry_sync::registry_settings(&session).await.map_err(registry_err)?;
        let (to_registry, to_parcel) = registry::parse_linkage_fields(settings.get("LinkageFields").map(String::as_str).unwrap_or("upin:UPIN;LRSID:suid,PropertyNumber:refnum,Plot:plotNumber,Block:blockNumber"));
        for m in &to_registry {
            session.save_entity(tid, &lrs::entity_xml("Parcel", &[("Id", Some(row[4].trim())), (&m.source, Some(""))])).await.map_err(registry_err)?;
        }
        Ok::<_, AppError>((row[1].clone(), to_parcel))
    }
    .await;
    session.end().await;
    let (txn, to_parcel) = result?;
    let updates = registry::cadastre_unlink_updates(&to_parcel);
    let mut tx = state.pool.begin().await?;
    registry_sync::write_attributes(&mut tx, &upin, &updates).await?;
    sqlx::query("DELETE FROM cadastre.registry_property WHERE su_id = $1").bind(&su).execute(&mut *tx).await?;
    audit::append(&mut tx, &actor.label, "registry_unlink_made", "registry_link", Some(&upin), serde_json::json!({ "su_id": su, "transaction_number": txn })).await?;
    tx.commit().await?;
    Ok(Json(serde_json::json!({ "upin": upin, "su_id": su, "transaction_number": txn })))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SendPlanRequest {
    /// The job file to send: the title deed plan PDF (a JPG is turned into a PDF first).
    pub file_id: Uuid,
    /// Resend a corrected plan: the job's plan was sent before. The new PDF is filed as
    /// "Survey Plan (corrected)" and the plans sent before are marked as replaced. Without it, a
    /// second send is refused so the transaction does not collect duplicates by accident.
    #[serde(default)]
    pub resend: bool,
}

/// The plan as the Registry receives it: always a PDF. A PDF is sent as it is; a JPG (the plan
/// image of earlier versions, or one attached by hand) becomes a one-page A4 PDF. Returns the
/// name to file it under (`.pdf`) and the bytes.
pub fn plan_as_pdf(name: &str, bytes: &[u8]) -> ApiResult<(String, Vec<u8>)> {
    let (stem, ext) = name.rsplit_once('.').map(|(s, e)| (s.to_string(), e.to_ascii_lowercase())).unwrap_or_else(|| (name.to_string(), String::new()));
    match ext.as_str() {
        "pdf" if bytes.starts_with(b"%PDF-") => Ok((name.to_string(), bytes.to_vec())),
        "pdf" => Err(AppError::BadRequest(format!("{name} is not a PDF file"))),
        "jpg" | "jpeg" => Ok((format!("{stem}.pdf"), crate::pdf::jpeg_a4_pdf(bytes).map_err(|e| AppError::BadRequest(format!("{name}: {e}")))?)),
        _ => Err(AppError::BadRequest("send the title deed plan as a PDF (Title Deed Plan → Attach as PDF); a JPG is also accepted and sent as a PDF".into())),
    }
}

/// The plan documents already filed in a transaction (ids, newest first), for a resend to mark as
/// replaced. Read-only. The Registry's HQL names for the document–transaction relation are not
/// documented, so the likely forms are tried in turn; `Err` says none answered.
async fn plan_documents(session: &Session, transaction_id: i64, doc_type: &str) -> Result<Vec<String>, String> {
    let doc_type = doc_type.replace('\'', "");
    let forms = [
        format!("select d.id from com.landsystems.lrs.model.IncomingDocument d where d.Transaction.id = {transaction_id} and d.DocumentType.Code = '{doc_type}' order by d.id desc"),
        format!("select d.id from com.landsystems.lrs.model.Transaction t join t.IncomingDocuments d where t.id = {transaction_id} and d.DocumentType.Code = '{doc_type}' order by d.id desc"),
        format!("select d.id from com.landsystems.lrs.model.Transaction t join t.Documents d where t.id = {transaction_id} and d.DocumentType.Code = '{doc_type}' order by d.id desc"),
    ];
    let mut last = String::new();
    for hql in &forms {
        match session.query(hql, 50).await {
            Ok(rows) => return Ok(rows.iter().filter_map(|r| r.first().map(|v| v.trim().to_string())).filter(|v| !v.is_empty()).collect()),
            Err(e) => last = e.to_string(),
        }
    }
    Err(last)
}

/// Send the title deed plan to the job's Registry transaction (Aumentum SendImageCommand →
/// SendImageForm → PersistDocumentInRegistry): Active transactions only, not a P- transaction.
/// The plan always goes as a PDF, uploaded and filed in the transaction as a new incoming
/// document of type SurveyPlanFromMC (sd_titledeedPlan) with an `attachment` page (setting
/// `aumentum.Integration.PlanPageType`). With `resend`, the corrected plan is filed and the plans
/// sent before are marked as replaced (their Comments); nothing in the Registry is deleted. A job
/// under plan correction (`correct_plan`) always sends this way, with the reason in the Comments:
/// the plan it replaces may have been filed by Aumentum, so only the Registry knows of it.
/// Once the plan is filed the job is completed (`complete_with_plan`): the Registry reads the job
/// when its stage is completed and refuses while it is open.
#[utoipa::path(post, path = "/api/v1/wf/jobs/{id}/registry/send-plan", tag = "registry", request_body = SendPlanRequest,
    params(("id" = i32, Path, description = "Job instance id")),
    responses((status = 200), (status = 400, description = "No transaction on the job, or not a PDF/JPG of this job"),
              (status = 409, description = "Transaction not Active, writing turned off, or already sent (use resend)"), (status = 422, description = "The Registry refused")))]
pub async fn send_plan(State(state): State<AppState>, user: AuthUser, Path(id): Path<i32>, Json(req): Json<SendPlanRequest>) -> ApiResult<Json<serde_json::Value>> {
    let actor = crate::routes::wf::resolve_actor(&state, &user).await?;
    let cfg = write_config()?;
    let txn: Option<String> = sqlx::query_scalar("SELECT lrs_transaction_number FROM cadastre.wf_job_instance WHERE id = $1").bind(id)
        .fetch_optional(&state.pool).await?.ok_or_else(|| AppError::NotFound(format!("job instance {id}")))?;
    let txn = txn.map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
        .ok_or_else(|| AppError::BadRequest("the job has no Registry transaction to send the plan to".into()))?;
    if txn.to_lowercase().starts_with("p-") {
        return Err(AppError::BadRequest(format!("{txn} is not a transaction a plan is sent to (Aumentum leaves P- transactions out)")));
    }
    let txn = ident("transaction number", &txn)?;
    let sent_before: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar(
        "SELECT max(s.created_at) FROM cadastre.audit_log s WHERE s.entity = 'wf_job_instance' AND s.entity_id = $1 AND s.action = 'registry_plan_sent' AND s.payload->>'transaction_number' = $2 \
           AND s.id > COALESCE((SELECT max(w.id) FROM cadastre.audit_log w WHERE w.entity = 'wf_job_instance' AND w.entity_id = $1 AND w.action = 'registry_plan_withdrawn'), 0)")
        .bind(id.to_string()).bind(&txn).fetch_one(&state.pool).await?;
    let correction = open_plan_correction(&state.pool, id).await?;
    if let (Some(at), false) = (sent_before, req.resend || correction.is_some()) {
        return Err(AppError::Conflict(format!("the plan was already sent to {txn} ({} UTC). To correct it, use Resend corrected plan.", at.format("%Y-%m-%d %H:%M"))));
    }
    let (name, _mime, bytes, job) = crate::routes::wfadmin::read_job_file(&state.pool, req.file_id).await?;
    if job != id {
        return Err(AppError::BadRequest("that file is not attached to this job".into()));
    }
    let (sent_name, pdf) = plan_as_pdf(&name, &bytes)?;
    let doc_type = registry_sync::setting(&state.pool, "aumentum.Integration.SurveyPlanFromMC").await?.unwrap_or_else(|| "sd_titledeedPlan".into());
    let page_type = registry_sync::setting(&state.pool, "aumentum.Integration.PlanPageType").await?.unwrap_or_else(|| "attachment".into());
    let resend = (req.resend && sent_before.is_some()) || correction.is_some();
    let now = chrono::Local::now();
    let stamp = now.format("%d-%m-%Y %H:%M").to_string();
    let comments = if resend { corrected_comments(&stamp, correction.as_ref().map(|c| c.reason.as_str())) } else { "Survey Plan".to_string() };
    let session = Session::start(&cfg).await.map_err(registry_err)?;
    let result = async {
        let t = load_transaction(&session, &txn).await?
            .ok_or_else(|| AppError::NotFound(format!("Cannot retrieve information of the Transaction. Transaction number:'{txn}'.")))?;
        if let Some(why) = &t.refused {
            return Err(AppError::Conflict(why.clone()));
        }
        // The plans already filed, looked up before the corrected one is added.
        let earlier = if resend { Some(plan_documents(&session, t.id, &doc_type).await) } else { None };
        let uploaded = session.upload("pdf", &pdf).await.map_err(registry_err)?;
        let submission = now.format("%Y-%m-%dT%H:%M:%S%.3f%:z").to_string();
        session.save_entity(t.id, &lrs::incoming_document_xml(&doc_type, t.id, &sent_name, &uploaded, &submission, &comments, &page_type)).await.map_err(registry_err)?;
        // Mark the earlier plans as replaced. The corrected plan is filed already, so a failure
        // here is reported, not fatal.
        let (mut replaced, mut replace_note) = (Vec::<String>::new(), None::<String>);
        match earlier {
            Some(Ok(ids)) => {
                let note = format!("Survey Plan - REPLACED by the corrected plan of {}", now.format("%d-%m-%Y %H:%M"));
                for d in ids {
                    match session.save_entity(t.id, &lrs::entity_xml("IncomingDocument", &[("Id", Some(&d)), ("Comments", Some(&note))])).await {
                        Ok(()) => replaced.push(d),
                        Err(e) => { replace_note = Some(format!("document {d} could not be marked as replaced: {e}")); }
                    }
                }
                if replaced.is_empty() && replace_note.is_none() {
                    replace_note = Some("no earlier plan document was found in the transaction to mark".into());
                }
            }
            Some(Err(e)) => replace_note = Some(format!("the earlier plan could not be looked up in the Registry ({e}); mark it as replaced in the Registry")),
            None => {}
        }
        Ok::<_, AppError>((t, replaced, replace_note))
    }
    .await;
    session.end().await;
    let (t, replaced, replace_note) = result?;
    let mut tx = state.pool.begin().await?;
    audit::append(&mut tx, &actor.label, "registry_plan_sent", "wf_job_instance", Some(&id.to_string()), serde_json::json!({
        "transaction_number": t.transaction_number, "file_id": req.file_id, "file_name": sent_name, "source_file": name, "bytes": pdf.len(),
        "document_type": doc_type, "page_type": page_type, "resend": resend, "comments": comments, "replaced": replaced, "replace_note": replace_note,
        "reason": correction.as_ref().map(|c| c.reason.clone()),
    })).await?;
    // The plan is filed: the job ends with it (a job already completed stays as it is).
    let ended = complete_with_plan(&mut tx, &actor.label, id, if resend { "resend" } else { "send" }).await?;
    tx.commit().await?;
    Ok(Json(serde_json::json!({ "transaction_number": t.transaction_number, "file_name": sent_name, "document_type": doc_type,
        "resend": resend, "replaced": replaced, "replace_note": replace_note, "job_completed": ended.completed, "job_left_open": ended.left_open })))
}

/// The Comments of a corrected plan in the Registry (the Registry keeps 240 characters).
fn corrected_comments(stamp: &str, reason: Option<&str>) -> String {
    match reason {
        Some(r) => format!("Survey Plan (corrected, {stamp}): {r}").chars().take(240).collect(),
        None => format!("Survey Plan (corrected, {stamp})"),
    }
}

/// What `complete_with_plan` did with the job.
struct PlanJobEnd {
    /// The job was completed now.
    completed: bool,
    /// Why an open job was left open (uncommitted parcels, a parcel retirement).
    left_open: Option<String>,
}

/// The job ends with its plan, as in Aumentum (Send image, then complete Generate Title Deed Plan):
/// the open task is closed and the job is completed (3, "Waiting for LRS Commit") with its end
/// date. The Registry reads the job (`lr_mc_job`, refreshed by GetJobInstance) when an officer
/// completes the stage, and refuses while it has no end date ("Aumentum Cadastre contains not
/// completed 'Deed Plan Creationv2' Job Instance"). A job already closed is left as it is; a job
/// with uncommitted parcels or a parcel retirement is left open, with the reason.
async fn complete_with_plan(tx: &mut sqlx::PgConnection, actor: &str, id: i32, via: &str) -> ApiResult<PlanJobEnd> {
    let row = sqlx::query("SELECT j.current_task_id, s.is_open FROM cadastre.wf_job_instance j JOIN cadastre.wf_job_status s ON s.code = j.status WHERE j.id = $1 FOR UPDATE OF j")
        .bind(id).fetch_optional(&mut *tx).await?.ok_or_else(|| AppError::NotFound(format!("job instance {id}")))?;
    let (task, open): (Option<i32>, bool) = (row.get(0), row.get(1));
    if !open {
        return Ok(PlanJobEnd { completed: false, left_open: None });
    }
    let drafts: i64 = sqlx::query_scalar("SELECT count(*) FROM cadastre.parcel p JOIN cadastre.wf_job_instance w ON w.cadastre_job_id = p.job_id WHERE w.id = $1 AND p.status = 'draft'")
        .bind(id).fetch_one(&mut *tx).await?;
    if drafts > 0 {
        return Ok(PlanJobEnd { completed: false, left_open: Some(format!("the job has {drafts} uncommitted parcel(s): commit or discard them, then Complete job")) });
    }
    if crate::routes::mutations::has_pending_retirement(&mut *tx, id).await? {
        return Ok(PlanJobEnd { completed: false, left_open: Some("the job has a parcel retirement waiting: commit or discard it, then Complete job".into()) });
    }
    sqlx::query("UPDATE cadastre.wf_task_instance SET ended = now()::timestamp, started = COALESCE(started, now()::timestamp) WHERE job_instance_id = $1 AND ended IS NULL")
        .bind(id).execute(&mut *tx).await?;
    sqlx::query("UPDATE cadastre.wf_job_instance SET status = 3, ended = now()::timestamp, updated = now()::timestamp WHERE id = $1")
        .bind(id).execute(&mut *tx).await?;
    audit::append(&mut *tx, actor, "wf_complete", "wf_job_instance", Some(&id.to_string()), serde_json::json!({ "plan_sent": true, "via": via, "at_task": task })).await?;
    Ok(PlanJobEnd { completed: true, left_open: None })
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CompletePlanJobDto {
    pub job_instance_id: i32,
    pub completed: bool,
}

/// Complete a job whose plan was sent (Send plan completes it; this is for a job left open, e.g.
/// one sent before that rule, or one that still had uncommitted parcels then). Refused when no
/// plan was sent from the job, when it is closed already, or while it has uncommitted parcels.
#[utoipa::path(post, path = "/api/v1/wf/jobs/{id}/registry/complete", tag = "registry",
    params(("id" = i32, Path, description = "Job instance id")),
    responses((status = 200, body = CompletePlanJobDto), (status = 404, description = "No such job"),
              (status = 409, description = "No plan sent from the job, closed already, or uncommitted parcels")))]
pub async fn complete_plan_job(State(state): State<AppState>, user: AuthUser, Path(id): Path<i32>) -> ApiResult<Json<CompletePlanJobDto>> {
    let actor = crate::routes::wf::resolve_actor(&state, &user).await?;
    let mut tx = state.pool.begin().await?;
    let sent: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM cadastre.audit_log s WHERE s.entity = 'wf_job_instance' AND s.entity_id = $1 AND s.action = 'registry_plan_sent' \
           AND s.id > COALESCE((SELECT max(w.id) FROM cadastre.audit_log w WHERE w.entity = 'wf_job_instance' AND w.entity_id = $1 AND w.action = 'registry_plan_withdrawn'), 0))")
        .bind(id.to_string()).fetch_one(&mut *tx).await?;
    if !sent {
        return Err(AppError::Conflict("no title deed plan has been sent from this job: send the plan first".into()));
    }
    let ended = complete_with_plan(&mut tx, &actor.label, id, "complete").await?;
    if let Some(why) = ended.left_open {
        return Err(AppError::Conflict(format!("the job is not completed: {why}")));
    }
    if !ended.completed {
        return Err(AppError::Conflict("the job is already closed".into()));
    }
    tx.commit().await?;
    Ok(Json(CompletePlanJobDto { job_instance_id: id, completed: true }))
}

#[derive(Debug, Clone, Serialize)]
struct PlanCorrection {
    reason: String,
    /// The job was opened for this correction, so it completes when the plan is sent.
    created_job: bool,
}

/// The job's correction still waiting for its plan: started, and no plan sent since.
async fn open_plan_correction(pool: &sqlx::PgPool, id: i32) -> ApiResult<Option<PlanCorrection>> {
    let payload: Option<serde_json::Value> = sqlx::query_scalar(
        "SELECT c.payload FROM cadastre.audit_log c WHERE c.entity = 'wf_job_instance' AND c.entity_id = $1 AND c.action = 'title_deed_plan_correction' \
           AND c.id > COALESCE((SELECT max(s.id) FROM cadastre.audit_log s WHERE s.entity = 'wf_job_instance' AND s.entity_id = $1 AND s.action = 'registry_plan_sent'), 0) \
         ORDER BY c.id DESC LIMIT 1")
        .bind(id.to_string()).fetch_optional(pool).await?;
    Ok(payload.map(|p| PlanCorrection {
        reason: p["reason"].as_str().unwrap_or_default().to_string(),
        created_job: p["created_job"].as_bool().unwrap_or(false),
    }))
}

/// The job's plan correction waiting to be sent, if any (Send plan shows it).
#[utoipa::path(get, path = "/api/v1/wf/jobs/{id}/registry/plan-correction", tag = "registry", params(("id" = i32, Path, description = "Job instance id")),
    responses((status = 200)))]
pub async fn plan_correction(State(state): State<AppState>, Path(id): Path<i32>) -> ApiResult<Json<serde_json::Value>> {
    Ok(Json(serde_json::json!({ "correction": open_plan_correction(&state.pool, id).await? })))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CorrectPlanRequest {
    /// The linked parcel whose plan is corrected.
    pub upin: String,
    /// The file's Registry transaction, if the caller knows it; found from the parcel otherwise.
    #[serde(default)]
    pub transaction_number: Option<String>,
    /// Why the plan is corrected (History, and the Comments of the plan in the Registry).
    pub reason: String,
}

/// Correct a title deed plan that came back: the parcel, its UPIN and its Registry link stay; only
/// the plan is redone and resent. Allowed for a parcel linked on an Active transaction at Raise
/// Title Deed Plan, read live from the Registry. Uses the transaction's open job, or a new
/// Job-<file number>, with the latest saved form of the parcel carried over.
#[utoipa::path(post, path = "/api/v1/registry/correct-plan", tag = "registry", request_body = CorrectPlanRequest,
    responses((status = 200), (status = 400, description = "No reason given"), (status = 404, description = "No such transaction"),
              (status = 409, description = "Not linked, not Active, not at Raise Title Deed Plan, or writing turned off"), (status = 422, description = "The Registry could not be read")))]
pub async fn correct_plan(State(state): State<AppState>, user: AuthUser, Json(req): Json<CorrectPlanRequest>) -> ApiResult<Json<serde_json::Value>> {
    let actor = crate::routes::wf::resolve_actor(&state, &user).await?;
    let upin = ident("UPIN", &req.upin)?.to_uppercase();
    let reason = req.reason.trim().to_string();
    if reason.is_empty() {
        return Err(AppError::BadRequest("say why the plan is corrected".into()));
    }
    let posted: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM cadastre.parcel WHERE parcel_number = $1 AND status = 'posted')")
        .bind(&upin).fetch_one(&state.pool).await?;
    if !posted {
        return Err(AppError::Conflict(format!("{upin} is not a committed parcel of this Cadastre")));
    }
    let work = parcel_work(State(state.clone()), Path(upin.clone())).await?.0;
    let txn = match req.transaction_number.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
        Some(t) => ident("transaction number", t)?.to_uppercase(),
        None => work.transaction_number.clone().ok_or_else(|| AppError::Conflict(format!(
            "the Registry transaction of {upin} is not known here: use Correct plan on its card in Registry → Find file")))?.to_uppercase(),
    };
    let cfg = write_config()?;
    let session = Session::start(&cfg).await.map_err(registry_err)?;
    let read = async {
        let t = load_transaction(&session, &txn).await?
            .ok_or_else(|| AppError::NotFound(format!("Cannot retrieve information of the Transaction. Transaction number:'{txn}'.")))?;
        let stage = session.query(&format!(
            "select ph.TaskName, ph.TaskDescription from com.landsystems.lrs.model.ProcessHistory ph \
             where ph.TaskComplete is null and ph.ProcessName = 'Raise Title Deed Plan' and ph.TransactionId = {}", t.id), 5).await.map_err(registry_err)?;
        Ok::<_, AppError>((t, !stage.is_empty()))
    }.await;
    session.end().await;
    let (t, at_stage) = read?;
    if let Some(why) = &t.refused {
        return Err(AppError::Conflict(why.clone()));
    }
    let property = t.properties.iter().find(|p| p.upin.as_deref().is_some_and(|u| u.eq_ignore_ascii_case(&upin)))
        .ok_or_else(|| AppError::Conflict(format!("{upin} is not linked to a property of {txn}: the Registry admin links it first")))?;
    if !at_stage {
        return Err(AppError::Conflict(format!("{txn} is not at Raise Title Deed Plan in the Registry: a plan is corrected at that stage")));
    }
    let file = property.refnum.clone().or(work.property_number.clone());
    let open: Option<i32> = sqlx::query_scalar("SELECT j.id FROM cadastre.wf_job_instance j JOIN cadastre.wf_job_status s ON s.code = j.status \
                                                WHERE upper(j.lrs_transaction_number) = $1 AND s.is_open ORDER BY j.id DESC LIMIT 1")
        .bind(&txn).fetch_optional(&state.pool).await?;
    if let Some(id) = open {
        if let Some(c) = open_plan_correction(&state.pool, id).await? {
            return Ok(Json(serde_json::json!({ "job_instance_id": id, "upin": upin, "transaction_number": txn, "reason": c.reason, "already_open": true })));
        }
    }
    let (id, created_job) = match open {
        Some(id) => (id, false),
        None => (create_file_job(&state.pool, &actor.label, &txn, file.as_deref(), "registry.correct-plan").await?.0, true),
    };
    let mut tx = state.pool.begin().await?;
    let carried = sqlx::query(
        "INSERT INTO cadastre.title_deed_plan (wf_job_instance_id, upin, fields, updated_by) \
         SELECT $1, upin, fields, updated_by FROM cadastre.title_deed_plan WHERE upin = $2 AND wf_job_instance_id <> $1 ORDER BY updated_at DESC LIMIT 1 \
         ON CONFLICT (wf_job_instance_id, upin) DO NOTHING")
        .bind(id).bind(&upin).execute(&mut *tx).await?.rows_affected() > 0;
    audit::append(&mut tx, &actor.label, "title_deed_plan_correction", "wf_job_instance", Some(&id.to_string()), serde_json::json!({
        "upin": upin, "transaction_number": txn, "reason": reason, "created_job": created_job, "form_carried_over": carried,
    })).await?;
    tx.commit().await?;
    Ok(Json(serde_json::json!({ "job_instance_id": id, "upin": upin, "transaction_number": txn, "reason": reason, "already_open": false })))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct WithdrawPlanRequest {
    /// Why the plan is withdrawn (recorded in the Registry document and the audit log).
    pub reason: String,
}

/// Withdraw a job's title deed plan: the plan documents in the job's Registry transaction are
/// marked "WITHDRAWN" (nothing is deleted in the Registry, as with a resend's "REPLACED"), and the
/// saved title deed form is cleared. The parcel and its UPIN stay. A new plan can then be sent.
#[utoipa::path(post, path = "/api/v1/wf/jobs/{id}/registry/withdraw-plan", tag = "registry", params(("id" = i32, Path, description = "Job instance id")),
    request_body = WithdrawPlanRequest, responses((status = 200), (status = 409, description = "No plan sent, or writing turned off")))]
pub async fn withdraw_plan(State(state): State<AppState>, user: AuthUser, Path(id): Path<i32>, Json(req): Json<WithdrawPlanRequest>) -> ApiResult<Json<serde_json::Value>> {
    let actor = crate::routes::wf::resolve_actor(&state, &user).await?;
    let reason = req.reason.trim().to_string();
    if reason.is_empty() {
        return Err(AppError::BadRequest("say why the plan is withdrawn".into()));
    }
    let txn: Option<String> = sqlx::query_scalar("SELECT lrs_transaction_number FROM cadastre.wf_job_instance WHERE id = $1").bind(id)
        .fetch_optional(&state.pool).await?.ok_or_else(|| AppError::NotFound(format!("job instance {id}")))?;
    let txn = ident("transaction number", txn.as_deref().map(str::trim).unwrap_or(""))?;
    let cfg = write_config()?;
    let doc_type = registry_sync::setting(&state.pool, "aumentum.Integration.SurveyPlanFromMC").await?.unwrap_or_else(|| "sd_titledeedPlan".into());
    let now = chrono::Local::now();
    let note: String = format!("Survey Plan - WITHDRAWN {} ({})", now.format("%d-%m-%Y %H:%M"), reason).chars().take(240).collect();
    let session = Session::start(&cfg).await.map_err(registry_err)?;
    let result = async {
        let t = load_transaction(&session, &txn).await?
            .ok_or_else(|| AppError::NotFound(format!("Cannot retrieve information of the Transaction. Transaction number:'{txn}'.")))?;
        let ids = plan_documents(&session, t.id, &doc_type).await
            .map_err(|e| AppError::Conflict(format!("the plans in {txn} could not be looked up in the Registry ({e}); nothing was changed")))?;
        if ids.is_empty() {
            return Err(AppError::Conflict(format!("there is no title deed plan in {txn} to withdraw")));
        }
        let (mut marked, mut failed) = (Vec::new(), Vec::new());
        for d in ids {
            match session.save_entity(t.id, &lrs::entity_xml("IncomingDocument", &[("Id", Some(&d)), ("Comments", Some(&note))])).await {
                Ok(()) => marked.push(d),
                Err(e) => failed.push(format!("{d}: {e}")),
            }
        }
        Ok::<_, AppError>((t, marked, failed))
    }.await;
    session.end().await;
    let (t, marked, failed) = result?;
    if marked.is_empty() {
        return Err(AppError::Conflict(format!("the plan could not be marked as withdrawn in the Registry: {}", failed.join("; "))));
    }
    let mut tx = state.pool.begin().await?;
    let cleared = sqlx::query("DELETE FROM cadastre.title_deed_plan WHERE wf_job_instance_id = $1").bind(id).execute(&mut *tx).await?.rows_affected();
    audit::append(&mut tx, &actor.label, "registry_plan_withdrawn", "wf_job_instance", Some(&id.to_string()), serde_json::json!({
        "transaction_number": t.transaction_number, "reason": reason, "documents": marked, "failed": failed, "comments": note, "forms_cleared": cleared,
    })).await?;
    tx.commit().await?;
    Ok(Json(serde_json::json!({ "transaction_number": t.transaction_number, "withdrawn": marked, "failed": failed, "forms_cleared": cleared })))
}

/// Link problems to check by hand (UPIN on several properties, UPIN unknown here, parcel linked
/// elsewhere, commit not linked).
#[utoipa::path(get, path = "/api/v1/registry/link-problems", tag = "registry", responses((status = 200)))]
pub async fn link_problems(State(state): State<AppState>) -> ApiResult<Json<serde_json::Value>> {
    Ok(Json(registry_sync::link_problems(&state.pool).await?))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct LinkJobRequest {
    /// Only this UPIN of the job (default: all its committed parcels).
    #[serde(default)]
    pub upin: Option<String>,
}

/// Link a job's committed parcel(s) to its Registry file now (as on commit): for a commit made
/// before linking on commit, or one that could not be linked then.
#[utoipa::path(post, path = "/api/v1/wf/jobs/{id}/registry/link", tag = "registry", params(("id" = i32, Path, description = "Job instance id")),
    request_body = LinkJobRequest, responses((status = 200)))]
pub async fn link_job(State(state): State<AppState>, user: AuthUser, Path(id): Path<i32>, Json(req): Json<LinkJobRequest>) -> ApiResult<Json<Vec<serde_json::Value>>> {
    let actor = crate::routes::wf::resolve_actor(&state, &user).await?;
    write_config()?;
    let upins: Vec<String> = sqlx::query_scalar(
        "SELECT p.parcel_number FROM cadastre.wf_job_instance w JOIN cadastre.parcel p ON p.job_id = w.cadastre_job_id \
         WHERE w.id = $1 AND p.status = 'posted' AND p.parcel_number IS NOT NULL AND ($2::text IS NULL OR p.parcel_number = upper($2)) ORDER BY p.posted_at")
        .bind(id).bind(req.upin.as_deref()).fetch_all(&state.pool).await?;
    if upins.is_empty() {
        return Err(AppError::Conflict("the job has no committed parcel to link".into()));
    }
    let out = link_after_commit_forced(&state, &actor.label, id, &upins).await;
    Ok(Json(out))
}

/// Whether a commit could ask Aumentum about a UPIN right now.
#[utoipa::path(get, path = "/api/v1/aumentum/status", tag = "registry", responses((status = 200, body = crate::aumentum::Health)))]
pub async fn aumentum_status() -> Json<crate::aumentum::Health> {
    Json(crate::aumentum::health().await)
}

/// The title deed queue: files the Registry has at Raise Title Deed Plan (C of O processing) whose
/// property the Registry admin has linked (holds a UPIN), read live from the Registry, with what this
/// Cadastre has for each (the parcel, the title deed job, the plan sent). The title deed unit works
/// them from here; the link itself stays with the Registry admin (Aumentum's flow).
#[utoipa::path(get, path = "/api/v1/registry/title-deed-queue", tag = "registry", responses((status = 200)))]
pub async fn title_deed_queue(State(state): State<AppState>) -> ApiResult<Json<serde_json::Value>> {
    let cfg = config()?;
    let session = Session::start(&cfg).await.map_err(registry_err)?;
    let read = async {
        let open = session.query(
            "select ph.TransactionId, ph.TaskName, ph.TaskDescription, ph.TaskCreate from com.landsystems.lrs.model.ProcessHistory ph \
             where ph.TaskComplete is null and ph.ProcessName = 'Raise Title Deed Plan'", 20000).await.map_err(registry_err)?;
        let mut stage: std::collections::HashMap<String, (String, String, String)> = std::collections::HashMap::new();
        for r in open.iter().filter(|r| r.len() >= 4) {
            stage.insert(r[0].trim().to_string(), (r[1].trim().to_string(), r[2].trim().to_string(), r[3].trim().to_string()));
        }
        let ids: Vec<String> = stage.keys().filter(|k| k.chars().all(|c| c.is_ascii_digit()) && !k.is_empty()).cloned().collect();
        let mut props = Vec::new();
        for chunk in ids.chunks(300) {
            let rows = session.query(&format!(
                "select t.Id, t.TransactionNumber, s.SuID, s.Refnum, s.Upin, s.SpatialDate1 from com.landsystems.lrs.model.Transaction t \
                 join t.Properties l join l.SpatialUnits s where t.Id in ({}) and l.EndLifespanVersion is null and s.EndLifespanVersion is null \
                 and s.Upin is not null and s.Upin <> ''", chunk.join(",")), 5000).await.map_err(registry_err)?;
            props.extend(rows);
        }
        Ok::<_, AppError>((stage, props))
    }.await;
    session.end().await;
    let (stage, props) = read?;
    let upins: Vec<String> = props.iter().filter(|r| r.len() >= 6).map(|r| r[4].trim().to_uppercase()).collect();
    let txns: Vec<String> = props.iter().filter(|r| r.len() >= 6).map(|r| r[1].trim().to_uppercase()).collect();
    // What this Cadastre has: the committed parcel; the latest job of the transaction; the plan sent.
    let parcels: Vec<(String, Option<chrono::DateTime<chrono::Utc>>)> = sqlx::query_as(
        "SELECT parcel_number, posted_at FROM cadastre.parcel WHERE status = 'posted' AND parcel_number = ANY($1)").bind(&upins).fetch_all(&state.pool).await?;
    let jobs: Vec<(String, i32, String, bool)> = sqlx::query_as(
        "SELECT DISTINCT ON (upper(j.lrs_transaction_number)) upper(j.lrs_transaction_number), j.id, s.label, s.is_open FROM cadastre.wf_job_instance j \
         JOIN cadastre.wf_job_status s ON s.code = j.status WHERE upper(j.lrs_transaction_number) = ANY($1) ORDER BY upper(j.lrs_transaction_number), j.id DESC")
        .bind(&txns).fetch_all(&state.pool).await?;
    let sent: Vec<(String, chrono::DateTime<chrono::Utc>, String)> = sqlx::query_as(
        "SELECT DISTINCT ON (upper(payload->>'transaction_number')) upper(payload->>'transaction_number'), created_at, action FROM cadastre.audit_log \
         WHERE action IN ('registry_plan_sent', 'registry_plan_withdrawn') AND upper(payload->>'transaction_number') = ANY($1) \
         ORDER BY upper(payload->>'transaction_number'), id DESC").bind(&txns).fetch_all(&state.pool).await?;
    let mut files: Vec<serde_json::Value> = props.iter().filter(|r| r.len() >= 6).map(|r| {
        let (tid, txn, upin) = (r[0].trim(), r[1].trim().to_uppercase(), r[4].trim().to_uppercase());
        let (task, task_label, since) = stage.get(tid).cloned().unwrap_or_default();
        let parcel = parcels.iter().find(|p| p.0 == upin);
        let job = jobs.iter().find(|j| j.0 == txn);
        let plan = sent.iter().find(|p| p.0 == txn);
        serde_json::json!({
            "transaction_number": txn, "su_id": r[2].trim(), "property_number": r[3].trim(), "upin": upin,
            "linked_at": lrs::parse_datetime(&r[5]).map(|d| d.format("%Y-%m-%dT%H:%M:%SZ").to_string()),
            "stage": task, "stage_label": task_label, "stage_since": lrs::parse_datetime(&since).map(|d| d.format("%Y-%m-%dT%H:%M:%SZ").to_string()),
            "in_cadastre": parcel.is_some(), "committed_at": parcel.and_then(|p| p.1).map(|t| t.to_rfc3339()),
            "job_instance_id": job.map(|j| j.1), "job_status": job.map(|j| j.2.clone()), "job_open": job.map(|j| j.3),
            "plan": plan.map(|p| serde_json::json!({ "action": p.2, "at": p.1.to_rfc3339() })),
        })
    }).collect();
    files.sort_by(|a, b| b["stage_since"].as_str().cmp(&a["stage_since"].as_str()));
    let waiting = stage.len();
    Ok(Json(serde_json::json!({ "files": files, "at_stage": waiting, "linked": files.len(), "read_at": chrono::Utc::now().to_rfc3339() })))
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FileHistoryQuery {
    /// The file's Registry transaction (PLS…). Or give a job or a UPIN and it is found.
    #[serde(default)]
    pub txn: Option<String>,
    #[serde(default)]
    pub job: Option<i32>,
    #[serde(default)]
    pub upin: Option<String>,
}

/// A file's history, read only: the Registry's stages for its transaction (workflow, stage, started,
/// finished, by, next) and the Registry's record of its Cadastre jobs, with this Cadastre's own
/// events (job and task history, commit, correction, title deed plan, links). Nothing is written.
/// When the Registry cannot be reached the Cadastre part is still returned.
#[utoipa::path(get, path = "/api/v1/registry/history", tag = "registry", params(FileHistoryQuery), responses((status = 200)))]
pub async fn file_history(State(state): State<AppState>, Query(q): Query<FileHistoryQuery>) -> ApiResult<Json<serde_json::Value>> {
    // Find the transaction from what was given.
    let mut txn = q.txn.as_deref().map(|t| t.trim().to_uppercase()).filter(|t| !t.is_empty());
    let upin_in = q.upin.as_deref().map(|u| u.trim().to_uppercase()).filter(|u| !u.is_empty());
    if txn.is_none() {
        if let Some(j) = q.job {
            txn = sqlx::query_scalar::<_, Option<String>>("SELECT upper(lrs_transaction_number) FROM cadastre.wf_job_instance WHERE id = $1").bind(j).fetch_optional(&state.pool).await?.flatten();
        }
    }
    if txn.is_none() {
        if let Some(u) = &upin_in {
            txn = sqlx::query_scalar::<_, Option<String>>(
                "SELECT upper(w.lrs_transaction_number) FROM cadastre.parcel p JOIN cadastre.wf_job_instance w ON w.cadastre_job_id = p.job_id \
                 WHERE p.parcel_number = $1 AND w.lrs_transaction_number IS NOT NULL ORDER BY w.id DESC LIMIT 1").bind(u).fetch_optional(&state.pool).await?.flatten();
            if txn.is_none() {
                // Aumentum's parcels: their job by the property number ("Job-PL68928").
                txn = sqlx::query_scalar::<_, Option<String>>(
                    "SELECT upper(w.lrs_transaction_number) FROM cadastre.parcel_attribute a JOIN cadastre.wf_job_instance w ON upper(w.name) = upper('Job-' || a.property_number) \
                     WHERE a.upin = $1 AND w.lrs_transaction_number IS NOT NULL ORDER BY w.id DESC LIMIT 1").bind(u).fetch_optional(&state.pool).await?.flatten();
            }
        }
    }
    let txn = txn.map(|t| t.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect::<String>());
    // This Cadastre: the file's jobs, their parcels, task history and audit events.
    let mut jobs: Vec<i32> = match &txn {
        Some(t) => sqlx::query_scalar("SELECT id FROM cadastre.wf_job_instance WHERE upper(lrs_transaction_number) = $1 ORDER BY id").bind(t).fetch_all(&state.pool).await?,
        None => Vec::new(),
    };
    if let Some(j) = q.job { if !jobs.contains(&j) { jobs.push(j); } }
    let mut upins: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT p.parcel_number FROM cadastre.wf_job_instance w JOIN cadastre.parcel p ON p.job_id = w.cadastre_job_id \
         WHERE w.id = ANY($1) AND p.parcel_number IS NOT NULL").bind(&jobs).fetch_all(&state.pool).await?;
    if let Some(u) = &upin_in { if !upins.contains(u) { upins.push(u.clone()); } }
    let tasks = sqlx::query(
        "SELECT ti.job_instance_id, w.name AS job_name, t.name AS task, \
                CASE WHEN w.source = 'aumentum' THEN ti.started AT TIME ZONE 'Africa/Lagos' ELSE ti.started AT TIME ZONE 'UTC' END AS started, \
                CASE WHEN w.source = 'aumentum' THEN ti.ended AT TIME ZONE 'Africa/Lagos' ELSE ti.ended AT TIME ZONE 'UTC' END AS ended, \
                COALESCE(p.name, p.code) AS who, ti.comment \
         FROM cadastre.wf_task_instance ti JOIN cadastre.wf_task t ON t.id = ti.task_id JOIN cadastre.wf_job_instance w ON w.id = ti.job_instance_id \
         LEFT JOIN cadastre.principal p ON p.sid = ti.sid WHERE ti.job_instance_id = ANY($1) ORDER BY ti.started NULLS LAST, ti.id").bind(&jobs).fetch_all(&state.pool).await?;
    let job_rows = sqlx::query(
        "SELECT w.id, w.name, jt.name AS job_type, \
                CASE WHEN w.source = 'aumentum' THEN w.created AT TIME ZONE 'Africa/Lagos' ELSE w.created AT TIME ZONE 'UTC' END AS created, \
                CASE WHEN w.source = 'aumentum' THEN w.ended AT TIME ZONE 'Africa/Lagos' ELSE w.ended AT TIME ZONE 'UTC' END AS ended, s.label \
         FROM cadastre.wf_job_instance w JOIN cadastre.wf_job_type jt ON jt.id = w.job_type_id \
         JOIN cadastre.wf_job_status s ON s.code = w.status WHERE w.id = ANY($1)").bind(&jobs).fetch_all(&state.pool).await?;
    let audits = sqlx::query(
        "SELECT created_at, actor, action, entity, entity_id, payload FROM cadastre.audit_log \
         WHERE (entity = 'wf_job_instance' AND entity_id = ANY($1)) OR (entity IN ('registry_link', 'upin') AND entity_id = ANY($2)) \
         ORDER BY id").bind(jobs.iter().map(|j| j.to_string()).collect::<Vec<_>>()).bind(&upins).fetch_all(&state.pool).await?;
    let ts = |d: Option<chrono::DateTime<chrono::Utc>>| d.map(|d| d.to_rfc3339());
    let mut cadastre: Vec<serde_json::Value> = Vec::new();
    for r in &job_rows {
        cadastre.push(serde_json::json!({ "at": ts(r.get("created")), "kind": "job_created", "job": r.get::<i32, _>("id"),
            "text": format!("Job {} created ({})", r.get::<Option<String>, _>("name").unwrap_or_default(), r.get::<String, _>("job_type")) }));
        if let Some(e) = r.get::<Option<chrono::DateTime<chrono::Utc>>, _>("ended") {
            cadastre.push(serde_json::json!({ "at": ts(Some(e)), "kind": "job_ended", "job": r.get::<i32, _>("id"),
                "text": format!("Job {} {}", r.get::<Option<String>, _>("name").unwrap_or_default(), r.get::<String, _>("label").to_lowercase()) }));
        }
    }
    for r in &tasks {
        cadastre.push(serde_json::json!({ "at": ts(r.get("started")), "end": ts(r.get("ended")), "kind": "task", "job": r.get::<i32, _>("job_instance_id"),
            "text": format!("Step: {}", r.get::<String, _>("task")), "by": r.get::<Option<String>, _>("who"), "note": r.get::<Option<String>, _>("comment") }));
    }
    for r in &audits {
        let action: String = r.get("action");
        if matches!(action.as_str(), "wf_advance" | "wf_create" | "wf_complete" | "wf_start_editing") { continue; } // shown by the task / job lines
        let p: serde_json::Value = r.get("payload");
        cadastre.push(serde_json::json!({ "at": r.get::<chrono::DateTime<chrono::Utc>, _>("created_at").to_rfc3339(), "kind": action, "by": r.get::<String, _>("actor"),
            "entity_id": r.get::<Option<String>, _>("entity_id"), "detail": p }));
    }
    // The Registry, read only.
    let mut registry = serde_json::json!({ "reachable": false });
    if let Some(t) = &txn {
        let read = async {
            let cfg = config()?;
            let session = Session::start(&cfg).await.map_err(registry_err)?;
            let out = async {
                let tr = load_transaction(&session, t).await?;
                let Some(tr) = tr else { return Ok::<_, AppError>(None) };
                let stages = session.query(&format!(
                    "select ph.ProcessName, ph.TaskName, ph.TaskDescription, ph.TaskCreate, ph.TaskComplete, ph.TaskTransition, ph.Actor \
                     from com.landsystems.lrs.model.ProcessHistory ph where ph.TransactionId = {} order by ph.TaskCreate", tr.id), 2000).await.map_err(registry_err)?;
                let mc = session.query(&format!(
                    "select j.JobInstanceId, j.JobName, j.JobStatus, j.JobStarted, j.JobEnded from com.landsystems.lrs.model.McJob j where j.TransactionId = {}", tr.id), 100).await.unwrap_or_default();
                Ok(Some((tr, stages, mc)))
            }.await;
            session.end().await;
            out
        }.await;
        let when = |v: &String| lrs::parse_datetime(v).map(|d| d.format("%Y-%m-%dT%H:%M:%SZ").to_string());
        let o = |v: &String| Some(v.trim().to_string()).filter(|s| !s.is_empty());
        registry = match read {
            Err(e) => {
                let why = match e { AppError::Conflict(m) | AppError::BadRequest(m) | AppError::NotFound(m) => m, other => other.to_string() };
                serde_json::json!({ "reachable": false, "error": format!("The Registry (10.10.10.5) can't be reached: its stages aren't shown ({why}).") })
            }
            Ok(None) => serde_json::json!({ "reachable": true, "found": false }),
            Ok(Some((tr, stages, mc))) => {
                let st: Vec<serde_json::Value> = stages.iter().filter(|r| r.len() >= 7).map(|r| serde_json::json!({
                    "workflow": r[0].trim(), "stage": r[1].trim(), "stage_label": o(&r[2]).unwrap_or_else(|| r[1].trim().to_string()),
                    "started": when(&r[3]), "finished": when(&r[4]), "next": o(&r[5]), "by": o(&r[6]),
                })).collect();
                let current: Vec<serde_json::Value> = st.iter().filter(|s| s["finished"].is_null()).cloned().collect();
                serde_json::json!({
                    "reachable": true, "found": true, "transaction_number": tr.transaction_number, "status": tr.status,
                    "properties": tr.properties.iter().map(|p| serde_json::json!({ "su_id": p.su_id, "property_number": p.refnum, "upin": p.upin, "linked_at": p.linked_at })).collect::<Vec<_>>(),
                    "current": current, "stages": st,
                    "cadastre_jobs": mc.iter().filter(|r| r.len() >= 5).map(|r| serde_json::json!({
                        "job": r[0].trim(), "name": r[1].trim(), "status": r[2].trim(), "started": when(&r[3]), "ended": when(&r[4]) })).collect::<Vec<_>>(),
                })
            }
        };
    }
    Ok(Json(serde_json::json!({ "transaction_number": txn, "jobs": jobs, "upins": upins, "registry": registry, "cadastre": cadastre })))
}

/// Plans this Cadastre has sent for a job (audit), newest first.
#[utoipa::path(get, path = "/api/v1/wf/jobs/{id}/registry/sent", tag = "registry", params(("id" = i32, Path, description = "Job instance id")),
    responses((status = 200)))]
pub async fn sent_plans(State(state): State<AppState>, Path(id): Path<i32>) -> ApiResult<Json<Vec<serde_json::Value>>> {
    let rows = sqlx::query("SELECT actor, payload, created_at FROM cadastre.audit_log WHERE entity = 'wf_job_instance' AND entity_id = $1 AND action = 'registry_plan_sent' ORDER BY id DESC")
        .bind(id.to_string()).fetch_all(&state.pool).await?;
    Ok(Json(rows.iter().map(|r| serde_json::json!({ "by": r.get::<String, _>("actor"), "at": r.get::<chrono::DateTime<chrono::Utc>, _>("created_at").to_rfc3339(), "detail": r.get::<serde_json::Value, _>("payload") })).collect()))
}

/// A parcel's Registry link history (synced and made here), newest first.
#[utoipa::path(get, path = "/api/v1/registry/parcels/{upin}/events", tag = "registry", params(("upin" = String, Path, description = "UPIN")),
    responses((status = 200)))]
pub async fn parcel_events(State(state): State<AppState>, Path(upin): Path<String>) -> ApiResult<Json<Vec<serde_json::Value>>> {
    let upin = ident("UPIN", &upin)?.to_uppercase();
    Ok(Json(registry_sync::events_for(&state.pool, &upin).await?))
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FindQuery {
    /// A transaction number (PLS…) or a file / property number (PL…).
    pub q: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RegistryFileDto {
    pub transaction_number: String,
    pub status_code: String,
    pub status: String,
    /// The file (property) number, e.g. PL68955.
    pub property_number: Option<String>,
    pub su_id: Option<String>,
    /// The UPIN the Registry holds for the property, once linked.
    pub upin: Option<String>,
    /// The open Cadastre job serving the transaction, if any.
    pub job_instance_id: Option<i32>,
    pub job_name: Option<String>,
    /// The UPINs this Cadastre already holds for the file number (current parcels): the file is
    /// digitised (here or in Aumentum) even when the Registry is not linked yet.
    pub cadastre_upins: Vec<String>,
}

/// The current parcels' UPINs for a file number (PL…), in this Cadastre.
pub(crate) async fn digitised_upins(pool: &sqlx::PgPool, file: &str) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT DISTINCT upin FROM cadastre.parcel_attribute WHERE upin IS NOT NULL AND upin <> '' \
           AND upper(replace(property_number, ' ', '')) = upper(replace($1, ' ', '')) AND (status = '0' OR status IS NULL) ORDER BY upin",
    ).bind(file).fetch_all(pool).await
}

/// Find a Registry file by transaction (PLS…) or file number (PL…), read live from the Registry,
/// with the Cadastre job already serving it (the digitising unit's search).
#[utoipa::path(get, path = "/api/v1/registry/find", tag = "registry", params(FindQuery),
    responses((status = 200, body = [RegistryFileDto]), (status = 422, description = "The Registry could not be read")))]
pub async fn find(State(state): State<AppState>, Query(q): Query<FindQuery>) -> ApiResult<Json<Vec<RegistryFileDto>>> {
    let term = ident("number", &registry::normalise_file_query(&q.q))?;
    let cfg = config()?;
    let by_txn = term.starts_with("PLS") || term.starts_with("P-");
    let hql = format!(
        "select t.TransactionNumber, ts.Code, ts.Label, s.Refnum, s.SuID, s.Upin from com.landsystems.lrs.model.Transaction t join t.TransactionStatus ts \
         left join t.Properties l left join l.SpatialUnits s where {} and (l is null or l.EndLifespanVersion is null) and (s is null or s.EndLifespanVersion is null) \
         order by t.TransactionNumber desc",
        if by_txn { format!("t.TransactionNumber = '{term}'") } else { format!("upper(s.Refnum) = '{term}'") }
    );
    let session = Session::start(&cfg).await.map_err(registry_err)?;
    let rows = session.query(&hql, 200).await;
    session.end().await;
    let o = |s: &String| Some(s.trim().to_string()).filter(|s| !s.is_empty());
    let mut out = Vec::new();
    for r in rows.map_err(registry_err)?.iter().filter(|r| r.len() >= 6) {
        let job = sqlx::query("SELECT j.id, j.name FROM cadastre.wf_job_instance j JOIN cadastre.wf_job_status s ON s.code = j.status \
                               WHERE j.lrs_transaction_number = $1 AND s.is_open ORDER BY j.id DESC LIMIT 1")
            .bind(r[0].trim()).fetch_optional(&state.pool).await?;
        let cadastre_upins = match o(&r[3]) { Some(f) => digitised_upins(&state.pool, &f).await?, None => Vec::new() };
        out.push(RegistryFileDto {
            transaction_number: r[0].trim().to_string(), status_code: r[1].trim().to_string(), status: r[2].trim().to_string(),
            property_number: o(&r[3]), su_id: o(&r[4]), upin: o(&r[5]),
            job_instance_id: job.as_ref().map(|j| j.get("id")), job_name: job.and_then(|j| j.get("name")), cadastre_upins,
        });
    }
    Ok(Json(out))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct StartDigitisingRequest {
    pub transaction_number: String,
    /// The file number for the job's name (Job-PL…); read from the Registry when omitted.
    pub property_number: Option<String>,
    /// The job is for the title deed of a file already digitised (its parcel exists). Without it
    /// the job is for digitising, refused for a file already digitised: a re-survey goes through
    /// Correct, so the file keeps one UPIN and its history.
    #[serde(default)]
    pub title_deed: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct StartedJobDto {
    pub job_instance_id: i32,
    pub name: Option<String>,
    /// false: a job for the transaction was already open, and is returned.
    pub created: bool,
}

/// Start the job for a Registry file, as the Registry created them in Aumentum: job type
/// "Deed Plan Creationv2" (the Registry's job_deedplan_creation), named Job-<file number>, with
/// parameters @LRS:[Transaction]:[TransactionNumber]=PLS…. An open job of the transaction is
/// returned instead of a second one.
#[utoipa::path(post, path = "/api/v1/registry/start-digitising", tag = "registry", request_body = StartDigitisingRequest,
    responses((status = 200, body = StartedJobDto), (status = 404, description = "No such transaction"), (status = 409, description = "Transaction not Active")))]
pub async fn start_digitising(State(state): State<AppState>, user: AuthUser, Json(req): Json<StartDigitisingRequest>) -> ApiResult<Json<StartedJobDto>> {
    let actor = crate::routes::wf::resolve_actor(&state, &user).await?;
    let txn = ident("transaction number", &req.transaction_number)?.to_uppercase();
    if let Some(j) = sqlx::query("SELECT j.id, j.name FROM cadastre.wf_job_instance j JOIN cadastre.wf_job_status s ON s.code = j.status \
                                  WHERE j.lrs_transaction_number = $1 AND s.is_open ORDER BY j.id DESC LIMIT 1")
        .bind(&txn).fetch_optional(&state.pool).await? {
        return Ok(Json(StartedJobDto { job_instance_id: j.get("id"), name: j.get("name"), created: false }));
    }
    let cfg = config()?;
    let session = Session::start(&cfg).await.map_err(registry_err)?;
    let t = load_transaction(&session, &txn).await;
    session.end().await;
    let t = t?.ok_or_else(|| AppError::NotFound(format!("Cannot retrieve information of the Transaction. Transaction number:'{txn}'.")))?;
    if let Some(why) = &t.refused {
        return Err(AppError::Conflict(why.clone()));
    }
    let file = req.property_number.clone().filter(|s| !s.trim().is_empty()).or_else(|| t.properties.iter().find_map(|p| p.refnum.clone()));
    let digitised = match &file { Some(f) => digitised_upins(&state.pool, f).await?, None => Vec::new() };
    let registry_upins: Vec<String> = t.properties.iter().filter_map(|p| p.upin.clone()).filter(|u| !u.trim().is_empty()).collect();
    if !req.title_deed && (!digitised.is_empty() || !registry_upins.is_empty()) {
        let upins = if digitised.is_empty() { registry_upins } else { digitised };
        return Err(AppError::Conflict(format!(
            "{} is already digitised (UPIN {}). It is not digitised again: to change its parcel use Correct & Recommit (right-click the parcel, or Correct on its card), which keeps its UPIN and its history.",
            file.as_deref().unwrap_or(&txn), upins.join(", "))));
    }
    if req.title_deed && digitised.is_empty() {
        return Err(AppError::Conflict(format!("{} is not digitised in this Cadastre yet: there is no parcel for its title deed.", file.as_deref().unwrap_or(&txn))));
    }
    let (id, name) = create_file_job(&state.pool, &actor.label, &txn, file.as_deref(), "registry.start-digitising").await?;
    Ok(Json(StartedJobDto { job_instance_id: id, name: Some(name), created: true }))
}

/// A new file job, as the Registry created them in Aumentum: "Deed Plan Creationv2", named
/// Job-<file number>, parameters @LRS:[Transaction]:[TransactionNumber]=PLS…, with the file's
/// property number on its linkage (no LRSID: the Registry admin links it).
pub(crate) async fn create_file_job(pool: &sqlx::PgPool, actor: &str, txn: &str, file: Option<&str>, via: &str) -> ApiResult<(i32, String)> {
    let job_type: i32 = sqlx::query_scalar("SELECT id FROM cadastre.wf_job_type WHERE active AND lower(name) = lower($1) ORDER BY id LIMIT 1")
        .bind(registry_sync::setting(pool, "registry.job_deedplan_creation").await?.unwrap_or_else(|| "Deed Plan Creationv2".into()))
        .fetch_optional(pool).await?.ok_or_else(|| AppError::Conflict("the job type Deed Plan Creationv2 is not active".into()))?;
    let name = format!("Job-{}", file.unwrap_or(txn));
    let params = format!("@LRS:[Transaction]:[TransactionNumber]={txn}");
    let id = crate::routes::mcservice::create_job_instance(pool, job_type, Some(&name), Some("Deed Plan Creationv2"), Some(&params), "1", actor, via, true)
        .await.map_err(AppError::UnprocessableEntity)?;
    if let Some(f) = file {
        sqlx::query("INSERT INTO cadastre.wf_job_linkage (wf_job_instance_id, property_number, linkage_status) VALUES ($1, $2, 0) ON CONFLICT (wf_job_instance_id) DO NOTHING")
            .bind(id).bind(f).execute(pool).await?;
    }
    Ok((id, name))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ParcelWorkDto {
    pub upin: String,
    /// Linked to a Registry property (LinkageStatus 1).
    pub linked: bool,
    pub su_id: Option<String>,
    pub property_number: Option<String>,
    /// The Registry transaction of the link (from the link the Registry made), if known.
    pub transaction_number: Option<String>,
    /// The open job serving that transaction, if any.
    pub job_instance_id: Option<i32>,
    pub job_name: Option<String>,
    pub current_task: Option<String>,
}

/// What a parcel's title deed work needs: its Registry link and the job of the transaction
/// (the map's Start edit).
#[utoipa::path(get, path = "/api/v1/registry/parcels/{upin}/work", tag = "registry", params(("upin" = String, Path, description = "UPIN")),
    responses((status = 200, body = ParcelWorkDto)))]
pub async fn parcel_work(State(state): State<AppState>, Path(upin): Path<String>) -> ApiResult<Json<ParcelWorkDto>> {
    let upin = ident("UPIN", &upin)?.to_uppercase();
    let a = sqlx::query("SELECT lrs_id, property_number, linkage_status FROM cadastre.parcel_attribute WHERE upin = $1 ORDER BY (status = '0') DESC NULLS LAST, object_id DESC LIMIT 1")
        .bind(&upin).fetch_optional(&state.pool).await?;
    let (su, pn, linked) = match &a {
        Some(r) => (r.get::<Option<String>, _>("lrs_id"), r.get::<Option<String>, _>("property_number"), r.get::<Option<i16>, _>("linkage_status") == Some(1)),
        None => (None, None, false),
    };
    // The transaction: the link the Registry made (sync events), else a job that committed the parcel.
    let txn: Option<String> = sqlx::query_scalar(
        "SELECT payload->>'transaction_number' FROM cadastre.audit_log WHERE entity = 'registry_link' AND entity_id = $1 \
         AND action IN ('registry_link_synced', 'registry_link_made') AND payload->>'transaction_number' <> '' ORDER BY id DESC LIMIT 1",
    ).bind(&upin).fetch_optional(&state.pool).await?;
    let txn = match txn {
        Some(t) => Some(t),
        None => sqlx::query_scalar("SELECT w.lrs_transaction_number FROM cadastre.parcel p JOIN cadastre.wf_job_instance w ON w.cadastre_job_id = p.job_id \
                                    WHERE p.parcel_number = $1 AND w.lrs_transaction_number IS NOT NULL ORDER BY w.id DESC LIMIT 1")
            .bind(&upin).fetch_optional(&state.pool).await?.flatten(),
    };
    let mut job = match &txn {
        Some(t) => sqlx::query("SELECT j.id, j.name, tk.name AS task FROM cadastre.wf_job_instance j JOIN cadastre.wf_job_status s ON s.code = j.status \
                                LEFT JOIN cadastre.wf_task tk ON tk.id = j.current_task_id WHERE j.lrs_transaction_number = $1 AND s.is_open ORDER BY j.id DESC LIMIT 1")
            .bind(t).fetch_optional(&state.pool).await?,
        None => None,
    };
    // Fallback: links that reached the attributes without a sync event (Aumentum-era links) carry
    // no transaction. An open job for the parcel's file still serves it: found by its linkage
    // (property number / LRSID) or by its name Job-<file number> / Job-<UPIN>, and its
    // transaction is the parcel's. A closed job gives the transaction too (a file back after its
    // title deed has only its completed job), but it is not a job to work on.
    let mut txn = txn;
    if txn.is_none() {
        let found = sqlx::query(
            "SELECT j.id, j.name, tk.name AS task, j.lrs_transaction_number, s.is_open FROM cadastre.wf_job_instance j \
             JOIN cadastre.wf_job_status s ON s.code = j.status \
             LEFT JOIN cadastre.wf_job_linkage l ON l.wf_job_instance_id = j.id \
             LEFT JOIN cadastre.wf_task tk ON tk.id = j.current_task_id \
             WHERE j.lrs_transaction_number IS NOT NULL AND j.lrs_transaction_number <> '' \
               AND ((NULLIF($2, '') IS NOT NULL AND upper(l.property_number) = upper($2)) \
                 OR (NULLIF($3, '') IS NOT NULL AND upper(l.lrs_id) = upper($3)) \
                 OR upper(j.name) = upper('Job-' || $1) \
                 OR (NULLIF($2, '') IS NOT NULL AND upper(j.name) = upper('Job-' || $2))) \
             ORDER BY s.is_open DESC, j.id DESC LIMIT 1",
        )
        .bind(&upin).bind(&pn).bind(&su)
        .fetch_optional(&state.pool).await?;
        txn = found.as_ref().and_then(|j| j.get::<Option<String>, _>("lrs_transaction_number"));
        job = found.filter(|j| j.get::<bool, _>("is_open"));
    }
    Ok(Json(ParcelWorkDto {
        upin, linked, su_id: su, property_number: pn, transaction_number: txn,
        job_instance_id: job.as_ref().map(|j| j.get("id")), job_name: job.as_ref().and_then(|j| j.get("name")), current_task: job.and_then(|j| j.get("task")),
    }))
}

/// Linking is done by the Registry admin in the Registry (PLAGIS, 2026-11-02); the Cadastre's own
/// link / unlink (Aumentum's desktop LinkParcel) stays off unless CADASTRE_REGISTRY_CADASTRE_LINKING=1.
fn cadastre_linking_allowed() -> ApiResult<()> {
    if std::env::var("CADASTRE_REGISTRY_CADASTRE_LINKING").map(|v| v.trim() == "1").unwrap_or(false) {
        Ok(())
    } else {
        Err(AppError::Conflict("Linking is done by the Registry admin in the Registry, not in the Cadastre.".into()))
    }
}

#[cfg(test)]
mod send_plan_tests {
    use super::{corrected_comments, plan_as_pdf};

    #[test]
    fn the_registry_always_gets_a_pdf() {
        // A PDF goes as it is.
        assert_eq!(corrected_comments("05-11-2026 10:00", None), "Survey Plan (corrected, 05-11-2026 10:00)");
        assert_eq!(corrected_comments("05-11-2026 10:00", Some("Wrong scale")), "Survey Plan (corrected, 05-11-2026 10:00): Wrong scale");
        assert_eq!(corrected_comments("05-11-2026 10:00", Some(&"é".repeat(300))).chars().count(), 240);
        let (n, b) = plan_as_pdf("Title Deed Plan PL67363.pdf", b"%PDF-1.4 x %%EOF").unwrap();
        assert_eq!((n.as_str(), b.as_slice()), ("Title Deed Plan PL67363.pdf", b"%PDF-1.4 x %%EOF".as_slice()));
        // A JPG becomes a PDF under the same name.
        let jpg = [0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x0B, 0x08, 0x00, 0x03, 0x00, 0x02, 0x01, 0x01, 0x11, 0x00, 0xFF, 0xD9];
        let (n, b) = plan_as_pdf("Title Deed Plan PL67363.JPG", &jpg).unwrap();
        assert_eq!(n, "Title Deed Plan PL67363.pdf");
        assert!(b.starts_with(b"%PDF-") && b.ends_with(b"%%EOF"));
        // Not a PDF in disguise, not a PNG, not a broken JPG.
        assert!(plan_as_pdf("plan.pdf", b"\x89PNG").is_err());
        assert!(plan_as_pdf("plan.png", b"\x89PNG").is_err());
        assert!(plan_as_pdf("plan.jpg", b"not a jpeg").is_err());
    }
}

#[cfg(test)]
mod map_ok_tests {
    use super::only_active_transaction;

    fn row(number: &str, code: &str) -> Vec<String> {
        vec!["1".into(), number.into(), code.into(), "x".into(), "2".into(), "".into()]
    }

    #[test]
    fn the_map_ok_writes_only_to_the_one_active_transaction() {
        let closed = row("PLS1", "transaction_completed");
        let active = row("PLS2", "transaction_lodged");
        assert_eq!(only_active_transaction(&[closed.clone(), active.clone()]), Ok("PLS2".to_string()));
        // The same file twice (a property's two rows) is still one file.
        assert_eq!(only_active_transaction(&[active.clone(), active.clone()]), Ok("PLS2".to_string()));
        assert!(only_active_transaction(&[closed]).is_err(), "nothing Active: nothing to write to");
        assert!(only_active_transaction(&[active, row("PLS3", "transaction_lodged")]).is_err(), "two Active: not guessed at");
        assert!(only_active_transaction(&[]).is_err());
    }
}
