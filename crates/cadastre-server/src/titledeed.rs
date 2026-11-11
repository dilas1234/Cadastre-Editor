//! The title deed plan (Generate Title Deed Plan, task 12 of Deed Plan Creationv2): one A4 sheet
//! laid out as PLAGIS's Aumentum plans (`Plan_.mxd`), modelled on a printed plan of 2026 (PL68061):
//!
//! * header: region / location / plan no. / tracing no. / scale + scale bar (left), north arrow
//!   (centre), title no. / address / L.G. area / area and UPIN in red (right), holder (centre);
//! * frame: the parcel to scale, a cross at each beacon with its number, on each side the bearing
//!   (degrees at one end, minutes at the other) and the distance, the file no. and area in the
//!   middle, access roads (dashed, parallel to chosen sides), grid values at the frame's lower
//!   corners and a tick on its edges every 100 mm of paper (so the ground step follows the scale);
//!   features from the symbol list (roads, water, drainage, point features), drawn on the map or
//!   tied to a side, clipped to the frame;
//! * footer: WGS 1984 co-ordinates of the first beacon, surveyed by / survey date / sheet no. /
//!   prepared by (left), the computation note and plotted / checked by (centre), the
//!   Surveyor-General's signature and date lines (right).
//!
//! Legs run clockwise from the westernmost beacon, as on the printed plans; bearings are rounded to
//! the minute, distances to the centimetre, the area is given to the square millimetre.
//! Pure: [`render_svg`] takes the plan's data and returns SVG (mm units, `viewBox 0 0 210 297`),
//! so it is unit-tested without a database. The editor prints it and turns it into the JPEG that
//! Send plan files with the Registry transaction.

use std::collections::BTreeMap;
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

/// A feature added to the plan: a symbol from [`SYMBOLS`], placed either along a side of the parcel
/// (`side`) or at the ground co-ordinates drawn on the map (`points`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(default)]
pub struct PlanFeature {
    /// The symbol's id in [`SYMBOLS`].
    pub symbol: String,
    /// Printed beside the feature, e.g. a river's name; blank = none.
    pub label: Option<String>,
    /// The side it runs along, outside the parcel (side k runs from corner k to corner k+1).
    pub side: Option<usize>,
    /// Drawn on the map: Easting and Northing of its points (one for a point symbol).
    pub points: Vec<[f64; 2]>,
    /// A line runs as a smooth curve through its points, rather than straight from one to the next.
    pub smooth: bool,
    /// The line or symbol's colour (`#rrggbb`); blank = the symbol's own.
    pub color: Option<String>,
    /// A double line: how far apart its two lines are, on paper (mm); blank = the symbol's own.
    pub width: Option<f64>,
}

/// Free text written on the plan.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(default)]
pub struct PlanNote {
    pub text: String,
    /// Where it starts: Easting and Northing.
    pub at: [f64; 2],
    /// `#rrggbb`; blank = black.
    pub color: Option<String>,
    /// Letter size on paper (mm); blank = `NOTE_SIZE`.
    pub size: Option<f64>,
    /// Turned this many degrees clockwise.
    pub angle: Option<f64>,
}

/// The officer's changes to something the sheet writes or draws itself, by its `data-move` key (or
/// `roadline:<side>` for the lines of an access road).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(default)]
pub struct LabelStyle {
    /// `#rrggbb`; blank = as the sheet has it.
    pub color: Option<String>,
    /// Words typed over the sheet's own: an access road's label, or an override of a computed or
    /// fixed label (an area, a distance, "SURVEYOR-GENERAL" …). Never the UPIN.
    pub text: Option<String>,
    /// For an override: what the sheet computed when it was typed (set by the server on save). If
    /// the co-ordinates change and the sheet computes something else, the override is stale.
    pub was: Option<String>,
    /// Turned this many degrees clockwise about where it is written. Only the writing turns.
    pub angle: Option<f64>,
}

/// One entry of the plan's symbol list.
pub struct Symbol {
    pub id: &'static str,
    pub label: &'static str,
    pub group: &'static str,
    /// Placed at one spot; the others are lines.
    pub point: bool,
    stroke: &'static str,
    /// Line width on paper (mm).
    width: f64,
    dash: &'static str,
    /// A white line down the middle this wide (mm): a double line.
    inner: f64,
    /// Drawn as two lines this far apart (mm), as an access road is; 0 = one line.
    pub double: f64,
}

impl Symbol {
    /// Its colour, unless a feature sets its own.
    pub fn stroke(&self) -> &'static str {
        if self.point { match self.id { "tree" => "#2e7d32", "well" => WATER, _ => "#000000" } } else { self.stroke }
    }
}

const fn line(id: &'static str, label: &'static str, group: &'static str, stroke: &'static str, width: f64, dash: &'static str, inner: f64) -> Symbol {
    Symbol { id, label, group, point: false, stroke, width, dash, inner, double: 0.0 }
}

const fn double(id: &'static str, label: &'static str, dash: &'static str) -> Symbol {
    Symbol { id, label, group: "Roads", point: false, stroke: "#000", width: 0.3, dash, inner: 0.0, double: ROAD_GAP }
}

const fn spot(id: &'static str, label: &'static str) -> Symbol {
    Symbol { id, label, group: "Point features", point: true, stroke: "#000", width: 0.0, dash: "", inner: 0.0, double: 0.0 }
}

const WATER: &str = "#1f6fb5";

/// The symbols of the plan form, in its order.
pub const SYMBOLS: [Symbol; 16] = [
    line("road", "Road", "Roads", "#000", 0.3, "2.5 1.5", 0.0),
    double("road_access", "Access road (two dashed lines)", "2.5 1.5"),
    double("road_double", "Road (two solid lines)", ""),
    line("road_unpaved", "Road, unpaved", "Roads", "#555", 0.25, "1.2 1.2", 0.0),
    line("road_proposed", "Road, proposed", "Roads", RED, 0.3, "", 0.0),
    line("footpath", "Footpath", "Roads", "#000", 0.2, "0.5 1", 0.0),
    line("river", "River", "Water", WATER, 0.6, "", 0.0),
    line("stream", "Stream or creek", "Water", WATER, 0.3, "", 0.0),
    line("stream_intermittent", "Stream, intermittent", "Water", WATER, 0.3, "2 0.8 0.5 0.8", 0.0),
    line("canal", "Canal", "Water", WATER, 0.9, "", 0.5),
    line("gutter", "Gutter or drain", "Drainage", "#444", 0.8, "", 0.4),
    line("culvert", "Culvert", "Drainage", "#444", 0.6, "1 1", 0.0),
    spot("tree", "Tree"),
    spot("pole", "Electric pole"),
    spot("well", "Well or borehole"),
    spot("building_corner", "Building corner"),
];

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
    /// Legacy single access road side (forms saved before `road_sides`); read when `road_sides` is
    /// empty.
    pub road_side: Option<usize>,
    /// Sides (index in the plan's order: side k runs from corner k to corner k+1) access roads run
    /// along. Two roads on neighbouring sides meet in a junction at their shared beacon.
    pub road_sides: Vec<usize>,
    pub road_label: Option<String>,
    /// The grid values and ticks on the frame (Easting / Northing, see `GRID_MM`). `None` = on.
    pub grid: Option<bool>,
    /// No longer used (the beacon of an earlier co-ordinate cross); kept so saved forms read.
    pub grid_corner: Option<usize>,
    /// Roads, water, drainage and point features added to the plan.
    pub features: Vec<PlanFeature>,
    /// Labels the officer dragged on the sheet: by label (`data-move` on the sheet), how far it
    /// moved on paper (mm, x right and y down). Only the writing moves, never a beacon.
    pub label_moves: BTreeMap<String, [f64; 2]>,
    /// Free text on the plan.
    pub notes: Vec<PlanNote>,
    /// Colours and wording the officer set on the sheet's own labels and access roads.
    pub styles: BTreeMap<String, LabelStyle>,
}

impl PlanFields {
    /// Plans saved when the middle's file no. and area were one label (`parcel`): its move and
    /// style go to both of the labels it became.
    pub fn split_parcel_block(&mut self) {
        if let Some(m) = self.label_moves.remove("parcel") {
            for k in ["file_no", "parcel_area"] {
                self.label_moves.entry(k.into()).or_insert(m);
            }
        }
        if let Some(st) = self.styles.remove("parcel") {
            for k in ["file_no", "parcel_area"] {
                self.styles.entry(k.into()).or_insert_with(|| LabelStyle { text: None, was: None, ..st.clone() });
            }
        }
    }

    /// The access road sides: `road_sides`, else the legacy `road_side`.
    pub fn roads(&self) -> Vec<usize> {
        let mut v = if self.road_sides.is_empty() { self.road_side.into_iter().collect() } else { self.road_sides.clone() };
        v.sort_unstable();
        v.dedup();
        v
    }
}

