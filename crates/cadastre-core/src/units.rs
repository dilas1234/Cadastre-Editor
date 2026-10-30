//! Angles and whole-circle bearings.
//!
//! Internally angles are held in radians (best for trigonometry). Survey input and
//! storage are degrees-minutes-seconds and decimal seconds of arc (SRS §6), so both
//! conversions are first-class and lossless to `f64`.

use serde::{Deserialize, Serialize};
use std::f64::consts::{PI, TAU};

/// A plane angle. Not normalised — may be negative or exceed one turn, which matters
/// for residuals and corrections. Use [`Bearing`] for a normalised whole-circle direction.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct Angle {
    radians: f64,
}

impl Angle {
    /// An angle from radians.
    pub const fn from_radians(radians: f64) -> Self {
        Self { radians }
    }

    /// An angle from decimal degrees.
    pub fn from_degrees(degrees: f64) -> Self {
        Self {
            radians: degrees.to_radians(),
        }
    }

    /// An angle from degrees, minutes and seconds. The sign of `degrees` sets the sign
    /// of the whole angle; `minutes` and `seconds` are magnitudes.
    pub fn from_dms(degrees: i32, minutes: u32, seconds: f64) -> Self {
        let magnitude = degrees.unsigned_abs() as f64 + minutes as f64 / 60.0 + seconds / 3600.0;
        let signed = if degrees < 0 { -magnitude } else { magnitude };
        Self::from_degrees(signed)
    }

    /// An angle from decimal seconds of arc (the storage unit for bearings, SRS §6).
    pub fn from_seconds(seconds: f64) -> Self {
        Self::from_degrees(seconds / 3600.0)
    }

    /// The angle in radians.
    pub fn radians(self) -> f64 {
        self.radians
    }

    /// The angle in decimal degrees.
    pub fn degrees(self) -> f64 {
        self.radians.to_degrees()
    }

    /// The angle in decimal seconds of arc.
    pub fn total_seconds(self) -> f64 {
        self.degrees() * 3600.0
    }

    /// Decompose into `(degrees, minutes, seconds)`. `degrees` carries the sign;
    /// `minutes` and `seconds` are non-negative magnitudes.
    pub fn to_dms(self) -> (i32, u32, f64) {
        let total = self.degrees();
        let sign = if total.is_sign_negative() { -1 } else { 1 };
        let mut mag = total.abs();
        let d = mag.trunc();
        mag = (mag - d) * 60.0;
        let m = mag.trunc();
        let s = (mag - m) * 60.0;
        (sign * d as i32, m as u32, s)
    }

    /// This angle wrapped into `[0, 2π)`.
    pub fn normalized(self) -> Self {
        let mut r = self.radians % TAU;
        if r < 0.0 {
            r += TAU;
        }
        Self { radians: r }
    }

    /// This angle wrapped into `(-π, π]` — the correct range for an angular residual.
    pub fn wrapped_signed(self) -> Self {
        let mut r = (self.radians + PI) % TAU;
        if r <= 0.0 {
            r += TAU;
        }
        Self { radians: r - PI }
    }

    /// `sin` of the angle.
    pub fn sin(self) -> f64 {
        self.radians.sin()
    }

    /// `cos` of the angle.
    pub fn cos(self) -> f64 {
        self.radians.cos()
    }

    /// `tan` of the angle.
    pub fn tan(self) -> f64 {
        self.radians.tan()
    }
}

/// A whole-circle bearing: clockwise from grid north, normalised to `[0, 2π)`.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct Bearing(Angle);

impl Bearing {
    /// A bearing from an angle, normalised into `[0, 2π)`.
    pub fn from_angle(angle: Angle) -> Self {
        Self(angle.normalized())
    }

    /// A bearing from decimal degrees.
    pub fn from_degrees(degrees: f64) -> Self {
        Self::from_angle(Angle::from_degrees(degrees))
    }

    /// A bearing from degrees, minutes and seconds (the way it appears on a survey plan).
    pub fn from_dms(degrees: i32, minutes: u32, seconds: f64) -> Self {
        Self::from_angle(Angle::from_dms(degrees, minutes, seconds))
    }

    /// The underlying angle in `[0, 2π)`.
    pub fn angle(self) -> Angle {
        self.0
    }

    /// The bearing in radians, in `[0, 2π)`.
    pub fn radians(self) -> f64 {
        self.0.radians()
    }

    /// The bearing in decimal degrees, in `[0, 360)`.
    pub fn degrees(self) -> f64 {
        self.0.degrees()
    }

    /// The back bearing (this bearing plus 180°).
    pub fn back(self) -> Self {
        Self::from_angle(Angle::from_radians(self.0.radians() + PI))
    }
}
