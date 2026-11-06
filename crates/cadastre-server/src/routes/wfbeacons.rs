//! Beacon numbers at commit (Aumentum parity). When a job's parcels are committed the officer
//! enters the beacon number of every corner, as written on the surveyor's point list (e.g.
//! `CA6782`). The system never generates a beacon number. A corner lying on a beacon that is
//! already in the fabric (a shared boundary pillar) is that beacon. Corners are grouped into
//! points: two new parcels sharing a corner share one beacon.
//!
//! A typed number that a beacon elsewhere already has is one of two things. On a property sharing
//! a boundary with this parcel, within `commit.adjoining_beacon_m` (1 m), it is that same beacon:
//! the corner takes the beacon's position. Anywhere else the digitizer is shown the property
//! holding it (`check`) and may go on only with a reason; the number then repeats.

use std::collections::HashMap;

use axum::extract::{Path, State};
use axum::Json;
use serde_json::{json, Value};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::audit;
use crate::auth::AuthUser;
use crate::error::{ApiResult, AppError};
use crate::state::AppState;

/// One corner point of the job's draft parcels.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct WfCornerPointDto {
    /// 1-based point number, for the map labels and the list.
    pub point: i32,
    pub easting: f64,
    pub northing: f64,
    /// The fabric beacon this corner lies on, if any (its number is then fixed).
    pub beacon_id: Option<Uuid>,
    pub beacon_number: Option<String>,
    /// How many draft-parcel corners are at this point.
    pub corners: i32,
}

/// A beacon number typed for a corner point, identified by its coordinates.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct WfBeaconNumber {
    pub easting: f64,
    pub northing: f64,
    pub number: String,
    /// Why this number is used although another property holds it (see `check`).
    #[serde(default)]
    pub reason: Option<String>,
}

/// A property holding a beacon.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct BeaconProperty {
    pub upin: String,
    /// The file number.
    pub file_number: Option<String>,
}

/// A typed beacon number that a beacon elsewhere already has.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct BeaconConflict {
    /// The corner it was typed for, as in the Beacon Numbers window.
    pub point: i32,
    pub easting: f64,
    pub northing: f64,
    pub number: String,
    /// `adjoining`: the same beacon, the corner takes its position. `elsewhere`: needs a reason.
    pub kind: String,
    pub beacon_id: Uuid,
    pub beacon_easting: f64,
    pub beacon_northing: f64,
    pub distance_m: f64,
    /// The properties at that beacon. Empty if it is on no parcel of this Cadastre.
    pub properties: Vec<BeaconProperty>,
}

/// How near a beacon's parcel boundary must be for the parcel to be holding it.
const ON_BOUNDARY_M: f64 = 0.1;

/// What `apply` did besides adding numbers, for the commit's audit record.
#[derive(Debug, Default)]
pub struct Applied {
    pub added: Vec<String>,
    pub notes: Vec<Value>,
}

#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct WfCommitRequest {
    /// Beacon numbers for the new corner points.
    #[serde(default)]
    pub beacons: Vec<WfBeaconNumber>,
    /// The staff member committing (recorded in the audit log with the commit).
    #[serde(default)]
    pub staff_name: Option<String>,
    /// Their user id (cadastre.principal), when picked from the users list.
    #[serde(default)]
    pub staff_sid: Option<i32>,
}

#[derive(Debug, Clone)]
struct Corner {
    parcel_id: Uuid,
    seq: i32,
    e: f64,
    n: f64,
}

#[derive(Debug, Clone)]
struct Point {
    e: f64,
    n: f64,
    beacon: Option<(Uuid, String)>,
    corners: Vec<(Uuid, i32)>,
}

async fn setting(conn: &mut sqlx::PgConnection, key: &str) -> ApiResult<Option<String>> {
    Ok(sqlx::query_scalar("SELECT value FROM cadastre.app_setting WHERE key = $1").bind(key).fetch_optional(conn).await?)
}

