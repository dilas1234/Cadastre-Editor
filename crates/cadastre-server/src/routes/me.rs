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

/// What the editor needs to sign users in: whether sign-in is required, the Keycloak realm as
/// browsers reach it, and the editor's client id. Public (needed before signing in).
#[derive(serde::Serialize, utoipa::ToSchema)]
pub struct AuthConfigDto {
    pub enabled: bool,
    /// This server's own sign-in page is in use (`CADASTRE_AUTH=local`).
    pub local: bool,
    /// Realm URL for browsers (CADASTRE_KEYCLOAK_PUBLIC_ISSUER, else the issuer).
    pub issuer: Option<String>,
    /// The editor's OIDC client (CADASTRE_KEYCLOAK_CLIENT, default `cadastre-editor`).
    pub client_id: String,
}

#[utoipa::path(get, path = "/api/v1/auth/config", tag = "auth", responses((status = 200, body = AuthConfigDto)))]
pub async fn auth_config(axum::extract::State(state): axum::extract::State<crate::state::AppState>) -> Json<AuthConfigDto> {
    Json(AuthConfigDto {
        enabled: state.config.auth_mode == crate::config::AuthMode::Enabled,
        local: state.config.auth_mode == crate::config::AuthMode::Local,
        issuer: std::env::var("CADASTRE_KEYCLOAK_PUBLIC_ISSUER").ok().filter(|s| !s.is_empty()).or_else(|| state.config.keycloak_issuer.clone()),
        client_id: std::env::var("CADASTRE_KEYCLOAK_CLIENT").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "cadastre-editor".into()),
    })
}
