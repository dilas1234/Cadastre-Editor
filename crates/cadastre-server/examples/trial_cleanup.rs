//! Administrator tool for the switch from Aumentum: remove what was made in this Cadastre while
//! testing, and bring over the jobs Aumentum created (or changed) since the data was copied, with
//! Aumentum's own job numbers, so every job the Registry knows (lr_mc_job) means the same job here.
//!
//! cargo run -p cadastre-server --example trial_cleanup -- <aumentum-jobs.json> <aumentum-changed-jobs.json> [--apply]
//!
//! * Trial jobs: every job made in this Cadastre (`source = 'cadastre'`), with its parcels (the
//!   posted ones too), the attribute records and beacons its commits added, its job area, tasks,
//!   linkage and its working job. UPINs stay in the issued-UPIN ledger (never reissued).
//! * Aumentum's new jobs (from its job service, GetJobInstance): inserted with their numbers.
//! * Aumentum's changed jobs: status and current task brought to Aumentum's.
//! * The job counter continues after the highest job number.
//!
//! Without --apply it only reports. One transaction; audit entries `trial_cleanup` and
//! `aumentum_jobs_caught_up`. Reads CADASTRE_DATABASE_URL.

use serde_json::{json, Value};
use sqlx::Row;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let apply = args.iter().any(|a| a == "--apply");
    let files: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
    let (Some(new_path), Some(changed_path)) = (files.first(), files.get(1)) else {
        return Err("usage: trial_cleanup <aumentum-jobs.json> <aumentum-changed-jobs.json> [--apply]".into());
    };
    let new_jobs: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(new_path)?)?;
    let changed: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(changed_path)?)?;
    let pool = sqlx::PgPool::connect(&std::env::var("CADASTRE_DATABASE_URL")?).await?;
    let mut tx = pool.begin().await?;

    // ---- 1. The trial (jobs made in this Cadastre).
    let trial: Vec<(i32, Option<String>, Option<uuid::Uuid>)> =
        sqlx::query_as("SELECT id, name, cadastre_job_id FROM cadastre.wf_job_instance WHERE source = 'cadastre' ORDER BY id").fetch_all(&mut *tx).await?;
    let cjobs: Vec<uuid::Uuid> = trial.iter().filter_map(|t| t.2).collect();
    let parcels = sqlx::query("SELECT id, parcel_number, status, posted_at FROM cadastre.parcel WHERE job_id = ANY($1)").bind(&cjobs).fetch_all(&mut *tx).await?;
    let pids: Vec<uuid::Uuid> = parcels.iter().map(|r| r.get("id")).collect();
    let upins: Vec<String> = parcels.iter().filter_map(|r| r.get::<Option<String>, _>("parcel_number")).collect();
    let linked: i64 = sqlx::query_scalar("SELECT count(*) FROM cadastre.parcel_attribute WHERE upin = ANY($1) AND (linkage_status = 1 OR lrs_id IS NOT NULL)")
        .bind(&upins).fetch_one(&mut *tx).await?;
    if linked > 0 {
        return Err(format!("a trial parcel is linked to the Registry ({upins:?}): unlink it there first").into());
    }
    let beacons: Vec<(uuid::Uuid, String)> = sqlx::query_as(
        "SELECT DISTINCT b.id, b.number FROM cadastre.parcel_corner pc JOIN cadastre.beacon b ON b.id = pc.beacon_id JOIN cadastre.parcel p ON p.id = pc.parcel_id \
         WHERE pc.parcel_id = ANY($1) AND p.posted_at IS NOT NULL AND b.created_at >= p.posted_at - interval '5 seconds' \
           AND NOT EXISTS (SELECT 1 FROM cadastre.parcel_corner x WHERE x.beacon_id = b.id AND NOT (x.parcel_id = ANY($1)))",
    ).bind(&pids).fetch_all(&mut *tx).await?;
    let attrs: Vec<i32> = sqlx::query_scalar("SELECT object_id FROM cadastre.parcel_attribute WHERE upin = ANY($1) AND object_id >= 1000000000")
        .bind(&upins).fetch_all(&mut *tx).await?;
    println!("trial jobs: {}", trial.iter().map(|t| format!("{} {}", t.0, t.1.as_deref().unwrap_or("(no name)"))).collect::<Vec<_>>().join(", "));
    println!("  parcels: {} ({}), attribute records {:?}, beacons {}", parcels.len(),
        parcels.iter().map(|r| format!("{}:{}", r.get::<Option<String>, _>("parcel_number").unwrap_or_else(|| "draft".into()), r.get::<String, _>("status"))).collect::<Vec<_>>().join(", "),
        attrs, beacons.iter().map(|b| b.1.as_str()).collect::<Vec<_>>().join(", "));

    // ---- 2. Aumentum's jobs since the copy.
    let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).map(str::to_string).filter(|x| !x.is_empty());
    let n = |v: &Value, k: &str| s(v, k).and_then(|x| x.parse::<i32>().ok());
    println!("Aumentum jobs to bring over: {} ({} .. {})", new_jobs.len(),
        new_jobs.first().and_then(|j| s(j, "ID")).unwrap_or_default(), new_jobs.last().and_then(|j| s(j, "ID")).unwrap_or_default());
    let real_changes: Vec<&Value> = changed.iter().filter(|c| c["aumentum"][0].is_string()).collect();
    for c in &real_changes {
        println!("  changed in Aumentum: job {} {} here {} -> Aumentum {}", c["id"], c["name"], c["here"], c["aumentum"]);
    }
    if !apply {
        println!("dry run: add --apply to do it");
        return Ok(());
    }

    sqlx::query("DELETE FROM cadastre.parcel_attribute WHERE object_id = ANY($1)").bind(&attrs).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM cadastre.parcel WHERE id = ANY($1)").bind(&pids).execute(&mut *tx).await?; // corners, draft attributes cascade
    for (id, _) in &beacons {
        sqlx::query("DELETE FROM cadastre.beacon WHERE id = $1").bind(id).execute(&mut *tx).await?;
    }
    let trial_ids: Vec<i32> = trial.iter().map(|t| t.0).collect();
    sqlx::query("DELETE FROM cadastre.wf_job_instance WHERE id = ANY($1) AND source = 'cadastre'").bind(&trial_ids).execute(&mut *tx).await?; // tasks, area, files, linkage cascade
    sqlx::query("DELETE FROM cadastre.job WHERE id = ANY($1)").bind(&cjobs).execute(&mut *tx).await?;
    cadastre_server::audit::append(&mut tx, "administrator", "trial_cleanup", "wf_job_instance", None, json!({
        "jobs": trial.iter().map(|t| json!({"id": t.0, "name": t.1})).collect::<Vec<_>>(),
        "parcels": upins, "attribute_records": attrs, "beacons": beacons.iter().map(|b| &b.1).collect::<Vec<_>>(),
        "note": "UPINs stay in the issued-UPIN ledger",
    })).await?;

    for j in &new_jobs {
        let (Some(id), Some(job_type)) = (n(j, "ID"), n(j, "JobID")) else { continue };
        let task = n(j, "TaskID");
        let sid: Option<i32> = match n(j, "Sid") {
            Some(sid) => sqlx::query_scalar("SELECT sid FROM cadastre.principal WHERE sid = $1").bind(sid).fetch_optional(&mut *tx).await?,
            None => None,
        };
        sqlx::query(
            "INSERT INTO cadastre.wf_job_instance (id, job_type_id, current_task_id, assigned_sid, priority, status, name, comment, parameters, lrs_transaction_number, created, source) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, now()::timestamp, 'aumentum')",
        )
        .bind(id).bind(job_type).bind(task).bind(sid).bind(s(j, "Priority")).bind(n(j, "JobStatus").unwrap_or(2) as i16)
        .bind(s(j, "Name")).bind(s(j, "Comment")).bind(s(j, "Parameters")).bind(s(j, "LrsTransactionNumber"))
        .execute(&mut *tx).await?;
        if let Some(t) = task {
            sqlx::query("INSERT INTO cadastre.wf_task_instance (job_instance_id, job_type_id, task_id, sid, assigned) VALUES ($1, $2, $3, $4, now()::timestamp)")
                .bind(id).bind(job_type).bind(t).bind(sid).execute(&mut *tx).await?;
        }
    }
    for c in &real_changes {
        let id: i32 = c["id"].as_str().unwrap_or("0").parse()?;
        let status: i16 = c["aumentum"][0].as_str().unwrap_or("2").parse()?;
        let task: Option<i32> = c["aumentum"][1].as_str().and_then(|t| t.parse().ok());
        let open: bool = sqlx::query_scalar("SELECT is_open FROM cadastre.wf_job_status WHERE code = $1").bind(status).fetch_one(&mut *tx).await?;
        sqlx::query("UPDATE cadastre.wf_task_instance SET ended = COALESCE(ended, now()::timestamp) WHERE job_instance_id = $1 AND ended IS NULL").bind(id).execute(&mut *tx).await?;
        sqlx::query("UPDATE cadastre.wf_job_instance SET status = $2, current_task_id = $3, updated = now()::timestamp, ended = CASE WHEN $4 THEN NULL ELSE now()::timestamp END WHERE id = $1")
            .bind(id).bind(status).bind(task).bind(open).execute(&mut *tx).await?;
    }
    let next: i64 = sqlx::query_scalar("SELECT setval('cadastre.wf_job_instance_seq', (SELECT max(id) FROM cadastre.wf_job_instance))").fetch_one(&mut *tx).await?;
    cadastre_server::audit::append(&mut tx, "administrator", "aumentum_jobs_caught_up", "wf_job_instance", None, json!({
        "new_jobs": new_jobs.iter().filter_map(|j| s(j, "ID")).collect::<Vec<_>>(),
        "changed": real_changes.iter().map(|c| json!({"id": c["id"], "aumentum": c["aumentum"]})).collect::<Vec<_>>(),
        "job_counter_at": next, "source": "Aumentum job service GetJobInstance (read-only)",
    })).await?;
    tx.commit().await?;
    println!("done; the next job number is {}", next + 1);
    Ok(())
}
