#!/usr/bin/env bash
# Phase 2 exit check: replay every imported Aumentum job's real task history against the new
# workflow engine's rules (wfengine): a job starts on its job type's start task, each move goes
# along an edge of the job type's task graph (a repeated task = a reassignment on that task), and
# a completed job ends on its end task. Read-only. Prints a summary and every move the engine
# would refuse, grouped, with example jobs.
#
#   tools/import/replay-check.sh            (PSQL as in the other loaders)
set -euo pipefail
PSQL="${PSQL:-docker exec -i cadastre-db psql -U cadastre -d cadastre -v ON_ERROR_STOP=1 -q -At}"
$PSQL <<'SQL'
CREATE TEMP VIEW hist AS
SELECT ti.job_instance_id AS job, j.job_type_id AS jt, ti.task_id AS task,
       row_number() OVER (PARTITION BY ti.job_instance_id ORDER BY COALESCE(ti.assigned, ti.started, ti.ended), ti.id) AS n
FROM cadastre.wf_task_instance ti JOIN cadastre.wf_job_instance j ON j.id = ti.job_instance_id
WHERE j.source = 'aumentum';

CREATE TEMP TABLE moves AS
SELECT a.job, a.jt, a.task AS from_task, b.task AS to_task,
       a.task = b.task AS same_task,
       EXISTS (SELECT 1 FROM cadastre.wf_task_flow f WHERE f.job_type_id = a.jt AND f.task_id = a.task AND f.next_task_id = b.task) AS is_edge
FROM hist a JOIN hist b ON b.job = a.job AND b.n = a.n + 1;

CREATE TEMP TABLE verdict AS
SELECT j.id AS job, j.job_type_id AS jt, j.status,
       (SELECT h.task FROM hist h WHERE h.job = j.id AND h.n = 1) AS first_task,
       (SELECT h.task FROM hist h WHERE h.job = j.id ORDER BY h.n DESC LIMIT 1) AS last_task,
       (SELECT count(*) FROM hist h WHERE h.job = j.id) AS tasks,
       COALESCE((SELECT bool_and(m.is_edge OR m.same_task) FROM moves m WHERE m.job = j.id), true) AS moves_ok,
       t.start_task_id, t.end_task_id
FROM cadastre.wf_job_instance j JOIN cadastre.wf_job_type t ON t.id = j.job_type_id
WHERE j.source = 'aumentum';

SELECT format('jobs replayed: %s (with task history: %s)', count(*), count(*) FILTER (WHERE tasks > 0)) FROM verdict;
SELECT format('  started on the start task:          %s', count(*) FILTER (WHERE first_task = start_task_id)) FROM verdict WHERE tasks > 0;
SELECT format('  every move allowed by the engine:   %s', count(*) FILTER (WHERE moves_ok)) FROM verdict WHERE tasks > 0;
SELECT format('  completed jobs ending on end task:  %s of %s completed', count(*) FILTER (WHERE last_task = end_task_id), count(*)) FROM verdict WHERE status = 3 AND tasks > 0;
SELECT format('  FULLY REPLAYABLE:                   %s of %s jobs with history (%s%%)',
  count(*) FILTER (WHERE first_task = start_task_id AND moves_ok AND (status <> 3 OR last_task = end_task_id)),
  count(*), round(100.0 * count(*) FILTER (WHERE first_task = start_task_id AND moves_ok AND (status <> 3 OR last_task = end_task_id)) / NULLIF(count(*), 0), 2))
FROM verdict WHERE tasks > 0;
SELECT format('  jobs with no task history:          %s', count(*)) FROM verdict WHERE tasks = 0;
SELECT format('  + reproducible with End Job:        %s (completed before reaching the end task, as Aumentum allows)',
  count(*) FILTER (WHERE first_task = start_task_id AND moves_ok AND status = 3 AND last_task IS DISTINCT FROM end_task_id)) FROM verdict WHERE tasks > 0;
SELECT format('  = REPRODUCIBLE IN THE NEW SYSTEM:   %s of %s (%s%%); the rest followed an earlier version of their job type''s workflow',
  count(*) FILTER (WHERE first_task = start_task_id AND moves_ok), count(*),
  round(100.0 * count(*) FILTER (WHERE first_task = start_task_id AND moves_ok) / NULLIF(count(*), 0), 2)) FROM verdict WHERE tasks > 0;
SELECT format('  earlier-workflow jobs still open:   %s (job ids %s): close them with End Job', count(*), string_agg(job::text, ', ' ORDER BY job))
FROM verdict v JOIN cadastre.wf_job_status s ON s.code = v.status WHERE v.tasks > 0 AND s.is_open AND NOT (v.first_task = v.start_task_id AND v.moves_ok);

SELECT '--- moves the engine would refuse (job type: from -> to, count, example jobs) ---';
SELECT format('  %s: %s -> %s  x%s  e.g. %s', jt.name, ft.name, tt.name, count(*), string_agg(DISTINCT m.job::text, ', ' ORDER BY m.job::text) FILTER (WHERE true))
FROM moves m JOIN cadastre.wf_job_type jt ON jt.id = m.jt JOIN cadastre.wf_task ft ON ft.id = m.from_task JOIN cadastre.wf_task tt ON tt.id = m.to_task
WHERE NOT m.is_edge AND NOT m.same_task
GROUP BY jt.name, ft.name, tt.name ORDER BY count(*) DESC LIMIT 25;

SELECT '--- jobs not starting on their start task (job type: first task, count) ---';
SELECT format('  %s: starts on %s (start task %s)  x%s', jt.name, ft.name, st.name, count(*))
FROM verdict v JOIN cadastre.wf_job_type jt ON jt.id = v.jt JOIN cadastre.wf_task ft ON ft.id = v.first_task LEFT JOIN cadastre.wf_task st ON st.id = v.start_task_id
WHERE v.tasks > 0 AND v.first_task IS DISTINCT FROM v.start_task_id GROUP BY jt.name, ft.name, st.name ORDER BY count(*) DESC LIMIT 15;

SELECT '--- completed jobs not ending on their end task (job type: last task, count) ---';
SELECT format('  %s: ends on %s (end task %s)  x%s', jt.name, lt.name, et.name, count(*))
FROM verdict v JOIN cadastre.wf_job_type jt ON jt.id = v.jt JOIN cadastre.wf_task lt ON lt.id = v.last_task LEFT JOIN cadastre.wf_task et ON et.id = v.end_task_id
WHERE v.status = 3 AND v.tasks > 0 AND v.last_task IS DISTINCT FROM v.end_task_id GROUP BY jt.name, lt.name, et.name ORDER BY count(*) DESC LIMIT 15;
SQL
