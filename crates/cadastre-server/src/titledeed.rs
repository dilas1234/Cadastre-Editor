//! The title deed plan (Generate Title Deed Plan, task 12 of Deed Plan Creationv2): one A4 sheet
//! laid out as PLAGIS's Aumentum plans (`Plan_.mxd`), modelled on a printed plan of 2026 (PL68061):
//!
//! * header: region / location / plan no. / tracing no. / scale + scale bar (left), north arrow
//!   (centre), title no. / address / L.G. area / area and UPIN in red (right), holder (centre);
//! * frame: the parcel to scale, a cross at each beacon with its number, on each side the bearing
//!   (degrees at one end, minutes at the other) and the distance, the file no. and area in the
//!   middle, an optional access road (dashed, parallel to one side), grid values at the frame;
//! * footer: WGS 1984 co-ordinates of the first beacon, surveyed by / survey date / sheet no. /
//!   prepared by (left), the computation note and plotted / checked by (centre), the
//!   Surveyor-General's signature and date lines (right).
//!
//! Legs run clockwise from the westernmost beacon, as on the printed plans; bearings are rounded to
//! the minute, distances to the centimetre, the area is given to the square millimetre.
//! Pure: [`render_svg`] takes the plan's data and returns SVG (mm units, `viewBox 0 0 210 297`),
//! so it is unit-tested without a database. The editor prints it and turns it into the JPEG that
//! Send plan files with the Registry transaction.

use std::fmt::Write;

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// One corner of the parcel, in the plan's (clockwise) order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct Corner {
    /// Beacon number as printed (from the fabric, or typed by the officer).
    pub name: String,
    pub e: f64,
    pub n: f64,
}

/// What the officer confirms or types on the plan form (saved per job). Every field is optional:
/// a blank one is printed blank.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(default)]
pub struct PlanFields {
    pub region: Option<String>,
    pub location: Option<String>,
    pub plan_no: Option<String>,
    pub tracing_no: Option<String>,
    pub title_no: Option<String>,
    pub address: Option<String>,
    pub lga: Option<String>,
    pub holder: Option<String>,
    pub surveyed_by: Option<String>,
    pub survey_date: Option<String>,
    pub plotted_by: Option<String>,
    pub checked_by: Option<String>,
    pub surveyor_general: Option<String>,
    pub prepared_by: Option<String>,
    /// Scale denominator (250 = 1:250); `None` = the largest standard scale that fits.
    pub scale: Option<u32>,
    /// Corner names typed over the fabric's, by corner index in the plan's order.
    pub corner_names: Vec<Option<String>>,
    /// Side (index in the plan's order: side k runs from corner k to corner k+1) the access road
    /// runs along, if any, and its label.
    pub road_side: Option<usize>,
    pub road_label: Option<String>,
}

/// Everything the sheet shows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct PlanData {
    pub upin: String,
    /// Area in m² (the parcel's computed area).
    pub area_m2: f64,
    /// Corners in the plan's order (clockwise from the westernmost).
    pub corners: Vec<Corner>,
    pub fields: PlanFields,
}

/// Standard plan scales, largest first.
pub const SCALES: [u32; 14] = [250, 500, 750, 1000, 1250, 1500, 2000, 2500, 3000, 4000, 5000, 7500, 10000, 25000];

// Sheet geometry (mm).
const W: f64 = 210.0;
const FRAME: (f64, f64, f64, f64) = (12.0, 60.0, 198.0, 228.0); // x0, y0, x1, y1
const RED: &str = "#c00000";
const LINE: &str = "#8b2f2f";
/// A white outline under labels that may cross a line (beacon numbers, bearings, the road).
const HALO: &str = r##"stroke="#fff" stroke-width="0.7" paint-order="stroke""##;

/// Put a ring in the plan's order: clockwise, starting at the westernmost corner (south-most on a
/// tie). A closing vertex equal to the first, and repeated vertices, are dropped.
pub fn plan_order(ring: &[(f64, f64, String)]) -> Vec<Corner> {
    let mut pts: Vec<Corner> = Vec::new();
    for (e, n, name) in ring {
        if pts.last().is_some_and(|p| (p.e - e).abs() < 1e-6 && (p.n - n).abs() < 1e-6) {
            continue;
        }
        pts.push(Corner { name: name.clone(), e: *e, n: *n });
    }
    if pts.len() > 1 && (pts[0].e - pts[pts.len() - 1].e).abs() < 1e-6 && (pts[0].n - pts[pts.len() - 1].n).abs() < 1e-6 {
        pts.pop();
    }
    // Shoelace in (E, N): positive = anticlockwise.
    let signed: f64 = (0..pts.len()).map(|i| { let (a, b) = (&pts[i], &pts[(i + 1) % pts.len()]); a.e * b.n - b.e * a.n }).sum();
    if signed > 0.0 {
        pts.reverse();
    }
    if let Some(start) = (0..pts.len()).min_by(|&i, &j| pts[i].e.total_cmp(&pts[j].e).then(pts[i].n.total_cmp(&pts[j].n))) {
        pts.rotate_left(start);
    }
    pts
}

