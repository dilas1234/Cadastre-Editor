//! Coordinate reference systems — pure Rust, no `proj` C dependency (protects the Windows
//! Server 2012 R2 `win7` target; SRS §2.4, NFR-02).
//!
//! The working CRS is **EPSG:26392 — Minna / Nigeria Mid Belt** (Transverse Mercator on the
//! Clarke 1880 (RGS) ellipsoid). A 3-parameter geocentric translation relates Minna to WGS 84.
//!
//! ⚠️ The datum-shift parameters below are provisional. The exact Minna→WGS 84 transformation
//! PLAGIS uses must be confirmed (SRS §9) and is configurable via [`DatumShift`].

use crate::types::Point;

/// A reference ellipsoid defined by semi-major axis and inverse flattening.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ellipsoid {
    /// Semi-major axis (metres).
    pub a: f64,
    /// Inverse flattening `1/f`.
    pub inv_f: f64,
}

impl Ellipsoid {
    /// Clarke 1880 (RGS) — the Minna datum ellipsoid.
    pub const CLARKE_1880_RGS: Ellipsoid = Ellipsoid {
        a: 6_378_249.145,
        inv_f: 293.465,
    };
    /// WGS 84.
    pub const WGS84: Ellipsoid = Ellipsoid {
        a: 6_378_137.0,
        inv_f: 298.257_223_563,
    };

    /// Flattening `f`.
    pub fn f(self) -> f64 {
        1.0 / self.inv_f
    }
    /// First eccentricity squared `e²`.
    pub fn e2(self) -> f64 {
        let f = self.f();
        f * (2.0 - f)
    }
    /// Second eccentricity squared `e'²`.
    pub fn ep2(self) -> f64 {
        self.e2() / (1.0 - self.e2())
    }
    /// Semi-minor axis `b`.
    pub fn b(self) -> f64 {
        self.a * (1.0 - self.f())
    }
}

/// Transverse Mercator projection parameters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TransverseMercator {
    /// The projection ellipsoid.
    pub ellipsoid: Ellipsoid,
    /// Latitude of natural origin (radians).
    pub lat_origin: f64,
    /// Longitude of natural origin / central meridian (radians).
    pub lon_origin: f64,
    /// Scale factor at the central meridian.
    pub k0: f64,
    /// False easting (metres).
    pub false_easting: f64,
    /// False northing (metres).
    pub false_northing: f64,
}

impl TransverseMercator {
    /// EPSG:26392 — Minna / Nigeria Mid Belt.
    pub fn nigeria_mid_belt() -> Self {
        Self {
            ellipsoid: Ellipsoid::CLARKE_1880_RGS,
            lat_origin: 4.0_f64.to_radians(),
            lon_origin: 8.5_f64.to_radians(),
            k0: 0.999_75,
            false_easting: 670_553.98,
            false_northing: 0.0,
        }
    }

    /// Meridian arc length from the equator to latitude `phi`.
    fn meridian_arc(&self, phi: f64) -> f64 {
        let a = self.ellipsoid.a;
        let e2 = self.ellipsoid.e2();
        let e4 = e2 * e2;
        let e6 = e4 * e2;
        a * ((1.0 - e2 / 4.0 - 3.0 * e4 / 64.0 - 5.0 * e6 / 256.0) * phi
            - (3.0 * e2 / 8.0 + 3.0 * e4 / 32.0 + 45.0 * e6 / 1024.0) * (2.0 * phi).sin()
            + (15.0 * e4 / 256.0 + 45.0 * e6 / 1024.0) * (4.0 * phi).sin()
            - (35.0 * e6 / 3072.0) * (6.0 * phi).sin())
    }

