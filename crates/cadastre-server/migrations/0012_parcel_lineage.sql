-- Phase 4: mutations and lineage (Aumentum parity). Agreed with PLAGIS (2026-11-02): one new
-- table, mirroring Aumentum's MCServer dbo.parcelLineage, and "one ACTIVE parcel per UPIN" so
-- retired parcels (historic / archived) stay in the fabric as history, as they do in Aumentum.

-- A UPIN has at most one posted (active) parcel; its retired shapes keep the same UPIN.
-- UPINs are still never reused: cadastre.issued_upin is unchanged.
ALTER TABLE cadastre.parcel DROP CONSTRAINT IF EXISTS parcel_parcel_number_key;
CREATE UNIQUE INDEX IF NOT EXISTS parcel_one_posted_per_upin ON cadastre.parcel (parcel_number) WHERE status = 'posted';
CREATE INDEX IF NOT EXISTS parcel_number_idx ON cadastre.parcel (parcel_number);
-- Parcel statuses: draft, submitted, posted (Aumentum 2 active), historic (3: replaced by a
-- mutation), archived (4: retired, with an archive reason).

-- Aumentum dbo.parcelLineage: one row per parcel per mutation. A mutation's parent has a row
-- with status 0 (history) and changed_to_history; each child has a row with status 1 (active),
-- parent_id = the parent, changed_to_active, and the mutation's id and type. A merge's child
-- has one row per parent. parcel_id is Aumentum's ParcelID (ParcelAttributeTable.ParcelID) for
-- Aumentum's parcels and cadastre.parcel.id for parcels created here.
CREATE TABLE IF NOT EXISTS cadastre.parcel_lineage (
    id                 uuid PRIMARY KEY DEFAULT uuid_generate_v4(),
    parcel_id          uuid NOT NULL,
    upin               text,
    status             smallint NOT NULL CHECK (status IN (0, 1)),   -- 0 history, 1 active
    parent_id          uuid,
    parent_upin        text,
    changed_to_active  timestamp,
    changed_to_history timestamp,
    mutation_id        text,
    -- Aumentum mutationTypeID: 1 split (1 -> many), 2 merge (many -> 1), 3 re-survey (1 -> 1, new
    -- UPIN); 4 and 6 occur in Aumentum's history with meanings PLAGIS is to confirm. NULL with
    -- status 0 and no children: a retirement (the parcel archived, reason in `reason`).
    mutation_type      smallint,
    reason             text,
    -- A mutation recorded in a workflow job and applied when the job's parcels are committed.
    pending            boolean NOT NULL DEFAULT false,
    wf_job_instance_id integer REFERENCES cadastre.wf_job_instance (id) ON DELETE SET NULL,
    created_by         text,
    created_at         timestamptz NOT NULL DEFAULT now(),
    source             text NOT NULL DEFAULT 'cadastre' CHECK (source IN ('aumentum', 'cadastre')),
    aumentum_id        uuid UNIQUE
);
CREATE INDEX IF NOT EXISTS parcel_lineage_parcel_idx ON cadastre.parcel_lineage (parcel_id);
CREATE INDEX IF NOT EXISTS parcel_lineage_parent_idx ON cadastre.parcel_lineage (parent_id);
CREATE INDEX IF NOT EXISTS parcel_lineage_upin_idx ON cadastre.parcel_lineage (upin);
CREATE INDEX IF NOT EXISTS parcel_lineage_parent_upin_idx ON cadastre.parcel_lineage (parent_upin);
CREATE INDEX IF NOT EXISTS parcel_lineage_pending_idx ON cadastre.parcel_lineage (wf_job_instance_id) WHERE pending;
