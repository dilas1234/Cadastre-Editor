//! The computation-tool registry: the 35 COGO, curve, area, adjustment and transformation
//! tools exposed by the editor and the API.
//!
//! Every tool is pure `cadastre-core` maths and can be executed in-process (WASM) or through
//! `POST /api/v1/survey/tools/{id}` on the server. Inputs are JSON objects; the field list in
//! [`catalog`] is the contract, and each tool validates its own typed input.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::adjust::{least_squares, AdjustmentInput, Observation};
use crate::alignment::{
    conflate_points, ConflationTolerances, LineSegment,
};
use crate::area::{self, area_check};
use crate::cogo;
use crate::crs::WorkingCrs;
use crate::curves::{self, CircularCurve};
use crate::error::{CoreError, Result};
use crate::survey::SurveySession;
use crate::tolerances::Tolerances;
use crate::transform::{fit_similarity, Similarity};
use crate::types::{Course, Point, Segment, Turn};
use crate::units::{Angle, Bearing};

/// Number of tools in the registry.
pub const TOOL_COUNT: usize = 35;

/// A group of related tools, used for the editor's menus.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ToolCategory {
    /// Forward/inverse, traverse, angle and distance conversion.
    Basic,
    /// Line and circle intersections.
    Intersections,
    /// Circular curves and spirals.
    Curves,
    /// Area, closure and adjustment.
    Area,
    /// Rotation, translation, scale and Helmert fitting.
    Transformations,
    /// Projection and scale factor.
    Geodetic,
    /// Point/line utilities.
    LineUtilities,
}

impl ToolCategory {
    /// Human label for the category.
    pub const fn label(self) -> &'static str {
        match self {
            ToolCategory::Basic => "Basic COGO",
            ToolCategory::Intersections => "Intersections",
            ToolCategory::Curves => "Curves",
            ToolCategory::Area => "Area & adjustment",
            ToolCategory::Transformations => "Transformations",
            ToolCategory::Geodetic => "Geodetic",
            ToolCategory::LineUtilities => "Line utilities",
        }
    }
}

/// One input field a tool accepts.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolField {
    /// JSON field name.
    pub name: &'static str,
    /// JSON type: `point`, `points`, `number`, `string`, `boolean`, `object`, `array`.
    pub kind: &'static str,
    /// Whether the field must be present.
    pub required: bool,
    /// Short description.
    pub description: &'static str,
}

/// A tool's catalogue entry.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolSpec {
    /// Stable identifier used in the API path.
    pub id: &'static str,
    /// Display name.
    pub name: &'static str,
    /// Category.
    pub category: ToolCategory,
    /// What the tool computes.
    pub description: &'static str,
    /// Accepted input fields.
    pub inputs: Vec<ToolField>,
    /// Shape of the result.
    pub output: &'static str,
}

fn field(name: &'static str, kind: &'static str, required: bool, description: &'static str) -> ToolField {
    ToolField {
        name,
        kind,
        required,
        description,
    }
}

