//! Registry → Cadastre link sync (Phase 5). The Registry (LRS43) is only READ here.
//!
//! **Links.** When the Registry admin links a property to a Cadastre parcel, the Registry stamps
//! the property's `SpatialDate1` and holds the parcel's UPIN. Every `CADASTRE_REGISTRY_POLL_SECS`
//! (default 60) the properties linked since the last check are read, through the same HQL
//! interface Aumentum used, and written onto the parcel's attribute record as the Registry's own
//! `mcintegration` setting `GISUpdatedFields` says, read live each run. On PRODUCTIONMC:
//! `suID:LRSID,1:LinkageStatus,spatialDate1:LinkageDate,refnum:PropertyNumber,plotNumber:Plot,
//! blockNumber:Block`, resolved as Aumentum resolved it (`registry::resolve`: names it does not
//! resolve are written as text, which is how 6,496 Aumentum records came to hold Plot =
//! "plotNumber"). Change the setting in the Registry and this follows.
//!
//! **Unlinks.** Every `CADASTRE_REGISTRY_RECONCILE_SECS` (default 900) the Registry's current
//! properties holding a UPIN are snapshotted (`cadastre.registry_property`). A property whose
//! UPIN changed or went since the last snapshot, while the Cadastre parcel it held is linked to
//! it, is an unlink made in the Registry: LinkageStatus 0, the other link fields kept, as
//! Aumentum's 18 unlinked records show. The first snapshot is a baseline (no events).
//!
//! Each change is one audit event (entity `registry_link`, id = the UPIN): `registry_link_synced`
//! or `registry_unlink_synced`. Re-reading the same state changes nothing.

use std::collections::{BTreeMap, HashMap};

use chrono::{Duration, NaiveDateTime, Utc};
use serde_json::json;
use sqlx::{PgPool, Row};

use crate::audit;
use crate::lrs::{parse_datetime, LrsConfig, Session};
use crate::registry::{self, FieldMap, RegistryProperty};

const WATERMARK_KEY: &str = "registry.sync.since";
const STATUS_KEY: &str = "registry.sync.status";
const RECONCILE_KEY: &str = "registry.reconcile.status";
/// Links are re-read this far back from the newest one seen.
const OVERLAP_HOURS: i64 = 2;
/// GISUpdatedFields as on PRODUCTIONMC, used only if the Registry setting cannot be read.
pub const DEFAULT_UPDATED_FIELDS: &str = "suID:LRSID,1:LinkageStatus,spatialDate1:LinkageDate,refnum:PropertyNumber,plotNumber:Plot,blockNumber:Block";

/// One Registry property linked to a UPIN.
#[derive(Clone, Debug, PartialEq)]
pub struct RegistryLink {
    pub su_id: String,
    pub upin: String,
    pub refnum: Option<String>,
    pub linked_at: NaiveDateTime,
    pub transaction_number: String,
    pub transaction_status_code: String,
    pub transaction_status: String,
    pub lot: Option<String>,
    pub block: Option<String>,
}

/// The HQL: current versions of properties with a UPIN, linked (SpatialDate1) since `since`.
pub fn links_query(since: NaiveDateTime) -> String {
    format!(
        "select s.SuID, s.Upin, s.Refnum, s.SpatialDate1, t.TransactionNumber, ts.Code, ts.Label, s.LotNumber, s.BlockNumber \
         from com.landsystems.lrs.model.Transaction t join t.TransactionStatus ts join t.Properties l join l.SpatialUnits s \
         where s.SpatialDate1 >= '{}' and s.Upin is not null and l.EndLifespanVersion is null and s.EndLifespanVersion is null \
         order by s.SpatialDate1",
        since.format("%Y-%m-%d %H:%M:%S")
    )
}

/// Aumentum's A_MCINTEGRATION query: the Registry's Cadastre integration settings.
pub const SETTINGS_QUERY: &str = "select s.Name, s.Value from com.landsystems.lrs.model.Setting s where s.Category = 'mcintegration'";

