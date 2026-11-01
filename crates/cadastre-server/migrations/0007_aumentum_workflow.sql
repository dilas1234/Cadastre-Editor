-- Aumentum workflow engine, reproduced (Phase 2).
--
-- Mirrors Aumentum's MCServer model one-to-one and keeps its ids, so every job, task and user
-- stays traceable to the system it replaces:
--   wfjob -> wf_job_type, wftask -> wf_task, Step -> wf_step, TaskSteps -> wf_task_step,
--   wftaskflow -> wf_task_flow, wftaskPermission -> wf_task_permission,
--   sec_sids / sec_sid_nesting / sec_userinfo / role / sec_policy -> principal* / role*,
--   wfjobInstance -> wf_job_instance, wfTaskInstance -> wf_task_instance.
-- A job instance moves only along its job type's task graph (rework loops included), and only
-- principals permitted for a task may act on it.

-- Users and groups (sec_sids), group membership (sec_sid_nesting), contact details (sec_userinfo).
CREATE TABLE IF NOT EXISTS cadastre.principal (
    sid          integer PRIMARY KEY,
    code         text NOT NULL,             -- login / group code
    name         text,
    description  text,
    is_user      boolean NOT NULL,
    is_locked    boolean NOT NULL DEFAULT false,
    is_system    boolean NOT NULL DEFAULT false,
    os_account   text,                      -- e.g. PLAGIS\jdoe: maps to the AD / Keycloak account
    email        text,
    department   text,
    job_title    text,
    phone        text
);
CREATE UNIQUE INDEX IF NOT EXISTS principal_code_idx ON cadastre.principal (lower(code));
CREATE INDEX IF NOT EXISTS principal_os_account_idx ON cadastre.principal (lower(os_account));

CREATE TABLE IF NOT EXISTS cadastre.principal_member (
    group_sid  integer NOT NULL REFERENCES cadastre.principal (sid) ON DELETE CASCADE,
    member_sid integer NOT NULL REFERENCES cadastre.principal (sid) ON DELETE CASCADE,
    PRIMARY KEY (group_sid, member_sid)
);

-- Application roles (role) and their grants to users/groups (sec_policy).
CREATE TABLE IF NOT EXISTS cadastre.role (
    code        text PRIMARY KEY,
    description text
);
CREATE TABLE IF NOT EXISTS cadastre.principal_role (
    sid    integer NOT NULL REFERENCES cadastre.principal (sid) ON DELETE CASCADE,
    role   text NOT NULL REFERENCES cadastre.role (code) ON DELETE CASCADE,
    allow  boolean NOT NULL DEFAULT true,
    PRIMARY KEY (sid, role)
);

-- Definitions.
CREATE TABLE IF NOT EXISTS cadastre.wf_task (
    id          integer PRIMARY KEY,
    name        text NOT NULL,
    description text
);
CREATE TABLE IF NOT EXISTS cadastre.wf_step (
    id               integer PRIMARY KEY,
    name             text NOT NULL,
    icon             text,
    commands         text[] NOT NULL DEFAULT '{}',   -- the tools the step enables, in order
    execute_or_apply text
);
CREATE TABLE IF NOT EXISTS cadastre.wf_task_step (
    task_id integer NOT NULL REFERENCES cadastre.wf_task (id) ON DELETE CASCADE,
    step_id integer NOT NULL REFERENCES cadastre.wf_step (id) ON DELETE CASCADE,
    ord     integer NOT NULL DEFAULT 0,
    PRIMARY KEY (task_id, step_id)
);
CREATE TABLE IF NOT EXISTS cadastre.wf_job_type (
    id            integer PRIMARY KEY,
    name          text NOT NULL,
    description   text,
    start_task_id integer REFERENCES cadastre.wf_task (id),
    end_task_id   integer REFERENCES cadastre.wf_task (id),
    design_xml    text,                     -- Aumentum's workflow designer document, verbatim
    active        boolean NOT NULL DEFAULT true
);
-- Edges of a job type's task graph. A task may have several next tasks (a branch or a rework
-- loop); next_task_id NULL marks a task that ends the job.
CREATE TABLE IF NOT EXISTS cadastre.wf_task_flow (
    id           bigserial PRIMARY KEY,
    job_type_id  integer NOT NULL REFERENCES cadastre.wf_job_type (id) ON DELETE CASCADE,
    task_id      integer NOT NULL REFERENCES cadastre.wf_task (id),
    task_order   integer,
    next_task_id integer REFERENCES cadastre.wf_task (id),
    UNIQUE (job_type_id, task_id, next_task_id)
);
CREATE TABLE IF NOT EXISTS cadastre.wf_task_permission (
    task_id    integer NOT NULL REFERENCES cadastre.wf_task (id) ON DELETE CASCADE,
    sid        integer NOT NULL REFERENCES cadastre.principal (sid) ON DELETE CASCADE,
    is_default boolean NOT NULL DEFAULT false,   -- the default assignee for the task
    PRIMARY KEY (task_id, sid)
);
CREATE TABLE IF NOT EXISTS cadastre.wf_job_permission (
    job_type_id integer NOT NULL REFERENCES cadastre.wf_job_type (id) ON DELETE CASCADE,
    sid         integer NOT NULL REFERENCES cadastre.principal (sid) ON DELETE CASCADE,
    can_start boolean NOT NULL DEFAULT false, can_end boolean NOT NULL DEFAULT false,
    can_update boolean NOT NULL DEFAULT false, can_delete boolean NOT NULL DEFAULT false,
    can_execute boolean NOT NULL DEFAULT false,
    PRIMARY KEY (job_type_id, sid)
);

