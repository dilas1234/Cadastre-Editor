#!/usr/bin/env bash
# Load Aumentum's workflow engine (MCServer export: aumentum-export-mcserver.ps1) into the new
# Cadastre: users/groups/roles, job types, tasks, steps, task graph, permissions, every job
# instance and task instance, and the UPIN register (reserved in the UPIN ledger).
#
#   tools/import/load-aumentum-workflow.sh <export-dir>/mcserver
#
# Re-runnable: rows are upserted on Aumentum's ids, so it can be run again during the parallel
# run to pick up new and changed jobs. Ends with a file-vs-database reconciliation.
# Env: PSQL (default: psql inside the cadastre-db container, database cadastre).
set -euo pipefail
DIR="${1:?usage: load-aumentum-workflow.sh <export-dir>/mcserver}"
PSQL="${PSQL:-docker exec -i cadastre-db psql -U cadastre -d cadastre -v ON_ERROR_STOP=1 -q -At}"
need=(sec_sids sec_userinfo sec_sid_nesting role sec_policy wftask Step TaskSteps wfjob wftaskflow wftaskPermission wfjobPermission wfjobInstance wfTaskInstance UPIN
      object version value_string value_page event event_log auditlog dsssettings)
for t in "${need[@]}"; do [ -f "$DIR/dbo.$t.psv" ] || { echo "missing: $DIR/dbo.$t.psv" >&2; exit 1; }; done

copy_opts="(FORMAT csv, DELIMITER '|', HEADER true, NULL '', QUOTE E'\\x01')"
# File as Unix lines (the export is written on Windows) with exactly one final newline.
with_eol() { tr -d '\r' < "$1"; [ -z "$(tail -c1 "$1" | tr -d '\r')" ] || echo; }
# stage <table> "<col defs>": a temp table named s_<table> (plus a line number), loaded from the file.
stage() {
  printf 'CREATE TEMP TABLE s_%s (line_no bigserial, %s);\nCOPY s_%s (%s) FROM STDIN %s;\n' \
    "$1" "$2" "$1" "$(sed -E 's/ text//g' <<<"$2")" "$copy_opts"
  with_eol "$DIR/dbo.$1.psv"; printf '\\.\n'
}
rows() { echo $(( $(tr -d '\r' < "$DIR/dbo.$1.psv" | grep -cv '^$') - 1 )); }