/// The complete, ordered registry of the 35 computation tools.
pub fn catalog() -> Vec<ToolSpec> {
    use ToolCategory::*;
    vec![
        ToolSpec {
            id: "inverse",
            name: "Inverse (bearing & distance)",
            category: Basic,
            description: "Bearing and distance from one point to another.",
            inputs: vec![
                field("from", "point", true, "Start point"),
                field("to", "point", true, "End point"),
            ],
            output: "bearing (decimal/DMS), distance, dx, dy",
        },
        ToolSpec {
            id: "forward",
            name: "Forward (point from bearing & distance)",
            category: Basic,
            description: "New point from a start point, whole-circle bearing and distance.",
            inputs: vec![
                field("from", "point", true, "Start point"),
                field("bearing_deg", "number", true, "Whole-circle bearing, decimal degrees"),
                field("distance", "number", true, "Distance in metres"),
            ],
            output: "point, dx, dy",
        },
        ToolSpec {
            id: "traverse",
            name: "Traverse",
            category: Basic,
            description: "Run a traverse of straight and curved legs, with closure and area.",
            inputs: vec![
                field("start", "point", true, "Start beacon coordinate"),
                field("legs", "array", true, "Line/curve legs"),
                field("stated_area", "number", false, "Area stated on the plan (m²)"),
            ],
            output: "vertices, closure, area, perimeter, checks",
        },
        ToolSpec {
            id: "angle-convert",
            name: "Angle conversion",
            category: Basic,
            description: "Convert decimal degrees, DMS, radians or seconds of arc.",
            inputs: vec![
                field("value", "number", true, "Value in the `unit` given"),
                field("unit", "string", false, "degrees | dms | radians | seconds (default degrees)"),
                field("minutes", "number", false, "Minutes when unit = dms"),
                field("seconds", "number", false, "Seconds when unit = dms"),
            ],
            output: "degrees, radians, seconds, DMS, normalised degrees",
        },
        ToolSpec {
            id: "distance-convert",
            name: "Distance conversion",
            category: Basic,
            description: "Convert between metres, feet, US feet, chains, links, kilometres and miles.",
            inputs: vec![
                field("value", "number", true, "Value to convert"),
                field("from", "string", false, "Unit name (default m)"),
                field("to", "string", false, "Target unit name"),
            ],
            output: "metres, converted value, all common units",
        },
        ToolSpec {
            id: "point-on-line",
            name: "Point on line",
            category: Basic,
            description: "Point at a distance along a line, optionally offset perpendicular.",
            inputs: vec![
                field("from", "point", true, "Line start"),
                field("to", "point", true, "Line end"),
                field("distance", "number", true, "Distance from the start (m)"),
                field("offset", "number", false, "Signed perpendicular offset (positive right)"),
            ],
            output: "point, line bearing, line length",
        },
        ToolSpec {
            id: "perpendicular-foot",
            name: "Perpendicular foot",
            category: Basic,
            description: "Foot of the perpendicular from a point to a line.",
            inputs: vec![
                field("point", "point", true, "Source point"),
                field("a", "point", true, "Line start"),
                field("b", "point", true, "Line end"),
            ],
            output: "foot, distance along line, signed offset",
        },
        ToolSpec {
            id: "intersect-bearings",
            name: "Intersection: bearing–bearing",
            category: Intersections,
            description: "Intersect two lines given by a point and a bearing.",
            inputs: vec![
                field("p1", "point", true, "First line point"),
                field("bearing1_deg", "number", true, "First line bearing"),
                field("p2", "point", true, "Second line point"),
                field("bearing2_deg", "number", true, "Second line bearing"),
            ],
            output: "point (null when parallel)",
        },
        ToolSpec {
            id: "intersect-lines",
            name: "Intersection: line–line",
            category: Intersections,
            description: "Intersect two lines given by pairs of points.",
            inputs: vec![
                field("a1", "point", true, "First line start"),
                field("a2", "point", true, "First line end"),
                field("b1", "point", true, "Second line start"),
                field("b2", "point", true, "Second line end"),
            ],
            output: "point (null when parallel)",
        },
        ToolSpec {
            id: "intersect-bearing-distance",
            name: "Intersection: bearing–distance",
            category: Intersections,
            description: "Intersect a ray (point + bearing) with a circle (centre + radius).",
            inputs: vec![
                field("from", "point", true, "Ray start"),
                field("bearing_deg", "number", true, "Ray bearing"),
                field("centre", "point", true, "Circle centre"),
                field("radius", "number", true, "Circle radius (m)"),
            ],
            output: "points (0, 1 or 2)",
        },
        ToolSpec {
            id: "intersect-distances",
            name: "Intersection: distance–distance",
            category: Intersections,
            description: "Intersect two circles (centre + radius).",
            inputs: vec![
                field("c1", "point", true, "First centre"),
                field("r1", "number", true, "First radius"),
                field("c2", "point", true, "Second centre"),
                field("r2", "number", true, "Second radius"),
            ],
            output: "points (0, 1 or 2)",
        },
        ToolSpec {
            id: "offset-intersection",
            name: "Offset intersection",
            category: Intersections,
            description: "Intersect two lines after offsetting each perpendicular by a signed distance.",
            inputs: vec![
                field("p1", "point", true, "First line point"),
                field("bearing1_deg", "number", true, "First line bearing"),
                field("offset1", "number", true, "First line offset (positive right)"),
                field("p2", "point", true, "Second line point"),
                field("bearing2_deg", "number", true, "Second line bearing"),
                field("offset2", "number", true, "Second line offset"),
            ],
            output: "point (null when parallel)",
        },
        ToolSpec {
            id: "curve-radius-delta",
            name: "Curve: radius + deflection",
            category: Curves,
            description: "Resolve a simple circular curve from PC, initial tangent, radius and deflection.",
            inputs: vec![
                field("pc", "point", true, "Point of curve"),
                field("initial_bearing_deg", "number", true, "Tangent bearing at PC"),
                field("radius", "number", true, "Radius (m)"),
                field("delta_deg", "number", true, "Deflection angle (degrees)"),
                field("turn", "string", false, "left | right (default right)"),
            ],
            output: "PC, PI, PT, centre and all curve components",
        },
        ToolSpec {
            id: "curve-components",
            name: "Curve components",
            category: Curves,
            description: "Tangent, chord, external, mid-ordinate, arc length and degree of curve.",
            inputs: vec![
                field("radius", "number", true, "Radius (m)"),
                field("delta_deg", "number", true, "Deflection angle (degrees)"),
            ],
            output: "standard curve components",
        },
        ToolSpec {
            id: "curve-three-points",
            name: "Curve through three points",
            category: Curves,
            description: "Circular arc passing through three ordered points.",
            inputs: vec![
                field("a", "point", true, "First point (PC)"),
                field("b", "point", true, "Point on the arc"),
                field("c", "point", true, "Last point (PT)"),
            ],
            output: "PC, PI, PT, centre and all curve components",
        },
        ToolSpec {
            id: "curve-tangents",
            name: "Curve between tangents",
            category: Curves,
            description: "Simple curve from the PI, the two tangent bearings and a radius.",
            inputs: vec![
                field("pi", "point", true, "Point of intersection"),
                field("incoming_bearing_deg", "number", true, "Incoming tangent bearing"),
                field("outgoing_bearing_deg", "number", true, "Outgoing tangent bearing"),
                field("radius", "number", true, "Radius (m)"),
            ],
            output: "PC, PI, PT, centre and all curve components",
        },
        ToolSpec {
            id: "curve-point-at",
            name: "Point on curve",
            category: Curves,
            description: "Point and tangent bearing at an arc distance along a simple curve.",
            inputs: vec![
                field("pc", "point", true, "Point of curve"),
                field("initial_bearing_deg", "number", true, "Tangent bearing at PC"),
                field("radius", "number", true, "Radius (m)"),
                field("delta_deg", "number", true, "Deflection angle (degrees)"),
                field("turn", "string", false, "left | right"),
                field("arc_distance", "number", true, "Distance along the arc from PC"),
                field("offset", "number", false, "Perpendicular offset (positive right)"),
            ],
            output: "point, tangent bearing, arc distance",
        },
        ToolSpec {
            id: "spiral",
            name: "Spiral (clothoid)",
            category: Curves,
            description: "Standard components of a clothoid transition ending at a radius.",
            inputs: vec![
                field("radius", "number", true, "Radius at the end of the spiral (m)"),
                field("length", "number", true, "Spiral length (m)"),
            ],
            output: "spiral angle, shift, offset, local coordinates, tangents",
        },
        ToolSpec {
            id: "area",
            name: "Area, centroid & perimeter",
            category: Area,
            description: "Shoelace area, centroid, perimeter and bounding box of a ring.",
            inputs: vec![field("ring", "points", true, "Boundary ring (open or closed)")],
            output: "area, signed area, perimeter, centroid, bounds",
        },
        ToolSpec {
            id: "closure",
            name: "Traverse closure",
            category: Area,
            description: "Misclosure vector, precision ratio and perimeter of a closed traverse.",
            inputs: vec![
                field("start", "point", true, "Start coordinate"),
                field("legs", "array", true, "Line/curve legs"),
            ],
            output: "misclosure, bearing, perimeter, precision ratio",
        },
        ToolSpec {
            id: "bowditch",
            name: "Bowditch adjustment",
            category: Area,
            description: "Compass-rule adjustment of a traverse onto its start.",
            inputs: vec![
                field("start", "point", true, "Start coordinate"),
                field("legs", "array", true, "Line/curve legs"),
            ],
            output: "raw and adjusted vertices, misclosure",
        },
        ToolSpec {
            id: "least-squares",
            name: "Least-squares adjustment",
            category: Area,
            description: "Weighted least-squares network adjustment holding control fixed.",
            inputs: vec![
                field("coords", "points", true, "Initial coordinates"),
                field("fixed", "array", true, "Control mask (one boolean per coordinate)"),
                field("observations", "array", true, "Observed distances and bearings"),
                field("max_iter", "number", false, "Maximum iterations (default 50)"),
                field("tol", "number", false, "Convergence shift in metres (default 1e-4)"),
            ],
            output: "adjusted coordinates, residuals, iterations, blunders",
        },
        ToolSpec {
            id: "area-check",
            name: "Area comparison",
            category: Area,
            description: "Compare a computed area with the area stated on the plan.",
            inputs: vec![
                field("computed", "number", true, "Computed area (m²)"),
                field("stated", "number", true, "Stated area (m²)"),
                field("max_fraction", "number", false, "Allowed fractional difference (default 0.01)"),
            ],
            output: "difference, fraction, within tolerance",
        },
        ToolSpec {
            id: "rotate",
            name: "Rotate",
            category: Transformations,
            description: "Rotate points about an origin (anticlockwise positive).",
            inputs: vec![
                field("points", "points", true, "Points to rotate"),
                field("angle_deg", "number", true, "Rotation angle (anticlockwise)"),
                field("origin", "point", false, "Rotation origin (default 0,0)"),
            ],
            output: "rotated points",
        },
        ToolSpec {
            id: "translate",
            name: "Translate",
            category: Transformations,
            description: "Translate points by an easting/northing vector.",
            inputs: vec![
                field("points", "points", true, "Points to translate"),
                field("dx", "number", true, "Easting shift"),
                field("dy", "number", true, "Northing shift"),
            ],
            output: "translated points",
        },
        ToolSpec {
            id: "scale",
            name: "Scale",
            category: Transformations,
            description: "Scale points uniformly about an origin.",
            inputs: vec![
                field("points", "points", true, "Points to scale"),
                field("factor", "number", true, "Scale factor"),
                field("origin", "point", false, "Scale origin (default 0,0)"),
            ],
            output: "scaled points",
        },
        ToolSpec {
            id: "helmert-fit",
            name: "Helmert fit",
            category: Transformations,
            description: "Fit a 4-parameter Helmert transform to control point pairs.",
            inputs: vec![
                field("source", "points", true, "Source control coordinates"),
                field("target", "points", true, "Target control coordinates"),
            ],
            output: "a, b, tx, ty, scale, rotation, residuals, RMS",
        },
        ToolSpec {
            id: "helmert-apply",
            name: "Helmert apply",
            category: Transformations,
            description: "Apply a 4-parameter Helmert transform to points.",
            inputs: vec![
                field("points", "points", true, "Points to transform"),
                field("a", "number", true, "Cosine-scale coefficient"),
                field("b", "number", true, "Sine-scale coefficient"),
                field("tx", "number", true, "Easting translation"),
                field("ty", "number", true, "Northing translation"),
            ],
            output: "transformed points",
        },
        ToolSpec {
            id: "conflate-points",
            name: "Conflate beacons",
            category: Transformations,
            description: "Match new beacons to existing control and fit the joining transform.",
            inputs: vec![
                field("source", "points", true, "New beacon coordinates"),
                field("target", "points", true, "Existing control coordinates"),
                field("max_distance", "number", false, "Match distance (default 2 m)"),
                field("min_matches", "number", false, "Minimum matches (default 2)"),
            ],
            output: "transform, matches, unmatched, RMS, transformed source",
        },
        ToolSpec {
            id: "conflate-lines",
            name: "Conflate linework",
            category: Transformations,
            description: "Match new lines to existing lines by midpoint, bearing and length.",
            inputs: vec![
                field("source", "array", true, "Source lines ({from,to} or [a,b])"),
                field("target", "array", true, "Target lines ({from,to} or [a,b])"),
                field("max_distance", "number", false, "Midpoint match distance (default 2 m)"),
                field("max_bearing_sec", "number", false, "Bearing tolerance (default 60″)"),
                field("max_length_diff", "number", false, "Length tolerance (default 0.5 m)"),
            ],
            output: "transform, line matches, unmatched, RMS",
        },
        ToolSpec {
            id: "project",
            name: "Project latitude/longitude",
            category: Geodetic,
            description: "Project geographic coordinates to grid metres (UTM 32N or Minna).",
            inputs: vec![
                field("lat_deg", "number", true, "Latitude, decimal degrees"),
                field("lon_deg", "number", true, "Longitude, decimal degrees"),
                field("crs", "string", false, "utm32n | minna (default utm32n)"),
            ],
            output: "easting, northing, grid scale factor",
        },
        ToolSpec {
            id: "unproject",
            name: "Unproject grid coordinates",
            category: Geodetic,
            description: "Unproject grid metres to latitude/longitude (UTM 32N or Minna).",
            inputs: vec![
                field("x", "number", true, "Easting (m)"),
                field("y", "number", true, "Northing (m)"),
                field("crs", "string", false, "utm32n | minna (default utm32n)"),
            ],
            output: "latitude, longitude, grid scale factor",
        },
        ToolSpec {
            id: "grid-scale-factor",
            name: "Grid scale factor",
            category: Geodetic,
            description: "Point scale factor and ground/grid distance conversion at a position.",
            inputs: vec![
                field("lat_deg", "number", true, "Latitude, decimal degrees"),
                field("lon_deg", "number", true, "Longitude, decimal degrees"),
                field("crs", "string", false, "utm32n | minna"),
            ],
            output: "scale factor, grid distance for 1000 m ground",
        },
        ToolSpec {
            id: "line-divide",
            name: "Divide line",
            category: LineUtilities,
            description: "Divide a line into equal parts, returning every division point.",
            inputs: vec![
                field("from", "point", true, "Line start"),
                field("to", "point", true, "Line end"),
                field("parts", "number", true, "Number of equal parts"),
            ],
            output: "points including both endpoints",
        },
        ToolSpec {
            id: "offset-point",
            name: "Offset point",
            category: LineUtilities,
            description: "Offset a point perpendicular to a bearing (positive right).",
            inputs: vec![
                field("point", "point", true, "Source point"),
                field("bearing_deg", "number", true, "Travel bearing"),
                field("offset", "number", true, "Signed offset (positive right)"),
            ],
            output: "offset point",
        },
    ]
}

