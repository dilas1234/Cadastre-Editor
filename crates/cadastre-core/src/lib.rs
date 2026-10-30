//! `cadastre-core` — the shared survey geometry engine for PLAGIS Cadastre Editor.
//!
//! One engine runs in the desktop editor, the web viewer and the server, so a
//! closure, area or topology check gives the same answer everywhere (SRS §3).
//!
//! Design rules (SRS §2.4, §6):
//! - Coordinates are `f64` in the projected CRS (metres); geometry is never generalised.
//! - No I/O, no DB, no OS APIs here — pure maths, so it compiles to native **and** wasm32.
//! - Bearings are stored as decimal seconds of arc internally; distances to the millimetre.

pub mod adjust;
pub mod area;
pub mod closure;
pub mod cogo;
pub mod crs;
pub mod error;
pub mod survey;
pub mod tolerances;
pub mod topology;
pub mod types;
pub mod units;

pub use error::{CoreError, Result};
pub use survey::{SurveyChecks, SurveySession, SurveySummary};
pub use types::{Beacon, Course, Curve, Point, Segment, Traverse};
pub use units::{Angle, Bearing};
