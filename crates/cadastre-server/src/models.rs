//! Request/response DTOs. These are the API's stable surface; they are deliberately
//! decoupled from both the database rows and the `cadastre-core` types.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// Configurable tolerances (FR-ADM-03). Mirrors `cadastre_core::tolerances::Tolerances`.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, ToSchema)]
pub struct TolerancesDto {
    /// Minimum traverse precision-ratio denominator (e.g. 10000 for 1:10,000).
    pub closure_precision: f64,
    /// Maximum fractional difference between computed and stated area.
    pub area_fraction: f64,
    /// Snap/join search distance (metres).
    pub join_distance: f64,
    /// Minimum polygon/gap area; smaller is a sliver (m²).
    pub sliver_area: f64,
    /// Distance under which two beacons are duplicates (metres).
    pub duplicate_beacon: f64,
    /// Maximum post-adjustment distance residual (metres).
    pub distance_residual: f64,
    /// Maximum post-adjustment bearing residual (arcseconds).
    pub bearing_residual_sec: f64,
}

impl From<cadastre_core::tolerances::Tolerances> for TolerancesDto {
    fn from(t: cadastre_core::tolerances::Tolerances) -> Self {
        Self {
            closure_precision: t.closure_precision,
            area_fraction: t.area_fraction,
            join_distance: t.join_distance,
            sliver_area: t.sliver_area,
            duplicate_beacon: t.duplicate_beacon,
            distance_residual: t.distance_residual,
            bearing_residual_sec: t.bearing_residual_sec,
        }
    }
}

impl From<TolerancesDto> for cadastre_core::tolerances::Tolerances {
    fn from(d: TolerancesDto) -> Self {
        Self {
            closure_precision: d.closure_precision,
            area_fraction: d.area_fraction,
            join_distance: d.join_distance,
            sliver_area: d.sliver_area,
            duplicate_beacon: d.duplicate_beacon,
            distance_residual: d.distance_residual,
            bearing_residual_sec: d.bearing_residual_sec,
        }
    }
}

/// A beacon as returned by the API. Coordinates are easting/northing in EPSG:26392.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct BeaconDto {
    /// Server-assigned identifier.
    pub id: Uuid,
    /// Beacon number (unique).
    pub number: String,
    /// Easting (metres).
    pub easting: f64,
    /// Northing (metres).
    pub northing: f64,
    /// A-priori 1σ coordinate accuracy (metres).
    pub accuracy: f64,
    /// Whether the beacon is a control point held fixed in adjustment.
    pub is_control: bool,
    /// Creation time.
    pub created_at: DateTime<Utc>,
    /// Last update time.
    pub updated_at: DateTime<Utc>,
}

/// Payload to create a beacon.
#[derive(Clone, Debug, Deserialize, ToSchema)]
pub struct NewBeacon {
    /// Beacon number (must be unique).
    pub number: String,
    /// Easting (metres, EPSG:26392).
    pub easting: f64,
    /// Northing (metres, EPSG:26392).
    pub northing: f64,
    /// A-priori 1σ accuracy (metres); defaults to 0.05 if omitted.
    pub accuracy: Option<f64>,
    /// Whether this is a control point; defaults to false.
    pub is_control: Option<bool>,
}

/// A parcel as returned by the API. The boundary is GeoJSON in EPSG:26392.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct ParcelDto {
    /// Server-assigned identifier.
    pub id: Uuid,
    /// Parcel number (unique).
    pub parcel_number: String,
    /// Boundary polygon as GeoJSON.
    #[schema(value_type = Object)]
    pub geometry: serde_json::Value,
    /// Area stated on the survey plan (m²), if known.
    pub stated_area: Option<f64>,
    /// Area computed from the geometry (m²), if known.
    pub computed_area: Option<f64>,
    /// Workflow status (draft | submitted | posted).
    pub status: String,
    /// Creation time.
    pub created_at: DateTime<Utc>,
    /// Last update time.
    pub updated_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Phase 3: workflow, jobs, edit sessions.
// ---------------------------------------------------------------------------

/// A workflow stage.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct StageDto {
    /// Stable code (e.g. `draft`).
    pub code: String,
    /// Display name.
    pub name: String,
    /// Ordering position.
    pub seq: i32,
    /// Whether a new job starts here.
    pub is_initial: bool,
    /// Whether the job is finished here.
    pub is_terminal: bool,
}

