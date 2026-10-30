//! Live SQL Server reader (feature `live`).
//!
//! Reads a **restored, non-production** copy of the Aumentum `sde` database with tiberius and runs
//! the reconstruction SQL from [`crate::sde`]. It issues only `SELECT`s — it never writes to the
//! source. Connection details come from the caller (see the `cadastre-import` binary).

use futures_util::TryStreamExt;
use tiberius::{AuthMethod, Client, Config};
use tokio::net::TcpStream;
use tokio_util::compat::{Compat, TokioAsyncWriteCompatExt};

use crate::model::{RawCoordinate, RawParcel};
use crate::{sde, ImportError, Result};

fn rt(e: impl std::fmt::Display) -> ImportError {
    ImportError::Runtime(e.to_string())
}

/// How to reach the restored `sde` database.
#[derive(Clone, Debug)]
pub struct ConnectOptions {
    /// Server host.
    pub host: String,
    /// Server port (SQL Server default 1433).
    pub port: u16,
    /// Database name (the restored `sde`).
    pub database: String,
    /// SQL login user.
    pub username: String,
    /// SQL login password.
    pub password: String,
}

/// A connected, read-only import source.
pub struct ImportSource {
    client: Client<Compat<TcpStream>>,
}

impl ImportSource {
    /// Connect to the restored database using SQL authentication.
    pub async fn connect(opts: &ConnectOptions) -> Result<Self> {
        let mut config = Config::new();
        config.host(&opts.host);
        config.port(opts.port);
        config.database(&opts.database);
        config.authentication(AuthMethod::sql_server(&opts.username, &opts.password));
        // A restored dev copy typically has a self-signed cert; trust it.
        config.trust_cert();

        let tcp = TcpStream::connect(config.get_addr()).await.map_err(rt)?;
        tcp.set_nodelay(true).ok();
        let client = Client::connect(config, tcp.compat_write()).await.map_err(rt)?;
        Ok(Self { client })
    }

    /// Reconstruct and fetch the DEFAULT-version parcels.
    pub async fn fetch_parcels(&mut self) -> Result<Vec<RawParcel>> {
        let sql = sde::current_rows_sql(&sde::PARCELS, sde::PARCEL_COLUMNS);
        let mut stream = self.client.simple_query(sql).await.map_err(rt)?;
        let mut out = Vec::new();
        while let Some(item) = stream.try_next().await.map_err(rt)? {
            if let tiberius::QueryItem::Row(row) = item {
                out.push(RawParcel {
                    object_id: row.get::<i32, _>("OBJECTID").unwrap_or_default(),
                    upin: row.get::<&str, _>("UPIN").map(str::to_string),
                    status_code: row.get::<i16, _>("Status").unwrap_or_default(),
                    job_id: row.get::<&str, _>("JobID").map(str::to_string),
                    plan_no: row.get::<&str, _>("PlanNo").map(str::to_string),
                    cad_zone: row.get::<&str, _>("CadZone").map(str::to_string),
                    plot_no: row.get::<&str, _>("PlotNo").map(str::to_string),
                    block_no: row.get::<&str, _>("BlockNo").map(str::to_string),
                    wkt: row.get::<&str, _>("wkt").map(str::to_string),
                    srid: row.get::<i32, _>("srid").unwrap_or(32632),
                });
            }
        }
        Ok(out)
    }

    /// Fetch the (non-versioned) control coordinates.
    pub async fn fetch_coordinates(&mut self) -> Result<Vec<RawCoordinate>> {
        let sql = sde::current_rows_sql(&sde::COORDINATES, sde::COORDINATE_COLUMNS);
        let mut stream = self.client.simple_query(sql).await.map_err(rt)?;
        let mut out = Vec::new();
        while let Some(item) = stream.try_next().await.map_err(rt)? {
            if let tiberius::QueryItem::Row(row) = item {
                out.push(RawCoordinate {
                    alias: row.get::<&str, _>("Alias").map(str::to_string),
                    x: row.get::<f64, _>("X").unwrap_or(f64::NAN),
                    y: row.get::<f64, _>("Y").unwrap_or(f64::NAN),
                });
            }
        }
        Ok(out)
    }
}
