//! Beacon endpoints.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use sqlx::Row;
use uuid::Uuid;

use crate::auth::AuthUser;
use crate::error::{ApiResult, AppError};
use crate::models::{BeaconDto, NewBeacon};
use crate::state::AppState;

const SELECT_COLS: &str = "id, number, ST_X(geom) AS easting, ST_Y(geom) AS northing, \
     accuracy, is_control, created_at, updated_at";

fn row_to_dto(row: &sqlx::postgres::PgRow) -> BeaconDto {
    BeaconDto {
        id: row.get("id"),
        number: row.get("number"),
        easting: row.get("easting"),
        northing: row.get("northing"),
        accuracy: row.get("accuracy"),
        is_control: row.get("is_control"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

/// List all beacons, ordered by number.
#[utoipa::path(
    get, path = "/api/v1/beacons", tag = "beacons",
    responses((status = 200, body = [BeaconDto]))
)]
pub async fn list_beacons(State(state): State<AppState>) -> ApiResult<Json<Vec<BeaconDto>>> {
    let sql = format!("SELECT {SELECT_COLS} FROM cadastre.beacon ORDER BY number");
    let rows = sqlx::query(&sql).fetch_all(&state.pool).await?;
    Ok(Json(rows.iter().map(row_to_dto).collect()))
}

/// Query params for the beacon search.
#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct BeaconSearch {
    /// Text the beacon number contains (case-insensitive); empty = any.
    pub q: Option<String>,
    /// UTM 32N easting to rank by distance from (with `n`).
    pub e: Option<f64>,
    /// UTM 32N northing to rank by distance from (with `e`).
    pub n: Option<f64>,
    /// Max results (default 50, at most 500).
    pub limit: Option<i64>,
    /// `true`: control points only; `false`: survey beacons only; absent: both.
    pub control: Option<bool>,
}

