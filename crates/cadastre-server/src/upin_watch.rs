//! Parallel run with Aumentum: watch for duplicate UPINs after the fact.
//!
//! Before giving a UPIN this Cadastre asks Aumentum (`aumentum::UpinCheck`), so it never takes a
//! number Aumentum has issued. The reverse cannot be prevented from here: Aumentum does not know
//! this Cadastre's numbers and may later issue one already given here. This task notices that:
//! every `CADASTRE_AUMENTUM_WATCH_SECS` (default 900) it asks Aumentum about each UPIN committed
//! here (ledger source `posted`) and records any that Aumentum now has as well. The outcome is
//! stored in `cadastre.app_setting` (`upin.watch.status`) and shown by `GET /api/v1/registry/status`;
//! each newly found duplicate is written to the audit log once.
//!
//! Runs only while the parallel-run check is configured (`CADASTRE_AUMENTUM_JOBSERVICE_URL`).
//! Read-only towards Aumentum.

use serde_json::{json, Value};
use sqlx::PgPool;

use crate::audit;
use crate::aumentum::UpinCheck;

pub const STATUS_KEY: &str = "upin.watch.status";

/// The last stored outcome, for the status endpoint.
pub async fn last_status(pool: &PgPool) -> Option<Value> {
    let v: Option<String> = sqlx::query_scalar("SELECT value FROM cadastre.app_setting WHERE key = $1")
        .bind(STATUS_KEY)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
    v.and_then(|s| serde_json::from_str(&s).ok())
}

async fn store(pool: &PgPool, status: &Value) {
    let _ = sqlx::query(
        "INSERT INTO cadastre.app_setting (key, value, source, updated_by) VALUES ($1, $2, 'cadastre', 'upin-watch') \
         ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_by = EXCLUDED.updated_by, updated_at = now()",
    )
    .bind(STATUS_KEY)
    .bind(status.to_string())
    .execute(pool)
    .await;
}

/// One pass: ask Aumentum about every UPIN committed here. Returns the stored status.
pub async fn check_once(pool: &PgPool) -> Value {
    let at = chrono::Utc::now().to_rfc3339();
    let result: Result<Value, String> = async {
        let upins: Vec<(String, chrono::DateTime<chrono::Utc>)> =
            sqlx::query_as("SELECT upin, first_seen FROM cadastre.issued_upin WHERE source = 'posted' ORDER BY upin")
                .fetch_all(pool)
                .await
                .map_err(|e| e.to_string())?;
        let Some(check) = UpinCheck::connect().await? else {
            return Err("the parallel-run check is not configured (CADASTRE_AUMENTUM_JOBSERVICE_URL)".into());
        };
        let mut duplicates = Vec::new();
        let mut failed = None;
        for (upin, first_seen) in &upins {
            match check.issued(upin).await {
                Ok(true) => duplicates.push(json!({ "upin": upin, "committed_here": first_seen.to_rfc3339() })),
                Ok(false) => {}
                Err(e) => { failed = Some(e); break; }
            }
        }
        check.close().await;
        if let Some(e) = failed {
            return Err(e);
        }
        Ok(json!({ "checked": upins.len(), "duplicates": duplicates }))
    }
    .await;

    let status = match result {
        Ok(v) => {
            // Audit each duplicate the first time it is seen.
            let before: Vec<String> = last_status(pool).await
                .and_then(|s| s.get("duplicates").cloned())
                .and_then(|d| serde_json::from_value::<Vec<Value>>(d).ok())
                .unwrap_or_default()
                .iter()
                .filter_map(|d| d.get("upin").and_then(Value::as_str).map(str::to_string))
                .collect();
            for d in v["duplicates"].as_array().cloned().unwrap_or_default() {
                let upin = d["upin"].as_str().unwrap_or_default().to_string();
                if before.contains(&upin) {
                    continue;
                }
                tracing::error!("duplicate UPIN: {upin} was committed here and Aumentum has issued it too");
                if let Ok(mut tx) = pool.begin().await {
                    if audit::append(&mut tx, "upin-watch", "duplicate_upin_found", "upin", Some(&upin), d.clone()).await.is_ok() {
                        let _ = tx.commit().await;
                    }
                }
            }
            json!({ "at": at, "ok": true, "checked": v["checked"], "duplicates": v["duplicates"] })
        }
        Err(e) => {
            // Keep the duplicates already known: a failed pass does not clear them.
            let known = last_status(pool).await.and_then(|s| s.get("duplicates").cloned()).unwrap_or_else(|| json!([]));
            json!({ "at": at, "ok": false, "error": e, "duplicates": known })
        }
    };
    store(pool, &status).await;
    status
}

/// Run the watch forever (spawned from `main` while the parallel-run check is configured).
pub async fn run(pool: PgPool) {
    let secs = std::env::var("CADASTRE_AUMENTUM_WATCH_SECS").ok().and_then(|s| s.trim().parse::<u64>().ok()).unwrap_or(900).max(60);
    tracing::info!("duplicate-UPIN watch on: Aumentum is asked about this Cadastre's UPINs every {secs}s");
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(secs));
    loop {
        tick.tick().await;
        let s = check_once(&pool).await;
        if s["ok"] != json!(true) {
            tracing::warn!("duplicate-UPIN watch could not ask Aumentum: {}", s["error"]);
        }
    }
}
