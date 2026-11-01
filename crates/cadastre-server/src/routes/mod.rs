//! HTTP routing.

pub mod audit;
pub mod beacons;
pub mod catalog;
pub mod editor;
pub mod fabric;
pub mod health;
pub mod jobs;
pub mod me;
pub mod parcels;
pub mod promote;
pub mod reports;
pub mod sessions;
pub mod survey;
pub mod tolerances;
pub mod viewer;
pub mod wf;
pub mod wfadmin;
pub mod workflow;

use axum::routing::{delete, get, post, put};
use axum::Router;
use tower_http::trace::TraceLayer;

use crate::state::AppState;

/// Build the application router with all routes and middleware.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health::health))
        .route("/ready", get(health::ready))
        // Browser viewer + editor (Phase 7/4, FR-VIEW/FR-SUR)
        .route("/viewer", get(viewer::viewer))
        .route("/editor", get(editor::index))
        .route("/editor/{*path}", get(editor::asset))
        .route("/api/v1/me", get(me::me))
        .route("/api/v1/auth/config", get(me::auth_config))
        // Aumentum workflow engine (Phase 2)
        .route("/api/v1/wf/definitions", get(wf::definitions))
        .route("/api/v1/wf/principals", get(wf::principals))
        .route("/api/v1/wf/jobs", get(wf::list_jobs).post(wf::create_job))
        .route("/api/v1/wf/jobs/{id}", get(wf::get_job))
        .route("/api/v1/wf/jobs/{id}/advance", post(wf::advance))
        .route("/api/v1/wf/jobs/{id}/assign", post(wf::assign))
        .route("/api/v1/wf/jobs/{id}/hold", post(wf::hold))
        .route("/api/v1/wf/jobs/{id}/editing", get(wf::get_editing).post(wf::start_editing))
        .route("/api/v1/wf/jobs/{id}/area", put(wf::put_area).delete(wf::delete_area))
        .route("/api/v1/wf/jobs/{id}/area/{action}", post(wf::area_action))
        .route("/api/v1/wf/jobs/{id}/discard", post(wf::discard))
        .route("/api/v1/wf/jobs/{id}/commit", post(wf::commit))
        .route("/api/v1/wf/jobs/{id}/end", post(wf::end_job))
        .route("/api/v1/wf/jobs/{id}/files", get(wfadmin::list_files)
            .post(wfadmin::upload_file).layer(axum::extract::DefaultBodyLimit::max(50 * 1024 * 1024)))
        .route("/api/v1/wf/files/{file_id}", get(wfadmin::get_file))
        .route("/api/v1/wf/productivity", get(wfadmin::productivity))
        .route("/api/v1/wf/notifications", get(wfadmin::notifications))
        .route("/api/v1/admin/security", get(wfadmin::security))
        .route("/api/v1/admin/change", post(wfadmin::change))
        .route("/api/v1/admin/settings", get(wfadmin::settings).put(wfadmin::put_setting))
        .route("/api/v1/catalog", get(catalog::folders))
        .route("/api/v1/catalog/items", get(catalog::items))
        .route("/api/v1/catalog/databases", get(catalog::databases))
        .route("/api/v1/catalog/database/items", get(catalog::database_items))
        .route("/api/v1/catalog/raster/info", get(catalog::raster_info))
        .route("/api/v1/catalog/raster/overview", get(catalog::raster_overview))
        .route("/api/v1/catalog/shapefile", get(catalog::shapefile))
        .route("/api/v1/parcels.geojson", get(viewer::parcels_geojson))
        .route("/api/v1/beacons.geojson", get(viewer::beacons_geojson))
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