/// A workflow transition, by stage code.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct TransitionDto {
    /// Action name (e.g. `submit`).
    pub action: String,
    /// Stage the job must be in.
    pub from_stage: String,
    /// Stage the job moves to.
    pub to_stage: String,
    /// Role required to perform it.
    pub required_role: String,
}

/// The full workflow definition (FR-WF-01, FR-ADM-02).
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct WorkflowDto {
    /// Stages ordered by `seq`.
    pub stages: Vec<StageDto>,
    /// All transitions.
    pub transitions: Vec<TransitionDto>,
}

/// Payload to add a workflow stage (admin).
#[derive(Clone, Debug, Deserialize, ToSchema)]
pub struct NewStage {
    /// Stable code (unique).
    pub code: String,
    /// Display name.
    pub name: String,
    /// Ordering position.
    pub seq: i32,
    /// Whether the job is finished here. Defaults to false.
    pub is_terminal: Option<bool>,
}

/// Payload to add a workflow transition (admin).
#[derive(Clone, Debug, Deserialize, ToSchema)]
pub struct NewTransition {
    /// Action name.
    pub action: String,
    /// Source stage code.
    pub from_stage: String,
    /// Target stage code.
    pub to_stage: String,
    /// Role required to perform it.
    pub required_role: String,
}

/// A job (FR-JOB-*).
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct JobDto {
    /// Server-assigned identifier.
    pub id: Uuid,
    /// Human reference (unique).
    pub reference: String,
    /// Job type (capture | maintenance | subdivision | ...).
    pub job_type: String,
    /// Optional title.
    pub title: Option<String>,
    /// Current workflow stage code.
    pub status: String,
    /// Assignee subject, if any.
    pub assigned_to: Option<String>,
    /// Who created the job.
    pub created_by: String,
    /// Creation time.
    pub created_at: DateTime<Utc>,
    /// Last update time.
    pub updated_at: DateTime<Utc>,
}

/// Payload to create a job.
#[derive(Clone, Debug, Deserialize, ToSchema)]
pub struct NewJob {
    /// Human reference (must be unique).
    pub reference: String,
    /// Job type.
    pub job_type: String,
    /// Optional title.
    pub title: Option<String>,
    /// Optional assignee subject.
    pub assigned_to: Option<String>,
}

/// Payload to move a job along the workflow.
#[derive(Clone, Debug, Deserialize, ToSchema)]
pub struct JobTransition {
    /// The action to apply (must be valid from the current stage).
    pub action: String,
    /// Optional note recorded in the job history.
    pub note: Option<String>,
}

/// A job workflow history entry.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct JobEventDto {
    /// Monotonic event id.
    pub id: i64,
    /// Action applied.
    pub action: String,
    /// Stage moved from, if any.
    pub from_stage: Option<String>,
    /// Stage moved to.
    pub to_stage: String,
    /// Who performed it.
    pub actor: String,
    /// Optional note.
    pub note: Option<String>,
    /// When it happened.
    pub created_at: DateTime<Utc>,
}

/// An edit session.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct SessionDto {
    /// Session identifier.
    pub id: Uuid,
    /// Job the session is working on, if any.
    pub job_id: Option<Uuid>,
    /// Owning user subject.
    pub user_subject: String,
    /// Lifecycle status (active | released | expired).
    pub status: String,
    /// When the session opened.
    pub started_at: DateTime<Utc>,
    /// Last heartbeat.
    pub last_heartbeat_at: DateTime<Utc>,
}

/// Payload to open an edit session.
#[derive(Clone, Debug, Deserialize, ToSchema)]
pub struct NewSession {
    /// Optional job to scope the session to.
    pub job_id: Option<Uuid>,
}

/// An acquired resource lock.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct LockDto {
    /// Lock identifier.
    pub id: Uuid,
    /// Owning session.
    pub session_id: Uuid,
    /// Resource kind (parcel | beacon | job).
    pub resource_type: String,
    /// Resource identifier.
    pub resource_id: Uuid,
    /// When the lock was taken.
    pub acquired_at: DateTime<Utc>,
}

/// Payload to acquire a lock.
#[derive(Clone, Debug, Deserialize, ToSchema)]
pub struct NewLock {
    /// Resource kind (parcel | beacon | job).
    pub resource_type: String,
    /// Resource identifier.
    pub resource_id: Uuid,
}
