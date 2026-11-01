//! Alignment and conflation: fit a survey onto existing control and match beacons/lines
//! (SRS scope: "least-squares adjustment and joining new work to the existing fabric").
//!
//! Conflation is iterative. It matches the nearest candidate within tolerance, fits a
//! 4-parameter Helmert transform from the matches, reapplies it, and repeats until the match
//! set is stable. That handles the usual case where a new survey is a few metres out and
//! rotated slightly relative to the fabric.

use serde::{Deserialize, Serialize};

use crate::cogo;
use crate::error::{CoreError, Result};
use crate::transform::{fit_similarity, Similarity};
use crate::types::Point;

/// Tolerances for matching new work against existing geometry.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConflationTolerances {
    /// Maximum separation of a matching beacon/line midpoint (metres).
    pub max_distance: f64,
    /// Maximum bearing difference for a line match (arcseconds).
    pub max_bearing_sec: f64,
    /// Maximum length difference for a line match (metres).
    pub max_length_diff: f64,
    /// Minimum number of matches needed to fit a transform.
    pub min_matches: usize,
}

impl Default for ConflationTolerances {
    fn default() -> Self {
        Self {
            max_distance: 2.0,
            max_bearing_sec: 60.0,
            max_length_diff: 0.5,
            min_matches: 2,
        }
    }
}

/// A matched beacon pair.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PointMatch {
    /// Index into the source list.
    pub source: usize,
    /// Index into the target list.
    pub target: usize,
    /// Separation after the current transform (metres).
    pub distance: f64,
}

/// A straight line used for line conflation.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct LineSegment {
    /// Start point.
    pub from: Point,
    /// End point.
    pub to: Point,
}

impl LineSegment {
    /// A segment between two points.
    pub const fn new(from: Point, to: Point) -> Self {
        Self { from, to }
    }

    /// Length (m).
    pub fn length(self) -> f64 {
        self.from.distance_to(self.to)
    }

    /// Whole-circle bearing from start to end (decimal degrees).
    pub fn bearing_deg(self) -> f64 {
        cogo::inverse(self.from, self.to).0.degrees()
    }

    /// Midpoint.
    pub fn midpoint(self) -> Point {
        cogo::midpoint(self.from, self.to, 0.5)
    }
}

/// A matched line pair.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct LineMatch {
    /// Index into the source list.
    pub source: usize,
    /// Index into the target list.
    pub target: usize,
    /// Distance between midpoints after the current transform (metres).
    pub midpoint_distance: f64,
    /// Bearing difference (arcseconds, absolute).
    pub bearing_diff_sec: f64,
    /// Length difference (metres, absolute).
    pub length_diff: f64,
    /// Whether the target line runs in the opposite direction to the source.
    pub reversed: bool,
}

/// The result of conflating a new point set onto existing control.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PointConflation {
    /// Fitted transformation (source → target).
    pub transform: Similarity,
    /// Source points after the transformation.
    pub transformed_source: Vec<Point>,
    /// Matched control pairs.
    pub matches: Vec<PointMatch>,
    /// Source indices with no match.
    pub unmatched_source: Vec<usize>,
    /// Target indices with no match.
    pub unmatched_target: Vec<usize>,
    /// RMS of matched residuals (metres).
    pub rms: f64,
    /// Largest matched residual (metres).
    pub max_residual: f64,
    /// Iterations taken.
    pub iterations: usize,
}

/// The result of conflating new linework onto existing linework.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LineConflation {
    /// Fitted transformation (source → target).
    pub transform: Similarity,
    /// Matched line pairs.
    pub matches: Vec<LineMatch>,
    /// Source indices with no match.
    pub unmatched_source: Vec<usize>,
    /// Target indices with no match.
    pub unmatched_target: Vec<usize>,
    /// RMS of matched endpoint residuals (metres).
    pub rms: f64,
    /// Largest matched endpoint residual (metres).
    pub max_residual: f64,
    /// Iterations taken.
    pub iterations: usize,
}

