//! OpenAPI document assembled from the `#[utoipa::path]` annotations on each handler.

use utoipa::OpenApi;

use crate::models::{
    AdjustRequest, AdjustResponse, AreaCheckDto, AuditEntryDto, AuditVerifyDto, BeaconDto,
    ClosureDto, FabricValidateRequest, JobDto, JobEventDto, JobTransition, LegDto, LineResidualDto,
    CatalogDatabaseDto, CatalogFolderDto, LockDto, MeDto, NewBeacon, NewJob, NewLock, NewParcel, NewSession, NewStage, NewTransition,
    ObservationDto, OverlapDto, ParcelDto, PointDto, PostResult, PromoteResult, SessionDto,
    StageDto, SurveyCheckRequest, SurveyCheckResponse, SurveyChecksDto, TolerancesDto,
    TopologyReportDto, TransitionDto, WorkflowDto,
};
use crate::routes::{
    audit, beacons, catalog, fabric, health, jobs, me, parcels, promote, reports, sessions, survey, tolerances,
    workflow,
};

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
        me::me,
        catalog::folders,
        catalog::items,
        catalog::databases,
        catalog::database_items,
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
        fabric::validate,
        fabric::adjust,
        parcels::create_job_parcel,
        parcels::search_parcels,
        jobs::post_job,
        audit::list_audit,
        audit::verify_audit,
        reports::job_report,
        reports::site_plan,
        promote::promote_run,
    ),
    components(schemas(
        MeDto, crate::catalog::CatalogItem, crate::catalog::ItemKind, CatalogFolderDto,
        CatalogDatabaseDto, crate::catalog::FeatureClass, TolerancesDto, BeaconDto, NewBeacon, ParcelDto,
        WorkflowDto, StageDto, TransitionDto, NewStage, NewTransition,
        JobDto, NewJob, JobTransition, JobEventDto,
        SessionDto, NewSession, LockDto, NewLock,
        PointDto, LegDto, SurveyCheckRequest, SurveyCheckResponse,
        ClosureDto, AreaCheckDto, SurveyChecksDto,
        FabricValidateRequest, TopologyReportDto, OverlapDto,
        AdjustRequest, AdjustResponse, ObservationDto, LineResidualDto,
        NewParcel, PostResult, AuditEntryDto, AuditVerifyDto, PromoteResult,
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
        (name = "fabric", description = "Topology validation + least-squares adjustment (FR-ADJ-01..04)"),
        (name = "audit", description = "Hash-chained audit log (NFR-07/11)"),
        (name = "output", description = "PDF job reports and site plans (FR-OUT-*)"),
        (name = "import", description = "Promote staged Aumentum imports into the live fabric"),
    )
)]
pub struct ApiDoc;