    /// Project geographic coordinates (radians) to grid easting/northing (metres).
    pub fn forward(&self, lat: f64, lon: f64) -> Point {
        let a = self.ellipsoid.a;
        let e2 = self.ellipsoid.e2();
        let ep2 = self.ellipsoid.ep2();

        let (sin_phi, cos_phi) = lat.sin_cos();
        let tan_phi = sin_phi / cos_phi;

        let n = a / (1.0 - e2 * sin_phi * sin_phi).sqrt();
        let t = tan_phi * tan_phi;
        let c = ep2 * cos_phi * cos_phi;
        let big_a = (lon - self.lon_origin) * cos_phi;
        let m = self.meridian_arc(lat);
        let m0 = self.meridian_arc(self.lat_origin);

        let a2 = big_a * big_a;
        let a3 = a2 * big_a;
        let a4 = a2 * a2;
        let a5 = a4 * big_a;
        let a6 = a4 * a2;

        let easting = self.false_easting
            + self.k0
                * n
                * (big_a
                    + (1.0 - t + c) * a3 / 6.0
                    + (5.0 - 18.0 * t + t * t + 72.0 * c - 58.0 * ep2) * a5 / 120.0);

        let northing = self.false_northing
            + self.k0
                * (m - m0
                    + n * tan_phi
                        * (a2 / 2.0
                            + (5.0 - t + 9.0 * c + 4.0 * c * c) * a4 / 24.0
                            + (61.0 - 58.0 * t + t * t + 600.0 * c - 330.0 * ep2) * a6 / 720.0));

        Point::new(easting, northing)
    }

    /// Inverse-project grid easting/northing (metres) to geographic coordinates (radians).
    pub fn inverse(&self, grid: Point) -> (f64, f64) {
        let a = self.ellipsoid.a;
        let e2 = self.ellipsoid.e2();
        let ep2 = self.ellipsoid.ep2();
        let e4 = e2 * e2;
        let e6 = e4 * e2;

        let m0 = self.meridian_arc(self.lat_origin);
        let m = m0 + (grid.y - self.false_northing) / self.k0;
        let mu = m / (a * (1.0 - e2 / 4.0 - 3.0 * e4 / 64.0 - 5.0 * e6 / 256.0));

        let e1 = (1.0 - (1.0 - e2).sqrt()) / (1.0 + (1.0 - e2).sqrt());
        let e1_2 = e1 * e1;
        let e1_3 = e1_2 * e1;
        let e1_4 = e1_2 * e1_2;

        let phi1 = mu
            + (3.0 * e1 / 2.0 - 27.0 * e1_3 / 32.0) * (2.0 * mu).sin()
            + (21.0 * e1_2 / 16.0 - 55.0 * e1_4 / 32.0) * (4.0 * mu).sin()
            + (151.0 * e1_3 / 96.0) * (6.0 * mu).sin()
            + (1097.0 * e1_4 / 512.0) * (8.0 * mu).sin();

        let (sin_p1, cos_p1) = phi1.sin_cos();
        let tan_p1 = sin_p1 / cos_p1;
        let c1 = ep2 * cos_p1 * cos_p1;
        let t1 = tan_p1 * tan_p1;
        let n1 = a / (1.0 - e2 * sin_p1 * sin_p1).sqrt();
        let r1 = a * (1.0 - e2) / (1.0 - e2 * sin_p1 * sin_p1).powf(1.5);
        let d = (grid.x - self.false_easting) / (n1 * self.k0);

        let d2 = d * d;
        let d3 = d2 * d;
        let d4 = d2 * d2;
        let d5 = d4 * d;
        let d6 = d4 * d2;

        let lat = phi1
            - (n1 * tan_p1 / r1)
                * (d2 / 2.0 - (5.0 + 3.0 * t1 + 10.0 * c1 - 4.0 * c1 * c1 - 9.0 * ep2) * d4 / 24.0
                    + (61.0 + 90.0 * t1 + 298.0 * c1 + 45.0 * t1 * t1
                        - 252.0 * ep2
                        - 3.0 * c1 * c1)
                        * d6
                        / 720.0);

        let lon = self.lon_origin
            + (d - (1.0 + 2.0 * t1 + c1) * d3 / 6.0
                + (5.0 - 2.0 * c1 + 28.0 * t1 - 3.0 * c1 * c1 + 8.0 * ep2 + 24.0 * t1 * t1) * d5
                    / 120.0)
                / cos_p1;

        (lat, lon)
    }
}

/// A 3-parameter geocentric translation between two datums (metres).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DatumShift {
    /// X translation (metres).
    pub dx: f64,
    /// Y translation (metres).
    pub dy: f64,
    /// Z translation (metres).
    pub dz: f64,
}

impl DatumShift {
    /// Provisional Minna → WGS 84 geocentric translation. **TO BE CONFIRMED with PLAGIS**
    /// (SRS §9 / NFR-02) before any production use.
    pub const MINNA_TO_WGS84_PROVISIONAL: DatumShift = DatumShift {
        dx: -92.0,
        dy: -93.0,
        dz: 122.0,
    };