/// Every current property holding a UPIN (the reconcile snapshot).
pub const SNAPSHOT_QUERY: &str = "select s.Id, s.SuID, s.Upin, s.Refnum from com.landsystems.lrs.model.SpatialUnit s \
    where s.Upin is not null and s.Upin <> '' and s.EndLifespanVersion is null";

fn opt(s: &str) -> Option<String> {
    Some(s.trim().to_string()).filter(|s| !s.is_empty())
}

/// Rows of [`links_query`] → one link per property. A property in several current transactions
/// is shown with its Active (lodged) one, else the first.
pub fn links_from_rows(rows: &[Vec<String>]) -> Vec<RegistryLink> {
    let mut by: BTreeMap<String, RegistryLink> = BTreeMap::new();
    for r in rows {
        if r.len() < 9 {
            continue;
        }
        let (Some(su), Some(upin), Some(at)) = (opt(&r[0]), opt(&r[1]), parse_datetime(&r[3])) else { continue };
        let link = RegistryLink {
            su_id: su.clone(),
            upin: upin.trim().to_uppercase(),
            refnum: opt(&r[2]),
            linked_at: at,
            transaction_number: r[4].trim().to_string(),
            transaction_status_code: r[5].trim().to_string(),
            transaction_status: r[6].trim().to_string(),
            lot: opt(&r[7]),
            block: opt(&r[8]),
        };
        match by.get(&su) {
            Some(prev) if prev.transaction_status_code == "transaction_lodged" => {}
            _ => {
                by.insert(su, link);
            }
        }
    }
    let mut v: Vec<RegistryLink> = by.into_values().collect();
    v.sort_by_key(|l| l.linked_at);
    v
}

/// Aumentum ParcelAttributeTable field → `cadastre.parcel_attribute` column (and whether it is
/// the smallint LinkageStatus).
pub fn attribute_column(field: &str) -> Option<(&'static str, bool)> {
    Some(match field.to_ascii_lowercase().as_str() {
        "lrsid" => ("lrs_id", false),
        "linkagestatus" => ("linkage_status", true),
        "linkagedate" => ("linkage_date", false),
        "propertynumber" => ("property_number", false),
        "plot" => ("plot", false),
        "block" => ("block", false),
        "parcelname" => ("parcel_name", false),
        "location" => ("location", false),
        "plannumber" => ("plan_number", false),
        _ => return None,
    })
}

