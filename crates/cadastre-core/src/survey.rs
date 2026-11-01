//! Live survey-entry session (SRS FR-SUR-*, FR-COG-02, NFR-03).
//!
//! The editor captures a traverse one leg at a time from a known start beacon. After every
//! edit it needs an immediate recomputation of closure, area and precision so the surveyor
//! sees whether the figure is good (the "live check", NFR-03: < 100 ms). This module is that
//! engine: an editable list of legs plus a cheap `summary` that bundles the checks.
//!
//! It is pure maths (like the rest of `cadastre-core`) and compiles to wasm32, so the desktop
//! editor runs exactly this code for the live check and the server runs it to re-validate.

use serde::{Deserialize, Serialize};

use crate::area::{self, AreaCheck};
use crate::closure::{self, Closure};
use crate::cogo;
use crate::error::{CoreError, Result};
use crate::tolerances::Tolerances;
use crate::types::{Course, Curve, Point, Segment, Traverse};

/// An editable survey-entry session: a start beacon and an ordered list of legs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SurveySession {
    /// Coordinate of the known starting beacon.
    pub start: Point,
    /// Number of the starting beacon, if recorded.
    pub start_beacon: Option<String>,
    /// Ordered legs captured so far.
    pub segments: Vec<Segment>,
    /// Area stated on the survey plan (m²), used for the area check when present.
    pub stated_area: Option<f64>,
}

impl SurveySession {
    /// Start a new, empty session from a known beacon coordinate.
    pub fn new(start: Point) -> Self {
        Self {
            start,
            start_beacon: None,
            segments: Vec::new(),
            stated_area: None,
        }
    }

    /// Start a session, naming the start beacon.
    pub fn with_beacon(start: Point, beacon: impl Into<String>) -> Self {
        Self {
            start_beacon: Some(beacon.into()),
            ..Self::new(start)
        }
    }

    /// Record the plan's stated area (or clear it with `None`).
    pub fn set_stated_area(&mut self, area: Option<f64>) {
        self.stated_area = area;
    }

    /// Append a straight course.
    pub fn push_course(&mut self, course: Course) {
        self.segments.push(Segment::Line(course));
    }

    /// Append a circular-arc leg.
    pub fn push_curve(&mut self, curve: Curve) {
        self.segments.push(Segment::Curve(curve));
    }

    /// Append a leg (straight or curved).
    pub fn push_segment(&mut self, segment: Segment) {
        self.segments.push(segment);
    }

    /// Remove and return the last leg, if any.
    pub fn pop(&mut self) -> Option<Segment> {
        self.segments.pop()
    }

    /// Replace the leg at `index` (grid edit of one row).
    pub fn replace(&mut self, index: usize, segment: Segment) -> Result<()> {
        *self.segment_mut(index)? = segment;
        Ok(())
    }

    /// Insert a leg before `index`. `index == len` appends.
    pub fn insert(&mut self, index: usize, segment: Segment) -> Result<()> {
        if index > self.segments.len() {
            return Err(self.index_error(index));
        }
        self.segments.insert(index, segment);
        Ok(())
    }

    /// Remove and return the leg at `index`.
    pub fn remove(&mut self, index: usize) -> Result<Segment> {
        if index >= self.segments.len() {
            return Err(self.index_error(index));
        }
        Ok(self.segments.remove(index))
    }

    /// Number of legs.
    pub fn len(&self) -> usize {
        self.segments.len()
    }

    /// Whether no legs have been entered.
    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    /// The traverse formed by the current legs.
    pub fn traverse(&self) -> Traverse {
        Traverse::new(self.start, self.segments.clone())
    }

    /// Every vertex of the current figure, starting at the start beacon
    /// (`len() + 1` points; curves reduced to chords).
    pub fn vertices(&self) -> Vec<Point> {
        cogo::traverse_points(&self.traverse())
    }