{
echo "BEGIN;"
stage sec_sids "sid text, code text, name text, descr text, is_user text, is_locked text, can_change_pass text, can_be_expired text, os_account text, os_sid__bytes text, is_system text"
stage sec_userinfo "sid text, login text, name text, department text, office text, email text, phone text, job_title text, office_id text"
stage sec_sid_nesting "parent_sid text, child_sid text"
stage role "code text, description text"
stage sec_policy "id text, sid text, role text, allow text"
stage wftask "id text, name text, description text"
stage Step "id text, name text, iconImage text, commands text, executeOrApplyCommand text"
stage TaskSteps "taskId text, stepId text"
stage wfjob "id text, job text, description text, startTaskID text, endTaskID text, workflowDesignXML text"
stage wftaskflow "jobID text, taskID text, taskOrder text, nextTaskID text"
stage wftaskPermission "taskID text, sid text, isDefault text"
stage wfjobPermission "jobID text, sid text, canStart text, canEnd text, canUpdate text, canDelete text, canExecute text"
stage wfjobInstance "id text, jobID text, taskID text, sid text, priority text, started text, updated text, ended text, name text, COMMENT text, parameters text, lrsTransactionNumber text, jobstatus text, created text, lrsKey text, lrsProperties text, linkParcelsProperties text"
stage wfTaskInstance "id text, jobID text, taskID text, sid text, assigned text, started text, ended text, jobInstanceID text, comment text, parameters text"
stage UPIN "id text, upin text, beg_date text, end_date text, status text, parcelID text, regionID text"
stage object "id text, class text, updated text, deleted text, version text, owner text"
stage version "object_id text, version text, sid text, time text"
stage value_string "id text, object_id text, prop_id text, value text, index text, ver_born text, ver_dead text"
stage value_page "id text, object_id text, prop_id text, value text, index text, ver_born text, ver_dead text, file_name text, mime_type text, file_size text, img_width text, img_height text, img_bpp text, img_dpi text, ocr_text text, date_modified text"
stage event "id text, eventName text"
stage event_log "id text, time text, eventid text, userid text, module text, regionid text, oldValue text, newValue text, ip text, jobInstanceId text, taskInstanceId text"
stage auditlog "id text, sid text, action text, object text, success text, time text, description text"
stage dsssettings "id text, category text, name text, value text"
cat <<'SQL'
-- Aumentum's job and task ids stay below this Cadastre's own range (migration 0019); an export
-- with ids in that range is refused rather than mixed with the jobs created here.
DO $$
BEGIN
  IF EXISTS (SELECT 1 FROM s_wfjobInstance WHERE id::int > cadastre.wf_own_base())
     OR EXISTS (SELECT 1 FROM s_wfTaskInstance WHERE id::int > cadastre.wf_own_base()) THEN
    RAISE EXCEPTION 'Aumentum job/task ids of %+ found in the export: that range belongs to this Cadastre; nothing was loaded',
      cadastre.wf_own_base();
  END IF;
END $$;

-- People
INSERT INTO cadastre.principal (sid, code, name, description, is_user, is_locked, is_system, os_account, email, department, job_title, phone)
SELECT s.sid::int, s.code, COALESCE(u.name, s.name), s.descr, s.is_user = '1', s.is_locked = '1', s.is_system = '1', s.os_account,
       u.email, u.department, u.job_title, u.phone
FROM s_sec_sids s LEFT JOIN s_sec_userinfo u ON u.sid = s.sid
ON CONFLICT (sid) DO UPDATE SET code = EXCLUDED.code, name = EXCLUDED.name, description = EXCLUDED.description,
  is_user = EXCLUDED.is_user, is_locked = EXCLUDED.is_locked, is_system = EXCLUDED.is_system, os_account = EXCLUDED.os_account,
  email = EXCLUDED.email, department = EXCLUDED.department, job_title = EXCLUDED.job_title, phone = EXCLUDED.phone;
INSERT INTO cadastre.principal_member (group_sid, member_sid)
SELECT n.parent_sid::int, n.child_sid::int FROM s_sec_sid_nesting n
WHERE EXISTS (SELECT 1 FROM cadastre.principal p WHERE p.sid = n.parent_sid::int) AND EXISTS (SELECT 1 FROM cadastre.principal p WHERE p.sid = n.child_sid::int)
ON CONFLICT DO NOTHING;
INSERT INTO cadastre.role (code, description) SELECT code, description FROM s_role
ON CONFLICT (code) DO UPDATE SET description = EXCLUDED.description;
INSERT INTO cadastre.principal_role (sid, role, allow)
SELECT DISTINCT ON (sp.sid::int, sp.role) sp.sid::int, sp.role, sp.allow = '1' FROM s_sec_policy sp
WHERE EXISTS (SELECT 1 FROM cadastre.principal p WHERE p.sid = sp.sid::int) AND EXISTS (SELECT 1 FROM cadastre.role r WHERE r.code = sp.role)
ORDER BY sp.sid::int, sp.role, sp.id::int DESC
ON CONFLICT (sid, role) DO UPDATE SET allow = EXCLUDED.allow;

-- Definitions
INSERT INTO cadastre.wf_task (id, name, description) SELECT id::int, name, description FROM s_wftask
ON CONFLICT (id) DO UPDATE SET name = EXCLUDED.name, description = EXCLUDED.description;
INSERT INTO cadastre.wf_step (id, name, icon, commands, execute_or_apply)
SELECT id::int, name, iconImage,
       ARRAY(SELECT btrim(m[1]) FROM regexp_matches(COALESCE(commands, ''), '<command>([^<]*)</command>', 'g') m), executeOrApplyCommand
FROM s_Step
ON CONFLICT (id) DO UPDATE SET name = EXCLUDED.name, icon = EXCLUDED.icon, commands = EXCLUDED.commands, execute_or_apply = EXCLUDED.execute_or_apply;
INSERT INTO cadastre.wf_task_step (task_id, step_id, ord)
SELECT taskId::int, stepId::int, line_no FROM s_TaskSteps
WHERE taskId::int IN (SELECT id FROM cadastre.wf_task) AND stepId::int IN (SELECT id FROM cadastre.wf_step)
ON CONFLICT (task_id, step_id) DO UPDATE SET ord = EXCLUDED.ord;
INSERT INTO cadastre.wf_job_type (id, name, description, start_task_id, end_task_id, design_xml, active)
SELECT id::int, job, description, NULLIF(startTaskID, '')::int, NULLIF(endTaskID, '')::int, workflowDesignXML, NULLIF(startTaskID, '') IS NOT NULL
FROM s_wfjob
ON CONFLICT (id) DO UPDATE SET name = EXCLUDED.name, description = EXCLUDED.description, start_task_id = EXCLUDED.start_task_id,
  end_task_id = EXCLUDED.end_task_id, design_xml = EXCLUDED.design_xml, active = EXCLUDED.active;
DELETE FROM cadastre.wf_task_flow WHERE job_type_id IN (SELECT id::int FROM s_wfjob);
INSERT INTO cadastre.wf_task_flow (job_type_id, task_id, task_order, next_task_id)
SELECT DISTINCT jobID::int, taskID::int, NULLIF(taskOrder, '')::int, NULLIF(nextTaskID, '')::int FROM s_wftaskflow
ON CONFLICT DO NOTHING;
INSERT INTO cadastre.wf_task_permission (task_id, sid, is_default)
SELECT taskID::int, sid::int, isDefault = '1' FROM s_wftaskPermission
WHERE taskID::int IN (SELECT id FROM cadastre.wf_task) AND sid::int IN (SELECT sid FROM cadastre.principal)
ON CONFLICT (task_id, sid) DO UPDATE SET is_default = EXCLUDED.is_default;
INSERT INTO cadastre.wf_job_permission (job_type_id, sid, can_start, can_end, can_update, can_delete, can_execute)
SELECT jobID::int, sid::int, canStart = '1', canEnd = '1', canUpdate = '1', canDelete = '1', canExecute = '1' FROM s_wfjobPermission
WHERE jobID::int IN (SELECT id FROM cadastre.wf_job_type) AND sid::int IN (SELECT sid FROM cadastre.principal)
ON CONFLICT (job_type_id, sid) DO UPDATE SET can_start = EXCLUDED.can_start, can_end = EXCLUDED.can_end,
  can_update = EXCLUDED.can_update, can_delete = EXCLUDED.can_delete, can_execute = EXCLUDED.can_execute;

-- Work. A status code not yet known is registered (open, unconfirmed) rather than dropped.
INSERT INTO cadastre.wf_job_status (code, label, is_open, confirmed)
SELECT DISTINCT jobstatus::smallint, 'Code ' || jobstatus, true, false FROM s_wfjobInstance WHERE jobstatus <> ''
ON CONFLICT (code) DO NOTHING;
INSERT INTO cadastre.wf_job_instance (id, job_type_id, current_task_id, assigned_sid, priority, status, name, comment, parameters,
  lrs_transaction_number, lrs_key, lrs_properties, link_parcels_properties, created, started, updated, ended, source)
SELECT j.id::int, j.jobID::int,
       CASE WHEN NULLIF(j.taskID, '')::int IN (SELECT id FROM cadastre.wf_task) THEN NULLIF(j.taskID, '')::int END,
       CASE WHEN NULLIF(j.sid, '')::int IN (SELECT sid FROM cadastre.principal) THEN NULLIF(j.sid, '')::int END,
       j.priority, COALESCE(NULLIF(j.jobstatus, ''), '1')::smallint, j.name, j.COMMENT, j.parameters,
       NULLIF(j.lrsTransactionNumber, ''), j.lrsKey, j.lrsProperties, j.linkParcelsProperties,
       NULLIF(j.created, '')::timestamp, NULLIF(j.started, '')::timestamp, NULLIF(j.updated, '')::timestamp, NULLIF(j.ended, '')::timestamp,
       'aumentum'
FROM s_wfjobInstance j WHERE j.jobID::int IN (SELECT id FROM cadastre.wf_job_type)
ON CONFLICT (id) DO UPDATE SET job_type_id = EXCLUDED.job_type_id, current_task_id = EXCLUDED.current_task_id,
  assigned_sid = EXCLUDED.assigned_sid, priority = EXCLUDED.priority, status = EXCLUDED.status, name = EXCLUDED.name,
  comment = EXCLUDED.comment, parameters = EXCLUDED.parameters, lrs_transaction_number = EXCLUDED.lrs_transaction_number,
  lrs_key = EXCLUDED.lrs_key, lrs_properties = EXCLUDED.lrs_properties, link_parcels_properties = EXCLUDED.link_parcels_properties,
  created = EXCLUDED.created, started = EXCLUDED.started, updated = EXCLUDED.updated, ended = EXCLUDED.ended
WHERE cadastre.wf_job_instance.source = 'aumentum';
INSERT INTO cadastre.wf_task_instance (id, job_instance_id, job_type_id, task_id, sid, assigned, started, ended, comment, parameters)
SELECT t.id::int, t.jobInstanceID::int, NULLIF(t.jobID, '')::int, t.taskID::int,
       CASE WHEN NULLIF(t.sid, '')::int IN (SELECT sid FROM cadastre.principal) THEN NULLIF(t.sid, '')::int END,
       NULLIF(t.assigned, '')::timestamp, NULLIF(t.started, '')::timestamp, NULLIF(t.ended, '')::timestamp, t.comment, t.parameters
FROM s_wfTaskInstance t
WHERE t.jobInstanceID::int IN (SELECT id FROM cadastre.wf_job_instance) AND t.taskID::int IN (SELECT id FROM cadastre.wf_task)
ON CONFLICT (id) DO UPDATE SET job_type_id = EXCLUDED.job_type_id, task_id = EXCLUDED.task_id,
  sid = EXCLUDED.sid, assigned = EXCLUDED.assigned, started = EXCLUDED.started, ended = EXCLUDED.ended,
  comment = EXCLUDED.comment, parameters = EXCLUDED.parameters
-- Only Aumentum's own task row of the same job is refreshed. A row with this number that belongs to
-- another job was made here (before migration 0019 moved this Cadastre's numbers): it is kept, and
-- reported below as a clash.
WHERE cadastre.wf_task_instance.job_instance_id = EXCLUDED.job_instance_id
  AND EXISTS (SELECT 1 FROM cadastre.wf_job_instance w WHERE w.id = EXCLUDED.job_instance_id AND w.source = 'aumentum');
SELECT cadastre.wf_sync_sequences();

-- The UPIN register: every UPIN Aumentum allocated is reserved, used by a parcel or not.
INSERT INTO cadastre.issued_upin (upin, source)
SELECT DISTINCT btrim(upin), 'aumentum:register' FROM s_UPIN WHERE btrim(upin) NOT IN ('', '0')
ON CONFLICT (upin) DO NOTHING;
SELECT cadastre.sync_upin_sequence();

-- Job attachments (mc_class objects): jobInstanceName (prop 19) links the job, fileName (20) is
-- the original name, fileContent (21) the file in Aumentum's document store. Current versions only.
INSERT INTO cadastre.wf_job_file (job_instance_id, file_name, store, store_path, size_bytes, mime_type, uploaded_by_sid, uploaded_at, aumentum_object_id)
SELECT js.value::int, COALESCE(NULLIF(fn.value, ''), pg.file_name), 'aumentum', pg.value, NULLIF(pg.file_size, '')::bigint, NULLIF(pg.mime_type, ''),
       CASE WHEN NULLIF(v.sid, '')::int IN (SELECT sid FROM cadastre.principal) THEN v.sid::int END, NULLIF(v.time, '')::timestamp, o.id::uuid
FROM s_object o
JOIN s_value_string js ON js.object_id = o.id AND js.prop_id = '19' AND js.ver_dead IS NULL
JOIN s_value_page pg ON pg.object_id = o.id AND pg.prop_id = '21' AND pg.ver_dead IS NULL
LEFT JOIN s_value_string fn ON fn.object_id = o.id AND fn.prop_id = '20' AND fn.ver_dead IS NULL
LEFT JOIN s_version v ON v.object_id = o.id AND v.version = o.version
WHERE o.class = '3' AND o.deleted = '0' AND js.value ~ '^[0-9]+$' AND js.value::int IN (SELECT id FROM cadastre.wf_job_instance)
ON CONFLICT (aumentum_object_id) DO UPDATE SET job_instance_id = EXCLUDED.job_instance_id, file_name = EXCLUDED.file_name,
  store_path = EXCLUDED.store_path, size_bytes = EXCLUDED.size_bytes, mime_type = EXCLUDED.mime_type,
  uploaded_by_sid = EXCLUDED.uploaded_by_sid, uploaded_at = EXCLUDED.uploaded_at;

-- Event log and security audit log, verbatim.
INSERT INTO cadastre.aumentum_event (id, name) SELECT id::int, eventName FROM s_event ON CONFLICT (id) DO UPDATE SET name = EXCLUDED.name;
INSERT INTO cadastre.aumentum_event_log (id, time, event_id, user_sid, module, region_id, old_value, new_value, ip, job_instance_id, task_instance_id)
SELECT id::int, NULLIF(time, '')::timestamp, NULLIF(eventid, '')::int, NULLIF(userid, '')::int, module, regionid, oldValue, newValue, ip,
       NULLIF(jobInstanceId, '')::int, NULLIF(taskInstanceId, '')::int FROM s_event_log
ON CONFLICT (id) DO NOTHING;
INSERT INTO cadastre.aumentum_auditlog (id, sid, action, object, success, time, description)
SELECT id::uuid, NULLIF(sid, '')::int, NULLIF(action, '')::int, NULLIF(object, '')::uuid, success = '1', NULLIF(time, '')::timestamp, description FROM s_auditlog
ON CONFLICT (id) DO NOTHING;

-- Aumentum's settings, except those holding credentials, as aumentum.<category>.<name>.
INSERT INTO cadastre.app_setting (key, value, source)
SELECT 'aumentum.' || ltrim(category, '#') || '.' || name, value, 'aumentum' FROM s_dsssettings
WHERE name !~* '(pass|pwd|secret)' AND name NOT IN ('connectionConfig', 'RegParam')
ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value WHERE cadastre.app_setting.source = 'aumentum';

COMMIT;
SQL
cat <<SQL
SELECT format('%-22s file %6s | in database %6s  %s', 'users & groups', $(rows sec_sids), count(*), CASE WHEN count(*) >= $(rows sec_sids) THEN 'OK' ELSE 'CHECK' END) FROM cadastre.principal;
SELECT format('%-22s file %6s | in database %6s', 'group memberships', $(rows sec_sid_nesting), count(*)) FROM cadastre.principal_member;
SELECT format('%-22s file %6s | in database %6s', 'roles', $(rows role), count(*)) FROM cadastre.role;
SELECT format('%-22s file %6s | in database %6s', 'role grants', $(rows sec_policy), count(*)) FROM cadastre.principal_role;
SELECT format('%-22s file %6s | in database %6s', 'tasks', $(rows wftask), count(*)) FROM cadastre.wf_task;
SELECT format('%-22s file %6s | in database %6s', 'steps', $(rows Step), count(*)) FROM cadastre.wf_step;
SELECT format('%-22s file %6s | in database %6s', 'task steps', $(rows TaskSteps), count(*)) FROM cadastre.wf_task_step;
SELECT format('%-22s file %6s | in database %6s', 'job types', $(rows wfjob), count(*)) FROM cadastre.wf_job_type;
SELECT format('%-22s file %6s | in database %6s', 'task graph edges', $(rows wftaskflow), count(*)) FROM cadastre.wf_task_flow;
SELECT format('%-22s file %6s | in database %6s', 'task permissions', $(rows wftaskPermission), count(*)) FROM cadastre.wf_task_permission;
SELECT format('%-22s file %6s | in database %6s  %s', 'job instances', $(rows wfjobInstance), count(*), CASE WHEN count(*) = $(rows wfjobInstance) THEN 'OK' ELSE 'MISMATCH' END) FROM cadastre.wf_job_instance w JOIN s_wfjobinstance s ON s.id::int = w.id WHERE w.source = 'aumentum';
SELECT format('%-22s file %6s | in database %6s  %s', 'task instances', $(rows wfTaskInstance), count(*), CASE WHEN count(*) = $(rows wfTaskInstance) THEN 'OK' ELSE 'MISMATCH' END) FROM cadastre.wf_task_instance t JOIN s_wftaskinstance s ON s.id::int = t.id AND s.jobInstanceID::int = t.job_instance_id;
-- Clashes: an Aumentum id already used here for a job or task of this Cadastre (kept, not overwritten).
SELECT format('%-22s jobs %6s | tasks %6s  %s', 'number clashes',
  (SELECT count(*) FROM s_wfjobinstance s JOIN cadastre.wf_job_instance w ON w.id = s.id::int WHERE w.source IS DISTINCT FROM 'aumentum'),
  (SELECT count(*) FROM s_wftaskinstance s JOIN cadastre.wf_task_instance t ON t.id = s.id::int WHERE t.job_instance_id <> s.jobInstanceID::int),
  CASE WHEN EXISTS (SELECT 1 FROM s_wfjobinstance s JOIN cadastre.wf_job_instance w ON w.id = s.id::int WHERE w.source IS DISTINCT FROM 'aumentum')
         OR EXISTS (SELECT 1 FROM s_wftaskinstance s JOIN cadastre.wf_task_instance t ON t.id = s.id::int WHERE t.job_instance_id <> s.jobInstanceID::int)
       THEN 'CHECK: Aumentum rows with these numbers were not loaded' ELSE 'OK' END);
SELECT format('%-22s next new job #%s, next task #%s', 'this Cadastre''s range',
  (SELECT last_value + 1 FROM cadastre.wf_job_instance_seq), (SELECT last_value + 1 FROM cadastre.wf_task_instance_seq));
SELECT format('%-22s file %6s | reserved %6s  (ledger now %s; next UPIN PL%s)', 'UPIN register', $(rows UPIN),
  (SELECT count(*) FROM cadastre.issued_upin WHERE source = 'aumentum:register'), (SELECT count(*) FROM cadastre.issued_upin),
  lpad((SELECT CASE WHEN is_called THEN last_value + 1 ELSE last_value END FROM cadastre.parcel_number_seq)::text, 12, '0'));
SELECT format('%-22s objects %6s | linked to jobs %6s  (%s MB in Aumentum''s document store)', 'job attachments',
  (SELECT count(*) FROM s_object WHERE class = '3' AND deleted = '0'), (SELECT count(*) FROM cadastre.wf_job_file WHERE store = 'aumentum'),
  (SELECT round(sum(size_bytes) / 1e6) FROM cadastre.wf_job_file WHERE store = 'aumentum'));
SELECT format('%-22s file %6s | in database %6s', 'event log', $(rows event_log), count(*)) FROM cadastre.aumentum_event_log;
SELECT format('%-22s file %6s | in database %6s', 'security audit log', $(rows auditlog), count(*)) FROM cadastre.aumentum_auditlog;
SELECT format('%-22s file %6s | imported %6s (credential-bearing settings skipped)', 'settings', $(rows dsssettings), count(*)) FROM cadastre.app_setting WHERE key LIKE 'aumentum.%';
SELECT format('  jobs %-16s %6s', s.label, count(*)) FROM cadastre.wf_job_instance j JOIN cadastre.wf_job_status s ON s.code = j.status GROUP BY s.label, s.code ORDER BY s.code;
SELECT format('  job instances with a Registry transaction: %s; jobs without a known assignee: %s; open task instances: %s',
  (SELECT count(*) FROM cadastre.wf_job_instance WHERE lrs_transaction_number IS NOT NULL),
  (SELECT count(*) FROM cadastre.wf_job_instance WHERE assigned_sid IS NULL),
  (SELECT count(*) FROM cadastre.wf_task_instance WHERE ended IS NULL));
SQL
} | $PSQL
