//! The current user (`GET /api/v1/me`) — drives the editor's "Connected as <user>" title and
//! which workflow actions it offers.

use axum::Json;

use crate::auth::AuthUser;
use crate::models::MeDto;

/// Who the caller is, as resolved from their token (or the dev user when auth is disabled).
#[utoipa::path(get, path = "/api/v1/me", tag = "auth",
    responses((status = 200, body = MeDto), (status = 401, description = "Missing or invalid token")))]
pub async fn me(user: AuthUser) -> Json<MeDto> {
    Json(MeDto {
        username: user.username.clone().unwrap_or_else(|| user.subject.clone()),
        subject: user.subject,
        roles: user.roles,
    })
}
