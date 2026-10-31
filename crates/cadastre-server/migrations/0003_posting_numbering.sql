-- Phase 6: posting, parcel numbering, and parcel↔job linkage.

-- A parcel is captured under a job and only gets its official number when the job is posted,
-- so the number is nullable until then (UNIQUE still allows many NULLs in Postgres).
ALTER TABLE cadastre.parcel ALTER COLUMN parcel_number DROP NOT NULL;

ALTER TABLE cadastre.parcel
    ADD COLUMN job_id    uuid REFERENCES cadastre.job (id) ON DELETE SET NULL,
    ADD COLUMN posted_at timestamptz;

CREATE INDEX parcel_job_idx ON cadastre.parcel (job_id);

-- Allocator for official parcel numbers (UPIN) — formatted as 'PL' + 12 digits in the app
-- (FR-NUM-01), matching the Aumentum convention.
CREATE SEQUENCE IF NOT EXISTS cadastre.parcel_number_seq AS bigint START WITH 1 INCREMENT BY 1;