/// Write `updates` (Aumentum field, value) onto the active attribute record(s) of `upin`, only
/// where a value differs. `None` values are written as NULL. Returns records changed.
pub async fn write_attributes(conn: &mut sqlx::PgConnection, upin: &str, updates: &[(String, Option<String>)]) -> Result<u64, sqlx::Error> {
    let cols: Vec<(&'static str, bool, Option<String>)> =
        updates.iter().filter_map(|(f, v)| attribute_column(f).map(|(c, small)| (c, small, v.clone()))).collect();
    if cols.is_empty() {
        return Ok(0);
    }
    let mut qb = sqlx::QueryBuilder::<sqlx::Postgres>::new("UPDATE cadastre.parcel_attribute SET last_edit_date = now()::timestamp, updated_at = now()");
    for (c, small, v) in &cols {
        qb.push(format!(", {c} = "));
        if *small { qb.push_bind(v.as_ref().and_then(|s| s.trim().parse::<i16>().ok())); } else { qb.push_bind(v.clone()); }
    }
    qb.push(" WHERE upin = ").push_bind(upin.to_string()).push(" AND (status = '0' OR status IS NULL) AND (false");
    for (c, small, v) in &cols {
        qb.push(format!(" OR {c} IS DISTINCT FROM "));
        if *small { qb.push_bind(v.as_ref().and_then(|s| s.trim().parse::<i16>().ok())); } else { qb.push_bind(v.clone()); }
    }
    qb.push(")");
    Ok(qb.build().execute(conn).await?.rows_affected())
}

#[derive(Debug, Default, serde::Serialize, utoipa::ToSchema)]
pub struct SyncReport {
    /// Links read from the Registry this run (the overlap window included).
    pub read: usize,
    /// Links not seen before (new events).
    pub new: usize,
    /// Parcel attribute records changed.
    pub updated: usize,
    /// New links whose UPIN has no attribute record or no parcel in the fabric yet.
    pub unmatched: usize,
    pub since: String,
    pub clock_offset_secs: i64,
    /// The GISUpdatedFields mapping used.
    pub updated_fields: String,
}

#[derive(Debug, Default, serde::Serialize, utoipa::ToSchema)]
pub struct ReconcileReport {
    /// Properties holding a UPIN in the Registry now.
    pub registry_properties: usize,
    /// Unlinks found (parcels set to LinkageStatus 0).
    pub unlinked: usize,
    /// This run only recorded the baseline snapshot.
    pub baseline: bool,
    /// Parcels found linked in the Registry (the property holds their UPIN, typed or carried over,
    /// with or without a link date) that the Cadastre did not know as linked: now marked linked.
    #[serde(default)]
    pub recognised: usize,
}

pub async fn setting(pool: &PgPool, key: &str) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT value FROM cadastre.app_setting WHERE key = $1").bind(key).fetch_optional(pool).await.map(Option::flatten)
}

async fn set_setting(conn: &mut sqlx::PgConnection, key: &str, value: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO cadastre.app_setting (key, value, source, updated_by) VALUES ($1, $2, 'cadastre', 'registry-sync') \
         ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_by = EXCLUDED.updated_by, updated_at = now()",
    )
    .bind(key)
    .bind(value)
    .execute(conn)
    .await
    .map(|_| ())
}

/// Where to start: the stored watermark, else the newest LinkageDate imported from Aumentum (so
/// every link made after the import is picked up), else 7 days back.
async fn since(pool: &PgPool) -> Result<NaiveDateTime, sqlx::Error> {
    if let Some(w) = setting(pool, WATERMARK_KEY).await?.and_then(|w| NaiveDateTime::parse_from_str(&w, "%Y-%m-%d %H:%M:%S").ok()) {
        return Ok(w - Duration::hours(OVERLAP_HOURS));
    }
    let last: Option<String> = sqlx::query_scalar(
        "SELECT max(linkage_date) FROM cadastre.parcel_attribute WHERE linkage_status = 1 AND linkage_date ~ '^\\d{4}-\\d{2}-\\d{2}$'",
    )
    .fetch_one(pool)
    .await?;
    Ok(last
        .and_then(|d| chrono::NaiveDate::parse_from_str(&d, "%Y-%m-%d").ok())
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .unwrap_or_else(|| (Utc::now() - Duration::days(7)).naive_utc()))
}

