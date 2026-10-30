//! Read-only import of the Aumentum **ArcSDE versioned geodatabase** into the Cadastre schema.
//!
//! The source and its quirks are documented in `docs/aumentum-import.md`. The crucial one: a
//! feature class's current rows are *not* its business table — they must be reconstructed from
//! the versioned delta tables for the DEFAULT version (see [`sde`]).
//!
//! This crate is split so the risky, DB-touching part is isolated:
//! - pure, DB-free, unit-tested logic — [`sde`] (SQL builder), [`geometry`] (WKT), [`status`],
//!   [`model`], [`exceptions`] — is always compiled;
//! - the live SQL Server reader ([`client`]) is behind the `live` feature and reads only a
//!   **restored, non-production** copy of the `sde` database. It never writes to Aumentum.

pub mod exceptions;
pub mod geometry;
pub mod model;
pub mod sde;
pub mod status;

#[cfg(feature = "live")]
pub mod client;

/// Everything the importer can fail with.
#[derive(Debug, thiserror::Error, Clone, PartialEq)]
pub enum ImportError {
    /// A parcel status code was not one of the known values (1..=4).
    #[error("unknown parcel status code {0} (expected 1=pending, 2=active, 3=historic, 4=archived)")]
    UnknownStatus(i16),

    /// A WKT geometry string could not be parsed.
    #[error("invalid WKT geometry: {0}")]
    InvalidWkt(String),

    /// A live-import runtime error (connection, query, decoding). Only from the `live` client.
    #[error("{0}")]
    Runtime(String),
}

/// Crate result alias.
pub type Result<T> = core::result::Result<T, ImportError>;
