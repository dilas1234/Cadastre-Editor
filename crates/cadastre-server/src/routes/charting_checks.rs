//! Charting's automatic checks (docs/charting-and-reporting.docx, phase C3): for a file's committed
//! parcel, a suggested answer to each question of the Survey Report on Right of Occupancy, worked
//! out from this Cadastre, each with its evidence (which parcel, which beacon, which layer).
//! The officer confirms or corrects them; nothing is written. A question whose reference layer
//! this Cadastre does not hold (town plans, Government land, rail, major roads, settlements) says
//! so instead of guessing.

use axum::extract::{Path, State};
use axum::Json;
use serde::Serialize;
use serde_json::{json, Map, Value};
use sqlx::Row;

use crate::error::{ApiResult, AppError};
use crate::routes::precheck::{counts_sql, overlap_min_width};
use crate::state::AppState;

/// A corner counts as beaconed when a beacon lies within this distance (metres), as the point list
/// names corners.
const BEACON_SNAP_M: f64 = 0.05;
/// Overlaps at or under this area (m²) are rounding, as in the system check.
const MIN_OVERLAP_M2: f64 = 0.01;

/// One question's suggestion.
#[derive(Debug, Serialize, PartialEq)]
pub struct Check {
    /// The report field it answers (`identified`, `ground_open`, …).
    pub field: &'static str,
    /// The question's number on the form.
    pub question: &'static str,
    /// `suggested` (an answer is offered), `no_layer` (the layer it needs is not in this
    /// Cadastre) or `unknown` (the data could not decide it).
    pub result: &'static str,
    /// The suggested answer (`Yes` / `No`), when there is one.
    pub value: Option<&'static str>,
    /// Follow-up fields filled with it (e.g. `ground_detail`, `tracing_no`).
    pub follow: Map<String, Value>,
    /// Why, in the officer's words.
    pub evidence: String,
    /// The parcels named in the evidence, to zoom to.
    pub upins: Vec<String>,
}

fn check(field: &'static str, question: &'static str, result: &'static str, value: Option<&'static str>, evidence: impl Into<String>) -> Check {
    Check { field, question, result, value, follow: Map::new(), evidence: evidence.into(), upins: Vec::new() }
}

fn no_layer(field: &'static str, question: &'static str, what: &str) -> Check {
    check(field, question, "no_layer", None, format!("This Cadastre holds no {what} layer, so this is not checked: answer it from the sheet and the file."))
}

/// A parcel the plot touches.
#[derive(Debug, Clone)]
pub struct Touch {
    pub upin: String,
    /// `posted`, `archived` (retired) or `historic` (replaced by a mutation).
    pub status: String,
    pub area: f64,
    /// Counts as an overlap (wider than the shared-boundary tolerance).
    pub counts: bool,
    /// The Registry property it is titled to (property number or SU id), if any.
    pub title: Option<String>,
    pub file_no: Option<String>,
}

/// One corner of the plot and the beacon on it, if any.
#[derive(Debug, Clone)]
pub struct Corner {
    pub upin: String,
    pub index: usize,
    pub beacon: Option<String>,
}

/// The beacon series ("PM" of PM935), or `None` when the beacons do not share one.
pub fn beacon_series(names: &[String]) -> Option<String> {
    let mut series = names.iter().map(|n| n.trim().chars().take_while(|c| c.is_ascii_alphabetic()).collect::<String>().to_uppercase());
    let first = series.next().filter(|s| !s.is_empty())?;
    series.all(|s| s == first).then_some(first)
}

fn m2(a: f64) -> String {
    format!("{a:.2} m²")
}

fn names(t: &[&Touch]) -> String {
    t.iter().take(8).map(|t| format!("{}{} ({})", t.upin, t.file_no.as_deref().map(|f| format!(", {f}")).unwrap_or_default(), m2(t.area))).collect::<Vec<_>>().join("; ")
}

