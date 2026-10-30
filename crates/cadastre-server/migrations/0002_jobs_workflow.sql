-- Phase 3: jobs, a configurable workflow, and edit sessions with resource locks.

-- ---------------------------------------------------------------------------
-- Configurable workflow (FR-WF-01..03, FR-ADM-02).
-- Stages form an ordered set; transitions are the allowed (from, action) -> to
-- moves, each gated by a role. The default workflow below is seed data only —
-- admins add/replace stages and transitions at runtime.
-- ---------------------------------------------------------------------------
CREATE TABLE cadastre.workflow_stage (
    id          uuid PRIMARY KEY DEFAULT uuid_generate_v4(),
    code        text NOT NULL UNIQUE,
    name        text NOT NULL,
    seq         integer NOT NULL,
    is_initial  boolean NOT NULL DEFAULT false,
    is_terminal boolean NOT NULL DEFAULT false
);

-- Exactly one initial stage.
CREATE UNIQUE INDEX workflow_stage_single_initial
    ON cadastre.workflow_stage (is_initial) WHERE is_initial;

CREATE TABLE cadastre.workflow_transition (
    id            uuid PRIMARY KEY DEFAULT uuid_generate_v4(),
    action        text NOT NULL,
    from_stage_id uuid NOT NULL REFERENCES cadastre.workflow_stage (id) ON DELETE CASCADE,
    to_stage_id   uuid NOT NULL REFERENCES cadastre.workflow_stage (id) ON DELETE CASCADE,
    required_role text NOT NULL,
    -- One outgoing transition per (stage, action).
    UNIQUE (from_stage_id, action)
);

-- Default workflow: draft -> submitted -> under_review -> approved | rejected,
-- with a reopen path back to draft.
INSERT INTO cadastre.workflow_stage (code, name, seq, is_initial, is_terminal) VALUES
    ('draft',        'Draft',        1, true,  false),
    ('submitted',    'Submitted',    2, false, false),
    ('under_review', 'Under Review', 3, false, false),
    ('approved',     'Approved',     4, false, true),
    ('rejected',     'Rejected',     5, false, true);

INSERT INTO cadastre.workflow_transition (action, from_stage_id, to_stage_id, required_role)
SELECT v.action, f.id, t.id, v.required_role
FROM (VALUES
    ('submit',       'draft',        'submitted',    'editor'),
    ('start_review', 'submitted',    'under_review', 'reviewer'),
    ('approve',      'under_review', 'approved',     'approver'),
    ('reject',       'under_review', 'rejected',     'approver'),
    ('reopen',       'rejected',     'draft',        'editor')
) AS v(action, from_code, to_code, required_role)
JOIN cadastre.workflow_stage f ON f.code = v.from_code
JOIN cadastre.workflow_stage t ON t.code = v.to_code;

-- ---------------------------------------------------------------------------
-- Jobs (FR-JOB-*). Extends the Phase 2 placeholder with a title, assignee and the
-- current workflow stage. `status` mirrors the current stage code for convenience.
-- ---------------------------------------------------------------------------
ALTER TABLE cadastre.job
    ADD COLUMN title            text,
    ADD COLUMN assigned_to      text,
    ADD COLUMN updated_by       text,
    ADD COLUMN current_stage_id uuid REFERENCES cadastre.workflow_stage (id);

-- Backfill any existing rows to the initial stage, then require the column.
UPDATE cadastre.job
    SET current_stage_id = (SELECT id FROM cadastre.workflow_stage WHERE is_initial),
        status = 'draft'
    WHERE current_stage_id IS NULL;
ALTER TABLE cadastre.job ALTER COLUMN current_stage_id SET NOT NULL;

-- Per-job workflow history (who moved it where, when).
CREATE TABLE cadastre.job_event (
    id         bigserial PRIMARY KEY,
    job_id     uuid NOT NULL REFERENCES cadastre.job (id) ON DELETE CASCADE,
    action     text NOT NULL,
    from_stage text,
    to_stage   text NOT NULL,
    actor      text NOT NULL,
    note       text,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX job_event_job_idx ON cadastre.job_event (job_id, id);

-- ---------------------------------------------------------------------------
-- Edit sessions and resource locks (concurrent-edit control).
-- A session is a user's working context on a job; it holds locks on the resources
-- being edited. A resource can be locked by at most one session at a time.
-- ---------------------------------------------------------------------------
CREATE TABLE cadastre.edit_session (
    id                uuid PRIMARY KEY DEFAULT uuid_generate_v4(),
    job_id            uuid REFERENCES cadastre.job (id) ON DELETE CASCADE,
    user_subject      text NOT NULL,
    status            text NOT NULL DEFAULT 'active',  -- active | released | expired
    started_at        timestamptz NOT NULL DEFAULT now(),
    last_heartbeat_at timestamptz NOT NULL DEFAULT now(),
    released_at       timestamptz
);

CREATE INDEX edit_session_active_user_idx
    ON cadastre.edit_session (user_subject) WHERE status = 'active';

CREATE TABLE cadastre.edit_lock (
    id            uuid PRIMARY KEY DEFAULT uuid_generate_v4(),
    session_id    uuid NOT NULL REFERENCES cadastre.edit_session (id) ON DELETE CASCADE,
    resource_type text NOT NULL,  -- parcel | beacon | job
    resource_id   uuid NOT NULL,
    acquired_at   timestamptz NOT NULL DEFAULT now(),
    -- At most one lock per resource.
    UNIQUE (resource_type, resource_id)
);

CREATE INDEX edit_lock_session_idx ON cadastre.edit_lock (session_id);