/// Greedy nearest-neighbour matching under `max_distance`, one-to-one in both directions.
pub fn nearest_matches(source: &[Point], target: &[Point], max_distance: f64) -> Vec<PointMatch> {
    let mut candidates = Vec::new();
    for (si, s) in source.iter().enumerate() {
        for (ti, t) in target.iter().enumerate() {
            let distance = s.distance_to(*t);
            if distance <= max_distance {
                candidates.push(PointMatch {
                    source: si,
                    target: ti,
                    distance,
                });
            }
        }
    }
    candidates.sort_by(|a, b| a.distance.total_cmp(&b.distance));
    let mut used_source = vec![false; source.len()];
    let mut used_target = vec![false; target.len()];
    let mut matches = Vec::new();
    for c in candidates {
        if !used_source[c.source] && !used_target[c.target] {
            used_source[c.source] = true;
            used_target[c.target] = true;
            matches.push(c);
        }
    }
    matches.sort_by_key(|m| m.source);
    matches
}

/// The arithmetic mean of a point set.
fn centroid(points: &[Point]) -> Point {
    if points.is_empty() {
        return Point::new(0.0, 0.0);
    }
    let n = points.len() as f64;
    Point::new(
        points.iter().map(|p| p.x).sum::<f64>() / n,
        points.iter().map(|p| p.y).sum::<f64>() / n,
    )
}

/// Root-mean-square distance of a point set from its centroid.
fn rms_radius(points: &[Point], centre: Point) -> f64 {
    if points.is_empty() {
        return 0.0;
    }
    (points
        .iter()
        .map(|p| p.distance_to(centre).powi(2))
        .sum::<f64>()
        / points.len() as f64)
        .sqrt()
}

/// Build the similarity that maps `source_centre` onto `target_centre` with a rotation and scale.
fn similarity_about(
    source_centre: Point,
    target_centre: Point,
    angle: f64,
    scale: f64,
) -> Similarity {
    let a = scale * angle.cos();
    let b = scale * angle.sin();
    Similarity {
        a,
        b,
        tx: target_centre.x - (a * source_centre.x - b * source_centre.y),
        ty: target_centre.y - (b * source_centre.x + a * source_centre.y),
    }
}

/// Bootstrap a rough source → target transform before strict matching.
///
/// Conflation usually starts with a survey that is rotated and scaled slightly as well as
/// translated. Matching nearest points under a tight distance therefore finds nothing on the
/// first pass. We first align centroids, then derive candidate rotations from the directions of
/// the points farthest from each centroid (the most reliable control) and keep the candidate
/// that matches the most points. The strict iterative refinement then takes over.
fn bootstrap_similarity(
    source: &[Point],
    target: &[Point],
    max_distance: f64,
) -> Similarity {
    let source_centre = centroid(source);
    let target_centre = centroid(target);
    let source_radius = rms_radius(source, source_centre);
    let target_radius = rms_radius(target, target_centre);
    let estimated_scale = if source_radius > 1e-9 && target_radius > 1e-9 {
        target_radius / source_radius
    } else {
        1.0
    };

    // Identity must be scored too: when a few outliers drag the centroid, identity can already
    // match the bulk of the control better than any centroid-based candidate.
    let mut best = Similarity::identity();
    let mut best_count = nearest_matches(source, target, max_distance).len();

    // The extreme points define the orientation best.
    let extremes = |points: &[Point], centre: Point| {
        let mut idx: Vec<usize> = (0..points.len()).collect();
        idx.sort_by(|&a, &b| {
            points[b]
                .distance_to(centre)
                .total_cmp(&points[a].distance_to(centre))
        });
        idx.truncate(6);
        idx
    };
    let source_extremes = extremes(source, source_centre);
    let target_extremes = extremes(target, target_centre);

    for &si in &source_extremes {
        let s = source[si];
        let angle_s = (s.y - source_centre.y).atan2(s.x - source_centre.x);
        for &ti in &target_extremes {
            let t = target[ti];
            let angle_t = (t.y - target_centre.y).atan2(t.x - target_centre.x);
            let angle = angle_t - angle_s;
            for scale in [estimated_scale, 1.0] {
                if !scale.is_finite() || scale <= 0.0 {
                    continue;
                }
                let candidate = similarity_about(source_centre, target_centre, angle, scale);
                let moved: Vec<Point> = source.iter().map(|&p| candidate.apply(p)).collect();
                let count = nearest_matches(&moved, target, max_distance).len();
                if count > best_count {
                    best_count = count;
                    best = candidate;
                }
            }
        }
    }
    best
}

