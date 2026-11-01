//! 2D similarity (4-parameter Helmert) and affine transformations, fitted by least squares
//! (SRS FR-ADJ-02, FR-SUR alignment/conflation).
//!
//! The similarity model is
//!
//! ```text
//! X = a·x − b·y + tx
//! Y = b·x + a·y + ty
//! ```
//!
//! so `scale = hypot(a, b)` and the rotation is `atan2(b, a)` (mathematical, anticlockwise
//! positive). It preserves shape exactly, which is what joining a new survey to existing
//! control requires.

use nalgebra::{DMatrix, DVector};
use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};
use crate::types::Point;
use crate::units::Angle;

/// A 2D similarity transformation (translation, rotation and uniform scale).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Similarity {
    /// Cosine-scale coefficient `a = s·cos θ`.
    pub a: f64,
    /// Sine-scale coefficient `b = s·sin θ`.
    pub b: f64,
    /// Easting translation.
    pub tx: f64,
    /// Northing translation.
    pub ty: f64,
}

impl Similarity {
    /// The identity transform.
    pub const fn identity() -> Self {
        Self {
            a: 1.0,
            b: 0.0,
            tx: 0.0,
            ty: 0.0,
        }
    }

    /// A pure translation.
    pub const fn translation(dx: f64, dy: f64) -> Self {
        Self {
            a: 1.0,
            b: 0.0,
            tx: dx,
            ty: dy,
        }
    }

    /// A rotation (anticlockwise positive) and uniform scale about `origin`.
    pub fn rotation_scale(angle: Angle, scale: f64, origin: Point) -> Self {
        let (sin, cos) = angle.sin_cos_pair();
        let a = scale * cos;
        let b = scale * sin;
        Self {
            a,
            b,
            tx: origin.x - a * origin.x + b * origin.y,
            ty: origin.y - b * origin.x - a * origin.y,
        }
    }

    /// Apply the transform to a point.
    pub fn apply(&self, p: Point) -> Point {
        Point::new(
            self.a * p.x - self.b * p.y + self.tx,
            self.b * p.x + self.a * p.y + self.ty,
        )
    }

    /// The uniform scale factor.
    pub fn scale(&self) -> f64 {
        self.a.hypot(self.b)
    }

    /// The rotation angle (anticlockwise positive).
    pub fn rotation(&self) -> Angle {
        Angle::from_radians(self.b.atan2(self.a))
    }

    /// The inverse transform.
    pub fn inverse(&self) -> Self {
        let denom = self.a * self.a + self.b * self.b;
        if denom < 1e-30 {
            return Self::identity();
        }
        let a = self.a / denom;
        let b = -self.b / denom;
        Self {
            a,
            b,
            tx: -(a * self.tx - b * self.ty),
            ty: -(b * self.tx + a * self.ty),
        }
    }

    /// Compose transforms: apply `other` first, then `self`.
    pub fn compose(&self, other: &Self) -> Self {
        Self {
            a: self.a * other.a - self.b * other.b,
            b: self.b * other.a + self.a * other.b,
            tx: self.a * other.tx - self.b * other.ty + self.tx,
            ty: self.b * other.tx + self.a * other.ty + self.ty,
        }
    }

    /// Per-point residuals (metres) after applying the transform to each source point.
    pub fn residuals(&self, source: &[Point], target: &[Point]) -> Vec<f64> {
        source
            .iter()
            .zip(target)
            .map(|(&s, &t)| self.apply(s).distance_to(t))
            .collect()
    }

    /// Root-mean-square of the residuals (metres).
    pub fn rms(&self, source: &[Point], target: &[Point]) -> f64 {
        let r = self.residuals(source, target);
        if r.is_empty() {
            return 0.0;
        }
        (r.iter().map(|d| d * d).sum::<f64>() / r.len() as f64).sqrt()
    }
}

/// Fit a 4-parameter similarity from control pairs by least squares.
///
/// At least two non-coincident source points are required.
pub fn fit_similarity(source: &[Point], target: &[Point]) -> Result<Similarity> {
    if source.len() != target.len() {
        return Err(CoreError::InvalidInput(
            "source and target control lists must have the same length".to_string(),
        ));
    }
    if source.len() < 2 {
        return Err(CoreError::InvalidInput(
            "at least two control point pairs are required".to_string(),
        ));
    }
    let n = source.len() as f64;
    let mx = source.iter().map(|p| p.x).sum::<f64>() / n;
    let my = source.iter().map(|p| p.y).sum::<f64>() / n;
    let mx_t = target.iter().map(|p| p.x).sum::<f64>() / n;
    let my_t = target.iter().map(|p| p.y).sum::<f64>() / n;

    let mut denominator = 0.0;
    let mut aa = 0.0;
    let mut bb = 0.0;
    for (s, t) in source.iter().zip(target) {
        let x = s.x - mx;
        let y = s.y - my;
        let x_t = t.x - mx_t;
        let y_t = t.y - my_t;
        denominator += x * x + y * y;
        aa += x * x_t + y * y_t;
        bb += x * y_t - y * x_t;
    }
    if denominator < 1e-18 {
        return Err(CoreError::InvalidInput(
            "the source control points are coincident".to_string(),
        ));
    }
    let a = aa / denominator;
    let b = bb / denominator;
    Ok(Similarity {
        a,
        b,
        tx: mx_t - a * mx + b * my,
        ty: my_t - b * mx - a * my,
    })
}

