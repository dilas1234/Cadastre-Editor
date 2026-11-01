//! Catalog endpoints: read-only browsing of the configured folder connections.

use axum::extract::{Query, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use utoipa::IntoParams;

use crate::catalog::{self, CatalogItem, FeatureClass};
use crate::error::{ApiResult, AppError};
use crate::models::{CatalogDatabaseDto, CatalogFolderDto, RasterInfoDto};
use crate::state::AppState;

/// The folder connections shown under "Folder Connections".
#[utoipa::path(get, path = "/api/v1/catalog", tag = "catalog",
    responses((status = 200, body = [CatalogFolderDto])))]
pub async fn folders(State(state): State<AppState>) -> Json<Vec<CatalogFolderDto>> {
    Json(state.config.catalog_folders.iter().enumerate()
        .map(|(id, f)| CatalogFolderDto { id, name: f.name.clone() })
        .collect())
}

/// Which location to list.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ItemsQuery {
    /// Folder connection id (from `GET /api/v1/catalog`).
    pub folder: usize,
    /// Path inside it, `/`-separated (empty = the connection itself). A raster path lists its bands.
    #[serde(default)]
    pub path: String,
}

/// List a Catalog location the way ArcCatalog shows it (GIS items only, sidecars hidden).
#[utoipa::path(get, path = "/api/v1/catalog/items", tag = "catalog", params(ItemsQuery),
    responses((status = 200, body = [CatalogItem]), (status = 400, description = "Invalid path"),
              (status = 404, description = "Unknown folder connection or path")))]
pub async fn items(State(state): State<AppState>, Query(q): Query<ItemsQuery>) -> ApiResult<Json<Vec<CatalogItem>>> {
    let folder = state.config.catalog_folders.get(q.folder)
        .ok_or_else(|| AppError::NotFound(format!("no folder connection {}", q.folder)))?;
    let path = catalog::resolve(&folder.path, &q.path)
        .ok_or_else(|| AppError::BadRequest("invalid catalog path".to_string()))?;
    // Network shares can be slow: keep filesystem IO off the async workers.
    let listed = tokio::task::spawn_blocking(move || catalog::list(&path))
        .await
        .map_err(|e| AppError::BadRequest(format!("catalog listing failed: {e}")))?;
    match listed {
        Ok(items) => Ok(Json(items)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(AppError::NotFound(format!("not found: {}", q.path))),
        Err(e) => Err(AppError::BadRequest(format!("cannot read {}: {e}", q.path))),
    }
}

/// The database (geodatabase) connections shown under "Database Connections".
#[utoipa::path(get, path = "/api/v1/catalog/databases", tag = "catalog",
    responses((status = 200, body = [CatalogDatabaseDto])))]
pub async fn databases(State(state): State<AppState>) -> Json<Vec<CatalogDatabaseDto>> {
    Json(state.config.catalog_databases.iter().enumerate()
        .map(|(id, d)| CatalogDatabaseDto { id, name: d.name.clone() })
        .collect())
}

/// Which database connection to list.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct DbItemsQuery {
    /// Database connection id (from `GET /api/v1/catalog/databases`).
    pub db: usize,
}

/// List a geodatabase connection's feature classes.
#[utoipa::path(get, path = "/api/v1/catalog/database/items", tag = "catalog", params(DbItemsQuery),
    responses((status = 200, body = [FeatureClass]), (status = 404, description = "Unknown connection")))]
pub async fn database_items(
    State(state): State<AppState>,
    Query(q): Query<DbItemsQuery>,
) -> ApiResult<Json<Vec<FeatureClass>>> {
    let conn = state.config.catalog_databases.get(q.db)
        .ok_or_else(|| AppError::NotFound(format!("no database connection {}", q.db)))?
        .clone();
    let fcs = tokio::task::spawn_blocking(move || catalog::feature_classes(&conn))
        .await
        .map_err(|e| AppError::BadRequest(format!("listing failed: {e}")))?
        .map_err(|e| AppError::BadRequest(format!("cannot read snapshot: {e}")))?;
    Ok(Json(fcs))
}

