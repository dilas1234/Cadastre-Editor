-- This Cadastre's own job and task numbers: 1,000,000 and up.
--
-- Aumentum keeps creating jobs during the parallel run, and its ids (17,536 on 2026-11-03,
-- ~3,000 a year) used to share one series with the jobs created here: wf_sync_sequences() moved
-- the counters to Aumentum's max, so the next reload of Aumentum's workflow met numbers already
-- used here. The job upsert then skipped Aumentum's job, and the task upsert overwrote ours (on
-- the dev server, job 17511 / Job-PL67363 was lost that way). From now on:
--   * jobs and task instances created here are numbered from 1,000,001 up;
--   * Aumentum's ids (below 1,000,000) never move these counters;
--   * jobs already created here below 1,000,000 are renumbered into the range, and every table
--     holding the number follows (ON UPDATE CASCADE); the old -> new numbers are kept in
--     cadastre.wf_renumbered so a number given to someone earlier can still be traced.

CREATE OR REPLACE FUNCTION cadastre.wf_own_base() RETURNS integer LANGUAGE sql IMMUTABLE AS $$ SELECT 1000000 $$;

CREATE TABLE IF NOT EXISTS cadastre.wf_renumbered (
    kind        text        NOT NULL CHECK (kind IN ('job', 'task')),
    old_id      integer     NOT NULL,
    new_id      integer     NOT NULL,
    renumbered  timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (kind, old_id)
);

-- Every holder of a job number follows a renumbering.
ALTER TABLE cadastre.wf_task_instance DROP CONSTRAINT wf_task_instance_job_instance_id_fkey,
  ADD CONSTRAINT wf_task_instance_job_instance_id_fkey FOREIGN KEY (job_instance_id)
  REFERENCES cadastre.wf_job_instance (id) ON UPDATE CASCADE ON DELETE CASCADE;
ALTER TABLE cadastre.job_area DROP CONSTRAINT job_area_wf_job_instance_id_fkey,
  ADD CONSTRAINT job_area_wf_job_instance_id_fkey FOREIGN KEY (wf_job_instance_id)
  REFERENCES cadastre.wf_job_instance (id) ON UPDATE CASCADE ON DELETE CASCADE;
ALTER TABLE cadastre.wf_job_file DROP CONSTRAINT wf_job_file_job_instance_id_fkey,
  ADD CONSTRAINT wf_job_file_job_instance_id_fkey FOREIGN KEY (job_instance_id)
  REFERENCES cadastre.wf_job_instance (id) ON UPDATE CASCADE ON DELETE CASCADE;
ALTER TABLE cadastre.wf_notification DROP CONSTRAINT wf_notification_job_instance_id_fkey,
  ADD CONSTRAINT wf_notification_job_instance_id_fkey FOREIGN KEY (job_instance_id)
  REFERENCES cadastre.wf_job_instance (id) ON UPDATE CASCADE ON DELETE CASCADE;
ALTER TABLE cadastre.parcel_lineage DROP CONSTRAINT parcel_lineage_wf_job_instance_id_fkey,
  ADD CONSTRAINT parcel_lineage_wf_job_instance_id_fkey FOREIGN KEY (wf_job_instance_id)
  REFERENCES cadastre.wf_job_instance (id) ON UPDATE CASCADE ON DELETE SET NULL;
ALTER TABLE cadastre.wf_job_linkage DROP CONSTRAINT wf_job_linkage_wf_job_instance_id_fkey,
  ADD CONSTRAINT wf_job_linkage_wf_job_instance_id_fkey FOREIGN KEY (wf_job_instance_id)
  REFERENCES cadastre.wf_job_instance (id) ON UPDATE CASCADE ON DELETE CASCADE;

-- Counters: only numbers in this Cadastre's range count. Never below the base, never backwards.
CREATE OR REPLACE FUNCTION cadastre.wf_sync_sequences() RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    base constant integer := cadastre.wf_own_base();
BEGIN
    PERFORM setval('cadastre.wf_job_instance_seq', GREATEST(base,
        COALESCE((SELECT max(id) FROM cadastre.wf_job_instance WHERE id > base), base),
        (SELECT last_value FROM cadastre.wf_job_instance_seq)), true);
    PERFORM setval('cadastre.wf_task_instance_seq', GREATEST(base,
        COALESCE((SELECT max(id) FROM cadastre.wf_task_instance WHERE id > base), base),
        (SELECT last_value FROM cadastre.wf_task_instance_seq)), true);
END $$;

-- Renumber the jobs created here that still sit in Aumentum's range, with their task instances.
DO $$
DECLARE
    base constant integer := cadastre.wf_own_base();
    next_job integer := GREATEST(base, COALESCE((SELECT max(id) FROM cadastre.wf_job_instance WHERE id > base), base));
    next_task integer := GREATEST(base, COALESCE((SELECT max(id) FROM cadastre.wf_task_instance WHERE id > base), base));
    j record;
    t record;
BEGIN
    FOR j IN SELECT id FROM cadastre.wf_job_instance
             WHERE source IS DISTINCT FROM 'aumentum' AND id <= base ORDER BY id LOOP
        next_job := next_job + 1;
        INSERT INTO cadastre.wf_renumbered (kind, old_id, new_id) VALUES ('job', j.id, next_job)
            ON CONFLICT (kind, old_id) DO NOTHING;
        UPDATE cadastre.wf_job_instance SET id = next_job WHERE id = j.id;   -- children cascade
        UPDATE cadastre.job SET reference = 'WF-' || next_job WHERE reference = 'WF-' || j.id;
        FOR t IN SELECT id FROM cadastre.wf_task_instance
                 WHERE job_instance_id = next_job AND id <= base ORDER BY id LOOP
            next_task := next_task + 1;
            INSERT INTO cadastre.wf_renumbered (kind, old_id, new_id) VALUES ('task', t.id, next_task)
                ON CONFLICT (kind, old_id) DO NOTHING;
            UPDATE cadastre.wf_task_instance SET id = next_task WHERE id = t.id;
        END LOOP;
    END LOOP;
    PERFORM cadastre.wf_sync_sequences();
END $$;
