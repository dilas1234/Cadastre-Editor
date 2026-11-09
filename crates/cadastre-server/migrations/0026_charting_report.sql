-- The Survey Report on Right of Occupancy, one per file (docs/charting-and-reporting.docx, C4). The
-- charting officer signs the first part and a second officer the Final Survey Report. Every save
-- is kept as a version.
CREATE TABLE IF NOT EXISTS cadastre.charting_report (
    wf_job_instance_id integer     PRIMARY KEY REFERENCES cadastre.wf_job_instance (id),
    answers            jsonb       NOT NULL DEFAULT '{}'::jsonb,
    state              text        NOT NULL DEFAULT 'draft' CHECK (state IN ('draft', 'charted', 'final')),
    version            integer     NOT NULL DEFAULT 1,
    charted_by         text,
    charted_at         timestamptz,
    final_by           text,
    final_at           timestamptz,
    updated_by         text        NOT NULL,
    updated_at         timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS cadastre.charting_report_version (
    wf_job_instance_id integer     NOT NULL REFERENCES cadastre.charting_report (wf_job_instance_id),
    version            integer     NOT NULL,
    answers            jsonb       NOT NULL,
    state              text        NOT NULL,
    saved_by           text        NOT NULL,
    saved_at           timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (wf_job_instance_id, version)
);