/// Questions 2 and 5 from the parcels the plot touches: the ground is open when no committed
/// parcel is overlapped (a shared boundary drawn a little differently does not count); the plot is
/// under title when an overlapped committed parcel is titled in the Registry. Retired and replaced
/// parcels under the plot are named, but do not close the ground.
pub fn ground_and_title(touches: &[Touch], width: f64) -> (Check, Check) {
    let live: Vec<&Touch> = touches.iter().filter(|t| t.status == "posted" && t.counts).collect();
    let slivers: Vec<&Touch> = touches.iter().filter(|t| t.status == "posted" && !t.counts).collect();
    let former: Vec<&Touch> = touches.iter().filter(|t| t.status != "posted" && t.counts).collect();
    let mut notes = Vec::new();
    if !slivers.is_empty() {
        notes.push(format!("Shared boundaries drawn a little differently (narrower than {:.0} cm), not counted: {}.", width * 100.0, names(&slivers)));
    }
    if !former.is_empty() {
        notes.push(format!("Under the plot, no longer in force: {}.", former.iter().take(8)
            .map(|t| format!("{} ({}, {})", t.upin, if t.status == "archived" { "retired" } else { "replaced by a mutation" }, m2(t.area))).collect::<Vec<_>>().join("; ")));
    }
    notes.push("The earlier charting (Willams / JGOMOS) is not loaded yet: check the sheet too.".into());
    let tail = format!(" {}", notes.join(" "));

    let mut ground = if live.is_empty() {
        check("ground_open", "2", "suggested", Some("Yes"), format!("No committed parcel lies on the plot.{tail}"))
    } else {
        let mut c = check("ground_open", "2", "suggested", Some("No"), format!("The plot overlaps {}.{tail}", names(&live)));
        c.follow.insert("ground_detail".into(), json!(format!("Overlaps {}", names(&live))));
        c
    };
    ground.upins = live.iter().chain(former.iter()).map(|t| t.upin.clone()).collect();

    let titled: Vec<&&Touch> = live.iter().filter(|t| t.title.is_some()).collect();
    let mut title = if titled.is_empty() {
        let why = if live.is_empty() { "No committed parcel lies on the plot, so no title found on it." } else { "The parcels it overlaps are not titled in the Registry." };
        check("under_title", "5", "suggested", Some("No"), format!("{why} Only titles linked to a parcel in this Cadastre are seen."))
    } else {
        let list = titled.iter().map(|t| format!("{} is titled to {}", t.upin, t.title.as_deref().unwrap_or("?"))).collect::<Vec<_>>().join("; ");
        let mut c = check("under_title", "5", "suggested", Some("Yes"), format!("{list}."));
        c.follow.insert("title_no".into(), json!(titled.iter().filter_map(|t| t.title.clone()).collect::<Vec<_>>().join(", ")));
        c
    };
    title.upins = titled.iter().map(|t| t.upin.clone()).collect();
    (ground, title)
}

/// Question 3 from the plot's corners: completely beaconed when every corner has a beacon on it.
/// The Tracing No is the beacon series, the Deposited plan No the parcel's plan number.
pub fn beaconing(corners: &[Corner], plan_no: Option<&str>) -> Check {
    if corners.is_empty() {
        return check("beaconed", "3", "unknown", None, "The plot's corners could not be read.");
    }
    let named: Vec<String> = corners.iter().filter_map(|c| c.beacon.clone()).collect();
    let bare: Vec<String> = corners.iter().filter(|c| c.beacon.is_none()).map(|c| format!("{} corner {}", c.upin, c.index)).collect();
    let mut c = if bare.is_empty() {
        let mut c = check("beaconed", "3", "suggested", Some("Yes"), format!("All {} corners have a beacon: {}.", corners.len(), named.join(", ")));
        match beacon_series(&named) {
            Some(s) => { c.follow.insert("tracing_no".into(), json!(s)); }
            None => c.evidence.push_str(" The beacons are of more than one series, so the Tracing No is not filled."),
        }
        if let Some(p) = plan_no.map(str::trim).filter(|p| !p.is_empty()) {
            c.follow.insert("deposited_plan_no".into(), json!(p));
        }
        c.follow.insert("separate_survey".into(), json!("No"));
        c.evidence.push_str(" Government property (c) is not checked: this Cadastre holds no Government land layer.");
        c
    } else {
        check("beaconed", "3", "suggested", Some("No"), format!("{} of {} corners have no beacon within {:.0} cm: {}.", bare.len(), corners.len(), BEACON_SNAP_M * 100.0, bare.join(", ")))
    };
    c.upins = corners.iter().map(|c| c.upin.clone()).fold(Vec::new(), |mut v, u| { if !v.contains(&u) { v.push(u); } v });
    c
}