/// Look up one tool specification.
pub fn tool_spec(id: &str) -> Option<ToolSpec> {
    catalog().into_iter().find(|t| t.id == id)
}

/// Execute a tool by id with a JSON input object.
pub fn execute(id: &str, input: &Value) -> Result<Value> {
    match id {
        "inverse" => {
            let i: TwoPoints = parse(input)?;
            let (bearing, distance) = cogo::inverse(i.from, i.to);
            Ok(json!({
                "bearing_deg": bearing.degrees(),
                "bearing_dms": dms(bearing.angle()),
                "distance": distance,
                "dx": i.to.x - i.from.x,
                "dy": i.to.y - i.from.y,
            }))
        }
        "forward" => {
            let i: ForwardInput = parse(input)?;
            let p = cogo::forward(i.from, Bearing::from_degrees(i.bearing_deg), i.distance);
            Ok(json!({
                "point": point(p),
                "dx": p.x - i.from.x,
                "dy": p.y - i.from.y,
            }))
        }
        "traverse" => {
            let i: TraverseInput = parse(input)?;
            let session = i.session();
            let summary = session.summary(&Tolerances::default());
            Ok(traverse_json(&session, &summary))
        }
        "angle-convert" => {
            let i: AngleInput = parse(input)?;
            let angle = i.angle()?;
            Ok(json!({
                "degrees": angle.degrees(),
                "normalized_degrees": angle.normalized().degrees(),
                "radians": angle.radians(),
                "seconds": angle.total_seconds(),
                "dms": dms(angle),
            }))
        }
        "distance-convert" => {
            let i: DistanceInput = parse(input)?;
            Ok(distance_json(&i)?)
        }
        "point-on-line" => {
            let i: PointOnLineInput = parse(input)?;
            let (bearing, length) = cogo::inverse(i.from, i.to);
            let p = cogo::point_on_line(i.from, i.to, i.distance, i.offset.unwrap_or(0.0))
                .ok_or_else(|| CoreError::NoSolution("the line is degenerate".to_string()))?;
            Ok(json!({
                "point": point(p),
                "bearing_deg": bearing.degrees(),
                "line_length": length,
            }))
        }
        "perpendicular-foot" => {
            let i: FootInput = parse(input)?;
            let (foot, along, offset) = cogo::perpendicular_foot(i.point, i.a, i.b)
                .ok_or_else(|| CoreError::NoSolution("the line is degenerate".to_string()))?;
            Ok(json!({
                "foot": point(foot),
                "along": along,
                "offset": offset,
                "distance": i.point.distance_to(foot),
            }))
        }
        "intersect-bearings" => {
            let i: BearingIntersection = parse(input)?;
            let p = cogo::intersection(
                i.p1,
                Bearing::from_degrees(i.bearing1_deg),
                i.p2,
                Bearing::from_degrees(i.bearing2_deg),
            );
            Ok(json!({ "point": p.map(point), "parallel": p.is_none() }))
        }
        "intersect-lines" => {
            let i: LineIntersection = parse(input)?;
            let p = cogo::line_intersection(i.a1, i.a2, i.b1, i.b2);
            Ok(json!({ "point": p.map(point), "parallel": p.is_none() }))
        }
        "intersect-bearing-distance" => {
            let i: BearingDistance = parse(input)?;
            let points = cogo::intersect_bearing_distance(
                i.from,
                Bearing::from_degrees(i.bearing_deg),
                i.centre,
                i.radius,
            );
            Ok(json!({ "points": points.into_iter().map(point).collect::<Vec<_>>() }))
        }
        "intersect-distances" => {
            let i: DistanceIntersection = parse(input)?;
            let points = cogo::intersect_distances(i.c1, i.r1, i.c2, i.r2);
            Ok(json!({ "points": points.into_iter().map(point).collect::<Vec<_>>() }))
        }
        "offset-intersection" => {
            let i: OffsetIntersection = parse(input)?;
            let p1 = cogo::offset(i.p1, Bearing::from_degrees(i.bearing1_deg), i.offset1);
            let p2 = cogo::offset(i.p2, Bearing::from_degrees(i.bearing2_deg), i.offset2);
            let p = cogo::intersection(
                p1,
                Bearing::from_degrees(i.bearing1_deg),
                p2,
                Bearing::from_degrees(i.bearing2_deg),
            );
            Ok(json!({ "point": p.map(point), "parallel": p.is_none() }))
        }
        "curve-radius-delta" => {
            let i: CurveRadiusDelta = parse(input)?;
            let c = curves::from_radius_delta(
                i.pc,
                Bearing::from_degrees(i.initial_bearing_deg),
                i.radius,
                i.delta_deg.to_radians(),
                i.turn(),
            )?;
            Ok(curve_json(&c))
        }
        "curve-components" => {
            let i: CurveComponentsInput = parse(input)?;
            let c = curves::components(i.radius, i.delta_deg.to_radians())?;
            Ok(json!({
                "radius": c.radius,
                "delta_deg": c.delta.to_degrees(),
                "tangent": c.tangent,
                "chord": c.chord,
                "external": c.external,
                "mid_ordinate": c.mid_ordinate,
                "arc_length": c.arc_length,
                "degree_of_curve": c.degree_of_curve,
            }))
        }
        "curve-three-points" => {
            let i: ThreePoints = parse(input)?;
            Ok(curve_json(&curves::through_three_points(i.a, i.b, i.c)?))
        }
        "curve-tangents" => {
            let i: CurveTangents = parse(input)?;
            let c = curves::from_tangents(
                i.pi,
                Bearing::from_degrees(i.incoming_bearing_deg),
                Bearing::from_degrees(i.outgoing_bearing_deg),
                i.radius,
            )?;
            Ok(curve_json(&c))
        }
        "curve-point-at" => {
            let i: CurvePointInput = parse(input)?;
            let c = curves::from_radius_delta(
                i.pc,
                Bearing::from_degrees(i.initial_bearing_deg),
                i.radius,
                i.delta_deg.to_radians(),
                i.turn(),
            )?;
            let p = curves::point_on_curve(&c, i.arc_distance, i.offset.unwrap_or(0.0))?;
            Ok(json!({
                "point": point(p.point),
                "tangent_bearing_deg": p.tangent_bearing.degrees(),
                "arc_distance": p.arc_distance,
                "offset": p.offset,
            }))
        }
        "spiral" => {
            let i: SpiralInput = parse(input)?;
            let s = curves::spiral(i.radius, i.length)?;
            Ok(json!({
                "radius": s.radius,
                "length": s.length,
                "theta": s.theta,
                "theta_deg": s.theta.to_degrees(),
                "shift": s.shift,
                "offset_k": s.offset_k,
                "local_x": s.local_x,
                "local_y": s.local_y,
                "long_tangent": s.long_tangent,
                "short_tangent": s.short_tangent,
            }))
        }
        "area" => {
            let i: RingInput = parse(input)?;
            let area = area::polygon_area(&i.ring)?;
            let signed = area::signed_area(&i.ring);
            let centroid = area::centroid(&i.ring);
            let bounds = area::bounding_box(&i.ring);
            Ok(json!({
                "area": area,
                "signed_area": signed,
                "perimeter": area::perimeter(&i.ring),
                "centroid": point(centroid),
                "bounds": bounds.map(|(min, max)| json!({ "min": point(min), "max": point(max) })),
                "vertices": i.ring.len(),
            }))
        }
        "closure" => {
            let i: TraverseInput = parse(input)?;
            let session = i.session();
            let traverse = session.traverse();
            let c = crate::closure::closure(&traverse)?;
            Ok(closure_json(&c))
        }
        "bowditch" => {
            let i: TraverseInput = parse(input)?;
            let session = i.session();
            let traverse = session.traverse();
            let adjusted = crate::adjust::bowditch(&traverse)?;
            let c = crate::closure::closure(&traverse)?;
            Ok(json!({
                "raw": session.vertices().into_iter().map(point).collect::<Vec<_>>(),
                "adjusted": adjusted.into_iter().map(point).collect::<Vec<_>>(),
                "misclosure_east": c.misclosure_east,
                "misclosure_north": c.misclosure_north,
                "linear_misclosure": c.linear_misclosure,
            }))
        }
        "least-squares" => {
            let i: LeastSquaresInput = parse(input)?;
            let input = AdjustmentInput {
                coords: i.coords,
                fixed: i.fixed,
                observations: i
                    .observations
                    .iter()
                    .map(|o| {
                        Observation::with_sigmas(
                            o.from,
                            o.to,
                            o.distance,
                            Bearing::from_degrees(o.bearing_deg),
                            o.sigma_distance_m,
                            o.sigma_bearing_sec,
                        )
                    })
                    .collect(),
            };
            let tol = Tolerances::default();
            let adj = least_squares(&input, i.max_iter, i.tol)?;
            let blunders = adj.blunders(tol.distance_residual, tol.bearing_residual_sec);
            Ok(json!({
                "coords": adj.coords.into_iter().map(point).collect::<Vec<_>>(),
                "residuals": adj.residuals.iter().map(|r| json!({
                    "observation": r.observation,
                    "distance_residual": r.distance_residual,
                    "bearing_residual_sec": r.bearing_residual_sec,
                })).collect::<Vec<_>>(),
                "iterations": adj.iterations,
                "final_shift": adj.final_shift,
                "blunders": blunders,
            }))
        }
        "area-check" => {
            let i: AreaCheckInput = parse(input)?;
            let check = area_check(i.computed, i.stated);
            Ok(json!({
                "computed": check.computed,
                "stated": check.stated,
                "difference": check.difference,
                "fraction": check.fraction,
                "within_tolerance": check.within(i.max_fraction),
                "max_fraction": i.max_fraction,
            }))
        }
        "rotate" => {
            let i: RotateInput = parse(input)?;
            let origin = i.origin.unwrap_or(Point::new(0.0, 0.0));
            let t = Similarity::rotation_scale(Angle::from_degrees(i.angle_deg), 1.0, origin);
            Ok(json!({ "points": i.points.into_iter().map(|p| point(t.apply(p))).collect::<Vec<_>>() }))
        }
        "translate" => {
            let i: TranslateInput = parse(input)?;
            let t = Similarity::translation(i.dx, i.dy);
            Ok(json!({ "points": i.points.into_iter().map(|p| point(t.apply(p))).collect::<Vec<_>>() }))
        }
        "scale" => {
            let i: ScaleInput = parse(input)?;
            if i.factor == 0.0 || !i.factor.is_finite() {
                return Err(CoreError::InvalidInput(
                    "scale factor must be finite and non-zero".to_string(),
                ));
            }
            let origin = i.origin.unwrap_or(Point::new(0.0, 0.0));
            let t = Similarity::rotation_scale(Angle::from_degrees(0.0), i.factor, origin);
            Ok(json!({ "points": i.points.into_iter().map(|p| point(t.apply(p))).collect::<Vec<_>>() }))
        }
        "helmert-fit" => {
            let i: HelmertFitInput = parse(input)?;
            let t = fit_similarity(&i.source, &i.target)?;
            let residuals = t.residuals(&i.source, &i.target);
            Ok(json!({
                "a": t.a,
                "b": t.b,
                "tx": t.tx,
                "ty": t.ty,
                "scale": t.scale(),
                "rotation_deg": t.rotation().degrees(),
                "residuals": residuals,
                "rms": t.rms(&i.source, &i.target),
                "max_residual": residuals.iter().copied().fold(0.0, f64::max),
            }))
        }
        "helmert-apply" => {
            let i: HelmertApplyInput = parse(input)?;
            let t = Similarity {
                a: i.a,
                b: i.b,
                tx: i.tx,
                ty: i.ty,
            };
            Ok(json!({ "points": i.points.into_iter().map(|p| point(t.apply(p))).collect::<Vec<_>>() }))
        }
        "conflate-points" => {
            let i: ConflateInput = parse(input)?;
            let report = conflate_points(
                &i.source,
                &i.target,
                &ConflationTolerances {
                    max_distance: i.max_distance.unwrap_or(2.0),
                    min_matches: i.min_matches.unwrap_or(2),
                    ..Default::default()
                },
            )?;
            Ok(json!({
                "transform": similarity_json(&report.transform),
                "transformed_source": report.transformed_source.into_iter().map(point).collect::<Vec<_>>(),
                "matches": report.matches.iter().map(|m| json!({
                    "source": m.source, "target": m.target, "distance": m.distance
                })).collect::<Vec<_>>(),
                "unmatched_source": report.unmatched_source,
                "unmatched_target": report.unmatched_target,
                "rms": report.rms,
                "max_residual": report.max_residual,
                "iterations": report.iterations,
            }))
        }
        "conflate-lines" => {
            let i: ConflateLinesInput = parse(input)?;
            let source = i.source();
            let target = i.target();
            let report = crate::alignment::conflate_lines(
                &source,
                &target,
                &ConflationTolerances {
                    max_distance: i.max_distance.unwrap_or(2.0),
                    max_bearing_sec: i.max_bearing_sec.unwrap_or(60.0),
                    max_length_diff: i.max_length_diff.unwrap_or(0.5),
                    min_matches: 2,
                },
            )?;
            Ok(json!({
                "transform": similarity_json(&report.transform),
                "matches": report.matches.iter().map(|m| json!({
                    "source": m.source, "target": m.target,
                    "midpoint_distance": m.midpoint_distance,
                    "bearing_diff_sec": m.bearing_diff_sec,
                    "length_diff": m.length_diff,
                    "reversed": m.reversed,
                })).collect::<Vec<_>>(),
                "unmatched_source": report.unmatched_source,
                "unmatched_target": report.unmatched_target,
                "rms": report.rms,
                "max_residual": report.max_residual,
                "iterations": report.iterations,
            }))
        }
        "project" => {
            let i: ProjectInput = parse(input)?;
            let crs = working_crs(&i.crs)?;
            let p = crs.project_deg(i.lat_deg, i.lon_deg);
            Ok(json!({
                "point": point(p),
                "crs": i.crs,
                "point_scale_factor": crs.point_scale_factor_deg(i.lat_deg, i.lon_deg),
            }))
        }
        "unproject" => {
            let i: UnprojectInput = parse(input)?;
            let crs = working_crs(&i.crs)?;
            let (lat, lon) = crs.unproject_deg(Point::new(i.x, i.y));
            Ok(json!({
                "lat_deg": lat,
                "lon_deg": lon,
                "crs": i.crs,
                "point_scale_factor": crs.point_scale_factor_deg(lat, lon),
            }))
        }
        "grid-scale-factor" => {
            let i: GridScaleInput = parse(input)?;
            let crs = working_crs(&i.crs)?;
            let k = crs.point_scale_factor_deg(i.lat_deg, i.lon_deg);
            Ok(json!({
                "point_scale_factor": k,
                "grid_distance_for_1000m_ground": 1000.0 * k,
                "ground_distance_for_1000m_grid": 1000.0 / k,
                "crs": i.crs,
            }))
        }
        "line-divide" => {
            let i: LineDivideInput = parse(input)?;
            Ok(json!({
                "points": cogo::divide_line(i.from, i.to, i.parts).into_iter().map(point).collect::<Vec<_>>()
            }))
        }
        "offset-point" => {
            let i: OffsetPointInput = parse(input)?;
            let p = cogo::offset(i.point, Bearing::from_degrees(i.bearing_deg), i.offset);
            Ok(json!({ "point": point(p) }))
        }
        _ => Err(CoreError::UnknownTool(id.to_string())),
    }
}

