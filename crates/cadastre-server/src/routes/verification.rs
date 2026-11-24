//! Checking and Data Verification unit (`/verification`; docs/checking-and-data-verification.md).
//!
//! The unit checks a surveyor's boundary (imported from CAD or a coordinate list, or sketched)
//! before any Digitising job exists. Check-only mode runs the system check's land checks on the
//! boundary and saves nothing: overlap with committed parcels here and in Aumentum, the drop
//! point, a title on the overlapped land and gaps to neighbours.

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::Row;
use utoipa::ToSchema;

use crate::auth::AuthUser;
use crate::error::{ApiResult, AppError};
use crate::routes::precheck::{self, PrecheckReport};
use crate::state::AppState;

#[derive(Debug, Deserialize, ToSchema)]
pub struct VerificationCheckRequest {
    /// The boundary's corners (UTM 32N metres, ring order, not closed).
    pub vertices: Vec<[f64; 2]>,
}

/// Check a boundary without a job, saving nothing (the unit's check-only mode).
#[utoipa::path(post, path = "/api/v1/verification/check", tag = "verification",
    request_body = VerificationCheckRequest,
    responses((status = 200, body = PrecheckReport), (status = 400, description = "Fewer than 3 corners, or corners that are not numbers")))]
pub async fn check(State(state): State<AppState>, user: AuthUser, Json(req): Json<VerificationCheckRequest>) -> ApiResult<Json<PrecheckReport>> {
    user.require_role("editor")?;
    if req.vertices.len() < 3 {
        return Err(AppError::BadRequest("a boundary needs at least 3 corners".into()));
    }
    Ok(Json(precheck::run(&state, None, &req.vertices, None).await?))
}

// ---------------------------------------------------------------------------------------------
// V3: the adjoining coordinates, for the surveyor to conform to
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Deserialize, ToSchema)]
pub struct AdjoiningRequest {
    /// The boundary's corners (UTM 32N metres, ring order, not closed).
    pub vertices: Vec<[f64; 2]>,
    /// How close (m) a parcel must come to adjoin, and its corners to be listed: 0.05 to 20, default 2.
    #[serde(default)]
    pub within_m: Option<f64>,
}

/// One corner of a neighbour, as the database has it.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AdjoiningCorner {
    /// Beacon number, when a beacon is there.
    pub beacon: Option<String>,
    pub easting: f64,
    pub northing: f64,
    /// `corner` (one of the boundary's corners is near), `line` (near one of its sides only), or
    /// `line_end` (further off: the end of a neighbour's side that runs along the boundary).
    pub kind: String,
    /// The boundary's corner it is compared with (1-based), for `corner`.
    pub your_corner: Option<usize>,
    /// Database minus boundary corner (m), for `corner`.
    pub d_east: Option<f64>,
    pub d_north: Option<f64>,
    /// Distance (m) to that corner, else to the boundary's nearest side.
    pub off: f64,
    /// Within the tolerance; null for `line_end`.
    pub agrees: Option<bool>,
    /// The neighbour's side from here to the next listed corner is a side of its own.
    pub joined_to_next: bool,
}

