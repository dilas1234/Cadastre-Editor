//! Administrator tool: undo a workflow commit that was made by mistake, before the parcel is linked
//! anywhere. The parcel goes back to a draft of its job (same shape), its attribute record and
//! the beacons the commit added are removed, and the job is reopened at its commit task, so the
//! parcel can be committed again. The UPIN stays in the issued-UPIN ledger (it is never handed
//! out again). Refused if the parcel is linked to the Registry, has successors, or its beacons are
//! used by other parcels. One transaction; an audit entry `wf_commit_reverted`.
//!
//! cargo run -p cadastre-server --example revert_commit -- <job instance id> <UPIN> [property number] [--apply]
//! Without --apply it only reports what it would do. Reads CADASTRE_DATABASE_URL.

use serde_json::json;
use sqlx::Row;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let apply = args.iter().any(|a| a == "--apply");
    let pos: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
    let (Some(job), Some(upin)) = (pos.first(), pos.get(1)) else {
        return Err("usage: revert_commit <job instance id> <UPIN> [property number] [--apply]".into());
    };
    let job: i32 = job.parse()?;
    let property = pos.get(2).map(|s| s.to_string());
    let pool = sqlx::PgPool::connect(&std::env::var("CADASTRE_DATABASE_URL")?).await?;
    let mut tx = pool.begin().await?;

    let w = sqlx::query("SELECT cadastre_job_id, job_type_id, assigned_sid FROM cadastre.wf_job_instance WHERE id = $1 FOR UPDATE")
        .bind(job).fetch_optional(&mut *tx).await?.ok_or("no such job")?;
    let (cjob, job_type, sid): (Option<uuid::Uuid>, i32, Option<i32>) = (w.get(0), w.get(1), w.get(2));
    let p = sqlx::query("SELECT id, posted_at FROM cadastre.parcel WHERE parcel_number = $1 AND status = 'posted' AND job_id = $2 FOR UPDATE")
        .bind(upin).bind(cjob).fetch_optional(&mut *tx).await?.ok_or("the UPIN is not a posted parcel of this job")?;
    let (pid, posted_at): (uuid::Uuid, chrono::DateTime<chrono::Utc>) = (p.get(0), p.get(1));

    let linked: i64 = sqlx::query_scalar("SELECT count(*) FROM cadastre.parcel_attribute WHERE upin = $1 AND (linkage_status = 1 OR lrs_id IS NOT NULL)")
        .bind(upin).fetch_one(&mut *tx).await?;
    let lineage: i64 = sqlx::query_scalar("SELECT count(*) FROM cadastre.parcel_lineage WHERE parcel_id = $1 OR parent_id = $1 OR upin = $2 OR parent_upin = $2")
        .bind(pid).bind(upin).fetch_one(&mut *tx).await?;
    if linked > 0 { return Err("the parcel is linked to the Registry: unlink it there first".into()); }
    if lineage > 0 { return Err("the parcel has mutation lineage: not reverted by this tool".into()); }

    // The beacons this commit added: created at the commit and used only by this parcel.
    let beacons: Vec<(uuid::Uuid, String)> = sqlx::query_as(
        "SELECT b.id, b.number FROM cadastre.parcel_corner pc JOIN cadastre.beacon b ON b.id = pc.beacon_id \
         WHERE pc.parcel_id = $1 AND b.created_at >= $2 - interval '5 seconds' \
           AND NOT EXISTS (SELECT 1 FROM cadastre.parcel_corner x WHERE x.beacon_id = b.id AND x.parcel_id <> $1) ORDER BY pc.seq",
    ).bind(pid).bind(posted_at).fetch_all(&mut *tx).await?;
    let attr = sqlx::query(
        "SELECT object_id, property_number, property_type, plot, block, location, plan_number, lga, parcel_name, area_unit, surveyor_general \
         FROM cadastre.parcel_attribute WHERE upin = $1 AND parcel_id = $2 AND object_id >= 1000000000",
    ).bind(upin).bind(pid).fetch_optional(&mut *tx).await?;
    let commit_task: i32 = sqlx::query_scalar(
        "SELECT f.task_id FROM cadastre.wf_task_flow f JOIN cadastre.wf_task_step ts ON ts.task_id = f.task_id JOIN cadastre.wf_step s ON s.id = ts.step_id \
         WHERE f.job_type_id = $1 AND EXISTS (SELECT 1 FROM unnest(s.commands) c WHERE upper(c) = upper($2)) ORDER BY f.task_id LIMIT 1",
    ).bind(job_type).bind(cadastre_server::routes::wf::COMMIT_TOOL).fetch_one(&mut *tx).await?;

    let property_number = property.clone().or_else(|| attr.as_ref().and_then(|a| a.get::<Option<String>, _>("property_number")));
    println!("job {job}: {upin} (parcel {pid}) back to a draft; property number {property_number:?}");
    println!("  attribute record removed: {:?}", attr.as_ref().map(|a| a.get::<i32, _>("object_id")));
    println!("  beacons removed: {}", beacons.iter().map(|b| b.1.as_str()).collect::<Vec<_>>().join(", "));
    println!("  job reopened at task {commit_task} (status In progress)");
    println!("  {upin} stays in the issued-UPIN ledger");
    if !apply {
        println!("dry run: add --apply to do it");
        return Ok(());
    }

    if let Some(a) = &attr {
        sqlx::query(
            "INSERT INTO cadastre.parcel_draft_attribute (parcel_id, property_number, property_type, plot, block, location, plan_number, lga, parcel_name, area_unit, surveyor_general, status) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, 'Active') ON CONFLICT (parcel_id) DO NOTHING",
        )
        .bind(pid).bind(&property_number)
        .bind(a.get::<Option<String>, _>("property_type")).bind(a.get::<Option<String>, _>("plot")).bind(a.get::<Option<String>, _>("block"))
        .bind(a.get::<Option<String>, _>("location")).bind(a.get::<Option<String>, _>("plan_number")).bind(a.get::<Option<String>, _>("lga"))
        .bind(a.get::<Option<String>, _>("parcel_name")).bind(a.get::<Option<i16>, _>("area_unit")).bind(a.get::<Option<String>, _>("surveyor_general"))
        .execute(&mut *tx).await?;
        sqlx::query("DELETE FROM cadastre.parcel_attribute WHERE object_id = $1").bind(a.get::<i32, _>("object_id")).execute(&mut *tx).await?;
    }
    sqlx::query("DELETE FROM cadastre.parcel_corner WHERE parcel_id = $1").bind(pid).execute(&mut *tx).await?;
    for (id, _) in &beacons {
        sqlx::query("DELETE FROM cadastre.beacon WHERE id = $1").bind(id).execute(&mut *tx).await?;
    }
    sqlx::query("UPDATE cadastre.parcel SET status = 'draft', parcel_number = NULL, posted_at = NULL, updated_at = now() WHERE id = $1")
        .bind(pid).execute(&mut *tx).await?;
    sqlx::query("UPDATE cadastre.wf_job_instance SET status = 2, ended = NULL, current_task_id = $2, updated = now()::timestamp WHERE id = $1")
        .bind(job).bind(commit_task).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO cadastre.wf_task_instance (job_instance_id, job_type_id, task_id, sid, assigned, started) VALUES ($1, $2, $3, $4, now()::timestamp, now()::timestamp)")
        .bind(job).bind(job_type).bind(commit_task).bind(sid).execute(&mut *tx).await?;
    cadastre_server::audit::append(&mut tx, "administrator", "wf_commit_reverted", "wf_job_instance", Some(&job.to_string()), json!({
        "upin": upin, "parcel_id": pid, "beacons_removed": beacons.iter().map(|b| &b.1).collect::<Vec<_>>(),
        "attribute_object_id": attr.as_ref().map(|a| a.get::<i32, _>("object_id")), "property_number": property_number,
        "reopened_at_task": commit_task,
        "reason": "UPIN already issued by Aumentum (parallel run); the ledger keeps it so it is never reissued",
    })).await?;
    tx.commit().await?;
    println!("done");
    Ok(())
}
