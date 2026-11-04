//! Administrator tool: undo Registry writes made from the Cadastre, to return to Aumentum's flow
//! (the UPIN goes onto the Registry file at title deed time, linked by the Registry admin).
//!
//!   cargo run -p cadastre-server --example registry_revert -- links <UPIN>... [--apply]
//!     For each UPIN linked from the Cadastre (audit `registry_link_made`): the Registry property's
//!     UPIN and link date (SpatialDate1) are cleared, if it still holds that UPIN; the parcel's link
//!     fields (LRSID, LinkageStatus, LinkageDate) are cleared; its property number is kept.
//!
//!   cargo run -p cadastre-server --example registry_revert -- plan <job instance id> [--apply]
//!     The job's title deed plan: the saved form and the plan files are deleted here, and the plan
//!     documents this Cadastre sent to the job's Registry transaction are deleted there.
//!
//! Without --apply it only reports. Reads CADASTRE_DATABASE_URL, the Registry connection
//! (CADASTRE_REGISTRY_URL / _USER / _PASSWORD, CADASTRE_REGISTRY_WRITE=1) and CADASTRE_FILES_DIR.
//! Each change is audited (`registry_link_reverted`, `title_deed_plan_deleted`).

use cadastre_server::lrs::{self, LrsConfig, Session};
use serde_json::json;
use sqlx::Row;

fn q(s: &str) -> String {
    s.replace('\'', "")
}

async fn one(session: &Session, hql: &str) -> Result<Vec<Vec<String>>, Box<dyn std::error::Error>> {
    Ok(session.query(hql, 50).await?)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let apply = args.iter().any(|a| a == "--apply");
    let rest: Vec<&String> = args.iter().filter(|a| *a != "--apply").collect();
    let Some(cmd) = rest.first() else {
        eprintln!("usage: registry_revert links <UPIN>... | plan <job instance id>  [--apply]");
        std::process::exit(2);
    };
    let pool = sqlx::PgPool::connect(&std::env::var("CADASTRE_DATABASE_URL")?).await?;
    let cfg = LrsConfig::from_env().ok_or("the Registry connection is not configured (CADASTRE_REGISTRY_URL / _USER / _PASSWORD)")?;
    let session = Session::start(&cfg).await?;
    let result = match cmd.as_str() {
        "links" => links(&pool, &session, &rest[1..], apply).await,
        "plan" => plan(&pool, &session, rest.get(1).ok_or("give the job instance id")?.parse()?, apply).await,
        other => Err(format!("unknown command {other}").into()),
    };
    session.end().await;
    result?;
    if !apply {
        println!("dry run: add --apply to do it");
    }
    Ok(())
}

async fn links(pool: &sqlx::PgPool, session: &Session, upins: &[&String], apply: bool) -> Result<(), Box<dyn std::error::Error>> {
    for upin in upins {
        let upin = q(&upin.to_uppercase());
        let made = sqlx::query("SELECT payload FROM cadastre.audit_log WHERE action = 'registry_link_made' AND entity = 'registry_link' AND entity_id = $1 ORDER BY id DESC LIMIT 1")
            .bind(&upin).fetch_optional(pool).await?;
        let Some(made) = made else { println!("{upin}: no link made from the Cadastre: skipped"); continue };
        let p: serde_json::Value = made.get(0);
        let su = q(p["su_id"].as_str().unwrap_or_default());
        let txn = q(p["transaction_number"].as_str().unwrap_or_default());
        let t = one(session, &format!("select t.Id from com.landsystems.lrs.model.Transaction t where t.TransactionNumber = '{txn}'")).await?;
        let tid: i64 = t.first().and_then(|r| r.first()).ok_or(format!("{txn} not found in the Registry"))?.trim().parse()?;
        let props = one(session, &format!(
            "select s.Id, s.Upin, s.SpatialDate1 from com.landsystems.lrs.model.SpatialUnit s where s.SuID = '{su}' and s.EndLifespanVersion is null")).await?;
        let Some(prop) = props.first() else { println!("{upin}: Registry property {su} not found: skipped"); continue };
        let (pid, now_upin) = (prop[0].trim().to_string(), prop.get(1).map(|v| v.trim().to_uppercase()).unwrap_or_default());
        let registry = if now_upin == upin { "UPIN and link date cleared on the Registry property" } else { "the Registry property no longer holds it (left as it is)" };
        println!("{upin}: {txn} / {su} (property {pid}): {registry}; Cadastre link fields cleared");
        if !apply { continue; }
        if now_upin == upin {
            session.save_entity(tid, &lrs::entity_xml("Parcel", &[("Id", Some(&pid)), ("upin", Some("")), ("SpatialDate1", None)])).await?;
        }
        let mut tx = pool.begin().await?;
        sqlx::query("UPDATE cadastre.parcel_attribute SET lrs_id = NULL, linkage_status = 0, linkage_date = NULL, last_edit_date = now()::timestamp, updated_at = now() \
                     WHERE upin = $1 AND (status = '0' OR status IS NULL)").bind(&upin).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM cadastre.registry_property WHERE su_id = $1 AND upin = $2").bind(&su).bind(&upin).execute(&mut *tx).await?;
        cadastre_server::audit::append(&mut tx, "administrator", "registry_link_reverted", "registry_link", Some(&upin), json!({
            "su_id": su, "transaction_number": txn, "registry_cleared": now_upin == upin,
            "reason": "Aumentum's flow kept: the UPIN goes onto the Registry file at title deed time, linked by the Registry admin",
        })).await?;
        tx.commit().await?;
    }
    Ok(())
}

