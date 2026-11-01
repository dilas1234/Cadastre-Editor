//! World files and projection sidecars for raster import.
//!
//! A world file gives the affine mapping between pixel (column, row) coordinates and map
//! coordinates:
//!
//! ```text
//! x = A·col + B·row + C
//! y = D·col + E·row + F
//! ```
//!
//! Six numeric lines are the standard form (`A, D, B, E, C, F`), with the first and second
//! lines being the pixel size in x and y. Some writers append a seventh line with the rotation
//! in degrees; it is accepted and ignored because `B`/`D` already carry the rotation.
//!
//! The `.prj` sidecar carries the CRS. It is treated as an opaque WKT string here; the server
//! matches its authority code when it can and otherwise records it for review.

use std::fmt;

use cadastre_core::types::Point;
use serde::{Deserialize, Serialize};

/// A world-file affine transform between pixel and map coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorldFile {
    /// X size of one pixel, in map units.
    pub a: f64,
    /// Y size of one pixel, in map units (normally negative: rows increase southwards).
    pub e: f64,
    /// Rotation/shear terms.
    pub b: f64,
    /// Rotation/shear terms.
    pub d: f64,
    /// Map x of the centre of the top-left pixel.
    pub c: f64,
    /// Map y of the centre of the top-left pixel.
    pub f: f64,
}

/// Why a world file could not be used.
#[derive(Clone, Debug, PartialEq)]
pub enum WorldFileError {
    /// Fewer than six numeric lines.
    TooFewValues(usize),
    /// A line was not a finite number.
    NotANumber {
        /// One-based line number.
        line: usize,
        /// The offending text.
        value: String,
    },
    /// The linear part is singular, so there is no inverse mapping.
    Singular,
}

impl fmt::Display for WorldFileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WorldFileError::TooFewValues(n) => {
                write!(f, "world file needs at least 6 values, found {n}")
            }
            WorldFileError::NotANumber { line, value } => {
                write!(f, "world file line {line} is not a number: `{value}`")
            }
            WorldFileError::Singular => write!(f, "world file transform is singular"),
        }
    }
}

impl std::error::Error for WorldFileError {}

impl WorldFile {
    /// Parse the six standard world-file values.
    pub fn parse(contents: &str) -> Result<Self, WorldFileError> {
        let values: Vec<(usize, &str)> = contents
            .lines()
            .enumerate()
            .map(|(i, line)| (i + 1, line.trim()))
            .filter(|(_, line)| !line.is_empty())
            .collect();
        if values.len() < 6 {
            return Err(WorldFileError::TooFewValues(values.len()));
        }
        let mut numbers = Vec::with_capacity(6);
        for (line, text) in values.iter().take(6) {
            let value = text.parse::<f64>().map_err(|_| WorldFileError::NotANumber {
                line: *line,
                value: (*text).to_string(),
            })?;
            if !value.is_finite() {
                return Err(WorldFileError::NotANumber {
                    line: *line,
                    value: (*text).to_string(),
                });
            }
            numbers.push(value);
        }
        // Standard order: A, D, B, E, C, F.
        let world = Self {
            a: numbers[0],
            d: numbers[1],
            b: numbers[2],
            e: numbers[3],
            c: numbers[4],
            f: numbers[5],
        };
        world.validate()?;
        Ok(world)
    }

    /// A north-up world file from the top-left corner and pixel size.
    ///
    /// `x`/`y` are the map coordinates of the top-left pixel's **outer** corner; the world file
    /// convention stores the pixel centre, hence the half-pixel shift.
    pub fn north_up(x: f64, y: f64, pixel_width: f64, pixel_height: f64) -> Self {
        Self {
            a: pixel_width,
            b: 0.0,
            c: x + pixel_width / 2.0,
            d: 0.0,
            e: -pixel_height.abs(),
            f: y - pixel_height.abs() / 2.0,
        }
    }

    fn validate(&self) -> Result<(), WorldFileError> {
        let determinant = self.a * self.e - self.b * self.d;
        if determinant.abs() < 1e-18 || !determinant.is_finite() {
            return Err(WorldFileError::Singular);
        }
        Ok(())
    }

