//! Authoritative live survey check (FR-COG-02/03).
//!
//! The desktop editor runs `cadastre_core::survey` locally (compiled to WASM) for its sub-100ms
//! live check. This endpoint runs the *same* engine on the server against the active tolerance
//! config, so submission-time validation cannot drift from what the surveyor saw.

use axum::extract::State;
use axum::Json;
use cadastre_core::survey::{SurveySession, SurveySummary};
use cadastre_core::{Bearing, Course, Curve, Point, Segment};

use crate::error::ApiResult;
use crate::models::{
    AreaCheckDto, ClosureDto, LegDto, SurveyCheckRequest, SurveyCheckResponse, SurveyChecksDto,
};
use crate::routes::tolerances;
use crate::state::AppState;

/// Re-validate a traverse against the server's active tolerances.
#[utoipa::path(
    post, path = "/api/v1/survey/check", tag = "survey",
    request_body = SurveyCheckRequest,
    responses(
        (status = 200, body = SurveyCheckResponse),
        (status = 404, description = "No active tolerance config"),
    )
)]
pub async fn check_survey(
    State(state): State<AppState>,
    Json(req): Json<SurveyCheckRequest>,
) -> ApiResult<Json<SurveyCheckResponse>> {
    let tolerances = tolerances::load_active(&state.pool).await?.into();

    let mut session = SurveySession::new(Point::new(req.start.x, req.start.y));
    session.start_beacon = req.start_beacon;
    session.stated_area = req.stated_area;
    for leg in req.legs {
        session.push_segment(leg.into());
    }

    Ok(Json(to_response(session.summary(&tolerances))))
}

impl From<LegDto> for Segment {
    fn from(leg: LegDto) -> Self {
        match leg {
            LegDto::Line { bearing, distance } => {
                Segment::Line(Course::new(Bearing::from_degrees(bearing), distance))
            }
            LegDto::Curve {
                radius,
                arc_length,
                chord_bearing,
            } => Segment::Curve(Curve::new(
                radius,
                arc_length,
                Bearing::from_degrees(chord_bearing),
            )),
        }
    }
}

fn to_response(summary: SurveySummary) -> SurveyCheckResponse {
    let closure = summary.closure.map(|c| ClosureDto {
        misclosure_east: c.misclosure_east,
        misclosure_north: c.misclosure_north,
        linear_misclosure: c.linear_misclosure,
        misclosure_bearing_deg: c.misclosure_bearing.degrees(),
        perimeter: c.perimeter,
        // JSON has no infinity; an exact closure is reported as null.
        precision_ratio: c.precision_ratio.is_finite().then_some(c.precision_ratio),
    });
    let area_check = summary.area_check.map(|a| AreaCheckDto {
        computed: a.computed,
        stated: a.stated,
        difference: a.difference,
        fraction: a.fraction,
    });
    SurveyCheckResponse {
        leg_count: summary.leg_count,
        vertex_count: summary.vertex_count,
        perimeter: summary.perimeter,
        closure,
        area: summary.area,
        area_check,
        checks: SurveyChecksDto {
            closes_on_start: summary.checks.closes_on_start,
            meets_precision: summary.checks.meets_precision,
            area_within_tolerance: summary.checks.area_within_tolerance,
        },
    }
}