async fn tolerance(conn: &mut sqlx::PgConnection) -> ApiResult<f64> {
    Ok(setting(conn, "commit.beacon_tolerance_m").await?.and_then(|v| v.trim().parse().ok()).filter(|t: &f64| *t > 0.0 && *t < 1.0).unwrap_or(0.05))
}

/// The job's draft-parcel corners grouped into points (in parcel order, then ring order).
async fn corner_points(conn: &mut sqlx::PgConnection, job_id: Uuid, tol: f64) -> ApiResult<Vec<Point>> {
    // Every vertex of every ring except each ring's closing repeat.
    let rows = sqlx::query(
        "WITH v AS ( \
           SELECT d.id AS parcel_id, d.created_at, dp.path[1] AS part, dp.path[2] AS ring, dp.path[3] AS idx, dp.geom, \
                  max(dp.path[3]) OVER (PARTITION BY d.id, dp.path[1], dp.path[2]) AS last \
           FROM cadastre.parcel d, ST_DumpPoints(ST_Multi(d.geom)) dp \
           WHERE d.job_id = $1 AND d.status = 'draft') \
         SELECT v.parcel_id, (row_number() OVER (PARTITION BY v.parcel_id ORDER BY v.part, v.ring, v.idx))::int AS seq, \
                ST_X(v.geom) AS e, ST_Y(v.geom) AS n, b.id AS beacon_id, b.number AS beacon_number \
         FROM v LEFT JOIN LATERAL (SELECT id, number FROM cadastre.beacon \
                 WHERE ST_DWithin(geom, v.geom, $2) ORDER BY geom <-> v.geom LIMIT 1) b ON true \
         WHERE v.idx < v.last \
         ORDER BY v.created_at, v.parcel_id, seq",
    )
    .bind(job_id)
    .bind(tol)
    .fetch_all(&mut *conn)
    .await?;
    let mut points: Vec<Point> = Vec::new();
    for r in &rows {
        let c = Corner { parcel_id: r.get("parcel_id"), seq: r.get("seq"), e: r.get("e"), n: r.get("n") };
        let beacon: Option<(Uuid, String)> = r.get::<Option<Uuid>, _>("beacon_id").map(|id| (id, r.get("beacon_number")));
        let same = points.iter_mut().find(|p| match (&p.beacon, &beacon) {
            (Some(a), Some(b)) => a.0 == b.0,
            (None, None) => (p.e - c.e).hypot(p.n - c.n) <= tol,
            _ => false,
        });
        match same {
            Some(p) => p.corners.push((c.parcel_id, c.seq)),
            None => points.push(Point { e: c.e, n: c.n, beacon, corners: vec![(c.parcel_id, c.seq)] }),
        }
    }
    Ok(points)
}

async fn adjoining_limit(conn: &mut sqlx::PgConnection) -> ApiResult<f64> {
    Ok(setting(conn, "commit.adjoining_beacon_m").await?.and_then(|v| v.trim().parse().ok()).filter(|t: &f64| *t > 0.0).unwrap_or(1.0))
}

/// The committed properties at a beacon, by position: Aumentum's parcels have no corner records.
async fn properties_at(conn: &mut sqlx::PgConnection, beacon: Uuid) -> ApiResult<Vec<BeaconProperty>> {
    let rows = sqlx::query(
        "SELECT p.parcel_number, a.property_number FROM cadastre.beacon b \
         JOIN cadastre.parcel p ON p.status = 'posted' AND p.geom && ST_Expand(b.geom, $2) AND ST_DWithin(ST_Boundary(p.geom), b.geom, $2) \
         LEFT JOIN LATERAL (SELECT property_number FROM cadastre.parcel_attribute x WHERE x.upin = p.parcel_number \
                            ORDER BY (x.status = '0') DESC NULLS LAST, x.object_id DESC LIMIT 1) a ON true \
         WHERE b.id = $1 ORDER BY p.parcel_number LIMIT 20",
    )
    .bind(beacon).bind(ON_BOUNDARY_M).fetch_all(&mut *conn).await?;
    Ok(rows.iter().map(|r| BeaconProperty { upin: r.get("parcel_number"), file_number: r.get("property_number") }).collect())
}

