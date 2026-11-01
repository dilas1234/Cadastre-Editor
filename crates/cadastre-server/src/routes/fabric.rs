//! Fabric topology validation (FR-ADJ-04) and least-squares adjustment (FR-ADJ-01..03).
//!
//! Both reuse the `cadastre-core` engine so the server gives the same answers as the editor.

use axum::extract::State;
use axum::Json;
use cadastre_core::adjust::{least_squares, AdjustmentInput, Observation};
use cadastre_core::topology::check_topology;
use cadastre_core::{Bearing, CoreError, Point};

use crate::error::{ApiResult, AppError};
use crate::models::{
    AdjustRequest, AdjustResponse, FabricValidateRequest, GapDto, LineResidualDto, NearMissDto,
    ObservationDto, OverlapDto, PointDto, TopologyReportDto,
};
use crate::routes::tolerances;
use crate::state::AppState;

fn pt(p: &PointDto) -> Point {
    Point::new(p.x, p.y)
}

/// Validate parcel rings + beacons against the active tolerances.
#[utoipa::path(
    post, path = "/api/v1/fabric/validate", tag = "fabric",
    request_body = FabricValidateRequest,
    responses((status = 200, body = TopologyReportDto), (status = 404, description = "No active tolerances"))
)]
pub async fn validate(
    State(state): State<AppState>,
    Json(req): Json<FabricValidateRequest>,
) -> ApiResult<Json<TopologyReportDto>> {
    let tol = tolerances::load_active(&state.pool).await?.into();

    let rings: Vec<Vec<Point>> = req
        .parcels
        .iter()
        .map(|r| r.iter().map(pt).collect())
        .collect();
    let refs: Vec<&[Point]> = rings.iter().map(|r| r.as_slice()).collect();
    let beacons: Vec<Point> = req.beacons.iter().map(pt).collect();

    let report = check_topology(&refs, &beacons, &tol);
    let is_clean = report.is_clean();
    Ok(Json(TopologyReportDto {
        invalid_rings: report.invalid_rings,
        slivers: report.slivers,
        overlaps: report
            .overlaps
            .iter()
            .map(|o| OverlapDto {
                a: o.a,
                b: o.b,
                area: o.area,
            })
            .collect(),
        duplicate_beacons: report.duplicate_beacons.iter().map(|&(a, b)| [a, b]).collect(),
        gaps: report
            .gaps
            .iter()
            .map(|g| GapDto {
                ring: g.ring.iter().map(|p| PointDto { x: p.x, y: p.y }).collect(),
                area: g.area,
            })
            .collect(),
        self_intersections: report.self_intersections,
        near_misses: report
            .near_misses
            .iter()
            .map(|n| NearMissDto {
                a_parcel: n.a_parcel,
                a_vertex: n.a_vertex,
                b_parcel: n.b_parcel,
                b_vertex: n.b_vertex,
                distance: n.distance,
            })
            .collect(),
        is_clean,
    }))
}

/// Run a weighted least-squares adjustment; flag blunders against the active tolerances.
#[utoipa::path(
    post, path = "/api/v1/fabric/adjust", tag = "fabric",
    request_body = AdjustRequest,
    responses(
        (status = 200, body = AdjustResponse),
        (status = 400, description = "No control, under-constrained, or non-convergent"),
    )
)]
pub async fn adjust(
    State(state): State<AppState>,
    Json(req): Json<AdjustRequest>,
) -> ApiResult<Json<AdjustResponse>> {
    let tol: cadastre_core::tolerances::Tolerances = tolerances::load_active(&state.pool).await?.into();

    if req.fixed.len() != req.coords.len() {
        return Err(AppError::BadRequest(
            "`fixed` must have one flag per coordinate".to_string(),
        ));
    }

    let input = AdjustmentInput {
        coords: req.coords.iter().map(pt).collect(),
        fixed: req.fixed.clone(),
        observations: req
            .observations
            .iter()
            .map(|o: &ObservationDto| {
                Observation::with_sigmas(
                    o.from,
                    o.to,
                    o.distance,
                    Bearing::from_degrees(o.bearing),
                    o.sigma_distance_m,
                    o.sigma_bearing_sec,
                )
            })
            .collect(),
    };

    let max_iter = req.max_iter.unwrap_or(50);
    let adj = least_squares(&input, max_iter, req.tol.unwrap_or(1e-4)).map_err(core_err)?;
    let blunders = adj.blunders(tol.distance_residual, tol.bearing_residual_sec);

    Ok(Json(AdjustResponse {
        coords: adj.coords.iter().map(|p| PointDto { x: p.x, y: p.y }).collect(),
        residuals: adj
            .residuals
            .iter()
            .map(|r| LineResidualDto {
                observation: r.observation,
                distance_residual: r.distance_residual,
                bearing_residual_sec: r.bearing_residual_sec,
            })
            .collect(),
        iterations: adj.iterations,
        final_shift: adj.final_shift,
        blunders,
    }))
}

/// Map geometry-engine errors to client-facing 400s with a clear message.
fn core_err(e: CoreError) -> AppError {
    AppError::BadRequest(e.to_string())
}