/// A corner of the boundary near a neighbour's side, with no neighbour's corner to compare with.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct OffLine {
    pub your_corner: usize,
    /// Distance (m) to the neighbour's nearest side.
    pub off: f64,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AdjoiningParcel {
    /// UPIN; empty for an Aumentum parcel without one.
    pub upin: String,
    /// `cadastre`, or `aumentum` (live there, not in this Cadastre).
    pub source: String,
    /// `overlaps`, `side` (shares a side) or `point` (meets at a corner or comes close).
    pub relation: String,
    /// Least distance (m); 0 when they touch.
    pub gap: f64,
    pub overlap_m2: f64,
    pub corners: Vec<AdjoiningCorner>,
    pub off_line: Vec<OffLine>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AdjoiningReport {
    pub within_m: f64,
    /// Corners this close (m) agree (`commit.beacon_tolerance_m`).
    pub tolerance_m: f64,
    pub parcels: Vec<AdjoiningParcel>,
    /// Why the list may be incomplete.
    pub warnings: Vec<String>,
    pub queried_at: String,
}

type P = [f64; 2];
fn apart(a: P, b: P) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}
fn to_segment(p: P, a: P, b: P) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let l2 = dx * dx + dy * dy;
    let t = if l2 > 0.0 { (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / l2).clamp(0.0, 1.0) } else { 0.0 };
    apart(p, [a[0] + t * dx, a[1] + t * dy])
}
fn to_ring(p: P, ring: &[P]) -> f64 {
    (0..ring.len()).map(|i| to_segment(p, ring[i], ring[(i + 1) % ring.len()])).fold(f64::INFINITY, f64::min)
}
fn mm(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

/// Whether a side runs along `ring`: at least 4 × `within` of it, or 60 % of a side too
/// short for that, lies within `within` of the ring.
fn runs_along(a: P, b: P, ring: &[P], within: f64) -> bool {
    const STEPS: usize = 40;
    let box_of = |pts: &mut dyn Iterator<Item = P>| pts.fold([f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY],
        |c, p| [c[0].min(p[0]), c[1].min(p[1]), c[2].max(p[0]), c[3].max(p[1])]);
    let (s, r) = (box_of(&mut [a, b].into_iter()), box_of(&mut ring.iter().copied()));
    if s[0] > r[2] + within || s[2] < r[0] - within || s[1] > r[3] + within || s[3] < r[1] - within {
        return false;
    }
    // Only the part of the side inside the ring's box is sampled: a long side may pass a small plot.
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    for k in 0..2 {
        let d = b[k] - a[k];
        let (lo, hi) = (r[k] - within, r[k + 2] + within);
        if d.abs() > 1e-9 {
            let (u, v) = ((lo - a[k]) / d, (hi - a[k]) / d);
            t0 = t0.max(u.min(v));
            t1 = t1.min(u.max(v));
        }
    }
    if t1 <= t0 {
        return false;
    }
    let part = apart(a, b) * (t1 - t0);
    let near = (0..STEPS).filter(|i| {
        let t = t0 + (t1 - t0) * (*i as f64 + 0.5) / STEPS as f64;
        to_ring([a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])], ring) <= within
    }).count();
    part * near as f64 / STEPS as f64 > (4.0 * within).min(apart(a, b) * 0.6)
}

/// A neighbour's rings against the boundary.
fn compare(rings: &[Vec<P>], own: &[P], within: f64, tol: f64) -> (Vec<AdjoiningCorner>, Vec<OffLine>, bool) {
    let mut corners = Vec::new();
    let mut side = false;
    let mut matched = vec![false; own.len()];
    for ring in rings.iter().filter(|r| r.len() >= 3) {
        let n = ring.len();
        let along: Vec<bool> = (0..n).map(|i| runs_along(ring[i], ring[(i + 1) % n], own, within)).collect();
        side |= along.iter().any(|a| *a);
        let mut listed: Vec<Option<AdjoiningCorner>> = Vec::with_capacity(n);
        for i in 0..n {
            let p = ring[i];
            let (k, d) = own.iter().enumerate().map(|(k, q)| (k, apart(p, *q))).min_by(|a, b| a.1.total_cmp(&b.1)).unwrap_or((0, f64::INFINITY));
            let line = to_ring(p, own);
            let at = |kind: &str, your: Option<usize>, off: f64, agrees: Option<bool>| AdjoiningCorner {
                beacon: None, easting: mm(p[0]), northing: mm(p[1]), kind: kind.into(), your_corner: your.map(|k| k + 1),
                d_east: your.map(|k| mm(p[0] - own[k][0])), d_north: your.map(|k| mm(p[1] - own[k][1])), off: mm(off), agrees, joined_to_next: false,
            };
            listed.push(if d <= within {
                matched[k] = true;
                Some(at("corner", Some(k), d, Some(d <= tol)))
            } else if line <= within {
                Some(at("line", None, line, Some(line <= tol)))
            } else if along[i] || along[(i + n - 1) % n] {
                Some(at("line_end", None, line, None))
            } else {
                None
            });
        }
        for i in 0..n {
            let next = listed[(i + 1) % n].is_some();
            if let Some(c) = listed[i].as_mut() {
                c.joined_to_next = next;
            }
        }
        // Start after a break, so that joined corners follow one another.
        let start = (0..n).find(|i| listed[*i].is_some() && listed[(i + n - 1) % n].is_none()).unwrap_or(0);
        for i in 0..n {
            if let Some(c) = listed[(start + i) % n].take() {
                corners.push(c);
            }
        }
    }
    let off_line = own.iter().enumerate().filter(|(k, _)| !matched[*k]).filter_map(|(k, p)| {
        let off = rings.iter().filter(|r| r.len() >= 3).map(|r| to_ring(*p, r)).fold(f64::INFINITY, f64::min);
        (off <= within).then(|| OffLine { your_corner: k + 1, off: mm(off) })
    }).collect();
    (corners, off_line, side)
}

