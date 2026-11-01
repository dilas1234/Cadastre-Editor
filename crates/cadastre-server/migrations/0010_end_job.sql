-- "End Job" (Aumentum: a job may be ended from any task by a user allowed to end jobs, and
-- deleted with ds_delete_jobinstance). Ending as completed uses status 3; cancelling uses a new,
-- explicit status 9: Aumentum's codes 4, 5 and 7 are ended states whose exact meaning PLAGIS has
-- not confirmed, so the new system does not guess which of them means "cancelled".
INSERT INTO cadastre.wf_job_status (code, label, is_open, confirmed)
VALUES (9, 'Cancelled', false, true)
ON CONFLICT (code) DO NOTHING;
