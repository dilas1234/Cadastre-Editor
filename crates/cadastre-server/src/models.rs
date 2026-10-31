//! Request/response DTOs. These are the API's stable surface; they are deliberately
//! decoupled from both the database rows and the `cadastre-core` types.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
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

/// A beacon as returned by the API. Coordinates are easting/northing in EPSG:32632.
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
    /// Easting (metres, EPSG:32632).
    pub easting: f64,
    /// Northing (metres, EPSG:32632).
    pub northing: f64,
    /// A-priori 1σ accuracy (metres); defaults to 0.05 if omitted.
    pub accuracy: Option<f64>,
    /// Whether this is a control point; defaults to false.
    pub is_control: Option<bool>,
}

/// A parcel as returned by the API. The boundary is GeoJSON in EPSG:32632.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct ParcelDto {
    /// Server-assigned identifier.
    pub id: Uuid,
    /// Official parcel number (UPIN). `null` for draft parcels not yet posted.
    pub parcel_number: Option<String>,
    /// Owning job, if captured under one.
    pub job_id: Option<Uuid>,
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

/// Payload to capture a draft parcel under a job (geometry as GeoJSON).
#[derive(Clone, Debug, Deserialize, ToSchema)]
pub struct NewParcel {
    /// Boundary polygon as GeoJSON (in the server CRS).
    #[schema(value_type = Object)]
    pub geometry: serde_json::Value,
    /// Area stated on the plan (m²), optional.
    pub stated_area: Option<f64>,
}

/// Query parameters for parcel search (FR-VIEW).
#[derive(Clone, Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ParcelSearch {
    /// Parcel number to find (case- and space-insensitive): a full UPIN, a short plot number
    /// (`5621` finds `PL000000005621` first), or any part of the number.
    pub number: Option<String>,
    /// Exact status (`draft` | `posted` | …).
    pub status: Option<String>,
    /// Bounding box `minx,miny,maxx,maxy` in the server CRS.
    pub bbox: Option<String>,
}

/// Result of promoting an import run into the live fabric.
#[derive(Clone, Copy, Debug, Serialize, ToSchema)]
pub struct PromoteResult {
    /// The import run promoted.
    pub run_id: i64,
    /// Active parcels inserted/updated into `cadastre.parcel`.
    pub parcels_promoted: usize,
    /// Beacons inserted/updated into `cadastre.beacon`.
    pub beacons_promoted: usize,
}

/// Result of posting a job's parcels.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct PostResult {
    /// Number of parcels posted.
    pub posted: usize,
    /// Parcel numbers assigned/confirmed, in order.
    pub parcel_numbers: Vec<String>,
}

/// An audit-log entry (summary) for listing.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct AuditEntryDto {
    /// Monotonic id.
    pub id: i64,
    /// Who performed the action.
    pub actor: String,
    /// Action name.
    pub action: String,
    /// Entity kind.
    pub entity: String,
    /// Entity id, if any.
    pub entity_id: Option<String>,
    /// This entry's chain hash.
    pub hash: String,
    /// When it was recorded.
    pub created_at: DateTime<Utc>,
}

/// Result of verifying the audit chain's integrity.
#[derive(Clone, Copy, Debug, Serialize, ToSchema)]
pub struct AuditVerifyDto {
    /// Whether the whole chain is intact.
    pub ok: bool,
    /// Number of entries checked.
    pub checked: usize,
    /// Id of the first broken entry, if any.
    pub first_bad: Option<i64>,
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

// ---------------------------------------------------------------------------
// Phase 4: live survey check (FR-COG-02/03, FR-SUR-*).
// ---------------------------------------------------------------------------

/// A planar coordinate (easting/northing, metres, EPSG:32632).
#[derive(Clone, Copy, Debug, Deserialize, Serialize, ToSchema)]
pub struct PointDto {
    /// Easting (metres).
    pub x: f64,
    /// Northing (metres).
    pub y: f64,
}

/// One traverse leg. `bearing`/`chord_bearing` are whole-circle **decimal degrees**.
#[derive(Clone, Copy, Debug, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum LegDto {
    /// A straight course.
    Line {
        /// Whole-circle bearing (decimal degrees).
        bearing: f64,
        /// Distance (metres).
        distance: f64,
    },
    /// A circular arc, reduced to its chord.
    Curve {
        /// Arc radius (metres).
        radius: f64,
        /// Arc length (metres).
        arc_length: f64,
        /// Chord whole-circle bearing (decimal degrees).
        chord_bearing: f64,
    },
}

/// Request to re-validate a traverse against the server's active tolerances.
#[derive(Clone, Debug, Deserialize, ToSchema)]
pub struct SurveyCheckRequest {
    /// Start beacon coordinate.
    pub start: PointDto,
    /// Optional start beacon number.
    pub start_beacon: Option<String>,
    /// Ordered legs captured so far.
    pub legs: Vec<LegDto>,
    /// Optional stated area from the plan (m²).
    pub stated_area: Option<f64>,
}

/// Closure summary with friendly units.
#[derive(Clone, Copy, Debug, Serialize, ToSchema)]
pub struct ClosureDto {
    /// Easting component of the misclosure (m).
    pub misclosure_east: f64,
    /// Northing component of the misclosure (m).
    pub misclosure_north: f64,
    /// Linear misclosure (m).
    pub linear_misclosure: f64,
    /// Bearing of the misclosure vector (decimal degrees).
    pub misclosure_bearing_deg: f64,
    /// Perimeter used for the precision ratio (m).
    pub perimeter: f64,
    /// Precision-ratio denominator (1:N). `null` means an exact closure.
    pub precision_ratio: Option<f64>,
}

