-- The title deed plan form of a job (Generate Title Deed Plan, task 12 of Deed Plan Creationv2):
-- what the officer confirmed or typed before printing (holder, address, surveyed/plotted/checked
-- by, survey date, tracing no., scale, corner names, the access road side, ...), saved so a reprint
-- is identical. Values the Registry provides are prefilled from it (Aumentum's R_PROPINFO and
-- R_HOLDERS queries) and stored here once the officer saves.
CREATE TABLE IF NOT EXISTS cadastre.title_deed_plan (
    wf_job_instance_id integer     NOT NULL REFERENCES cadastre.wf_job_instance (id) ON UPDATE CASCADE ON DELETE CASCADE,
    upin               text        NOT NULL,
    fields             jsonb       NOT NULL DEFAULT '{}'::jsonb,
    updated_by         text,
    updated_at         timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (wf_job_instance_id, upin)
);