// ---------------------------------------------------------------------------
// Input types
// ---------------------------------------------------------------------------

fn parse<T: serde::de::DeserializeOwned>(value: &Value) -> Result<T> {
    serde_json::from_value(value.clone())
        .map_err(|e| CoreError::InvalidInput(format!("tool input: {e}")))
}

fn point(p: Point) -> Value {
    json!({ "x": p.x, "y": p.y })
}

fn dms(angle: Angle) -> Value {
    let (degrees, minutes, seconds) = angle.to_dms();
    json!({
        "degrees": degrees,
        "minutes": minutes,
        "seconds": seconds,
        "text": format!("{degrees}°{minutes:02}'{seconds:06.3}\""),
    })
}

fn curve_json(c: &CircularCurve) -> Value {
    json!({
        "pc": point(c.pc),
        "pi": point(c.pi),
        "pt": point(c.pt),
        "centre": point(c.centre),
        "radius": c.radius,
        "delta_deg": c.delta.to_degrees(),
        "turn": c.turn,
        "tangent": c.tangent,
        "chord": c.chord,
        "external": c.external,
        "mid_ordinate": c.mid_ordinate,
        "arc_length": c.arc_length,
        "initial_bearing_deg": c.initial_bearing.degrees(),
        "final_bearing_deg": c.final_bearing.degrees(),
        "chord_bearing_deg": c.chord_bearing.degrees(),
    })
}