/// The other beacon with this number nearest to the point, if any.
async fn nearest_with_number(conn: &mut sqlx::PgConnection, number: &str, e: f64, n: f64, except: Option<Uuid>) -> ApiResult<Option<(Uuid, f64, f64)>> {
    let row = sqlx::query(
        "SELECT id, ST_X(geom) AS e, ST_Y(geom) AS n FROM cadastre.beacon WHERE upper(number) = upper($1) AND id IS DISTINCT FROM $4 \
         ORDER BY geom <-> ST_SetSRID(ST_MakePoint($2, $3), 32632) LIMIT 1",
    )
    .bind(number).bind(e).bind(n).bind(except).fetch_optional(&mut *conn).await?;
    Ok(row.map(|r| (r.get("id"), r.get("e"), r.get("n"))))
}

/// A number typed for a point, if a beacon elsewhere has it: on an adjoining property or not.
async fn conflict(conn: &mut sqlx::PgConnection, index: usize, p: &Point, number: &str, limit: f64) -> ApiResult<Option<BeaconConflict>> {
    let Some((beacon_id, be, bn)) = nearest_with_number(conn, number, p.e, p.n, None).await? else { return Ok(None) };
    let distance_m = (p.e - be).hypot(p.n - bn);
    let drafts: Vec<Uuid> = p.corners.iter().map(|c| c.0).collect();
    let adjoining = distance_m <= limit && sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM cadastre.beacon b \
           JOIN cadastre.parcel n ON n.status = 'posted' AND n.geom && ST_Expand(b.geom, $3) AND ST_DWithin(ST_Boundary(n.geom), b.geom, $3) \
           JOIN cadastre.parcel d ON d.id = ANY($2) AND ST_DWithin(n.geom, d.geom, $4) \
           WHERE b.id = $1)",
    )
    .bind(beacon_id).bind(&drafts).bind(ON_BOUNDARY_M).bind(limit).fetch_one(&mut *conn).await?;
    Ok(Some(BeaconConflict {
        point: index as i32 + 1, easting: p.e, northing: p.n, number: number.to_string(),
        kind: if adjoining { "adjoining" } else { "elsewhere" }.to_string(),
        beacon_id, beacon_easting: be, beacon_northing: bn, distance_m,
        properties: properties_at(conn, beacon_id).await?,
    }))
}

/// Where a beacon is held, for messages: "file PL67436 (UPIN PL…)".
fn held_by(properties: &[BeaconProperty]) -> String {
    if properties.is_empty() {
        return "no property in this Cadastre".to_string();
    }
    properties.iter().map(|p| match &p.file_number {
        Some(f) if !f.trim().is_empty() => format!("file {} (UPIN {})", f.trim(), p.upin),
        _ => format!("UPIN {} (no file number)", p.upin),
    }).collect::<Vec<_>>().join(", ")
}

