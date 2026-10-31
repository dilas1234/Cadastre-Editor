//! `cadastre-import` — read a restored Aumentum `sde` database and stage it into PostGIS.
//!
//! Built only with `--features live`. It reconstructs the DEFAULT-version parcels + control
//! coordinates from MSSQL (read-only — never writes to Aumentum), validates/stages them, and
//! prints the import report. If `CADASTRE_IMPORT_PG` is set it then loads the staged records and
//! exceptions into the `import` schema of that PostGIS; otherwise it is a read-only dry run.
//! Configure via env:
//!
//!   CADASTRE_IMPORT_HOST (default 127.0.0.1)   CADASTRE_IMPORT_PORT (default 1433)
//!   CADASTRE_IMPORT_DB   (default sde)          CADASTRE_IMPORT_USER / CADASTRE_IMPORT_PASSWORD
//!   CADASTRE_IMPORT_PG   (optional target, e.g. postgres://cadastre:cadastre@localhost:5432/cadastre)

use std::env;

use cadastre_import::client::{ConnectOptions, ImportSource};
use cadastre_import::exceptions::stage_parcels;
use cadastre_import::sink::Sink;
use cadastre_import::Result;

fn env_or(key: &str, default: &str) -> String {
    env::var(key).unwrap_or_else(|_| default.to_string())
}

#[tokio::main]
async fn main() -> Result<()> {
    let opts = ConnectOptions {
        host: env_or("CADASTRE_IMPORT_HOST", "127.0.0.1"),
        port: env_or("CADASTRE_IMPORT_PORT", "1433").parse().unwrap_or(1433),
        database: env_or("CADASTRE_IMPORT_DB", "sde"),
        username: env_or("CADASTRE_IMPORT_USER", "sa"),
        password: env::var("CADASTRE_IMPORT_PASSWORD").unwrap_or_default(),
    };

    eprintln!(
        "connecting to {}:{}/{} as {} (read-only dry run)",
        opts.host, opts.port, opts.database, opts.username
    );
    let mut source = ImportSource::connect(&opts).await?;

    let parcels = source.fetch_parcels().await?;
    let report = stage_parcels(&parcels);
    println!("Parcels: {} raw rows", report.total);
    println!("  staged (clean, unique): {}", report.staged_count());
    println!("  exceptions:             {}", report.exception_count());
    println!("  by status: {:?}", report.status_counts);

    // Group exception reasons for a quick overview.
    let mut reasons: std::collections::BTreeMap<String, usize> = Default::default();
    for e in &report.exceptions {
        *reasons.entry(format!("{:?}", e.kind)).or_default() += 1;
    }
    println!("  exception reasons: {reasons:?}");

    let coords = source.fetch_coordinates().await?;
    let beacons: Vec<_> = coords.iter().filter_map(|c| c.validate().ok()).collect();
    println!("Control beacons: {} of {} rows valid", beacons.len(), coords.len());

    match env::var("CADASTRE_IMPORT_PG") {
        Ok(pg) if !pg.is_empty() => {
            eprintln!("loading into PostGIS import schema …");
            let sink = Sink::connect(&pg).await?;
            sink.ensure_schema().await?;
            let summary = sink.load(&report, &beacons).await?;
            println!(
                "Loaded run {}: {} parcels, {} beacons, {} exceptions into schema 'import'.",
                summary.run_id, summary.parcels, summary.beacons, summary.exceptions
            );
        }
        _ => println!("CADASTRE_IMPORT_PG not set — dry run only, nothing written."),
    }

    Ok(())
}
