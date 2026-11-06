-- Create a Registry user without the plagis_admin procedures: usp_create_user of
-- tools/registry-admin/plagis_admin_procedures.sql, run as one batch by a login that can write the
-- Alfresco tables (`CADASTRE_REGISTRY_SQL_DIRECT`). Keep the two in step. The server checks the
-- username and works out the GUIDs and CRCs (@P7-@P17); it keeps the audit record in the Cadastre.
SET NOCOUNT ON; SET XACT_ABORT ON;
DECLARE @UserName nvarchar(100) = @P1, @FirstName nvarchar(100) = @P2, @LastName nvarchar(100) = @P3,
        @Password nvarchar(100) = @P4, @RefUser nvarchar(100) = @P5, @ExtraGroups nvarchar(max) = @P6,
        @pu varchar(36) = @P7, @uu varchar(36) = @P8, @pan varchar(36) = @P9, @uan varchar(36) = @P10,
        @AuthCrc bigint = @P11, @CmCrc bigint = @P12, @UsrCrc bigint = @P13, @PuCrc bigint = @P14,
        @PanCrc bigint = @P15, @UanCrc bigint = @P16, @RefCrc bigint = @P17;

DECLARE @Person TABLE (userName nvarchar(100), id bigint);
INSERT INTO @Person
SELECT p.string_value, n.id
FROM LRSAdmin.alf_node n
JOIN LRSAdmin.alf_store s ON s.id = n.store_id
JOIN LRSAdmin.alf_qname t ON t.id = n.type_qname_id
JOIN LRSAdmin.alf_node_properties p ON p.node_id = n.id
JOIN LRSAdmin.alf_qname q ON q.id = p.qname_id
WHERE s.protocol = 'workspace' AND s.identifier = 'SpacesStore' AND t.local_name = 'person'
  AND q.local_name = 'userName' AND p.string_value IN (@UserName, @RefUser) AND n.node_deleted = 0;

DECLARE @Login TABLE (userName nvarchar(100), id bigint);
INSERT INTO @Login
SELECT p.string_value, n.id
FROM LRSAdmin.alf_node n
JOIN LRSAdmin.alf_store s ON s.id = n.store_id
JOIN LRSAdmin.alf_qname t ON t.id = n.type_qname_id
JOIN LRSAdmin.alf_node_properties p ON p.node_id = n.id
JOIN LRSAdmin.alf_qname q ON q.id = p.qname_id
WHERE s.protocol = 'user' AND s.identifier = 'alfrescoUserStore' AND t.local_name = 'user'
  AND q.local_name = 'username' AND p.string_value IN (@UserName, @RefUser) AND n.node_deleted = 0;

IF EXISTS (SELECT 1 FROM @Person WHERE userName = @UserName) OR EXISTS (SELECT 1 FROM @Login WHERE userName = @UserName)
   OR EXISTS (SELECT 1 FROM LRSAdmin.alf_authority WHERE authority = @UserName)
    THROW 50004, 'Username already exists.', 1;

DECLARE @RefPerson bigint = (SELECT TOP 1 id FROM @Person WHERE userName = @RefUser),
        @RefLogin  bigint = (SELECT TOP 1 id FROM @Login WHERE userName = @RefUser);
IF @RefPerson IS NULL OR @RefLogin IS NULL
    THROW 50005, 'Reference user not found.', 1;

-- CRC self-test against the reference user's real People-folder link
IF @RefCrc <> (SELECT qname_crc FROM LRSAdmin.alf_child_assoc WHERE child_node_id = @RefPerson AND is_primary = 1)
    THROW 50006, 'CRC self-test failed. Nothing was changed.', 1;

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
    UPDATE e SET nodeId = (SELECT TOP 1 n.id
                           FROM LRSAdmin.alf_node n
                           JOIN LRSAdmin.alf_qname t ON t.id = n.type_qname_id
                           JOIN LRSAdmin.alf_node_properties p ON p.node_id = n.id
                           JOIN LRSAdmin.alf_qname q ON q.id = p.qname_id
                           WHERE t.local_name = 'authorityContainer' AND q.local_name = 'authorityName'
                             AND p.string_value = e.groupName AND n.node_deleted = 0)
    FROM @Extra e;
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

BEGIN TRY
    BEGIN TRANSACTION;
    DECLARE @Txn bigint, @NewPerson bigint, @NewLogin bigint;

    INSERT INTO LRSAdmin.alf_transaction (version, server_id, change_txn_id, commit_time_ms)
    SELECT 1, t.server_id, LOWER(CONVERT(varchar(36), NEWID())),
           CAST(DATEDIFF(SECOND, '19700101', GETUTCDATE()) AS bigint) * 1000
    FROM LRSAdmin.alf_transaction t JOIN LRSAdmin.alf_node n ON n.transaction_id = t.id
    WHERE n.id = @RefPerson;
    SET @Txn = SCOPE_IDENTITY();
    IF @Txn IS NULL THROW 50000, 'Could not create transaction row.', 1;

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

    COMMIT TRANSACTION;
END TRY
BEGIN CATCH
    IF @@TRANCOUNT > 0 ROLLBACK TRANSACTION;
    THROW;
END CATCH
