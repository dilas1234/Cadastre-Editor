//! Adjustment: the Bowditch (compass) rule for a single traverse (FR-COG-06) and a
//! weighted least-squares network adjustment holding control fixed (FR-ADJ-01..03).

use nalgebra::{DMatrix, DVector};
use serde::{Deserialize, Serialize};

use crate::cogo::{self, traverse_points};
use crate::error::{CoreError, Result};
use crate::types::{Point, Traverse};
use crate::units::Bearing;

/// Apply the Bowditch (compass) rule to a traverse, distributing the misclosure across
/// vertices in proportion to the cumulative distance travelled.
///
/// FR-COG-06. Returns the adjusted vertices (`segments.len() + 1` points); the last vertex
/// is pulled back onto the start.
pub fn bowditch(traverse: &Traverse) -> Result<Vec<Point>> {
    cogo::require_min_courses(traverse, 1)?;

    let raw = traverse_points(traverse);
    let start = raw[0];
    let end = *raw.last().expect("at least start + one leg");
    let misclosure_e = end.x - start.x;
    let misclosure_n = end.y - start.y;

    let perimeter = cogo::traverse_length(traverse);
    if perimeter == 0.0 {
        return Ok(raw);
    }

    let mut adjusted = Vec::with_capacity(raw.len());
    let mut run = 0.0;
    adjusted.push(start);
    for (i, segment) in traverse.segments.iter().enumerate() {
        run += segment.as_course().distance;
        let fraction = run / perimeter;
        let p = raw[i + 1];
        adjusted.push(Point::new(
            p.x - misclosure_e * fraction,
            p.y - misclosure_n * fraction,
        ));
    }
    adjusted
        .iter()
        .for_each(|p| debug_assert!(p.x.is_finite() && p.y.is_finite()));
    Ok(adjusted)
}

/// One observed line between two beacons, with separate weights for its distance and bearing.
///
/// Weights are `1/σ²` in consistent units (metres for distance, radians for bearing).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Observation {
    /// Index of the "from" beacon in the coordinate list.
    pub from: usize,
    /// Index of the "to" beacon in the coordinate list.
    pub to: usize,
    /// Measured distance (metres).
    pub distance: f64,
    /// Measured whole-circle bearing.
    pub bearing: Bearing,
    /// Weight on the distance observation (`1/σ_d²`).
    pub distance_weight: f64,
    /// Weight on the bearing observation (`1/σ_β²`, with σ_β in radians).
    pub bearing_weight: f64,
}

impl Observation {
    /// An observation with weights derived from standard deviations (metres and seconds of arc).
    pub fn with_sigmas(
        from: usize,
        to: usize,
        distance: f64,
        bearing: Bearing,
        sigma_distance_m: f64,
        sigma_bearing_sec: f64,
    ) -> Self {
        let sigma_bearing_rad = (sigma_bearing_sec / 3600.0).to_radians();
        Self {
            from,
            to,
            distance,
            bearing,
            distance_weight: 1.0 / (sigma_distance_m * sigma_distance_m),
            bearing_weight: 1.0 / (sigma_bearing_rad * sigma_bearing_rad),
        }
    }
}

/// The input to a least-squares network adjustment.
#[derive(Clone, Debug, PartialEq)]
pub struct AdjustmentInput {
    /// Initial (approximate) coordinates for every beacon, indexed.
    pub coords: Vec<Point>,
    /// Per-beacon control mask: `true` beacons are held fixed.
    pub fixed: Vec<bool>,
    /// Observed lines.
    pub observations: Vec<Observation>,
}

/// The residual of one observation after adjustment.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct LineResidual {
    /// Index of the observation in the input list.
    pub observation: usize,
    /// Distance residual `observed - adjusted` (metres).
    pub distance_residual: f64,
    /// Bearing residual `observed - adjusted` (seconds of arc).
    pub bearing_residual_sec: f64,
}

/// The result of a least-squares adjustment.
#[derive(Clone, Debug, PartialEq)]
pub struct Adjustment {
    /// Adjusted coordinates (fixed beacons unchanged).
    pub coords: Vec<Point>,
    /// Per-line residuals.
    pub residuals: Vec<LineResidual>,
    /// Iterations taken to converge.
    pub iterations: usize,
    /// Largest coordinate shift in the final iteration (metres).
    pub final_shift: f64,
}

impl Adjustment {
    /// Observations whose residuals exceed either tolerance — likely blunders (FR-ADJ-03).
    pub fn blunders(&self, distance_tol_m: f64, bearing_tol_sec: f64) -> Vec<usize> {
        self.residuals
            .iter()
            .filter(|r| {
                r.distance_residual.abs() > distance_tol_m
                    || r.bearing_residual_sec.abs() > bearing_tol_sec
            })
            .map(|r| r.observation)
            .collect()
    }
}

