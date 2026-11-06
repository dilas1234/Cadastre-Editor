/* =====================================================================
   preflight.sql  —  run BEFORE plagis_admin_procedures.sql

   READ-ONLY: every statement here is a SELECT. It changes nothing, and it
   never returns a password or a password hash. Run it in SSMS on the
   Registry's SQL Server and read each result set against the "want" column.

   Anything marked FIX must be sorted out before installing.
   ===================================================================== */
USE LRS43;
GO
SET NOCOUNT ON;

/* 1. Server and database. THROW and CONCAT need SQL Server 2012 or later. */
SELECT CAST(SERVERPROPERTY('ProductVersion') AS nvarchar(30)) AS product_version,
       CAST(SERVERPROPERTY('Edition') AS nvarchar(100))       AS edition,
       d.compatibility_level, d.collation_name, d.recovery_model_desc, d.state_desc,
       CASE WHEN CAST(PARSENAME(CAST(SERVERPROPERTY('ProductVersion') AS nvarchar(30)), 4) AS int) >= 11
            THEN 'OK' ELSE 'FIX: needs SQL Server 2012 or later' END AS verdict
FROM sys.databases d WHERE d.name = 'LRS43';

/* 2. Last backups. Want a full backup taken just before the install. */
SELECT b.type AS backup_type,   -- D = full, I = differential, L = log
       MAX(b.backup_finish_date) AS last_finished,
       DATEDIFF(HOUR, MAX(b.backup_finish_date), GETDATE()) AS hours_ago
FROM msdb.dbo.backupset b
WHERE b.database_name = 'LRS43'
GROUP BY b.type;

/* 3. Ownership. plagis_admin is created AUTHORIZATION LRSAdmin; the app login can only reach the
      Alfresco tables through ownership chaining, so that principal must own the LRSAdmin schema. */
SELECT s.name AS schema_name, p.name AS owner_name, p.type_desc,
       CASE WHEN p.name = 'LRSAdmin' THEN 'OK' ELSE 'FIX: chaining needs one owner; see the WARNING in the install script' END AS verdict
FROM sys.schemas s JOIN sys.database_principals p ON p.principal_id = s.principal_id
WHERE s.name = 'LRSAdmin';

SELECT CASE WHEN DATABASE_PRINCIPAL_ID('LRSAdmin') IS NOT NULL THEN 'OK'
            ELSE 'FIX: no database principal LRSAdmin; CREATE SCHEMA will fail' END AS lrsadmin_principal;

/* 4. Is anything already installed? Want all of these 0 on a first install. */
SELECT (SELECT COUNT(*) FROM sys.schemas WHERE name = 'plagis_admin')               AS plagis_admin_schema,
       (SELECT COUNT(*) FROM sys.objects o JOIN sys.schemas s ON s.schema_id = o.schema_id
         WHERE s.name = 'plagis_admin')                                             AS plagis_admin_objects,
       (SELECT COUNT(*) FROM sys.server_principals WHERE name = 'plagis_admin_app') AS app_login,
       (SELECT COUNT(*) FROM sys.database_principals WHERE name = 'plagis_admin_app') AS app_db_user;

/* 5. MD4. usp_create_user depends on this exact hash. */
SELECT CASE WHEN HASHBYTES('MD4', N'password') = 0x8846F7EAEE8FB117AD06BDD830B7586C
            THEN 'OK' ELSE 'FIX: MD4 does not work on this server' END AS md4_self_test;

/* 6. Every Alfresco column the procedures use. Want NO ROWS. */
;WITH need (tbl, col) AS (
    SELECT * FROM (VALUES
    ('alf_node','id'),('alf_node','version'),('alf_node','store_id'),('alf_node','uuid'),('alf_node','transaction_id'),
    ('alf_node','node_deleted'),('alf_node','type_qname_id'),('alf_node','acl_id'),('alf_node','audit_creator'),
    ('alf_node','audit_created'),('alf_node','audit_modifier'),('alf_node','audit_modified'),('alf_node','audit_accessed'),
    ('alf_node_properties','node_id'),('alf_node_properties','actual_type_n'),('alf_node_properties','persisted_type_n'),
    ('alf_node_properties','boolean_value'),('alf_node_properties','long_value'),('alf_node_properties','float_value'),
    ('alf_node_properties','double_value'),('alf_node_properties','string_value'),('alf_node_properties','serializable_value'),
    ('alf_node_properties','qname_id'),('alf_node_properties','list_index'),('alf_node_properties','locale_id'),
    ('alf_node_aspects','node_id'),('alf_node_aspects','qname_id'),
    ('alf_child_assoc','id'),('alf_child_assoc','version'),('alf_child_assoc','parent_node_id'),('alf_child_assoc','type_qname_id'),
    ('alf_child_assoc','child_node_name_crc'),('alf_child_assoc','child_node_name'),('alf_child_assoc','child_node_id'),
    ('alf_child_assoc','qname_ns_id'),('alf_child_assoc','qname_localname'),('alf_child_assoc','qname_crc'),
    ('alf_child_assoc','is_primary'),('alf_child_assoc','assoc_index'),
    ('alf_transaction','id'),('alf_transaction','version'),('alf_transaction','server_id'),('alf_transaction','change_txn_id'),
    ('alf_transaction','commit_time_ms'),
    ('alf_authority','id'),('alf_authority','version'),('alf_authority','authority'),('alf_authority','crc'),
    ('alf_qname','id'),('alf_qname','local_name'),
    ('alf_store','id'),('alf_store','protocol'),('alf_store','identifier'),
    ('alf_access_control_list','id'),('alf_access_control_list','inherited_acl')
    ) v (tbl, col)
)
SELECT n.tbl, n.col, 'FIX: column missing' AS verdict
FROM need n
LEFT JOIN sys.columns c ON c.object_id = OBJECT_ID('LRSAdmin.' + n.tbl) AND c.name = n.col
WHERE c.column_id IS NULL;

