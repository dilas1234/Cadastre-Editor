//! OpenAPI document assembled from the `#[utoipa::path]` annotations on each handler.

use utoipa::OpenApi;

use crate::models::{
    AreaCheckDto, BeaconDto, ClosureDto, JobDto, JobEventDto, JobTransition, LegDto, LockDto,
    NewBeacon, NewJob, NewLock, NewSession, NewStage, NewTransition, ParcelDto, PointDto,
    SessionDto, StageDto, SurveyCheckRequest, SurveyCheckResponse, SurveyChecksDto, TolerancesDto,
    TransitionDto, WorkflowDto,
};
use crate::routes::{beacons, health, jobs, parcels, sessions, survey, tolerances, workflow};

/// The generated API description, served at `/api-docs/openapi.json`.
#[derive(OpenApi)]
#[openapi(
    info(
        title = "PLAGIS Cadastre Editor API",
        version = "0.1.0",
        description = "Parcel capture and maintenance server (Phases 2-3)."
    ),
    paths(
        health::health,
        health::ready,
        tolerances::get_tolerances,
        tolerances::put_tolerances,
        beacons::list_beacons,
        beacons::get_beacon,
        beacons::create_beacon,
        parcels::list_parcels,
        workflow::get_workflow,
        workflow::create_stage,
        workflow::create_transition,
        jobs::list_jobs,
        jobs::get_job,
        jobs::create_job,
        jobs::transition_job,
        jobs::list_job_events,
        sessions::open_session,
        sessions::heartbeat_session,
        sessions::release_session,
        sessions::list_locks,
        sessions::acquire_lock,
        sessions::release_lock,
        survey::check_survey,
    ),
    components(schemas(
        TolerancesDto, BeaconDto, NewBeacon, ParcelDto,
        WorkflowDto, StageDto, TransitionDto, NewStage, NewTransition,
        JobDto, NewJob, JobTransition, JobEventDto,
        SessionDto, NewSession, LockDto, NewLock,
        PointDto, LegDto, SurveyCheckRequest, SurveyCheckResponse,
        ClosureDto, AreaCheckDto, SurveyChecksDto,
    )),
    tags(
        (name = "ops", description = "Liveness and readiness"),
        (name = "config", description = "Configurable tolerances (FR-ADM-03)"),
        (name = "beacons", description = "Survey beacons"),
        (name = "parcels", description = "Parcels"),
        (name = "workflow", description = "Configurable workflow (FR-WF-01, FR-ADM-02)"),
        (name = "jobs", description = "Jobs and workflow transitions (FR-JOB-*, FR-WF-02/03)"),
        (name = "sessions", description = "Edit sessions and resource locks"),
        (name = "survey", description = "Live survey closure/area check (FR-COG-02/03)"),
    )
)]
pub struct ApiDoc;
