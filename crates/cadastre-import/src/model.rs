//! Source rows and their mapping to validated, importable records.

use cadastre_core::{area, Point};
use serde::Serialize;

use crate::exceptions::ExceptionKind;
use crate::geometry::ImportedGeometry;
use crate::status::ParcelStatus;

/// A raw parcel row from the reconstruction query, before validation.
#[derive(Clone, Debug)]
pub struct RawParcel {
    /// SDE `OBJECTID`.
    pub object_id: i32,
    /// `UPIN` (may be null/blank in placeholder rows).
    pub upin: Option<String>,
    /// `Status` smallint code.
    pub status_code: i16,
    /// `JobID` as a string GUID, if any.
    pub job_id: Option<String>,
    /// Plan number.
    pub plan_no: Option<String>,
    /// Cadastral zone.
    pub cad_zone: Option<String>,
    /// Plot number.
    pub plot_no: Option<String>,
    /// Block number.
    pub block_no: Option<String>,
    /// `SHAPE.STAsText()` WKT (null if no geometry).
    pub wkt: Option<String>,
    /// Stored SRID (expected 32632).
    pub srid: i32,
}

/// A validated parcel ready for staging.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct StagedParcel {
    /// Source `OBJECTID` (for traceability back to Aumentum).
    pub object_id: i32,
    /// Parcel number (from `UPIN`).
    pub parcel_number: String,
    /// Lifecycle status.
    pub status: ParcelStatus,
    /// Originating job GUID, if any.
    pub job_id: Option<String>,
    /// Plan number.
    pub plan_no: Option<String>,
    /// Cadastral zone.
    pub cad_zone: Option<String>,
    /// Plot number.
    pub plot_no: Option<String>,
    /// Block number.
    pub block_no: Option<String>,
    /// Area computed from the geometry (m²).
    pub computed_area: f64,
    /// Stored SRID.
    pub srid: i32,
    /// Boundary geometry.
    #[serde(skip)]
    pub geometry: ImportedGeometry,
}

impl RawParcel {
    /// Validate and map to a [`StagedParcel`], or classify why it can't be imported.
    pub fn validate(&self) -> Result<StagedParcel, ExceptionKind> {
        let status = ParcelStatus::from_code(self.status_code)
            .map_err(|_| ExceptionKind::UnknownStatus(self.status_code))?;

        let parcel_number = match &self.upin {
            Some(u) if !u.trim().is_empty() => u.trim().to_string(),
            _ => return Err(ExceptionKind::MissingUpin),
        };

        let wkt = self.wkt.as_deref().unwrap_or("");
        let geometry =
            ImportedGeometry::from_wkt(wkt).map_err(|_| ExceptionKind::InvalidGeometry)?;
        if geometry.is_empty() || geometry.exterior_ring().is_none() {
            return Err(ExceptionKind::EmptyOrNonPolygon);
        }

        let computed_area = geometry_area(&geometry).map_err(|_| ExceptionKind::DegenerateGeometry)?;
        if computed_area <= 0.0 {
            return Err(ExceptionKind::DegenerateGeometry);
        }

        Ok(StagedParcel {
            object_id: self.object_id,
            parcel_number,
            status,
            job_id: self.job_id.clone(),
            plan_no: self.plan_no.clone(),
            cad_zone: self.cad_zone.clone(),
            plot_no: self.plot_no.clone(),
            block_no: self.block_no.clone(),
            computed_area,
            srid: self.srid,
            geometry,
        })
    }
}

/// Net planar area (m²) of a (multi)polygon: exterior rings minus holes. Errors on non-areal
/// or degenerate geometry.
fn geometry_area(g: &ImportedGeometry) -> Result<f64, ()> {
    match g {
        ImportedGeometry::Polygon(rings) => ring_set_area(rings),
        ImportedGeometry::MultiPolygon(polys) => {
            let mut total = 0.0;
            for rings in polys {
                total += ring_set_area(rings)?;
            }
            Ok(total)
        }
        _ => Err(()),
    }
}

fn ring_set_area(rings: &[Vec<Point>]) -> Result<f64, ()> {
    let exterior = rings.first().ok_or(())?;
    // Degenerate exterior (fewer than 3 distinct vertices) is rejected.
    let mut net = area::polygon_area(exterior).map_err(|_| ())?;
    for hole in &rings[1..] {
        net -= area::signed_area(hole).abs();
    }
    Ok(net)
}

/// A raw control coordinate (`PLAGIS_COORDINATES`).
#[derive(Clone, Debug)]
pub struct RawCoordinate {
    /// `Alias` — the beacon/control name.
    pub alias: Option<String>,
    /// Easting (metres, EPSG:32632).
    pub x: f64,
    /// Northing (metres, EPSG:32632).
    pub y: f64,
}

/// A validated control beacon.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct StagedBeacon {
    /// Beacon number (from `Alias`).
    pub number: String,
    /// Easting (metres).
    pub easting: f64,
    /// Northing (metres).
    pub northing: f64,
}

impl RawCoordinate {
    /// Validate and map to a [`StagedBeacon`], or classify the failure.
    pub fn validate(&self) -> Result<StagedBeacon, ExceptionKind> {
        let number = match &self.alias {
            Some(a) if !a.trim().is_empty() => a.trim().to_string(),
            _ => return Err(ExceptionKind::MissingUpin),
        };
        if !self.x.is_finite() || !self.y.is_finite() {
            return Err(ExceptionKind::DegenerateGeometry);
        }
        Ok(StagedBeacon {
            number,
            easting: self.x,
            northing: self.y,
        })
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
    fn valid_active_parcel_maps_with_area() {
        let p = raw(1, Some("PL000000000002"), 2, Some(SQUARE)).validate().unwrap();
        assert_eq!(p.parcel_number, "PL000000000002");
        assert_eq!(p.status, ParcelStatus::Active);
        assert!((p.computed_area - 10_000.0).abs() < 0.01);
        assert_eq!(p.srid, 32632);
    }

    #[test]
    fn polygon_with_hole_subtracts_the_hole() {
        let wkt = "POLYGON ((0 0, 10 0, 10 10, 0 10, 0 0), (2 2, 4 2, 4 4, 2 4, 2 2))";
        let p = raw(1, Some("PL1"), 2, Some(wkt)).validate().unwrap();
        assert!((p.computed_area - (100.0 - 4.0)).abs() < 1e-6);
    }

    #[test]
    fn classifies_each_failure() {
        assert_eq!(raw(1, Some("PL1"), 9, Some(SQUARE)).validate().unwrap_err(), ExceptionKind::UnknownStatus(9));
        assert_eq!(raw(2, None, 2, Some(SQUARE)).validate().unwrap_err(), ExceptionKind::MissingUpin);
        assert_eq!(raw(3, Some("PL3"), 1, Some("POINT EMPTY")).validate().unwrap_err(), ExceptionKind::EmptyOrNonPolygon);
        assert_eq!(raw(4, Some("PL4"), 2, Some("POINT (5 5)")).validate().unwrap_err(), ExceptionKind::EmptyOrNonPolygon);
        assert_eq!(raw(5, Some("PL5"), 2, Some("LINESTRING (0 0,1 1)")).validate().unwrap_err(), ExceptionKind::InvalidGeometry);
    }

    #[test]
    fn coordinate_maps_to_beacon() {
        let b = RawCoordinate { alias: Some("N1".into()), x: 488_249.908, y: 1_099_097.516 }
            .validate()
            .unwrap();
        assert_eq!(b.number, "N1");
        assert!((b.easting - 488_249.908).abs() < 1e-6);
    }
}