/* 7. Identity. The procedures insert without an id and read SCOPE_IDENTITY(). Want 'OK' on all four. */
SELECT v.tbl,
       CASE WHEN ic.object_id IS NOT NULL THEN 'OK' ELSE 'FIX: id is not an identity column' END AS verdict
FROM (VALUES ('alf_transaction'),('alf_node'),('alf_child_assoc'),('alf_authority')) v (tbl)
LEFT JOIN sys.identity_columns ic ON ic.object_id = OBJECT_ID('LRSAdmin.' + v.tbl) AND ic.name = 'id';

/* 8. Required columns the INSERTs do not fill. Want NO ROWS, or the INSERT fails at run time. */
SELECT t.name AS tbl, c.name AS col, 'FIX: NOT NULL, no default, not set by the procedures' AS verdict
FROM sys.tables t
JOIN sys.columns c ON c.object_id = t.object_id
WHERE t.schema_id = SCHEMA_ID('LRSAdmin')
  AND t.name IN ('alf_node','alf_node_properties','alf_node_aspects','alf_child_assoc','alf_transaction','alf_authority')
  AND c.is_nullable = 0 AND c.is_identity = 0 AND c.is_computed = 0 AND c.default_object_id = 0
  AND c.name NOT IN ('version','store_id','uuid','transaction_id','node_deleted','type_qname_id',
                     'node_id','actual_type_n','persisted_type_n','boolean_value','long_value','float_value','double_value',
                     'qname_id','list_index','locale_id',
                     'parent_node_id','child_node_name_crc','child_node_name','child_node_id','qname_ns_id','qname_localname',
                     'qname_crc','is_primary','assoc_index',
                     'server_id','change_txn_id','commit_time_ms','authority','crc');

/* 9. Reference user and the system accounts. Want person_nodes = 1 and login_nodes = 1 for each;
      'cgugin' is the dashboard's default "copy groups & office from" user. */
SELECT u.name AS user_name,
       (SELECT COUNT(*) FROM LRSAdmin.alf_node n
         JOIN LRSAdmin.alf_store s ON s.id = n.store_id
         JOIN LRSAdmin.alf_qname t ON t.id = n.type_qname_id
         JOIN LRSAdmin.alf_node_properties p ON p.node_id = n.id
         JOIN LRSAdmin.alf_qname q ON q.id = p.qname_id
         WHERE s.protocol = 'workspace' AND s.identifier = 'SpacesStore' AND t.local_name = 'person'
           AND q.local_name = 'userName' AND p.string_value = u.name AND n.node_deleted = 0) AS person_nodes,
       (SELECT COUNT(*) FROM LRSAdmin.alf_node n
         JOIN LRSAdmin.alf_store s ON s.id = n.store_id
         JOIN LRSAdmin.alf_qname t ON t.id = n.type_qname_id
         JOIN LRSAdmin.alf_node_properties p ON p.node_id = n.id
         JOIN LRSAdmin.alf_qname q ON q.id = p.qname_id
         WHERE s.protocol = 'user' AND s.identifier = 'alfrescoUserStore' AND t.local_name = 'user'
           AND q.local_name = 'username' AND p.string_value = u.name AND n.node_deleted = 0) AS login_nodes
FROM (VALUES (N'cgugin'),(N'admin'),(N'AumentumWS'),(N'MCConfig')) u (name);

/* 10. Reference group. fn_group_node takes TOP 1 with no store filter, so want exactly 1 row
       here: a copy in the archive store could be picked by mistake. */
SELECT s.protocol, s.identifier, COUNT(*) AS nodes
FROM LRSAdmin.alf_node n
JOIN LRSAdmin.alf_store s ON s.id = n.store_id
JOIN LRSAdmin.alf_qname t ON t.id = n.type_qname_id AND t.local_name = 'authorityContainer'
JOIN LRSAdmin.alf_node_properties p ON p.node_id = n.id
JOIN LRSAdmin.alf_qname q ON q.id = p.qname_id AND q.local_name = 'authorityName'
WHERE p.string_value = N'GROUP_Map Compilation Officer' AND n.node_deleted = 0
GROUP BY s.protocol, s.identifier;

/* 11. Password format: lengths only, no values. usp_reset_password assumes 32-character MD4 and,
       unlike usp_create_user, does not check. Want every login at length 32. */
SELECT LEN(p.string_value) AS password_length, COUNT(*) AS logins
FROM LRSAdmin.alf_node n
JOIN LRSAdmin.alf_qname t ON t.id = n.type_qname_id AND t.local_name = 'user'
JOIN LRSAdmin.alf_node_properties p ON p.node_id = n.id
JOIN LRSAdmin.alf_qname q ON q.id = p.qname_id AND q.local_name = 'password'
WHERE n.node_deleted = 0
GROUP BY LEN(p.string_value);

/* 12. Stores holding person nodes. usp_list_users has no store filter, so anything outside
       workspace://SpacesStore would appear in the dashboard as a duplicate user. */
SELECT s.protocol, s.identifier, COUNT(*) AS person_nodes
FROM LRSAdmin.alf_node n
JOIN LRSAdmin.alf_store s ON s.id = n.store_id
JOIN LRSAdmin.alf_qname t ON t.id = n.type_qname_id AND t.local_name = 'person'
WHERE n.node_deleted = 0
GROUP BY s.protocol, s.identifier;
GO
