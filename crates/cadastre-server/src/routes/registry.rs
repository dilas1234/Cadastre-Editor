//! "Linked from the Registry": the parcels the Registry admin has linked to a Registry property,
//! as the background sync (`registry_sync`) records them, with each parcel's place in the fabric
//! and the Cadastre job (if any) serving its transaction.

use axum::extract::{Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::auth::AuthUser;
use crate::error::{ApiResult, AppError};
use crate::lrs::LrsConfig;
use crate::registry_sync::{self, SyncReport};
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
}

/// Parcels linked in the Registry, newest first.
#[utoipa::path(get, path = "/api/v1/registry/links", tag = "registry", params(LinksQuery),
    responses((status = 200, body = [RegistryLinkDto])))]
pub async fn links(State(state): State<AppState>, Query(q): Query<LinksQuery>) -> ApiResult<Json<Vec<RegistryLinkDto>>> {
    let days = q.days.unwrap_or(14).clamp(1, 365);
    let rows = sqlx::query(
        "SELECT a.entity_id AS upin, a.payload, a.created_at, \
                (SELECT p.id FROM cadastre.parcel p WHERE p.parcel_number = a.entity_id AND p.status = 'posted' LIMIT 1) AS parcel_id, \
                j.id AS job_id, j.name AS job_name, j.status AS job_status \
         FROM cadastre.audit_log a \
         LEFT JOIN LATERAL (SELECT w.id, w.name, w.status FROM cadastre.wf_job_instance w \
                            WHERE w.lrs_transaction_number = a.payload->>'transaction_number' ORDER BY w.id DESC LIMIT 1) j ON true \
         WHERE a.entity = 'registry_link' AND a.action = 'registry_link_synced' \
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
        }
    }).collect()))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RegistryStatusDto {
    /// The Registry sync is configured on this server.
    pub enabled: bool,
    pub url: Option<String>,
    /// The last check: `{ ok, at, report | error }`.
    #[schema(value_type = Object)]
    pub last: Option<serde_json::Value>,
}

/// Whether the Registry sync runs, and how its last check went.
#[utoipa::path(get, path = "/api/v1/registry/status", tag = "registry", responses((status = 200, body = RegistryStatusDto)))]
pub async fn status(State(state): State<AppState>) -> ApiResult<Json<RegistryStatusDto>> {
    let cfg = LrsConfig::from_env();
    Ok(Json(RegistryStatusDto { enabled: cfg.is_some(), url: cfg.map(|c| c.base_url), last: registry_sync::last_status(&state.pool).await }))
}

/// Check the Registry for new links now (instead of waiting for the next minute).
#[utoipa::path(post, path = "/api/v1/registry/sync", tag = "registry",
    responses((status = 200, body = SyncReport), (status = 409, description = "The Registry is not configured"), (status = 422, description = "The Registry could not be read")))]
pub async fn sync_now(State(state): State<AppState>, _user: AuthUser) -> ApiResult<Json<SyncReport>> {
    let cfg = LrsConfig::from_env().ok_or_else(|| AppError::Conflict("the Registry connection is not configured on this server".into()))?;
    registry_sync::sync_and_record(&state.pool, &cfg).await.map(Json).map_err(AppError::UnprocessableEntity)
}