/// A committed parcel of the file: UPIN, file number, plan number, layout, area name, m².
type ParcelRow = (String, Option<String>, Option<String>, Option<String>, Option<String>, f64);

/// The checks for a file's committed parcels (`upins`, filed under `file_no`).
pub async fn run(pool: &sqlx::PgPool, upins: &[String], file_no: Option<&str>) -> ApiResult<Vec<Check>> {
    let mut conn = pool.acquire().await?;
    let found: Vec<ParcelRow> = sqlx::query_as(
        "SELECT p.parcel_number, a.property_number, a.plan_number, a.layout, a.area_name, ST_Area(p.geom) FROM cadastre.parcel p \
         LEFT JOIN LATERAL (SELECT * FROM cadastre.parcel_attribute x WHERE x.upin = p.parcel_number AND (x.status = '0' OR x.status IS NULL) LIMIT 1) a ON true \
         WHERE p.status = 'posted' AND p.parcel_number = ANY($1) ORDER BY p.parcel_number")
        .bind(upins).fetch_all(&mut *conn).await?;
    if found.is_empty() {
        let mut out = vec![check("identified", "1", "unknown", None, format!("{} has no committed parcel in this Cadastre: digitise and commit it, then chart it.", file_no.unwrap_or("This file")))];
        for (f, q, w) in [("ground_open", "2", "the plot"), ("beaconed", "3", "the plot's corners"), ("under_title", "5", "the plot")] {
            out.push(check(f, q, "unknown", None, format!("Not checked: {w} is not in this Cadastre.")));
        }
        out.extend(reference_checks(&found));
        return Ok(out);
    }

    // 1. The land is identified: a committed parcel filed under this file number.
    let area: f64 = found.iter().map(|f| f.5).sum();
    let elsewhere: Vec<String> = found.iter().filter(|f| file_no.is_some_and(|n| f.1.as_deref().is_some_and(|p| !p.trim().eq_ignore_ascii_case(n.trim())))).map(|f| format!("{} is filed under {}", f.0, f.1.as_deref().unwrap_or("?"))).collect();
    let list = found.iter().map(|f| f.0.clone()).collect::<Vec<_>>();
    let mut identified = if elsewhere.is_empty() {
        check("identified", "1", "suggested", Some("Yes"), format!("Committed parcel {} ({}){}.", list.join(", "), m2(area), file_no.map(|n| format!(", filed under {n}")).unwrap_or_default()))
    } else {
        check("identified", "1", "unknown", None, format!("The parcel does not carry this file number: {}. Check the file before answering.", elsewhere.join("; ")))
    };
    let absent: Vec<&String> = upins.iter().filter(|u| !list.contains(u)).collect();
    if !absent.is_empty() {
        identified.evidence.push_str(&format!(" Not committed in this Cadastre (retired, replaced or not digitised), so not checked: {}.",
            absent.iter().map(|u| u.as_str()).collect::<Vec<_>>().join(", ")));
    }
    identified.upins = list.clone();

    // 2 + 5. What the plot falls on.
    let width = overlap_min_width(&mut conn).await?;
    let me = "(SELECT ST_Union(geom) FROM cadastre.parcel WHERE status = 'posted' AND parcel_number = ANY($1))";
    let rows = sqlx::query(&format!(
        "WITH me AS (SELECT {me} AS g) \
         SELECT p.parcel_number, p.status, ST_Area(ST_Intersection(p.geom, me.g)) AS a, {} AS counts, \
                COALESCE(NULLIF(btrim(r.refnum), ''), r.su_id, CASE WHEN at.linkage_status = 1 THEN COALESCE(NULLIF(btrim(at.property_number), ''), at.lrs_id) END) AS title, \
                at.property_number AS file_no \
         FROM cadastre.parcel p CROSS JOIN me \
         LEFT JOIN LATERAL (SELECT x.linkage_status, x.lrs_id, x.property_number FROM cadastre.parcel_attribute x WHERE x.upin = p.parcel_number ORDER BY (x.status = '0') DESC NULLS LAST LIMIT 1) at ON true \
         LEFT JOIN LATERAL (SELECT y.refnum, y.su_id FROM cadastre.registry_property y WHERE p.status = 'posted' AND y.upin = p.parcel_number LIMIT 1) r ON true \
         WHERE p.geom && me.g AND p.status IN ('posted', 'archived', 'historic') AND p.parcel_number IS NOT NULL AND NOT (p.parcel_number = ANY($1)) \
           AND NOT EXISTS (SELECT 1 FROM cadastre.parcel_lineage l WHERE (l.upin = ANY($1)) AND (l.parent_id = p.id OR l.parent_upin = p.parcel_number)) \
           AND ST_Area(ST_Intersection(p.geom, me.g)) > $3 \
         ORDER BY (p.status = 'posted') DESC, a DESC LIMIT 50", counts_sql("p.geom", "me.g", "$2")))
        .bind(upins).bind(width).bind(MIN_OVERLAP_M2).fetch_all(&mut *conn).await?;
    let mut touches: Vec<Touch> = Vec::new();
    for r in rows {
        let t = Touch { upin: r.get("parcel_number"), status: r.get("status"), area: r.get("a"), counts: r.get::<Option<bool>, _>("counts").unwrap_or(false),
            title: r.get("title"), file_no: r.get("file_no") };
        // A parcel replaced or retired more than once is named once.
        if !touches.iter().any(|x| x.upin == t.upin && x.status == t.status) { touches.push(t); }
    }
    let (ground, title) = ground_and_title(&touches, width);

    // 3. The corners and their beacons.
    let corner_rows: Vec<(String, i32, Option<String>)> = sqlx::query_as(
        "WITH g AS (SELECT parcel_number AS upin, (ST_Dump(geom)).geom AS poly FROM cadastre.parcel WHERE status = 'posted' AND parcel_number = ANY($1)), \
              r AS (SELECT upin, row_number() OVER (PARTITION BY upin) AS k, ST_ExteriorRing(poly) AS ring FROM g), \
              v AS (SELECT upin, k, (d).path[1] AS i, (d).geom AS pt, ST_NPoints(ring) AS n FROM (SELECT upin, k, ring, ST_DumpPoints(ring) AS d FROM r) x) \
         SELECT upin, (row_number() OVER (PARTITION BY upin ORDER BY k, i))::int4, \
                (SELECT b.number FROM cadastre.beacon b WHERE ST_DWithin(b.geom, pt, $2) ORDER BY ST_Distance(b.geom, pt) LIMIT 1) \
         FROM v WHERE i < n ORDER BY upin, k, i")
        .bind(upins).bind(BEACON_SNAP_M).fetch_all(&mut *conn).await?;
    let corners: Vec<Corner> = corner_rows.into_iter().map(|(upin, i, beacon)| Corner { upin, index: i as usize, beacon }).collect();
    let plan_no = found.iter().find_map(|f| f.2.clone().filter(|p| !p.trim().is_empty()));
    let beaconed = beaconing(&corners, plan_no.as_deref());

    let mut out = vec![identified, ground, beaconed];
    let mut refs = reference_checks(&found);
    out.push(refs.remove(0));
    out.push(title);
    out.extend(refs);
    Ok(out)
}

