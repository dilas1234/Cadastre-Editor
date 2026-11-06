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

/// The body of `usp_list_users`, for [`RegistrySqlConfig::direct`]. Dirty reads: this must not wait
/// on, or hold up, the Registry's own work on `alf_node`.
const LIST_USERS_SQL: &str = r#"
SET NOCOUNT ON; SET TRANSACTION ISOLATION LEVEL READ UNCOMMITTED;
DECLARE @Search nvarchar(100) = @P1;
WITH person AS (
    SELECT n.id, n.uuid,
           MAX(CASE WHEN q.local_name = 'userName'  THEN p.string_value END) AS userName,
           MAX(CASE WHEN q.local_name = 'firstName' THEN p.string_value END) AS firstName,
           MAX(CASE WHEN q.local_name = 'lastName'  THEN p.string_value END) AS lastName,
           MAX(CASE WHEN q.local_name = 'officeId'  THEN p.string_value END) AS officeId
    FROM LRSAdmin.alf_node n
    JOIN LRSAdmin.alf_qname t ON t.id = n.type_qname_id AND t.local_name = 'person'
    JOIN LRSAdmin.alf_node_properties p ON p.node_id = n.id
    JOIN LRSAdmin.alf_qname q ON q.id = p.qname_id
    WHERE n.node_deleted = 0
    GROUP BY n.id, n.uuid
), login AS (
    SELECT MAX(CASE WHEN q.local_name = 'username' THEN p.string_value END) AS userName,
           MAX(CASE WHEN q.local_name = 'enabled' THEN CAST(p.boolean_value AS int) END) AS enabled
    FROM LRSAdmin.alf_node n
    JOIN LRSAdmin.alf_qname t ON t.id = n.type_qname_id AND t.local_name = 'user'
    JOIN LRSAdmin.alf_node_properties p ON p.node_id = n.id
    JOIN LRSAdmin.alf_qname q ON q.id = p.qname_id
    WHERE n.node_deleted = 0
    GROUP BY n.id
)
SELECT pr.userName, pr.firstName, pr.lastName, pr.officeId,
       l.enabled, CASE WHEN l.userName IS NULL THEN 0 ELSE 1 END AS has_login,
       STUFF((SELECT ', ' + g.string_value
              FROM LRSAdmin.alf_child_assoc ca
              JOIN LRSAdmin.alf_node_properties g ON g.node_id = ca.parent_node_id
              JOIN LRSAdmin.alf_qname qg ON qg.id = g.qname_id AND qg.local_name = 'authorityName'
              WHERE ca.child_node_id = pr.id AND ca.is_primary = 0
              ORDER BY g.string_value FOR XML PATH(''), TYPE).value('.', 'nvarchar(max)'), 1, 2, '') AS groups
FROM person pr
LEFT JOIN login l ON l.userName = pr.userName
WHERE pr.userName IS NOT NULL
  AND (@Search IS NULL OR pr.userName LIKE '%' + @Search + '%'
       OR pr.firstName LIKE '%' + @Search + '%' OR pr.lastName LIKE '%' + @Search + '%')
ORDER BY pr.userName;
"#;

/// The body of `usp_list_groups`, for [`RegistrySqlConfig::direct`].
const LIST_GROUPS_SQL: &str = r#"
SET NOCOUNT ON; SET TRANSACTION ISOLATION LEVEL READ UNCOMMITTED;
SELECT an.string_value AS group_name,
       (SELECT TOP 1 d.string_value FROM LRSAdmin.alf_node_properties d
        JOIN LRSAdmin.alf_qname qd ON qd.id = d.qname_id AND qd.local_name = 'authorityDisplayName'
        WHERE d.node_id = n.id) AS display_name,
       (SELECT COUNT(*) FROM LRSAdmin.alf_child_assoc ca
        JOIN LRSAdmin.alf_qname qt ON qt.id = ca.type_qname_id AND qt.local_name = 'member'
        WHERE ca.parent_node_id = n.id) AS members
FROM LRSAdmin.alf_node n
JOIN LRSAdmin.alf_qname t ON t.id = n.type_qname_id AND t.local_name = 'authorityContainer'
JOIN LRSAdmin.alf_node_properties an ON an.node_id = n.id
JOIN LRSAdmin.alf_qname q ON q.id = an.qname_id AND q.local_name = 'authorityName'
WHERE n.node_deleted = 0
ORDER BY an.string_value;
"#;

/// [`create_user`] without the procedures; see `regadmin_create_user.sql`.
pub const CREATE_USER_SQL: &str = include_str!("regadmin_create_user.sql");