/// Apply links to the Cadastre (attribute records per `updated_fields`, events, snapshot).
/// Pure database work, tested without a Registry.
pub async fn apply(pool: &PgPool, links: &[RegistryLink], updated_fields: &[FieldMap]) -> Result<(usize, usize, usize), sqlx::Error> {
    let (mut new, mut updated, mut unmatched) = (0, 0, 0);
    let mut tx = pool.begin().await?;
    for l in links {
        let prop = RegistryProperty { su_id: l.su_id.clone(), refnum: l.refnum.clone(), spatial_date1: Some(l.linked_at), upin: Some(l.upin.clone()) };
        // Empty Registry values never blank a Cadastre field.
        let updates: Vec<(String, Option<String>)> = registry::link_updates(updated_fields, &prop).into_iter().filter(|(_, v)| v.is_some()).collect();
        updated += write_attributes(&mut tx, &l.upin, &updates).await? as usize;
        sqlx::query(
            "INSERT INTO cadastre.registry_property (su_id, upin, refnum) VALUES ($1, $2, $3) \
             ON CONFLICT (su_id) DO UPDATE SET upin = EXCLUDED.upin, refnum = EXCLUDED.refnum, seen_at = now()",
        )
        .bind(&l.su_id)
        .bind(&l.upin)
        .bind(&l.refnum)
        .execute(&mut *tx)
        .await?;
        let at = l.linked_at.format("%Y-%m-%dT%H:%M:%S").to_string();
        let seen: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM cadastre.audit_log WHERE entity = 'registry_link' AND entity_id = $1 \
             AND action = 'registry_link_synced' AND payload->>'su_id' = $2 AND payload->>'linked_at' = $3)",
        )
        .bind(&l.upin)
        .bind(&l.su_id)
        .bind(&at)
        .fetch_one(&mut *tx)
        .await?;
        if seen {
            continue;
        }
        let (has_attr, in_fabric): (bool, bool) = sqlx::query_as(
            "SELECT EXISTS (SELECT 1 FROM cadastre.parcel_attribute WHERE upin = $1), \
                    EXISTS (SELECT 1 FROM cadastre.parcel WHERE parcel_number = $1 AND status = 'posted')",
        )
        .bind(&l.upin)
        .fetch_one(&mut *tx)
        .await?;
        if !(has_attr && in_fabric) {
            unmatched += 1;
        }
        audit::append(&mut tx, "registry-sync", "registry_link_synced", "registry_link", Some(&l.upin), json!({
            "su_id": l.su_id, "property_number": l.refnum, "linked_at": at,
            "transaction_number": l.transaction_number, "transaction_status_code": l.transaction_status_code,
            "transaction_status": l.transaction_status, "lot": l.lot, "block": l.block,
            "attribute_record": has_attr, "in_fabric": in_fabric, "written": updates,
        }))
        .await?;
        new += 1;
    }
    if let Some(max) = links.iter().map(|l| l.linked_at).max() {
        let current = setting(pool, WATERMARK_KEY).await?.and_then(|w| NaiveDateTime::parse_from_str(&w, "%Y-%m-%d %H:%M:%S").ok());
        if current.is_none_or(|c| max > c) {
            set_setting(&mut tx, WATERMARK_KEY, &max.format("%Y-%m-%d %H:%M:%S").to_string()).await?;
        }
    }
    tx.commit().await?;
    Ok((new, updated, unmatched))
}

/// The Registry's mcintegration settings (name → value).
pub async fn registry_settings(session: &Session) -> Result<HashMap<String, String>, String> {
    let rows = session.query(SETTINGS_QUERY, 200).await.map_err(|e| e.to_string())?;
    Ok(rows.into_iter().filter(|r| r.len() >= 2).map(|r| (r[0].clone(), r[1].clone())).collect())
}

/// One check: read new links from the Registry and apply them.
pub async fn sync_once(pool: &PgPool, cfg: &LrsConfig) -> Result<SyncReport, String> {
    let from = since(pool).await.map_err(|e| e.to_string())?;
    let session = Session::start(cfg).await.map_err(|e| e.to_string())?;
    let offset = session.clock_offset_secs();
    let settings = registry_settings(&session).await;
    let rows = session.query(&links_query(from), 5000).await;
    session.end().await;
    let fields_setting = settings.ok().and_then(|s| s.get("GISUpdatedFields").cloned()).filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_UPDATED_FIELDS.to_string());
    let fields = registry::parse_field_map(&fields_setting);
    let links = links_from_rows(&rows.map_err(|e| e.to_string())?);
    let (new, updated, unmatched) = apply(pool, &links, &fields).await.map_err(|e| e.to_string())?;
    Ok(SyncReport { read: links.len(), new, updated, unmatched, since: from.format("%Y-%m-%d %H:%M:%S").to_string(), clock_offset_secs: offset, updated_fields: fields_setting })
}