    /// Determinant of the linear part.
    pub fn determinant(&self) -> f64 {
        self.a * self.e - self.b * self.d
    }

    /// Map coordinates of a pixel centre.
    pub fn pixel_to_map(&self, col: f64, row: f64) -> Point {
        Point::new(
            self.a * col + self.b * row + self.c,
            self.d * col + self.e * row + self.f,
        )
    }

    /// The pixel coordinate containing a map point.
    pub fn map_to_pixel(&self, point: Point) -> Point {
        let det = self.determinant();
        if det.abs() < 1e-18 {
            return Point::new(f64::NAN, f64::NAN);
        }
        let dx = point.x - self.c;
        let dy = point.y - self.f;
        Point::new(
            (self.e * dx - self.b * dy) / det,
            (-self.d * dx + self.a * dy) / det,
        )
    }

    /// Whether the image is rotated or sheared relative to the map axes.
    pub fn is_rotated(&self) -> bool {
        self.b.abs() > 1e-12 || self.d.abs() > 1e-12
    }

    /// Rotation of the image in degrees (anticlockwise positive), from the +x pixel direction.
    pub fn rotation_degrees(&self) -> f64 {
        self.d.atan2(self.a).to_degrees()
    }

    /// Pixel size (x, y) in map units.
    pub fn pixel_size(&self) -> (f64, f64) {
        (self.a.hypot(self.d), self.b.hypot(self.e))
    }

    /// A `[a, b, c, d, e, f]` array for JSON/SQL storage.
    pub fn to_array(self) -> [f64; 6] {
        [self.a, self.b, self.c, self.d, self.e, self.f]
    }
}

/// A world file together with its projection sidecar.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RasterReference {
    /// Pixel-to-map transform.
    pub world: WorldFile,
    /// Raw WKT from the sibling `.prj`, if present.
    pub wkt: Option<String>,
    /// EPSG authority code found in the WKT, if any.
    pub epsg: Option<u32>,
}

/// The world-file extension for a raster file name, if one is conventional.
pub fn world_file_extension(filename: &str) -> Option<&'static str> {
    let lower = filename.to_ascii_lowercase();
    let ext = lower.rsplit('.').next()?;
    Some(match ext {
        "tif" | "tiff" => "tfw",
        "jpg" | "jpeg" => "jgw",
        "png" => "pgw",
        "gif" => "gfw",
        "bmp" => "bpw",
        "jp2" => "j2w",
        _ => return None,
    })
}

/// Whether a file name is a recognised world file / projection sidecar.
pub fn is_world_file(filename: &str) -> bool {
    let lower = filename.to_ascii_lowercase();
    matches!(
        lower.rsplit('.').next().unwrap_or(""),
        "tfw" | "tifw" | "jgw" | "jpgw" | "jpegw" | "pgw" | "gfw" | "bpw" | "j2w" | "wld" | "prj"
    )
}

/// Extract an EPSG code from WKT (`AUTHORITY["EPSG","32632"]` or `EPSG:32632`).
pub fn epsg_from_wkt(wkt: &str) -> Option<u32> {
    let upper = wkt.to_ascii_uppercase();
    if let Some(pos) = upper.rfind("AUTHORITY[\"EPSG\",\"") {
        let rest = &upper[pos + "AUTHORITY[\"EPSG\",\"".len()..];
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        return digits.parse().ok();
    }
    // PROJ string / simple form.
    for prefix in ["EPSG:", "EPSG\",\""] {
        if let Some(pos) = upper.find(prefix) {
            let rest = &upper[pos + prefix.len()..];
            let digits: String = rest
                .trim_start_matches('"')
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect();
            if let Ok(code) = digits.parse() {
                return Some(code);
            }
        }
    }
    None
}

/// Build a raster reference from a world file and optional `.prj` text.
pub fn reference_from(
    world_contents: &str,
    prj_contents: Option<&str>,
) -> Result<RasterReference, WorldFileError> {
    let world = WorldFile::parse(world_contents)?;
    let wkt = prj_contents
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let epsg = wkt.as_deref().and_then(epsg_from_wkt);
    Ok(RasterReference { world, wkt, epsg })
}

#[cfg(test)]
mod tests {
    use super::*;

