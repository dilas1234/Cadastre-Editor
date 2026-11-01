//! One entry point for every survey-exchange format the editor accepts.
//!
//! CSV/TSV coordinate lists, ASCII DXF drawings, LandXML documents and raster world files are
//! detected, parsed and normalised into a single [`ImportedSurvey`] (or [`RasterReference`]).
//! The web editor and the API call this module so a file gives the same result everywhere.

use std::collections::BTreeMap;
use std::fmt;

use cadastre_core::types::Point;
use serde::{Deserialize, Serialize};

use crate::dxf;
use crate::landxml;
use crate::tabular::{self, RowIssue};
use crate::worldfile::{self, RasterReference};

/// A recognised import format.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ImportFormat {
    /// Delimited coordinate list.
    Coordinates,
    /// ASCII DXF drawing.
    Dxf,
    /// LandXML survey exchange.
    LandXml,
    /// Raster world file plus optional projection sidecar.
    WorldFile,
}

impl ImportFormat {
    /// A human label.
    pub const fn label(self) -> &'static str {
        match self {
            ImportFormat::Coordinates => "Coordinate list",
            ImportFormat::Dxf => "DXF drawing",
            ImportFormat::LandXml => "LandXML",
            ImportFormat::WorldFile => "Raster world file",
        }
    }
}

/// One imported point.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImportedPoint {
    /// Point/beacon identifier.
    pub id: String,
    /// Coordinate.
    pub point: Point,
    /// Elevation, when present.
    pub elevation: Option<f64>,
    /// Feature code or description.
    pub code: Option<String>,
    /// Any other imported attributes.
    pub attributes: BTreeMap<String, String>,
}

/// One imported boundary.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImportedBoundary {
    /// Boundary/parcel name.
    pub id: String,
    /// Source kind (Parcel, PlanFeature, Alignment, DXF layer …).
    pub kind: String,
    /// Ring vertices in order.
    pub ring: Vec<Point>,
    /// Whether the source ring was closed.
    pub closed: bool,
    /// Stated area (m²), when the source carried one.
    pub stated_area: Option<f64>,
}

/// A normalised survey import.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ImportedSurvey {
    /// Format that produced this result.
    pub format: Option<ImportFormat>,
    /// Points.
    pub points: Vec<ImportedPoint>,
    /// Boundaries.
    pub boundaries: Vec<ImportedBoundary>,
    /// CRS named by the file or the caller.
    pub crs: Option<String>,
    /// EPSG code found in the file.
    pub epsg: Option<u32>,
    /// Non-fatal findings.
    pub warnings: Vec<String>,
    /// Rows that could not be imported.
    pub issues: Vec<ImportIssue>,
}

/// A non-fatal row problem, serialisable for the API.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImportIssue {
    /// One-based line number.
    pub line: usize,
    /// Description.
    pub message: String,
}

impl From<RowIssue> for ImportIssue {
    fn from(issue: RowIssue) -> Self {
        Self {
            line: issue.line,
            message: issue.message,
        }
    }
}

/// The result of importing one file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ImportResult {
    /// Vector survey data.
    Survey(ImportedSurvey),
    /// Raster georeferencing.
    Raster(RasterReference),
}

/// Why an import failed.
#[derive(Clone, Debug, PartialEq)]
pub enum ImportError {
    /// The format could not be identified.
    UnknownFormat,
    /// The file is too large for an in-request import.
    TooLarge {
        /// Actual size in bytes.
        size: usize,
        /// Configured limit.
        limit: usize,
    },
    /// A DXF parser error.
    Dxf(dxf::DxfError),
    /// A LandXML parser error.
    LandXml(landxml::LandXmlError),
    /// A world-file parser error.
    WorldFile(worldfile::WorldFileError),
    /// The input was identified but unusable.
    Invalid(String),
}

