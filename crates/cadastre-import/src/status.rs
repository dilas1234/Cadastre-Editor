//! Aumentum parcel status codes, confirmed with PLAGIS.

use serde::{Deserialize, Serialize};

use crate::ImportError;

/// The lifecycle status of an Aumentum parcel (column `Status`, a smallint).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParcelStatus {
    /// 1 — drafted in GIS but not approved.
    Pending,
    /// 2 — the current, valid parcel.
    Active,
    /// 3 — replaced by newer parcels.
    Historic,
    /// 4 — taken out of use or kept for reference.
    Archived,
}

impl ParcelStatus {
    /// Map the raw `Status` code. Errors on anything outside 1..=4.
    pub fn from_code(code: i16) -> Result<Self, ImportError> {
        match code {
            1 => Ok(Self::Pending),
            2 => Ok(Self::Active),
            3 => Ok(Self::Historic),
            4 => Ok(Self::Archived),
            other => Err(ImportError::UnknownStatus(other)),
        }
    }

    /// The source code for this status.
    pub fn code(self) -> i16 {
        match self {
            Self::Pending => 1,
            Self::Active => 2,
            Self::Historic => 3,
            Self::Archived => 4,
        }
    }

    /// A lowercase label for storage/reporting.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Active => "active",
            Self::Historic => "historic",
            Self::Archived => "archived",
        }
    }

    /// Whether this parcel is the current, valid version (only `Active`). Import defaults to
    /// bringing across active parcels as the live fabric; the rest are retained as history.
    pub fn is_current(self) -> bool {
        matches!(self, Self::Active)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_all_known_codes() {
        assert_eq!(ParcelStatus::from_code(1).unwrap(), ParcelStatus::Pending);
        assert_eq!(ParcelStatus::from_code(2).unwrap(), ParcelStatus::Active);
        assert_eq!(ParcelStatus::from_code(3).unwrap(), ParcelStatus::Historic);
        assert_eq!(ParcelStatus::from_code(4).unwrap(), ParcelStatus::Archived);
    }

    #[test]
    fn code_round_trips_and_labels() {
        for s in [
            ParcelStatus::Pending,
            ParcelStatus::Active,
            ParcelStatus::Historic,
            ParcelStatus::Archived,
        ] {
            assert_eq!(ParcelStatus::from_code(s.code()).unwrap(), s);
        }
        assert_eq!(ParcelStatus::Active.as_str(), "active");
        assert!(ParcelStatus::Active.is_current());
        assert!(!ParcelStatus::Historic.is_current());
    }

    #[test]
    fn rejects_unknown_codes() {
        assert_eq!(ParcelStatus::from_code(0), Err(ImportError::UnknownStatus(0)));
        assert_eq!(ParcelStatus::from_code(9), Err(ImportError::UnknownStatus(9)));
    }
}