    const NORTH_UP: &str = "0.5\n0.0\n0.0\n-0.5\n352440.25\n1233094.75\n";

    #[test]
    fn reads_a_north_up_world_file() {
        let w = WorldFile::parse(NORTH_UP).unwrap();
        assert_eq!(w.a, 0.5);
        assert_eq!(w.e, -0.5);
        assert!((w.c - 352_440.25).abs() < 1e-9);
        assert!((w.f - 1_233_094.75).abs() < 1e-9);
        assert!(!w.is_rotated());
        // Centre of the first pixel is the stored origin.
        let p = w.pixel_to_map(0.0, 0.0);
        assert!((p.x - 352_440.25).abs() < 1e-9);
        assert!((p.y - 1_233_094.75).abs() < 1e-9);
        // Moving one column east and one row south.
        let p = w.pixel_to_map(1.0, 1.0);
        assert!((p.x - 352_440.75).abs() < 1e-9);
        assert!((p.y - 1_233_094.25).abs() < 1e-9);
    }

    #[test]
    fn map_to_pixel_round_trips() {
        let w = WorldFile::parse(NORTH_UP).unwrap();
        for (col, row) in [(0.0, 0.0), (10.5, 3.25), (-4.0, 900.0)] {
            let map = w.pixel_to_map(col, row);
            let pixel = w.map_to_pixel(map);
            assert!((pixel.x - col).abs() < 1e-9);
            assert!((pixel.y - row).abs() < 1e-9);
        }
    }

    #[test]
    fn rotated_world_file_round_trips() {
        // 30° rotation, 2 m pixels.
        let theta = 30f64.to_radians();
        let w = WorldFile {
            a: 2.0 * theta.cos(),
            d: 2.0 * theta.sin(),
            b: 2.0 * theta.sin(),
            e: -2.0 * theta.cos(),
            c: 500_000.0,
            f: 1_000_000.0,
        };
        assert!(w.is_rotated());
        assert!((w.rotation_degrees() - 30.0).abs() < 1e-9);
        let map = w.pixel_to_map(12.0, 34.0);
        let pixel = w.map_to_pixel(map);
        assert!((pixel.x - 12.0).abs() < 1e-9);
        assert!((pixel.y - 34.0).abs() < 1e-9);
    }

    #[test]
    fn rejects_bad_files() {
        assert_eq!(
            WorldFile::parse("1\n2\n3\n"),
            Err(WorldFileError::TooFewValues(3))
        );
        assert!(matches!(
            WorldFile::parse("x\n0\n0\n1\n0\n0\n"),
            Err(WorldFileError::NotANumber { line: 1, .. })
        ));
        assert_eq!(
            WorldFile::parse("0\n0\n0\n0\n0\n0\n"),
            Err(WorldFileError::Singular)
        );
    }

    #[test]
    fn north_up_helper_offsets_the_outer_corner() {
        let w = WorldFile::north_up(352_440.0, 1_233_095.0, 1.0, 1.0);
        let p = w.pixel_to_map(0.0, 0.0);
        assert!((p.x - 352_440.5).abs() < 1e-9);
        assert!((p.y - 1_233_094.5).abs() < 1e-9);
    }

    #[test]
    fn recognises_sidecars_and_epsg_codes() {
        assert_eq!(world_file_extension("ortho.TIF"), Some("tfw"));
        assert_eq!(world_file_extension("scan.jpg"), Some("jgw"));
        assert!(is_world_file("scan.tfw"));
        assert!(is_world_file("scan.prj"));
        let wkt = r#"PROJCS["WGS 84 / UTM zone 32N",GEOGCS["WGS 84"],AUTHORITY["EPSG","32632"]]"#;
        assert_eq!(epsg_from_wkt(wkt), Some(32632));
        assert_eq!(epsg_from_wkt("EPSG:26392"), Some(26392));
    }

    #[test]
    fn reference_keeps_the_prj_and_its_code() {
        let r = reference_from(
            NORTH_UP,
            Some(r#"PROJCS["Minna / Nigeria Mid Belt",AUTHORITY["EPSG","26392"]]"#),
        )
        .unwrap();
        assert_eq!(r.epsg, Some(26392));
        assert!(r.wkt.unwrap().starts_with("PROJCS"));
    }
}