const CM_NS: &str = "http://www.alfresco.org/model/content/1.0";
const USR_NS: &str = "http://www.alfresco.org/model/user/1.0";

/// CRC-32 as Alfresco stores it (`plagis_admin.fn_crc32`).
pub fn crc32(text: &str) -> i64 {
    let mut c: u32 = 0xFFFF_FFFF;
    for b in text.bytes() {
        c ^= u32::from(b);
        for _ in 0..8 {
            c = if c & 1 == 1 { (c >> 1) ^ 0xEDB8_8320 } else { c >> 1 };
        }
    }
    i64::from(!c)
}

/// The people-folder name CRC of a username, which the Registry holds on every person.
pub fn person_qname_crc(user: &str) -> i64 {
    crc32(&format!("{CM_NS}{user}"))
}

/// What `usp_create_user` checks before it writes; gives the login as it will be stored.
pub fn checked_login(req: &CreateUser) -> Result<String, RegAdminError> {
    let login = req.user_name.trim().to_lowercase();
    if login.is_empty() || !login.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-')) {
        return Err(RegAdminError::Refused("Username may only contain a-z, 0-9, dot, dash, underscore.".into()));
    }
    if req.first_name.trim().is_empty() || req.last_name.trim().is_empty() {
        return Err(RegAdminError::Refused("First name and last name are required.".into()));
    }
    if req.password.chars().count() < 6 {
        return Err(RegAdminError::Refused("Password must be at least 6 characters.".into()));
    }
    if !req.ref_user.is_ascii() {
        return Err(RegAdminError::Refused("Reference user not found.".into()));
    }
    Ok(login)
}

async fn create_user_direct(cfg: &RegistrySqlConfig, req: &CreateUser) -> Result<(), RegAdminError> {
    let login = checked_login(req)?;
    let extra = req.extra_groups.as_deref().filter(|s| !s.trim().is_empty());
    let guid = || uuid::Uuid::new_v4().to_string();
    let (pu, uu, pan, uan) = (guid(), guid(), guid(), guid());
    let (auth, cm, usr) = (crc32(&login), person_qname_crc(&login), crc32(&format!("{USR_NS}{login}")));
    let (pu_crc, pan_crc, uan_crc, ref_crc) = (crc32(&pu), -crc32(&pan), -crc32(&uan), person_qname_crc(&req.ref_user));
    let mut conn = connect(cfg).await?;
    run(
        &mut conn,
        CREATE_USER_SQL,
        &[&login, &req.first_name, &req.last_name, &req.password, &req.ref_user, &extra, &pu, &uu, &pan, &uan,
          &auth, &cm, &usr, &pu_crc, &pan_crc, &uan_crc, &ref_crc],
    )
    .await
}

/// Users, with name, office, enabled flag and groups. `search` matches username/first/last.
pub async fn list_users(cfg: &RegistrySqlConfig, search: Option<&str>) -> Result<Vec<RegistryUser>, RegAdminError> {
    let mut conn = connect(cfg).await?;
    let rows = conn
        .query(if cfg.direct { LIST_USERS_SQL } else { "EXEC plagis_admin.usp_list_users @Search = @P1" }, &[&search])
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
        .query(if cfg.direct { LIST_GROUPS_SQL } else { "EXEC plagis_admin.usp_list_groups" }, &[])
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
    if cfg.direct {
        return create_user_direct(cfg, req).await;
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn request(user_name: &str) -> CreateUser {
        CreateUser { user_name: user_name.into(), first_name: "A".into(), last_name: "B".into(), password: "secret1".into(), ref_user: "cgugin".into(), extra_groups: None }
    }

    #[test]
    fn crc_is_the_standard_crc32() {
        assert_eq!(crc32("123456789"), 0xCBF4_3926);
        assert_eq!(crc32(""), 0);
    }

    #[test]
    fn logins_are_checked_as_the_procedure_does() {
        assert_eq!(checked_login(&request("  T.Person-1_ ")).unwrap(), "t.person-1_");
        for bad in ["", "two words", "a'b", "naïve"] {
            assert!(checked_login(&request(bad)).is_err(), "{bad}");
        }
        assert!(checked_login(&CreateUser { password: "short".into(), ..request("ok") }).is_err());
        assert!(checked_login(&CreateUser { first_name: " ".into(), ..request("ok") }).is_err());
    }

    #[test]
    fn every_parameter_of_the_create_batch_is_used() {
        for n in 1..=17 {
            assert!(CREATE_USER_SQL.contains(&format!("@P{n}")), "@P{n}");
        }
        assert!(!CREATE_USER_SQL.contains("plagis_admin."), "the batch must not need the procedures");
    }
}