/// Compare a fresh snapshot (su_id → (registry id, upin, refnum)) with the stored one; record
/// unlinks; replace the snapshot. Pure database work, tested without a Registry.
pub async fn reconcile_with(pool: &PgPool, fresh: &HashMap<String, (Option<i64>, String, Option<String>)>) -> Result<ReconcileReport, String> {
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    let old: Vec<(String, String)> = sqlx::query_as("SELECT su_id, upin FROM cadastre.registry_property").fetch_all(&mut *tx).await.map_err(|e| e.to_string())?;
    let baseline = old.is_empty();
    // A Registry hiccup (a short answer) must not unlink parcels.
    if !baseline && fresh.len() * 2 < old.len() {
        return Err(format!("the Registry returned {} linked properties where {} were seen before; reconcile skipped", fresh.len(), old.len()));
    }
    let mut unlinked = 0;
    if !baseline {
        for (su, old_upin) in &old {
            let now = fresh.get(su).map(|f| f.1.as_str());
            if now == Some(old_upin.as_str()) {
                continue;
            }
            // The parcel held before: still linked to this property in the Cadastre?
            let n = sqlx::query(
                "UPDATE cadastre.parcel_attribute SET linkage_status = 0, last_edit_date = now()::timestamp, updated_at = now() \
                 WHERE upin = $1 AND lrs_id = $2 AND linkage_status = 1 AND (status = '0' OR status IS NULL)",
            )
            .bind(old_upin)
            .bind(su)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?
            .rows_affected();
            if n > 0 {
                unlinked += 1;
                audit::append(&mut tx, "registry-sync", "registry_unlink_synced", "registry_link", Some(old_upin), json!({
                    "su_id": su, "registry_upin_now": now,
                }))
                .await
                .map_err(|e| e.to_string())?;
            }
        }
    }
    sqlx::query("DELETE FROM cadastre.registry_property").execute(&mut *tx).await.map_err(|e| e.to_string())?;
    let (mut sus, mut ids, mut upins, mut refs) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for (su, (id, upin, refnum)) in fresh {
        sus.push(su.clone());
        ids.push(*id);
        upins.push(upin.clone());
        refs.push(refnum.clone());
    }
    sqlx::query("INSERT INTO cadastre.registry_property (su_id, registry_id, upin, refnum) SELECT * FROM unnest($1::text[], $2::bigint[], $3::text[], $4::text[])")
        .bind(&sus)
        .bind(&ids)
        .bind(&upins)
        .bind(&refs)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    let recognised = recognise_links(&mut tx).await.map_err(|e| e.to_string())?;
    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(ReconcileReport { registry_properties: fresh.len(), unlinked, baseline, recognised })
}

/// Read the Registry's linked properties and reconcile.
/// A UPIN on a Registry property makes the Registry treat the file as linked (its check reads only
/// "UPIN assigned"), whether it came from Map → Connect, was typed in, or was carried over from an
/// earlier transaction. Mark the Cadastre side linked too (LRSID, LinkageStatus 1, property number)
/// for every committed parcel whose UPIN exactly one current property holds and which the Cadastre
/// did not know as linked. No link date is invented (the Registry has none for a typed UPIN); the
/// audit records when it was recognised. A UPIN on several properties is left for link problems.
pub async fn recognise_links(conn: &mut sqlx::PgConnection) -> Result<usize, sqlx::Error> {
    let done: Vec<(String, String)> = sqlx::query_as(
        "WITH holders AS (SELECT upin, min(su_id) AS su_id, min(refnum) AS refnum, count(*) AS n FROM cadastre.registry_property GROUP BY upin), \
              one AS (SELECT h.upin, h.su_id, h.refnum FROM holders h WHERE h.n = 1 \
                      AND EXISTS (SELECT 1 FROM cadastre.parcel p WHERE p.parcel_number = h.upin AND p.status = 'posted')) \
         UPDATE cadastre.parcel_attribute a SET lrs_id = o.su_id, linkage_status = 1, \
                property_number = COALESCE(NULLIF(btrim(o.refnum), ''), a.property_number), last_edit_date = now()::timestamp, updated_at = now() \
         FROM one o WHERE a.upin = o.upin AND (a.status = '0' OR a.status IS NULL) AND a.linkage_status IS DISTINCT FROM 1 \
         RETURNING a.upin, o.su_id",
    )
    .fetch_all(&mut *conn)
    .await?;
    if !done.is_empty() {
        audit::append(&mut *conn, "registry-sync", "registry_links_recognised", "registry_link", None, json!({
            "count": done.len(),
            "note": "the Registry property holds the UPIN (typed, carried over or linked); the Cadastre side is marked linked",
            "links": done.iter().take(5000).map(|(u, s)| json!({ "upin": u, "su_id": s })).collect::<Vec<_>>(),
        }))
        .await
        .map_err(|e| sqlx::Error::Protocol(e.to_string()))?;
    }
    Ok(done.len())
}

