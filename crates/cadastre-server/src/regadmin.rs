//! Registry (LRS43 / Alfresco) user-administration write-path (feature `registry-admin`).
//!
//! The Cadastre reads the Registry read-only over SOAP ([`crate::lrs`]). This is the one place it
//! *writes* to it, and only through the `plagis_admin` stored procedures installed on the
//! Registry's SQL Server (see `tools/registry-admin/`), never the Alfresco tables directly. It
//! connects as the dedicated `plagis_admin_app` login, which can do nothing but `EXECUTE` those
//! procedures.
//!
//! Each procedure validates its own input and raises a `THROW` with a message meant to be read by
//! a person ("Username already exists.", "Group already exists.", …); those surface here as
//! [`RegAdminError::Refused`] and are shown to the administrator. Alfresco caches users and groups
//! in memory, so a change does not take effect until the Registry (JBoss) service is restarted.

use tiberius::{AuthMethod, Client, Config as TdsConfig, EncryptionLevel, Row};
use tokio::net::TcpStream;
use tokio_util::compat::{Compat, TokioAsyncWriteCompatExt};

use crate::config::RegistrySqlConfig;

/// What a Registry-admin call can fail with.
#[derive(Debug, thiserror::Error)]
pub enum RegAdminError {
    /// The dashboard is not configured (`CADASTRE_REGISTRY_SQL_*` unset).
    #[error("Registry user administration is not configured on this server.")]
    NotConfigured,
    /// Could not reach the Registry's SQL Server.
    #[error("could not reach the Registry database: {0}")]
    Connect(String),
    /// A procedure refused the request — its message is safe to show the administrator.
    #[error("{0}")]
    Refused(String),
    /// Any other database error (logged, not shown verbatim).
    #[error("Registry database error: {0}")]
    Db(String),
}

/// A procedure's `THROW` (a validation refusal) is shown to the user; anything else is a `Db`
/// error. The transport/connect errors are mapped by the caller.
fn map_err(e: tiberius::error::Error) -> RegAdminError {
    match e {
        tiberius::error::Error::Server(token) => RegAdminError::Refused(token.message().to_string()),
        other => RegAdminError::Db(other.to_string()),
    }
}

/// One Registry user (a row of `usp_list_users`).
#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct RegistryUser {
    pub user_name: String,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub office_id: Option<String>,
    /// The login's enabled flag; `None` when the person has no login node.
    pub enabled: Option<bool>,
    /// Whether a login (not just a person) exists.
    pub has_login: bool,
    /// The groups ("units") the user belongs to, e.g. `GROUP_Title Deed Plan`.
    pub groups: Vec<String>,
}

/// One Registry group ("unit") (a row of `usp_list_groups`).
#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct RegistryGroup {
    pub group_name: String,
    pub display_name: Option<String>,
    pub members: i32,
}

/// Create a user by cloning a reference user, optionally into extra groups.
#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct CreateUser {
    pub user_name: String,
    pub first_name: String,
    pub last_name: String,
    pub password: String,
    /// The existing user whose groups + office are copied (e.g. `cgugin`).
    pub ref_user: String,
    /// Optional comma-separated groups to add on top, e.g. `GROUP_Title Deed Plan,GROUP_LRS`.
    #[serde(default)]
    pub extra_groups: Option<String>,
}

/// Create a group ("unit") by cloning a reference group's structure.
#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct CreateGroup {
    /// The display name, without the `GROUP_` prefix (e.g. `Title Deed Plan`).
    pub display_name: String,
    /// Structure to copy; defaults to `GROUP_Map Compilation Officer` when omitted.
    #[serde(default)]
    pub ref_group: Option<String>,
}

/// Fields of `usp_update_user`; `None` leaves a field unchanged.
#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct UpdateUser {
    #[serde(default)]
    pub first_name: Option<String>,
    #[serde(default)]
    pub last_name: Option<String>,
    #[serde(default)]
    pub office_id: Option<String>,
}

