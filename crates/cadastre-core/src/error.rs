//! Error type for the geometry engine.

use thiserror::Error;

/// Result alias used throughout `cadastre-core`.
pub type Result<T> = core::result::Result<T, CoreError>;

/// All ways a geometry computation can fail.
#[derive(Debug, Error, Clone, PartialEq)]
pub enum CoreError {
    /// A traverse did not carry enough courses for the requested operation.
    #[error("traverse needs at least {min} course(s), got {got}")]
    TooFewCourses { min: usize, got: usize },

    /// A polygon ring had fewer than three distinct vertices.
    #[error("polygon ring is degenerate: {0} distinct vertices (need >= 3)")]
    DegeneratePolygon(usize),

    /// A least-squares adjustment did not converge within the iteration budget.
    #[error("least-squares adjustment did not converge after {0} iterations")]
    NoConvergence(usize),

    /// The normal equation matrix was singular (network under-constrained).
    #[error("normal matrix is singular — the network is under-constrained; fix more control")]
    SingularNormalMatrix,

    /// An adjustment was requested with no fixed control point.
    #[error("adjustment needs at least one fixed control point")]
    NoControl,

    /// An observation referenced a beacon index outside the coordinate list.
    #[error("observation references beacon index {0}, which is out of range")]
    BeaconIndexOutOfRange(usize),

    /// A value was not finite where a finite value was required.
    #[error("expected a finite value for {0}")]
    NotFinite(&'static str),
}