/// Link problems, for checking by hand: a UPIN on several Registry properties; a UPIN on a Registry
/// property that is not a committed parcel here; a parcel linked to another property than the one
/// now holding its UPIN; a commit whose parcel could not be linked to its file.
pub async fn link_problems(pool: &PgPool) -> Result<serde_json::Value, sqlx::Error> {
    let several: Vec<(String, Vec<String>)> = sqlx::query_as(
        "SELECT upin, array_agg(su_id ORDER BY su_id) FROM cadastre.registry_property GROUP BY upin HAVING count(*) > 1 ORDER BY upin LIMIT 500").fetch_all(pool).await?;
    let missing: Vec<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT r.upin, r.su_id, r.refnum FROM cadastre.registry_property r \
         WHERE NOT EXISTS (SELECT 1 FROM cadastre.parcel p WHERE p.parcel_number = r.upin AND p.status = 'posted') ORDER BY r.upin LIMIT 500").fetch_all(pool).await?;
    let missing_total: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM cadastre.registry_property r WHERE NOT EXISTS (SELECT 1 FROM cadastre.parcel p WHERE p.parcel_number = r.upin AND p.status = 'posted')").fetch_one(pool).await?;
    let mismatch: Vec<(String, Option<String>, String)> = sqlx::query_as(
        "SELECT a.upin, a.lrs_id, r.su_id FROM cadastre.parcel_attribute a JOIN cadastre.registry_property r ON r.upin = a.upin \
         WHERE a.linkage_status = 1 AND (a.status = '0' OR a.status IS NULL) AND a.lrs_id IS NOT NULL AND a.lrs_id <> r.su_id \
           AND (SELECT count(*) FROM cadastre.registry_property x WHERE x.upin = a.upin) = 1 ORDER BY a.upin LIMIT 500").fetch_all(pool).await?;
    let not_linked: Vec<(String, chrono::DateTime<chrono::Utc>, serde_json::Value)> = sqlx::query_as(
        "SELECT DISTINCT ON (f.entity_id) f.entity_id, f.created_at, f.payload FROM cadastre.audit_log f \
         WHERE f.action = 'registry_link_failed' AND (NOT EXISTS (SELECT 1 FROM cadastre.parcel_attribute a WHERE a.upin = f.entity_id AND a.linkage_status = 1) \
                OR (f.payload->>'how' = 'map_ok' AND NOT EXISTS (SELECT 1 FROM cadastre.audit_log m WHERE m.entity = 'registry_link' AND m.entity_id = f.entity_id \
                    AND m.action IN ('registry_link_made', 'registry_link_synced') AND m.id > f.id))) \
         ORDER BY f.entity_id, f.id DESC LIMIT 500").fetch_all(pool).await?;
    Ok(json!({
        "several_properties": several.iter().map(|(u, s)| json!({ "upin": u, "su_ids": s })).collect::<Vec<_>>(),
        "not_in_cadastre": missing.iter().map(|(u, s, r)| json!({ "upin": u, "su_id": s, "refnum": r })).collect::<Vec<_>>(),
        "not_in_cadastre_total": missing_total,
        "linked_elsewhere": mismatch.iter().map(|(u, l, s)| json!({ "upin": u, "cadastre_lrs_id": l, "registry_su_id": s })).collect::<Vec<_>>(),
        "commits_not_linked": not_linked.iter().map(|(u, at, p)| json!({ "upin": u, "at": at.to_rfc3339(), "transaction_number": p["transaction_number"], "reason": p["reason"], "how": p["how"] })).collect::<Vec<_>>(),
    }))
}

