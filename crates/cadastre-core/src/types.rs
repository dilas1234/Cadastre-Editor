//! Core survey data types: points, beacons, courses, curves and traverses.
//!
//! Coordinates are easting/northing in metres in the projected CRS (SRS §6). Nothing here
//! is ever generalised or rounded on construction.

use serde::{Deserialize, Serialize};

use crate::units::Bearing;

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

/// A circular-arc course (SRS FR-SUR-05). Represented by its radius, arc length and the
/// bearing of its chord. The chord is used for traverse/closure computation; full arc
/// geometry (sweep direction, bulge) is a later concern.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Curve {
    /// Arc radius (metres). Positive.
    pub radius: f64,
    /// Arc length along the curve (metres).
    pub arc_length: f64,
    /// Whole-circle bearing of the chord joining the arc's endpoints.
    pub chord_bearing: Bearing,
}

impl Curve {
    /// A curve from radius, arc length and chord bearing.
    pub const fn new(radius: f64, arc_length: f64, chord_bearing: Bearing) -> Self {
        Self {
            radius,
            arc_length,
            chord_bearing,
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