fn closure_json(c: &crate::closure::Closure) -> Value {
    json!({
        "misclosure_east": c.misclosure_east,
        "misclosure_north": c.misclosure_north,
        "linear_misclosure": c.linear_misclosure,
        "misclosure_bearing_deg": c.misclosure_bearing.degrees(),
        "perimeter": c.perimeter,
        "precision_ratio": c.precision_ratio.is_finite().then_some(c.precision_ratio),
    })
}

fn similarity_json(t: &Similarity) -> Value {
    json!({
        "a": t.a, "b": t.b, "tx": t.tx, "ty": t.ty,
        "scale": t.scale(), "rotation_deg": t.rotation().degrees(),
    })
}

fn traverse_json(session: &SurveySession, summary: &crate::survey::SurveySummary) -> Value {
    json!({
        "vertices": session.vertices().into_iter().map(point).collect::<Vec<_>>(),
        "leg_count": summary.leg_count,
        "vertex_count": summary.vertex_count,
        "perimeter": summary.perimeter,
        "closure": summary.closure.as_ref().map(closure_json),
        "area": summary.area,
        "stated_area": session.stated_area,
        "area_check": summary.area_check.map(|a| json!({
            "computed": a.computed, "stated": a.stated,
            "difference": a.difference, "fraction": a.fraction,
        })),
        "checks": {
            "closes_on_start": summary.checks.closes_on_start,
            "meets_precision": summary.checks.meets_precision,
            "area_within_tolerance": summary.checks.area_within_tolerance,
        },
    })
}