    /// The reverse translation.
    pub fn inverse(self) -> Self {
        Self {
            dx: -self.dx,
            dy: -self.dy,
            dz: -self.dz,
        }
    }
}

/// Geodetic (radians, metres) to Earth-centred, Earth-fixed XYZ (metres).
fn geodetic_to_ecef(e: Ellipsoid, lat: f64, lon: f64, h: f64) -> (f64, f64, f64) {
    let e2 = e.e2();
    let (sin_lat, cos_lat) = lat.sin_cos();
    let (sin_lon, cos_lon) = lon.sin_cos();
    let n = e.a / (1.0 - e2 * sin_lat * sin_lat).sqrt();
    let x = (n + h) * cos_lat * cos_lon;
    let y = (n + h) * cos_lat * sin_lon;
    let z = (n * (1.0 - e2) + h) * sin_lat;
    (x, y, z)
}

/// ECEF XYZ (metres) to geodetic (radians, metres) via Bowring's closed-form method.
fn ecef_to_geodetic(e: Ellipsoid, x: f64, y: f64, z: f64) -> (f64, f64, f64) {
    let a = e.a;
    let b = e.b();
    let e2 = e.e2();
    let ep2 = e.ep2();

    let p = (x * x + y * y).sqrt();
    let theta = (z * a).atan2(p * b);
    let (sin_t, cos_t) = theta.sin_cos();

    let lat = (z + ep2 * b * sin_t.powi(3)).atan2(p - e2 * a * cos_t.powi(3));
    let lon = y.atan2(x);
    let sin_lat = lat.sin();
    let n = a / (1.0 - e2 * sin_lat * sin_lat).sqrt();
    let h = p / lat.cos() - n;
    (lat, lon, h)
}

/// A full working CRS: the projection plus the datum relationship to WGS 84.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorkingCrs {
    /// The projected CRS (EPSG:26392 by default).
    pub projection: TransverseMercator,
    /// Datum shift from the local datum (Minna) to WGS 84.
    pub to_wgs84: DatumShift,
}

impl Default for WorkingCrs {
    fn default() -> Self {
        Self {
            projection: TransverseMercator::nigeria_mid_belt(),
            to_wgs84: DatumShift::MINNA_TO_WGS84_PROVISIONAL,
        }
    }
}

impl WorkingCrs {
    /// Project local-datum geographic coordinates (decimal degrees) to grid metres.
    pub fn project_deg(&self, lat_deg: f64, lon_deg: f64) -> Point {
        self.projection
            .forward(lat_deg.to_radians(), lon_deg.to_radians())
    }

    /// Inverse-project grid metres to local-datum geographic coordinates (decimal degrees).
    pub fn unproject_deg(&self, grid: Point) -> (f64, f64) {
        let (lat, lon) = self.projection.inverse(grid);
        (lat.to_degrees(), lon.to_degrees())
    }

    /// Convert a grid point (Minna) to WGS 84 latitude/longitude (decimal degrees).
    pub fn grid_to_wgs84_deg(&self, grid: Point) -> (f64, f64) {
        let (lat, lon) = self.projection.inverse(grid);
        let (x, y, z) = geodetic_to_ecef(self.projection.ellipsoid, lat, lon, 0.0);
        let (x, y, z) = (
            x + self.to_wgs84.dx,
            y + self.to_wgs84.dy,
            z + self.to_wgs84.dz,
        );
        let (wlat, wlon, _h) = ecef_to_geodetic(Ellipsoid::WGS84, x, y, z);
        (wlat.to_degrees(), wlon.to_degrees())
    }

    /// Convert WGS 84 latitude/longitude (decimal degrees) to a grid point (Minna).
    pub fn wgs84_to_grid_deg(&self, lat_deg: f64, lon_deg: f64) -> Point {
        let (x, y, z) = geodetic_to_ecef(
            Ellipsoid::WGS84,
            lat_deg.to_radians(),
            lon_deg.to_radians(),
            0.0,
        );
        let back = self.to_wgs84.inverse();
        let (x, y, z) = (x + back.dx, y + back.dy, z + back.dz);
        let (lat, lon, _h) = ecef_to_geodetic(self.projection.ellipsoid, x, y, z);
        self.projection.forward(lat, lon)
    }
}