/// Iteratively conflate `source` points onto `target` control.
pub fn conflate_points(
    source: &[Point],
    target: &[Point],
    tol: &ConflationTolerances,
) -> Result<PointConflation> {
    if source.len() < tol.min_matches || target.len() < tol.min_matches {
        return Err(CoreError::NoSolution(format!(
            "need at least {} control pairs, got {} source and {} target",
            tol.min_matches,
            source.len(),
            target.len()
        )));
    }

    let mut transform = bootstrap_similarity(source, target, tol.max_distance);
    let mut previous: Vec<(usize, usize)> = Vec::new();
    let mut iterations = 0;

    for iter in 1..=20 {
        iterations = iter;
        let moved: Vec<Point> = source.iter().map(|&p| transform.apply(p)).collect();
        let matches = nearest_matches(&moved, target, tol.max_distance);
        if matches.len() < tol.min_matches {
            if iter == 1 {
                return Err(CoreError::NoSolution(format!(
                    "only {} points are within {} m of the target; need {}",
                    matches.len(),
                    tol.max_distance,
                    tol.min_matches
                )));
            }
            break;
        }
        let src: Vec<Point> = matches.iter().map(|m| source[m.source]).collect();
        let tgt: Vec<Point> = matches.iter().map(|m| target[m.target]).collect();
        let fitted = fit_similarity(&src, &tgt)?;
        let pairs: Vec<(usize, usize)> = matches.iter().map(|m| (m.source, m.target)).collect();
        let stable = pairs == previous;
        let change = (fitted.a - transform.a).abs()
            + (fitted.b - transform.b).abs()
            + (fitted.tx - transform.tx).abs()
            + (fitted.ty - transform.ty).abs();
        transform = fitted;
        previous = pairs;
        if stable && change < 1e-12 {
            break;
        }
    }

    let transformed_source: Vec<Point> = source.iter().map(|&p| transform.apply(p)).collect();
    let matches = nearest_matches(&transformed_source, target, tol.max_distance);
    if matches.len() < tol.min_matches {
        return Err(CoreError::NoSolution(
            "the matched points do not converge to a usable transformation".to_string(),
        ));
    }

    let matched_source: Vec<Point> = matches.iter().map(|m| source[m.source]).collect();
    let matched_target: Vec<Point> = matches.iter().map(|m| target[m.target]).collect();
    let residuals = transform.residuals(&matched_source, &matched_target);
    let rms = (residuals.iter().map(|d| d * d).sum::<f64>() / residuals.len() as f64).sqrt();
    let max_residual = residuals.iter().copied().fold(0.0, f64::max);

    let matched_sources: Vec<bool> = {
        let mut v = vec![false; source.len()];
        matches.iter().for_each(|m| v[m.source] = true);
        v
    };
    let matched_targets: Vec<bool> = {
        let mut v = vec![false; target.len()];
        matches.iter().for_each(|m| v[m.target] = true);
        v
    };

    Ok(PointConflation {
        transform,
        transformed_source,
        matches,
        unmatched_source: (0..source.len()).filter(|&i| !matched_sources[i]).collect(),
        unmatched_target: (0..target.len()).filter(|&i| !matched_targets[i]).collect(),
        rms,
        max_residual,
        iterations,
    })
}