fn distance_json(i: &DistanceInput) -> Result<Value> {
    let metres = to_metres(i.value, &i.from)?;
    let converted = match &i.to {
        Some(to) => Some((to.clone(), from_metres(metres, to)?)),
        None => None,
    };
    Ok(json!({
        "metres": metres,
        "converted": converted.as_ref().map(|(_, v)| *v),
        "to": converted.map(|(u, _)| u),
        "units": {
            "m": metres,
            "km": metres / 1000.0,
            "ft": metres / 0.3048,
            "usft": metres * 3937.0 / 1200.0,
            "chain": metres / 20.1168,
            "link": metres / 0.201168,
            "mile": metres / 1609.344,
        }
    }))
}

fn to_metres(value: f64, unit: &str) -> Result<f64> {
    let metres = match unit.to_ascii_lowercase().as_str() {
        "m" | "metre" | "meter" | "metres" | "meters" => value,
        "km" | "kilometre" | "kilometer" => value * 1000.0,
        "ft" | "foot" | "feet" => value * 0.3048,
        "usft" | "us_survey_foot" | "us-survey-foot" => value * 1200.0 / 3937.0,
        "chain" | "chains" => value * 20.1168,
        "link" | "links" => value * 0.201168,
        "mile" | "miles" => value * 1609.344,
        other => {
            return Err(CoreError::InvalidInput(format!(
                "unknown distance unit `{other}`"
            )))
        }
    };
    Ok(metres)
}

fn from_metres(metres: f64, unit: &str) -> Result<f64> {
    let value = match unit.to_ascii_lowercase().as_str() {
        "m" | "metre" | "meter" | "metres" | "meters" => metres,
        "km" | "kilometre" | "kilometer" => metres / 1000.0,
        "ft" | "foot" | "feet" => metres / 0.3048,
        "usft" | "us_survey_foot" | "us-survey-foot" => metres * 3937.0 / 1200.0,
        "chain" | "chains" => metres / 20.1168,
        "link" | "links" => metres / 0.201168,
        "mile" | "miles" => metres / 1609.344,
        other => {
            return Err(CoreError::InvalidInput(format!(
                "unknown distance unit `{other}`"
            )))
        }
    };
    Ok(value)
}

fn working_crs(name: &str) -> Result<WorkingCrs> {
    match name.to_ascii_lowercase().as_str() {
        "utm32n" | "utm-32n" | "epsg:32632" | "32632" | "" => Ok(WorkingCrs::utm_zone_32n()),
        "minna" | "nigeria-mid-belt" | "epsg:26392" | "26392" => Ok(WorkingCrs::default()),
        other => Err(CoreError::InvalidInput(format!("unknown CRS `{other}`"))),
    }
}

#[derive(Deserialize)]
struct TwoPoints {
    from: Point,
    to: Point,
}

#[derive(Deserialize)]
struct ForwardInput {
    from: Point,
    bearing_deg: f64,
    distance: f64,
}

#[derive(Clone, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum LegInput {
    Line {
        bearing_deg: f64,
        distance: f64,
    },
    Curve {
        radius: f64,
        arc_length: f64,
        chord_bearing_deg: f64,
        #[serde(default)]
        turn: Turn,
    },
}

#[derive(Deserialize)]
struct TraverseInput {
    start: Point,
    legs: Vec<LegInput>,
    #[serde(default)]
    stated_area: Option<f64>,
}

impl TraverseInput {
    fn session(&self) -> SurveySession {
        let mut session = SurveySession::new(self.start);
        session.stated_area = self.stated_area;
        for leg in &self.legs {
            match *leg {
                LegInput::Line {
                    bearing_deg,
                    distance,
                } => session.push_course(Course::new(Bearing::from_degrees(bearing_deg), distance)),
                LegInput::Curve {
                    radius,
                    arc_length,
                    chord_bearing_deg,
                    turn,
                } => session.push_segment(Segment::Curve(crate::types::Curve::with_turn(
                    radius,
                    arc_length,
                    Bearing::from_degrees(chord_bearing_deg),
                    turn,
                ))),
            }
        }
        session
    }
}

#[derive(Deserialize)]
struct AngleInput {
    value: f64,
    #[serde(default)]
    unit: String,
    #[serde(default)]
    minutes: Option<f64>,
    #[serde(default)]
    seconds: Option<f64>,
}

impl AngleInput {
    fn angle(&self) -> Result<Angle> {
        let unit = if self.unit.is_empty() {
            "degrees"
        } else {
            self.unit.as_str()
        };
        Ok(match unit.to_ascii_lowercase().as_str() {
            "degrees" | "degree" | "deg" => Angle::from_degrees(self.value),
            "radians" | "radian" | "rad" => Angle::from_radians(self.value),
            "seconds" | "second" | "sec" => Angle::from_seconds(self.value),
            "dms" => {
                let degrees = self.value.trunc() as i32;
                let minutes = self.minutes.unwrap_or(0.0) as u32;
                let seconds = self.seconds.unwrap_or(0.0);
                Angle::from_dms(degrees, minutes, seconds)
            }
            other => {
                return Err(CoreError::InvalidInput(format!(
                    "unknown angle unit `{other}`"
                )))
            }
        })
    }
}

#[derive(Deserialize)]
struct DistanceInput {
    value: f64,
    #[serde(default = "metre_unit")]
    from: String,
    #[serde(default)]
    to: Option<String>,
}

fn metre_unit() -> String {
    "m".to_string()
}