/// Area comparison with friendly fields.
#[derive(Clone, Copy, Debug, Serialize, ToSchema)]
pub struct AreaCheckDto {
    /// Computed area (m²).
    pub computed: f64,
    /// Stated area (m²).
    pub stated: f64,
    /// `computed - stated` (m²).
    pub difference: f64,
    /// Absolute fractional difference.
    pub fraction: f64,
}

/// Tolerance pass/fail flags for the figure.
#[derive(Clone, Copy, Debug, Serialize, ToSchema)]
pub struct SurveyChecksDto {
    /// The loop returns onto its start within the join distance.
    pub closes_on_start: bool,
    /// The precision ratio meets the configured minimum.
    pub meets_precision: bool,
    /// Area within tolerance of the stated area (`null` if not checkable).
    pub area_within_tolerance: Option<bool>,
}

// ---------------------------------------------------------------------------
// Phase 5: fabric topology validation + least-squares adjustment.
// ---------------------------------------------------------------------------

/// Request to validate a set of parcel rings + beacons against the active tolerances.
#[derive(Clone, Debug, Deserialize, ToSchema)]
pub struct FabricValidateRequest {
    /// Parcel boundary rings (each a list of points, open or closed).
    pub parcels: Vec<Vec<PointDto>>,
    /// Beacon coordinates to check for duplicates.
    #[serde(default)]
    pub beacons: Vec<PointDto>,
}

/// An overlap between two parcels (by index).
#[derive(Clone, Copy, Debug, Serialize, ToSchema)]
pub struct OverlapDto {
    /// First parcel index.
    pub a: usize,
    /// Second parcel index.
    pub b: usize,
    /// Overlapping area (m²).
    pub area: f64,
}

/// Topology findings (FR-ADJ-04).
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct TopologyReportDto {
    /// Parcels with fewer than three distinct vertices.
    pub invalid_rings: Vec<usize>,
    /// Parcels below the sliver-area tolerance.
    pub slivers: Vec<usize>,
    /// Overlapping parcel pairs.
    pub overlaps: Vec<OverlapDto>,
    /// Beacon index pairs closer than the duplicate tolerance.
    pub duplicate_beacons: Vec<[usize; 2]>,
    /// Whether the fabric is free of all of the above.
    pub is_clean: bool,
}

/// One observed line for the adjustment (bearing in decimal degrees).
#[derive(Clone, Copy, Debug, Deserialize, ToSchema)]
pub struct ObservationDto {
    /// "From" beacon index.
    pub from: usize,
    /// "To" beacon index.
    pub to: usize,
    /// Measured distance (m).
    pub distance: f64,
    /// Measured whole-circle bearing (decimal degrees).
    pub bearing: f64,
    /// Distance std-dev (m) for weighting.
    pub sigma_distance_m: f64,
    /// Bearing std-dev (arcseconds) for weighting.
    pub sigma_bearing_sec: f64,
}

/// Request for a least-squares network adjustment (FR-ADJ-01..03).
#[derive(Clone, Debug, Deserialize, ToSchema)]
pub struct AdjustRequest {
    /// Initial coordinates per beacon.
    pub coords: Vec<PointDto>,
    /// Control mask: `true` beacons are held fixed.
    pub fixed: Vec<bool>,
    /// Observed lines.
    pub observations: Vec<ObservationDto>,
    /// Max iterations (default 50).
    pub max_iter: Option<usize>,
    /// Convergence threshold on the largest shift, metres (default 1e-4).
    pub tol: Option<f64>,
}

/// One line residual after adjustment.
#[derive(Clone, Copy, Debug, Serialize, ToSchema)]
pub struct LineResidualDto {
    /// Observation index.
    pub observation: usize,
    /// Distance residual (m).
    pub distance_residual: f64,
    /// Bearing residual (arcseconds).
    pub bearing_residual_sec: f64,
}

/// Adjustment result.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct AdjustResponse {
    /// Adjusted coordinates (fixed beacons unchanged).
    pub coords: Vec<PointDto>,
    /// Per-line residuals.
    pub residuals: Vec<LineResidualDto>,
    /// Iterations taken.
    pub iterations: usize,
    /// Largest shift in the final iteration (m).
    pub final_shift: f64,
    /// Observation indices flagged as likely blunders against the active tolerances.
    pub blunders: Vec<usize>,
}

/// Authoritative live-check result (mirrors `cadastre_core::survey::SurveySummary`).
#[derive(Clone, Copy, Debug, Serialize, ToSchema)]
pub struct SurveyCheckResponse {
    /// Number of legs.
    pub leg_count: usize,
    /// Number of vertices (legs + 1).
    pub vertex_count: usize,
    /// Perimeter (m).
    pub perimeter: f64,
    /// Closure, or `null` with no legs.
    pub closure: Option<ClosureDto>,
    /// Computed area (m²), or `null` with fewer than 3 vertices.
    pub area: Option<f64>,
    /// Area comparison, or `null` if not checkable.
    pub area_check: Option<AreaCheckDto>,
    /// Tolerance checks.
    pub checks: SurveyChecksDto,
}

/// The authenticated caller (`GET /api/v1/me`).
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct MeDto {
    /// Display name (`preferred_username`, falling back to the subject).
    pub username: String,
    /// Subject identifier (`sub`).
    pub subject: String,
    /// Realm roles.
    pub roles: Vec<String>,
}

/// A Catalog folder connection (`GET /api/v1/catalog`).
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct CatalogFolderDto {
    /// Index to pass as `folder` when listing items.
    pub id: usize,
    /// Display name, e.g. the UNC name of the share.
    pub name: String,
}
