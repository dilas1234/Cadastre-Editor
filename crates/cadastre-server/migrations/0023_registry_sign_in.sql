-- Sign-in with Registry users: the Registry checks the password, and what its groups allow is kept
-- on the session (Registry users have no password here). The lockout now covers every login name,
-- so it also protects accounts that live in the Registry.
ALTER TABLE cadastre.user_session
    ADD COLUMN IF NOT EXISTS source         text    NOT NULL DEFAULT 'local',
    ADD COLUMN IF NOT EXISTS registry_admin boolean NOT NULL DEFAULT false,
    ADD COLUMN IF NOT EXISTS registry_staff boolean NOT NULL DEFAULT false;

CREATE TABLE IF NOT EXISTS cadastre.login_throttle (
    login        text        PRIMARY KEY,
    failed_count integer     NOT NULL DEFAULT 0,
    locked_until timestamptz
);
