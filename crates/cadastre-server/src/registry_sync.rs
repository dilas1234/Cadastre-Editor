//! Registry → Cadastre link sync (Phase 5). The Registry (LRS43) is only READ.
//!
//! When the Registry admin links a property to a Cadastre parcel, the Registry stamps the
//! property's `SpatialDate1` and holds the parcel's UPIN. Every `CADASTRE_REGISTRY_POLL_SECS`
//! (default 60) this asks the Registry, through the same HQL interface Aumentum used, for the
//! properties linked since the last check, and writes the link onto the parcel's attribute
//! record as the Registry's `mcintegration` setting `GISUpdatedFields` defines:
//! `suID → LRSID`, `1 → LinkageStatus`, `spatialDate1 → LinkageDate` (yyyy-MM-dd),
//! `refnum → PropertyNumber`. (`plotNumber → Plot` and `blockNumber → Block` are written by
//! Aumentum as the literal names, see `registry.rs`; they are left alone until PLAGIS decides.)
//!
//! Each link seen for the first time is an audit event (`registry_link_synced`, entity
//! `registry_link`, id = the UPIN), which the editor's "Linked from the Registry" list shows.
//! Re-reading the same links changes nothing: updates are conditional and events deduplicated.

use std::collections::BTreeMap;

use chrono::{Duration, NaiveDateTime, Utc};
use serde_json::json;
use sqlx::PgPool;

use crate::audit;
use crate::lrs::{parse_datetime, LrsConfig, Session};

const WATERMARK_KEY: &str = "registry.sync.since";
const STATUS_KEY: &str = "registry.sync.status";
/// Links are re-read this far back from the newest one seen, so a link stamped while a check
/// was running (or with a slightly older time) is never missed.
const OVERLAP_HOURS: i64 = 2;

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
}

async fn setting(pool: &PgPool, key: &str) -> Result<Option<String>, sqlx::Error> {
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

/// Apply links to the Cadastre (attribute records and events). Pure database work, so it is
/// tested without a Registry.
pub async fn apply(pool: &PgPool, links: &[RegistryLink]) -> Result<(usize, usize, usize), sqlx::Error> {
    let (mut new, mut updated, mut unmatched) = (0, 0, 0);
    let mut tx = pool.begin().await?;
    for l in links {
        let linkage_date = l.linked_at.format("%Y-%m-%d").to_string();
        let n = sqlx::query(
            "UPDATE cadastre.parcel_attribute SET lrs_id = $2, linkage_status = 1, linkage_date = $3, property_number = COALESCE($4, property_number), \
                last_edit_date = now()::timestamp, updated_at = now() \
             WHERE upin = $1 AND (status = '0' OR status IS NULL) \
               AND (lrs_id IS DISTINCT FROM $2 OR linkage_status IS DISTINCT FROM 1 OR linkage_date IS DISTINCT FROM $3 \
                    OR ($4::text IS NOT NULL AND property_number IS DISTINCT FROM $4))",
        )
        .bind(&l.upin)
        .bind(&l.su_id)
        .bind(&linkage_date)
        .bind(&l.refnum)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        updated += n as usize;
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
            "attribute_record": has_attr, "in_fabric": in_fabric,
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

/// One check: read new links from the Registry and apply them.
pub async fn sync_once(pool: &PgPool, cfg: &LrsConfig) -> Result<SyncReport, String> {
    let from = since(pool).await.map_err(|e| e.to_string())?;
    let session = Session::start(cfg).await.map_err(|e| e.to_string())?;
    let offset = session.clock_offset_secs();
    let rows = session.query(&links_query(from), 5000).await;
    session.end().await;
    let links = links_from_rows(&rows.map_err(|e| e.to_string())?);
    let (new, updated, unmatched) = apply(pool, &links).await.map_err(|e| e.to_string())?;
    Ok(SyncReport { read: links.len(), new, updated, unmatched, since: from.format("%Y-%m-%d %H:%M:%S").to_string(), clock_offset_secs: offset })
}

/// Run one check and store its outcome for the editor (`registry.sync.status`).
pub async fn sync_and_record(pool: &PgPool, cfg: &LrsConfig) -> Result<SyncReport, String> {
    let result = sync_once(pool, cfg).await;
    let status = match &result {
        Ok(r) => json!({ "ok": true, "at": Utc::now().to_rfc3339(), "report": r }),
        Err(e) => json!({ "ok": false, "at": Utc::now().to_rfc3339(), "error": e }),
    };
    if let Ok(mut conn) = pool.acquire().await {
        let _ = set_setting(&mut conn, STATUS_KEY, &status.to_string()).await;
    }
    result
}

/// The last check's outcome, as stored.
pub async fn last_status(pool: &PgPool) -> Option<serde_json::Value> {
    setting(pool, STATUS_KEY).await.ok().flatten().and_then(|s| serde_json::from_str(&s).ok())
}

/// Background loop (started by `main` when the Registry is configured).
pub async fn run(pool: PgPool, cfg: LrsConfig) {
    let secs = std::env::var("CADASTRE_REGISTRY_POLL_SECS").ok().and_then(|s| s.trim().parse::<u64>().ok()).filter(|s| *s >= 15).unwrap_or(60);
    tracing::info!("Registry link sync on: {} every {secs}s (read-only), as {}", cfg.base_url, cfg.user);
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
}
