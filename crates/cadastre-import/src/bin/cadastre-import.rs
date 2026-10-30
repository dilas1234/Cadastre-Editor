//! `cadastre-import` — read a restored Aumentum `sde` database and report what would import.
//!
//! Built only with `--features live`. This is a **read-only dry run**: it connects, reconstructs
//! the DEFAULT-version parcels + control coordinates, validates/stages them, and prints the import
//! report. It writes nothing — neither to Aumentum nor (yet) to PostGIS; the staging load is the
//! next step. Configure via env:
//!
//!   CADASTRE_IMPORT_HOST (default 127.0.0.1)   CADASTRE_IMPORT_PORT (default 1433)
//!   CADASTRE_IMPORT_DB   (default sde)          CADASTRE_IMPORT_USER / CADASTRE_IMPORT_PASSWORD

use std::env;

use cadastre_import::client::{ConnectOptions, ImportSource};
use cadastre_import::exceptions::stage_parcels;
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

    Ok(())
}
