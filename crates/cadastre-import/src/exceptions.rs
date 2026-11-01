//! Exception classification and the import report (FR §8 "exceptions report").

use std::collections::BTreeMap;
use std::collections::HashMap;

use serde::Serialize;

use crate::model::{RawParcel, StagedParcel};

/// Why a source row could not be imported cleanly.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum ExceptionKind {
    /// `Status` was not 1..=4.
    UnknownStatus(i16),
    /// `UPIN` (or beacon `Alias`) was null/blank.
    MissingUpin,
    /// The WKT geometry could not be parsed or was an unsupported type.
    InvalidGeometry,
    /// The geometry was empty or not an areal (polygon) feature — e.g. placeholder `POINT`s.
    EmptyOrNonPolygon,
    /// The polygon had fewer than three distinct vertices / non-positive area.
    DegenerateGeometry,
    /// Another row with the same parcel number was also present; both need human resolution.
    DuplicateUpin,
}

/// One rejected row.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Exception {
    /// Source `OBJECTID`.
    pub object_id: i32,
    /// The parcel number if known (helps triage).
    pub parcel_number: Option<String>,
    /// Why it was rejected.
    pub kind: ExceptionKind,
}

/// The outcome of staging a batch of parcels.
#[derive(Clone, Debug, Default, Serialize)]
pub struct ImportReport {
    /// Number of raw rows considered.
    pub total: usize,
    /// Clean, importable parcels with unique parcel numbers.
    pub staged: Vec<StagedParcel>,
    /// Rows that could not be imported, with a reason.
    pub exceptions: Vec<Exception>,
    /// Count of staged parcels by status label.
    pub status_counts: BTreeMap<String, usize>,
}

impl ImportReport {
    /// Convenience: number of clean parcels.
    pub fn staged_count(&self) -> usize {
        self.staged.len()
    }

    /// Convenience: number of rejected rows.
    pub fn exception_count(&self) -> usize {
        self.exceptions.len()
    }
}

/// Validate and stage a batch of raw parcels. Invalid rows become exceptions.
///
/// In Aumentum a UPIN keeps its identity through re-survey and mutation: the old geometry is kept
/// as historic/archived under the same UPIN and the new one is active. So a UPIN that appears on
/// one active row plus history rows is normal lineage and all of its rows are staged. Only a UPIN
/// held by two or more *active* rows is a genuine conflict (the live fabric can hold one parcel per
/// UPIN); those active rows are moved to exceptions for review rather than either being chosen.
pub fn stage_parcels(raws: &[RawParcel]) -> ImportReport {
    let mut staged: Vec<StagedParcel> = Vec::new();
    let mut exceptions: Vec<Exception> = Vec::new();

    for raw in raws {
        match raw.validate() {
            Ok(p) => staged.push(p),
            Err(kind) => exceptions.push(Exception {
                object_id: raw.object_id,
                parcel_number: raw.upin.clone(),
                kind,
            }),
        }
    }

    // Detect parcel numbers held by more than one current (active) row.
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for p in staged.iter().filter(|p| p.status.is_current()) {
        *counts.entry(p.parcel_number.as_str()).or_default() += 1;
    }
    let dup_numbers: std::collections::HashSet<String> = counts
        .iter()
        .filter(|(_, &n)| n > 1)
        .map(|(k, _)| k.to_string())
        .collect();

    if !dup_numbers.is_empty() {
        let (dups, keep): (Vec<_>, Vec<_>) = staged
            .into_iter()
            .partition(|p| p.status.is_current() && dup_numbers.contains(&p.parcel_number));
        staged = keep;
        for p in dups {
            exceptions.push(Exception {
                object_id: p.object_id,
                parcel_number: Some(p.parcel_number),
                kind: ExceptionKind::DuplicateUpin,
            });
        }
    }

    let mut status_counts: BTreeMap<String, usize> = BTreeMap::new();
    for p in &staged {
        *status_counts.entry(p.status.as_str().to_string()).or_default() += 1;
    }

    ImportReport {
        total: raws.len(),
        staged,
        exceptions,
        status_counts,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(object_id: i32, upin: Option<&str>, status: i16, wkt: Option<&str>) -> RawParcel {
        RawParcel {
            object_id,
            upin: upin.map(str::to_string),
            status_code: status,
            job_id: None,
            plan_no: None,
            cad_zone: None,
            plot_no: None,
            block_no: None,
            wkt: wkt.map(str::to_string),
            srid: 32632,
        }
    }

    const SQUARE: &str = "POLYGON ((0 0, 100 0, 100 100, 0 100, 0 0))";

    #[test]
    fn stages_clean_rows_and_reports_exceptions() {
        let raws = vec![
            raw(1, Some("PL1"), 2, Some(SQUARE)),       // active, ok
            raw(2, Some("PL2"), 1, Some(SQUARE)),       // pending, ok
            raw(3, Some("PL3"), 3, Some(SQUARE)),       // historic, ok
            raw(4, None, 2, Some(SQUARE)),              // missing upin
            raw(5, Some("PL5"), 9, Some(SQUARE)),       // bad status
            raw(6, Some("PL6"), 2, Some("POINT EMPTY")), // empty geom
        ];
        let r = stage_parcels(&raws);
        assert_eq!(r.total, 6);
        assert_eq!(r.staged_count(), 3);
        assert_eq!(r.exception_count(), 3);
        assert_eq!(r.status_counts.get("active"), Some(&1));
        assert_eq!(r.status_counts.get("pending"), Some(&1));
        assert_eq!(r.status_counts.get("historic"), Some(&1));
    }

    #[test]
    fn two_active_parcels_with_one_upin_are_moved_to_exceptions() {
        let raws = vec![
            raw(1, Some("PL1"), 2, Some(SQUARE)),
            raw(2, Some("PL1"), 2, Some(SQUARE)), // two ACTIVE rows, same UPIN -> both for review
            raw(3, Some("PL2"), 2, Some(SQUARE)),
        ];
        let r = stage_parcels(&raws);
        assert_eq!(r.staged_count(), 1);
        assert_eq!(r.staged[0].parcel_number, "PL2");
        assert_eq!(r.exception_count(), 2);
        assert!(r.exceptions.iter().all(|e| e.kind == ExceptionKind::DuplicateUpin));
    }

    #[test]
    fn a_upin_with_one_active_row_and_history_is_lineage_not_a_conflict() {
        // As on production: PL000000002470 is active (re-surveyed) and also archived (old shape).
        let raws = vec![
            raw(6388, Some("PL000000002470"), 2, Some(SQUARE)), // active
            raw(7192, Some("PL000000002470"), 4, Some(SQUARE)), // archived predecessor
            raw(6387, Some("PL000000002469"), 4, Some(SQUARE)), // two archived rows: history only
            raw(7191, Some("PL000000002469"), 4, Some(SQUARE)),
        ];
        let r = stage_parcels(&raws);
        assert_eq!(r.exception_count(), 0, "{:?}", r.exceptions);
        assert_eq!(r.staged_count(), 4);
        assert_eq!(r.status_counts.get("active"), Some(&1));
        assert_eq!(r.status_counts.get("archived"), Some(&3));
    }
}
