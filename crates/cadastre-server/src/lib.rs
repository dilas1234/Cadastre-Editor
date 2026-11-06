//! PLAGIS Cadastre Editor server library. The `cadastre-server` binary is a thin wrapper
//! around this; integration tests drive the same `routes::router` in-process.

pub mod audit;
pub mod auth;
pub mod catalog;
pub mod config;
pub mod db;
pub mod domain;
pub mod error;
pub mod idp;
pub mod jwks;
pub mod models;
pub mod numbering;
pub mod openapi;
pub mod pdf;
pub mod raster;
pub mod report;
pub mod routes;
pub mod shp;
pub mod mail;
pub mod state;
pub mod lrs;
pub mod aumentum;
pub mod registry;
#[cfg(feature = "registry-admin")]
pub mod regadmin;
pub mod registry_sync;
pub mod upin_watch;
pub mod titledeed;
pub mod wfengine;
