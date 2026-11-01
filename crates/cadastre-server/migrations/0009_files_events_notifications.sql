-- Job attachments, Aumentum's event and audit history, and task notifications (Phase 2).

-- Files attached to job instances. Aumentum's (15.6k scanned plans, mc_class objects) stay in
-- Aumentum's document store and are referenced by their store path; new uploads go to the new
-- store. `store` says which root `store_path` is relative to.
CREATE TABLE IF NOT EXISTS cadastre.wf_job_file (
    id                 uuid PRIMARY KEY DEFAULT uuid_generate_v4(),
    job_instance_id    integer NOT NULL REFERENCES cadastre.wf_job_instance (id) ON DELETE CASCADE,
    file_name          text NOT NULL,
    store              text NOT NULL CHECK (store IN ('aumentum', 'cadastre')),
    store_path         text NOT NULL,
    size_bytes         bigint,
    mime_type          text,
    uploaded_by_sid    integer REFERENCES cadastre.principal (sid),
    uploaded_by        text,
    uploaded_at        timestamp,
    aumentum_object_id uuid UNIQUE
);
CREATE INDEX IF NOT EXISTS wf_job_file_job_idx ON cadastre.wf_job_file (job_instance_id);

-- Aumentum's typed event log (event, event_log) and its security audit log (auditlog), kept
-- verbatim for history and the productivity report. New activity is in cadastre.audit_log.
CREATE TABLE IF NOT EXISTS cadastre.aumentum_event (
    id   integer PRIMARY KEY,
    name text NOT NULL
);
CREATE TABLE IF NOT EXISTS cadastre.aumentum_event_log (
    id               integer PRIMARY KEY,
    time             timestamp,
    event_id         integer,
    user_sid         integer,
    module           text,
    region_id        text,
    old_value        text,
    new_value        text,
    ip               text,
    job_instance_id  integer,
    task_instance_id integer
);
CREATE INDEX IF NOT EXISTS aumentum_event_log_time_idx ON cadastre.aumentum_event_log (time);
CREATE TABLE IF NOT EXISTS cadastre.aumentum_auditlog (
    id          uuid PRIMARY KEY,
    sid         integer,
    action      integer,
    object      uuid,
    success     boolean,
    time        timestamp,
    description text
);

-- Task notifications (Aumentum EmailTemplate.xml "UserTask"). Every assignment is recorded;
-- it is sent when an SMTP server is configured (mail.smtp_server), else marked 'skipped', as
-- Aumentum behaves with its blank smtpServer.
CREATE TABLE IF NOT EXISTS cadastre.mail_template (
    feature text PRIMARY KEY,
    subject text NOT NULL,
    body    text NOT NULL
);
INSERT INTO cadastre.mail_template (feature, subject, body) VALUES ('UserTask',
    'PLAGIS CADASTRE *JobInstance* Assigned - NOREPLY',
    '*FinalUser*, <br />You have been assigned the following task *Task* in PLAGIS CADASTRE<br/><br/>Use the PLAGIS CADASTRE Editor to view and execute this task<br/><br/>Please do not reply to this email <br/><br/>PLAGIS CADASTRE Automatic Email')
ON CONFLICT (feature) DO NOTHING;

CREATE TABLE IF NOT EXISTS cadastre.wf_notification (
    id              bigserial PRIMARY KEY,
    feature         text NOT NULL,
    job_instance_id integer REFERENCES cadastre.wf_job_instance (id) ON DELETE CASCADE,
    task_id         integer,
    to_sid          integer,
    to_email        text,
    subject         text NOT NULL,
    body            text NOT NULL,
    status          text NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'sent', 'skipped', 'failed')),
    detail          text,
    created_at      timestamptz NOT NULL DEFAULT now(),
    sent_at         timestamptz
);
CREATE INDEX IF NOT EXISTS wf_notification_status_idx ON cadastre.wf_notification (status);