/// Run a weighted least-squares adjustment over the free beacons, holding control fixed,
/// and report residuals per line (FR-ADJ-01, FR-ADJ-02 preview, FR-ADJ-03 blunders).
///
/// Observations are distances and bearings. Solved by Gauss–Newton on the normal equations
/// via a Cholesky factorisation. `tol` is the convergence threshold on the largest coordinate
/// shift (metres); `max_iter` bounds the iterations.
pub fn least_squares(input: &AdjustmentInput, max_iter: usize, tol: f64) -> Result<Adjustment> {
    let n = input.coords.len();
    if input.fixed.len() != n {
        return Err(CoreError::BeaconIndexOutOfRange(input.fixed.len()));
    }
    if !input.fixed.iter().any(|&f| f) {
        return Err(CoreError::NoControl);
    }
    for obs in &input.observations {
        if obs.from >= n {
            return Err(CoreError::BeaconIndexOutOfRange(obs.from));
        }
        if obs.to >= n {
            return Err(CoreError::BeaconIndexOutOfRange(obs.to));
        }
    }

    // Column index for each free beacon's (x, y) unknowns; `None` for fixed beacons.
    let mut col_of = vec![None; n];
    let mut num_free = 0usize;
    for (i, &fixed) in input.fixed.iter().enumerate() {
        if !fixed {
            col_of[i] = Some(num_free * 2);
            num_free += 1;
        }
    }
    let unknowns = num_free * 2;
    if unknowns == 0 {
        // Nothing to solve; everything is control.
        return Ok(Adjustment {
            coords: input.coords.clone(),
            residuals: residuals(&input.coords, &input.observations),
            iterations: 0,
            final_shift: 0.0,
        });
    }

    let rows = input.observations.len() * 2;
    let mut coords = input.coords.clone();

    for iter in 1..=max_iter {
        let mut a = DMatrix::<f64>::zeros(rows, unknowns);
        let mut wa = DMatrix::<f64>::zeros(rows, unknowns);
        let mut wl = DVector::<f64>::zeros(rows);

        for (o, obs) in input.observations.iter().enumerate() {
            let pa = coords[obs.from];
            let pb = coords[obs.to];
            let dx = pb.x - pa.x;
            let dy = pb.y - pa.y;
            let d2 = dx * dx + dy * dy;
            let d = d2.sqrt();
            if d < 1e-9 {
                return Err(CoreError::SingularNormalMatrix);
            }

            // --- distance observation (row 2o) ---
            let rd = 2 * o;
            let (ddx_b, ddy_b) = (dx / d, dy / d); // ∂d/∂xB, ∂d/∂yB
            set_row(
                &mut a,
                &mut wa,
                rd,
                obs.distance_weight,
                col_of[obs.from],
                col_of[obs.to],
                -ddx_b,
                -ddy_b,
                ddx_b,
                ddy_b,
            );
            let l_d = obs.distance - d;
            wl[rd] = obs.distance_weight * l_d;

            // --- bearing observation (row 2o+1) ---
            let rb = 2 * o + 1;
            let (dbx_b, dby_b) = (dy / d2, -dx / d2); // ∂β/∂xB, ∂β/∂yB with β = atan2(dx,dy)
            set_row(
                &mut a,
                &mut wa,
                rb,
                obs.bearing_weight,
                col_of[obs.from],
                col_of[obs.to],
                -dbx_b,
                -dby_b,
                dbx_b,
                dby_b,
            );
            let computed_bearing = dx.atan2(dy);
            let l_b = wrap_to_pi(obs.bearing.radians() - computed_bearing);
            wl[rb] = obs.bearing_weight * l_b;
        }

        // Normal equations: N = AᵀWA, rhs = AᵀWl.
        let normal = a.transpose() * &wa;
        let rhs = a.transpose() * &wl;

        let chol = normal.cholesky().ok_or(CoreError::SingularNormalMatrix)?;
        let delta = chol.solve(&rhs);

        // Apply the corrections and track the largest shift.
        let mut max_shift = 0.0f64;
        for (i, col) in col_of.iter().enumerate() {
            if let Some(c) = col {
                let dxc = delta[*c];
                let dyc = delta[*c + 1];
                coords[i].x += dxc;
                coords[i].y += dyc;
                max_shift = max_shift.max(dxc.hypot(dyc));
            }
        }
        if max_shift < tol {
            return Ok(Adjustment {
                coords: coords.clone(),
                residuals: residuals(&coords, &input.observations),
                iterations: iter,
                final_shift: max_shift,
            });
        }
    }

    Err(CoreError::NoConvergence(max_iter))
}

/// Fill one observation row of the design matrix `a` and its weighted copy `wa`.
#[allow(clippy::too_many_arguments)]
fn set_row(
    a: &mut DMatrix<f64>,
    wa: &mut DMatrix<f64>,
    row: usize,
    weight: f64,
    col_from: Option<usize>,
    col_to: Option<usize>,
    d_from_x: f64,
    d_from_y: f64,
    d_to_x: f64,
    d_to_y: f64,
) {
    if let Some(c) = col_from {
        a[(row, c)] = d_from_x;
        a[(row, c + 1)] = d_from_y;
        wa[(row, c)] = weight * d_from_x;
        wa[(row, c + 1)] = weight * d_from_y;
    }
    if let Some(c) = col_to {
        a[(row, c)] = d_to_x;
        a[(row, c + 1)] = d_to_y;
        wa[(row, c)] = weight * d_to_x;
        wa[(row, c + 1)] = weight * d_to_y;
    }
}

/// Residuals (observed - computed) for each observation at the given coordinates.
fn residuals(coords: &[Point], observations: &[Observation]) -> Vec<LineResidual> {
    observations
        .iter()
        .enumerate()
        .map(|(o, obs)| {
            let (bearing, distance) = cogo::inverse(coords[obs.from], coords[obs.to]);
            let distance_residual = obs.distance - distance;
            let bearing_residual_rad = wrap_to_pi(obs.bearing.radians() - bearing.radians());
            LineResidual {
                observation: o,
                distance_residual,
                bearing_residual_sec: bearing_residual_rad.to_degrees() * 3600.0,
            }
        })
        .collect()
}

/// Wrap an angular difference into `(-π, π]`.
fn wrap_to_pi(mut r: f64) -> f64 {
    use std::f64::consts::{PI, TAU};
    r = (r + PI) % TAU;
    if r <= 0.0 {
        r += TAU;
    }
    r - PI
}