pub async fn reconcile_once(pool: &PgPool, cfg: &LrsConfig) -> Result<ReconcileReport, String> {
    let session = Session::start(cfg).await.map_err(|e| e.to_string())?;
    let rows = session.query(SNAPSHOT_QUERY, 200_000).await;
    session.end().await;
    let mut fresh = HashMap::new();
    for r in rows.map_err(|e| e.to_string())? {
        if r.len() < 4 {
            continue;
        }
        let (Some(su), Some(upin)) = (opt(&r[1]), opt(&r[2])) else { continue };
        fresh.insert(su, (r[0].trim().parse::<i64>().ok(), upin.to_uppercase(), opt(&r[3])));
    }
    if fresh.is_empty() {
        return Err("the Registry returned no linked properties; reconcile skipped".into());
    }
    reconcile_with(pool, &fresh).await
}

async fn record(pool: &PgPool, key: &str, status: serde_json::Value) {
    if let Ok(mut conn) = pool.acquire().await {
        let _ = set_setting(&mut conn, key, &status.to_string()).await;
    }
}

/// Run one check and store its outcome for the editor (`registry.sync.status`).
pub async fn sync_and_record(pool: &PgPool, cfg: &LrsConfig) -> Result<SyncReport, String> {
    let result = sync_once(pool, cfg).await;
    record(pool, STATUS_KEY, match &result {
        Ok(r) => json!({ "ok": true, "at": Utc::now().to_rfc3339(), "report": r }),
        Err(e) => json!({ "ok": false, "at": Utc::now().to_rfc3339(), "error": e }),
    })
    .await;
    result
}

pub async fn reconcile_and_record(pool: &PgPool, cfg: &LrsConfig) -> Result<ReconcileReport, String> {
    let result = reconcile_once(pool, cfg).await;
    record(pool, RECONCILE_KEY, match &result {
        Ok(r) => json!({ "ok": true, "at": Utc::now().to_rfc3339(), "report": r }),
        Err(e) => json!({ "ok": false, "at": Utc::now().to_rfc3339(), "error": e }),
    })
    .await;
    result
}

/// The last check's and reconcile's outcomes, as stored.
pub async fn last_status(pool: &PgPool) -> Option<serde_json::Value> {
    setting(pool, STATUS_KEY).await.ok().flatten().and_then(|s| serde_json::from_str(&s).ok())
}
pub async fn last_reconcile(pool: &PgPool) -> Option<serde_json::Value> {
    setting(pool, RECONCILE_KEY).await.ok().flatten().and_then(|s| serde_json::from_str(&s).ok())
}