/// A raster inside a folder connection: used to add a `.tif` to the map (drag from Catalog to TOC).
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct RasterQuery {
    /// Folder connection id (from `GET /api/v1/catalog`).
    pub folder: usize,
    /// Path to the raster inside it, `/`-separated.
    pub path: String,
    /// Target longest side (pixels) for the overview PNG; clamped to 256..=4096. Default 2048.
    #[serde(default)]
    pub max: Option<u32>,
}

/// Resolve a raster query to an absolute filesystem path under its folder connection.
fn raster_path(state: &AppState, q: &RasterQuery) -> ApiResult<std::path::PathBuf> {
    let folder = state.config.catalog_folders.get(q.folder)
        .ok_or_else(|| AppError::NotFound(format!("no folder connection {}", q.folder)))?;
    catalog::resolve(&folder.path, &q.path)
        .ok_or_else(|| AppError::BadRequest("invalid catalog path".to_string()))
}

/// Georeferencing + previewability for a raster, read from its header.
#[utoipa::path(get, path = "/api/v1/catalog/raster/info", tag = "catalog", params(RasterQuery),
    responses((status = 200, body = RasterInfoDto), (status = 400, description = "Not a readable georeferenced raster"),
              (status = 404, description = "Unknown folder or path")))]
pub async fn raster_info(State(state): State<AppState>, Query(q): Query<RasterQuery>) -> ApiResult<Json<RasterInfoDto>> {
    let path = raster_path(&state, &q)?;
    let info = tokio::task::spawn_blocking(move || crate::raster::read_info(&path))
        .await
        .map_err(|e| AppError::BadRequest(format!("raster read failed: {e}")))?
        .map_err(|e| AppError::BadRequest(format!("cannot read raster: {e}")))?;
    Ok(Json(info.into()))
}

/// Render an overview PNG for a raster (a ground-overlay the browser places by corner coords).
#[utoipa::path(get, path = "/api/v1/catalog/raster/overview", tag = "catalog", params(RasterQuery),
    responses((status = 200, description = "PNG image", content_type = "image/png"),
              (status = 422, description = "Too large to preview without tiling"),
              (status = 404, description = "Unknown folder or path")))]
pub async fn raster_overview(State(state): State<AppState>, Query(q): Query<RasterQuery>) -> ApiResult<Response> {
    let path = raster_path(&state, &q)?;
    let max = q.max.unwrap_or(2048).clamp(256, 4096);
    let png = tokio::task::spawn_blocking(move || crate::raster::render_overview(&path, max))
        .await
        .map_err(|e| AppError::BadRequest(format!("raster render failed: {e}")))?
        .map_err(|e| AppError::UnprocessableEntity(format!("cannot preview raster: {e}")))?;
    Ok((
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, "private, max-age=86400"),
        ],
        png.0,
    )
        .into_response())
}

/// Read a shapefile's geometry as GeoJSON (native CRS) + best-effort EPSG, so the editor can add
/// it to the map (drag a `.shp` from the Catalog to the TOC).
#[utoipa::path(get, path = "/api/v1/catalog/shapefile", tag = "catalog", params(RasterQuery),
    responses((status = 200, description = "Shapefile as a GeoJSON FeatureCollection + CRS"),
              (status = 400, description = "Not a readable shapefile"),
              (status = 404, description = "Unknown folder or path")))]
pub async fn shapefile(State(state): State<AppState>, Query(q): Query<RasterQuery>) -> ApiResult<Json<crate::shp::ShapeData>> {
    let path = raster_path(&state, &q)?;
    let data = tokio::task::spawn_blocking(move || crate::shp::read_shapefile(&path, 400_000))
        .await
        .map_err(|e| AppError::BadRequest(format!("shapefile read failed: {e}")))?
        .map_err(|e| AppError::BadRequest(format!("cannot read shapefile: {e}")))?;
    Ok(Json(data))
}
