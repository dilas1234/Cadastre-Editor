//! Catalog endpoints: read-only browsing of the configured folder connections.

use axum::extract::{Query, State};
use axum::Json;
use serde::Deserialize;
use utoipa::IntoParams;

use crate::catalog::{self, CatalogItem};
use crate::error::{ApiResult, AppError};
use crate::models::CatalogFolderDto;
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