/// The rings of a GeoJSON Polygon or MultiPolygon, not closed, without repeated points.
fn rings_of(geojson: &str) -> Vec<Vec<P>> {
    let v: Value = serde_json::from_str(geojson).unwrap_or(Value::Null);
    let one = |r: &Value| -> Vec<P> {
        let mut pts: Vec<P> = r.as_array().into_iter().flatten().filter_map(|c| Some([c[0].as_f64()?, c[1].as_f64()?])).collect();
        pts.dedup_by(|a, b| apart(*a, *b) < 1e-4);
        if pts.len() > 1 && apart(pts[0], pts[pts.len() - 1]) < 1e-4 {
            pts.pop();
        }
        pts
    };
    let polys: Vec<&Value> = match v["type"].as_str() {
        Some("Polygon") => vec![&v["coordinates"]],
        Some("MultiPolygon") => v["coordinates"].as_array().into_iter().flatten().collect(),
        _ => vec![],
    };
    polys.into_iter().flat_map(|p| p.as_array().into_iter().flatten()).map(one).collect()
}

const MAX_NEIGHBOURS: usize = 100;

/// The parcels adjoining a boundary and their corners as the database has them, compared with the
/// boundary's: committed parcels here, and Aumentum's live parcels that are not here. Saves nothing.
#[utoipa::path(post, path = "/api/v1/verification/adjoining", tag = "verification",
    request_body = AdjoiningRequest,
    responses((status = 200, body = AdjoiningReport), (status = 400, description = "Fewer than 3 corners, or corners that are not numbers")))]
