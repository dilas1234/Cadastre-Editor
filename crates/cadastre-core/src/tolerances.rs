//! Configurable tolerances for checks (SRS FR-ADM-03).
//!
//! Defaults here are provisional starting values. The real figures PLAGIS applies are to be
//! confirmed (SRS §9) and are stored as configuration on the server, not hard-coded.

use serde::{Deserialize, Serialize};

/// The set of tolerances that gate submission of a job.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Tolerances {
    /// Minimum acceptable traverse precision ratio denominator (e.g. `10_000.0` for 1:10,000).
    pub closure_precision: f64,
    /// Maximum acceptable fractional difference between computed and stated area (e.g. `0.01`).
    pub area_fraction: f64,
    /// Search distance for snapping/join proposals when connecting to the fabric (metres).
    pub join_distance: f64,
    /// Minimum parcel/gap area; anything smaller is a sliver (m²).
    pub sliver_area: f64,
    /// Distance under which two beacons are considered duplicates (metres).
    pub duplicate_beacon: f64,
    /// Maximum acceptable distance residual after adjustment (metres).
    pub distance_residual: f64,
    /// Maximum acceptable bearing residual after adjustment (seconds of arc).
    pub bearing_residual_sec: f64,
}

impl Default for Tolerances {
    fn default() -> Self {
        Self {
            closure_precision: 10_000.0,
            area_fraction: 0.01,
            join_distance: 0.5,
            sliver_area: 1.0,
            duplicate_beacon: 0.01,
            distance_residual: 0.05,
            bearing_residual_sec: 30.0,
        }
    }
}
