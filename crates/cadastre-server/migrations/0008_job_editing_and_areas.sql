-- Jobs drive editing (Phase 2): each Aumentum job instance gets its own editing job (drafts,
-- posting) on first use; a job area bounds where its parcels may be drawn, as Aumentum's
-- "Job Area" does; application settings carry Aumentum's editor settings (dsssettings).

ALTER TABLE cadastre.wf_job_instance
    ADD COLUMN IF NOT EXISTS cadastre_job_id uuid UNIQUE REFERENCES cadastre.job (id) ON DELETE SET NULL;

-- One job area per job instance (Aumentum: create / load / edit / save / discard / close / delete).
CREATE TABLE IF NOT EXISTS cadastre.job_area (
    wf_job_instance_id integer PRIMARY KEY REFERENCES cadastre.wf_job_instance (id) ON DELETE CASCADE,
    geom        geometry(Polygon, 32632) NOT NULL,
    status      text NOT NULL DEFAULT 'open' CHECK (status IN ('open', 'closed')),
    created_by  text NOT NULL,
    created_at  timestamptz NOT NULL DEFAULT now(),
    updated_by  text,
    updated_at  timestamptz NOT NULL DEFAULT now(),
    closed_at   timestamptz
);
CREATE INDEX IF NOT EXISTS job_area_geom_gix ON cadastre.job_area USING gist (geom);

-- Application settings (Aumentum dsssettings, category.name -> value).
CREATE TABLE IF NOT EXISTS cadastre.app_setting (
    key        text PRIMARY KEY,          -- e.g. editor.allow_overlap
    value      text,
    source     text NOT NULL DEFAULT 'cadastre',   -- 'aumentum' when taken from dsssettings
    updated_by text,
    updated_at timestamptz NOT NULL DEFAULT now()
);
INSERT INTO cadastre.app_setting (key, value, source) VALUES
    ('editor.allow_overlap', '0', 'aumentum'),          -- Editor.AllowOverlap: committed parcels must not overlap the fabric
    ('editor.lock_expire_minutes', '10080', 'aumentum'), -- #config.lock_expire_time (7 days)
    ('editor.label_min_scale', '100000', 'aumentum'),    -- Editor.ScaleLabel OutBoundMinScale
    ('mail.smtp_server', '', 'aumentum'),                -- Office.smtpServer (blank in production: no e-mail sent)
    ('mail.smtp_port', '25', 'aumentum'),
    ('mail.from', '', 'aumentum')
ON CONFLICT (key) DO NOTHING;