/// Iteratively conflate linework onto an existing network.
///
/// Lines are matched by midpoint proximity, bearing difference and length difference. The
/// transform is fitted from the endpoints of the matched lines, respecting reversed direction.
pub fn conflate_lines(
    source: &[LineSegment],
    target: &[LineSegment],
    tol: &ConflationTolerances,
) -> Result<LineConflation> {
    if source.is_empty() || target.is_empty() {
        return Err(CoreError::InvalidInput(
            "both line lists must be non-empty".to_string(),
        ));
    }

    let mut transform = Similarity::identity();
    let mut iterations = 0;
    let mut previous: Vec<(usize, usize)> = Vec::new();
    let mut last_matches = Vec::new();

    for iter in 1..=20 {
        iterations = iter;
        let moved: Vec<LineSegment> = source
            .iter()
            .map(|l| LineSegment::new(transform.apply(l.from), transform.apply(l.to)))
            .collect();
        let matches = match_lines(&moved, source, target, tol);
        if matches.len() < tol.min_matches {
            if iter == 1 {
                return Err(CoreError::NoSolution(format!(
                    "only {} line(s) matched within tolerance; need {}",
                    matches.len(),
                    tol.min_matches
                )));
            }
            break;
        }

        let mut src_pts = Vec::with_capacity(matches.len() * 2);
        let mut tgt_pts = Vec::with_capacity(matches.len() * 2);
        for m in &matches {
            let s = source[m.source];
            let t = target[m.target];
            if m.reversed {
                src_pts.push(s.from);
                tgt_pts.push(t.to);
                src_pts.push(s.to);
                tgt_pts.push(t.from);
            } else {
                src_pts.push(s.from);
                tgt_pts.push(t.from);
                src_pts.push(s.to);
                tgt_pts.push(t.to);
            }
        }
        let fitted = fit_similarity(&src_pts, &tgt_pts)?;
        let pairs: Vec<(usize, usize)> = matches.iter().map(|m| (m.source, m.target)).collect();
        let stable = pairs == previous;
        let change = (fitted.a - transform.a).abs()
            + (fitted.b - transform.b).abs()
            + (fitted.tx - transform.tx).abs()
            + (fitted.ty - transform.ty).abs();
        transform = fitted;
        previous = pairs;
        last_matches = matches;
        if stable && change < 1e-12 {
            break;
        }
    }

    if last_matches.is_empty() {
        return Err(CoreError::NoSolution(
            "no lines matched within tolerance".to_string(),
        ));
    }

    let mut residuals = Vec::new();
    for m in &last_matches {
        let s = source[m.source];
        let t = target[m.target];
        let (a, b) = if m.reversed {
            (t.to, t.from)
        } else {
            (t.from, t.to)
        };
        residuals.push(transform.apply(s.from).distance_to(a));
        residuals.push(transform.apply(s.to).distance_to(b));
    }
    let rms = (residuals.iter().map(|d| d * d).sum::<f64>() / residuals.len() as f64).sqrt();
    let max_residual = residuals.iter().copied().fold(0.0, f64::max);

    let mut used_source = vec![false; source.len()];
    let mut used_target = vec![false; target.len()];
    last_matches.iter().for_each(|m| {
        used_source[m.source] = true;
        used_target[m.target] = true;
    });

    Ok(LineConflation {
        transform,
        matches: last_matches,
        unmatched_source: (0..source.len()).filter(|&i| !used_source[i]).collect(),
        unmatched_target: (0..target.len()).filter(|&i| !used_target[i]).collect(),
        rms,
        max_residual,
        iterations,
    })
}