    /// Recompute the live summary against the given tolerances. O(legs); this is the
    /// call the editor makes after every keystroke (NFR-03).
    pub fn summary(&self, tolerances: &Tolerances) -> SurveySummary {
        let traverse = self.traverse();
        let vertices = cogo::traverse_points(&traverse);
        let perimeter = area::traverse_perimeter(&traverse);

        // `closure` needs >= 1 leg; `traverse_area` needs >= 3 distinct vertices and adds the
        // circular-segment area of curved legs.
        let closure = closure::closure(&traverse).ok();
        let area = area::traverse_area(&traverse).ok();
        let area_check = match (area, self.stated_area) {
            (Some(computed), Some(stated)) => Some(area::area_check(computed, stated)),
            _ => None,
        };

        let checks = SurveyChecks {
            closes_on_start: closure
                .is_some_and(|c| c.linear_misclosure <= tolerances.join_distance),
            meets_precision: closure.is_some_and(|c| c.meets_precision(tolerances.closure_precision)),
            area_within_tolerance: area_check.map(|c| c.within(tolerances.area_fraction)),
        };

        SurveySummary {
            leg_count: self.segments.len(),
            vertex_count: vertices.len(),
            perimeter,
            closure,
            area,
            area_check,
            checks,
        }
    }

    fn segment_mut(&mut self, index: usize) -> Result<&mut Segment> {
        let len = self.segments.len();
        self.segments
            .get_mut(index)
            .ok_or(CoreError::SegmentIndexOutOfRange { index, len })
    }

    fn index_error(&self, index: usize) -> CoreError {
        CoreError::SegmentIndexOutOfRange {
            index,
            len: self.segments.len(),
        }
    }
}

/// The live result of a survey session: geometry plus pass/fail checks (FR-COG-02/03).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SurveySummary {
    /// Number of legs entered.
    pub leg_count: usize,
    /// Number of vertices (legs + 1).
    pub vertex_count: usize,
    /// Perimeter / total traverse length (m).
    pub perimeter: f64,
    /// Closure back onto the start, or `None` with no legs.
    pub closure: Option<Closure>,
    /// Computed area (m²), or `None` with fewer than 3 distinct vertices.
    pub area: Option<f64>,
    /// Area comparison to the stated area, or `None` if either is unavailable.
    pub area_check: Option<AreaCheck>,
    /// The tolerance checks the editor turns into red/green indicators.
    pub checks: SurveyChecks,
}