/// Move a draft's corners onto a point. The drafts are simple rings: a corner's `seq` is its
/// position in the ring, and the first corner also closes it.
async fn move_corners(conn: &mut sqlx::PgConnection, corners: &[(Uuid, i32)], e: f64, n: f64) -> ApiResult<()> {
    for (parcel, seq) in corners {
        let moved = sqlx::query(
            "WITH r AS (SELECT id, GeometryType(geom) AS t, ST_ExteriorRing(ST_GeometryN(ST_Multi(geom), 1)) AS l, ST_SetSRID(ST_MakePoint($3, $4), ST_SRID(geom)) AS pt \
                        FROM cadastre.parcel WHERE id = $1 AND ST_NumGeometries(ST_Multi(geom)) = 1 AND ST_NumInteriorRings(ST_GeometryN(ST_Multi(geom), 1)) = 0), \
                  s AS (SELECT id, t, ST_MakePolygon(ST_SetPoint(ST_SetPoint(l, $2 - 1, pt), CASE WHEN $2 = 1 THEN ST_NPoints(l) - 1 ELSE $2 - 1 END, pt)) AS g FROM r) \
             UPDATE cadastre.parcel d SET geom = CASE WHEN s.t = 'MULTIPOLYGON' THEN ST_Multi(s.g) ELSE s.g END, computed_area = ST_Area(s.g), updated_at = now() FROM s WHERE d.id = s.id",
        )
        .bind(parcel).bind(seq).bind(e).bind(n).execute(&mut *conn).await?.rows_affected();
        if moved == 0 {
            return Err(AppError::Conflict("a parcel with several parts or holes cannot take an adjoining beacon's position: move its corner onto the beacon".into()));
        }
    }
    Ok(())
}

/// The next free repeat of a number.
async fn next_repeat(conn: &mut sqlx::PgConnection, number: &str) -> ApiResult<i32> {
    Ok(sqlx::query_scalar("SELECT COALESCE(max(duplicate_seq) + 1, 0) FROM cadastre.beacon WHERE number = $1").bind(number).fetch_one(&mut *conn).await?)
}

fn to_dto(points: &[Point]) -> Vec<WfCornerPointDto> {
    points
        .iter()
        .enumerate()
        .map(|(i, p)| WfCornerPointDto {
            point: i as i32 + 1,
            easting: (p.e * 1000.0).round() / 1000.0,
            northing: (p.n * 1000.0).round() / 1000.0,
            beacon_id: p.beacon.as_ref().map(|b| b.0),
            beacon_number: p.beacon.as_ref().map(|b| b.1.clone()),
            corners: p.corners.len() as i32,
        })
        .collect()
}

/// The corner points of a job's draft parcels, for entering beacon numbers before commit.
#[utoipa::path(get, path = "/api/v1/wf/jobs/{id}/editing/corners", tag = "wf", params(("id" = i32, Path, description = "Job instance id")),
    responses((status = 200, body = [WfCornerPointDto])))]
pub async fn corners(State(state): State<AppState>, Path(id): Path<i32>) -> ApiResult<Json<Vec<WfCornerPointDto>>> {
    let mut conn = state.pool.acquire().await?;
    let job_id: Option<Uuid> = sqlx::query_scalar("SELECT cadastre_job_id FROM cadastre.wf_job_instance WHERE id = $1")
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("job instance {id}")))?;
    let Some(job_id) = job_id else { return Ok(Json(Vec::new())) };
    let tol = tolerance(&mut conn).await?;
    Ok(Json(to_dto(&corner_points(&mut conn, job_id, tol).await?)))
}

/// The sketch's corners to identify (UTM 32N metres, ring order, no closing repeat).
#[derive(Debug, Deserialize, ToSchema)]
pub struct IdentifyVerticesRequest {
    pub vertices: Vec<[f64; 2]>,
}

/// What a sketched corner is: a fabric beacon, a corner shared with a committed parcel, or new.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct IdentifiedVertexDto {
    /// 1-based corner number.
    pub point: i32,
    pub easting: f64,
    pub northing: f64,
    /// `beacon` (on a fabric beacon), `shared` (on a corner of a committed parcel), or `new`.
    pub source: String,
    /// The fabric beacon's number, when `source` is `beacon`.
    pub beacon_number: Option<String>,
    /// How far the corner is from that beacon or shared corner (m).
    pub offset_m: Option<f64>,
    /// The committed parcel(s) whose corner this is.
    pub shared_with: Vec<String>,
    /// Distance to the next corner (m), closing back to the first.
    pub next_distance_m: f64,
}