/// Capitalise the first letter of each word ("wiss resources" → "Wiss Resources",
/// "blessing j.wophill" → "Blessing J.Wophill"). Letters already in capitals stay; a letter after
/// an apostrophe is a word's continuation ("Sa'ad").
pub fn cap_words(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev: Option<char> = None;
    for c in s.chars() {
        let start = prev.is_none_or(|p| !(p.is_alphanumeric() || p == '\'' || p == '’'));
        if start && c.is_alphabetic() { out.extend(c.to_uppercase()) } else { out.push(c) }
        prev = Some(c);
    }
    out
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
/// The WGS 1984 co-ordinates in the footer: blue, as PLAGIS asked (2026-11-05).
const COORD_BLUE: &str = "#1f6fb5";
/// A white outline under labels that may cross a line (beacon numbers, bearings, the road).
const HALO: &str = r##"stroke="#fff" stroke-width="0.7" paint-order="stroke""##;
/// A side shorter than this on paper (mm) has no room for its bearing and distance: they are
/// written in the frame's lower-right corner instead ("PN6503 - PN6499 = 21° 16' @ 2.57m").
const SHORT_SIDE_MM: f64 = 12.0;
/// A feature tied to a side runs this far outside it (mm), and a second one on the same side this
/// much further out.
const FEATURE_OFF: f64 = 4.0;
const FEATURE_STEP: f64 = 2.5;
/// The corners of an access road T-junction are rounded, starting this far from the corner (mm).
const ROAD_FILLET: f64 = 6.0;
/// How far an access road's two lines are apart, and the inner one from the side (mm).
const ROAD_GAP: f64 = 8.0;
const ROAD_IN: f64 = 5.0;
const ROAD_OUT: f64 = ROAD_IN + ROAD_GAP;
/// Straight pieces per stretch of a smooth curve: finer than the eye sees at A4.
const CURVE_STEPS: usize = 16;
/// A note's letter size (mm), unless the officer sets one.
const NOTE_SIZE: f64 = 2.8;
/// Road junctions, on paper (mm). An end this close past another road is cut back to it...
const TRIM_MM: f64 = 12.0;
/// ...and one this close short of it is carried on to it.
const SNAP_MM: f64 = 3.0;
/// A road tied to a side runs this far past each end of the side, as an access road does, so that
/// roads on neighbouring sides reach each other.
const ROAD_EXT_MM: f64 = 6.0;
/// Beacon numbers: their font size, and the least distance between two that share a column (mm).
const BEACON_SIZE: f64 = 2.7;
const BEACON_LINE: f64 = 3.3;
/// The grid step on paper, from the frame's lower-left corner: on the ground this is the scale's
/// tenth in metres (50 m at 1:500, as PL65487's 484781 → 484831).
const GRID_MM: f64 = 100.0;

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

/// Sheet number: the thousands of the Easting and Northing of a point, each rounded down to an
/// even number (491176 E, 1092270 N → `490-1092`; 484903 E, 1082927 N → `484-1082`).
pub fn sheet_no(e: f64, n: f64) -> String {
    let part = |v: f64| { let k = (v / 1000.0).floor() as i64; k - k.rem_euclid(2) };
    format!("{}-{}", part(e), part(n))
}

/// Where line `p + t·u` meets line `q + s·v`: `t`, or `None` if they are (nearly) parallel.
fn meet(p: (f64, f64), u: (f64, f64), q: (f64, f64), v: (f64, f64)) -> Option<f64> {
    let den = u.0 * v.1 - u.1 * v.0;
    (den.abs() > 0.1).then(|| ((q.0 - p.0) * v.1 - (q.1 - p.1) * v.0) / den)
}

type Pt = (f64, f64);

/// Where segment a–b crosses segment c–d: the fraction along a–b.
fn seg_cross(a: Pt, b: Pt, c: Pt, d: Pt) -> Option<f64> {
    let (r, q) = ((b.0 - a.0, b.1 - a.1), (d.0 - c.0, d.1 - c.1));
    let den = r.0 * q.1 - r.1 * q.0;
    if den.abs() < 1e-9 {
        return None;
    }
    let t = ((c.0 - a.0) * q.1 - (c.1 - a.1) * q.0) / den;
    let u = ((c.0 - a.0) * r.1 - (c.1 - a.1) * r.0) / den;
    ((0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u)).then_some(t)
}

fn dist(a: Pt, b: Pt) -> f64 {
    (b.0 - a.0).hypot(b.1 - a.1)
}

fn length(line: &[Pt]) -> f64 {
    line.windows(2).map(|w| dist(w[0], w[1])).sum()
}

/// The T-junctions of the drawn roads: each end of a road stops at the road it meets. An end that
/// runs up to `TRIM_MM` past a road is cut back to it (the crossing nearest the road's body, when
/// it runs over both lines of an access road), and one up to `SNAP_MM` short is carried on to it.
/// `fixed` lines (access roads) always win; of two roads ending at each other the longer runs
/// through. A road that carries on further than that crosses, and is left alone.
fn trim_roads(roads: &mut [Vec<Pt>], fixed: &[[Pt; 2]]) {
    let all: Vec<Vec<Pt>> = roads.to_vec();
    // Whether a crossing at `x` on road `i` stops road `i` against road `j`.
    let yields = |i: usize, j: usize, x: Pt| {
        let (other, ends_there) = (&all[j], dist(x, all[j][0]).min(dist(x, all[j][all[j].len() - 1])) <= TRIM_MM);
        !ends_there || length(&all[i]) < length(other) || (length(&all[i]) == length(other) && i > j)
    };
    for i in 0..roads.len() {
        for _end in 0..2 {
            roads[i].reverse(); // both ends, and back to the way it was drawn
            let line = roads[i].clone();
            let n = line.len();
            let total = length(&line);
            // Walk back from the end: the crossing furthest from it, within reach.
            let mut cut: Option<(f64, usize, Pt)> = None;
            let mut walked = 0.0;
            for k in (0..n - 1).rev() {
                let (a, b) = (line[k], line[k + 1]);
                let seg = dist(a, b);
                let mut consider = |c: Pt, d: Pt, wins: &dyn Fn(Pt) -> bool| {
                    if let Some(t) = seg_cross(a, b, c, d) {
                        let x = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
                        let back = walked + seg * (1.0 - t);
                        if back > 1e-6 && back <= TRIM_MM && back < total / 2.0 && wins(x) && cut.is_none_or(|(best, _, _)| back > best) {
                            cut = Some((back, k, x));
                        }
                    }
                };
                for w in fixed {
                    consider(w[0], w[1], &|_| true);
                }
                for (j, other) in all.iter().enumerate().filter(|&(j, _)| j != i) {
                    for w in other.windows(2) {
                        consider(w[0], w[1], &|x| yields(i, j, x));
                    }
                }
                walked += seg;
                if walked > TRIM_MM {
                    break;
                }
            }
            if let Some((_, k, x)) = cut {
                roads[i].truncate(k + 1);
                roads[i].push(x);
                continue;
            }
            // Short of a road: carry the last stretch on, to the nearest within reach.
            let (a, b) = (line[n - 2], line[n - 1]);
            let seg = dist(a, b).max(1e-9);
            let far = (b.0 + (b.0 - a.0) / seg * SNAP_MM, b.1 + (b.1 - a.1) / seg * SNAP_MM);
            let targets = fixed.iter().map(|w| (w[0], w[1], None)).chain(all.iter().enumerate().filter(|&(j, _)| j != i)
                .flat_map(|(j, o)| o.windows(2).map(move |w| (w[0], w[1], Some(j)))));
            let hit = targets.filter_map(|(c, d, j)| {
                let t = seg_cross(b, far, c, d)?;
                let x = (b.0 + (far.0 - b.0) * t, b.1 + (far.1 - b.1) * t);
                j.is_none_or(|j| yields(i, j, x)).then_some((t, x))
            }).min_by(|x, y| x.0.total_cmp(&y.0));
            if let Some((_, x)) = hit {
                roads[i].push(x);
            }
        }
    }
}

/// An access road line from `o` along `u`, `t0`..`t1`, over a side `len` long: the stretch past the
/// side stops at a drawn road running across it (not at one that ends there, which stops at it).
fn stop_at_through(o: Pt, u: Pt, len: f64, (start, end): (f64, f64), roads: &[Vec<Pt>]) -> (f64, f64) {
    let (a, b) = ((o.0 + u.0 * start, o.1 + u.1 * start), (o.0 + u.0 * end, o.1 + u.1 * end));
    let (mut t0, mut t1) = (start, end);
    for road in roads {
        for w in road.windows(2) {
            let Some(f) = seg_cross(a, b, w[0], w[1]) else { continue };
            let x = (a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f);
            if dist(x, road[0]).min(dist(x, road[road.len() - 1])) <= TRIM_MM {
                continue;
            }
            let t = start + (end - start) * f;
            if t < 0.0 {
                t0 = t0.max(t);
            } else if t > len {
                t1 = t1.min(t);
            }
        }
    }
    (t0, t1)
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

/// A form value in capitals, not yet escaped.
fn upr(s: &Option<String>) -> String {
    s.clone().unwrap_or_default().to_uppercase()
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

/// A beacon number on paper: its left and right edges (estimated from its length) and baseline.
struct LabelBox {
    left: f64,
    right: f64,
    y: f64,
}

/// Move labels that would touch apart, up and down by the same amount, to a line's height (a few
/// passes, as a move can bring a label to a third). Labels that do not touch stay where they are.
fn spread_labels(boxes: &mut [LabelBox]) {
    for _ in 0..8 {
        let mut moved = false;
        for i in 0..boxes.len() {
            for j in i + 1..boxes.len() {
                let (head, tail) = boxes.split_at_mut(j);
                let (a, b) = (&mut head[i], &mut tail[0]);
                let gap = (a.y - b.y).abs();
                if a.left < b.right && b.left < a.right && gap < BEACON_LINE - 1e-6 {
                    let push = (BEACON_LINE - gap) / 2.0;
                    let (upper, lower) = if a.y <= b.y { (a, b) } else { (b, a) };
                    upper.y -= push;
                    lower.y += push;
                    moved = true;
                }
            }
        }
        if !moved {
            break;
        }
    }
}

/// Open a label's group, moved, turned and coloured as the officer set it; `data-move` names it
/// for the sheet's editor, `data-at` is the paper point it turns about. Close it with [`end_move`].
fn begin_move(svg: &mut String, f: &PlanFields, key: &str, at: Pt) {
    begin_move_with(svg, f, key, at, "");
}

/// [`begin_move`] with more attributes on the group (`attrs` starts with a space).
fn begin_move_with(svg: &mut String, f: &PlanFields, key: &str, at: Pt, attrs: &str) {
    let _ = write!(svg, r#"<g data-move="{key}"{attrs}"#);
    let style = f.styles.get(key);
    let shift = f.label_moves.get(key).map(|[dx, dy]| format!("translate({dx:.2} {dy:.2})"));
    let turn = style.and_then(|x| x.angle).filter(|a| a.is_finite() && *a != 0.0).map(|a| format!("rotate({a:.1} {:.2} {:.2})", at.0, at.1));
    let transform = [shift, turn].into_iter().flatten().collect::<Vec<_>>().join(" ");
    if !transform.is_empty() {
        let _ = write!(svg, r#" transform="{transform}""#);
    }
    if let Some(c) = safe_color(style.and_then(|x| x.color.as_deref())) {
        let _ = write!(svg, r#" fill="{c}""#);
    }
    let _ = writeln!(svg, r#" data-at="{:.2} {:.2}">"#, at.0, at.1);
}

fn end_move(svg: &mut String) {
    svg.push_str("</g>\n");
}

/// A label typed over what the sheet computes, as the plan prints it.
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub struct Override {
    /// The label (`distance:2`, `area`, `short:3:degrees` …).
    pub key: String,
    /// What the sheet prints: the officer's words.
    pub typed: String,
    /// What the sheet computes now.
    pub calculated: String,
    /// What it computed when the words were typed, if that is known.
    pub was: Option<String>,
    /// The co-ordinates have changed since: `was` is no longer what the sheet computes.
    pub stale: bool,
}

/// What the sheet computed while drawing: each overridable label's own words, the overrides it
/// printed, and whether a bearing or distance was typed over (the computation note then says so).
#[derive(Default)]
struct Calc {
    values: BTreeMap<String, String>,
    overrides: Vec<Override>,
    amended: bool,
}

/// What a label's words are.
enum Words {
    /// The officer's form field of that name: typed on the sheet, the field changes.
    Field(&'static str, String),
    /// Computed or fixed wording, which the officer may type over (an override).
    Calc(String),
    /// Always as computed (the UPIN).
    Locked(String),
}

/// The words a label prints, and the attributes that tell the sheet's editor what they are.
fn words_of(f: &PlanFields, calc: &mut Calc, key: &str, w: Words) -> (String, String) {
    match w {
        Words::Field(name, v) => (v, format!(r#" data-field="{name}""#)),
        Words::Locked(v) => (v, r#" data-locked="1""#.to_string()),
        Words::Calc(v) => {
            calc.values.insert(key.to_string(), v.clone());
            let st = f.styles.get(key);
            match st.and_then(|x| x.text.as_deref()).map(str::trim).filter(|t| !t.is_empty() && *t != v) {
                Some(t) => {
                    let was = st.and_then(|x| x.was.clone());
                    let stale = was.as_deref().is_some_and(|w| w != v);
                    calc.overrides.push(Override { key: key.to_string(), typed: t.to_string(), calculated: v.clone(), was, stale });
                    if key.starts_with("degrees:") || key.starts_with("minutes:") || key.starts_with("distance:") || key.starts_with("short:") {
                        calc.amended = true;
                    }
                    (t.to_string(), format!(r#" data-calc="{}" data-override="1"{}"#, esc(&v), if stale { r#" data-stale="1""# } else { "" }))
                }
                None => (v.clone(), format!(r#" data-calc="{}""#, esc(&v))),
            }
        }
    }
}

/// `extra` without its `fill`, for a label the officer has recoloured (its group carries the colour).
fn strip_fill(extra: &str) -> String {
    match extra.find(r#"fill=""#) {
        Some(i) => {
            let rest = &extra[i + 6..];
            let end = rest.find('"').map_or(extra.len(), |j| i + 6 + j + 1);
            format!("{}{}", &extra[..i], &extra[end..]).trim().to_string()
        }
        None => extra.to_string(),
    }
}

/// One label of the sheet: its own group (moved, turned, coloured, retyped as the officer set it)
/// holding its text at (`x`, `y`).
#[allow(clippy::too_many_arguments)]
fn put(svg: &mut String, f: &PlanFields, calc: &mut Calc, key: &str, w: Words, x: f64, y: f64, size: f64, anchor: &str, extra: &str) {
    let (words, attrs) = words_of(f, calc, key, w);
    let recoloured = safe_color(f.styles.get(key).and_then(|st| st.color.as_deref())).is_some();
    let extra = if recoloured { strip_fill(extra) } else { extra.to_string() };
    begin_move_with(svg, f, key, (x, y), &attrs);
    text(svg, x, y, size, anchor, &extra, &esc(&words));
    end_move(svg);
}

/// A side too short to carry its bearing and distance, written in the corner table instead.
struct ShortSide {
    from: String,
    to: String,
    /// Each value cell: the words it prints, and its attributes (`data-cell` names its override).
    degrees: (String, String),
    minutes: (String, String),
    distance: (String, String),
}

/// Width of `s` in Arial at `size` mm (its advance widths, in ems). Close enough to set columns:
/// the digits are all one width, so right-aligned numbers line up exactly whatever the estimate.
fn text_width(s: &str, size: f64) -> f64 {
    s.chars().map(|c| match c {
        '0'..='9' => 0.556,
        '.' | ',' | ' ' | ':' => 0.278,
        '\'' => 0.191,
        '°' => 0.4,
        '-' => 0.333,
        'm' | 'M' | 'W' => 0.833,
        'I' | 'i' | 'l' | 'j' => 0.25,
        c if c.is_ascii_uppercase() => 0.722,
        _ => 0.556,
    }).sum::<f64>() * size
}

/// The short sides as a table in the lower right of the frame, last side lowest:
/// `FROM - TO = 33° 41' @ 1.43m`, each part in its own column. Names are left-aligned, numbers
/// right-aligned (so the degrees end together and the distances' decimal points line up). The
/// whole table moves as one (`short`).
fn short_sides_table(svg: &mut String, f: &PlanFields, rows: &[ShortSide]) {
    if rows.is_empty() {
        return;
    }
    const SIZE: f64 = 2.6;
    const ROW: f64 = 4.0;
    const GAP: f64 = 1.0;
    let widest = |pick: fn(&ShortSide) -> &str| rows.iter().map(|r| text_width(pick(r), SIZE)).fold(0.0, f64::max);
    let (w_from, w_to) = (widest(|r| &r.from), widest(|r| &r.to));
    let (w_deg, w_min, w_dist) = (widest(|r| &r.degrees.0), widest(|r| &r.minutes.0), widest(|r| &r.distance.0));
    let (w_dash, w_eq, w_at) = (text_width("-", SIZE), text_width("=", SIZE).max(1.5), text_width("@", SIZE).max(2.6));
    // Column positions from the left edge of the table.
    let c_dash = w_from + GAP + w_dash / 2.0; // middle
    let c_to = c_dash + w_dash / 2.0 + GAP; // start
    let c_eq = c_to + w_to + GAP + w_eq / 2.0; // middle
    let c_deg = c_eq + w_eq / 2.0 + GAP + w_deg; // end
    let c_min = c_deg + GAP + w_min; // end
    let c_at = c_min + GAP + w_at / 2.0; // middle
    let c_dist = c_at + w_at / 2.0 + GAP + w_dist; // end
    let right = FRAME.2 - 2.0;
    let left = right - c_dist;
    let bottom = FRAME.3 - 2.5;
    let top = bottom - (rows.len() - 1) as f64 * ROW;
    begin_move(svg, f, "short", (left + c_dist / 2.0, (top + bottom) / 2.0));
    for (j, r) in rows.iter().rev().enumerate() {
        let y = bottom - j as f64 * ROW;
        for (x, anchor, cell, attrs) in [
            (left, "start", esc(&r.from), ""),
            (left + c_dash, "middle", "-".to_string(), ""),
            (left + c_to, "start", esc(&r.to), ""),
            (left + c_eq, "middle", "=".to_string(), ""),
            (left + c_deg, "end", esc(&r.degrees.0), r.degrees.1.as_str()),
            (left + c_min, "end", esc(&r.minutes.0), r.minutes.1.as_str()),
            (left + c_at, "middle", "@".to_string(), ""),
            (left + c_dist, "end", esc(&r.distance.0), r.distance.1.as_str()),
        ] {
            text(svg, x, y, SIZE, anchor, attrs, &cell);
        }
    }
    end_move(svg);
}

/// A colour as it may go on the sheet: `#rrggbb`, nothing else.
fn safe_color(c: Option<&str>) -> Option<&str> {
    c.map(str::trim).filter(|c| c.len() == 7 && c.starts_with('#') && c[1..].chars().all(|h| h.is_ascii_hexdigit()))
}

/// A smooth curve through `pts` (centripetal Catmull-Rom: it passes through every point and does
/// not overshoot where they are unevenly spaced), as short straight pieces.
fn spline(pts: &[Pt]) -> Vec<Pt> {
    let mut p: Vec<Pt> = pts.to_vec();
    p.dedup_by(|a, b| dist(*a, *b) < 1e-6);
    if p.len() < 3 {
        return p;
    }
    let n = p.len();
    // Past each end, the end's reflection, so that the curve runs out straight.
    let get = |i: isize| -> Pt {
        if i < 0 {
            (2.0 * p[0].0 - p[1].0, 2.0 * p[0].1 - p[1].1)
        } else if i as usize >= n {
            (2.0 * p[n - 1].0 - p[n - 2].0, 2.0 * p[n - 1].1 - p[n - 2].1)
        } else {
            p[i as usize]
        }
    };
    let mix = |a: Pt, b: Pt, ta: f64, tb: f64, t: f64| {
        let w = (t - ta) / (tb - ta);
        (a.0 + (b.0 - a.0) * w, a.1 + (b.1 - a.1) * w)
    };
    let mut out = vec![p[0]];
    for i in 0..n - 1 {
        let (p0, p1, p2, p3) = (get(i as isize - 1), p[i], p[i + 1], get(i as isize + 2));
        let (t0, t1) = (0.0, dist(p0, p1).sqrt());
        let t2 = t1 + dist(p1, p2).sqrt();
        let t3 = t2 + dist(p2, p3).sqrt();
        for step in 1..=CURVE_STEPS {
            let t = t1 + (t2 - t1) * step as f64 / CURVE_STEPS as f64;
            let (a1, a2, a3) = (mix(p0, p1, t0, t1, t), mix(p1, p2, t1, t2, t), mix(p2, p3, t2, t3, t));
            out.push(mix(mix(a1, a2, t0, t2, t), mix(a2, a3, t1, t3, t), t1, t2, t));
        }
    }
    out
}

/// A line `off` mm to the left of `pts` (paper, y down), mitred at its bends.
fn offset_line(pts: &[Pt], off: f64) -> Vec<Pt> {
    let normal = |a: Pt, b: Pt| { let l = dist(a, b).max(1e-9); ((b.1 - a.1) / l, -(b.0 - a.0) / l) };
    (0..pts.len()).map(|i| {
        let before = (i > 0).then(|| normal(pts[i - 1], pts[i]));
        let after = (i + 1 < pts.len()).then(|| normal(pts[i], pts[i + 1]));
        let (a, b) = (before.or(after).unwrap_or((0.0, 0.0)), after.or(before).unwrap_or((0.0, 0.0)));
        let (mx, my) = (a.0 + b.0, a.1 + b.1);
        let l = (mx * mx + my * my).sqrt().max(1e-9);
        let (mx, my) = (mx / l, my / l);
        let stretch = 1.0 / (mx * a.0 + my * a.1).max(0.5); // a sharp bend keeps its width, up to double
        (pts[i].0 + mx * off * stretch, pts[i].1 + my * off * stretch)
    }).collect()
}

/// The point halfway along a line and the stretch it lies on.
fn middle(pts: &[Pt]) -> Option<(Pt, Pt)> {
    let mut left = length(pts) / 2.0;
    for w in pts.windows(2) {
        let seg = dist(w[0], w[1]);
        if seg >= left && seg > 0.0 {
            return Some((w[0], w[1]));
        }
        left -= seg;
    }
    None
}

/// Paper points as an SVG attribute.
fn pts_attr(pts: &[Pt]) -> String {
    pts.iter().map(|q| format!("{:.2},{:.2}", q.0, q.1)).collect::<Vec<_>>().join(" ")
}

/// A line symbol along paper points.
fn feature_line(svg: &mut String, sym: &Symbol, pts: &[(f64, f64)], color: Option<&str>) {
    let d = pts_attr(pts);
    let dash = if sym.dash.is_empty() { String::new() } else { format!(r#" stroke-dasharray="{}""#, sym.dash) };
    let _ = writeln!(svg, r#"<polyline points="{d}" fill="none" stroke="{}" stroke-width="{}" stroke-linejoin="round"{dash}/>"#, color.unwrap_or(sym.stroke), sym.width);
    if sym.inner > 0.0 {
        let _ = writeln!(svg, r##"<polyline points="{d}" fill="none" stroke="#fff" stroke-width="{}" stroke-linejoin="round"/>"##, sym.inner);
    }
}

/// A point symbol at a paper point.
fn feature_point(svg: &mut String, sym: &Symbol, q: (f64, f64), color: Option<&str>) {
    let glyph = match sym.id {
        "tree" => { let c = color.unwrap_or("#2e7d32"); format!(r##"<g stroke="{c}" stroke-width="0.25" fill="none"><circle r="1.5"/><circle r="0.3" fill="{c}"/></g>"##) }
        "pole" => { let c = color.unwrap_or("#000"); format!(r##"<g stroke="{c}" stroke-width="0.25"><line x1="0" y1="-1.6" x2="0" y2="1.6"/><line x1="-1" y1="-1.6" x2="1" y2="-1.6"/><circle r="0.5" fill="{c}"/></g>"##) }
        "well" => { let c = color.unwrap_or(WATER); format!(r##"<g stroke="{c}" stroke-width="0.25" fill="none"><circle r="1.2"/><circle r="0.5" fill="{c}"/></g>"##) }
        _ => format!(r##"<path d="M-1.5 1.5 L-1.5 -1.5 L1.5 -1.5" fill="none" stroke="{}" stroke-width="0.35"/>"##, color.unwrap_or("#000")),
    };
    let _ = writeln!(svg, r#"<g transform="translate({:.2} {:.2})">{glyph}</g>"#, q.0, q.1);
}

/// The sheet as SVG.
pub fn render_svg(d: &PlanData) -> String {
    draw(d).0
}

/// The labels the sheet prints over what it computes (overrides), stale ones marked.
pub fn overrides(d: &PlanData) -> Vec<Override> {
    draw(d).1.overrides
}

/// What the sheet computes for each label that can be typed over, by label.
pub fn computed_labels(d: &PlanData) -> BTreeMap<String, String> {
    draw(d).1.values
}

fn draw(d: &PlanData) -> (String, Calc) {
    let mut fields = d.fields.clone();
    fields.split_parcel_block();
    let f = &fields;
    let mut calc = Calc::default();
    let cl = &mut calc;
    let corners: Vec<Corner> = d.corners.iter().enumerate().map(|(i, c)| {
        let name = f.corner_names.get(i).cloned().flatten().filter(|s| !s.trim().is_empty()).unwrap_or_else(|| c.name.clone());
        Corner { name, ..c.clone() }
    }).collect();
    let scale = f.scale.unwrap_or_else(|| fit_scale(&corners));
    let k = 1000.0 / scale as f64; // mm on paper per metre on the ground
    let (minx, maxx) = corners.iter().fold((f64::MAX, f64::MIN), |(a, b), c| (a.min(c.e), b.max(c.e)));
    let (miny, maxy) = corners.iter().fold((f64::MAX, f64::MIN), |(a, b), c| (a.min(c.n), b.max(c.n)));
    let (ce, cn) = ((minx + maxx) / 2.0, (miny + maxy) / 2.0);
    // The parcel is centred in the frame, to the nearest metre: the frame's lower-left corner sits
    // on a whole-metre Easting and Northing, so the grid values printed from it are exact.
    let (x0, y0, x1, y1) = FRAME;
    let e_left = (ce - (x1 - x0) / 2.0 / k).round();
    let n_bottom = (cn - (y1 - y0) / 2.0 / k).round();
    let paper = |e: f64, n: f64| (x0 + (e - e_left) * k, y1 - (n - n_bottom) * k);

    let mut s = String::new();
    let _ = writeln!(s, r#"<svg xmlns="http://www.w3.org/2000/svg" width="210mm" height="297mm" viewBox="0 0 210 297" font-family="Arial, Helvetica, sans-serif" data-grid="{e_left} {n_bottom} {k} {x0} {y1}">"#);
    let _ = writeln!(s, r##"<rect width="210" height="297" fill="#fff"/>"##);

    // Header, left.
    let lx = 50.0;
    const BOLD: &str = r#"font-weight="bold""#;
    let region = if f.region.as_deref().unwrap_or("").is_empty() { "PLATEAU".to_string() } else { upr(&f.region) };
    put(&mut s, f, cl, "region", Words::Field("region", region), lx, 16.0, 3.2, "middle", BOLD);
    put(&mut s, f, cl, "location", Words::Field("location", upr(&f.location)), lx, 21.0, 3.2, "middle", BOLD);
    put(&mut s, f, cl, "plan_no", Words::Field("plan_no", format!("PLAN NO. {}", upr(&f.plan_no))), lx, 26.0, 3.2, "middle", BOLD);
    put(&mut s, f, cl, "tracing_no", Words::Field("tracing_no", format!("TRACING NO. {}", upr(&f.tracing_no))), lx, 31.0, 3.2, "middle", BOLD);
    put(&mut s, f, cl, "scale", Words::Field("scale", format!("Scale = 1:{scale}")), lx, 36.0, 3.2, "middle", BOLD);
    // Scale bar: a round ground length near 40 mm on paper.
    let bar_m = [1.0, 2.0, 5.0, 10.0, 20.0, 25.0, 50.0, 100.0, 200.0, 250.0, 500.0, 1000.0, 2000.0, 5000.0]
        .into_iter().rev().find(|m| m * k <= 40.0).unwrap_or(1.0);
    let (bx, by, bw) = (lx - bar_m * k / 2.0, 39.5, bar_m * k);
    for i in 0..4 {
        let fill = if i % 2 == 0 { "#000" } else { "#fff" };
        let _ = writeln!(s, r##"<rect x="{:.2}" y="{by}" width="{:.2}" height="1.2" fill="{fill}" stroke="#000" stroke-width="0.2"/>"##, bx + bw * i as f64 / 4.0, bw / 4.0);
    }
    put(&mut s, f, cl, "bar:0", Words::Calc("0".into()), bx, by + 4.2, 2.2, "middle", "");
    put(&mut s, f, cl, "bar:len", Words::Calc(format!("{} m", bar_m)), bx + bw, by + 4.2, 2.2, "middle", "");

    // North arrow, top centre: the shaft doubled down to its cross, "U.N." either side of the tip.
    let _ = writeln!(s, r##"<g stroke="#000" stroke-width="0.3"><line x1="105" y1="7" x2="105" y2="46"/><line x1="104.4" y1="16.5" x2="104.4" y2="34.5"/><line x1="105.6" y1="16.5" x2="105.6" y2="34.5"/><line x1="97.5" y1="22" x2="112.5" y2="22"/><line x1="98.5" y1="23.2" x2="111.5" y2="23.2"/></g><path d="M105 6 L104 9 L106 9 Z" fill="#000"/>"##);
    put(&mut s, f, cl, "north:u", Words::Calc("U.".into()), 104.2, 14.0, 3.0, "end", BOLD);
    put(&mut s, f, cl, "north:n", Words::Calc("N.".into()), 105.8, 14.0, 3.0, "start", BOLD);

    // Header, right.
    let rx = 160.0;
    let title_no = if f.title_no.as_deref().unwrap_or("").is_empty() { upr(&f.plan_no) } else { upr(&f.title_no) };
    put(&mut s, f, cl, "title_no", Words::Field("title_no", format!("TITLE NO. {title_no}")), rx, 17.0, 4.6, "middle", BOLD);
    put(&mut s, f, cl, "address", Words::Field("address", upr(&f.address)), rx, 23.0, 3.0, "middle", BOLD);
    let lga = upr(&f.lga);
    put(&mut s, f, cl, "lga", Words::Field("lga", if lga.is_empty() { String::new() } else { format!("{lga} L.G. AREA") }), rx, 28.0, 3.0, "middle", BOLD);
    let red = format!(r#"font-weight="bold" fill="{RED}""#);
    put(&mut s, f, cl, "area", Words::Calc(area_line(d.area_m2)), rx, 33.0, 3.0, "middle", &red);
    put(&mut s, f, cl, "upin", Words::Locked(format!("UPIN {}", d.upin)), rx, 38.0, 3.0, "middle", &red);
    put(&mut s, f, cl, "holder", Words::Field("holder", format!("HOLDER: {}", upr(&f.holder))), 105.0, 52.0, 3.4, "middle", BOLD);

    // Frame and grid: as on the printed plans, the ground co-ordinates of the lower-left corner
    // (Easting under it, Northing beside the lower-right corner), then a tick on each edge every
    // `GRID_MM` of paper, labelled outside the frame, in the survey's co-ordinates (UTM).
    let _ = writeln!(s, r##"<rect x="{x0}" y="{y0}" width="{}" height="{}" fill="none" stroke="#1f3f8f" stroke-width="0.5"/>"##, x1 - x0, y1 - y0);
    if f.grid.unwrap_or(true) && !corners.is_empty() {
        let upward = |s: &mut String, cl: &mut Calc, key: &str, x: f64, y: f64, anchor: &str, label: String| {
            let (words, attrs) = words_of(f, cl, key, Words::Calc(label));
            begin_move_with(s, f, key, (x, y), &attrs);
            let _ = writeln!(s, r#"<text x="{x:.2}" y="{y:.2}" font-size="2" text-anchor="{anchor}" transform="rotate(-90 {x:.2} {y:.2})">{}</text>"#, esc(&words));
            end_move(s);
        };
        put(&mut s, f, cl, "grid:e:0", Words::Calc(format!("{e_left:.0} mE")), x0, y1 + 3.0, 2.0, "start", "");
        upward(&mut s, cl, "grid:n:0", x1 + 2.8, y1, "start", format!("{n_bottom:.0} mN"));
        for i in 1..=((x1 - x0) / GRID_MM) as u32 {
            let gx = x0 + i as f64 * GRID_MM;
            let label = format!("{:.0} mE", e_left + i as f64 * GRID_MM / k);
            let _ = writeln!(s, r##"<g stroke="#1f3f8f" stroke-width="0.3"><line x1="{gx:.2}" y1="{y0}" x2="{gx:.2}" y2="{:.2}"/><line x1="{gx:.2}" y1="{y1}" x2="{gx:.2}" y2="{:.2}"/></g>"##, y0 + 3.0, y1 - 3.0);
            put(&mut s, f, cl, &format!("grid:e:{i}:top"), Words::Calc(label.clone()), gx, y0 - 1.2, 2.0, "middle", "");
            put(&mut s, f, cl, &format!("grid:e:{i}:bottom"), Words::Calc(label), gx, y1 + 3.0, 2.0, "middle", "");
        }
        for i in 1..=((y1 - y0) / GRID_MM) as u32 {
            let gy = y1 - i as f64 * GRID_MM;
            let label = format!("{:.0} mN", n_bottom + i as f64 * GRID_MM / k);
            let _ = writeln!(s, r##"<g stroke="#1f3f8f" stroke-width="0.3"><line x1="{x0}" y1="{gy:.2}" x2="{:.2}" y2="{gy:.2}"/><line x1="{x1}" y1="{gy:.2}" x2="{:.2}" y2="{gy:.2}"/></g>"##, x0 + 3.0, x1 - 3.0);
            upward(&mut s, cl, &format!("grid:n:{i}:left"), x0 - 1.2, gy, "middle", label.clone());
            upward(&mut s, cl, &format!("grid:n:{i}:right"), x1 + 2.8, gy, "middle", label);
        }
    }

    // Access road: two dashed lines parallel to the chosen side, outside the parcel.
    let p: Vec<(f64, f64)> = corners.iter().map(|c| paper(c.e, c.n)).collect();
    let (pcx, pcy) = (p.iter().map(|q| q.0).sum::<f64>() / p.len().max(1) as f64, p.iter().map(|q| q.1).sum::<f64>() / p.len().max(1) as f64);
    // Access roads: two dashed lines parallel to each chosen side, outside the parcel. Roads on
    // neighbouring sides meet in a junction: the longer runs through, the other ends at it, and
    // the two corners of the opening are rounded, as on the printed plans.
    let roads: Vec<usize> = f.roads().into_iter().filter(|&i| i < p.len() && p.len() >= 3).collect();
    struct Road { a: (f64, f64), u: (f64, f64), nrm: (f64, f64), len: f64, t0: f64, t1: f64, t0_out: f64, t1_out: f64, gaps: Vec<(f64, f64)> }
    let mut rd: Vec<Road> = roads.iter().map(|&side| {
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
        Road { a, u: (ux, uy), nrm: (nx, ny), len, t0: -ext, t1: len + ext, t0_out: -ext, t1_out: len + ext, gaps: Vec::new() }
    }).collect();
    let off_pt = |r: &Road, off: f64| (r.a.0 + r.nrm.0 * off, r.a.1 + r.nrm.1 * off);
    let mut fillets: Vec<(Pt, Pt, Pt, usize)> = Vec::new(); // from, corner, to, the joining road's side
    for i in 0..rd.len() {
        for j in i + 1..rd.len() {
            // Two roads whose inner lines would cross (neighbouring sides, or sides a short side
            // apart) meet in a junction where they cross.
            let (pi, pj) = (off_pt(&rd[i], ROAD_IN), off_pt(&rd[j], ROAD_IN));
            let (Some(ti), Some(tj)) = (meet(pi, rd[i].u, pj, rd[j].u), meet(pj, rd[j].u, pi, rd[i].u)) else { continue };
            let reach = |r: &Road, t: f64| t > r.t0 - ROAD_OUT && t < r.t1 + ROAD_OUT;
            if !(reach(&rd[i], ti) && reach(&rd[j], tj)) {
                continue;
            }
            let (thr, join, t_thr, t_join) = if rd[i].len >= rd[j].len { (i, j, ti, tj) } else { (j, i, tj, ti) };
            let tin = (off_pt(&rd[thr], ROAD_IN), rd[thr].u);
            let jout = off_pt(&rd[join], ROAD_OUT);
            let (Some(cut_out), Some(g_out)) = (meet(jout, rd[join].u, tin.0, tin.1), meet(tin.0, tin.1, jout, rd[join].u)) else { continue };
            // The joining road ends at the through road's inner line, at its end nearer the crossing,
            // short of it by the rounded corner.
            let (jin, ju) = (off_pt(&rd[join], ROAD_IN), rd[join].u);
            let r = &mut rd[join];
            let back = if t_join < r.len / 2.0 { ROAD_FILLET } else { -ROAD_FILLET };
            if back > 0.0 { r.t0 = t_join + back; r.t0_out = cut_out + back; } else { r.t1 = t_join + back; r.t1_out = cut_out + back; }
            // The rounded corners: the through road's line, round the corner, into the joining road's.
            let on = |o: Pt, u: Pt, t: f64| (o.0 + u.0 * t, o.1 + u.1 * t);
            let away = if t_thr < g_out { -ROAD_FILLET } else { ROAD_FILLET };
            fillets.push((on(tin.0, tin.1, t_thr + away), on(tin.0, tin.1, t_thr), on(jin, ju, t_join + back), roads[join]));
            fillets.push((on(tin.0, tin.1, g_out - away), on(tin.0, tin.1, g_out), on(jout, ju, cut_out + back), roads[join]));
            // The through road runs on past the junction, open where the other road comes in.
            let r = &mut rd[thr];
            let (g0, g1) = (t_thr.min(g_out) - ROAD_FILLET, t_thr.max(g_out) + ROAD_FILLET);
            if g1 + 6.0 > r.t1 { r.t1 = g1 + 6.0; r.t1_out = r.t1_out.max(r.t1); }
            if g0 - 6.0 < r.t0 { r.t0 = g0 - 6.0; r.t0_out = r.t0_out.min(r.t0); }
            r.gaps.push((g0, g1));
        }
    }
    // Where each feature goes on paper. One on a side keeps that side's length, outside the parcel.
    // `nodes` are the points the officer placed, `lines` what is drawn through them.
    struct Placed<'a> { fi: usize, sym: &'a Symbol, label: Option<&'a str>, nodes: Vec<Pt>, lines: Vec<Vec<Pt>>, label_off: f64, color: Option<&'a str> }
    let mut on_side = vec![0u32; p.len()];
    let mut placed: Vec<Placed> = Vec::new();
    for (fi, feat) in f.features.iter().enumerate() {
        let Some(sym) = SYMBOLS.iter().find(|y| y.id == feat.symbol) else { continue };
        let label = feat.label.as_deref().map(str::trim).filter(|l| !l.is_empty());
        let gap = if sym.double > 0.0 { feat.width.filter(|w| w.is_finite()).unwrap_or(sym.double).clamp(2.0, 30.0) } else { 0.0 };
        let (nodes, label_off): (Vec<Pt>, f64) = match feat.side {
            Some(i) if i < p.len() && p.len() >= 3 => {
                let (a, b) = (p[i], p[(i + 1) % p.len()]);
                let len = dist(a, b).max(1e-9);
                let u = ((b.0 - a.0) / len, (b.1 - a.1) / len);
                let left = (u.1, -u.0);
                let (mx, my) = ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
                let sign = if (mx + left.0 - pcx).powi(2) + (my + left.1 - pcy).powi(2) > (mx - pcx).powi(2) + (my - pcy).powi(2) { 1.0 } else { -1.0 };
                let off = sign * (if gap > 0.0 { ROAD_IN + gap / 2.0 } else { FEATURE_OFF } + FEATURE_STEP * on_side[i] as f64);
                on_side[i] += 1;
                let shift = |q: Pt, along: f64| (q.0 + left.0 * off + u.0 * along, q.1 + left.1 * off + u.1 * along);
                let ext = if sym.group == "Roads" { ROAD_EXT_MM } else { 0.0 };
                (if sym.point { vec![shift((mx, my), 0.0)] } else { vec![shift(a, -ext), shift(b, ext)] }, if gap > 0.0 { 0.0 } else { sign * 1.8 })
            }
            Some(_) => continue,
            None => (feat.points.iter().map(|q| paper(q[0], q[1])).collect(), if gap > 0.0 { 0.0 } else { 1.8 }),
        };
        let centre = if feat.smooth && !sym.point { spline(&nodes) } else { nodes.clone() };
        let lines = if gap > 0.0 { vec![offset_line(&centre, gap / 2.0), offset_line(&centre, -gap / 2.0)] } else { vec![centre] };
        placed.push(Placed { fi, sym, label, nodes, lines, label_off, color: safe_color(feat.color.as_deref()) });
    }

    // Road junctions: drawn roads stop at the roads they meet, and an access road's overshoot at a
    // drawn road running across it.
    let inner_pieces = |r: &Road| {
        let mut gaps = r.gaps.clone();
        gaps.sort_by(|x, y| x.0.total_cmp(&y.0));
        let (mut out, mut t) = (Vec::new(), r.t0);
        for (g0, g1) in gaps {
            out.push((t, g0.min(r.t1)));
            t = t.max(g1);
        }
        out.push((t, r.t1));
        out
    };
    let at = |o: Pt, u: Pt, t: f64| (o.0 + u.0 * t, o.1 + u.1 * t);
    let access: Vec<[Pt; 2]> = rd.iter().flat_map(|r| {
        let (i, o) = (off_pt(r, ROAD_IN), off_pt(r, ROAD_OUT));
        inner_pieces(r).into_iter().map(move |(t0, t1)| [at(i, r.u, t0), at(i, r.u, t1)]).chain([[at(o, r.u, r.t0_out), at(o, r.u, r.t1_out)]]).collect::<Vec<_>>()
    }).collect();
    // Each line of a double road meets the others on its own, so that both stop at a road.
    let is_road = |x: &Placed| x.sym.group == "Roads" && !x.sym.point && x.nodes.len() >= 2;
    let mut drawn: Vec<Vec<Pt>> = placed.iter().filter(|x| is_road(x)).flat_map(|x| x.lines.clone()).collect();
    trim_roads(&mut drawn, &access);
    let mut trimmed = drawn.iter();
    for x in &mut placed {
        if is_road(x) {
            for line in &mut x.lines {
                line.clone_from(trimmed.next().expect("one per road line"));
            }
        }
    }
    for r in &mut rd {
        (r.t0, r.t1) = stop_at_through(off_pt(r, ROAD_IN), r.u, r.len, (r.t0, r.t1), &drawn);
        (r.t0_out, r.t1_out) = stop_at_through(off_pt(r, ROAD_OUT), r.u, r.len, (r.t0_out, r.t1_out), &drawn);
    }

    let road_color = |side: usize| safe_color(f.styles.get(&format!("roadline:{side}")).and_then(|x| x.color.as_deref())).unwrap_or("#000");
    let dashed = |s: &mut String, o: (f64, f64), u: (f64, f64), t0: f64, t1: f64, c: &str| {
        if t1 - t0 > 0.2 {
            let _ = writeln!(s, r##"<line x1="{:.2}" y1="{:.2}" x2="{:.2}" y2="{:.2}" stroke="{c}" stroke-width="0.3" stroke-dasharray="2.5 1.5"/>"##,
                o.0 + u.0 * t0, o.1 + u.1 * t0, o.0 + u.0 * t1, o.1 + u.1 * t1);
        }
    };
    let label = f.road_label.clone().filter(|l| !l.trim().is_empty()).unwrap_or_else(|| "Access Road".into());
    for (r, &side) in rd.iter().zip(&roads) {
        let c = road_color(side);
        // The road's centre line, for the editor to turn it into a road that can bend.
        let mid = off_pt(r, (ROAD_IN + ROAD_OUT) / 2.0);
        let nodes = [at(mid, r.u, r.t0.max(r.t0_out)), at(mid, r.u, (r.t0.max(r.t0_out) + r.t1.min(r.t1_out)) / 2.0), at(mid, r.u, r.t1.min(r.t1_out))];
        let _ = writeln!(s, r#"<g data-road="{side}" data-nodes="{}">"#, pts_attr(&nodes));
        // Inner line, broken where a joining road comes in.
        for (t0, t1) in inner_pieces(r) {
            dashed(&mut s, off_pt(r, ROAD_IN), r.u, t0, t1, c);
        }
        dashed(&mut s, off_pt(r, ROAD_OUT), r.u, r.t0_out, r.t1_out, c);
        s.push_str("</g>\n");
        let m = off_pt(r, (ROAD_IN + ROAD_OUT) / 2.0);
        let key = format!("road:{side}");
        let words = f.styles.get(&key).and_then(|x| x.text.as_deref()).map(str::trim).filter(|l| !l.is_empty()).unwrap_or(&label);
        begin_move(&mut s, f, &key, (m.0 + r.u.0 * r.len / 2.0, m.1 + r.u.1 * r.len / 2.0));
        along(&mut s, m, (m.0 + r.u.0 * r.len, m.1 + r.u.1 * r.len), 0.5, 0.0, 2.8, words);
        end_move(&mut s);
    }

    for (a, c, b, side) in &fillets {
        let _ = writeln!(s, r##"<path d="M{:.2} {:.2} Q{:.2} {:.2} {:.2} {:.2}" fill="none" stroke="{}" stroke-width="0.3" stroke-dasharray="2.5 1.5"/>"##, a.0, a.1, c.0, c.1, b.0, b.1, road_color(*side));
    }

    // Features from the symbol list, under the parcel, and cut at the frame: a drawn one can run
    // past it.
    let _ = writeln!(s, r#"<clipPath id="planFrame"><rect x="{x0}" y="{y0}" width="{}" height="{}"/></clipPath><g clip-path="url(#planFrame)">"#, x1 - x0, y1 - y0);
    for x in &placed {
        let (fi, sym, label) = (x.fi, x.sym, x.label);
        let _ = writeln!(s, r#"<g data-feat="{fi}" data-nodes="{}">"#, pts_attr(&x.nodes));
        if sym.point {
            let Some(&q) = x.nodes.first() else { s.push_str("</g>\n"); continue };
            feature_point(&mut s, sym, q, x.color);
            s.push_str("</g>\n");
            if let Some(l) = label {
                begin_move(&mut s, f, &format!("feature:{fi}"), (q.0 + 2.2, q.1));
                text(&mut s, q.0 + 2.2, q.1 + 0.9, 2.4, "start", HALO, &esc(l));
                end_move(&mut s);
            }
        } else if x.nodes.len() >= 2 {
            for line in &x.lines {
                feature_line(&mut s, sym, line, x.color);
            }
            s.push_str("</g>\n");
            if let Some(l) = label {
                // A curve: halfway along, turned to it; else the longest stretch, the most room.
                let seg = |w: &&[(f64, f64)]| (w[1].0 - w[0].0).powi(2) + (w[1].1 - w[0].1).powi(2);
                let centre: Vec<Pt> = if x.lines.len() == 2 { x.lines[0].iter().zip(&x.lines[1]).map(|(a, b)| ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0)).collect() } else { x.lines[0].clone() };
                let spot = if f.features[fi].smooth { middle(&centre) } else { centre.windows(2).max_by(|p, q| seg(p).total_cmp(&seg(q))).map(|w| (w[0], w[1])) };
                if let Some((a, b)) = spot {
                    begin_move(&mut s, f, &format!("feature:{fi}"), ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0));
                    along(&mut s, a, b, 0.5, x.label_off, 2.4, l);
                    end_move(&mut s);
                }
            }
        } else {
            s.push_str("</g>\n");
        }
    }
    s.push_str("</g>\n");

    // Parcel, beacons, bearings and distances.
    if p.len() >= 2 {
        let pts: String = p.iter().map(|q| format!("{:.2},{:.2}", q.0, q.1)).collect::<Vec<_>>().join(" ");
        let _ = writeln!(s, r#"<polygon points="{pts}" fill="none" stroke="{LINE}" stroke-width="0.45"/>"#);
    }
    let mut short: Vec<ShortSide> = Vec::new();
    for i in 0..p.len() {
        let (a, b) = (p[i], p[(i + 1) % p.len()]);
        if p.len() < 2 || (i == p.len() - 1 && p.len() == 2) {
            break;
        }
        let (deg, min) = bearing_dm(&corners[i], &corners[(i + 1) % corners.len()]);
        let paper_len = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
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
        if paper_len < SHORT_SIDE_MM && p.len() >= 3 {
            let cell = |cl: &mut Calc, part: &str, v: String| {
                let key = format!("short:{i}:{part}");
                let (words, attrs) = words_of(f, cl, &key, Words::Calc(v));
                (words, format!(r#"data-cell="{key}"{attrs}"#))
            };
            short.push(ShortSide {
                from: corners[i].name.clone(),
                to: corners[(i + 1) % corners.len()].name.clone(),
                degrees: cell(cl, "degrees", format!("{deg}°")),
                minutes: cell(cl, "minutes", format!("{min:02}'")),
                distance: cell(cl, "distance", format!("{dist:.2}m")),
            });
            continue;
        }
        for (key, t, off, label) in [("degrees", t_deg, out, format!("{deg}°")), ("minutes", t_min, out, format!("{min:02}'")), ("distance", 0.5, -out, format!("{dist:.2}m"))] {
            let key = format!("{key}:{i}");
            let (words, attrs) = words_of(f, cl, &key, Words::Calc(label));
            begin_move_with(&mut s, f, &key, (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t), &attrs);
            along(&mut s, a, b, t, off, 2.6, &words);
            end_move(&mut s);
        }
    }
    let mut labels: Vec<(f64, f64, &str)> = Vec::new();
    for q in &p {
        let _ = writeln!(s, r##"<path d="M{:.2} {:.2} h2 M{:.2} {:.2} v2" stroke="#000" stroke-width="0.3"/>"##, q.0 - 1.0, q.1, q.0, q.1 - 1.0);
        let (vx, vy) = (q.0 - pcx, q.1 - pcy);
        let vl = (vx * vx + vy * vy).sqrt().max(1e-9);
        let (tx, ty) = (q.0 + vx / vl * 5.0, q.1 + vy / vl * 4.0 + 1.0);
        labels.push((tx, ty, if vx < -0.5 { "end" } else if vx > 0.5 { "start" } else { "middle" }));
    }
    // Close beacons: their numbers are moved apart so that none is written over another.
    let mut boxes: Vec<LabelBox> = labels.iter().zip(&corners).map(|(&(x, y, anchor), c)| {
        let w = c.name.chars().count() as f64 * BEACON_SIZE * 0.65;
        let left = match anchor { "end" => x - w, "start" => x, _ => x - w / 2.0 };
        LabelBox { left, right: left + w, y }
    }).collect();
    spread_labels(&mut boxes);
    for (i, ((&(x, _, anchor), b), c)) in labels.iter().zip(&boxes).zip(&corners).enumerate() {
        begin_move(&mut s, f, &format!("beacon:{i}"), (x, b.y));
        text(&mut s, x, b.y, BEACON_SIZE, anchor, HALO, &esc(&c.name));
        end_move(&mut s);
    }
    // The middle: the file no. and the area, each its own label.
    let file_no = if f.plan_no.as_deref().unwrap_or("").is_empty() { title_no.clone() } else { upr(&f.plan_no) };
    put(&mut s, f, cl, "file_no", Words::Calc(file_no), pcx, pcy - 0.8, 2.6, "middle", "");
    put(&mut s, f, cl, "parcel_area", Words::Calc(parcel_area(d.area_m2)), pcx, pcy + 2.8, 2.6, "middle", "");

    // The officer's own text.
    for (i, note) in f.notes.iter().enumerate() {
        let words = note.text.trim();
        if words.is_empty() || !note.at.iter().all(|v| v.is_finite()) {
            continue;
        }
        let (x, y) = paper(note.at[0], note.at[1]);
        let size = note.size.filter(|v| v.is_finite()).unwrap_or(NOTE_SIZE).clamp(1.5, 12.0);
        let turn = note.angle.filter(|a| a.is_finite() && *a != 0.0).map(|a| format!(r#" transform="rotate({a:.1} {x:.2} {y:.2})""#)).unwrap_or_default();
        let color = safe_color(note.color.as_deref()).unwrap_or("#000");
        begin_move(&mut s, f, &format!("note:{i}"), (x, y));
        text(&mut s, x, y, size, "start", &format!(r#"fill="{color}"{turn}"#), &esc(words));
        end_move(&mut s);
    }

    // Short sides, bottom-right inside the frame (the last one lowest), as one table that moves as
    // a block: every column lines up from row to row.
    short_sides_table(&mut s, f, &short);

    // Footer.
    let fx0 = 12.0;
    let blue = format!(r#"font-weight="bold" fill="{COORD_BLUE}""#);
    if let Some(o) = corners.first() {
        put(&mut s, f, cl, "coord:title", Words::Calc(format!("WGS 1984 Co-ordinates of {}", o.name)), fx0, 236.0, 2.8, "start", &blue);
        put(&mut s, f, cl, "coord:n", Words::Calc(format!("N: {:.3}m", o.n)), fx0, 240.5, 2.8, "start", &blue);
        put(&mut s, f, cl, "coord:e", Words::Calc(format!("E: {:.3}m", o.e)), fx0, 245.0, 2.8, "start", &blue);
    }
    put(&mut s, f, cl, "surveyed_by", Words::Field("surveyed_by", format!("SURVEYED BY: {}", cap_words(f.surveyed_by.as_deref().unwrap_or("")))), fx0, 251.0, 2.8, "start", BOLD);
    put(&mut s, f, cl, "survey_date", Words::Field("survey_date", format!("SURVEY DATE: {}", f.survey_date.as_deref().unwrap_or(""))), fx0, 256.0, 2.8, "start", BOLD);
    if let Some(o) = corners.first() {
        put(&mut s, f, cl, "sheet_no", Words::Calc(format!("SHEET NO. {}", sheet_no(o.e, o.n).replace('-', " - "))), fx0, 261.0, 2.8, "start", BOLD);
    }
    let prepared = f.prepared_by.clone().filter(|p| !p.trim().is_empty()).unwrap_or_else(|| "MLSTP".into());
    put(&mut s, f, cl, "prepared_by", Words::Field("prepared_by", format!("Prepared By {prepared}")), fx0, 266.0, 2.8, "start", BOLD);
    // The computation note. A bearing or distance typed over the computed one is no longer
    // "computed from Registered Co-ordinates": the note then says that some were amended.
    let amended = cl.amended;
    let note: &[&str] = if amended {
        &["Bearings and distances shown on this plan have", "been computed from Registered Co-ordinates,", "except where amended by the plotting officer."]
    } else {
        &["All bearings and distances shown on this plan", "have been computed from Registered Co-ordinates."]
    };
    for (j, line) in note.iter().enumerate() {
        let key = if j == 2 { "computation:amended".to_string() } else { format!("computation:{j}") };
        put(&mut s, f, cl, &key, Words::Calc(line.to_string()), 92.0, 238.0 + 3.5 * j as f64, 2.5, "middle", "");
    }
    put(&mut s, f, cl, "roads_survey", Words::Calc("Roads shown are not as a result of accurate survey".into()), 92.0, 238.0 + 3.5 * note.len() as f64, 2.5, "middle", "");
    put(&mut s, f, cl, "plotted_by", Words::Field("plotted_by", format!("PLOTTED BY: {}", upr(&f.plotted_by))), 92.0, 255.0, 2.8, "middle", BOLD);
    put(&mut s, f, cl, "checked_by", Words::Field("checked_by", format!("CHECKED BY: {}", upr(&f.checked_by))), 92.0, 264.0, 2.8, "middle", BOLD);
    let sx = 168.0;
    let _ = writeln!(s, r##"<line x1="140" y1="247" x2="196" y2="247" stroke="#000" stroke-width="0.25" stroke-dasharray="1 0.8"/>"##);
    put(&mut s, f, cl, "surveyor_general", Words::Field("surveyor_general", upr(&f.surveyor_general)), sx, 251.0, 2.8, "middle", BOLD);
    put(&mut s, f, cl, "sg_title", Words::Calc("SURVEYOR-GENERAL".into()), sx, 255.0, 2.8, "middle", BOLD);
    let _ = writeln!(s, r##"<line x1="140" y1="262" x2="196" y2="262" stroke="#000" stroke-width="0.25" stroke-dasharray="1 0.8"/>"##);
    put(&mut s, f, cl, "sg_date", Words::Calc("DATE".into()), sx, 266.0, 2.8, "middle", BOLD);
    let _ = W;
    s.push_str("</svg>\n");
    (s, calc)
}

/// The area line of the header: square metres below one hectare, hectares from there up (4
/// decimals, the whole square metre, cut like `area3`).
fn area_line(m2: f64) -> String {
    if m2 < 10_000.0 {
        format!("AREA {} SQUARE METERS", area3(m2))
    } else {
        format!("AREA {:.4} HECTARES", (m2 + 1e-7).floor() / 10_000.0)
    }
}

/// The area in the middle of the parcel: square metres below one hectare, hectares only from
/// there up, the same figure as the header's `area_line`.
fn parcel_area(m2: f64) -> String {
    if m2 < 10_000.0 {
        format!("{} Sq.m", area3(m2))
    } else {
        format!("{:.4} Ha", (m2 + 1e-7).floor() / 10_000.0)
    }
}

/// The plan's area: cut to 3 decimals, never rounded up (903.1976 → "903.197"), as the printed
/// plans give it (528.215). The small epsilon keeps 903.197 itself from becoming 903.196 through
/// floating-point noise.
pub fn area3(m2: f64) -> String {
    format!("{:.3}", (m2 * 1000.0 + 1e-7).floor() / 1000.0)
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
        for want in ["TITLE NO. PL68061", "HOLDER: SA&#39;AD ABUBAKAR", "JOS SOUTH L.G. AREA", "AREA 249.489 SQUARE METERS", "UPIN PL000000023268",
                     "Scale = 1:250", "WGS 1984 Co-ordinates of H7255M", "N: 1082927.804m", "E: 484903.293m", "SHEET NO. 484 - 1082",
                     ">Q0210M<", ">Q0209M<", ">30°<", ">50'<", ">16.08m<", "Access Road", "249.489 Sq.m", "SURVEYOR-GENERAL"] {
            let want = want.replace("&#39;", "'");
            assert!(svg.contains(&want), "missing {want}");
        }
        // An explicit scale is kept.
        fields.scale = Some(750);
        let svg = render_svg(&PlanData { upin: "PL000000023268".into(), area_m2: 249.489, corners: c, fields });
        assert!(svg.contains("Scale = 1:750"));
        for line in ["WGS 1984 Co-ordinates of", "N: ", "E: "] {
            assert!(svg.contains(&format!(r##"fill="#1f6fb5">{line}"##)), "the co-ordinates are blue: {line}");
        }
    }

    #[test]
    fn sheet_number_rounds_the_thousands_down_to_even() {
        assert_eq!(sheet_no(491176.685, 1092270.579), "490-1092", "491 → 490, 1092 stays");
        assert_eq!(sheet_no(495142.578, 1092430.989), "494-1092", "PL68374");
        assert_eq!(sheet_no(484903.293, 1082927.804), "484-1082", "even thousands stay");
        assert_eq!(sheet_no(497000.0, 1093999.0), "496-1092", "an odd Northing steps down too");
    }

    #[test]
    fn surveyed_by_words_start_with_a_capital() {
        assert_eq!(cap_words("wiss resources"), "Wiss Resources");
        assert_eq!(cap_words("dakyil survey/mapping consultants"), "Dakyil Survey/Mapping Consultants");
        assert_eq!(cap_words("blessing j.wophill"), "Blessing J.Wophill");
        assert_eq!(cap_words("sa'ad ABUBAKAR"), "Sa'ad ABUBAKAR");
    }

    /// PL68374 (PLAGIS), clockwise from N9994V, as on its printed plan.
    fn pl68374() -> Vec<Corner> {
        plan_order(&[
            (495142.578, 1092430.989, "N9994V".into()),
            (495171.460, 1092438.493, "N9995V".into()),
            (495182.474, 1092408.984, "N9996V".into()),
            (495157.613, 1092403.167, "N9997V".into()),
            (495153.926, 1092403.981, "N9998V".into()),
        ])
    }

    #[test]
    fn roads_on_two_sides_and_the_grid_ticks() {
        let c = pl68374();
        assert_eq!(c[0].name, "N9994V");
        // West side (N9998V → N9994V, side 4) and south side (N9996V → N9997V, side 2), as the photo.
        let fields = PlanFields { road_sides: vec![4, 2], road_label: Some("ACCESS ROAD".into()), ..Default::default() };
        let svg = render_svg(&PlanData { upin: "PL000000023480".into(), area_m2: 902.203, corners: c.clone(), fields: fields.clone() });
        assert_eq!(svg.matches(">ACCESS ROAD<").count(), 2, "one label per road");
        assert!(svg.matches("stroke-dasharray=\"2.5 1.5\"").count() >= 4, "two lines per road");
        assert!(svg.contains(" mE<") && svg.contains(" mN<"));
        assert!(svg.contains("SHEET NO. 494 - 1092"));
        // Grid off: no ticks, no values.
        let svg = render_svg(&PlanData { upin: "PL000000023480".into(), area_m2: 902.203, corners: c.clone(), fields: PlanFields { grid: Some(false), ..fields } });
        assert!(!svg.contains(" mE<") && !svg.contains(" mN<"));
        // A form saved before road_sides keeps its one road.
        let legacy = PlanFields { road_side: Some(3), ..Default::default() };
        assert_eq!(legacy.roads(), vec![3]);
        let svg = render_svg(&PlanData { upin: "X".into(), area_m2: 1.0, corners: c, fields: legacy });
        assert_eq!(svg.matches(">Access Road<").count(), 1);
    }

    #[test]
    fn the_grid_steps_with_the_scale_from_the_frames_corner() {
        let at = |scale| render_svg(&PlanData { upin: "X".into(), area_m2: 902.203, corners: pl68374(), fields: PlanFields { scale: Some(scale), ..Default::default() } });
        // 1:500: the frame is 93 m wide; the corner value, then a tick 50 m on, top and bottom.
        let svg = at(500);
        assert_eq!(svg.matches(">495116 mE<").count(), 1, "lower-left corner");
        assert_eq!(svg.matches(">495166 mE<").count(), 2, "top and bottom");
        // 1:1000: the same 100 mm of paper is 100 m on the ground.
        let svg = at(1000);
        assert_eq!(svg.matches(">495070 mE<").count(), 1);
        assert_eq!(svg.matches(">495170 mE<").count(), 2);
        // The Northing likewise: one corner value, a tick on the left and right edges.
        let n: Vec<i64> = svg.match_indices(" mN<").map(|(i, _)| svg[..i].rsplit('>').next().unwrap().parse().unwrap()).collect();
        assert_eq!(n.len(), 3);
        assert_eq!((n[1] - n[0], n[2] - n[0]), (100, 100));
    }

    #[test]
    fn the_header_area_is_in_hectares_from_10000_square_metres() {
        assert_eq!(area_line(9_999.999), "AREA 9999.999 SQUARE METERS");
        assert_eq!(area_line(10_000.0), "AREA 1.0000 HECTARES");
        assert_eq!(area_line(25_437.9), "AREA 2.5437 HECTARES");
        assert_eq!(parcel_area(9_999.999), "9999.999 Sq.m");
        assert_eq!(parcel_area(10_000.0), "1.0000 Ha");
        assert_eq!(parcel_area(25_437.9), "2.5437 Ha");
        let svg = render_svg(&PlanData { upin: "X".into(), area_m2: 25_437.9, corners: pl68374(), fields: PlanFields::default() });
        assert!(svg.contains(">2.5437 Ha<") && !svg.contains("Sq.m"), "a big parcel shows hectares only in its middle");
    }

    #[test]
    fn a_short_side_is_written_in_the_corner_not_on_the_drawing() {
        // PL68374 at 1:500: N9997V → N9998V is 3.78 m (7.6 mm on paper), as its printed plan.
        let fields = PlanFields { scale: Some(500), ..Default::default() };
        let svg = render_svg(&PlanData { upin: "X".into(), area_m2: 902.203, corners: pl68374(), fields });
        let line = svg.lines().find(|l| l.contains(r#"data-calc="3.78m">3.78m<"#)).expect("the short side's distance");
        assert!(line.contains(r#"x="196.00" y="225.50" font-size="2.6" text-anchor="end" data-cell="short:"#) && line.contains(":distance"), "the distance ends at the lower-right corner: {line}");
        let table = short_group(&svg);
        for cell in [">N9997V<", ">-<", ">N9998V<", ">=<", ">282°<", ">27'<", ">@<", ">3.78m<"] {
            assert!(table.contains(cell), "{cell} in the corner table");
        }
        assert_eq!(svg.matches(">3.78m<").count(), 1, "not on the side itself");
        assert_eq!(svg.matches(">282°<").count(), 1, "not on the side itself");
        assert!(svg.contains(">29.30m<") && svg.contains(">25.53m<"), "the long sides keep theirs");
    }

    /// The `<g data-move="short">` group of a rendered plan.
    fn short_group(svg: &str) -> &str {
        let start = svg.find(r#"<g data-move="short""#).expect("a short-sides table");
        &svg[start..start + svg[start..].find("</g>").unwrap()]
    }

    #[test]
    fn the_short_sides_table_lines_up_in_columns_and_moves_as_one() {
        // At 1:5000 every side of PL68374 is short: five rows, degrees of 2 and 3 digits.
        let plan = |label_moves| PlanData { upin: "X".into(), area_m2: 902.203, corners: pl68374(), fields: PlanFields { scale: Some(5000), label_moves, ..Default::default() } };
        let svg = render_svg(&plan(BTreeMap::new()));
        let table = short_group(&svg);
        // Each cell as (x, y, anchor, text).
        let cells: Vec<(String, String, String, String)> = table.lines().filter(|l| l.starts_with("<text")).map(|l| {
            let attr = |name: &str| l.split(&format!(r#"{name}=""#)).nth(1).and_then(|r| r.split('"').next()).unwrap_or("").to_string();
            let txt = l.split('>').nth(1).and_then(|r| r.split('<').next()).unwrap_or("").to_string();
            (attr("x"), attr("y"), attr("text-anchor"), txt)
        }).collect();
        let rows: std::collections::BTreeSet<&str> = cells.iter().map(|c| c.1.as_str()).collect();
        assert_eq!(rows.len(), 5, "one row per side");
        assert_eq!(cells.len(), 5 * 8, "eight cells a row");
        // Column k of every row is at the same x with the same alignment.
        for k in 0..8 {
            let col: std::collections::BTreeSet<(&str, &str)> = cells.iter().skip(k).step_by(8).map(|c| (c.0.as_str(), c.2.as_str())).collect();
            assert_eq!(col.len(), 1, "column {k} lines up: {col:?}");
        }
        let anchors: Vec<&str> = cells[..8].iter().map(|c| c.2.as_str()).collect();
        assert_eq!(anchors, ["start", "middle", "start", "middle", "end", "end", "middle", "end"], "names left, numbers right");
        assert!(cells.iter().any(|c| c.3.ends_with('°') && c.3.trim_end_matches('°').len() == 3), "a three-digit bearing");
        assert!(cells.iter().all(|c| c.0.parse::<f64>().unwrap() >= FRAME.0 && c.0.parse::<f64>().unwrap() <= FRAME.2), "inside the frame");
        // One drag moves the whole table; the old per-line keys are gone.
        assert!(!svg.contains(r#"data-move="short:"#));
        let moved = render_svg(&plan(BTreeMap::from([("short".to_string(), [-20.0, -5.0])])));
        assert!(moved.contains(r#"<g data-move="short" transform="translate(-20.00 -5.00)""#));
    }

    #[test]
    fn close_beacon_numbers_are_moved_apart() {
        // PL68374 at 1:500: N9997V and N9998V are 7.6 mm apart on paper.
        let fields = PlanFields { scale: Some(500), ..Default::default() };
        let svg = render_svg(&PlanData { upin: "X".into(), area_m2: 902.203, corners: pl68374(), fields });
        let y_of = |name: &str| -> f64 {
            let line = svg.lines().find(|l| l.ends_with(&format!(">{name}</text>"))).unwrap();
            line.split("y=\"").nth(1).unwrap().split('"').next().unwrap().parse().unwrap()
        };
        assert!((y_of("N9997V") - y_of("N9998V")).abs() >= BEACON_LINE - 0.01);

        let mut b = [LabelBox { left: 0.0, right: 10.0, y: 50.0 }, LabelBox { left: 5.0, right: 15.0, y: 51.0 }, LabelBox { left: 40.0, right: 50.0, y: 50.5 }];
        spread_labels(&mut b);
        assert!((b[1].y - b[0].y - BEACON_LINE).abs() < 1e-6, "touching: a line apart, the upper one stays upper");
        assert!((b[0].y + b[1].y - 101.0).abs() < 1e-6, "each moved by the same amount");
        assert_eq!(b[2].y, 50.5, "a label that touches none stays");
    }

    #[test]
    fn features_are_drawn_at_their_co_ordinates_or_along_a_side() {
        let feat = |symbol: &str, label: Option<&str>, side, points| PlanFeature { symbol: symbol.into(), label: label.map(Into::into), side, points, ..Default::default() };
        let features = vec![
            // 1:500 puts 495116 / 1092379 on the frame's lower-left corner (12, 228): 2 mm per metre.
            feat("river", Some("River Delimi"), None, vec![[495126.0, 1092389.0], [495146.0, 1092389.0]]),
            feat("tree", Some("Mango"), None, vec![[495126.0, 1092399.0]]),
            feat("gutter", None, Some(2), vec![]),
            feat("no_such_symbol", None, None, vec![[495126.0, 1092389.0], [495146.0, 1092389.0]]),
            feat("road", None, Some(99), vec![]),
            feat("river", None, None, vec![[495126.0, 1092389.0]]),
        ];
        let fields = PlanFields { scale: Some(500), features, ..Default::default() };
        let svg = render_svg(&PlanData { upin: "X".into(), area_m2: 902.203, corners: pl68374(), fields });
        assert_eq!(svg.matches("<polyline").count(), 3, "the river, and the gutter's two lines; nothing for the bad ones");
        assert!(svg.contains(r##"points="32.00,208.00 72.00,208.00" fill="none" stroke="#1f6fb5""##), "the river at its ground co-ordinates");
        assert!(svg.contains(">River Delimi<") && svg.contains(">Mango<"));
        assert!(svg.contains(r#"translate(32.00 188.00)"#), "the tree");
        assert!(svg.find("planFrame").unwrap() < svg.find("<polygon").unwrap(), "under the parcel, cut at the frame");

        // The gutter on side 2 (N9996V → N9997V) runs parallel to it, on the side away from the parcel.
        let c = pl68374();
        let (a, b) = (&c[2], &c[3]);
        let at = |c: &Corner| (12.0 + (c.e - 495116.0) * 2.0, 228.0 - (c.n - 1092379.0) * 2.0);
        let (pa, pb) = (at(a), at(b));
        let line = svg.lines().find(|l| l.contains(r##"stroke="#444""##)).unwrap();
        let pts: Vec<f64> = line.split('"').nth(1).unwrap().split([' ', ',']).map(|v| v.parse().unwrap()).collect();
        let (dx, dy) = (pts[0] - pa.0, pts[1] - pa.1);
        assert!((dx.hypot(dy) - FEATURE_OFF).abs() < 0.02, "4 mm from the side");
        assert!(((pts[2] - pb.0) - dx).abs() < 0.02 && ((pts[3] - pb.1) - dy).abs() < 0.02, "parallel");
        assert!(dy > 0.0, "south of the parcel, lower on the sheet");
    }

    #[test]
    fn a_second_feature_on_a_side_goes_further_out_and_a_point_sits_at_its_middle() {
        let f = |symbol: &str| PlanFeature { symbol: symbol.into(), side: Some(2), ..Default::default() };
        let fields = PlanFields { scale: Some(500), features: vec![f("gutter"), f("tree")], ..Default::default() };
        let c = pl68374();
        let svg = render_svg(&PlanData { upin: "X".into(), area_m2: 1.0, corners: c.clone(), fields });
        let at = |c: &Corner| (12.0 + (c.e - 495116.0) * 2.0, 228.0 - (c.n - 1092379.0) * 2.0);
        let (a, b) = (at(&c[2]), at(&c[3]));
        let (mx, my) = ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
        let t = svg.split("translate(").nth(1).unwrap().split(')').next().unwrap();
        let q: Vec<f64> = t.split(' ').map(|v| v.parse().unwrap()).collect();
        assert!(((q[0] - mx).hypot(q[1] - my) - (FEATURE_OFF + FEATURE_STEP)).abs() < 0.02);
    }

    #[test]
    fn a_form_saved_before_features_still_reads() {
        let f: PlanFields = serde_json::from_str(r#"{"plan_no":"PL1"}"#).unwrap();
        assert!(f.features.is_empty());
        assert_eq!(SYMBOLS.iter().map(|y| y.id).collect::<std::collections::BTreeSet<_>>().len(), SYMBOLS.len(), "ids are unique");
    }

    #[test]
    fn beacons_are_plus_marks_and_a_dragged_label_moves_but_its_beacon_does_not() {
        let plan = |label_moves| PlanData { upin: "X".into(), area_m2: 902.203, corners: pl68374(), fields: PlanFields { scale: Some(500), label_moves, ..Default::default() } };
        let before = render_svg(&plan(BTreeMap::new()));
        assert_eq!(before.matches(" h2 M").count(), 5, "a + at each beacon");
        assert!(!before.contains("l2 2"), "no x");
        for key in ["beacon:0", "degrees:0", "minutes:0", "distance:0", "short", "file_no", "parcel_area", "area", "upin", "holder", "sg_title", "grid:e:0"] {
            assert!(before.contains(&format!(r#"data-move="{key}""#)), "{key} can be dragged");
        }
        let after = render_svg(&plan(BTreeMap::from([("beacon:0".to_string(), [3.5, -2.0])])));
        assert!(after.contains(r#"<g data-move="beacon:0" transform="translate(3.50 -2.00)""#));
        let marks = |s: &str| s.lines().filter(|l| l.contains(" h2 M") || l.contains("<polygon")).map(String::from).collect::<Vec<_>>();
        assert_eq!(marks(&before), marks(&after), "the beacons and the boundary stay where they are");
    }

    #[test]
    fn the_sheet_says_where_its_grid_is_for_drawing_on_it() {
        let fields = PlanFields { scale: Some(500), ..Default::default() };
        let svg = render_svg(&PlanData { upin: "X".into(), area_m2: 1.0, corners: pl68374(), fields });
        assert!(svg.contains(r#"data-grid="495116 1092379 2 12 228""#));
    }

    #[test]
    fn a_road_end_stops_at_the_road_it_meets() {
        let bar = |x: f64| [(x, -30.0), (x, 30.0)];
        let trim = |road: Vec<Pt>, fixed: &[[Pt; 2]]| { let mut r = vec![road]; trim_roads(&mut r, fixed); r.remove(0) };
        let close = |a: Pt, b: Pt| assert!(dist(a, b) < 1e-6, "{a:?} {b:?}");
        // Past it: cut back. Short of it: carried on. Far past it: a crossing, left alone.
        close(*trim(vec![(0.0, 0.0), (40.0, 0.0)], &[bar(35.0)]).last().unwrap(), (35.0, 0.0));
        close(*trim(vec![(0.0, 0.0), (33.0, 0.0)], &[bar(35.0)]).last().unwrap(), (35.0, 0.0));
        assert_eq!(trim(vec![(0.0, 0.0), (60.0, 0.0)], &[bar(35.0)]), vec![(0.0, 0.0), (60.0, 0.0)]);
        // Over both lines of an access road: stops at the first it comes to.
        close(*trim(vec![(0.0, 0.0), (45.0, 0.0)], &[bar(35.0), bar(43.0)]).last().unwrap(), (35.0, 0.0));
        // The start of a road too, and a bend past the junction goes with the cut.
        let r = trim(vec![(40.0, 5.0), (40.0, 0.0), (30.0, 0.0), (0.0, 0.0)], &[bar(35.0)]);
        assert_eq!(r.len(), 3);
        close(r[0], (35.0, 0.0));
    }

    #[test]
    fn two_roads_make_a_t_the_one_that_ends_stops() {
        let mut r = vec![vec![(0.0, 0.0), (50.0, 0.0)], vec![(25.0, -20.0), (25.0, 3.0)]];
        trim_roads(&mut r, &[]);
        assert_eq!(r[0], vec![(0.0, 0.0), (50.0, 0.0)], "the through road");
        assert_eq!(r[1], vec![(25.0, -20.0), (25.0, 0.0)]);
        // A long road ending at the middle of a short one is still the one that stops.
        let mut r = vec![vec![(0.0, 0.0), (50.0, 0.0)], vec![(45.0, -15.0), (45.0, 15.0)]];
        trim_roads(&mut r, &[]);
        assert_eq!(r[0], vec![(0.0, 0.0), (45.0, 0.0)]);
        // Both ending at the crossing: the longer runs through.
        let mut r = vec![vec![(0.0, 0.0), (50.0, 0.0)], vec![(48.0, -20.0), (48.0, 3.0)]];
        trim_roads(&mut r, &[]);
        assert_eq!(r[0], vec![(0.0, 0.0), (50.0, 0.0)]);
        assert_eq!(r[1], vec![(48.0, -20.0), (48.0, 0.0)]);
    }

    #[test]
    fn an_access_road_overshoot_stops_at_a_drawn_road_across_it() {
        let through = vec![vec![(60.0, -40.0), (60.0, 40.0)]];
        assert_eq!(stop_at_through((0.0, 0.0), (1.0, 0.0), 50.0, (-10.0, 70.0), &through), (-10.0, 60.0));
        // One that ends on the access road stops at it, and leaves it whole.
        let ending = vec![vec![(60.0, -40.0), (60.0, 0.0)]];
        assert_eq!(stop_at_through((0.0, 0.0), (1.0, 0.0), 50.0, (-10.0, 70.0), &ending), (-10.0, 70.0));
        // Along the side itself it is not cut.
        let beside = vec![vec![(30.0, -40.0), (30.0, 40.0)]];
        assert_eq!(stop_at_through((0.0, 0.0), (1.0, 0.0), 50.0, (-10.0, 70.0), &beside), (-10.0, 70.0));
    }

    #[test]
    fn a_drawn_road_stops_at_the_access_roads_outer_line() {
        let c = pl68374();
        let at = |c: &Corner| (12.0 + (c.e - 495116.0) * 2.0, 228.0 - (c.n - 1092379.0) * 2.0);
        let (a, b) = (at(&c[2]), at(&c[3]));
        let len = dist(a, b);
        let mut n = ((b.1 - a.1) / len, -(b.0 - a.0) / len);
        if n.1 < 0.0 { n = (-n.0, -n.1); } // south, down the sheet: away from the parcel
        let mid = ((a.0 + b.0) / 2.0 + n.0 * 13.0, (a.1 + b.1) / 2.0 + n.1 * 13.0);
        let ground = |q: Pt| [495116.0 + (q.0 - 12.0) / 2.0, 1092379.0 + (228.0 - q.1) / 2.0];
        let road = PlanFeature { symbol: "road".into(), points: vec![ground((mid.0 + n.0 * 30.0, mid.1 + n.1 * 30.0)), ground((mid.0 - n.0 * 3.0, mid.1 - n.1 * 3.0))], ..Default::default() };
        let fields = PlanFields { scale: Some(500), road_sides: vec![2], features: vec![road], ..Default::default() };
        let svg = render_svg(&PlanData { upin: "X".into(), area_m2: 1.0, corners: c, fields });
        let line = svg.lines().find(|l| l.starts_with("<polyline")).unwrap();
        let v: Vec<f64> = line.split('"').nth(1).unwrap().split([' ', ',']).map(|v| v.parse().unwrap()).collect();
        assert!(dist((v[v.len() - 2], v[v.len() - 1]), mid) < 0.02, "ends on the outer line, not inside the road");
    }

    #[test]
    fn neighbouring_roads_meet_in_a_junction() {
        let c = pl68374();
        // Sides 1 (N9995V → N9996V) and 2 (N9996V → N9997V) share N9996V.
        let svg = render_svg(&PlanData { upin: "X".into(), area_m2: 1.0, corners: c, fields: PlanFields { road_sides: vec![1, 2], grid: Some(false), ..Default::default() } });
        // Through road: inner line broken in two + outer; joining road: inner + outer; two rounded corners.
        assert_eq!(svg.matches("stroke-dasharray=\"2.5 1.5\"").count(), 7);
        assert_eq!(svg.matches(" Q").count(), 2, "the corners of the opening are rounded");
    }

    fn editor_plan() -> PlanData {
        let corners = vec![Corner { name: "A".into(), e: 1000.0, n: 1000.0 }, Corner { name: "B".into(), e: 1040.0, n: 1000.0 }, Corner { name: "C".into(), e: 1040.0, n: 1040.0 }, Corner { name: "D".into(), e: 1000.0, n: 1040.0 }];
        PlanData { upin: "PL1".into(), area_m2: 1600.0, corners, fields: PlanFields { scale: Some(500), ..Default::default() } }
    }

    #[test]
    fn a_spline_goes_through_every_point_and_bends_between_them() {
        let nodes = [(0.0, 0.0), (10.0, 10.0), (20.0, 0.0), (45.0, 12.0)];
        let c = spline(&nodes);
        assert_eq!(c.len(), 3 * CURVE_STEPS + 1);
        for (i, q) in nodes.iter().enumerate() {
            assert!(dist(c[i * CURVE_STEPS], *q) < 1e-9, "point {i}");
        }
        // Between the first two it is off the straight line, and has no break at the second.
        let (m, chord) = (c[CURVE_STEPS / 2], (5.0, 5.0));
        assert!(dist(m, chord) > 0.2);
        let dir = |a: Pt, b: Pt| (b.1 - a.1).atan2(b.0 - a.0);
        assert!((dir(c[CURVE_STEPS - 1], c[CURVE_STEPS]) - dir(c[CURVE_STEPS], c[CURVE_STEPS + 1])).abs() < 0.4); // a corner here would be 1.6
        assert_eq!(spline(&[(0.0, 0.0), (1.0, 1.0)]).len(), 2);
    }

    #[test]
    fn a_double_line_keeps_its_width_round_a_bend() {
        let line = offset_line(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)], 2.0);
        assert!(dist(line[0], (0.0, -2.0)) < 1e-9);
        assert!(dist(line[1], (12.0, -2.0)) < 1e-9, "{:?}", line[1]);
        assert!(dist(line[2], (12.0, 10.0)) < 1e-9);
    }

    #[test]
    fn a_curved_double_road_prints_two_lines_with_its_colour() {
        let mut d = editor_plan();
        d.fields.features.push(PlanFeature { symbol: "road_access".into(), smooth: true, color: Some("#ff0000".into()), label: Some("A & B <Road>".into()),
            points: vec![[990.0, 1060.0], [1020.0, 1070.0], [1050.0, 1060.0]], ..Default::default() });
        let svg = render_svg(&d);
        let g = &svg[svg.find(r#"<g data-feat="0""#).unwrap()..];
        let g = &g[..g.find("</g>").unwrap()];
        assert_eq!(g.matches("<polyline").count(), 2, "{g}");
        assert_eq!(g.matches(r##"stroke="#ff0000""##).count(), 2);
        assert_eq!(g.lines().nth(1).unwrap().matches(',').count(), 2 * CURVE_STEPS + 1);
        // The nodes stay the three the officer placed; the label is written safely.
        assert_eq!(g.lines().next().unwrap().matches(',').count(), 3);
        assert!(svg.contains("A &amp; B &lt;Road&gt;"));
    }

    #[test]
    fn both_lines_of_a_double_road_stop_at_the_road_they_meet() {
        let mut d = editor_plan();
        let across = PlanFeature { symbol: "road".into(), points: vec![[960.0, 1065.0], [1080.0, 1065.0]], ..Default::default() };
        let stem = PlanFeature { symbol: "road_double".into(), points: vec![[1020.0, 1100.0], [1020.0, 1062.0]], ..Default::default() };
        d.fields.features = vec![across, stem];
        let svg = render_svg(&d);
        let g = &svg[svg.find(r#"<g data-feat="1""#).unwrap()..];
        let g = &g[..g.find("</g>").unwrap()];
        let [e0, n0, k, x0, y1]: [f64; 5] = svg.split("data-grid=\"").nth(1).unwrap().split('"').next().unwrap().split(' ').map(|v| v.parse().unwrap()).collect::<Vec<_>>().try_into().unwrap();
        let y = y1 - (1065.0 - n0) * k;
        let _ = (e0, x0);
        let ends: Vec<f64> = g.lines().skip(1).map(|l| l.split("points=\"").nth(1).unwrap().split('"').next().unwrap().split(' ').last().unwrap().split(',').nth(1).unwrap().parse().unwrap()).collect();
        assert_eq!(ends.len(), 2);
        assert!(ends.iter().all(|v| (v - y).abs() < 0.02), "{ends:?} {y}");
    }

    #[test]
    fn a_colour_is_only_ever_a_colour() {
        assert_eq!(safe_color(Some(" #A1b2C3 ")), Some("#A1b2C3"));
        for bad in ["red", "#12345", "#12345g", "\"><script>", ""] {
            assert_eq!(safe_color(Some(bad)), None, "{bad}");
        }
        let mut d = editor_plan();
        d.fields.features.push(PlanFeature { symbol: "tree".into(), color: Some("\"/><script>".into()), points: vec![[1020.0, 1020.0]], ..Default::default() });
        d.fields.styles.insert("beacon:0".into(), LabelStyle { color: Some("\"/><script>".into()), ..Default::default() });
        d.fields.notes.push(PlanNote { text: "x".into(), at: [1010.0, 1010.0], color: Some("\"/><script>".into()), ..Default::default() });
        assert!(!render_svg(&d).contains("<script>"));
    }

    #[test]
    fn a_label_takes_its_colour_and_an_access_road_its_own_words() {
        let mut d = editor_plan();
        d.fields.road_sides = vec![2];
        d.fields.styles.insert("beacon:1".into(), LabelStyle { color: Some("#0000ff".into()), ..Default::default() });
        d.fields.styles.insert("road:2".into(), LabelStyle { text: Some("Rayfield Road".into()), ..Default::default() });
        d.fields.styles.insert("roadline:2".into(), LabelStyle { color: Some("#00aa00".into()), ..Default::default() });
        let svg = render_svg(&d);
        assert!(svg.contains(r##"fill="#0000ff" data-at="##));
        assert!(svg.contains("Rayfield Road") && !svg.contains("Access Road"));
        let g = &svg[svg.find(r#"<g data-road="2""#).unwrap()..];
        assert_eq!(g[..g.find("</g>").unwrap()].matches(r##"stroke="#00aa00""##).count(), 2);
    }

    #[test]
    fn a_note_is_written_where_it_was_placed_and_can_be_turned() {
        let mut d = editor_plan();
        d.fields.notes.push(PlanNote { text: "Rock <outcrop>".into(), at: [1020.0, 1020.0], color: Some("#123456".into()), size: Some(3.5), angle: Some(30.0) });
        d.fields.notes.push(PlanNote { text: "  ".into(), ..Default::default() });
        let svg = render_svg(&d);
        assert!(svg.contains(r#"<g data-move="note:0""#) && !svg.contains("note:1"));
        assert!(svg.contains("Rock &lt;outcrop&gt;") && svg.contains(r##"fill="#123456""##) && svg.contains(r#"font-size="3.5""#) && svg.contains("rotate(30.0 "));
    }

    #[test]
    fn the_editor_changes_nothing_of_the_beacons_or_the_figures() {
        let plain = editor_plan();
        let mut d = editor_plan();
        d.fields.features.push(PlanFeature { symbol: "road_access".into(), smooth: true, points: vec![[990.0, 1060.0], [1020.0, 1070.0], [1050.0, 1060.0]], ..Default::default() });
        d.fields.styles.insert("distance:0".into(), LabelStyle { color: Some("#ff0000".into()), ..Default::default() });
        let (a, b) = (render_svg(&plain), render_svg(&d));
        assert!(b.contains("40.00m"));
        let beacons = |svg: &str| svg.lines().filter(|l| l.contains(" h2 M")).map(str::to_string).collect::<Vec<_>>();
        assert_eq!(beacons(&a), beacons(&b));
        assert_eq!(beacons(&b).len(), 4);
    }

    #[test]
    fn a_form_saved_before_the_editor_still_reads() {
        let f: PlanFeature = serde_json::from_str(r#"{"symbol":"road","label":null,"side":null,"points":[[1,2],[3,4]]}"#).unwrap();
        assert!(!f.smooth && f.color.is_none() && f.width.is_none());
        let p: PlanFields = serde_json::from_str(r#"{"features":[],"label_moves":{}}"#).unwrap();
        assert!(p.notes.is_empty() && p.styles.is_empty());
    }

    #[test]
    fn a_turned_label_turns_about_where_it_is_written_and_changes_nothing_else() {
        let plain = editor_plan();
        let mut d = editor_plan();
        d.fields.styles.insert("distance:0".into(), LabelStyle { angle: Some(45.0), ..Default::default() });
        d.fields.label_moves.insert("distance:0".into(), [2.0, -1.0]);
        d.fields.styles.insert("beacon:1".into(), LabelStyle { angle: Some(f64::NAN), ..Default::default() });
        let (a, b) = (render_svg(&plain), render_svg(&d));
        let g = b.lines().find(|l| l.contains(r#"data-move="distance:0""#)).unwrap();
        let at = g.split("data-at=\"").nth(1).unwrap().split('"').next().unwrap();
        assert!(g.contains(&format!(r#"transform="translate(2.00 -1.00) rotate(45.0 {at})""#)), "{g}");
        assert!(!b.lines().find(|l| l.contains(r#"data-move="beacon:1""#)).unwrap().contains("rotate"));
        let writing = |svg: &str| svg.lines().filter(|l| l.starts_with("<text") || l.contains(" h2 M") || l.starts_with("<polygon")).map(str::to_string).collect::<Vec<_>>();
        assert_eq!(writing(&a), writing(&b));
    }

    #[test]
    fn plan_area_is_cut_not_rounded() {
        assert_eq!(super::area3(903.1976520011156), "903.197");
        assert_eq!(super::area3(249.4899), "249.489");
        assert_eq!(super::area3(528.215), "528.215");
        assert_eq!(super::area3(100.0), "100.000");
    }

    fn typed(text: &str, was: Option<&str>) -> LabelStyle {
        LabelStyle { text: Some(text.into()), was: was.map(str::to_string), ..Default::default() }
    }

    #[test]
    fn computed_labels_can_be_typed_over_and_say_so() {
        let mut d = editor_plan();
        for (key, words) in [("area", "AREA 1600.5 SQUARE METERS"), ("parcel_area", "1600.5 Sq.m"), ("sg_title", "SURVEYOR GENERAL"), ("grid:e:0", "999 mE")] {
            d.fields.styles.insert(key.into(), typed(words, None));
        }
        let svg = render_svg(&d);
        assert!(svg.contains(">AREA 1600.5 SQUARE METERS<") && !svg.contains(">AREA 1600.000 SQUARE METERS<"), "the header area as typed");
        assert!(svg.contains(r#"<g data-move="area" data-calc="AREA 1600.000 SQUARE METERS" data-override="1""#), "the editor sees what it replaced");
        assert!(svg.contains(">1600.5 Sq.m<") && svg.contains(">SURVEYOR GENERAL<") && svg.contains(">999 mE<"));
        let o = overrides(&d);
        assert_eq!(o.len(), 4);
        assert!(o.iter().all(|x| !x.stale), "nothing known to have changed");
        // Typed the same as computed: not an override.
        d.fields.styles.insert("area".into(), typed("AREA 1600.000 SQUARE METERS", None));
        assert_eq!(overrides(&d).len(), 3);
    }

    #[test]
    fn an_override_is_stale_once_the_coordinates_give_another_value() {
        let mut d = editor_plan();
        d.fields.styles.insert("distance:0".into(), typed("40.10m", Some("40.00m")));
        assert!(!overrides(&d)[0].stale);
        assert!(!render_svg(&d).contains("data-stale"));
        // The parcel is corrected: side 1 is now 41 m.
        d.corners[1].e = 1041.0;
        let o = overrides(&d);
        assert_eq!((o[0].key.as_str(), o[0].typed.as_str(), o[0].calculated.as_str(), o[0].stale), ("distance:0", "40.10m", "41.00m", true));
        assert!(render_svg(&d).contains(r#"data-calc="41.00m" data-override="1" data-stale="1""#));
    }

    #[test]
    fn the_upin_is_never_typed_over() {
        let mut d = editor_plan();
        d.fields.styles.insert("upin".into(), typed("UPIN PL999", None));
        let svg = render_svg(&d);
        assert!(svg.contains(">UPIN PL1<") && !svg.contains("PL999"));
        assert!(svg.contains(r#"<g data-move="upin" data-locked="1""#));
        assert!(overrides(&d).is_empty());
    }

    #[test]
    fn a_typed_bearing_or_distance_changes_the_computation_note() {
        let mut d = editor_plan();
        let svg = render_svg(&d);
        assert!(svg.contains(">All bearings and distances shown on this plan<") && !svg.contains("amended"));
        // An area typed over: bearings and distances are still all computed.
        d.fields.styles.insert("area".into(), typed("AREA 1 SQUARE METERS", None));
        assert!(!render_svg(&d).contains("amended"));
        d.fields.styles.insert("minutes:2".into(), typed("01'", None));
        let svg = render_svg(&d);
        for line in [">Bearings and distances shown on this plan have<", ">been computed from Registered Co-ordinates,<", ">except where amended by the plotting officer.<", ">Roads shown are not as a result of accurate survey<"] {
            assert!(svg.contains(line), "{line}");
        }
        assert!(!svg.contains(">All bearings"));
        // A typed cell of the short-sides table counts too.
        let mut d = PlanData { upin: "X".into(), area_m2: 902.203, corners: pl68374(), fields: PlanFields { scale: Some(500), ..Default::default() } };
        let key = computed_labels(&d).into_keys().find(|k| k.starts_with("short:") && k.ends_with(":distance")).expect("a short side");
        d.fields.styles.insert(key.clone(), typed("3.80m", None));
        let svg = render_svg(&d);
        assert!(short_group(&svg).contains(&format!(r#"data-cell="{key}" data-calc="3.78m" data-override="1">3.80m<"#)));
        assert!(svg.contains("amended by the plotting officer"));
    }

    #[test]
    fn form_labels_name_their_field_and_a_recoloured_label_drops_its_own_colour() {
        let mut d = editor_plan();
        d.fields.holder = Some("Ada Musa".into());
        d.fields.styles.insert("area".into(), LabelStyle { color: Some("#00aa00".into()), ..Default::default() });
        let svg = render_svg(&d);
        assert!(svg.contains(r#"<g data-move="holder" data-field="holder""#) && svg.contains(">HOLDER: ADA MUSA<"));
        let area = svg.lines().find(|l| l.contains(">AREA 1600.000 SQUARE METERS<")).unwrap();
        assert!(!area.contains("fill="), "the group's green shows, not the sheet's red: {area}");
        assert!(svg.contains(r##"<g data-move="area" data-calc="AREA 1600.000 SQUARE METERS" fill="#00aa00""##));
    }

    #[test]
    fn a_plan_saved_with_the_middle_as_one_block_keeps_both_lines_where_they_were() {
        let mut d = editor_plan();
        d.fields.label_moves.insert("parcel".into(), [4.0, -3.0]);
        let svg = render_svg(&d);
        for key in ["file_no", "parcel_area"] {
            assert!(svg.contains(&format!(r#"<g data-move="{key}" data-calc"#)) && svg.contains(&format!(r#"data-move="{key}""#)));
        }
        assert_eq!(svg.matches("translate(4.00 -3.00)").count(), 2);
        let mut f = d.fields.clone();
        f.split_parcel_block();
        assert!(!f.label_moves.contains_key("parcel") && f.label_moves["file_no"] == [4.0, -3.0]);
    }
}