async fn plan(pool: &sqlx::PgPool, session: &Session, job: i32, apply: bool) -> Result<(), Box<dyn std::error::Error>> {
    let txn: Option<String> = sqlx::query_scalar("SELECT lrs_transaction_number FROM cadastre.wf_job_instance WHERE id = $1").bind(job).fetch_one(pool).await?;
    let txn = q(&txn.ok_or("the job has no Registry transaction")?);
    let forms: i64 = sqlx::query_scalar("SELECT count(*) FROM cadastre.title_deed_plan WHERE wf_job_instance_id = $1").bind(job).fetch_one(pool).await?;
    // The plan files: the ones sent to the Registry, and any PDF/JPG plan made for the job.
    let files: Vec<(uuid::Uuid, String, String, String)> = sqlx::query_as(
        "SELECT id, file_name, store, store_path FROM cadastre.wf_job_file WHERE job_instance_id = $1 AND store = 'cadastre' \
           AND (file_name ILIKE '%title deed%' OR id::text IN (SELECT payload->>'file_id' FROM cadastre.audit_log WHERE entity = 'wf_job_instance' AND entity_id = $2 AND action = 'registry_plan_sent'))")
        .bind(job).bind(job.to_string()).fetch_all(pool).await?;
    let sent: i64 = sqlx::query_scalar("SELECT count(*) FROM cadastre.audit_log WHERE entity = 'wf_job_instance' AND entity_id = $1 AND action = 'registry_plan_sent'")
        .bind(job.to_string()).fetch_one(pool).await?;
    let t = one(session, &format!("select t.Id from com.landsystems.lrs.model.Transaction t where t.TransactionNumber = '{txn}'")).await?;
    let tid: i64 = t.first().and_then(|r| r.first()).ok_or(format!("{txn} not found in the Registry"))?.trim().parse()?;
    // The plan documents this Cadastre filed: type Title Deed Plan, comments "Survey Plan…", filed
    // since the first send from here.
    let first_sent: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar(
        "SELECT min(created_at) FROM cadastre.audit_log WHERE entity = 'wf_job_instance' AND entity_id = $1 AND action = 'registry_plan_sent'").bind(job.to_string()).fetch_one(pool).await?;
    let mut docs: Vec<String> = Vec::new();
    if sent > 0 {
        for hql in [
            format!("select d.id, d.Comments from com.landsystems.lrs.model.IncomingDocument d where d.Transaction.id = {tid} and d.DocumentType.Code = 'sd_titledeedPlan'"),
            format!("select d.id, d.Comments from com.landsystems.lrs.model.Transaction t join t.IncomingDocuments d where t.id = {tid} and d.DocumentType.Code = 'sd_titledeedPlan'"),
            format!("select d.id, d.Comments from com.landsystems.lrs.model.Transaction t join t.Documents d where t.id = {tid} and d.DocumentType.Code = 'sd_titledeedPlan'"),
        ] {
            if let Ok(rows) = session.query(&hql, 50).await {
                docs = rows.iter().filter(|r| r.get(1).is_some_and(|c| c.trim().starts_with("Survey Plan"))).filter_map(|r| r.first().map(|v| v.trim().to_string())).collect();
                break;
            }
        }
    }
    println!("job {job} ({txn}): {forms} saved title deed form(s), {} plan file(s) [{}], {} plan document(s) in the Registry [{}] (sent {sent} time(s), first {:?})",
        files.len(), files.iter().map(|f| f.1.as_str()).collect::<Vec<_>>().join(", "), docs.len(), docs.join(", "), first_sent);
    if !apply {
        return Ok(());
    }
    let mut removed = Vec::new();
    let mut kept = Vec::new();
    for d in &docs {
        let xml = format!("<int:entities><lrs:className>com.landsystems.lrs.model.IncomingDocument</lrs:className><lrs:status>deleted</lrs:status>\
                           <lrs:properties><lrs:name>Id</lrs:name><lrs:value>{}</lrs:value></lrs:properties></int:entities>", lrs::escape(d));
        match session.save_entity(tid, &xml).await {
            Ok(()) => removed.push(d.clone()),
            // A delete answers with no items: it is done.
            Err(e) if e.to_string().contains("no items returned") => removed.push(d.clone()),
            Err(e) => {
                // The Registry refused a delete: mark it withdrawn instead, so it is not used.
                println!("  document {d}: the Registry refused the delete ({e}); marking it withdrawn");
                session.save_entity(tid, &lrs::entity_xml("IncomingDocument", &[("Id", Some(d)), ("Comments", Some("Survey Plan - WITHDRAWN (trial plan removed)"))])).await?;
                kept.push(d.clone());
            }
        }
    }
    let root = std::path::PathBuf::from(std::env::var("CADASTRE_FILES_DIR").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "files".into()));
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM cadastre.title_deed_plan WHERE wf_job_instance_id = $1").bind(job).execute(&mut *tx).await?;
    for (id, name, _, rel) in &files {
        sqlx::query("DELETE FROM cadastre.wf_job_file WHERE id = $1").bind(id).execute(&mut *tx).await?;
        let path = root.join(rel);
        match std::fs::remove_file(&path) { Ok(()) => println!("  deleted {name} ({})", path.display()), Err(e) => println!("  {name}: file not removed ({e})") }
    }
    cadastre_server::audit::append(&mut tx, "administrator", "title_deed_plan_deleted", "wf_job_instance", Some(&job.to_string()), json!({
        "transaction_number": txn, "forms": forms, "files": files.iter().map(|f| &f.1).collect::<Vec<_>>(),
        "registry_documents_deleted": removed, "registry_documents_withdrawn": kept, "reason": "trial: title deed plan removed",
    })).await?;
    tx.commit().await?;
    println!("  Registry documents deleted: {removed:?}; marked withdrawn: {kept:?}");
    Ok(())
}