pub async fn adjoining(State(state): State<AppState>, user: AuthUser, Json(req): Json<AdjoiningRequest>) -> ApiResult<Json<AdjoiningReport>> {
    user.require_role("editor")?;
    let own = &req.vertices;
    if own.len() < 3 {
        return Err(AppError::BadRequest("a boundary needs at least 3 corners".into()));
    }
    if own.len() > 2000 {
        return Err(AppError::BadRequest("too many corners".into()));
    }
    if own.iter().flatten().any(|v| !v.is_finite()) {
        return Err(AppError::BadRequest("vertices must be numbers (UTM 32N metres)".into()));
    }
    let within = req.within_m.unwrap_or(2.0);
    if !(0.05..=20.0).contains(&within) {
        return Err(AppError::BadRequest("the distance must be 0.05 to 20 m".into()));
    }
    let mut conn = state.pool.acquire().await?;
    let tol = crate::routes::wfbeacons::tolerance(&mut conn).await?;
    let width = precheck::overlap_min_width(&mut conn).await?;
    let mut closed = own.clone();
    closed.push(own[0]);
    let shape = json!({ "type": "Polygon", "coordinates": [closed] }).to_string();
    let g = "ST_MakeValid(ST_SetSRID(ST_GeomFromGeoJSON($1), 32632))";
    let mut warnings = Vec::new();
    let mut found: Vec<(String, &str, String)> = sqlx::query_as::<_, (String, String)>(&format!(
        "SELECT COALESCE(p.parcel_number, ''), ST_AsGeoJSON(p.geom, 6) FROM cadastre.parcel p WHERE p.status = 'posted' AND ST_DWithin(p.geom, {g}, $2) \
         ORDER BY ST_Distance(p.geom, {g}), p.parcel_number LIMIT {}", MAX_NEIGHBOURS + 1))
        .bind(&shape).bind(within).fetch_all(&mut *conn).await?
        .into_iter().map(|(u, s)| (u, "cadastre", s)).collect();
    let fold = |k: usize, f: fn(f64, f64) -> f64| own.iter().map(|p| p[k]).reduce(f).unwrap_or(0.0);
    let env = (fold(0, f64::min) - within, fold(1, f64::min) - within, fold(0, f64::max) + within, fold(1, f64::max) + within);
    match precheck::aumentum_parcels(env).await {
        None => warnings.push("Aumentum's parcels could not be looked up: the Aumentum link is not configured on this server.".to_string()),
        Some(Err(e)) => warnings.push(e),
        Some(Ok(list)) => {
            let new: Vec<(String, String)> = list.into_iter().filter(|(u, _)| u.is_empty() || !found.iter().any(|f| &f.0 == u)).collect();
            // Known here, in whatever state (committed elsewhere, retired): this Cadastre's word goes.
            let upins: Vec<String> = new.iter().map(|n| n.0.clone()).filter(|u| !u.is_empty()).collect();
            let known: Vec<String> = sqlx::query_scalar("SELECT DISTINCT parcel_number FROM cadastre.parcel WHERE parcel_number = ANY($1) AND status IN ('posted', 'historic', 'archived')")
                .bind(&upins).fetch_all(&mut *conn).await?;
            for (u, s) in new {
                if !known.contains(&u) { found.push((u, "aumentum", s)); }
            }
        }
    }
    if found.len() > MAX_NEIGHBOURS {
        warnings.push(format!("More than {MAX_NEIGHBOURS} parcels adjoin this boundary; only the first {MAX_NEIGHBOURS} are listed."));
        found.truncate(MAX_NEIGHBOURS);
    }
    let mut parcels = Vec::new();
    for (upin, source, other) in found {
        let o = "ST_MakeValid(ST_SetSRID(ST_GeomFromGeoJSON($2), 32632))";
        let row = sqlx::query(&format!(
            "SELECT ST_Distance(o, g) AS d, ST_Area(ST_Intersection(o, g)) AS a, {} AS counts FROM (SELECT {o} AS o, {g} AS g) s", precheck::counts_sql("o", "g", "$3")))
            .bind(&shape).bind(&other).bind(width).fetch_one(&mut *conn).await;
        let Ok(row) = row else {
            warnings.push(format!("The shape of {} could not be read.", if upin.is_empty() { "an Aumentum parcel without UPIN" } else { &upin }));
            continue;
        };
        let gap: f64 = row.get::<Option<f64>, _>("d").unwrap_or(0.0);
        if gap > within {
            continue;
        }
        let (corners, off_line, side) = compare(&rings_of(&other), own, within, tol);
        let overlaps = row.get::<Option<bool>, _>("counts").unwrap_or(false);
        parcels.push(AdjoiningParcel {
            upin, source: source.into(), relation: if overlaps { "overlaps" } else if side { "side" } else { "point" }.into(),
            gap: mm(gap), overlap_m2: if overlaps { (row.get::<Option<f64>, _>("a").unwrap_or(0.0) * 100.0).round() / 100.0 } else { 0.0 },
            corners, off_line,
        });
    }
    // Beacon numbers, by position.
    let (es, ns): (Vec<f64>, Vec<f64>) = parcels.iter().flat_map(|p| &p.corners).map(|c| (c.easting, c.northing)).unzip();
    let names: Vec<Option<String>> = sqlx::query_scalar(
        "SELECT b.number FROM unnest($1::float8[], $2::float8[]) WITH ORDINALITY AS t(e, n, i) \
         LEFT JOIN LATERAL (SELECT bb.number FROM cadastre.beacon bb WHERE ST_DWithin(bb.geom, ST_SetSRID(ST_MakePoint(t.e, t.n), 32632), $3) \
                            ORDER BY bb.geom <-> ST_SetSRID(ST_MakePoint(t.e, t.n), 32632) LIMIT 1) b ON true ORDER BY t.i")
        .bind(&es).bind(&ns).bind(tol).fetch_all(&mut *conn).await?;
    for (c, name) in parcels.iter_mut().flat_map(|p| p.corners.iter_mut()).zip(names) {
        c.beacon = name;
    }
    Ok(Json(AdjoiningReport { within_m: within, tolerance_m: tol, parcels, warnings, queried_at: chrono::Utc::now().to_rfc3339() }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const OWN: [P; 4] = [[0.0, 0.0], [30.0, 0.0], [30.0, 30.0], [0.0, 30.0]];

    #[test]
    fn a_neighbour_sharing_a_side_lists_its_two_corners() {
        let east = vec![[30.02, 0.0], [60.0, 0.0], [60.0, 30.0], [30.2, 30.1]];
        let (c, off, side) = compare(&[east], &OWN, 2.0, 0.05);
        assert!(side && off.is_empty());
        assert_eq!(c.len(), 2, "{c:?}");
        // Ring order, starting after the break, so the two are joined.
        assert_eq!((c[0].your_corner, c[0].agrees, c[0].joined_to_next), (Some(3), Some(false), true));
        assert_eq!((c[0].d_east, c[0].d_north, c[0].off), (Some(0.2), Some(0.1), 0.224));
        assert_eq!((c[1].your_corner, c[1].agrees, c[1].joined_to_next), (Some(2), Some(true), false));
    }

    #[test]
    fn a_longer_side_gives_its_ends_and_our_corners_off_its_line() {
        let south = vec![[-500.0, -0.1], [-500.0, -40.0], [600.0, -40.0], [600.0, -0.1]];
        let (c, off, side) = compare(&[south], &OWN, 1.0, 0.05);
        assert!(side);
        assert_eq!(c.iter().map(|c| c.kind.as_str()).collect::<Vec<_>>(), ["line_end", "line_end"]);
        assert!(c[0].joined_to_next && c[0].easting == 600.0 && c[0].agrees.is_none());
        assert_eq!(off.iter().map(|o| (o.your_corner, o.off)).collect::<Vec<_>>(), [(1, 0.1), (2, 0.1)]);
    }

    #[test]
    fn a_corner_on_our_line_and_one_met_at_a_point() {
        let north = vec![[10.0, 30.01], [10.0, 50.0], [20.0, 50.0], [20.0, 30.01]];
        let (c, _, side) = compare(&[north], &OWN, 1.0, 0.05);
        assert!(side);
        assert!(c.len() == 2 && c.iter().all(|c| c.kind == "line" && c.agrees == Some(true)), "{c:?}");
        let diagonal = vec![[30.0, 30.0], [60.0, 30.0], [60.0, 60.0], [30.0, 60.0]];
        let (c, off, side) = compare(&[diagonal], &OWN, 2.0, 0.05);
        assert!(!side, "only meets at a corner");
        assert!(c.len() == 1 && c[0].your_corner == Some(3) && off.is_empty(), "{c:?} {off:?}");
    }

    #[test]
    fn rings_are_read_open_and_without_repeats() {
        let r = rings_of(r#"{"type":"MultiPolygon","coordinates":[[[[0,0],[1,0],[1,0],[1,1],[0,0]]]]}"#);
        assert_eq!(r, vec![vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]]]);
    }
}