/// Questions 4, 6, 7 and 8 need reference layers this Cadastre does not hold yet. The town plan
/// says which Registry layout the parcel is in, when known.
fn reference_checks(found: &[ParcelRow]) -> Vec<Check> {
    let mut town = no_layer("town_plan", "4", "town plan (TPO)");
    let layouts: Vec<String> = found.iter().filter_map(|f| {
        let parts: Vec<&str> = [f.3.as_deref(), f.4.as_deref()].into_iter().flatten().map(str::trim).filter(|s| !s.is_empty()).collect();
        (!parts.is_empty()).then(|| format!("{}: {}", f.0, parts.join(", ")))
    }).collect();
    if !layouts.is_empty() {
        town.evidence.push_str(&format!(" Layout and area recorded on the parcel: {}.", layouts.join("; ")));
    }
    vec![town, no_layer("rail", "6", "rail line"), no_layer("major_road", "7", "major road"), no_layer("settlement", "8", "settlement area")]
}

/// The automatic checks for one file of the charting queue.
#[utoipa::path(get, path = "/api/v1/charting/files/{id}/checks", tag = "charting", params(("id" = i32, Path, description = "Job instance id")),
    responses((status = 200, description = "A suggestion per question, with its evidence"), (status = 404, description = "Not a completed file")))]