/// Match transformed source lines to target lines under the tolerances.
fn match_lines(
    moved: &[LineSegment],
    source: &[LineSegment],
    target: &[LineSegment],
    tol: &ConflationTolerances,
) -> Vec<LineMatch> {
    let mut candidates = Vec::new();
    for (si, m) in moved.iter().enumerate() {
        let sm = m.midpoint();
        for (ti, t) in target.iter().enumerate() {
            let midpoint_distance = sm.distance_to(t.midpoint());
            if midpoint_distance > tol.max_distance {
                continue;
            }
            let sb = source[si].bearing_deg();
            let tb = t.bearing_deg();
            let diff = bearing_diff_sec(sb, tb);
            let reversed = diff > 90.0 * 3600.0;
            let effective = if reversed { 180.0 * 3600.0 - diff } else { diff };
            if effective > tol.max_bearing_sec {
                continue;
            }
            let length_diff = (source[si].length() - t.length()).abs();
            if length_diff > tol.max_length_diff {
                continue;
            }
            candidates.push(LineMatch {
                source: si,
                target: ti,
                midpoint_distance,
                bearing_diff_sec: effective,
                length_diff,
                reversed,
            });
        }
    }
    candidates.sort_by(|a, b| {
        a.midpoint_distance
            .total_cmp(&b.midpoint_distance)
            .then(a.bearing_diff_sec.total_cmp(&b.bearing_diff_sec))
    });
    let mut used_source = vec![false; source.len()];
    let mut used_target = vec![false; target.len()];
    let mut matches = Vec::new();
    for c in candidates {
        if !used_source[c.source] && !used_target[c.target] {
            used_source[c.source] = true;
            used_target[c.target] = true;
            matches.push(c);
        }
    }
    matches.sort_by_key(|m| m.source);
    matches
}

/// Absolute difference between two bearings, in arcseconds, wrapped to `[0, 180]` degrees.
fn bearing_diff_sec(a_deg: f64, b_deg: f64) -> f64 {
    let diff = (a_deg - b_deg).rem_euclid(360.0);
    let wrapped = if diff > 180.0 { 360.0 - diff } else { diff };
    wrapped * 3600.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::units::Angle;

    fn grid(origin: Point) -> Vec<Point> {
        vec![
            origin,
            Point::new(origin.x + 100.0, origin.y),
            Point::new(origin.x, origin.y + 100.0),
            Point::new(origin.x + 100.0, origin.y + 100.0),
        ]
    }

    #[test]
    fn points_conflate_under_rotation_and_shift() {
        let target = grid(Point::new(500_000.0, 1_000_000.0));
        let truth = Similarity::rotation_scale(
            Angle::from_degrees(-3.0),
            0.9998,
            Point::new(500_000.0, 1_000_000.0),
        );
        let source: Vec<Point> = target.iter().map(|&p| truth.inverse().apply(p)).collect();

        let report = conflate_points(&source, &target, &ConflationTolerances::default()).unwrap();
        assert_eq!(report.matches.len(), 4);
        assert!(report.rms < 1e-6, "rms {}", report.rms);
        // Applying the fitted transform reproduces the target control.
        for (p, t) in report.transformed_source.iter().zip(&target) {
            assert!(p.distance_to(*t) < 1e-6);
        }
    }

    #[test]
    fn unmatched_points_are_reported() {
        let target = grid(Point::new(0.0, 0.0));
        let mut source = target.clone();
        source.push(Point::new(1000.0, 1000.0));
        let report = conflate_points(&source, &target, &ConflationTolerances::default()).unwrap();
        assert_eq!(report.matches.len(), 4);
        assert_eq!(report.unmatched_source, vec![4]);
    }

    #[test]
    fn lines_conflate_with_reversal() {
        let target = vec![
            LineSegment::new(Point::new(0.0, 0.0), Point::new(100.0, 0.0)),
            LineSegment::new(Point::new(100.0, 0.0), Point::new(100.0, 100.0)),
        ];
        let shift = Similarity::translation(3.0, -2.0);
        let source: Vec<LineSegment> = target
            .iter()
            .map(|l| LineSegment::new(shift.inverse().apply(l.to), shift.inverse().apply(l.from)))
            .collect();
        let report = conflate_lines(
            &source,
            &target,
            &ConflationTolerances {
                max_distance: 10.0,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(report.matches.len(), 2);
        assert!(report.rms < 1e-6, "rms {}", report.rms);
    }

    #[test]
    fn too_few_points_is_an_error() {
        assert!(conflate_points(
            &[Point::new(0.0, 0.0)],
            &[Point::new(0.0, 0.0)],
            &ConflationTolerances::default()
        )
        .is_err());
    }
}
