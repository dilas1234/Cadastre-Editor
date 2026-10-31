//! HTTP routing.

pub mod audit;
pub mod beacons;
pub mod fabric;
pub mod health;
pub mod jobs;
pub mod parcels;
pub mod promote;
pub mod reports;
pub mod sessions;
pub mod survey;
pub mod tolerances;
pub mod workflow;

use axum::routing::{delete, get, post};
use axum::Router;
use tower_http::trace::TraceLayer;

use crate::state::AppState;

/// Build the application router with all routes and middleware.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health::health))
        .route("/ready", get(health::ready))
        .route(
            "/api/v1/tolerances",
            get(tolerances::get_tolerances).put(tolerances::put_tolerances),
        )
        .route(
            "/api/v1/beacons",
            get(beacons::list_beacons).post(beacons::create_beacon),
        )
        .route("/api/v1/beacons/{id}", get(beacons::get_beacon))
        .route("/api/v1/parcels", get(parcels::list_parcels))
        .route("/api/v1/parcels/search", get(parcels::search_parcels))
        // Live survey check (FR-COG-02/03)
        .route("/api/v1/survey/check", post(survey::check_survey))
        // Fabric topology validation + least-squares adjustment (FR-ADJ-01..04)
        .route("/api/v1/fabric/validate", post(fabric::validate))
        .route("/api/v1/fabric/adjust", post(fabric::adjust))
        // Workflow configuration (FR-WF-01, FR-ADM-02)
        .route("/api/v1/workflow", get(workflow::get_workflow))
        .route("/api/v1/workflow/stages", post(workflow::create_stage))
        .route(
            "/api/v1/workflow/transitions",
            post(workflow::create_transition),
        )
        // Jobs (FR-JOB-*, FR-WF-02/03)
        .route("/api/v1/jobs", get(jobs::list_jobs).post(jobs::create_job))
        .route("/api/v1/jobs/{id}", get(jobs::get_job))
        .route("/api/v1/jobs/{id}/transition", post(jobs::transition_job))
        .route("/api/v1/jobs/{id}/events", get(jobs::list_job_events))
        .route("/api/v1/jobs/{id}/post", post(jobs::post_job))
        .route("/api/v1/jobs/{id}/parcels", post(parcels::create_job_parcel))
        .route("/api/v1/jobs/{id}/report.pdf", get(reports::job_report))
        .route("/api/v1/jobs/{id}/siteplan.pdf", get(reports::site_plan))
        // Audit log (NFR-07/11)
        .route("/api/v1/audit", get(audit::list_audit))
        .route("/api/v1/audit/verify", get(audit::verify_audit))
        // Import promotion (Phase 8)
        .route("/api/v1/import/promote/{run_id}", post(promote::promote_run))
        // Edit sessions and locks
        .route("/api/v1/sessions", post(sessions::open_session))
        .route(
            "/api/v1/sessions/{id}/heartbeat",
            post(sessions::heartbeat_session),
        )
        .route(
            "/api/v1/sessions/{id}/release",
            post(sessions::release_session),
        )
        .route(
            "/api/v1/sessions/{id}/locks",
            get(sessions::list_locks).post(sessions::acquire_lock),
        )
        .route(
            "/api/v1/sessions/{id}/locks/{lock_id}",
            delete(sessions::release_lock),
        )
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