/// Background loops (started by `main` when the Registry is configured).
pub async fn run(pool: PgPool, cfg: LrsConfig) {
    let env_secs = |k: &str, d: u64, min: u64| std::env::var(k).ok().and_then(|s| s.trim().parse::<u64>().ok()).filter(|s| *s >= min).unwrap_or(d);
    let secs = env_secs("CADASTRE_REGISTRY_POLL_SECS", 60, 15);
    let rsecs = env_secs("CADASTRE_REGISTRY_RECONCILE_SECS", 900, 120);
    tracing::info!("Registry link sync on: {} every {secs}s, unlink reconcile every {rsecs}s, as {} (writes {})",
        cfg.base_url, cfg.user, if cfg.write_enabled { "allowed on request" } else { "off" });
    {
        let (pool, cfg) = (pool.clone(), cfg.clone());
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(rsecs));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tick.tick().await;
                match reconcile_and_record(&pool, &cfg).await {
                    Ok(r) if r.unlinked > 0 => tracing::info!("Registry reconcile: {} unlink(s) applied", r.unlinked),
                    Ok(_) => {}
                    Err(e) => tracing::warn!("Registry reconcile failed: {e}"),
                }
            }
        });
    }
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(secs));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        match sync_and_record(&pool, &cfg).await {
            Ok(r) if r.new > 0 => tracing::info!("Registry sync: {} new link(s), {} parcel record(s) updated", r.new, r.updated),
            Ok(_) => {}
            Err(e) => tracing::warn!("Registry sync failed: {e}"),
        }
    }
}

/// The audit events of one UPIN's Registry link, newest first (for the parcel's history).
pub async fn events_for(pool: &PgPool, upin: &str) -> Result<Vec<serde_json::Value>, sqlx::Error> {
    let rows = sqlx::query("SELECT action, actor, payload, created_at FROM cadastre.audit_log WHERE entity = 'registry_link' AND entity_id = $1 ORDER BY id DESC LIMIT 50")
        .bind(upin)
        .fetch_all(pool)
        .await?;
    Ok(rows.iter().map(|r| json!({ "action": r.get::<String, _>("action"), "actor": r.get::<String, _>("actor"),
        "payload": r.get::<serde_json::Value, _>("payload"), "at": r.get::<chrono::DateTime<Utc>, _>("created_at").to_rfc3339() })).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(su: &str, upin: &str, at: &str, txn: &str, code: &str) -> Vec<String> {
        [su, upin, "PL1", at, txn, code, "x", "", "B1"].iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn one_link_per_property_preferring_the_active_transaction() {
        let rows = vec![
            row("PLT2", "pl000000000002", "2 Nov 2026 12:00:00 GMT", "PLS2", "transaction_lodged"),
            row("PLT1", "PL000000000001", "2 Nov 2026 09:00:00 GMT", "PLS1a", "transaction_completed"),
            row("PLT1", "PL000000000001", "2 Nov 2026 09:00:00 GMT", "PLS1b", "transaction_lodged"),
            row("PLT1", "PL000000000001", "2 Nov 2026 09:00:00 GMT", "PLS1c", "transaction_withdrawn"),
            row("", "PL3", "2 Nov 2026 09:00:00 GMT", "PLS3", "transaction_lodged"),
        ];
        let l = links_from_rows(&rows);
        assert_eq!(l.len(), 2, "rows without a property id are skipped");
        assert_eq!((l[0].su_id.as_str(), l[0].transaction_number.as_str()), ("PLT1", "PLS1b"));
        assert_eq!(l[1].upin, "PL000000000002", "UPINs upper-cased");
        assert_eq!(l[1].lot, None);
        assert_eq!(l[1].block.as_deref(), Some("B1"));
    }

    #[test]
    fn query_reads_current_linked_properties_since() {
        let q = links_query(NaiveDateTime::parse_from_str("2026-11-02 07:00:00", "%Y-%m-%d %H:%M:%S").unwrap());
        assert!(q.contains("s.SpatialDate1 >= '2026-11-02 07:00:00'"));
        assert!(q.contains("s.Upin is not null") && q.contains("l.EndLifespanVersion is null"));
    }

    #[test]
    fn aumentum_fields_map_to_columns() {
        assert_eq!(attribute_column("LRSID"), Some(("lrs_id", false)));
        assert_eq!(attribute_column("LinkageStatus"), Some(("linkage_status", true)));
        assert_eq!(attribute_column("Plot"), Some(("plot", false)));
        assert_eq!(attribute_column("Nonsense"), None);
    }
}