/// A full 6-parameter affine transform.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Affine {
    /// X from easting scale.
    pub a: f64,
    /// X from northing shear.
    pub b: f64,
    /// X translation.
    pub c: f64,
    /// Y from easting shear.
    pub d: f64,
    /// Y from northing scale.
    pub e: f64,
    /// Y translation.
    pub f: f64,
}

impl Affine {
    /// Apply the affine transform.
    pub fn apply(&self, p: Point) -> Point {
        Point::new(
            self.a * p.x + self.b * p.y + self.c,
            self.d * p.x + self.e * p.y + self.f,
        )
    }

    /// Per-point residuals (metres).
    pub fn residuals(&self, source: &[Point], target: &[Point]) -> Vec<f64> {
        source
            .iter()
            .zip(target)
            .map(|(&s, &t)| self.apply(s).distance_to(t))
            .collect()
    }
}

/// Fit a full affine transform from at least three non-collinear control pairs.
pub fn fit_affine(source: &[Point], target: &[Point]) -> Result<Affine> {
    if source.len() != target.len() {
        return Err(CoreError::InvalidInput(
            "source and target control lists must have the same length".to_string(),
        ));
    }
    if source.len() < 3 {
        return Err(CoreError::InvalidInput(
            "at least three control point pairs are required".to_string(),
        ));
    }
    let n = source.len();
    let mut design = DMatrix::<f64>::zeros(2 * n, 6);
    let mut rhs = DVector::<f64>::zeros(2 * n);
    for (i, (s, t)) in source.iter().zip(target).enumerate() {
        design[(2 * i, 0)] = s.x;
        design[(2 * i, 1)] = s.y;
        design[(2 * i, 2)] = 1.0;
        rhs[2 * i] = t.x;
        design[(2 * i + 1, 3)] = s.x;
        design[(2 * i + 1, 4)] = s.y;
        design[(2 * i + 1, 5)] = 1.0;
        rhs[2 * i + 1] = t.y;
    }
    let normal = design.transpose() * &design;
    let atb = design.transpose() * rhs;
    let solution = normal.lu().solve(&atb).ok_or(CoreError::SingularNormalMatrix)?;
    Ok(Affine {
        a: solution[0],
        b: solution[1],
        c: solution[2],
        d: solution[3],
        e: solution[4],
        f: solution[5],
    })
}

/// Small helper on [`Angle`] for sine and cosine in one call (keeps the transform code tidy).
trait SinCos {
    fn sin_cos_pair(self) -> (f64, f64);
}

impl SinCos for Angle {
    fn sin_cos_pair(self) -> (f64, f64) {
        (self.radians().sin(), self.radians().cos())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fitted_similarity_recovers_a_known_transform() {
        let source = vec![
            Point::new(0.0, 0.0),
            Point::new(100.0, 0.0),
            Point::new(0.0, 100.0),
            Point::new(100.0, 100.0),
        ];
        let truth = Similarity::rotation_scale(Angle::from_degrees(30.0), 1.5, Point::new(500.0, 1000.0));
        let target: Vec<Point> = source.iter().map(|&p| truth.apply(p)).collect();
        let fitted = fit_similarity(&source, &target).unwrap();
        assert!((fitted.scale() - 1.5).abs() < 1e-9);
        assert!((fitted.rotation().degrees() - 30.0).abs() < 1e-9);
        assert!(fitted.rms(&source, &target) < 1e-9);
        // Inverse round-trips.
        let back = fitted.inverse().apply(truth.apply(Point::new(12.0, 34.0)));
        assert!(back.distance_to(Point::new(12.0, 34.0)) < 1e-9);
    }

    #[test]
    fn translation_and_rotation_about_origin() {
        let t = Similarity::translation(10.0, -5.0);
        assert_eq!(t.apply(Point::new(1.0, 2.0)), Point::new(11.0, -3.0));

        let r = Similarity::rotation_scale(Angle::from_degrees(90.0), 1.0, Point::new(0.0, 0.0));
        let p = r.apply(Point::new(1.0, 0.0));
        assert!(p.distance_to(Point::new(0.0, 1.0)) < 1e-12);
    }

    #[test]
    fn affine_recovers_an_affine_map() {
        let source = vec![
            Point::new(0.0, 0.0),
            Point::new(10.0, 0.0),
            Point::new(0.0, 10.0),
            Point::new(10.0, 10.0),
        ];
        let truth = Affine {
            a: 2.0,
            b: 0.5,
            c: 100.0,
            d: -0.25,
            e: 3.0,
            f: 200.0,
        };
        let target: Vec<Point> = source.iter().map(|&p| truth.apply(p)).collect();
        let fitted = fit_affine(&source, &target).unwrap();
        for (s, t) in source.iter().zip(&target) {
            assert!(fitted.apply(*s).distance_to(*t) < 1e-9);
        }
    }

    #[test]
    fn fit_requires_enough_control() {
        assert!(fit_similarity(&[Point::new(0.0, 0.0)], &[Point::new(0.0, 0.0)]).is_err());
        assert!(fit_affine(
            &[Point::new(0.0, 0.0), Point::new(1.0, 1.0)],
            &[Point::new(0.0, 0.0), Point::new(1.0, 1.0)]
        )
        .is_err());
    }
}