impl fmt::Display for ImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ImportError::UnknownFormat => write!(
                f,
                "unrecognised import format: expected CSV/TSV coordinates, ASCII DXF, LandXML or a world file"
            ),
            ImportError::TooLarge { size, limit } => {
                write!(f, "file is {size} bytes; the limit is {limit} bytes")
            }
            ImportError::Dxf(e) => write!(f, "{e}"),
            ImportError::LandXml(e) => write!(f, "{e}"),
            ImportError::WorldFile(e) => write!(f, "{e}"),
            ImportError::Invalid(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for ImportError {}

impl From<dxf::DxfError> for ImportError {
    fn from(e: dxf::DxfError) -> Self {
        ImportError::Dxf(e)
    }
}

impl From<landxml::LandXmlError> for ImportError {
    fn from(e: landxml::LandXmlError) -> Self {
        ImportError::LandXml(e)
    }
}

impl From<worldfile::WorldFileError> for ImportError {
    fn from(e: worldfile::WorldFileError) -> Self {
        ImportError::WorldFile(e)
    }
}

/// Default in-request limit (16 MiB); a survey exchange file above this should be imported as a
/// job, not synchronously in the editor.
pub const MAX_IMPORT_BYTES: usize = 16 * 1024 * 1024;

/// Detect the format from the file name, falling back to content sniffing.
pub fn detect_format(filename: &str, contents: &str) -> Option<ImportFormat> {
    let lower = filename.to_ascii_lowercase();
    let ext = lower.rsplit('.').next().unwrap_or("");
    match ext {
        "dxf" => return Some(ImportFormat::Dxf),
        "xml" | "landxml" => return Some(ImportFormat::LandXml),
        "csv" | "tsv" | "txt" | "psv" => return Some(ImportFormat::Coordinates),
        "tfw" | "tifw" | "jgw" | "jpgw" | "jpegw" | "pgw" | "gfw" | "bpw" | "j2w" | "wld" => {
            return Some(ImportFormat::WorldFile)
        }
        _ => {}
    }

    let head = contents.trim_start();
    if head.starts_with("<?xml") || head.starts_with("<LandXML") || head.contains("<LandXML") {
        return Some(ImportFormat::LandXml);
    }
    if head.starts_with("AutoCAD Binary DXF") || head.starts_with("0\nSECTION") || head.starts_with("0\r\nSECTION") {
        return Some(ImportFormat::Dxf);
    }
    let non_empty: Vec<&str> = head.lines().filter(|l| !l.trim().is_empty()).take(8).collect();
    if non_empty.len() >= 6
        && non_empty
            .iter()
            .take(6)
            .all(|l| l.trim().parse::<f64>().is_ok())
    {
        return Some(ImportFormat::WorldFile);
    }
    if !non_empty.is_empty()
        && non_empty
            .iter()
            .any(|l| l.contains(',') || l.contains('\t') || l.contains(';') || l.contains('|'))
    {
        return Some(ImportFormat::Coordinates);
    }
    None
}

fn survey_from_coordinates(
    contents: &str,
    crs: Option<&str>,
) -> ImportedSurvey {
    let list = tabular::parse_coordinates(contents, crs);
    ImportedSurvey {
        format: Some(ImportFormat::Coordinates),
        points: list
            .rows
            .into_iter()
            .map(|row| ImportedPoint {
                id: row.id,
                point: row.point,
                elevation: row.elevation,
                code: row.code,
                attributes: row.attributes,
            })
            .collect(),
        crs: list.crs,
        issues: list.issues.into_iter().map(ImportIssue::from).collect(),
        ..Default::default()
    }
}

fn survey_from_dxf(contents: &str) -> Result<ImportedSurvey, ImportError> {
    let drawing = dxf::parse(contents)?;
    let mut survey = ImportedSurvey {
        format: Some(ImportFormat::Dxf),
        warnings: drawing.warnings,
        ..Default::default()
    };
    for (i, p) in drawing.points.iter().enumerate() {
        survey.points.push(ImportedPoint {
            id: p
                .handle
                .clone()
                .unwrap_or_else(|| format!("P{}", i + 1)),
            point: p.point,
            elevation: Some(p.z).filter(|z| *z != 0.0),
            code: Some(p.layer.clone()),
            attributes: BTreeMap::new(),
        });
    }
    for (i, text) in drawing.texts.iter().enumerate() {
        // A text label is a named point in most survey drawings; keep its insertion point so a
        // beacon number can be matched later, but mark it so it is not confused with a mark.
        survey.points.push(ImportedPoint {
            id: if text.text.is_empty() {
                format!("T{}", i + 1)
            } else {
                text.text.clone()
            },
            point: text.point,
            elevation: None,
            code: Some(format!("TEXT:{}", text.layer)),
            attributes: BTreeMap::new(),
        });
    }
    for (i, line) in drawing.lines.iter().enumerate() {
        survey.boundaries.push(ImportedBoundary {
            id: format!("LINE{}", i + 1),
            kind: line.layer.clone(),
            ring: vec![line.from, line.to],
            closed: false,
            stated_area: None,
        });
    }
    for (i, polyline) in drawing.polylines.iter().enumerate() {
        survey.boundaries.push(ImportedBoundary {
            id: format!("POLYLINE{}", i + 1),
            kind: polyline.layer.clone(),
            ring: polyline.vertices.clone(),
            closed: polyline.closed,
            stated_area: None,
        });
    }
    for (i, circle) in drawing.circles.iter().enumerate() {
        // 64-gon keeps the circle's area to <0.01 % at survey scales.
        let ring: Vec<Point> = (0..64)
            .map(|k| {
                let a = std::f64::consts::TAU * k as f64 / 64.0;
                Point::new(
                    circle.centre.x + circle.radius * a.cos(),
                    circle.centre.y + circle.radius * a.sin(),
                )
            })
            .collect();
        survey.boundaries.push(ImportedBoundary {
            id: format!("CIRCLE{}", i + 1),
            kind: circle.layer.clone(),
            ring,
            closed: true,
            stated_area: Some(std::f64::consts::PI * circle.radius * circle.radius),
        });
    }
    Ok(survey)
}

fn survey_from_landxml(contents: &str) -> Result<ImportedSurvey, ImportError> {
    let doc = landxml::parse(contents)?;
    Ok(ImportedSurvey {
        format: Some(ImportFormat::LandXml),
        points: doc
            .points
            .into_iter()
            .map(|p| ImportedPoint {
                id: p.id,
                point: p.point,
                elevation: p.elevation,
                code: p.code,
                attributes: p.description.map(|d| {
                    let mut m = BTreeMap::new();
                    m.insert("description".to_string(), d);
                    m
                }).unwrap_or_default(),
            })
            .collect(),
        boundaries: doc
            .boundaries
            .into_iter()
            .map(|b| ImportedBoundary {
                id: b.name,
                kind: b.kind,
                ring: b.ring,
                closed: b.closed,
                stated_area: b.stated_area,
            })
            .collect(),
        epsg: doc.epsg,
        warnings: doc.warnings,
        ..Default::default()
    })
}

/// Import a text-based survey file.
///
/// `filename` selects the format (with content sniffing as a fallback); `crs` is used when the
/// file itself does not name a CRS.
pub fn import_text(
    filename: &str,
    contents: &str,
    crs: Option<&str>,
) -> Result<ImportResult, ImportError> {
    if contents.len() > MAX_IMPORT_BYTES {
        return Err(ImportError::TooLarge {
            size: contents.len(),
            limit: MAX_IMPORT_BYTES,
        });
    }
    let format = detect_format(filename, contents).ok_or(ImportError::UnknownFormat)?;
    let mut survey = match format {
        ImportFormat::Coordinates => survey_from_coordinates(contents, crs),
        ImportFormat::Dxf => survey_from_dxf(contents)?,
        ImportFormat::LandXml => survey_from_landxml(contents)?,
        ImportFormat::WorldFile => {
            return Ok(ImportResult::Raster(worldfile::reference_from(
                contents, None,
            )?))
        }
    };
    if survey.crs.is_none() {
        survey.crs = crs.map(str::to_string);
    }
    Ok(ImportResult::Survey(survey))
}

/// Import a coordinate list and join a separately supplied attribute table onto it.
pub fn import_coordinates_with_attributes(
    contents: &str,
    attributes: &str,
    key_column: Option<&str>,
    crs: Option<&str>,
) -> Result<ImportedSurvey, ImportError> {
    let mut list = tabular::parse_coordinates(contents, crs);
    let table = tabular::parse_attributes(attributes, key_column);
    let report = tabular::join_attributes(&mut list, &table);
    let mut survey = survey_from_coordinates(contents, crs);
    // Rebuild from the joined list so the attributes are carried across.
    survey.points = list
        .rows
        .into_iter()
        .map(|row| ImportedPoint {
            id: row.id,
            point: row.point,
            elevation: row.elevation,
            code: row.code,
            attributes: row.attributes,
        })
        .collect();
    if report.matched == 0 && !report.unmatched_points.is_empty() {
        survey.warnings.push(format!(
            "no attribute rows matched; check the key column `{}`",
            table.key_column
        ));
    }
    if !report.unmatched_keys.is_empty() {
        survey.warnings.push(format!(
            "{} attribute key(s) had no matching point",
            report.unmatched_keys.len()
        ));
    }
    if !report.unmatched_points.is_empty() {
        survey.warnings.push(format!(
            "{} point(s) had no attributes",
            report.unmatched_points.len()
        ));
    }
    Ok(survey)
}

/// Import a raster world file with its projection sidecar text.
pub fn import_world_file(
    world_contents: &str,
    prj_contents: Option<&str>,
) -> Result<RasterReference, ImportError> {
    Ok(worldfile::reference_from(world_contents, prj_contents)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CSV: &str = "Point,Easting,Northing,Code\n1,352440.677,1233094.048,BC\n2,352540.677,1233094.048,BC\n";

    #[test]
    fn detects_formats_by_extension_and_content() {
        assert_eq!(detect_format("pts.csv", ""), Some(ImportFormat::Coordinates));
        assert_eq!(detect_format("plan.dxf", ""), Some(ImportFormat::Dxf));
        assert_eq!(detect_format("plan.xml", ""), Some(ImportFormat::LandXml));
        assert_eq!(detect_format("ortho.tfw", ""), Some(ImportFormat::WorldFile));
        assert_eq!(
            detect_format("mystery", "<LandXML></LandXML>"),
            Some(ImportFormat::LandXml)
        );
        assert_eq!(
            detect_format("mystery", "0\nSECTION\n2\nENTITIES\n"),
            Some(ImportFormat::Dxf)
        );
        assert_eq!(
            detect_format("mystery", "1\n0\n0\n-1\n0\n0\n"),
            Some(ImportFormat::WorldFile)
        );
        assert_eq!(detect_format("mystery", "a,b\n1,2\n"), Some(ImportFormat::Coordinates));
    }

    #[test]
    fn imports_a_coordinate_list_and_keeps_metadata() {
        let ImportResult::Survey(survey) = import_text("pts.csv", CSV, Some("EPSG:32632")).unwrap() else {
            panic!("expected a survey");
        };
        assert_eq!(survey.format, Some(ImportFormat::Coordinates));
        assert_eq!(survey.points.len(), 2);
        assert_eq!(survey.points[0].id, "1");
        assert_eq!(survey.points[0].code.as_deref(), Some("BC"));
        assert_eq!(survey.crs.as_deref(), Some("EPSG:32632"));
    }

    #[test]
    fn imports_dxf_polyline_as_a_boundary() {
        let mut dxf_text = String::new();
        for (code, value) in [
            (0, "SECTION"),
            (2, "ENTITIES"),
            (0, "LWPOLYLINE"),
            (8, "PARCEL"),
            (70, "1"),
            (10, "0"),
            (20, "0"),
            (10, "100"),
            (20, "0"),
            (10, "100"),
            (20, "100"),
            (0, "ENDSEC"),
        ] {
            dxf_text.push_str(&format!("{code}\n{value}\n"));
        }
        let ImportResult::Survey(survey) = import_text("plan.dxf", &dxf_text, None).unwrap() else {
            panic!("expected a survey");
        };
        assert_eq!(survey.boundaries.len(), 1);
        assert!(survey.boundaries[0].closed);
        assert_eq!(survey.boundaries[0].kind, "PARCEL");
    }

    #[test]
    fn imports_landxml_points_and_parcels() {
        let doc = r#"<LandXML><CoordinateSystem epsgCode="32632"/><CgPoints>
            <CgPoint name="1">1233094.048 352440.677</CgPoint></CgPoints>
          <Parcels><Parcel name="PL1"><CoordGeom>
            <Line><Start>1233094.048 352440.677</Start><End>1233094.048 352540.677</End></Line>
            <Line><Start>1233094.048 352540.677</Start><End>1233194.048 352540.677</End></Line>
            <Line><Start>1233194.048 352540.677</Start><End>1233094.048 352440.677</End></Line>
          </CoordGeom></Parcel></Parcels></LandXML>"#;
        let ImportResult::Survey(survey) = import_text("plan.xml", doc, None).unwrap() else {
            panic!("expected a survey");
        };
        assert_eq!(survey.epsg, Some(32632));
        assert_eq!(survey.points.len(), 1);
        assert_eq!(survey.boundaries.len(), 1);
        assert_eq!(survey.boundaries[0].ring.len(), 3);
    }

    #[test]
    fn world_file_import_returns_georeferencing() {
        let ImportResult::Raster(raster) =
            import_text("ortho.tfw", "0.5\n0\n0\n-0.5\n100\n200\n", None).unwrap()
        else {
            panic!("expected a raster reference");
        };
        assert_eq!(raster.world.a, 0.5);
    }

    #[test]
    fn joins_attributes_onto_coordinates() {
        let attrs = "Point,Owner,PlanNumber\n1,Ada,PL1\n2,Bello,PL2\n";
        let survey = import_coordinates_with_attributes(CSV, attrs, Some("Point"), None).unwrap();
        assert_eq!(
            survey.points[0].attributes.get("Owner").map(String::as_str),
            Some("Ada")
        );
        assert_eq!(
            survey.points[1].attributes.get("PlanNumber").map(String::as_str),
            Some("PL2")
        );
    }

    #[test]
    fn unknown_and_oversized_input_is_rejected() {
        assert_eq!(import_text("x.bin", "hello", None).unwrap_err(), ImportError::UnknownFormat);
    }
}
