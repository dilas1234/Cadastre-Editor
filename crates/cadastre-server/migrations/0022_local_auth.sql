-- Sign-in on this server (CADASTRE_AUTH=local): the accounts stay in cadastre.principal; a password
-- is set by an administrator and changed by its owner at first sign-in. Only a hash is stored.
CREATE TABLE IF NOT EXISTS cadastre.user_credential (
    sid           integer     PRIMARY KEY REFERENCES cadastre.principal (sid) ON UPDATE CASCADE ON DELETE CASCADE,
    password_hash text        NOT NULL,
    must_change   boolean     NOT NULL DEFAULT true,
    failed_count  integer     NOT NULL DEFAULT 0,
    locked_until  timestamptz,
    set_by        text,
    updated_at    timestamptz NOT NULL DEFAULT now(),
    last_login    timestamptz
);

-- Signed-in browsers. The cookie holds a random token; only its SHA-256 is kept here.
CREATE TABLE IF NOT EXISTS cadastre.user_session (
    token_hash text        PRIMARY KEY,
    sid        integer     NOT NULL REFERENCES cadastre.principal (sid) ON UPDATE CASCADE ON DELETE CASCADE,
    view       text        NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL
);
CREATE INDEX IF NOT EXISTS user_session_sid_idx ON cadastre.user_session (sid);
CREATE INDEX IF NOT EXISTS user_session_expires_idx ON cadastre.user_session (expires_at);