pub async fn checks(State(state): State<AppState>, Path(id): Path<i32>) -> ApiResult<Json<Value>> {
    let row = sqlx::query(&format!("{} SELECT upins, pn FROM file WHERE id = $1", crate::routes::charting::FILES)).bind(id).fetch_optional(&state.pool).await?
        .ok_or_else(|| AppError::NotFound(format!("completed file {id}")))?;
    let (upins, pn): (Vec<String>, Option<String>) = (row.get("upins"), row.get("pn"));
    let checks = run(&state.pool, &upins, pn.as_deref()).await?;
    Ok(Json(json!({ "checked_at": chrono::Utc::now().to_rfc3339(), "upins": upins, "checks": checks })))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(upin: &str, status: &str, area: f64, counts: bool, title: Option<&str>) -> Touch {
        Touch { upin: upin.into(), status: status.into(), area, counts, title: title.map(Into::into), file_no: None }
    }

    #[test]
    fn the_beacon_series_is_the_shared_letters() {
        assert_eq!(beacon_series(&["PM935".into(), "pm936".into(), "PM937".into()]), Some("PM".into()));
        assert_eq!(beacon_series(&["PM935".into(), "PJ8233".into()]), None, "two series");
        assert_eq!(beacon_series(&["1234".into()]), None, "no letters");
        assert_eq!(beacon_series(&[]), None);
    }

    #[test]
    fn open_ground_and_no_title_when_nothing_counts() {
        let t = [touch("PL1", "posted", 0.3, false, Some("PL9")), touch("PL2", "archived", 50.0, true, None)];
        let (g, title) = ground_and_title(&t, 0.05);
        assert_eq!((g.value, title.value), (Some("Yes"), Some("No")));
        assert!(g.evidence.contains("Shared boundaries") && g.evidence.contains("PL1"), "{}", g.evidence);
        assert!(g.evidence.contains("PL2 (retired"), "a retired parcel is named: {}", g.evidence);
        assert!(g.follow.is_empty());
        assert_eq!(g.upins, ["PL2"]);
    }

    #[test]
    fn an_overlap_closes_the_ground_and_a_titled_one_is_a_title() {
        let t = [touch("PL5", "posted", 12.345, true, Some("PL68000")), touch("PL6", "posted", 3.0, true, None)];
        let (g, title) = ground_and_title(&t, 0.05);
        assert_eq!(g.value, Some("No"));
        assert!(g.follow["ground_detail"].as_str().unwrap().contains("PL5 (12.35 m²)"));
        assert_eq!((title.value, title.follow["title_no"].as_str()), (Some("Yes"), Some("PL68000")));
        assert_eq!(title.upins, ["PL5"]);
    }

    #[test]
    fn a_plot_is_completely_beaconed_when_every_corner_has_a_beacon() {
        let c = |i, b: Option<&str>| Corner { upin: "PL1".into(), index: i, beacon: b.map(Into::into) };
        let all = [c(1, Some("PM935")), c(2, Some("PM936")), c(3, Some("PM937")), c(4, Some("PM938"))];
        let r = beaconing(&all, Some("PLA/123"));
        assert_eq!(r.value, Some("Yes"));
        assert_eq!((r.follow["tracing_no"].as_str(), r.follow["deposited_plan_no"].as_str(), r.follow["separate_survey"].as_str()), (Some("PM"), Some("PLA/123"), Some("No")));
        let gap = [c(1, Some("PM935")), c(2, None), c(3, Some("PM937"))];
        let r = beaconing(&gap, None);
        assert_eq!(r.value, Some("No"));
        assert!(r.evidence.contains("1 of 3 corners") && r.evidence.contains("PL1 corner 2"), "{}", r.evidence);
        assert!(r.follow.is_empty());
        assert_eq!(beaconing(&[], None).result, "unknown");
    }
}
