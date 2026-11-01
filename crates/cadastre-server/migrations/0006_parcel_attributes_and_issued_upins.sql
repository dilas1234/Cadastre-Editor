-- Parcel attributes (Aumentum sde.ParcelAttributeTable) and the full issued-UPIN list.
--
-- ParcelAttributeTable carries the Registry linkage: LRSID (the Registry property a parcel is
-- linked to), LinkageStatus and LinkageDate, plus the fields the Registry matches on (property
-- number, plot, block, plan number, LGA) and lineage/mutation history. Losing any of it would break
-- links that completed, paid Registry transactions depend on, so every column is kept.

-- Staging, as exported (aumentum-export.ps1 -> parcel-attributes.psv). Text, verbatim.
CREATE TABLE IF NOT EXISTS import.parcel_attribute (
    id bigserial PRIMARY KEY,
    run_id bigint NOT NULL REFERENCES import.run (id) ON DELETE CASCADE,
    object_id integer NOT NULL,
    dataset_name text, region_name text, parcel_id text, upin text, parcel_name text, parent text,
    property_number text, property_type text, plot text, block text, location text,
    parcel_area text, area_unit text, plan_number text, lga text, status text,
    can_rollback text, is_strata text, mutation_type text,
    creation_date text, last_edit_date text, consolidation_date text,
    archive_reason text, archive_date text,
    lrs_id text, linkage_status text, linkage_date text, surveyor_general text
);
CREATE INDEX IF NOT EXISTS import_parcel_attribute_run_idx ON import.parcel_attribute (run_id);

-- Staging for every UPIN found in any version or history state (upins-issued.psv).
CREATE TABLE IF NOT EXISTS import.issued_upin (
    run_id bigint NOT NULL REFERENCES import.run (id) ON DELETE CASCADE,
    upin text NOT NULL,
    first_source text,
    occurrences integer,
    PRIMARY KEY (run_id, upin)
);

-- Live attributes, one row per Aumentum attribute record (OBJECTID is unique in the table).
-- Kept separate from cadastre.parcel so every Aumentum field survives exactly; joined by UPIN.
CREATE TABLE IF NOT EXISTS cadastre.parcel_attribute (
    object_id          integer PRIMARY KEY,
    upin               text,
    dataset_name       text,
    region_name        text,
    parcel_id          uuid,
    parcel_name        text,
    parent             text,
    property_number    text,
    property_type      text,
    plot               text,
    block              text,
    location           text,
    parcel_area        double precision,
    area_unit          smallint,
    plan_number        text,
    lga                text,
    status             text,
    can_rollback       smallint,
    is_strata          smallint,
    mutation_type      smallint,
    creation_date      timestamp,
    last_edit_date     timestamp,
    consolidation_date timestamp,
    archive_reason     text,
    archive_date       timestamp,
    lrs_id             text,              -- Registry property the parcel is linked to
    linkage_status     smallint,          -- 1 = linked to the Registry
    linkage_date       text,              -- as Aumentum stores it (yyyymmdd text)
    surveyor_general   text,
    import_run         bigint,
    updated_at         timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS parcel_attribute_upin_idx ON cadastre.parcel_attribute (upin);
CREATE INDEX IF NOT EXISTS parcel_attribute_lrs_idx ON cadastre.parcel_attribute (lrs_id) WHERE lrs_id IS NOT NULL;