type Conn = Client<Compat<TcpStream>>;

/// Connect to the Registry's SQL Server as the admin login. SQL authentication; the server's
/// certificate is trusted (same as the importer — a LAN host, often a self-signed cert). With
/// `encrypt` off there is no TLS at all: see [`RegistrySqlConfig::encrypt`].
async fn connect(cfg: &RegistrySqlConfig) -> Result<Conn, RegAdminError> {
    let mut config = TdsConfig::new();
    config.host(&cfg.host);
    config.port(cfg.port);
    config.database(&cfg.database);
    config.authentication(AuthMethod::sql_server(&cfg.username, &cfg.password));
    config.trust_cert();
    if !cfg.encrypt {
        config.encryption(EncryptionLevel::NotSupported);
    }

    let tcp = TcpStream::connect(config.get_addr()).await.map_err(|e| RegAdminError::Connect(e.to_string()))?;
    tcp.set_nodelay(true).ok();
    Client::connect(config, tcp.compat_write()).await.map_err(|e| RegAdminError::Connect(e.to_string()))
}

/// Run a statement that returns no rows of interest, draining every result set so that a `THROW`
/// raised mid-batch surfaces as an error.
async fn run(conn: &mut Conn, sql: &str, params: &[&dyn tiberius::ToSql]) -> Result<(), RegAdminError> {
    let stream = conn.query(sql, params).await.map_err(map_err)?;
    stream.into_results().await.map_err(map_err)?;
    Ok(())
}

fn text(row: &Row, col: &str) -> Option<String> {
    row.get::<&str, _>(col).map(str::to_string)
}

fn user_from_row(row: &Row) -> RegistryUser {
    let groups = row
        .get::<&str, _>("groups")
        .map(|g| g.split(", ").filter(|s| !s.is_empty()).map(str::to_string).collect())
        .unwrap_or_default();
    RegistryUser {
        user_name: text(row, "userName").unwrap_or_default(),
        first_name: text(row, "firstName"),
        last_name: text(row, "lastName"),
        office_id: text(row, "officeId"),
        enabled: row.get::<i32, _>("enabled").map(|v| v != 0),
        has_login: row.get::<i32, _>("has_login").unwrap_or(0) != 0,
        groups,
    }
}

/// Users, with name, office, enabled flag and groups. `search` matches username/first/last.
pub async fn list_users(cfg: &RegistrySqlConfig, search: Option<&str>) -> Result<Vec<RegistryUser>, RegAdminError> {
    let mut conn = connect(cfg).await?;
    let rows = conn
        .query("EXEC plagis_admin.usp_list_users @Search = @P1", &[&search])
        .await
        .map_err(map_err)?
        .into_first_result()
        .await
        .map_err(map_err)?;
    Ok(rows.iter().map(user_from_row).collect())
}

/// Groups ("units"), with display name and member count.
pub async fn list_groups(cfg: &RegistrySqlConfig) -> Result<Vec<RegistryGroup>, RegAdminError> {
    let mut conn = connect(cfg).await?;
    let rows = conn
        .query("EXEC plagis_admin.usp_list_groups", &[])
        .await
        .map_err(map_err)?
        .into_first_result()
        .await
        .map_err(map_err)?;
    Ok(rows
        .iter()
        .map(|r| RegistryGroup {
            group_name: text(r, "group_name").unwrap_or_default(),
            display_name: text(r, "display_name"),
            members: r.get::<i32, _>("members").unwrap_or(0),
        })
        .collect())
}

/// The groups of one user (raises if the user is not found).
pub async fn user_groups(cfg: &RegistrySqlConfig, user: &str) -> Result<Vec<String>, RegAdminError> {
    let mut conn = connect(cfg).await?;
    let rows = conn
        .query("EXEC plagis_admin.usp_user_groups @UserName = @P1", &[&user])
        .await
        .map_err(map_err)?
        .into_first_result()
        .await
        .map_err(map_err)?;
    Ok(rows.iter().filter_map(|r| text(r, "group_name")).collect())
}

