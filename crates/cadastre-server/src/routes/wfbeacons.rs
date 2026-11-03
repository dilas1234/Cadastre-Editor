//! Beacon numbers at commit (Aumentum parity). When a job's parcels are committed the officer
//! enters the beacon number of every corner, as written on the surveyor's point list (e.g.
//! `CA6782`). The system never generates a beacon number. A corner lying on a beacon that is
//! already in the fabric (a shared boundary pillar) is that beacon. Corners are grouped into
//! points: two new parcels sharing a corner share one beacon.

use std::collections::HashMap;

use axum::extract::{Path, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use utoipa::ToSchema;
use uuid::Uuid;

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
}

#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct WfCommitRequest {
    /// Beacon numbers for the new corner points.
    #[serde(default)]
    pub beacons: Vec<WfBeaconNumber>,
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
pub async fn apply(conn: &mut sqlx::PgConnection, job_id: Uuid, req: &WfCommitRequest) -> ApiResult<Vec<String>> {
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

    let mut added = Vec::new();
    let mut beacon_of: Vec<Option<Uuid>> = points.iter().map(|p| p.beacon.as_ref().map(|b| b.0)).collect();
    for (i, p) in points.iter().enumerate() {
        let Some(number) = typed[i].as_ref().filter(|_| p.beacon.is_none()) else { continue };
        // A number already in the fabric belongs to a beacon elsewhere (this corner is on none).
        if let Some(row) = sqlx::query(
            "SELECT number, ST_X(geom) AS e, ST_Y(geom) AS n FROM cadastre.beacon WHERE upper(number) = upper($1) LIMIT 1",
        )
        .bind(number)
        .fetch_optional(&mut *conn)
        .await?
        {
            let (e, n): (f64, f64) = (row.get("e"), row.get("n"));
            return Err(AppError::Conflict(format!(
                "beacon {} already exists at {e:.3} {n:.3}, {:.2} m from point {} ({:.3} {:.3}); check the number",
                row.get::<String, _>("number"),
                (p.e - e).hypot(p.n - n),
                i + 1,
                p.e,
                p.n
            )));
        }
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO cadastre.beacon (number, geom, accuracy, is_control) \
             VALUES ($1, ST_SetSRID(ST_MakePoint($2, $3), 32632), 0.05, false) RETURNING id",
        )
        .bind(number)
        .bind(p.e)
        .bind(p.n)
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
    Ok(added)
}