/// Search beacons (control points and survey beacons) by number, for pickers: an exact number
/// first, then numbers starting with the text, then containing it; nearest first when `e`/`n`
/// are given, else by number.
#[utoipa::path(
    get, path = "/api/v1/beacons/search", tag = "beacons",
    params(BeaconSearch),
    responses((status = 200, body = [BeaconDto]))
)]
pub async fn search_beacons(
    State(state): State<AppState>,
    Query(q): Query<BeaconSearch>,
) -> ApiResult<Json<Vec<BeaconDto>>> {
    let text = q.q.as_deref().map(str::trim).unwrap_or("").to_string();
    let limit = q.limit.unwrap_or(50).clamp(1, 500);
    let near = match (q.e, q.n) {
        (Some(e), Some(n)) if e.is_finite() && n.is_finite() => Some((e, n)),
        _ => None,
    };
    let alt = plagis_alternate(&text).unwrap_or_default();
    let like = format!("%{}%", text.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"));
    let sql = format!(
        "SELECT {SELECT_COLS} FROM cadastre.beacon \
         WHERE ($1 = '' OR number ILIKE $2 OR ($6 <> '' AND upper(number) = upper($6))) AND ($7::boolean IS NULL OR is_control = $7) \
         ORDER BY (upper(number) = upper($1) OR ($6 <> '' AND upper(number) = upper($6))) DESC, starts_with(upper(number), upper($1)) DESC, \
                  CASE WHEN $3::float8 IS NULL THEN 0 ELSE ST_Distance(geom, ST_SetSRID(ST_MakePoint($3, $4), 32632)) END, \
                  length(number), number \
         LIMIT $5"
    );
    let rows = sqlx::query(&sql)
        .bind(&text)
        .bind(&like)
        .bind(near.map(|p| p.0))
        .bind(near.map(|p| p.1))
        .bind(limit)
        .bind(&alt)
        .bind(q.control)
        .fetch_all(&state.pool)
        .await?;
    Ok(Json(rows.iter().map(row_to_dto).collect()))
}

/// The other way of writing a beacon number used at PLAGIS: the surveyor's `CA6782` was entered
/// in Aumentum as `C6782A` (first letter, digits, second letter), and back. `None` otherwise.
pub fn plagis_alternate(number: &str) -> Option<String> {
    let s: Vec<char> = number.trim().chars().collect();
    let digits = |cs: &[char]| !cs.is_empty() && cs.iter().all(|c| c.is_ascii_digit());
    match s.as_slice() {
        [a, b, rest @ ..] if a.is_ascii_alphabetic() && b.is_ascii_alphabetic() && digits(rest) => {
            Some(format!("{a}{}{b}", rest.iter().collect::<String>()))
        }
        [a, mid @ .., z] if a.is_ascii_alphabetic() && z.is_ascii_alphabetic() && digits(mid) => {
            Some(format!("{a}{z}{}", mid.iter().collect::<String>()))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::plagis_alternate;

    #[test]
    fn surveyor_and_aumentum_forms_map_to_each_other() {
        assert_eq!(plagis_alternate("CA6782").as_deref(), Some("C6782A"));
        assert_eq!(plagis_alternate("C6782A").as_deref(), Some("CA6782"));
        assert_eq!(plagis_alternate(" rx1271 ").as_deref(), Some("r1271x"));
        assert_eq!(plagis_alternate("SCS01"), None);
        assert_eq!(plagis_alternate("PL58813"), Some("P58813L".into()));
        assert_eq!(plagis_alternate("XSK100"), None);
        assert_eq!(plagis_alternate("AB"), None);
    }
}

/// Get one beacon by id.
#[utoipa::path(
    get, path = "/api/v1/beacons/{id}", tag = "beacons",
    params(("id" = Uuid, Path, description = "Beacon id")),
    responses((status = 200, body = BeaconDto), (status = 404, description = "Not found"))
)]
pub async fn get_beacon(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<BeaconDto>> {
    let sql = format!("SELECT {SELECT_COLS} FROM cadastre.beacon WHERE id = $1");
    let row = sqlx::query(&sql)
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("beacon {id}")))?;
    Ok(Json(row_to_dto(&row)))
}

/// Create a beacon (requires the `editor` role).
#[utoipa::path(
    post, path = "/api/v1/beacons", tag = "beacons",
    request_body = NewBeacon,
    responses(
        (status = 201, body = BeaconDto),
        (status = 400, description = "Duplicate beacon number"),
        (status = 403, description = "Requires editor role"),
    )
)]
pub async fn create_beacon(
    State(state): State<AppState>,
    user: AuthUser,
    Json(new): Json<NewBeacon>,
) -> ApiResult<(StatusCode, Json<BeaconDto>)> {
    user.require_role("editor")?;

    let sql = format!(
        "INSERT INTO cadastre.beacon (number, geom, accuracy, is_control) \
         VALUES ($1, ST_SetSRID(ST_MakePoint($2, $3), 32632), $4, $5) \
         RETURNING {SELECT_COLS}"
    );
    let result = sqlx::query(&sql)
        .bind(&new.number)
        .bind(new.easting)
        .bind(new.northing)
        .bind(new.accuracy.unwrap_or(0.05))
        .bind(new.is_control.unwrap_or(false))
        .fetch_one(&state.pool)
        .await;

    match result {
        Ok(row) => Ok((StatusCode::CREATED, Json(row_to_dto(&row)))),
        Err(e) if is_unique_violation(&e) => Err(AppError::BadRequest(format!(
            "beacon number '{}' already exists",
            new.number
        ))),
        Err(e) => Err(e.into()),
    }
}

/// Whether a sqlx error is a Postgres unique-constraint violation (SQLSTATE 23505).
fn is_unique_violation(e: &sqlx::Error) -> bool {
    e.as_database_error()
        .and_then(|d| d.code())
        .is_some_and(|c| c == "23505")
}