pub async fn create_user(cfg: &RegistrySqlConfig, req: &CreateUser, actor: &str) -> Result<(), RegAdminError> {
    let mut conn = connect(cfg).await?;
    let extra = req.extra_groups.as_deref().filter(|s| !s.trim().is_empty());
    run(
        &mut conn,
        "EXEC plagis_admin.usp_create_user @UserName=@P1, @FirstName=@P2, @LastName=@P3, \
         @Password=@P4, @RefUser=@P5, @ExtraGroups=@P6, @Actor=@P7",
        &[&req.user_name, &req.first_name, &req.last_name, &req.password, &req.ref_user, &extra, &actor],
    )
    .await
}

pub async fn create_group(cfg: &RegistrySqlConfig, req: &CreateGroup, actor: &str) -> Result<(), RegAdminError> {
    let mut conn = connect(cfg).await?;
    // Omit @RefGroup when not given, so the procedure's own default applies rather than NULL.
    match req.ref_group.as_deref().filter(|s| !s.trim().is_empty()) {
        Some(reference) => {
            run(
                &mut conn,
                "EXEC plagis_admin.usp_create_group @DisplayName=@P1, @RefGroup=@P2, @Actor=@P3",
                &[&req.display_name, &reference, &actor],
            )
            .await
        }
        None => {
            run(
                &mut conn,
                "EXEC plagis_admin.usp_create_group @DisplayName=@P1, @Actor=@P2",
                &[&req.display_name, &actor],
            )
            .await
        }
    }
}

pub async fn add_to_group(cfg: &RegistrySqlConfig, user: &str, group: &str, actor: &str) -> Result<(), RegAdminError> {
    let mut conn = connect(cfg).await?;
    run(
        &mut conn,
        "EXEC plagis_admin.usp_add_to_group @UserName=@P1, @GroupName=@P2, @Actor=@P3",
        &[&user, &group, &actor],
    )
    .await
}

pub async fn remove_from_group(cfg: &RegistrySqlConfig, user: &str, group: &str, actor: &str) -> Result<(), RegAdminError> {
    let mut conn = connect(cfg).await?;
    run(
        &mut conn,
        "EXEC plagis_admin.usp_remove_from_group @UserName=@P1, @GroupName=@P2, @Actor=@P3",
        &[&user, &group, &actor],
    )
    .await
}

pub async fn reset_password(cfg: &RegistrySqlConfig, user: &str, new_password: &str, actor: &str) -> Result<(), RegAdminError> {
    let mut conn = connect(cfg).await?;
    run(
        &mut conn,
        "EXEC plagis_admin.usp_reset_password @UserName=@P1, @NewPassword=@P2, @Actor=@P3",
        &[&user, &new_password, &actor],
    )
    .await
}

pub async fn set_enabled(cfg: &RegistrySqlConfig, user: &str, enabled: bool, actor: &str) -> Result<(), RegAdminError> {
    let mut conn = connect(cfg).await?;
    run(
        &mut conn,
        "EXEC plagis_admin.usp_set_user_enabled @UserName=@P1, @Enabled=@P2, @Actor=@P3",
        &[&user, &enabled, &actor],
    )
    .await
}

pub async fn update_user(cfg: &RegistrySqlConfig, user: &str, req: &UpdateUser, actor: &str) -> Result<(), RegAdminError> {
    let mut conn = connect(cfg).await?;
    let first = req.first_name.as_deref();
    let last = req.last_name.as_deref();
    let office = req.office_id.as_deref();
    run(
        &mut conn,
        "EXEC plagis_admin.usp_update_user @UserName=@P1, @FirstName=@P2, @LastName=@P3, @OfficeId=@P4, @Actor=@P5",
        &[&user, &first, &last, &office, &actor],
    )
    .await
}