/// The largest standard scale at which the corners fit the frame (with room for the labels).
pub fn fit_scale(corners: &[Corner]) -> u32 {
    let (minx, maxx) = corners.iter().fold((f64::MAX, f64::MIN), |(a, b), c| (a.min(c.e), b.max(c.e)));
    let (miny, maxy) = corners.iter().fold((f64::MAX, f64::MIN), |(a, b), c| (a.min(c.n), b.max(c.n)));
    let (w, h) = ((maxx - minx).max(0.01), (maxy - miny).max(0.01));
    let (fw, fh) = ((FRAME.2 - FRAME.0) * 0.7, (FRAME.3 - FRAME.1) * 0.7);
    *SCALES.iter().find(|&&s| w * 1000.0 / s as f64 <= fw && h * 1000.0 / s as f64 <= fh).unwrap_or(SCALES.last().unwrap())
}

/// Sheet number: the thousands of the Easting and Northing of a point (484903 E, 1082927 N → `484-1082`).
pub fn sheet_no(e: f64, n: f64) -> String {
    format!("{}-{}", (e / 1000.0).floor() as i64, (n / 1000.0).floor() as i64)
}

/// Whole-circle bearing of a leg, rounded to the minute: (degrees, minutes).
pub fn bearing_dm(from: &Corner, to: &Corner) -> (u32, u32) {
    let mut az = (to.e - from.e).atan2(to.n - from.n).to_degrees();
    if az < 0.0 {
        az += 360.0;
    }
    let total = (az * 60.0).round() as u32 % (360 * 60);
    (total / 60, total % 60)
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn up(s: &Option<String>) -> String {
    esc(&s.clone().unwrap_or_default().to_uppercase())
}

fn text(svg: &mut String, x: f64, y: f64, size: f64, anchor: &str, extra: &str, s: &str) {
    let _ = writeln!(svg, r#"<text x="{x:.2}" y="{y:.2}" font-size="{size}" text-anchor="{anchor}" {extra}>{s}</text>"#);
}

/// Text along a line from `a` to `b` (paper mm), at fraction `t`, offset `off` mm to the left of
/// the direction a→b, kept upright.
fn along(svg: &mut String, a: (f64, f64), b: (f64, f64), t: f64, off: f64, size: f64, s: &str) {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = (dx * dx + dy * dy).sqrt().max(1e-9);
    let (ux, uy) = (dx / len, dy / len);
    // Left normal in paper coordinates (y down).
    let (nx, ny) = (uy, -ux);
    let (x, y) = (a.0 + dx * t + nx * off, a.1 + dy * t + ny * off);
    let mut ang = dy.atan2(dx).to_degrees();
    if ang > 90.0 {
        ang -= 180.0;
    } else if ang <= -90.0 {
        ang += 180.0;
    }
    let _ = writeln!(svg, r##"<text x="{x:.2}" y="{y:.2}" font-size="{size}" text-anchor="middle" dominant-baseline="middle" stroke="#fff" stroke-width="0.7" paint-order="stroke" transform="rotate({ang:.2} {x:.2} {y:.2})">{}</text>"##, esc(s));
}

/// The sheet as SVG.
pub fn render_svg(d: &PlanData) -> String {
    let f = &d.fields;
    let corners: Vec<Corner> = d.corners.iter().enumerate().map(|(i, c)| {
        let name = f.corner_names.get(i).cloned().flatten().filter(|s| !s.trim().is_empty()).unwrap_or_else(|| c.name.clone());
        Corner { name, ..c.clone() }
    }).collect();
    let scale = f.scale.unwrap_or_else(|| fit_scale(&corners));
    let k = 1000.0 / scale as f64; // mm on paper per metre on the ground
    let (minx, maxx) = corners.iter().fold((f64::MAX, f64::MIN), |(a, b), c| (a.min(c.e), b.max(c.e)));
    let (miny, maxy) = corners.iter().fold((f64::MAX, f64::MIN), |(a, b), c| (a.min(c.n), b.max(c.n)));
    let (ce, cn) = ((minx + maxx) / 2.0, (miny + maxy) / 2.0);
    let (fx, fy) = ((FRAME.0 + FRAME.2) / 2.0, (FRAME.1 + FRAME.3) / 2.0);
    let paper = |e: f64, n: f64| (fx + (e - ce) * k, fy - (n - cn) * k);

    let mut s = String::new();
    let _ = writeln!(s, r#"<svg xmlns="http://www.w3.org/2000/svg" width="210mm" height="297mm" viewBox="0 0 210 297" font-family="Arial, Helvetica, sans-serif">"#);
    let _ = writeln!(s, r##"<rect width="210" height="297" fill="#fff"/>"##);

    // Header, left.
    let lx = 50.0;
    let region = if f.region.as_deref().unwrap_or("").is_empty() { "PLATEAU".to_string() } else { up(&f.region) };
    text(&mut s, lx, 16.0, 3.2, "middle", r#"font-weight="bold""#, &region);
    text(&mut s, lx, 21.0, 3.2, "middle", r#"font-weight="bold""#, &up(&f.location));
    text(&mut s, lx, 26.0, 3.2, "middle", r#"font-weight="bold""#, &format!("PLAN NO. {}", up(&f.plan_no)));
    text(&mut s, lx, 31.0, 3.2, "middle", r#"font-weight="bold""#, &format!("TRACING NO. {}", up(&f.tracing_no)));
    text(&mut s, lx, 36.0, 3.2, "middle", r#"font-weight="bold""#, &format!("Scale = 1:{scale}"));
    // Scale bar: a round ground length near 40 mm on paper.
    let bar_m = [1.0, 2.0, 5.0, 10.0, 20.0, 25.0, 50.0, 100.0, 200.0, 250.0, 500.0, 1000.0, 2000.0, 5000.0]
        .into_iter().rev().find(|m| m * k <= 40.0).unwrap_or(1.0);
    let (bx, by, bw) = (lx - bar_m * k / 2.0, 39.5, bar_m * k);
    for i in 0..4 {
        let fill = if i % 2 == 0 { "#000" } else { "#fff" };
        let _ = writeln!(s, r##"<rect x="{:.2}" y="{by}" width="{:.2}" height="1.2" fill="{fill}" stroke="#000" stroke-width="0.2"/>"##, bx + bw * i as f64 / 4.0, bw / 4.0);
    }
    text(&mut s, bx, by + 4.2, 2.2, "middle", "", "0");
    text(&mut s, bx + bw, by + 4.2, 2.2, "middle", "", &format!("{} m", bar_m));

    // North arrow.
    let _ = writeln!(s, r##"<line x1="105" y1="14" x2="105" y2="36" stroke="#000" stroke-width="0.35"/><line x1="99" y1="27" x2="111" y2="27" stroke="#000" stroke-width="0.35"/><path d="M105 12 L103 17 L107 17 Z" fill="#000"/>"##);
    text(&mut s, 105.0, 11.0, 3.0, "middle", r#"font-weight="bold""#, "N");

    // Header, right.
    let rx = 160.0;
    let title_no = if f.title_no.as_deref().unwrap_or("").is_empty() { up(&f.plan_no) } else { up(&f.title_no) };
    text(&mut s, rx, 17.0, 4.6, "middle", r#"font-weight="bold""#, &format!("TITLE NO. {title_no}"));
    text(&mut s, rx, 23.0, 3.0, "middle", r#"font-weight="bold""#, &up(&f.address));
    let lga = up(&f.lga);
    text(&mut s, rx, 28.0, 3.0, "middle", r#"font-weight="bold""#, &if lga.is_empty() { String::new() } else { format!("{lga} L.G. AREA") });
    text(&mut s, rx, 33.0, 3.0, "middle", &format!(r#"font-weight="bold" fill="{RED}""#), &format!("AREA {} SQUARE METERS", area2(d.area_m2)));
    text(&mut s, rx, 38.0, 3.0, "middle", &format!(r#"font-weight="bold" fill="{RED}""#), &format!("UPIN {}", esc(&d.upin)));
    text(&mut s, 105.0, 52.0, 3.4, "middle", r#"font-weight="bold""#, &format!("HOLDER: {}", up(&f.holder)));

    // Frame and grid values (the ground co-ordinates of its lower-left corner).
    let (x0, y0, x1, y1) = FRAME;
    let _ = writeln!(s, r##"<rect x="{x0}" y="{y0}" width="{}" height="{}" fill="none" stroke="#1f3f8f" stroke-width="0.5"/>"##, x1 - x0, y1 - y0);
    let e_left = ce + (x0 - fx) / k;
    let n_bottom = cn - (y1 - fy) / k;
    text(&mut s, x0, y1 + 3.0, 2.0, "start", "", &format!("{:.0}", e_left));
    let _ = writeln!(s, r#"<text x="{:.2}" y="{:.2}" font-size="2" text-anchor="start" transform="rotate(-90 {:.2} {:.2})">{:.0}</text>"#, x1 + 3.0, y1, x1 + 3.0, y1, n_bottom);

    // Access road: two dashed lines parallel to the chosen side, outside the parcel.
    let p: Vec<(f64, f64)> = corners.iter().map(|c| paper(c.e, c.n)).collect();
    let (pcx, pcy) = (p.iter().map(|q| q.0).sum::<f64>() / p.len().max(1) as f64, p.iter().map(|q| q.1).sum::<f64>() / p.len().max(1) as f64);
    if let Some(side) = f.road_side.filter(|&i| i < p.len() && p.len() >= 3) {
        let (a, b) = (p[side], p[(side + 1) % p.len()]);
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let len = (dx * dx + dy * dy).sqrt().max(1e-9);
        let (ux, uy) = (dx / len, dy / len);
        let (mut nx, mut ny) = (uy, -ux);
        let (mx, my) = ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
        if (mx + nx - pcx).powi(2) + (my + ny - pcy).powi(2) < (mx - pcx).powi(2) + (my - pcy).powi(2) {
            nx = -nx; ny = -ny; // point away from the parcel
        }
        let ext = len * 0.35;
        for off in [5.0, 13.0] {
            let (sx, sy) = (a.0 - ux * ext + nx * off, a.1 - uy * ext + ny * off);
            let (ex, ey) = (b.0 + ux * ext + nx * off, b.1 + uy * ext + ny * off);
            let _ = writeln!(s, r##"<line x1="{sx:.2}" y1="{sy:.2}" x2="{ex:.2}" y2="{ey:.2}" stroke="#000" stroke-width="0.3" stroke-dasharray="2.5 1.5"/>"##);
        }
        let label = f.road_label.clone().filter(|l| !l.trim().is_empty()).unwrap_or_else(|| "Access Road".into());
        let (la, lb) = ((a.0 + nx * 9.0, a.1 + ny * 9.0), (b.0 + nx * 9.0, b.1 + ny * 9.0));
        along(&mut s, la, lb, 0.5, 0.0, 2.8, &label);
    }

    // Parcel, beacons, bearings and distances.
    if p.len() >= 2 {
        let pts: String = p.iter().map(|q| format!("{:.2},{:.2}", q.0, q.1)).collect::<Vec<_>>().join(" ");
        let _ = writeln!(s, r#"<polygon points="{pts}" fill="none" stroke="{LINE}" stroke-width="0.45"/>"#);
    }
    for i in 0..p.len() {
        let (a, b) = (p[i], p[(i + 1) % p.len()]);
        if p.len() < 2 || (i == p.len() - 1 && p.len() == 2) {
            break;
        }
        let (deg, min) = bearing_dm(&corners[i], &corners[(i + 1) % corners.len()]);
        let dist = ((corners[(i + 1) % corners.len()].e - corners[i].e).powi(2) + (corners[(i + 1) % corners.len()].n - corners[i].n).powi(2)).sqrt();
        // Outside the parcel = away from its centre.
        let (mx, my) = ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let len = (dx * dx + dy * dy).sqrt().max(1e-9);
        let left_out = ((mx + dy / len) - pcx).powi(2) + ((my - dx / len) - pcy).powi(2) > (mx - pcx).powi(2) + (my - pcy).powi(2);
        let out = if left_out { 2.2 } else { -2.2 };
        // As on the printed plans: degrees at the end where the (upright) text starts reading,
        // minutes at the other end.
        let reads_from_a = if dx.abs() > 1e-6 { dx > 0.0 } else { a.1 < b.1 };
        let (t_deg, t_min) = if reads_from_a { (0.2, 0.8) } else { (0.8, 0.2) };
        along(&mut s, a, b, t_deg, out, 2.6, &format!("{deg}°"));
        along(&mut s, a, b, t_min, out, 2.6, &format!("{min:02}'"));
        along(&mut s, a, b, 0.5, -out, 2.6, &format!("{dist:.2}m"));
    }
    for (i, q) in p.iter().enumerate() {
        let _ = writeln!(s, r##"<path d="M{:.2} {:.2} l2 2 M{:.2} {:.2} l-2 2" stroke="#000" stroke-width="0.3" transform="translate(-1 -1)"/>"##, q.0 - 0.0, q.1, q.0 + 2.0, q.1);
        let (vx, vy) = (q.0 - pcx, q.1 - pcy);
        let vl = (vx * vx + vy * vy).sqrt().max(1e-9);
        let (tx, ty) = (q.0 + vx / vl * 5.0, q.1 + vy / vl * 4.0 + 1.0);
        let anchor = if vx < -0.5 { "end" } else if vx > 0.5 { "start" } else { "middle" };
        text(&mut s, tx, ty, 2.7, anchor, HALO, &esc(&corners[i].name));
    }
    let file_no = if f.plan_no.as_deref().unwrap_or("").is_empty() { title_no.clone() } else { up(&f.plan_no) };
    text(&mut s, pcx, pcy - 0.8, 2.6, "middle", "", &file_no);
    text(&mut s, pcx, pcy + 2.8, 2.6, "middle", "", &format!("{} Sq.m", area2(d.area_m2)));

    // Footer.
    let fx0 = 12.0;
    if let Some(o) = corners.first() {
        text(&mut s, fx0, 236.0, 2.8, "start", &format!(r#"font-weight="bold" fill="{LINE}""#), &format!("WGS 1984 Co-ordinates of {}", esc(&o.name)));
        text(&mut s, fx0, 240.5, 2.8, "start", &format!(r#"font-weight="bold" fill="{LINE}""#), &format!("N: {:.3}m", o.n));
        text(&mut s, fx0, 245.0, 2.8, "start", &format!(r#"font-weight="bold" fill="{LINE}""#), &format!("E: {:.3}m", o.e));
    }
    text(&mut s, fx0, 251.0, 2.8, "start", r#"font-weight="bold""#, &format!("SURVEYED BY: {}", esc(f.surveyed_by.as_deref().unwrap_or(""))));
    text(&mut s, fx0, 256.0, 2.8, "start", r#"font-weight="bold""#, &format!("SURVEY DATE: {}", esc(f.survey_date.as_deref().unwrap_or(""))));
    if let Some(o) = corners.first() {
        text(&mut s, fx0, 261.0, 2.8, "start", r#"font-weight="bold""#, &format!("SHEET NO. {}", sheet_no(o.e, o.n).replace('-', " - ")));
    }
    let prepared = f.prepared_by.clone().filter(|p| !p.trim().is_empty()).unwrap_or_else(|| "MLSTP".into());
    text(&mut s, fx0, 266.0, 2.8, "start", r#"font-weight="bold""#, &format!("Prepared By {}", esc(&prepared)));
    text(&mut s, 92.0, 238.0, 2.5, "middle", "", "All bearings and distances shown on this plan");
    text(&mut s, 92.0, 241.5, 2.5, "middle", "", "have been computed from Registered Co-ordinates.");
    text(&mut s, 92.0, 245.0, 2.5, "middle", "", "Roads shown are not as a result of accurate survey");
    text(&mut s, 92.0, 255.0, 2.8, "middle", r#"font-weight="bold""#, &format!("PLOTTED BY: {}", up(&f.plotted_by)));
    text(&mut s, 92.0, 264.0, 2.8, "middle", r#"font-weight="bold""#, &format!("CHECKED BY: {}", up(&f.checked_by)));
    let sx = 168.0;
    let _ = writeln!(s, r##"<line x1="140" y1="247" x2="196" y2="247" stroke="#000" stroke-width="0.25" stroke-dasharray="1 0.8"/>"##);
    text(&mut s, sx, 251.0, 2.8, "middle", r#"font-weight="bold""#, &up(&f.surveyor_general));
    text(&mut s, sx, 255.0, 2.8, "middle", r#"font-weight="bold""#, "SURVEYOR-GENERAL");
    let _ = writeln!(s, r##"<line x1="140" y1="262" x2="196" y2="262" stroke="#000" stroke-width="0.25" stroke-dasharray="1 0.8"/>"##);
    text(&mut s, sx, 266.0, 2.8, "middle", r#"font-weight="bold""#, "DATE");
    let _ = W;
    s.push_str("</svg>\n");
    s
}

/// The plan's area: cut to 2 decimals, never rounded up (903.198 → "903.19"), as the parcel
/// attribute record stores it (`trunc(ST_Area, 2)`). The small epsilon keeps 903.19 itself from
/// becoming 903.18 through floating-point noise.
pub fn area2(m2: f64) -> String {
    format!("{:.2}", (m2 * 100.0 + 1e-7).floor() / 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// PL68061 as stored (anticlockwise, starting at Q0210M), from the PLAGIS data.
    fn pl68061() -> Vec<(f64, f64, String)> {
        vec![
            (484925.296, 1082934.628, "P1".into()),
            (484911.537, 1082941.613, "Q0209M".into()),
            (484903.293, 1082927.804, "P3".into()),
            (484917.027, 1082920.638, "Q0208M".into()),
            (484925.296, 1082934.628, "P1".into()),
        ]
    }

    #[test]
    fn plan_order_is_clockwise_from_the_westernmost_beacon() {
        let c = plan_order(&pl68061());
        let names: Vec<&str> = c.iter().map(|c| c.name.as_str()).collect();
        // H7255M (stored as P3) is the westernmost; clockwise: H7255M, Q0209M, Q0210M, Q0208M.
        assert_eq!(names, vec!["P3", "Q0209M", "P1", "Q0208M"]);
    }

    #[test]
    fn bearings_and_distances_match_the_printed_plan() {
        let c = plan_order(&pl68061());
        // The printed plan of PL68061: 30°50' 16.08m, 11?°55' 15.43m, 210°35' 16.25m, 297°33' 15.49m.
        let legs: Vec<((u32, u32), String)> = (0..4).map(|i| {
            let (a, b) = (&c[i], &c[(i + 1) % 4]);
            (bearing_dm(a, b), format!("{:.2}", ((b.e - a.e).powi(2) + (b.n - a.n).powi(2)).sqrt()))
        }).collect();
        assert_eq!(legs[0], ((30, 50), "16.08".into()));
        assert_eq!(legs[1], ((116, 55), "15.43".into()));
        assert_eq!(legs[2], ((210, 35), "16.25".into()));
        assert_eq!(legs[3], ((297, 33), "15.49".into()));
    }

    #[test]
    fn scale_sheet_and_svg() {
        let c = plan_order(&pl68061());
        assert_eq!(fit_scale(&c), 250, "a 249 m² parcel is printed at 1:250, as PL68061");
        assert_eq!(sheet_no(484903.293, 1082927.804), "484-1082");
        let mut fields = PlanFields { plan_no: Some("PL68061".into()), holder: Some("Sa'ad Abubakar".into()), lga: Some("Jos South".into()),
            road_side: Some(3), corner_names: vec![Some("H7255M".into()), None, Some("Q0210M".into())], ..Default::default() };
        let svg = render_svg(&PlanData { upin: "PL000000023268".into(), area_m2: 249.489, corners: c.clone(), fields: fields.clone() });
        for want in ["TITLE NO. PL68061", "HOLDER: SA&#39;AD ABUBAKAR", "JOS SOUTH L.G. AREA", "AREA 249.48 SQUARE METERS", "UPIN PL000000023268",
                     "Scale = 1:250", "WGS 1984 Co-ordinates of H7255M", "N: 1082927.804m", "E: 484903.293m", "SHEET NO. 484 - 1082",
                     ">Q0210M<", ">Q0209M<", ">30°<", ">50'<", ">16.08m<", "Access Road", "249.48 Sq.m", "SURVEYOR-GENERAL"] {
            let want = want.replace("&#39;", "'");
            assert!(svg.contains(&want), "missing {want}");
        }
        // An explicit scale is kept.
        fields.scale = Some(750);
        let svg = render_svg(&PlanData { upin: "PL000000023268".into(), area_m2: 249.489, corners: c, fields });
        assert!(svg.contains("Scale = 1:750"));
    }

    #[test]
    fn plan_area_is_cut_not_rounded() {
        assert_eq!(super::area2(903.1976520011156), "903.19");
        assert_eq!(super::area2(249.489), "249.48");
        assert_eq!(super::area2(903.19), "903.19");
        assert_eq!(super::area2(100.0), "100.00");
    }
}
