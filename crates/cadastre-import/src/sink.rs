//! PostGIS staging-load sink (feature `sink`).
//!
//! Writes validated records and the exceptions report of one import run into a dedicated `import`
//! schema (separate from the live `cadastre` tables), so results can be reviewed before promotion.
//! Everything in a run goes in a single transaction. Testable against a local PostGIS alone —
//! it does not need the MSSQL source.

use std::time::Duration;

use sqlx::postgres::PgPoolOptions;
use sqlx::{Executor, PgPool};

use crate::exceptions::ImportReport;
use crate::model::StagedBeacon;
use crate::{ImportError, Result};

fn rt(e: impl std::fmt::Display) -> ImportError {
    ImportError::Runtime(e.to_string())
}

/// DDL for the staging area. Idempotent, so the importer is self-sufficient against any PostGIS.
const SCHEMA_SQL: &str = "\
CREATE EXTENSION IF NOT EXISTS postgis;\n\
CREATE SCHEMA IF NOT EXISTS import;\n\
CREATE TABLE IF NOT EXISTS import.run (\n\
  id bigserial PRIMARY KEY,\n\
  started_at timestamptz NOT NULL DEFAULT now(),\n\
  parcels integer NOT NULL DEFAULT 0,\n\
  beacons integer NOT NULL DEFAULT 0,\n\
  exceptions integer NOT NULL DEFAULT 0);\n\
CREATE TABLE IF NOT EXISTS import.parcel (\n\
  id bigserial PRIMARY KEY,\n\
  run_id bigint NOT NULL REFERENCES import.run(id) ON DELETE CASCADE,\n\
  source_object_id integer NOT NULL,\n\
  parcel_number text,\n\
  status text NOT NULL,\n\
  job_id text, plan_no text, cad_zone text, plot_no text, block_no text,\n\
  computed_area double precision,\n\
  srid integer NOT NULL,\n\
  geom geometry(Geometry, 32632) NOT NULL);\n\
CREATE INDEX IF NOT EXISTS import_parcel_run_idx ON import.parcel(run_id);\n\
CREATE TABLE IF NOT EXISTS import.beacon (\n\
  id bigserial PRIMARY KEY,\n\
  run_id bigint NOT NULL REFERENCES import.run(id) ON DELETE CASCADE,\n\
  number text NOT NULL,\n\
  geom geometry(Point, 32632) NOT NULL);\n\
CREATE TABLE IF NOT EXISTS import.exception (\n\
  id bigserial PRIMARY KEY,\n\
  run_id bigint NOT NULL REFERENCES import.run(id) ON DELETE CASCADE,\n\
  source_object_id integer,\n\
  parcel_number text,\n\
  kind text NOT NULL);\n";

/// A connected staging sink.
pub struct Sink {
    pool: PgPool,
}

/// Counts written by one load.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoadSummary {
    /// The `import.run` id this load was recorded under.
    pub run_id: i64,
    /// Parcels staged.
    pub parcels: usize,
    /// Beacons staged.
    pub beacons: usize,
    /// Exceptions recorded.
    pub exceptions: usize,
}

impl Sink {
    /// Connect to the target PostGIS.
    pub async fn connect(database_url: &str) -> Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(8)
            .acquire_timeout(Duration::from_secs(10))
            .connect(database_url)
            .await
            .map_err(rt)?;
        Ok(Self { pool })
    }

    /// The underlying pool (for querying results back, e.g. in tests/reviews).
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Create the `import` schema and tables if they do not exist.
    pub async fn ensure_schema(&self) -> Result<()> {
        self.pool.execute(sqlx::raw_sql(SCHEMA_SQL)).await.map_err(rt)?;
        Ok(())
    }

    /// Load one run's staged parcels, beacons and exceptions in a single transaction.
    pub async fn load(&self, report: &ImportReport, beacons: &[StagedBeacon]) -> Result<LoadSummary> {
        let mut tx = self.pool.begin().await.map_err(rt)?;

        let run_id: i64 = sqlx::query_scalar(
            "INSERT INTO import.run (parcels, beacons, exceptions) VALUES ($1, $2, $3) RETURNING id",
        )
        .bind(report.staged.len() as i32)
        .bind(beacons.len() as i32)
        .bind(report.exceptions.len() as i32)
        .fetch_one(&mut *tx)
        .await
        .map_err(rt)?;

        for p in &report.staged {
            sqlx::query(
                "INSERT INTO import.parcel \
                 (run_id, source_object_id, parcel_number, status, job_id, plan_no, cad_zone, \
                  plot_no, block_no, computed_area, srid, geom) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11, ST_GeomFromText($12, $11))",
            )
            .bind(run_id)
            .bind(p.object_id)
            .bind(&p.parcel_number)
            .bind(p.status.as_str())
            .bind(&p.job_id)
            .bind(&p.plan_no)
            .bind(&p.cad_zone)
            .bind(&p.plot_no)
            .bind(&p.block_no)
            .bind(p.computed_area)
            .bind(p.srid)
            .bind(&p.wkt)
            .execute(&mut *tx)
            .await
            .map_err(rt)?;
        }

        for b in beacons {
            sqlx::query(
                "INSERT INTO import.beacon (run_id, number, geom) \
                 VALUES ($1, $2, ST_SetSRID(ST_MakePoint($3, $4), 32632))",
            )
            .bind(run_id)
            .bind(&b.number)
            .bind(b.easting)
            .bind(b.northing)
            .execute(&mut *tx)
            .await
            .map_err(rt)?;
        }

        for e in &report.exceptions {
            sqlx::query(
                "INSERT INTO import.exception (run_id, source_object_id, parcel_number, kind) \
                 VALUES ($1, $2, $3, $4)",
            )
            .bind(run_id)
            .bind(e.object_id)
            .bind(&e.parcel_number)
            .bind(format!("{:?}", e.kind))
            .execute(&mut *tx)
            .await
            .map_err(rt)?;
        }

        tx.commit().await.map_err(rt)?;
        Ok(LoadSummary {
            run_id,
            parcels: report.staged.len(),
            beacons: beacons.len(),
            exceptions: report.exceptions.len(),
        })
    }
}