/// Identify every corner of a sketch before it is saved (Verify Points): the digitiser clicks on
/// the map, and each corner is matched, within the commit's beacon tolerance
/// (`commit.beacon_tolerance_m`, 5 cm), to a fabric beacon (its number) or to a corner of a
/// committed parcel. Nothing is created; a new corner's number is entered by the officer, as in
/// Aumentum (the system never makes up a beacon number).
#[utoipa::path(post, path = "/api/v1/beacons/identify", tag = "beacons", request_body = IdentifyVerticesRequest,
    responses((status = 200, body = [IdentifiedVertexDto])))]
pub async fn identify(State(state): State<AppState>, Json(req): Json<IdentifyVerticesRequest>) -> ApiResult<Json<Vec<IdentifiedVertexDto>>> {
    if req.vertices.len() > 2000 {
        return Err(AppError::BadRequest("too many vertices".into()));
    }
    if req.vertices.iter().flatten().any(|v| !v.is_finite()) {
        return Err(AppError::BadRequest("vertices must be numbers (UTM 32N metres)".into()));
    }
    let mut conn = state.pool.acquire().await?;
    let tol = tolerance(&mut conn).await?;
    let es: Vec<f64> = req.vertices.iter().map(|v| v[0]).collect();
    let ns: Vec<f64> = req.vertices.iter().map(|v| v[1]).collect();
    let rows = sqlx::query(
        "WITH v AS (SELECT i::int AS i, ST_SetSRID(ST_MakePoint(e, n), 32632) AS g \
                    FROM unnest($1::float8[], $2::float8[]) WITH ORDINALITY AS t(e, n, i)) \
         SELECT v.i, b.number AS beacon_number, b.d AS beacon_d, s.upins, s.d AS shared_d \
         FROM v \
         LEFT JOIN LATERAL (SELECT bb.number, ST_Distance(bb.geom, v.g) AS d FROM cadastre.beacon bb \
                            WHERE ST_DWithin(bb.geom, v.g, $3) ORDER BY bb.geom <-> v.g LIMIT 1) b ON true \
         LEFT JOIN LATERAL (SELECT array_agg(DISTINCT p.parcel_number ORDER BY p.parcel_number) AS upins, min(ST_Distance(dp.geom, v.g)) AS d \
                            FROM cadastre.parcel p, ST_DumpPoints(p.geom) dp \
                            WHERE p.status = 'posted' AND p.geom && ST_Expand(v.g, $3) AND ST_DWithin(dp.geom, v.g, $3)) s ON true \
         ORDER BY v.i",
    )
    .bind(&es)
    .bind(&ns)
    .bind(tol)
    .fetch_all(&mut *conn)
    .await?;
    let n = req.vertices.len();
    let mut out = Vec::with_capacity(n);
    for (k, r) in rows.iter().enumerate() {
        let [e, nn] = req.vertices[k];
        let [e2, n2] = req.vertices[(k + 1) % n];
        let beacon: Option<String> = r.get("beacon_number");
        let shared: Vec<String> = r.get::<Option<Vec<String>>, _>("upins").unwrap_or_default();
        let (source, offset) = if beacon.is_some() {
            ("beacon", r.get::<Option<f64>, _>("beacon_d"))
        } else if !shared.is_empty() {
            ("shared", r.get::<Option<f64>, _>("shared_d"))
        } else {
            ("new", None)
        };
        out.push(IdentifiedVertexDto {
            point: k as i32 + 1,
            easting: (e * 1000.0).round() / 1000.0,
            northing: (nn * 1000.0).round() / 1000.0,
            source: source.into(),
            beacon_number: beacon,
            offset_m: offset.map(|d| (d * 1000.0).round() / 1000.0),
            shared_with: shared,
            next_distance_m: if n > 1 { ((e2 - e).hypot(n2 - nn) * 1000.0).round() / 1000.0 } else { 0.0 },
        });
    }
    Ok(Json(out))
}

