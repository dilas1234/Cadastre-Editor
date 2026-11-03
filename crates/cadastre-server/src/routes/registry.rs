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

fn config() -> ApiResult<LrsConfig> {
    LrsConfig::from_env().ok_or_else(|| AppError::Conflict("the Registry connection is not configured on this server".into()))
}

fn write_config() -> ApiResult<LrsConfig> {
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
fn ident(kind: &str, v: &str) -> ApiResult<String> {
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
            "select s.Id, s.SuID, s.Upin, s.Refnum, s.LotNumber, s.BlockNumber, s.SpatialDate1, z.Label, y.Label from com.landsystems.lrs.model.Transaction t \
             join t.Properties l join l.SpatialUnits s left join s.Zone z left join s.Layout y \
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
            location: r.get(7).and_then(o), layout: r.get(8).and_then(o),
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
    let cfg = write_config()?;
    let posted: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM cadastre.parcel WHERE parcel_number = $1 AND status = 'posted')").bind(&upin).fetch_one(&state.pool).await?;
    if !posted {
        return Err(AppError::BadRequest(format!("{upin} is not a posted parcel")));
    }
    let linked: Option<String> = sqlx::query_scalar("SELECT lrs_id FROM cadastre.parcel_attribute WHERE upin = $1 AND linkage_status = 1 AND (status = '0' OR status IS NULL) LIMIT 1")
        .bind(&upin).fetch_optional(&state.pool).await?.flatten();
    if let Some(p) = linked {
        return Err(AppError::Conflict(format!("The parcel {upin} is already linked to the Registry property {p}.")));
    }
    let session = Session::start(&cfg).await.map_err(registry_err)?;
    let result = async {
        let t = load_transaction(&session, &number).await?
            .ok_or_else(|| AppError::NotFound(format!("Cannot retrieve information of the Transaction. Transaction number:'{number}'.")))?;
        if let Some(why) = &t.refused {
            return Err(AppError::Conflict(why.clone()));
        }
        let prop = t.properties.iter().find(|p| p.su_id.eq_ignore_ascii_case(&su)).cloned()
            .ok_or_else(|| AppError::BadRequest(format!("{su} is not a property of {number}")))?;
        if let Some(other) = prop.upin.as_deref().filter(|u| !u.eq_ignore_ascii_case(&upin)) {
            return Err(AppError::Conflict(format!("The Registry property {su} is already linked to {other}.")));
        }
        let settings = registry_sync::registry_settings(&session).await.map_err(registry_err)?;
        let (to_registry, to_parcel) = registry::parse_linkage_fields(settings.get("LinkageFields").map(String::as_str).unwrap_or("upin:UPIN;LRSID:suid,PropertyNumber:refnum,Plot:plotNumber,Block:blockNumber"));
        // The Registry side first (Aumentum: SaveProperty per LinkageFields entry).
        for m in &to_registry {
            let value = if m.target.eq_ignore_ascii_case("UPIN") { Some(upin.as_str()) } else { None };
            let Some(v) = value else { continue };
            session.save_entity(t.id, &lrs::entity_xml("Parcel", &[("Id", Some(&prop.id)), (&m.source, Some(v))])).await.map_err(registry_err)?;
        }
        let cols = [("SuID", Some(prop.su_id.clone())), ("Refnum", prop.refnum.clone()), ("LotNumber", prop.lot.clone()), ("BlockNumber", prop.block.clone()), ("Upin", Some(upin.clone())), ("Id", Some(prop.id.clone()))];
        let updates = registry::cadastre_link_updates(&to_parcel, &cols, chrono::Utc::now().date_naive());
        Ok::<_, AppError>((t, prop, updates))
    }
    .await;
    session.end().await;
    let (t, prop, updates) = result?;
    let mut tx = state.pool.begin().await?;
    ensure_attribute_record(&mut tx, &upin).await?;
    registry_sync::write_attributes(&mut tx, &upin, &updates).await?;
    sqlx::query("INSERT INTO cadastre.registry_property (su_id, registry_id, upin, refnum) VALUES ($1, $2, $3, $4) ON CONFLICT (su_id) DO UPDATE SET upin = EXCLUDED.upin, registry_id = EXCLUDED.registry_id, refnum = EXCLUDED.refnum, seen_at = now()")
        .bind(&prop.su_id).bind(prop.id.parse::<i64>().ok()).bind(&upin).bind(&prop.refnum).execute(&mut *tx).await?;
    audit::append(&mut tx, &actor.label, "registry_link_made", "registry_link", Some(&upin), serde_json::json!({
        "su_id": prop.su_id, "transaction_number": t.transaction_number, "property_number": prop.refnum, "written": updates,
        "linked_at": chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S").to_string(), "transaction_status": t.status, "transaction_status_code": t.status_code,
    })).await?;
    tx.commit().await?;
    Ok(Json(RegistryPropertyDto { upin: Some(upin), ..prop }))
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
/// sent before are marked as replaced (their Comments); nothing in the Registry is deleted.
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
        "SELECT max(created_at) FROM cadastre.audit_log WHERE entity = 'wf_job_instance' AND entity_id = $1 AND action = 'registry_plan_sent' AND payload->>'transaction_number' = $2")
        .bind(id.to_string()).bind(&txn).fetch_one(&state.pool).await?;
    if let (Some(at), false) = (sent_before, req.resend) {
        return Err(AppError::Conflict(format!("the plan was already sent to {txn} ({} UTC). To correct it, use Resend corrected plan.", at.format("%Y-%m-%d %H:%M"))));
    }
    let (name, _mime, bytes, job) = crate::routes::wfadmin::read_job_file(&state.pool, req.file_id).await?;
    if job != id {
        return Err(AppError::BadRequest("that file is not attached to this job".into()));
    }
    let (sent_name, pdf) = plan_as_pdf(&name, &bytes)?;
    let doc_type = registry_sync::setting(&state.pool, "aumentum.Integration.SurveyPlanFromMC").await?.unwrap_or_else(|| "sd_titledeedPlan".into());
    let page_type = registry_sync::setting(&state.pool, "aumentum.Integration.PlanPageType").await?.unwrap_or_else(|| "attachment".into());
    let resend = req.resend && sent_before.is_some();
    let now = chrono::Local::now();
    let comments = if resend { format!("Survey Plan (corrected, {})", now.format("%d-%m-%Y %H:%M")) } else { "Survey Plan".to_string() };
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
    })).await?;
    tx.commit().await?;
    Ok(Json(serde_json::json!({ "transaction_number": t.transaction_number, "file_name": sent_name, "document_type": doc_type,
        "resend": resend, "replaced": replaced, "replace_note": replace_note })))
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
        out.push(RegistryFileDto {
            transaction_number: r[0].trim().to_string(), status_code: r[1].trim().to_string(), status: r[2].trim().to_string(),
            property_number: o(&r[3]), su_id: o(&r[4]), upin: o(&r[5]),
            job_instance_id: job.as_ref().map(|j| j.get("id")), job_name: job.and_then(|j| j.get("name")),
        });
    }
    Ok(Json(out))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct StartDigitisingRequest {
    pub transaction_number: String,
    /// The file number for the job's name (Job-PL…); read from the Registry when omitted.
    pub property_number: Option<String>,
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
    let job_type: i32 = sqlx::query_scalar("SELECT id FROM cadastre.wf_job_type WHERE active AND lower(name) = lower($1) ORDER BY id LIMIT 1")
        .bind(registry_sync::setting(&state.pool, "registry.job_deedplan_creation").await?.unwrap_or_else(|| "Deed Plan Creationv2".into()))
        .fetch_optional(&state.pool).await?.ok_or_else(|| AppError::Conflict("the job type Deed Plan Creationv2 is not active".into()))?;
    let name = format!("Job-{}", file.as_deref().unwrap_or(&txn));
    let params = format!("@LRS:[Transaction]:[TransactionNumber]={txn}");
    let id = crate::routes::mcservice::create_job_instance(&state.pool, job_type, Some(&name), Some("Deed Plan Creationv2"), Some(&params), "1", &actor.label, "registry.start-digitising", true)
        .await.map_err(AppError::UnprocessableEntity)?;
    // The file's property number is what the parcels of this job carry (Update Parcel Attribute
    // defaults to it). No LRSID: the parcel is linked only when the Registry admin links it.
    if let Some(f) = &file {
        sqlx::query("INSERT INTO cadastre.wf_job_linkage (wf_job_instance_id, property_number, linkage_status) VALUES ($1, $2, 0) ON CONFLICT (wf_job_instance_id) DO NOTHING")
            .bind(id).bind(f).execute(&state.pool).await?;
    }
    Ok(Json(StartedJobDto { job_instance_id: id, name: Some(name), created: true }))
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
    // transaction is the parcel's.
    let mut txn = txn;
    if txn.is_none() {
        job = sqlx::query(
            "SELECT j.id, j.name, tk.name AS task, j.lrs_transaction_number FROM cadastre.wf_job_instance j \
             JOIN cadastre.wf_job_status s ON s.code = j.status \
             LEFT JOIN cadastre.wf_job_linkage l ON l.wf_job_instance_id = j.id \
             LEFT JOIN cadastre.wf_task tk ON tk.id = j.current_task_id \
             WHERE s.is_open AND j.lrs_transaction_number IS NOT NULL AND j.lrs_transaction_number <> '' \
               AND ((NULLIF($2, '') IS NOT NULL AND upper(l.property_number) = upper($2)) \
                 OR (NULLIF($3, '') IS NOT NULL AND upper(l.lrs_id) = upper($3)) \
                 OR upper(j.name) = upper('Job-' || $1) \
                 OR (NULLIF($2, '') IS NOT NULL AND upper(j.name) = upper('Job-' || $2))) \
             ORDER BY j.id DESC LIMIT 1",
        )
        .bind(&upin).bind(&pn).bind(&su)
        .fetch_optional(&state.pool).await?;
        txn = job.as_ref().and_then(|j| j.get::<Option<String>, _>("lrs_transaction_number"));
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
    use super::plan_as_pdf;

    #[test]
    fn the_registry_always_gets_a_pdf() {
        // A PDF goes as it is.
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
