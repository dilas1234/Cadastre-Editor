/* =====================================================================
   plagis_admin_procedures.sql  —  Aumentum Registry (LRS43 / Alfresco)

   Stored procedures for a user-administration tool. Built from the
   create-users / create-group scripts that were tested on this server.
   Everything lives in its own schema [plagis_admin], separate from
   Alfresco's LRSAdmin tables, and every change is written to an audit log.

   INSTALL: back up LRS43, then run this whole file once in SSMS.
            Re-running it is safe (it drops and recreates the procedures;
            the audit log table is kept).

   IMPORTANT: Alfresco caches users and groups in memory. After changes,
   restart the Registry (JBoss) service before they take effect.

   PROCEDURES
     usp_create_user        clone a reference user (+ optional extra groups)
     usp_create_group       clone a reference group (nested under GROUP_LRS etc.)
     usp_add_to_group       add a user to a group
     usp_remove_from_group  remove a user from a group
     usp_reset_password     set a new password
     usp_set_user_enabled   disable (0) / enable (1) a user  — use instead of delete
     usp_update_user        change first name, last name, office id
     usp_list_users         users with name, office, enabled flag, groups
     usp_list_groups        groups with display name and member count
     usp_user_groups        groups of one user
   ===================================================================== */
USE LRS43;
GO

-- Owned by LRSAdmin (the owner of the Alfresco tables), so the app login only
-- needs EXECUTE on these procedures and never direct access to the tables.
IF SCHEMA_ID('plagis_admin') IS NULL EXEC('CREATE SCHEMA plagis_admin AUTHORIZATION LRSAdmin');
GO
IF (SELECT USER_NAME(principal_id) FROM sys.schemas WHERE name = 'LRSAdmin')
   <> (SELECT USER_NAME(principal_id) FROM sys.schemas WHERE name = 'plagis_admin')
    PRINT 'WARNING: plagis_admin and LRSAdmin schemas have different owners; the app login will need extra grants.';
GO

IF OBJECT_ID('plagis_admin.audit_log') IS NULL
CREATE TABLE plagis_admin.audit_log (
    id         bigint IDENTITY(1,1) PRIMARY KEY,
    at_utc     datetime2     NOT NULL DEFAULT SYSUTCDATETIME(),
    actor      nvarchar(100) NOT NULL,
    action     varchar(50)   NOT NULL,
    target     nvarchar(200) NOT NULL,
    details    nvarchar(max) NULL
);
GO

/* ------------------------------ helpers ------------------------------ */
IF OBJECT_ID('plagis_admin.fn_crc32') IS NOT NULL DROP FUNCTION plagis_admin.fn_crc32;
GO
-- CRC32 exactly as Alfresco computes it (verified against existing rows). ASCII input only.
CREATE FUNCTION plagis_admin.fn_crc32 (@s varchar(4000))
RETURNS bigint
AS
BEGIN
    DECLARE @c bigint = CAST(4294967295 AS bigint), @Poly bigint = CAST(3988292384 AS bigint),
            @Mask bigint = CAST(4294967295 AS bigint), @p int = 1, @b int;
    WHILE @p <= DATALENGTH(@s)
    BEGIN
        SET @c = @c ^ ASCII(SUBSTRING(@s, @p, 1)); SET @b = 0;
        WHILE @b < 8
        BEGIN
            IF (@c & 1) = 1 SET @c = (@c / 2) ^ @Poly ELSE SET @c = @c / 2;
            SET @b = @b + 1;
        END
        SET @p = @p + 1;
    END
    RETURN @c ^ @Mask;
END
GO

IF OBJECT_ID('plagis_admin.fn_person_node') IS NOT NULL DROP FUNCTION plagis_admin.fn_person_node;
GO
CREATE FUNCTION plagis_admin.fn_person_node (@u nvarchar(100))
RETURNS bigint
AS
BEGIN
    RETURN (SELECT TOP 1 n.id
            FROM LRSAdmin.alf_node n
            JOIN LRSAdmin.alf_store s ON s.id = n.store_id
            JOIN LRSAdmin.alf_qname t ON t.id = n.type_qname_id
            JOIN LRSAdmin.alf_node_properties p ON p.node_id = n.id
            JOIN LRSAdmin.alf_qname q ON q.id = p.qname_id
            WHERE s.protocol = 'workspace' AND s.identifier = 'SpacesStore' AND t.local_name = 'person'
              AND q.local_name = 'userName' AND p.string_value = @u AND n.node_deleted = 0);
END
GO

IF OBJECT_ID('plagis_admin.fn_login_node') IS NOT NULL DROP FUNCTION plagis_admin.fn_login_node;
GO
CREATE FUNCTION plagis_admin.fn_login_node (@u nvarchar(100))
RETURNS bigint
AS
BEGIN
    RETURN (SELECT TOP 1 n.id
            FROM LRSAdmin.alf_node n
            JOIN LRSAdmin.alf_store s ON s.id = n.store_id
            JOIN LRSAdmin.alf_qname t ON t.id = n.type_qname_id
            JOIN LRSAdmin.alf_node_properties p ON p.node_id = n.id
            JOIN LRSAdmin.alf_qname q ON q.id = p.qname_id
            WHERE s.protocol = 'user' AND s.identifier = 'alfrescoUserStore' AND t.local_name = 'user'
              AND q.local_name = 'username' AND p.string_value = @u AND n.node_deleted = 0);