/// Within the commit transaction: check the typed beacon numbers, add the new beacons and link
/// every corner to its beacon. Returns the numbers of the beacons added.
pub async fn apply(conn: &mut sqlx::PgConnection, job_id: Uuid, req: &WfCommitRequest) -> ApiResult<Applied> {
    let tol = tolerance(conn).await?;
    let required = setting(conn, "commit.require_beacon_numbers").await?.map(|v| v.trim() != "0").unwrap_or(true);
    let points = corner_points(conn, job_id, tol).await?;

    // Match each typed number to its point by coordinates.
    let mut typed: Vec<Option<String>> = vec![None; points.len()];
    for b in &req.beacons {
        let number = b.number.trim();
        if number.is_empty() {
            continue;
        }
        let at = points.iter().position(|p| (p.e - b.easting).hypot(p.n - b.northing) <= tol).ok_or_else(|| {
            AppError::BadRequest(format!("beacon {number} at {:.3} {:.3} is not a corner of the job's parcels", b.easting, b.northing))
        })?;
        typed[at] = Some(number.to_string());
    }

    let mut missing = Vec::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    for (i, p) in points.iter().enumerate() {
        if p.beacon.is_some() {
            continue; // already a fabric beacon: its number stands
        }
        match &typed[i] {
            None => missing.push(i + 1),
            Some(n) => {
                if let Some(j) = seen.insert(n.to_uppercase(), i) {
                    return Err(AppError::BadRequest(format!("beacon number {n} is given for two different corners (points {} and {})", j + 1, i + 1)));
                }
            }
        }
    }
    if required && !missing.is_empty() {
        let list: Vec<String> = missing.iter().map(|m| m.to_string()).collect();
        return Err(AppError::Conflict(format!(
            "enter the beacon number of every corner before committing; missing for point(s) {}",
            list.join(", ")
        )));
    }

    let limit = adjoining_limit(conn).await?;
    let reason_for = |p: &Point| req.beacons.iter().find(|b| (p.e - b.easting).hypot(p.n - b.northing) <= tol)
        .and_then(|b| b.reason.as_deref()).map(str::trim).filter(|r| !r.is_empty()).map(str::to_string);
    let mut added = Vec::new();
    let mut notes = Vec::new();
    let mut beacon_of: Vec<Option<Uuid>> = points.iter().map(|p| p.beacon.as_ref().map(|b| b.0)).collect();
    for (i, p) in points.iter().enumerate() {
        let Some(number) = typed[i].as_ref().filter(|_| p.beacon.is_none()) else { continue };
        // A number already in the fabric, on a beacon this corner is not on.
        let mut repeat = 0;
        if let Some(c) = conflict(&mut *conn, i, p, number, limit).await? {
            if c.kind == "adjoining" {
                move_corners(&mut *conn, &p.corners, c.beacon_easting, c.beacon_northing).await?;
                beacon_of[i] = Some(c.beacon_id);
                notes.push(json!({ "number": number, "kind": "adjoining", "moved_m": c.distance_m, "held_by": held_by(&c.properties) }));
                continue;
            }
            let Some(reason) = reason_for(p) else {
                return Err(AppError::Conflict(format!(
                    "Beacon {number} already exists on {}, at {:.3} {:.3}, {:.2} m from point {}. Check that property; to use the number anyway, give a reason.",
                    held_by(&c.properties), c.beacon_easting, c.beacon_northing, c.distance_m, i + 1
                )));
            };
            repeat = next_repeat(&mut *conn, number).await?;
            notes.push(json!({ "number": number, "kind": "repeated", "reason": reason, "distance_m": c.distance_m, "held_by": held_by(&c.properties) }));
        }
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO cadastre.beacon (number, geom, accuracy, is_control, duplicate_seq) \
             VALUES ($1, ST_SetSRID(ST_MakePoint($2, $3), 32632), 0.05, false, $4) RETURNING id",
        )
        .bind(number)
        .bind(p.e)
        .bind(p.n)
        .bind(repeat)
        .fetch_one(&mut *conn)
        .await?;
        beacon_of[i] = Some(id);
        added.push(number.clone());
    }

    for (i, p) in points.iter().enumerate() {
        let Some(beacon_id) = beacon_of[i] else { continue };
        for (parcel_id, seq) in &p.corners {
            sqlx::query(
                "INSERT INTO cadastre.parcel_corner (parcel_id, seq, beacon_id) VALUES ($1, $2, $3) \
                 ON CONFLICT (parcel_id, seq) DO UPDATE SET beacon_id = EXCLUDED.beacon_id",
            )
            .bind(parcel_id)
            .bind(seq)
            .bind(beacon_id)
            .execute(&mut *conn)
            .await?;
        }
    }
    Ok(Applied { added, notes })
}