#[derive(Deserialize)]
struct PointOnLineInput {
    from: Point,
    to: Point,
    distance: f64,
    #[serde(default)]
    offset: Option<f64>,
}

#[derive(Deserialize)]
struct FootInput {
    point: Point,
    a: Point,
    b: Point,
}

#[derive(Deserialize)]
struct BearingIntersection {
    p1: Point,
    bearing1_deg: f64,
    p2: Point,
    bearing2_deg: f64,
}

#[derive(Deserialize)]
struct LineIntersection {
    a1: Point,
    a2: Point,
    b1: Point,
    b2: Point,
}

#[derive(Deserialize)]
struct BearingDistance {
    from: Point,
    bearing_deg: f64,
    centre: Point,
    radius: f64,
}

#[derive(Deserialize)]
struct DistanceIntersection {
    c1: Point,
    r1: f64,
    c2: Point,
    r2: f64,
}

#[derive(Deserialize)]
struct OffsetIntersection {
    p1: Point,
    bearing1_deg: f64,
    offset1: f64,
    p2: Point,
    bearing2_deg: f64,
    offset2: f64,
}

fn default_turn() -> Turn {
    Turn::Right
}

#[derive(Deserialize)]
struct CurveRadiusDelta {
    pc: Point,
    initial_bearing_deg: f64,
    radius: f64,
    delta_deg: f64,
    #[serde(default = "default_turn")]
    turn: Turn,
}

impl CurveRadiusDelta {
    fn turn(&self) -> Turn {
        self.turn
    }
}

#[derive(Deserialize)]
struct CurveComponentsInput {
    radius: f64,
    delta_deg: f64,
}

#[derive(Deserialize)]
struct ThreePoints {
    a: Point,
    b: Point,
    c: Point,
}

#[derive(Deserialize)]
struct CurveTangents {
    pi: Point,
    incoming_bearing_deg: f64,
    outgoing_bearing_deg: f64,
    radius: f64,
}

#[derive(Deserialize)]
struct CurvePointInput {
    pc: Point,
    initial_bearing_deg: f64,
    radius: f64,
    delta_deg: f64,
    #[serde(default = "default_turn")]
    turn: Turn,
    arc_distance: f64,
    #[serde(default)]
    offset: Option<f64>,
}

impl CurvePointInput {
    fn turn(&self) -> Turn {
        self.turn
    }
}

#[derive(Deserialize)]
struct SpiralInput {
    radius: f64,
    length: f64,
}

#[derive(Deserialize)]
struct RingInput {
    ring: Vec<Point>,
}

fn default_max_iter() -> usize {
    50
}

fn default_tol() -> f64 {
    1e-4
}

fn default_sigma_distance() -> f64 {
    0.01
}

fn default_sigma_bearing() -> f64 {
    10.0
}

#[derive(Deserialize)]
struct ObservationInput {
    from: usize,
    to: usize,
    distance: f64,
    bearing_deg: f64,
    #[serde(default = "default_sigma_distance")]
    sigma_distance_m: f64,
    #[serde(default = "default_sigma_bearing")]
    sigma_bearing_sec: f64,
}

#[derive(Deserialize)]
struct LeastSquaresInput {
    coords: Vec<Point>,
    fixed: Vec<bool>,
    observations: Vec<ObservationInput>,
    #[serde(default = "default_max_iter")]
    max_iter: usize,
    #[serde(default = "default_tol")]
    tol: f64,
}

fn default_area_fraction() -> f64 {
    0.01
}

#[derive(Deserialize)]
struct AreaCheckInput {
    computed: f64,
    stated: f64,
    #[serde(default = "default_area_fraction")]
    max_fraction: f64,
}

#[derive(Deserialize)]
struct RotateInput {
    points: Vec<Point>,
    angle_deg: f64,
    #[serde(default)]
    origin: Option<Point>,
}

#[derive(Deserialize)]
struct TranslateInput {
    points: Vec<Point>,
    dx: f64,
    dy: f64,
}

#[derive(Deserialize)]
struct ScaleInput {
    points: Vec<Point>,
    factor: f64,
    #[serde(default)]
    origin: Option<Point>,
}

#[derive(Deserialize)]
struct HelmertFitInput {
    source: Vec<Point>,
    target: Vec<Point>,
}

#[derive(Deserialize)]
struct HelmertApplyInput {
    points: Vec<Point>,
    a: f64,
    b: f64,
    tx: f64,
    ty: f64,
}

#[derive(Deserialize)]
struct ConflateInput {
    source: Vec<Point>,
    target: Vec<Point>,
    #[serde(default)]
    max_distance: Option<f64>,
    #[serde(default)]
    min_matches: Option<usize>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(untagged)]
enum LineInput {
    Pair([Point; 2]),
    Object { from: Point, to: Point },
}

impl LineInput {
    fn segment(self) -> LineSegment {
        match self {
            LineInput::Pair([from, to]) => LineSegment::new(from, to),
            LineInput::Object { from, to } => LineSegment::new(from, to),
        }
    }
}

#[derive(Deserialize)]
struct ConflateLinesInput {
    source: Vec<LineInput>,
    target: Vec<LineInput>,
    #[serde(default)]
    max_distance: Option<f64>,
    #[serde(default)]
    max_bearing_sec: Option<f64>,
    #[serde(default)]
    max_length_diff: Option<f64>,
}

impl ConflateLinesInput {
    fn source(&self) -> Vec<LineSegment> {
        self.source.iter().copied().map(LineInput::segment).collect()
    }
    fn target(&self) -> Vec<LineSegment> {
        self.target.iter().copied().map(LineInput::segment).collect()
    }
}

#[derive(Deserialize)]
struct ProjectInput {
    lat_deg: f64,
    lon_deg: f64,
    #[serde(default)]
    crs: String,
}

#[derive(Deserialize)]
struct UnprojectInput {
    x: f64,
    y: f64,
    #[serde(default)]
    crs: String,
}

#[derive(Deserialize)]
struct GridScaleInput {
    lat_deg: f64,
    lon_deg: f64,
    #[serde(default)]
    crs: String,
}

#[derive(Deserialize)]
struct LineDivideInput {
    from: Point,
    to: Point,
    parts: usize,
}