END
GO

IF OBJECT_ID('plagis_admin.fn_group_node') IS NOT NULL DROP FUNCTION plagis_admin.fn_group_node;
GO
CREATE FUNCTION plagis_admin.fn_group_node (@g nvarchar(200))
RETURNS bigint
AS
BEGIN
    RETURN (SELECT TOP 1 n.id
            FROM LRSAdmin.alf_node n
            JOIN LRSAdmin.alf_qname t ON t.id = n.type_qname_id
            JOIN LRSAdmin.alf_node_properties p ON p.node_id = n.id
            JOIN LRSAdmin.alf_qname q ON q.id = p.qname_id
            WHERE t.local_name = 'authorityContainer'
              AND q.local_name = 'authorityName' AND p.string_value = @g AND n.node_deleted = 0);
END
GO

IF OBJECT_ID('plagis_admin.usp__new_txn') IS NOT NULL DROP PROCEDURE plagis_admin.usp__new_txn;
GO
-- New Alfresco transaction row (copies server_id from a template node's transaction)
CREATE PROCEDURE plagis_admin.usp__new_txn @TemplateNode bigint, @Txn bigint OUTPUT
AS
BEGIN
    SET NOCOUNT ON;
    INSERT INTO LRSAdmin.alf_transaction (version, server_id, change_txn_id, commit_time_ms)
    SELECT 1, t.server_id, LOWER(CONVERT(varchar(36), NEWID())),
           CAST(DATEDIFF(SECOND, '19700101', GETUTCDATE()) AS bigint) * 1000
    FROM LRSAdmin.alf_transaction t JOIN LRSAdmin.alf_node n ON n.transaction_id = t.id
    WHERE n.id = @TemplateNode;
    SET @Txn = SCOPE_IDENTITY();
    IF @Txn IS NULL THROW 50000, 'Could not create transaction row.', 1;
END
GO

/* --------------------------- create user ----------------------------- */
IF OBJECT_ID('plagis_admin.usp_create_user') IS NOT NULL DROP PROCEDURE plagis_admin.usp_create_user;
GO
CREATE PROCEDURE plagis_admin.usp_create_user
    @UserName    nvarchar(100),
    @FirstName   nvarchar(100),
    @LastName    nvarchar(100),
    @Password    nvarchar(100),
    @RefUser     nvarchar(100),            -- existing user whose groups + office are copied
    @ExtraGroups nvarchar(max) = NULL,     -- optional, comma-separated: N'GROUP_Title Deed Plan,GROUP_LRS'
    @Actor       nvarchar(100)             -- who is doing this (admin tool login)
AS
BEGIN
    SET NOCOUNT ON; SET XACT_ABORT ON;
    SET @UserName = LOWER(LTRIM(RTRIM(@UserName)));

    IF @UserName = '' OR @UserName LIKE '%[^a-z0-9._-]%'
        THROW 50001, 'Username may only contain a-z, 0-9, dot, dash, underscore.', 1;
    IF LTRIM(RTRIM(ISNULL(@FirstName, ''))) = '' OR LTRIM(RTRIM(ISNULL(@LastName, ''))) = ''
        THROW 50002, 'First name and last name are required.', 1;
    IF LEN(ISNULL(@Password, '')) < 6
        THROW 50003, 'Password must be at least 6 characters.', 1;
    IF plagis_admin.fn_person_node(@UserName) IS NOT NULL OR plagis_admin.fn_login_node(@UserName) IS NOT NULL
       OR EXISTS (SELECT 1 FROM LRSAdmin.alf_authority WHERE authority = @UserName)
        THROW 50004, 'Username already exists.', 1;

    DECLARE @RefPerson bigint = plagis_admin.fn_person_node(@RefUser),
            @RefLogin  bigint = plagis_admin.fn_login_node(@RefUser);
    IF @RefPerson IS NULL OR @RefLogin IS NULL
        THROW 50005, 'Reference user not found.', 1;

    -- CRC self-test against the reference user's real People-folder link
    DECLARE @Cm varchar(100) = 'http://www.alfresco.org/model/content/1.0',
            @Usr varchar(100) = 'http://www.alfresco.org/model/user/1.0';
    IF plagis_admin.fn_crc32(@Cm + CAST(@RefUser AS varchar(100)))
       <> (SELECT qname_crc FROM LRSAdmin.alf_child_assoc WHERE child_node_id = @RefPerson AND is_primary = 1)
        THROW 50006, 'CRC self-test failed. Do not continue; check the procedure.', 1;

    -- Password hash in the same format as the reference user's
    IF HASHBYTES('MD4', N'password') <> 0x8846F7EAEE8FB117AD06BDD830B7586C
        THROW 50007, 'MD4 self-test failed on this SQL Server.', 1;
    DECLARE @PwHash nvarchar(64) = LOWER(CONVERT(varchar(32), HASHBYTES('MD4', @Password), 2));
    IF EXISTS (SELECT 1 FROM LRSAdmin.alf_node_properties p JOIN LRSAdmin.alf_qname q ON q.id = p.qname_id
               WHERE p.node_id = @RefLogin AND q.local_name = 'password' AND LEN(p.string_value) <> 32)
        THROW 50008, 'Reference password is not in the 32-char MD4 format.', 1;
    IF EXISTS (SELECT 1 FROM LRSAdmin.alf_node_properties p JOIN LRSAdmin.alf_qname q ON q.id = p.qname_id
               WHERE p.node_id = @RefLogin AND q.local_name = 'password'
                 AND p.string_value COLLATE Latin1_General_BIN LIKE '%[A-F]%')
        SET @PwHash = UPPER(@PwHash);

    -- Extra groups (split the comma list; '&' in names like "RofO & CofO Reviewer" is handled)
    DECLARE @Extra TABLE (groupName nvarchar(200), nodeId bigint);
    IF LTRIM(RTRIM(ISNULL(@ExtraGroups, ''))) <> ''
    BEGIN
        DECLARE @x xml = CAST('<g>' + REPLACE(REPLACE(REPLACE(REPLACE(@ExtraGroups, '&', '&amp;'), '<', '&lt;'), '>', '&gt;'), ',', '</g><g>') + '</g>' AS xml);
        INSERT INTO @Extra (groupName)
        SELECT DISTINCT LTRIM(RTRIM(t.c.value('.', 'nvarchar(200)'))) FROM @x.nodes('/g') t(c);
        DELETE FROM @Extra WHERE groupName = '';
        UPDATE @Extra SET nodeId = plagis_admin.fn_group_node(groupName);
        IF EXISTS (SELECT 1 FROM @Extra WHERE nodeId IS NULL)
        BEGIN
            DECLARE @missing nvarchar(max) = STUFF((SELECT ', ' + groupName FROM @Extra WHERE nodeId IS NULL FOR XML PATH('')), 1, 2, '');
            DECLARE @msg nvarchar(2048) = N'Group(s) not found: ' + @missing;
            THROW 50009, @msg, 1;
        END
        DELETE e FROM @Extra e   -- already copied from the reference user
        WHERE EXISTS (SELECT 1 FROM LRSAdmin.alf_child_assoc ca WHERE ca.parent_node_id = e.nodeId AND ca.child_node_id = @RefPerson);
    END

    DECLARE @PeopleFolder bigint = (SELECT parent_node_id FROM LRSAdmin.alf_child_assoc WHERE child_node_id = @RefPerson AND is_primary = 1);
    DECLARE @PersonAcl bigint = (SELECT acl.inherited_acl FROM LRSAdmin.alf_node n
                                 JOIN LRSAdmin.alf_access_control_list acl ON acl.id = n.acl_id WHERE n.id = @PeopleFolder);
    IF @PersonAcl IS NULL THROW 50010, 'Could not find the People folder inherited ACL.', 1;

    DECLARE @MemberType bigint = (SELECT TOP 1 ca.type_qname_id FROM LRSAdmin.alf_child_assoc ca
                                  JOIN LRSAdmin.alf_qname q ON q.id = ca.type_qname_id WHERE q.local_name = 'member');

    -- GUIDs + checksums
    DECLARE @pu  varchar(36) = LOWER(CONVERT(varchar(36), NEWID())),
            @uu  varchar(36) = LOWER(CONVERT(varchar(36), NEWID())),
            @pan varchar(36) = LOWER(CONVERT(varchar(36), NEWID())),
            @uan varchar(36) = LOWER(CONVERT(varchar(36), NEWID()));
    DECLARE @uA varchar(100) = CAST(@UserName AS varchar(100));
    DECLARE @AuthCrc bigint = plagis_admin.fn_crc32(@uA),
            @CmCrc   bigint = plagis_admin.fn_crc32(@Cm + @uA),
            @UsrCrc  bigint = plagis_admin.fn_crc32(@Usr + @uA),
            @PuCrc   bigint = plagis_admin.fn_crc32(@pu),
            @PanCrc  bigint = -plagis_admin.fn_crc32(@pan),
            @UanCrc  bigint = -plagis_admin.fn_crc32(@uan);

    BEGIN TRY
        BEGIN TRANSACTION;
        DECLARE @Txn bigint, @NewPerson bigint, @NewLogin bigint;
        EXEC plagis_admin.usp__new_txn @RefPerson, @Txn OUTPUT;

        INSERT INTO LRSAdmin.alf_node (version, store_id, uuid, transaction_id, node_deleted, type_qname_id, acl_id,
                                       audit_creator, audit_created, audit_modifier, audit_modified, audit_accessed)
        SELECT 1, store_id, @pu, @Txn, 0, type_qname_id, @PersonAcl, NULL, NULL, NULL, NULL, NULL
        FROM LRSAdmin.alf_node WHERE id = @RefPerson;
        SET @NewPerson = SCOPE_IDENTITY();

        INSERT INTO LRSAdmin.alf_node (version, store_id, uuid, transaction_id, node_deleted, type_qname_id, acl_id,
                                       audit_creator, audit_created, audit_modifier, audit_modified, audit_accessed)
        SELECT 1, store_id, @uu, @Txn, 0, type_qname_id, acl_id, NULL, NULL, NULL, NULL, NULL
        FROM LRSAdmin.alf_node WHERE id = @RefLogin;
        SET @NewLogin = SCOPE_IDENTITY();

        INSERT INTO LRSAdmin.alf_node_properties (node_id, actual_type_n, persisted_type_n, boolean_value, long_value,
                    float_value, double_value, string_value, serializable_value, qname_id, list_index, locale_id)
        SELECT @NewPerson, p.actual_type_n, p.persisted_type_n, p.boolean_value, p.long_value, p.float_value, p.double_value,
               CASE q.local_name WHEN 'userName' THEN @UserName WHEN 'owner' THEN @UserName
                                 WHEN 'firstName' THEN LTRIM(RTRIM(@FirstName)) WHEN 'lastName' THEN LTRIM(RTRIM(@LastName))
                                 ELSE p.string_value END,
               p.serializable_value, p.qname_id, p.list_index, p.locale_id
        FROM LRSAdmin.alf_node_properties p JOIN LRSAdmin.alf_qname q ON q.id = p.qname_id
        WHERE p.node_id = @RefPerson
          AND q.local_name IN ('userName','owner','firstName','lastName','homeFolderProvider','officeId','sizeCurrent');

        INSERT INTO LRSAdmin.alf_node_properties (node_id, actual_type_n, persisted_type_n, boolean_value, long_value,
                    float_value, double_value, string_value, serializable_value, qname_id, list_index, locale_id)
        SELECT @NewLogin, p.actual_type_n, p.persisted_type_n,
               CASE q.local_name WHEN 'enabled' THEN 1 WHEN 'accountLocked' THEN 0 ELSE p.boolean_value END,
               p.long_value, p.float_value, p.double_value,
               CASE q.local_name WHEN 'username' THEN @UserName WHEN 'password' THEN @PwHash ELSE p.string_value END,
               p.serializable_value, p.qname_id, p.list_index, p.locale_id
        FROM LRSAdmin.alf_node_properties p JOIN LRSAdmin.alf_qname q ON q.id = p.qname_id
        WHERE p.node_id = @RefLogin
          AND q.local_name IN ('username','password','salt','enabled','accountExpires','accountLocked','credentialsExpire');

        INSERT INTO LRSAdmin.alf_node_aspects (node_id, qname_id)
        SELECT @NewPerson, a.qname_id FROM LRSAdmin.alf_node_aspects a JOIN LRSAdmin.alf_qname q ON q.id = a.qname_id
        WHERE a.node_id = @RefPerson AND q.local_name <> 'configurable';
        INSERT INTO LRSAdmin.alf_node_aspects (node_id, qname_id)
        SELECT @NewLogin, qname_id FROM LRSAdmin.alf_node_aspects WHERE node_id = @RefLogin;

        INSERT INTO LRSAdmin.alf_child_assoc (version, parent_node_id, type_qname_id, child_node_name_crc, child_node_name,
                    child_node_id, qname_ns_id, qname_localname, qname_crc, is_primary, assoc_index)
        SELECT 1, parent_node_id, type_qname_id, @PanCrc, @pan, @NewPerson, qname_ns_id, @UserName, @CmCrc, 1, assoc_index
        FROM LRSAdmin.alf_child_assoc WHERE child_node_id = @RefPerson AND is_primary = 1;

        INSERT INTO LRSAdmin.alf_child_assoc (version, parent_node_id, type_qname_id, child_node_name_crc, child_node_name,
                    child_node_id, qname_ns_id, qname_localname, qname_crc, is_primary, assoc_index)
        SELECT 1, parent_node_id, type_qname_id, @UanCrc, @uan, @NewLogin, qname_ns_id, @UserName, @UsrCrc, 1, assoc_index
        FROM LRSAdmin.alf_child_assoc WHERE child_node_id = @RefLogin AND is_primary = 1;

        -- zones + groups of the reference user
        INSERT INTO LRSAdmin.alf_child_assoc (version, parent_node_id, type_qname_id, child_node_name_crc, child_node_name,
                    child_node_id, qname_ns_id, qname_localname, qname_crc, is_primary, assoc_index)
        SELECT 1, parent_node_id, type_qname_id, @PuCrc, @pu, @NewPerson, qname_ns_id, @UserName, @CmCrc, 0, assoc_index
        FROM LRSAdmin.alf_child_assoc WHERE child_node_id = @RefPerson AND is_primary = 0;

        -- extra groups
        INSERT INTO LRSAdmin.alf_child_assoc (version, parent_node_id, type_qname_id, child_node_name_crc, child_node_name,
                    child_node_id, qname_ns_id, qname_localname, qname_crc, is_primary, assoc_index)
        SELECT 1, e.nodeId, @MemberType, @PuCrc, @pu, @NewPerson,
               (SELECT qname_ns_id FROM LRSAdmin.alf_child_assoc WHERE child_node_id = @RefPerson AND is_primary = 1),
               @UserName, @CmCrc, 0, -1
        FROM @Extra e;

        INSERT INTO LRSAdmin.alf_authority (version, authority, crc) VALUES (0, @UserName, @AuthCrc);

        INSERT INTO plagis_admin.audit_log (actor, action, target, details)
        VALUES (@Actor, 'create_user', @UserName,
                CONCAT(N'name=', @FirstName, N' ', @LastName, N'; ref=', @RefUser, N'; extra=', ISNULL(@ExtraGroups, N'')));

        COMMIT TRANSACTION;
        SELECT @UserName AS user_created, 'Restart the Registry (JBoss) service before first login.' AS note;
    END TRY
    BEGIN CATCH
        IF @@TRANCOUNT > 0 ROLLBACK TRANSACTION;
        THROW;
    END CATCH
END
GO

/* --------------------------- create group ---------------------------- */
IF OBJECT_ID('plagis_admin.usp_create_group') IS NOT NULL DROP PROCEDURE plagis_admin.usp_create_group;
GO
CREATE PROCEDURE plagis_admin.usp_create_group
    @DisplayName nvarchar(200),                                   -- e.g. N'Title Deed Plan'
    @RefGroup    nvarchar(200) = N'GROUP_Map Compilation Officer', -- structure to copy
    @Actor       nvarchar(100)
AS
BEGIN
    SET NOCOUNT ON; SET XACT_ABORT ON;
    SET @DisplayName = LTRIM(RTRIM(@DisplayName));
    IF @DisplayName = '' OR @DisplayName LIKE '%[^a-zA-Z0-9 &()._-]%'
        THROW 50101, 'Group name may contain letters, digits, spaces and & ( ) . _ - only.', 1;
    DECLARE @Group nvarchar(200) = N'GROUP_' + @DisplayName;

    DECLARE @RefNode bigint = plagis_admin.fn_group_node(@RefGroup);
    IF @RefNode IS NULL THROW 50102, 'Reference group not found.', 1;
    IF plagis_admin.fn_group_node(@Group) IS NOT NULL OR EXISTS (SELECT 1 FROM LRSAdmin.alf_authority WHERE authority = @Group)
        THROW 50103, 'Group already exists.', 1;

    DECLARE @Cm varchar(100) = 'http://www.alfresco.org/model/content/1.0';
    IF plagis_admin.fn_crc32(@Cm + CAST(@RefGroup AS varchar(200)))
       <> (SELECT qname_crc FROM LRSAdmin.alf_child_assoc WHERE child_node_id = @RefNode AND is_primary = 1)
        THROW 50104, 'CRC self-test failed. Do not continue; check the procedure.', 1;

    DECLARE @gu varchar(36) = LOWER(CONVERT(varchar(36), NEWID())),
            @an varchar(36) = LOWER(CONVERT(varchar(36), NEWID()));
    DECLARE @gA varchar(200) = CAST(@Group AS varchar(200));
    DECLARE @CmCrc bigint = plagis_admin.fn_crc32(@Cm + @gA),
            @GuCrc bigint = plagis_admin.fn_crc32(@gu),
            @AnCrc bigint = -plagis_admin.fn_crc32(@an),
            @AuthCrc bigint = plagis_admin.fn_crc32(@gA);

    BEGIN TRY
        BEGIN TRANSACTION;
        DECLARE @Txn bigint, @NewGroup bigint;
        EXEC plagis_admin.usp__new_txn @RefNode, @Txn OUTPUT;

        INSERT INTO LRSAdmin.alf_node (version, store_id, uuid, transaction_id, node_deleted, type_qname_id, acl_id,
                                       audit_creator, audit_created, audit_modifier, audit_modified, audit_accessed)
        SELECT 1, store_id, @gu, @Txn, 0, type_qname_id, acl_id, NULL, NULL, NULL, NULL, NULL
        FROM LRSAdmin.alf_node WHERE id = @RefNode;
        SET @NewGroup = SCOPE_IDENTITY();

        INSERT INTO LRSAdmin.alf_node_properties (node_id, actual_type_n, persisted_type_n, boolean_value, long_value,
                    float_value, double_value, string_value, serializable_value, qname_id, list_index, locale_id)
        SELECT @NewGroup, p.actual_type_n, p.persisted_type_n, p.boolean_value, p.long_value, p.float_value, p.double_value,
               CASE q.local_name WHEN 'authorityName' THEN @Group WHEN 'authorityDisplayName' THEN @DisplayName END,
               p.serializable_value, p.qname_id, p.list_index, p.locale_id
        FROM LRSAdmin.alf_node_properties p JOIN LRSAdmin.alf_qname q ON q.id = p.qname_id
        WHERE p.node_id = @RefNode AND q.local_name IN ('authorityName','authorityDisplayName');

        INSERT INTO LRSAdmin.alf_node_aspects (node_id, qname_id)
        SELECT @NewGroup, qname_id FROM LRSAdmin.alf_node_aspects WHERE node_id = @RefNode;

        INSERT INTO LRSAdmin.alf_child_assoc (version, parent_node_id, type_qname_id, child_node_name_crc, child_node_name,
                    child_node_id, qname_ns_id, qname_localname, qname_crc, is_primary, assoc_index)
        SELECT 1, parent_node_id, type_qname_id, @AnCrc, @an, @NewGroup, qname_ns_id, @Group, @CmCrc, 1, assoc_index
        FROM LRSAdmin.alf_child_assoc WHERE child_node_id = @RefNode AND is_primary = 1;

        INSERT INTO LRSAdmin.alf_child_assoc (version, parent_node_id, type_qname_id, child_node_name_crc, child_node_name,
                    child_node_id, qname_ns_id, qname_localname, qname_crc, is_primary, assoc_index)
        SELECT 1, parent_node_id, type_qname_id, @GuCrc, @gu, @NewGroup, qname_ns_id, @Group, @CmCrc, 0, assoc_index
        FROM LRSAdmin.alf_child_assoc WHERE child_node_id = @RefNode AND is_primary = 0;

        IF EXISTS (SELECT 1 FROM LRSAdmin.alf_authority WHERE authority = @RefGroup)
            INSERT INTO LRSAdmin.alf_authority (version, authority, crc) VALUES (0, @Group, @AuthCrc);

        INSERT INTO plagis_admin.audit_log (actor, action, target, details)
        VALUES (@Actor, 'create_group', @Group, N'ref=' + @RefGroup);

        COMMIT TRANSACTION;
        SELECT @Group AS group_created, 'Restart the Registry (JBoss) service.' AS note;
    END TRY
    BEGIN CATCH
        IF @@TRANCOUNT > 0 ROLLBACK TRANSACTION;
        THROW;
    END CATCH
END
GO

/* ------------------------- group membership -------------------------- */
IF OBJECT_ID('plagis_admin.usp_add_to_group') IS NOT NULL DROP PROCEDURE plagis_admin.usp_add_to_group;
GO
CREATE PROCEDURE plagis_admin.usp_add_to_group
    @UserName nvarchar(100), @GroupName nvarchar(200), @Actor nvarchar(100)
AS
BEGIN
    SET NOCOUNT ON; SET XACT_ABORT ON;
    DECLARE @Person bigint = plagis_admin.fn_person_node(@UserName),
            @Grp    bigint = plagis_admin.fn_group_node(@GroupName);
    IF @Person IS NULL THROW 50201, 'User not found.', 1;
    IF @Grp    IS NULL THROW 50202, 'Group not found.', 1;
    IF EXISTS (SELECT 1 FROM LRSAdmin.alf_child_assoc WHERE parent_node_id = @Grp AND child_node_id = @Person)
        THROW 50203, 'User is already in this group.', 1;

    DECLARE @MemberType bigint = (SELECT TOP 1 ca.type_qname_id FROM LRSAdmin.alf_child_assoc ca
                                  JOIN LRSAdmin.alf_qname q ON q.id = ca.type_qname_id WHERE q.local_name = 'member');
    DECLARE @uuid varchar(36) = (SELECT uuid FROM LRSAdmin.alf_node WHERE id = @Person);

    BEGIN TRY
        BEGIN TRANSACTION;
        -- name/CRC values are the same as on the user's own People-folder link
        INSERT INTO LRSAdmin.alf_child_assoc (version, parent_node_id, type_qname_id, child_node_name_crc, child_node_name,
                    child_node_id, qname_ns_id, qname_localname, qname_crc, is_primary, assoc_index)
        SELECT 1, @Grp, @MemberType, plagis_admin.fn_crc32(@uuid), @uuid, @Person,
               qname_ns_id, qname_localname, qname_crc, 0, -1
        FROM LRSAdmin.alf_child_assoc WHERE child_node_id = @Person AND is_primary = 1;

        INSERT INTO plagis_admin.audit_log (actor, action, target, details)
        VALUES (@Actor, 'add_to_group', @UserName, @GroupName);
        COMMIT TRANSACTION;
    END TRY
    BEGIN CATCH
        IF @@TRANCOUNT > 0 ROLLBACK TRANSACTION;
        THROW;
    END CATCH
END
GO

IF OBJECT_ID('plagis_admin.usp_remove_from_group') IS NOT NULL DROP PROCEDURE plagis_admin.usp_remove_from_group;
GO
CREATE PROCEDURE plagis_admin.usp_remove_from_group
    @UserName nvarchar(100), @GroupName nvarchar(200), @Actor nvarchar(100)
AS
BEGIN
    SET NOCOUNT ON; SET XACT_ABORT ON;
    DECLARE @Person bigint = plagis_admin.fn_person_node(@UserName),
            @Grp    bigint = plagis_admin.fn_group_node(@GroupName);
    IF @Person IS NULL THROW 50211, 'User not found.', 1;
    IF @Grp    IS NULL THROW 50212, 'Group not found.', 1;

    BEGIN TRY
        BEGIN TRANSACTION;
        DELETE ca FROM LRSAdmin.alf_child_assoc ca
        JOIN LRSAdmin.alf_qname q ON q.id = ca.type_qname_id
        WHERE ca.parent_node_id = @Grp AND ca.child_node_id = @Person AND q.local_name = 'member' AND ca.is_primary = 0;
        IF @@ROWCOUNT = 0 THROW 50213, 'User is not in this group.', 1;

        INSERT INTO plagis_admin.audit_log (actor, action, target, details)
        VALUES (@Actor, 'remove_from_group', @UserName, @GroupName);
        COMMIT TRANSACTION;
    END TRY
    BEGIN CATCH
        IF @@TRANCOUNT > 0 ROLLBACK TRANSACTION;
        THROW;
    END CATCH
END
GO

/* ------------------------- password / enable ------------------------- */
IF OBJECT_ID('plagis_admin.usp_reset_password') IS NOT NULL DROP PROCEDURE plagis_admin.usp_reset_password;
GO
CREATE PROCEDURE plagis_admin.usp_reset_password
    @UserName nvarchar(100), @NewPassword nvarchar(100), @Actor nvarchar(100)
AS
BEGIN
    SET NOCOUNT ON; SET XACT_ABORT ON;
    DECLARE @Login bigint = plagis_admin.fn_login_node(@UserName);
    IF @Login IS NULL THROW 50301, 'User not found.', 1;
    IF LEN(ISNULL(@NewPassword, '')) < 6 THROW 50302, 'Password must be at least 6 characters.', 1;

    DECLARE @PwHash nvarchar(64) = LOWER(CONVERT(varchar(32), HASHBYTES('MD4', @NewPassword), 2));
    IF EXISTS (SELECT 1 FROM LRSAdmin.alf_node_properties p JOIN LRSAdmin.alf_qname q ON q.id = p.qname_id
               WHERE p.node_id = @Login AND q.local_name = 'password'
                 AND p.string_value COLLATE Latin1_General_BIN LIKE '%[A-F]%')
        SET @PwHash = UPPER(@PwHash);

    BEGIN TRY
        BEGIN TRANSACTION;
        DECLARE @Txn bigint;
        EXEC plagis_admin.usp__new_txn @Login, @Txn OUTPUT;

        UPDATE p SET p.string_value = @PwHash
        FROM LRSAdmin.alf_node_properties p JOIN LRSAdmin.alf_qname q ON q.id = p.qname_id
        WHERE p.node_id = @Login AND q.local_name = 'password';
        IF @@ROWCOUNT <> 1 THROW 50303, 'Password property not found exactly once.', 1;

        -- unlock as well, so a locked-out user can log in with the new password
        UPDATE p SET p.boolean_value = 0
        FROM LRSAdmin.alf_node_properties p JOIN LRSAdmin.alf_qname q ON q.id = p.qname_id
        WHERE p.node_id = @Login AND q.local_name = 'accountLocked';

        UPDATE LRSAdmin.alf_node SET version = version + 1, transaction_id = @Txn WHERE id = @Login;

        INSERT INTO plagis_admin.audit_log (actor, action, target, details)
        VALUES (@Actor, 'reset_password', @UserName, NULL);   -- never log the password
        COMMIT TRANSACTION;
    END TRY
    BEGIN CATCH
        IF @@TRANCOUNT > 0 ROLLBACK TRANSACTION;
        THROW;
    END CATCH
END
GO

IF OBJECT_ID('plagis_admin.usp_set_user_enabled') IS NOT NULL DROP PROCEDURE plagis_admin.usp_set_user_enabled;
GO
CREATE PROCEDURE plagis_admin.usp_set_user_enabled
    @UserName nvarchar(100), @Enabled bit, @Actor nvarchar(100)
AS
BEGIN
    SET NOCOUNT ON; SET XACT_ABORT ON;
    DECLARE @Login bigint = plagis_admin.fn_login_node(@UserName);
    IF @Login IS NULL THROW 50311, 'User not found.', 1;
    IF @UserName IN (N'admin', N'AumentumWS', N'MCConfig')
        THROW 50312, 'System accounts cannot be changed here.', 1;

    BEGIN TRY
        BEGIN TRANSACTION;
        DECLARE @Txn bigint;
        EXEC plagis_admin.usp__new_txn @Login, @Txn OUTPUT;

        UPDATE p SET p.boolean_value = @Enabled
        FROM LRSAdmin.alf_node_properties p JOIN LRSAdmin.alf_qname q ON q.id = p.qname_id
        WHERE p.node_id = @Login AND q.local_name = 'enabled';
        IF @@ROWCOUNT <> 1 THROW 50313, 'Enabled property not found exactly once.', 1;

        UPDATE LRSAdmin.alf_node SET version = version + 1, transaction_id = @Txn WHERE id = @Login;

        INSERT INTO plagis_admin.audit_log (actor, action, target, details)
        VALUES (@Actor, CASE WHEN @Enabled = 1 THEN 'enable_user' ELSE 'disable_user' END, @UserName, NULL);
        COMMIT TRANSACTION;
    END TRY
    BEGIN CATCH
        IF @@TRANCOUNT > 0 ROLLBACK TRANSACTION;
        THROW;
    END CATCH
END
GO

/* ------------------------------ update ------------------------------- */
IF OBJECT_ID('plagis_admin.usp_update_user') IS NOT NULL DROP PROCEDURE plagis_admin.usp_update_user;
GO
CREATE PROCEDURE plagis_admin.usp_update_user
    @UserName  nvarchar(100),
    @FirstName nvarchar(100) = NULL,   -- NULL = leave unchanged
    @LastName  nvarchar(100) = NULL,
    @OfficeId  nvarchar(20)  = NULL,
    @Actor     nvarchar(100)
AS
BEGIN
    SET NOCOUNT ON; SET XACT_ABORT ON;
    DECLARE @Person bigint = plagis_admin.fn_person_node(@UserName);
    IF @Person IS NULL THROW 50401, 'User not found.', 1;

    BEGIN TRY
        BEGIN TRANSACTION;
        DECLARE @Txn bigint;
        EXEC plagis_admin.usp__new_txn @Person, @Txn OUTPUT;

        UPDATE p SET p.string_value =
            CASE q.local_name WHEN 'firstName' THEN ISNULL(LTRIM(RTRIM(@FirstName)), p.string_value)
                              WHEN 'lastName'  THEN ISNULL(LTRIM(RTRIM(@LastName)),  p.string_value)
                              WHEN 'officeId'  THEN ISNULL(LTRIM(RTRIM(@OfficeId)),  p.string_value) END
        FROM LRSAdmin.alf_node_properties p JOIN LRSAdmin.alf_qname q ON q.id = p.qname_id
        WHERE p.node_id = @Person AND q.local_name IN ('firstName','lastName','officeId');

        UPDATE LRSAdmin.alf_node SET version = version + 1, transaction_id = @Txn WHERE id = @Person;

        INSERT INTO plagis_admin.audit_log (actor, action, target, details)
        VALUES (@Actor, 'update_user', @UserName,
                CONCAT(N'first=', ISNULL(@FirstName, N'-'), N'; last=', ISNULL(@LastName, N'-'), N'; office=', ISNULL(@OfficeId, N'-')));
        COMMIT TRANSACTION;
    END TRY
    BEGIN CATCH
        IF @@TRANCOUNT > 0 ROLLBACK TRANSACTION;
        THROW;
    END CATCH
END
GO

/* ------------------------------ listing ------------------------------ */
IF OBJECT_ID('plagis_admin.usp_list_users') IS NOT NULL DROP PROCEDURE plagis_admin.usp_list_users;
GO
CREATE PROCEDURE plagis_admin.usp_list_users @Search nvarchar(100) = NULL
AS
BEGIN
    SET NOCOUNT ON;
    ;WITH person AS (
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
END
GO

IF OBJECT_ID('plagis_admin.usp_list_groups') IS NOT NULL DROP PROCEDURE plagis_admin.usp_list_groups;
GO
CREATE PROCEDURE plagis_admin.usp_list_groups
AS
BEGIN
    SET NOCOUNT ON;
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
END
GO

IF OBJECT_ID('plagis_admin.usp_user_groups') IS NOT NULL DROP PROCEDURE plagis_admin.usp_user_groups;
GO
CREATE PROCEDURE plagis_admin.usp_user_groups @UserName nvarchar(100)
AS
BEGIN
    SET NOCOUNT ON;
    DECLARE @Person bigint = plagis_admin.fn_person_node(@UserName);
    IF @Person IS NULL THROW 50501, 'User not found.', 1;
    SELECT g.string_value AS group_name
    FROM LRSAdmin.alf_child_assoc ca
    JOIN LRSAdmin.alf_qname qt ON qt.id = ca.type_qname_id AND qt.local_name = 'member'
    JOIN LRSAdmin.alf_node_properties g ON g.node_id = ca.parent_node_id
    JOIN LRSAdmin.alf_qname qg ON qg.id = g.qname_id AND qg.local_name = 'authorityName'
    WHERE ca.child_node_id = @Person
    ORDER BY g.string_value;
END
GO

/* ---------------------- app login (run separately) ------------------- */
/* Create a dedicated login for the admin web app instead of using sa.
   It can only run these procedures — no direct table access.

USE master;
CREATE LOGIN plagis_admin_app WITH PASSWORD = 'CHANGE-ME-strong-password', CHECK_POLICY = ON;
USE LRS43;
CREATE USER plagis_admin_app FOR LOGIN plagis_admin_app;
GRANT EXECUTE ON SCHEMA::plagis_admin TO plagis_admin_app;
GRANT SELECT  ON plagis_admin.audit_log TO plagis_admin_app;
*/

PRINT 'plagis_admin procedures installed.';
GO