/// Before committing: which typed numbers another beacon already has, with the properties holding
/// them, so the digitizer can check them. Nothing is changed.
#[utoipa::path(post, path = "/api/v1/wf/jobs/{id}/editing/beacon-check", tag = "wf", params(("id" = i32, Path, description = "Job instance id")),
    request_body = WfCommitRequest, responses((status = 200, body = [BeaconConflict])))]
pub async fn check(State(state): State<AppState>, Path(id): Path<i32>, Json(req): Json<WfCommitRequest>) -> ApiResult<Json<Vec<BeaconConflict>>> {
    let job_id: Option<Uuid> = sqlx::query_scalar("SELECT cadastre_job_id FROM cadastre.wf_job_instance WHERE id = $1").bind(id)
        .fetch_optional(&state.pool).await?.ok_or_else(|| AppError::NotFound(format!("job instance {id}")))?;
    let Some(job_id) = job_id else { return Ok(Json(Vec::new())) };
    let mut conn = state.pool.acquire().await?;
    let tol = tolerance(&mut conn).await?;
    let limit = adjoining_limit(&mut conn).await?;
    let points = corner_points(&mut conn, job_id, tol).await?;
    let mut out = Vec::new();
    for b in &req.beacons {
        let number = b.number.trim();
        if number.is_empty() {
            continue;
        }
        let Some(i) = points.iter().position(|p| p.beacon.is_none() && (p.e - b.easting).hypot(p.n - b.northing) <= tol) else { continue };
        out.extend(conflict(&mut conn, i, &points[i], number, limit).await?);
    }
    Ok(Json(out))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct BeaconNumberChange {
    /// A committed parcel, and the corner of it whose beacon number is wrong.
    pub upin: String,
    pub easting: f64,
    pub northing: f64,
    /// The right number.
    pub number: String,
    pub reason: String,
    /// The digitizer has seen the property already holding this number and goes on.
    #[serde(default)]
    pub confirm_repeat: bool,
}

/// Which of these parcels are linked in the Registry (the rule of Correct & Recommit).
async fn linked(conn: &mut sqlx::PgConnection, upins: &[String]) -> ApiResult<Vec<String>> {
    Ok(sqlx::query_scalar(
        "SELECT u FROM unnest($1::text[]) u WHERE EXISTS (SELECT 1 FROM cadastre.parcel_attribute a WHERE a.upin = u AND (a.linkage_status = 1 OR a.lrs_id IS NOT NULL)) \
           OR EXISTS (SELECT 1 FROM cadastre.registry_property r WHERE r.upin = u) ORDER BY 1")
        .bind(upins).fetch_all(conn).await?)
}

/// Correct the beacon number of a committed corner, at the digitizing stage only: refused while
/// the parcel, or a parcel sharing the beacon, is linked in the Registry (unlink it there first).
/// The parcel's shape and UPIN are not touched. A beacon shared with other parcels changes for
/// them as well: they are listed in the answer.
#[utoipa::path(post, path = "/api/v1/parcels/beacon-number", tag = "parcels", request_body = BeaconNumberChange,
    responses((status = 200), (status = 400, description = "No number or reason"), (status = 404, description = "Not a corner of that parcel"), (status = 409, description = "Linked in the Registry, or another property holds the number")))]
