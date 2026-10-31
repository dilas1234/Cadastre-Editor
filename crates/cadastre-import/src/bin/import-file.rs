//! `import-file` — stage a read-only Aumentum dump into PostGIS (feature `sink`).
//!
//! Loads the `parcels.psv` / `beacons.psv` produced by the read-only `sqlcmd` export on the
//! Aumentum box, validates/stages them, and writes the run + exceptions into the `import` schema.
//!
//!   import-file <parcels.psv> <beacons.psv>
//!   env: CADASTRE_IMPORT_PG (target PostGIS, required)

use std::env;

use cadastre_import::exceptions::stage_parcels;
use cadastre_import::file_source::{parse_beacons, parse_parcels};
use cadastre_import::sink::Sink;
use cadastre_import::Result;

#[tokio::main]
async fn main() -> Result<()> {
    let mut args = env::args().skip(1);
    let parcels_path = args.next().expect("usage: import-file <parcels.psv> <beacons.psv>");
    let beacons_path = args.next().expect("usage: import-file <parcels.psv> <beacons.psv>");
    let pg = env::var("CADASTRE_IMPORT_PG").expect("CADASTRE_IMPORT_PG must be set");

    let parcel_text = std::fs::read_to_string(&parcels_path)
        .unwrap_or_else(|e| panic!("reading {parcels_path}: {e}"));
    let beacon_text = std::fs::read_to_string(&beacons_path)
        .unwrap_or_else(|e| panic!("reading {beacons_path}: {e}"));

    let raw_parcels = parse_parcels(&parcel_text);
    let raw_beacons = parse_beacons(&beacon_text);
    println!("parsed {} parcel rows, {} beacon rows", raw_parcels.len(), raw_beacons.len());

    let report = stage_parcels(&raw_parcels);
    let beacons: Vec<_> = raw_beacons.iter().filter_map(|b| b.validate().ok()).collect();
    println!(
        "staged {} parcels ({} exceptions); {} beacons valid. status: {:?}",
        report.staged_count(),
        report.exception_count(),
        beacons.len(),
        report.status_counts
    );

    let sink = Sink::connect(&pg).await?;
    sink.ensure_schema().await?;
    let summary = sink.load(&report, &beacons).await?;
    println!(
        "LOADED run {}: {} parcels, {} beacons, {} exceptions into schema 'import'.",
        summary.run_id, summary.parcels, summary.beacons, summary.exceptions
    );
    Ok(())
}
