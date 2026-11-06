-- Registry users given a temporary password: at first sign-in they must set their own, which is
-- changed in the Registry. The row goes once they have. No password is kept here.
CREATE TABLE IF NOT EXISTS cadastre.registry_password_reset (
    login      text        PRIMARY KEY,
    set_by     text,
    created_at timestamptz NOT NULL DEFAULT now()
);