/// Tolerance checks for the live figure. Each is derived from the active [`Tolerances`].
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SurveyChecks {
    /// The loop returns onto its start within the join distance.
    pub closes_on_start: bool,
    /// The precision ratio meets the minimum (e.g. 1:10,000).
    pub meets_precision: bool,
    /// Computed area is within tolerance of the stated area (`None` if not checkable).
    pub area_within_tolerance: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::units::Bearing;

    /// A 100 m × 100 m square entered clockwise from the SW corner. Closes exactly.
    fn unit_square() -> SurveySession {
        let mut s = SurveySession::with_beacon(Point::new(1000.0, 1000.0), "B1");
        s.push_course(Course::new(Bearing::from_degrees(0.0), 100.0)); // N
        s.push_course(Course::new(Bearing::from_degrees(90.0), 100.0)); // E
        s.push_course(Course::new(Bearing::from_degrees(180.0), 100.0)); // S
        s.push_course(Course::new(Bearing::from_degrees(270.0), 100.0)); // W
        s
    }

    #[test]
    fn empty_session_has_no_closure_or_area() {
        let s = SurveySession::new(Point::new(0.0, 0.0));
        let sum = s.summary(&Tolerances::default());
        assert_eq!(sum.leg_count, 0);
        assert_eq!(sum.vertex_count, 1);
        assert!(sum.closure.is_none());
        assert!(sum.area.is_none());
        assert!(!sum.checks.closes_on_start);
        assert!(!sum.checks.meets_precision);
    }

    #[test]
    fn closed_square_closes_and_areas_to_one_hectare() {
        let s = unit_square();
        let sum = s.summary(&Tolerances::default());
        assert_eq!(sum.leg_count, 4);
        assert_eq!(sum.vertex_count, 5);
        let closure = sum.closure.expect("closed loop has a closure");
        assert!(closure.linear_misclosure < 1e-6, "mm closure");
        assert!(closure.precision_ratio.is_infinite());
        // 100 m square = 10,000 m² (one hectare), exact to 0.01 m² (NFR-01).
        assert!((sum.area.unwrap() - 10_000.0).abs() < 0.01);
        assert!(sum.checks.closes_on_start);
        assert!(sum.checks.meets_precision);
    }

    #[test]
    fn area_check_flags_a_stated_area_mismatch() {
        let mut s = unit_square();
        s.set_stated_area(Some(10_000.0));
        assert_eq!(s.summary(&Tolerances::default()).checks.area_within_tolerance, Some(true));

        // Claim 10,500 m²: a 5% error against a 1% tolerance should fail.
        s.set_stated_area(Some(10_500.0));
        let sum = s.summary(&Tolerances::default());
        assert_eq!(sum.checks.area_within_tolerance, Some(false));
        assert!(sum.area_check.unwrap().fraction > 0.01);
    }

    #[test]
    fn an_open_traverse_does_not_close() {
        let mut s = SurveySession::new(Point::new(0.0, 0.0));
        s.push_course(Course::new(Bearing::from_degrees(0.0), 100.0));
        s.push_course(Course::new(Bearing::from_degrees(90.0), 100.0));
        let sum = s.summary(&Tolerances::default());
        // Two legs: 3 vertices, so an area exists, but it is far from closing.
        assert!(sum.area.is_some());
        let closure = sum.closure.unwrap();
        assert!(closure.linear_misclosure > 100.0);
        assert!(!sum.checks.closes_on_start);
        assert!(!sum.checks.meets_precision);
    }

    #[test]
    fn a_small_misclosure_still_meets_precision() {
        // Square with the last leg 1 cm short: perimeter ~400 m, misclosure 0.01 m →
        // precision ~1:40,000, which clears the default 1:10,000 but it does not land
        // back on the start within the 0.5 m join distance... actually 0.01 m does.
        let mut s = SurveySession::new(Point::new(0.0, 0.0));
        s.push_course(Course::new(Bearing::from_degrees(0.0), 100.0));
        s.push_course(Course::new(Bearing::from_degrees(90.0), 100.0));
        s.push_course(Course::new(Bearing::from_degrees(180.0), 100.0));
        s.push_course(Course::new(Bearing::from_degrees(270.0), 99.99));
        let sum = s.summary(&Tolerances::default());
        let closure = sum.closure.unwrap();
        assert!((closure.linear_misclosure - 0.01).abs() < 1e-6);
        assert!(closure.precision_ratio > 10_000.0);
        assert!(sum.checks.meets_precision);
        assert!(sum.checks.closes_on_start); // 0.01 m <= 0.5 m join distance
    }

    #[test]
    fn edit_operations_mutate_the_right_leg() {
        let mut s = unit_square();
        // Replace the last leg with a too-long one, then confirm it no longer closes.
        s.replace(3, Segment::Line(Course::new(Bearing::from_degrees(270.0), 150.0)))
            .unwrap();
        assert!(!s.summary(&Tolerances::default()).checks.closes_on_start);

        // Put it back by removing and inserting.
        let removed = s.remove(3).unwrap();
        assert_eq!(removed.as_course().distance, 150.0);
        s.insert(3, Segment::Line(Course::new(Bearing::from_degrees(270.0), 100.0)))
            .unwrap();
        assert!(s.summary(&Tolerances::default()).checks.closes_on_start);
    }

    #[test]
    fn out_of_range_edits_error() {
        let mut s = unit_square();
        assert!(matches!(
            s.replace(9, Segment::Line(Course::new(Bearing::from_degrees(0.0), 1.0))),
            Err(CoreError::SegmentIndexOutOfRange { index: 9, len: 4 })
        ));
        assert!(s.remove(9).is_err());
        // insert at len appends; past len errors.
        assert!(s.insert(4, Segment::Line(Course::new(Bearing::from_degrees(0.0), 1.0))).is_ok());
        assert!(s.insert(99, Segment::Line(Course::new(Bearing::from_degrees(0.0), 1.0))).is_err());
    }

    #[test]
    fn live_check_is_well_within_the_100ms_budget() {
        // NFR-03: the live recompute must be < 100 ms. Build a large figure and time a
        // single summary; O(legs) float work has orders of magnitude of headroom.
        let mut s = SurveySession::new(Point::new(0.0, 0.0));
        for i in 0..2000 {
            let bearing = Bearing::from_degrees((i as f64 * 0.18) % 360.0);
            s.push_course(Course::new(bearing, 10.0));
        }
        let tol = Tolerances::default();
        let t0 = std::time::Instant::now();
        let sum = s.summary(&tol);
        let elapsed = t0.elapsed();
        assert_eq!(sum.leg_count, 2000);
        assert!(
            elapsed < std::time::Duration::from_millis(50),
            "live check took {elapsed:?}, NFR-03 budget is 100 ms"
        );
    }
}
