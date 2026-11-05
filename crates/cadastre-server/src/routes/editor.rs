//! Editor web app: static assets embedded into the binary (self-contained deployment).
//!
//! The editor is a browser app (MapLibre) served from `/editor`; it binds to the existing REST
//! API. Embedding the assets keeps the server a single deployable binary.

use axum::extract::Path;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "web/"]
struct WebAssets;

/// Serve the editor shell (`/editor`).
pub async fn index() -> Response {
    serve("index.html")
}

/// Serve an editor asset (`/editor/{path}`).
pub async fn asset(Path(path): Path<String>) -> Response {
    serve(&path)
}

pub(crate) fn serve(path: &str) -> Response {
    match WebAssets::get(path) {
        Some(file) => (
            [(header::CONTENT_TYPE, content_type(path))],
            file.data.into_owned(),
        )
            .into_response(),
        None => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

fn content_type(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "application/javascript",
        Some("css") => "text/css",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("pbf") => "application/x-protobuf",
        _ => "application/octet-stream",
    }
}