#[derive(Deserialize)]
struct OffsetPointInput {
    point: Point,
    bearing_deg: f64,
    offset: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_has_exactly_35_unique_tools() {
        let tools = catalog();
        assert_eq!(tools.len(), TOOL_COUNT);
        let mut ids: Vec<&str> = tools.iter().map(|t| t.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), TOOL_COUNT);
        for t in &tools {
            assert!(tool_spec(t.id).is_some());
            assert!(!t.description.is_empty());
            assert!(!t.inputs.is_empty());
        }
    }

    #[test]
    fn inverse_and_forward_are_consistent() {
        let input = json!({
            "from": {"x": 500000.0, "y": 1000000.0},
            "to": {"x": 500100.0, "y": 1000000.0}
        });
        let out = execute("inverse", &input).unwrap();
        assert!((out["bearing_deg"].as_f64().unwrap() - 90.0).abs() < 1e-9);
        assert!((out["distance"].as_f64().unwrap() - 100.0).abs() < 1e-9);

        let fwd = execute(
            "forward",
            &json!({
                "from": {"x": 500000.0, "y": 1000000.0},
                "bearing_deg": 90.0,
                "distance": 100.0
            }),
        )
        .unwrap();
        assert!((fwd["point"]["x"].as_f64().unwrap() - 500100.0).abs() < 1e-9);
    }

    #[test]
    fn traverse_tool_reports_closure_and_area() {
        let out = execute(
            "traverse",
            &json!({
                "start": {"x": 0.0, "y": 0.0},
                "legs": [
                    {"type": "line", "bearing_deg": 0.0, "distance": 100.0},
                    {"type": "line", "bearing_deg": 90.0, "distance": 100.0},
                    {"type": "line", "bearing_deg": 180.0, "distance": 100.0},
                    {"type": "line", "bearing_deg": 270.0, "distance": 100.0}
                ]
            }),
        )
        .unwrap();
        assert!((out["area"].as_f64().unwrap() - 10_000.0).abs() < 1e-6);
        assert!(out["checks"]["closes_on_start"].as_bool().unwrap());
        assert_eq!(out["vertices"].as_array().unwrap().len(), 5);
    }

    #[test]
    fn every_tool_accepts_a_sample_input() {
        for (id, input) in sample_inputs() {
            let result = execute(id, &input);
            assert!(result.is_ok(), "tool {id} failed: {result:?}");
        }
    }

    fn pt(x: f64, y: f64) -> Value {
        json!({"x": x, "y": y})
    }

    fn sample_inputs() -> Vec<(&'static str, Value)> {
        let line = json!({"type": "line", "bearing_deg": 0.0, "distance": 100.0});
        let curve = json!({"type": "curve", "radius": 50.0, "arc_length": 78.5398, "chord_bearing_deg": 0.0, "turn": "right"});
        let legs = json!([line, curve, {"type": "line", "bearing_deg": 180.0, "distance": 100.0}]);
        let points = json!([pt(0.0,0.0), pt(100.0,0.0), pt(0.0,100.0)]);
        let source = json!([pt(0.0,0.0), pt(100.0,0.0), pt(0.0,100.0)]);
        let target = json!([pt(500000.0,1000000.0), pt(500100.0,1000000.0), pt(500000.0,1000100.0)]);
        let lines = json!([{"from": pt(0.0,0.0), "to": pt(100.0,0.0)}, {"from": pt(100.0,0.0), "to": pt(100.0,100.0)}]);
        vec![
            ("inverse", json!({"from": pt(0.0,0.0), "to": pt(100.0,100.0)})),
            ("forward", json!({"from": pt(0.0,0.0), "bearing_deg": 45.0, "distance": 100.0})),
            ("traverse", json!({"start": pt(0.0,0.0), "legs": legs})),
            ("angle-convert", json!({"value": 37.5, "unit": "degrees"})),
            ("distance-convert", json!({"value": 1.0, "from": "km", "to": "m"})),
            ("point-on-line", json!({"from": pt(0.0,0.0), "to": pt(100.0,0.0), "distance": 25.0, "offset": 2.0})),
            ("perpendicular-foot", json!({"point": pt(10.0,5.0), "a": pt(0.0,0.0), "b": pt(100.0,0.0)})),
            ("intersect-bearings", json!({"p1": pt(0.0,0.0), "bearing1_deg": 0.0, "p2": pt(100.0,0.0), "bearing2_deg": 270.0})),
            ("intersect-lines", json!({"a1": pt(0.0,0.0), "a2": pt(100.0,0.0), "b1": pt(50.0,-50.0), "b2": pt(50.0,50.0)})),
            ("intersect-bearing-distance", json!({"from": pt(0.0,0.0), "bearing_deg": 90.0, "centre": pt(100.0,0.0), "radius": 10.0})),
            ("intersect-distances", json!({"c1": pt(0.0,0.0), "r1": 100.0, "c2": pt(100.0,0.0), "r2": 100.0})),
            ("offset-intersection", json!({"p1": pt(0.0,0.0), "bearing1_deg": 0.0, "offset1": 10.0, "p2": pt(100.0,0.0), "bearing2_deg": 270.0, "offset2": 5.0})),
            ("curve-radius-delta", json!({"pc": pt(0.0,0.0), "initial_bearing_deg": 0.0, "radius": 100.0, "delta_deg": 90.0, "turn": "right"})),
            ("curve-components", json!({"radius": 100.0, "delta_deg": 90.0})),
            ("curve-three-points", json!({"a": pt(0.0,0.0), "b": pt(50.0,30.0), "c": pt(100.0,0.0)})),
            ("curve-tangents", json!({"pi": pt(0.0,0.0), "incoming_bearing_deg": 0.0, "outgoing_bearing_deg": 90.0, "radius": 100.0})),
            ("curve-point-at", json!({"pc": pt(0.0,0.0), "initial_bearing_deg": 0.0, "radius": 100.0, "delta_deg": 90.0, "turn": "right", "arc_distance": 50.0})),
            ("spiral", json!({"radius": 300.0, "length": 60.0})),
            ("area", json!({"ring": points})),
            ("closure", json!({"start": pt(0.0,0.0), "legs": legs})),
            ("bowditch", json!({"start": pt(0.0,0.0), "legs": legs})),
            ("least-squares", json!({
                "coords": [pt(0.0,0.0), pt(100.0,0.0), pt(0.0,100.0)],
                "fixed": [true, true, false],
                "observations": [
                    {"from": 0, "to": 1, "distance": 100.0, "bearing_deg": 90.0},
                    {"from": 0, "to": 2, "distance": 100.0, "bearing_deg": 0.0},
                    {"from": 1, "to": 2, "distance": 141.421356, "bearing_deg": 315.0}
                ]
            })),
            ("area-check", json!({"computed": 10000.0, "stated": 10000.0})),
            ("rotate", json!({"points": points, "angle_deg": 90.0})),
            ("translate", json!({"points": points, "dx": 10.0, "dy": 20.0})),
            ("scale", json!({"points": points, "factor": 2.0})),
            ("helmert-fit", json!({"source": source, "target": target})),
            ("helmert-apply", json!({"points": points, "a": 1.0, "b": 0.0, "tx": 10.0, "ty": 20.0})),
            ("conflate-points", json!({"source": [pt(0.0,0.0), pt(100.0,0.0), pt(0.0,100.0)], "target": [pt(10.0,20.0), pt(110.0,20.0), pt(10.0,120.0)], "max_distance": 100.0})),
            ("conflate-lines", json!({"source": lines, "target": lines, "max_distance": 10.0})),
            ("project", json!({"lat_deg": 6.5, "lon_deg": 7.5, "crs": "utm32n"})),
            ("unproject", json!({"x": 500000.0, "y": 720000.0, "crs": "utm32n"})),
            ("grid-scale-factor", json!({"lat_deg": 6.5, "lon_deg": 7.5, "crs": "utm32n"})),
            ("line-divide", json!({"from": pt(0.0,0.0), "to": pt(100.0,0.0), "parts": 4})),
            ("offset-point", json!({"point": pt(10.0,10.0), "bearing_deg": 0.0, "offset": 5.0})),
        ]
    }
}