pub async fn change_number(State(state): State<AppState>, user: AuthUser, Json(req): Json<BeaconNumberChange>) -> ApiResult<Json<Value>> {
    user.require_role("editor")?;
    let actor = crate::routes::wf::resolve_actor(&state, &user).await?;
    let (number, reason) = (req.number.trim().to_string(), req.reason.trim().to_string());
    if number.is_empty() || reason.is_empty() {
        return Err(AppError::BadRequest("give the beacon number and the reason".into()));
    }
    let upin = req.upin.trim().to_uppercase();
    let mut tx = state.pool.begin().await?;
    let tol = tolerance(&mut tx).await?;
    let corner: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM cadastre.parcel WHERE parcel_number = $1 AND status = 'posted' \
           AND ST_DWithin(ST_Boundary(geom), ST_SetSRID(ST_MakePoint($2, $3), ST_SRID(geom)), $4))")
        .bind(&upin).bind(req.easting).bind(req.northing).bind(tol).fetch_one(&mut *tx).await?;
    if !corner {
        return Err(AppError::NotFound(format!("{:.3} {:.3} is not a corner of committed parcel {upin}", req.easting, req.northing)));
    }
    let here = sqlx::query("SELECT id, number FROM cadastre.beacon WHERE ST_DWithin(geom, ST_SetSRID(ST_MakePoint($1, $2), 32632), $3) \
                            ORDER BY geom <-> ST_SetSRID(ST_MakePoint($1, $2), 32632) LIMIT 1")
        .bind(req.easting).bind(req.northing).bind(tol).fetch_optional(&mut *tx).await?;
    let (id, old): (Option<Uuid>, Option<String>) = match &here {
        Some(r) => (Some(r.get("id")), Some(r.get("number"))),
        None => (None, None),
    };
    let mut affected = vec![upin.clone()];
    if let Some(id) = id {
        affected.extend(properties_at(&mut tx, id).await?.into_iter().map(|p| p.upin));
    }
    let held = linked(&mut tx, &affected).await?;
    if !held.is_empty() {
        return Err(AppError::Conflict(format!("{} is linked in the Registry: unlink it there first", held.join(", "))));
    }
    let mut repeat = 0;
    if let Some((other, be, bn)) = nearest_with_number(&mut tx, &number, req.easting, req.northing, id).await? {
        let holders = properties_at(&mut tx, other).await?;
        if !req.confirm_repeat {
            return Err(AppError::Conflict(format!("Beacon {number} already exists on {}, at {be:.3} {bn:.3}, {:.2} m from this corner.",
                held_by(&holders), (req.easting - be).hypot(req.northing - bn))));
        }
        repeat = next_repeat(&mut tx, &number).await?;
    }
    let id = match id {
        Some(id) => {
            sqlx::query("UPDATE cadastre.beacon SET number = $2, duplicate_seq = $3, updated_at = now() WHERE id = $1")
                .bind(id).bind(&number).bind(repeat).execute(&mut *tx).await?;
            id
        }
        None => sqlx::query_scalar("INSERT INTO cadastre.beacon (number, geom, accuracy, is_control, duplicate_seq) \
                                    VALUES ($1, ST_SetSRID(ST_MakePoint($2, $3), 32632), 0.05, false, $4) RETURNING id")
            .bind(&number).bind(req.easting).bind(req.northing).bind(repeat).fetch_one(&mut *tx).await?,
    };
    let shared: Vec<BeaconProperty> = properties_at(&mut tx, id).await?.into_iter().filter(|p| p.upin != upin).collect();
    audit::append(&mut tx, &actor.label, "beacon_number_changed", "upin", Some(&upin), json!({
        "from": old, "to": number, "reason": reason, "easting": req.easting, "northing": req.northing,
        "repeat": repeat > 0, "also_on": held_by(&shared),
    })).await?;
    tx.commit().await?;
    Ok(Json(json!({ "upin": upin, "from": old, "number": number, "also_on": shared })))
}
