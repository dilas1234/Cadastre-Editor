//! Core survey data types: points, beacons, courses, curves and traverses.
//!
//! Coordinates are easting/northing in metres in the projected CRS (SRS §6). Nothing here
//! is ever generalised or rounded on construction.

use serde::{Deserialize, Serialize};

use crate::units::{Angle, Bearing};

/// The direction a curve turns as it is traversed from start to end.
///
/// Whole-circle bearings increase clockwise, so a **right** turn increases the tangent bearing
/// and a **left** turn decreases it. The default is right, which keeps older serialised surveys
/// (written before curve handedness was recorded) loadable.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Turn {
    /// Tangent bearing increases (clockwise curvature).
    #[default]
    Right,
    /// Tangent bearing decreases (anticlockwise curvature).
    Left,
}

impl Turn {
    /// `+1` for a right turn, `-1` for a left turn.
    pub const fn sign(self) -> f64 {
        match self {
            Turn::Right => 1.0,
            Turn::Left => -1.0,
        }
    }

    /// The opposite handedness.
    pub const fn opposite(self) -> Self {
        match self {
            Turn::Right => Turn::Left,
            Turn::Left => Turn::Right,
        }
    }
}

/// A planar coordinate in metres: `x` = easting, `y` = northing.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Point {
    /// Easting (metres).
    pub x: f64,
    /// Northing (metres).
    pub y: f64,
}

impl Point {
    /// A point from easting and northing.
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    /// Straight-line (plane) distance to another point.
    pub fn distance_to(self, other: Point) -> f64 {
        (other.x - self.x).hypot(other.y - self.y)
    }
}

/// A physical survey mark at a parcel corner (SRS §1.3, §6).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Beacon {
    /// Beacon number (unique among current beacons).
    pub number: String,
    /// Coordinate of the mark.
    pub point: Point,
    /// A-priori coordinate accuracy (metres, 1σ). Used to weight adjustment.
    pub accuracy: f64,
    /// Whether this is a control point (trig / reference beacon) held fixed in adjustment.
    pub is_control: bool,
}

/// A straight boundary course: a whole-circle bearing and a distance in metres.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Course {
    /// Whole-circle bearing of the course.
    pub bearing: Bearing,
    /// Distance along the course (metres).
    pub distance: f64,
}

impl Course {
    /// A course from a bearing and distance.
    pub const fn new(bearing: Bearing, distance: f64) -> Self {
        Self { bearing, distance }
    }
}

/// A circular-arc course (SRS FR-SUR-05). Represented by its radius, arc length, the
/// bearing of its chord and the direction of curvature. The chord is used for traverse/closing
/// geometry; the arc itself is used for area and perimeter so a curved boundary is not
/// under-measured.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Curve {
    /// Arc radius (metres). Positive.
    pub radius: f64,
    /// Arc length along the curve (metres).
    pub arc_length: f64,
    /// Whole-circle bearing of the chord joining the arc's endpoints.
    pub chord_bearing: Bearing,
    /// Direction the curve turns when traversed from start to end.
    #[serde(default)]
    pub turn: Turn,
}

impl Curve {
    /// A right-turning curve from radius, arc length and chord bearing.
    pub const fn new(radius: f64, arc_length: f64, chord_bearing: Bearing) -> Self {
        Self::with_turn(radius, arc_length, chord_bearing, Turn::Right)
    }

    /// A curve with an explicit direction of curvature.
    pub const fn with_turn(
        radius: f64,
        arc_length: f64,
        chord_bearing: Bearing,
        turn: Turn,
    ) -> Self {
        Self {
            radius,
            arc_length,
            chord_bearing,
            turn,
        }
    }

    /// The chord length between the arc endpoints: `2R·sin(L / 2R)`.
    pub fn chord_length(self) -> f64 {
        if self.radius <= 0.0 {
            return self.arc_length;
        }
        2.0 * self.radius * (self.arc_length / (2.0 * self.radius)).sin()
    }

    /// The equivalent straight course along the curve's chord.
    pub fn as_chord_course(self) -> Course {
        Course::new(self.chord_bearing, self.chord_length())
    }

    /// The central angle swept by the arc (magnitude, radians). Zero for an invalid radius.
    pub fn central_angle(self) -> f64 {
        if self.radius <= 0.0 {
            0.0
        } else {
            self.arc_length / self.radius
        }
    }

    /// Signed central angle: positive anticlockwise (left), negative clockwise (right).
    pub fn signed_central_angle(self) -> f64 {
        -self.turn.sign() * self.central_angle()
    }

    /// The tangent bearing at the start of the arc, derived from the chord bearing and the
    /// deflection half-angle: `chord − sign·Δ/2`.
    pub fn initial_bearing(self) -> Bearing {
        Bearing::from_angle(Angle::from_radians(
            self.chord_bearing.radians() - self.turn.sign() * self.central_angle() / 2.0,
        ))
    }

    /// The tangent bearing at the end of the arc.
    pub fn final_bearing(self) -> Bearing {
        Bearing::from_angle(Angle::from_radians(
            self.initial_bearing().radians() + self.signed_central_angle(),
        ))
    }

    /// The arc's centre, given the arc's start point.
    pub fn centre(self, start: Point) -> Point {
        let radius_bearing = Bearing::from_angle(Angle::from_radians(
            self.initial_bearing().radians() + self.turn.sign() * std::f64::consts::FRAC_PI_2,
        ));
        crate::cogo::forward(start, radius_bearing, self.radius)
    }

    /// The signed area between the chord and the arc, as a contribution to the shoelace
    /// integral of a closed ring traversed through this arc.
    ///
    /// This is exact for any sweep (including a semicircle), unlike treating the curve as a
    /// chord. It is zero when the radius is non-positive.
    pub fn arc_area_correction(self, start: Point) -> f64 {
        let radius = self.radius;
        if radius <= 0.0 {
            return 0.0;
        }
        let end = crate::cogo::forward(start, self.chord_bearing, self.chord_length());
        let centre = self.centre(start);
        let cross_centre = centre.x * (end.y - start.y) - centre.y * (end.x - start.x);
        let cross_chord = start.x * end.y - end.x * start.y;
        0.5 * (cross_centre + radius * radius * self.signed_central_angle() - cross_chord)
    }
}

/// One leg of a traverse: either a straight course or a circular arc.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Segment {
    /// A straight course.
    Line(Course),
    /// A circular arc, reduced to its chord for coordinate geometry.
    Curve(Curve),
}

impl Segment {
    /// The straight course representing this segment (the chord, for a curve).
    pub fn as_course(self) -> Course {
        match self {
            Segment::Line(c) => c,
            Segment::Curve(c) => c.as_chord_course(),
        }
    }
}

/// A chain of courses from a known starting beacon (SRS §1.3, FR-SUR-02).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Traverse {
    /// Coordinate of the starting beacon.
    pub start: Point,
    /// Ordered legs of the traverse.
    pub segments: Vec<Segment>,
}

impl Traverse {
    /// A traverse from a start point and segments.
    pub fn new(start: Point, segments: Vec<Segment>) -> Self {
        Self { start, segments }
    }

    /// A traverse built from straight courses only.
    pub fn from_courses(start: Point, courses: impl IntoIterator<Item = Course>) -> Self {
        Self {
            start,
            segments: courses.into_iter().map(Segment::Line).collect(),
        }
    }

    /// Number of legs.
    pub fn len(&self) -> usize {
        self.segments.len()
    }

    /// Whether the traverse has no legs.
    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }
}
