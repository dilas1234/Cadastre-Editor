# Registry admin (plagis_admin)

The user-administration layer for the Aumentum Registry (LRS43 / Alfresco) on
`10.10.10.5`. The Cadastre server reads the Registry read-only over SOAP; this is
the one place it *writes* to it, and it does so only through stored procedures in a
separate `plagis_admin` schema that never touch the Alfresco tables directly from the
app login.

## Files

- `preflight.sql` — read-only checks to run before installing.
- `plagis_admin_procedures.sql` — the whole database layer: CRC32/node-lookup helpers,
  an `audit_log` table, and ten procedures (`usp_create_user`, `usp_create_group`,
  `usp_add_to_group`, `usp_remove_from_group`, `usp_reset_password`,
  `usp_set_user_enabled`, `usp_update_user`, `usp_list_users`, `usp_list_groups`,
  `usp_user_groups`).

## Install (once, on the Registry's SQL Server)

0. Run `preflight.sql` in SSMS. It is read-only and returns no passwords or hashes. Sort out
   anything marked `FIX` before going on.
1. Back up `LRS43`.
2. Run the whole `plagis_admin_procedures.sql` in SSMS. Re-running is safe (it drops
   and recreates the procedures; the audit log table is kept).
3. Create the dedicated app login (footer of the SQL file), replacing the password:

   ```sql
   USE master;
   CREATE LOGIN plagis_admin_app WITH PASSWORD = 'CHANGE-ME-strong-password', CHECK_POLICY = ON;
   USE LRS43;
   CREATE USER plagis_admin_app FOR LOGIN plagis_admin_app;
   GRANT EXECUTE ON SCHEMA::plagis_admin TO plagis_admin_app;
   GRANT SELECT  ON plagis_admin.audit_log TO plagis_admin_app;
   ```

4. Smoke-test with a throwaway user before the dashboard relies on it:

   ```sql
   EXEC plagis_admin.usp_create_user @UserName='ztest', @FirstName='Test', @LastName='User',
        @Password='pa55W0rd', @RefUser='cgugin', @Actor='che';
   EXEC plagis_admin.usp_list_users @Search='ztest';
   ```

   Restart JBoss and sign in as `ztest`, then exercise `usp_reset_password` and
   `usp_set_user_enabled`, restarting and testing the login after each. This proves the
   reset/disable paths (only create-user has been run on this server before).

> **After any change, restart the Registry (JBoss) service** — Alfresco caches users
> and groups in memory, so changes do not take effect until it restarts.

## The dashboard

The Cadastre server reaches these procedures through `tiberius` behind the
`registry-admin` cargo feature (the default build stays TLS-free for the win7 target,
NFR-02). Build with the feature on the host that runs the admin tool:

```
cargo run -p cadastre-server --features registry-admin
```

Connection settings (the `plagis_admin_app` login above):

```
CADASTRE_REGISTRY_SQL_HOST=10.10.10.5
CADASTRE_REGISTRY_SQL_PORT=1433
CADASTRE_REGISTRY_SQL_DATABASE=LRS43
CADASTRE_REGISTRY_SQL_USER=plagis_admin_app
CADASTRE_REGISTRY_SQL_PASSWORD=...
```

The dashboard is the User Management page (`/user-management`), also in the Sysadmin view
(**Administration… → Registry Users**), and is
reachable only by administrators: the Registry's `GROUP_LRS_ADMINISTRATION_SYSTEM` and this
server's Administrators group. Every
change is written to `plagis_admin.audit_log` (who, what, when — never passwords).

Changes need `CADASTRE_REGISTRY_WRITE=1`; with it off the dashboard can only read. A password
set here is temporary: the user must replace it at their next sign-in, which needs
`CADASTRE_AUTH=local`. `usp_reset_password` refuses `admin`, `AumentumWS` and `MCConfig`.