-- Job status codes as Aumentum stores them. Labels for 1-3 and 8 follow from the data (see
-- docs); 4, 5 and 7 are ended states whose exact meaning PLAGIS is to confirm. The raw code is
-- what every job keeps.
CREATE TABLE IF NOT EXISTS cadastre.wf_job_status (
    code      smallint PRIMARY KEY,
    label     text NOT NULL,
    is_open   boolean NOT NULL,
    confirmed boolean NOT NULL DEFAULT false
);
INSERT INTO cadastre.wf_job_status (code, label, is_open, confirmed) VALUES
    (1, 'New', true, false),
    (2, 'In progress', true, false),
    (3, 'Completed', false, false),
    (4, 'Ended (code 4)', false, false),
    (5, 'Ended (code 5)', false, false),
    (7, 'Ended (code 7)', false, false),
    (8, 'On hold', true, false)
ON CONFLICT (code) DO NOTHING;

-- Work.
CREATE SEQUENCE IF NOT EXISTS cadastre.wf_job_instance_seq;
CREATE TABLE IF NOT EXISTS cadastre.wf_job_instance (
    id                      integer PRIMARY KEY DEFAULT nextval('cadastre.wf_job_instance_seq'),
    job_type_id             integer NOT NULL REFERENCES cadastre.wf_job_type (id),
    current_task_id         integer REFERENCES cadastre.wf_task (id),
    assigned_sid            integer REFERENCES cadastre.principal (sid),
    priority                text,
    status                  smallint NOT NULL REFERENCES cadastre.wf_job_status (code),
    name                    text,
    comment                 text,
    parameters              text,
    lrs_transaction_number  text,           -- the Registry transaction this job serves
    lrs_key                 text,
    lrs_properties          text,           -- Registry properties (XML, verbatim)
    link_parcels_properties text,           -- parcel <-> property links (XML, verbatim)
    created                 timestamp,
    started                 timestamp,
    updated                 timestamp,
    ended                   timestamp,
    source                  text NOT NULL DEFAULT 'cadastre'  -- 'aumentum' when imported
);
ALTER SEQUENCE cadastre.wf_job_instance_seq OWNED BY cadastre.wf_job_instance.id;
CREATE INDEX IF NOT EXISTS wf_job_instance_status_idx ON cadastre.wf_job_instance (status);
CREATE INDEX IF NOT EXISTS wf_job_instance_assigned_idx ON cadastre.wf_job_instance (assigned_sid);
CREATE INDEX IF NOT EXISTS wf_job_instance_lrs_idx ON cadastre.wf_job_instance (lrs_transaction_number);

CREATE SEQUENCE IF NOT EXISTS cadastre.wf_task_instance_seq;
CREATE TABLE IF NOT EXISTS cadastre.wf_task_instance (
    id              integer PRIMARY KEY DEFAULT nextval('cadastre.wf_task_instance_seq'),
    job_instance_id integer NOT NULL REFERENCES cadastre.wf_job_instance (id) ON DELETE CASCADE,
    job_type_id     integer,
    task_id         integer NOT NULL REFERENCES cadastre.wf_task (id),
    sid             integer REFERENCES cadastre.principal (sid),
    assigned        timestamp,
    started         timestamp,
    ended           timestamp,
    comment         text,
    parameters      text
);
ALTER SEQUENCE cadastre.wf_task_instance_seq OWNED BY cadastre.wf_task_instance.id;
CREATE INDEX IF NOT EXISTS wf_task_instance_job_idx ON cadastre.wf_task_instance (job_instance_id);

-- New ids continue above the imported Aumentum ids.
CREATE OR REPLACE FUNCTION cadastre.wf_sync_sequences() RETURNS void LANGUAGE plpgsql AS $$
BEGIN
    PERFORM setval('cadastre.wf_job_instance_seq', GREATEST(1, COALESCE((SELECT max(id) FROM cadastre.wf_job_instance), 0)), (SELECT count(*) > 0 FROM cadastre.wf_job_instance));
    PERFORM setval('cadastre.wf_task_instance_seq', GREATEST(1, COALESCE((SELECT max(id) FROM cadastre.wf_task_instance), 0)), (SELECT count(*) > 0 FROM cadastre.wf_task_instance));
END $$;
